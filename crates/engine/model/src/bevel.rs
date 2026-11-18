//! Edge bevel: selected edges are cut back by `width` on both faces (measured square to the edge,
//! a modeller's "Offset" width) and replaced by a strip of `segments` faces following a round
//! profile; where selected edges meet at a vertex the gap left is filled with a corner patch.
//!
//! Supported: manifold edges between exactly two faces, whose vertices have closed fans of faces,
//! and where every unselected edge at a bevelled vertex borders a face with a selected edge (so
//! the cut can slide along it). Other configurations are refused, never half-built.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use glam::Vec3;

use crate::error::{Result, SceneError};
use crate::mesh::{Mesh, face_edges};
use crate::modifiers::boundary_edges;

fn key(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}

fn unsupported(message: impl Into<String>) -> SceneError {
    SceneError::Unsupported(message.into())
}

/// Where a new point sits around a bevelled vertex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Slot {
    /// Inside a face whose two edges at the vertex are both bevelled.
    Face(usize),
    /// On the unselected edge from the vertex to this vertex.
    Edge(u32),
}

/// One corner of a face at a vertex: the face, the vertex before, and the vertex after.
#[derive(Debug, Clone, Copy)]
struct Corner {
    face: usize,
    prev: u32,
    next: u32,
}

/// The offset points at a given width, and the largest share of the width that fits.
struct Layout {
    points: BTreeMap<(u32, Slot), Vec3>,
    fits: f32,
}

struct Plan<'a> {
    mesh: &'a Mesh,
    selected: BTreeSet<(u32, u32)>,
    corners: BTreeMap<u32, Vec<Corner>>,
}

impl Plan<'_> {
    fn is_selected(&self, a: u32, b: u32) -> bool {
        self.selected.contains(&key(a, b))
    }

    fn position(&self, vertex: u32) -> Result<Vec3> {
        self.mesh.position(vertex)
    }

    fn layout(&self, width: f32) -> Result<Layout> {
        let mut sums: BTreeMap<(u32, Slot), (Vec3, f32)> = BTreeMap::new();
        let mut fits = 1.0f32;
        for (&vertex, corners) in &self.corners {
            let v = self.position(vertex)?;
            for corner in corners {
                let face = self.mesh.faces.get(corner.face).ok_or_else(|| unsupported("a face is missing"))?;
                let normal = self.mesh.face_normal(face);
                let (prev, next) = (self.position(corner.prev)?, self.position(corner.next)?);
                let (length_in, length_out) = (prev.distance(v), next.distance(v));
                if normal == Vec3::ZERO || length_in < 1e-9 || length_out < 1e-9 {
                    return Err(unsupported("a face at a bevelled vertex has no area"));
                }
                let (u_in, u_out) = ((prev - v) / length_in, (next - v) / length_out);
                let n_in = u_in.cross(normal);
                let n_out = normal.cross(u_out);
                let (sel_in, sel_out) = (self.is_selected(corner.prev, vertex), self.is_selected(vertex, corner.next));
                // How far along an edge direction `u` the line `width` inside an edge with
                // inward normal `n` crosses it.
                let along = |u: Vec3, n: Vec3, limit: f32, fits: &mut f32| -> Result<f32> {
                    let cosine = u.dot(n);
                    if cosine < 1e-4 {
                        return Err(unsupported("two edges at a bevelled vertex are in line"));
                    }
                    let t = width / cosine;
                    if t > limit {
                        *fits = fits.min(limit / t);
                    }
                    Ok(t)
                };
                let mut add = |slot: Slot, point: Vec3| {
                    let entry = sums.entry((vertex, slot)).or_insert((Vec3::ZERO, 0.0));
                    entry.0 += point;
                    entry.1 += 1.0;
                };
                // An end of an edge may be cut from both of its vertices: each takes at most half.
                match (sel_in, sel_out) {
                    (true, true) => {
                        let t_in = along(u_in, n_out, length_in * 0.5, &mut fits)?;
                        let t_out = along(u_out, n_in, length_out * 0.5, &mut fits)?;
                        add(Slot::Face(corner.face), v + u_in * t_in + u_out * t_out);
                    }
                    (true, false) => {
                        let t = along(u_out, n_in, length_out * 0.5, &mut fits)?;
                        add(Slot::Edge(corner.next), v + u_out * t);
                    }
                    (false, true) => {
                        let t = along(u_in, n_out, length_in * 0.5, &mut fits)?;
                        add(Slot::Edge(corner.prev), v + u_in * t);
                    }
                    (false, false) => {}
                }
            }
        }
        let points = sums.into_iter().map(|(slot, (sum, count))| (slot, sum / count)).collect();
        Ok(Layout { points, fits })
    }
}

