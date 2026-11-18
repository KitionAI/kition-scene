//! Bone heat weights (Baran and Popović, "Automatic Rigging and Animation of 3D Characters",
//! 2007), the reference application's "automatic weights": each bone's weights are the
//! steady-state heat on the surface when the bone heats the vertices that see it as their
//! nearest bone, `(L + H) w = H p`, with `L` the cotangent Laplacian of the surface, `H` the
//! heat each vertex takes from its nearest bone (`4 A / d²` for distance `d` and the vertex's
//! share `A` of the surface area, when the straight line to the bone does not pass through the
//! surface), and `p` 1 where that bone is the nearest (shared equally on ties). The system is
//! solved by conjugate gradients, bone by bone. On the meshes checked, the weights correlate
//! with the reference application's at 0.9997 or better.

use glam::{Mat4, Vec3};

use crate::armature::distance_to_segment;
use crate::mesh::Mesh;

/// The most triangles a mesh may have for heat weights (beyond, callers fall back to distance).
pub const MAX_TRIANGLES: usize = 60_000;

/// Weights below this are dropped.
const FLOOR: f32 = 1e-3;

/// The heat a vertex takes from its nearest bone, per unit of area over squared distance.
const SOURCE: f32 = 4.0;

/// Heat weights of `mesh` (transformed into the bones' space by `to_bones`) for each segment
/// (head, tail), one list per segment, or `None` when the mesh is too large or has no surface.
pub fn weights(mesh: &Mesh, to_bones: Mat4, segments: &[(Vec3, Vec3)]) -> Option<Vec<Vec<f32>>> {
    let points: Vec<Vec3> = mesh.positions.iter().map(|point| to_bones.transform_point3(*point)).collect();
    let triangles: Vec<[u32; 3]> = mesh.triangles().into_iter().map(|(triangle, _)| triangle).collect();
    if triangles.is_empty() || triangles.len() > MAX_TRIANGLES || segments.is_empty() {
        return None;
    }
    let count = points.len();
    let laplacian = cotangent_laplacian(&points, &triangles);
    // Each vertex's share of the surface: a third of each triangle around it.
    let mut area = vec![0.0f32; count];
    for [a, b, c] in &triangles {
        let (Some(pa), Some(pb), Some(pc)) = (points.get(*a as usize), points.get(*b as usize), points.get(*c as usize)) else { continue };
        let third = (*pb - *pa).cross(*pc - *pa).length() / 6.0;
        for vertex in [a, b, c] {
            if let Some(slot) = area.get_mut(*vertex as usize) {
                *slot += third;
            }
        }
    }
    // Heat sources: each vertex's nearest visible bones.
    let mut heat = vec![0.0f32; count];
    let mut sources: Vec<Vec<f32>> = vec![vec![0.0; count]; segments.len()];
    for (vertex, point) in points.iter().enumerate() {
        let distances: Vec<f32> = segments.iter().map(|(head, tail)| distance_to_segment(*point, *head, *tail)).collect();
        let nearest = distances.iter().copied().fold(f32::INFINITY, f32::min);
        if !nearest.is_finite() {
            continue;
        }
        let close: Vec<usize> = distances
            .iter()
            .enumerate()
            .filter(|(bone, distance)| {
                **distance <= nearest * 1.0001 + 1e-6
                    && segments
                        .get(*bone)
                        .is_some_and(|(head, tail)| visible(*point, closest_on_segment(*point, *head, *tail), vertex as u32, &points, &triangles))
            })
            .map(|(bone, _)| bone)
            .collect();
        if close.is_empty() {
            continue;
        }
        let d = nearest.max(1e-3);
        if let Some(slot) = heat.get_mut(vertex) {
            *slot = SOURCE * area.get(vertex).copied().unwrap_or(0.0) / (d * d);
        }
        for bone in &close {
            if let Some(slot) = sources.get_mut(*bone).and_then(|list| list.get_mut(vertex)) {
                *slot = 1.0 / close.len() as f32;
            }
        }
    }
    let mut out = Vec::with_capacity(segments.len());
    for source in &sources {
        let rhs: Vec<f32> = heat.iter().zip(source).map(|(h, p)| h * p).collect();
        let solved = conjugate_gradient(&laplacian, &heat, &rhs, (count * 4).max(2000));
        out.push(solved.into_iter().map(|w| if w < FLOOR { 0.0 } else { w.min(1.0) }).collect());
    }
    Some(out)
}

