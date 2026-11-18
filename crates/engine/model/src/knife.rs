//! The knife and edge slide.
//!
//! The knife cuts faces along a line drawn over the view: each straight segment of the line
//! sweeps a plane through the view's rays at its ends, faces it crosses are split where the
//! plane meets their edges (between the segment's ends on screen), and the new vertices are
//! shared with the neighbouring faces so the surface stays closed. Only faces turned towards the
//! viewer are cut unless the cut goes through. A point of the line that lands inside a face
//! becomes a vertex there: a face the line enters, bends in, and leaves is split along that path;
//! a cut that starts or stops inside a face leaves its last stretch there as loose edges, as the
//! reference application's knife leaves a dangling cut.
//!
//! Edge slide moves the vertices of selected edges along the edges that leave them sideways,
//! to one side (positive factors) or the other.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use glam::{Vec2, Vec3};

use crate::error::{Result, SceneError};
use crate::mesh::{Mesh, face_edges};

/// How the knife sees the mesh: from a mesh position to the screen (in pixels, or `None` behind
/// the eye), and from a screen point to its view ray (origin and direction, in mesh space).
pub struct View<'a> {
    pub project: &'a dyn Fn(Vec3) -> Option<Vec2>,
    pub ray: &'a dyn Fn(Vec2) -> (Vec3, Vec3),
}

/// The largest number of segments one cut takes.
const MAX_SEGMENTS: usize = 256;

/// Cuts along the screen polyline `line`. With `through`, faces turned away are cut too.
/// Answers the new vertices.
pub fn knife(mesh: &mut Mesh, line: &[Vec2], view: &View, through: bool) -> Result<BTreeSet<u32>> {
    if line.len() < 2 {
        return Err(SceneError::params("a knife line needs at least two points"));
    }
    if line.len() > MAX_SEGMENTS + 1 || line.iter().any(|point| !point.is_finite()) {
        return Err(SceneError::params("the knife line is out of range"));
    }
    let mut made = BTreeSet::new();
    crate::uv::with_uvs(mesh, |mesh| {
        // Where each point of the line lands on the surface (before any cut moves faces about).
        let hits: Vec<Option<Vec3>> = line.iter().map(|point| hit(mesh, view, *point, through)).collect();
        let mut crossings: Vec<BTreeMap<u32, f32>> = Vec::with_capacity(line.len());
        for pair in line.windows(2) {
            if let [from, to] = pair {
                if from.distance(*to) < 1e-6 {
                    crossings.push(BTreeMap::new());
                    continue;
                }
                let cut = cut_segment(mesh, *from, *to, view, through);
                made.extend(cut.keys().copied());
                crossings.push(cut);
            }
        }
        made.extend(interior(mesh, &hits, &crossings));
    });
    Ok(made)
}

/// The plane swept by the view rays at `from` and `to`: a point on it and its normal.
fn plane(view: &View, from: Vec2, to: Vec2) -> Option<(Vec3, Vec3)> {
    let (origin_a, direction_a) = (view.ray)(from);
    let (origin_b, direction_b) = (view.ray)(to);
    let p1 = origin_a;
    let p2 = origin_a + direction_a;
    let p3 = origin_b + direction_b;
    let mut normal = (p2 - p1).cross(p3 - p1);
    if normal.length_squared() < 1e-20 {
        normal = direction_a.cross(origin_b - origin_a);
    }
    let normal = normal.normalize_or_zero();
    (normal != Vec3::ZERO && normal.is_finite()).then_some((p1, normal))
}

/// Where `point` falls along the screen segment (0 at `from`, 1 at `to`), if it projects.
fn along(view: &View, point: Vec3, from: Vec2, to: Vec2) -> Option<f32> {
    let screen = (view.project)(point)?;
    let span = to - from;
    Some((screen - from).dot(span) / span.length_squared())
}

