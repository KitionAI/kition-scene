//! glTF 2.0: binary `.glb` and `.gltf` JSON with one embedded data-URI buffer.
//!
//! Nodes are converted to Y-up at every level (a node's matrix `M` becomes `C M C⁻¹`, `C` the
//! Z-up to Y-up basis change), so hierarchies keep their shape. Cameras and lights look down
//! their local -Z in both conventions, so their nodes are `C M` instead. Mesh polygons are
//! triangulated; the polygon sizes are kept in the mesh's `extras` so a file this crate wrote
//! reads back as the same polygons.

use std::collections::{BTreeSet, HashMap};

use kition_scene_model::glam::{Mat3, Mat4, Quat, Vec2, Vec3};
use kition_scene_model::scene::matrix_euler;
use kition_scene_model::{Camera, Id, Image, Light, LightKind, Material, Mesh, ObjectData, Scene, SceneError};
use serde_json::{Map, Value, json};

use crate::{Imported, ImportedObject, MAX_ITEMS, base64, chosen, from_y_up, invalid, to_y_up, weld_exact, y_up};

#[path = "gltf_anim.rs"]
mod anim;
#[path = "gltf_skin.rs"]
mod skin;

/// Candela per watt for point and spot lights, and lux per W/m² for suns (the luminous
/// efficacy 683 lm/W, spread over 4π steradians for a point).
const POINT_CANDELA_PER_WATT: f32 = 683.0 / (4.0 * std::f32::consts::PI);
const SUN_LUX_PER_WATT: f32 = 683.0;
/// The blend between a spot's inner and outer cone a new spot gets.
const SPOT_BLEND: f32 = 0.15;

const GLB_MAGIC: &[u8; 4] = b"glTF";
const CHUNK_JSON: u32 = 0x4E4F_534A;
const CHUNK_BIN: u32 = 0x004E_4942;

const FLOAT: u64 = 5126;
const UNSIGNED_BYTE: u64 = 5121;
const UNSIGNED_SHORT: u64 = 5123;
const UNSIGNED_INT: u64 = 5125;

fn is_oriented(data: &ObjectData) -> bool {
    matches!(data, ObjectData::Camera(_) | ObjectData::Light(_))
}

/// A node's world matrix in Y-up space.
fn to_gltf_world(world: Mat4, oriented: bool) -> Mat4 {
    if oriented { y_up() * world } else { y_up() * world * y_up().inverse() }
}

/// A node's world matrix in the scene's Z-up space.
fn from_gltf_world(world: Mat4, oriented: bool) -> Mat4 {
    if oriented { y_up().inverse() * world } else { y_up().inverse() * world * y_up() }
}

struct Writer {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Writer {
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        let mut view = json!({ "buffer": 0, "byteOffset": self.bin.len(), "byteLength": bytes.len() });
        if let (Some(target), Value::Object(map)) = (target, &mut view) {
            map.insert("target".into(), json!(target));
        }
        self.bin.extend_from_slice(bytes);
        self.views.push(view);
        self.views.len() - 1
    }

    fn vec3s(&mut self, values: &[Vec3], bounds: bool) -> usize {
        let mut bytes = Vec::with_capacity(values.len() * 12);
        for value in values {
            for component in value.to_array() {
                bytes.extend(component.to_le_bytes());
            }
        }
        let view = self.view(&bytes, Some(34962));
        let mut accessor = json!({ "bufferView": view, "componentType": FLOAT, "count": values.len(), "type": "VEC3" });
        if bounds && !values.is_empty() {
            let min = values.iter().copied().fold(Vec3::splat(f32::INFINITY), Vec3::min);
            let max = values.iter().copied().fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
            if let Value::Object(map) = &mut accessor {
                map.insert("min".into(), json!(min.to_array()));
                map.insert("max".into(), json!(max.to_array()));
            }
        }
        self.accessors.push(accessor);
        self.accessors.len() - 1
    }

    fn vec2s(&mut self, values: &[Vec2]) -> usize {
        let mut bytes = Vec::with_capacity(values.len() * 8);
        for value in values {
            for component in value.to_array() {
                bytes.extend(component.to_le_bytes());
            }
        }
        let view = self.view(&bytes, Some(34962));
        self.accessors.push(json!({ "bufferView": view, "componentType": FLOAT, "count": values.len(), "type": "VEC2" }));
        self.accessors.len() - 1
    }

    /// An image's bytes as a buffer view (for `images[].bufferView`).
    fn image(&mut self, bytes: &[u8]) -> usize {
        self.view(bytes, None)
    }

    fn indices(&mut self, values: &[u32]) -> usize {
        let mut bytes = Vec::with_capacity(values.len() * 4);
        for value in values {
            bytes.extend(value.to_le_bytes());
        }
        let view = self.view(&bytes, Some(34963));
        self.accessors.push(json!({ "bufferView": view, "componentType": UNSIGNED_INT, "count": values.len(), "type": "SCALAR" }));
        self.accessors.len() - 1
    }
}

/// The mesh in Y-up space as glTF primitives' data.
fn mesh_json(writer: &mut Writer, mesh: &Mesh, name: &str, material: Option<usize>, extra: Option<&skin::Attributes>) -> Value {
    let mut primitives = Vec::new();
    let mut face_triangles: Vec<u32> = vec![0; mesh.faces.len()];
    let triangles = mesh.triangles();
    for (_, face) in &triangles {
        if let Some(count) = face_triangles.get_mut(*face as usize) {
            *count += 1;
        }
    }
    if !triangles.is_empty() {
        let uvs = mesh.has_uvs() && mesh.uvs_aligned();
        let mut coordinates: Vec<Vec2> = Vec::new();
        // The mesh vertex each written vertex is (for joints, weights, and shape offsets).
        let mut sources: Vec<u32> = Vec::new();
        // Smooth by angle gives corners their own normals: each face then gets its own corners.
        let (positions, normals, indices) = if mesh.smooth && mesh.smooth_angle.is_none() && !uvs {
            let positions: Vec<Vec3> = mesh.positions.iter().map(|point| to_y_up(*point)).collect();
            let normals: Vec<Vec3> = mesh.vertex_normals().into_iter().map(to_y_up).collect();
            let indices: Vec<u32> = triangles.iter().flat_map(|(triangle, _)| *triangle).collect();
            sources = (0..mesh.vertex_count()).collect();
            (positions, normals, indices)
        } else {
            // Every face gets its own corners, with the face's normal (or the vertex's, smooth)
            // and the corner's texture coordinate (V flipped: glTF's runs down).
            let corner_normals = mesh.corner_normals();
            let mut positions = Vec::new();
            let mut normals = Vec::new();
            let mut first_corner = Vec::with_capacity(mesh.faces.len());
            for (face_index, face) in mesh.faces.iter().enumerate() {
                first_corner.push(u32::try_from(positions.len()).unwrap_or(u32::MAX));
                let normal = to_y_up(mesh.face_normal(face));
                for (corner, index) in face.iter().enumerate() {
                    sources.push(*index);
                    positions.push(to_y_up(mesh.positions.get(*index as usize).copied().unwrap_or(Vec3::ZERO)));
                    normals.push(corner_normals.get(face_index).and_then(|list| list.get(corner)).map_or(normal, |normal| to_y_up(*normal)));
                    if uvs {
                        let uv = mesh.corner_uv(face_index, corner);
                        coordinates.push(Vec2::new(uv.x, 1.0 - uv.y));
                    }
                }
            }
            let mut indices = Vec::with_capacity(triangles.len() * 3);
            for (triangle, face) in &triangles {
                let corners = mesh.faces.get(*face as usize).map_or(&[][..], Vec::as_slice);
                let base = first_corner.get(*face as usize).copied().unwrap_or(0);
                let lookup: Option<HashMap<u32, u32>> = (corners.len() > 16)
                    .then(|| corners.iter().enumerate().map(|(slot, index)| (*index, u32::try_from(slot).unwrap_or(0))).collect());
                for index in triangle {
                    let slot = match &lookup {
                        Some(map) => map.get(index).copied().unwrap_or(0),
                        None => corners.iter().position(|corner| corner == index).and_then(|slot| u32::try_from(slot).ok()).unwrap_or(0),
                    };
                    indices.push(base + slot);
                }
            }
            (positions, normals, indices)
        };
        let position = writer.vec3s(&positions, true);
        let normal = writer.vec3s(&normals, false);
        let index = writer.indices(&indices);
        let mut primitive = json!({ "attributes": { "POSITION": position, "NORMAL": normal }, "indices": index, "mode": 4 });
        if !coordinates.is_empty() {
            let texcoord = writer.vec2s(&coordinates);
            if let Some(attributes) = primitive.get_mut("attributes").and_then(Value::as_object_mut) {
                attributes.insert("TEXCOORD_0".into(), json!(texcoord));
            }
        }
        if let (Some(material), Value::Object(map)) = (material, &mut primitive) {
            map.insert("material".into(), json!(material));
        }
        if let Some(extra) = extra {
            skin::attach(writer, &mut primitive, &sources, extra);
        }
        primitives.push(primitive);
    }
    if !mesh.loose_edges.is_empty() {
        let positions: Vec<Vec3> = mesh.positions.iter().map(|point| to_y_up(*point)).collect();
        let position = writer.vec3s(&positions, true);
        let indices: Vec<u32> = mesh.loose_edges.iter().flatten().copied().collect();
        let index = writer.indices(&indices);
        let mut primitive = json!({ "attributes": { "POSITION": position }, "indices": index, "mode": 1 });
        if let Some(extra) = extra {
            skin::attach(writer, &mut primitive, &(0..mesh.vertex_count()).collect::<Vec<_>>(), extra);
        }
        primitives.push(primitive);
    }
    let mut value = json!({
        "name": name,
        "primitives": primitives,
        "extras": { "kition": { "faceTriangles": face_triangles, "smooth": mesh.smooth } },
    });
    if let Some(extra) = extra {
        skin::mesh_extras(&mut value, extra);
    }
    value
}

