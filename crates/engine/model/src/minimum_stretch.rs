//! SLIM for the symmetric Dirichlet energy, independently derived from Rabinovich, Poranne,
//! Panozzo and Sorkine-Hornung, ACM TOG 2017, equations 3, 6, 17 and 20.
//! Publication: https://igl.ethz.ch/projects/slim/SLIM2017.pdf (no implementation consulted).
//! Each local step computes a polar rotation and the left-singular-vector matrix weights;
//! the global step solves the weighted least squares problem with exact pin elimination.
//! Backtracking stays before the first zero determinant along the entire step.

use std::collections::{BTreeMap, BTreeSet};

use glam::{DMat2, DVec2, DVec3};

use crate::error::{Result, SceneError};
use crate::unwrap::{Shape, conjugate_gradient, farthest_pair, lscm_shapes};

struct Triangle {
    vertices: [usize; 3],
    gradients: [DVec2; 3],
    area: f64,
}

impl Triangle {
    fn jacobian(&self, uv: &[DVec2]) -> DMat2 {
        let mut result = DMat2::ZERO;
        for (vertex, gradient) in self.vertices.iter().zip(self.gradients) {
            let point = uv.get(*vertex).copied().unwrap_or_default();
            result += DMat2::from_cols(point * gradient.x, point * gradient.y);
        }
        result
    }
}

