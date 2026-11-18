//! Texture coordinates: the layouts primitives are made with (the reference application's, so
//! textures made for its primitives fit ours), carrying coordinates through edits, and the
//! projections and unwrap people run from the UV menu.

use std::collections::{BTreeSet, HashMap};
use std::f32::consts::{PI, TAU};

use glam::{Vec2, Vec3};

use crate::mesh::{Mesh, newell_normal};
use crate::primitives::{Fill, Primitive};

/// The layout a primitive gets from Add ▸ Mesh.
pub fn primitive_uvs(primitive: &Primitive, mesh: &Mesh) -> Vec<Vec<Vec2>> {
    let per_corner = |rule: &dyn Fn(usize, &[u32]) -> Vec<Vec2>| -> Vec<Vec<Vec2>> {
        mesh.faces.iter().enumerate().map(|(index, face)| rule(index, face)).collect()
    };
    let at = |vertex: u32| mesh.positions.get(vertex as usize).copied().unwrap_or(Vec3::ZERO);
    match *primitive {
        Primitive::Plane { size } | Primitive::Grid { size, .. } => {
            let half = (size / 2.0).max(1e-6);
            per_corner(&|_, face| face.iter().map(|vertex| planar(at(*vertex), half)).collect())
        }
        Primitive::Circle { radius, fill, .. } => {
            if fill == Fill::Nothing {
                return Vec::new();
            }
            let half = radius.max(1e-6);
            per_corner(&|_, face| face.iter().map(|vertex| planar(at(*vertex), half)).collect())
        }
        Primitive::Cube { size } => {
            let half = (size / 2.0).max(1e-6);
            per_corner(&|_, face| {
                let normal = mesh.face_normal(face);
                face.iter().map(|vertex| cube_corner(at(*vertex) / half, normal)).collect()
            })
        }
        Primitive::UvSphere { .. } | Primitive::IcoSphere { .. } => per_corner(&|_, face| sphere_face(&mesh.face_positions(face))),
        Primitive::Cylinder { radius, depth, .. } => cone_uvs(mesh, radius, radius, depth),
        Primitive::Cone { radius1, radius2, depth, .. } => cone_uvs(mesh, radius1, radius2, depth),
        Primitive::Torus { major_segments, minor_segments, major_radius, .. } => {
            per_corner(&|_, face| torus_face(&mesh.face_positions(face), major_segments, minor_segments, major_radius))
        }
    }
}

/// Plane and grid: the face's square mapped to the whole texture.
fn planar(point: Vec3, half: f32) -> Vec2 {
    Vec2::new(point.x / half * 0.5 + 0.5, point.y / half * 0.5 + 0.5)
}

/// The cube's cross: each side's square in its slot of a 4 × 4 grid (quarters, eighths).
fn cube_corner(p: Vec3, normal: Vec3) -> Vec2 {
    let (x, y, z) = (p.x, p.y, p.z);
    let eighth = |value: f32| (value + 1.0) / 8.0;
    if normal.x < -0.5 {
        Vec2::new(0.375 + eighth(z), eighth(y))
    } else if normal.y > 0.5 {
        Vec2::new(0.375 + eighth(z), 0.25 + eighth(x))
    } else if normal.x > 0.5 {
        Vec2::new(0.375 + eighth(z), 0.5 + eighth(-y))
    } else if normal.y < -0.5 {
        Vec2::new(0.375 + eighth(z), 0.75 + eighth(-x))
    } else if normal.z < -0.5 {
        Vec2::new(0.125 + eighth(x), 0.5 + eighth(-y))
    } else {
        Vec2::new(0.625 + eighth(-x), 0.5 + eighth(-y))
    }
}

/// Moves values across the seam so one face's corners stay together: when a face spans more
/// than half the texture, its corners near 0 are taken to 1.
fn close_seam(values: &mut [f32], skip: &[bool]) {
    let used: Vec<f32> = values.iter().zip(skip).filter(|(_, skip)| !**skip).map(|(value, _)| *value).collect();
    let (low, high) = used.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), value| (low.min(*value), high.max(*value)));
    if high - low > 0.5 {
        for (value, skip) in values.iter_mut().zip(skip) {
            if !*skip && *value < 0.5 {
                *value += 1.0;
            }
        }
    }
}

