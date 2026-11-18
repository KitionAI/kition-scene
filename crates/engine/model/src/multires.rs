//! Multiresolution: a mesh sculpted at several subdivision levels at once, as the reference
//! application's Multiresolution modifier does.
//!
//! Each level is one more Catmull–Clark subdivision of the level below, shown on its limit
//! surface (so a level with no detail is the same shape as a Subdivision Surface modifier with
//! limit positions). The detail of level `k` is an offset per vertex of that level, kept in the
//! surface's own frame there (normal, and a tangent towards the vertex's first neighbour), so it
//! rides along when a lower level or the base mesh changes:
//!
//! - the control mesh of level 0 is the base mesh;
//! - level `k`'s surface is the limit surface of one subdivision of level `k - 1`'s control mesh;
//! - level `k` shows its surface plus its offsets, and its control mesh is that subdivision
//!   plus the same offsets.
//!
//! Sculpting at a level changes that level's offsets only: the levels above follow smoothly
//! through the subdivision, and the levels below stay. The offsets are stored on the mesh
//! ([`Mesh::multires`]); offsets that no longer fit the mesh's faces are ignored.

use glam::Vec3;

use crate::error::{Result, SceneError};
use crate::mesh::Mesh;
use crate::modifiers::{catmull_clark, limit_positions};

#[path = "multires_recover.rs"]
mod recovery;

/// The most levels a mesh may have.
pub const MAX_LEVELS: u32 = 6;

/// The most faces the highest level may have.
const MAX_FACES: usize = 2_000_000;

/// A vertex's frame on a surface: tangent, bitangent, normal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub tangent: Vec3,
    pub bitangent: Vec3,
    pub normal: Vec3,
}

impl Frame {
    /// The offset `local` (tangent, bitangent, normal) in object space.
    pub fn to_object(&self, local: Vec3) -> Vec3 {
        self.tangent * local.x + self.bitangent * local.y + self.normal * local.z
    }

    /// The object-space offset `offset` in this frame.
    pub fn to_local(&self, offset: Vec3) -> Vec3 {
        Vec3::new(offset.dot(self.tangent), offset.dot(self.bitangent), offset.dot(self.normal))
    }
}

/// Each vertex's frame on `mesh`: its normal, and the direction to its first neighbour (in the
/// lowest-numbered face it belongs to) made square to it.
pub fn frames(mesh: &Mesh) -> Vec<Frame> {
    let normals = mesh.vertex_normals();
    let mut towards: Vec<Option<u32>> = vec![None; mesh.positions.len()];
    for face in &mesh.faces {
        for (corner, vertex) in face.iter().enumerate() {
            if let Some(slot) = towards.get_mut(*vertex as usize)
                && slot.is_none()
            {
                *slot = face.get((corner + 1) % face.len()).copied();
            }
        }
    }
    mesh.positions
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let normal = normals.get(index).copied().filter(|normal| *normal != Vec3::ZERO).unwrap_or(Vec3::Z);
            let along = towards
                .get(index)
                .copied()
                .flatten()
                .and_then(|other| mesh.positions.get(other as usize))
                .map(|other| *other - *point)
                .unwrap_or(Vec3::X);
            let tangent = (along - normal * along.dot(normal)).normalize_or_zero();
            let tangent = if tangent == Vec3::ZERO { normal.any_orthonormal_vector() } else { tangent };
            Frame { tangent, bitangent: normal.cross(tangent), normal }
        })
        .collect()
}

/// How many vertices each level from 1 has, for a base mesh with `mesh`'s faces.
pub fn vertex_counts(mesh: &Mesh, levels: u32) -> Vec<usize> {
    let mut vertices = mesh.positions.len();
    let mut edges = mesh.edges().len();
    let mut faces = mesh.faces.len();
    let mut corners: usize = mesh.faces.iter().map(Vec::len).sum();
    let mut counts = Vec::new();
    for _ in 0..levels {
        // Every vertex stays, every edge and face adds one; each edge splits in two and each
        // corner adds an edge to its face's centre; each corner becomes a quad.
        let next_vertices = vertices + edges + faces;
        let next_edges = 2 * edges + corners;
        (vertices, edges, faces, corners) = (next_vertices, next_edges, corners, corners * 4);
        counts.push(vertices);
    }
    counts
}

