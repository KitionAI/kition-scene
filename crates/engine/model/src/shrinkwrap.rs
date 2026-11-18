//! Shrinkwrap: moves every vertex onto another object's surface — to the nearest point on it,
//! along the vertex normal until it meets it, or to its nearest vertex — then `offset` back
//! out, as the reference application does (checked in `tests/shrinkwrap_blender.rs`). The other
//! surface's triangles sit in a bounding volume hierarchy so each vertex costs a logarithmic
//! search.

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::mesh::Mesh;
use crate::scene::{Id, Object, Scene};

/// Where shrinkwrap puts a vertex.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WrapMethod {
    /// The nearest point on the other surface.
    #[default]
    NearestSurface,
    /// Where the vertex normal meets the other surface (a vertex whose normal misses stays).
    Project,
    /// The other mesh's nearest vertex.
    NearestVertex,
}

/// An axis-aligned box.
#[derive(Debug, Clone, Copy)]
struct Aabb {
    min: Vec3,
    max: Vec3,
}

impl Aabb {
    const EMPTY: Self = Self { min: Vec3::splat(f32::INFINITY), max: Vec3::splat(f32::NEG_INFINITY) };

    fn with(self, point: Vec3) -> Self {
        Self { min: self.min.min(point), max: self.max.max(point) }
    }

    fn distance_squared(&self, point: Vec3) -> f32 {
        let outside = (self.min - point).max(point - self.max).max(Vec3::ZERO);
        outside.length_squared()
    }

    /// Where a ray enters the box (`None` when it misses or the box is behind it).
    fn ray_entry(&self, origin: Vec3, inverse: Vec3, limit: f32) -> Option<f32> {
        let a = (self.min - origin) * inverse;
        let b = (self.max - origin) * inverse;
        let near = a.min(b).max_element().max(0.0);
        let far = a.max(b).min_element().min(limit);
        (near <= far).then_some(near)
    }
}

enum Node {
    Leaf { bounds: Aabb, items: Vec<u32> },
    Inner { bounds: Aabb, left: usize, right: usize },
}

impl Node {
    fn bounds(&self) -> &Aabb {
        match self {
            Self::Leaf { bounds, .. } | Self::Inner { bounds, .. } => bounds,
        }
    }
}

/// A bounding volume hierarchy over triangles.
pub struct Surface {
    triangles: Vec<[Vec3; 3]>,
    nodes: Vec<Node>,
}

/// The point on triangle `abc` nearest to `p`.
fn closest_on_triangle(p: Vec3, [a, b, c]: [Vec3; 3]) -> Vec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denominator = 1.0 / (va + vb + vc);
    a + ab * (vb * denominator) + ac * (vc * denominator)
}

/// How far along `direction` a ray from `origin` meets triangle `abc`, from either side.
fn ray_triangle(origin: Vec3, direction: Vec3, [a, b, c]: [Vec3; 3]) -> Option<f32> {
    let (e1, e2) = (b - a, c - a);
    let p = direction.cross(e2);
    let determinant = e1.dot(p);
    if determinant.abs() < 1e-12 {
        return None;
    }
    let inverse = 1.0 / determinant;
    let s = origin - a;
    let u = s.dot(p) * inverse;
    if !(-1e-6..=1.0 + 1e-6).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = direction.dot(q) * inverse;
    if v < -1e-6 || u + v > 1.0 + 1e-6 {
        return None;
    }
    let t = e2.dot(q) * inverse;
    (t >= 0.0).then_some(t)
}

const LEAF: usize = 4;

impl Surface {
    /// The hierarchy over a mesh's triangles.
    pub fn new(mesh: &Mesh) -> Self {
        let point = |index: u32| mesh.positions.get(index as usize).copied().unwrap_or(Vec3::ZERO);
        let triangles: Vec<[Vec3; 3]> = mesh.triangles().into_iter().map(|([a, b, c], _)| [point(a), point(b), point(c)]).collect();
        let mut surface = Self { triangles, nodes: Vec::new() };
        let items: Vec<u32> = (0..u32::try_from(surface.triangles.len()).unwrap_or(0)).collect();
        if !items.is_empty() {
            surface.build(items);
        }
        surface
    }