/// The faces around `vertex` in order (each one's next edge is the following one's previous
/// edge), or an error when they do not close around it.
fn fan(mesh: &Mesh, vertex: u32, corners: &[Corner]) -> Result<Vec<Corner>> {
    let Some(first) = corners.first().copied() else { return Err(unsupported(format!("vertex {vertex} has no faces"))) };
    let mut ordered = vec![first];
    let mut current = first;
    loop {
        let following: Vec<&Corner> = corners.iter().filter(|corner| corner.prev == current.next).collect();
        let [next] = following.as_slice() else {
            return Err(unsupported(format!("the faces around vertex {vertex} are not one closed fan")));
        };
        if next.face == first.face {
            break;
        }
        ordered.push(**next);
        current = **next;
        if ordered.len() > corners.len() {
            return Err(unsupported(format!("the faces around vertex {vertex} are not one closed fan")));
        }
    }
    if ordered.len() != corners.len() {
        return Err(unsupported(format!("vertex {vertex} joins more than one fan of faces")));
    }
    let _ = mesh;
    Ok(ordered)
}

/// Bevels `edges` (vertex pairs) by `width` with `segments` faces across each. Returns the new
/// faces (the strips and the corner patches); the faces around them keep their indices.
pub fn bevel_edges(mesh: &mut Mesh, edges: &[(u32, u32)], width: f32, segments: u32) -> Result<Vec<u32>> {
    crate::uv::with_uvs(mesh, |mesh| bevel_edges_inner(mesh, edges, width, segments))
}

