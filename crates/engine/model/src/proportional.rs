//! Proportional editing: when selected vertices move, unselected ones within a radius follow,
//! less the farther they are, by a falloff curve (the reference application's curves).

use std::collections::{BTreeSet, HashMap};

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::mesh::Mesh;

/// How the pull fades with distance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Falloff {
    #[default]
    Smooth,
    Sphere,
    Root,
    Linear,
    Sharp,
    Constant,
}

/// Proportional editing settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Proportional {
    pub radius: f32,
    #[serde(default)]
    pub falloff: Falloff,
    /// Only vertices joined to the selection by edges follow, by their distance along the edges
    /// (the reference's "Connected Only").
    #[serde(default)]
    pub connected: bool,
}

impl Default for Proportional {
    fn default() -> Self {
        Self { radius: 1.0, falloff: Falloff::Smooth, connected: false }
    }
}

/// The weight at `distance` from the selection within `radius` (1 at the selection, 0 at the
/// radius).
pub fn weight(falloff: Falloff, distance: f32, radius: f32) -> f32 {
    if radius.is_nan() || radius <= 0.0 || distance >= radius {
        return 0.0;
    }
    let t = (1.0 - distance / radius).clamp(0.0, 1.0);
    match falloff {
        Falloff::Smooth => t * t * (3.0 - 2.0 * t),
        Falloff::Sphere => (2.0 * t - t * t).max(0.0).sqrt(),
        Falloff::Root => t.sqrt(),
        Falloff::Linear => t,
        Falloff::Sharp => t * t,
        Falloff::Constant => 1.0,
    }
}

/// Distances along edges from the selection (Dijkstra), within the radius.
fn connected_weights(mesh: &Mesh, selected: &BTreeSet<u32>, settings: Proportional) -> Vec<(u32, f32)> {
    use std::cmp::Ordering;
    use std::collections::BinaryHeap;
    #[derive(PartialEq)]
    struct Next(f32, u32);
    impl Eq for Next {}
    impl PartialOrd for Next {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for Next {
        fn cmp(&self, other: &Self) -> Ordering {
            other.0.total_cmp(&self.0).then_with(|| other.1.cmp(&self.1))
        }
    }
    let mut neighbours: HashMap<u32, Vec<u32>> = HashMap::new();
    for [a, b] in mesh.edges() {
        neighbours.entry(a).or_default().push(b);
        neighbours.entry(b).or_default().push(a);
    }
    let position = |vertex: u32| mesh.positions.get(vertex as usize).copied().unwrap_or(Vec3::ZERO);
    let mut best: HashMap<u32, f32> = selected.iter().map(|vertex| (*vertex, 0.0)).collect();
    let mut queue: BinaryHeap<Next> = selected.iter().map(|vertex| Next(0.0, *vertex)).collect();
    while let Some(Next(distance, vertex)) = queue.pop() {
        if best.get(&vertex).is_some_and(|known| distance > *known) {
            continue;
        }
        for &other in neighbours.get(&vertex).map_or(&[][..], Vec::as_slice) {
            let next = distance + position(vertex).distance(position(other));
            if next < settings.radius && best.get(&other).is_none_or(|known| next < *known) {
                best.insert(other, next);
                queue.push(Next(next, other));
            }
        }
    }
    let mut out: Vec<(u32, f32)> = best
        .into_iter()
        .filter(|(vertex, _)| !selected.contains(vertex))
        .map(|(vertex, distance)| (vertex, weight(settings.falloff, distance, settings.radius)))
        .filter(|(_, value)| *value > 0.0)
        .collect();
    out.sort_by_key(|(vertex, _)| *vertex);
    out
}

/// The unselected vertices within `settings.radius` of a selected one, with their weights.
pub fn weights(mesh: &Mesh, selected: &BTreeSet<u32>, settings: Proportional) -> Vec<(u32, f32)> {
    let radius = settings.radius;
    if !(radius.is_finite() && radius > 0.0) || selected.is_empty() {
        return Vec::new();
    }
    if settings.connected {
        return connected_weights(mesh, selected, settings);
    }
    // Selected vertices in a grid of cells one radius wide.
    let cell = |point: Vec3| ((point.x / radius).floor() as i64, (point.y / radius).floor() as i64, (point.z / radius).floor() as i64);
    let mut grid: HashMap<(i64, i64, i64), Vec<Vec3>> = HashMap::new();
    for vertex in selected {
        if let Some(point) = mesh.positions.get(*vertex as usize) {
            grid.entry(cell(*point)).or_default().push(*point);
        }
    }
    let mut out = Vec::new();
    for (at, point) in mesh.positions.iter().enumerate() {
        let vertex = u32::try_from(at).unwrap_or(u32::MAX);
        if selected.contains(&vertex) {
            continue;
        }
        let (x, y, z) = cell(*point);
        let mut nearest = f32::INFINITY;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    for other in grid.get(&(x + dx, y + dy, z + dz)).map_or(&[][..], Vec::as_slice) {
                        nearest = nearest.min(point.distance(*other));
                    }
                }
            }
        }
        let value = weight(settings.falloff, nearest, radius);
        if value > 0.0 {
            out.push((vertex, value));
        }
    }
    out
}

