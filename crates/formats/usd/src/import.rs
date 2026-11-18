//! A USDA layer into a scene's objects.
//!
//! Xformable prims become objects; an Xform whose only child is a mesh, camera, or light
//! becomes one object with that data (the common layout of exported scenes), and a SkelRoot
//! with one Skeleton child becomes the armature object. Skeletons are armatures, their
//! animations bone keys; bound meshes get vertex groups, shape keys, and an Armature modifier
//! (see [`crate::skel`]). Scopes, materials, and shaders make no objects. The stage's up axis and units are
//! converted to the scene's Z-up metres.

use std::collections::{BTreeSet, HashMap};

use kition_scene_exchange::{Imported, ImportedChannel, ImportedObject};
use kition_scene_model::glam::{DMat4, DQuat, DVec3, Mat4, Vec2, Vec3};
use kition_scene_model::{Camera, Interpolation, Light, LightKind, Material, Mesh, ObjectData, Property, SceneError};

use crate::usda::{Layer, Prim, Specifier, Value, invalid};

/// More points, faces, or corners than this in one mesh are refused.
const MAX_ITEMS: usize = 20_000_000;

/// Light conversions shared with the writer: USD's normalized intensity per watt.
pub(crate) const SPHERE_INTENSITY_PER_WATT: f64 = 1.0 / std::f64::consts::PI;
/// A distant light's intensity per W/m² of sun strength.
pub(crate) const DISTANT_INTENSITY_PER_STRENGTH: f64 = 0.25;

/// The name exporters keep an object's original (unsanitized) name under.
pub(crate) const OBJECT_NAME: &str = "userProperties:blender:object_name";

/// What a prim makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    /// A mesh, camera, or light.
    Data,
    /// A transform: an empty (or the object of its one data child).
    Transform,
    /// Nothing itself; its children are read as its parent's.
    PassThrough,
    /// Nothing, nor do its children.
    Skip,
}

fn role(prim: &Prim) -> Role {
    if prim.specifier != Specifier::Def {
        return Role::Skip;
    }
    match prim.type_name.as_str() {
        "Skeleton" if crate::skel::is_carrier(prim) => Role::Skip,
        "Mesh" | "Camera" | "SphereLight" | "DistantLight" | "RectLight" | "DiskLight" | "Skeleton" => Role::Data,
        "Xform" | "SkelRoot" => Role::Transform,
        "Scope" => Role::PassThrough,
        "Material" | "Shader" | "NodeGraph" | "GeomSubset" | "SkelAnimation" | "BlendShape" | "Shader_" => Role::Skip,
        _ => {
            if prim.property("xformOpOrder").is_some() {
                Role::Transform
            } else {
                Role::PassThrough
            }
        }
    }
}

struct Stage<'a> {
    prims: HashMap<String, &'a Prim>,
    /// The file's up axis and units into the scene's.
    to_scene: DMat4,
    time_codes_per_second: f64,
    materials: HashMap<String, Option<usize>>,
    /// Skeleton prim paths and the armature objects they made.
    skeletons: HashMap<String, usize>,
    /// Bound meshes: (mesh object, skeleton path).
    bindings: Vec<(usize, String)>,
    /// The files of the package the layer came in (its first file the layer).
    assets: &'a [(String, &'a [u8])],
}

fn index<'a>(prim: &'a Prim, path: String, prims: &mut HashMap<String, &'a Prim>) {
    for child in &prim.children {
        index(child, format!("{path}/{}", child.name), prims);
    }
    prims.insert(path, prim);
}

/// The numbers of an attribute at `time` (interpolated between samples), else its default.
fn numbers_at(prim: &Prim, name: &str, time: Option<f64>) -> Option<Vec<f64>> {
    let property = prim.property(name)?;
    if let Some(time) = time
        && !property.samples.is_empty()
    {
        let after = property.samples.iter().position(|(at, _)| *at >= time);
        let pick = |index: usize| property.samples.get(index).map(|(at, value)| (*at, value.numbers()));
        return match after {
            Some(0) => pick(0).map(|(_, value)| value),
            None => pick(property.samples.len() - 1).map(|(_, value)| value),
            Some(index) => {
                let (from_time, from) = pick(index - 1)?;
                let (to_time, to) = pick(index)?;
                let t = if to_time > from_time { (time - from_time) / (to_time - from_time) } else { 0.0 };
                Some(from.iter().zip(&to).map(|(a, b)| a + (b - a) * t).collect())
            }
        };
    }
    prim.value(name).map(Value::numbers)
}

fn vec3(values: &[f64], fallback: DVec3) -> DVec3 {
    match values {
        [x, y, z, ..] => DVec3::new(*x, *y, *z),
        _ => fallback,
    }
}

