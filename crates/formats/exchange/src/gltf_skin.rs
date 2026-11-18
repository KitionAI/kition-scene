//! glTF skins and morph targets: armatures, vertex weights, shape keys, and their animation.
//!
//! Bones go out as joint nodes the way the reference application's exporter writes them: a
//! joint's frame is the bone's own frame turned to Y-up as a whole (`C · W · P`, `C` the Z-up
//! to Y-up change, `W` the armature's world matrix, `P` the bone's pose in the armature), so a
//! joint's +Y runs along its bone. Root joints are children of the armature's node. A mesh
//! deformed only by exported armatures goes out at rest with JOINTS_0 and WEIGHTS_0 (the four
//! strongest bones of each vertex, summing to 1) and a skin per mesh whose inverse bind matrices
//! take the mesh's bind space to each joint's rest frame. Shape keys go out as morph targets
//! (position offsets) with their values as the mesh's weights and their names in
//! `extras.targetNames`. Pose and shape key animation is sampled every frame, as object
//! animation is.
//!
//! In, a skin's joints become the bones of one armature: the node the root joints hang from (or
//! a new armature at the origin), each bone's rest frame from the inverse bind matrices, its
//! length to its child joint along its +Y (else its parent's), and its pose from the joints'
//! node transforms. Weights become vertex groups named after the bones, morph targets shape keys.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use kition_scene_model::armature::{Armature, Bone, armature_of, roll_towards};
use kition_scene_model::glam::{Mat3, Mat4, Quat, Vec3};
use kition_scene_model::scene::matrix_euler;
use kition_scene_model::vertex_data::{ShapeKey, VertexGroup};
use kition_scene_model::{Id, Interpolation, Mesh, ModifierKind, Object, Property, Scene, SceneError};
use serde_json::{Map, Value, json};

use super::{Document, FLOAT, UNSIGNED_BYTE, UNSIGNED_SHORT, View, Writer, read_u32, to_gltf_world, usize_of, y_up};
use crate::{ImportedChannel, MAX_ITEMS, invalid, to_y_up};

/// The most samples one exported pose animation may write per joint.
const MAX_SAMPLES: i64 = 100_000;

/// Four joints and their weights per vertex.
type Influences = (Vec<[u16; 4]>, Vec<[f32; 4]>);
/// glTF animation channels and their samplers.
pub(super) type Tracks = (Vec<Value>, Vec<Value>);
/// A joint's sampled translations, rotations, scales, and its last rotation.
type JointTrack = (Vec<f32>, Vec<f32>, Vec<f32>, Option<Quat>);

/// What a mesh adds to its primitives: joints and weights per vertex, and shape keys.
#[derive(Debug, Clone, Default)]
pub(super) struct Attributes {
    /// Per mesh vertex: four joints (indices into the armature's bones) and their weights.
    joints: Option<Influences>,
    /// Shape keys: name and an offset per mesh vertex, in Y-up.
    morphs: Vec<(String, Vec<Vec3>)>,
    /// The shape keys' values.
    weights: Vec<f32>,
}

/// How a mesh object goes out at rest.
#[derive(Debug, Clone)]
pub(super) struct Plan {
    /// The mesh at rest (its own shape, shape keys not mixed in).
    pub mesh: Mesh,
    pub attributes: Attributes,
    /// The armature that deforms it, when one does.
    rig: Option<Id>,
}

/// The plan for a mesh object deformed only by armatures that are exported (or carrying shape
/// keys and no other modifier); `None` when it should go out baked.
pub(super) fn plan(scene: &Scene, object: &Object, exported: &HashMap<Id, usize>) -> Option<Plan> {
    let mesh = object.mesh()?;
    let enabled: Vec<&ModifierKind> = object.modifiers.iter().filter(|modifier| modifier.enabled).map(|modifier| &modifier.kind).collect();
    let mut rig = None;
    for kind in &enabled {
        match kind {
            ModifierKind::Armature { object: Some(id), .. } if rig.is_none() && exported.contains_key(id) && armature_of(scene, *id).is_some() => {
                rig = Some(*id)
            }
            _ => return None,
        }
    }
    if rig.is_none() && mesh.shape_keys.is_empty() {
        return None;
    }
    let mut attributes = Attributes::default();
    if let Some(armature) = rig.and_then(|id| armature_of(scene, id)) {
        let bone_index: HashMap<&str, u16> = armature
            .bones
            .iter()
            .enumerate()
            .filter(|(_, bone)| bone.deform)
            .filter_map(|(index, bone)| Some((bone.name.as_str(), u16::try_from(index).ok()?)))
            .collect();
        let groups: Vec<(u16, &VertexGroup)> = mesh
            .groups
            .iter()
            .filter(|group| group.weights.len() == mesh.positions.len())
            .filter_map(|group| Some((*bone_index.get(group.name.as_str())?, group)))
            .collect();
        let mut joints = Vec::with_capacity(mesh.positions.len());
        let mut weights = Vec::with_capacity(mesh.positions.len());
        for vertex in 0..mesh.positions.len() {
            let mut influences: Vec<(u16, f32)> = groups
                .iter()
                .filter_map(|(joint, group)| group.weights.get(vertex).filter(|weight| **weight > 0.0).map(|weight| (*joint, *weight)))
                .collect();
            influences.sort_by(|a, b| b.1.total_cmp(&a.1));
            influences.truncate(4);
            let total: f32 = influences.iter().map(|(_, weight)| weight).sum();
            let (mut joint, mut weight) = ([0u16; 4], [0f32; 4]);
            for (slot, (bone, value)) in influences.iter().enumerate() {
                if let (Some(j), Some(w)) = (joint.get_mut(slot), weight.get_mut(slot)) {
                    *j = *bone;
                    *w = if total > 0.0 { value / total } else { 0.0 };
                }
            }
            joints.push(joint);
            weights.push(weight);
        }
        attributes.joints = Some((joints, weights));
    }
    for key in &mesh.shape_keys {
        attributes.morphs.push((key.name.clone(), key.offsets.iter().map(|offset| to_y_up(*offset)).collect()));
        attributes.weights.push(key.value);
    }
    Some(Plan { mesh: Mesh { groups: Vec::new(), shape_keys: Vec::new(), ..mesh.clone() }, attributes, rig })
}

