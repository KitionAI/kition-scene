//! The Decimate modifier's other two ways, as the reference application has them (checked in
//! `tests/decimate_blender.rs`):
//!
//! - Un-subdivide undoes grid subdivision one half step at a time: the regular vertices (four
//!   edges inside, three on an open edge) are coloured alternately across their edges, starting
//!   next to an irregular vertex, and every vertex of the removed colour gives way to one face
//!   through its neighbours. Old faces that keep three corners or more stay.
//! - Planar dissolves the edges between faces that meet at less than an angle, growing flat
//!   regions face by face (the flattest join first, against the region's mean normal), then
//!   drops the vertices left between two nearly straight edges.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, VecDeque};

use glam::Vec3;

use crate::mesh::{Mesh, face_edges};

fn key(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}

/// Each vertex's faces and its neighbours along edges, and whether it lies on an open edge.
struct Topology {
    faces_of: Vec<Vec<usize>>,
    neighbours: Vec<BTreeSet<u32>>,
    boundary: Vec<bool>,
}

fn topology(mesh: &Mesh) -> Topology {
    let count = mesh.positions.len();
    let mut faces_of = vec![Vec::new(); count];
    let mut neighbours = vec![BTreeSet::new(); count];
    let mut uses: HashMap<(u32, u32), u32> = HashMap::new();
    for (index, face) in mesh.faces.iter().enumerate() {
        for vertex in face {
            if let Some(list) = faces_of.get_mut(*vertex as usize) {
                list.push(index);
            }
        }
        for (a, b) in face_edges(face) {
            *uses.entry(key(a, b)).or_default() += 1;
            if let Some(set) = neighbours.get_mut(a as usize) {
                set.insert(b);
            }
            if let Some(set) = neighbours.get_mut(b as usize) {
                set.insert(a);
            }
        }
    }
    let mut boundary = vec![false; count];
    for ((a, b), used) in uses {
        if used == 1 {
            for vertex in [a, b] {
                if let Some(flag) = boundary.get_mut(vertex as usize) {
                    *flag = true;
                }
            }
        }
    }
    Topology { faces_of, neighbours, boundary }
}

/// The neighbours of `vertex` in order round it, following the winding of its faces; open at an
/// edge of the surface. `None` when its faces do not make one fan.
fn fan(mesh: &Mesh, topology: &Topology, vertex: u32) -> Option<Vec<u32>> {
    // For each face round the vertex: the corner before it and the one after.
    let mut next_of: BTreeMap<u32, u32> = BTreeMap::new();
    for face in topology.faces_of.get(vertex as usize)? {
        let corners = mesh.faces.get(*face)?;
        let slot = corners.iter().position(|corner| *corner == vertex)?;
        let count = corners.len();
        let before = *corners.get((slot + count - 1) % count)?;
        let after = *corners.get((slot + 1) % count)?;
        // Seen from outside, going round the vertex, `after` comes before `before`.
        if next_of.insert(after, before).is_some() {
            return None;
        }
    }
    let targets: BTreeSet<u32> = next_of.values().copied().collect();
    let start = next_of.keys().copied().find(|from| !targets.contains(from)).or_else(|| next_of.keys().next().copied())?;
    let mut ring = vec![start];
    let mut at = start;
    while let Some(next) = next_of.get(&at) {
        if *next == start {
            break;
        }
        if ring.len() > next_of.len() {
            return None;
        }
        ring.push(*next);
        at = *next;
    }
    (ring.len() == topology.neighbours.get(vertex as usize)?.len()).then_some(ring)
}