fn axis_rotation(axis: char, degrees: f64) -> DMat4 {
    let radians = degrees.to_radians();
    match axis {
        'X' => DMat4::from_rotation_x(radians),
        'Y' => DMat4::from_rotation_y(radians),
        _ => DMat4::from_rotation_z(radians),
    }
}

/// A prim's local transform at `time`, and whether it resets its parents' (`!resetXformStack!`).
fn local_matrix(prim: &Prim, time: Option<f64>) -> (DMat4, bool) {
    let Some(order) = prim.value("xformOpOrder") else { return (DMat4::IDENTITY, false) };
    let mut matrix = DMat4::IDENTITY;
    let mut reset = false;
    for op in order.items().iter().filter_map(Value::as_str) {
        if op == "!resetXformStack!" {
            reset = true;
            matrix = DMat4::IDENTITY;
            continue;
        }
        let (invert, name) = match op.strip_prefix("!invert!") {
            Some(name) => (true, name),
            None => (false, op),
        };
        let Some(kind) = name.split(':').nth(1) else { continue };
        let Some(values) = numbers_at(prim, name, time) else { continue };
        let step = match kind {
            "translate" => DMat4::from_translation(vec3(&values, DVec3::ZERO)),
            "scale" => DMat4::from_scale(vec3(&values, DVec3::ONE)),
            "rotateX" | "rotateY" | "rotateZ" => {
                let axis = kind.chars().last().unwrap_or('Z');
                axis_rotation(axis, values.first().copied().unwrap_or(0.0))
            }
            "orient" => match values.as_slice() {
                [w, x, y, z, ..] => DMat4::from_quat(DQuat::from_xyzw(*x, *y, *z, *w).normalize()),
                _ => DMat4::IDENTITY,
            },
            "transform" => match <[f64; 16]>::try_from(values.as_slice()) {
                Ok(array) => DMat4::from_cols_array(&array),
                Err(_) => DMat4::IDENTITY,
            },
            rotate if rotate.len() == 9 && rotate.starts_with("rotate") => {
                // rotateXYZ turns about X first: the matrix is Rz · Ry · Rx.
                // The values are the X, Y, and Z angles whatever the order.
                let angles = vec3(&values, DVec3::ZERO);
                let mut step = DMat4::IDENTITY;
                for axis in rotate.chars().skip(6) {
                    let degrees = match axis {
                        'X' => angles.x,
                        'Y' => angles.y,
                        _ => angles.z,
                    };
                    step = axis_rotation(axis, degrees) * step;
                }
                step
            }
            _ => DMat4::IDENTITY,
        };
        let step = if invert { step.inverse() } else { step };
        if step.is_finite() {
            matrix *= step;
        }
    }
    (matrix, reset)
}

/// Time codes at which any of the prims' transform ops are sampled.
fn sample_times(prims: &[&Prim]) -> Vec<f64> {
    let mut times: BTreeSet<u64> = BTreeSet::new();
    for prim in prims {
        for property in prim.properties.iter().filter(|property| property.name.starts_with("xformOp:")) {
            times.extend(property.samples.iter().map(|(time, _)| time.to_bits()));
        }
    }
    let mut times: Vec<f64> = times.into_iter().map(f64::from_bits).collect();
    times.sort_by(f64::total_cmp);
    times.truncate(100_000);
    times
}

fn decompose(matrix: DMat4) -> Option<(Vec3, Vec3, Vec3)> {
    let matrix = matrix.as_mat4();
    let (scale, rotation, translation) = matrix.to_scale_rotation_translation();
    let rebuilt = Mat4::from_scale_rotation_translation(scale, rotation, translation);
    let tolerance = 1e-3 * matrix.to_cols_array().iter().fold(1.0f32, |most, value| most.max(value.abs()));
    if !rebuilt.abs_diff_eq(matrix, tolerance) || !scale.is_finite() || !translation.is_finite() {
        return None;
    }
    Some((translation, kition_scene_model::scene::matrix_euler(kition_scene_model::glam::Mat3::from_quat(rotation)), scale))
}

/// The object's display name: an exporter's saved original, else the prim's.
fn object_name(prim: &Prim) -> String {
    prim.value(OBJECT_NAME).and_then(Value::as_str).filter(|name| !name.is_empty()).unwrap_or(&prim.name).to_string()
}

