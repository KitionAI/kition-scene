//! Data a mesh keeps per vertex beside its positions: vertex groups (named weights, which the
//! Armature modifier deforms by) and shape keys (named offsets from the basis shape, mixed in by
//! their values). Both stay parallel to the positions: an edit that adds, removes, or moves
//! vertices carries them over by position ([`carry`]).

use std::collections::HashMap;

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::error::{Result, SceneError};
use crate::mesh::Mesh;

/// A named weight per vertex, 0 to 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VertexGroup {
    pub name: String,
    /// One weight per vertex; 0 means not in the group.
    pub weights: Vec<f32>,
}

/// A named shape: an offset per vertex from the basis (the mesh's own positions), mixed in by
/// `value` (relative shape keys, as the reference application's default).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeKey {
    pub name: String,
    /// One offset per vertex.
    pub offsets: Vec<Vec3>,
    /// How much of the shape is mixed in, usually 0 to 1.
    #[serde(default)]
    pub value: f32,
}

/// The range a shape key value may take.
pub const SHAPE_KEY_RANGE: (f32, f32) = (-10.0, 10.0);

/// Whether the mesh has vertex groups or shape keys.
pub fn has_vertex_data(mesh: &Mesh) -> bool {
    !mesh.groups.is_empty() || !mesh.shape_keys.is_empty() || !mesh.mask.is_empty()
}

/// Whether every group and key has exactly one entry per vertex.
pub fn aligned(mesh: &Mesh) -> bool {
    let count = mesh.positions.len();
    mesh.groups.iter().all(|group| group.weights.len() == count)
        && mesh.shape_keys.iter().all(|key| key.offsets.len() == count)
        && (mesh.mask.is_empty() || mesh.mask.len() == count)
}

/// Checks the groups and keys of a mesh read from a file.
pub fn check(mesh: &Mesh) -> Result<()> {
    if !aligned(mesh) {
        return Err(SceneError::InvalidFile("the vertex groups or shape keys do not match the vertices".into()));
    }
    let mut names = std::collections::BTreeSet::new();
    for group in &mesh.groups {
        if group.name.is_empty() || !names.insert(("group", group.name.as_str())) {
            return Err(SceneError::InvalidFile(format!("vertex group name `{}` is empty or repeated", group.name)));
        }
        if group.weights.iter().any(|weight| !(weight.is_finite() && (0.0..=1.0).contains(weight))) {
            return Err(SceneError::InvalidFile(format!("vertex group `{}` has a weight outside 0 to 1", group.name)));
        }
    }
    if mesh.mask.iter().any(|value| !(value.is_finite() && (0.0..=1.0).contains(value))) {
        return Err(SceneError::InvalidFile("the sculpt mask has a value outside 0 to 1".into()));
    }
    for key in &mesh.shape_keys {
        if key.name.is_empty() || !names.insert(("key", key.name.as_str())) {
            return Err(SceneError::InvalidFile(format!("shape key name `{}` is empty or repeated", key.name)));
        }
        if !key.value.is_finite() || key.offsets.iter().any(|offset| !offset.is_finite()) {
            return Err(SceneError::InvalidFile(format!("shape key `{}` is not finite", key.name)));
        }
    }
    Ok(())
}

/// The vertex group named `name`.
pub fn group<'a>(mesh: &'a Mesh, name: &str) -> Option<&'a VertexGroup> {
    mesh.groups.iter().find(|group| group.name == name)
}

/// The vertex group named `name`, made (empty) when missing.
pub fn group_mut<'a>(mesh: &'a mut Mesh, name: &str) -> Option<&'a mut VertexGroup> {
    if !mesh.groups.iter().any(|group| group.name == name) {
        let weights = vec![0.0; mesh.positions.len()];
        mesh.groups.push(VertexGroup { name: name.to_string(), weights });
    }
    mesh.groups.iter_mut().find(|group| group.name == name)
}

/// Mixes the shape keys into the positions (and drops them: the result is a plain shape).
pub fn apply_shape_keys(mesh: &mut Mesh) {
    let keys = std::mem::take(&mut mesh.shape_keys);
    let count = mesh.positions.len();
    for key in keys.iter().filter(|key| key.value != 0.0 && key.offsets.len() == count) {
        for (point, offset) in mesh.positions.iter_mut().zip(&key.offsets) {
            *point += *offset * key.value;
        }
    }
}

/// The groups and keys of `mesh` and where its vertices were, to carry them through an edit.
#[derive(Debug, Clone)]
pub struct Snapshot {
    positions: Vec<Vec3>,
    groups: Vec<VertexGroup>,
    shape_keys: Vec<ShapeKey>,
    mask: Vec<f32>,
}

/// A snapshot to carry the mesh's groups and keys from, when it has any (aligned).
pub fn snapshot(mesh: &Mesh) -> Option<Snapshot> {
    (has_vertex_data(mesh) && aligned(mesh)).then(|| Snapshot {
        positions: mesh.positions.clone(),
        groups: mesh.groups.clone(),
        shape_keys: mesh.shape_keys.clone(),
        mask: mesh.mask.clone(),
    })
}

fn cell_of(point: Vec3, size: f32) -> (i64, i64, i64) {
    ((point.x / size).floor() as i64, (point.y / size).floor() as i64, (point.z / size).floor() as i64)
}

