//! Polygon meshes: positions, n-gon faces (counter-clockwise seen from outside), and loose
//! edges, the way a modeller keeps them. Triangles exist only for drawing and export.

use std::collections::{BTreeSet, HashMap};

use glam::{Mat4, Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::error::{Result, SceneError};

/// A polygon mesh in its object's local space (Z up).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mesh {
    /// Vertex positions.
    pub positions: Vec<Vec3>,
    /// Faces as vertex indices, counter-clockwise seen from the side their normal points to.
    pub faces: Vec<Vec<u32>>,
    /// Edges that belong to no face (a circle without fill, a wire).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub loose_edges: Vec<[u32; 2]>,
    /// Smooth shading (vertex normals) instead of flat faces.
    #[serde(default)]
    pub smooth: bool,
    /// Smooth by angle: with smooth shading, edges whose faces meet at more than this angle
    /// (radians) stay sharp (the reference's Shade Smooth by Angle); `None` smooths every edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smooth_angle: Option<f32>,
    /// Texture coordinates per face corner, parallel to `faces` (each list as long as its face);
    /// empty when the mesh has none. U goes right and V up, as in the reference application.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uvs: Vec<Vec<Vec2>>,
    /// Edges marked as UV seams, each as (low, high) vertex indices: unwrapping cuts the surface
    /// open along them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub seams: Vec<[u32; 2]>,
    /// Vertices pinned in UV space: unwrapping keeps their texture coordinates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pinned: Vec<u32>,
    /// Vertex groups: named weights per vertex (the Armature modifier deforms by them).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<crate::vertex_data::VertexGroup>,
    /// Shape keys: named offsets per vertex from these positions, mixed in by their values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shape_keys: Vec<crate::vertex_data::ShapeKey>,
    /// Sculpt mask per vertex, 0 (free) to 1 (protected from sculpt brushes); empty when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mask: Vec<f32>,
    /// Sculpt face sets: an id per face, 0 for none; empty when no face has one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub face_sets: Vec<u32>,
    /// Faces hidden in sculpt mode, one flag per face; empty when none is hidden.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<bool>,
    /// Multiresolution detail: for each subdivision level from 1, an offset per vertex of that
    /// level in the surface's own frame there ([`crate::multires`]); empty without levels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub multires: Vec<Vec<Vec3>>,
}

/// An axis-aligned box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl Bounds {
    /// The empty box, which any point extends.
    pub const EMPTY: Self = Self { min: Vec3::splat(f32::INFINITY), max: Vec3::splat(f32::NEG_INFINITY) };

    /// Whether no point was added.
    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x
    }

    /// The box grown to hold `point`.
    pub fn with(self, point: Vec3) -> Self {
        Self { min: self.min.min(point), max: self.max.max(point) }
    }

    /// The union of two boxes.
    pub fn union(self, other: Self) -> Self {
        if other.is_empty() { self } else { self.with(other.min).with(other.max) }
    }

    /// The centre.
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// The size along each axis.
    pub fn size(&self) -> Vec3 {
        if self.is_empty() { Vec3::ZERO } else { self.max - self.min }
    }
}

impl Mesh {
    /// The position of vertex `index`.
    pub fn position(&self, index: u32) -> Result<Vec3> {
        self.positions.get(index as usize).copied().ok_or_else(|| SceneError::not_found(format!("vertex {index}")))
    }

    /// The number of vertices, as the `u32` faces use.
    pub fn vertex_count(&self) -> u32 {
        u32::try_from(self.positions.len()).unwrap_or(u32::MAX)
    }

    /// The face's vertex positions; indices outside the mesh are skipped.
    pub fn face_positions(&self, face: &[u32]) -> Vec<Vec3> {
        face.iter().filter_map(|index| self.positions.get(*index as usize).copied()).collect()
    }

    /// The face's normal by Newell's method (unit length, or zero for a degenerate face).
    pub fn face_normal(&self, face: &[u32]) -> Vec3 {
        newell_normal(&self.face_positions(face))
    }

    /// The face's centre (the mean of its corners).
    pub fn face_center(&self, face: &[u32]) -> Vec3 {
        let points = self.face_positions(face);
        if points.is_empty() { Vec3::ZERO } else { points.iter().copied().sum::<Vec3>() / points.len() as f32 }
    }

    /// Every edge once, as `(low, high)` vertex pairs, in a stable order: face edges by first
    /// appearance, then loose edges.
    pub fn edges(&self) -> Vec<[u32; 2]> {
        let mut seen = BTreeSet::new();
        let mut edges = Vec::new();
        let mut push = |a: u32, b: u32| {
            let key = if a < b { [a, b] } else { [b, a] };
            if a != b && seen.insert(key) {
                edges.push(key);
            }
        };
        for face in &self.faces {
            for (a, b) in face_edges(face) {
                push(a, b);
            }
        }
        for [a, b] in &self.loose_edges {
            push(*a, *b);
        }
        edges
    }