fn bevel_edges_inner(mesh: &mut Mesh, edges: &[(u32, u32)], width: f32, segments: u32) -> Result<Vec<u32>> {
    if !(width.is_finite() && width > 0.0) {
        return Err(SceneError::params("the width must be a positive number"));
    }
    if !(1..=64).contains(&segments) {
        return Err(SceneError::params("segments must be between 1 and 64"));
    }
    if edges.is_empty() {
        return Err(SceneError::params("no edges to bevel"));
    }
    let mut faces_of: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (index, face) in mesh.faces.iter().enumerate() {
        for (a, b) in face_edges(face) {
            faces_of.entry(key(a, b)).or_default().push(index);
        }
    }
    let mut selected = BTreeSet::new();
    for (a, b) in edges {
        let edge = key(*a, *b);
        match faces_of.get(&edge).map(Vec::len) {
            None => return Err(SceneError::not_found(format!("edge {a}-{b}"))),
            Some(2) => {
                selected.insert(edge);
            }
            Some(_) => return Err(unsupported(format!("edge {a}-{b} is not between exactly two faces"))),
        }
    }
    let was_closed = !mesh.faces.is_empty() && boundary_edges(mesh).is_empty();

    // The corners of every bevelled vertex, in fan order.
    let bevelled: BTreeSet<u32> = selected.iter().flat_map(|(a, b)| [*a, *b]).collect();
    let mut all: BTreeMap<u32, Vec<Corner>> = BTreeMap::new();
    for (face_index, face) in mesh.faces.iter().enumerate() {
        let count = face.len();
        for slot in 0..count {
            let (Some(&prev), Some(&vertex), Some(&next)) = (face.get((slot + count - 1) % count), face.get(slot), face.get((slot + 1) % count))
            else {
                continue;
            };
            if bevelled.contains(&vertex) {
                all.entry(vertex).or_default().push(Corner { face: face_index, prev, next });
            }
        }
    }
    let mut corners = BTreeMap::new();
    for (vertex, list) in all {
        corners.insert(vertex, fan(mesh, vertex, &list)?);
    }
    let plan = Plan { mesh, selected, corners };

    // Every unselected edge at a bevelled vertex must have a face with a selected edge beside it.
    for (&vertex, list) in &plan.corners {
        for corner in list {
            let other = corner.prev;
            if plan.is_selected(other, vertex) {
                continue;
            }
            let slides = list.iter().any(|candidate| {
                (candidate.prev == other && plan.is_selected(vertex, candidate.next))
                    || (candidate.next == other && plan.is_selected(candidate.prev, vertex))
            });
            if !slides {
                return Err(unsupported(format!("vertex {vertex} has an edge with no bevelled edge beside it")));
            }
        }
    }

    // Shrink the width when cuts would cross (as a clamped bevel does).
    let first = plan.layout(width)?;
    let layout = if first.fits < 1.0 { plan.layout(width * first.fits * 0.999)? } else { first };

    let mut out = mesh.clone();
    let mut index: BTreeMap<(u32, Slot), u32> = BTreeMap::new();
    let mut owner: HashMap<u32, u32> = HashMap::new();
    let mut point = |out: &mut Mesh, owner: &mut HashMap<u32, u32>, vertex: u32, slot: Slot| -> Result<u32> {
        if let Some(found) = index.get(&(vertex, slot)) {
            return Ok(*found);
        }
        let position = layout.points.get(&(vertex, slot)).copied().ok_or_else(|| unsupported(format!("vertex {vertex} has no cut point there")))?;
        let made = out.vertex_count();
        out.positions.push(position);
        index.insert((vertex, slot), made);
        owner.insert(made, vertex);
        Ok(made)
    };

    // Faces: each corner at a bevelled vertex becomes its cut points.
    let mut sequences: HashMap<(usize, u32), Vec<u32>> = HashMap::new();
    for (&vertex, list) in &plan.corners {
        for corner in list {
            let (sel_in, sel_out) = (plan.is_selected(corner.prev, vertex), plan.is_selected(vertex, corner.next));
            let mut sequence = Vec::new();
            if !sel_in {
                sequence.push(point(&mut out, &mut owner, vertex, Slot::Edge(corner.prev))?);
            }
            if sel_in && sel_out {
                sequence.push(point(&mut out, &mut owner, vertex, Slot::Face(corner.face))?);
            }
            if !sel_out {
                sequence.push(point(&mut out, &mut owner, vertex, Slot::Edge(corner.next))?);
            }
            sequence.dedup();
            sequences.insert((corner.face, vertex), sequence);
        }
    }
    for (face_index, face) in out.faces.iter_mut().enumerate() {
        let mut rebuilt = Vec::with_capacity(face.len() + 2);
        for vertex in face.iter() {
            match sequences.get(&(face_index, *vertex)) {
                Some(sequence) => rebuilt.extend(sequence.iter().copied()),
                None => rebuilt.push(*vertex),
            }
        }
        rebuilt.dedup();
        while rebuilt.len() > 1 && rebuilt.first() == rebuilt.last() {
            rebuilt.pop();
        }
        *face = rebuilt;
    }

    // Strips along each selected edge.
    let first_new = out.faces.len();
    let mut mids: BTreeSet<u32> = BTreeSet::new();
    let mut profiles: HashMap<(u32, u32, u32), Vec<u32>> = HashMap::new();
    for &(low, high) in &plan.selected {
        let faces = faces_of.get(&(low, high)).cloned().unwrap_or_default();
        let runs_forward =
            |face: usize, a: u32, b: u32| plan.mesh.faces.get(face).is_some_and(|corners| face_edges(corners).any(|(x, y)| x == a && y == b));
        let (Some(f1), Some(f2)) =
            (faces.iter().copied().find(|face| runs_forward(*face, low, high)), faces.iter().copied().find(|face| runs_forward(*face, high, low)))
        else {
            return Err(unsupported(format!("the faces at edge {low}-{high} disagree on their winding")));
        };
        let (a, b) = (low, high);
        let side = |face: usize, vertex: u32, last: bool| -> Result<u32> {
            let sequence = sequences.get(&(face, vertex)).ok_or_else(|| unsupported("a cut is missing"))?;
            let found = if last { sequence.last() } else { sequence.first() };
            found.copied().ok_or_else(|| unsupported("a cut is empty"))
        };
        let (a1, b1) = (side(f1, a, true)?, side(f1, b, false)?);
        let (b2, a2) = (side(f2, b, true)?, side(f2, a, false)?);
        let mut profile = |out: &mut Mesh, from: u32, to: u32, corner: u32| -> Result<Vec<u32>> {
            // Two strips ending on the same pair of cut points share their profile.
            if let Some(known) = profiles.get(&(from, to, corner)) {
                return Ok(known.clone());
            }
            if let Some(known) = profiles.get(&(to, from, corner)) {
                return Ok(known.iter().rev().copied().collect());
            }
            let (start, end, origin) = (out.position(from)?, out.position(to)?, plan.position(corner)?);
            let centre = start + end - origin;
            let mut ring = vec![from];
            for step in 1..segments {
                let angle = std::f32::consts::FRAC_PI_2 * step as f32 / segments as f32;
                let made = out.vertex_count();
                out.positions.push(centre + (start - centre) * angle.cos() + (end - centre) * angle.sin());
                owner.insert(made, corner);
                mids.insert(made);
                ring.push(made);
            }
            ring.push(to);
            profiles.insert((from, to, corner), ring.clone());
            Ok(ring)
        };
        let pa = profile(&mut out, a1, a2, a)?;
        let pb = profile(&mut out, b1, b2, b)?;
        for step in 0..segments as usize {
            let (Some(&pb0), Some(&pa0), Some(&pa1), Some(&pb1)) = (pb.get(step), pa.get(step), pa.get(step + 1), pb.get(step + 1)) else { continue };
            let mut quad = vec![pb0, pa0, pa1, pb1];
            quad.dedup();
            if quad.len() >= 3 {
                out.faces.push(quad);
            }
        }
    }

    // Corner patches: whatever is still open is a hole around a vertex; walk each one back.
    let mut directed: BTreeSet<(u32, u32)> = BTreeSet::new();
    for face in &out.faces {
        for (x, y) in face_edges(face) {
            if !directed.insert((x, y)) {
                return Err(unsupported("the bevel would make an edge used twice in one direction"));
            }
        }
    }
    let mut hole_next: BTreeMap<u32, u32> = BTreeMap::new();
    for &(x, y) in &directed {
        if !directed.contains(&(y, x)) && (owner.contains_key(&x) || owner.contains_key(&y)) && hole_next.insert(y, x).is_some() {
            return Err(unsupported("the corner left by the bevel is not a simple loop"));
        }
    }
    let mut visited = BTreeSet::new();
    let starts: Vec<u32> = hole_next.keys().copied().collect();
    for start in starts {
        if visited.contains(&start) {
            continue;
        }
        let mut ring = vec![start];
        visited.insert(start);
        let mut at = start;
        loop {
            let Some(&next) = hole_next.get(&at) else { return Err(unsupported("the corner left by the bevel does not close")) };
            if next == start {
                break;
            }
            if !visited.insert(next) {
                return Err(unsupported("the corner left by the bevel crosses itself"));
            }
            ring.push(next);
            at = next;
        }
        // A hole bounded by one old face and strip ends belongs to that face: the profile points
        // become corners of it.
        let count = ring.len();
        let old_sides: Vec<(usize, usize)> = (0..count)
            .filter_map(|step| {
                let (x, y) = (ring.get(step).copied()?, ring.get((step + 1) % count).copied()?);
                let face = out.faces.iter().take(first_new).position(|corners| face_edges(corners).any(|(a, b)| a == y && b == x))?;
                Some((step, face))
            })
            .collect();
        if let [(step, face)] = old_sides.as_slice() {
            let rest: Vec<u32> = (2..count).filter_map(|offset| ring.get((step + offset) % count).copied()).collect();
            let x = ring.get(*step).copied().unwrap_or(0);
            if let Some(corners) = out.faces.get_mut(*face)
                && let Some(at) = corners.iter().position(|vertex| *vertex == x)
            {
                // The face runs y -> x; the hole's own path from y to x goes in between.
                for (offset, vertex) in rest.iter().enumerate() {
                    corners.insert(at + offset, *vertex);
                }
            }
            continue;
        }
        fill(&mut out, &ring, segments, &mids, &owner, plan.mesh)?;
    }

    let made: Vec<u32> = (first_new..out.faces.len()).map(|face| u32::try_from(face).unwrap_or(u32::MAX)).collect();
    let face_count = out.faces.len();
    out.remove_unused_vertices();
    if out.faces.len() != face_count {
        return Err(unsupported("the bevel collapsed a face"));
    }
    out.validate()?;
    if was_closed && !boundary_edges(&out).is_empty() {
        return Err(unsupported("the bevel would open the mesh"));
    }
    *mesh = out;
    Ok(made)
}