/// A fraction in [0, 1) of a full turn starting at `start` (a fraction).
fn turn_fraction(angle: f32, start: f32) -> f32 {
    (start + angle / TAU).rem_euclid(1.0)
}

/// A sphere face: longitude from the +X side (0.5 there, rising towards +Y), latitude from the
/// bottom pole. Pole corners sit between their face's other corners.
fn sphere_face(points: &[Vec3]) -> Vec<Vec2> {
    let poles: Vec<bool> = points.iter().map(|point| Vec2::new(point.x, point.y).length() < 1e-5 * point.length().max(1.0)).collect();
    let mut u: Vec<f32> = points.iter().map(|point| turn_fraction(point.y.atan2(point.x), 0.5)).collect();
    close_seam(&mut u, &poles);
    let others: Vec<f32> = u.iter().zip(&poles).filter(|(_, pole)| !**pole).map(|(value, _)| *value).collect();
    let middle = if others.is_empty() { 0.5 } else { others.iter().sum::<f32>() / others.len() as f32 };
    points
        .iter()
        .zip(u)
        .zip(&poles)
        .map(|((point, u), pole)| {
            let polar = (point.z / point.length().max(1e-9)).clamp(-1.0, 1.0).acos();
            Vec2::new(if *pole { middle } else { u }, 1.0 - polar / PI)
        })
        .collect()
}

/// Cylinders and cones: the side unrolled across the top half (U from 1 at +Y falling towards
/// +X), the top cap in the bottom-left quarter and the bottom cap in the bottom-right one. A cone
/// that closes to a point maps its side like the top cap.
fn cone_uvs(mesh: &Mesh, bottom_radius: f32, top_radius: f32, depth: f32) -> Vec<Vec<Vec2>> {
    let half = depth / 2.0;
    let cap = |point: Vec3, centre: Vec2, radius: f32| centre + Vec2::new(point.x, point.y) / radius.max(1e-6) * 0.24;
    mesh.faces
        .iter()
        .map(|face| {
            let points = mesh.face_positions(face);
            let normal = newell_normal(&points);
            let on_top = points.iter().all(|point| (point.z - half).abs() < 1e-5);
            let on_bottom = points.iter().all(|point| (point.z + half).abs() < 1e-5);
            if on_top && normal.z > 0.5 {
                return points.iter().map(|point| cap(*point, Vec2::new(0.25, 0.25), top_radius)).collect();
            }
            if on_bottom && normal.z < -0.5 {
                return points.iter().map(|point| cap(*point, Vec2::new(0.75, 0.25), bottom_radius)).collect();
            }
            if top_radius == 0.0 {
                return points.iter().map(|point| cap(*point, Vec2::new(0.25, 0.25), bottom_radius)).collect();
            }
            if bottom_radius == 0.0 {
                return points.iter().map(|point| cap(*point, Vec2::new(0.75, 0.25), top_radius)).collect();
            }
            let mut u: Vec<f32> = points.iter().map(|point| turn_fraction(-point.x.atan2(point.y), 0.0)).collect();
            close_seam(&mut u, &vec![false; points.len()]);
            points.iter().zip(u).map(|(point, u)| Vec2::new(u, if point.z > 0.0 { 1.0 } else { 0.5 })).collect()
        })
        .collect()
}

/// A torus face: U around the ring and V around the tube, each counted in segments from the
/// segment opposite the start, as the reference lays them out.
fn torus_face(points: &[Vec3], major: u32, minor: u32, major_radius: f32) -> Vec<Vec2> {
    let step = |angle: f32, count: u32| -> f32 {
        let count = count.max(1);
        let index = (angle.rem_euclid(TAU) / TAU * count as f32).round() as u32 % count;
        ((index + count.div_ceil(2)) % count) as f32 / count as f32
    };
    let mut u: Vec<f32> = points.iter().map(|point| step(point.y.atan2(point.x), major)).collect();
    let mut v: Vec<f32> = points
        .iter()
        .map(|point| {
            let ring = Vec2::new(point.x, point.y).length() - major_radius;
            step(point.z.atan2(ring), minor)
        })
        .collect();
    let none = vec![false; points.len()];
    close_seam(&mut u, &none);
    close_seam(&mut v, &none);
    u.into_iter().zip(v).map(|(u, v)| Vec2::new(u, v)).collect()
}