/// Images and textures written so far, by image hash.
#[derive(Default)]
struct Textures {
    images: Vec<Value>,
    textures: Vec<Value>,
    by_hash: HashMap<String, usize>,
}

impl Textures {
    /// The texture index of image `hash`, writing the image the first time.
    fn index(&mut self, writer: &mut Writer, scene: &Scene, hash: &str) -> Option<usize> {
        if let Some(index) = self.by_hash.get(hash) {
            return Some(*index);
        }
        let image = scene.image(hash)?;
        // glTF carries PNG and JPEG images only.
        if image.format == kition_scene_model::ImageFormat::Hdr {
            return None;
        }
        let view = writer.image(&image.bytes);
        self.images.push(json!({ "name": image.name, "bufferView": view, "mimeType": image.format.mime() }));
        self.textures.push(json!({ "source": self.images.len() - 1, "sampler": 0 }));
        let index = self.textures.len() - 1;
        self.by_hash.insert(hash.to_string(), index);
        Some(index)
    }
}

fn material_json(material: &Material, scene: &Scene, writer: &mut Writer, textures: &mut Textures) -> Value {
    let mut out = json!({
        "name": material.name,
        "doubleSided": true,
        "pbrMetallicRoughness": {
            "baseColorFactor": material.base_color,
            "metallicFactor": material.metallic,
            "roughnessFactor": material.roughness,
        },
    });
    for (key, hash) in [("baseColorTexture", &material.base_color_texture), ("metallicRoughnessTexture", &material.metallic_roughness_texture)] {
        let Some(index) = hash.as_deref().and_then(|hash| textures.index(writer, scene, hash)) else { continue };
        if let Some(pbr) = out.get_mut("pbrMetallicRoughness").and_then(Value::as_object_mut) {
            pbr.insert(key.into(), json!({ "index": index }));
        }
    }
    if let Some(index) = material.normal_texture.as_deref().and_then(|hash| textures.index(writer, scene, hash))
        && let Value::Object(map) = &mut out
    {
        let mut normal = json!({ "index": index });
        if material.normal_strength != 1.0
            && let Value::Object(fields) = &mut normal
        {
            fields.insert("scale".into(), json!(material.normal_strength));
        }
        map.insert("normalTexture".into(), normal);
    }
    let mut extensions = serde_json::Map::new();
    let emission = material.emission * material.emission_strength;
    if emission.max_element() > 0.0 {
        let peak = emission.max_element();
        let (factor, strength) = if peak > 1.0 { (emission / peak, peak) } else { (emission, 1.0) };
        if let Value::Object(map) = &mut out {
            map.insert("emissiveFactor".into(), json!(factor.to_array()));
        }
        if strength > 1.0 {
            extensions.insert("KHR_materials_emissive_strength".into(), json!({ "emissiveStrength": strength }));
        }
    }
    if material.transmission > 0.0 {
        extensions.insert("KHR_materials_transmission".into(), json!({ "transmissionFactor": material.transmission }));
    }
    if material.ior != 1.5 {
        extensions.insert("KHR_materials_ior".into(), json!({ "ior": material.ior }));
    }
    if let Value::Object(map) = &mut out {
        match material.alpha_mode {
            kition_scene_model::AlphaMode::Opaque => {}
            kition_scene_model::AlphaMode::Clip => {
                map.insert("alphaMode".into(), json!("MASK"));
                map.insert("alphaCutoff".into(), json!(material.alpha_cutoff));
            }
            kition_scene_model::AlphaMode::Blend => {
                map.insert("alphaMode".into(), json!("BLEND"));
            }
        }
        if !extensions.is_empty() {
            map.insert("extensions".into(), Value::Object(extensions));
        }
    }
    out
}

fn camera_json(camera: &Camera, name: &str, aspect: f32) -> Value {
    let (xmag, ymag) = camera.frame_half_size(aspect);
    if camera.orthographic {
        return json!({ "name": name, "type": "orthographic", "orthographic": { "xmag": xmag, "ymag": ymag, "znear": camera.clip_start, "zfar": camera.clip_end } });
    }
    let yfov = 2.0 * f64::from(ymag).atan();
    json!({ "name": name, "type": "perspective", "perspective": { "aspectRatio": aspect, "yfov": yfov, "znear": camera.clip_start, "zfar": camera.clip_end } })
}

fn light_json(light: &Light, name: &str) -> Value {
    let color = light.color.to_array();
    match light.kind {
        LightKind::Sun => json!({ "name": name, "type": "directional", "color": color, "intensity": light.power * SUN_LUX_PER_WATT }),
        LightKind::Spot => {
            let outer = light.spot_size / 2.0;
            json!({ "name": name, "type": "spot", "color": color, "intensity": light.power * POINT_CANDELA_PER_WATT, "spot": { "outerConeAngle": outer, "innerConeAngle": outer * (1.0 - SPOT_BLEND) } })
        }
        // An area light has no glTF counterpart; it goes out as a point light of its power.
        LightKind::Point | LightKind::Area => {
            json!({ "name": name, "type": "point", "color": color, "intensity": light.power * POINT_CANDELA_PER_WATT })
        }
    }
}

/// Splits a matrix into glTF TRS, or keeps the matrix when that would lose shear.
fn node_transform(node: &mut Map<String, Value>, local: Mat4) {
    let (scale, rotation, translation) = local.to_scale_rotation_translation();
    let rebuilt = Mat4::from_scale_rotation_translation(scale, rotation, translation);
    if !rebuilt.abs_diff_eq(local, 1e-4) {
        node.insert("matrix".into(), json!(local.to_cols_array()));
        return;
    }
    if translation != Vec3::ZERO {
        node.insert("translation".into(), json!(translation.to_array()));
    }
    if !rotation.abs_diff_eq(Quat::IDENTITY, 1e-7) {
        node.insert("rotation".into(), json!(rotation.normalize().to_array()));
    }
    if !scale.abs_diff_eq(Vec3::ONE, 1e-7) {
        node.insert("scale".into(), json!(scale.to_array()));
    }
}