    /// The bounding box of the positions, after `transform`.
    pub fn bounds(&self, transform: Mat4) -> Bounds {
        self.positions.iter().fold(Bounds::EMPTY, |bounds, point| bounds.with(transform.transform_point3(*point)))
    }

    /// Per-vertex normals: the area-weighted mean of the normals of the faces around a vertex.
    pub fn vertex_normals(&self) -> Vec<Vec3> {
        let mut normals = vec![Vec3::ZERO; self.positions.len()];
        for face in &self.faces {
            let points = self.face_positions(face);
            let weighted = newell_vector(&points);
            for index in face {
                if let Some(normal) = normals.get_mut(*index as usize) {
                    *normal += weighted;
                }
            }
        }
        normals.into_iter().map(|normal| normal.normalize_or_zero()).collect()
    }

    /// Normals per face corner, parallel to `faces`: each face's own when flat; when smooth, the
    /// vertex's ([`Mesh::vertex_normals`]); and when smooth by angle, the mean over the faces
    /// around the vertex that can be reached from the corner's face without crossing a sharp
    /// edge (one whose faces meet at more than the angle, or with other than two faces).
    pub fn corner_normals(&self) -> Vec<Vec<Vec3>> {
        if !self.smooth {
            return self.faces.iter().map(|face| vec![self.face_normal(face); face.len()]).collect();
        }
        let Some(angle) = self.smooth_angle.filter(|angle| angle.is_finite()) else {
            let normals = self.vertex_normals();
            return self
                .faces
                .iter()
                .map(|face| {
                    let flat = self.face_normal(face);
                    face.iter().map(|vertex| normals.get(*vertex as usize).copied().unwrap_or(flat)).collect()
                })
                .collect();
        };
        let weighted: Vec<Vec3> = self.faces.iter().map(|face| newell_vector(&self.face_positions(face))).collect();
        let unit: Vec<Vec3> = weighted.iter().map(|normal| normal.normalize_or_zero()).collect();
        // The faces on each edge, and the faces (with the corner) around each vertex.
        let mut edge_faces: HashMap<[u32; 2], Vec<usize>> = HashMap::new();
        let mut vertex_faces: Vec<Vec<usize>> = vec![Vec::new(); self.positions.len()];
        for (index, face) in self.faces.iter().enumerate() {
            for (a, b) in face_edges(face) {
                let key = if a < b { [a, b] } else { [b, a] };
                let list = edge_faces.entry(key).or_default();
                if !list.contains(&index) {
                    list.push(index);
                }
            }
            for vertex in face {
                if let Some(list) = vertex_faces.get_mut(*vertex as usize)
                    && !list.contains(&index)
                {
                    list.push(index);
                }
            }
        }
        let limit = angle.clamp(0.0, std::f32::consts::PI).cos();
        let smooth_edge = |a: u32, b: u32| -> Option<(usize, usize)> {
            let key = if a < b { [a, b] } else { [b, a] };
            match edge_faces.get(&key).map(Vec::as_slice) {
                Some([first, second]) => {
                    let (n1, n2) = (unit.get(*first).copied().unwrap_or(Vec3::ZERO), unit.get(*second).copied().unwrap_or(Vec3::ZERO));
                    (n1.dot(n2) >= limit - 1e-6).then_some((*first, *second))
                }
                _ => None,
            }
        };
        // Per vertex, the fan each face around it belongs to: faces joined across smooth edges
        // at the vertex share one.
        let mut fans: Vec<Vec<(usize, Vec3)>> = vec![Vec::new(); self.positions.len()];
        for (vertex, around) in vertex_faces.iter().enumerate() {
            let vertex = u32::try_from(vertex).unwrap_or(u32::MAX);
            let mut label: Vec<usize> = (0..around.len()).collect();
            let find = |label: &[usize], mut at: usize| {
                while let Some(next) = label.get(at).copied().filter(|next| *next != at) {
                    at = next;
                }
                at
            };
            for (slot, face) in around.iter().enumerate() {
                let Some(corners) = self.faces.get(*face) else { continue };
                let Some(position) = corners.iter().position(|corner| *corner == vertex) else { continue };
                let count = corners.len();
                let neighbours = [corners.get((position + count - 1) % count), corners.get((position + 1) % count)];
                for neighbour in neighbours.into_iter().flatten() {
                    let Some((first, second)) = smooth_edge(vertex, *neighbour) else { continue };
                    let other = if first == *face { second } else { first };
                    let Some(other_slot) = around.iter().position(|candidate| *candidate == other) else { continue };
                    let (root, other_root) = (find(&label, slot), find(&label, other_slot));
                    if let Some(entry) = label.get_mut(root.max(other_root)) {
                        *entry = root.min(other_root);
                    }
                }
            }
            let mut sums: Vec<Vec3> = vec![Vec3::ZERO; around.len()];
            for (slot, face) in around.iter().enumerate() {
                if let Some(sum) = sums.get_mut(find(&label, slot)) {
                    *sum += weighted.get(*face).copied().unwrap_or(Vec3::ZERO);
                }
            }
            if let Some(list) = fans.get_mut(vertex as usize) {
                *list = around
                    .iter()
                    .enumerate()
                    .map(|(slot, face)| (*face, sums.get(find(&label, slot)).copied().unwrap_or(Vec3::ZERO).normalize_or_zero()))
                    .collect();
            }
        }
        self.faces
            .iter()
            .enumerate()
            .map(|(index, face)| {
                let flat = unit.get(index).copied().unwrap_or(Vec3::ZERO);
                face.iter()
                    .map(|vertex| {
                        let normal = fans
                            .get(*vertex as usize)
                            .and_then(|fan| fan.iter().find(|(owner, _)| *owner == index))
                            .map_or(flat, |(_, normal)| *normal);
                        if normal == Vec3::ZERO { flat } else { normal }
                    })
                    .collect()
            })
            .collect()
    }

