//! The second set of modifiers: displace, decimate, wireframe, simple deform, and cast. Defaults
//! and results follow the reference application (checked in `tests/modifiers2.rs`).

use std::cmp::Ordering;
use std::collections::{BTreeSet, BinaryHeap, HashMap};

use glam::{DMat4, DVec3, DVec4, Vec3};
use serde::{Deserialize, Serialize};

use crate::mesh::{Mesh, face_edges};
use crate::modifiers::{ModifierKind, ScrewAxis};

/// Where displace pushes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DisplaceDirection {
    #[default]
    Normal,
    X,
    Y,
    Z,
}

/// What simple deform does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeformMode {
    #[default]
    Twist,
    Bend,
    Taper,
    Stretch,
}

fn axis_index(axis: ScrewAxis) -> usize {
    match axis {
        ScrewAxis::X => 0,
        ScrewAxis::Y => 1,
        ScrewAxis::Z => 2,
    }
}

fn component(point: Vec3, index: usize) -> f32 {
    match index {
        0 => point.x,
        1 => point.y,
        _ => point.z,
    }
}

fn set_component(point: &mut Vec3, index: usize, value: f32) {
    match index {
        0 => point.x = value,
        1 => point.y = value,
        _ => point.z = value,
    }
}

/// A smooth, deterministic noise value in 0..1 (value noise on an integer lattice).
pub fn noise(point: Vec3) -> f32 {
    let hash = |x: i64, y: i64, z: i64| -> f32 {
        let mut h = (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663) ^ z.wrapping_mul(83_492_791)) as u64;
        h ^= h >> 33;
        h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
        h ^= h >> 33;
        h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        h ^= h >> 33;
        (h & 0xff_ffff) as f32 / 0xff_ffff as f32
    };
    let base = point.floor();
    let fraction = point - base;
    let smooth = fraction * fraction * (Vec3::splat(3.0) - fraction * 2.0);
    let (x, y, z) = (base.x as i64, base.y as i64, base.z as i64);
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let mut layers = [0.0f32; 2];
    for (dz, layer) in layers.iter_mut().enumerate() {
        let dz = dz as i64;
        let near = lerp(hash(x, y, z + dz), hash(x + 1, y, z + dz), smooth.x);
        let far = lerp(hash(x, y + 1, z + dz), hash(x + 1, y + 1, z + dz), smooth.x);
        *layer = lerp(near, far, smooth.y);
    }
    lerp(layers[0], layers[1], smooth.z)
}

/// Where a displace image is looked up for each vertex.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextureCoordinates {
    /// The object's own axes: -1 to 1 covers the image once.
    #[default]
    Local,
    /// World axes, likewise.
    Global,
    /// The mesh's texture coordinates (a vertex takes its first corner's).
    Uv,
}

/// An image's brightness per pixel (the mean of its stored red, green, and blue, 0 to 1), with
/// its size; row 0 at the top.
type Brightness = (usize, usize, Vec<f32>);

static BRIGHTNESS: std::sync::Mutex<Option<HashMap<String, std::sync::Arc<Brightness>>>> = std::sync::Mutex::new(None);

fn brightness(image: &crate::scene::Image) -> Option<std::sync::Arc<Brightness>> {
    let mut cache = BRIGHTNESS.lock().ok()?;
    let cache = cache.get_or_insert_with(HashMap::new);
    if let Some(found) = cache.get(&image.hash) {
        return Some(found.clone());
    }
    let (width, height, rgba) = crate::pixels::decode_rgba8(image)?;
    let values = rgba.as_chunks::<4>().0.iter().map(|[r, g, b, _]| (f32::from(*r) + f32::from(*g) + f32::from(*b)) / (3.0 * 255.0)).collect();
    let decoded = std::sync::Arc::new((width, height, values));
    // A few images at a time.
    if cache.len() >= 8 {
        cache.clear();
    }
    cache.insert(image.hash.clone(), decoded.clone());
    Some(decoded)
}

