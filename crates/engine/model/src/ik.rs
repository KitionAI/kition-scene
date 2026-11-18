//! The solver of the IK constraint: selectively damped least squares on a bone chain (see
//! [`solve_chain`]).

use glam::{Mat3, Mat4, Quat, Vec3};

/// Bends a chain (world matrices, root first, each bone's head at its origin and its Y axis
/// along it) so the tip's tail (`tip_length` along the last bone's Y) reaches `goal`, starting
/// from the current pose; then turns it about the root→goal line for a pole. Returns the new
/// world matrices.
///
/// The solver is selectively damped least squares (Buss and Kim, 2005) on three rotations per
/// bone about the bone's own axes, with the goal error clamped to a fifth of the chain's length
/// per step and a one-radian step limit: from the same starting pose it lands where the
/// reference application's solver does (the bend side, and for longer chains how the bend is
/// shared), and an unreachable goal straightens the chain towards it.
pub fn solve_chain(worlds: &[Mat4], tip_length: f32, goal: Vec3, pole: Option<Vec3>, pole_angle: f32, iterations: u32) -> Vec<Mat4> {
    let count = worlds.len();
    let mut rotations: Vec<Quat> = Vec::with_capacity(count);
    let mut scales: Vec<Vec3> = Vec::with_capacity(count);
    let mut heads: Vec<Vec3> = Vec::with_capacity(count);
    for world in worlds {
        let (scale, rotation, location) = world.to_scale_rotation_translation();
        rotations.push(rotation);
        scales.push(scale);
        heads.push(location);
    }
    // Each child's head in its parent's frame (rotation and scale removed), fixed while solving.
    let offsets: Vec<Vec3> = (1..count)
        .map(|index| {
            let (Some(parent), Some(head), Some(rotation), Some(scale)) =
                (heads.get(index - 1), heads.get(index), rotations.get(index - 1), scales.get(index - 1))
            else {
                return Vec3::ZERO;
            };
            (rotation.inverse() * (*head - *parent)) / *scale
        })
        .collect();
    let tip_offset = Vec3::new(0.0, tip_length, 0.0);
    let root = heads.first().copied().unwrap_or(Vec3::ZERO);
    let reach: f32 = offsets.iter().zip(&scales).map(|(offset, scale)| (*offset * *scale).length()).sum::<f32>()
        + (tip_offset * scales.last().copied().unwrap_or(Vec3::ONE)).length();
    let positions = |rotations: &[Quat]| -> (Vec<Vec3>, Vec3) {
        let mut out = vec![root];
        for index in 1..count {
            let (Some(previous), Some(rotation), Some(scale), Some(offset)) =
                (out.last().copied(), rotations.get(index - 1), scales.get(index - 1), offsets.get(index - 1))
            else {
                break;
            };
            out.push(previous + *rotation * (*scale * *offset));
        }
        let last = out.last().copied().unwrap_or(root);
        let tip = last + rotations.last().copied().unwrap_or(Quat::IDENTITY) * (scales.last().copied().unwrap_or(Vec3::ONE) * tip_offset);
        (out, tip)
    };
    const MAX_STEP: f32 = 1.0;
    let clamp = (reach * 0.2).max(1e-6);
    for _ in 0..iterations {
        let (joints, tip) = positions(&rotations);
        let mut error = goal - tip;
        if error.length() < 1e-6 {
            break;
        }
        error = error.clamp_length_max(clamp);
        // J is 3 × 3n: for bone j and its own axis a, a × (tip − head j).
        let columns: Vec<Vec3> = joints
            .iter()
            .zip(&rotations)
            .flat_map(|(joint, rotation)| [Vec3::X, Vec3::Y, Vec3::Z].map(|axis| (*rotation * axis).cross(tip - *joint)))
            .collect();
        let step = sdls(&columns, error, MAX_STEP);
        let mut accumulated = Quat::IDENTITY;
        for (index, rotation) in rotations.iter_mut().enumerate() {
            let local = Vec3::new(
                step.get(index * 3).copied().unwrap_or(0.0),
                step.get(index * 3 + 1).copied().unwrap_or(0.0),
                step.get(index * 3 + 2).copied().unwrap_or(0.0),
            );
            accumulated = Quat::from_scaled_axis(*rotation * local) * accumulated;
            *rotation = (accumulated * *rotation).normalize();
        }
    }
    if let Some(pole) = pole {
        let (_, tip) = positions(&rotations);
        let axis = (tip - root).normalize_or_zero();
        let first = rotations.first().copied().unwrap_or(Quat::IDENTITY);
        let along = |vector: Vec3| (vector - axis * vector.dot(axis)).normalize_or_zero();
        let x_axis = along(first * Vec3::X);
        let wanted = Quat::from_axis_angle(axis, pole_angle) * along(pole - root);
        if axis != Vec3::ZERO && x_axis != Vec3::ZERO && wanted != Vec3::ZERO {
            let angle = x_axis.cross(wanted).dot(axis).atan2(x_axis.dot(wanted));
            let turn = Quat::from_axis_angle(axis, angle);
            for rotation in &mut rotations {
                *rotation = (turn * *rotation).normalize();
            }
        }
    }
    let (joints, _) = positions(&rotations);
    joints
        .iter()
        .zip(rotations.iter().zip(scales.iter()))
        .map(|(head, (rotation, scale))| Mat4::from_scale_rotation_translation(*scale, *rotation, *head))
        .collect()
}