/// The number of levels the mesh has, when its offsets fit its faces (else none).
pub fn levels(mesh: &Mesh) -> u32 {
    if mesh.multires.is_empty() {
        return 0;
    }
    let counts = vertex_counts(mesh, u32::try_from(mesh.multires.len()).unwrap_or(u32::MAX).min(MAX_LEVELS));
    let fits = counts.len() == mesh.multires.len() && mesh.multires.iter().zip(&counts).all(|(level, count)| level.len() == *count);
    if fits { u32::try_from(counts.len()).unwrap_or(0) } else { 0 }
}

/// One level worked out: what it shows, and the surface and frames its offsets are taken from.
#[derive(Debug, Clone)]
pub struct Level {
    /// The level as it shows (surface plus offsets), with the base's shading.
    pub mesh: Mesh,
    /// The surface without this level's offsets.
    pub surface: Vec<Vec3>,
    pub frames: Vec<Frame>,
}

/// A plain copy of the base mesh's shape: positions, faces, loose edges, shading.
fn shape(mesh: &Mesh) -> Mesh {
    Mesh {
        positions: mesh.positions.clone(),
        faces: mesh.faces.clone(),
        loose_edges: mesh.loose_edges.clone(),
        smooth: mesh.smooth,
        smooth_angle: mesh.smooth_angle,
        ..Mesh::default()
    }
}

/// Works out the levels up to `level` (at most the mesh's levels), calling `visit` with each
/// level from 1. The returned mesh is the control mesh of the last level.
fn walk(mesh: &Mesh, level: u32, mut visit: impl FnMut(u32, &Mesh, &[Frame], &[Vec3]) -> Option<Vec<Vec3>>) -> Mesh {
    let top = level.min(levels(mesh));
    let mut control = shape(mesh);
    for at in 1..=top {
        let subdivided = catmull_clark(&control);
        let mut surface = subdivided.clone();
        limit_positions(&mut surface);
        let frames = frames(&surface);
        let stored = mesh.multires.get(at as usize - 1).map_or(&[][..], Vec::as_slice);
        // A visitor may give the level new offsets (to rebuild detail level by level).
        let offsets = visit(at, &surface, &frames, stored);
        let offsets = offsets.as_deref().unwrap_or(stored);
        control = subdivided;
        for ((point, frame), offset) in control.positions.iter_mut().zip(&frames).zip(offsets) {
            *point += frame.to_object(*offset);
        }
    }
    control
}

/// The level `level` as it shows (the base mesh at 0); levels above the mesh's own show its
/// highest. Texture coordinates subdivide with the faces.
pub fn evaluate(mesh: &Mesh, level: u32) -> Mesh {
    let top = level.min(levels(mesh));
    if top == 0 {
        let mut out = mesh.clone();
        out.multires.clear();
        return out;
    }
    let mut shown = None;
    let mut control = shape(mesh);
    control.uvs = mesh.uvs.clone();
    for at in 1..=top {
        let subdivided = catmull_clark(&control);
        let mut surface = subdivided.clone();
        limit_positions(&mut surface);
        let frames = frames(&surface);
        let offsets = mesh.multires.get(at as usize - 1).map_or(&[][..], Vec::as_slice);
        if at == top {
            for ((point, frame), offset) in surface.positions.iter_mut().zip(&frames).zip(offsets) {
                *point += frame.to_object(*offset);
            }
            shown = Some(surface);
            break;
        }
        control = subdivided;
        for ((point, frame), offset) in control.positions.iter_mut().zip(&frames).zip(offsets) {
            *point += frame.to_object(*offset);
        }
    }
    shown.unwrap_or_else(|| shape(mesh))
}