/// The image's brightness at texture coordinate `(u, v)` (V up), bilinear between pixel
/// centres, repeating past the edges.
fn sample((width, height, values): &Brightness, u: f32, v: f32) -> f32 {
    let (w, h) = (*width as i64, *height as i64);
    let x = u * *width as f32 - 0.5;
    let y = (1.0 - v) * *height as f32 - 0.5;
    let (x0, y0) = (x.floor(), y.floor());
    let (tx, ty) = (x - x0, y - y0);
    let at = |column: i64, row: i64| values.get((row.rem_euclid(h) * w + column.rem_euclid(w)) as usize).copied().unwrap_or(0.0);
    let (cx, cy) = (x0 as i64, y0 as i64);
    let top = at(cx, cy) * (1.0 - tx) + at(cx + 1, cy) * tx;
    let bottom = at(cx, cy + 1) * (1.0 - tx) + at(cx + 1, cy + 1) * tx;
    top * (1.0 - ty) + bottom * ty
}

/// Each vertex's displace value from an image, or `None` when it cannot be read.
pub fn image_values(mesh: &Mesh, image: &crate::scene::Image, coordinates: TextureCoordinates, to_world: glam::Mat4) -> Option<Vec<f32>> {
    let pixels = brightness(image)?;
    let mut uvs: Vec<Option<glam::Vec2>> = vec![None; mesh.positions.len()];
    if coordinates == TextureCoordinates::Uv {
        for (face, corners) in mesh.faces.iter().enumerate() {
            for (corner, vertex) in corners.iter().enumerate() {
                if let Some(slot) = uvs.get_mut(*vertex as usize)
                    && slot.is_none()
                    && mesh.has_uvs()
                {
                    *slot = Some(mesh.corner_uv(face, corner));
                }
            }
        }
    }
    let values = mesh
        .positions
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let (u, v) = match coordinates {
                TextureCoordinates::Local => ((point.x + 1.0) / 2.0, (point.y + 1.0) / 2.0),
                TextureCoordinates::Global => {
                    let world = to_world.transform_point3(*point);
                    ((world.x + 1.0) / 2.0, (world.y + 1.0) / 2.0)
                }
                TextureCoordinates::Uv => uvs.get(index).copied().flatten().map_or((0.0, 0.0), |uv| (uv.x, uv.y)),
            };
            sample(&pixels, u, v)
        })
        .collect();
    Some(values)
}

/// Pushes each vertex by `(value - mid_level) × strength`: its value from `values` (an image's,
/// per vertex), else procedural noise when `noise_scale` is above zero, else 1.
pub fn displace(mesh: &Mesh, strength: f32, mid_level: f32, direction: DisplaceDirection, noise_scale: f32, values: Option<&[f32]>) -> Mesh {
    let normals = if direction == DisplaceDirection::Normal { mesh.vertex_normals() } else { Vec::new() };
    let mut out = mesh.clone();
    for (index, point) in out.positions.iter_mut().enumerate() {
        let value = match values {
            Some(values) => values.get(index).copied().unwrap_or(0.0),
            None if noise_scale > 0.0 => noise(*point / noise_scale),
            None => 1.0,
        };
        let amount = (value - mid_level) * strength;
        let along = match direction {
            DisplaceDirection::Normal => normals.get(index).copied().unwrap_or(Vec3::ZERO),
            DisplaceDirection::X => Vec3::X,
            DisplaceDirection::Y => Vec3::Y,
            DisplaceDirection::Z => Vec3::Z,
        };
        *point += along * amount;
    }
    out
}

pub fn cast(mesh: &Mesh, factor: f32, axes: [bool; 3]) -> Mesh {
    let lengths: Vec<f32> = mesh.positions.iter().map(|point| point.length()).collect();
    if lengths.is_empty() {
        return mesh.clone();
    }
    let radius = lengths.iter().sum::<f32>() / lengths.len() as f32;
    let mut out = mesh.clone();
    for (point, length) in out.positions.iter_mut().zip(lengths) {
        if length <= 1e-9 {
            continue;
        }
        let target = *point / length * radius;
        for axis in 0..3 {
            if axes.get(axis).copied().unwrap_or(false) {
                let value = component(*point, axis);
                set_component(point, axis, value + factor * (component(target, axis) - value));
            }
        }
    }
    out
}