/// Bounded work: interactive callers must split very large charts before asking for SLIM.
pub(crate) fn solve(points: &[DVec3], indices: &[[usize; 3]], pins: &[(usize, DVec2)], iterations: u32) -> Result<Vec<DVec2>> {
    if points.len() > 20_000 || indices.len() > 40_000 || !(1..=100).contains(&iterations) {
        return Err(SceneError::params("minimum stretch supports up to 20000 chart vertices, 40000 triangles, and 1 to 100 iterations"));
    }
    let mut triangles = Vec::with_capacity(indices.len());
    let mut shapes: Vec<Shape> = Vec::with_capacity(indices.len());
    for &[a, b, c] in indices {
        let (Some(pa), Some(pb), Some(pc)) = (points.get(a), points.get(b), points.get(c)) else {
            return Err(SceneError::params("minimum stretch found an invalid triangle"));
        };
        let (ab, ac) = (*pb - *pa, *pc - *pa);
        let length = ab.length();
        let twice_area = ab.cross(ac).length();
        if !length.is_finite() || !twice_area.is_finite() || twice_area <= length.max(ac.length()).powi(2) * 1e-12 || length <= 1e-12 {
            return Err(SceneError::params("minimum stretch needs nondegenerate triangles"));
        }
        let x = ac.dot(ab) / length;
        let y = twice_area / length;
        let gb = DVec2::new(1.0 / length, -x / (length * y));
        let gc = DVec2::new(0.0, 1.0 / y);
        triangles.push(Triangle { vertices: [a, b, c], gradients: [-gb - gc, gb, gc], area: twice_area * 0.5 });
        shapes.push(([a, b, c], [DVec2::ZERO, DVec2::new(length, 0.0), DVec2::new(x, y)], 1.0));
    }
    if triangles.is_empty() {
        return Err(SceneError::params("minimum stretch needs a surface"));
    }
    let total_area: f64 = triangles.iter().map(|t| t.area).sum();
    for triangle in &mut triangles {
        triangle.area /= total_area;
    }
    let anchors = if pins.len() >= 2 {
        pins.to_vec()
    } else {
        let (a, b) = farthest_pair(points);
        let distance = points.get(a).zip(points.get(b)).map_or(1.0, |(a, b)| a.distance(*b));
        vec![(a, DVec2::ZERO), (b, DVec2::new(distance, 0.0))]
    };
    let mut uv = lscm_shapes(points.len(), &shapes, &anchors);
    if pins.len() == 1
        && let Some(&(vertex, target)) = pins.first()
    {
        let offset = target - uv.get(vertex).copied().unwrap_or_default();
        for point in &mut uv {
            *point += offset;
        }
    }
    if !energy(&triangles, &uv).is_finite() {
        if !pins.is_empty() {
            return Err(SceneError::params("the pins do not give minimum stretch an unfolded starting chart"));
        }
        uv = tutte(points.len(), indices)?;
        let flat: f64 = triangles.iter().map(|t| t.area * t.jacobian(&uv).determinant()).sum();
        if flat > 0.0 {
            for point in &mut uv {
                *point /= flat.sqrt();
            }
        }
    }
    if !pins.is_empty() {
        // A UV chart has no physical unit. Normalize source area to the initial constrained
        // UV area so the pinned result is invariant under changing the mesh's unit scale.
        let ratio: f64 = triangles.iter().map(|t| t.area * t.jacobian(&uv).determinant()).sum();
        if !ratio.is_finite() || ratio <= 1e-14 {
            return Err(SceneError::params("the pins collapse the chart"));
        }
        let scale = ratio.sqrt();
        for triangle in &mut triangles {
            for gradient in &mut triangle.gradients {
                *gradient /= scale;
            }
        }
    }
    let fixed: BTreeMap<usize, DVec2> = pins.iter().copied().collect();
    let mut slots = vec![None; points.len()];
    let mut count = 0;
    for (vertex, slot) in slots.iter_mut().enumerate() {
        if !fixed.contains_key(&vertex) {
            *slot = Some(count);
            count += 2;
        }
    }
    let mut old_energy = energy(&triangles, &uv);
    if !old_energy.is_finite() {
        return Err(SceneError::params("minimum stretch needs an orientable chart with a boundary; mark seams first"));
    }
    for _ in 0..iterations {
        if !pins.is_empty() {
            // Pins set a UV frame, not a physical texel density. Minimize over the uniform
            // source scale as well: D/s^2 + I*s^2 has its minimum at s=(D/I)^(1/4).
            // This is the same symmetric-Dirichlet objective modulo uniform UV scaling.
            let (mut direct, mut inverse) = (0.0, 0.0);
            for triangle in &triangles {
                let j = triangle.jacobian(&uv);
                let norm = j.x_axis.length_squared() + j.y_axis.length_squared();
                direct += triangle.area * norm;
                inverse += triangle.area * norm / j.determinant().powi(2);
            }
            let scale = (direct / inverse).sqrt().sqrt();
            if scale.is_finite() && scale > 1e-12 {
                for triangle in &mut triangles {
                    for gradient in &mut triangle.gradients {
                        *gradient /= scale;
                    }
                }
                old_energy = energy(&triangles, &uv);
            }
        }
        let mut rows = Vec::with_capacity(triangles.len() * 4 + count);
        for triangle in &triangles {
            let jacobian = triangle.jacobian(&uv);
            let (weight, rotation) = local(jacobian);
            let target = weight * rotation;
            let scale = triangle.area.sqrt();
            for column in 0..2 {
                for row in 0..2 {
                    let mut entries = Vec::with_capacity(6);
                    let mut rhs = target.col(column).to_array().get(row).copied().unwrap_or(0.0) * scale;
                    for (vertex, gradient) in triangle.vertices.iter().zip(triangle.gradients) {
                        let derivative = gradient.to_array().get(column).copied().unwrap_or(0.0);
                        for axis in 0..2 {
                            let coefficient = weight.col(axis).to_array().get(row).copied().unwrap_or(0.0) * derivative * scale;
                            if let Some(point) = fixed.get(vertex) {
                                rhs -= coefficient * point.to_array().get(axis).copied().unwrap_or(0.0);
                            } else if let Some(Some(slot)) = slots.get(*vertex) {
                                entries.push((slot + axis, coefficient));
                            }
                        }
                    }
                    rows.push((entries, rhs));
                }
            }
        }
        let mut x = vec![0.0; count];
        for (vertex, point) in uv.iter().enumerate() {
            if let Some(Some(slot)) = slots.get(vertex) {
                for (axis, value) in point.to_array().iter().enumerate() {
                    if let Some(entry) = x.get_mut(slot + axis) {
                        *entry = *value;
                    }
                    // Equation 6: a proximal term removes the free chart's rigid nullspace.
                    rows.push((vec![(slot + axis, 0.01)], value * 0.01));
                }
            }
        }
        let solution = conjugate_gradient(&rows, count, &mut x);
        let candidate: Vec<DVec2> = uv
            .iter()
            .enumerate()
            .map(|(vertex, point)| match slots.get(vertex) {
                Some(Some(slot)) => DVec2::new(solution.get(*slot).copied().unwrap_or(point.x), solution.get(slot + 1).copied().unwrap_or(point.y)),
                _ => *point,
            })
            .collect();
        let mut step = maximum_step(&triangles, &uv, &candidate).min(1.0);
        let mut accepted = None;
        for _ in 0..40 {
            let next: Vec<DVec2> = uv.iter().zip(&candidate).map(|(old, new)| old.lerp(*new, step)).collect();
            let next_energy = energy(&triangles, &next);
            if next_energy.is_finite() && next_energy < old_energy {
                accepted = Some((next, next_energy));
                break;
            }
            step *= 0.5;
        }
        let Some((next, next_energy)) = accepted else { break };
        let improvement = old_energy - next_energy;
        uv = next;
        old_energy = next_energy;
        if improvement < old_energy * 1e-10 {
            break;
        }
    }
    if uv.iter().any(|point| !point.is_finite() || point.abs().max_element() >= 1e6) {
        return Err(SceneError::params("minimum stretch produced coordinates outside the supported range"));
    }
    Ok(uv)
}