/// The level `level` (1 or more, at most the mesh's levels) for sculpting it.
pub fn level(mesh: &Mesh, level: u32) -> Option<Level> {
    if level == 0 || level > levels(mesh) {
        return None;
    }
    let mut found = None;
    walk(mesh, level, |at, surface, frames, offsets| {
        if at == level {
            let mut shown = surface.clone();
            for ((point, frame), offset) in shown.positions.iter_mut().zip(frames).zip(offsets) {
                *point += frame.to_object(*offset);
            }
            found = Some(Level { mesh: shown, surface: surface.positions.clone(), frames: frames.to_vec() });
        }
        None
    });
    found
}

/// Stores `positions` (the level's vertices after sculpting) as level `at`'s offsets.
pub fn store(mesh: &mut Mesh, at: u32, level: &Level, positions: &[Vec3]) {
    let Some(offsets) = at.checked_sub(1).and_then(|index| mesh.multires.get_mut(index as usize)) else { return };
    for (((offset, point), surface), frame) in offsets.iter_mut().zip(positions).zip(&level.surface).zip(&level.frames) {
        *offset = frame.to_local(*point - *surface);
    }
}

/// Stores only the vertices `touched` of `positions` as level `at`'s offsets.
pub fn store_some(mesh: &mut Mesh, at: u32, level: &Level, positions: &[Vec3], touched: impl IntoIterator<Item = u32>) {
    let Some(offsets) = at.checked_sub(1).and_then(|index| mesh.multires.get_mut(index as usize)) else { return };
    for vertex in touched {
        let slot = vertex as usize;
        let (Some(offset), Some(point), Some(surface), Some(frame)) =
            (offsets.get_mut(slot), positions.get(slot), level.surface.get(slot), level.frames.get(slot))
        else {
            continue;
        };
        *offset = frame.to_local(*point - *surface);
    }
}

/// The base face each face of level `level` came from (each face of n corners becomes n quads,
/// and each quad four, in order).
pub fn face_origins(mesh: &Mesh, level: u32) -> Vec<u32> {
    let mut origins: Vec<u32> = (0..u32::try_from(mesh.faces.len()).unwrap_or(u32::MAX)).collect();
    for at in 0..level {
        origins = if at == 0 {
            mesh.faces.iter().zip(&origins).flat_map(|(face, origin)| std::iter::repeat_n(*origin, face.len())).collect()
        } else {
            origins.iter().flat_map(|origin| std::iter::repeat_n(*origin, 4)).collect()
        };
    }
    origins
}

/// A value per base vertex (the mask) carried to level `level`'s vertices, as the positions are
/// (subdivided, then onto the limit).
pub fn carry_vertices(mesh: &Mesh, level: u32, values: &[f32]) -> Vec<f32> {
    if values.len() != mesh.positions.len() {
        return Vec::new();
    }
    if level == 0 {
        return values.to_vec();
    }
    let mut carrier = shape(mesh);
    carrier.positions = values.iter().map(|value| Vec3::new(*value, 0.0, 0.0)).collect();
    for _ in 0..level {
        carrier = catmull_clark(&carrier);
    }
    limit_positions(&mut carrier);
    carrier.positions.iter().map(|point| point.x.clamp(0.0, 1.0)).collect()
}

/// Per-face data of the base mesh carried to level `level`'s faces.
pub fn carry_faces<T: Copy>(values: &[T], origins: &[u32]) -> Vec<T> {
    origins.iter().filter_map(|origin| values.get(*origin as usize).copied()).collect()
}

/// Adds a level above the highest, with no detail of its own; answers the new number of levels.
pub fn subdivide(mesh: &mut Mesh) -> Result<u32> {
    let current = levels(mesh);
    if current == 0 {
        mesh.multires.clear();
    }
    if mesh.faces.is_empty() {
        return Err(SceneError::params("a mesh without faces cannot be subdivided"));
    }
    if current >= MAX_LEVELS {
        return Err(SceneError::params(format!("a mesh has at most {MAX_LEVELS} levels")));
    }
    let corners: usize = mesh.faces.iter().map(Vec::len).sum();
    let faces = corners.saturating_mul(4usize.saturating_pow(current));
    if faces > MAX_FACES {
        return Err(SceneError::params("another level would make too many faces"));
    }
    let counts = vertex_counts(mesh, current + 1);
    let count = counts.last().copied().unwrap_or(0);
    mesh.multires.push(vec![Vec3::ZERO; count]);
    Ok(current + 1)
}

