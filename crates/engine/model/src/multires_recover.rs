//! Invert the combinatorics of quad subdivision, independently of positions and numbering.
//! A refined quad has old/edge/face/edge corners. Propagating those roles around each
//! connected component avoids geometric guesses and unbounded search. Every candidate is
//! checked by subdividing its recovered topology and finding an oriented bijection.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::error::{Result, SceneError};
use crate::mesh::{Mesh, face_edges};
use crate::modifiers::catmull_clark;

const UNKNOWN: u8 = 3;
const OLD: u8 = 0;
const EDGE: u8 = 1;
const FACE: u8 = 2;
pub(super) const MAX_RECOVERY_FACES: usize = 100_000;

fn unsupported() -> SceneError {
    SceneError::params("no complete quad subdivision level can be recovered from this topology")
}

fn key(a: u32, b: u32) -> [u32; 2] {
    [a.min(b), a.max(b)]
}

/// Subdivision's edge numbering is sorted by its endpoint indices.
fn edges(mesh: &Mesh) -> Vec<[u32; 2]> {
    mesh.faces.iter().flat_map(|face| face_edges(face).map(|(a, b)| key(a, b))).collect::<BTreeSet<_>>().into_iter().collect()
}

struct Topology {
    incident: Vec<Vec<usize>>,
    neighbours: Vec<BTreeSet<u32>>,
    boundary: BTreeSet<u32>,
}

impl Topology {
    fn new(mesh: &Mesh) -> Result<Self> {
        mesh.validate()?;
        if mesh.faces.is_empty()
            || mesh.faces.len() > MAX_RECOVERY_FACES
            || mesh.positions.len() > mesh.faces.len().saturating_mul(4)
            || !mesh.loose_edges.is_empty()
            || mesh.faces.iter().any(|f| f.len() != 4)
        {
            return Err(SceneError::params("recovery needs quad faces, no loose edges, and at most 100000 base faces"));
        }
        let mut incident = vec![Vec::new(); mesh.positions.len()];
        let mut neighbours = vec![BTreeSet::new(); mesh.positions.len()];
        let mut uses: BTreeMap<[u32; 2], Vec<(u32, u32)>> = BTreeMap::new();
        for (index, face) in mesh.faces.iter().enumerate() {
            for vertex in face {
                incident.get_mut(*vertex as usize).ok_or_else(unsupported)?.push(index);
            }
            for (a, b) in face_edges(face) {
                neighbours.get_mut(a as usize).ok_or_else(unsupported)?.insert(b);
                neighbours.get_mut(b as usize).ok_or_else(unsupported)?.insert(a);
                uses.entry(key(a, b)).or_default().push((a, b));
            }
        }
        let mut boundary = BTreeSet::new();
        for (edge, directions) in &uses {
            match directions.as_slice() {
                [_] => boundary.extend(edge),
                [a, b] if a.0 == b.1 && a.1 == b.0 => {}
                _ => return Err(unsupported()),
            }
        }
        // Each vertex has exactly one disk or half-disk of incident faces; reject bow ties.
        for (vertex, faces) in incident.iter().enumerate() {
            let mut ring: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
            for index in faces {
                let face = mesh.faces.get(*index).ok_or_else(unsupported)?;
                let at = face.iter().position(|v| *v as usize == vertex).ok_or_else(unsupported)?;
                let a = *face.get((at + 3) % 4).ok_or_else(unsupported)?;
                let b = *face.get((at + 1) % 4).ok_or_else(unsupported)?;
                ring.entry(a).or_default().push(b);
                ring.entry(b).or_default().push(a);
            }
            let ends = ring.values().filter(|v| v.len() == 1).count();
            if ring.is_empty() || ring.values().any(|v| v.len() > 2) || ![0, 2].contains(&ends) {
                return Err(unsupported());
            }
            let mut visited = BTreeSet::new();
            let mut pending: Vec<u32> = ring.keys().next().copied().into_iter().collect();
            while let Some(v) = pending.pop() {
                if visited.insert(v) {
                    pending.extend(ring.get(&v).into_iter().flatten().copied());
                }
            }
            if visited.len() != ring.len() {
                return Err(unsupported());
            }
        }
        Ok(Self { incident, neighbours, boundary })
    }