fn energy(triangles: &[Triangle], uv: &[DVec2]) -> f64 {
    triangles
        .iter()
        .map(|triangle| {
            let j = triangle.jacobian(uv);
            let det = j.determinant();
            if !j.is_finite() || det <= 1e-14 {
                return f64::INFINITY;
            }
            let norm = j.x_axis.length_squared() + j.y_axis.length_squared();
            triangle.area * norm * (1.0 + 1.0 / (det * det))
        })
        .sum()
}

/// A 2D polar rotation and the matrix W from equation 20. For a singular value s,
/// W's eigenvalue is sqrt((s - s^-3)/(s - 1)) = sqrt(1 + s^-1 + s^-2 + s^-3).
/// The polynomial identity also handles s = 1 without cancellation.
fn local(j: DMat2) -> (DMat2, DMat2) {
    let turn = DVec2::new(j.x_axis.x + j.y_axis.y, j.x_axis.y - j.y_axis.x).normalize_or_zero();
    let rotation = DMat2::from_cols(turn, DVec2::new(-turn.y, turn.x));
    let a = j.x_axis.x.powi(2) + j.y_axis.x.powi(2);
    let b = j.x_axis.x * j.x_axis.y + j.y_axis.x * j.y_axis.y;
    let d = j.x_axis.y.powi(2) + j.y_axis.y.powi(2);
    let angle = 0.5 * (2.0 * b).atan2(a - d);
    let (sin, cos) = angle.sin_cos();
    let u = DVec2::new(cos, sin);
    let v = DVec2::new(-sin, cos);
    let largest = (0.5 * (a + d + (a - d).hypot(2.0 * b))).sqrt().max(1e-12);
    let smallest = (j.determinant() / largest).max(1e-12);
    let weight = |s: f64| {
        let r = 1.0 / s;
        (1.0 + r + r * r + r * r * r).sqrt()
    };
    let w = DMat2::from_cols(u * u.x, u * u.y) * weight(largest) + DMat2::from_cols(v * v.x, v * v.y) * weight(smallest);
    (w, rotation)
}