/// Reads a USDA layer.
/// Reads `layer`, with the files of the package it came in (`assets`; empty for a lone layer).
pub fn import<'a>(layer: &'a Layer, assets: &'a [(String, &'a [u8])]) -> Result<Imported, SceneError> {
    let mut prims = HashMap::new();
    for prim in &layer.prims {
        index(prim, format!("/{}", prim.name), &mut prims);
    }
    let meters = layer.meta("metersPerUnit").and_then(Value::as_f64).filter(|value| value.is_finite() && *value > 0.0).unwrap_or(0.01);
    let up_y = layer.meta("upAxis").and_then(Value::as_str).is_none_or(|axis| axis == "Y");
    let rate = layer
        .meta("timeCodesPerSecond")
        .or_else(|| layer.meta("framesPerSecond"))
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(24.0);
    let mut to_scene = DMat4::from_scale(DVec3::splat(meters));
    if up_y {
        to_scene *= DMat4::from_rotation_x(std::f64::consts::FRAC_PI_2);
    }
    let mut stage =
        Stage { prims, to_scene, time_codes_per_second: rate, materials: HashMap::new(), skeletons: HashMap::new(), bindings: Vec::new(), assets };
    let mut imported = Imported::default();
    let default_prim = layer.meta("defaultPrim").and_then(Value::as_str).map(str::to_string);
    for prim in &layer.prims {
        // A default prim that only groups the scene (no transform of its own) makes no object.
        let grouping = default_prim.as_deref() == Some(prim.name.as_str())
            && role(prim) == Role::Transform
            && merged_child(prim).is_none()
            && local_matrix(prim, None).0.abs_diff_eq(DMat4::IDENTITY, 1e-9)
            && sample_times(&[prim]).is_empty();
        if grouping {
            for child in &prim.children {
                walk(&mut stage, child, &format!("/{}/{}", prim.name, child.name), None, DMat4::IDENTITY, &mut imported, 1)?;
            }
        } else {
            walk(&mut stage, prim, &format!("/{}", prim.name), None, DMat4::IDENTITY, &mut imported, 0)?;
        }
    }
    finish_skeletons(&stage, &mut imported);
    Ok(imported)
}

/// The skeletons' animations, and the bindings of meshes to the armatures their skeletons made.
fn finish_skeletons(stage: &Stage<'_>, imported: &mut Imported) {
    let seconds_per_code = 1.0 / stage.time_codes_per_second;
    let animation_of = |skeleton: &str| -> Option<&Prim> {
        let target = stage.prims.get(skeleton)?.property("skel:animationSource")?.value.as_ref()?;
        let target = match target {
            Value::List(items) => items.first()?.as_str()?.to_string(),
            other => other.as_str()?.to_string(),
        };
        let target = if target.starts_with('/') { target } else { format!("{skeleton}/{target}") };
        stage.prims.get(&target).copied().filter(|prim| prim.type_name == "SkelAnimation")
    };
    for (path, object) in &stage.skeletons {
        if let (Some(skeleton), Some(animation)) = (stage.prims.get(path), animation_of(path)) {
            crate::skel::read_animation(skeleton, animation, *object, seconds_per_code, imported);
        }
    }
    for (mesh, skeleton) in &stage.bindings {
        if let Some(rig) = stage.skeletons.get(skeleton) {
            let has_groups =
                matches!(imported.objects.get(*mesh).map(|object| &object.data), Some(ObjectData::Mesh(data)) if !data.groups.is_empty());
            if has_groups {
                imported.deforms.push((*mesh, *rig));
            }
        }
        if let Some(animation) = animation_of(skeleton) {
            crate::skel::read_weights(animation, *mesh, seconds_per_code, imported);
        }
    }
}

/// The data child an Xform takes as its own: its only child when that is a mesh, camera, or
/// light, or (beside other children) its only data child of the same name, as exporters nest an
/// object's data.
fn merged_child(prim: &Prim) -> Option<&Prim> {
    // A SkelRoot is the armature object of its one skeleton (the exporters' layout).
    if prim.type_name == "SkelRoot" {
        let mut skeletons = prim.children.iter().filter(|child| child.type_name == "Skeleton" && role(child) == Role::Data);
        if let (Some(skeleton), None) = (skeletons.next(), skeletons.next()) {
            return Some(skeleton);
        }
    }
    let mut data = prim.children.iter().filter(|child| role(child) == Role::Data);
    match (data.next(), data.next()) {
        // Skipped children (a blend-shape carrier skeleton) do not count.
        (Some(child), None) if prim.children.iter().filter(|other| role(other) != Role::Skip).count() == 1 || child.name == prim.name => Some(child),
        _ => None,
    }
}

