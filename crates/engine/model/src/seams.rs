//! UV seams and pins: marking edges where unwrapping cuts the surface open, pinning vertices
//! whose texture coordinates unwrapping keeps, and carrying both through edits that renumber or
//! split the mesh.

use std::collections::{BTreeSet, HashMap, HashSet};

use glam::Vec3;

use crate::error::{Result, SceneError};
use crate::mesh::Mesh;

/// An edge as (low, high).
fn key(a: u32, b: u32) -> [u32; 2] {
    [a.min(b), a.max(b)]
}

/// Every edge of the mesh, as (low, high).
pub fn edge_set(mesh: &Mesh) -> HashSet<[u32; 2]> {
    mesh.edges().into_iter().collect()
}

/// Whether the edge between `a` and `b` is a seam.
pub fn is_seam(mesh: &Mesh, a: u32, b: u32) -> bool {
    mesh.seams.binary_search(&key(a, b)).is_ok()
}

/// Marks (`on`) or clears the seams on `edges`; edges that are not edges of the mesh are refused.
/// Answers how many edges changed.
pub fn mark(mesh: &mut Mesh, edges: &[(u32, u32)], on: bool) -> Result<usize> {
    let existing = edge_set(mesh);
    let mut seams: BTreeSet<[u32; 2]> = mesh.seams.iter().copied().collect();
    let mut changed = 0;
    for (a, b) in edges {
        let edge = key(*a, *b);
        if !existing.contains(&edge) {
            return Err(SceneError::not_found(format!("edge {a}-{b}")));
        }
        let did = if on { seams.insert(edge) } else { seams.remove(&edge) };
        if did {
            changed += 1;
        }
    }
    mesh.seams = seams.into_iter().collect();
    Ok(changed)
}

/// Pins (`on`) or unpins vertices. Answers how many changed.
pub fn pin(mesh: &mut Mesh, vertices: &BTreeSet<u32>, on: bool) -> Result<usize> {
    if let Some(bad) = vertices.iter().find(|vertex| **vertex >= mesh.vertex_count()) {
        return Err(SceneError::not_found(format!("vertex {bad}")));
    }
    let mut pinned: BTreeSet<u32> = mesh.pinned.iter().copied().collect();
    let mut changed = 0;
    for vertex in vertices {
        let did = if on { pinned.insert(*vertex) } else { pinned.remove(vertex) };
        if did {
            changed += 1;
        }
    }
    mesh.pinned = pinned.into_iter().collect();
    Ok(changed)
}

/// Where seams and pins were before an edit, by position.
#[derive(Debug, Clone, Default)]
pub struct Marks {
    seams: Vec<(Vec3, Vec3)>,
    pins: Vec<Vec3>,
}

impl Marks {
    /// The marks of `mesh`, by position (empty when it has none).
    pub fn of(mesh: &Mesh) -> Self {
        let at = |vertex: u32| mesh.positions.get(vertex as usize).copied();
        Self {
            seams: mesh.seams.iter().filter_map(|[a, b]| Some((at(*a)?, at(*b)?))).collect(),
            pins: mesh.pinned.iter().filter_map(|vertex| at(*vertex)).collect(),
        }
    }

    /// Whether there is nothing to carry.
    pub fn is_empty(&self) -> bool {
        self.seams.is_empty() && self.pins.is_empty()
    }
}

/// Bit pattern of a position, for exact matching.
fn bits(point: Vec3) -> [u32; 3] {
    point.to_array().map(|value| (value + 0.0).to_bits())
}

/// After an edit: keeps the seams and pins whose vertices the edit kept (by index, as
/// renumbering left them), and finds the rest by position: an edge lying along an old seam (a
/// seam split by a cut) is a seam, and a vertex where a pinned one was is pinned.
pub fn carry(before: &Marks, mesh: &mut Mesh) {
    let edges = edge_set(mesh);
    let mut seams: BTreeSet<[u32; 2]> = mesh.seams.iter().copied().filter(|edge| edges.contains(edge)).collect();
    let mut pins: BTreeSet<u32> = mesh.pinned.iter().copied().filter(|vertex| (*vertex as usize) < mesh.positions.len()).collect();
    if !before.is_empty() {
        let mut at: HashMap<[u32; 3], Vec<u32>> = HashMap::new();
        for (index, point) in mesh.positions.iter().enumerate() {
            at.entry(bits(*point)).or_default().push(u32::try_from(index).unwrap_or(u32::MAX));
        }
        for point in &before.pins {
            pins.extend(at.get(&bits(*point)).into_iter().flatten().copied());
        }
        if !before.seams.is_empty() {
            for edge in &edges {
                let (Some(a), Some(b)) = (mesh.positions.get(edge[0] as usize), mesh.positions.get(edge[1] as usize)) else { continue };
                if before.seams.iter().any(|(from, to)| on_segment(*a, *from, *to) && on_segment(*b, *from, *to)) {
                    seams.insert(*edge);
                }
            }
        }
    }
    mesh.seams = seams.into_iter().collect();
    mesh.pinned = pins.into_iter().collect();
}

/// Whether `point` lies on the segment `from`–`to` (within a hair of its length).
fn on_segment(point: Vec3, from: Vec3, to: Vec3) -> bool {
    let span = to - from;
    let length = span.length();
    if length < 1e-9 {
        return point.distance(from) < 1e-6;
    }
    let t = (point - from).dot(span) / (length * length);
    if !(-1e-5..=1.0 + 1e-5).contains(&t) {
        return false;
    }
    point.distance(from + span * t) <= length * 1e-5 + 1e-7
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::Primitive;

    #[test]
    fn marking_refuses_non_edges_and_counts_changes() {
        let mut cube = Primitive::Cube { size: 2.0 }.build().unwrap();
        assert_eq!(mark(&mut cube, &[(0, 1), (1, 0), (0, 2)], true).unwrap(), 2);
        assert!(is_seam(&cube, 1, 0));
        assert!(mark(&mut cube, &[(0, 7)], true).is_err());
        assert_eq!(mark(&mut cube, &[(0, 1)], false).unwrap(), 1);
        assert_eq!(cube.seams, vec![[0, 2]]);
    }

    #[test]
    fn a_split_seam_stays_a_seam() {
        let mut cube = Primitive::Cube { size: 2.0 }.build().unwrap();
        mark(&mut cube, &[(0, 1)], true).unwrap();
        let marks = Marks::of(&cube);
        // A loop cut across the seam's edge splits it in two.
        crate::ops::loop_cut(&mut cube, (0, 1), 1, 0.5).unwrap();
        carry(&marks, &mut cube);
        assert_eq!(cube.seams.len(), 2, "{:?}", cube.seams);
        cube.validate().unwrap();
    }
}