    fn propagate(&self, mesh: &Mesh, seed: u32) -> Result<BTreeMap<u32, u8>> {
        let mut roles = BTreeMap::new();
        let mut pending = VecDeque::from([(seed, OLD)]);
        while let Some((vertex, role)) = pending.pop_front() {
            if let Some(known) = roles.get(&vertex) {
                if *known != role {
                    return Err(unsupported());
                }
                continue;
            }
            if role == FACE && (self.boundary.contains(&vertex) || self.incident.get(vertex as usize).map_or(0, Vec::len) != 4) {
                return Err(unsupported());
            }
            roles.insert(vertex, role);
            for face in self.incident.get(vertex as usize).into_iter().flatten() {
                let face = mesh.faces.get(*face).ok_or_else(unsupported)?;
                let at = face.iter().position(|v| *v == vertex).ok_or_else(unsupported)?;
                for delta in 1..4 {
                    let next = *face.get((at + delta) % 4).ok_or_else(unsupported)?;
                    if delta == 2 {
                        pending.push_back((next, if role == EDGE { EDGE } else { 2 - role }));
                    } else if role != EDGE {
                        pending.push_back((next, EDGE));
                    }
                }
            }
        }
        Ok(roles)
    }

    fn roles(&self, mesh: &Mesh) -> Result<Vec<u8>> {
        let mut roles = vec![UNKNOWN; mesh.positions.len()];
        for start in 0..mesh.vertex_count() {
            if roles.get(start as usize).copied() != Some(UNKNOWN) {
                continue;
            }
            let mut component = BTreeSet::new();
            let mut pending = vec![start];
            while let Some(v) = pending.pop() {
                if component.insert(v) {
                    pending.extend(self.neighbours.get(v as usize).into_iter().flatten().copied());
                }
            }
            // Non-four-valent interior vertices and boundary corners must be old vertices.
            let forced = component.iter().copied().find(|v| {
                let degree = self.neighbours.get(*v as usize).map_or(0, BTreeSet::len);
                if self.boundary.contains(v) { degree == 2 || degree > 3 } else { degree != 4 }
            });
            // Fully regular components have several possible phases, often with different UV
            // compatibility. Refuse that ambiguity rather than choosing a numbering-based phase.
            let seed = forced.ok_or_else(|| {
                SceneError::params("each component needs a boundary corner or a non-four-valent interior vertex to identify its subdivision phase")
            })?;
            let recovered = self.propagate(mesh, seed)?;
            if recovered.len() != component.len() {
                return Err(unsupported());
            }
            for (v, role) in recovered {
                *roles.get_mut(v as usize).ok_or_else(unsupported)? = role;
            }
        }
        Ok(roles)
    }
}

/// Canonical oriented face key: the smallest vertex first, winding preserved.
fn cycle(face: &[u32]) -> Vec<u32> {
    let at = face.iter().enumerate().min_by_key(|(_, v)| *v).map_or(0, |(i, _)| i);
    face.iter().cycle().skip(at).take(face.len()).copied().collect()
}

/// Each new face corresponds to one old face, with the same winding.
fn face_map(old: &Mesh, new: &Mesh, vertices: &[u32]) -> Result<Vec<usize>> {
    let old_faces: BTreeMap<Vec<u32>, usize> = old.faces.iter().enumerate().map(|(i, f)| (cycle(f), i)).collect();
    if old_faces.len() != old.faces.len() || new.faces.len() != old.faces.len() {
        return Err(unsupported());
    }
    let found: Vec<usize> = new
        .faces
        .iter()
        .map(|face| {
            let mapped = face.iter().map(|v| vertices.get(*v as usize).copied().ok_or_else(unsupported)).collect::<Result<Vec<_>>>()?;
            old_faces.get(&cycle(&mapped)).copied().ok_or_else(unsupported)
        })
        .collect::<Result<_>>()?;
    if found.iter().collect::<BTreeSet<_>>().len() != found.len() {
        return Err(unsupported());
    }
    Ok(found)
}

