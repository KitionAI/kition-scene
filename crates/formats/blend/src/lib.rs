//! Reads `.blend` files, the reference application's own format, into a Kition scene: objects
//! (with their hierarchy, transforms, visibility, and keyframes), meshes, curves, texts,
//! materials (the Principled BSDF's values when the material uses one), cameras, lights,
//! empties, and the modifiers Kition Scene has.
//!
//! The reader follows the file's own structure catalogue (SDNA) for every field it reads, so
//! layouts are looked up by name. Files are 64-bit little-endian, plain or zstd-compressed, in
//! the classic or the newer (large block header) container. Everything is bounds-checked;
//! damaged files are refused with a typed error.
#![forbid(unsafe_code)]

mod anim;
mod file;
mod mesh;
mod shape;

use std::collections::HashMap;

use kition_scene_exchange::{Imported, ImportedObject};
use kition_scene_model::glam::{Mat3, Mat4, Quat, Vec3};
use kition_scene_model::modifiers::{ModifierKind, ScrewAxis};
use kition_scene_model::scene::matrix_euler;
use kition_scene_model::{Camera, Id, Light, LightKind, Material, Modifier, ObjectData, RenderSettings, Scene, SceneError, World};

pub use file::File;
use file::{View, invalid};

/// What a `.blend` file holds, ready to be placed into a scene.
#[derive(Debug, Clone)]
pub struct BlendScene {
    pub imported: Imported,
    /// Each object's modifiers (by index into `imported.objects`).
    pub modifiers: Vec<Vec<Modifier>>,
    /// Hidden objects (by index into `imported.objects`).
    pub hidden: Vec<usize>,
    /// Booleans whose operand is another imported object: (object, modifier, operand).
    pub booleans: Vec<(usize, usize, usize)>,
    /// The scene camera, as an object index.
    pub camera: Option<usize>,
    pub world: Option<World>,
    pub render: Option<RenderSettings>,
}

/// The largest number of objects read.
const MAX_OBJECTS: usize = 100_000;

/// Reads a `.blend` file.
pub fn read(bytes: &[u8]) -> Result<BlendScene, SceneError> {
    let file = File::parse(bytes)?;
    let object_blocks: Vec<usize> = file
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, block)| &block.code == b"OB\0\0")
        .filter(|(_, block)| file.view_in("Object", block, 0).and_then(|object| object.nested("id")).is_some_and(|id| id.pointer("lib") == 0))
        .map(|(index, _)| index)
        .take(MAX_OBJECTS)
        .collect();
    let index_of: HashMap<usize, usize> = object_blocks.iter().enumerate().map(|(index, block)| (*block, index)).collect();
    let mut imported = Imported::default();
    let mut material_index: HashMap<usize, usize> = HashMap::new();
    let mut modifiers = Vec::new();
    let mut booleans = Vec::new();
    let mut hidden = Vec::new();
    let fps = frame_rate(&file);
    for (index, block) in object_blocks.iter().enumerate() {
        let Some(block) = file.blocks.get(*block) else { continue };
        let object = file.view_in("Object", block, 0).ok_or_else(|| invalid("the file's objects have no layout"))?;
        let name = id_name(&object);
        let data = object_data(&file, &object).map_err(|error| invalid(format!("{name}: {error}")))?;
        let material = material_pointer(&file, &object).and_then(|(pointer, from)| file.block(pointer, from)).map(|block| {
            let next = imported.materials.len();
            *material_index.entry(block.index).or_insert_with(|| {
                imported.materials.push(read_material(&file, block));
                next
            })
        });
        let (location, rotation, scale) = transform(&object);
        let parent = object.target("parent").and_then(|block| index_of.get(&block.index)).copied().filter(|parent| *parent != index);
        let parent_inverse = if parent.is_some() { matrix(&object, "parentinv").unwrap_or(Mat4::IDENTITY) } else { Mat4::IDENTITY };
        let (list, operands) = read_modifiers(&file, &object, &index_of);
        for (slot, operand) in operands {
            booleans.push((index, slot, operand));
        }
        // Hidden in the viewport (the first restriction bit).
        if object.int("restrictflag").unwrap_or(0) & 1 != 0 {
            hidden.push(index);
        }
        modifiers.push(list);
        imported.animation.extend(anim::read(&file, &object, index, fps));
        imported.objects.push(ImportedObject { name, data, location, rotation, scale, parent, parent_inverse, material });
    }
    // The scene: its camera, world, and picture size.
    let mut camera = None;
    let mut world = None;
    let mut render = None;
    if let Some(scene) = file.blocks.iter().find(|block| &block.code == b"SC\0\0").and_then(|block| file.view_in("Scene", block, 0)) {
        camera = scene.target("camera").and_then(|block| index_of.get(&block.index)).copied();
        if let Some(found) = scene.follow("world")
            && let (Some(r), Some(g), Some(b)) = (found.f32("horr"), found.f32("horg"), found.f32("horb"))
            && r.is_finite()
            && g.is_finite()
            && b.is_finite()
        {
            world = Some(World { color: Vec3::new(r, g, b), ..World::default() });
        }
        if let Some(settings) = scene.nested("r") {
            let percent = settings.number("size").filter(|value| *value > 0.0).unwrap_or(100.0) / 100.0;
            if let (Some(width), Some(height)) = (settings.number("xsch"), settings.number("ysch")) {
                let width = (width * percent).round().clamp(1.0, 8192.0) as u32;
                let height = (height * percent).round().clamp(1.0, 8192.0) as u32;
                render = Some(RenderSettings { width, height, ..RenderSettings::default() });
            }
        }
    }
    Ok(BlendScene { imported, modifiers, hidden, booleans, camera, world, render })
}