pub fn simple_deform(mesh: &Mesh, mode: DeformMode, axis: ScrewAxis, angle: f32, factor: f32, limits: [f32; 2]) -> Mesh {
    let mut out = mesh.clone();
    let main = axis_index(axis);
    // Bend turns round the axis, so the coordinate that bends is another one.
    let (along, across) = match (mode, axis) {
        (DeformMode::Bend, ScrewAxis::Z) => (0, 1),
        (DeformMode::Bend, ScrewAxis::X) => (2, 1),
        (DeformMode::Bend, ScrewAxis::Y) => (2, 0),
        _ => (main, main),
    };
    let values: Vec<f32> = mesh.positions.iter().map(|point| component(*point, along)).collect();
    let (Some(low), Some(high)) = (values.iter().copied().reduce(f32::min), values.iter().copied().reduce(f32::max)) else { return out };
    let span = high - low;
    let (lower, upper) = (low + span * limits[0], low + span * limits[1]);
    let amount = if matches!(mode, DeformMode::Twist | DeformMode::Bend) { angle } else { factor };
    let per_unit = amount / (upper - lower).max(1e-6);
    let (u, v) = match main {
        0 => (1, 2),
        1 => (2, 0),
        _ => (0, 1),
    };
    for point in &mut out.positions {
        let raw = component(*point, along);
        let clamped = raw.clamp(lower, upper);
        match mode {
            DeformMode::Twist => {
                let (sin, cos) = (clamped * per_unit).sin_cos();
                let (pu, pv) = (component(*point, u), component(*point, v));
                set_component(point, u, pu * cos - pv * sin);
                set_component(point, v, pu * sin + pv * cos);
            }
            DeformMode::Taper => {
                let scale = 1.0 + clamped * per_unit;
                for other in [u, v] {
                    let value = component(*point, other);
                    set_component(point, other, value * scale);
                }
            }
            DeformMode::Stretch => {
                let scale = clamped * clamped * per_unit - per_unit + 1.0;
                for other in [u, v] {
                    let value = component(*point, other);
                    set_component(point, other, value * scale);
                }
                set_component(point, main, raw * (1.0 + per_unit));
            }
            DeformMode::Bend => {
                if per_unit.abs() < 1e-9 {
                    continue;
                }
                let radius = 1.0 / per_unit;
                let theta = clamped * per_unit;
                let (sin, cos) = theta.sin_cos();
                let b = component(*point, across) - radius;
                let extra = raw - clamped;
                set_component(point, along, -b * sin + cos * extra);
                set_component(point, across, b * cos + radius + sin * extra);
            }
        }
    }
    out
}

/// Every edge as a strut: per vertex two points along its normal, per face corner one point
/// inset in the face, joined into a closed frame.
pub fn wireframe(mesh: &Mesh, thickness: f32, offset: f32) -> Mesh {
    if mesh.faces.is_empty() {
        return mesh.clone();
    }
    let normals = mesh.vertex_normals();
    let mut out = Mesh { smooth: mesh.smooth, smooth_angle: mesh.smooth_angle, ..Default::default() };
    let inner = thickness * (offset - 1.0) / 2.0;
    let outer = thickness * (offset + 1.0) / 2.0;
    let mut pairs = Vec::with_capacity(mesh.positions.len());
    for (index, point) in mesh.positions.iter().enumerate() {
        let normal = normals.get(index).copied().unwrap_or(Vec3::Z);
        let first = out.vertex_count();
        out.positions.push(*point + normal * inner);
        out.positions.push(*point + normal * outer);
        pairs.push((first, first + 1));
    }
    // One inset point per face corner, at the struts' middle height.
    let mut corner: HashMap<(usize, u32), u32> = HashMap::new();
    for (face_index, face) in mesh.faces.iter().enumerate() {
        let points = mesh.face_positions(face);
        let normal = mesh.face_normal(face);
        let count = points.len();
        for slot in 0..count {
            let (Some(previous), Some(current), Some(next), Some(vertex)) =
                (points.get((slot + count - 1) % count), points.get(slot), points.get((slot + 1) % count), face.get(slot))
            else {
                continue;
            };
            let incoming = normal.cross((*current - *previous).normalize_or_zero());
            let outgoing = normal.cross((*next - *current).normalize_or_zero());
            let bisector = (incoming + outgoing).normalize_or_zero();
            let cosine = bisector.dot(incoming).max(0.1);
            let middle = normal * ((inner + outer) / 2.0);
            corner.insert((face_index, *vertex), out.vertex_count());
            out.positions.push(*current + bisector * (thickness / 2.0 / cosine) + middle);
        }
    }
    let mut faces_of: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (index, face) in mesh.faces.iter().enumerate() {
        for (a, b) in face_edges(face) {
            faces_of.entry((a.min(b), a.max(b))).or_default().push(index);
        }
    }
    let mut keys: Vec<(u32, u32)> = faces_of.keys().copied().collect();
    keys.sort_unstable();
    for (a, b) in keys {
        let faces = faces_of.get(&(a, b)).map_or(&[][..], Vec::as_slice);
        let section = |vertex: u32| -> Option<Vec<u32>> {
            let (low, high) = *pairs.get(vertex as usize)?;
            let mut ring = vec![low, *corner.get(&(*faces.first()?, vertex))?, high];
            if let Some(second) = faces.get(1) {
                ring.push(*corner.get(&(*second, vertex))?);
            }
            Some(ring)
        };
        let (Some(start), Some(end)) = (section(a), section(b)) else { continue };
        // Round the strut: low, inset, high, and the other face's inset when there is one.
        let count = start.len();
        for slot in 0..count {
            let next = (slot + 1) % count;
            let (Some(&s0), Some(&s1), Some(&e0), Some(&e1)) = (start.get(slot), start.get(next), end.get(slot), end.get(next)) else { continue };
            out.faces.push(vec![s0, s1, e1, e0]);
        }
    }
    out.remove_unused_vertices();
    crate::ops::recalculate_outside(&mut out);
    out
}

