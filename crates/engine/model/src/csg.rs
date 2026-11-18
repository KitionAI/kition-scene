//! Boolean operations on closed meshes (difference, union, intersection) with binary space
//! partitioning trees, in double precision. The result is welded, its T-junctions are closed,
//! coplanar neighbours are merged back into polygons, and straight-through vertices dissolved,
//! so a cube minus a cube comes back as a handful of n-gons rather than a triangle soup.

use std::collections::{BTreeSet, HashMap};

use glam::{DVec3, Vec3};

use crate::mesh::{Mesh, face_edges, newell_vector};

/// What a boolean keeps.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Operation {
    /// The first mesh without the second.
    #[default]
    Difference,
    /// Both meshes as one.
    Union,
    /// Only where both are.
    Intersect,
}

/// The most triangles a boolean takes from both meshes together.
pub const MAX_TRIANGLES: usize = 60_000;

const EPSILON: f64 = 1e-7;

#[derive(Debug, Clone, Copy)]
struct Plane {
    normal: DVec3,
    w: f64,
}

impl Plane {
    fn from_points(a: DVec3, b: DVec3, c: DVec3) -> Option<Self> {
        let normal = (b - a).cross(c - a);
        let length = normal.length();
        if length.is_nan() || length <= 1e-14 {
            return None;
        }
        let normal = normal / length;
        Some(Self { normal, w: normal.dot(a) })
    }

    fn flip(&mut self) {
        self.normal = -self.normal;
        self.w = -self.w;
    }
}

#[derive(Debug, Clone)]
struct Polygon {
    points: Vec<DVec3>,
    plane: Plane,
}

impl Polygon {
    fn flip(&mut self) {
        self.points.reverse();
        self.plane.flip();
    }
}

const COPLANAR: u8 = 0;
const FRONT: u8 = 1;
const BACK: u8 = 2;
const SPANNING: u8 = 3;