impl Writer {
    fn joints(&mut self, values: &[[u16; 4]]) -> usize {
        let mut bytes = Vec::with_capacity(values.len() * 8);
        for value in values {
            for component in value {
                bytes.extend(component.to_le_bytes());
            }
        }
        let view = self.view(&bytes, Some(34962));
        self.accessors.push(json!({ "bufferView": view, "componentType": UNSIGNED_SHORT, "count": values.len(), "type": "VEC4" }));
        self.accessors.len() - 1
    }

    fn vec4s(&mut self, values: &[[f32; 4]]) -> usize {
        let mut bytes = Vec::with_capacity(values.len() * 16);
        for value in values {
            for component in value {
                bytes.extend(component.to_le_bytes());
            }
        }
        let view = self.view(&bytes, Some(34962));
        self.accessors.push(json!({ "bufferView": view, "componentType": FLOAT, "count": values.len(), "type": "VEC4" }));
        self.accessors.len() - 1
    }

    fn matrices(&mut self, values: &[Mat4]) -> usize {
        let mut bytes = Vec::with_capacity(values.len() * 64);
        for value in values {
            for component in value.to_cols_array() {
                bytes.extend(component.to_le_bytes());
            }
        }
        let view = self.view(&bytes, None);
        self.accessors.push(json!({ "bufferView": view, "componentType": FLOAT, "count": values.len(), "type": "MAT4" }));
        self.accessors.len() - 1
    }
}

/// Adds the joints, weights, and morph targets of the vertices the primitive wrote (`sources`
/// names the mesh vertex of each).
pub(super) fn attach(writer: &mut Writer, primitive: &mut Value, sources: &[u32], extra: &Attributes) {
    let pick = |index: &u32| *index as usize;
    if let Some((joints, weights)) = &extra.joints {
        let joints: Vec<[u16; 4]> = sources.iter().map(|index| joints.get(pick(index)).copied().unwrap_or_default()).collect();
        let weights: Vec<[f32; 4]> = sources.iter().map(|index| weights.get(pick(index)).copied().unwrap_or_default()).collect();
        let (joint, weight) = (writer.joints(&joints), writer.vec4s(&weights));
        if let Some(attributes) = primitive.get_mut("attributes").and_then(Value::as_object_mut) {
            attributes.insert("JOINTS_0".into(), json!(joint));
            attributes.insert("WEIGHTS_0".into(), json!(weight));
        }
    }
    if !extra.morphs.is_empty() {
        let targets: Vec<Value> = extra
            .morphs
            .iter()
            .map(|(_, offsets)| {
                let values: Vec<Vec3> = sources.iter().map(|index| offsets.get(pick(index)).copied().unwrap_or(Vec3::ZERO)).collect();
                json!({ "POSITION": writer.vec3s(&values, true) })
            })
            .collect();
        if let Some(map) = primitive.as_object_mut() {
            map.insert("targets".into(), Value::Array(targets));
        }
    }
}

/// The mesh's shape key values and names.
pub(super) fn mesh_extras(mesh: &mut Value, extra: &Attributes) {
    if extra.morphs.is_empty() {
        return;
    }
    let Some(map) = mesh.as_object_mut() else { return };
    map.insert("weights".into(), json!(extra.weights));
    let names: Vec<&str> = extra.morphs.iter().map(|(name, _)| name.as_str()).collect();
    if let Some(extras) = map.get_mut("extras").and_then(Value::as_object_mut) {
        extras.insert("targetNames".into(), json!(names));
    }
}

/// One armature's joint nodes.
#[derive(Debug, Clone)]
pub(super) struct Joints {
    armature: Id,
    /// Node index of each bone, in the armature's bone order.
    nodes: Vec<usize>,
}

/// What the joints and skins of an export are.
#[derive(Debug, Clone, Default)]
pub(super) struct Rigs {
    pub skins: Vec<Value>,
    joints: Vec<Joints>,
    /// Morphed meshes: node index and object.
    morphed: Vec<(usize, Id)>,
}

/// A joint's world matrix in glTF space: the bone's frame turned to Y-up as a whole.
fn joint_world(armature_world: Mat4, pose: Mat4) -> Mat4 {
    y_up() * armature_world * pose
}

/// A joint's local matrix: under its parent joint, or (a root) under the armature's node.
fn joint_local(armature: &Armature, poses: &BTreeMap<String, Mat4>, bone: &Bone) -> Mat4 {
    let pose = poses.get(&bone.name).copied().unwrap_or(Mat4::IDENTITY);
    match bone
        .parent
        .as_deref()
        .and_then(|parent| poses.get(parent))
        .filter(|_| bone.parent.as_deref().is_some_and(|parent| armature.bone(parent).is_some()))
    {
        Some(parent) => parent.inverse() * pose,
        None => y_up() * pose,
    }
}

