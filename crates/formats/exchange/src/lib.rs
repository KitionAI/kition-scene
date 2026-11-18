//! Scene exchange formats: glTF 2.0 (`.glb`, `.gltf` with an embedded buffer), Wavefront OBJ,
//! and STL, in and out.
//!
//! The scene is Z-up. glTF and OBJ are Y-up: a scene point (x, y, z) is written as (x, z, -y),
//! the convention of the common modeller's default exporters. STL stays Z-up.
#![forbid(unsafe_code)]

mod base64;
mod gltf;
mod obj;
mod stl;

use std::collections::{BTreeSet, HashMap};

use kition_scene_model::glam::{Mat4, Vec3, Vec4};
use kition_scene_model::{Id, Material, ObjectData, Scene, SceneError};

/// An exchange format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Binary glTF 2.0.
    Glb,
    /// glTF 2.0 JSON with its buffer embedded as a data URI.
    Gltf,
    /// Wavefront OBJ.
    Obj,
    /// STL (binary written, binary and ASCII read).
    Stl,
}

impl Format {
    /// The format of a file extension (without the dot, any case).
    pub fn from_extension(ext: &str) -> Option<Format> {
        match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "glb" => Some(Self::Glb),
            "gltf" => Some(Self::Gltf),
            "obj" => Some(Self::Obj),
            "stl" => Some(Self::Stl),
            _ => None,
        }
    }

    /// The file extension, without the dot.
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Glb => "glb",
            Self::Gltf => "gltf",
            Self::Obj => "obj",
            Self::Stl => "stl",
        }
    }

    /// The media type.
    pub fn mime(&self) -> &'static str {
        match self {
            Self::Glb => "model/gltf-binary",
            Self::Gltf => "model/gltf+json",
            Self::Obj => "model/obj",
            Self::Stl => "model/stl",
        }
    }
}

/// Objects read from a file, ready to be added to a scene.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Imported {
    pub objects: Vec<ImportedObject>,
    /// Materials; their ids are ignored ([`place`] gives new ones).
    pub materials: Vec<Material>,
    /// Images the materials name by hash.
    pub images: Vec<kition_scene_model::Image>,
    /// Keyframes of the objects' transforms.
    pub animation: Vec<ImportedChannel>,
    /// Meshes deformed by armatures: (mesh object, armature object), indices into `objects`.
    pub deforms: Vec<(usize, usize)>,
}

/// The keys of one component of an imported object's property.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedChannel {
    /// An index into [`Imported::objects`].
    pub object: usize,
    pub property: kition_scene_model::Property,
    pub index: u8,
    /// The bone or shape key a named property belongs to; empty otherwise.
    pub path: String,
    /// Times in seconds, and values.
    pub keys: Vec<(f32, f32)>,
    pub interpolation: kition_scene_model::Interpolation,
}

/// One object read from a file.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedObject {
    pub name: String,
    pub data: ObjectData,
    /// Location, XYZ Euler rotation, and scale relative to the parent.
    pub location: Vec3,
    pub rotation: Vec3,
    pub scale: Vec3,
    /// An index into [`Imported::objects`].
    pub parent: Option<usize>,
    /// Applied between the parent and this object (the scene's parent inverse); the identity
    /// unless the file's local matrix has shear that location, rotation, and scale cannot hold.
    pub parent_inverse: Mat4,
    /// An index into [`Imported::materials`].
    pub material: Option<usize>,
}

/// Writes the scene's visible objects (only those in `only`, when given) in `format`.
pub fn export(scene: &Scene, format: Format, only: Option<&[Id]>) -> Result<Vec<u8>, SceneError> {
    // The NLA and F-curve modifiers go out as the keys they make on every frame.
    let baked = if matches!(format, Format::Glb | Format::Gltf) { scene.baked_animation()? } else { None };
    let scene = baked.as_ref().unwrap_or(scene);
    match format {
        Format::Glb => gltf::export(scene, only, true),
        Format::Gltf => gltf::export(scene, only, false),
        Format::Obj => obj::export(scene, only),
        Format::Stl => stl::export(scene, only),
    }
}

/// Reads a file of `format`.
pub fn import(bytes: &[u8], format: Format) -> Result<Imported, SceneError> {
    match format {
        Format::Glb | Format::Gltf => gltf::import(bytes),
        Format::Obj => obj::import(bytes),
        Format::Stl => stl::import(bytes),
    }
}