/// Drops the levels above `keep`.
pub fn delete_higher(mesh: &mut Mesh, keep: u32) {
    mesh.multires.truncate(keep as usize);
}

/// Each level as it shows now, from 1 to the highest.
fn shown_levels(mesh: &Mesh) -> Vec<Vec<Vec3>> {
    let top = levels(mesh);
    let mut shown = Vec::new();
    walk(mesh, top, |_, surface, frames, offsets| {
        shown.push(surface.positions.iter().zip(frames).zip(offsets).map(|((point, frame), offset)| *point + frame.to_object(*offset)).collect());
        None
    });
    shown
}

/// Each level's surface and what it shows, for [`transformed`]; `None` without levels.
pub(crate) fn shown_for_transform(mesh: &Mesh) -> Option<Vec<(Vec<Vec3>, Vec<Vec3>)>> {
    let top = levels(mesh);
    if top == 0 {
        return None;
    }
    let mut out = Vec::new();
    walk(mesh, top, |_, surface, frames, offsets| {
        let shown = surface.positions.iter().zip(frames).zip(offsets).map(|((point, frame), offset)| *point + frame.to_object(*offset)).collect();
        out.push((surface.positions.clone(), shown));
        None
    });
    Some(out)
}

/// Gives the levels of `mesh` (whose base was just moved by `transform`) the offsets that make
/// each show what it showed before (`before`, from [`shown_for_transform`]) moved by
/// `transform`. A mirror reverses the faces, which renumbers the higher levels' face points:
/// their vertices are then found by where the moved old surface has them.
pub(crate) fn transformed(mesh: &mut Mesh, before: &[(Vec<Vec3>, Vec<Vec3>)], transform: glam::Mat4) {
    let mirrored = transform.determinant() < 0.0;
    let mut rebuilt: Vec<Vec<Vec3>> = Vec::new();
    walk(mesh, levels(mesh), |at, surface, frames, stored| {
        let (old_surface, old_shown) = before.get(at as usize - 1)?;
        let from: Vec<Option<usize>> = if mirrored {
            let moved: Vec<Vec3> = old_surface.iter().map(|point| transform.transform_point3(*point)).collect();
            crate::vertex_data::sources(&moved, &surface.positions)
        } else {
            (0..surface.positions.len()).map(Some).collect()
        };
        let offsets: Vec<Vec3> = surface
            .positions
            .iter()
            .zip(frames)
            .zip(&from)
            .enumerate()
            .map(|(index, ((point, frame), source))| {
                let goal = source.and_then(|source| old_shown.get(source)).map(|goal| transform.transform_point3(*goal));
                goal.map_or_else(|| stored.get(index).copied().unwrap_or(Vec3::ZERO), |goal| frame.to_local(goal - *point))
            })
            .collect();
        rebuilt.push(offsets.clone());
        Some(offsets)
    });
    if rebuilt.len() == mesh.multires.len() {
        mesh.multires = rebuilt;
    }
}

/// Gives the levels the offsets that make them show `targets` (from level 1) over the mesh's
/// current base; a level without a target keeps showing what it does.
fn rebuild(mesh: &mut Mesh, targets: &[Vec<Vec3>]) {
    let top = levels(mesh);
    let mut rebuilt: Vec<Vec<Vec3>> = Vec::new();
    walk(mesh, top, |at, surface, frames, stored| {
        let offsets: Vec<Vec3> = match targets.get(at as usize - 1).filter(|target| target.len() == surface.positions.len()) {
            Some(target) => target.iter().zip(&surface.positions).zip(frames).map(|((goal, point), frame)| frame.to_local(*goal - *point)).collect(),
            None => stored.to_vec(),
        };
        rebuilt.push(offsets.clone());
        Some(offsets)
    });
    mesh.multires = rebuilt;
}