/// Writes every exported armature's bones as joint nodes, and a skin for every skinned mesh.
pub(super) fn export_joints(
    scene: &Scene,
    ids: &[Id],
    nodes: &mut Vec<Value>,
    skinned: &[(usize, Id, Plan)],
    writer: &mut Writer,
) -> Result<Rigs, SceneError> {
    let mut rigs = Rigs::default();
    for (armature_node, id) in ids.iter().enumerate() {
        let Some(armature) = armature_of(scene, *id) else { continue };
        if scene.object(*id)?.hidden {
            continue;
        }
        // Constraints are baked in, as the reference application's exporter does.
        let poses = kition_scene_model::constraints::bone_matrices(scene, *id, 0);
        let first = nodes.len();
        let node_of: HashMap<&str, usize> = armature.bones.iter().enumerate().map(|(index, bone)| (bone.name.as_str(), first + index)).collect();
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); armature.bones.len()];
        let mut roots = Vec::new();
        for bone in &armature.bones {
            let mut node = Map::new();
            node.insert("name".into(), json!(bone.name));
            super::node_transform(&mut node, joint_local(armature, &poses, bone));
            nodes.push(Value::Object(node));
            let me = node_of.get(bone.name.as_str()).copied().unwrap_or(first);
            match bone.parent.as_deref().and_then(|parent| node_of.get(parent)) {
                Some(parent) => {
                    if let Some(list) = children.get_mut(parent - first) {
                        list.push(me);
                    }
                }
                None => roots.push(me),
            }
        }
        for (slot, list) in children.into_iter().enumerate() {
            if let (Some(Value::Object(map)), false) = (nodes.get_mut(first + slot), list.is_empty()) {
                map.insert("children".into(), json!(list));
            }
        }
        if let Some(Value::Object(map)) = nodes.get_mut(armature_node) {
            let mut list: Vec<Value> = map.get("children").and_then(Value::as_array).cloned().unwrap_or_default();
            list.extend(roots.iter().map(|root| json!(root)));
            map.insert("children".into(), Value::Array(list));
        }
        rigs.joints.push(Joints { armature: *id, nodes: (first..first + armature.bones.len()).collect() });
    }
    for (node, mesh_id, plan) in skinned {
        if !plan.attributes.morphs.is_empty() {
            rigs.morphed.push((*node, *mesh_id));
        }
        let Some(rig) = plan.rig else { continue };
        let (Some(armature), Some(joints)) = (armature_of(scene, rig), rigs.joints.iter().find(|joints| joints.armature == rig)) else { continue };
        let armature_world = scene.world_matrix(rig)?;
        let mesh_world = to_gltf_world(scene.world_matrix(*mesh_id)?, false);
        let inverse_binds: Vec<Mat4> =
            armature.bones.iter().map(|bone| joint_world(armature_world, bone.rest_matrix()).inverse() * mesh_world).collect();
        let matrices = writer.matrices(&inverse_binds);
        let armature_node = ids.iter().position(|id| *id == rig);
        let mut skin = json!({ "name": scene.object(rig)?.name, "joints": joints.nodes, "inverseBindMatrices": matrices });
        if let (Some(armature_node), Some(map)) = (armature_node, skin.as_object_mut()) {
            map.insert("skeleton".into(), json!(armature_node));
        }
        rigs.skins.push(skin);
        if let Some(Value::Object(map)) = nodes.get_mut(*node) {
            map.insert("skin".into(), json!(rigs.skins.len() - 1));
        }
    }
    Ok(rigs)
}

/// The frame range keys of `channels` (target, named properties) cover.
fn key_range<'a>(channels: impl Iterator<Item = &'a kition_scene_model::Channel>) -> Option<(f32, f32)> {
    channels.filter_map(|channel| Some((channel.keys.first()?.frame, channel.keys.last()?.frame))).reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
}