    /// The faces cut into triangles (a fan for convex faces, ear clipping for concave ones),
    /// each with the index of the face it came from.
    pub fn triangles(&self) -> Vec<([u32; 3], u32)> {
        self.corner_triangles()
            .into_iter()
            .filter_map(|(corners, face)| {
                let corners_of = self.faces.get(face as usize)?;
                let vertex = |corner: u32| corners_of.get(corner as usize).copied();
                Some(([vertex(corners[0])?, vertex(corners[1])?, vertex(corners[2])?], face))
            })
            .collect()
    }

    /// The faces cut into triangles as corner positions within their face (so per-corner data
    /// such as texture coordinates can follow), each with the index of the face.
    pub fn corner_triangles(&self) -> Vec<([u32; 3], u32)> {
        let mut out = Vec::new();
        for (face_index, face) in self.faces.iter().enumerate() {
            let face_index = u32::try_from(face_index).unwrap_or(u32::MAX);
            let slots: Vec<u32> = (0..u32::try_from(face.len()).unwrap_or(0)).collect();
            for triangle in triangulate(&self.face_positions(face), &slots) {
                out.push((triangle, face_index));
            }
        }
        out
    }

    /// Whether the mesh carries texture coordinates.
    pub fn has_uvs(&self) -> bool {
        !self.uvs.is_empty()
    }

    /// The texture coordinate of corner `corner` of face `face` (zero without coordinates).
    pub fn corner_uv(&self, face: usize, corner: usize) -> Vec2 {
        self.uvs.get(face).and_then(|corners| corners.get(corner)).copied().unwrap_or(Vec2::ZERO)
    }

    /// Whether the texture coordinates line up with the faces (or there are none).
    pub fn uvs_aligned(&self) -> bool {
        self.uvs.is_empty() || (self.uvs.len() == self.faces.len() && self.uvs.iter().zip(&self.faces).all(|(uvs, face)| uvs.len() == face.len()))
    }

    /// Gives every face corner a zero texture coordinate where it has none, so faces built
    /// without coordinates can join a mesh that has them.
    pub fn fill_uvs(&mut self) {
        self.uvs.resize_with(self.faces.len(), Vec::new);
        self.uvs.truncate(self.faces.len());
        for (uvs, face) in self.uvs.iter_mut().zip(&self.faces) {
            uvs.resize(face.len(), Vec2::ZERO);
        }
    }