pub fn export(scene: &Scene, only: Option<&[Id]>, binary: bool) -> Result<Vec<u8>, SceneError> {
    let ids = chosen(scene, only, true)?;
    let requested: Option<BTreeSet<Id>> = only.map(|ids| ids.iter().copied().collect());
    let index_of: HashMap<Id, usize> = ids.iter().enumerate().map(|(index, id)| (*id, index)).collect();
    let mut writer = Writer { bin: Vec::new(), views: Vec::new(), accessors: Vec::new() };
    let mut nodes: Vec<Value> = Vec::with_capacity(ids.len());
    let mut meshes = Vec::new();
    let mut cameras = Vec::new();
    let mut lights = Vec::new();
    let mut materials: Vec<Value> = Vec::new();
    let mut material_index: HashMap<Id, usize> = HashMap::new();
    let mut textures = Textures::default();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); ids.len()];
    let mut roots = Vec::new();
    let mut skinned = Vec::new();
    let aspect = scene.render.width.max(1) as f32 / scene.render.height.max(1) as f32;
    for (index, id) in ids.iter().enumerate() {
        let object = scene.object(*id)?;
        let oriented = is_oriented(&object.data);
        let world = to_gltf_world(scene.world_matrix(*id)?, oriented);
        // The nearest exported ancestor is the node's parent.
        let mut parent = object.parent;
        while let Some(at) = parent {
            if index_of.contains_key(&at) {
                break;
            }
            parent = scene.object(at)?.parent;
        }
        let local = match parent {
            Some(parent) => {
                let parent_object = scene.object(parent)?;
                to_gltf_world(scene.world_matrix(parent)?, is_oriented(&parent_object.data)).inverse() * world
            }
            None => world,
        };
        match parent.and_then(|parent| index_of.get(&parent)) {
            Some(parent) => {
                if let Some(list) = children.get_mut(*parent) {
                    list.push(index);
                }
            }
            None => roots.push(index),
        }
        let mut node = Map::new();
        node.insert("name".into(), json!(object.name));
        node_transform(&mut node, local);
        // Ancestors pulled in only to keep the hierarchy are plain nodes; so are hidden objects.
        let content = !object.hidden && requested.as_ref().is_none_or(|set| set.contains(id));
        if content {
            match &object.data {
                // Curves, texts and surfaces go out as the meshes they make.
                ObjectData::Mesh(_) | ObjectData::Curve(_) | ObjectData::Text(_) | ObjectData::Surface(_) => {
                    // A mesh deformed only by exported armatures (and shape keys) goes out at rest,
                    // with its joints, weights, and shape keys; any other is baked.
                    let plan = skin::plan(scene, object, &index_of);
                    let mesh = match &plan {
                        Some(plan) => plan.mesh.clone(),
                        None => object.evaluated_mesh(scene).unwrap_or_default(),
                    };
                    let material = match object.material {
                        Some(material_id) => match material_index.get(&material_id) {
                            Some(found) => Some(*found),
                            None => {
                                let material = scene.material(material_id)?;
                                materials.push(material_json(material, scene, &mut writer, &mut textures));
                                material_index.insert(material_id, materials.len() - 1);
                                Some(materials.len() - 1)
                            }
                        },
                        None => None,
                    };
                    if !mesh.faces.is_empty() || !mesh.loose_edges.is_empty() {
                        meshes.push(mesh_json(&mut writer, &mesh, &object.name, material, plan.as_ref().map(|plan| &plan.attributes)));
                        node.insert("mesh".into(), json!(meshes.len() - 1));
                        if let Some(plan) = plan {
                            skinned.push((index, *id, plan));
                        }
                    }
                }
                ObjectData::Camera(camera) => {
                    cameras.push(camera_json(camera, &object.name, aspect));
                    node.insert("camera".into(), json!(cameras.len() - 1));
                }
                ObjectData::Light(light) => {
                    lights.push(light_json(light, &object.name));
                    node.insert("extensions".into(), json!({ "KHR_lights_punctual": { "light": lights.len() - 1 } }));
                }
                // Armatures are written as joint nodes with the skins that use them.
                ObjectData::Empty { .. } | ObjectData::Armature(_) | ObjectData::Lattice(_) => {}
            }
        }
        nodes.push(Value::Object(node));
    }
    for (node, list) in nodes.iter_mut().zip(children) {
        if let (Value::Object(map), false) = (node, list.is_empty()) {
            map.insert("children".into(), json!(list));
        }
    }
    // Bones become joint nodes under their armatures' nodes, and skinned meshes get skins.
    let rigs = skin::export_joints(scene, &ids, &mut nodes, &skinned, &mut writer)?;
    let mut root = Map::new();
    root.insert("asset".into(), json!({ "version": "2.0", "generator": "Kition Scene" }));
    root.insert("scene".into(), json!(0));
    root.insert("scenes".into(), json!([{ "name": "Scene", "nodes": roots }]));
    root.insert("nodes".into(), Value::Array(nodes));
    let mut used = Vec::new();
    if !meshes.is_empty() {
        root.insert("meshes".into(), Value::Array(meshes));
    }
    if !materials.is_empty() {
        for extension in ["KHR_materials_emissive_strength", "KHR_materials_transmission", "KHR_materials_ior"] {
            if materials.iter().any(|material| material.pointer(&format!("/extensions/{extension}")).is_some()) {
                used.push(extension);
            }
        }
        root.insert("materials".into(), Value::Array(materials));
    }
    if !textures.images.is_empty() {
        root.insert("images".into(), Value::Array(std::mem::take(&mut textures.images)));
        root.insert("textures".into(), Value::Array(std::mem::take(&mut textures.textures)));
        // Linear filtering with mipmaps, repeating: what a texture in the scene does.
        root.insert("samplers".into(), json!([{ "magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497 }]));
    }
    if !cameras.is_empty() {
        root.insert("cameras".into(), Value::Array(cameras));
    }
    if !lights.is_empty() {
        used.push("KHR_lights_punctual");
        root.insert("extensions".into(), json!({ "KHR_lights_punctual": { "lights": lights } }));
    }
    if !used.is_empty() {
        root.insert("extensionsUsed".into(), json!(used));
    }
    if !rigs.skins.is_empty() {
        root.insert("skins".into(), Value::Array(rigs.skins.clone()));
    }
    let objects = anim::export(scene, &ids, &index_of, &mut writer)?;
    let poses = skin::export_animation(scene, &rigs, &mut writer)?;
    if let Some(animations) = skin::merge_animations(objects, poses) {
        root.insert("animations".into(), animations);
    }
    while !writer.bin.len().is_multiple_of(4) {
        writer.bin.push(0);
    }
    if !writer.bin.is_empty() {
        let mut buffer = json!({ "byteLength": writer.bin.len() });
        if !binary && let Value::Object(map) = &mut buffer {
            map.insert("uri".into(), json!(format!("data:application/octet-stream;base64,{}", base64::encode(&writer.bin))));
        }
        root.insert("buffers".into(), json!([buffer]));
        root.insert("bufferViews".into(), Value::Array(std::mem::take(&mut writer.views)));
        root.insert("accessors".into(), Value::Array(std::mem::take(&mut writer.accessors)));
    }
    let json = serde_json::to_vec(&Value::Object(root)).map_err(|error| invalid(error.to_string()))?;
    if !binary {
        return Ok(json);
    }
    let mut json = json;
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let bin_chunk = if writer.bin.is_empty() { 0 } else { 8 + writer.bin.len() };
    let total = 12 + 8 + json.len() + bin_chunk;
    let length = |value: usize| u32::try_from(value).map_err(|_| invalid("the scene is too large for a .glb file"));
    let mut out = Vec::with_capacity(total);
    out.extend(GLB_MAGIC);
    out.extend(2u32.to_le_bytes());
    out.extend(length(total)?.to_le_bytes());
    out.extend(length(json.len())?.to_le_bytes());
    out.extend(CHUNK_JSON.to_le_bytes());
    out.extend(&json);
    if !writer.bin.is_empty() {
        out.extend(length(writer.bin.len())?.to_le_bytes());
        out.extend(CHUNK_BIN.to_le_bytes());
        out.extend(&writer.bin);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Import

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([*slice.first()?, *slice.get(1)?, *slice.get(2)?, *slice.get(3)?]))
}

/// The JSON and the binary chunk of a `.glb`, or the JSON of a `.gltf`.
fn split(bytes: &[u8]) -> Result<(Value, Option<&[u8]>), SceneError> {
    if bytes.starts_with(GLB_MAGIC) {
        let version = read_u32(bytes, 4).ok_or_else(|| invalid("the .glb header is cut short"))?;
        if version != 2 {
            return Err(invalid(format!("glTF version {version} is not supported")));
        }
        let total = read_u32(bytes, 8).ok_or_else(|| invalid("the .glb header is cut short"))? as usize;
        let bytes = bytes.get(..total).ok_or_else(|| invalid("the .glb file is cut short"))?;
        let mut at = 12usize;
        let mut json = None;
        let mut bin = None;
        while at < bytes.len() {
            let length = read_u32(bytes, at).ok_or_else(|| invalid("a .glb chunk header is cut short"))? as usize;
            let kind = read_u32(bytes, at + 4).ok_or_else(|| invalid("a .glb chunk header is cut short"))?;
            let start = at + 8;
            let end = start.checked_add(length).ok_or_else(|| invalid("a .glb chunk is too long"))?;
            let data = bytes.get(start..end).ok_or_else(|| invalid("a .glb chunk is cut short"))?;
            match kind {
                CHUNK_JSON if json.is_none() => json = Some(data),
                CHUNK_BIN if bin.is_none() => bin = Some(data),
                _ => {}
            }
            at = end;
        }
        let json = json.ok_or_else(|| invalid("the .glb file has no JSON chunk"))?;
        let value = serde_json::from_slice(json).map_err(|error| invalid(format!("the glTF JSON is not valid: {error}")))?;
        return Ok((value, bin));
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| invalid("not a glTF file"))?;
    Ok((value, None))
}

fn usize_of(value: &Value, key: &str) -> Option<usize> {
    value.get(key).and_then(Value::as_u64).and_then(|number| usize::try_from(number).ok())
}

fn f32_of(value: &Value, key: &str) -> Option<f32> {
    value.get(key).and_then(Value::as_f64).map(|number| number as f32).filter(|number| number.is_finite())
}

fn floats(value: Option<&Value>, count: usize) -> Option<Vec<f32>> {
    let array = value?.as_array()?;
    if array.len() != count {
        return None;
    }
    array.iter().map(|item| item.as_f64().map(|number| number as f32).filter(|number| number.is_finite())).collect()
}

/// An accessor's bytes from its first element, and how to step through them.
struct View<'a> {
    data: &'a [u8],
    stride: usize,
    count: usize,
    component: u64,
    components: usize,
}

struct Document<'a> {
    json: &'a Value,
    buffers: Vec<Vec<u8>>,
}

impl Document<'_> {
    fn list(&self, key: &str) -> &[Value] {
        self.json.get(key).and_then(Value::as_array).map_or(&[], Vec::as_slice)
    }

    fn item(&self, key: &str, index: usize) -> Result<&Value, SceneError> {
        self.list(key).get(index).ok_or_else(|| invalid(format!("{key}[{index}] is not there")))
    }

    /// An accessor's elements as raw bytes.
    fn accessor(&self, index: usize) -> Result<View<'_>, SceneError> {
        let accessor = self.item("accessors", index)?;
        if accessor.get("sparse").is_some() {
            return Err(invalid("sparse accessors are not supported"));
        }
        let count = usize_of(accessor, "count").ok_or_else(|| invalid(format!("accessor {index} has no count")))?;
        if count > MAX_ITEMS {
            return Err(invalid(format!("accessor {index} is too large")));
        }
        let component =
            accessor.get("componentType").and_then(Value::as_u64).ok_or_else(|| invalid(format!("accessor {index} has no component type")))?;
        let components = match accessor.get("type").and_then(Value::as_str) {
            Some("SCALAR") => 1,
            Some("VEC2") => 2,
            Some("VEC3") => 3,
            Some("VEC4") => 4,
            Some("MAT4") => 16,
            _ => return Err(invalid(format!("accessor {index} has an unsupported type"))),
        };
        let component_size = match component {
            UNSIGNED_BYTE | 5120 => 1,
            UNSIGNED_SHORT | 5122 => 2,
            UNSIGNED_INT | FLOAT => 4,
            _ => return Err(invalid(format!("accessor {index} has an unknown component type"))),
        };
        let element = component_size * components;
        let view_index = usize_of(accessor, "bufferView").ok_or_else(|| invalid(format!("accessor {index} has no buffer view")))?;
        let view = self.item("bufferViews", view_index)?;
        let buffer = self
            .buffers
            .get(usize_of(view, "buffer").unwrap_or(0))
            .ok_or_else(|| invalid(format!("buffer view {view_index} names a buffer that is not there")))?;
        let view_offset = usize_of(view, "byteOffset").unwrap_or(0);
        let view_length = usize_of(view, "byteLength").ok_or_else(|| invalid(format!("buffer view {view_index} has no length")))?;
        let view_end = view_offset.checked_add(view_length).ok_or_else(|| invalid("a buffer view is out of range"))?;
        let data = buffer.get(view_offset..view_end).ok_or_else(|| invalid(format!("buffer view {view_index} runs past its buffer")))?;
        let stride = usize_of(view, "byteStride").unwrap_or(element).max(element);
        let offset = usize_of(accessor, "byteOffset").unwrap_or(0);
        if count > 0 {
            let last = stride.checked_mul(count - 1).and_then(|value| value.checked_add(offset)).and_then(|value| value.checked_add(element));
            if last.is_none_or(|end| end > data.len()) {
                return Err(invalid(format!("accessor {index} runs past its buffer view")));
            }
        }
        let data = data.get(offset..).ok_or_else(|| invalid(format!("accessor {index} runs past its buffer view")))?;
        Ok(View { data, stride, count, component, components })
    }

    fn vec3s(&self, index: usize) -> Result<Vec<Vec3>, SceneError> {
        let View { data, stride, count, component, components } = self.accessor(index)?;
        if component != FLOAT || components != 3 {
            return Err(invalid(format!("accessor {index} is not a float VEC3")));
        }
        let mut out = Vec::with_capacity(count);
        for element in 0..count {
            let at = element * stride;
            let value = |slot: usize| read_u32(data, at + slot * 4).map(f32::from_bits).filter(|value| value.is_finite());
            match (value(0), value(1), value(2)) {
                (Some(x), Some(y), Some(z)) => out.push(Vec3::new(x, y, z)),
                _ => return Err(invalid(format!("accessor {index} holds a value that is not a finite number"))),
            }
        }
        Ok(out)
    }

    /// Texture coordinates: floats, or normalised unsigned bytes or shorts.
    fn vec2s(&self, index: usize) -> Result<Vec<Vec2>, SceneError> {
        let View { data, stride, count, component, components } = self.accessor(index)?;
        if components != 2 {
            return Err(invalid(format!("accessor {index} is not a VEC2")));
        }
        let mut out = Vec::with_capacity(count);
        for element in 0..count {
            let at = element * stride;
            let value = |slot: usize| -> Option<f32> {
                match component {
                    FLOAT => read_u32(data, at + slot * 4).map(f32::from_bits).filter(|value| value.is_finite()),
                    UNSIGNED_BYTE => data.get(at + slot).map(|byte| f32::from(*byte) / 255.0),
                    UNSIGNED_SHORT => data
                        .get(at + slot * 2..at + slot * 2 + 2)
                        .and_then(|bytes| Some(f32::from(u16::from_le_bytes([*bytes.first()?, *bytes.get(1)?])) / 65535.0)),
                    _ => None,
                }
            };
            match (value(0), value(1)) {
                (Some(u), Some(v)) => out.push(Vec2::new(u, v)),
                _ => return Err(invalid(format!("accessor {index} holds a texture coordinate that cannot be read"))),
            }
        }
        Ok(out)
    }

    /// A buffer view's bytes.
    fn view_bytes(&self, view_index: usize) -> Result<&[u8], SceneError> {
        let view = self.item("bufferViews", view_index)?;
        let buffer = self
            .buffers
            .get(usize_of(view, "buffer").unwrap_or(0))
            .ok_or_else(|| invalid(format!("buffer view {view_index} names a buffer that is not there")))?;
        let offset = usize_of(view, "byteOffset").unwrap_or(0);
        let length = usize_of(view, "byteLength").ok_or_else(|| invalid(format!("buffer view {view_index} has no length")))?;
        let end = offset.checked_add(length).ok_or_else(|| invalid("a buffer view is out of range"))?;
        buffer.get(offset..end).ok_or_else(|| invalid(format!("buffer view {view_index} runs past its buffer")))
    }

    /// The image texture `index` shows, decoded from its buffer view or data URI. Images in
    /// separate files are refused with their name; formats other than PNG and JPEG are skipped.
    fn texture_image(&self, index: usize) -> Result<Option<Image>, SceneError> {
        let texture = self.item("textures", index)?;
        let Some(source) = usize_of(texture, "source") else { return Ok(None) };
        let image = self.item("images", source)?;
        let name = image.get("name").and_then(Value::as_str).map_or_else(|| format!("Image {source}"), str::to_string);
        let bytes = if let Some(view) = usize_of(image, "bufferView") {
            self.view_bytes(view)?.to_vec()
        } else if let Some(uri) = image.get("uri").and_then(Value::as_str) {
            match uri.strip_prefix("data:").and_then(|rest| rest.split_once(";base64,")) {
                Some((_, data)) => base64::decode(data).ok_or_else(|| invalid(format!("image {source}'s data URI is not base64")))?,
                None => return Err(invalid(format!("the image {uri} is a separate file; use a .glb or a .gltf with embedded images"))),
            }
        } else {
            return Ok(None);
        };
        if !matches!(
            kition_scene_model::ImageFormat::sniff(&bytes),
            Some(kition_scene_model::ImageFormat::Png | kition_scene_model::ImageFormat::Jpeg)
        ) || bytes.len() > kition_scene_model::scene::MAX_IMAGE_BYTES
        {
            return Ok(None);
        }
        Ok(Image::new(name, bytes).ok())
    }

    fn indices(&self, index: usize) -> Result<Vec<u32>, SceneError> {
        let View { data, stride, count, component, components } = self.accessor(index)?;
        if components != 1 {
            return Err(invalid(format!("index accessor {index} is not scalar")));
        }
        let mut out = Vec::with_capacity(count);
        for element in 0..count {
            let at = element * stride;
            let value = match component {
                UNSIGNED_BYTE => data.get(at).map(|byte| u32::from(*byte)),
                UNSIGNED_SHORT => data.get(at..at + 2).and_then(|bytes| Some(u32::from(u16::from_le_bytes([*bytes.first()?, *bytes.get(1)?])))),
                UNSIGNED_INT => read_u32(data, at),
                _ => return Err(invalid(format!("index accessor {index} has an unsupported component type"))),
            };
            out.push(value.ok_or_else(|| invalid(format!("index accessor {index} is cut short")))?);
        }
        Ok(out)
    }
}