/// Pose and shape key animation as glTF channels and samplers (sampler indices from 0).
pub(super) fn export_animation(scene: &Scene, rigs: &Rigs, writer: &mut Writer) -> Result<Option<Tracks>, SceneError> {
    let fps = scene.animation.timeline.fps;
    let mut channels = Vec::new();
    let mut samplers = Vec::new();
    let mut stepper = scene.clone();
    let varies = |values: &[f32], width: usize| {
        values.chunks(width).any(|chunk| values.get(..width).is_some_and(|first| chunk.iter().zip(first).any(|(a, b)| (a - b).abs() > 1e-6)))
    };
    let sample = |first: i64, last: i64| -> Result<Vec<i64>, SceneError> {
        if last - first + 1 > MAX_SAMPLES {
            return Err(invalid("an animation is too long to export"));
        }
        Ok((first..=last).collect())
    };
    for joints in &rigs.joints {
        let bone_channels = || {
            scene.animation.channels.iter().filter(|channel| {
                channel.target == joints.armature && matches!(channel.property, Property::BoneLocation | Property::BoneRotation | Property::BoneScale)
            })
        };
        // Constrained and driven bones move with what they follow: sample them over every key
        // in the scene.
        let constrained = armature_of(scene, joints.armature).is_some_and(|armature| armature.bones.iter().any(|bone| !bone.constraints.is_empty()))
            || bone_channels().any(|channel| channel.driver.is_some());
        let range = if constrained { key_range(scene.animation.channels.iter()) } else { key_range(bone_channels()) };
        let Some((start, end)) = range else { continue };
        let animated: BTreeSet<String> = if constrained {
            armature_of(scene, joints.armature).map(|armature| armature.bones.iter().map(|bone| bone.name.clone()).collect()).unwrap_or_default()
        } else {
            bone_channels().map(|channel| channel.path.clone()).collect()
        };
        let frames = sample(start.floor() as i64, end.ceil() as i64)?;
        let times: Vec<f32> = frames.iter().map(|frame| *frame as f32 / fps).collect();
        let Some(bones) = armature_of(scene, joints.armature).map(|armature| armature.bones.clone()) else { continue };
        // Per bone: translations, rotations, scales.
        let mut tracks: Vec<JointTrack> = vec![(Vec::new(), Vec::new(), Vec::new(), None); bones.len()];
        for frame in &frames {
            stepper.go_to_frame(*frame as f32);
            let Some(armature) = armature_of(&stepper, joints.armature) else { continue };
            let poses = kition_scene_model::constraints::bone_matrices(&stepper, joints.armature, 0);
            for (bone, track) in armature.bones.iter().zip(tracks.iter_mut()) {
                let (scale, mut rotation, translation) = joint_local(armature, &poses, bone).to_scale_rotation_translation();
                if track.3.is_some_and(|previous| previous.dot(rotation) < 0.0) {
                    rotation = -rotation;
                }
                track.3 = Some(rotation);
                track.0.extend(translation.to_array());
                track.1.extend(rotation.normalize().to_array());
                track.2.extend(scale.to_array());
            }
        }
        let input = writer.floats(&times, "SCALAR", 1, true);
        for ((bone, track), node) in bones.iter().zip(&tracks).zip(&joints.nodes) {
            if !animated.contains(&bone.name) {
                continue;
            }
            for (path, values, kind, width) in
                [("translation", &track.0, "VEC3", 3), ("rotation", &track.1, "VEC4", 4), ("scale", &track.2, "VEC3", 3)]
            {
                if !varies(values, width) {
                    continue;
                }
                let output = writer.floats(values, kind, width, false);
                samplers.push(json!({ "input": input, "output": output, "interpolation": "LINEAR" }));
                channels.push(json!({ "sampler": samplers.len() - 1, "target": { "node": node, "path": path } }));
            }
        }
    }
    for (node, mesh_id) in &rigs.morphed {
        let key_channels = || scene.animation.channels.iter().filter(|channel| channel.target == *mesh_id && channel.property == Property::ShapeKey);
        let Some((start, end)) = key_range(key_channels()) else { continue };
        let frames = sample(start.floor() as i64, end.ceil() as i64)?;
        let mut values = Vec::new();
        for frame in &frames {
            stepper.go_to_frame(*frame as f32);
            if let Some(mesh) = stepper.object(*mesh_id).ok().and_then(|object| object.mesh()) {
                values.extend(mesh.shape_keys.iter().map(|key| key.value));
            }
        }
        let times: Vec<f32> = frames.iter().map(|frame| *frame as f32 / fps).collect();
        let input = writer.floats(&times, "SCALAR", 1, true);
        let output = writer.floats(&values, "SCALAR", 1, false);
        samplers.push(json!({ "input": input, "output": output, "interpolation": "LINEAR" }));
        channels.push(json!({ "sampler": samplers.len() - 1, "target": { "node": node, "path": "weights" } }));
    }
    Ok((!channels.is_empty()).then_some((channels, samplers)))
}

/// One animation of object, pose, and shape key channels together (as the reference's exporter
/// writes one per scene).
pub(super) fn merge_animations(objects: Option<Value>, poses: Option<Tracks>) -> Option<Value> {
    let Some((channels, samplers)) = poses else { return objects };
    let mut animations = objects.unwrap_or_else(|| json!([{ "name": "Animation", "channels": [], "samplers": [] }]));
    let animation = animations.get_mut(0)?.as_object_mut()?;
    let offset = animation.get("samplers").and_then(Value::as_array).map_or(0, Vec::len);
    if let Some(list) = animation.get_mut("samplers").and_then(Value::as_array_mut) {
        list.extend(samplers);
    }
    if let Some(list) = animation.get_mut("channels").and_then(Value::as_array_mut) {
        for mut channel in channels {
            if let Some(sampler) = channel.get("sampler").and_then(Value::as_u64)
                && let Some(map) = channel.as_object_mut()
            {
                map.insert("sampler".into(), json!(sampler + offset as u64));
            }
            list.push(channel);
        }
    }
    Some(animations)
}

// ---------------------------------------------------------------------------------------------
// Import

/// What a primitive's vertices carry beside positions: four joints and weights each, and the
/// morph targets' position offsets (Y-up).
#[derive(Debug, Clone, Default)]
pub(super) struct Extras {
    pub joints: Vec<Option<([u32; 4], [f32; 4])>>,
    pub targets: Vec<Vec<Vec3>>,
}

fn element(view: &View<'_>, element: usize, slot: usize) -> Option<f32> {
    let at = element * view.stride;
    match view.component {
        FLOAT => read_u32(view.data, at + slot * 4).map(f32::from_bits).filter(|value| value.is_finite()),
        UNSIGNED_BYTE => view.data.get(at + slot).map(|byte| f32::from(*byte)),
        UNSIGNED_SHORT => {
            view.data.get(at + slot * 2..at + slot * 2 + 2).and_then(|bytes| Some(f32::from(u16::from_le_bytes([*bytes.first()?, *bytes.get(1)?]))))
        }
        _ => None,
    }
}

