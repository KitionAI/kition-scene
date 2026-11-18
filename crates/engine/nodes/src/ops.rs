//! The mesh-changing nodes' work: deleting, merging, extruding, distributing points. Each keeps
//! the attributes it can follow (points' and faces' through deletion, for instance) and drops
//! the rest.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use glam::{Vec2, Vec3};
use kition_scene_model::Mesh;

use crate::field::{Column, Scalar};
use crate::geometry::{Attribute, Attributes, Domain, MAX_ELEMENTS, MeshComponent, PointsComponent, filter};
use crate::random;

fn index(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn edge_key(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}

fn face_edges(face: &[u32]) -> impl Iterator<Item = (u32, u32)> + '_ {
    face.iter().zip(face.iter().cycle().skip(1)).map(|(a, b)| (*a, *b))
}

/// What a deletion takes away.
#[derive(Default)]
pub struct Removal {
    pub points: Vec<bool>,
    pub edges: BTreeSet<(u32, u32)>,
    pub faces: Vec<bool>,
    /// Edges of removed faces that no kept face uses stay as loose edges.
    pub keep_face_edges: bool,
    /// Vertices only removed elements used go too.
    pub remove_orphans: bool,
}

impl MeshComponent {
    /// Removes points, edges, and faces (and what uses them), keeping attributes on points,
    /// faces, and corners.
    pub fn remove(&mut self, removal: &Removal) {
        let points = self.mesh.positions.len();
        let removed_point = |vertex: u32| removal.points.get(vertex as usize).copied().unwrap_or(false);
        let removed_edge = |a: u32, b: u32| removal.edges.contains(&edge_key(a, b));
        let keep_face: Vec<bool> = self
            .mesh
            .faces
            .iter()
            .enumerate()
            .map(|(number, face)| {
                !removal.faces.get(number).copied().unwrap_or(false)
                    && !face.iter().any(|vertex| removed_point(*vertex))
                    && !face_edges(face).any(|(a, b)| removed_edge(a, b))
            })
            .collect();
        let mut used_before = vec![false; points];
        for vertex in self.mesh.faces.iter().flatten().chain(self.mesh.loose_edges.iter().flatten()) {
            if let Some(flag) = used_before.get_mut(*vertex as usize) {
                *flag = true;
            }
        }
        let kept_edges: BTreeSet<(u32, u32)> = self
            .mesh
            .faces
            .iter()
            .zip(&keep_face)
            .filter(|(_, keep)| **keep)
            .flat_map(|(face, _)| face_edges(face).map(|(a, b)| edge_key(a, b)))
            .collect();
        let mut loose: BTreeSet<(u32, u32)> = self
            .mesh
            .loose_edges
            .iter()
            .map(|[a, b]| edge_key(*a, *b))
            .filter(|(a, b)| !removed_point(*a) && !removed_point(*b) && !removed_edge(*a, *b))
            .collect();
        for (face, keep) in self.mesh.faces.iter().zip(&keep_face) {
            if *keep {
                continue;
            }
            for (a, b) in face_edges(face) {
                let key = edge_key(a, b);
                let selected_face = removal.keep_face_edges;
                // A face that went because one of its points did leaves its other edges.
                let by_point = face.iter().any(|vertex| removed_point(*vertex));
                if (selected_face || by_point) && !removed_point(a) && !removed_point(b) && !removed_edge(a, b) && !kept_edges.contains(&key) {
                    loose.insert(key);
                }
            }
        }
        // Faces, their corners, and their attributes.
        let corner_keep: Vec<bool> = self.mesh.faces.iter().zip(&keep_face).flat_map(|(face, keep)| std::iter::repeat_n(*keep, face.len())).collect();
        let uvs = self.mesh.has_uvs() && self.mesh.uvs_aligned();
        if uvs {
            self.mesh.uvs = filter(&self.mesh.uvs, &keep_face);
        } else {
            self.mesh.uvs.clear();
        }
        self.mesh.faces = filter(&self.mesh.faces, &keep_face);
        if !self.materials.is_empty() {
            self.materials.resize(keep_face.len(), None);
            self.materials = filter(&self.materials, &keep_face);
        }
        self.mesh.loose_edges = loose.into_iter().map(|(a, b)| [a, b]).collect();
        // Points: removed ones, and orphans when asked.
        let mut used_after = vec![false; points];
        for vertex in self.mesh.faces.iter().flatten().chain(self.mesh.loose_edges.iter().flatten()) {
            if let Some(flag) = used_after.get_mut(*vertex as usize) {
                *flag = true;
            }
        }
        let keep_point: Vec<bool> = (0..points)
            .map(|vertex| {
                let used = used_after.get(vertex).copied().unwrap_or(false);
                // A point the removal left without its faces goes too, when orphans are removed.
                let orphaned = removal.remove_orphans && used_before.get(vertex).copied().unwrap_or(false) && !used;
                !removed_point(index(vertex)) && !orphaned
            })
            .collect();
        let mut remap = vec![u32::MAX; points];
        let mut next = 0u32;
        for (vertex, keep) in keep_point.iter().enumerate() {
            if *keep && let Some(slot) = remap.get_mut(vertex) {
                *slot = next;
                next += 1;
            }
        }
        self.mesh.positions = filter(&self.mesh.positions, &keep_point);
        let map = |vertex: &u32| remap.get(*vertex as usize).copied().unwrap_or(u32::MAX);
        for face in &mut self.mesh.faces {
            for vertex in face.iter_mut() {
                *vertex = map(vertex);
            }
        }
        for edge in &mut self.mesh.loose_edges {
            *edge = [map(&edge[0]), map(&edge[1])];
        }
        self.mesh.seams.clear();
        self.mesh.pinned.clear();
        drop_vertex_data(&mut self.mesh);
        for attribute in self.attributes.values_mut() {
            attribute.values = match attribute.domain {
                Domain::Point => filter(&attribute.values, &keep_point),
                Domain::Face => filter(&attribute.values, &keep_face),
                Domain::Corner => filter(&attribute.values, &corner_keep),
                Domain::Edge | Domain::Instance | Domain::Curve => Vec::new(),
            };
        }
        self.attributes.retain(|_, attribute| attribute.domain != Domain::Edge);
    }
}