/// The scene's frame rate (24 when the file has none).
fn frame_rate(file: &File) -> f32 {
    let Some(scene) = file.blocks.iter().find(|block| &block.code == b"SC\0\0").and_then(|block| file.view_in("Scene", block, 0)) else {
        return 24.0;
    };
    let Some(settings) = scene.nested("r") else { return 24.0 };
    let fps = settings.f32("frs_sec").unwrap_or(24.0) / settings.f32("frs_sec_base").filter(|base| *base > 0.0).unwrap_or(1.0);
    if fps.is_finite() && fps > 0.0 { fps } else { 24.0 }
}

/// Reads a `.blend` file as objects to place (without modifiers; see [`place`]).
pub fn import(bytes: &[u8]) -> Result<Imported, SceneError> {
    read(bytes).map(|file| file.imported)
}

/// Adds a read file's objects to `scene` with their modifiers (a boolean's operand becomes the
/// placed object) and visibility, and makes the file's camera the scene camera when the scene
/// has none. Returns the new top-level object ids.
pub fn place(scene: &mut Scene, file: BlendScene) -> Result<Vec<Id>, SceneError> {
    let before = scene.objects.len();
    let top = kition_scene_exchange::place(scene, file.imported)?;
    let placed: Vec<Id> = scene.objects.iter().skip(before).map(|object| object.id).collect();
    for (index, list) in file.modifiers.into_iter().enumerate() {
        let Some(id) = placed.get(index) else { continue };
        if let Ok(object) = scene.object_mut(*id)
            && object.mesh().is_some()
        {
            object.modifiers = list;
        }
    }
    for (index, slot, operand) in file.booleans {
        let (Some(id), Some(other)) = (placed.get(index), placed.get(operand)) else { continue };
        if let Some(modifier) = scene.object_mut(*id).ok().and_then(|object| object.modifiers.get_mut(slot))
            && let ModifierKind::Boolean { object, .. } = &mut modifier.kind
        {
            *object = Some(*other);
        }
    }
    for index in file.hidden {
        if let Some(object) = placed.get(index).and_then(|id| scene.objects.iter_mut().find(|object| object.id == *id)) {
            object.hidden = true;
        }
    }
    if scene.camera.is_none() {
        scene.camera = file
            .camera
            .and_then(|index| placed.get(index).copied())
            .filter(|id| matches!(scene.object(*id).map(|object| &object.data), Ok(ObjectData::Camera(_))));
    }
    Ok(top)
}

fn id_name(view: &View<'_>) -> String {
    let name = view.nested("id").and_then(|id| id.text("name")).unwrap_or_default();
    let name = name.get(2..).unwrap_or_default().to_string();
    if name.is_empty() { "Object".into() } else { name }
}

fn matrix(view: &View<'_>, field: &str) -> Option<Mat4> {
    let values = view.floats(field)?;
    let array: [f32; 16] = values.try_into().ok()?;
    let matrix = Mat4::from_cols_array(&array);
    matrix.is_finite().then_some(matrix)
}

fn vector(view: &View<'_>, field: &str, default: Vec3) -> Vec3 {
    view.floats(field).and_then(|values| <[f32; 3]>::try_from(values).ok()).map(Vec3::from).filter(|value| value.is_finite()).unwrap_or(default)
}