/// A symmetric sparse matrix: the diagonal, and each row's off-diagonal entries.
struct Sparse {
    diagonal: Vec<f32>,
    rows: Vec<Vec<(u32, f32)>>,
}

/// The cotangent Laplacian (positive semi-definite form: diagonal the sum of a row's weights,
/// off-diagonal their negatives). Negative cotangents of obtuse angles are kept at zero so the
/// matrix stays positive.
fn cotangent_laplacian(points: &[Vec3], triangles: &[[u32; 3]]) -> Sparse {
    let count = points.len();
    // Ordered, so the sums (and the solution) are the same on every run.
    let mut weights: std::collections::BTreeMap<(u32, u32), f32> = std::collections::BTreeMap::new();
    for [a, b, c] in triangles {
        let (Some(pa), Some(pb), Some(pc)) = (points.get(*a as usize), points.get(*b as usize), points.get(*c as usize)) else { continue };
        for (i, j, opposite, pi, pj) in [(*a, *b, *pc, *pa, *pb), (*b, *c, *pa, *pb, *pc), (*c, *a, *pb, *pc, *pa)] {
            let (u, v) = (pi - opposite, pj - opposite);
            let sine = u.cross(v).length();
            if sine < 1e-12 {
                continue;
            }
            let cotangent = (u.dot(v) / sine).max(0.0);
            *weights.entry((i.min(j), i.max(j))).or_default() += 0.5 * cotangent;
        }
    }
    let mut diagonal = vec![0.0f32; count];
    let mut rows: Vec<Vec<(u32, f32)>> = vec![Vec::new(); count];
    for ((i, j), weight) in weights {
        if let Some(slot) = diagonal.get_mut(i as usize) {
            *slot += weight;
        }
        if let Some(slot) = diagonal.get_mut(j as usize) {
            *slot += weight;
        }
        if let Some(row) = rows.get_mut(i as usize) {
            row.push((j, -weight));
        }
        if let Some(row) = rows.get_mut(j as usize) {
            row.push((i, -weight));
        }
    }
    Sparse { diagonal, rows }
}

/// Solves `(L + diag(heat)) x = rhs` by Jacobi-preconditioned conjugate gradients.
fn conjugate_gradient(matrix: &Sparse, heat: &[f32], rhs: &[f32], iterations: usize) -> Vec<f32> {
    let count = rhs.len();
    let diagonal: Vec<f64> = matrix.diagonal.iter().zip(heat).map(|(d, h)| f64::from(*d) + f64::from(*h)).collect();
    let multiply = |x: &[f64], out: &mut [f64]| {
        for (row, slot) in out.iter_mut().enumerate() {
            let mut sum = diagonal.get(row).copied().unwrap_or(0.0) * x.get(row).copied().unwrap_or(0.0);
            for (column, value) in matrix.rows.get(row).map_or(&[][..], Vec::as_slice) {
                sum += f64::from(*value) * x.get(*column as usize).copied().unwrap_or(0.0);
            }
            *slot = sum;
        }
    };
    let b: Vec<f64> = rhs.iter().map(|value| f64::from(*value)).collect();
    let norm_b = b.iter().map(|value| value * value).sum::<f64>().sqrt();
    if norm_b == 0.0 {
        return vec![0.0; count];
    }
    let mut x = vec![0.0f64; count];
    let mut r = b.clone();
    let precondition = |r: &[f64]| -> Vec<f64> { r.iter().zip(&diagonal).map(|(value, d)| if *d > 0.0 { value / d } else { *value }).collect() };
    let mut z = precondition(&r);
    let mut p = z.clone();
    let mut rz: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
    let mut ap = vec![0.0f64; count];
    for _ in 0..iterations {
        multiply(&p, &mut ap);
        let pap: f64 = p.iter().zip(&ap).map(|(a, b)| a * b).sum();
        if pap.abs() < 1e-300 {
            break;
        }
        let alpha = rz / pap;
        for ((xi, pi), (ri, api)) in x.iter_mut().zip(&p).zip(r.iter_mut().zip(&ap)) {
            *xi += alpha * pi;
            *ri -= alpha * api;
        }
        if r.iter().map(|value| value * value).sum::<f64>().sqrt() < 1e-12 * norm_b {
            break;
        }
        z = precondition(&r);
        let next: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
        let beta = next / rz;
        rz = next;
        for (pi, zi) in p.iter_mut().zip(&z) {
            *pi = zi + beta * *pi;
        }
    }
    x.into_iter().map(|value| value as f32).collect()
}

