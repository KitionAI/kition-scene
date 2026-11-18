//! Dynamic topology: brushes that add detail where they work, as the reference application's
//! Dyntopo does. The mesh is made of triangles; after each dab, edges under the brush longer
//! than the detail size are split in two, and edges shorter than two fifths of it are collapsed,
//! so the surface keeps about the same density wherever the brush has been.
//!
//! Splitting and collapsing carry what the mesh keeps per vertex (the mask, vertex groups, shape
//! keys), per face (face sets, hidden flags), and per corner (texture coordinates), and the UV
//! seams along split edges.

use std::collections::{BTreeSet, HashMap};

use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::error::{Result, SceneError};
use crate::mesh::Mesh;

/// How the detail size is measured.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DetailMode {
    /// In screen pixels: closer views add finer detail.
    #[default]
    Relative,
    /// A fixed resolution in the object's own space (edges per metre).
    Constant,
    /// A share of the brush's radius.
    Brush,
}

/// What a dab does to the edges under it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Refine {
    /// Splits long edges and collapses short ones.
    #[default]
    SubdivideCollapse,
    Subdivide,
    Collapse,
}

/// Dynamic topology settings. The defaults are the reference application's.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dyntopo {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub mode: DetailMode,
    /// Relative detail: the longest edge in screen pixels (12).
    #[serde(default = "twelve")]
    pub detail_size: f32,
    /// Constant detail: edges per metre (12).
    #[serde(default = "twelve")]
    pub resolution: f32,
    /// Brush detail: the longest edge as a percentage of the brush radius (25).
    #[serde(default = "quarter")]
    pub percent: f32,
    #[serde(default)]
    pub refine: Refine,
}

fn twelve() -> f32 {
    12.0
}
fn quarter() -> f32 {
    25.0
}

impl Default for Dyntopo {
    fn default() -> Self {
        Self { enabled: false, mode: DetailMode::Relative, detail_size: 12.0, resolution: 12.0, percent: 25.0, refine: Refine::SubdivideCollapse }
    }
}

impl Dyntopo {
    /// Checks the numbers.
    pub fn check(&self) -> Result<()> {
        if !(self.detail_size.is_finite() && (0.5..=40.0).contains(&self.detail_size)) {
            return Err(SceneError::params("the detail size goes from 0.5 to 40 pixels"));
        }
        if !(self.resolution.is_finite() && (1.0..=10_000.0).contains(&self.resolution)) {
            return Err(SceneError::params("the resolution goes from 1 to 10000"));
        }
        if !(self.percent.is_finite() && (0.5..=100.0).contains(&self.percent)) {
            return Err(SceneError::params("the brush detail goes from 0.5 to 100 percent"));
        }
        Ok(())
    }

    /// The longest edge a dab leaves, in the object's own space, for a dab of `radius` (object
    /// space) drawn `radius_px` pixels wide.
    pub fn edge_length(&self, radius: f32, radius_px: f32) -> f32 {
        match self.mode {
            DetailMode::Relative => radius / radius_px.max(1.0) * self.detail_size,
            DetailMode::Constant => 1.0 / self.resolution,
            DetailMode::Brush => radius * self.percent / 100.0,
        }
    }
}

/// Cuts every face into triangles (dynamic topology works on triangles), carrying texture
/// coordinates, face sets, and hidden flags; answers whether anything changed.
pub fn triangulate(mesh: &mut Mesh) -> bool {
    if mesh.faces.iter().all(|face| face.len() == 3) {
        return false;
    }
    crate::sculpt::face_sets::tidy(mesh);
    let uvs = mesh.has_uvs() && mesh.uvs_aligned();
    let (mut faces, mut corner_uvs, mut sets, mut hidden) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (corners, face) in mesh.corner_triangles() {
        let Some(vertices) = mesh.faces.get(face as usize) else { continue };
        faces.push(corners.iter().filter_map(|corner| vertices.get(*corner as usize).copied()).collect::<Vec<u32>>());
        if uvs {
            corner_uvs.push(corners.iter().map(|corner| mesh.corner_uv(face as usize, *corner as usize)).collect());
        }
        if let Some(set) = mesh.face_sets.get(face as usize) {
            sets.push(*set);
        }
        if let Some(flag) = mesh.hidden.get(face as usize) {
            hidden.push(*flag);
        }
    }
    mesh.faces = faces;
    mesh.uvs = corner_uvs;
    mesh.face_sets = sets;
    mesh.hidden = hidden;
    true
}