/// The surface point the view ray at screen `point` meets first, on a face turned towards the
/// viewer (any face with `through`).
fn hit(mesh: &Mesh, view: &View, point: Vec2, through: bool) -> Option<Vec3> {
    let (origin, direction) = (view.ray)(point);
    let mut best: Option<f32> = None;
    for (triangle, face) in mesh.triangles() {
        let Some(corners) = mesh.faces.get(face as usize) else { continue };
        if !through && mesh.face_normal(corners).dot(-direction) <= 0.0 {
            continue;
        }
        let [a, b, c] = triangle.map(|vertex| mesh.positions.get(vertex as usize).copied().unwrap_or(Vec3::ZERO));
        // Möller–Trumbore.
        let (e1, e2) = (b - a, c - a);
        let p = direction.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-12 {
            continue;
        }
        let inv = 1.0 / det;
        let s = origin - a;
        let u = s.dot(p) * inv;
        let q = s.cross(e1);
        let v = direction.dot(q) * inv;
        let t = e2.dot(q) * inv;
        if u >= 0.0 && v >= 0.0 && u + v <= 1.0 && t > 1e-6 && best.is_none_or(|nearest| t < nearest) {
            best = Some(t);
        }
    }
    best.map(|t| origin + direction * t)
}

/// The face `point` lies inside (on its plane, within its outline, off its edges).
fn face_containing(mesh: &Mesh, point: Vec3, eps: f32) -> Option<usize> {
    mesh.faces.iter().position(|corners| {
        let normal = mesh.face_normal(corners);
        let points = mesh.face_positions(corners);
        let Some(first) = points.first() else { return false };
        if normal == Vec3::ZERO || (point - *first).dot(normal).abs() > eps {
            return false;
        }
        let (u, v) = normal.any_orthonormal_pair();
        let flat = |p: Vec3| Vec2::new(p.dot(u), p.dot(v));
        let at = flat(point);
        let mut inside = false;
        for (a, b) in points.iter().zip(points.iter().cycle().skip(1)) {
            let (pa, pb) = (flat(*a), flat(*b));
            // Too near an edge or corner counts as on the outline, not inside.
            let span = pb - pa;
            let t = ((at - pa).dot(span) / span.length_squared().max(1e-20)).clamp(0.0, 1.0);
            if at.distance(pa + span * t) <= eps {
                return false;
            }
            if (pa.y > at.y) != (pb.y > at.y) && at.x < pa.x + (at.y - pa.y) / (pb.y - pa.y) * (pb.x - pa.x) {
                inside = !inside;
            }
        }
        inside
    })
}