/// Adds imported objects to the scene with new ids and unique names, appending their
/// materials. Returns the ids of the new objects that have no parent.
pub fn place(scene: &mut Scene, imported: Imported) -> Result<Vec<Id>, SceneError> {
    let count = imported.objects.len();
    for (index, object) in imported.objects.iter().enumerate() {
        if object.parent.is_some_and(|parent| parent >= count) {
            return Err(invalid(format!("object {index} names a parent that is not there")));
        }
        if object.material.is_some_and(|material| material >= imported.materials.len()) {
            return Err(invalid(format!("object {index} names a material that is not there")));
        }
        if !(object.location.is_finite() && object.rotation.is_finite() && object.scale.is_finite() && object.parent_inverse.is_finite()) {
            return Err(invalid(format!("object {index} has a transform that is not finite")));
        }
        match &object.data {
            ObjectData::Mesh(mesh) => mesh.validate()?,
            ObjectData::Armature(armature) => armature.validate()?,
            _ => {}
        }
        // No cycles: walking up from any object ends within `count` steps.
        let mut current = object.parent;
        let mut steps = 0;
        while let Some(at) = current {
            steps += 1;
            if steps > count {
                return Err(invalid("the imported hierarchy has a cycle"));
            }
            current = imported.objects.get(at).and_then(|object| object.parent);
        }
    }
    for material in &imported.materials {
        for hash in material.images() {
            if !imported.images.iter().any(|image| &image.hash == hash) && scene.image(hash).is_none() {
                return Err(invalid(format!("material {} names an image that is not there", material.name)));
            }
        }
    }
    for image in imported.images {
        scene.add_image(image);
    }
    let mut material_ids = Vec::with_capacity(imported.materials.len());
    for material in &imported.materials {
        let id = scene.add_material(&material.name);
        let slot = scene.material_mut(id)?;
        *slot = Material { id, name: slot.name.clone(), ..material.clone() };
        material_ids.push(id);
    }
    let mut ids = Vec::with_capacity(count);
    for object in &imported.objects {
        let id = scene.add_object(&object.name, object.data.clone());
        let slot = scene.object_mut(id)?;
        slot.location = object.location;
        slot.rotation = object.rotation;
        slot.scale = object.scale;
        slot.material = object.material.and_then(|index| material_ids.get(index).copied());
        ids.push(id);
    }
    for (mesh, rig) in &imported.deforms {
        let (Some(mesh), Some(rig)) = (ids.get(*mesh).copied(), ids.get(*rig).copied()) else {
            return Err(invalid("an imported deform names objects that are not there"));
        };
        if kition_scene_model::armature::armature_of(scene, rig).is_none() {
            return Err(invalid("an imported deform names something that is not an armature"));
        }
        let object = scene.object_mut(mesh)?;
        if object.mesh().is_some() {
            object.modifiers.insert(
                0,
                kition_scene_model::Modifier::new(kition_scene_model::ModifierKind::Armature {
                    object: Some(rig),
                    preserve_volume: false,
                    deform: Default::default(),
                }),
            );
        }
    }
    let mut roots = Vec::new();
    for (object, id) in imported.objects.iter().zip(&ids) {
        match object.parent.and_then(|parent| ids.get(parent).copied()) {
            Some(parent) => {
                let slot = scene.object_mut(*id)?;
                slot.parent = Some(parent);
                slot.parent_inverse = object.parent_inverse;
            }
            None => roots.push(*id),
        }
    }
    if scene.camera.is_none() {
        scene.camera = ids.iter().copied().find(|id| scene.object(*id).is_ok_and(|object| matches!(object.data, ObjectData::Camera(_))));
    }
    // Keys land on the scene's frames (seconds times its frame rate); the objects take their
    // values at the current frame.
    let fps = scene.animation.timeline.fps;
    let frame = scene.animation.timeline.frame as f32;
    for channel in imported.animation {
        let Some(&target) = ids.get(channel.object) else { continue };
        let mut keys: Vec<kition_scene_model::Keyframe> = Vec::with_capacity(channel.keys.len());
        for (time, value) in channel.keys {
            let frame = (time * fps * 1000.0).round() / 1000.0;
            if !(frame.is_finite() && value.is_finite()) || frame.abs() > kition_scene_model::animation::MAX_FRAME as f32 {
                return Err(invalid("an imported keyframe is out of range"));
            }
            match keys.last_mut() {
                Some(last) if (last.frame - frame).abs() < 1e-4 => last.value = value,
                Some(last) if last.frame > frame => return Err(invalid("imported keyframes are not in order")),
                _ => keys.push(kition_scene_model::Keyframe { frame, value, interpolation: channel.interpolation }),
            }
        }
        if keys.is_empty() || channel.index >= channel.property.components() {
            continue;
        }
        let animated = kition_scene_model::Channel {
            target,
            property: channel.property,
            index: channel.index,
            path: channel.path.clone(),
            keys,
            driver: None,
            modifiers: Vec::new(),
        };
        if let Some(value) = animated.evaluate(frame) {
            kition_scene_model::animation::write_at(scene, target, channel.property, channel.index, &channel.path, value);
        }
        scene.animation.channels.retain(|other| {
            !(other.target == target && other.property == channel.property && other.index == channel.index && other.path == channel.path)
        });
        scene.animation.channels.push(animated);
    }
    Ok(roots)
}