/// What a refine changed.
#[derive(Debug, Clone, Default)]
pub struct Refined {
    pub split: usize,
    pub collapsed: usize,
    /// Where each old vertex went, when vertices were removed (`None` for removed ones).
    pub remap: Option<Vec<Option<u32>>>,
}

impl Refined {
    /// Whether the faces changed.
    pub fn changed(&self) -> bool {
        self.split > 0 || self.collapsed > 0
    }
}

/// The most splits one dab makes.
const MAX_SPLITS: usize = 20_000;

fn key(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}

/// Refines the triangles within `radius` of `center` towards edges no longer than `longest`
/// (hidden faces stay as they are).
pub fn refine(mesh: &mut Mesh, center: Vec3, radius: f32, longest: f32, refine: Refine) -> Refined {
    let mut refined = Refined::default();
    if !(longest.is_finite() && longest > 0.0) || mesh.faces.is_empty() {
        return refined;
    }
    crate::sculpt::face_sets::tidy(mesh);
    let mut edit = Edit::new(mesh);
    let inside = |point: Vec3| point.distance_squared(center) <= radius * radius;
    if refine != Refine::Collapse {
        refined.split = edit.split_long(center, radius, longest);
    }
    if refine != Refine::Subdivide {
        refined.collapsed = edit.collapse_short(&inside, longest * 0.4);
    }
    if refined.collapsed > 0 {
        refined.remap = Some(edit.compact());
    } else {
        edit.finish();
    }
    refined
}

/// A mesh under refinement, with what the per-vertex and per-corner data need.
struct Edit<'a> {
    mesh: &'a mut Mesh,
    uvs: bool,
    dead_faces: Vec<bool>,
    dead_vertices: BTreeSet<u32>,
    seams: BTreeSet<(u32, u32)>,
}

impl<'a> Edit<'a> {
    fn new(mesh: &'a mut Mesh) -> Self {
        let uvs = mesh.has_uvs() && mesh.uvs_aligned();
        let seams = mesh.seams.iter().map(|[a, b]| (*a, *b)).collect();
        let dead_faces = vec![false; mesh.faces.len()];
        Self { mesh, uvs, dead_faces, dead_vertices: BTreeSet::new(), seams }
    }

    fn hidden(&self, face: usize) -> bool {
        self.mesh.hidden.get(face).copied().unwrap_or(false)
    }

    fn point(&self, vertex: u32) -> Vec3 {
        self.mesh.positions.get(vertex as usize).copied().unwrap_or(Vec3::ZERO)
    }

    /// A new vertex mixing `a` and `b` half and half (position and per-vertex data).
    fn mix_vertex(&mut self, a: u32, b: u32) -> u32 {
        let mesh = &mut *self.mesh;
        let index = u32::try_from(mesh.positions.len()).unwrap_or(u32::MAX);
        let half = |values: &[f32]| (values.get(a as usize).copied().unwrap_or(0.0) + values.get(b as usize).copied().unwrap_or(0.0)) * 0.5;
        let middle =
            (mesh.positions.get(a as usize).copied().unwrap_or(Vec3::ZERO) + mesh.positions.get(b as usize).copied().unwrap_or(Vec3::ZERO)) * 0.5;
        mesh.positions.push(middle);
        if !mesh.mask.is_empty() {
            let value = half(&mesh.mask);
            mesh.mask.push(value);
        }
        for group in &mut mesh.groups {
            let value = half(&group.weights);
            group.weights.push(value);
        }
        for key in &mut mesh.shape_keys {
            let value =
                (key.offsets.get(a as usize).copied().unwrap_or(Vec3::ZERO) + key.offsets.get(b as usize).copied().unwrap_or(Vec3::ZERO)) * 0.5;
            key.offsets.push(value);
        }
        index
    }

    /// Edges of live faces, with their faces.
    fn edge_faces(&self, wanted: &dyn Fn(u32, u32) -> bool) -> HashMap<(u32, u32), Vec<usize>> {
        let mut edges: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
        for (index, face) in self.mesh.faces.iter().enumerate() {
            if self.dead_faces.get(index).copied().unwrap_or(true) {
                continue;
            }
            for (a, b) in crate::mesh::face_edges(face) {
                if wanted(a, b) {
                    edges.entry(key(a, b)).or_default().push(index);
                }
            }
        }
        edges
    }

