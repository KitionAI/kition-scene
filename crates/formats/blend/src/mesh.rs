//! Meshes: positions, faces (face offsets into corner vertices), loose edges, and flat or
//! smooth shading. Newer files keep them as named attributes in the mesh's attribute storage;
//! older ones as named custom-data layers. Both are read by name.

use std::collections::BTreeSet;

use kition_scene_model::Mesh;
use kition_scene_model::SceneError;
use kition_scene_model::glam::Vec3;

use crate::file::{File, View, invalid};

/// The largest mesh read.
const MAX_ELEMENTS: usize = 20_000_000;

/// A named attribute's bytes and element count (`single` when one value stands for all).
struct Attribute<'a> {
    bytes: &'a [u8],
    single: bool,
}

/// Every attribute of the mesh, by name.
fn attributes<'a>(file: &'a File, mesh: &View<'a>) -> Vec<(String, Attribute<'a>)> {
    let mut out = Vec::new();
    if let Some(storage) = mesh.nested("attribute_storage") {
        let count = storage.int("dna_attributes_num").unwrap_or(0).clamp(0, 10_000) as usize;
        if let Some(block) = storage.target("dna_attributes") {
            for index in 0..count.min(block.count) {
                let Some(attribute) = file.view_in("Attribute", block, index) else { continue };
                let Some(name) = attribute.target("name").map(|block| {
                    let bytes = file.bytes(block);
                    String::from_utf8_lossy(bytes.get(..bytes.iter().position(|byte| *byte == 0).unwrap_or(bytes.len())).unwrap_or_default())
                        .into_owned()
                }) else {
                    continue;
                };
                let Some(data) = attribute.follow("data") else { continue };
                let single = attribute.int("storage_type") == Some(1) || data.info.name == "AttributeSingle";
                let Some(values) = data.target("data") else { continue };
                out.push((name, Attribute { bytes: file.bytes(values), single }));
            }
        }
    }
    // Older files: custom-data layers on each domain.
    for domain in ["vdata", "edata", "pdata", "ldata"] {
        let Some(data) = mesh.nested(domain) else { continue };
        let count = data.int("totlayer").unwrap_or(0).clamp(0, 10_000) as usize;
        let Some(block) = data.target("layers") else { continue };
        for index in 0..count.min(block.count) {
            let Some(layer) = file.view_in("CustomDataLayer", block, index) else { continue };
            let (Some(name), Some(values)) = (layer.text("name"), layer.target("data")) else { continue };
            if !out.iter().any(|(known, _)| *known == name) {
                out.push((name, Attribute { bytes: file.bytes(values), single: false }));
            }
        }
    }
    out
}

fn words(bytes: &[u8], count: usize, stride: usize) -> Result<Vec<u32>, SceneError> {
    let needed = count.checked_mul(stride).ok_or_else(|| invalid("a mesh is too large"))?;
    let bytes = bytes.get(..needed).ok_or_else(|| invalid("a mesh attribute is cut short"))?;
    Ok(bytes.as_chunks::<4>().0.iter().map(|word| u32::from_le_bytes(*word)).collect())
}

/// Reads a mesh.
pub fn read(file: &File, mesh: &View<'_>) -> Result<Mesh, SceneError> {
    let count = |field: &str| -> Result<usize, SceneError> {
        let value = mesh.int(field).unwrap_or(0);
        usize::try_from(value).ok().filter(|value| *value <= MAX_ELEMENTS).ok_or_else(|| invalid(format!("the mesh's {field} is out of range")))
    };
    let (vertices, faces, corners) = (count("totvert")?, count("totpoly")?, count("totloop")?);
    let attributes = attributes(file, mesh);
    let find = |name: &str| attributes.iter().find(|(known, _)| known == name).map(|(_, attribute)| attribute);

    let mut out = Mesh::default();
    if vertices > 0 {
        let position = find("position").ok_or_else(|| invalid("a mesh has no positions"))?;
        let values = words(position.bytes, vertices, 12)?;
        out.positions =
            values.as_chunks::<3>().0.iter().map(|[x, y, z]| Vec3::new(f32::from_bits(*x), f32::from_bits(*y), f32::from_bits(*z))).collect();
    }
    if faces > 0 {
        let offsets = mesh.target("poly_offset_indices").ok_or_else(|| invalid("a mesh has faces but no face offsets"))?;
        let offsets = words(file.bytes(offsets), faces + 1, 4)?;
        let corner = find(".corner_vert").ok_or_else(|| invalid("a mesh has faces but no corners"))?;
        let corner_vertices = words(corner.bytes, corners, 4)?;
        for window in offsets.windows(2) {
            let [start, end] = window else { continue };
            let (start, end) = (*start as usize, *end as usize);
            let face = corner_vertices.get(start..end).ok_or_else(|| invalid("a face's corners are out of range"))?;
            out.faces.push(face.to_vec());
        }
    }
    // Edges in no face are loose edges (a circle without a fill, a wire).
    if let Some(edges) = find(".edge_verts") {
        let total = count("totedge")?;
        let pairs = words(edges.bytes, total, 8)?;
        let mut in_faces = BTreeSet::new();
        for face in &out.faces {
            for (a, b) in kition_scene_model::mesh::face_edges(face) {
                in_faces.insert((a.min(b), a.max(b)));
            }
        }
        for [a, b] in pairs.as_chunks::<2>().0 {
            let (a, b) = (*a, *b);
            if a != b && !in_faces.contains(&(a.min(b), a.max(b))) {
                out.loose_edges.push([a, b]);
            }
        }
    }
    // Shading: smooth unless most faces are marked sharp.
    out.smooth = match find("sharp_face") {
        None => !out.faces.is_empty(),
        Some(sharp) if sharp.single => sharp.bytes.first().copied().unwrap_or(1) == 0,
        Some(sharp) => {
            let marked = sharp.bytes.iter().take(faces).filter(|byte| **byte != 0).count();
            marked * 2 < faces
        }
    };
    out.validate()?;
    Ok(out)
}