/// Runs an edit of the mesh and carries its texture coordinates through it, and its sculpt face
/// sets and hidden faces (each face takes those of the old face it is, lies in, or is next to).
pub fn with_uvs<T>(mesh: &mut Mesh, edit: impl FnOnce(&mut Mesh) -> T) -> T {
    let had_uvs = mesh.has_uvs() && mesh.uvs_aligned();
    let had_face_data = has_face_data(mesh);
    let before = (had_uvs || had_face_data).then(|| mesh.clone());
    let marks = crate::seams::Marks::of(mesh);
    // Vertex groups and shape keys go along by position.
    let vertex_data = crate::vertex_data::snapshot(mesh);
    let out = edit(mesh);
    if !marks.is_empty() || !mesh.seams.is_empty() || !mesh.pinned.is_empty() {
        crate::seams::carry(&marks, mesh);
    }
    crate::vertex_data::carry(vertex_data.as_ref(), mesh);
    let Some(before) = before else {
        if !mesh.uvs_aligned() {
            mesh.uvs.clear();
        }
        crate::sculpt::face_sets::tidy(mesh);
        return out;
    };
    let same_faces = before.faces == mesh.faces;
    let carry_uvs = had_uvs && !(mesh.uvs_aligned() && same_faces && before.positions == mesh.positions);
    let carry_face_data = had_face_data && !(same_faces && has_face_data(mesh));
    if carry_uvs || carry_face_data {
        let from = face_sources(&before, mesh);
        if carry_uvs {
            carry_from(&before, mesh, &from);
        }
        if carry_face_data {
            carry_faces(&before, mesh, &from);
        }
    }
    if !mesh.uvs_aligned() {
        mesh.uvs.clear();
    }
    crate::sculpt::face_sets::tidy(mesh);
    out
}

/// Whether the mesh has face sets or hidden faces, one per face.
fn has_face_data(mesh: &Mesh) -> bool {
    let faces = mesh.faces.len();
    (!mesh.face_sets.is_empty() && mesh.face_sets.len() == faces) || (!mesh.hidden.is_empty() && mesh.hidden.len() == faces)
}

/// Gives `after`'s faces the face sets and hidden flags of the old faces `from` names (none
/// and shown where it names none).
fn carry_faces(before: &Mesh, after: &mut Mesh, from: &[Option<usize>]) {
    let faces = before.faces.len();
    if before.face_sets.len() == faces && !before.face_sets.is_empty() {
        after.face_sets = from.iter().map(|source| source.and_then(|face| before.face_sets.get(face)).copied().unwrap_or(0)).collect();
    } else {
        after.face_sets.clear();
    }
    if before.hidden.len() == faces && !before.hidden.is_empty() {
        after.hidden = from.iter().map(|source| source.and_then(|face| before.hidden.get(face)).copied().unwrap_or(false)).collect();
    } else {
        after.hidden.clear();
    }
}

/// A copy of the mesh to carry texture coordinates from, when it has any.
pub fn snapshot(mesh: &Mesh) -> Option<Mesh> {
    (mesh.has_uvs() && mesh.uvs_aligned()).then(|| mesh.clone())
}

fn position_key(point: Vec3) -> [u32; 3] {
    point.to_array().map(|value| (value + 0.0).to_bits())
}

/// Gives `after` (an edit of `before`) texture coordinates: faces that are still there keep
/// theirs exactly, and new or changed faces take them from the old face they lie in or next to,
/// corners at old corners copying those exactly (so seams stay where they were).
pub fn carry(before: Option<&Mesh>, after: &mut Mesh) {
    let Some(before) = before else {
        if !after.uvs_aligned() {
            after.uvs.clear();
        }
        return;
    };
    let from = face_sources(before, after);
    carry_from(before, after, &from);
}

/// [`carry`] with the old face each new face comes from worked out.
fn carry_from(before: &Mesh, after: &mut Mesh, from: &[Option<usize>]) {
    let uvs = after
        .faces
        .iter()
        .zip(from)
        .map(|(face, source)| {
            let points = after.face_positions(face);
            match source {
                Some(old) => points.iter().map(|point| exact(before, *old, *point).unwrap_or_else(|| interpolate(before, *old, *point))).collect(),
                None => vec![Vec2::ZERO; face.len()],
            }
        })
        .collect();
    after.uvs = uvs;
}

