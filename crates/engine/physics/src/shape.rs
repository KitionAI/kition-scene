//! Collision geometry: convex polyhedra (faces merged into polygons, edges with the faces on
//! each side, for separating-axis tests and face clipping), spheres, and triangle meshes (for
//! passive bodies), all in the body's own frame.

use glam::Vec3;

/// A convex polyhedron in a body's frame.
#[derive(Debug, Clone)]
pub struct Poly {
    pub vertices: Vec<Vec3>,
    pub faces: Vec<Face>,
    pub edges: Vec<Edge>,
}

/// A face: its outward normal, plane offset (`normal · p = offset` on it), and corners in order
/// (counter-clockwise seen from outside).
#[derive(Debug, Clone)]
pub struct Face {
    pub normal: Vec3,
    pub offset: f32,
    pub corners: Vec<usize>,
}

/// An edge between two corners, with the faces on each side.
#[derive(Debug, Clone, Copy)]
pub struct Edge {
    pub a: usize,
    pub b: usize,
    pub left: usize,
    pub right: usize,
}

/// What a body collides as.
#[derive(Debug, Clone)]
pub enum Shape {
    Sphere {
        radius: f32,
    },
    Poly(Poly),
    /// Triangles in the body's frame (passive bodies only), with a grid to find them.
    Mesh(TriangleMesh),
}

impl Shape {
    /// The farthest distance of the shape from the body's origin.
    pub fn reach(&self) -> f32 {
        match self {
            Shape::Sphere { radius } => *radius,
            Shape::Poly(poly) => poly.vertices.iter().map(|vertex| vertex.length()).fold(0.0, f32::max),
            Shape::Mesh(mesh) => mesh.triangles.iter().flatten().map(|vertex| vertex.length()).fold(0.0, f32::max),
        }
    }

    /// The box the shape spans in its own frame.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        let mut low = Vec3::splat(f32::INFINITY);
        let mut high = Vec3::splat(f32::NEG_INFINITY);
        let mut add = |point: Vec3| {
            low = low.min(point);
            high = high.max(point);
        };
        match self {
            Shape::Sphere { radius } => {
                add(Vec3::splat(-*radius));
                add(Vec3::splat(*radius));
            }
            Shape::Poly(poly) => poly.vertices.iter().for_each(|vertex| add(*vertex)),
            Shape::Mesh(mesh) => mesh.triangles.iter().flatten().for_each(|vertex| add(*vertex)),
        }
        if low.x > high.x { (Vec3::ZERO, Vec3::ZERO) } else { (low, high) }
    }
}

impl Poly {
    /// The vertex farthest along `direction`.
    pub fn support(&self, direction: Vec3) -> Vec3 {
        let mut best = Vec3::ZERO;
        let mut most = f32::NEG_INFINITY;
        for vertex in &self.vertices {
            let along = vertex.dot(direction);
            if along > most {
                most = along;
                best = *vertex;
            }
        }
        best
    }

    /// A box with these half extents about `centre`.
    pub fn cuboid(centre: Vec3, half: Vec3) -> Option<Poly> {
        let mut points = Vec::with_capacity(8);
        for bits in 0..8u32 {
            let sign =
                Vec3::new(if bits & 1 == 0 { -1.0 } else { 1.0 }, if bits & 2 == 0 { -1.0 } else { 1.0 }, if bits & 4 == 0 { -1.0 } else { 1.0 });
            points.push(centre + half * sign);
        }
        hull(&points)
    }

    /// A triangle as a flat polyhedron (both sides are faces).
    pub fn triangle(a: Vec3, b: Vec3, c: Vec3) -> Option<Poly> {
        let normal = (b - a).cross(c - a).normalize_or_zero();
        if normal == Vec3::ZERO {
            return None;
        }
        let faces = vec![
            Face { normal, offset: normal.dot(a), corners: vec![0, 1, 2] },
            Face { normal: -normal, offset: -normal.dot(a), corners: vec![0, 2, 1] },
        ];
        let edges = vec![Edge { a: 0, b: 1, left: 0, right: 1 }, Edge { a: 1, b: 2, left: 0, right: 1 }, Edge { a: 2, b: 0, left: 0, right: 1 }];
        Some(Poly { vertices: vec![a, b, c], faces, edges })
    }
}

