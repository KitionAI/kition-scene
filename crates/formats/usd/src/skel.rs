//! UsdSkel in and out: a Skeleton prim is an armature (joints are bone paths, bind transforms
//! the bones' rest frames in skeleton space with +Y along the bone), a SkelAnimation its pose
//! over time (joint transforms relative to their parents) and its blend shape weights, and a
//! bound mesh's joint indices and weights its vertex groups, its BlendShape children its shape
//! keys.

use std::collections::{BTreeMap, HashMap};

use kition_scene_exchange::{Imported, ImportedChannel};
use kition_scene_model::armature::{Armature, Bone, roll_towards};
use kition_scene_model::glam::{DMat4, Mat3, Mat4, Quat, Vec3};
use kition_scene_model::scene::matrix_euler;
use kition_scene_model::vertex_data::{ShapeKey, VertexGroup};
use kition_scene_model::{Interpolation, Mesh, ObjectData, Property, SceneError};

use crate::usda::{Prim, Value, invalid};

/// Where a skeleton's bone lengths are kept by the reference application's exporter.
pub(crate) const BONE_LENGTHS: &str = "primvars:blender:bone_lengths";

/// The most influences a vertex keeps when written.
pub(crate) const MAX_INFLUENCES: usize = 8;

/// Whether a skeleton only carries blend shape weights (the exporters' way to animate shape keys
/// of a mesh no armature deforms): at most one joint, at rest where it stands, and no joint
/// animation of its own.
pub(crate) fn is_carrier(prim: &Prim) -> bool {
    let joints = tokens(prim.value("joints"));
    let binds = matrices(prim.value("bindTransforms"));
    let animated = prim
        .children
        .iter()
        .any(|child| child.type_name == "SkelAnimation" && ["translations", "rotations", "scales"].iter().any(|name| child.property(name).is_some()));
    joints.len() <= 1 && binds.iter().all(|bind| bind.abs_diff_eq(Mat4::IDENTITY, 1e-6)) && !animated
}

/// Matrices of a `matrix4d[]` value (rows as USD writes them; glam reads them as columns).
fn matrices(value: Option<&Value>) -> Vec<Mat4> {
    value
        .map(|value| {
            value
                .items()
                .iter()
                .filter_map(|item| <[f64; 16]>::try_from(item.numbers().as_slice()).ok())
                .map(|array| DMat4::from_cols_array(&array).as_mat4())
                .collect()
        })
        .unwrap_or_default()
}