/// Turns runs of line points inside one face into vertices: split along entry → points → exit,
/// or loose edges where the run starts or ends the line.
fn interior(mesh: &mut Mesh, hits: &[Option<Vec3>], crossings: &[BTreeMap<u32, f32>]) -> BTreeSet<u32> {
    let mut made = BTreeSet::new();
    let eps = mesh.bounds(glam::Mat4::IDENTITY).size().length().max(1e-6) * 1e-5;
    let mut index = 0;
    while index < hits.len() {
        let Some(Some(first)) = hits.get(index) else {
            index += 1;
            continue;
        };
        let Some(face) = face_containing(mesh, *first, eps) else {
            index += 1;
            continue;
        };
        let mut last = index;
        while let Some(Some(next)) = hits.get(last + 1) {
            if face_containing(mesh, *next, eps) != Some(face) {
                break;
            }
            last += 1;
        }
        let path: Vec<u32> = (index..=last)
            .filter_map(|at| hits.get(at).copied().flatten())
            .map(|point| {
                mesh.positions.push(point);
                mesh.vertex_count() - 1
            })
            .collect();
        made.extend(path.iter().copied());
        let corners = mesh.faces.get(face).cloned().unwrap_or_default();
        let on_face = |cut: Option<&BTreeMap<u32, f32>>, last_one: bool| -> Option<u32> {
            let cut = cut?;
            let mut found: Vec<(u32, f32)> = cut.iter().filter(|(vertex, _)| corners.contains(vertex)).map(|(vertex, t)| (*vertex, *t)).collect();
            found.sort_by(|a, b| a.1.total_cmp(&b.1));
            if last_one { found.last().map(|(vertex, _)| *vertex) } else { found.first().map(|(vertex, _)| *vertex) }
        };
        let entry = if index > 0 { on_face(crossings.get(index - 1), true) } else { None };
        let exit = on_face(crossings.get(last), false);
        match (entry, exit) {
            (Some(entry), Some(exit)) if entry != exit => {
                let (Some(p), Some(q)) = (corners.iter().position(|vertex| *vertex == entry), corners.iter().position(|vertex| *vertex == exit))
                else {
                    index = last + 1;
                    continue;
                };
                let count = corners.len();
                let walk = |from: usize, to: usize| -> Vec<u32> {
                    let mut out = Vec::new();
                    let mut at = from;
                    loop {
                        out.push(corners.get(at).copied().unwrap_or(0));
                        if at == to || out.len() > count {
                            break;
                        }
                        at = (at + 1) % count;
                    }
                    out
                };
                let mut one = walk(p, q);
                one.extend(path.iter().rev().copied());
                let mut two = walk(q, p);
                two.extend(path.iter().copied());
                if let Some(slot) = mesh.faces.get_mut(face) {
                    *slot = one;
                }
                mesh.faces.push(two);
            }
            (entry, exit) => {
                let chain: Vec<u32> = entry.into_iter().chain(path.iter().copied()).chain(exit).collect();
                for pair in chain.windows(2) {
                    if let [a, b] = pair
                        && a != b
                    {
                        mesh.loose_edges.push([*a, *b]);
                    }
                }
            }
        }
        index = last + 1;
    }
    made
}

/// Cuts along one segment: the new vertices, each with where it falls along the segment.
fn cut_segment(mesh: &mut Mesh, from: Vec2, to: Vec2, view: &View, through: bool) -> BTreeMap<u32, f32> {
    let mut made = BTreeMap::new();
    let Some((origin, normal)) = plane(view, from, to) else { return made };
    let size = mesh.bounds(glam::Mat4::IDENTITY).size().length().max(1e-6);
    let eps = size * 1e-6;
    let distance = |point: Vec3| (point - origin).dot(normal);
    // Faces the cut may touch: turned towards the viewer (unless through).
    let candidates: BTreeSet<usize> = mesh
        .faces
        .iter()
        .enumerate()
        .filter(|(_, face)| {
            if through {
                return true;
            }
            let centre = mesh.face_center(face);
            let Some(screen) = (view.project)(centre) else { return false };
            let (_, direction) = (view.ray)(screen);
            mesh.face_normal(face).dot(-direction) > 0.0
        })
        .map(|(index, _)| index)
        .collect();
    // Vertices on the plane, within the segment.
    let within = |point: Vec3| along(view, point, from, to).is_some_and(|t| (-1e-4..=1.0 + 1e-4).contains(&t));
    let on_plane: BTreeSet<u32> = mesh
        .positions
        .iter()
        .enumerate()
        .filter(|(_, point)| distance(**point).abs() <= eps && within(**point))
        .map(|(index, _)| u32::try_from(index).unwrap_or(u32::MAX))
        .collect();
    // Edges of candidate faces the plane crosses within the segment: one new vertex each.
    let mut split: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for &face in &candidates {
        let Some(corners) = mesh.faces.get(face).cloned() else { continue };
        for (a, b) in face_edges(&corners) {
            let key = (a.min(b), a.max(b));
            if split.contains_key(&key) {
                continue;
            }
            let (Some(pa), Some(pb)) = (mesh.positions.get(a as usize).copied(), mesh.positions.get(b as usize).copied()) else { continue };
            let (da, db) = (distance(pa), distance(pb));
            if da.abs() <= eps || db.abs() <= eps || (da > 0.0) == (db > 0.0) {
                continue;
            }
            let point = pa.lerp(pb, da / (da - db));
            if !within(point) {
                continue;
            }
            split.insert(key, mesh.vertex_count());
            mesh.positions.push(point);
        }
    }
    for (&(a, b), &vertex) in &split {
        let _ = (a, b);
        if let Some(point) = mesh.positions.get(vertex as usize).copied() {
            made.insert(vertex, along(view, point, from, to).unwrap_or(0.0));
        }
    }
    for &vertex in &on_plane {
        if let Some(point) = mesh.positions.get(vertex as usize).copied() {
            made.insert(vertex, along(view, point, from, to).unwrap_or(0.0));
        }
    }
    // Every face gains the new vertices on its edges (neighbours too, so nothing opens).
    for face in &mut mesh.faces {
        let mut out = Vec::with_capacity(face.len() + 2);
        for (a, b) in face_edges(face) {
            out.push(a);
            if let Some(&middle) = split.get(&(a.min(b), a.max(b))) {
                out.push(middle);
            }
        }
        *face = out;
    }
    // Candidate faces with two cut points that are not neighbours split in two.
    let cut_points: BTreeSet<u32> = split.values().copied().chain(on_plane.iter().copied()).collect();
    let mut extra = Vec::new();
    for &face in &candidates {
        let Some(corners) = mesh.faces.get(face).cloned() else { continue };
        let hits: Vec<usize> = corners.iter().enumerate().filter(|(_, vertex)| cut_points.contains(vertex)).map(|(slot, _)| slot).collect();
        let [first, second] = hits.as_slice() else { continue };
        let count = corners.len();
        if second - first < 2 || count - (second - first) < 2 {
            continue;
        }
        let one: Vec<u32> = corners.get(*first..=*second).map(<[u32]>::to_vec).unwrap_or_default();
        let mut two: Vec<u32> = corners.get(*second..).map(<[u32]>::to_vec).unwrap_or_default();
        two.extend(corners.get(..=*first).into_iter().flatten().copied());
        if let Some(slot) = mesh.faces.get_mut(face) {
            *slot = one;
        }
        extra.push(two);
        for vertex in [corners.get(*first), corners.get(*second)].into_iter().flatten() {
            if let Some(point) = mesh.positions.get(*vertex as usize).copied() {
                made.entry(*vertex).or_insert_with(|| along(view, point, from, to).unwrap_or(0.0));
            }
        }
    }
    mesh.faces.extend(extra);
    made
}