/// The convex hull of points, its coplanar triangles merged into polygons. `None` for points
/// that span no volume.
pub fn hull(points: &[Vec3]) -> Option<Poly> {
    let points: Vec<Vec3> = points.iter().copied().filter(|point| point.is_finite()).collect();
    let (low, high) =
        points.iter().fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(low, high), point| (low.min(*point), high.max(*point)));
    let size = (high - low).max_element();
    if size.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return None;
    }
    let epsilon = size * 1e-5;
    let triangles = quickhull(&points, epsilon)?;
    Some(merge(&points, &triangles, epsilon))
}

/// Incremental hull: a starting tetrahedron, then each outside point replaces the faces it sees
/// with a fan from the horizon.
fn quickhull(points: &[Vec3], epsilon: f32) -> Option<Vec<[usize; 3]>> {
    let at = |index: usize| points.get(index).copied().unwrap_or(Vec3::ZERO);
    // Extreme points for the starting tetrahedron.
    let first = (0..points.len()).min_by(|a, b| at(*a).x.total_cmp(&at(*b).x))?;
    let second = (0..points.len()).max_by(|a, b| at(*a).distance(at(first)).total_cmp(&at(*b).distance(at(first))))?;
    let line = (at(second) - at(first)).normalize_or_zero();
    let off_line = |index: usize| {
        let offset = at(index) - at(first);
        (offset - line * offset.dot(line)).length()
    };
    let third = (0..points.len()).max_by(|a, b| off_line(*a).total_cmp(&off_line(*b)))?;
    if off_line(third) <= epsilon {
        return None;
    }
    let plane = (at(second) - at(first)).cross(at(third) - at(first)).normalize_or_zero();
    let off_plane = |index: usize| (at(index) - at(first)).dot(plane).abs();
    let fourth = (0..points.len()).max_by(|a, b| off_plane(*a).total_cmp(&off_plane(*b)))?;
    if off_plane(fourth) <= epsilon {
        return None;
    }
    let centre = (at(first) + at(second) + at(third) + at(fourth)) / 4.0;
    let outward = |face: [usize; 3]| -> [usize; 3] {
        let [a, b, c] = face;
        let normal = (at(b) - at(a)).cross(at(c) - at(a));
        if normal.dot(at(a) - centre) < 0.0 { [a, c, b] } else { [a, b, c] }
    };
    let mut faces: Vec<[usize; 3]> =
        vec![outward([first, second, third]), outward([first, second, fourth]), outward([first, third, fourth]), outward([second, third, fourth])];
    let distance = |face: &[usize; 3], point: Vec3| {
        let [a, b, c] = *face;
        let normal = (at(b) - at(a)).cross(at(c) - at(a)).normalize_or_zero();
        (point - at(a)).dot(normal)
    };
    for index in 0..points.len() {
        if [first, second, third, fourth].contains(&index) {
            continue;
        }
        let point = at(index);
        let visible: Vec<usize> = faces.iter().enumerate().filter(|(_, face)| distance(face, point) > epsilon).map(|(slot, _)| slot).collect();
        if visible.is_empty() {
            continue;
        }
        // The horizon: edges of visible faces whose reverse is not on a visible face.
        let mut horizon = Vec::new();
        for slot in &visible {
            let Some(face) = faces.get(*slot) else { continue };
            let [a, b, c] = *face;
            for (from, to) in [(a, b), (b, c), (c, a)] {
                let shared = visible.iter().any(|other| {
                    faces.get(*other).is_some_and(|face| {
                        let [x, y, z] = *face;
                        [(x, y), (y, z), (z, x)].contains(&(to, from))
                    })
                });
                if !shared {
                    horizon.push((from, to));
                }
            }
        }
        let mut kept: Vec<[usize; 3]> = faces.iter().enumerate().filter(|(slot, _)| !visible.contains(slot)).map(|(_, face)| *face).collect();
        for (from, to) in horizon {
            kept.push([from, to, index]);
        }
        faces = kept;
        if faces.len() > 20_000 {
            return None;
        }
    }
    Some(faces)
}