/// Drops data kept per vertex that topology changes cannot follow (shape keys, the sculpt mask;
/// vertex groups travel as attributes while nodes evaluate).
pub fn drop_vertex_data(mesh: &mut Mesh) {
    mesh.shape_keys.clear();
    mesh.mask.clear();
    mesh.groups.clear();
}

/// Points of `positions` within `distance` of an earlier selected one map to it: the map from
/// each point to the one it merges into.
fn merge_map(positions: &[Vec3], selected: &[bool], distance: f32) -> Vec<usize> {
    let cell = distance.max(1e-6);
    let key = |point: Vec3| ((point.x / cell).floor() as i64, (point.y / cell).floor() as i64, (point.z / cell).floor() as i64);
    let mut grid: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    let mut target: Vec<usize> = (0..positions.len()).collect();
    for (at, point) in positions.iter().enumerate() {
        if !selected.get(at).copied().unwrap_or(false) {
            continue;
        }
        let (x, y, z) = key(*point);
        let mut found = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    for other in grid.get(&(x + dx, y + dy, z + dz)).map_or(&[][..], Vec::as_slice) {
                        if positions.get(*other).is_some_and(|other| other.distance(*point) <= distance) && found.is_none_or(|best| *other < best) {
                            found = Some(*other);
                        }
                    }
                }
            }
        }
        match found {
            Some(other) => {
                if let Some(slot) = target.get_mut(at) {
                    *slot = other;
                }
            }
            None => grid.entry((x, y, z)).or_default().push(at),
        }
    }
    target
}

