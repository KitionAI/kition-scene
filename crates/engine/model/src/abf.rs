//! Angle-based flattening for unwrapping (linearized ABF): the angles a flat chart can have
//! closest to the surface's own, with each triangle's angles summing to π, each inner vertex's to
//! 2π, and the sine ("wheel") condition around each inner vertex that lets the triangles close up
//! (taken to first order). Coordinates are then rebuilt by least-squares conformal maps on the
//! triangle shapes those angles give (`crate::unwrap::lscm_shapes`). Inner vertices' angles are
//! first scaled to sum to 2π, the way angle-based unwrapping starts.

use std::collections::{BTreeMap, HashMap};
use std::f64::consts::{PI, TAU};

use glam::{DVec2, DVec3};

use crate::mesh::Mesh;
use crate::unwrap::Shape;

/// The smallest angle a solved triangle keeps (radians).
const SMALLEST: f64 = 1e-3;

/// A chart's triangles as chart vertices (the faces split as for drawing, never overlapping).
pub(crate) fn chart_triangles(mesh: &Mesh, faces: &[usize], corner_vertex: &HashMap<(usize, usize), usize>) -> Vec<[usize; 3]> {
    let wanted: std::collections::BTreeSet<usize> = faces.iter().copied().collect();
    let mut out = Vec::new();
    for (corners, face) in mesh.corner_triangles() {
        let face = face as usize;
        if !wanted.contains(&face) {
            continue;
        }
        let local = |corner: u32| corner_vertex.get(&(face, corner as usize)).copied();
        if let (Some(a), Some(b), Some(c)) = (local(corners[0]), local(corners[1]), local(corners[2])) {
            out.push([a, b, c]);
        }
    }
    out
}

/// The angle at `a` of triangle `a b c`.
fn angle(a: DVec3, b: DVec3, c: DVec3) -> f64 {
    let (u, v) = (b - a, c - a);
    if u.length_squared() < 1e-30 || v.length_squared() < 1e-30 { PI / 3.0 } else { u.angle_between(v).clamp(SMALLEST, PI - SMALLEST) }
}

/// Each triangle's shape in its own plane from the solved angles (the first edge keeps its
/// surface length), ready for `lscm_shapes`.
pub(crate) fn shapes(points: &[DVec3], triangles: &[[usize; 3]]) -> Vec<Shape> {
    let usable: Vec<[usize; 3]> = triangles
        .iter()
        .copied()
        .filter(|[a, b, c]| match (points.get(*a), points.get(*b), points.get(*c)) {
            (Some(pa), Some(pb), Some(pc)) => (*pb - *pa).cross(*pc - *pa).length() > 1e-14,
            _ => false,
        })
        .collect();
    let angles = solve(points, &usable);
    usable
        .iter()
        .zip(angles)
        .filter_map(|(&[a, b, c], [alpha, beta, gamma])| {
            let length = points.get(a)?.distance(*points.get(b)?);
            let side = length * beta.sin() / gamma.sin().max(1e-12);
            Some(([a, b, c], [DVec2::ZERO, DVec2::new(length, 0.0), DVec2::new(side * alpha.cos(), side * alpha.sin())], 1.0))
        })
        .collect()
}