/// The old face each face of `after` (an edit of `before`) comes from: the one with the same
/// corner positions, else, of the old faces at any of its corners (or round the old vertex
/// nearest its centre), the one it lies in or next to best.
pub fn face_sources(before: &Mesh, after: &Mesh) -> Vec<Option<usize>> {
    // Faces by the set of their corner positions.
    let mut by_corners: HashMap<Vec<[u32; 3]>, usize> = HashMap::new();
    for (index, face) in before.faces.iter().enumerate() {
        let mut key: Vec<[u32; 3]> = before.face_positions(face).into_iter().map(position_key).collect();
        key.sort_unstable();
        by_corners.entry(key).or_insert(index);
    }
    // Old corners by position, and old faces by vertex.
    let mut corners_at: HashMap<[u32; 3], Vec<(usize, usize)>> = HashMap::new();
    for (face, corners) in before.faces.iter().enumerate() {
        for (corner, vertex) in corners.iter().enumerate() {
            if let Some(point) = before.positions.get(*vertex as usize) {
                corners_at.entry(position_key(*point)).or_default().push((face, corner));
            }
        }
    }
    let grid = Grid::new(&before.positions);
    let mut faces_of: Vec<Vec<usize>> = vec![Vec::new(); before.positions.len()];
    for (face, corners) in before.faces.iter().enumerate() {
        for vertex in corners {
            if let Some(list) = faces_of.get_mut(*vertex as usize) {
                list.push(face);
            }
        }
    }
    let mut sources = Vec::with_capacity(after.faces.len());
    for face in &after.faces {
        let points = after.face_positions(face);
        let mut key: Vec<[u32; 3]> = points.iter().map(|point| position_key(*point)).collect();
        key.sort_unstable();
        if let Some(&old) = by_corners.get(&key) {
            sources.push(Some(old));
            continue;
        }
        // Candidates: old faces at any of this face's corners, else the nearest old vertex's.
        let mut candidates: BTreeSet<usize> =
            points.iter().filter_map(|point| corners_at.get(&position_key(*point))).flatten().map(|(face, _)| *face).collect();
        if candidates.is_empty() {
            let centre = points.iter().copied().sum::<Vec3>() / points.len().max(1) as f32;
            if let Some(vertex) = grid.nearest(&before.positions, centre) {
                candidates.extend(faces_of.get(vertex).into_iter().flatten().copied());
            }
        }
        sources.push(candidates.into_iter().min_by(|a, b| fit(before, *a, &points).total_cmp(&fit(before, *b, &points))));
    }
    sources
}

/// The coordinate of old face `face`'s corner at `point`, if it has one there.
fn exact(mesh: &Mesh, face: usize, point: Vec3) -> Option<Vec2> {
    let corners = mesh.faces.get(face)?;
    let key = position_key(point);
    corners
        .iter()
        .position(|vertex| mesh.positions.get(*vertex as usize).is_some_and(|other| position_key(*other) == key))
        .map(|corner| mesh.corner_uv(face, corner))
}

/// How badly `points` sit in old face `face`'s plane and outline (smaller is better).
fn fit(mesh: &Mesh, face: usize, points: &[Vec3]) -> f32 {
    let Some(corners) = mesh.faces.get(face) else { return f32::INFINITY };
    let outline = mesh.face_positions(corners);
    let normal = newell_normal(&outline);
    let centre = mesh.face_center(corners);
    let size = outline.iter().map(|point| point.distance(centre)).fold(0.0f32, f32::max).max(1e-6);
    points.iter().map(|point| (point - centre).dot(normal).abs() + (point.distance(centre) / size - 1.0).max(0.0) * size).sum()
}

/// The coordinate at `point` inside old face `face`: projected into its plane and found in the
/// nearest of its triangles (clamped to it).
fn interpolate(mesh: &Mesh, face: usize, point: Vec3) -> Vec2 {
    let Some(corners) = mesh.faces.get(face) else { return Vec2::ZERO };
    let outline = mesh.face_positions(corners);
    let normal = newell_normal(&outline);
    let mut best: Option<(f32, Vec2)> = None;
    let count = outline.len();
    for slot in 1..count.saturating_sub(1) {
        let (Some(a), Some(b), Some(c)) = (outline.first(), outline.get(slot), outline.get(slot + 1)) else { continue };
        let projected = point - normal * (point - *a).dot(normal);
        let (weights, distance) = barycentric(*a, *b, *c, projected);
        let uv = mesh.corner_uv(face, 0) * weights.x + mesh.corner_uv(face, slot) * weights.y + mesh.corner_uv(face, slot + 1) * weights.z;
        if best.is_none_or(|(nearest, _)| distance < nearest) {
            best = Some((distance, uv));
        }
    }
    best.map_or(Vec2::ZERO, |(_, uv)| uv)
}