/// Slides the vertices of the edges between `vertices` along the edges that leave them
/// sideways: towards the faces on one side for positive `factor`, the other side for negative,
/// `|factor|` of the way (at most 1). Answers how many vertices moved.
pub fn edge_slide(mesh: &mut Mesh, vertices: &BTreeSet<u32>, factor: f32) -> Result<usize> {
    if !factor.is_finite() || factor.abs() > 1.0 {
        return Err(SceneError::params("the slide factor must be between -1 and 1"));
    }
    if let Some(bad) = vertices.iter().find(|vertex| **vertex >= mesh.vertex_count()) {
        return Err(SceneError::not_found(format!("vertex {bad}")));
    }
    let selected: BTreeSet<(u32, u32)> =
        mesh.edges().into_iter().filter(|[a, b]| vertices.contains(a) && vertices.contains(b)).map(|[a, b]| (a, b)).collect();
    if selected.is_empty() {
        return Err(SceneError::params("select edges to slide"));
    }
    let oriented = orient(&selected);
    // For an edge oriented a→b, the face that runs a→b is on the + side; its corner before a and
    // its corner after b (when not selected) are a's and b's rails on that side. The face that
    // runs b→a gives the − rails the same way.
    let mut plus: HashMap<u32, u32> = HashMap::new();
    let mut minus: HashMap<u32, u32> = HashMap::new();
    for face in &mesh.faces {
        let count = face.len();
        for (slot, (a, b)) in face_edges(face).enumerate() {
            let side = if oriented.contains(&(a, b)) {
                &mut plus
            } else if oriented.contains(&(b, a)) {
                &mut minus
            } else {
                continue;
            };
            let before_a = face.get((slot + count - 1) % count).copied().filter(|rail| !vertices.contains(rail));
            let after_b = face.get((slot + 2) % count).copied().filter(|rail| !vertices.contains(rail));
            if let Some(rail) = before_a {
                side.entry(a).or_insert(rail);
            }
            if let Some(rail) = after_b {
                side.entry(b).or_insert(rail);
            }
        }
    }
    let rails = if factor >= 0.0 { plus } else { minus };
    let amount = factor.abs();
    let mut moved = 0;
    crate::uv::with_uvs(mesh, |mesh| {
        let targets: Vec<(u32, Vec3)> = rails.iter().filter_map(|(vertex, rail)| Some((*vertex, *mesh.positions.get(*rail as usize)?))).collect();
        for (vertex, target) in targets {
            if let Some(point) = mesh.positions.get_mut(vertex as usize) {
                *point = point.lerp(target, amount);
                moved += 1;
            }
        }
    });
    Ok(moved)
}

