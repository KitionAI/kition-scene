//! More Edit Mode operations: extruding vertices and edges, extruding faces one by one,
//! dissolving, bridging edge loops, bisecting, symmetrizing, spinning, bevelling vertices, and
//! splitting a selection off. Counts follow the reference application (`tests/ops2.rs`).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use glam::{Mat3, Vec3};

use crate::error::{Result, SceneError};
use crate::mesh::{Mesh, face_edges};

const EPSILON: f32 = 1e-5;

fn index(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn key(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}

/// Edges with both ends in `vertices`.
pub fn selected_edges(mesh: &Mesh, vertices: &BTreeSet<u32>) -> Vec<(u32, u32)> {
    mesh.edges().into_iter().filter(|[a, b]| vertices.contains(a) && vertices.contains(b)).map(|[a, b]| (a, b)).collect()
}

/// Drops loose edges that faces now use, and repeats.
fn tidy_loose_edges(mesh: &mut Mesh) {
    let mut in_faces: BTreeSet<(u32, u32)> = BTreeSet::new();
    for face in &mesh.faces {
        for (a, b) in face_edges(face) {
            in_faces.insert(key(a, b));
        }
    }
    let mut seen = BTreeSet::new();
    mesh.loose_edges.retain(|[a, b]| !in_faces.contains(&key(*a, *b)) && seen.insert(key(*a, *b)));
}

/// The direction an existing face runs an edge in, if any face uses it.
fn face_direction(mesh: &Mesh, a: u32, b: u32) -> Option<bool> {
    mesh.faces.iter().find_map(|face| {
        face_edges(face).find_map(|(x, y)| {
            if (x, y) == (a, b) {
                Some(true)
            } else if (x, y) == (b, a) {
                Some(false)
            } else {
                None
            }
        })
    })
}

/// Extrudes selected vertices and edges that are not part of selected faces: each edge pulls
/// out a quad, each lone vertex an edge, all moved by `offset`. Returns the new vertices.
fn extrude_loose_raw(mesh: &mut Mesh, vertices: &BTreeSet<u32>, offset: Vec3) -> Result<BTreeSet<u32>> {
    if !offset.is_finite() {
        return Err(SceneError::params("the offset must be numbers"));
    }
    if vertices.is_empty() {
        return Err(SceneError::params("nothing is selected"));
    }
    let edges = selected_edges(mesh, vertices);
    let mut copies: BTreeMap<u32, u32> = BTreeMap::new();
    for vertex in vertices {
        let point = mesh.position(*vertex)?;
        copies.insert(*vertex, mesh.vertex_count());
        mesh.positions.push(point + offset);
    }
    let copy = |vertex: u32| copies.get(&vertex).copied().unwrap_or(vertex);
    let mut on_edge = BTreeSet::new();
    for (a, b) in edges {
        on_edge.insert(a);
        on_edge.insert(b);
        // The new face runs the edge against the face already on it.
        let face = if face_direction(mesh, a, b) == Some(true) { vec![b, a, copy(a), copy(b)] } else { vec![a, b, copy(b), copy(a)] };
        mesh.faces.push(face);
    }
    for vertex in vertices.iter().filter(|vertex| !on_edge.contains(vertex)) {
        mesh.loose_edges.push([*vertex, copy(*vertex)]);
    }
    tidy_loose_edges(mesh);
    Ok(copies.values().copied().collect())
}

/// Extrudes each face on its own along its normal. Returns the moved faces' vertices.
fn extrude_individual_raw(mesh: &mut Mesh, faces: &[u32], distance: f32) -> Result<BTreeSet<u32>> {
    let mut seen = BTreeSet::new();
    for face in faces {
        if *face as usize >= mesh.faces.len() {
            return Err(SceneError::not_found(format!("face {face}")));
        }
        if seen.insert(*face) {
            crate::ops::extrude_region(mesh, &[*face], distance, Vec3::ZERO)?;
        }
    }
    Ok(seen.iter().filter_map(|face| mesh.faces.get(*face as usize)).flatten().copied().collect())
}

/// How many distinct neighbours a vertex has along edges.
fn degree(mesh: &Mesh, vertex: u32) -> usize {
    let mut near = BTreeSet::new();
    for face in &mesh.faces {
        for (a, b) in face_edges(face) {
            if a == vertex {
                near.insert(b);
            } else if b == vertex {
                near.insert(a);
            }
        }
    }
    for [a, b] in &mesh.loose_edges {
        if *a == vertex {
            near.insert(*b);
        } else if *b == vertex {
            near.insert(*a);
        }
    }
    near.len()
}

/// Takes a vertex out of every face (faces left with fewer than three corners go).
fn remove_from_faces(mesh: &mut Mesh, vertex: u32) {
    for face in &mut mesh.faces {
        face.retain(|corner| *corner != vertex);
    }
    mesh.faces.retain(|face| face.len() >= 3);
    mesh.loose_edges.retain(|edge| !edge.contains(&vertex));
}

/// Merges the faces round each vertex into one face without it (a vertex between two edges
/// just leaves the faces it was in). Vertices on an open outline are left alone.
fn dissolve_vertices_raw(mesh: &mut Mesh, vertices: &BTreeSet<u32>) -> Result<()> {
    for vertex in vertices {
        if *vertex as usize >= mesh.positions.len() {
            return Err(SceneError::not_found(format!("vertex {vertex}")));
        }
    }
    for vertex in vertices {
        if degree(mesh, *vertex) == 2 {
            remove_from_faces(mesh, *vertex);
            continue;
        }
        let around: Vec<usize> = mesh.faces.iter().enumerate().filter(|(_, face)| face.contains(vertex)).map(|(at, _)| at).collect();
        // Each face as its corners after the vertex: [x1, ..., xn].
        let mut fans: Vec<Vec<u32>> = Vec::new();
        for at in &around {
            let Some(face) = mesh.faces.get(*at) else { continue };
            let Some(start) = face.iter().position(|corner| corner == vertex) else { continue };
            let count = face.len();
            fans.push((1..count).filter_map(|step| face.get((start + step) % count).copied()).collect());
        }
        let Some(first) = fans.first().cloned() else { continue };
        let Some(&opening) = first.first() else { continue };
        let mut polygon = first;
        let mut used = vec![false; fans.len()];
        if let Some(flag) = used.first_mut() {
            *flag = true;
        }
        let mut closed = false;
        for _ in 0..fans.len() {
            let Some(&last) = polygon.last() else { break };
            if fans.len() > 1 && last == opening && used.iter().all(|flag| *flag) {
                closed = true;
                break;
            }
            let Some(next) =
                fans.iter().enumerate().find(|(at, fan)| !used.get(*at).copied().unwrap_or(true) && fan.first() == Some(&last)).map(|(at, _)| at)
            else {
                break;
            };
            if let Some(flag) = used.get_mut(next) {
                *flag = true;
            }
            if let Some(fan) = fans.get(next) {
                polygon.extend(fan.iter().skip(1));
            }
        }
        if !closed && polygon.last() == Some(&opening) && used.iter().all(|flag| *flag) {
            closed = true;
        }
        if !closed {
            continue;
        }
        polygon.pop();
        let distinct: BTreeSet<u32> = polygon.iter().copied().collect();
        if polygon.len() < 3 || distinct.len() != polygon.len() {
            continue;
        }
        let mut index_set: BTreeSet<usize> = around.into_iter().collect();
        let mut kept = Vec::with_capacity(mesh.faces.len());
        for (at, face) in std::mem::take(&mut mesh.faces).into_iter().enumerate() {
            if index_set.remove(&at) {
                continue;
            }
            kept.push(face);
        }
        kept.push(polygon);
        mesh.faces = kept;
    }
    mesh.remove_unused_vertices();
    Ok(())
}

/// Merges the two faces on each edge into one, then takes out vertices left between only two
/// edges.
fn dissolve_edges_raw(mesh: &mut Mesh, edges: &[(u32, u32)]) -> Result<()> {
    let mut ends = BTreeSet::new();
    for (a, b) in edges {
        let faces: Vec<usize> =
            mesh.faces.iter().enumerate().filter(|(_, face)| face_edges(face).any(|(x, y)| key(x, y) == key(*a, *b))).map(|(at, _)| at).collect();
        let [first, second] = faces.as_slice() else { continue };
        let (Some(one), Some(two)) = (mesh.faces.get(*first).cloned(), mesh.faces.get(*second).cloned()) else { continue };
        // `one` runs a→b; start it at b (it ends at a), and `two` at a (it ends at b).
        let (a, b) = if face_edges(&one).any(|edge| edge == (*a, *b)) { (*a, *b) } else { (*b, *a) };
        let rotate = |face: &[u32], start: u32| -> Option<Vec<u32>> {
            let at = face.iter().position(|corner| *corner == start)?;
            Some(face.iter().cycle().skip(at).take(face.len()).copied().collect())
        };
        let (Some(from_b), Some(from_a)) = (rotate(&one, b), rotate(&two, a)) else { continue };
        let mut merged = from_b;
        merged.extend(from_a.iter().skip(1).take(from_a.len().saturating_sub(2)));
        let distinct: BTreeSet<u32> = merged.iter().copied().collect();
        if distinct.len() != merged.len() {
            continue;
        }
        let (low, high) = ((*first).min(*second), (*first).max(*second));
        mesh.faces.remove(high);
        if let Some(slot) = mesh.faces.get_mut(low) {
            *slot = merged;
        }
        ends.insert(a);
        ends.insert(b);
    }
    for vertex in ends {
        if degree(mesh, vertex) == 2 {
            remove_from_faces(mesh, vertex);
        }
    }
    mesh.remove_unused_vertices();
    Ok(())
}

/// Merges each connected group of faces whose outline is one loop into a single face. Returns
/// the indices of the merged faces.
fn dissolve_faces_raw(mesh: &mut Mesh, faces: &[u32]) -> Result<Vec<u32>> {
    let selected: BTreeSet<usize> = faces.iter().map(|face| *face as usize).collect();
    if let Some(bad) = selected.iter().find(|face| **face >= mesh.faces.len()) {
        return Err(SceneError::not_found(format!("face {bad}")));
    }
    // Group by shared edges.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut left: BTreeSet<usize> = selected;
    while let Some(&start) = left.iter().next() {
        left.remove(&start);
        let mut group = vec![start];
        let mut cursor = 0;
        while let Some(&current) = group.get(cursor) {
            cursor += 1;
            let edges: BTreeSet<(u32, u32)> =
                mesh.faces.get(current).map(|face| face_edges(face).map(|(a, b)| key(a, b)).collect()).unwrap_or_default();
            let near: Vec<usize> = left
                .iter()
                .copied()
                .filter(|other| mesh.faces.get(*other).is_some_and(|face| face_edges(face).any(|(a, b)| edges.contains(&key(a, b)))))
                .collect();
            for other in near {
                left.remove(&other);
                group.push(other);
            }
        }
        groups.push(group);
    }
    let mut replaced: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
    let mut dropped: BTreeSet<usize> = BTreeSet::new();
    for group in groups {
        if group.len() < 2 {
            continue;
        }
        let mut uses: HashMap<(u32, u32), usize> = HashMap::new();
        for face in group.iter().filter_map(|at| mesh.faces.get(*at)) {
            for (a, b) in face_edges(face) {
                *uses.entry(key(a, b)).or_default() += 1;
            }
        }
        let mut next: HashMap<u32, u32> = HashMap::new();
        let mut ok = true;
        for face in group.iter().filter_map(|at| mesh.faces.get(*at)) {
            for (a, b) in face_edges(face) {
                if uses.get(&key(a, b)) == Some(&1) && next.insert(a, b).is_some() {
                    ok = false;
                }
            }
        }
        let Some(&start) = next.keys().min() else { continue };
        let mut polygon = vec![start];
        let mut current = start;
        while ok {
            let Some(&following) = next.get(&current) else {
                ok = false;
                break;
            };
            if following == start {
                break;
            }
            polygon.push(following);
            current = following;
            if polygon.len() > next.len() {
                ok = false;
            }
        }
        if !ok || polygon.len() != next.len() || polygon.len() < 3 {
            continue;
        }
        let Some(&keep) = group.iter().min() else { continue };
        replaced.insert(keep, polygon);
        dropped.extend(group.into_iter().filter(|at| *at != keep));
    }
    let mut out = Vec::new();
    let mut kept = Vec::with_capacity(mesh.faces.len());
    for (at, face) in std::mem::take(&mut mesh.faces).into_iter().enumerate() {
        if dropped.contains(&at) {
            continue;
        }
        if let Some(polygon) = replaced.remove(&at) {
            out.push(index(kept.len()));
            kept.push(polygon);
        } else {
            kept.push(face);
        }
    }
    mesh.faces = kept;
    mesh.remove_unused_vertices();
    Ok(out)
}

/// The selected edges' vertices as ordered chains (paths or loops), one per connected piece.
fn chains(mesh: &Mesh, vertices: &BTreeSet<u32>) -> Vec<(Vec<u32>, bool)> {
    let edges = selected_edges(mesh, vertices);
    let mut near: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (a, b) in &edges {
        near.entry(*a).or_default().push(*b);
        near.entry(*b).or_default().push(*a);
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    // Paths start at their ends; loops anywhere.
    let starts: Vec<u32> = near.iter().filter(|(_, list)| list.len() == 1).map(|(vertex, _)| *vertex).chain(near.keys().copied()).collect();
    for start in starts {
        if seen.contains(&start) {
            continue;
        }
        let mut chain = vec![start];
        seen.insert(start);
        let mut current = start;
        while let Some(next) = near.get(&current).and_then(|list| list.iter().find(|other| !seen.contains(other)).copied()) {
            seen.insert(next);
            chain.push(next);
            current = next;
        }
        let closed = chain.len() > 2 && near.get(&current).is_some_and(|list| list.contains(&start));
        out.push((chain, closed));
    }
    out
}

/// Joins two selected edge loops (or paths) of the same length with a strip of quads. Returns
/// the new faces.
fn bridge_raw(mesh: &mut Mesh, vertices: &BTreeSet<u32>) -> Result<Vec<u32>> {
    let pieces = chains(mesh, vertices);
    let [(first, closed_a), (second, closed_b)] = pieces.as_slice() else {
        return Err(SceneError::params("select two edge loops to bridge"));
    };
    if first.len() != second.len() || closed_a != closed_b || first.len() < 2 {
        return Err(SceneError::params("the loops must have the same number of vertices"));
    }
    let closed = *closed_a;
    let count = first.len();
    let point = |vertex: u32| mesh.positions.get(vertex as usize).copied().unwrap_or(Vec3::ZERO);
    // Line the second loop up with the first: the turn and direction with the shortest rungs.
    let mut best: Option<(f32, Vec<u32>)> = None;
    let shifts = if closed { count } else { 1 };
    for reversed in [false, true] {
        for shift in 0..shifts {
            let candidate: Vec<u32> = (0..count)
                .filter_map(|slot| {
                    let at = match (closed, reversed) {
                        (true, false) => (shift + slot) % count,
                        (true, true) => (count + shift - slot) % count,
                        (false, false) => slot,
                        (false, true) => count - 1 - slot,
                    };
                    second.get(at).copied()
                })
                .collect();
            let cost: f32 = first.iter().zip(&candidate).map(|(a, b)| point(*a).distance(point(*b))).sum();
            if best.as_ref().is_none_or(|(known, _)| cost < *known) {
                best = Some((cost, candidate));
            }
        }
    }
    let Some((_, second)) = best else { return Err(SceneError::params("the loops cannot be lined up")) };
    let mut made = Vec::new();
    let rungs = if closed { count } else { count - 1 };
    for slot in 0..rungs {
        let next = (slot + 1) % count;
        let (Some(&a0), Some(&a1), Some(&b0), Some(&b1)) = (first.get(slot), first.get(next), second.get(slot), second.get(next)) else { continue };
        let mut quad = vec![a0, a1, b1, b0];
        // Run each rail against the face already on it.
        if face_direction(mesh, a0, a1) == Some(true) || face_direction(mesh, b1, b0) == Some(true) {
            quad.reverse();
        }
        made.push(index(mesh.faces.len()));
        mesh.faces.push(quad);
    }
    tidy_loose_edges(mesh);
    Ok(made)
}

/// Cuts the mesh with a plane, optionally deleting what lies behind it (`clear_inner`) or in
/// front (`clear_outer`) and filling the cut. Returns the vertices on the cut.
fn bisect_raw(mesh: &mut Mesh, point: Vec3, normal: Vec3, clear_inner: bool, clear_outer: bool, fill: bool) -> Result<BTreeSet<u32>> {
    let normal = normal.normalize_or_zero();
    if !point.is_finite() || normal == Vec3::ZERO {
        return Err(SceneError::params("the plane needs a point and a normal"));
    }
    let side = |p: Vec3| (p - point).dot(normal);
    let mut cuts: HashMap<(u32, u32), u32> = HashMap::new();
    let mut faces = Vec::with_capacity(mesh.faces.len() + 8);
    for face in std::mem::take(&mut mesh.faces) {
        let distances: Vec<f32> = face.iter().map(|vertex| side(mesh.positions.get(*vertex as usize).copied().unwrap_or(Vec3::ZERO))).collect();
        let above = distances.iter().any(|d| *d > EPSILON);
        let below = distances.iter().any(|d| *d < -EPSILON);
        if !(above && below) {
            faces.push(face);
            continue;
        }
        let mut front = Vec::new();
        let mut back = Vec::new();
        let count = face.len();
        for slot in 0..count {
            let (Some(&a), Some(&b), Some(&da), Some(&db)) =
                (face.get(slot), face.get((slot + 1) % count), distances.get(slot), distances.get((slot + 1) % count))
            else {
                continue;
            };
            if da >= -EPSILON {
                front.push(a);
            }
            if da <= EPSILON {
                back.push(a);
            }
            if (da > EPSILON && db < -EPSILON) || (da < -EPSILON && db > EPSILON) {
                let made = *cuts.entry(key(a, b)).or_insert_with(|| {
                    let (pa, pb) = (
                        mesh.positions.get(a as usize).copied().unwrap_or(Vec3::ZERO),
                        mesh.positions.get(b as usize).copied().unwrap_or(Vec3::ZERO),
                    );
                    mesh.positions.push(pa.lerp(pb, da / (da - db)));
                    index(mesh.positions.len() - 1)
                });
                front.push(made);
                back.push(made);
            }
        }
        for piece in [front, back] {
            if piece.len() >= 3 {
                faces.push(piece);
            }
        }
    }
    mesh.faces = faces;
    // Loose edges are cut too.
    let loose = std::mem::take(&mut mesh.loose_edges);
    for [a, b] in loose {
        let (pa, pb) = (mesh.position(a)?, mesh.position(b)?);
        let (da, db) = (side(pa), side(pb));
        if (da > EPSILON && db < -EPSILON) || (da < -EPSILON && db > EPSILON) {
            let made = *cuts.entry(key(a, b)).or_insert_with(|| {
                mesh.positions.push(pa.lerp(pb, da / (da - db)));
                index(mesh.positions.len() - 1)
            });
            mesh.loose_edges.push([a, made]);
            mesh.loose_edges.push([made, b]);
        } else {
            mesh.loose_edges.push([a, b]);
        }
    }
    let distances: Vec<f32> = mesh.positions.iter().map(|p| side(*p)).collect();
    let distance = |vertex: u32| distances.get(vertex as usize).copied().unwrap_or(0.0);
    let gone = |vertices: &[u32]| -> bool {
        let values: Vec<f32> = vertices.iter().map(|vertex| distance(*vertex)).collect();
        (clear_inner && values.iter().all(|d| *d <= EPSILON) && values.iter().any(|d| *d < -EPSILON))
            || (clear_outer && values.iter().all(|d| *d >= -EPSILON) && values.iter().any(|d| *d > EPSILON))
    };
    let faces_now = std::mem::take(&mut mesh.faces);
    mesh.faces = faces_now.into_iter().filter(|face| !gone(face)).collect();
    let loose_now = std::mem::take(&mut mesh.loose_edges);
    mesh.loose_edges = loose_now.into_iter().filter(|edge| !gone(edge)).collect();
    if fill && (clear_inner || clear_outer) {
        // The open outline on the plane, run backwards, closes the cut.
        let mut uses: HashMap<(u32, u32), usize> = HashMap::new();
        for face in &mesh.faces {
            for (a, b) in face_edges(face) {
                *uses.entry(key(a, b)).or_default() += 1;
            }
        }
        let mut next: BTreeMap<u32, u32> = BTreeMap::new();
        for face in &mesh.faces {
            for (a, b) in face_edges(face) {
                if uses.get(&key(a, b)) == Some(&1) && distance(a).abs() <= EPSILON && distance(b).abs() <= EPSILON {
                    next.insert(b, a);
                }
            }
        }
        let mut seen = BTreeSet::new();
        let starts: Vec<u32> = next.keys().copied().collect();
        for start in starts {
            if seen.contains(&start) {
                continue;
            }
            let mut ring = vec![start];
            seen.insert(start);
            let mut current = start;
            let mut closed = false;
            while let Some(&following) = next.get(&current) {
                if following == start {
                    closed = true;
                    break;
                }
                if !seen.insert(following) {
                    break;
                }
                ring.push(following);
                current = following;
            }
            if closed && ring.len() >= 3 {
                mesh.faces.push(ring);
            }
        }
    }
    let cut: Vec<bool> = distances.iter().map(|d| d.abs() <= EPSILON).collect();
    let mut keep_marks = vec![false; mesh.positions.len()];
    for vertex in mesh.faces.iter().flatten().chain(mesh.loose_edges.iter().flatten()) {
        if let Some(flag) = keep_marks.get_mut(*vertex as usize) {
            *flag = true;
        }
    }
    mesh.remove_unused_vertices();
    tidy_loose_edges(mesh);
    // The cut, numbered as the vertices are after unused ones went.
    let mut out = BTreeSet::new();
    let mut next_index = 0u32;
    for (at, used) in keep_marks.iter().enumerate() {
        if *used {
            if cut.get(at).copied().unwrap_or(false) {
                out.insert(next_index);
            }
            next_index += 1;
        }
    }
    Ok(out)
}

/// Makes the mesh symmetric across a local axis plane through the origin: the side the
/// direction comes from replaces the other side, joined on the plane.
fn symmetrize_raw(mesh: &mut Mesh, axis: usize, positive_to_negative: bool) -> Result<()> {
    let mut normal = Vec3::ZERO;
    match axis {
        0 => normal.x = 1.0,
        1 => normal.y = 1.0,
        2 => normal.z = 1.0,
        _ => return Err(SceneError::params("the axis must be x, y, or z")),
    }
    let normal = if positive_to_negative { normal } else { -normal };
    bisect(mesh, Vec3::ZERO, normal, true, false, false)?;
    let flip = Vec3::ONE - normal.abs() * 2.0;
    let mut copy = mesh.clone();
    for point in &mut copy.positions {
        *point *= flip;
    }
    copy.flip_all();
    let original = mesh.vertex_count();
    let offset = mesh.append(&copy);
    let mut remap: Vec<u32> = (0..mesh.vertex_count()).collect();
    for vertex in 0..original {
        let on_plane = mesh.positions.get(vertex as usize).is_some_and(|point| point.dot(normal).abs() <= EPSILON);
        if on_plane && let Some(slot) = remap.get_mut((vertex + offset) as usize) {
            *slot = vertex;
        }
    }
    mesh.reindex(&remap);
    mesh.remove_unused_vertices();
    tidy_loose_edges(mesh);
    Ok(())
}

/// Sweeps the selected edges (and lone vertices) round an axis through `center` in `steps`
/// steps, making faces (and edges); a whole turn joins the end to the start. Turns the way the
/// reference application does: clockwise looking down the axis. Returns the last ring.
fn spin_raw(mesh: &mut Mesh, vertices: &BTreeSet<u32>, steps: u32, angle: f32, axis: Vec3, center: Vec3) -> Result<BTreeSet<u32>> {
    if !(1..=1024).contains(&steps) {
        return Err(SceneError::params("steps must be between 1 and 1024"));
    }
    let axis = axis.normalize_or_zero();
    if axis == Vec3::ZERO || !angle.is_finite() || !center.is_finite() {
        return Err(SceneError::params("spin needs an axis, an angle, and a centre"));
    }
    if vertices.is_empty() {
        return Err(SceneError::params("nothing is selected"));
    }
    if (vertices.len() as u64).saturating_mul(u64::from(steps)) > 2_000_000 {
        return Err(SceneError::params("the spin would make too many vertices"));
    }
    let edges = selected_edges(mesh, vertices);
    let loose_before = edges.iter().all(|(a, b)| face_direction(mesh, *a, *b).is_none());
    let whole = (angle.abs() - std::f32::consts::TAU).abs() < 1e-4;
    let order: Vec<u32> = vertices.iter().copied().collect();
    let mut rings: Vec<BTreeMap<u32, u32>> = vec![order.iter().map(|vertex| (*vertex, *vertex)).collect()];
    let made_steps = if whole { steps - 1 } else { steps };
    for step in 1..=made_steps {
        let turn = Mat3::from_axis_angle(axis, -angle * step as f32 / steps as f32);
        let mut ring = BTreeMap::new();
        for vertex in &order {
            let point = mesh.position(*vertex)?;
            ring.insert(*vertex, mesh.vertex_count());
            mesh.positions.push(center + turn * (point - center));
        }
        rings.push(ring);
    }
    if whole {
        rings.push(rings.first().cloned().unwrap_or_default());
    }
    let on_edge: BTreeSet<u32> = edges.iter().flat_map(|(a, b)| [*a, *b]).collect();
    for pair in rings.windows(2) {
        let [from, to] = pair else { continue };
        let at = |ring: &BTreeMap<u32, u32>, vertex: u32| ring.get(&vertex).copied().unwrap_or(vertex);
        for (a, b) in &edges {
            mesh.faces.push(vec![at(from, *a), at(from, *b), at(to, *b), at(to, *a)]);
        }
        for vertex in order.iter().filter(|vertex| !on_edge.contains(vertex)) {
            mesh.loose_edges.push([at(from, *vertex), at(to, *vertex)]);
        }
    }
    if loose_before && !edges.is_empty() {
        crate::ops::recalculate_outside(mesh);
    }
    tidy_loose_edges(mesh);
    Ok(rings.last().map(|ring| ring.values().copied().collect()).unwrap_or_default())
}

/// Cuts each selected vertex off with a face through points `width` along its edges. Vertices
/// on an open outline are left alone. Returns the new faces.
fn bevel_vertices_raw(mesh: &mut Mesh, vertices: &BTreeSet<u32>, width: f32) -> Result<Vec<u32>> {
    if !(width.is_finite() && width > 0.0) {
        return Err(SceneError::params("width must be a positive number"));
    }
    let mut uses: HashMap<(u32, u32), usize> = HashMap::new();
    for face in &mesh.faces {
        for (a, b) in face_edges(face) {
            *uses.entry(key(a, b)).or_default() += 1;
        }
    }
    // Only vertices whose every edge has a face on each side.
    let chosen: BTreeSet<u32> = vertices
        .iter()
        .copied()
        .filter(|vertex| {
            let edges: Vec<usize> = uses.iter().filter(|((a, b), _)| a == vertex || b == vertex).map(|(_, count)| *count).collect();
            !edges.is_empty() && edges.iter().all(|count| *count == 2)
        })
        .collect();
    if chosen.is_empty() {
        return Err(SceneError::params("select vertices inside the surface to bevel"));
    }
    let mut points: HashMap<(u32, u32), u32> = HashMap::new();
    let mut point_at = |mesh: &mut Mesh, vertex: u32, toward: u32| -> u32 {
        *points.entry((vertex, toward)).or_insert_with(|| {
            let (p, q) = (
                mesh.positions.get(vertex as usize).copied().unwrap_or(Vec3::ZERO),
                mesh.positions.get(toward as usize).copied().unwrap_or(Vec3::ZERO),
            );
            let length = p.distance(q);
            let limit = if chosen.contains(&toward) { length * 0.5 } else { length * 0.999 };
            mesh.positions.push(p + (q - p).normalize_or_zero() * width.min(limit));
            index(mesh.positions.len() - 1)
        })
    };
    let mut cap_edges: BTreeMap<u32, BTreeMap<u32, u32>> = BTreeMap::new();
    let old = std::mem::take(&mut mesh.faces);
    let mut faces = Vec::with_capacity(old.len() + chosen.len());
    for face in old {
        let count = face.len();
        let mut corners = Vec::with_capacity(count + 4);
        for slot in 0..count {
            let (Some(&previous), Some(&current), Some(&next)) = (face.get((slot + count - 1) % count), face.get(slot), face.get((slot + 1) % count))
            else {
                continue;
            };
            if chosen.contains(&current) {
                let incoming = point_at(mesh, current, previous);
                let outgoing = point_at(mesh, current, next);
                corners.push(incoming);
                corners.push(outgoing);
                // The face runs incoming→outgoing, so the cap runs outgoing→incoming.
                cap_edges.entry(current).or_default().insert(outgoing, incoming);
            } else {
                corners.push(current);
            }
        }
        faces.push(corners);
    }
    mesh.faces = faces;
    let mut made = Vec::new();
    for edges in cap_edges.values() {
        let Some((&start, _)) = edges.iter().next() else { continue };
        let mut polygon = vec![start];
        let mut current = start;
        while let Some(&next) = edges.get(&current) {
            if next == start || polygon.len() > edges.len() {
                break;
            }
            polygon.push(next);
            current = next;
        }
        if polygon.len() == edges.len() && polygon.len() >= 3 {
            made.push(index(mesh.faces.len()));
            mesh.faces.push(polygon);
        }
    }
    mesh.remove_unused_vertices();
    Ok(made)
}

/// The selected part as a mesh of its own: the faces all of whose corners are selected, the
/// selected loose edges, and the selected vertices they use (lone selected vertices too).
fn split_off_raw(mesh: &Mesh, vertices: &BTreeSet<u32>) -> Mesh {
    let mut map: BTreeMap<u32, u32> = BTreeMap::new();
    let mut out = Mesh { smooth: mesh.smooth, smooth_angle: mesh.smooth_angle, ..Default::default() };
    for vertex in vertices {
        if let Some(point) = mesh.positions.get(*vertex as usize) {
            map.insert(*vertex, out.vertex_count());
            out.positions.push(*point);
        }
    }
    let mapped = |vertex: &u32| map.get(vertex).copied();
    for face in mesh.faces.iter().filter(|face| face.iter().all(|corner| vertices.contains(corner))) {
        if let Some(corners) = face.iter().map(mapped).collect::<Option<Vec<u32>>>() {
            out.faces.push(corners);
        }
    }
    // Selected edges the faces taken do not cover come as loose edges.
    let mut taken: BTreeSet<(u32, u32)> = out.faces.iter().flat_map(|face| face_edges(face).map(|(a, b)| key(a, b)).collect::<Vec<_>>()).collect();
    for (a, b) in selected_edges(mesh, vertices) {
        if let (Some(a), Some(b)) = (mapped(&a), mapped(&b))
            && taken.insert(key(a, b))
        {
            out.loose_edges.push([a, b]);
        }
    }
    out
}

/// Removes the part [`split_off`] takes: selected faces and edges, then selected vertices
/// nothing uses any more.
fn remove_split_raw(mesh: &mut Mesh, vertices: &BTreeSet<u32>) {
    mesh.faces.retain(|face| !face.iter().all(|corner| vertices.contains(corner)));
    mesh.loose_edges.retain(|edge| !edge.iter().all(|vertex| vertices.contains(vertex)));
    let mut used = vec![false; mesh.positions.len()];
    for vertex in mesh.faces.iter().flatten().chain(mesh.loose_edges.iter().flatten()) {
        if let Some(flag) = used.get_mut(*vertex as usize) {
            *flag = true;
        }
    }
    let mut remap = Vec::with_capacity(mesh.positions.len());
    let mut kept = Vec::with_capacity(mesh.positions.len());
    for (at, point) in mesh.positions.iter().enumerate() {
        let selected = vertices.contains(&index(at));
        if selected && !used.get(at).copied().unwrap_or(false) {
            remap.push(u32::MAX);
        } else {
            remap.push(index(kept.len()));
            kept.push(*point);
        }
    }
    mesh.positions = kept;
    mesh.reindex(&remap);
}

/// [`extrude_loose_raw`] with the mesh's texture coordinates carried through.
pub fn extrude_loose(mesh: &mut Mesh, vertices: &BTreeSet<u32>, offset: Vec3) -> Result<BTreeSet<u32>> {
    crate::uv::with_uvs(mesh, |mesh| extrude_loose_raw(mesh, vertices, offset))
}

/// [`extrude_individual_raw`] with the mesh's texture coordinates carried through.
pub fn extrude_individual(mesh: &mut Mesh, faces: &[u32], distance: f32) -> Result<BTreeSet<u32>> {
    crate::uv::with_uvs(mesh, |mesh| extrude_individual_raw(mesh, faces, distance))
}

/// [`dissolve_vertices_raw`] with the mesh's texture coordinates carried through.
pub fn dissolve_vertices(mesh: &mut Mesh, vertices: &BTreeSet<u32>) -> Result<()> {
    crate::uv::with_uvs(mesh, |mesh| dissolve_vertices_raw(mesh, vertices))
}

/// [`dissolve_edges_raw`] with the mesh's texture coordinates carried through.
pub fn dissolve_edges(mesh: &mut Mesh, edges: &[(u32, u32)]) -> Result<()> {
    crate::uv::with_uvs(mesh, |mesh| dissolve_edges_raw(mesh, edges))
}

/// [`dissolve_faces_raw`] with the mesh's texture coordinates carried through.
pub fn dissolve_faces(mesh: &mut Mesh, faces: &[u32]) -> Result<Vec<u32>> {
    crate::uv::with_uvs(mesh, |mesh| dissolve_faces_raw(mesh, faces))
}

/// [`bridge_raw`] with the mesh's texture coordinates carried through.
pub fn bridge(mesh: &mut Mesh, vertices: &BTreeSet<u32>) -> Result<Vec<u32>> {
    crate::uv::with_uvs(mesh, |mesh| bridge_raw(mesh, vertices))
}

/// [`bisect_raw`] with the mesh's texture coordinates carried through.
pub fn bisect(mesh: &mut Mesh, point: Vec3, normal: Vec3, clear_inner: bool, clear_outer: bool, fill: bool) -> Result<BTreeSet<u32>> {
    crate::uv::with_uvs(mesh, |mesh| bisect_raw(mesh, point, normal, clear_inner, clear_outer, fill))
}

/// [`symmetrize_raw`] with the mesh's texture coordinates carried through.
pub fn symmetrize(mesh: &mut Mesh, axis: usize, positive_to_negative: bool) -> Result<()> {
    crate::uv::with_uvs(mesh, |mesh| symmetrize_raw(mesh, axis, positive_to_negative))
}

/// [`spin_raw`] with the mesh's texture coordinates carried through.
pub fn spin(mesh: &mut Mesh, vertices: &BTreeSet<u32>, steps: u32, angle: f32, axis: Vec3, center: Vec3) -> Result<BTreeSet<u32>> {
    crate::uv::with_uvs(mesh, |mesh| spin_raw(mesh, vertices, steps, angle, axis, center))
}

/// [`bevel_vertices_raw`] with the mesh's texture coordinates carried through.
pub fn bevel_vertices(mesh: &mut Mesh, vertices: &BTreeSet<u32>, width: f32) -> Result<Vec<u32>> {
    crate::uv::with_uvs(mesh, |mesh| bevel_vertices_raw(mesh, vertices, width))
}

/// [`remove_split_raw`] with the mesh's texture coordinates carried through.
pub fn remove_split(mesh: &mut Mesh, vertices: &BTreeSet<u32>) {
    crate::uv::with_uvs(mesh, |mesh| remove_split_raw(mesh, vertices))
}

/// [`split_off_raw`], the new mesh taking its texture coordinates from the faces it came from.
pub fn split_off(mesh: &Mesh, vertices: &BTreeSet<u32>) -> Mesh {
    let mut out = split_off_raw(mesh, vertices);
    if mesh.has_uvs() && mesh.uvs_aligned() {
        crate::uv::carry(Some(mesh), &mut out);
    } else if !out.uvs_aligned() {
        out.uvs.clear();
    }
    out
}