/// Clamped barycentric weights of `p` in triangle `a b c`, and how far outside it lies.
fn barycentric(a: Vec3, b: Vec3, c: Vec3, p: Vec3) -> (Vec3, f32) {
    let (v0, v1, v2) = (b - a, c - a, p - a);
    let (d00, d01, d11, d20, d21) = (v0.dot(v0), v0.dot(v1), v1.dot(v1), v2.dot(v0), v2.dot(v1));
    let denominator = d00 * d11 - d01 * d01;
    if denominator.abs() < 1e-20 {
        return (Vec3::new(1.0, 0.0, 0.0), p.distance(a));
    }
    let v = (d11 * d20 - d01 * d21) / denominator;
    let w = (d00 * d21 - d01 * d20) / denominator;
    let u = 1.0 - v - w;
    let outside = (-u).max(0.0) + (-v).max(0.0) + (-w).max(0.0);
    let clamped = Vec3::new(u.max(0.0), v.max(0.0), w.max(0.0));
    let sum = clamped.x + clamped.y + clamped.z;
    (if sum > 0.0 { clamped / sum } else { Vec3::new(1.0, 0.0, 0.0) }, outside)
}

/// A coarse spatial grid over points, for nearest-point questions.
struct Grid {
    cell: f32,
    cells: HashMap<(i64, i64, i64), Vec<usize>>,
}

impl Grid {
    fn new(points: &[Vec3]) -> Self {
        let bounds = points.iter().fold(crate::mesh::Bounds::EMPTY, |bounds, point| bounds.with(*point));
        let size = bounds.size().max_element().max(1e-3);
        let cell = size / (points.len() as f32).cbrt().max(1.0);
        let mut cells: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
        for (index, point) in points.iter().enumerate() {
            cells.entry(Self::key(*point, cell)).or_default().push(index);
        }
        Self { cell, cells }
    }

    fn key(point: Vec3, cell: f32) -> (i64, i64, i64) {
        ((point.x / cell).floor() as i64, (point.y / cell).floor() as i64, (point.z / cell).floor() as i64)
    }

    fn nearest(&self, points: &[Vec3], target: Vec3) -> Option<usize> {
        let (cx, cy, cz) = Self::key(target, self.cell);
        for reach in 0..64i64 {
            let mut best: Option<(f32, usize)> = None;
            for dx in -reach..=reach {
                for dy in -reach..=reach {
                    for dz in -reach..=reach {
                        if dx.abs().max(dy.abs()).max(dz.abs()) != reach {
                            continue;
                        }
                        for index in self.cells.get(&(cx + dx, cy + dy, cz + dz)).into_iter().flatten() {
                            let distance = points.get(*index).map_or(f32::INFINITY, |point| point.distance_squared(target));
                            if best.is_none_or(|(nearest, _)| distance < nearest) {
                                best = Some((distance, *index));
                            }
                        }
                    }
                }
            }
            if let Some((_, index)) = best {
                return Some(index);
            }
        }
        None
    }
}

/// How a projection lays out the faces it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projection {
    /// Each face from the box side it faces most (six planar projections).
    Cube,
    /// Around the object's local Z.
    Cylinder,
    /// Longitude and latitude around the object's origin.
    Sphere,
    /// Straight down one local axis.
    Planar(Axis),
    /// Every face fills the whole texture.
    Reset,
}

/// A local axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