/// Merges coplanar triangles into polygons and lists the edges between faces.
fn merge(points: &[Vec3], triangles: &[[usize; 3]], epsilon: f32) -> Poly {
    let at = |index: usize| points.get(index).copied().unwrap_or(Vec3::ZERO);
    // Group triangles by plane.
    let mut groups: Vec<(Vec3, f32, Vec<[usize; 3]>)> = Vec::new();
    for triangle in triangles {
        let [a, b, c] = *triangle;
        let normal = (at(b) - at(a)).cross(at(c) - at(a)).normalize_or_zero();
        if normal == Vec3::ZERO {
            continue;
        }
        let offset = normal.dot(at(a));
        match groups.iter_mut().find(|(other, other_offset, _)| other.dot(normal) > 1.0 - 1e-5 && (other_offset - offset).abs() <= epsilon * 4.0) {
            Some((_, _, members)) => members.push(*triangle),
            None => groups.push((normal, offset, vec![*triangle])),
        }
    }
    // Each group's outline: the edges used once within it, chained into a loop.
    let mut used: Vec<usize> = Vec::new();
    let mut faces = Vec::new();
    for (normal, offset, members) in &groups {
        let mut outline: Vec<(usize, usize)> = Vec::new();
        for [a, b, c] in members {
            for (from, to) in [(*a, *b), (*b, *c), (*c, *a)] {
                if let Some(slot) = outline.iter().position(|edge| *edge == (to, from)) {
                    outline.remove(slot);
                } else {
                    outline.push((from, to));
                }
            }
        }
        let Some(&(start, mut next)) = outline.first() else { continue };
        let mut corners = vec![start];
        let mut guard = 0;
        while next != start && guard <= outline.len() {
            corners.push(next);
            next = outline.iter().find(|(from, _)| *from == next).map_or(start, |(_, to)| *to);
            guard += 1;
        }
        // Drop corners on straight runs (collinear with their neighbours).
        let count = corners.len();
        let corners: Vec<usize> = (0..count)
            .filter(|slot| {
                let (Some(&previous), Some(&current), Some(&following)) =
                    (corners.get((slot + count - 1) % count), corners.get(*slot), corners.get((slot + 1) % count))
                else {
                    return true;
                };
                (at(current) - at(previous)).cross(at(following) - at(current)).length() > epsilon * epsilon
            })
            .map(|slot| corners.get(slot).copied().unwrap_or(0))
            .collect();
        for corner in &corners {
            if !used.contains(corner) {
                used.push(*corner);
            }
        }
        faces.push(Face { normal: *normal, offset: *offset, corners });
    }
    // Renumber the vertices the faces use.
    used.sort_unstable();
    let vertices: Vec<Vec3> = used.iter().map(|index| at(*index)).collect();
    let renumber = |index: usize| used.binary_search(&index).unwrap_or(0);
    for face in &mut faces {
        for corner in &mut face.corners {
            *corner = renumber(*corner);
        }
    }
    // Edges: each pair of corners on a face, matched with the face that runs it backwards.
    let mut edges = Vec::new();
    for (left, face) in faces.iter().enumerate() {
        let count = face.corners.len();
        for slot in 0..count {
            let (Some(&a), Some(&b)) = (face.corners.get(slot), face.corners.get((slot + 1) % count)) else { continue };
            if a > b {
                continue;
            }
            let right = faces.iter().position(|other| {
                let n = other.corners.len();
                (0..n).any(|k| other.corners.get(k) == Some(&b) && other.corners.get((k + 1) % n) == Some(&a))
            });
            if let Some(right) = right {
                edges.push(Edge { a, b, left, right });
            }
        }
    }
    Poly { vertices, faces, edges }
}