/// Reads JOINTS_0, WEIGHTS_0, and the targets' POSITION of a primitive with `count` vertices.
pub(super) fn read_extras(document: &Document<'_>, primitive: &Value, count: usize) -> Result<Extras, SceneError> {
    let mut extras = Extras::default();
    let attributes = primitive.get("attributes");
    if let (Some(joints), Some(weights)) =
        (attributes.and_then(|value| usize_of(value, "JOINTS_0")), attributes.and_then(|value| usize_of(value, "WEIGHTS_0")))
    {
        let (joints, weights) = (document.accessor(joints)?, document.accessor(weights)?);
        if joints.count != count || weights.count != count || joints.components != 4 || weights.components != 4 || joints.component == FLOAT {
            return Err(invalid("a primitive's joints or weights do not match its vertices"));
        }
        let scale = match weights.component {
            UNSIGNED_BYTE => 255.0,
            UNSIGNED_SHORT => 65535.0,
            _ => 1.0,
        };
        for vertex in 0..count {
            let mut joint = [0u32; 4];
            let mut weight = [0f32; 4];
            for slot in 0..4 {
                let (Some(j), Some(w)) = (element(&joints, vertex, slot), element(&weights, vertex, slot)) else {
                    return Err(invalid("a primitive's joints or weights cannot be read"));
                };
                if let (Some(js), Some(ws)) = (joint.get_mut(slot), weight.get_mut(slot)) {
                    *js = j as u32;
                    *ws = (w / scale).clamp(0.0, 1.0);
                }
            }
            extras.joints.push(Some((joint, weight)));
        }
    }
    for target in primitive.get("targets").and_then(Value::as_array).map_or(&[][..], Vec::as_slice) {
        match usize_of(target, "POSITION") {
            Some(position) => {
                let offsets = document.vec3s(position)?;
                if offsets.len() != count {
                    return Err(invalid("a morph target does not match its primitive's vertices"));
                }
                extras.targets.push(offsets);
            }
            None => extras.targets.push(vec![Vec3::ZERO; count]),
        }
    }
    Ok(extras)
}

/// The groups (named `joint <k>` until the skin is known) and shape keys of a mesh welded from
/// raw vertices: each welded vertex takes the first raw vertex's values (`remap` raw → welded).
pub(super) fn welded_extras(raw: &Extras, remap: &[u32], welded: usize, names: &[String], weights: &[f32]) -> (Vec<VertexGroup>, Vec<ShapeKey>) {
    let mut taken = vec![false; welded];
    let mut groups: BTreeMap<u32, Vec<f32>> = BTreeMap::new();
    let mut keys: Vec<Vec<Vec3>> = vec![vec![Vec3::ZERO; welded]; raw.targets.len()];
    for (raw_index, target) in remap.iter().enumerate() {
        let target = *target as usize;
        if taken.get(target).copied().unwrap_or(true) {
            continue;
        }
        if let Some(flag) = taken.get_mut(target) {
            *flag = true;
        }
        if let Some(Some((joints, weights))) = raw.joints.get(raw_index) {
            for (joint, weight) in joints.iter().zip(weights) {
                if *weight > 0.0 {
                    let list = groups.entry(*joint).or_insert_with(|| vec![0.0; welded]);
                    if let Some(slot) = list.get_mut(target) {
                        *slot += *weight;
                    }
                }
            }
        }
        for (key, offsets) in keys.iter_mut().zip(&raw.targets) {
            if let (Some(slot), Some(offset)) = (key.get_mut(target), offsets.get(raw_index)) {
                *slot = crate::from_y_up(*offset);
            }
        }
    }
    let groups = groups
        .into_iter()
        .map(|(joint, weights)| VertexGroup {
            name: format!("joint {joint}"),
            weights: weights.into_iter().map(|weight| weight.clamp(0.0, 1.0)).collect(),
        })
        .collect();
    let mut used = BTreeSet::new();
    let shape_keys = keys
        .into_iter()
        .enumerate()
        .map(|(index, offsets)| {
            let base = names.get(index).cloned().filter(|name| !name.is_empty()).unwrap_or_else(|| format!("Key {}", index + 1));
            let name = kition_scene_model::scene::unique_name(&base, |candidate| used.contains(candidate));
            used.insert(name.clone());
            ShapeKey { name, offsets, value: weights.get(index).copied().filter(|value| value.is_finite()).unwrap_or(0.0) }
        })
        .collect();
    (groups, shape_keys)
}

/// The skins of a file: for each, its joints, inverse bind matrices, and the node its root
/// joints hang from.
#[derive(Debug, Clone)]
pub(super) struct Skin {
    pub joints: Vec<usize>,
    pub inverse_binds: Option<Vec<Mat4>>,
}

pub(super) fn read_skins(document: &Document<'_>) -> Result<Vec<Skin>, SceneError> {
    let mut out = Vec::new();
    for skin in document.list("skins") {
        let joints: Vec<usize> = skin
            .get("joints")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("a skin has no joints"))?
            .iter()
            .map(|value| value.as_u64().and_then(|value| usize::try_from(value).ok()).ok_or_else(|| invalid("a skin's joint is not a node index")))
            .collect::<Result<_, _>>()?;
        if joints.len() > kition_scene_model::armature::MAX_BONES || joints.iter().any(|joint| *joint >= document.list("nodes").len()) {
            return Err(invalid("a skin names joints that are not there, or too many"));
        }
        let inverse_binds = match usize_of(skin, "inverseBindMatrices") {
            Some(accessor) => {
                let view = document.accessor(accessor)?;
                if view.component != FLOAT || view.components != 16 || view.count != joints.len() {
                    return Err(invalid("a skin's inverse bind matrices do not match its joints"));
                }
                let mut matrices = Vec::with_capacity(view.count);
                for index in 0..view.count {
                    let mut values = [0f32; 16];
                    for (slot, value) in values.iter_mut().enumerate() {
                        *value = element(&view, index, slot).ok_or_else(|| invalid("an inverse bind matrix is not finite"))?;
                    }
                    matrices.push(Mat4::from_cols_array(&values));
                }
                Some(matrices)
            }
            None => None,
        };
        out.push(Skin { joints, inverse_binds });
    }
    Ok(out)
}