    /// Splits the edge `a`–`b` of its (triangle) faces at its middle.
    fn split(&mut self, a: u32, b: u32, faces: &[usize]) -> u32 {
        let middle = self.mix_vertex(a, b);
        for face in faces {
            let Some(corners) = self.mesh.faces.get(*face).cloned() else { continue };
            let (Some(at_a), Some(at_b)) = (corners.iter().position(|vertex| *vertex == a), corners.iter().position(|vertex| *vertex == b)) else {
                continue;
            };
            let mut first = corners.clone();
            let mut second = corners;
            if let Some(slot) = first.get_mut(at_b) {
                *slot = middle;
            }
            if let Some(slot) = second.get_mut(at_a) {
                *slot = middle;
            }
            if let Some(slot) = self.mesh.faces.get_mut(*face) {
                *slot = first;
            }
            self.mesh.faces.push(second);
            self.dead_faces.push(false);
            if self.uvs {
                let uvs = self.mesh.uvs.get(*face).cloned().unwrap_or_default();
                let mid: Vec2 = (uvs.get(at_a).copied().unwrap_or(Vec2::ZERO) + uvs.get(at_b).copied().unwrap_or(Vec2::ZERO)) * 0.5;
                let (mut first, mut second) = (uvs.clone(), uvs);
                if let Some(slot) = first.get_mut(at_b) {
                    *slot = mid;
                }
                if let Some(slot) = second.get_mut(at_a) {
                    *slot = mid;
                }
                if let Some(slot) = self.mesh.uvs.get_mut(*face) {
                    *slot = first;
                }
                self.mesh.uvs.push(second);
            }
            if let Some(set) = self.mesh.face_sets.get(*face).copied() {
                self.mesh.face_sets.push(set);
            }
            if let Some(flag) = self.mesh.hidden.get(*face).copied() {
                self.mesh.hidden.push(flag);
            }
        }
        if self.seams.remove(&key(a, b)) {
            self.seams.insert(key(a, middle));
            self.seams.insert(key(middle, b));
        }
        middle
    }

    /// Splits the edges that pass within `radius` of `center` and are longer than `longest`,
    /// longest first, until none is left (or the dab has made as many as it may).
    fn split_long(&mut self, center: Vec3, radius: f32, longest: f32) -> usize {
        let mut splits = 0;
        for _ in 0..8 {
            let positions = &self.mesh.positions;
            let at = |vertex: u32| positions.get(vertex as usize).copied().unwrap_or(Vec3::ZERO);
            let near = |a: u32, b: u32| {
                let (start, end) = (at(a), at(b));
                let along = end - start;
                let t = if along.length_squared() > 0.0 { ((center - start).dot(along) / along.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
                (start + along * t).distance_squared(center) <= radius * radius
            };
            let edges = self.edge_faces(&near);
            let mut long: Vec<((u32, u32), f32)> = edges
                .iter()
                .filter(|(_, faces)| {
                    faces.iter().all(|face| self.mesh.faces.get(*face).is_some_and(|corners| corners.len() == 3) && !self.hidden(*face))
                })
                .map(|(edge, _)| (*edge, self.point(edge.0).distance(self.point(edge.1))))
                .filter(|(_, length)| *length > longest)
                .collect();
            if long.is_empty() {
                break;
            }
            long.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.cmp(&y.0)));
            // One split per face per pass: a face split once has new edges, found next pass.
            let mut used = BTreeSet::new();
            for ((a, b), _) in long {
                let Some(faces) = edges.get(&(a, b)) else { continue };
                if faces.iter().any(|face| used.contains(face)) {
                    continue;
                }
                used.extend(faces.iter().copied());
                let before = self.mesh.faces.len();
                self.split(a, b, faces);
                used.extend(before..self.mesh.faces.len());
                splits += 1;
                if splits >= MAX_SPLITS {
                    return splits;
                }
            }
        }
        splits
    }