/// The flat angles (at each triangle's three corners) closest to the surface's, under the
/// triangle, vertex, and linearized wheel conditions.
pub fn solve(points: &[DVec3], triangles: &[[usize; 3]]) -> Vec<[f64; 3]> {
    let position = |vertex: usize| points.get(vertex).copied().unwrap_or(DVec3::ZERO);
    let surface: Vec<[f64; 3]> = triangles
        .iter()
        .map(|&[a, b, c]| {
            let (pa, pb, pc) = (position(a), position(b), position(c));
            [angle(pa, pb, pc), angle(pb, pc, pa), angle(pc, pa, pb)]
        })
        .collect();
    // Inner vertices: every edge around them is shared by two triangles.
    let mut edge_uses: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for &[a, b, c] in triangles {
        for (p, q) in [(a, b), (b, c), (c, a)] {
            *edge_uses.entry((p.min(q), p.max(q))).or_default() += 1;
        }
    }
    let mut around: BTreeMap<usize, Vec<(usize, usize)>> = BTreeMap::new();
    for (t, corners) in triangles.iter().enumerate() {
        for (k, vertex) in corners.iter().enumerate() {
            around.entry(*vertex).or_default().push((t, k));
        }
    }
    let boundary = |vertex: usize| edge_uses.iter().any(|((p, q), uses)| (*p == vertex || *q == vertex) && *uses != 2);
    let inner: Vec<usize> = around.keys().copied().filter(|vertex| !boundary(*vertex)).collect();

    // Targets: the surface's angles, an inner vertex's scaled to sum to 2π.
    let mut target = surface.clone();
    for vertex in &inner {
        let corners = around.get(vertex).cloned().unwrap_or_default();
        let sum: f64 = corners.iter().filter_map(|(t, k)| surface.get(*t).and_then(|angles| angles.get(*k))).sum();
        if sum > 1e-9 {
            for (t, k) in corners {
                if let Some(slot) = target.get_mut(t).and_then(|angles| angles.get_mut(k)) {
                    *slot *= TAU / sum;
                }
            }
        }
    }

    // Unknown e (3 per triangle): the change from the target. Rows of A e = b.
    let unknown = |t: usize, k: usize| 3 * t + k;
    let value = |t: usize, k: usize| target.get(t).and_then(|angles| angles.get(k)).copied().unwrap_or(PI / 3.0);
    let mut rows: Vec<(Vec<(usize, f64)>, f64)> = Vec::new();
    for t in 0..triangles.len() {
        rows.push(((0..3).map(|k| (unknown(t, k), 1.0)).collect(), PI - (0..3).map(|k| value(t, k)).sum::<f64>()));
    }
    for vertex in &inner {
        let corners = around.get(vertex).cloned().unwrap_or_default();
        rows.push((corners.iter().map(|(t, k)| (unknown(*t, *k), 1.0)).collect(), TAU - corners.iter().map(|(t, k)| value(*t, *k)).sum::<f64>()));
        // Wheel: Σ ln sin(next) = Σ ln sin(previous), to first order around the targets.
        let mut entries = Vec::new();
        let mut residual = 0.0;
        for (t, k) in corners {
            let (next, previous) = ((k + 1) % 3, (k + 2) % 3);
            let (a, b) = (value(t, next), value(t, previous));
            entries.push((unknown(t, next), 1.0 / a.tan()));
            entries.push((unknown(t, previous), -1.0 / b.tan()));
            residual += a.sin().ln() - b.sin().ln();
        }
        rows.push((entries, -residual));
    }
    // Minimise Σ e² / target² under the rows: e = W A^T λ with (A W A^T) λ = b, W = target².
    let weight: Vec<f64> = (0..3 * triangles.len()).map(|index| value(index / 3, index % 3).powi(2)).collect();
    let lambda = conjugate(&rows, &weight);
    let mut out = target;
    for (row, (entries, _)) in rows.iter().enumerate() {
        let l = lambda.get(row).copied().unwrap_or(0.0);
        for (column, coefficient) in entries {
            if let Some(slot) = out.get_mut(column / 3).and_then(|angles| angles.get_mut(column % 3)) {
                *slot += weight.get(*column).copied().unwrap_or(0.0) * coefficient * l;
            }
        }
    }
    // Keep every triangle a triangle.
    for angles in &mut out {
        for slot in angles.iter_mut() {
            *slot = slot.clamp(SMALLEST, PI - SMALLEST);
        }
        let sum: f64 = angles.iter().sum();
        for slot in angles.iter_mut() {
            *slot *= PI / sum;
        }
    }
    out
}

/// Solves (A W Aᵀ) λ = b by conjugate gradients, the rows being A's with their right-hand sides.
fn conjugate(rows: &[(Vec<(usize, f64)>, f64)], weight: &[f64]) -> Vec<f64> {
    let count = rows.len();
    let apply = |lambda: &[f64]| -> Vec<f64> {
        // x = W Aᵀ λ, then A x.
        let mut x = vec![0.0; weight.len()];
        for ((entries, _), l) in rows.iter().zip(lambda) {
            for (column, coefficient) in entries {
                if let Some(slot) = x.get_mut(*column) {
                    *slot += coefficient * l;
                }
            }
        }
        for (slot, w) in x.iter_mut().zip(weight) {
            *slot *= w;
        }
        rows.iter()
            .map(|(entries, _)| entries.iter().map(|(column, coefficient)| coefficient * x.get(*column).copied().unwrap_or(0.0)).sum())
            .collect()
    };
    let dot = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>();
    let b: Vec<f64> = rows.iter().map(|(_, rhs)| *rhs).collect();
    let mut lambda = vec![0.0; count];
    let mut residual = b.clone();
    let mut direction = residual.clone();
    let mut old = dot(&residual, &residual);
    let scale = dot(&b, &b).max(1e-30);
    for _ in 0..(count * 3).clamp(200, 30_000) {
        if old <= scale * 1e-26 {
            break;
        }
        let towards = apply(&direction);
        let curvature = dot(&direction, &towards);
        if curvature.abs() < 1e-300 {
            break;
        }
        let step = old / curvature;
        for (value, d) in lambda.iter_mut().zip(&direction) {
            *value += step * d;
        }
        for (r, t) in residual.iter_mut().zip(&towards) {
            *r -= step * t;
        }
        let new = dot(&residual, &residual);
        let ratio = new / old;
        for (d, r) in direction.iter_mut().zip(&residual) {
            *d = r + ratio * *d;
        }
        old = new;
    }
    lambda
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_fan_keeps_its_angles_and_a_cone_opens_to_two_pi() {
        // A flat hexagon fan: already flat, so the angles stay.
        let mut points = vec![DVec3::ZERO];
        for step in 0..6 {
            let a = f64::from(step) * TAU / 6.0;
            points.push(DVec3::new(a.cos(), a.sin(), 0.0));
        }
        let triangles: Vec<[usize; 3]> = (0..6).map(|step| [0, 1 + step, 1 + (step + 1) % 6]).collect();
        let solved = solve(&points, &triangles);
        for angles in &solved {
            for value in angles {
                assert!((value - PI / 3.0).abs() < 1e-6, "{value}");
            }
        }
        // A cone's apex: the six apex angles sum to 2π after flattening.
        points[0] = DVec3::new(0.0, 0.0, 0.8);
        let solved = solve(&points, &triangles);
        // The apex is on the boundary of this open fan? No: the fan is closed around it.
        let apex: f64 = solved.iter().map(|angles| angles[0]).sum();
        assert!((apex - TAU).abs() < 1e-6, "{apex}");
        for angles in &solved {
            assert!((angles.iter().sum::<f64>() - PI).abs() < 1e-9);
        }
    }
}