fn load_buffers(json: &Value, bin: Option<&[u8]>) -> Result<Vec<Vec<u8>>, SceneError> {
    let mut out = Vec::new();
    for (index, buffer) in json.get("buffers").and_then(Value::as_array).map_or(&[][..], Vec::as_slice).iter().enumerate() {
        let length = usize_of(buffer, "byteLength").ok_or_else(|| invalid(format!("buffer {index} has no length")))?;
        let bytes = match buffer.get("uri").and_then(Value::as_str) {
            Some(uri) if uri.starts_with("data:") => {
                let (_, data) = uri.split_once(";base64,").ok_or_else(|| invalid(format!("buffer {index}'s data URI is not base64")))?;
                base64::decode(data).ok_or_else(|| invalid(format!("buffer {index}'s data URI is not valid base64")))?
            }
            Some(uri) => return Err(invalid(format!("the glTF file needs the external file `{uri}`; use a .glb or a .gltf with embedded buffers"))),
            None if index == 0 => bin.ok_or_else(|| invalid("the glTF file has no binary chunk"))?.to_vec(),
            None => return Err(invalid(format!("buffer {index} has no data"))),
        };
        if bytes.len() < length {
            return Err(invalid(format!("buffer {index} is shorter than its length")));
        }
        out.push(bytes);
    }
    Ok(out)
}