/// Sorts `polygon` against `plane` into the four lists (coplanar ones by facing).
fn split(
    plane: &Plane,
    polygon: Polygon,
    coplanar_front: &mut Vec<Polygon>,
    coplanar_back: &mut Vec<Polygon>,
    front: &mut Vec<Polygon>,
    back: &mut Vec<Polygon>,
) {
    let mut kind = 0u8;
    let types: Vec<u8> = polygon
        .points
        .iter()
        .map(|point| {
            let t = plane.normal.dot(*point) - plane.w;
            let side = if t < -EPSILON {
                BACK
            } else if t > EPSILON {
                FRONT
            } else {
                COPLANAR
            };
            kind |= side;
            side
        })
        .collect();
    match kind {
        COPLANAR => {
            if plane.normal.dot(polygon.plane.normal) > 0.0 {
                coplanar_front.push(polygon);
            } else {
                coplanar_back.push(polygon);
            }
        }
        FRONT => front.push(polygon),
        BACK => back.push(polygon),
        _ => {
            let count = polygon.points.len();
            let (mut f, mut b) = (Vec::with_capacity(count + 1), Vec::with_capacity(count + 1));
            for i in 0..count {
                let j = (i + 1) % count;
                let (Some(&ti), Some(&tj), Some(&vi), Some(&vj)) = (types.get(i), types.get(j), polygon.points.get(i), polygon.points.get(j)) else {
                    continue;
                };
                if ti != BACK {
                    f.push(vi);
                }
                if ti != FRONT {
                    b.push(vi);
                }
                if (ti | tj) == SPANNING {
                    let denominator = plane.normal.dot(vj - vi);
                    if denominator.abs() > 1e-300 {
                        let t = (plane.w - plane.normal.dot(vi)) / denominator;
                        let point = vi.lerp(vj, t);
                        f.push(point);
                        b.push(point);
                    }
                }
            }
            if f.len() >= 3 {
                front.push(Polygon { points: f, plane: polygon.plane });
            }
            if b.len() >= 3 {
                back.push(Polygon { points: b, plane: polygon.plane });
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Node {
    plane: Option<Plane>,
    front: Option<usize>,
    back: Option<usize>,
    polygons: Vec<Polygon>,
}

/// A BSP tree in an arena, walked without recursion so deep trees cannot overflow the stack.
#[derive(Debug, Clone, Default)]
struct Tree {
    nodes: Vec<Node>,
}

impl Tree {
    fn new(polygons: Vec<Polygon>) -> Self {
        let mut tree = Self { nodes: vec![Node::default()] };
        tree.build(polygons);
        tree
    }

    fn build(&mut self, polygons: Vec<Polygon>) {
        if self.nodes.is_empty() {
            self.nodes.push(Node::default());
        }
        let mut work = vec![(0usize, polygons)];
        while let Some((at, polygons)) = work.pop() {
            if polygons.is_empty() {
                continue;
            }
            let Some(node) = self.nodes.get_mut(at) else { continue };
            let plane = match node.plane {
                Some(plane) => plane,
                None => {
                    let Some(first) = polygons.first() else { continue };
                    node.plane = Some(first.plane);
                    first.plane
                }
            };
            let (mut coplanar_front, mut coplanar_back, mut front, mut back) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            for polygon in polygons {
                split(&plane, polygon, &mut coplanar_front, &mut coplanar_back, &mut front, &mut back);
            }
            node.polygons.extend(coplanar_front);
            node.polygons.extend(coplanar_back);
            let (front_child, back_child) = (node.front, node.back);
            if !front.is_empty() {
                let child = front_child.unwrap_or_else(|| self.add(at, true));
                work.push((child, front));
            }
            if !back.is_empty() {
                let child = back_child.unwrap_or_else(|| self.add(at, false));
                work.push((child, back));
            }
        }
    }

    fn add(&mut self, parent: usize, front: bool) -> usize {
        let index = self.nodes.len();
        self.nodes.push(Node::default());
        if let Some(node) = self.nodes.get_mut(parent) {
            if front {
                node.front = Some(index);
            } else {
                node.back = Some(index);
            }
        }
        index
    }

    fn invert(&mut self) {
        for node in &mut self.nodes {
            for polygon in &mut node.polygons {
                polygon.flip();
            }
            if let Some(plane) = &mut node.plane {
                plane.flip();
            }
            std::mem::swap(&mut node.front, &mut node.back);
        }
    }

    /// The parts of `polygons` outside this tree's solid.
    fn clip_polygons(&self, polygons: Vec<Polygon>) -> Vec<Polygon> {
        let mut out = Vec::new();
        if self.nodes.is_empty() {
            return polygons;
        }
        let mut work = vec![(0usize, polygons)];
        while let Some((at, polygons)) = work.pop() {
            let Some(node) = self.nodes.get(at) else { continue };
            let Some(plane) = node.plane else {
                out.extend(polygons);
                continue;
            };
            let (mut coplanar_front, mut coplanar_back, mut front, mut back) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            for polygon in polygons {
                split(&plane, polygon, &mut coplanar_front, &mut coplanar_back, &mut front, &mut back);
            }
            front.extend(coplanar_front);
            back.extend(coplanar_back);
            match node.front {
                Some(child) => work.push((child, front)),
                None => out.extend(front),
            }
            if let Some(child) = node.back {
                work.push((child, back));
            }
        }
        out
    }

    fn clip_to(&mut self, other: &Tree) {
        for node in &mut self.nodes {
            let polygons = std::mem::take(&mut node.polygons);
            node.polygons = other.clip_polygons(polygons);
        }
    }

    fn polygons(&self) -> Vec<Polygon> {
        self.nodes.iter().flat_map(|node| node.polygons.iter().cloned()).collect()
    }
}

fn polygons_of(mesh: &Mesh) -> Vec<Polygon> {
    mesh.triangles()
        .into_iter()
        .filter_map(|(triangle, _)| {
            let points: Vec<DVec3> =
                triangle.iter().map(|index| mesh.positions.get(*index as usize).map(|point| point.as_dvec3())).collect::<Option<_>>()?;
            let [a, b, c] = points.as_slice() else { return None };
            let plane = Plane::from_points(*a, *b, *c)?;
            Some(Polygon { points, plane })
        })
        .collect()
}

/// `a` combined with `b` (both closed meshes in the same space), or `None` when the inputs are
/// too large or the result does not come out as a valid mesh.
pub fn boolean(a: &Mesh, b: &Mesh, operation: Operation) -> Option<Mesh> {
    let (pa, pb) = (polygons_of(a), polygons_of(b));
    if pa.len() + pb.len() > MAX_TRIANGLES {
        return None;
    }
    let mut ta = Tree::new(pa);
    let mut tb = Tree::new(pb);
    let polygons = match operation {
        Operation::Union => {
            ta.clip_to(&tb);
            tb.clip_to(&ta);
            tb.invert();
            tb.clip_to(&ta);
            tb.invert();
            ta.build(tb.polygons());
            ta.polygons()
        }
        Operation::Difference => {
            ta.invert();
            ta.clip_to(&tb);
            tb.clip_to(&ta);
            tb.invert();
            tb.clip_to(&ta);
            tb.invert();
            ta.build(tb.polygons());
            ta.invert();
            ta.polygons()
        }
        Operation::Intersect => {
            ta.invert();
            tb.clip_to(&ta);
            tb.invert();
            ta.clip_to(&tb);
            tb.clip_to(&ta);
            ta.build(tb.polygons());
            ta.invert();
            ta.polygons()
        }
    };
    let size = a.bounds(glam::Mat4::IDENTITY).union(b.bounds(glam::Mat4::IDENTITY)).size().length().max(1e-3);
    let mut mesh = assemble(&polygons, f64::from(size) * 1e-6);
    mesh.smooth = a.smooth;
    mesh.smooth_angle = a.smooth_angle;
    tidy(&mut mesh);
    mesh.validate().ok()?;
    Some(mesh)
}

/// Polygons to a mesh, joining points closer than `tolerance`.
fn assemble(polygons: &[Polygon], tolerance: f64) -> Mesh {
    let cell = tolerance.max(1e-12);
    let mut grid: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::new();
    let mut points: Vec<DVec3> = Vec::new();
    let mut find = |point: DVec3| -> u32 {
        let key = ((point.x / cell).round() as i64, (point.y / cell).round() as i64, (point.z / cell).round() as i64);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(list) = grid.get(&(key.0 + dx, key.1 + dy, key.2 + dz)) {
                        for index in list {
                            if points.get(*index as usize).is_some_and(|other| other.distance(point) <= tolerance) {
                                return *index;
                            }
                        }
                    }
                }
            }
        }
        let index = u32::try_from(points.len()).unwrap_or(u32::MAX);
        points.push(point);
        grid.entry(key).or_default().push(index);
        index
    };
    let mut faces = Vec::new();
    for polygon in polygons {
        let mut face: Vec<u32> = polygon.points.iter().map(|point| find(*point)).collect();
        face.dedup();
        while face.len() > 1 && face.first() == face.last() {
            face.pop();
        }
        let distinct: BTreeSet<u32> = face.iter().copied().collect();
        if face.len() >= 3 && distinct.len() == face.len() {
            faces.push(face);
        }
    }
    Mesh { positions: points.iter().map(|point| point.as_vec3()).collect(), faces, ..Default::default() }
}

/// Closes T-junctions, merges coplanar neighbours, and dissolves straight-through vertices.
fn tidy(mesh: &mut Mesh) {
    split_t_junctions(mesh);
    merge_coplanar(mesh);
    dissolve_straight(mesh);
    let positions = mesh.positions.clone();
    mesh.faces.retain(|face| {
        let points: Vec<Vec3> = face.iter().filter_map(|index| positions.get(*index as usize).copied()).collect();
        newell_vector(&points).length() > 1e-12
    });
    mesh.remove_unused_vertices();
}

/// Inserts into every face edge the vertices lying on it, so neighbours share their corners.
fn split_t_junctions(mesh: &mut Mesh) {
    let positions = mesh.positions.clone();
    let scale = mesh.bounds(glam::Mat4::IDENTITY).size().length().max(1e-3) * 1e-5;
    for face in &mut mesh.faces {
        let mut rebuilt = Vec::with_capacity(face.len());
        for (a, b) in face_edges(face) {
            rebuilt.push(a);
            let (Some(&pa), Some(&pb)) = (positions.get(a as usize), positions.get(b as usize)) else { continue };
            let direction = pb - pa;
            let length_squared = direction.length_squared();
            if length_squared <= 0.0 {
                continue;
            }
            let (low, high) = (pa.min(pb) - Vec3::splat(scale), pa.max(pb) + Vec3::splat(scale));
            let mut on: Vec<(f32, u32)> = positions
                .iter()
                .enumerate()
                .filter_map(|(index, point)| {
                    let index = u32::try_from(index).ok()?;
                    if index == a || index == b || point.cmplt(low).any() || point.cmpgt(high).any() {
                        return None;
                    }
                    let t = (*point - pa).dot(direction) / length_squared;
                    if t <= 1e-6 || t >= 1.0 - 1e-6 {
                        return None;
                    }
                    ((pa + direction * t).distance(*point) <= scale).then_some((t, index))
                })
                .collect();
            on.sort_by(|x, y| x.0.total_cmp(&y.0));
            rebuilt.extend(on.into_iter().map(|(_, index)| index));
        }
        rebuilt.dedup();
        *face = rebuilt;
    }
}

fn plane_of(mesh: &Mesh, face: &[u32]) -> (Vec3, f32) {
    let normal = mesh.face_normal(face);
    (normal, normal.dot(mesh.face_center(face)))
}

/// Joins neighbouring faces in one plane into one polygon, while the result stays simple.
fn merge_coplanar(mesh: &mut Mesh) {
    let tolerance = mesh.bounds(glam::Mat4::IDENTITY).size().length().max(1e-3) * 1e-5;
    let mut changed = true;
    let mut rounds = 0;
    while changed && rounds < 64 {
        changed = false;
        rounds += 1;
        let mut owner: HashMap<(u32, u32), usize> = HashMap::new();
        for (index, face) in mesh.faces.iter().enumerate() {
            for edge in face_edges(face) {
                owner.insert(edge, index);
            }
        }
        let mut gone = vec![false; mesh.faces.len()];
        let mut merged_faces: Vec<Vec<u32>> = Vec::new();
        for index in 0..mesh.faces.len() {
            if gone.get(index).copied().unwrap_or(true) {
                continue;
            }
            let Some(face) = mesh.faces.get(index).cloned() else { continue };
            let (normal, offset) = plane_of(mesh, &face);
            let mut result = None;
            for (u, v) in face_edges(&face) {
                let Some(&other) = owner.get(&(v, u)) else { continue };
                if other == index || gone.get(other).copied().unwrap_or(true) {
                    continue;
                }
                let Some(neighbour) = mesh.faces.get(other) else { continue };
                let (other_normal, other_offset) = plane_of(mesh, neighbour);
                if normal.dot(other_normal) < 1.0 - 1e-6 || (offset - other_offset).abs() > tolerance {
                    continue;
                }
                if let Some(joined) = splice(&face, neighbour, u, v) {
                    result = Some((other, joined));
                    break;
                }
            }
            if let Some((other, joined)) = result {
                if let Some(flag) = gone.get_mut(index) {
                    *flag = true;
                }
                if let Some(flag) = gone.get_mut(other) {
                    *flag = true;
                }
                merged_faces.push(joined);
                changed = true;
            }
        }
        let mut next: Vec<Vec<u32>> =
            mesh.faces.iter().enumerate().filter(|(index, _)| !gone.get(*index).copied().unwrap_or(true)).map(|(_, face)| face.clone()).collect();
        next.extend(merged_faces);
        mesh.faces = next;
    }
}

/// `a` (with edge u -> v) and `b` (with v -> u) as one polygon, or `None` when the join would
/// not be a simple loop.
fn splice(a: &[u32], b: &[u32], u: u32, v: u32) -> Option<Vec<u32>> {
    let at_v = a.iter().position(|vertex| *vertex == v)?;
    let at_u = b.iter().position(|vertex| *vertex == u)?;
    // a from v round to u, then b's inside from after u to before v.
    let first: Vec<u32> = (0..a.len()).filter_map(|step| a.get((at_v + step) % a.len()).copied()).collect();
    let second: Vec<u32> = (1..b.len().saturating_sub(1)).filter_map(|step| b.get((at_u + step) % b.len()).copied()).collect();
    let mut joined = first;
    joined.extend(second);
    // Remove there-and-back spikes left where the two shared more than one edge.
    let mut guard = 0;
    loop {
        guard += 1;
        let count = joined.len();
        if count < 3 || guard > 4096 {
            break;
        }
        let spike = (0..count).find(|i| joined.get((i + count - 1) % count) == joined.get((i + 1) % count));
        let Some(i) = spike else { break };
        let (first_removed, second_removed) = (i, (i + 1) % count);
        let (high, low) = (first_removed.max(second_removed), first_removed.min(second_removed));
        joined.remove(high);
        joined.remove(low);
    }
    let distinct: BTreeSet<u32> = joined.iter().copied().collect();
    (joined.len() >= 3 && distinct.len() == joined.len()).then_some(joined)
}

/// Removes vertices that only continue a straight edge (two neighbours, in line).
fn dissolve_straight(mesh: &mut Mesh) {
    let mut neighbours: HashMap<u32, BTreeSet<u32>> = HashMap::new();
    for face in &mesh.faces {
        for (a, b) in face_edges(face) {
            neighbours.entry(a).or_default().insert(b);
            neighbours.entry(b).or_default().insert(a);
        }
    }
    let positions = mesh.positions.clone();
    let straight = |vertex: u32| -> bool {
        let Some(around) = neighbours.get(&vertex) else { return false };
        let ends: Vec<u32> = around.iter().copied().collect();
        let [a, b] = ends.as_slice() else { return false };
        let (Some(&p), Some(&pa), Some(&pb)) = (positions.get(vertex as usize), positions.get(*a as usize), positions.get(*b as usize)) else {
            return false;
        };
        let (da, db) = ((pa - p).normalize_or_zero(), (pb - p).normalize_or_zero());
        da.dot(db) < -1.0 + 1e-6
    };
    for face in &mut mesh.faces {
        if face.len() <= 3 {
            continue;
        }
        let kept: Vec<u32> = face.iter().copied().filter(|vertex| !straight(*vertex)).collect();
        if kept.len() >= 3 {
            *face = kept;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modifiers::boundary_edges;
    use crate::primitives::Primitive;

    fn cube_at(offset: Vec3) -> Mesh {
        let mut mesh = Primitive::Cube { size: 2.0 }.build().unwrap();
        mesh.transform(glam::Mat4::from_translation(offset));
        mesh
    }

    fn volume(mesh: &Mesh) -> f32 {
        mesh.faces
            .iter()
            .map(|face| {
                let points = mesh.face_positions(face);
                let first = points[0];
                points.windows(2).skip(1).map(|pair| first.dot(pair[0].cross(pair[1]))).sum::<f32>()
            })
            .sum::<f32>()
            / 6.0
    }

    // The reference application's exact solver gives 14 vertices / 9 faces, 20 / 12, and 8 / 6.
    #[test]
    fn two_cubes_match_the_reference() {
        let a = cube_at(Vec3::ZERO);
        let b = cube_at(Vec3::ONE);
        for (operation, (vertices, faces), expected_volume, (low, high)) in [
            (Operation::Difference, (14, 9), 7.0, (Vec3::splat(-1.0), Vec3::splat(1.0))),
            (Operation::Union, (20, 12), 15.0, (Vec3::splat(-1.0), Vec3::splat(2.0))),
            (Operation::Intersect, (8, 6), 1.0, (Vec3::ZERO, Vec3::ONE)),
        ] {
            let out = boolean(&a, &b, operation).unwrap();
            assert!(boundary_edges(&out).is_empty(), "{operation:?} is closed");
            assert!((volume(&out) - expected_volume).abs() < 1e-4, "{operation:?} volume {}", volume(&out));
            let bounds = out.bounds(glam::Mat4::IDENTITY);
            assert!(bounds.min.abs_diff_eq(low, 1e-5) && bounds.max.abs_diff_eq(high, 1e-5), "{operation:?} {bounds:?}");
            assert_eq!((out.positions.len(), out.faces.len()), (vertices, faces), "{operation:?}");
        }
    }

    #[test]
    fn a_sphere_cut_from_a_cube_stays_closed() {
        let a = cube_at(Vec3::ZERO);
        let mut sphere = Primitive::UvSphere { segments: 16, rings: 8, radius: 1.0 }.build().unwrap();
        sphere.transform(glam::Mat4::from_translation(Vec3::new(1.0, 0.0, 0.3)));
        let out = boolean(&a, &sphere, Operation::Difference).unwrap();
        assert!(boundary_edges(&out).is_empty());
        let v = volume(&out);
        assert!(v > 4.0 && v < 8.0, "volume {v}");
    }

    #[test]
    fn disjoint_and_too_large_inputs() {
        let a = cube_at(Vec3::ZERO);
        let far = cube_at(Vec3::splat(10.0));
        assert!((volume(&boolean(&a, &far, Operation::Union).unwrap()) - 16.0).abs() < 1e-3);
        assert!((volume(&boolean(&a, &far, Operation::Difference).unwrap()) - 8.0).abs() < 1e-3);
        let huge = Primitive::UvSphere { segments: 400, rings: 200, radius: 1.0 }.build().unwrap();
        assert!(boolean(&a, &huge, Operation::Union).is_none());
    }
}