    /// Checks every index, and that faces have at least three distinct corners.
    pub fn validate(&self) -> Result<()> {
        let count = self.positions.len();
        for (number, face) in self.faces.iter().enumerate() {
            if face.len() < 3 {
                return Err(SceneError::InvalidFile(format!("face {number} has {} corners", face.len())));
            }
            if face.iter().any(|index| *index as usize >= count) {
                return Err(SceneError::InvalidFile(format!("face {number} names a vertex outside the mesh")));
            }
            let distinct: BTreeSet<_> = face.iter().collect();
            if distinct.len() != face.len() {
                return Err(SceneError::InvalidFile(format!("face {number} repeats a vertex")));
            }
        }
        if !self.uvs_aligned() {
            return Err(SceneError::InvalidFile("the texture coordinates do not match the faces".into()));
        }
        if self.uvs.iter().flatten().any(|uv| !uv.is_finite()) {
            return Err(SceneError::InvalidFile("a texture coordinate is not a finite number".into()));
        }
        for (number, edge) in self.loose_edges.iter().enumerate() {
            if edge.iter().any(|index| *index as usize >= count) || edge[0] == edge[1] {
                return Err(SceneError::InvalidFile(format!("loose edge {number} is not an edge of the mesh")));
            }
        }
        if self.positions.iter().any(|point| !point.is_finite()) {
            return Err(SceneError::InvalidFile("a vertex position is not a finite number".into()));
        }
        if self.smooth_angle.is_some_and(|angle| !(angle.is_finite() && (0.0..=std::f32::consts::PI).contains(&angle))) {
            return Err(SceneError::InvalidFile("the smoothing angle is not between 0° and 180°".into()));
        }
        if self.seams.iter().any(|[a, b]| a >= b || *b as usize >= count) {
            return Err(SceneError::InvalidFile("a seam is not an edge of the mesh".into()));
        }
        if self.pinned.iter().any(|vertex| *vertex as usize >= count) {
            return Err(SceneError::InvalidFile("a pinned vertex is outside the mesh".into()));
        }
        crate::vertex_data::check(self)
    }

    /// Applies `transform` to every position (and keeps faces outward when it mirrors). Shape
    /// key offsets turn and scale with it, and multiresolution levels keep showing their detail
    /// where `transform` puts it.
    pub fn transform(&mut self, transform: Mat4) {
        let levels = crate::multires::shown_for_transform(self);
        for point in &mut self.positions {
            *point = transform.transform_point3(*point);
        }
        for offset in self.shape_keys.iter_mut().flat_map(|key| key.offsets.iter_mut()) {
            *offset = transform.transform_vector3(*offset);
        }
        if transform.determinant() < 0.0 {
            self.flip_all();
        }
        if let Some(levels) = levels {
            crate::multires::transformed(self, &levels, transform);
        }
    }

    /// Reverses every face's winding.
    pub fn flip_all(&mut self) {
        for face in &mut self.faces {
            face.reverse();
        }
        for uvs in &mut self.uvs {
            uvs.reverse();
        }
    }

    /// Adds `other`'s geometry, returning the offset its vertex indices got. Every list kept per
    /// vertex or per face goes along: vertex groups and shape keys join by name (a vertex
    /// takes weight 0 and offset 0 in a group or key its side lacks, and a key only `other` has
    /// keeps its value), the sculpt mask is 0, face sets 0 and hidden flags off where a side has
    /// none. Multiresolution levels no longer fit the joined faces (they are dropped, as after
    /// any change of topology).
    pub fn append(&mut self, other: &Mesh) -> u32 {
        let (count, extra) = (self.positions.len(), other.positions.len());
        let (faces, extra_faces) = (self.faces.len(), other.faces.len());
        self.append_vertex_data(other, count, extra);
        self.append_face_data(other, faces, extra_faces);
        self.append_geometry(other)
    }

    /// A copy of the vertices `vertices` and the faces `faces` (whose corners are among them),
    /// renumbered in that order, with everything kept per vertex and per face: texture
    /// coordinates, seams and pins between them, vertex group weights, shape key offsets, the
    /// mask, face sets and hidden flags (what Duplicate copies).
    pub fn part(&self, vertices: &BTreeSet<u32>, faces: &[u32]) -> Result<Mesh> {
        let map: HashMap<u32, u32> = vertices.iter().enumerate().map(|(index, vertex)| (*vertex, u32::try_from(index).unwrap_or(u32::MAX))).collect();
        let order: Vec<usize> = vertices.iter().map(|vertex| *vertex as usize).collect();
        let positions = vertices.iter().map(|vertex| self.position(*vertex)).collect::<Result<Vec<_>>>()?;
        let mut part = Mesh { positions, smooth: self.smooth, smooth_angle: self.smooth_angle, ..Mesh::default() };
        let face_sets = self.face_sets.len() == self.faces.len() && !self.face_sets.is_empty();
        let hidden = self.hidden.len() == self.faces.len() && !self.hidden.is_empty();
        let uvs = self.has_uvs() && self.uvs_aligned();
        for face in faces {
            let Some(corners) = self.faces.get(*face as usize) else { continue };
            let mapped: Vec<u32> = corners.iter().filter_map(|corner| map.get(corner).copied()).collect();
            if mapped.len() != corners.len() {
                continue;
            }
            part.faces.push(mapped);
            if uvs {
                part.uvs.push(self.uvs.get(*face as usize).cloned().unwrap_or_default());
            }
            if face_sets {
                part.face_sets.push(self.face_sets.get(*face as usize).copied().unwrap_or(0));
            }
            if hidden {
                part.hidden.push(self.hidden.get(*face as usize).copied().unwrap_or(false));
            }
        }
        let both = |[a, b]: [u32; 2]| Some([*map.get(&a)?, *map.get(&b)?]);
        part.seams = self.seams.iter().filter_map(|edge| both(*edge)).map(|[a, b]| [a.min(b), a.max(b)]).collect();
        part.pinned = self.pinned.iter().filter_map(|vertex| map.get(vertex).copied()).collect();
        if crate::vertex_data::aligned(self) {
            let pick = |values: &[f32]| order.iter().map(|vertex| values.get(*vertex).copied().unwrap_or(0.0)).collect::<Vec<f32>>();
            part.groups =
                self.groups.iter().map(|group| crate::vertex_data::VertexGroup { name: group.name.clone(), weights: pick(&group.weights) }).collect();
            part.shape_keys = self
                .shape_keys
                .iter()
                .map(|key| crate::vertex_data::ShapeKey {
                    name: key.name.clone(),
                    offsets: order.iter().map(|vertex| key.offsets.get(*vertex).copied().unwrap_or(Vec3::ZERO)).collect(),
                    value: key.value,
                })
                .collect();
            if !self.mask.is_empty() {
                part.mask = pick(&self.mask);
            }
        }
        Ok(part)
    }