fn walk(
    stage: &mut Stage<'_>,
    prim: &Prim,
    path: &str,
    parent: Option<usize>,
    parent_file_world: DMat4,
    imported: &mut Imported,
    depth: usize,
) -> Result<(), SceneError> {
    if depth > 256 {
        return Err(invalid("prims nest too deep"));
    }
    if imported.objects.len() > 100_000 {
        return Err(invalid("too many objects"));
    }
    let (data_prim, data_path, transforms): (Option<&Prim>, String, Vec<&Prim>) = match role(prim) {
        Role::Skip => return Ok(()),
        Role::PassThrough => {
            for child in &prim.children {
                walk(stage, child, &format!("{path}/{}", child.name), parent, parent_file_world, imported, depth + 1)?;
            }
            return Ok(());
        }
        Role::Data => (Some(prim), path.to_string(), vec![prim]),
        Role::Transform => match merged_child(prim) {
            Some(child) => (Some(child), format!("{path}/{}", child.name), vec![prim, child]),
            None => (None, String::new(), vec![prim]),
        },
    };
    let local_at = |time: Option<f64>| {
        let mut matrix = DMat4::IDENTITY;
        let mut reset = false;
        for transform in &transforms {
            let (step, resets) = local_matrix(transform, time);
            if resets {
                reset = true;
                matrix = DMat4::IDENTITY;
            }
            matrix *= step;
        }
        (matrix, reset)
    };
    let to_scene = stage.to_scene;
    let (local, reset) = local_at(None);
    // The file-space world, and the transform relative to the parent object in scene space.
    let file_world = if reset { local } else { parent_file_world * local };
    let relative = |local: DMat4| match (parent, reset) {
        (None, _) => to_scene * if reset { local } else { parent_file_world * local },
        (Some(_), false) => local,
        (Some(_), true) => parent_file_world.inverse() * local,
    };
    let data = match data_prim {
        Some(data) => read_data(stage, data, &data_path, imported)?,
        None => (ObjectData::Empty { size: 1.0 }, None),
    };
    let index = imported.objects.len();
    if data_prim.is_some_and(|data| data.type_name == "Skeleton") {
        stage.skeletons.insert(data_path.clone(), index);
    }
    let mut object = ImportedObject {
        name: object_name(prim),
        data: data.0,
        location: Vec3::ZERO,
        rotation: Vec3::ZERO,
        scale: Vec3::ONE,
        parent,
        parent_inverse: Mat4::IDENTITY,
        material: data.1,
    };
    let matrix = relative(local);
    let decomposed = decompose(matrix);
    match decomposed {
        Some((location, rotation, scale)) => {
            object.location = location;
            object.rotation = rotation;
            object.scale = scale;
        }
        None => object.parent_inverse = matrix.as_mat4(),
    }
    imported.objects.push(object);
    if decomposed.is_some() {
        read_animation(stage, &transforms, index, &relative, &local_at, imported);
    }
    if role(prim) == Role::Transform {
        for child in prim.children.iter().filter(|child| data_prim.is_none_or(|data| !std::ptr::eq(*child, data))) {
            walk(stage, child, &format!("{path}/{}", child.name), Some(index), file_world, imported, depth + 1)?;
        }
    }
    Ok(())
}

/// Keys for an object's sampled transform: location, rotation, and scale at every sample time.
fn read_animation(
    stage: &Stage<'_>,
    transforms: &[&Prim],
    object: usize,
    relative: &dyn Fn(DMat4) -> DMat4,
    local_at: &dyn Fn(Option<f64>) -> (DMat4, bool),
    imported: &mut Imported,
) {
    let times = sample_times(transforms);
    if times.len() < 2 {
        return;
    }
    let mut rows: Vec<[f32; 9]> = Vec::with_capacity(times.len());
    for time in &times {
        let Some((location, rotation, scale)) = decompose(relative(local_at(Some(*time)).0)) else { return };
        rows.push([location.x, location.y, location.z, rotation.x, rotation.y, rotation.z, scale.x, scale.y, scale.z]);
    }
    // Keep Euler angles continuous between samples.
    for index in 1..rows.len() {
        for component in 3..6 {
            let (Some(previous), Some(current)) =
                (rows.get(index - 1).and_then(|row| row.get(component)).copied(), rows.get(index).and_then(|row| row.get(component)).copied())
            else {
                continue;
            };
            let mut value = current;
            while value - previous > std::f32::consts::PI {
                value -= std::f32::consts::TAU;
            }
            while previous - value > std::f32::consts::PI {
                value += std::f32::consts::TAU;
            }
            if let Some(slot) = rows.get_mut(index).and_then(|row| row.get_mut(component)) {
                *slot = value;
            }
        }
    }
    let seconds: Vec<f32> = times.iter().map(|time| (time / stage.time_codes_per_second) as f32).collect();
    for (slot, property) in [Property::Location, Property::Rotation, Property::Scale].into_iter().enumerate() {
        for component in 0..3 {
            let column = slot * 3 + component;
            let keys: Vec<(f32, f32)> = seconds.iter().zip(&rows).map(|(time, row)| (*time, row.get(column).copied().unwrap_or(0.0))).collect();
            let first = keys.first().map_or(0.0, |key| key.1);
            if keys.iter().all(|key| (key.1 - first).abs() < 1e-6) {
                continue;
            }
            imported.animation.push(ImportedChannel {
                object,
                property,
                index: u8::try_from(component).unwrap_or(0),
                path: String::new(),
                keys,
                interpolation: Interpolation::Linear,
            });
        }
    }
}