/// Directions for the edges such that edges meeting at a vertex continue each other (a→b, b→c),
/// so a loop or chain has one + side all the way along.
fn orient(edges: &BTreeSet<(u32, u32)>) -> BTreeSet<(u32, u32)> {
    let mut at: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
    for &(a, b) in edges {
        at.entry(a).or_default().push((a, b));
        at.entry(b).or_default().push((a, b));
    }
    let mut done: BTreeMap<(u32, u32), (u32, u32)> = BTreeMap::new();
    for &start in edges {
        if done.contains_key(&start) {
            continue;
        }
        done.insert(start, start);
        let mut queue = std::collections::VecDeque::from([start]);
        while let Some(edge) = queue.pop_front() {
            let Some(&(from, to)) = done.get(&edge) else { continue };
            for (vertex, leaving) in [(to, true), (from, false)] {
                for &next in at.get(&vertex).into_iter().flatten() {
                    if done.contains_key(&next) {
                        continue;
                    }
                    let other = if next.0 == vertex { next.1 } else { next.0 };
                    // Leaving `to` continues forward; arriving at `from` comes before.
                    let direction = if leaving { (vertex, other) } else { (other, vertex) };
                    done.insert(next, direction);
                    queue.push_back(next);
                }
            }
        }
    }
    done.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modifiers::boundary_edges;
    use crate::primitives::Primitive;

    /// An orthographic view from above: x right, y up on screen, 100 pixels per metre.
    fn top_view<'a>(project: &'a dyn Fn(Vec3) -> Option<Vec2>, ray: &'a dyn Fn(Vec2) -> (Vec3, Vec3)) -> View<'a> {
        View { project, ray }
    }

    #[test]
    fn a_knife_line_across_the_top_splits_it_and_keeps_the_cube_closed() {
        let mut cube = Primitive::Cube { size: 2.0 }.build().unwrap();
        let project = |point: Vec3| Some(Vec2::new(point.x * 100.0, -point.y * 100.0));
        let ray = |screen: Vec2| (Vec3::new(screen.x / 100.0, -screen.y / 100.0, 10.0), Vec3::NEG_Z);
        let view = top_view(&project, &ray);
        // A line across the whole top, at y = 0.
        let made = knife(&mut cube, &[Vec2::new(-150.0, 0.0), Vec2::new(150.0, 0.0)], &view, false).unwrap();
        assert_eq!(made.len(), 2);
        assert_eq!((cube.positions.len(), cube.faces.len()), (10, 7));
        assert!(boundary_edges(&cube).is_empty());
        cube.validate().unwrap();
        // Through the cube: the bottom is cut as well.
        let mut cube = Primitive::Cube { size: 2.0 }.build().unwrap();
        knife(&mut cube, &[Vec2::new(-150.0, 0.0), Vec2::new(150.0, 0.0)], &view, true).unwrap();
        assert_eq!((cube.positions.len(), cube.faces.len()), (12, 10));
        cube.validate().unwrap();
    }

    #[test]
    fn a_line_inside_a_face_leaves_a_loose_cut_and_crosses_no_edge() {
        let mut cube = Primitive::Cube { size: 2.0 }.build().unwrap();
        let project = |point: Vec3| Some(Vec2::new(point.x * 100.0, -point.y * 100.0));
        let ray = |screen: Vec2| (Vec3::new(screen.x / 100.0, -screen.y / 100.0, 10.0), Vec3::NEG_Z);
        let view = top_view(&project, &ray);
        // The line starts and ends inside the top face: its two points become vertices joined by a
        // loose edge; no face edge is crossed, so no face is split.
        let made = knife(&mut cube, &[Vec2::new(-50.0, 0.0), Vec2::new(50.0, 0.0)], &view, false).unwrap();
        assert_eq!(made.len(), 2);
        assert_eq!(cube.faces.len(), 6);
        assert_eq!(cube.loose_edges.len(), 1);
        cube.validate().unwrap();
    }

    #[test]
    fn a_line_bending_inside_a_face_splits_it_along_the_bend() {
        let mut cube = Primitive::Cube { size: 2.0 }.build().unwrap();
        let project = |point: Vec3| Some(Vec2::new(point.x * 100.0, -point.y * 100.0));
        let ray = |screen: Vec2| (Vec3::new(screen.x / 100.0, -screen.y / 100.0, 10.0), Vec3::NEG_Z);
        let view = top_view(&project, &ray);
        // In over the top's left edge, a bend at the middle, out over its bottom edge (screen y up
        // is +y in the scene): the top splits into two along entry, bend, exit.
        let made = knife(&mut cube, &[Vec2::new(-150.0, -50.0), Vec2::new(0.0, 0.0), Vec2::new(50.0, 150.0)], &view, false).unwrap();
        assert_eq!(made.len(), 3, "entry, bend, exit");
        assert_eq!(cube.faces.len(), 7);
        assert!(boundary_edges(&cube).is_empty(), "still closed");
        cube.validate().unwrap();
        let bend = cube.positions.iter().position(|point| point.distance(Vec3::new(0.0, 0.0, 1.0)) < 1e-4).unwrap() as u32;
        assert_eq!(cube.faces.iter().filter(|face| face.contains(&bend)).count(), 2, "the bend is a corner of both halves");

        // A cut that starts inside the top and runs out over an edge: a dangling (loose) edge.
        let mut cube = Primitive::Cube { size: 2.0 }.build().unwrap();
        knife(&mut cube, &[Vec2::new(0.0, 0.0), Vec2::new(150.0, 0.0)], &view, false).unwrap();
        assert_eq!(cube.faces.len(), 6);
        assert_eq!(cube.loose_edges.len(), 1);
        cube.validate().unwrap();
    }

    #[test]
    fn edge_slide_moves_a_loop_towards_one_side() {
        let mut cube = Primitive::Cube { size: 2.0 }.build().unwrap();
        crate::ops::loop_cut(&mut cube, (0, 1), 1, 0.5).unwrap();
        let ring: BTreeSet<u32> = (8..12).collect();
        assert!(ring.iter().all(|vertex| cube.positions[*vertex as usize].z.abs() < 1e-6));
        let moved = edge_slide(&mut cube, &ring, 0.5).unwrap();
        assert_eq!(moved, 4);
        let heights: BTreeSet<i32> = ring.iter().map(|vertex| (cube.positions[*vertex as usize].z * 100.0).round() as i32).collect();
        assert_eq!(heights.len(), 1, "the loop moves together: {heights:?}");
        assert_eq!(heights.iter().next().unwrap().abs(), 50);
        let mut back = cube.clone();
        edge_slide(&mut back, &ring, -0.5).unwrap();
        let other: i32 = (back.positions[8].z * 100.0).round() as i32;
        assert_ne!(other.signum(), heights.iter().next().unwrap().signum(), "the other side");
        cube.validate().unwrap();
        assert!(edge_slide(&mut cube, &ring, 2.0).is_err());
    }
}