/// Moves weighted vertices part of the way `change` would take them.
pub fn apply(mesh: &mut Mesh, weights: &[(u32, f32)], change: impl Fn(Vec3) -> Vec3) {
    for (vertex, value) in weights {
        if let Some(point) = mesh.positions.get_mut(*vertex as usize) {
            let target = change(*point);
            if target.is_finite() {
                *point += (target - *point) * *value;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connected_only_leaves_a_separate_piece_alone() {
        // Two plane pieces side by side, not joined: only the selected piece's vertices follow.
        let mut mesh = crate::primitives::Primitive::Plane { size: 2.0 }.build().unwrap();
        let mut other = mesh.clone();
        for point in &mut other.positions {
            point.x += 2.5;
        }
        mesh.append(&other);
        let selected: BTreeSet<u32> = BTreeSet::from([1, 3]); // the right edge of the first plane
        let near = Proportional { radius: 3.0, falloff: Falloff::Linear, connected: false };
        let joined = Proportional { connected: true, ..near };
        let free: BTreeSet<u32> = weights(&mesh, &selected, near).into_iter().map(|(vertex, _)| vertex).collect();
        let along: BTreeSet<u32> = weights(&mesh, &selected, joined).into_iter().map(|(vertex, _)| vertex).collect();
        assert!(free.iter().any(|vertex| *vertex >= 4), "by distance, the other piece follows too: {free:?}");
        assert!(along.iter().all(|vertex| *vertex < 4) && !along.is_empty(), "connected only stays on its piece: {along:?}");
    }

    #[test]
    fn curves_run_from_one_to_zero() {
        for falloff in [Falloff::Smooth, Falloff::Sphere, Falloff::Root, Falloff::Linear, Falloff::Sharp] {
            assert!((weight(falloff, 0.0, 2.0) - 1.0).abs() < 1e-6, "{falloff:?}");
            assert_eq!(weight(falloff, 2.0, 2.0), 0.0);
            assert!(weight(falloff, 1.0, 2.0) > 0.0 && weight(falloff, 1.0, 2.0) < 1.0);
        }
        assert_eq!(weight(Falloff::Constant, 1.9, 2.0), 1.0);
        assert!((weight(Falloff::Smooth, 1.0, 2.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn nearby_vertices_follow() {
        let mut mesh = crate::Primitive::Grid { x_subdivisions: 4, y_subdivisions: 1, size: 4.0 }.build().unwrap_or_default();
        // Pull the middle column up; its neighbours follow half way (smooth at half the radius).
        let middle: BTreeSet<u32> =
            (0..mesh.vertex_count()).filter(|vertex| mesh.positions.get(*vertex as usize).is_some_and(|point| point.x.abs() < 1e-6)).collect();
        let found = weights(&mesh, &middle, Proportional { radius: 2.0, falloff: Falloff::Smooth, connected: false });
        assert_eq!(found.len(), 4, "the two columns one metre away");
        apply(&mut mesh, &found, |point| point + Vec3::Z);
        for (vertex, value) in found {
            assert!((value - 0.5).abs() < 1e-6);
            assert!((mesh.positions.get(vertex as usize).map_or(0.0, |point| point.z) - 0.5).abs() < 1e-6);
        }
    }
}