/// A data prim's object data and material.
fn read_data(stage: &mut Stage<'_>, prim: &Prim, path: &str, imported: &mut Imported) -> Result<(ObjectData, Option<usize>), SceneError> {
    let meters = stage.to_scene.x_axis.truncate().length();
    Ok(match prim.type_name.as_str() {
        "Mesh" => {
            let mut mesh = read_mesh(prim)?;
            // The object this mesh makes is the next one.
            let object = imported.objects.len();
            let prims = &stage.prims;
            crate::skel::read_shape_keys(prim, path, &mut mesh, &|target| prims.get(target).copied())?;
            if let Some(skeleton) = bound_skeleton(&stage.prims, prim, path) {
                // A skeleton that only carries blend shape weights deforms nothing.
                if let Some(skeleton_prim) = stage.prims.get(&skeleton).filter(|skeleton| !crate::skel::is_carrier(skeleton)) {
                    let joints: Vec<String> = skeleton_prim
                        .value("joints")
                        .map(|value| value.items().iter().filter_map(|item| item.as_str().map(str::to_string)).collect())
                        .unwrap_or_default();
                    crate::skel::read_groups(prim, &mut mesh, &joints)?;
                }
                stage.bindings.push((object, skeleton));
            }
            let material = material_binding(prim, path).and_then(|target| read_material(stage, &target, imported));
            (ObjectData::Mesh(mesh), material)
        }
        "Camera" => (ObjectData::Camera(read_camera(prim, meters)), None),
        "Skeleton" => (ObjectData::Armature(crate::skel::read_skeleton(prim)?), None),
        _ => (ObjectData::Light(read_light(prim, meters)), None),
    })
}

/// The skeleton a mesh is bound to: its own `skel:skeleton`, else the nearest ancestor's.
fn bound_skeleton(prims: &HashMap<String, &Prim>, prim: &Prim, path: &str) -> Option<String> {
    let target_of = |prim: &Prim, at: &str| -> Option<String> {
        let target = prim.property("skel:skeleton")?.value.as_ref()?;
        let target = match target {
            Value::List(items) => items.first()?.as_str()?.to_string(),
            other => other.as_str()?.to_string(),
        };
        Some(if target.starts_with('/') { target } else { format!("{at}/{target}") })
    };
    if let Some(found) = target_of(prim, path) {
        return Some(found);
    }
    let mut at = path.to_string();
    while let Some((parent, _)) = at.rsplit_once('/') {
        if parent.is_empty() {
            break;
        }
        if let Some(found) = prims.get(parent).and_then(|ancestor| target_of(ancestor, parent)) {
            return Some(found);
        }
        at = parent.to_string();
    }
    None
}

fn material_binding(prim: &Prim, path: &str) -> Option<String> {
    let target = prim.property("material:binding")?.value.as_ref()?;
    let target = match target {
        Value::List(items) => items.first()?.as_str()?.to_string(),
        other => other.as_str()?.to_string(),
    };
    // Relative targets name a prim from the bound one.
    Some(if target.starts_with('/') { target } else { format!("{path}/{target}") })
}