#[derive(PartialEq)]
struct Candidate {
    cost: f64,
    edge: (u32, u32),
    stamp: (u32, u32),
}

impl Eq for Candidate {}

impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        // A min-heap on cost.
        other.cost.total_cmp(&self.cost).then_with(|| other.edge.cmp(&self.edge))
    }
}

fn plane_quadric(a: DVec3, b: DVec3, c: DVec3) -> DMat4 {
    let normal = (b - a).cross(c - a);
    let length = normal.length();
    if length < 1e-18 {
        return DMat4::ZERO;
    }
    let n = normal / length;
    let plane = DVec4::new(n.x, n.y, n.z, -n.dot(a));
    DMat4::from_cols(plane * plane.x, plane * plane.y, plane * plane.z, plane * plane.w)
}

fn error(quadric: &DMat4, point: DVec3) -> f64 {
    let p = point.extend(1.0);
    p.dot(*quadric * p)
}

/// Edge collapse driven by quadric error until about `ratio` of the triangles are left. Open
/// outlines are held in place by boundary planes; a collapse that would fold a triangle over or
/// make the surface non-manifold is skipped.
pub fn decimate(mesh: &Mesh, ratio: f32) -> Mesh {
    if ratio >= 1.0 || mesh.faces.is_empty() {
        return mesh.clone();
    }
    let mut positions: Vec<DVec3> = mesh.positions.iter().map(|point| point.as_dvec3()).collect();
    let mut triangles: Vec<[u32; 3]> = mesh.triangles().into_iter().map(|(triangle, _)| triangle).collect();
    let target = ((triangles.len() as f64) * f64::from(ratio.max(0.0))).round() as usize;
    let mut alive = vec![true; triangles.len()];
    let mut alive_count = triangles.len();
    let mut around: Vec<Vec<usize>> = vec![Vec::new(); positions.len()];
    for (index, triangle) in triangles.iter().enumerate() {
        for vertex in triangle {
            if let Some(list) = around.get_mut(*vertex as usize) {
                list.push(index);
            }
        }
    }
    let point = |positions: &[DVec3], vertex: u32| positions.get(vertex as usize).copied().unwrap_or(DVec3::ZERO);
    let mut quadrics = vec![DMat4::ZERO; positions.len()];
    let mut edge_uses: HashMap<(u32, u32), u32> = HashMap::new();
    for triangle in &triangles {
        let [a, b, c] = *triangle;
        let q = plane_quadric(point(&positions, a), point(&positions, b), point(&positions, c));
        for vertex in triangle {
            if let Some(slot) = quadrics.get_mut(*vertex as usize) {
                *slot += q;
            }
        }
        for (x, y) in [(a, b), (b, c), (c, a)] {
            *edge_uses.entry((x.min(y), x.max(y))).or_default() += 1;
        }
    }
    // Boundary edges get a heavy plane through them, perpendicular to their triangle.
    for triangle in &triangles {
        let [a, b, c] = *triangle;
        let normal = (point(&positions, b) - point(&positions, a)).cross(point(&positions, c) - point(&positions, a));
        for (x, y) in [(a, b), (b, c), (c, a)] {
            if edge_uses.get(&(x.min(y), x.max(y))) == Some(&1) {
                let (px, py) = (point(&positions, x), point(&positions, y));
                let q = plane_quadric(px, py, px + normal) * 1000.0;
                for vertex in [x, y] {
                    if let Some(slot) = quadrics.get_mut(vertex as usize) {
                        *slot += q;
                    }
                }
            }
        }
    }
    let mut versions = vec![0u32; positions.len()];
    let mut removed = vec![false; positions.len()];
    let best = |quadrics: &[DMat4], positions: &[DVec3], u: u32, v: u32| -> (f64, DVec3) {
        let q = quadrics.get(u as usize).copied().unwrap_or(DMat4::ZERO) + quadrics.get(v as usize).copied().unwrap_or(DMat4::ZERO);
        let (pu, pv) = (point(positions, u), point(positions, v));
        [pu, pv, (pu + pv) * 0.5]
            .into_iter()
            .map(|candidate| (error(&q, candidate), candidate))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .unwrap_or((0.0, pu))
    };
    let mut heap = BinaryHeap::new();
    for &(u, v) in edge_uses.keys() {
        let (cost, _) = best(&quadrics, &positions, u, v);
        heap.push(Candidate { cost, edge: (u, v), stamp: (0, 0) });
    }
    let neighbours = |around: &[Vec<usize>], triangles: &[[u32; 3]], alive: &[bool], vertex: u32| -> BTreeSet<u32> {
        let mut out = BTreeSet::new();
        for face in around.get(vertex as usize).map_or(&[][..], Vec::as_slice) {
            if alive.get(*face).copied().unwrap_or(false)
                && let Some(triangle) = triangles.get(*face)
            {
                out.extend(triangle.iter().copied().filter(|other| *other != vertex));
            }
        }
        out
    };
    while alive_count > target {
        let Some(Candidate { edge: (u, v), stamp, .. }) = heap.pop() else { break };
        let (Some(&vu), Some(&vv)) = (versions.get(u as usize), versions.get(v as usize)) else { continue };
        if removed.get(u as usize).copied().unwrap_or(true) || removed.get(v as usize).copied().unwrap_or(true) || stamp != (vu, vv) {
            continue;
        }
        let near_u = neighbours(&around, &triangles, &alive, u);
        if !near_u.contains(&v) {
            continue;
        }
        let near_v = neighbours(&around, &triangles, &alive, v);
        // Link condition: the shared neighbours are exactly the edge's opposite corners.
        let shared_faces: Vec<usize> = around
            .get(u as usize)
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .copied()
            .filter(|face| alive.get(*face).copied().unwrap_or(false) && triangles.get(*face).is_some_and(|triangle| triangle.contains(&v)))
            .collect();
        let opposite: BTreeSet<u32> = shared_faces
            .iter()
            .filter_map(|face| triangles.get(*face))
            .flat_map(|triangle| triangle.iter().copied())
            .filter(|other| *other != u && *other != v)
            .collect();
        let common: BTreeSet<u32> = near_u.intersection(&near_v).copied().collect();
        if common != opposite {
            continue;
        }
        let (_, target_point) = best(&quadrics, &positions, u, v);
        // No triangle may fold over.
        let mut folds = false;
        for (vertex, other) in [(u, v), (v, u)] {
            for face in around.get(vertex as usize).map_or(&[][..], Vec::as_slice) {
                if !alive.get(*face).copied().unwrap_or(false) {
                    continue;
                }
                let Some(triangle) = triangles.get(*face) else { continue };
                if triangle.contains(&other) {
                    continue;
                }
                let corners = triangle.map(|corner| point(&positions, corner));
                let moved = triangle.map(|corner| if corner == vertex { target_point } else { point(&positions, corner) });
                let before = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
                let after = (moved[1] - moved[0]).cross(moved[2] - moved[0]);
                if before.dot(after) <= 0.0 || after.length() < 1e-12 {
                    folds = true;
                }
            }
        }
        if folds {
            continue;
        }
        // Collapse v into u.
        for face in shared_faces {
            if let Some(flag) = alive.get_mut(face)
                && *flag
            {
                *flag = false;
                alive_count -= 1;
            }
        }
        let moved_faces = around.get(v as usize).cloned().unwrap_or_default();
        for face in &moved_faces {
            if let Some(triangle) = triangles.get_mut(*face) {
                for corner in triangle.iter_mut() {
                    if *corner == v {
                        *corner = u;
                    }
                }
            }
        }
        if let Some(list) = around.get_mut(u as usize) {
            list.extend(moved_faces);
        }
        if let Some(slot) = positions.get_mut(u as usize) {
            *slot = target_point;
        }
        let qv = quadrics.get(v as usize).copied().unwrap_or(DMat4::ZERO);
        if let Some(slot) = quadrics.get_mut(u as usize) {
            *slot += qv;
        }
        if let Some(flag) = removed.get_mut(v as usize) {
            *flag = true;
        }
        if let Some(version) = versions.get_mut(u as usize) {
            *version += 1;
        }
        for other in neighbours(&around, &triangles, &alive, u) {
            let (a, b) = (u.min(other), u.max(other));
            let (cost, _) = best(&quadrics, &positions, a, b);
            let stamp = (versions.get(a as usize).copied().unwrap_or(0), versions.get(b as usize).copied().unwrap_or(0));
            heap.push(Candidate { cost, edge: (a, b), stamp });
        }
    }
    let mut out = Mesh {
        positions: positions.iter().map(|point| point.as_vec3()).collect(),
        smooth: mesh.smooth,
        smooth_angle: mesh.smooth_angle,
        ..Default::default()
    };
    out.faces = triangles.iter().zip(&alive).filter(|(_, alive)| **alive).map(|(triangle, _)| triangle.to_vec()).collect();
    out.remove_unused_vertices();
    if out.validate().is_err() {
        return mesh.clone();
    }
    out
}