/// Apply Base: moves the base mesh towards the highest level's shape (its vertices' limit
/// positions onto the highest level's, which keep the base's numbering) and keeps every level
/// showing what it showed, so the detail left in the offsets is only the fine part.
pub fn apply_base(mesh: &mut Mesh) -> Result<()> {
    let top = levels(mesh);
    if top == 0 {
        return Err(SceneError::params("the mesh has no multiresolution levels"));
    }
    let shown = shown_levels(mesh);
    let Some(highest) = shown.last() else { return Ok(()) };
    let base_count = mesh.positions.len();
    let goal: Vec<Vec3> = highest.iter().take(base_count).copied().collect();
    // The limit position of a vertex does not change with subdivision: fit the base's limit
    // positions to the goal, a few relaxation steps.
    let mut base = shape(mesh);
    for _ in 0..16 {
        let mut limit = base.clone();
        limit_positions(&mut limit);
        let mut largest = 0f32;
        for ((point, at), want) in base.positions.iter_mut().zip(&limit.positions).zip(&goal) {
            let step = *want - *at;
            largest = largest.max(step.length());
            *point += step;
        }
        if largest < 1e-6 {
            break;
        }
    }
    mesh.positions = base.positions;
    rebuild(mesh, &shown);
    Ok(())
}

/// Reshape: makes the highest level show `positions` (another mesh with the same faces as the
/// highest level), keeping the lower levels.
pub fn reshape(mesh: &mut Mesh, positions: &[Vec3]) -> Result<()> {
    let top = levels(mesh);
    let expected = vertex_counts(mesh, top).last().copied().unwrap_or(0);
    if top == 0 || positions.len() != expected {
        return Err(SceneError::params(format!("reshape needs a mesh with the highest level's {expected} vertices (it has {})", positions.len())));
    }
    let mut targets: Vec<Vec<Vec3>> = vec![Vec::new(); top as usize];
    if let Some(last) = targets.last_mut() {
        *last = positions.to_vec();
    }
    rebuild(mesh, &targets);
    Ok(())
}

/// Recover one level below the base, preserving every existing level's visible positions.
/// The old base becomes level 1; disconnected components must all admit the same quad step.
/// Work is staged so unsupported topology or data never partly changes the mesh.
pub fn unsubdivide(mesh: &mut Mesh) -> Result<u32> {
    let current = levels(mesh);
    if current >= MAX_LEVELS || current as usize != mesh.multires.len() {
        return Err(SceneError::params("recovery needs valid multiresolution data and room for another level (at most 6)"));
    }
    if mesh.multires.iter().flatten().any(|offset| !offset.is_finite()) {
        return Err(SceneError::params("multiresolution offsets must be finite"));
    }
    if mesh.faces.len().saturating_mul(4usize.saturating_pow(current)) > 250_000 {
        return Err(SceneError::params("recovery supports at most 250000 faces at the highest level"));
    }
    let (mut coarse, mut map) = recovery::recover(mesh)?;
    let mut old = shape(mesh);
    let mut new = catmull_clark(&shape(&coarse));
    let mut targets = Vec::new();
    for index in 0..=current {
        // Only one source level is materialized at a time; target offsets are bounded above.
        let positions = if index == 0 { mesh.positions.clone() } else { evaluate(mesh, index).positions };
        if index > 0 {
            map = recovery::refine_map(&old, &new, &map)?;
            old = catmull_clark(&old);
            new = catmull_clark(&new);
        }
        let target = map
            .iter()
            .map(|vertex| positions.get(*vertex as usize).copied().ok_or_else(|| SceneError::params("the recovered level does not match its source")))
            .collect::<Result<Vec<_>>>()?;
        coarse.multires.push(vec![Vec3::ZERO; target.len()]);
        targets.push(target);
    }
    rebuild(&mut coarse, &targets);
    if coarse.multires.iter().flatten().any(|offset| !offset.is_finite()) {
        return Err(SceneError::params("the recovered offsets exceed the supported numeric range"));
    }
    coarse.validate()?;
    *mesh = coarse;
    Ok(current + 1)
}

