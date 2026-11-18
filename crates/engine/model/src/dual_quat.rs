//! Dual-quaternion blending of bone transforms ("preserve volume" skinning), as the reference
//! application blends them: each bone's skinning matrix is split into a rigid part (a dual
//! quaternion, following the posed bone's head and direction) and the scale and shear left
//! over; the rigid parts are
//! summed by weight (each turned to the same hemisphere as the first) and normalised, and a
//! point is scaled first, then moved rigidly.
//!
//! The scales are not averaged plainly: each bone's scale is weighted by its weight over the
//! length of the summed rotations, and turned by the bone's rotation relative to the blended one
//! as a quaternion product does (`c·p + v × p` for the relative rotation `(c, v)`). This is what
//! the reference's results fit, to float precision, on bones both turned and scaled
//! (`tests/rig5_blender.rs`); with no scaled bone the sum is the identity. An Armature
//! constraint's owner moves as such a point, and its axes take the scales averaged plainly by
//! weight, as the reference's do.

use glam::{Mat3, Mat4, Quat, Vec3, Vec4};

/// One bone's part of a blend.
#[derive(Debug, Clone, Copy)]
pub struct Part {
    /// The rotation.
    pub real: Quat,
    /// Half the translation times the rotation.
    pub dual: Quat,
    /// What is left of the skinning matrix after the rigid part (scale and shear, applied
    /// first), when the bone is scaled or sheared.
    pub scale: Option<Mat4>,
}

/// The part of a bone whose skinning matrix is `skin` (pose times inverse rest) and whose rest
/// matrix is `rest`, both in the same space. The rigid part keeps the posed bone's head and its
/// Y axis (the bone's direction), with X made square to it and Z square to both; the scale is
/// the rest of the skinning matrix.
pub fn part(skin: Mat4, rest: Mat4) -> Part {
    let posed = skin * rest;
    let axes = [posed.x_axis.truncate(), posed.y_axis.truncate(), posed.z_axis.truncate()];
    let square = axes.iter().all(|axis| (axis.length() - 1.0).abs() <= 1e-6)
        && axes[0].dot(axes[1]).abs() <= 1e-6
        && axes[1].dot(axes[2]).abs() <= 1e-6
        && axes[0].dot(axes[2]).abs() <= 1e-6;
    let y = axes[1].normalize_or_zero();
    let x = (axes[0] - y * axes[0].dot(y)).normalize_or_zero();
    let (rigid, scale) = if !square && y != Vec3::ZERO && x != Vec3::ZERO {
        let rotation = Mat3::from_cols(x, y, x.cross(y));
        let base = Mat4::from_mat3(rotation).with_w(posed.w_axis);
        let rigid = base * rest.inverse();
        (rigid, Some(rigid.inverse() * skin))
    } else {
        (skin, None)
    };
    let (_, rotation, translation) = rigid.to_scale_rotation_translation();
    let real = rotation.normalize();
    let dual = Quat::from_xyzw(translation.x, translation.y, translation.z, 0.0) * real * 0.5;
    Part { real, dual, scale }
}

trait WithW {
    fn with_w(self, w: Vec4) -> Self;
}

impl WithW for Mat4 {
    fn with_w(mut self, w: Vec4) -> Self {
        self.w_axis = w;
        self
    }
}

/// A blend of bones: the rigid matrix, and the scale applied before it to points and to
/// directions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Blend {
    pub rigid: Mat4,
    /// For points: each bone's scale turned by its rotation relative to the blended one.
    pub scale: Mat4,
    /// For directions (an owner's axes): the bones' scales averaged by weight.
    pub linear: Mat4,
}

/// The blend of weighted parts. `None` when the weights sum to nothing.
pub fn blend(parts: &[(f32, Part)]) -> Option<Blend> {
    let first = parts.iter().find(|(weight, _)| *weight > 0.0)?.1.real;
    let mut real = Vec4::ZERO;
    let mut dual = Vec4::ZERO;
    let mut total = 0.0;
    let mut signed = Vec::with_capacity(parts.len());
    for (weight, part) in parts.iter().filter(|(weight, _)| *weight > 0.0) {
        let sign = if Vec4::from(part.real).dot(Vec4::from(first)) < 0.0 { -1.0 } else { 1.0 };
        real += Vec4::from(part.real) * *weight * sign;
        dual += Vec4::from(part.dual) * *weight * sign;
        total += *weight;
        signed.push((*weight, sign, part));
    }
    let length = real.length();
    if total <= 0.0 || length <= 1e-12 {
        return None;
    }
    let real = Quat::from_vec4(real / length);
    let dual = Quat::from_vec4(dual / length);
    let moved = dual * real.conjugate() * 2.0;
    let rigid = Mat4::from_rotation_translation(real, Vec3::new(moved.x, moved.y, moved.z));
    let mut scale = Mat4::ZERO;
    let mut linear = Mat4::ZERO;
    for (weight, sign, part) in signed {
        let own = part.scale.unwrap_or(Mat4::IDENTITY);
        let relative = real.conjugate() * part.real;
        let turn = Mat4::from_mat3(Mat3::from_cols(
            Vec3::new(relative.w, relative.z, -relative.y),
            Vec3::new(-relative.z, relative.w, relative.x),
            Vec3::new(relative.y, -relative.x, relative.w),
        ));
        scale += turn * own * (weight * sign / length);
        linear += own * (weight / total);
    }
    scale.w_axis.w = 1.0;
    Some(Blend { rigid, scale, linear })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_rigid_bone_is_its_own_matrix() {
        let skin = Mat4::from_rotation_translation(Quat::from_euler(glam::EulerRot::XYZ, 0.3, -0.2, 0.9), Vec3::new(1.0, 2.0, -0.5));
        let rest = Mat4::from_translation(Vec3::new(0.0, 0.0, 1.0));
        let blended = blend(&[(0.7, part(skin, rest))]).unwrap();
        assert!(blended.rigid.abs_diff_eq(skin, 1e-5));
        assert!(blended.scale.abs_diff_eq(Mat4::IDENTITY, 1e-6));
    }

    #[test]
    fn opposite_turns_do_not_collapse() {
        // Two bones turned ±90° about Z: linear blending shrinks a point to the axis, dual
        // quaternions keep its distance.
        let rest = Mat4::IDENTITY;
        let a = part(Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2), rest);
        let b = part(Mat4::from_rotation_z(-std::f32::consts::FRAC_PI_2), rest);
        let blended = blend(&[(0.5, a), (0.5, b)]).unwrap();
        let point = blended.rigid.transform_point3(Vec3::X);
        assert!((point.length() - 1.0).abs() < 1e-5, "{point}");
    }
}