fn node_matrix(node: &Value) -> Result<Mat4, SceneError> {
    if let Some(matrix) = node.get("matrix") {
        let values = floats(Some(matrix), 16).ok_or_else(|| invalid("a node matrix is not 16 numbers"))?;
        let mut array = [0.0f32; 16];
        for (slot, value) in array.iter_mut().zip(values) {
            *slot = value;
        }
        return Ok(Mat4::from_cols_array(&array));
    }
    let translation =
        node.get("translation").map(|value| floats(Some(value), 3).ok_or_else(|| invalid("a node translation is not 3 numbers"))).transpose()?;
    let rotation = node.get("rotation").map(|value| floats(Some(value), 4).ok_or_else(|| invalid("a node rotation is not 4 numbers"))).transpose()?;
    let scale = node.get("scale").map(|value| floats(Some(value), 3).ok_or_else(|| invalid("a node scale is not 3 numbers"))).transpose()?;
    let vec3 = |values: Option<Vec<f32>>, default: Vec3| {
        values.map_or(default, |values| {
            Vec3::new(values.first().copied().unwrap_or(0.0), values.get(1).copied().unwrap_or(0.0), values.get(2).copied().unwrap_or(0.0))
        })
    };
    let rotation = rotation.map_or(Quat::IDENTITY, |values| {
        Quat::from_xyzw(
            values.first().copied().unwrap_or(0.0),
            values.get(1).copied().unwrap_or(0.0),
            values.get(2).copied().unwrap_or(0.0),
            values.get(3).copied().unwrap_or(1.0),
        )
        .normalize()
    });
    Ok(Mat4::from_scale_rotation_translation(
        vec3(scale, Vec3::ONE),
        if rotation.is_finite() { rotation } else { Quat::IDENTITY },
        vec3(translation, Vec3::ZERO),
    ))
}

fn import_material(material: &Value, index: usize) -> Material {
    let pbr = material.get("pbrMetallicRoughness");
    let color = floats(pbr.and_then(|pbr| pbr.get("baseColorFactor")), 4).unwrap_or_else(|| vec![1.0; 4]);
    let mut out = Material::new(0, material.get("name").and_then(Value::as_str).map_or_else(|| format!("Material {index}"), str::to_string));
    out.base_color = [
        color.first().copied().unwrap_or(1.0),
        color.get(1).copied().unwrap_or(1.0),
        color.get(2).copied().unwrap_or(1.0),
        color.get(3).copied().unwrap_or(1.0),
    ]
    .map(|value| value.clamp(0.0, 1.0));
    out.metallic = pbr.and_then(|pbr| f32_of(pbr, "metallicFactor")).unwrap_or(1.0).clamp(0.0, 1.0);
    out.roughness = pbr.and_then(|pbr| f32_of(pbr, "roughnessFactor")).unwrap_or(1.0).clamp(0.0, 1.0);
    if let Some(emissive) = floats(material.get("emissiveFactor"), 3) {
        let emission =
            Vec3::new(emissive.first().copied().unwrap_or(0.0), emissive.get(1).copied().unwrap_or(0.0), emissive.get(2).copied().unwrap_or(0.0))
                .clamp(Vec3::ZERO, Vec3::ONE);
        if emission.max_element() > 0.0 {
            out.emission = emission;
            out.emission_strength = material
                .pointer("/extensions/KHR_materials_emissive_strength")
                .and_then(|value| f32_of(value, "emissiveStrength"))
                .unwrap_or(1.0)
                .max(0.0);
        }
    }
    out.alpha_mode = match material.get("alphaMode").and_then(Value::as_str) {
        Some("MASK") => kition_scene_model::AlphaMode::Clip,
        Some("BLEND") => kition_scene_model::AlphaMode::Blend,
        _ => kition_scene_model::AlphaMode::Opaque,
    };
    out.alpha_cutoff = f32_of(material, "alphaCutoff").filter(|value| value.is_finite()).unwrap_or(0.5).clamp(0.0, 1.0);
    if let Some(transmission) = material.pointer("/extensions/KHR_materials_transmission").and_then(|value| f32_of(value, "transmissionFactor")) {
        out.transmission = transmission.clamp(0.0, 1.0);
    }
    if let Some(ior) = material.pointer("/extensions/KHR_materials_ior").and_then(|value| f32_of(value, "ior")) {
        // glTF allows 0 for "no refraction"; keep a physical index.
        out.ior = if ior.is_finite() && ior >= 1.0 { ior.min(10.0) } else { 1.5 };
    }
    out
}