fn read_mesh(prim: &Prim) -> Result<Mesh, SceneError> {
    let numbers = |name: &str| prim.value(name).map(Value::numbers).unwrap_or_default();
    let points = numbers("points");
    let counts = numbers("faceVertexCounts");
    let indices = numbers("faceVertexIndices");
    if points.len() / 3 > MAX_ITEMS || counts.len() > MAX_ITEMS || indices.len() > MAX_ITEMS {
        return Err(invalid(format!("mesh {} is too large", prim.name)));
    }
    let positions: Vec<Vec3> = points.as_chunks::<3>().0.iter().map(|point| DVec3::from_array(*point).as_vec3()).collect();
    if positions.iter().any(|point| !point.is_finite()) {
        return Err(invalid(format!("mesh {} has a point that is not a number", prim.name)));
    }
    let vertex_count = positions.len();
    let left_handed = prim.value("orientation").and_then(Value::as_str) == Some("leftHanded");
    // Faces as corner indices into `indices`, with their winding.
    let mut corners_of_faces: Vec<Vec<usize>> = Vec::with_capacity(counts.len());
    let mut corner = 0usize;
    for count in &counts {
        let count = if count.is_finite() && *count >= 0.0 { *count as usize } else { return Err(invalid("a negative face size")) };
        let end = corner
            .checked_add(count)
            .filter(|end| *end <= indices.len())
            .ok_or_else(|| invalid(format!("mesh {}'s faces need more corners than it has", prim.name)))?;
        let mut face: Vec<usize> = (corner..end).collect();
        if left_handed {
            face.reverse();
        }
        corners_of_faces.push(face);
        corner = end;
    }
    let vertex = |corner: usize| -> Result<u32, SceneError> {
        let value = indices.get(corner).copied().unwrap_or(-1.0);
        if !(value >= 0.0 && (value as usize) < vertex_count) {
            return Err(invalid(format!("mesh {} names a point it does not have", prim.name)));
        }
        u32::try_from(value as usize).map_err(|_| invalid("an index is out of range"))
    };
    let mut mesh = Mesh { positions, ..Mesh::default() };
    let mut kept: Vec<&Vec<usize>> = Vec::new();
    for face in &corners_of_faces {
        let vertices: Vec<u32> = face.iter().map(|corner| vertex(*corner)).collect::<Result<_, _>>()?;
        let distinct: BTreeSet<u32> = vertices.iter().copied().collect();
        match vertices.as_slice() {
            [a, b] if a != b => mesh.loose_edges.push([*a, *b]),
            _ if vertices.len() >= 3 && distinct.len() == vertices.len() => {
                mesh.faces.push(vertices);
                kept.push(face);
            }
            _ => {}
        }
    }
    // Texture coordinates: `st`, else the first 2D texture-coordinate primvar.
    let uv_property = prim.property("primvars:st").or_else(|| {
        prim.properties.iter().find(|property| {
            property.name.starts_with("primvars:")
                && !property.name.ends_with(":indices")
                && matches!(property.type_name.as_str(), "texCoord2f[]" | "texCoord2d[]" | "texCoord2h[]")
        })
    });
    if let Some(property) = uv_property
        && let Some(value) = property.value.as_ref().or_else(|| property.samples.first().map(|(_, value)| value))
    {
        let values: Vec<Vec2> = value.numbers().as_chunks::<2>().0.iter().map(|uv| Vec2::new(uv[0] as f32, uv[1] as f32)).collect();
        let uv_indices = prim.value(&format!("{}:indices", property.name)).map(Value::numbers);
        let interpolation = property.meta("interpolation").and_then(Value::as_str).unwrap_or("faceVarying");
        let lookup = |slot: usize| -> Option<Vec2> {
            let slot = match &uv_indices {
                Some(indices) => {
                    let index = *indices.get(slot)?;
                    if index < 0.0 {
                        return None;
                    }
                    index as usize
                }
                None => slot,
            };
            values.get(slot).copied()
        };
        let uvs: Option<Vec<Vec<Vec2>>> = kept
            .iter()
            .map(|face| {
                face.iter()
                    .map(|corner| match interpolation {
                        "vertex" | "varying" => lookup(indices.get(*corner).copied().unwrap_or(-1.0).max(0.0) as usize),
                        _ => lookup(*corner),
                    })
                    .collect()
            })
            .collect();
        if let Some(uvs) = uvs {
            mesh.uvs = uvs;
            if !mesh.uvs_aligned() {
                mesh.uvs.clear();
            }
        }
    }
    // Smooth when the stored normals differ from the faces' own; without normals, when subdivided.
    let normals = prim.property("normals").or_else(|| prim.property("primvars:normals"));
    match normals
        .and_then(|property| property.value.as_ref().or_else(|| property.samples.first().map(|(_, value)| value)).map(|value| (property, value)))
    {
        Some((property, value)) => {
            let normals: Vec<DVec3> = value.numbers().as_chunks::<3>().0.iter().map(|normal| DVec3::from_array(*normal)).collect();
            let per_vertex = normals.len() == vertex_count && normals.len() != indices.len()
                || matches!(property.meta("interpolation").and_then(Value::as_str), Some("vertex" | "varying"));
            let mut differ = 0usize;
            let mut total = 0usize;
            for (face, corners) in mesh.faces.iter().zip(&kept) {
                let flat = mesh.face_normal(face).as_dvec3();
                for (vertex, corner) in face.iter().zip(corners.iter()) {
                    let slot = if per_vertex { *vertex as usize } else { *corner };
                    if let Some(normal) = normals.get(slot) {
                        total += 1;
                        let normal = if left_handed { -*normal } else { *normal };
                        if normal.normalize_or_zero().dot(flat).abs() < 0.999 {
                            differ += 1;
                        }
                    }
                }
            }
            mesh.smooth = total > 0 && differ * 2 > total;
        }
        None => mesh.smooth = prim.value("subdivisionScheme").and_then(Value::as_str).unwrap_or("catmullClark") != "none",
    }
    mesh.validate().map_err(|error| invalid(format!("mesh {}: {error}", prim.name)))?;
    Ok(mesh)
}