/// Projects texture coordinates onto `faces` (all faces when empty). `size` scales the cube
/// and planar projections (metres per texture repeat). Gives the mesh coordinates first if it
/// had none.
pub fn project(mesh: &mut Mesh, faces: &BTreeSet<u32>, projection: Projection, size: f32) {
    mesh.fill_uvs();
    let size = if size.is_finite() && size > 0.0 { size } else { 2.0 };
    let bounds = mesh.bounds(glam::Mat4::IDENTITY);
    let centre = if bounds.is_empty() { Vec3::ZERO } else { bounds.center() };
    for (index, face) in mesh.faces.iter().enumerate() {
        if !faces.is_empty() && !faces.contains(&u32::try_from(index).unwrap_or(u32::MAX)) {
            continue;
        }
        let points = mesh.face_positions(face);
        let normal = newell_normal(&points);
        let projected: Vec<Vec2> = match projection {
            Projection::Cube => {
                let axis = dominant(normal);
                points.iter().map(|point| plane_uv(*point - centre, axis, size)).collect()
            }
            Projection::Planar(axis) => points.iter().map(|point| plane_uv(*point - centre, axis, size)).collect(),
            Projection::Cylinder => {
                let mut u: Vec<f32> = points.iter().map(|point| turn_fraction((point.y - centre.y).atan2(point.x - centre.x), 0.5)).collect();
                close_seam(&mut u, &vec![false; points.len()]);
                let height = bounds.size().z.max(1e-6);
                points.iter().zip(u).map(|(point, u)| Vec2::new(u, (point.z - bounds.min.z) / height)).collect()
            }
            Projection::Sphere => sphere_face(&points.iter().map(|point| *point - centre).collect::<Vec<_>>()),
            Projection::Reset => reset_face(points.len()),
        };
        if let Some(slot) = mesh.uvs.get_mut(index) {
            *slot = projected;
        }
    }
}

fn dominant(normal: Vec3) -> Axis {
    let a = normal.abs();
    if a.x >= a.y && a.x >= a.z {
        Axis::X
    } else if a.y >= a.z {
        Axis::Y
    } else {
        Axis::Z
    }
}

fn plane_uv(point: Vec3, axis: Axis, size: f32) -> Vec2 {
    let flat = match axis {
        Axis::X => Vec2::new(point.y, point.z),
        Axis::Y => Vec2::new(point.x, point.z),
        Axis::Z => Vec2::new(point.x, point.y),
    };
    flat / size + Vec2::splat(0.5)
}

/// A face's corners around the unit square (a quad fills it; other faces are set on a circle).
fn reset_face(count: usize) -> Vec<Vec2> {
    if count == 4 {
        return vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0)];
    }
    (0..count)
        .map(|corner| {
            let angle = TAU * corner as f32 / count as f32 - PI / 2.0;
            Vec2::new(0.5 + 0.5 * angle.cos(), 0.5 + 0.5 * angle.sin())
        })
        .collect()
}

/// A flattened chart: each face's coordinates in metres, and the chart's lower and upper corners.
type Chart = (Vec<(usize, Vec<Vec2>)>, Vec2, Vec2);

