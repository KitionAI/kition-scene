//! Mesh editing operations, as Edit Mode runs them on a selection. Faces and vertices are named
//! by index; every operation checks its indices and returns what it made so the caller can
//! select it (a modeller keeps the extruded faces selected, for instance).

use std::collections::{BTreeSet, HashMap, VecDeque};

use glam::Vec3;

use crate::error::{Result, SceneError};
use crate::mesh::{Mesh, face_edges};

pub use crate::bevel::bevel_edges;

fn check_faces(mesh: &Mesh, faces: &[u32]) -> Result<BTreeSet<u32>> {
    let set: BTreeSet<u32> = faces.iter().copied().collect();
    if let Some(bad) = set.iter().find(|face| **face as usize >= mesh.faces.len()) {
        return Err(SceneError::not_found(format!("face {bad}")));
    }
    Ok(set)
}

fn check_vertices(mesh: &Mesh, vertices: &[u32]) -> Result<BTreeSet<u32>> {
    let set: BTreeSet<u32> = vertices.iter().copied().collect();
    if let Some(bad) = set.iter().find(|vertex| **vertex as usize >= mesh.positions.len()) {
        return Err(SceneError::not_found(format!("vertex {bad}")));
    }
    Ok(set)
}

fn finite(name: &str, value: f32) -> Result<()> {
    if value.is_finite() { Ok(()) } else { Err(SceneError::params(format!("{name} must be a number"))) }
}

/// The vertices the faces use.
pub fn face_vertices(mesh: &Mesh, faces: &BTreeSet<u32>) -> BTreeSet<u32> {
    faces.iter().filter_map(|face| mesh.faces.get(*face as usize)).flatten().copied().collect()
}

/// The faces all of whose vertices are in `vertices`.
pub fn faces_within(mesh: &Mesh, vertices: &BTreeSet<u32>) -> BTreeSet<u32> {
    mesh.faces
        .iter()
        .enumerate()
        .filter(|(_, face)| face.iter().all(|index| vertices.contains(index)))
        .map(|(index, _)| u32::try_from(index).unwrap_or(u32::MAX))
        .collect()
}

/// Moves vertices by `delta`.
pub fn translate(mesh: &mut Mesh, vertices: &[u32], delta: Vec3) -> Result<()> {
    if !delta.is_finite() {
        return Err(SceneError::params("the offset must be numbers"));
    }
    for vertex in check_vertices(mesh, vertices)? {
        if let Some(point) = mesh.positions.get_mut(vertex as usize) {
            *point += delta;
        }
    }
    Ok(())
}

/// Scales vertices by `factor` about `pivot`.
pub fn scale(mesh: &mut Mesh, vertices: &[u32], pivot: Vec3, factor: Vec3) -> Result<()> {
    if !(pivot.is_finite() && factor.is_finite()) {
        return Err(SceneError::params("the pivot and factor must be numbers"));
    }
    for vertex in check_vertices(mesh, vertices)? {
        if let Some(point) = mesh.positions.get_mut(vertex as usize) {
            *point = pivot + (*point - pivot) * factor;
        }
    }
    Ok(())
}

/// Extrudes the faces as one region: their open outline gets side walls, and the region moves
/// `distance` along its mean normal (plus `offset`). The region's faces keep their indices.
/// Returns the new side faces.
pub fn extrude_region(mesh: &mut Mesh, faces: &[u32], distance: f32, offset: Vec3) -> Result<Vec<u32>> {
    crate::uv::with_uvs(mesh, |mesh| extrude_region_inner(mesh, faces, distance, offset))
}