    /// Joins `other`'s vertex groups, shape keys and mask onto this mesh's (before the positions).
    fn append_vertex_data(&mut self, other: &Mesh, count: usize, extra: usize) {
        let aligned = |mesh: &Mesh, length: usize| crate::vertex_data::aligned(mesh) && mesh.positions.len() == length;
        let (mine, theirs) = (aligned(self, count), aligned(other, extra));
        if !mine {
            self.groups.clear();
            self.shape_keys.clear();
            self.mask.clear();
        }
        for group in &mut self.groups {
            let source = other.groups.iter().find(|candidate| candidate.name == group.name && theirs);
            group.weights.extend((0..extra).map(|vertex| source.and_then(|source| source.weights.get(vertex)).copied().unwrap_or(0.0)));
        }
        for group in other.groups.iter().filter(|_| theirs) {
            if !self.groups.iter().any(|mine| mine.name == group.name) {
                let mut weights = vec![0.0; count];
                weights.extend((0..extra).map(|vertex| group.weights.get(vertex).copied().unwrap_or(0.0)));
                self.groups.push(crate::vertex_data::VertexGroup { name: group.name.clone(), weights });
            }
        }
        for key in &mut self.shape_keys {
            let source = other.shape_keys.iter().find(|candidate| candidate.name == key.name && theirs);
            key.offsets.extend((0..extra).map(|vertex| source.and_then(|source| source.offsets.get(vertex)).copied().unwrap_or(Vec3::ZERO)));
        }
        for key in other.shape_keys.iter().filter(|_| theirs) {
            if !self.shape_keys.iter().any(|mine| mine.name == key.name) {
                let mut offsets = vec![Vec3::ZERO; count];
                offsets.extend((0..extra).map(|vertex| key.offsets.get(vertex).copied().unwrap_or(Vec3::ZERO)));
                self.shape_keys.push(crate::vertex_data::ShapeKey { name: key.name.clone(), offsets, value: key.value });
            }
        }
        let their_mask = theirs && other.mask.len() == extra && !other.mask.is_empty();
        if !self.mask.is_empty() || their_mask {
            self.mask.resize(count, 0.0);
            self.mask.extend((0..extra).map(|vertex| if their_mask { other.mask.get(vertex).copied().unwrap_or(0.0) } else { 0.0 }));
        }
    }

    /// Joins `other`'s face sets and hidden flags onto this mesh's (before the faces).
    fn append_face_data(&mut self, other: &Mesh, faces: usize, extra: usize) {
        let mine = self.face_sets.len() == faces && !self.face_sets.is_empty();
        let theirs = other.face_sets.len() == extra && !other.face_sets.is_empty();
        if !mine {
            self.face_sets.clear();
        }
        if mine || theirs {
            self.face_sets.resize(faces, 0);
            self.face_sets.extend((0..extra).map(|face| if theirs { other.face_sets.get(face).copied().unwrap_or(0) } else { 0 }));
        }
        let mine = self.hidden.len() == faces && !self.hidden.is_empty();
        let theirs = other.hidden.len() == extra && !other.hidden.is_empty();
        if !mine {
            self.hidden.clear();
        }
        if mine || theirs {
            self.hidden.resize(faces, false);
            self.hidden.extend((0..extra).map(|face| theirs && other.hidden.get(face).copied().unwrap_or(false)));
        }
    }