fn tokens(value: Option<&Value>) -> Vec<String> {
    value.map(|value| value.items().iter().filter_map(|item| item.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

fn orthonormal(matrix: Mat3) -> Mat3 {
    let y = matrix.y_axis.normalize_or(Vec3::Y);
    let z = (matrix.z_axis - y * y.dot(matrix.z_axis)).normalize_or(Vec3::Z);
    Mat3::from_cols(y.cross(z), y, z)
}

/// A skeleton's joints: (bone name, parent joint index).
fn joint_tree(joints: &[String]) -> Vec<(String, Option<usize>)> {
    let mut names: Vec<String> = Vec::with_capacity(joints.len());
    joints
        .iter()
        .map(|joint| {
            let base = joint.rsplit('/').next().unwrap_or(joint).to_string();
            let name = kition_scene_model::scene::unique_name(&base, |candidate| names.iter().any(|other| other == candidate));
            names.push(name.clone());
            let parent = joint.rsplit_once('/').and_then(|(parent, _)| joints.iter().position(|other| other == parent));
            (name, parent)
        })
        .collect()
}

/// Rest frames of a skeleton's joints in skeleton space: its bind transforms, else its rest
/// transforms composed down the joint tree.
fn rest_frames(prim: &Prim, tree: &[(String, Option<usize>)]) -> Vec<Mat4> {
    let binds = matrices(prim.value("bindTransforms"));
    if binds.len() == tree.len() {
        return binds;
    }
    let locals = matrices(prim.value("restTransforms"));
    let mut out: Vec<Mat4> = Vec::with_capacity(tree.len());
    for (index, (_, parent)) in tree.iter().enumerate() {
        let local = locals.get(index).copied().unwrap_or(Mat4::IDENTITY);
        let world = match parent.and_then(|parent| out.get(parent).copied()) {
            Some(parent) => parent * local,
            None => local,
        };
        out.push(world);
    }
    out
}

/// The armature a Skeleton prim makes (bones in skeleton space, at rest).
pub(crate) fn read_skeleton(prim: &Prim) -> Result<Armature, SceneError> {
    let joints = tokens(prim.value("joints"));
    if joints.len() > kition_scene_model::armature::MAX_BONES {
        return Err(invalid(format!("skeleton {} has too many joints", prim.name)));
    }
    let tree = joint_tree(&joints);
    let frames = rest_frames(prim, &tree);
    let lengths: Vec<f64> = prim.value(BONE_LENGTHS).map(Value::numbers).unwrap_or_default();
    let mut bones = Vec::with_capacity(tree.len());
    let mut resolved = vec![0f32; tree.len()];
    for (index, (name, parent)) in tree.iter().enumerate() {
        let frame = frames.get(index).copied().unwrap_or(Mat4::IDENTITY);
        if !frame.is_finite() {
            return Err(invalid(format!("skeleton {}'s joint {name} has a transform that is not a number", prim.name)));
        }
        let head = frame.w_axis.truncate();
        let axes = orthonormal(Mat3::from_mat4(frame));
        // The stored length, else the nearest child ahead along +Y, else the parent's.
        let stored = lengths.get(index).map(|length| *length as f32).filter(|length| length.is_finite() && *length > 1e-6);
        let ahead = tree
            .iter()
            .enumerate()
            .filter(|(_, (_, child_parent))| *child_parent == Some(index))
            .filter_map(|(child, _)| {
                let offset = frames.get(child)?.w_axis.truncate() - head;
                let along = offset.dot(axes.y_axis);
                (along > 1e-4 && (offset - axes.y_axis * along).length() < along * 0.2).then_some(along)
            })
            .reduce(f32::min);
        let length =
            stored.or(ahead).or_else(|| parent.and_then(|parent| resolved.get(parent).copied()).filter(|length| *length > 1e-6)).unwrap_or(1.0);
        if let Some(slot) = resolved.get_mut(index) {
            *slot = length;
        }
        let mut bone = Bone::new(name.clone(), head, head + axes.y_axis * length);
        bone.roll = roll_towards(axes.y_axis, axes.z_axis);
        bone.parent = parent.and_then(|parent| tree.get(parent)).map(|(name, _)| name.clone());
        bones.push(bone);
    }
    // A bone whose head sits on its parent's tail is connected, as the reference application
    // makes chains.
    let tails: HashMap<String, Vec3> = bones.iter().map(|bone| (bone.name.clone(), bone.tail)).collect();
    for bone in &mut bones {
        if let Some(tail) = bone.parent.as_ref().and_then(|parent| tails.get(parent)) {
            bone.connected = tail.distance(bone.head) < 1e-4;
        }
    }
    let armature = Armature { bones };
    armature.validate().map_err(|error| invalid(format!("skeleton {}: {error}", prim.name)))?;
    Ok(armature)
}

/// A joint's transform from a SkelAnimation: translation, rotation (w, x, y, z), and scale.
fn joint_local(translations: &[f64], rotations: &[f64], scales: &[f64], index: usize) -> Mat4 {
    let triple = |values: &[f64], fallback: Vec3| match values.get(index * 3..index * 3 + 3) {
        Some([x, y, z]) => Vec3::new(*x as f32, *y as f32, *z as f32),
        _ => fallback,
    };
    let rotation = match rotations.get(index * 4..index * 4 + 4) {
        Some([w, x, y, z]) => Quat::from_xyzw(*x as f32, *y as f32, *z as f32, *w as f32).normalize(),
        _ => Quat::IDENTITY,
    };
    Mat4::from_scale_rotation_translation(triple(scales, Vec3::ONE), rotation, triple(translations, Vec3::ZERO))
}

/// The pose (location, XYZ Euler rotation, scale) each bone takes for the animation's joint
/// transforms, by bone index.
fn poses(armature: &Armature, local_of: &dyn Fn(usize) -> Option<Mat4>, joint_of_bone: &[Option<usize>]) -> Vec<Option<(Vec3, Vec3, Vec3)>> {
    let rests: Vec<Mat4> = armature.bones.iter().map(Bone::rest_matrix).collect();
    let parent_index = |bone: &Bone| bone.parent.as_deref().and_then(|parent| armature.bones.iter().position(|other| other.name == parent));
    armature
        .bones
        .iter()
        .enumerate()
        .map(|(index, bone)| {
            let joint = joint_of_bone.get(index).copied().flatten()?;
            let animated = local_of(joint)?;
            let rest = *rests.get(index)?;
            let rest_local = match parent_index(bone) {
                Some(parent) => rests.get(parent)?.inverse() * rest,
                None => rest,
            };
            let pose = rest_local.inverse() * animated;
            let (scale, rotation, location) = pose.to_scale_rotation_translation();
            if !(scale.is_finite() && rotation.is_finite() && location.is_finite()) {
                return None;
            }
            Some((location, matrix_euler(Mat3::from_quat(rotation)), scale))
        })
        .collect()
}

fn numbers_at(prim: &Prim, name: &str, time: Option<f64>) -> Vec<f64> {
    let Some(property) = prim.property(name) else { return Vec::new() };
    match time {
        Some(time) => property
            .samples
            .iter()
            .find(|(at, _)| (*at - time).abs() < 1e-9)
            .map(|(_, value)| value.numbers())
            .or_else(|| property.value.as_ref().map(Value::numbers))
            .unwrap_or_default(),
        None => {
            property.value.as_ref().map(Value::numbers).or_else(|| property.samples.first().map(|(_, value)| value.numbers())).unwrap_or_default()
        }
    }
}

/// Sample times of an animation's joint and blend shape attributes.
fn animation_times(prim: &Prim, names: &[&str]) -> Vec<f64> {
    let mut times: Vec<f64> =
        names.iter().filter_map(|name| prim.property(name)).flat_map(|property| property.samples.iter().map(|(time, _)| *time)).collect();
    times.sort_by(f64::total_cmp);
    times.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    times.truncate(100_000);
    times
}

/// Applies a skeleton's animation to its armature object: the pose at rest time (or its first
/// sample) on the bones, and keys for every sampled joint.
pub(crate) fn read_animation(skeleton: &Prim, animation: &Prim, object: usize, seconds_per_code: f64, imported: &mut Imported) {
    let Some(ObjectData::Armature(armature)) = imported.objects.get(object).map(|object| object.data.clone()) else { return };
    let skeleton_joints = tokens(skeleton.value("joints"));
    let animation_joints = tokens(animation.value("joints"));
    let tree = joint_tree(&skeleton_joints);
    // Bone index → index into the animation's joint arrays.
    let joint_of_bone: Vec<Option<usize>> = tree
        .iter()
        .enumerate()
        .map(|(index, _)| skeleton_joints.get(index).and_then(|joint| animation_joints.iter().position(|other| other == joint)))
        .collect();
    let pose_at = |time: Option<f64>| {
        let (translations, rotations, scales) =
            (numbers_at(animation, "translations", time), numbers_at(animation, "rotations", time), numbers_at(animation, "scales", time));
        let local_of = |joint: usize| Some(joint_local(&translations, &rotations, &scales, joint));
        poses(&armature, &local_of, &joint_of_bone)
    };
    let times = animation_times(animation, &["translations", "rotations", "scales"]);
    let first = pose_at(times.first().copied());
    if let Some(ObjectData::Armature(target)) = imported.objects.get_mut(object).map(|object| &mut object.data) {
        for (bone, pose) in target.bones.iter_mut().zip(&first) {
            if let Some((location, rotation, scale)) = pose {
                bone.pose.location = if bone.connected || location.length() < 1e-6 { Vec3::ZERO } else { *location };
                bone.pose.rotation = if rotation.abs_diff_eq(Vec3::ZERO, 1e-6) { Vec3::ZERO } else { *rotation };
                bone.pose.scale = if scale.abs_diff_eq(Vec3::ONE, 1e-6) { Vec3::ONE } else { *scale };
            }
        }
    }
    if times.len() < 2 {
        return;
    }
    let rows: Vec<Vec<Option<(Vec3, Vec3, Vec3)>>> = times.iter().map(|time| pose_at(Some(*time))).collect();
    let seconds: Vec<f32> = times.iter().map(|time| (time * seconds_per_code) as f32).collect();
    for (bone_index, bone) in armature.bones.iter().enumerate() {
        for (property, part) in [(Property::BoneLocation, 0usize), (Property::BoneRotation, 1), (Property::BoneScale, 2)] {
            if property == Property::BoneLocation && bone.connected {
                continue;
            }
            for component in 0..3 {
                let mut keys: Vec<(f32, f32)> = Vec::with_capacity(times.len());
                for (time, row) in seconds.iter().zip(&rows) {
                    let Some(Some(pose)) = row.get(bone_index) else { continue };
                    let vector = match part {
                        0 => pose.0,
                        1 => pose.1,
                        _ => pose.2,
                    };
                    keys.push((*time, vector.to_array().get(component).copied().unwrap_or(0.0)));
                }
                if part == 1 {
                    unwrap_angles(&mut keys);
                }
                let first = keys.first().map_or(0.0, |key| key.1);
                if keys.len() < 2 || keys.iter().all(|key| (key.1 - first).abs() < 1e-6) {
                    continue;
                }
                imported.animation.push(ImportedChannel {
                    object,
                    property,
                    index: u8::try_from(component).unwrap_or(0),
                    path: bone.name.clone(),
                    keys,
                    interpolation: Interpolation::Linear,
                });
            }
        }
    }
}

/// Keeps a sequence of angles continuous (no jumps of a whole turn between samples).
fn unwrap_angles(keys: &mut [(f32, f32)]) {
    for index in 1..keys.len() {
        let Some(previous) = keys.get(index - 1).map(|key| key.1) else { continue };
        if let Some(key) = keys.get_mut(index) {
            while key.1 - previous > std::f32::consts::PI {
                key.1 -= std::f32::consts::TAU;
            }
            while previous - key.1 > std::f32::consts::PI {
                key.1 += std::f32::consts::TAU;
            }
        }
    }
}

/// A bound mesh's vertex groups (one per skeleton joint it uses, named after the bone), read
/// from its joint indices and weights. `skeleton_joints` are the skeleton's joints; the mesh's
/// own `skel:joints`, when present, says which joint each index means.
pub(crate) fn read_groups(prim: &Prim, mesh: &mut Mesh, skeleton_joints: &[String]) -> Result<(), SceneError> {
    let (Some(indices), Some(weights)) = (prim.property("primvars:skel:jointIndices"), prim.property("primvars:skel:jointWeights")) else {
        return Ok(());
    };
    let element = indices.meta("elementSize").and_then(Value::as_f64).filter(|size| *size >= 1.0 && *size <= 64.0).unwrap_or(1.0) as usize;
    let constant = indices.meta("interpolation").and_then(Value::as_str) == Some("constant");
    let indices = indices.value.as_ref().map(Value::numbers).unwrap_or_default();
    let weights = weights.value.as_ref().map(Value::numbers).unwrap_or_default();
    let tree = joint_tree(skeleton_joints);
    let mesh_joints = tokens(prim.value("skel:joints"));
    let bone_of = |joint: usize| -> Option<String> {
        let skeleton_index =
            if mesh_joints.is_empty() { joint } else { skeleton_joints.iter().position(|token| Some(token) == mesh_joints.get(joint))? };
        tree.get(skeleton_index).map(|(name, _)| name.clone())
    };
    let count = mesh.positions.len();
    if !constant && indices.len() != count * element {
        return Err(invalid(format!("mesh {}'s joint indices do not match its points", prim.name)));
    }
    let mut groups: BTreeMap<String, Vec<f32>> = BTreeMap::new();
    for vertex in 0..count {
        let base = if constant { 0 } else { vertex * element };
        for slot in base..base + element {
            let (Some(joint), Some(weight)) = (indices.get(slot), weights.get(slot)) else { continue };
            if !(*weight > 0.0 && weight.is_finite() && *joint >= 0.0) {
                continue;
            }
            let Some(name) = bone_of(*joint as usize) else { continue };
            let entry = groups.entry(name).or_insert_with(|| vec![0.0; count]);
            if let Some(value) = entry.get_mut(vertex) {
                *value += *weight as f32;
            }
        }
    }
    for (name, weights) in groups {
        match mesh.groups.iter_mut().find(|group| group.name == name) {
            Some(group) => group.weights = weights,
            None => mesh.groups.push(VertexGroup { name, weights }),
        }
    }
    Ok(())
}

/// A mesh's blend shapes as shape keys: the targets `skel:blendShapeTargets` names, in the order
/// of `skel:blendShapes`. `find` looks a prim up by path.
pub(crate) fn read_shape_keys<'a>(prim: &Prim, path: &str, mesh: &mut Mesh, find: &dyn Fn(&str) -> Option<&'a Prim>) -> Result<(), SceneError> {
    let names = tokens(prim.value("skel:blendShapes"));
    let targets: Vec<String> = match prim.property("skel:blendShapeTargets").and_then(|property| property.value.as_ref()) {
        Some(Value::List(items)) => items.iter().filter_map(|item| item.as_str().map(str::to_string)).collect(),
        Some(other) => other.as_str().map(|path| vec![path.to_string()]).unwrap_or_default(),
        None => Vec::new(),
    };
    let count = mesh.positions.len();
    for (name, target) in names.iter().zip(&targets) {
        let target = if target.starts_with('/') { target.clone() } else { format!("{path}/{target}") };
        let Some(shape) = find(&target) else { continue };
        let offsets: Vec<Vec3> = shape
            .value("offsets")
            .map(Value::numbers)
            .unwrap_or_default()
            .as_chunks::<3>()
            .0
            .iter()
            .map(|offset| Vec3::new(offset[0] as f32, offset[1] as f32, offset[2] as f32))
            .collect();
        let points: Option<Vec<f64>> = shape.value("pointIndices").map(Value::numbers);
        let mut full = vec![Vec3::ZERO; count];
        match points {
            Some(points) => {
                for (point, offset) in points.iter().zip(&offsets) {
                    if *point >= 0.0
                        && let Some(slot) = full.get_mut(*point as usize)
                    {
                        *slot = *offset;
                    }
                }
            }
            None if offsets.len() == count => full = offsets,
            None => return Err(invalid(format!("blend shape {target} has offsets for other points"))),
        }
        if full.iter().any(|offset| !offset.is_finite()) {
            return Err(invalid(format!("blend shape {target} has an offset that is not a number")));
        }
        mesh.shape_keys.push(ShapeKey { name: name.clone(), offsets: full, value: 0.0 });
    }
    Ok(())
}

/// Shape key values and keys from a SkelAnimation's blend shape weights, for a mesh object
/// whose shape keys share their names.
pub(crate) fn read_weights(animation: &Prim, object: usize, seconds_per_code: f64, imported: &mut Imported) {
    let names = tokens(animation.value("blendShapes"));
    if names.is_empty() {
        return;
    }
    let times = animation_times(animation, &["blendShapeWeights"]);
    let first = numbers_at(animation, "blendShapeWeights", times.first().copied());
    let keys: Vec<String> = match imported.objects.get_mut(object).map(|object| &mut object.data) {
        Some(ObjectData::Mesh(mesh)) => {
            for key in &mut mesh.shape_keys {
                if let Some(value) = names.iter().position(|name| *name == key.name).and_then(|slot| first.get(slot)) {
                    key.value = *value as f32;
                }
            }
            mesh.shape_keys.iter().map(|key| key.name.clone()).collect()
        }
        _ => return,
    };
    if times.len() < 2 {
        return;
    }
    for key in keys {
        let Some(slot) = names.iter().position(|name| *name == key) else { continue };
        let values: Vec<(f32, f32)> = times
            .iter()
            .filter_map(|time| {
                numbers_at(animation, "blendShapeWeights", Some(*time)).get(slot).map(|value| ((time * seconds_per_code) as f32, *value as f32))
            })
            .collect();
        let first = values.first().map_or(0.0, |value| value.1);
        if values.len() < 2 || values.iter().all(|value| (value.1 - first).abs() < 1e-6) {
            continue;
        }
        imported.animation.push(ImportedChannel {
            object,
            property: Property::ShapeKey,
            index: 0,
            path: key,
            keys: values,
            interpolation: Interpolation::Linear,
        });
    }
}