/// Location, XYZ Euler rotation, and scale, whatever rotation mode the object uses.
fn transform(object: &View<'_>) -> (Vec3, Vec3, Vec3) {
    let location = vector(object, "loc", Vec3::ZERO);
    let scale = vector(object, "size", Vec3::ONE);
    let euler = vector(object, "rot", Vec3::ZERO);
    let rotation = match object.int("rotmode").unwrap_or(1) {
        0 => {
            let values = object.floats("quat").unwrap_or_default();
            match values.as_slice() {
                [w, x, y, z] => {
                    let quat = Quat::from_xyzw(*x, *y, *z, *w);
                    if quat.length_squared() > 1e-12 && quat.is_finite() { matrix_euler(Mat3::from_quat(quat.normalize())) } else { Vec3::ZERO }
                }
                _ => Vec3::ZERO,
            }
        }
        -1 => {
            let axis = vector(object, "rotAxis", Vec3::Y);
            let angle = object.f32("rotAngle").unwrap_or(0.0);
            if axis.length_squared() > 1e-12 && angle.is_finite() { matrix_euler(Mat3::from_axis_angle(axis.normalize(), angle)) } else { Vec3::ZERO }
        }
        mode @ 2..=6 => {
            // The other Euler orders (XZY, YXZ, YZX, ZXY, ZYX): the first letter applies first.
            let (x, y, z) = (Mat3::from_rotation_x(euler.x), Mat3::from_rotation_y(euler.y), Mat3::from_rotation_z(euler.z));
            let combined = match mode {
                2 => y * z * x,
                3 => z * x * y,
                4 => x * z * y,
                5 => y * x * z,
                _ => x * y * z,
            };
            matrix_euler(combined)
        }
        _ => euler,
    };
    (location, rotation, scale)
}

fn object_data(file: &File, object: &View<'_>) -> Result<ObjectData, SceneError> {
    let kind = object.int("type").unwrap_or(0);
    let data = object.follow("data");
    Ok(match (kind, data) {
        (1, Some(mesh)) => ObjectData::Mesh(mesh::read(file, &mesh)?),
        (2, Some(curve)) => shape::read_curve(file, &curve)?,
        (4, Some(curve)) => shape::read_text(file, &curve)?,
        (10, Some(lamp)) => ObjectData::Light(read_light(&lamp)),
        (11, Some(camera)) => ObjectData::Camera(read_camera(&camera)),
        _ => ObjectData::Empty { size: object.f32("empty_drawsize").filter(|size| size.is_finite() && *size > 0.0).unwrap_or(1.0) },
    })
}

fn read_light(lamp: &View<'_>) -> Light {
    let kind = match lamp.int("type").unwrap_or(0) {
        1 => LightKind::Sun,
        2 => LightKind::Spot,
        4 => LightKind::Area,
        _ => LightKind::Point,
    };
    let mut light = Light::new(kind);
    if let (Some(r), Some(g), Some(b)) = (lamp.f32("r"), lamp.f32("g"), lamp.f32("b"))
        && r.is_finite()
        && g.is_finite()
        && b.is_finite()
    {
        light.color = Vec3::new(r, g, b);
    }
    if let Some(energy) = lamp.f32("energy").filter(|energy| energy.is_finite()) {
        light.power = energy.max(0.0);
    }
    let radius = if kind == LightKind::Area { lamp.f32("area_size") } else { lamp.f32("radius") };
    if let Some(radius) = radius.filter(|radius| radius.is_finite()) {
        light.radius = radius.max(0.0);
    }
    if let Some(spot) = lamp.f32("spotsize").filter(|spot| spot.is_finite() && *spot > 0.0) {
        light.spot_size = spot;
    }
    light
}

fn read_camera(camera: &View<'_>) -> Camera {
    let mut out = Camera::default();
    let positive = |value: Option<f32>| value.filter(|value| value.is_finite() && *value > 0.0);
    if let Some(lens) = positive(camera.f32("lens")) {
        out.lens = lens;
    }
    if let Some(sensor) = positive(camera.f32("sensor_x")) {
        out.sensor = sensor;
    }
    if let Some(height) = positive(camera.f32("sensor_y")) {
        out.sensor_height = height;
    }
    out.sensor_fit = match camera.int("sensor_fit") {
        Some(1) => kition_scene_model::scene::SensorFit::Horizontal,
        Some(2) => kition_scene_model::scene::SensorFit::Vertical,
        _ => kition_scene_model::scene::SensorFit::Auto,
    };
    if let Some(start) = positive(camera.f32("clipsta")) {
        out.clip_start = start;
    }
    if let Some(end) = positive(camera.f32("clipend")) {
        out.clip_end = end.max(out.clip_start * 2.0);
    }
    out.orthographic = camera.int("type") == Some(1);
    if let Some(scale) = positive(camera.f32("ortho_scale")) {
        out.ortho_scale = scale;
    }
    out
}