/// det(J + t D) is quadratic. Keep strictly before its earliest positive zero,
/// even when the endpoint itself would have positive determinant after two flips.
fn maximum_step(triangles: &[Triangle], old: &[DVec2], new: &[DVec2]) -> f64 {
    let mut bound = f64::INFINITY;
    for triangle in triangles {
        let j = triangle.jacobian(old);
        let d = triangle.jacobian(new) - j;
        let a = d.determinant();
        let b = j.x_axis.perp_dot(d.y_axis) + d.x_axis.perp_dot(j.y_axis);
        let c = j.determinant();
        let mut roots = Vec::new();
        if a.abs() <= 1e-20 {
            if b < 0.0 {
                roots.push(-c / b);
            }
        } else {
            let discriminant = b * b - 4.0 * a * c;
            if discriminant >= 0.0 {
                let q = -0.5 * (b + discriminant.sqrt().copysign(b));
                roots.push(q / a);
                if q != 0.0 {
                    roots.push(c / q);
                }
            }
        }
        for root in roots {
            if root.is_finite() && root > 0.0 {
                bound = bound.min(root * 0.8);
            }
        }
    }
    bound
}

/// Positive-weight Tutte embedding for a disk, used only when LSCM is not injective.
fn tutte(count: usize, triangles: &[[usize; 3]]) -> Result<Vec<DVec2>> {
    let mut edges = BTreeMap::new();
    let mut neighbours = vec![BTreeSet::new(); count];
    for &[a, b, c] in triangles {
        for (a, b) in [(a, b), (b, c), (c, a)] {
            let edge = edges.entry((a.min(b), a.max(b))).or_insert((0usize, (a, b)));
            edge.0 += 1;
            if let Some(list) = neighbours.get_mut(a) {
                list.insert(b);
            }
            if let Some(list) = neighbours.get_mut(b) {
                list.insert(a);
            }
        }
    }
    let boundary: BTreeMap<usize, usize> = edges.values().filter(|(n, _)| *n == 1).map(|(_, edge)| *edge).collect();
    let Some((&first, _)) = boundary.first_key_value() else {
        return Err(SceneError::params("minimum stretch needs boundary seams"));
    };
    let mut ring = vec![first];
    let mut cursor = boundary.get(&first).copied().unwrap_or(first);
    while cursor != first && ring.len() <= boundary.len() {
        ring.push(cursor);
        cursor = boundary.get(&cursor).copied().ok_or_else(|| SceneError::params("minimum stretch needs an oriented manifold boundary"))?;
    }
    if ring.len() != boundary.len() {
        return Err(SceneError::params("this chart needs additional seams for an injective initialization"));
    }
    let fixed: BTreeMap<usize, DVec2> = ring
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let angle = std::f64::consts::TAU * i as f64 / ring.len() as f64;
            (*v, DVec2::new(angle.cos(), angle.sin()))
        })
        .collect();
    let mut uv = vec![DVec2::ZERO; count];
    let mut rows = Vec::new();
    for (vertex, list) in neighbours.iter().enumerate() {
        for axis in 0..2 {
            if let Some(point) = fixed.get(&vertex) {
                rows.push((vec![(vertex * 2 + axis, 1.0)], point.to_array().get(axis).copied().unwrap_or(0.0)));
            } else {
                let mut row = vec![(vertex * 2 + axis, 1.0)];
                for other in list {
                    row.push((other * 2 + axis, -1.0 / list.len() as f64));
                }
                rows.push((row, 0.0));
            }
        }
    }
    let mut x = vec![0.0; count * 2];
    let solved = conjugate_gradient(&rows, count * 2, &mut x);
    for (vertex, point) in uv.iter_mut().enumerate() {
        *point = DVec2::new(solved.get(vertex * 2).copied().unwrap_or(0.0), solved.get(vertex * 2 + 1).copied().unwrap_or(0.0));
    }
    Ok(uv)
}