    /// Collapses edges with both ends inside shorter than `shortest`, where that keeps the
    /// surface a manifold that does not fold over.
    fn collapse_short(&mut self, inside: &dyn Fn(Vec3) -> bool, shortest: f32) -> usize {
        let mut collapsed = 0;
        for _ in 0..4 {
            let near: Vec<bool> = self.mesh.positions.iter().map(|point| inside(*point)).collect();
            let near_of = |vertex: u32| near.get(vertex as usize).copied().unwrap_or(false);
            let edges = self.edge_faces(&|_, _| true);
            let mut vertex_faces: HashMap<u32, Vec<usize>> = HashMap::new();
            for (index, face) in self.mesh.faces.iter().enumerate() {
                if self.dead_faces.get(index).copied().unwrap_or(true) {
                    continue;
                }
                for vertex in face {
                    vertex_faces.entry(*vertex).or_default().push(index);
                }
            }
            let mut short: Vec<((u32, u32), f32)> = edges
                .keys()
                .filter(|(a, b)| near_of(*a) && near_of(*b))
                .map(|edge| (*edge, self.point(edge.0).distance(self.point(edge.1))))
                .filter(|(_, length)| *length < shortest)
                .collect();
            if short.is_empty() {
                break;
            }
            short.sort_by(|x, y| x.1.total_cmp(&y.1).then(x.0.cmp(&y.0)));
            let mut dirty: BTreeSet<u32> = BTreeSet::new();
            let mut pass = 0;
            for ((a, b), _) in short {
                if dirty.contains(&a) || dirty.contains(&b) {
                    continue;
                }
                let Some(ring) = self.collapsible(a, b, &edges, &vertex_faces) else { continue };
                self.collapse(a, b, &edges, &vertex_faces);
                dirty.extend(ring);
                dirty.insert(a);
                dirty.insert(b);
                pass += 1;
            }
            collapsed += pass;
            if pass == 0 {
                break;
            }
        }
        collapsed
    }

    fn neighbours(&self, vertex: u32, vertex_faces: &HashMap<u32, Vec<usize>>) -> BTreeSet<u32> {
        let mut out = BTreeSet::new();
        for face in vertex_faces.get(&vertex).map_or(&[][..], Vec::as_slice) {
            if let Some(corners) = self.mesh.faces.get(*face) {
                out.extend(corners.iter().copied().filter(|other| *other != vertex));
            }
        }
        out
    }

    /// Whether `a`–`b` may collapse (into `a`), with the vertices whose faces it changes.
    fn collapsible(&self, a: u32, b: u32, edges: &HashMap<(u32, u32), Vec<usize>>, vertex_faces: &HashMap<u32, Vec<usize>>) -> Option<Vec<u32>> {
        let faces = edges.get(&key(a, b))?;
        if faces.len() != 2 {
            return None;
        }
        let triangles = |vertex: u32| vertex_faces.get(&vertex).map_or(&[][..], Vec::as_slice);
        // Both ends interior, every face around them a live, visible triangle.
        for vertex in [a, b] {
            for face in triangles(vertex) {
                let corners = self.mesh.faces.get(*face)?;
                if corners.len() != 3 || self.hidden(*face) {
                    return None;
                }
                for (x, y) in crate::mesh::face_edges(corners) {
                    if (x == vertex || y == vertex) && edges.get(&key(x, y)).is_none_or(|list| list.len() != 2) {
                        return None;
                    }
                }
            }
        }
        // The link condition: the ends share exactly the two opposite vertices.
        let (around_a, around_b) = (self.neighbours(a, vertex_faces), self.neighbours(b, vertex_faces));
        let shared: Vec<u32> = around_a.intersection(&around_b).copied().collect();
        let opposite: BTreeSet<u32> =
            faces.iter().filter_map(|face| self.mesh.faces.get(*face)?.iter().copied().find(|vertex| *vertex != a && *vertex != b)).collect();
        if shared.len() != 2 || shared.iter().any(|vertex| !opposite.contains(vertex)) {
            return None;
        }
        // The opposite vertices keep at least three neighbours.
        if opposite.iter().any(|vertex| self.neighbours(*vertex, vertex_faces).len() <= 3) {
            return None;
        }
        // No face around either end turns over.
        let middle = (self.point(a) + self.point(b)) * 0.5;
        for vertex in [a, b] {
            for face in triangles(vertex) {
                if faces.contains(face) {
                    continue;
                }
                let corners = self.mesh.faces.get(*face)?;
                let before: Vec<Vec3> = corners.iter().map(|corner| self.point(*corner)).collect();
                let after: Vec<Vec3> = corners.iter().map(|corner| if *corner == a || *corner == b { middle } else { self.point(*corner) }).collect();
                let (old, new) = (crate::mesh::newell_vector(&before), crate::mesh::newell_vector(&after));
                if old.normalize_or_zero().dot(new.normalize_or_zero()) < 0.2 {
                    return None;
                }
            }
        }
        let mut ring: Vec<u32> = around_a.union(&around_b).copied().collect();
        ring.sort_unstable();
        Some(ring)
    }