fn extrude_region_inner(mesh: &mut Mesh, faces: &[u32], distance: f32, offset: Vec3) -> Result<Vec<u32>> {
    let region = check_faces(mesh, faces)?;
    finite("distance", distance)?;
    if !offset.is_finite() {
        return Err(SceneError::params("the offset must be numbers"));
    }
    if region.is_empty() {
        return Err(SceneError::params("no faces to extrude"));
    }
    let normal = region.iter().filter_map(|face| mesh.faces.get(*face as usize)).map(|face| mesh.face_normal(face)).sum::<Vec3>().normalize_or_zero();

    // Edges of the region used by one region face are its outline.
    let mut uses: HashMap<(u32, u32), usize> = HashMap::new();
    for face in region.iter().filter_map(|face| mesh.faces.get(*face as usize)) {
        for (a, b) in face_edges(face) {
            *uses.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let mut outline: Vec<(u32, u32)> = Vec::new();
    for face in region.iter().filter_map(|face| mesh.faces.get(*face as usize)) {
        for (a, b) in face_edges(face) {
            if uses.get(&(a.min(b), a.max(b))) == Some(&1) {
                outline.push((a, b));
            }
        }
    }

    // Outline vertices are copied (the copies go with the region); inner ones just move.
    let mut copies: HashMap<u32, u32> = HashMap::new();
    for (a, b) in &outline {
        for vertex in [*a, *b] {
            if let std::collections::hash_map::Entry::Vacant(e) = copies.entry(vertex) {
                let point = mesh.position(vertex)?;
                e.insert(mesh.vertex_count());
                mesh.positions.push(point);
            }
        }
    }
    let moved: BTreeSet<u32> = face_vertices(mesh, &region).into_iter().map(|vertex| copies.get(&vertex).copied().unwrap_or(vertex)).collect();
    for face in &region {
        if let Some(corners) = mesh.faces.get_mut(*face as usize) {
            for corner in corners.iter_mut() {
                *corner = copies.get(corner).copied().unwrap_or(*corner);
            }
        }
    }
    let mut sides = Vec::new();
    for (a, b) in outline {
        let (top_a, top_b) = (copies.get(&a).copied().unwrap_or(a), copies.get(&b).copied().unwrap_or(b));
        sides.push(u32::try_from(mesh.faces.len()).unwrap_or(u32::MAX));
        mesh.faces.push(vec![a, b, top_b, top_a]);
    }
    let delta = normal * distance + offset;
    for vertex in moved {
        if let Some(point) = mesh.positions.get_mut(vertex as usize) {
            *point += delta;
        }
    }
    Ok(sides)
}

/// Insets each face on its own: the face shrinks by `thickness` inside its outline and a ring of
/// quads fills the gap. The inset faces keep their indices; returns the ring faces.
pub fn inset(mesh: &mut Mesh, faces: &[u32], thickness: f32) -> Result<Vec<u32>> {
    crate::uv::with_uvs(mesh, |mesh| inset_inner(mesh, faces, thickness))
}

fn inset_inner(mesh: &mut Mesh, faces: &[u32], thickness: f32) -> Result<Vec<u32>> {
    let selected = check_faces(mesh, faces)?;
    finite("thickness", thickness)?;
    let mut ring = Vec::new();
    for face_index in selected {
        let Some(face) = mesh.faces.get(face_index as usize).cloned() else { continue };
        let points = mesh.face_positions(&face);
        let normal = mesh.face_normal(&face);
        let count = points.len();
        let mut inner = Vec::with_capacity(count);
        for corner in 0..count {
            let (Some(previous), Some(current), Some(next)) =
                (points.get((corner + count - 1) % count), points.get(corner), points.get((corner + 1) % count))
            else {
                continue;
            };
            let incoming = normal.cross((*current - *previous).normalize_or_zero());
            let outgoing = normal.cross((*next - *current).normalize_or_zero());
            let bisector = (incoming + outgoing).normalize_or_zero();
            let cosine = bisector.dot(incoming).max(0.1);
            inner.push(*current + bisector * (thickness / cosine));
        }
        let first = mesh.vertex_count();
        mesh.positions.extend(inner);
        let inner_face: Vec<u32> = (0..count).map(|corner| first + u32::try_from(corner).unwrap_or(0)).collect();
        for corner in 0..count {
            let next = (corner + 1) % count;
            let (Some(&outer_a), Some(&outer_b), Some(&inner_a), Some(&inner_b)) =
                (face.get(corner), face.get(next), inner_face.get(corner), inner_face.get(next))
            else {
                continue;
            };
            ring.push(u32::try_from(mesh.faces.len()).unwrap_or(u32::MAX));
            mesh.faces.push(vec![outer_a, outer_b, inner_b, inner_a]);
        }
        if let Some(slot) = mesh.faces.get_mut(face_index as usize) {
            *slot = inner_face;
        }
    }
    Ok(ring)
}

/// Deletes faces, keeping their vertices only where other faces or edges still use them.
pub fn delete_faces(mesh: &mut Mesh, faces: &[u32]) -> Result<()> {
    crate::uv::with_uvs(mesh, |mesh| delete_faces_inner(mesh, faces))
}

fn delete_faces_inner(mesh: &mut Mesh, faces: &[u32]) -> Result<()> {
    let selected = check_faces(mesh, faces)?;
    let mut index = 0u32;
    mesh.faces.retain(|_| {
        let keep = !selected.contains(&index);
        index += 1;
        keep
    });
    mesh.remove_unused_vertices();
    Ok(())
}

/// Deletes vertices with every face and edge that uses them.
pub fn delete_vertices(mesh: &mut Mesh, vertices: &[u32]) -> Result<()> {
    crate::uv::with_uvs(mesh, |mesh| delete_vertices_inner(mesh, vertices))
}

fn delete_vertices_inner(mesh: &mut Mesh, vertices: &[u32]) -> Result<()> {
    let selected = check_vertices(mesh, vertices)?;
    let mut remap: Vec<u32> = Vec::with_capacity(mesh.positions.len());
    let mut kept = Vec::new();
    for (index, point) in mesh.positions.iter().enumerate() {
        if selected.contains(&u32::try_from(index).unwrap_or(u32::MAX)) {
            remap.push(u32::MAX);
        } else {
            remap.push(u32::try_from(kept.len()).unwrap_or(u32::MAX));
            kept.push(*point);
        }
    }
    mesh.faces.retain(|face| face.iter().all(|vertex| !selected.contains(vertex)));
    mesh.loose_edges.retain(|edge| edge.iter().all(|vertex| !selected.contains(vertex)));
    mesh.positions = kept;
    mesh.reindex(&remap);
    Ok(())
}

/// Merges vertices into one at their centre. Returns the merged vertex's index.
pub fn merge_at_center(mesh: &mut Mesh, vertices: &[u32]) -> Result<u32> {
    crate::uv::with_uvs(mesh, |mesh| merge_at_center_inner(mesh, vertices))
}

fn merge_at_center_inner(mesh: &mut Mesh, vertices: &[u32]) -> Result<u32> {
    let selected = check_vertices(mesh, vertices)?;
    let Some(&target) = selected.first() else {
        return Err(SceneError::params("no vertices to merge"));
    };
    let centre = selected.iter().filter_map(|vertex| mesh.positions.get(*vertex as usize)).copied().sum::<Vec3>() / selected.len() as f32;
    if let Some(point) = mesh.positions.get_mut(target as usize) {
        *point = centre;
    }
    let remap: Vec<u32> = (0..mesh.vertex_count()).map(|vertex| if selected.contains(&vertex) { target } else { vertex }).collect();
    mesh.reindex(&remap);
    // Every vertex before `target` is still used (merged ones all come after it), so removing
    // the unused ones leaves its index alone.
    mesh.remove_unused_vertices();
    Ok(target)
}

/// Reverses the faces' winding.
pub fn flip(mesh: &mut Mesh, faces: &[u32]) -> Result<()> {
    crate::uv::with_uvs(mesh, |mesh| flip_inner(mesh, faces))
}

fn flip_inner(mesh: &mut Mesh, faces: &[u32]) -> Result<()> {
    for face in check_faces(mesh, faces)? {
        if let Some(corners) = mesh.faces.get_mut(face as usize) {
            corners.reverse();
        }
    }
    Ok(())
}

/// Makes the faces' windings agree across shared edges, then turns each closed piece so its
/// normals point outwards (positive volume).
pub fn recalculate_outside(mesh: &mut Mesh) {
    crate::uv::with_uvs(mesh, recalculate_outside_inner)
}

fn recalculate_outside_inner(mesh: &mut Mesh) {
    let mut by_edge: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (index, face) in mesh.faces.iter().enumerate() {
        for (a, b) in face_edges(face) {
            by_edge.entry((a.min(b), a.max(b))).or_default().push(index);
        }
    }
    let count = mesh.faces.len();
    let mut seen = vec![false; count];
    for start in 0..count {
        if seen.get(start).copied().unwrap_or(true) {
            continue;
        }
        let mut piece = Vec::new();
        let mut queue = VecDeque::from([start]);
        if let Some(flag) = seen.get_mut(start) {
            *flag = true;
        }
        while let Some(current) = queue.pop_front() {
            piece.push(current);
            let Some(face) = mesh.faces.get(current).cloned() else { continue };
            for (a, b) in face_edges(&face) {
                for &other in by_edge.get(&(a.min(b), a.max(b))).map_or(&[][..], Vec::as_slice) {
                    if seen.get(other).copied().unwrap_or(true) {
                        continue;
                    }
                    if let Some(flag) = seen.get_mut(other) {
                        *flag = true;
                    }
                    // A neighbour that runs the shared edge the same way is flipped.
                    let same = mesh.faces.get(other).is_some_and(|corners| face_edges(corners).any(|(c, d)| c == a && d == b));
                    if same && let Some(corners) = mesh.faces.get_mut(other) {
                        corners.reverse();
                    }
                    queue.push_back(other);
                }
            }
        }
        let volume: f32 = piece
            .iter()
            .filter_map(|face| mesh.faces.get(*face))
            .flat_map(|face| {
                let points = mesh.face_positions(face);
                let first = points.first().copied().unwrap_or(Vec3::ZERO);
                points
                    .windows(2)
                    .skip(1)
                    .map(move |pair| match pair {
                        [b, c] => first.dot(b.cross(*c)),
                        _ => 0.0,
                    })
                    .collect::<Vec<_>>()
            })
            .sum();
        if volume < 0.0 {
            for face in piece {
                if let Some(corners) = mesh.faces.get_mut(face) {
                    corners.reverse();
                }
            }
        }
    }
}

/// Splits each face once: a vertex in the middle of each of its edges (shared with the
/// neighbours, which gain it as a corner) and, for quads and larger, one in its centre.
/// Returns the faces that replaced the selected ones.
pub fn subdivide(mesh: &mut Mesh, faces: &[u32]) -> Result<Vec<u32>> {
    crate::uv::with_uvs(mesh, |mesh| subdivide_inner(mesh, faces))
}

fn subdivide_inner(mesh: &mut Mesh, faces: &[u32]) -> Result<Vec<u32>> {
    let selected = check_faces(mesh, faces)?;
    let mut middles: HashMap<(u32, u32), u32> = HashMap::new();
    for face in selected.iter().filter_map(|face| mesh.faces.get(*face as usize)).cloned().collect::<Vec<_>>() {
        for (a, b) in face_edges(&face) {
            let key = (a.min(b), a.max(b));
            if let std::collections::hash_map::Entry::Vacant(e) = middles.entry(key) {
                let point = (mesh.position(a)? + mesh.position(b)?) * 0.5;
                e.insert(mesh.vertex_count());
                mesh.positions.push(point);
            }
        }
    }
    let middle = |a: u32, b: u32| middles.get(&(a.min(b), a.max(b))).copied();
    let mut made = Vec::new();
    let old = std::mem::take(&mut mesh.faces);
    let mut next: Vec<Vec<u32>> = Vec::with_capacity(old.len() * 2);
    for (index, face) in old.into_iter().enumerate() {
        let index = u32::try_from(index).unwrap_or(u32::MAX);
        if !selected.contains(&index) {
            // Neighbours take the new edge vertices as corners so the surface stays closed.
            let mut corners = Vec::with_capacity(face.len() * 2);
            for (a, b) in face_edges(&face) {
                corners.push(a);
                if let Some(m) = middle(a, b) {
                    corners.push(m);
                }
            }
            next.push(corners);
            continue;
        }
        let count = face.len();
        let mids: Vec<u32> = face_edges(&face).filter_map(|(a, b)| middle(a, b)).collect();
        if count == 3 {
            if let ([a, b, c], [ab, bc, ca]) = (face.as_slice(), mids.as_slice()) {
                for triangle in [[*a, *ab, *ca], [*ab, *b, *bc], [*ca, *bc, *c], [*ab, *bc, *ca]] {
                    made.push(u32::try_from(next.len()).unwrap_or(u32::MAX));
                    next.push(triangle.to_vec());
                }
            }
            continue;
        }
        let centre = mesh.vertex_count();
        mesh.positions.push(mesh.face_center(&face));
        for corner in 0..count {
            let (Some(&current), Some(&after), Some(&before)) = (face.get(corner), mids.get(corner), mids.get((corner + count - 1) % count)) else {
                continue;
            };
            made.push(u32::try_from(next.len()).unwrap_or(u32::MAX));
            next.push(vec![current, after, centre, before]);
        }
    }
    mesh.faces = next;
    Ok(made)
}

/// The ring of quads a loop cut across `edge` runs through: from the edge, across each quad to
/// its opposite edge, both ways, until a non-quad, an open edge, or the start again. Returns the
/// crossed edges in order and whether the ring closes.
pub fn edge_ring(mesh: &Mesh, edge: (u32, u32)) -> Result<(Vec<(u32, u32)>, bool)> {
    let key = |a: u32, b: u32| (a.min(b), a.max(b));
    let mut by_edge: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (index, face) in mesh.faces.iter().enumerate() {
        for (a, b) in face_edges(face) {
            by_edge.entry(key(a, b)).or_default().push(index);
        }
    }
    let start = key(edge.0, edge.1);
    let Some(first_faces) = by_edge.get(&start).cloned() else {
        return Err(SceneError::not_found(format!("edge {}-{}", edge.0, edge.1)));
    };
    let opposite = |face: usize, across: (u32, u32)| -> Option<(u32, u32)> {
        let corners = mesh.faces.get(face)?;
        if corners.len() != 4 {
            return None;
        }
        let position = (0..4).find(|slot| {
            let (Some(&a), Some(&b)) = (corners.get(*slot), corners.get((slot + 1) % 4)) else { return false };
            key(a, b) == across
        })?;
        let (Some(&c), Some(&d)) = (corners.get((position + 2) % 4), corners.get((position + 3) % 4)) else { return None };
        Some(key(c, d))
    };
    let walk = |from_face: usize| -> (Vec<(u32, u32)>, bool) {
        let mut out = Vec::new();
        let (mut face, mut across) = (from_face, start);
        let mut visited = BTreeSet::from([start]);
        loop {
            let Some(next) = opposite(face, across) else { return (out, false) };
            if next == start {
                return (out, true);
            }
            if !visited.insert(next) {
                return (out, false);
            }
            out.push(next);
            let Some(other) = by_edge.get(&next).and_then(|faces| faces.iter().find(|candidate| **candidate != face).copied()) else {
                return (out, false);
            };
            face = other;
            across = next;
        }
    };
    let mut ring = vec![start];
    let Some(&forward_face) = first_faces.first() else {
        return Ok((ring, false));
    };
    let (forward, closed) = walk(forward_face);
    ring.extend(forward);
    if !closed && let Some(&backward_face) = first_faces.get(1) {
        let (backward, _) = walk(backward_face);
        let mut before: Vec<(u32, u32)> = backward.into_iter().filter(|edge| !ring.contains(edge)).collect();
        before.reverse();
        before.extend(ring);
        ring = before;
    }
    Ok((ring, closed))
}

/// Cuts the quad ring across `edge` with `cuts` evenly spaced edge loops (`factor` slides a
/// single cut between the two sides, 0.5 is the middle). Returns the new vertices.
pub fn loop_cut(mesh: &mut Mesh, edge: (u32, u32), cuts: u32, factor: f32) -> Result<Vec<u32>> {
    crate::uv::with_uvs(mesh, |mesh| loop_cut_inner(mesh, edge, cuts, factor))
}

fn loop_cut_inner(mesh: &mut Mesh, edge: (u32, u32), cuts: u32, factor: f32) -> Result<Vec<u32>> {
    if !(1..=64).contains(&cuts) {
        return Err(SceneError::params("cuts must be between 1 and 64"));
    }
    finite("factor", factor)?;
    let (ring, _) = edge_ring(mesh, edge)?;
    // Each crossed edge gets `cuts` new vertices, ordered from the edge's low end; the ring's
    // orientation decides which end is "first" so loops line up.
    let mut along: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    let mut made = Vec::new();
    let fractions: Vec<f32> = if cuts == 1 { vec![factor.clamp(0.0, 1.0)] } else { (1..=cuts).map(|cut| cut as f32 / (cuts + 1) as f32).collect() };
    for (a, b) in &ring {
        let (pa, pb) = (mesh.position(*a)?, mesh.position(*b)?);
        let mut list = Vec::new();
        for t in &fractions {
            list.push(mesh.vertex_count());
            made.push(mesh.vertex_count());
            mesh.positions.push(pa.lerp(pb, *t));
        }
        along.insert((*a, *b), list);
    }
    let ring_set: BTreeSet<(u32, u32)> = ring.iter().copied().collect();
    let points_on = |a: u32, b: u32| -> Vec<u32> {
        // The new vertices from a to b.
        let low_first = along.get(&(a.min(b), a.max(b))).cloned().unwrap_or_default();
        if a < b { low_first } else { low_first.into_iter().rev().collect() }
    };
    let old = std::mem::take(&mut mesh.faces);
    for face in old {
        let crossed: Vec<usize> = (0..face.len())
            .filter(|slot| {
                let (Some(&a), Some(&b)) = (face.get(*slot), face.get((slot + 1) % face.len())) else { return false };
                ring_set.contains(&(a.min(b), a.max(b)))
            })
            .collect();
        if face.len() == 4
            && crossed.len() == 2
            && let (Some(&first), Some(&second)) = (crossed.first(), crossed.get(1))
            && second == first + 2
        {
            // Edge `first` runs a→b, edge `second` runs c→d (= the far side, reversed).
            let corner = |slot: usize| face.get(slot % 4).copied().unwrap_or(0);
            let (a, b, c, d) = (corner(first), corner(first + 1), corner(first + 2), corner(first + 3));
            let near = points_on(a, b);
            let far: Vec<u32> = points_on(d, c);
            let mut left = (a, d);
            for (n, f) in near.iter().zip(&far) {
                mesh.faces.push(vec![left.0, *n, *f, left.1]);
                left = (*n, *f);
            }
            mesh.faces.push(vec![left.0, b, c, left.1]);
            continue;
        }
        // Faces at the ring's open ends take the new vertices as corners.
        let mut corners = Vec::with_capacity(face.len() + crossed.len() * cuts as usize);
        for (a, b) in face_edges(&face) {
            corners.push(a);
            if ring_set.contains(&(a.min(b), a.max(b))) {
                corners.extend(points_on(a, b));
            }
        }
        mesh.faces.push(corners);
    }
    Ok(made)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modifiers::boundary_edges;
    use crate::primitives::Primitive;

    fn cube() -> Mesh {
        Primitive::Cube { size: 2.0 }.build().unwrap()
    }

    fn top_face(mesh: &Mesh) -> u32 {
        mesh.faces.iter().position(|face| mesh.face_normal(face).z > 0.99).unwrap() as u32
    }

    #[test]
    fn extruding_the_top_of_a_cube_adds_four_walls() {
        let mut mesh = cube();
        let top = top_face(&mesh);
        let sides = extrude_region(&mut mesh, &[top], 1.0, Vec3::ZERO).unwrap();
        assert_eq!(sides.len(), 4);
        assert_eq!(mesh.positions.len(), 12);
        assert_eq!(mesh.faces.len(), 10);
        assert!(boundary_edges(&mesh).is_empty(), "still closed");
        assert!((mesh.face_center(&mesh.faces[top as usize]).z - 2.0).abs() < 1e-5);
        let mut check = mesh.clone();
        recalculate_outside(&mut check);
        assert_eq!(check.faces, mesh.faces, "every face already points out");
    }

    #[test]
    fn inset_keeps_the_face_and_adds_a_ring() {
        let mut mesh = cube();
        let top = top_face(&mesh);
        let ring = inset(&mut mesh, &[top], 0.25).unwrap();
        assert_eq!(ring.len(), 4);
        let inner = mesh.face_positions(&mesh.faces[top as usize]);
        for point in inner {
            assert!((point.x.abs() - 0.75).abs() < 1e-5 && (point.y.abs() - 0.75).abs() < 1e-5);
        }
        assert!(boundary_edges(&mesh).is_empty());
    }

    #[test]
    fn deleting_a_face_opens_the_cube() {
        let mut mesh = cube();
        delete_faces(&mut mesh, &[0]).unwrap();
        assert_eq!(mesh.faces.len(), 5);
        assert_eq!(boundary_edges(&mesh).len(), 4);
        delete_vertices(&mut mesh, &[0]).unwrap();
        assert_eq!(mesh.positions.len(), 7);
    }

    #[test]
    fn recalculating_fixes_flipped_faces() {
        let mut mesh = cube();
        mesh.faces[2].reverse();
        mesh.flip_all();
        recalculate_outside(&mut mesh);
        assert_eq!(mesh.faces.iter().filter(|face| mesh.face_normal(face).dot(mesh.face_center(face)) > 0.0).count(), 6);
    }

    #[test]
    fn subdividing_one_face_keeps_the_surface_closed() {
        let mut mesh = cube();
        let top = top_face(&mesh);
        let made = subdivide(&mut mesh, &[top]).unwrap();
        assert_eq!(made.len(), 4);
        assert_eq!(mesh.faces.len(), 9);
        assert!(boundary_edges(&mesh).is_empty());
    }

    #[test]
    fn a_loop_cut_runs_around_the_cube() {
        let mut mesh = cube();
        // The vertical edge from (-1,-1,-1) to (-1,-1,1).
        let (ring, closed) = edge_ring(&mesh, (0, 1)).unwrap();
        assert!(closed);
        assert_eq!(ring.len(), 4);
        let made = loop_cut(&mut mesh, (0, 1), 1, 0.5).unwrap();
        assert_eq!(made.len(), 4);
        assert_eq!(mesh.faces.len(), 10);
        assert!(boundary_edges(&mesh).is_empty());
        for vertex in made {
            assert!(mesh.positions[vertex as usize].z.abs() < 1e-6);
        }
    }

    #[test]
    fn merging_joins_vertices() {
        let mut mesh = cube();
        let merged = merge_at_center(&mut mesh, &[1, 3, 5, 7]).unwrap();
        assert_eq!(mesh.positions.len(), 5);
        assert!((mesh.positions[merged as usize] - Vec3::new(0.0, 0.0, 1.0)).length() < 1e-5);
    }

    #[test]
    fn bad_indices_are_refused() {
        let mut mesh = cube();
        assert!(matches!(extrude_region(&mut mesh, &[99], 1.0, Vec3::ZERO), Err(SceneError::NotFound(_))));
        assert!(translate(&mut mesh, &[0], Vec3::NAN).is_err());
    }
}