/// Triangles of a passive mesh in its body's frame, bucketed in a grid.
#[derive(Debug, Clone)]
pub struct TriangleMesh {
    pub triangles: Vec<[Vec3; 3]>,
    low: Vec3,
    cell: f32,
    dims: [usize; 3],
    cells: Vec<Vec<u32>>,
}

impl TriangleMesh {
    pub fn new(triangles: Vec<[Vec3; 3]>) -> Self {
        let (low, high) = triangles
            .iter()
            .flatten()
            .fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(low, high), point| (low.min(*point), high.max(*point)));
        let (low, high) = if low.x > high.x { (Vec3::ZERO, Vec3::ZERO) } else { (low, high) };
        let size = (high - low).max(Vec3::splat(1e-3));
        let count = triangles.len().max(1) as f32;
        let cell = (size.x * size.y * size.z / count).cbrt().max(size.max_element() / 64.0).max(1e-3) * 2.0;
        let dims = [
            ((size.x / cell).ceil() as usize).clamp(1, 64),
            ((size.y / cell).ceil() as usize).clamp(1, 64),
            ((size.z / cell).ceil() as usize).clamp(1, 64),
        ];
        let mut mesh = Self { triangles, low, cell, dims, cells: vec![Vec::new(); dims[0] * dims[1] * dims[2]] };
        for (index, triangle) in mesh.triangles.iter().enumerate() {
            let (a, b) = triangle
                .iter()
                .fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(low, high), point| (low.min(*point), high.max(*point)));
            let (from, to) = (mesh.cell_of(a), mesh.cell_of(b));
            for x in from[0]..=to[0] {
                for y in from[1]..=to[1] {
                    for z in from[2]..=to[2] {
                        let slot = (z * dims[1] + y) * dims[0] + x;
                        if let Some(list) = mesh.cells.get_mut(slot) {
                            list.push(u32::try_from(index).unwrap_or(u32::MAX));
                        }
                    }
                }
            }
        }
        mesh
    }

    fn cell_of(&self, point: Vec3) -> [usize; 3] {
        let at = (point - self.low) / self.cell;
        [
            (at.x.max(0.0) as usize).min(self.dims[0] - 1),
            (at.y.max(0.0) as usize).min(self.dims[1] - 1),
            (at.z.max(0.0) as usize).min(self.dims[2] - 1),
        ]
    }

    /// The triangles that may touch the box from `low` to `high` (each once, in order).
    pub fn near(&self, low: Vec3, high: Vec3) -> Vec<u32> {
        let (from, to) = (self.cell_of(low), self.cell_of(high));
        let mut out = Vec::new();
        for x in from[0]..=to[0] {
            for y in from[1]..=to[1] {
                for z in from[2]..=to[2] {
                    let slot = (z * self.dims[1] + y) * self.dims[0] + x;
                    if let Some(list) = self.cells.get(slot) {
                        out.extend_from_slice(list);
                    }
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cube_hull_has_six_quads_and_twelve_edges() {
        let poly = Poly::cuboid(Vec3::ZERO, Vec3::ONE).unwrap();
        assert_eq!(poly.vertices.len(), 8);
        assert_eq!(poly.faces.len(), 6);
        assert!(poly.faces.iter().all(|face| face.corners.len() == 4));
        assert_eq!(poly.edges.len(), 12);
        for face in &poly.faces {
            assert!((face.offset - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn a_hull_ignores_inner_points() {
        let mut points: Vec<Vec3> = (0..8).map(|bits| Vec3::new((bits & 1) as f32, ((bits >> 1) & 1) as f32, ((bits >> 2) & 1) as f32)).collect();
        points.push(Vec3::splat(0.5));
        points.push(Vec3::new(0.5, 0.5, 1.0));
        let poly = hull(&points).unwrap();
        assert_eq!(poly.vertices.len(), 8);
        assert_eq!(poly.faces.len(), 6);
        assert!(hull(&[Vec3::ZERO, Vec3::X, Vec3::Y]).is_none(), "flat points span no volume");
    }
}