    /// Adds `other`'s positions, faces, texture coordinates, loose edges, seams and pins.
    fn append_geometry(&mut self, other: &Mesh) -> u32 {
        let offset = self.vertex_count();
        // Texture coordinates survive when either side has them; the other side's corners get zero.
        let uvs = self.has_uvs() || other.has_uvs();
        if uvs {
            self.fill_uvs();
        }
        self.positions.extend_from_slice(&other.positions);
        self.faces.extend(other.faces.iter().map(|face| face.iter().map(|index| index + offset).collect()));
        if uvs {
            if other.has_uvs() && other.uvs_aligned() {
                self.uvs.extend(other.uvs.iter().cloned());
            }
            self.fill_uvs();
        }
        self.loose_edges.extend(other.loose_edges.iter().map(|[a, b]| [a + offset, b + offset]));
        self.seams.extend(other.seams.iter().map(|[a, b]| [a + offset, b + offset]));
        self.pinned.extend(other.pinned.iter().map(|vertex| vertex + offset));
        offset
    }

    /// Merges vertices closer than `distance`, dropping faces that collapse. Returns how many
    /// vertices were removed.
    pub fn weld(&mut self, distance: f32) -> usize {
        let cell = distance.max(1e-6);
        let mut grid: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::new();
        let mut remap: Vec<u32> = Vec::with_capacity(self.positions.len());
        let mut kept: Vec<Vec3> = Vec::new();
        for point in &self.positions {
            let key = ((point.x / cell).floor() as i64, (point.y / cell).floor() as i64, (point.z / cell).floor() as i64);
            let mut found = None;
            'search: for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(candidates) = grid.get(&(key.0 + dx, key.1 + dy, key.2 + dz)) {
                            for candidate in candidates {
                                if kept.get(*candidate as usize).is_some_and(|other| other.distance(*point) <= distance) {
                                    found = Some(*candidate);
                                    break 'search;
                                }
                            }
                        }
                    }
                }
            }
            let index = match found {
                Some(index) => index,
                None => {
                    let index = u32::try_from(kept.len()).unwrap_or(u32::MAX);
                    kept.push(*point);
                    grid.entry(key).or_default().push(index);
                    index
                }
            };
            remap.push(index);
        }
        let removed = self.positions.len() - kept.len();
        self.positions = kept;
        self.reindex(&remap);
        removed
    }

    /// Renames every vertex index through `remap` (old index → new index), dropping repeated
    /// consecutive corners and faces left with fewer than three.
    pub fn reindex(&mut self, remap: &[u32]) {
        let map = |index: &u32| remap.get(*index as usize).copied();
        let carry = self.has_uvs() && self.uvs_aligned();
        let old_uvs = std::mem::take(&mut self.uvs);
        let mut new_uvs = Vec::new();
        let mut faces = Vec::with_capacity(self.faces.len());
        for (number, face) in std::mem::take(&mut self.faces).into_iter().enumerate() {
            let uvs = old_uvs.get(number).map_or(&[][..], Vec::as_slice);
            let mut out: Vec<u32> = Vec::with_capacity(face.len());
            let mut out_uvs: Vec<Vec2> = Vec::with_capacity(face.len());
            let mut whole = true;
            for (corner, index) in face.iter().enumerate() {
                let Some(mapped) = map(index) else {
                    whole = false;
                    break;
                };
                if out.last() != Some(&mapped) {
                    out.push(mapped);
                    out_uvs.push(uvs.get(corner).copied().unwrap_or(Vec2::ZERO));
                }
            }
            if !whole {
                continue;
            }
            while out.len() > 1 && out.first() == out.last() {
                out.pop();
                out_uvs.pop();
            }
            let distinct: BTreeSet<_> = out.iter().collect();
            if out.len() >= 3 && distinct.len() == out.len() {
                faces.push(out);
                new_uvs.push(out_uvs);
            }
        }
        self.faces = faces;
        if carry {
            self.uvs = new_uvs;
        }
        self.loose_edges = std::mem::take(&mut self.loose_edges)
            .into_iter()
            .filter_map(|[a, b]| {
                let (a, b) = (map(&a)?, map(&b)?);
                (a != b).then_some([a, b])
            })
            .collect();
        // Seams and pins follow their vertices; ones whose vertices went away go too.
        let count = u32::try_from(self.positions.len()).unwrap_or(u32::MAX);
        let mut seams: Vec<[u32; 2]> = std::mem::take(&mut self.seams)
            .into_iter()
            .filter_map(|[a, b]| {
                let (a, b) = (map(&a)?, map(&b)?);
                (a != b && a < count && b < count).then_some([a.min(b), a.max(b)])
            })
            .collect();
        seams.sort_unstable();
        seams.dedup();
        self.seams = seams;
        let mut pinned: Vec<u32> =
            std::mem::take(&mut self.pinned).into_iter().filter_map(|vertex| map(&vertex).filter(|vertex| *vertex < count)).collect();
        pinned.sort_unstable();
        pinned.dedup();
        self.pinned = pinned;
    }

    /// Drops vertices no face or edge uses, renumbering the rest.
    pub fn remove_unused_vertices(&mut self) {
        let mut used = vec![false; self.positions.len()];
        for index in self.faces.iter().flatten().chain(self.loose_edges.iter().flatten()) {
            if let Some(flag) = used.get_mut(*index as usize) {
                *flag = true;
            }
        }
        let mut remap = vec![u32::MAX; self.positions.len()];
        let mut kept = Vec::new();
        for (index, point) in self.positions.iter().enumerate() {
            if used.get(index).copied().unwrap_or(false) {
                if let Some(slot) = remap.get_mut(index) {
                    *slot = u32::try_from(kept.len()).unwrap_or(u32::MAX);
                }
                kept.push(*point);
            }
        }
        self.positions = kept;
        self.reindex(&remap);
    }
}