impl MeshComponent {
    /// Merges selected vertices closer than `distance` (faces that collapse go; attributes on
    /// points stay, others are dropped).
    pub fn merge(&mut self, selected: &[bool], distance: f32) {
        let target = merge_map(&self.mesh.positions, selected, distance);
        let keep: Vec<bool> = target.iter().enumerate().map(|(at, to)| at == *to).collect();
        let mut remap = vec![u32::MAX; target.len()];
        let mut next = 0u32;
        for (at, keep) in keep.iter().enumerate() {
            if *keep && let Some(slot) = remap.get_mut(at) {
                *slot = next;
                next += 1;
            }
        }
        let full: Vec<u32> = target.iter().map(|to| remap.get(*to).copied().unwrap_or(u32::MAX)).collect();
        self.mesh.positions = filter(&self.mesh.positions, &keep);
        let faces_before = self.mesh.faces.len();
        self.mesh.reindex(&full);
        drop_vertex_data(&mut self.mesh);
        let same_faces = self.mesh.faces.len() == faces_before;
        if !same_faces {
            self.materials.clear();
        }
        self.attributes.retain(|_, attribute| attribute.domain == Domain::Point || (same_faces && attribute.domain != Domain::Edge));
        for attribute in self.attributes.values_mut() {
            if attribute.domain == Domain::Point {
                attribute.values = filter(&attribute.values, &keep);
            }
        }
    }
}

impl PointsComponent {
    /// Merges selected points closer than `distance`.
    pub fn merge(&mut self, selected: &[bool], distance: f32) {
        let target = merge_map(&self.positions, selected, distance);
        let keep: Vec<bool> = target.iter().enumerate().map(|(at, to)| at == *to).collect();
        self.retain(&keep);
    }

    /// Keeps the points `keep` marks.
    pub fn retain(&mut self, keep: &[bool]) {
        self.positions = filter(&self.positions, keep);
        self.radius = filter(&self.radius, keep);
        for attribute in self.attributes.values_mut() {
            attribute.values = filter(&attribute.values, keep);
        }
    }
}

/// How Extrude Mesh extrudes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extrude {
    Vertices,
    Edges,
    Faces { individual: bool },
}