/// The modifier a `.blend` modifier struct describes, for the kinds of this set. `get` reads a
/// number field (element `index` of an array field).
pub fn from_blend(struct_name: &str, get: &dyn Fn(&str, usize) -> Option<f64>) -> Option<ModifierKind> {
    let number = |field: &str| get(field, 0).map(|value| value as f32).filter(|value| value.is_finite());
    let int = |field: &str| get(field, 0).map(|value| value as i64);
    let axis = |value: i64| match value {
        0 => ScrewAxis::X,
        1 => ScrewAxis::Y,
        _ => ScrewAxis::Z,
    };
    match struct_name {
        "DisplaceModifierData" => Some(ModifierKind::Displace {
            strength: number("strength").unwrap_or(1.0),
            mid_level: number("midlevel").unwrap_or(0.5),
            direction: match int("direction").unwrap_or(3) {
                0 => DisplaceDirection::X,
                1 => DisplaceDirection::Y,
                2 => DisplaceDirection::Z,
                _ => DisplaceDirection::Normal,
            },
            noise_scale: 0.0,
            image: None,
            coordinates: TextureCoordinates::Local,
        }),
        "DecimateModifierData" => Some(ModifierKind::Decimate {
            mode: match int("mode").unwrap_or(0) {
                1 => crate::modifiers::DecimateMode::Unsubdivide,
                2 => crate::modifiers::DecimateMode::Planar,
                _ => crate::modifiers::DecimateMode::Collapse,
            },
            ratio: number("percent").unwrap_or(1.0).clamp(0.0, 1.0),
            iterations: int("iter").unwrap_or(0).clamp(0, 16) as u32,
            angle_limit: number("angle").unwrap_or(5f32.to_radians()).clamp(0.0, std::f32::consts::PI),
        }),
        "WireframeModifierData" => Some(ModifierKind::Wireframe {
            thickness: number("offset").unwrap_or(0.02),
            offset: number("offset_fac").unwrap_or(0.0).clamp(-1.0, 1.0),
        }),
        "SimpleDeformModifierData" => {
            let factor = number("factor").unwrap_or(std::f32::consts::FRAC_PI_4);
            let low = get("limit", 0).map_or(0.0, |value| value as f32).clamp(0.0, 1.0);
            let high = get("limit", 1).map_or(1.0, |value| value as f32).clamp(low, 1.0);
            Some(ModifierKind::SimpleDeform {
                mode: match int("mode").unwrap_or(1) {
                    2 => DeformMode::Bend,
                    3 => DeformMode::Taper,
                    4 => DeformMode::Stretch,
                    _ => DeformMode::Twist,
                },
                axis: axis(int("deform_axis").unwrap_or(0)),
                angle: factor,
                factor,
                limits: [low, high],
            })
        }
        "CastModifierData" if int("type").unwrap_or(0) == 0 => {
            let flag = int("flag").unwrap_or(14);
            Some(ModifierKind::Cast { factor: number("fac").unwrap_or(0.5), axes: [flag & 2 != 0, flag & 4 != 0, flag & 8 != 0] })
        }
        _ => None,
    }
}