/// One selectively damped least-squares step for a single position goal: `columns` are the
/// Jacobian's columns (one per degree of freedom), `error` the goal's offset. Each singular
/// direction's step is limited by how far its rotations move the effector, and the whole step
/// by `max_step` radians per degree of freedom.
fn sdls(columns: &[Vec3], error: Vec3, max_step: f32) -> Vec<f32> {
    let mut jjt = Mat3::ZERO;
    for column in columns {
        jjt += Mat3::from_cols(*column * column.x, *column * column.y, *column * column.z);
    }
    let (values, vectors) = symmetric_eigen(jjt);
    let lengths: Vec<f32> = columns.iter().map(|column| column.length()).collect();
    let mut step = vec![0.0f32; columns.len()];
    for (value, u) in values.iter().zip(vectors.iter()) {
        if *value <= 1e-12 {
            continue;
        }
        let sigma = value.sqrt();
        // v = Jᵀ u / σ.
        let v: Vec<f32> = columns.iter().map(|column| column.dot(*u) / sigma).collect();
        let alpha = u.dot(error);
        let m = v.iter().zip(&lengths).map(|(vj, rho)| vj.abs() * rho).sum::<f32>() / sigma;
        let gamma = if m > 0.0 { (1.0 / m).min(1.0) * max_step } else { max_step };
        let mut w: Vec<f32> = v.iter().map(|vj| vj * alpha / sigma).collect();
        let largest = w.iter().fold(0.0f32, |most, value| most.max(value.abs()));
        if largest > gamma {
            for value in &mut w {
                *value *= gamma / largest;
            }
        }
        for (total, value) in step.iter_mut().zip(w) {
            *total += value;
        }
    }
    let largest = step.iter().fold(0.0f32, |most, value| most.max(value.abs()));
    if largest > max_step {
        for value in &mut step {
            *value *= max_step / largest;
        }
    }
    step
}

/// Eigenvalues and unit eigenvectors of a symmetric 3 × 3 matrix (cyclic Jacobi rotations).
fn symmetric_eigen(matrix: Mat3) -> ([f32; 3], [Vec3; 3]) {
    type M = [[f64; 3]; 3];
    fn at(m: &M, row: usize, column: usize) -> f64 {
        m.get(row).and_then(|line| line.get(column)).copied().unwrap_or(0.0)
    }
    fn put(m: &mut M, row: usize, column: usize, value: f64) {
        if let Some(slot) = m.get_mut(row).and_then(|line| line.get_mut(column)) {
            *slot = value;
        }
    }
    let column = |c: Vec3| [f64::from(c.x), f64::from(c.y), f64::from(c.z)];
    let (c0, c1, c2) = (column(matrix.x_axis), column(matrix.y_axis), column(matrix.z_axis));
    let mut a: M = [[c0[0], c1[0], c2[0]], [c0[1], c1[1], c2[1]], [c0[2], c1[2], c2[2]]];
    let mut v: M = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..32 {
        let off = at(&a, 0, 1).powi(2) + at(&a, 0, 2).powi(2) + at(&a, 1, 2).powi(2);
        if off < 1e-24 {
            break;
        }
        for (p, q) in [(0usize, 1usize), (0, 2), (1, 2)] {
            let apq = at(&a, p, q);
            if apq.abs() < 1e-30 {
                continue;
            }
            let theta = (at(&a, q, q) - at(&a, p, p)) / (2.0 * apq);
            let t = if theta == 0.0 { 1.0 } else { theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt()) };
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            for k in 0..3 {
                let (akp, akq) = (at(&a, k, p), at(&a, k, q));
                put(&mut a, k, p, c * akp - s * akq);
                put(&mut a, k, q, s * akp + c * akq);
            }
            for k in 0..3 {
                let (apk, aqk) = (at(&a, p, k), at(&a, q, k));
                put(&mut a, p, k, c * apk - s * aqk);
                put(&mut a, q, k, s * apk + c * aqk);
            }
            for k in 0..3 {
                let (vkp, vkq) = (at(&v, k, p), at(&v, k, q));
                put(&mut v, k, p, c * vkp - s * vkq);
                put(&mut v, k, q, s * vkp + c * vkq);
            }
        }
    }
    let vector = |c: usize| Vec3::new(at(&v, 0, c) as f32, at(&v, 1, c) as f32, at(&v, 2, c) as f32).normalize_or_zero();
    ([at(&a, 0, 0) as f32, at(&a, 1, 1) as f32, at(&a, 2, 2) as f32], [vector(0), vector(1), vector(2)])
}