/// Every node's world matrix (glTF space) and parent.
pub(super) fn node_worlds(document: &Document<'_>) -> Result<(Vec<Mat4>, Vec<Option<usize>>), SceneError> {
    let nodes = document.list("nodes");
    let mut parent = vec![None; nodes.len()];
    for (index, node) in nodes.iter().enumerate() {
        for child in node.get("children").and_then(Value::as_array).map_or(&[][..], Vec::as_slice) {
            if let Some(slot) = child.as_u64().and_then(|value| usize::try_from(value).ok()).and_then(|child| parent.get_mut(child)) {
                *slot = Some(index);
            }
        }
    }
    let mut worlds = vec![None; nodes.len()];
    for start in 0..nodes.len() {
        let mut chain = vec![start];
        while let Some(&last) = chain.last() {
            match parent.get(last).copied().flatten() {
                Some(up) if worlds.get(up).copied().flatten().is_none() => {
                    if chain.contains(&up) || chain.len() > nodes.len() {
                        return Err(invalid("the node hierarchy has a cycle"));
                    }
                    chain.push(up);
                }
                _ => break,
            }
        }
        for node in chain.into_iter().rev() {
            if worlds.get(node).copied().flatten().is_some() {
                continue;
            }
            let above = parent.get(node).copied().flatten().and_then(|up| worlds.get(up).copied().flatten()).unwrap_or(Mat4::IDENTITY);
            let world = above * super::node_matrix(nodes.get(node).ok_or_else(|| invalid("a node is not there"))?)?;
            if let Some(slot) = worlds.get_mut(node) {
                *slot = Some(world);
            }
        }
    }
    Ok((worlds.into_iter().map(|world| world.unwrap_or(Mat4::IDENTITY)).collect(), parent))
}

fn orthonormal(matrix: Mat3) -> Mat3 {
    let y = matrix.y_axis.normalize_or(Vec3::Y);
    let z = (matrix.z_axis - y * y.dot(matrix.z_axis)).normalize_or(y.any_orthonormal_vector());
    Mat3::from_cols(y.cross(z), y, z)
}

/// The armature a skin makes, in the space of armature world matrix `armature_world` (scene,
/// Z-up): bones in the skin's joint order, rest frames from the bind matrices (or the joints as
/// they stand), poses from the joints' node transforms. `mesh_world` is the glTF world matrix of
/// a mesh the skin deforms (for the bind matrices).
pub(super) fn build_armature(
    document: &Document<'_>,
    skin: &Skin,
    worlds: &[Mat4],
    parents: &[Option<usize>],
    armature_world: Mat4,
    mesh_world: Option<Mat4>,
) -> Armature {
    let nodes = document.list("nodes");
    let joint_set: BTreeSet<usize> = skin.joints.iter().copied().collect();
    let to_armature = armature_world.inverse() * y_up().inverse();
    // Names, unique.
    let mut names: Vec<String> = Vec::with_capacity(skin.joints.len());
    for (index, joint) in skin.joints.iter().enumerate() {
        let base =
            nodes.get(*joint).and_then(|node| node.get("name")).and_then(Value::as_str).map_or_else(|| format!("Bone {index}"), str::to_string);
        let name = kition_scene_model::scene::unique_name(&base, |candidate| names.iter().any(|other| other == candidate));
        names.push(name);
    }
    let rest: Vec<Mat4> = skin
        .joints
        .iter()
        .enumerate()
        .map(|(index, joint)| {
            let bind = match (&skin.inverse_binds, mesh_world) {
                (Some(binds), Some(mesh)) => binds.get(index).map(|inverse| mesh * inverse.inverse()),
                _ => None,
            };
            to_armature * bind.unwrap_or_else(|| worlds.get(*joint).copied().unwrap_or(Mat4::IDENTITY))
        })
        .collect();
    let parent_of = |index: usize| -> Option<usize> {
        let mut current = skin.joints.get(index).and_then(|joint| parents.get(*joint).copied().flatten());
        while let Some(node) = current {
            if joint_set.contains(&node) {
                return skin.joints.iter().position(|joint| *joint == node);
            }
            current = parents.get(node).copied().flatten();
        }
        None
    };
    let mut bones = Vec::with_capacity(skin.joints.len());
    let mut lengths = vec![0f32; skin.joints.len()];
    for index in 0..skin.joints.len() {
        let frame = rest.get(index).copied().unwrap_or(Mat4::IDENTITY);
        let head = frame.w_axis.truncate();
        let axis = orthonormal(Mat3::from_mat4(frame));
        // The nearest child head ahead along +Y gives the length.
        let ahead: Option<f32> = (0..skin.joints.len())
            .filter(|child| parent_of(*child) == Some(index))
            .filter_map(|child| {
                let offset = rest.get(child)?.w_axis.truncate() - head;
                let along = offset.dot(axis.y_axis);
                (along > 1e-4 && (offset - axis.y_axis * along).length() < along * 0.2).then_some(along)
            })
            .reduce(f32::min);
        let length =
            ahead.or_else(|| parent_of(index).and_then(|parent| lengths.get(parent).copied()).filter(|length| *length > 1e-4)).unwrap_or(1.0);
        if let Some(slot) = lengths.get_mut(index) {
            *slot = length;
        }
        let tail = head + axis.y_axis * length;
        let mut bone = Bone::new(names.get(index).cloned().unwrap_or_default(), head, tail);
        bone.roll = roll_towards(axis.y_axis, axis.z_axis);
        bone.parent = parent_of(index).and_then(|parent| names.get(parent).cloned());
        bones.push(bone);
    }
    let mut armature = Armature { bones };
    pose_from(&mut armature, &skin.joints, worlds, to_armature);
    armature
}