    fn triangle(&self, item: u32) -> [Vec3; 3] {
        self.triangles.get(item as usize).copied().unwrap_or([Vec3::ZERO; 3])
    }

    fn build(&mut self, mut items: Vec<u32>) -> usize {
        let bounds = items.iter().fold(Aabb::EMPTY, |bounds, item| self.triangle(*item).iter().fold(bounds, |bounds, point| bounds.with(*point)));
        if items.len() <= LEAF {
            self.nodes.push(Node::Leaf { bounds, items });
            return self.nodes.len() - 1;
        }
        let centre = |item: u32| {
            let [a, b, c] = self.triangle(item);
            (a + b + c) / 3.0
        };
        let size = bounds.max - bounds.min;
        let axis = if size.x >= size.y && size.x >= size.z {
            0
        } else if size.y >= size.z {
            1
        } else {
            2
        };
        items.sort_by(|a, b| centre(*a)[axis].total_cmp(&centre(*b)[axis]));
        let right_items = items.split_off(items.len() / 2);
        let slot = self.nodes.len();
        self.nodes.push(Node::Leaf { bounds, items: Vec::new() });
        let left = self.build(items);
        let right = self.build(right_items);
        if let Some(node) = self.nodes.get_mut(slot) {
            *node = Node::Inner { bounds, left, right };
        }
        slot
    }

    /// The nearest point on the surface to `point`, with the normal of its triangle.
    pub fn nearest(&self, point: Vec3) -> Option<(Vec3, Vec3)> {
        let mut best: Option<(f32, Vec3, u32)> = None;
        let mut stack = vec![0usize];
        while let Some(index) = stack.pop() {
            let Some(node) = self.nodes.get(index) else { continue };
            if best.is_some_and(|(distance, _, _)| node.bounds().distance_squared(point) > distance) {
                continue;
            }
            match node {
                Node::Leaf { items, .. } => {
                    for item in items {
                        let candidate = closest_on_triangle(point, self.triangle(*item));
                        let distance = candidate.distance_squared(point);
                        if best.is_none_or(|(nearest, _, _)| distance < nearest) {
                            best = Some((distance, candidate, *item));
                        }
                    }
                }
                Node::Inner { left, right, .. } => {
                    let near = |child: usize| self.nodes.get(child).map_or(f32::INFINITY, |node| node.bounds().distance_squared(point));
                    // The nearer child is searched first.
                    if near(*left) < near(*right) {
                        stack.extend([*right, *left]);
                    } else {
                        stack.extend([*left, *right]);
                    }
                }
            }
        }
        best.map(|(_, at, item)| {
            let [a, b, c] = self.triangle(item);
            (at, (b - a).cross(c - a).normalize_or_zero())
        })
    }

    /// The nearest place along a ray (both ways when `both`) where it meets the surface: how far
    /// (signed along `direction`) and the triangle's normal.
    pub fn cast(&self, origin: Vec3, direction: Vec3, both: bool) -> Option<(f32, Vec3)> {
        let mut best: Option<(f32, f32, u32)> = None;
        let directions: &[f32] = if both { &[1.0, -1.0] } else { &[1.0] };
        for sign in directions {
            let way = direction * *sign;
            let inverse = Vec3::ONE / way;
            let mut stack = vec![0usize];
            while let Some(index) = stack.pop() {
                let Some(node) = self.nodes.get(index) else { continue };
                let limit = best.map_or(f32::INFINITY, |(distance, _, _)| distance);
                if node.bounds().ray_entry(origin, inverse, limit).is_none() {
                    continue;
                }
                match node {
                    Node::Leaf { items, .. } => {
                        for item in items {
                            if let Some(distance) = ray_triangle(origin, way, self.triangle(*item))
                                && best.is_none_or(|(nearest, _, _)| distance < nearest)
                            {
                                best = Some((distance, *sign, *item));
                            }
                        }
                    }
                    Node::Inner { left, right, .. } => stack.extend([*left, *right]),
                }
            }
        }
        best.map(|(distance, sign, item)| {
            let [a, b, c] = self.triangle(item);
            (distance * sign, (b - a).cross(c - a).normalize_or_zero())
        })
    }
}