/// One un-subdivide step; the mesh unchanged when no vertex can go.
fn unsubdivide_once(mesh: &Mesh) -> Mesh {
    let topology = topology(mesh);
    let count = mesh.positions.len();
    let valence = |vertex: usize| topology.neighbours.get(vertex).map_or(0, BTreeSet::len);
    let on_edge = |vertex: usize| topology.boundary.get(vertex).copied().unwrap_or(false);
    let regular: Vec<bool> = (0..count).map(|vertex| valence(vertex) == if on_edge(vertex) { 3 } else { 4 }).collect();
    let is_regular = |vertex: u32| regular.get(vertex as usize).copied().unwrap_or(false);
    // Colour each group of connected regular vertices from a start next to an irregular vertex
    // inside the surface, which goes (so that vertex stays apart from the ones that go), else
    // from its first vertex, which stays.
    let mut remove: Vec<Option<bool>> = vec![None; count];
    let starts = (0..count as u32)
        .filter(|vertex| {
            is_regular(*vertex)
                && topology
                    .neighbours
                    .get(*vertex as usize)
                    .is_some_and(|near| near.iter().any(|other| !is_regular(*other) && !on_edge(*other as usize)))
        })
        .map(|vertex| (vertex, true))
        .chain((0..count as u32).filter(|vertex| is_regular(*vertex)).map(|vertex| (vertex, false)));
    for (start, goes) in starts.collect::<Vec<_>>() {
        if remove.get(start as usize).copied().flatten().is_some() {
            continue;
        }
        if let Some(slot) = remove.get_mut(start as usize) {
            *slot = Some(goes);
        }
        let mut queue = VecDeque::from([start]);
        while let Some(vertex) = queue.pop_front() {
            let colour = remove.get(vertex as usize).copied().flatten().unwrap_or(false);
            for other in topology.neighbours.get(vertex as usize).into_iter().flatten() {
                if is_regular(*other)
                    && let Some(slot) = remove.get_mut(*other as usize)
                    && slot.is_none()
                {
                    *slot = Some(!colour);
                    queue.push_back(*other);
                }
            }
        }
    }
    // The vertices that go: no two next to each other, each with one fan of neighbours.
    let mut going = vec![false; count];
    let mut fans: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for vertex in 0..count as u32 {
        if remove.get(vertex as usize).copied().flatten() != Some(true) {
            continue;
        }
        if topology.neighbours.get(vertex as usize).is_some_and(|near| near.iter().any(|other| going.get(*other as usize).copied().unwrap_or(false)))
        {
            continue;
        }
        let Some(ring) = fan(mesh, &topology, vertex) else { continue };
        if ring.len() < 3 {
            continue;
        }
        if let Some(flag) = going.get_mut(vertex as usize) {
            *flag = true;
        }
        fans.insert(vertex, ring);
    }
    if fans.is_empty() {
        return mesh.clone();
    }
    let gone = |vertex: &u32| going.get(*vertex as usize).copied().unwrap_or(false);
    let mut out = Mesh {
        positions: mesh.positions.clone(),
        smooth: mesh.smooth,
        smooth_angle: mesh.smooth_angle,
        loose_edges: mesh.loose_edges.clone(),
        ..Default::default()
    };
    out.loose_edges.retain(|[a, b]| !gone(a) && !gone(b));
    for face in &mesh.faces {
        let kept: Vec<u32> = face.iter().copied().filter(|vertex| !gone(vertex)).collect();
        if kept.len() == face.len() || kept.len() >= 3 {
            out.faces.push(kept);
        }
    }
    out.faces.extend(fans.into_values());
    out.remove_unused_vertices();
    if out.validate().is_err() {
        return mesh.clone();
    }
    out
}

/// Un-subdivide, `iterations` half steps.
pub fn unsubdivide(mesh: &Mesh, iterations: u32) -> Mesh {
    let mut out = mesh.clone();
    for _ in 0..iterations {
        let next = unsubdivide_once(&out);
        if next.positions.len() == out.positions.len() {
            break;
        }
        out = next;
    }
    out
}

#[derive(PartialEq)]
struct Join {
    angle: f32,
    edge: (u32, u32),
}

impl Eq for Join {}