/// Extrudes the selected elements by their offsets (per point for vertices and edges, per face
/// for faces), storing the new top and side elements as boolean attributes `top` and `side`.
pub fn extrude(component: &mut MeshComponent, mode: Extrude, selected: &[bool], offsets: &[Vec3], top: &str, side: &str) -> Result<(), String> {
    let before = [Domain::Point, Domain::Face].map(|domain| component.domain_len(domain));
    let mut top_points: Vec<u32> = Vec::new();
    let mut top_faces: Vec<usize> = Vec::new();
    let mut side_faces: Vec<usize> = Vec::new();
    let offset = |at: usize| offsets.get(at).copied().unwrap_or(Vec3::ZERO);
    let is_selected = |at: usize| selected.get(at).copied().unwrap_or(false);
    let mesh = &mut component.mesh;
    let uvs = mesh.has_uvs() && mesh.uvs_aligned();
    match mode {
        Extrude::Vertices => {
            for vertex in 0..before[0] {
                if !is_selected(vertex) {
                    continue;
                }
                let point = mesh.positions.get(vertex).copied().unwrap_or(Vec3::ZERO);
                mesh.positions.push(point + offset(vertex));
                let new = index(mesh.positions.len() - 1);
                mesh.loose_edges.push([index(vertex), new]);
                top_points.push(new);
            }
        }
        Extrude::Edges => {
            let edges = mesh.edges();
            let mut copies: BTreeMap<u32, u32> = BTreeMap::new();
            // Winding follows the face the edge borders, so the new side faces the same way.
            let mut direction: HashMap<(u32, u32), (u32, u32)> = HashMap::new();
            for face in &mesh.faces {
                for (a, b) in face_edges(face) {
                    direction.entry(edge_key(a, b)).or_insert((a, b));
                }
            }
            for (number, [a, b]) in edges.into_iter().enumerate() {
                if !is_selected(number) {
                    continue;
                }
                let (a, b) = direction.get(&edge_key(a, b)).copied().unwrap_or((a, b));
                let mut copy = |vertex: u32, mesh: &mut Mesh| -> u32 {
                    *copies.entry(vertex).or_insert_with(|| {
                        let point = mesh.positions.get(vertex as usize).copied().unwrap_or(Vec3::ZERO);
                        mesh.positions.push(point + offset(vertex as usize));
                        index(mesh.positions.len() - 1)
                    })
                };
                let (new_a, new_b) = (copy(a, mesh), copy(b, mesh));
                mesh.faces.push(vec![b, a, new_a, new_b]);
                side_faces.push(mesh.faces.len() - 1);
            }
            top_points.extend(copies.values().copied());
        }
        Extrude::Faces { individual: true } => {
            for number in 0..before[1] {
                if !is_selected(number) {
                    continue;
                }
                let Some(face) = mesh.faces.get(number).cloned() else { continue };
                let start = mesh.positions.len();
                for vertex in &face {
                    let point = mesh.positions.get(*vertex as usize).copied().unwrap_or(Vec3::ZERO);
                    mesh.positions.push(point + offset(number));
                }
                let new: Vec<u32> = (start..start + face.len()).map(index).collect();
                for corner in 0..face.len() {
                    let next = (corner + 1) % face.len();
                    let (Some(a), Some(b), Some(new_a), Some(new_b)) = (face.get(corner), face.get(next), new.get(corner), new.get(next)) else {
                        continue;
                    };
                    mesh.faces.push(vec![*a, *b, *new_b, *new_a]);
                    side_faces.push(mesh.faces.len() - 1);
                }
                if let Some(slot) = mesh.faces.get_mut(number) {
                    *slot = new;
                }
                top_faces.push(number);
            }
        }
        Extrude::Faces { individual: false } => {
            // Each region vertex moves by the mean of its selected faces' offsets.
            let mut sums: BTreeMap<u32, (Vec3, f32)> = BTreeMap::new();
            let mut edge_count: HashMap<(u32, u32), usize> = HashMap::new();
            for number in 0..before[1] {
                if !is_selected(number) {
                    continue;
                }
                let Some(face) = mesh.faces.get(number) else { continue };
                for vertex in face {
                    let entry = sums.entry(*vertex).or_insert((Vec3::ZERO, 0.0));
                    entry.0 += offset(number);
                    entry.1 += 1.0;
                }
                for (a, b) in face_edges(face) {
                    *edge_count.entry(edge_key(a, b)).or_default() += 1;
                }
            }
            let mut copies: BTreeMap<u32, u32> = BTreeMap::new();
            for (vertex, (sum, count)) in &sums {
                let point = mesh.positions.get(*vertex as usize).copied().unwrap_or(Vec3::ZERO);
                mesh.positions.push(point + *sum / count.max(1.0));
                copies.insert(*vertex, index(mesh.positions.len() - 1));
            }
            let moved = |vertex: u32| copies.get(&vertex).copied().unwrap_or(vertex);
            for number in 0..before[1] {
                if !is_selected(number) {
                    continue;
                }
                let Some(face) = mesh.faces.get(number).cloned() else { continue };
                for (a, b) in face_edges(&face) {
                    if edge_count.get(&edge_key(a, b)).copied().unwrap_or(0) == 1 {
                        mesh.faces.push(vec![a, b, moved(b), moved(a)]);
                        side_faces.push(mesh.faces.len() - 1);
                    }
                }
                if let Some(slot) = mesh.faces.get_mut(number) {
                    *slot = face.iter().map(|vertex| moved(*vertex)).collect();
                }
                top_faces.push(number);
            }
        }
    }
    if mesh.positions.len() > MAX_ELEMENTS {
        return Err(format!("extruding would make more than {MAX_ELEMENTS} vertices"));
    }
    if uvs {
        mesh.fill_uvs();
    }
    drop_vertex_data(mesh);
    mesh.seams.clear();
    mesh.pinned.clear();
    // Attributes: points and faces that existed keep theirs; new ones copy nothing (zero).
    let after = [mesh.positions.len(), mesh.faces.len()];
    component.attributes.retain(|_, attribute| matches!(attribute.domain, Domain::Point | Domain::Face));
    for attribute in component.attributes.values_mut() {
        let zero = attribute.values.first().map_or(Scalar::Float(0.0), |value| Scalar::zero(value.kind()));
        let len = if attribute.domain == Domain::Point { after[0] } else { after[1] };
        attribute.values.resize(len, zero);
    }
    if !component.materials.is_empty() {
        let mut materials = component.materials.clone();
        materials.resize(before[1], None);
        // Side faces take the material of nothing in particular: the first selected face's.
        let fill = top_faces.first().and_then(|face| materials.get(*face).copied()).flatten();
        materials.resize(after[1], fill);
        component.materials = materials;
    }
    if !top.is_empty() {
        let values: Column = if mode == Extrude::Vertices || mode == Extrude::Edges {
            let set: BTreeSet<u32> = top_points.iter().copied().collect();
            (0..after[0]).map(|vertex| Scalar::Bool(set.contains(&index(vertex)))).collect()
        } else {
            let set: BTreeSet<usize> = top_faces.iter().copied().collect();
            (0..after[1]).map(|face| Scalar::Bool(set.contains(&face))).collect()
        };
        let domain = if matches!(mode, Extrude::Faces { .. }) { Domain::Face } else { Domain::Point };
        component.attributes.insert(top.to_owned(), Attribute { domain, values });
    }
    if !side.is_empty() {
        let set: BTreeSet<usize> = side_faces.iter().copied().collect();
        let values = (0..after[1]).map(|face| Scalar::Bool(set.contains(&face))).collect();
        component.attributes.insert(side.to_owned(), Attribute { domain: Domain::Face, values });
    }
    // Region vertices left without a face (a closed region's old shell) go.
    if mode == (Extrude::Faces { individual: false }) {
        let mut used = vec![false; after[0]];
        for vertex in component.mesh.faces.iter().flatten().chain(component.mesh.loose_edges.iter().flatten()) {
            if let Some(flag) = used.get_mut(*vertex as usize) {
                *flag = true;
            }
        }
        let orphans: Vec<bool> = (0..after[0]).map(|vertex| vertex < before[0] && !used.get(vertex).copied().unwrap_or(true)).collect();
        if orphans.iter().any(|orphan| *orphan) {
            component.remove(&Removal { points: orphans, ..Removal::default() });
        }
    }
    Ok(())
}