/// Recover one coarse mesh, plus the map from its subdivided vertices to the input vertices.
pub(super) fn recover(mesh: &Mesh) -> Result<(Mesh, Vec<u32>)> {
    if !mesh.groups.is_empty()
        || !mesh.shape_keys.is_empty()
        || !mesh.mask.is_empty()
        || !mesh.face_sets.is_empty()
        || !mesh.hidden.is_empty()
        || !mesh.seams.is_empty()
        || !mesh.pinned.is_empty()
    {
        return Err(SceneError::params(
            "recovery cannot retain vertex groups, shape keys, masks, face sets, hidden faces, seams, or UV pins; remove those data first",
        ));
    }
    let topology = Topology::new(mesh)?;
    let roles = topology.roles(mesh)?;
    let is = |v: u32, role: u8| roles.get(v as usize).copied() == Some(role);
    let old: Vec<u32> = (0..mesh.vertex_count()).filter(|v| is(*v, OLD)).collect();
    let centers: Vec<u32> = (0..mesh.vertex_count()).filter(|v| is(*v, FACE)).collect();
    let ids: BTreeMap<u32, u32> = old.iter().copied().zip(0..).collect();
    let mut coarse = Mesh {
        positions: old.iter().filter_map(|v| mesh.positions.get(*v as usize).copied()).collect(),
        smooth: mesh.smooth,
        smooth_angle: mesh.smooth_angle,
        ..Default::default()
    };
    for center in &centers {
        let mut next = BTreeMap::new();
        let mut uvs = BTreeMap::new();
        for index in topology.incident.get(*center as usize).into_iter().flatten() {
            let face = mesh.faces.get(*index).ok_or_else(unsupported)?;
            let at = face.iter().position(|v| is(*v, OLD)).ok_or_else(unsupported)?;
            let vertex = *face.get(at).ok_or_else(unsupported)?;
            let edge = *face.get((at + 1) % 4).ok_or_else(unsupported)?;
            let other = topology
                .neighbours
                .get(edge as usize)
                .into_iter()
                .flatten()
                .copied()
                .find(|v| *v != vertex && is(*v, OLD))
                .ok_or_else(unsupported)?;
            next.insert(vertex, other);
            if mesh.has_uvs() {
                uvs.insert(vertex, mesh.corner_uv(*index, at));
            }
        }
        let start = *next.keys().next().ok_or_else(unsupported)?;
        let mut at = start;
        let mut face = Vec::new();
        let mut uv = Vec::new();
        for _ in 0..4 {
            face.push(*ids.get(&at).ok_or_else(unsupported)?);
            if mesh.has_uvs() {
                uv.push(*uvs.get(&at).ok_or_else(unsupported)?);
            }
            at = *next.get(&at).ok_or_else(unsupported)?;
        }
        if at != start || face.iter().collect::<BTreeSet<_>>().len() != 4 {
            return Err(unsupported());
        }
        coarse.faces.push(face);
        if mesh.has_uvs() {
            coarse.uvs.push(uv);
        }
    }
    let mut midpoints = BTreeMap::new();
    for v in (0..mesh.vertex_count()).filter(|v| is(*v, EDGE)) {
        let ends: Vec<u32> = topology.neighbours.get(v as usize).into_iter().flatten().filter_map(|v| ids.get(v).copied()).collect();
        let [a, b] = ends.as_slice() else {
            return Err(unsupported());
        };
        if midpoints.insert(key(*a, *b), v).is_some() {
            return Err(unsupported());
        }
    }
    let mut map = old;
    for edge in edges(&coarse) {
        map.push(*midpoints.get(&edge).ok_or_else(unsupported)?);
    }
    map.extend(centers);
    if map.len() != mesh.positions.len() || map.iter().collect::<BTreeSet<_>>().len() != map.len() {
        return Err(unsupported());
    }
    Topology::new(&coarse)?;
    let refined = catmull_clark(&coarse);
    let faces = face_map(mesh, &refined, &map)?;
    // UVs have no multires offsets: only accept maps exactly reproduced by linear subdivision.
    if mesh.has_uvs() {
        for (new_index, old_index) in faces.into_iter().enumerate() {
            let new = refined.faces.get(new_index).ok_or_else(unsupported)?;
            let old = mesh.faces.get(old_index).ok_or_else(unsupported)?;
            for (corner, v) in new.iter().enumerate() {
                let mapped = map.get(*v as usize).ok_or_else(unsupported)?;
                let at = old.iter().position(|v| v == mapped).ok_or_else(unsupported)?;
                if refined.corner_uv(new_index, corner).distance(mesh.corner_uv(old_index, at)) > 1e-6 {
                    return Err(SceneError::params("the UV map is not a linear subdivision; recovery would lose UV detail"));
                }
            }
        }
    }
    Ok((coarse, map))
}

/// Carry a vertex bijection through another subdivision, including edge and face vertices.
pub(super) fn refine_map(old: &Mesh, new: &Mesh, map: &[u32]) -> Result<Vec<u32>> {
    let faces = face_map(old, new, map)?;
    let old_edges = edges(old);
    let lookup: BTreeMap<[u32; 2], u32> = old_edges.iter().copied().zip(old.vertex_count()..).collect();
    let mut out = map.to_vec();
    for [a, b] in edges(new) {
        let a = *map.get(a as usize).ok_or_else(unsupported)?;
        let b = *map.get(b as usize).ok_or_else(unsupported)?;
        out.push(*lookup.get(&key(a, b)).ok_or_else(unsupported)?);
    }
    let start = old.positions.len() + old_edges.len();
    for face in faces {
        out.push(u32::try_from(start + face).map_err(|_| unsupported())?);
    }
    Ok(out)
}