/// An automatic unwrap: faces are grouped into charts of neighbours whose normals stay within
/// `angle_limit` (radians) of the chart's first face, each chart is projected flat along its
/// mean normal, and the charts are packed onto the texture in shelves with `margin` between them
/// (a fraction of the texture). Covers `faces` (all when empty).
pub fn smart_project(mesh: &mut Mesh, faces: &BTreeSet<u32>, angle_limit: f32, margin: f32) {
    mesh.fill_uvs();
    let count = mesh.faces.len();
    let chosen: Vec<bool> = (0..count).map(|index| faces.is_empty() || faces.contains(&u32::try_from(index).unwrap_or(u32::MAX))).collect();
    let normals: Vec<Vec3> = mesh.faces.iter().map(|face| mesh.face_normal(face)).collect();
    let mut by_edge: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (index, face) in mesh.faces.iter().enumerate() {
        for (a, b) in crate::mesh::face_edges(face) {
            by_edge.entry((a.min(b), a.max(b))).or_default().push(index);
        }
    }
    let limit = angle_limit.clamp(0.0, PI).cos();
    let mut chart_of = vec![usize::MAX; count];
    let mut charts: Vec<Vec<usize>> = Vec::new();
    for start in 0..count {
        if !chosen.get(start).copied().unwrap_or(false) || chart_of.get(start) != Some(&usize::MAX) {
            continue;
        }
        let id = charts.len();
        let seed = normals.get(start).copied().unwrap_or(Vec3::Z);
        let mut members = vec![start];
        if let Some(slot) = chart_of.get_mut(start) {
            *slot = id;
        }
        let mut cursor = 0;
        while let Some(&face) = members.get(cursor) {
            cursor += 1;
            let Some(corners) = mesh.faces.get(face) else { continue };
            for (a, b) in crate::mesh::face_edges(corners) {
                for &other in by_edge.get(&(a.min(b), a.max(b))).map_or(&[][..], Vec::as_slice) {
                    let free = chosen.get(other).copied().unwrap_or(false) && chart_of.get(other) == Some(&usize::MAX);
                    if free && normals.get(other).is_some_and(|normal| normal.dot(seed) >= limit) {
                        if let Some(slot) = chart_of.get_mut(other) {
                            *slot = id;
                        }
                        members.push(other);
                    }
                }
            }
        }
        charts.push(members);
    }
    // Flatten each chart along its mean normal, in metres.
    let mut flat: Vec<Chart> = Vec::new();
    for members in &charts {
        let normal = members.iter().filter_map(|face| normals.get(*face)).copied().sum::<Vec3>().normalize_or_zero();
        let normal = if normal == Vec3::ZERO { Vec3::Z } else { normal };
        let (u_axis, v_axis) = normal.any_orthonormal_pair();
        let mut low = Vec2::splat(f32::INFINITY);
        let mut high = Vec2::splat(f32::NEG_INFINITY);
        let mut laid = Vec::with_capacity(members.len());
        for face in members {
            let Some(corners) = mesh.faces.get(*face) else { continue };
            let uv: Vec<Vec2> = mesh.face_positions(corners).iter().map(|point| Vec2::new(point.dot(u_axis), point.dot(v_axis))).collect();
            for point in &uv {
                low = low.min(*point);
                high = high.max(*point);
            }
            laid.push((*face, uv));
        }
        if low.x.is_finite() {
            flat.push((laid, low, high));
        }
    }
    // Shelf packing, tallest charts first, into a square whose side fits the total area.
    flat.sort_by(|a, b| (b.2.y - b.1.y).total_cmp(&(a.2.y - a.1.y)));
    let area: f32 = flat.iter().map(|(_, low, high)| (high.x - low.x) * (high.y - low.y)).sum();
    let widest = flat.iter().map(|(_, low, high)| high.x - low.x).fold(0.0f32, f32::max);
    let side = area.sqrt().max(widest).max(1e-6) * 1.15;
    let gap = margin.max(0.0) * side;
    let (mut x, mut y, mut shelf) = (0.0f32, 0.0f32, 0.0f32);
    let mut placed = Vec::with_capacity(flat.len());
    for (laid, low, high) in flat {
        let size = high - low;
        if x > 0.0 && x + size.x > side {
            x = 0.0;
            y += shelf + gap;
            shelf = 0.0;
        }
        placed.push((laid, low, Vec2::new(x, y)));
        x += size.x + gap;
        shelf = shelf.max(size.y);
    }
    let extent = side.max(y + shelf).max(1e-6);
    for (laid, low, at) in placed {
        for (face, uv) in laid {
            if let Some(slot) = mesh.uvs.get_mut(face) {
                *slot = uv.into_iter().map(|point| (point - low + at) / extent).collect();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carry_keeps_untouched_faces_and_seams() {
        let mut mesh = Primitive::Cube { size: 2.0 }.build().unwrap();
        let before = snapshot(&mesh);
        let top = mesh.faces.iter().position(|face| mesh.face_normal(face).z > 0.99).unwrap() as u32;
        crate::ops::extrude_region(&mut mesh, &[top], 1.0, Vec3::ZERO).unwrap();
        assert!(mesh.uvs_aligned());
        mesh.validate().unwrap();
        let mut again = before.clone().unwrap();
        carry(before.as_ref(), &mut again);
        assert_eq!(again.uvs, before.unwrap().uvs, "unchanged faces keep their coordinates exactly");
    }

    #[test]
    fn projections_cover_the_mesh() {
        let mut mesh = Primitive::UvSphere { segments: 8, rings: 4, radius: 1.0 }.build().unwrap();
        for projection in [Projection::Cube, Projection::Cylinder, Projection::Sphere, Projection::Planar(Axis::Z), Projection::Reset] {
            project(&mut mesh, &BTreeSet::new(), projection, 2.0);
            mesh.validate().unwrap();
        }
        smart_project(&mut mesh, &BTreeSet::new(), 66f32.to_radians(), 0.02);
        mesh.validate().unwrap();
        assert!(mesh.uvs.iter().flatten().all(|uv| (-1e-4..=1.0001).contains(&uv.x) && (-1e-4..=1.0001).contains(&uv.y)));
    }
}
