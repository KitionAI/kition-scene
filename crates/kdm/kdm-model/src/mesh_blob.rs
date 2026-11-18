//! A mesh as one blob (`docs/design.md` section 7.1): the binary geometry buffer (positions,
//! faces, loose edges, UVs, seams, pins; magic `KSMB`, little-endian) followed by a JSON tail
//! with the shading, vertex groups, shape keys, the sculpt mask, face sets, hidden faces, and
//! multiresolution offsets (base64 little-endian floats, one string per level). Decoding checks
//! every count and index.

use std::collections::HashMap;
use std::sync::Arc;

use kition_scene_model::glam::{Vec2, Vec3};
use kition_scene_model::{Mesh, SceneError};
use serde::{Deserialize, Serialize};

/// A mesh blob's media type.
pub const MIME: &str = "application/vnd.kition.scene.mesh";

const MAGIC: &[u8; 4] = b"KSMB";

fn invalid(message: impl Into<String>) -> SceneError {
    SceneError::InvalidFile(message.into())
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Tail {
    #[serde(default)]
    smooth: bool,
    /// Smooth by angle, in radians.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    smooth_angle: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    groups: Vec<kition_scene_model::vertex_data::VertexGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    shape_keys: Vec<kition_scene_model::vertex_data::ShapeKey>,
    /// The sculpt mask (absent in older blobs, read as none).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    mask: Vec<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    face_sets: Vec<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    hidden: Vec<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    multires: Vec<String>,
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Offsets as base64 of little-endian floats.
fn encode_offsets(offsets: &[Vec3]) -> String {
    let bytes: Vec<u8> = offsets.iter().flat_map(|offset| offset.to_array()).flat_map(f32::to_le_bytes).collect();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let word = chunk.iter().enumerate().fold(0u32, |word, (index, byte)| word | u32::from(*byte) << (16 - 8 * index));
        for index in 0..4 {
            if index <= chunk.len() {
                let six = (word >> (18 - 6 * index)) & 63;
                out.push(char::from(BASE64.get(six as usize).copied().unwrap_or(b'A')));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The offsets of [`encode_offsets`]' text.
fn decode_offsets(text: &str) -> Result<Vec<Vec3>, SceneError> {
    let mut bytes = Vec::with_capacity(text.len() / 4 * 3);
    let digits = text.trim_end_matches('=').as_bytes();
    for chunk in digits.chunks(4) {
        let mut word = 0u32;
        for (index, digit) in chunk.iter().enumerate() {
            let value = BASE64.iter().position(|known| known == digit).ok_or_else(|| invalid("a mesh blob's multiresolution offsets are damaged"))?;
            word |= (value as u32) << (18 - 6 * index);
        }
        for index in 0..chunk.len().saturating_sub(1) {
            bytes.push((word >> (16 - 8 * index)) as u8);
        }
    }
    if bytes.len() % 12 != 0 {
        return Err(invalid("a mesh blob's multiresolution offsets are cut short"));
    }
    Ok(bytes
        .as_chunks::<12>()
        .0
        .iter()
        .map(|chunk| {
            let float = |at: usize| f32::from_le_bytes([0, 1, 2, 3].map(|offset| chunk.get(at + offset).copied().unwrap_or(0)));
            Vec3::new(float(0), float(4), float(8))
        })
        .collect())
}

/// The blob of `mesh`.
pub fn encode(mesh: &Mesh) -> Vec<u8> {
    let mut out = Vec::with_capacity(24 + mesh.positions.len() * 12 + mesh.faces.iter().map(|face| 4 + face.len() * 4).sum::<usize>());
    let u32s = |out: &mut Vec<u8>, value: usize| out.extend(u32::try_from(value).unwrap_or(u32::MAX).to_le_bytes());
    out.extend(MAGIC);
    u32s(&mut out, mesh.positions.len());
    for point in &mesh.positions {
        for value in point.to_array() {
            out.extend(value.to_le_bytes());
        }
    }
    u32s(&mut out, mesh.faces.len());
    for face in &mesh.faces {
        u32s(&mut out, face.len());
        for index in face {
            out.extend(index.to_le_bytes());
        }
    }
    u32s(&mut out, mesh.loose_edges.len());
    for [a, b] in &mesh.loose_edges {
        out.extend(a.to_le_bytes());
        out.extend(b.to_le_bytes());
    }
    // Texture coordinates: a corner count (0 for none), then u, v per corner in face order.
    let uvs = mesh.has_uvs() && mesh.uvs_aligned();
    u32s(&mut out, if uvs { mesh.faces.iter().map(Vec::len).sum() } else { 0 });
    if uvs {
        for uv in mesh.uvs.iter().flatten() {
            out.extend(uv.x.to_le_bytes());
            out.extend(uv.y.to_le_bytes());
        }
    }
    u32s(&mut out, mesh.seams.len());
    for [a, b] in &mesh.seams {
        out.extend(a.to_le_bytes());
        out.extend(b.to_le_bytes());
    }
    u32s(&mut out, mesh.pinned.len());
    for vertex in &mesh.pinned {
        out.extend(vertex.to_le_bytes());
    }
    let faces = mesh.faces.len();
    let multires =
        if kition_scene_model::multires::levels(mesh) > 0 { mesh.multires.iter().map(|level| encode_offsets(level)).collect() } else { Vec::new() };
    let tail = serde_json::to_vec(&Tail {
        smooth: mesh.smooth,
        smooth_angle: mesh.smooth_angle,
        groups: mesh.groups.clone(),
        shape_keys: mesh.shape_keys.clone(),
        mask: mesh.mask.clone(),
        face_sets: if mesh.face_sets.len() == faces { mesh.face_sets.clone() } else { Vec::new() },
        hidden: if mesh.hidden.len() == faces { mesh.hidden.clone() } else { Vec::new() },
        multires,
    })
    .unwrap_or_default();
    u32s(&mut out, tail.len());
    out.extend(tail);
    out
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, count: usize) -> Result<&[u8], SceneError> {
        let end = self.at.checked_add(count).ok_or_else(|| invalid("a mesh blob is damaged"))?;
        let slice = self.bytes.get(self.at..end).ok_or_else(|| invalid("a mesh blob is cut short"))?;
        self.at = end;
        Ok(slice)
    }

    fn u32(&mut self) -> Result<u32, SceneError> {
        let bytes = self.take(4)?;
        let mut le = [0u8; 4];
        le.copy_from_slice(bytes);
        Ok(u32::from_le_bytes(le))
    }

    fn f32(&mut self) -> Result<f32, SceneError> {
        self.u32().map(f32::from_bits)
    }

    /// A count of items each at least `size` bytes, checked against what is left.
    fn count(&mut self, size: usize) -> Result<usize, SceneError> {
        let count = self.u32()? as usize;
        if count.saturating_mul(size) > self.bytes.len().saturating_sub(self.at) {
            return Err(invalid("a mesh blob claims more data than it has"));
        }
        Ok(count)
    }
}

/// The mesh of a blob.
pub fn decode(bytes: &[u8]) -> Result<Mesh, SceneError> {
    let mut reader = Reader { bytes, at: 0 };
    if reader.take(4)? != MAGIC {
        return Err(invalid("a mesh blob has the wrong magic"));
    }
    let mut mesh = Mesh::default();
    let vertices = reader.count(12)?;
    mesh.positions.reserve(vertices);
    for _ in 0..vertices {
        mesh.positions.push(Vec3::new(reader.f32()?, reader.f32()?, reader.f32()?));
    }
    let faces = reader.count(4)?;
    mesh.faces.reserve(faces);
    for _ in 0..faces {
        let corners = reader.count(4)?;
        let mut face = Vec::with_capacity(corners);
        for _ in 0..corners {
            face.push(reader.u32()?);
        }
        mesh.faces.push(face);
    }
    let loose = reader.count(8)?;
    for _ in 0..loose {
        mesh.loose_edges.push([reader.u32()?, reader.u32()?]);
    }
    let corners = reader.count(8)?;
    if corners > 0 {
        if corners != mesh.faces.iter().map(Vec::len).sum::<usize>() {
            return Err(invalid("a mesh blob's texture coordinates do not match its faces"));
        }
        for face in &mesh.faces {
            let mut uvs = Vec::with_capacity(face.len());
            for _ in face {
                uvs.push(Vec2::new(reader.f32()?, reader.f32()?));
            }
            mesh.uvs.push(uvs);
        }
    }
    let seams = reader.count(8)?;
    for _ in 0..seams {
        mesh.seams.push([reader.u32()?, reader.u32()?]);
    }
    let pins = reader.count(4)?;
    for _ in 0..pins {
        mesh.pinned.push(reader.u32()?);
    }
    if !mesh.seams.windows(2).all(|pair| matches!(pair, [a, b] if a < b)) || !mesh.pinned.windows(2).all(|pair| matches!(pair, [a, b] if a < b)) {
        return Err(invalid("a mesh blob's seams or pins are out of order"));
    }
    let length = reader.count(1)?;
    let tail: Tail = serde_json::from_slice(reader.take(length)?).map_err(|error| invalid(format!("a mesh blob's tail: {error}")))?;
    mesh.smooth = tail.smooth;
    if tail.smooth_angle.is_some_and(|angle| !(angle.is_finite() && (0.0..=std::f32::consts::PI).contains(&angle))) {
        return Err(invalid("a mesh blob's smoothing angle is out of range"));
    }
    mesh.smooth_angle = tail.smooth_angle;
    mesh.groups = tail.groups;
    mesh.shape_keys = tail.shape_keys;
    mesh.mask = tail.mask;
    mesh.face_sets = tail.face_sets;
    mesh.hidden = tail.hidden;
    mesh.multires = tail.multires.iter().map(|level| decode_offsets(level)).collect::<Result<_, _>>()?;
    if mesh.multires.iter().flatten().any(|offset| !offset.is_finite()) {
        return Err(invalid("a mesh blob's multiresolution offsets are not finite"));
    }
    // Face data or levels that do not fit the faces (written by a peer that changed them) are
    // dropped, as when they are read in the editor.
    kition_scene_model::sculpt::face_sets::tidy(&mut mesh);
    if kition_scene_model::multires::levels(&mesh) == 0 {
        mesh.multires.clear();
    }
    if reader.at != bytes.len() {
        return Err(invalid("a mesh blob has trailing data"));
    }
    mesh.validate()?;
    Ok(mesh)
}

/// Decoded meshes by blob hash, so reading a replica again decodes only meshes that changed.
#[derive(Debug, Clone, Default)]
pub struct MeshCache(HashMap<String, Arc<Mesh>>);

impl MeshCache {
    /// The mesh of the blob `hash` with bytes `bytes`, decoded once.
    pub fn get(&mut self, hash: &str, bytes: &[u8]) -> Result<Arc<Mesh>, SceneError> {
        if let Some(mesh) = self.0.get(hash) {
            return Ok(mesh.clone());
        }
        let mesh = Arc::new(decode(bytes)?);
        self.0.insert(hash.to_owned(), mesh.clone());
        Ok(mesh)
    }

    /// Forgets meshes no longer named.
    pub fn retain(&mut self, keep: &std::collections::BTreeSet<String>) {
        self.0.retain(|hash, _| keep.contains(hash));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kition_scene_model::Primitive;

    #[test]
    fn a_mesh_round_trips_and_damage_is_refused() {
        let mut ball = Primitive::UvSphere { segments: 8, rings: 4, radius: 1.0 }.build().unwrap();
        let [a, b] = ball.edges()[0];
        kition_scene_model::seams::mark(&mut ball, &[(a, b)], true).unwrap();
        ball.pinned = vec![2, 5];
        ball.smooth = true;
        ball.face_sets = (0..ball.faces.len() as u32).map(|face| face % 3).collect();
        ball.hidden = (0..ball.faces.len()).map(|face| face == 2).collect();
        kition_scene_model::multires::subdivide(&mut ball).unwrap();
        for (index, offset) in ball.multires[0].iter_mut().enumerate() {
            *offset = Vec3::new(index as f32 * 0.001, -0.5, 1.0 / 3.0);
        }
        let bytes = encode(&ball);
        assert_eq!(decode(&bytes).unwrap(), ball);
        assert!(decode(b"KSMB\xff\xff\xff\xff").is_err());
        for cut in [3, bytes.len() / 2, bytes.len() - 1] {
            assert!(decode(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(decode(&longer).is_err());
    }

    #[test]
    fn a_smoothing_angle_round_trips_and_a_wild_one_is_refused() {
        let mut can = Primitive::Cylinder { vertices: 12, radius: 1.0, depth: 2.0, fill: Default::default() }.build().unwrap();
        can.smooth = true;
        can.smooth_angle = Some(30f32.to_radians());
        assert_eq!(decode(&encode(&can)).unwrap(), can);
        can.smooth_angle = None;
        assert_eq!(decode(&encode(&can)).unwrap().smooth_angle, None);
        can.smooth_angle = Some(9.0);
        assert!(decode(&encode(&can)).is_err());
    }
}