/// The material at `path` (read once), as an index into the imported materials.
fn read_material(stage: &mut Stage<'_>, path: &str, imported: &mut Imported) -> Option<usize> {
    if let Some(known) = stage.materials.get(path) {
        return *known;
    }
    let made = (|| {
        let prim = *stage.prims.get(path)?;
        if prim.type_name != "Material" {
            return None;
        }
        // The surface shader: the one `outputs:surface` connects to, else a preview-surface child.
        let connected = prim
            .property("outputs:surface")
            .and_then(|property| property.connect.as_ref())
            .and_then(|target| target.as_str().or_else(|| target.items().first().and_then(Value::as_str)))
            .and_then(|target| target.split('.').next())
            .and_then(|shader| stage.prims.get(shader).copied());
        let shader = connected.or_else(|| {
            prim.children
                .iter()
                .find(|child| child.type_name == "Shader" && child.value("info:id").and_then(Value::as_str) == Some("UsdPreviewSurface"))
        })?;
        let input = |name: &str| shader.property(&format!("inputs:{name}")).and_then(|property| property.value.as_ref()).map(Value::numbers);
        let mut material = Material::new(0, object_name(prim));
        let color = input("diffuseColor").map_or(DVec3::splat(0.18), |values| vec3(&values, DVec3::splat(0.18)));
        let opacity = input("opacity").and_then(|values| values.first().copied()).unwrap_or(1.0).clamp(0.0, 1.0);
        let clamp = |value: f64| (value as f32).clamp(0.0, 1.0);
        material.base_color = [clamp(color.x), clamp(color.y), clamp(color.z), opacity as f32];
        if opacity < 0.999 {
            material.alpha_mode = kition_scene_model::AlphaMode::Blend;
        }
        if let Some(metallic) = input("metallic").and_then(|values| values.first().copied()) {
            material.metallic = clamp(metallic);
        }
        if let Some(roughness) = input("roughness").and_then(|values| values.first().copied()) {
            material.roughness = clamp(roughness);
        }
        if let Some(ior) = input("ior").and_then(|values| values.first().copied()).filter(|ior| ior.is_finite() && *ior >= 1.0) {
            material.ior = ior as f32;
        }
        // Images in the package, through UsdUVTexture shaders; their scale is the factor.
        if let Some((hash, scale, _)) = texture(stage, shader, "diffuseColor", imported) {
            material.base_color_texture = Some(hash);
            let [red, green, blue, _] = scale;
            material.base_color = [clamp(red), clamp(green), clamp(blue), opacity as f32];
        }
        if let (Some((metal, scale, "b")), Some((rough, _, "g"))) =
            (texture(stage, shader, "metallic", imported), texture(stage, shader, "roughness", imported))
            && metal == rough
        {
            material.metallic_roughness_texture = Some(metal);
            material.roughness = clamp(scale[1]);
            material.metallic = clamp(scale[2]);
        }
        if let Some((hash, scale, _)) = texture(stage, shader, "normal", imported) {
            material.normal_texture = Some(hash);
            material.normal_strength = if scale[2].abs() > 1e-6 { ((scale[0] / scale[2]) as f32).clamp(0.0, 10.0) } else { 1.0 };
        }
        let emission = input("emissiveColor").map_or(DVec3::ZERO, |values| vec3(&values, DVec3::ZERO));
        if emission.is_finite() && emission.max_element() > 0.0 {
            material.emission = emission.as_vec3();
            material.emission_strength = 1.0;
        }
        imported.materials.push(material);
        Some(imported.materials.len() - 1)
    })();
    stage.materials.insert(path.to_string(), made);
    made
}