/// The face's edges in order, closing back to the first corner.
pub fn face_edges(face: &[u32]) -> impl Iterator<Item = (u32, u32)> + '_ {
    face.iter().zip(face.iter().cycle().skip(1)).map(|(a, b)| (*a, *b))
}

/// Newell's (unnormalised) polygon normal; its length is twice the polygon's area.
pub fn newell_vector(points: &[Vec3]) -> Vec3 {
    let mut normal = Vec3::ZERO;
    for (a, b) in points.iter().zip(points.iter().cycle().skip(1)) {
        normal.x += (a.y - b.y) * (a.z + b.z);
        normal.y += (a.z - b.z) * (a.x + b.x);
        normal.z += (a.x - b.x) * (a.y + b.y);
    }
    normal
}

/// Newell's polygon normal, unit length (zero when the polygon has no area).
pub fn newell_normal(points: &[Vec3]) -> Vec3 {
    newell_vector(points).normalize_or_zero()
}

/// Cuts one face into triangles of its vertex indices.
fn triangulate(points: &[Vec3], indices: &[u32]) -> Vec<[u32; 3]> {
    let count = points.len().min(indices.len());
    if count < 3 {
        return Vec::new();
    }
    let at = |slot: usize| indices.get(slot).copied().unwrap_or(0);
    if count == 3 {
        return vec![[at(0), at(1), at(2)]];
    }
    let normal = newell_vector(points);
    if is_convex(points, normal) {
        return (1..count - 1).map(|slot| [at(0), at(slot), at(slot + 1)]).collect();
    }
    ear_clip(points, normal).into_iter().map(|[a, b, c]| [at(a), at(b), at(c)]).collect()
}

fn is_convex(points: &[Vec3], normal: Vec3) -> bool {
    let count = points.len();
    (0..count).all(|slot| {
        let (Some(a), Some(b), Some(c)) = (points.get(slot), points.get((slot + 1) % count), points.get((slot + 2) % count)) else {
            return false;
        };
        (*b - *a).cross(*c - *b).dot(normal) >= -1e-9
    })
}