/// Scene (Z-up) to Y-up: (x, y, z) → (x, z, -y).
pub(crate) fn y_up() -> Mat4 {
    Mat4::from_cols(Vec4::X, Vec4::new(0.0, 0.0, -1.0, 0.0), Vec4::new(0.0, 1.0, 0.0, 0.0), Vec4::W)
}

/// A scene point in Y-up coordinates.
pub(crate) fn to_y_up(point: Vec3) -> Vec3 {
    Vec3::new(point.x, point.z, -point.y)
}

/// A Y-up point in scene coordinates.
pub(crate) fn from_y_up(point: Vec3) -> Vec3 {
    Vec3::new(point.x, -point.z, point.y)
}

/// The objects to export, in scene order: those in `only` (or every visible one) plus, when
/// `ancestors`, every ancestor of them (which may be hidden).
pub fn chosen(scene: &Scene, only: Option<&[Id]>, ancestors: bool) -> Result<Vec<Id>, SceneError> {
    let mut set: BTreeSet<Id> = BTreeSet::new();
    match only {
        Some(ids) => {
            for id in ids {
                scene.object(*id)?;
                set.insert(*id);
            }
        }
        None => set.extend(scene.objects.iter().filter(|object| !object.hidden).map(|object| object.id)),
    }
    if ancestors {
        let wanted: Vec<Id> = set.iter().copied().collect();
        for id in wanted {
            let mut current = scene.object(id)?.parent;
            let mut steps = 0;
            while let Some(at) = current {
                set.insert(at);
                current = scene.object(at)?.parent;
                steps += 1;
                if steps > scene.objects.len() {
                    return Err(invalid("the parent hierarchy has a cycle"));
                }
            }
        }
    }
    Ok(scene.objects.iter().map(|object| object.id).filter(|id| set.contains(id)).collect())
}

/// Joins vertices at exactly the same position (negative zero equals zero). Returns the new
/// positions and, for each old vertex, its new index.
pub(crate) fn weld_exact(positions: &[Vec3]) -> (Vec<Vec3>, Vec<u32>) {
    let mut seen: HashMap<[u32; 3], u32> = HashMap::with_capacity(positions.len());
    let mut out = Vec::new();
    let mut remap = Vec::with_capacity(positions.len());
    for point in positions {
        let key = point.to_array().map(|value| if value == 0.0 { 0u32 } else { value.to_bits() });
        let index = *seen.entry(key).or_insert_with(|| {
            out.push(*point);
            u32::try_from(out.len() - 1).unwrap_or(u32::MAX)
        });
        remap.push(index);
    }
    (out, remap)
}

/// Faces of a welded triangle soup: triangles whose corners collapsed are dropped.
pub(crate) fn welded_triangles(triangles: &[[u32; 3]], remap: &[u32]) -> Vec<Vec<u32>> {
    triangles
        .iter()
        .filter_map(|triangle| {
            let [a, b, c] = triangle.map(|index| remap.get(index as usize).copied().unwrap_or(u32::MAX));
            (a != b && b != c && a != c && a != u32::MAX && b != u32::MAX && c != u32::MAX).then(|| vec![a, b, c])
        })
        .collect()
}

/// The largest number of vertices, faces, or elements read from one file.
pub(crate) const MAX_ITEMS: usize = 20_000_000;

pub(crate) fn invalid(message: impl Into<String>) -> SceneError {
    SceneError::InvalidFile(message.into())
}