fn import_camera(camera: &Value) -> Camera {
    let mut out = Camera::default();
    if let Some(perspective) = camera.get("perspective") {
        // Preserve near-PI angles before taking tan: f32 can lose several percent of
        // the authored frame for a legal wide portrait camera.
        let yfov = perspective.get("yfov").and_then(Value::as_f64).filter(|value| *value > 0.0 && *value < std::f64::consts::PI).unwrap_or(0.4);
        let aspect = perspective.get("aspectRatio").and_then(Value::as_f64).filter(|value| value.is_finite() && *value > 0.0).unwrap_or(16.0 / 9.0);
        let half_width = if aspect >= 1.0 { (yfov / 2.0).tan() * aspect } else { (yfov / 2.0).tan() };
        let lens = (f64::from(out.sensor) / (2.0 * half_width)) as f32;
        if lens.is_finite() && lens > 0.0 {
            out.lens = lens;
        }
        out.clip_start = f32_of(perspective, "znear").filter(|value| *value > 0.0).unwrap_or(0.1);
        out.clip_end = f32_of(perspective, "zfar").filter(|value| *value > out.clip_start).unwrap_or(1000.0);
    } else if let Some(orthographic) = camera.get("orthographic") {
        out.orthographic = true;
        let xmag = f32_of(orthographic, "xmag").unwrap_or(1.0).abs();
        let ymag = f32_of(orthographic, "ymag").unwrap_or(1.0).abs();
        out.ortho_scale = (2.0 * xmag.max(ymag)).max(1e-3);
        out.clip_start = f32_of(orthographic, "znear").filter(|value| *value >= 0.0).unwrap_or(0.1);
        out.clip_end = f32_of(orthographic, "zfar").filter(|value| *value > out.clip_start).unwrap_or(1000.0);
    }
    out
}

fn import_light(light: &Value) -> Light {
    let kind = match light.get("type").and_then(Value::as_str) {
        Some("directional") => LightKind::Sun,
        Some("spot") => LightKind::Spot,
        _ => LightKind::Point,
    };
    let mut out = Light::new(kind);
    if let Some(color) = floats(light.get("color"), 3) {
        out.color = Vec3::new(color.first().copied().unwrap_or(1.0), color.get(1).copied().unwrap_or(1.0), color.get(2).copied().unwrap_or(1.0))
            .max(Vec3::ZERO);
    }
    let intensity = f32_of(light, "intensity").unwrap_or(1.0).max(0.0);
    out.power = match kind {
        LightKind::Sun => intensity / SUN_LUX_PER_WATT,
        _ => intensity / POINT_CANDELA_PER_WATT,
    };
    if kind == LightKind::Spot {
        let outer = light.get("spot").and_then(|spot| f32_of(spot, "outerConeAngle")).unwrap_or(std::f32::consts::FRAC_PI_4);
        out.spot_size = (outer * 2.0).clamp(0.01, std::f32::consts::PI);
    }
    out
}

/// A glTF mesh as one polygon mesh in scene space, and the material of its first primitive.
fn import_mesh(document: &Document<'_>, index: usize) -> Result<(Mesh, Option<usize>), SceneError> {
    let mesh = document.item("meshes", index)?;
    let mut positions: Vec<Vec3> = Vec::new();
    let mut normals: Vec<Option<Vec3>> = Vec::new();
    let mut coordinates: Vec<Option<Vec2>> = Vec::new();
    let mut triangles: Vec<[u32; 3]> = Vec::new();
    let mut lines: Vec<[u32; 2]> = Vec::new();
    let mut material = None;
    // Joints, weights, and morph offsets per file vertex.
    let mut vertex_extras = skin::Extras::default();
    for primitive in mesh.get("primitives").and_then(Value::as_array).map_or(&[][..], Vec::as_slice) {
        let mode = primitive.get("mode").and_then(Value::as_u64).unwrap_or(4);
        if mode != 4 && mode != 1 {
            continue;
        }
        let attributes = primitive.get("attributes").ok_or_else(|| invalid(format!("a primitive of mesh {index} has no attributes")))?;
        let Some(position) = usize_of(attributes, "POSITION") else { continue };
        let points = document.vec3s(position)?;
        let point_normals = match usize_of(attributes, "NORMAL") {
            Some(normal) => {
                let values = document.vec3s(normal)?;
                if values.len() != points.len() {
                    return Err(invalid(format!("a primitive of mesh {index} has a NORMAL count unlike its POSITION count")));
                }
                values.into_iter().map(Some).collect()
            }
            None => vec![None; points.len()],
        };
        let point_coordinates: Vec<Option<Vec2>> = match usize_of(attributes, "TEXCOORD_0") {
            Some(texcoord) => {
                let values = document.vec2s(texcoord)?;
                if values.len() != points.len() {
                    return Err(invalid(format!("a primitive of mesh {index} has a TEXCOORD_0 count unlike its POSITION count")));
                }
                values.into_iter().map(|uv| Some(Vec2::new(uv.x, 1.0 - uv.y))).collect()
            }
            None => vec![None; points.len()],
        };
        let base = u32::try_from(positions.len()).map_err(|_| invalid("the mesh is too large"))?;
        let count = u32::try_from(points.len()).map_err(|_| invalid("the mesh is too large"))?;
        let indices = match usize_of(primitive, "indices") {
            Some(accessor) => document.indices(accessor)?,
            None => (0..count).collect(),
        };
        if let Some(bad) = indices.iter().find(|value| **value >= count) {
            return Err(invalid(format!("a primitive of mesh {index} names vertex {bad}, past its {count}")));
        }
        let primitive_extras = skin::read_extras(document, primitive, points.len())?;
        if primitive_extras.joints.is_empty() {
            vertex_extras.joints.extend(std::iter::repeat_n(None, points.len()));
        } else {
            vertex_extras.joints.extend(primitive_extras.joints);
        }
        let before = positions.len();
        while vertex_extras.targets.len() < primitive_extras.targets.len() {
            vertex_extras.targets.push(vec![Vec3::ZERO; before]);
        }
        for (slot, target) in vertex_extras.targets.iter_mut().enumerate() {
            match primitive_extras.targets.get(slot) {
                Some(offsets) => target.extend(offsets),
                None => target.extend(std::iter::repeat_n(Vec3::ZERO, points.len())),
            }
        }
        positions.extend(points.into_iter().map(from_y_up));
        normals.extend(point_normals.into_iter().map(|normal| normal.map(from_y_up)));
        coordinates.extend(point_coordinates);
        if positions.len() > MAX_ITEMS {
            return Err(invalid("the mesh is too large"));
        }
        if mode == 4 {
            if material.is_none() {
                material = usize_of(primitive, "material");
            }
            for [a, b, c] in indices.as_chunks::<3>().0 {
                triangles.push([base + a, base + b, base + c]);
            }
        } else {
            for [a, b] in indices.as_chunks::<2>().0 {
                lines.push([base + a, base + b]);
            }
        }
    }
    let (welded, remap) = weld_exact(&positions);
    let map = |index: u32| remap.get(index as usize).copied().unwrap_or(u32::MAX);
    // Faces split by normal mean flat shading.
    let mut seen: HashMap<u32, Vec3> = HashMap::new();
    let mut any_normal = false;
    let mut flat = false;
    for (index, normal) in normals.iter().enumerate() {
        if let Some(normal) = normal {
            any_normal = true;
            let target = map(u32::try_from(index).unwrap_or(u32::MAX));
            match seen.get(&target) {
                Some(other) if !other.abs_diff_eq(*normal, 1e-4) => flat = true,
                Some(_) => {}
                None => {
                    seen.insert(target, *normal);
                }
            }
        }
    }
    let extras = mesh.pointer("/extras/kition");
    let smooth = extras.and_then(|extras| extras.get("smooth")).and_then(Value::as_bool).unwrap_or(any_normal && !flat);
    let welded_triangles: Vec<[u32; 3]> = triangles.iter().map(|triangle| triangle.map(map)).collect();
    let face_triangles: Option<Vec<usize>> = extras
        .and_then(|extras| extras.get("faceTriangles"))
        .and_then(Value::as_array)
        .and_then(|counts| counts.iter().map(|count| count.as_u64().and_then(|count| usize::try_from(count).ok())).collect())
        .filter(|counts: &Vec<usize>| counts.iter().try_fold(0usize, |sum, count| sum.checked_add(*count)) == Some(triangles.len()));
    // Each face corner's texture coordinate comes from the file vertex it was welded from.
    let has_uvs = coordinates.iter().any(Option::is_some);
    let uv_of = |raw: u32| coordinates.get(raw as usize).copied().flatten().unwrap_or(Vec2::ZERO);
    let mut faces: Vec<Vec<u32>> = Vec::new();
    let mut uvs: Vec<Vec<Vec2>> = Vec::new();
    let mut push = |face: Vec<u32>, raw: &[[u32; 3]]| {
        if has_uvs {
            let corner_uvs = face
                .iter()
                .map(|welded| raw.iter().flatten().find(|original| map(**original) == *welded).map_or(Vec2::ZERO, |original| uv_of(*original)))
                .collect();
            uvs.push(corner_uvs);
        }
        faces.push(face);
    };
    let usable = |[a, b, c]: &[u32; 3]| a != b && b != c && a != c && *a != u32::MAX && *b != u32::MAX && *c != u32::MAX;
    match face_triangles {
        Some(counts) => {
            let mut at = 0;
            for count in counts {
                let group = welded_triangles.get(at..at + count).unwrap_or(&[]);
                let raw = triangles.get(at..at + count).unwrap_or(&[]);
                at += count;
                match polygon_of(group) {
                    Some(polygon) => push(polygon, raw),
                    None => {
                        for (triangle, original) in group.iter().zip(raw) {
                            if usable(triangle) {
                                push(triangle.to_vec(), std::slice::from_ref(original));
                            }
                        }
                    }
                }
            }
        }
        None => {
            for (triangle, original) in welded_triangles.iter().zip(&triangles) {
                if usable(triangle) {
                    push(triangle.to_vec(), std::slice::from_ref(original));
                }
            }
        }
    }
    let loose_edges = lines.iter().map(|edge| edge.map(map)).filter(|[a, b]| a != b).collect();
    let names: Vec<String> = mesh
        .pointer("/extras/targetNames")
        .and_then(Value::as_array)
        .map(|names| names.iter().map(|name| name.as_str().unwrap_or_default().to_string()).collect())
        .unwrap_or_default();
    let weights: Vec<f32> = mesh
        .get("weights")
        .and_then(Value::as_array)
        .map(|values| values.iter().map(|value| value.as_f64().unwrap_or(0.0) as f32).collect())
        .unwrap_or_default();
    let (groups, shape_keys) = skin::welded_extras(&vertex_extras, &remap, welded.len(), &names, &weights);
    let mut out = Mesh { positions: welded, faces, loose_edges, smooth, uvs, groups, shape_keys, ..Default::default() };
    // Unused vertices go; groups and shape keys follow by position.
    let data = kition_scene_model::vertex_data::snapshot(&out);
    out.remove_unused_vertices();
    kition_scene_model::vertex_data::carry(data.as_ref(), &mut out);
    Ok((out, material))
}