impl PartialOrd for Join {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Join {
    fn cmp(&self, other: &Self) -> Ordering {
        // A min-heap on the angle.
        other.angle.total_cmp(&self.angle).then_with(|| other.edge.cmp(&self.edge))
    }
}

fn find(parent: &mut [usize], item: usize) -> usize {
    let mut root = item;
    while let Some(&up) = parent.get(root) {
        if up == root {
            break;
        }
        root = up;
    }
    let mut at = item;
    while let Some(&up) = parent.get(at) {
        if up == root {
            break;
        }
        if let Some(slot) = parent.get_mut(at) {
            *slot = root;
        }
        at = up;
    }
    root
}

/// The outline of a set of faces as one loop of vertices, when it is one (a disc).
fn outline(mesh: &Mesh, faces: &[usize]) -> Option<Vec<u32>> {
    let mut directed: BTreeSet<(u32, u32)> = BTreeSet::new();
    for face in faces {
        for (a, b) in face_edges(mesh.faces.get(*face)?) {
            if !directed.remove(&(b, a)) {
                directed.insert((a, b));
            }
        }
    }
    let mut next: BTreeMap<u32, u32> = BTreeMap::new();
    for (a, b) in &directed {
        if next.insert(*a, *b).is_some() {
            return None;
        }
    }
    let (&start, _) = next.iter().next()?;
    let mut ring = vec![start];
    let mut at = start;
    loop {
        let to = *next.get(&at)?;
        if to == start {
            break;
        }
        if ring.len() > next.len() {
            return None;
        }
        ring.push(to);
        at = to;
    }
    (ring.len() == next.len()).then_some(ring)
}

fn angle_between(a: Vec3, b: Vec3) -> f32 {
    a.normalize_or_zero().dot(b.normalize_or_zero()).clamp(-1.0, 1.0).acos()
}

/// Planar decimation: joins faces that meet at less than `angle_limit` (radians), then drops
/// vertices between two edges straighter than it.
pub fn planar(mesh: &Mesh, angle_limit: f32) -> Mesh {
    if mesh.faces.is_empty() {
        return mesh.clone();
    }
    let face_count = mesh.faces.len();
    let mut parent: Vec<usize> = (0..face_count).collect();
    // Area-weighted normals per region (the sum of Newell vectors) and the faces in each.
    let newell = |face: &[u32]| {
        let points = mesh.face_positions(face);
        let mut sum = Vec3::ZERO;
        for (index, point) in points.iter().enumerate() {
            let next = points.get((index + 1) % points.len()).copied().unwrap_or(*point);
            sum += point.cross(next);
        }
        sum * 0.5
    };
    let mut normal: Vec<Vec3> = mesh.faces.iter().map(|face| newell(face)).collect();
    let mut members: Vec<Vec<usize>> = (0..face_count).map(|face| vec![face]).collect();
    let mut faces_of_edge: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (index, face) in mesh.faces.iter().enumerate() {
        for (a, b) in face_edges(face) {
            faces_of_edge.entry(key(a, b)).or_default().push(index);
        }
    }
    let mut heap = BinaryHeap::new();
    for (edge, faces) in &faces_of_edge {
        if let [a, b] = faces.as_slice() {
            let angle = angle_between(normal.get(*a).copied().unwrap_or(Vec3::Z), normal.get(*b).copied().unwrap_or(Vec3::Z));
            if angle < angle_limit {
                heap.push(Join { angle, edge: *edge });
            }
        }
    }
    while let Some(Join { angle, edge }) = heap.pop() {
        let Some([a, b]) = faces_of_edge.get(&edge).and_then(|faces| <[usize; 2]>::try_from(faces.as_slice()).ok()) else { continue };
        let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
        if ra == rb {
            continue;
        }
        let (na, nb) = (normal.get(ra).copied().unwrap_or(Vec3::Z), normal.get(rb).copied().unwrap_or(Vec3::Z));
        let now = angle_between(na, nb);
        if now >= angle_limit {
            continue;
        }
        if now > angle + 1e-6 {
            // The regions changed since; try again in turn.
            heap.push(Join { angle: now, edge });
            continue;
        }
        let mut joined: Vec<usize> = members.get(ra).cloned().unwrap_or_default();
        joined.extend(members.get(rb).cloned().unwrap_or_default());
        if outline(mesh, &joined).is_none() {
            continue;
        }
        if let Some(slot) = parent.get_mut(rb) {
            *slot = ra;
        }
        if let Some(slot) = normal.get_mut(ra) {
            *slot = na + nb;
        }
        if let Some(slot) = members.get_mut(ra) {
            *slot = joined;
        }
        if let Some(slot) = members.get_mut(rb) {
            slot.clear();
        }
    }
    let mut out = Mesh {
        positions: mesh.positions.clone(),
        smooth: mesh.smooth,
        smooth_angle: mesh.smooth_angle,
        loose_edges: mesh.loose_edges.clone(),
        ..Default::default()
    };
    let mut done = BTreeSet::new();
    for face in 0..face_count {
        let root = find(&mut parent, face);
        if !done.insert(root) {
            continue;
        }
        let group = members.get(root).cloned().unwrap_or_default();
        match outline(mesh, &group) {
            Some(ring) if group.len() > 1 => out.faces.push(ring),
            _ => out.faces.extend(group.iter().filter_map(|index| mesh.faces.get(*index).cloned())),
        }
    }
    dissolve_straight_vertices(&mut out, angle_limit);
    out.remove_unused_vertices();
    if out.validate().is_err() {
        return mesh.clone();
    }
    out
}

/// Drops vertices that only join two edges in a nearly straight line from every face they are
/// on, where each face keeps three corners.
fn dissolve_straight_vertices(mesh: &mut Mesh, angle_limit: f32) {
    let topology = topology(mesh);
    let loose: BTreeSet<u32> = mesh.loose_edges.iter().flatten().copied().collect();
    let mut drop = BTreeSet::new();
    for vertex in 0..mesh.positions.len() as u32 {
        let Some(near) = topology.neighbours.get(vertex as usize) else { continue };
        if near.len() != 2 || loose.contains(&vertex) {
            continue;
        }
        let (Some(&a), Some(&b)) = (near.iter().next(), near.iter().nth(1)) else { continue };
        let (Some(&pa), Some(&pv), Some(&pb)) = (mesh.positions.get(a as usize), mesh.positions.get(vertex as usize), mesh.positions.get(b as usize))
        else {
            continue;
        };
        let bend = std::f32::consts::PI - angle_between(pa - pv, pb - pv);
        if bend < angle_limit {
            drop.insert(vertex);
        }
    }
    if drop.is_empty() {
        return;
    }
    for face in &mut mesh.faces {
        let kept: Vec<u32> = face.iter().copied().filter(|vertex| !drop.contains(vertex)).collect();
        if kept.len() >= 3 {
            *face = kept;
        }
    }
}
