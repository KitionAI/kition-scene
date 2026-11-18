//! Stitching UV islands along edges: for each chosen edge whose two faces lie in different
//! islands (a seam, or coordinates that drifted apart), the smaller island is moved (turned,
//! scaled, and shifted) so its side of the edge lands on the other's, and the corners at the
//! edge's ends are snapped together. Seams stay marked; only coordinates move.

use std::collections::{BTreeMap, BTreeSet};

use glam::Vec2;

use crate::error::{Result, SceneError};
use crate::mesh::{Mesh, face_edges};

/// Coordinates closer than this count as the same.
const SAME: f32 = 1e-5;

fn corner_of(mesh: &Mesh, face: usize, vertex: u32) -> Option<usize> {
    mesh.faces.get(face)?.iter().position(|corner| *corner == vertex)
}

/// The coordinates of `face`'s corner at `vertex`.
fn uv_at(mesh: &Mesh, face: usize, vertex: u32) -> Option<Vec2> {
    corner_of(mesh, face, vertex).map(|corner| mesh.corner_uv(face, corner))
}

/// Faces grouped into UV islands: joined across an edge where both sides' coordinates agree.
fn islands(mesh: &Mesh) -> Vec<usize> {
    let mut parent: Vec<usize> = (0..mesh.faces.len()).collect();
    fn root(parent: &mut [usize], mut at: usize) -> usize {
        while let Some(&up) = parent.get(at) {
            if up == at {
                break;
            }
            let grand = parent.get(up).copied().unwrap_or(up);
            if let Some(slot) = parent.get_mut(at) {
                *slot = grand;
            }
            at = up;
        }
        at
    }
    let mut by_edge: BTreeMap<(u32, u32), Vec<usize>> = BTreeMap::new();
    for (face, corners) in mesh.faces.iter().enumerate() {
        for (a, b) in face_edges(corners) {
            by_edge.entry((a.min(b), a.max(b))).or_default().push(face);
        }
    }
    for ((a, b), faces) in &by_edge {
        let [f, g] = faces.as_slice() else { continue };
        let agree = |vertex: u32| {
            let (Some(cf), Some(cg)) = (corner_of(mesh, *f, vertex), corner_of(mesh, *g, vertex)) else { return false };
            mesh.corner_uv(*f, cf).distance(mesh.corner_uv(*g, cg)) <= SAME
        };
        if agree(*a) && agree(*b) {
            let (rf, rg) = (root(&mut parent, *f), root(&mut parent, *g));
            if let Some(slot) = parent.get_mut(rf) {
                *slot = rg;
            }
        }
    }
    (0..mesh.faces.len()).map(|face| root(&mut parent, face)).collect()
}

/// The similarity (as a complex multiplier and an offset) taking `from` to `to`.
fn similarity(from: (Vec2, Vec2), to: (Vec2, Vec2)) -> Option<(Vec2, Vec2)> {
    let d = from.1 - from.0;
    let e = to.1 - to.0;
    let length = d.length_squared();
    if length < 1e-20 {
        return None;
    }
    // m = e / d as complex numbers.
    let m = Vec2::new(e.x * d.x + e.y * d.y, e.y * d.x - e.x * d.y) / length;
    let apply = |p: Vec2| Vec2::new(m.x * p.x - m.y * p.y, m.x * p.y + m.y * p.x);
    Some((m, to.0 - apply(from.0)))
}