/// The outline of a triangulated polygon: its boundary edges chained from the first corner.
fn polygon_of(triangles: &[[u32; 3]]) -> Option<Vec<u32>> {
    let first = triangles.first()?;
    if triangles.len() == 1 {
        let [a, b, c] = *first;
        return (a != b && b != c && a != c).then(|| vec![a, b, c]);
    }
    let mut directed: BTreeSet<(u32, u32)> = BTreeSet::new();
    let mut order = Vec::new();
    for [a, b, c] in triangles {
        for edge in [(*a, *b), (*b, *c), (*c, *a)] {
            if !directed.insert(edge) {
                return None;
            }
            order.push(edge);
        }
    }
    let boundary: Vec<(u32, u32)> = order.into_iter().filter(|(a, b)| !directed.contains(&(*b, *a))).collect();
    let next: HashMap<u32, u32> = boundary.iter().copied().collect();
    if next.len() != boundary.len() {
        return None;
    }
    let (start, _) = *boundary.first()?;
    let mut polygon = vec![start];
    let mut current = *next.get(&start)?;
    while current != start {
        if polygon.len() > boundary.len() {
            return None;
        }
        polygon.push(current);
        current = *next.get(&current)?;
    }
    (polygon.len() == boundary.len() && polygon.len() >= 3).then_some(polygon)
}