/// Points scattered on the selected faces, about `density` per unit of area, with the face
/// normal and a rotation that turns +Z onto it stored under `names` (normal, rotation). Other
/// attributes on points, faces, and corners are carried over (blended across each triangle).
pub fn distribute(component: &MeshComponent, selected: &[bool], density: &[f32], seed: i32, names: [&str; 2]) -> Result<PointsComponent, String> {
    let mesh = &component.mesh;
    let corner_starts: Vec<usize> = mesh
        .faces
        .iter()
        .scan(0usize, |start, face| {
            let at = *start;
            *start += face.len();
            Some(at)
        })
        .collect();
    let corner_attributes: Vec<(String, Column)> = component
        .attributes
        .iter()
        .filter(|(name, _)| !names.contains(&name.as_str()))
        .map(|(name, attribute)| (name.clone(), component.adapt(&attribute.values, attribute.domain, Domain::Corner)))
        .collect();
    let mut points = PointsComponent::default();
    let mut normals = Vec::new();
    let mut carried: Vec<Column> = vec![Vec::new(); corner_attributes.len()];
    for (triangle_number, (corners, face)) in mesh.corner_triangles().into_iter().enumerate() {
        let face = face as usize;
        if !selected.get(face).copied().unwrap_or(false) {
            continue;
        }
        let Some(face_vertices) = mesh.faces.get(face) else { continue };
        let start = corner_starts.get(face).copied().unwrap_or(0);
        let vertex =
            |corner: u32| face_vertices.get(corner as usize).and_then(|vertex| mesh.positions.get(*vertex as usize)).copied().unwrap_or(Vec3::ZERO);
        let [a, b, c] = corners.map(vertex);
        let area = (b - a).cross(c - a).length() / 2.0;
        let expected = area * density.get(face).copied().unwrap_or(0.0).max(0.0);
        let id = i32::try_from(triangle_number).unwrap_or(i32::MAX);
        let count = expected.floor() + if random::unit(id, seed, 7) < expected.fract() { 1.0 } else { 0.0 };
        if points.positions.len() as f32 + count > MAX_ELEMENTS as f32 {
            return Err(format!("distributing would make more than {MAX_ELEMENTS} points"));
        }
        let normal = (b - a).cross(c - a).normalize_or(Vec3::Z);
        for sample in 0..count as u32 {
            let channel = sample * 2;
            let (mut u, mut v) = (random::unit(id, seed, 8 + channel), random::unit(id, seed, 9 + channel));
            if u + v > 1.0 {
                (u, v) = (1.0 - u, 1.0 - v);
            }
            let weights = [1.0 - u - v, u, v];
            points.positions.push(a * weights[0] + b * weights[1] + c * weights[2]);
            points.radius.push(0.05);
            normals.push(normal);
            for ((_, column), out) in corner_attributes.iter().zip(&mut carried) {
                let at = |corner: u32| column.get(start + corner as usize).copied();
                let values: Vec<Scalar> = corners.iter().filter_map(|corner| at(*corner)).collect();
                out.push(blend(&values, &weights));
            }
        }
    }
    for ((name, _), values) in corner_attributes.into_iter().zip(carried) {
        points.attributes.insert(name, Attribute { domain: Domain::Point, values });
    }
    let [normal_name, rotation_name] = names;
    if !normal_name.is_empty() {
        points.attributes.insert(
            normal_name.to_owned(),
            Attribute { domain: Domain::Point, values: normals.iter().map(|normal| Scalar::Vector(*normal)).collect() },
        );
    }
    if !rotation_name.is_empty() {
        let values = normals.iter().map(|normal| Scalar::Vector(rotation_to(*normal))).collect();
        points.attributes.insert(rotation_name.to_owned(), Attribute { domain: Domain::Point, values });
    }
    Ok(points)
}