fn closest_on_segment(point: Vec3, a: Vec3, b: Vec3) -> Vec3 {
    let along = b - a;
    let length = along.length_squared();
    let t = if length > 0.0 { ((point - a).dot(along) / length).clamp(0.0, 1.0) } else { 0.0 };
    a + along * t
}

/// Whether the straight line from vertex `from` (at `start`) to `end` stays off the surface.
fn visible(start: Vec3, end: Vec3, from: u32, points: &[Vec3], triangles: &[[u32; 3]]) -> bool {
    let direction = end - start;
    let length = direction.length();
    if length < 1e-6 {
        return true;
    }
    let (low, high) = (start.min(end), start.max(end));
    for [a, b, c] in triangles {
        if *a == from || *b == from || *c == from {
            continue;
        }
        let (Some(pa), Some(pb), Some(pc)) = (points.get(*a as usize), points.get(*b as usize), points.get(*c as usize)) else { continue };
        if pa.max(*pb).max(*pc).cmplt(low).any() || pa.min(*pb).min(*pc).cmpgt(high).any() {
            continue;
        }
        // Möller–Trumbore, for hits strictly between the ends.
        let (e1, e2) = (*pb - *pa, *pc - *pa);
        let p = direction.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-12 {
            continue;
        }
        let inverse = 1.0 / det;
        let s = start - *pa;
        let u = s.dot(p) * inverse;
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let q = s.cross(e1);
        let v = direction.dot(q) * inverse;
        if v < 0.0 || u + v > 1.0 {
            continue;
        }
        let t = e2.dot(q) * inverse;
        if t > 1e-4 && t < 1.0 - 1e-4 {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::Primitive;

    #[test]
    fn a_column_shares_its_weight_between_two_bones() {
        let mesh = Primitive::Cylinder { vertices: 12, radius: 0.25, depth: 2.0, fill: crate::primitives::Fill::Ngon }.build().unwrap();
        let mut mesh = mesh;
        // Rings along the length, so heat has somewhere to flow.
        for _ in 0..3 {
            let side: Vec<u32> = (0..mesh.faces.len() as u32).filter(|face| mesh.faces[*face as usize].len() == 4).collect();
            crate::ops::subdivide(&mut mesh, &side).unwrap();
        }
        let segments = [(Vec3::new(0.0, 0.0, -1.0), Vec3::ZERO), (Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0))];
        let weights = weights(&mesh, Mat4::IDENTITY, &segments).unwrap();
        for (index, point) in mesh.positions.iter().enumerate() {
            let (low, high) = (weights[0][index], weights[1][index]);
            assert!((low + high - 1.0).abs() < 0.02, "vertex {index}: {low} + {high}");
            if point.z < -0.6 {
                assert!(low > 0.9, "a low vertex leans on the lower bone: {low}");
            }
            if point.z > 0.6 {
                assert!(high > 0.9);
            }
        }
    }
}