    /// Collapses `a`–`b` into `a` at their middle.
    fn collapse(&mut self, a: u32, b: u32, edges: &HashMap<(u32, u32), Vec<usize>>, vertex_faces: &HashMap<u32, Vec<usize>>) {
        let removed = edges.get(&key(a, b)).cloned().unwrap_or_default();
        let mesh = &mut *self.mesh;
        let mix = |values: &mut Vec<f32>| {
            let other = values.get(b as usize).copied().unwrap_or(0.0);
            if let Some(slot) = values.get_mut(a as usize) {
                *slot = (*slot + other) * 0.5;
            }
        };
        let middle =
            (mesh.positions.get(a as usize).copied().unwrap_or(Vec3::ZERO) + mesh.positions.get(b as usize).copied().unwrap_or(Vec3::ZERO)) * 0.5;
        if let Some(slot) = mesh.positions.get_mut(a as usize) {
            *slot = middle;
        }
        if !mesh.mask.is_empty() {
            mix(&mut mesh.mask);
        }
        for group in &mut mesh.groups {
            mix(&mut group.weights);
        }
        for key in &mut mesh.shape_keys {
            let other = key.offsets.get(b as usize).copied().unwrap_or(Vec3::ZERO);
            if let Some(slot) = key.offsets.get_mut(a as usize) {
                *slot = (*slot + other) * 0.5;
            }
        }
        for face in &removed {
            if let Some(flag) = self.dead_faces.get_mut(*face) {
                *flag = true;
            }
        }
        for face in vertex_faces.get(&b).map_or(&[][..], Vec::as_slice) {
            if removed.contains(face) {
                continue;
            }
            if let Some(corners) = self.mesh.faces.get_mut(*face) {
                for corner in corners.iter_mut().filter(|corner| **corner == b) {
                    *corner = a;
                }
            }
        }
        self.dead_vertices.insert(b);
        let moved: Vec<(u32, u32)> = self.seams.iter().copied().filter(|(x, y)| *x == b || *y == b).collect();
        for (x, y) in moved {
            self.seams.remove(&(x, y));
            let other = if x == b { y } else { x };
            if other != a {
                self.seams.insert(key(a, other));
            }
        }
    }

    /// Drops dead faces (with their per-face data) and writes the seams back.
    fn finish(&mut self) {
        let keep: Vec<bool> = self.dead_faces.iter().map(|dead| !dead).collect();
        let retain = |list_len: usize, keep: &[bool]| keep.len() == list_len;
        if keep.iter().any(|keep| !keep) {
            let mesh = &mut *self.mesh;
            let mut index = 0;
            mesh.faces.retain(|_| {
                index += 1;
                keep.get(index - 1).copied().unwrap_or(true)
            });
            if retain(mesh.uvs.len(), &keep) {
                let mut index = 0;
                mesh.uvs.retain(|_| {
                    index += 1;
                    keep.get(index - 1).copied().unwrap_or(true)
                });
            }
            if retain(mesh.face_sets.len(), &keep) {
                let mut index = 0;
                mesh.face_sets.retain(|_| {
                    index += 1;
                    keep.get(index - 1).copied().unwrap_or(true)
                });
            }
            if retain(mesh.hidden.len(), &keep) {
                let mut index = 0;
                mesh.hidden.retain(|_| {
                    index += 1;
                    keep.get(index - 1).copied().unwrap_or(true)
                });
            }
            self.dead_faces = vec![false; self.mesh.faces.len()];
        }
        self.mesh.seams = self.seams.iter().map(|(a, b)| [*a, *b]).collect();
    }