/// The first material slot: the object's own when its slot says so, else the mesh's.
fn material_pointer(file: &File, object: &View<'_>) -> Option<(u64, usize)> {
    let first_pointer = |block: Option<&file::Block>| -> Option<(u64, usize)> {
        let bytes = file.bytes(block?).get(..8)?;
        let mut array = [0u8; 8];
        array.copy_from_slice(bytes);
        let block = block?;
        Some((u64::from_le_bytes(array), block.index)).filter(|(pointer, _)| *pointer != 0)
    };
    let object_slot = object.target("matbits").and_then(|block| file.bytes(block).first().copied()).unwrap_or(0) != 0;
    if object_slot && let Some(found) = first_pointer(object.target("mat")) {
        return Some(found);
    }
    let data = object.follow("data")?;
    if data.info.name != "Mesh" {
        return None;
    }
    first_pointer(data.target("mat"))
}

fn read_material(file: &File, block: &file::Block) -> Material {
    let mut out = Material::new(0, "Material");
    let Some(material) = file.view_in("Material", block, 0) else { return out };
    out.name = id_name(&material);
    if let (Some(r), Some(g), Some(b)) = (material.f32("r"), material.f32("g"), material.f32("b")) {
        out.base_color = [r, g, b, material.f32("a").unwrap_or(1.0)];
    }
    if let Some(metallic) = material.f32("metallic") {
        out.metallic = metallic.clamp(0.0, 1.0);
    }
    if let Some(roughness) = material.f32("roughness") {
        out.roughness = roughness.clamp(0.0, 1.0);
    }
    // The Principled BSDF's own inputs, when the material uses nodes.
    if let Some(tree) = material.follow("nodetree") {
        fn first<'a>(list: Option<View<'a>>) -> Option<&'a file::Block> {
            list.and_then(|list| list.target("first"))
        }
        let mut node = first(tree.nested("nodes")).and_then(|block| file.view_in("bNode", block, 0));
        let mut guard = 0;
        while let Some(current) = node {
            guard += 1;
            if guard > 10_000 {
                break;
            }
            if current.text("idname").as_deref() == Some("ShaderNodeBsdfPrincipled") {
                let mut socket = first(current.nested("inputs")).and_then(|block| file.view_in("bNodeSocket", block, 0));
                let mut count = 0;
                while let Some(input) = socket {
                    count += 1;
                    if count > 1000 {
                        break;
                    }
                    let value = input.follow("default_value");
                    let color = |value: &View<'_>| value.floats("value").and_then(|values| <[f32; 4]>::try_from(values).ok());
                    match (input.text("identifier").as_deref(), value) {
                        (Some("Base Color"), Some(value)) => {
                            if let Some(rgba) = color(&value) {
                                out.base_color = rgba;
                            }
                        }
                        (Some("Metallic"), Some(value)) => out.metallic = value.f32("value").unwrap_or(out.metallic).clamp(0.0, 1.0),
                        (Some("Roughness"), Some(value)) => out.roughness = value.f32("value").unwrap_or(out.roughness).clamp(0.0, 1.0),
                        (Some("Emission Color"), Some(value)) => {
                            if let Some([r, g, b, _]) = color(&value) {
                                out.emission = Vec3::new(r, g, b);
                            }
                        }
                        (Some("Emission Strength"), Some(value)) => out.emission_strength = value.f32("value").unwrap_or(0.0).max(0.0),
                        _ => {}
                    }
                    socket = input.target("next").and_then(|block| file.view_in("bNodeSocket", block, 0));
                }
                break;
            }
            node = current.target("next").and_then(|block| file.view_in("bNode", block, 0));
        }
    }
    if !out.base_color.iter().all(|value| value.is_finite()) {
        out.base_color = [0.8, 0.8, 0.8, 1.0];
    }
    if !(out.emission.is_finite() && out.emission_strength.is_finite()) {
        out.emission = Vec3::ZERO;
        out.emission_strength = 0.0;
    }
    out
}