/// The XYZ Euler angles of the shortest rotation taking +Z onto `normal`.
pub fn rotation_to(normal: Vec3) -> Vec3 {
    let rotation = glam::Quat::from_rotation_arc(Vec3::Z, normal.normalize_or(Vec3::Z));
    kition_scene_model::scene::matrix_euler(glam::Mat3::from_quat(rotation))
}

fn blend(values: &[Scalar], weights: &[f32; 3]) -> Scalar {
    let Some(first) = values.first().copied() else { return Scalar::Float(0.0) };
    match first {
        Scalar::Float(_) => Scalar::Float(values.iter().zip(weights).map(|(value, weight)| value.float() * weight).sum()),
        Scalar::Vector(_) => Scalar::Vector(values.iter().zip(weights).map(|(value, weight)| value.vector() * *weight).sum()),
        // Integers and booleans take the nearest corner's value.
        _ => {
            let nearest = weights.iter().enumerate().fold((0, f32::MIN), |best, (at, weight)| if *weight > best.1 { (at, *weight) } else { best }).0;
            values.get(nearest).copied().unwrap_or(first)
        }
    }
}

/// Texture coordinates for faces added by an operation that had none: zero.
pub fn zero_uvs(mesh: &mut Mesh) {
    if mesh.has_uvs() {
        mesh.uvs.resize_with(mesh.faces.len(), Vec::new);
        for (uvs, face) in mesh.uvs.iter_mut().zip(&mesh.faces) {
            uvs.resize(face.len(), Vec2::ZERO);
        }
    }
}

/// A mesh's vertex groups as point attributes (and the groups taken off the mesh).
pub fn groups_to_attributes(component: &mut MeshComponent) {
    for group in std::mem::take(&mut component.mesh.groups) {
        let values = group.weights.iter().map(|weight| Scalar::Float(*weight)).collect();
        component.attributes.insert(group.name, Attribute { domain: Domain::Point, values });
    }
}

/// Named number attributes on points as vertex groups (anonymous ones, starting with a dot,
/// stay behind).
pub fn attributes_to_groups(component: &MeshComponent, mesh: &mut Mesh) {
    let count = mesh.positions.len();
    mesh.groups = component
        .attributes
        .iter()
        .filter(|(name, attribute)| !name.starts_with('.') && attribute.domain == Domain::Point && attribute.values.len() == count)
        .filter(|(_, attribute)| attribute.values.first().is_none_or(|value| matches!(value, Scalar::Float(_))))
        .map(|(name, attribute)| kition_scene_model::VertexGroup {
            name: name.clone(),
            weights: attribute.values.iter().map(|value| value.float().clamp(0.0, 1.0)).collect(),
        })
        .collect();
}