/// Recover all complete quad levels from a mesh without multiresolution data. Stops at the
/// common depth of disconnected components or the six-level limit, preserving the input at
/// the highest level. An input with no recoverable level is refused without mutation.
pub fn rebuild_subdivisions(mesh: &mut Mesh) -> Result<u32> {
    if !mesh.multires.is_empty() {
        return Err(SceneError::params("Rebuild Subdivisions requires a mesh without multiresolution levels"));
    }
    let mut recovered = mesh.clone();
    unsubdivide(&mut recovered)?;
    while levels(&recovered) < MAX_LEVELS {
        if unsubdivide(&mut recovered).is_err() {
            break;
        }
    }
    let total = levels(&recovered);
    *mesh = recovered;
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::Primitive;

    fn cube() -> Mesh {
        Primitive::Cube { size: 2.0 }.build().unwrap()
    }

    #[test]
    fn levels_count_like_subdivision_and_sit_on_the_limit_surface() {
        let mut mesh = cube();
        for expected in [26, 98, 386] {
            subdivide(&mut mesh).unwrap();
            let shown = evaluate(&mesh, levels(&mesh));
            assert_eq!(shown.positions.len(), expected);
            let corner = shown.positions.iter().map(|point| point.x + point.y + point.z).fold(f32::MIN, f32::max);
            assert!((corner - 1.5).abs() < 1e-5, "{corner}");
        }
        assert_eq!(vertex_counts(&cube(), 3), vec![26, 98, 386]);
    }

    #[test]
    fn detail_at_a_level_shows_there_and_carries_up() {
        let mut mesh = cube();
        subdivide(&mut mesh).unwrap();
        subdivide(&mut mesh).unwrap();
        let mut one = level(&mesh, 1).unwrap();
        let top = one.mesh.positions.iter().position(|point| point.z > 0.5 && point.x.abs() < 1e-4 && point.y.abs() < 1e-4).unwrap();
        one.mesh.positions[top].z += 0.5;
        let positions = one.mesh.positions.clone();
        store(&mut mesh, 1, &one, &positions);
        let shown = evaluate(&mesh, 1);
        assert!((shown.positions[top] - positions[top]).length() < 1e-5);
        let high = evaluate(&mesh, 2);
        let before = evaluate(&cube_with_levels(2), 2);
        assert!(high.positions[top].z > before.positions[top].z + 0.2, "{} vs {}", high.positions[top].z, before.positions[top].z);
    }

    fn cube_with_levels(count: u32) -> Mesh {
        let mut mesh = cube();
        for _ in 0..count {
            subdivide(&mut mesh).unwrap();
        }
        mesh
    }

    #[test]
    fn apply_base_and_reshape_keep_the_shape() {
        let mut mesh = cube_with_levels(2);
        let mut two = level(&mesh, 2).unwrap();
        for point in &mut two.mesh.positions {
            *point *= 1.3;
        }
        let positions = two.mesh.positions.clone();
        store(&mut mesh, 2, &two, &positions);
        let before = evaluate(&mesh, 2);
        apply_base(&mut mesh).unwrap();
        let after = evaluate(&mesh, 2);
        for (a, b) in before.positions.iter().zip(&after.positions) {
            assert!(a.distance(*b) < 1e-3, "{a} {b}");
        }
        // The base grew to take most of the scale.
        assert!(mesh.positions[0].length() > cube().positions[0].length() * 1.2);
        let flat: Vec<Vec3> = before.positions.iter().map(|point| *point * 0.5).collect();
        reshape(&mut mesh, &flat).unwrap();
        let reshaped = evaluate(&mesh, 2);
        for (a, b) in reshaped.positions.iter().zip(&flat) {
            assert!(a.distance(*b) < 1e-4);
        }
        assert!(reshape(&mut mesh, &flat[..10]).is_err());
    }

    #[test]
    fn faces_know_their_base_face() {
        let mesh = cube_with_levels(2);
        let origins = face_origins(&mesh, 2);
        assert_eq!(origins.len(), evaluate(&mesh, 2).faces.len());
        assert_eq!(&origins[..17], &[0; 16].iter().copied().chain([1]).collect::<Vec<u32>>()[..]);
    }

    #[test]
    fn offsets_that_no_longer_fit_are_ignored() {
        let mut mesh = cube_with_levels(1);
        mesh.faces.pop();
        assert_eq!(levels(&mesh), 0);
        assert_eq!(evaluate(&mesh, 1).positions.len(), mesh.positions.len());
    }
}