/// Vertex normals weighted by each face's angle at the corner (how the reference projects).
fn angle_weighted_normals(mesh: &Mesh) -> Vec<Vec3> {
    let mut normals = vec![Vec3::ZERO; mesh.positions.len()];
    for face in &mesh.faces {
        let normal = mesh.face_normal(face);
        let points = mesh.face_positions(face);
        let count = points.len();
        for (slot, vertex) in face.iter().enumerate() {
            let (Some(previous), Some(current), Some(next)) =
                (points.get((slot + count - 1) % count), points.get(slot), points.get((slot + 1) % count))
            else {
                continue;
            };
            let angle = (*previous - *current).angle_between(*next - *current);
            if let Some(sum) = normals.get_mut(*vertex as usize)
                && angle.is_finite()
            {
                *sum += normal * angle;
            }
        }
    }
    normals.into_iter().map(Vec3::normalize_or_zero).collect()
}

/// The settings of a Shrinkwrap modifier.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wrap {
    pub method: WrapMethod,
    pub offset: f32,
    /// Project: cast along the normal.
    pub positive: bool,
    /// Project: cast against the normal.
    pub negative: bool,
}

/// `mesh` (of `object`) wrapped onto object `target`'s evaluated surface; `None` when the target
/// has none (or is the object itself, or the chain of targets is too deep).
pub fn apply(mesh: &Mesh, object: &Object, scene: &Scene, target: Id, wrap: Wrap, depth: u32) -> Option<Mesh> {
    if target == object.id || depth >= 4 {
        return None;
    }
    let other = scene.object(target).ok()?;
    let mut surface_mesh = other.evaluated_mesh_at(scene, depth + 1)?;
    // The target in this object's axes.
    let relative = scene.world_matrix(object.id).ok()?.inverse() * scene.world_matrix(target).ok()?;
    if !relative.is_finite() {
        return None;
    }
    surface_mesh.transform(relative);
    let mut out = mesh.clone();
    match wrap.method {
        WrapMethod::NearestVertex => {
            if surface_mesh.positions.is_empty() {
                return None;
            }
            for point in &mut out.positions {
                let Some(nearest) =
                    surface_mesh.positions.iter().copied().min_by(|a, b| a.distance_squared(*point).total_cmp(&b.distance_squared(*point)))
                else {
                    continue;
                };
                let away = (*point - nearest).normalize_or_zero();
                *point = nearest + away * wrap.offset;
            }
        }
        WrapMethod::NearestSurface => {
            let surface = Surface::new(&surface_mesh);
            for point in &mut out.positions {
                let Some((hit, normal)) = surface.nearest(*point) else { continue };
                let away = (*point - hit).try_normalize().unwrap_or(normal);
                *point = hit + away * wrap.offset;
            }
        }
        WrapMethod::Project => {
            if !(wrap.positive || wrap.negative) {
                return Some(out);
            }
            let surface = Surface::new(&surface_mesh);
            let normals = angle_weighted_normals(mesh);
            for (point, normal) in out.positions.iter_mut().zip(normals) {
                if normal == Vec3::ZERO {
                    continue;
                }
                let direction = if wrap.positive { normal } else { -normal };
                let Some((distance, face_normal)) = surface.cast(*point, direction, wrap.positive && wrap.negative) else { continue };
                let hit = *point + direction * distance;
                // The offset goes back along the ray, towards where the vertex was.
                let away = (*point - hit).try_normalize().unwrap_or(face_normal);
                *point = hit + away * wrap.offset;
            }
        }
    }
    Some(out)
}