/// The image a shader input is connected to through a UsdUVTexture shader whose file is in the
/// package: its hash (the image added to `imported` once), the texture's scale, and the output
/// read (`rgb`, `r`, `g`, `b`, or `a`).
fn texture(stage: &Stage<'_>, shader: &Prim, input: &str, imported: &mut Imported) -> Option<(String, [f64; 4], &'static str)> {
    let target = shader.property(&format!("inputs:{input}"))?.connect.as_ref()?;
    let target = target.as_str().or_else(|| target.items().first().and_then(Value::as_str))?;
    let (path, output) = target.split_once(".outputs:")?;
    let output = ["rgb", "r", "g", "b", "a"].into_iter().find(|known| *known == output)?;
    let texture = *stage.prims.get(path)?;
    if texture.value("info:id").and_then(Value::as_str) != Some("UsdUVTexture") {
        return None;
    }
    let file = texture.value("inputs:file").and_then(Value::as_str)?;
    // Package paths are relative to the package's root ("./textures/a.png", or
    // "package.usdz[textures/a.png]").
    let file = file.rsplit_once('[').map_or(file, |(_, inner)| inner.trim_end_matches(']'));
    let file = file.trim_start_matches("./").replace('\\', "/");
    let (name, bytes) = stage.assets.iter().find(|(name, _)| name.trim_start_matches("./") == file)?;
    let image = kition_scene_model::Image::new(name.rsplit('/').next().unwrap_or(name), bytes.to_vec()).ok()?;
    let hash = image.hash.clone();
    if !imported.images.iter().any(|known| known.hash == hash) {
        imported.images.push(image);
    }
    let scale = texture.value("inputs:scale").map(Value::numbers).unwrap_or_default();
    let scale = [0, 1, 2, 3].map(|index| scale.get(index).copied().filter(|value| value.is_finite()).unwrap_or(1.0));
    Some((hash, scale, output))
}

/// Millimetres per unit of a camera's lens and aperture values (tenths of a scene unit).
fn millimetres(meters: f64) -> f64 {
    100.0 * meters
}

fn read_camera(prim: &Prim, meters: f64) -> Camera {
    let mut camera = Camera::default();
    let number = |name: &str, fallback: f64| prim.number(name).filter(|value| value.is_finite() && *value > 0.0).unwrap_or(fallback);
    let aperture = number("horizontalAperture", 20.955);
    // USD authors a fixed horizontal aperture, not an aspect-dependent fit policy.
    camera.sensor_fit = kition_scene_model::scene::SensorFit::Horizontal;
    camera.sensor_height = (number("verticalAperture", 15.2908) * millimetres(meters)) as f32;
    camera.orthographic = prim.value("projection").and_then(Value::as_str) == Some("orthographic");
    if camera.orthographic {
        camera.ortho_scale = (aperture / 10.0 * meters) as f32;
    } else {
        camera.lens = (number("focalLength", 50.0) * millimetres(meters)) as f32;
        camera.sensor = (aperture * millimetres(meters)) as f32;
    }
    if let Some([near, far, ..]) = prim.value("clippingRange").map(Value::numbers).as_deref()
        && near.is_finite()
        && far.is_finite()
        && *near > 0.0
        && far > near
    {
        camera.clip_start = (near * meters) as f32;
        camera.clip_end = (far * meters) as f32;
    }
    camera
}

fn read_light(prim: &Prim, meters: f64) -> Light {
    let input = |name: &str| prim.number(&format!("inputs:{name}")).or_else(|| prim.number(name)).filter(|value| value.is_finite());
    let intensity = input("intensity").unwrap_or(1.0) * 2f64.powf(input("exposure").unwrap_or(0.0).clamp(-64.0, 64.0));
    let shaped = prim.property("inputs:shaping:cone:angle").is_some();
    let kind = match prim.type_name.as_str() {
        "DistantLight" => LightKind::Sun,
        "RectLight" | "DiskLight" => LightKind::Area,
        _ if shaped => LightKind::Spot,
        _ => LightKind::Point,
    };
    let mut light = Light::new(kind);
    light.power = match kind {
        LightKind::Sun => intensity / DISTANT_INTENSITY_PER_STRENGTH,
        _ => intensity / SPHERE_INTENSITY_PER_WATT,
    }
    .max(0.0) as f32;
    let color = prim.value("inputs:color").or_else(|| prim.value("color")).map(Value::numbers).map_or(DVec3::ONE, |values| vec3(&values, DVec3::ONE));
    if color.is_finite() {
        light.color = color.max(DVec3::ZERO).as_vec3();
    }
    let length = |value: f64| (value * meters).max(0.0) as f32;
    match prim.type_name.as_str() {
        "RectLight" => light.radius = length(input("width").unwrap_or(1.0).max(input("height").unwrap_or(1.0))),
        "DiskLight" => light.radius = length(2.0 * input("radius").unwrap_or(0.5)),
        "SphereLight" => {
            let point = prim.number("treatAsPoint").is_some_and(|value| value != 0.0);
            light.radius = if point { 0.0 } else { length(input("radius").unwrap_or(0.5)) };
        }
        _ => {}
    }
    if kind == LightKind::Spot
        && let Some(angle) = input("shaping:cone:angle")
    {
        light.spot_size = (2.0 * angle).clamp(1.0, 180.0).to_radians() as f32;
    }
    light
}