pub fn import(bytes: &[u8]) -> Result<Imported, SceneError> {
    let (json, bin) = split(bytes)?;
    if !json.is_object() {
        return Err(invalid("not a glTF file"));
    }
    let version = json.pointer("/asset/version").and_then(Value::as_str).unwrap_or("");
    if !version.starts_with('2') {
        return Err(invalid("not a glTF 2.0 file"));
    }
    let document = Document { buffers: load_buffers(&json, bin)?, json: &json };
    let nodes = document.list("nodes");
    if nodes.len() > MAX_ITEMS {
        return Err(invalid("the file has too many nodes"));
    }
    let roots: Vec<usize> = match json.get("scenes").and_then(Value::as_array).and_then(|scenes| scenes.get(usize_of(&json, "scene").unwrap_or(0))) {
        Some(scene) => scene
            .get("nodes")
            .and_then(Value::as_array)
            .map_or_else(Vec::new, |list| list.iter().filter_map(|value| value.as_u64().and_then(|value| usize::try_from(value).ok())).collect()),
        None => {
            let children: BTreeSet<usize> = nodes
                .iter()
                .flat_map(|node| {
                    node.get("children")
                        .and_then(Value::as_array)
                        .map_or(&[][..], Vec::as_slice)
                        .iter()
                        .filter_map(|value| value.as_u64().and_then(|value| usize::try_from(value).ok()))
                })
                .collect();
            (0..nodes.len()).filter(|index| !children.contains(index)).collect()
        }
    };
    let mut images: Vec<Image> = Vec::new();
    let mut materials: Vec<Material> = Vec::new();
    for (index, value) in document.list("materials").iter().enumerate() {
        let mut material = import_material(value, index);
        for (pointer, slot) in [
            ("/pbrMetallicRoughness/baseColorTexture", kition_scene_model::TextureSlot::BaseColor),
            ("/pbrMetallicRoughness/metallicRoughnessTexture", kition_scene_model::TextureSlot::MetallicRoughness),
            ("/normalTexture", kition_scene_model::TextureSlot::Normal),
        ] {
            let Some(texture) = value.pointer(pointer).and_then(|reference| usize_of(reference, "index")) else {
                continue;
            };
            if slot == kition_scene_model::TextureSlot::Normal
                && let Some(scale) =
                    value.pointer("/normalTexture/scale").and_then(Value::as_f64).filter(|scale| scale.is_finite() && scale.abs() <= 10.0)
            {
                material.normal_strength = scale as f32;
            }
            if let Some(image) = document.texture_image(texture)? {
                material.set_texture(slot, Some(image.hash.clone()));
                if !images.iter().any(|other| other.hash == image.hash) {
                    images.push(image);
                }
            }
        }
        materials.push(material);
    }
    let lights = json.pointer("/extensions/KHR_lights_punctual/lights").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    let mut out = Imported { materials, images, ..Imported::default() };
    // Skins: their joints are bones, not objects; the node their root joints hang from is the
    // armature (its object is made where the node is, its bones after the walk).
    let skins = skin::read_skins(&document)?;
    let (worlds, parents) = skin::node_worlds(&document)?;
    let joint_nodes: BTreeSet<usize> = skins.iter().flat_map(|skin| skin.joints.iter().copied()).collect();
    let skin_root = |skin: &skin::Skin| -> Option<usize> {
        skin.joints.iter().filter_map(|joint| parents.get(*joint).copied().flatten()).find(|parent| !joint_nodes.contains(parent))
    };
    let armature_nodes: BTreeSet<usize> = skins.iter().filter_map(skin_root).collect();
    // (node → armature object index, its world matrices in glTF and scene space)
    let mut armature_objects: HashMap<usize, (usize, Mat4, Mat4)> = HashMap::new();
    // (mesh object, skin)
    let mut skinned: Vec<(usize, usize, usize)> = Vec::new();
    // (mesh node, mesh object, shape key names)
    let mut morphed: Vec<(usize, usize, Vec<String>)> = Vec::new();
    // The node each imported object came from, for its animation.
    let mut node_of = Vec::new();
    let mut visited = vec![false; nodes.len()];
    // (node, parent object index, parent's world matrix in glTF space and in scene space)
    let mut stack: Vec<(usize, Option<usize>, Mat4, Mat4)> =
        roots.into_iter().rev().map(|root| (root, None, Mat4::IDENTITY, Mat4::IDENTITY)).collect();
    while let Some((index, parent, parent_gltf, parent_scene)) = stack.pop() {
        let node = nodes.get(index).ok_or_else(|| invalid(format!("node {index} is not there")))?;
        let slot = visited.get_mut(index).ok_or_else(|| invalid(format!("node {index} is not there")))?;
        if *slot {
            return Err(invalid(format!("node {index} appears twice in the hierarchy")));
        }
        *slot = true;
        let gltf_world = parent_gltf * node_matrix(node)?;
        if joint_nodes.contains(&index) {
            // A joint: its non-joint children hang from the armature object.
            let armature =
                skins.iter().find(|skin| skin.joints.contains(&index)).and_then(skin_root).and_then(|root| armature_objects.get(&root).copied());
            for child in node.get("children").and_then(Value::as_array).map_or(&[][..], Vec::as_slice).iter().rev() {
                let child = child
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or_else(|| invalid(format!("node {index} has a child that is not a node index")))?;
                match armature {
                    Some((object, _, scene_world)) => stack.push((child, Some(object), gltf_world, scene_world)),
                    None => stack.push((child, parent, gltf_world, parent_scene)),
                }
            }
            continue;
        }
        let name = node.get("name").and_then(Value::as_str).map_or_else(|| format!("Node {index}"), str::to_string);
        let mut material = None;
        let mut skin_of = None;
        let data = if let Some(mesh_index) = usize_of(node, "mesh") {
            let (mut mesh, mesh_material) = import_mesh(&document, mesh_index)?;
            material = mesh_material.filter(|material| *material < out.materials.len());
            // A node's weights override its mesh's.
            if let Some(values) = node.get("weights").and_then(Value::as_array) {
                for (key, value) in mesh.shape_keys.iter_mut().zip(values) {
                    key.value = value.as_f64().map_or(key.value, |value| value as f32);
                }
            }
            skin_of = usize_of(node, "skin").filter(|skin| *skin < skins.len());
            if skin_of.is_none() {
                mesh.groups.clear();
            }
            if !mesh.shape_keys.is_empty() {
                morphed.push((index, out.objects.len(), mesh.shape_keys.iter().map(|key| key.name.clone()).collect()));
            }
            ObjectData::Mesh(mesh)
        } else if let Some(camera) = usize_of(node, "camera") {
            ObjectData::Camera(import_camera(document.item("cameras", camera)?))
        } else if let Some(light) =
            node.pointer("/extensions/KHR_lights_punctual/light").and_then(Value::as_u64).and_then(|value| usize::try_from(value).ok())
        {
            ObjectData::Light(import_light(
                lights.get(light).ok_or_else(|| invalid(format!("node {index} names light {light}, which is not there")))?,
            ))
        } else {
            ObjectData::Empty { size: 1.0 }
        };
        let scene_world = from_gltf_world(gltf_world, is_oriented(&data));
        // Location, rotation, and scale relative to the parent; when the local matrix has shear
        // they cannot hold, they hold the world matrix and the parent inverse cancels the parent.
        let mut local = parent_scene.inverse() * scene_world;
        let mut parent_inverse = Mat4::IDENTITY;
        let (scale, rotation, location) = local.to_scale_rotation_translation();
        if parent.is_some() && !Mat4::from_scale_rotation_translation(scale, rotation, location).abs_diff_eq(local, 1e-4) {
            local = scene_world;
            parent_inverse = parent_scene.inverse();
        }
        let (scale, rotation, location) = local.to_scale_rotation_translation();
        if !(scale.is_finite() && rotation.is_finite() && location.is_finite() && parent_inverse.is_finite()) {
            return Err(invalid(format!("node {index} has a transform that cannot be used")));
        }
        out.objects.push(ImportedObject {
            name,
            data,
            location,
            rotation: matrix_euler(Mat3::from_quat(rotation)),
            scale,
            parent,
            parent_inverse,
            material,
        });
        node_of.push(index);
        let me = out.objects.len() - 1;
        if let Some(skin) = skin_of {
            skinned.push((me, skin, index));
        }
        if armature_nodes.contains(&index) {
            armature_objects.insert(index, (me, gltf_world, scene_world));
        }
        for child in node.get("children").and_then(Value::as_array).map_or(&[][..], Vec::as_slice).iter().rev() {
            let child = child
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| invalid(format!("node {index} has a child that is not a node index")))?;
            stack.push((child, Some(me), gltf_world, scene_world));
        }
    }
    // Each skin's bones, on its armature object (made at the origin when its joints hang from
    // no node).
    let mut armatures: Vec<(skin::Skin, usize, Mat4, kition_scene_model::Armature)> = Vec::new();
    for (index, found) in skins.iter().enumerate() {
        let (object, scene_world) = match skin_root(found).and_then(|root| armature_objects.get(&root).copied()) {
            Some((object, _, scene_world)) => (object, scene_world),
            None => {
                out.objects.push(ImportedObject {
                    name: "Armature".into(),
                    data: ObjectData::Empty { size: 1.0 },
                    location: Vec3::ZERO,
                    rotation: Vec3::ZERO,
                    scale: Vec3::ONE,
                    parent: None,
                    parent_inverse: Mat4::IDENTITY,
                    material: None,
                });
                node_of.push(usize::MAX);
                (out.objects.len() - 1, Mat4::IDENTITY)
            }
        };
        // Skins sharing an armature node share the armature (the first one's bones).
        if armatures.iter().any(|(_, other, ..)| *other == object) {
            continue;
        }
        let mesh_world = skinned.iter().find(|(_, skin, _)| *skin == index).and_then(|(_, _, node)| worlds.get(*node).copied());
        let armature = skin::build_armature(&document, found, &worlds, &parents, scene_world, mesh_world);
        if let Some(slot) = out.objects.get_mut(object) {
            slot.data = ObjectData::Armature(armature.clone());
        }
        armatures.push((found.clone(), object, scene_world, armature));
    }
    for (mesh, skin, _) in &skinned {
        let Some(found) = skins.get(*skin) else { continue };
        let Some((_, rig, _, armature)) = armatures.iter().find(|(other, ..)| other.joints == found.joints).or_else(|| {
            skin_root(found)
                .and_then(|root| armature_objects.get(&root))
                .and_then(|(object, ..)| armatures.iter().find(|(_, other, ..)| other == object))
        }) else {
            continue;
        };
        // The k-th joint of this skin is the bone of that joint's node.
        let names: Vec<String> = found
            .joints
            .iter()
            .map(|joint| {
                let position =
                    armatures.iter().find(|(_, other, ..)| other == rig).and_then(|(skin, ..)| skin.joints.iter().position(|other| other == joint));
                position.and_then(|position| armature.bones.get(position)).map(|bone| bone.name.clone()).unwrap_or_default()
            })
            .collect();
        if let Some(ObjectData::Mesh(data)) = out.objects.get_mut(*mesh).map(|object| &mut object.data) {
            skin::name_groups(data, &names);
        }
        out.deforms.push((*mesh, *rig));
    }
    out.animation = anim::import(&document, &node_of, &out.objects)?;
    out.animation.extend(skin::import_animation(&document, &armatures, &morphed)?);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fan_comes_back_as_its_polygon() {
        assert_eq!(polygon_of(&[[0, 1, 2], [0, 2, 3], [0, 3, 4]]), Some(vec![0, 1, 2, 3, 4]));
        assert_eq!(polygon_of(&[[0, 1, 2], [0, 1, 2]]), None);
    }
}