/// Ear clipping in the face's plane; falls back to a fan for faces it cannot cut.
fn ear_clip(points: &[Vec3], normal: Vec3) -> Vec<[usize; 3]> {
    let axis = normal.normalize_or_zero();
    let (u, v) = axis.any_orthonormal_pair();
    let flat: Vec<(f32, f32)> = points.iter().map(|point| (point.dot(u), point.dot(v))).collect();
    let point = |slot: usize| flat.get(slot).copied().unwrap_or((0.0, 0.0));
    let cross = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    let mut remaining: Vec<usize> = (0..points.len()).collect();
    let mut out = Vec::new();
    let mut guard = 0;
    while remaining.len() > 3 && guard < points.len() * points.len() {
        guard += 1;
        let count = remaining.len();
        let mut clipped = false;
        for slot in 0..count {
            let (Some(&ia), Some(&ib), Some(&ic)) =
                (remaining.get((slot + count - 1) % count), remaining.get(slot), remaining.get((slot + 1) % count))
            else {
                continue;
            };
            let (a, b, c) = (point(ia), point(ib), point(ic));
            if cross(a, b, c) <= 0.0 {
                continue;
            }
            let inside = remaining.iter().any(|&other| {
                if other == ia || other == ib || other == ic {
                    return false;
                }
                let p = point(other);
                cross(a, b, p) >= 0.0 && cross(b, c, p) >= 0.0 && cross(c, a, p) >= 0.0
            });
            if inside {
                continue;
            }
            out.push([ia, ib, ic]);
            remaining.remove(slot);
            clipped = true;
            break;
        }
        if !clipped {
            break;
        }
    }
    if remaining.len() == 3 {
        if let (Some(&a), Some(&b), Some(&c)) = (remaining.first(), remaining.get(1), remaining.get(2)) {
            out.push([a, b, c]);
        }
    } else {
        let first = remaining.first().copied().unwrap_or(0);
        for pair in remaining.windows(2).skip(1) {
            if let [b, c] = pair {
                out.push([first, *b, *c]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concave_faces_are_ear_clipped() {
        // An L shape: six corners, four triangles, total area 3.
        let mesh = Mesh {
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 0.0),
                Vec3::new(2.0, 1.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
                Vec3::new(1.0, 2.0, 0.0),
                Vec3::new(0.0, 2.0, 0.0),
            ],
            faces: vec![vec![0, 1, 2, 3, 4, 5]],
            ..Default::default()
        };
        let triangles = mesh.triangles();
        assert_eq!(triangles.len(), 4);
        let area: f32 = triangles
            .iter()
            .map(|([a, b, c], _)| {
                let (a, b, c) = (mesh.positions[*a as usize], mesh.positions[*b as usize], mesh.positions[*c as usize]);
                (b - a).cross(c - a).z * 0.5
            })
            .sum();
        assert!((area - 3.0).abs() < 1e-5, "area {area}");
    }

    #[test]
    fn weld_merges_close_vertices() {
        let mut mesh = Mesh {
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::new(1e-5, 0.0, 0.0), Vec3::X, Vec3::new(1.0, 1.0, 0.0)],
            faces: vec![vec![0, 1, 2], vec![3, 4, 5]],
            ..Default::default()
        };
        assert_eq!(mesh.weld(1e-3), 2);
        assert_eq!(mesh.positions.len(), 4);
        assert_eq!(mesh.faces, vec![vec![0, 1, 2], vec![0, 1, 3]]);
    }

    fn cylinder() -> Mesh {
        crate::Primitive::Cylinder { vertices: 16, radius: 1.0, depth: 2.0, fill: crate::primitives::Fill::Ngon }.build().unwrap()
    }

    #[test]
    fn smooth_by_angle_keeps_a_cylinders_rims_sharp() {
        let mut can = cylinder();
        can.smooth = true;
        // Plain smooth shading bends the side's normals towards the caps at the rims.
        let smooth = can.corner_normals();
        assert!(smooth.iter().flatten().any(|normal| normal.z.abs() > 0.3 && normal.z.abs() < 0.95));
        can.smooth_angle = Some(30f32.to_radians());
        for (face, normals) in can.faces.iter().zip(can.corner_normals()) {
            let flat = can.face_normal(face);
            for normal in normals {
                if flat.z.abs() > 0.99 {
                    assert!((normal - flat).length() < 1e-5, "a cap is flat: {normal} against {flat}");
                } else {
                    assert!(normal.z.abs() < 1e-5, "the side stays horizontal: {normal}");
                    assert!(normal.dot(flat) > 0.98, "and smooth around: {normal} against {flat}");
                }
            }
        }
        // Past the rims' 90°, every edge is smooth again: the plain smooth normals.
        can.smooth_angle = Some(std::f32::consts::PI);
        assert_eq!(can.corner_normals(), smooth);
    }

    #[test]
    fn corner_normals_are_the_face_or_vertex_normals_without_an_angle() {
        let mut can = cylinder();
        let flat: Vec<Vec<Vec3>> = can.faces.iter().map(|face| vec![can.face_normal(face); face.len()]).collect();
        assert_eq!(can.corner_normals(), flat);
        can.smooth = true;
        let vertex = can.vertex_normals();
        let expected: Vec<Vec<Vec3>> = can.faces.iter().map(|face| face.iter().map(|index| vertex[*index as usize]).collect()).collect();
        assert_eq!(can.corner_normals(), expected);
        // A cube smoothed by 30° is a flat cube; the angle only counts with smooth shading.
        let mut cube = crate::Primitive::Cube { size: 2.0 }.build().unwrap();
        let flat: Vec<Vec<Vec3>> = cube.faces.iter().map(|face| vec![cube.face_normal(face); face.len()]).collect();
        cube.smooth = true;
        cube.smooth_angle = Some(30f32.to_radians());
        assert_eq!(cube.corner_normals(), flat);
        assert!(cube.validate().is_ok());
        cube.smooth_angle = Some(4.0);
        assert!(cube.validate().is_err());
    }
}