/// The old vertex each vertex of `after` takes its data from: one at the same position, else
/// the nearest.
pub(crate) fn sources(before: &[Vec3], after: &[Vec3]) -> Vec<Option<usize>> {
    if before.is_empty() {
        return vec![None; after.len()];
    }
    let bounds = before.iter().fold(crate::mesh::Bounds::EMPTY, |bounds, point| bounds.with(*point));
    let size = (bounds.size().max_element() / (before.len() as f32).cbrt().max(1.0)).max(1e-4);
    let mut grid: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    for (index, point) in before.iter().enumerate() {
        grid.entry(cell_of(*point, size)).or_default().push(index);
    }
    let exact: HashMap<[u32; 3], usize> =
        before.iter().enumerate().rev().map(|(index, point)| (point.to_array().map(|value| (value + 0.0).to_bits()), index)).collect();
    after
        .iter()
        .map(|point| {
            if let Some(index) = exact.get(&point.to_array().map(|value| (value + 0.0).to_bits())) {
                return Some(*index);
            }
            let centre = cell_of(*point, size);
            // Rings of cells outwards until something is found, then one ring more (a nearer
            // vertex can sit just across a cell boundary).
            let mut best: Option<(f32, usize)> = None;
            let mut found_at = None;
            for ring in 0..64i64 {
                if found_at.is_some_and(|at: i64| ring > at + 1) {
                    break;
                }
                for dx in -ring..=ring {
                    for dy in -ring..=ring {
                        for dz in -ring..=ring {
                            if dx.abs().max(dy.abs()).max(dz.abs()) != ring {
                                continue;
                            }
                            for &index in grid.get(&(centre.0 + dx, centre.1 + dy, centre.2 + dz)).into_iter().flatten() {
                                let Some(other) = before.get(index) else { continue };
                                let distance = other.distance_squared(*point);
                                if best.is_none_or(|(nearest, _)| distance < nearest) {
                                    best = Some((distance, index));
                                }
                            }
                        }
                    }
                }
                if best.is_some() && found_at.is_none() {
                    found_at = Some(ring);
                }
            }
            best.map(|(_, index)| index).or(Some(0))
        })
        .collect()
}

/// Gives `after` (an edit of the snapshot's mesh) groups and keys: each vertex takes the weights
/// and shape offsets of the old vertex at its position, or of the nearest one.
pub fn carry(before: Option<&Snapshot>, after: &mut Mesh) {
    let Some(before) = before else {
        if !aligned(after) {
            after.groups.clear();
            after.shape_keys.clear();
            after.mask.clear();
        }
        return;
    };
    if aligned(after)
        && after.positions == before.positions
        && after.groups.len() == before.groups.len()
        && after.shape_keys.len() == before.shape_keys.len()
        && after.mask.len() == before.mask.len()
    {
        return;
    }
    let from = sources(&before.positions, &after.positions);
    after.groups = before
        .groups
        .iter()
        .map(|group| VertexGroup {
            name: group.name.clone(),
            weights: from.iter().map(|source| source.and_then(|index| group.weights.get(index)).copied().unwrap_or(0.0)).collect(),
        })
        .collect();
    after.shape_keys = before
        .shape_keys
        .iter()
        .map(|key| ShapeKey {
            name: key.name.clone(),
            offsets: from.iter().map(|source| source.and_then(|index| key.offsets.get(index)).copied().unwrap_or(Vec3::ZERO)).collect(),
            value: key.value,
        })
        .collect();
    after.mask = if before.mask.is_empty() {
        Vec::new()
    } else {
        from.iter().map(|source| source.and_then(|index| before.mask.get(index)).copied().unwrap_or(0.0)).collect()
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::Primitive;

    #[test]
    fn groups_and_keys_follow_an_extrude() {
        let mut mesh = Primitive::Cube { size: 2.0 }.build().unwrap();
        let top: Vec<u32> = (0..mesh.vertex_count()).filter(|vertex| mesh.positions[*vertex as usize].z > 0.0).collect();
        let top_group = group_mut(&mut mesh, "Top").unwrap();
        for vertex in &top {
            top_group.weights[*vertex as usize] = 1.0;
        }
        mesh.shape_keys.push(ShapeKey {
            name: "Lift".into(),
            offsets: mesh.positions.iter().map(|point| Vec3::new(0.0, 0.0, point.z.max(0.0))).collect(),
            value: 0.5,
        });
        let face = mesh.faces.iter().position(|face| mesh.face_normal(face).z > 0.9).unwrap() as u32;
        crate::ops::extrude_region(&mut mesh, &[face], 1.0, Vec3::ZERO).unwrap();
        assert!(aligned(&mesh));
        check(&mesh).unwrap();
        let weights = &group(&mesh, "Top").unwrap().weights;
        for (point, weight) in mesh.positions.iter().zip(weights) {
            assert_eq!(*weight, if point.z > 0.0 { 1.0 } else { 0.0 }, "{point}");
        }
        let mut shaped = mesh.clone();
        apply_shape_keys(&mut shaped);
        assert!(shaped.shape_keys.is_empty());
        let highest = shaped.positions.iter().map(|point| point.z).fold(f32::MIN, f32::max);
        assert!((highest - 2.5).abs() < 1e-5, "{highest}");
    }

    #[test]
    fn misaligned_data_is_refused() {
        let mut mesh = Primitive::Plane { size: 2.0 }.build().unwrap();
        mesh.groups.push(VertexGroup { name: "A".into(), weights: vec![1.0] });
        assert!(check(&mesh).is_err());
        mesh.groups[0].weights = vec![2.0; 4];
        assert!(check(&mesh).is_err());
    }
}