/// Sets each bone's pose so its posed frame is its joint's world frame now.
fn pose_from(armature: &mut Armature, joints: &[usize], worlds: &[Mat4], to_armature: Mat4) {
    let posed: Vec<Mat4> = joints.iter().map(|joint| to_armature * worlds.get(*joint).copied().unwrap_or(Mat4::IDENTITY)).collect();
    let rests: Vec<Mat4> = armature.bones.iter().map(Bone::rest_matrix).collect();
    let parents: Vec<Option<usize>> = armature
        .bones
        .iter()
        .map(|bone| bone.parent.as_deref().and_then(|parent| armature.bones.iter().position(|other| other.name == parent)))
        .collect();
    for (index, bone) in armature.bones.iter_mut().enumerate() {
        let (Some(rest), Some(now)) = (rests.get(index), posed.get(index)) else { continue };
        let local = match parents.get(index).copied().flatten() {
            Some(parent) => {
                let (Some(parent_rest), Some(parent_now)) = (rests.get(parent), posed.get(parent)) else { continue };
                rest.inverse() * *parent_rest * parent_now.inverse() * *now
            }
            None => rest.inverse() * *now,
        };
        let (scale, rotation, location) = local.to_scale_rotation_translation();
        if !(scale.is_finite() && rotation.is_finite() && location.is_finite()) {
            continue;
        }
        bone.pose.location = if location.length() < 1e-6 { Vec3::ZERO } else { location };
        bone.pose.rotation = matrix_euler(Mat3::from_quat(rotation));
        bone.pose.scale = if scale.abs_diff_eq(Vec3::ONE, 1e-6) { Vec3::ONE } else { scale };
        if bone.pose.rotation.abs_diff_eq(Vec3::ZERO, 1e-6) {
            bone.pose.rotation = Vec3::ZERO;
        }
    }
}