/// Closes the hole `ring` (already in face order) around a bevelled vertex.
fn fill(out: &mut Mesh, ring: &[u32], segments: u32, mids: &BTreeSet<u32>, owner: &HashMap<u32, u32>, original: &Mesh) -> Result<()> {
    if ring.len() < 3 {
        return Err(unsupported("a corner left by the bevel has fewer than three sides"));
    }
    if segments == 1 || ring.len() <= 4 {
        out.faces.push(ring.to_vec());
        return Ok(());
    }
    // A centre point, pulled from the ring's mean towards the original vertex so the patch bulges.
    let points: Vec<Vec3> = ring.iter().map(|vertex| out.position(*vertex)).collect::<Result<_>>()?;
    let mean = points.iter().copied().sum::<Vec3>() / points.len() as f32;
    let corner = ring.iter().find_map(|vertex| owner.get(vertex)).and_then(|vertex| original.position(*vertex).ok()).unwrap_or(mean);
    let centre_index = out.vertex_count();
    out.positions.push(mean + (corner - mean) * 0.3);
    let count = ring.len();
    let start = ring.iter().position(|vertex| !mids.contains(vertex)).unwrap_or(0);
    let alternates = count.is_multiple_of(2)
        && (0..count).all(|step| ring.get((start + step) % count).is_some_and(|vertex| mids.contains(vertex) == (step % 2 == 1)));
    if segments == 2 && alternates {
        for pair in 0..count / 2 {
            let at = |offset: usize| ring.get((start + 2 * pair + offset) % count).copied().unwrap_or(0);
            out.faces.push(vec![at(0), at(1), at(2), centre_index]);
        }
    } else {
        for step in 0..count {
            let (Some(&a), Some(&b)) = (ring.get(step), ring.get((step + 1) % count)) else { continue };
            out.faces.push(vec![a, b, centre_index]);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::Primitive;

    fn cube() -> Mesh {
        Primitive::Cube { size: 2.0 }.build().unwrap()
    }

    fn counts(mesh: &Mesh) -> (usize, usize, usize) {
        (mesh.positions.len(), mesh.edges().len(), mesh.faces.len())
    }

    fn closed_and_outward(mesh: &Mesh) {
        assert!(boundary_edges(mesh).is_empty(), "the bevel left the mesh open");
        let mut oriented = mesh.clone();
        crate::ops::recalculate_outside(&mut oriented);
        assert_eq!(oriented.faces, mesh.faces, "a face points the wrong way");
        let bounds = mesh.bounds(glam::Mat4::IDENTITY);
        assert!(bounds.min.abs_diff_eq(Vec3::splat(-1.0), 1e-5) && bounds.max.abs_diff_eq(Vec3::splat(1.0), 1e-5), "{bounds:?}");
    }

    // Counts from the reference application's Bevel (width 0.1, Offset, profile 0.5).
    #[test]
    fn every_cube_edge_matches_the_reference() {
        for (segments, expected) in [(1, (24, 48, 26)), (2, (56, 108, 54))] {
            let mut mesh = cube();
            let edges: Vec<(u32, u32)> = mesh.edges().into_iter().map(|[a, b]| (a, b)).collect();
            bevel_edges(&mut mesh, &edges, 0.1, segments).unwrap();
            assert_eq!(counts(&mesh), expected, "segments {segments}");
            closed_and_outward(&mesh);
        }
    }

    #[test]
    fn one_edge_and_a_loop_match_the_reference() {
        // The vertical edge at x = 1, y = 1 (vertices 6 and 7).
        for (segments, expected) in [(1, (10, 15, 7)), (2, (12, 18, 8)), (3, (14, 21, 9))] {
            let mut mesh = cube();
            bevel_edges(&mut mesh, &[(6, 7)], 0.1, segments).unwrap();
            assert_eq!(counts(&mesh), expected, "one edge, segments {segments}");
            closed_and_outward(&mesh);
        }
        // The four edges round the top (z = 1: vertices 1, 3, 5, 7).
        for (segments, expected) in [(1, (12, 20, 10)), (2, (16, 28, 14)), (3, (20, 36, 18))] {
            let mut mesh = cube();
            bevel_edges(&mut mesh, &[(1, 3), (3, 7), (7, 5), (5, 1)], 0.1, segments).unwrap();
            assert_eq!(counts(&mesh), expected, "top loop, segments {segments}");
            closed_and_outward(&mesh);
        }
    }

    #[test]
    fn a_cylinder_cap_loop_matches_the_reference() {
        for (segments, expected) in [(1, (24, 40, 18)), (2, (32, 56, 26)), (3, (40, 72, 34))] {
            let mut mesh = Primitive::Cylinder { vertices: 8, radius: 1.0, depth: 2.0, fill: crate::primitives::Fill::Ngon }.build().unwrap();
            let top: Vec<(u32, u32)> = mesh
                .edges()
                .into_iter()
                .filter(|[a, b]| mesh.positions[*a as usize].z > 0.99 && mesh.positions[*b as usize].z > 0.99)
                .map(|[a, b]| (a, b))
                .collect();
            assert_eq!(top.len(), 8);
            bevel_edges(&mut mesh, &top, 0.1, segments).unwrap();
            assert_eq!(counts(&mesh), expected, "segments {segments}");
            assert!(boundary_edges(&mesh).is_empty());
        }
    }

    #[test]
    fn the_cut_is_the_width_from_the_edge() {
        let mut mesh = cube();
        bevel_edges(&mut mesh, &[(6, 7)], 0.25, 1).unwrap();
        // The cut points on the faces x = 1 and y = 1 sit 0.25 in from the old edge.
        let near: Vec<Vec3> = mesh.positions.iter().copied().filter(|point| (point.x - 0.75).abs() < 1e-5 || (point.y - 0.75).abs() < 1e-5).collect();
        assert_eq!(near.len(), 4, "{near:?}");
    }

    #[test]
    fn three_segments_stay_closed() {
        let mut mesh = cube();
        let edges: Vec<(u32, u32)> = mesh.edges().into_iter().map(|[a, b]| (a, b)).collect();
        bevel_edges(&mut mesh, &edges, 0.2, 3).unwrap();
        closed_and_outward(&mesh);
    }

    #[test]
    fn too_wide_a_bevel_is_clamped() {
        let mut mesh = cube();
        let edges: Vec<(u32, u32)> = mesh.edges().into_iter().map(|[a, b]| (a, b)).collect();
        bevel_edges(&mut mesh, &edges, 5.0, 1).unwrap();
        mesh.validate().unwrap();
        assert!(boundary_edges(&mesh).is_empty());
    }

    #[test]
    fn open_and_unknown_edges_are_refused() {
        let mut plane = Primitive::Plane { size: 2.0 }.build().unwrap();
        assert!(matches!(bevel_edges(&mut plane, &[(0, 1)], 0.1, 1), Err(SceneError::Unsupported(_))));
        let mut mesh = cube();
        assert!(matches!(bevel_edges(&mut mesh, &[(0, 7)], 0.1, 1), Err(SceneError::NotFound(_))));
        assert!(bevel_edges(&mut mesh, &[(6, 7)], f32::NAN, 1).is_err());
        assert_eq!(mesh, cube(), "a refused bevel changes nothing");
    }
}