/// The object's modifiers Kition Scene knows, in order, and booleans' operands as object indices.
fn read_modifiers(file: &File, object: &View<'_>, index_of: &HashMap<usize, usize>) -> (Vec<Modifier>, Vec<(usize, usize)>) {
    let mut out = Vec::new();
    let mut operands = Vec::new();
    let mut next = object.nested("modifiers").and_then(|list| list.target("first"));
    let mut guard = 0;
    while let Some(block) = next {
        guard += 1;
        if guard > 1000 {
            break;
        }
        let kind_name = file.struct_of(block).to_string();
        let Some(view) = file.view_in(&kind_name, block, 0) else { break };
        let header = view.nested("modifier");
        next = header.and_then(|header| header.target("next"));
        let name = header.and_then(|header| header.text("name")).unwrap_or_default();
        // Shown in the viewport: the first mode bit.
        let enabled = header.and_then(|header| header.int("mode")).is_none_or(|mode| mode & 1 != 0);
        let number = |field: &str| view.f32(field).filter(|value| value.is_finite());
        let int = |field: &str| view.int(field);
        let kind = match kind_name.as_str() {
            "MirrorModifierData" => {
                let flag = int("flag").unwrap_or(8);
                Some(ModifierKind::Mirror {
                    axes: [flag & 8 != 0, flag & 16 != 0, flag & 32 != 0],
                    merge: flag & 128 == 0,
                    merge_threshold: number("tolerance").unwrap_or(0.001),
                })
            }
            "ArrayModifierData" => {
                let offset_type = int("offset_type").unwrap_or(2);
                let relative = if offset_type & 2 != 0 { vector(&view, "scale", Vec3::X) } else { Vec3::ZERO };
                let constant = if offset_type & 1 != 0 { vector(&view, "offset", Vec3::ZERO) } else { Vec3::ZERO };
                let count = int("count").unwrap_or(2).clamp(1, 1000) as u32;
                Some(ModifierKind::Array { count, relative_offset: relative, constant_offset: constant, merge: int("flags").unwrap_or(0) & 1 != 0 })
            }
            "SubsurfModifierData" => Some(ModifierKind::Subdivision { levels: int("levels").unwrap_or(1).clamp(0, 6) as u32 }),
            "SolidifyModifierData" => Some(ModifierKind::Solidify {
                thickness: number("offset").unwrap_or(0.01),
                offset: number("offset_fac").unwrap_or(-1.0),
                rim: int("flag").unwrap_or(1) & 1 != 0,
            }),
            "TriangulateModifierData" => Some(ModifierKind::Triangulate),
            "WeldModifierData" => Some(ModifierKind::Weld { distance: number("merge_dist").unwrap_or(0.001) }),
            "SmoothModifierData" => {
                Some(ModifierKind::Smooth { factor: number("fac").unwrap_or(0.5), iterations: int("repeat").unwrap_or(1).clamp(0, 100) as u32 })
            }
            "BevelModifierData" => {
                // Limited by angle (limit bit 8), else every edge.
                let angle = if int("lim_flags").unwrap_or(8) & 8 != 0 { number("bevel_angle").unwrap_or(30f32.to_radians()) } else { 0.0 };
                Some(ModifierKind::Bevel {
                    width: number("value").unwrap_or(0.1),
                    segments: int("res").unwrap_or(1).clamp(1, 64) as u32,
                    angle_limit: angle,
                })
            }
            "BooleanModifierData" => {
                let operation = match int("operation").unwrap_or(2) {
                    0 => kition_scene_model::csg::Operation::Intersect,
                    1 => kition_scene_model::csg::Operation::Union,
                    _ => kition_scene_model::csg::Operation::Difference,
                };
                if let Some(operand) = view.target("object").and_then(|block| index_of.get(&block.index)) {
                    operands.push((out.len(), *operand));
                }
                Some(ModifierKind::Boolean { operation, object: None })
            }
            "ScrewModifierData" => Some(ModifierKind::Screw {
                steps: int("steps").unwrap_or(16).clamp(1, 1000) as u32,
                angle: number("angle").unwrap_or(std::f32::consts::TAU),
                screw_offset: number("screw_ofs").unwrap_or(0.0),
                axis: match int("axis").unwrap_or(2) {
                    0 => ScrewAxis::X,
                    1 => ScrewAxis::Y,
                    _ => ScrewAxis::Z,
                },
            }),
            other => kition_scene_model::modifiers2::from_blend(other, &|field, index| view.number_at(field, index)),
        };
        let Some(kind) = kind else { continue };
        if kind.check().is_err() {
            if operands.last().is_some_and(|(slot, _)| *slot == out.len()) {
                operands.pop();
            }
            continue;
        }
        let mut modifier = Modifier::new(kind);
        if !name.is_empty() {
            modifier.name = name;
        }
        modifier.enabled = enabled;
        out.push(modifier);
    }
    (out, operands)
}