/// Attributes of `values` kept where `keep` is set, for a component made from some elements.
pub fn pick_attributes(attributes: &[(String, Column)], keep: &[bool], domain: Domain) -> Attributes {
    attributes.iter().map(|(name, values)| (name.clone(), Attribute { domain, values: filter(values, keep) })).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prims;

    fn component(mesh: Mesh) -> MeshComponent {
        MeshComponent::new(mesh)
    }

    fn normals(component: &MeshComponent, scale: f32) -> Vec<Vec3> {
        component.normals(Domain::Face).into_iter().map(|normal| normal * scale).collect()
    }

    #[test]
    fn extrusion_matches_the_reference_counts() {
        let cube = component(prims::cube(Vec3::ONE, [2, 2, 2]).unwrap());
        let grid = component(prims::grid(1.0, 1.0, 3, 3).unwrap());
        let run = |base: &MeshComponent, mode: Extrude, selected: Vec<bool>, offsets: Vec<Vec3>| {
            let mut out = base.clone();
            extrude(&mut out, mode, &selected, &offsets, ".top", ".side").unwrap();
            out.mesh.validate().unwrap();
            let bounds = out.mesh.bounds(glam::Mat4::IDENTITY);
            (out.mesh.positions.len(), out.mesh.faces.len(), bounds.max)
        };
        let region = run(&cube, Extrude::Faces { individual: false }, vec![true; 6], normals(&cube, 0.5));
        assert_eq!((region.0, region.1), (8, 6));
        assert!((region.2.x - 2.0 / 3.0).abs() < 1e-5);
        let individual = run(&cube, Extrude::Faces { individual: true }, vec![true; 6], normals(&cube, 0.5));
        assert_eq!((individual.0, individual.1), (32, 30));
        assert!((individual.2.x - 1.0).abs() < 1e-5);
        assert_eq!(run(&grid, Extrude::Faces { individual: false }, vec![true; 4], normals(&grid, 0.5)).0, 17);
        assert_eq!(run(&grid, Extrude::Faces { individual: true }, vec![true; 4], normals(&grid, 0.5)).0, 25);
        assert_eq!(run(&grid, Extrude::Faces { individual: false }, vec![true, false, false, false], normals(&grid, 0.5)).0, 13);
        let up = vec![Vec3::Z; 9];
        assert_eq!(run(&grid, Extrude::Vertices, vec![true; 9], up.clone()).0, 18);
        let edges = grid.mesh.edges().len();
        let by_edges = run(&grid, Extrude::Edges, vec![true; edges], up);
        assert_eq!((by_edges.0, by_edges.1), (18, 16));
    }

    #[test]
    fn deleting_a_vertex_keeps_the_far_edges() {
        let mut grid = component(prims::grid(1.0, 1.0, 3, 3).unwrap());
        let mut points = vec![false; 9];
        points[4] = true;
        grid.remove(&Removal { points, remove_orphans: true, ..Removal::default() });
        assert_eq!(grid.mesh.positions.len(), 8);
        assert_eq!(grid.mesh.faces.len(), 0);
        assert_eq!(grid.mesh.edges().len(), 8);
        grid.mesh.validate().unwrap();
    }

    #[test]
    fn merging_welds_close_points() {
        let mut cube = component(prims::cube(Vec3::ONE, [2, 2, 2]).unwrap());
        cube.mesh.positions.push(Vec3::splat(0.5) + Vec3::splat(1e-5));
        let corner = cube.mesh.positions.iter().position(|point| *point == Vec3::splat(0.5)).unwrap() as u32;
        cube.mesh.loose_edges.push([corner, 8]);
        cube.merge(&[true; 9], 0.001);
        assert_eq!(cube.mesh.positions.len(), 8);
        assert!(cube.mesh.loose_edges.is_empty());
    }

    #[test]
    fn distribution_follows_density() {
        let grid = component(prims::grid(10.0, 10.0, 11, 11).unwrap());
        let points = distribute(&grid, &[true; 100], &[10.0; 100], 0, [".n", ".r"]).unwrap();
        let count = points.positions.len() as f32;
        assert!((count - 1000.0).abs() < 100.0, "{count}");
        assert!(points.positions.iter().all(|point| point.x.abs() <= 5.0 && point.y.abs() <= 5.0 && point.z == 0.0));
    }
}