    /// [`Edit::finish`], then drops the collapsed vertices and renumbers the rest; answers where
    /// each old vertex went.
    fn compact(&mut self) -> Vec<Option<u32>> {
        self.finish();
        let mesh = &mut *self.mesh;
        let count = mesh.positions.len();
        let mut remap: Vec<Option<u32>> = Vec::with_capacity(count);
        let mut next = 0u32;
        for vertex in 0..count {
            if self.dead_vertices.contains(&(vertex as u32)) {
                remap.push(None);
            } else {
                remap.push(Some(next));
                next += 1;
            }
        }
        let alive = |vertex: usize| remap.get(vertex).copied().flatten().is_some();
        let keep = |list: &mut Vec<f32>| {
            let mut index = 0;
            list.retain(|_| {
                index += 1;
                alive(index - 1)
            });
        };
        let mut index = 0;
        mesh.positions.retain(|_| {
            index += 1;
            alive(index - 1)
        });
        if !mesh.mask.is_empty() {
            keep(&mut mesh.mask);
        }
        for group in &mut mesh.groups {
            keep(&mut group.weights);
        }
        for key in &mut mesh.shape_keys {
            let mut index = 0;
            key.offsets.retain(|_| {
                index += 1;
                alive(index - 1)
            });
        }
        let to = |vertex: u32| remap.get(vertex as usize).copied().flatten().unwrap_or(0);
        for face in &mut mesh.faces {
            for corner in face.iter_mut() {
                *corner = to(*corner);
            }
        }
        mesh.loose_edges =
            mesh.loose_edges.iter().filter(|edge| edge.iter().all(|vertex| alive(*vertex as usize))).map(|[a, b]| [to(*a), to(*b)]).collect();
        let mut seams: Vec<[u32; 2]> = mesh
            .seams
            .iter()
            .filter(|edge| edge.iter().all(|vertex| alive(*vertex as usize)))
            .map(|[a, b]| {
                let (a, b) = (to(*a), to(*b));
                [a.min(b), a.max(b)]
            })
            .collect();
        seams.sort_unstable();
        seams.dedup();
        mesh.seams = seams;
        mesh.pinned = mesh.pinned.iter().filter(|vertex| alive(**vertex as usize)).map(|vertex| to(*vertex)).collect();
        mesh.pinned.sort_unstable();
        mesh.pinned.dedup();
        remap
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::Primitive;

    fn plane() -> Mesh {
        let mut mesh = Primitive::Grid { x_subdivisions: 4, y_subdivisions: 4, size: 2.0 }.build().unwrap();
        triangulate(&mut mesh);
        mesh
    }

    #[test]
    fn splits_long_edges_under_the_brush_only() {
        let mut mesh = plane();
        let before = mesh.faces.len();
        mesh.mask = vec![0.5; mesh.positions.len()];
        let refined = refine(&mut mesh, Vec3::ZERO, 0.6, 0.1, Refine::Subdivide);
        assert!(refined.split > 50, "{}", refined.split);
        assert!(mesh.faces.len() > before);
        mesh.validate().unwrap();
        assert_eq!(mesh.mask.len(), mesh.positions.len());
        assert!(mesh.mask.iter().all(|value| (*value - 0.5).abs() < 1e-6));
        // Edges near the centre are short now, far ones are as they were.
        let far = mesh.edges().into_iter().filter(|[a, b]| mesh.positions[*a as usize].length() > 1.2 && mesh.positions[*b as usize].length() > 1.2);
        assert!(far.into_iter().all(|[a, b]| mesh.positions[a as usize].distance(mesh.positions[b as usize]) > 0.4));
        assert!(crate::modifiers::boundary_edges(&mesh).len() >= 16);
    }

    #[test]
    fn collapses_short_edges_and_keeps_a_manifold() {
        let mut mesh = plane();
        refine(&mut mesh, Vec3::ZERO, 0.8, 0.05, Refine::Subdivide);
        let dense = mesh.positions.len();
        let refined = refine(&mut mesh, Vec3::ZERO, 0.8, 0.5, Refine::Collapse);
        assert!(refined.collapsed > 20, "{}", refined.collapsed);
        assert!(mesh.positions.len() < dense);
        mesh.validate().unwrap();
        let used: BTreeSet<u32> = mesh.faces.iter().flatten().copied().collect();
        assert_eq!(used.len(), mesh.positions.len(), "no vertex is left without faces");
        let remap = refined.remap.unwrap();
        assert_eq!(remap.len(), dense);
        // Every edge has one or two faces.
        let mut counts: HashMap<(u32, u32), usize> = HashMap::new();
        for face in &mesh.faces {
            for (a, b) in crate::mesh::face_edges(face) {
                *counts.entry(key(a, b)).or_default() += 1;
            }
        }
        assert!(counts.values().all(|count| *count <= 2));
    }

    #[test]
    fn detail_sizes() {
        let relative = Dyntopo::default();
        assert!((relative.edge_length(1.0, 50.0) - 0.24).abs() < 1e-6);
        assert!((Dyntopo { mode: DetailMode::Constant, ..relative }.edge_length(1.0, 50.0) - 1.0 / 12.0).abs() < 1e-6);
        assert!((Dyntopo { mode: DetailMode::Brush, ..relative }.edge_length(2.0, 50.0) - 0.5).abs() < 1e-6);
        assert!(Dyntopo { detail_size: 0.0, ..relative }.check().is_err());
    }
}