/// Stitches the edges whose both ends are in `vertices`. Returns how many edges were stitched.
pub fn stitch(mesh: &mut Mesh, vertices: &BTreeSet<u32>) -> Result<usize> {
    if !mesh.has_uvs() || !mesh.uvs_aligned() {
        return Err(SceneError::Unsupported("the mesh has no texture coordinates to stitch".into()));
    }
    let mut by_edge: BTreeMap<(u32, u32), Vec<usize>> = BTreeMap::new();
    for (face, corners) in mesh.faces.iter().enumerate() {
        for (a, b) in face_edges(corners) {
            if vertices.contains(&a) && vertices.contains(&b) {
                by_edge.entry((a.min(b), a.max(b))).or_default().push(face);
            }
        }
    }
    let mut stitched = 0;
    for ((a, b), faces) in by_edge {
        let [f, g] = faces.as_slice() else { continue };
        let (f, g) = (*f, *g);
        let island = islands(mesh);
        let (Some(&island_f), Some(&island_g)) = (island.get(f), island.get(g)) else { continue };
        let (Some(fa), Some(fb), Some(ga), Some(gb)) = (uv_at(mesh, f, a), uv_at(mesh, f, b), uv_at(mesh, g, a), uv_at(mesh, g, b)) else { continue };
        if fa.distance(ga) <= SAME && fb.distance(gb) <= SAME {
            continue;
        }
        // The smaller island moves onto the larger one.
        let size = |which: usize| island.iter().filter(|other| **other == which).count();
        let (moving, from, to) = if island_f == island_g {
            (None, (ga, gb), (fa, fb))
        } else if size(island_g) <= size(island_f) {
            (Some(island_g), (ga, gb), (fa, fb))
        } else {
            (Some(island_f), (fa, fb), (ga, gb))
        };
        if let (Some(moving), Some((m, offset))) = (moving, similarity(from, to)) {
            for (face, uvs) in mesh.uvs.iter_mut().enumerate() {
                if island.get(face) == Some(&moving) {
                    for value in uvs.iter_mut() {
                        *value = Vec2::new(m.x * value.x - m.y * value.y, m.x * value.y + m.y * value.x) + offset;
                    }
                }
            }
        }
        // Snap the corners at the edge's ends (on both faces, and every face of the moved island
        // meeting there) to the kept side's coordinates.
        let kept_face = if moving == Some(island_g) || moving.is_none() { f } else { g };
        let keep = |vertex: u32| uv_at(mesh, kept_face, vertex);
        let (Some(at_a), Some(at_b)) = (keep(a), keep(b)) else { continue };
        for (vertex, target) in [(a, at_a), (b, at_b)] {
            for face in [f, g] {
                if let Some(corner) = corner_of(mesh, face, vertex)
                    && let Some(slot) = mesh.uvs.get_mut(face).and_then(|uvs| uvs.get_mut(corner))
                {
                    *slot = target;
                }
            }
        }
        stitched += 1;
    }
    if stitched == 0 {
        return Err(SceneError::params("select edges between two islands to stitch them"));
    }
    Ok(stitched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::Primitive;

    #[test]
    fn two_islands_meet_along_the_stitched_edge() {
        // Two quads sharing edge 1-4, unwrapped apart: the right one shifted, turned, and scaled.
        let mut mesh = Primitive::Grid { x_subdivisions: 2, y_subdivisions: 1, size: 2.0 }.build().unwrap();
        mesh.uvs = vec![
            vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0)],
            vec![Vec2::new(5.0, 5.0), Vec2::new(5.0, 7.0), Vec2::new(3.0, 7.0), Vec2::new(3.0, 5.0)],
        ];
        let shared: Vec<u32> = mesh.faces[0].iter().copied().filter(|vertex| mesh.faces[1].contains(vertex)).collect();
        assert_eq!(shared.len(), 2);
        assert_eq!(stitch(&mut mesh, &shared.iter().copied().collect()).unwrap(), 1);
        for vertex in shared {
            let a = mesh.corner_uv(0, mesh.faces[0].iter().position(|v| *v == vertex).unwrap());
            let b = mesh.corner_uv(1, mesh.faces[1].iter().position(|v| *v == vertex).unwrap());
            assert!(a.distance(b) < 1e-5, "{a} {b}");
        }
        // The moved quad keeps its shape: a square of side 1.
        let quad = &mesh.uvs[1];
        assert!((quad[0].distance(quad[1]) - 1.0).abs() < 1e-5 && (quad[1].distance(quad[2]) - 1.0).abs() < 1e-5);
        assert!(stitch(&mut mesh, &BTreeSet::from([0])).is_err());
    }
}