/// The pose and shape key keys of the file's animations: joints of skin `armatures[k].0` belong
/// to the imported armature object `armatures[k].1` (with world matrix `armatures[k].2`), and
/// morphed meshes `meshes` map node → (object, key names).
pub(super) fn import_animation(
    document: &Document<'_>,
    armatures: &[(Skin, usize, Mat4, Armature)],
    meshes: &[(usize, usize, Vec<String>)],
) -> Result<Vec<ImportedChannel>, SceneError> {
    let animations = document.list("animations");
    let mut out = Vec::new();
    if animations.is_empty() {
        return Ok(out);
    }
    let nodes = document.list("nodes");
    // Node → path → sampler.
    let mut animated: BTreeMap<usize, BTreeMap<String, super::anim::Sampler>> = BTreeMap::new();
    let mut total = 0usize;
    for animation in animations {
        let samplers = animation.get("samplers").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
        for channel in animation.get("channels").and_then(Value::as_array).map_or(&[][..], Vec::as_slice) {
            let Some(target) = channel.get("target") else { continue };
            let (Some(node), Some(path)) = (usize_of(target, "node"), target.get("path").and_then(Value::as_str)) else { continue };
            let wanted_joint = armatures.iter().any(|(skin, ..)| skin.joints.contains(&node));
            let morph = meshes.iter().find(|(mesh_node, ..)| *mesh_node == node);
            let components = match path {
                "translation" | "scale" if wanted_joint => 3,
                "rotation" if wanted_joint => 4,
                "weights" if morph.is_some() => morph.map_or(1, |(_, _, names)| names.len().max(1)),
                _ => continue,
            };
            let index = usize_of(channel, "sampler").ok_or_else(|| invalid("an animation channel has no sampler"))?;
            let sampler = samplers.get(index).ok_or_else(|| invalid(format!("animation sampler {index} is not there")))?;
            let found = if path == "weights" {
                super::anim::weights_sampler(document, sampler, components)?
            } else {
                super::anim::sampler(document, sampler, components)?
            };
            total = total.saturating_add(found.times.len());
            if total > MAX_ITEMS {
                return Err(invalid("the file's animations are too large"));
            }
            animated.entry(node).or_default().insert(path.to_string(), found);
        }
    }
    for (skin, object, armature_world, armature) in armatures {
        let tracks: Vec<&BTreeMap<String, super::anim::Sampler>> = skin.joints.iter().filter_map(|joint| animated.get(joint)).collect();
        if tracks.is_empty() {
            continue;
        }
        let mut times: Vec<f32> = tracks.iter().flat_map(|paths| paths.values().flat_map(|sampler| sampler.times.iter().copied())).collect();
        times.sort_by(f32::total_cmp);
        times.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        let step = tracks.iter().all(|paths| paths.values().all(|sampler| sampler.step));
        let to_armature = armature_world.inverse() * y_up().inverse();
        // Joint worlds at each time: walk from the root of each joint through the nodes, every
        // node at its sampled (or rest) local transform.
        let (_, parents) = node_worlds(document)?;
        let local_at = |node: usize, time: f32| -> Result<Mat4, SceneError> {
            let rest = super::node_matrix(nodes.get(node).ok_or_else(|| invalid("a node is not there"))?)?;
            let Some(paths) = animated.get(&node) else { return Ok(rest) };
            let (rest_scale, rest_rotation, rest_translation) = rest.to_scale_rotation_translation();
            let vec3 = |path: &str, rest: Vec3| paths.get(path).and_then(|sampler| sampler.at(time)).map_or(rest, |value| Vec3::from_slice(&value));
            let rotation =
                paths.get("rotation").and_then(|sampler| sampler.at(time)).map_or(rest_rotation, |value| Quat::from_slice(&value).normalize());
            Ok(Mat4::from_scale_rotation_translation(vec3("scale", rest_scale), rotation, vec3("translation", rest_translation)))
        };
        let mut keys: Vec<Vec<[f32; 9]>> = vec![Vec::with_capacity(times.len()); armature.bones.len()];
        let mut previous: Vec<Option<Vec3>> = vec![None; armature.bones.len()];
        for time in &times {
            let mut worlds = vec![Mat4::IDENTITY; nodes.len()];
            for joint in &skin.joints {
                let mut chain = vec![*joint];
                while let Some(up) = chain.last().and_then(|last| parents.get(*last).copied().flatten()) {
                    if chain.len() > nodes.len() {
                        return Err(invalid("the node hierarchy has a cycle"));
                    }
                    chain.push(up);
                }
                let mut world = Mat4::IDENTITY;
                for node in chain.into_iter().rev() {
                    world *= local_at(node, *time)?;
                }
                if let Some(slot) = worlds.get_mut(*joint) {
                    *slot = world;
                }
            }
            let mut posed = armature.clone();
            pose_from(&mut posed, &skin.joints, &worlds, to_armature);
            for (index, bone) in posed.bones.iter().enumerate() {
                let mut euler = bone.pose.rotation;
                if let Some(Some(last)) = previous.get(index) {
                    let unwrap = |angle: f32, last: f32| angle + ((last - angle) / std::f32::consts::TAU).round() * std::f32::consts::TAU;
                    euler = Vec3::new(unwrap(euler.x, last.x), unwrap(euler.y, last.y), unwrap(euler.z, last.z));
                }
                if let Some(slot) = previous.get_mut(index) {
                    *slot = Some(euler);
                }
                let (location, scale) = (bone.pose.location, bone.pose.scale);
                if let Some(list) = keys.get_mut(index) {
                    list.push([location.x, location.y, location.z, euler.x, euler.y, euler.z, scale.x, scale.y, scale.z]);
                }
            }
        }
        let interpolation = if step { Interpolation::Constant } else { Interpolation::Linear };
        for (bone, bone_keys) in armature.bones.iter().zip(&keys) {
            for (slot, (property, index)) in [Property::BoneLocation, Property::BoneRotation, Property::BoneScale]
                .iter()
                .flat_map(|property| (0..3u8).map(move |index| (*property, index)))
                .enumerate()
            {
                let values: Vec<(f32, f32)> = times.iter().zip(bone_keys).filter_map(|(time, key)| Some((*time, *key.get(slot)?))).collect();
                let Some(&(_, first)) = values.first() else { continue };
                if values.iter().all(|(_, value)| (value - first).abs() < 1e-6) {
                    continue;
                }
                out.push(ImportedChannel { object: *object, property, index, path: bone.name.clone(), keys: values, interpolation });
            }
        }
    }
    for (node, object, names) in meshes {
        let Some(sampler) = animated.get(node).and_then(|paths| paths.get("weights")) else { continue };
        for (slot, name) in names.iter().enumerate() {
            let values: Vec<(f32, f32)> =
                sampler.times.iter().zip(&sampler.values).filter_map(|(time, value)| Some((*time, *value.get(slot)?))).collect();
            if values.is_empty() {
                continue;
            }
            out.push(ImportedChannel {
                object: *object,
                property: Property::ShapeKey,
                index: 0,
                path: name.clone(),
                keys: values,
                interpolation: if sampler.step { Interpolation::Constant } else { Interpolation::Linear },
            });
        }
    }
    Ok(out)
}

/// Gives the groups of a skinned mesh their bones' names (`joint <k>` → the skin's k-th joint).
pub(super) fn name_groups(mesh: &mut Mesh, bones: &[String]) {
    mesh.groups
        .retain(|group| group.name.strip_prefix("joint ").and_then(|index| index.parse::<usize>().ok()).is_some_and(|index| index < bones.len()));
    for group in &mut mesh.groups {
        if let Some(name) = group.name.strip_prefix("joint ").and_then(|index| index.parse::<usize>().ok()).and_then(|index| bones.get(index)) {
            group.name = name.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_joint_frame_runs_along_its_bone() {
        let bone = Bone::new("B", Vec3::ZERO, Vec3::Z);
        let world = joint_world(Mat4::IDENTITY, bone.rest_matrix());
        // A bone standing up in a Z-up scene is a joint with no rotation in Y-up glTF.
        assert!(world.abs_diff_eq(Mat4::IDENTITY, 1e-6), "{world}");
    }
}
