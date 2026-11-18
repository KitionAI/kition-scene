//! The arithmetic of the constraints that reshape their owner rather than aim it: Stretch To,
//! Floor, Transformation, Action, Armature, and Spline IK. Each works on matrices already in the
//! space the constraint reads (see [`crate::constraints`]); the behaviour follows what the
//! reference application does with the same settings, measured case by case.

use glam::{Mat3, Mat4, Quat, Vec3};

use crate::constraints::{ActionMix, KeepAxis, SplineXzScale, SplineYScale, StretchVolume, TrackAxis, TransformChannel, TransformMix};
use crate::scene::euler_matrix;

/// Location, rotation (an orthonormal matrix), and per-axis scale of a matrix: the column
/// lengths, so a mirrored matrix keeps a turn.
pub fn split(matrix: Mat4) -> (Vec3, Mat3, Vec3) {
    let axes = [matrix.x_axis.truncate(), matrix.y_axis.truncate(), matrix.z_axis.truncate()];
    let mut scale = Vec3::new(axes[0].length(), axes[1].length(), axes[2].length());
    if Mat3::from_cols(axes[0], axes[1], axes[2]).determinant() < 0.0 {
        scale = -scale;
    }
    let column = |axis: Vec3, length: f32| if length.abs() > 1e-12 { axis / length } else { Vec3::ZERO };
    let rotation = Mat3::from_cols(column(axes[0], scale.x), column(axes[1], scale.y), column(axes[2], scale.z));
    let rotation = Mat3::from_quat(Quat::from_mat3(&rotation).normalize());
    (matrix.w_axis.truncate(), rotation, scale)
}

/// XYZ Euler angles of a matrix whose columns are unit length but perhaps not square to each
/// other (a sheared transform's), read as the reference application reads them: from the first
/// column and the last row, choosing the smaller of the two solutions.
pub fn euler_of(m: Mat3) -> Vec3 {
    let (c0, c1, c2) = (m.x_axis, m.y_axis, m.z_axis);
    let cy = c0.x.hypot(c0.y);
    if cy > 16.0 * f32::EPSILON {
        let first = Vec3::new(c1.z.atan2(c2.z), (-c0.z).atan2(cy), c0.y.atan2(c0.x));
        let second = Vec3::new((-c1.z).atan2(-c2.z), (-c0.z).atan2(-cy), (-c0.y).atan2(-c0.x));
        if first.abs().element_sum() > second.abs().element_sum() { second } else { first }
    } else {
        Vec3::new((-c2.y).atan2(c1.y), (-c0.z).atan2(cy), 0.0)
    }
}

/// The columns of a matrix's 3 × 3 part made unit length (a rotation when it has no shear).
pub fn normalized(matrix: Mat4) -> Mat3 {
    let unit = |axis: Vec3| axis.normalize_or_zero();
    Mat3::from_cols(unit(matrix.x_axis.truncate()), unit(matrix.y_axis.truncate()), unit(matrix.z_axis.truncate()))
}

/// A matrix from location, rotation, and scale.
pub fn compose(location: Vec3, rotation: Mat3, scale: Vec3) -> Mat4 {
    Mat4::from_translation(location) * Mat4::from_mat3(rotation) * Mat4::from_scale(scale)
}

/// The three components of a channel of `matrix`: location, XYZ Euler angles, or scale.
pub fn channel_values(matrix: Mat4, channel: TransformChannel) -> Vec3 {
    let (location, _, scale) = split(matrix);
    match channel {
        TransformChannel::Location => location,
        TransformChannel::Rotation => euler_of(normalized(matrix)),
        TransformChannel::Scale => scale.abs(),
    }
}

/// Stretch To: the owner's Y axis turned to the target and stretched to reach it, the other
/// two axes thinned (or thickened) by `bulge` to keep its volume. `rest_length` is the
/// distance at which nothing stretches, in the owner's Y units (0 reads as no stretch).
pub fn stretch_to(owner: Mat4, target: Vec3, rest_length: f32, bulge: f32, volume: StretchVolume, keep: KeepAxis) -> Option<Mat4> {
    let (location, _, _) = split(owner);
    let axes = [owner.x_axis.truncate(), owner.y_axis.truncate(), owner.z_axis.truncate()];
    let sizes = Vec3::new(axes[0].length(), axes[1].length(), axes[2].length());
    let offset = target - location;
    let distance = offset.length();
    if distance <= 1e-9 || sizes.min_element() <= 1e-12 {
        return None;
    }
    let direction = offset / distance;
    let stretch = if rest_length > 1e-9 { distance / (rest_length * sizes.y) } else { 1.0 };
    let factor = (1.0 / stretch).powf(bulge);
    let (x_scale, z_scale) = match volume {
        StretchVolume::Xzx => (factor.sqrt(), factor.sqrt()),
        StretchVolume::X => (factor, 1.0),
        StretchVolume::Z => (1.0, factor),
        StretchVolume::None => (1.0, 1.0),
    };
    let (x, y, z) = (axes[0] / sizes.x, axes[1] / sizes.y, axes[2] / sizes.z);
    let (x, z) = match keep {
        KeepAxis::SwingY => {
            let turn = Quat::from_rotation_arc(y.normalize_or_zero(), direction);
            (turn * x, turn * z)
        }
        KeepAxis::PlaneX => {
            let z = x.cross(direction).normalize_or_zero();
            (direction.cross(z), z)
        }
        KeepAxis::PlaneZ => {
            let x = direction.cross(z).normalize_or_zero();
            (x, x.cross(direction))
        }
    };
    if x == Vec3::ZERO || z == Vec3::ZERO {
        return None;
    }
    Some(Mat4::from_cols(
        (x * sizes.x * x_scale).extend(0.0),
        (direction * sizes.y * stretch).extend(0.0),
        (z * sizes.z * z_scale).extend(0.0),
        location.extend(1.0),
    ))
}

/// Floor: keeps the owner's location on one side of a plane through the target (along a world
/// axis, or with `rotation` along the target's own), `offset` beyond it.
pub fn floor(owner: Mat4, target: Mat4, axis: TrackAxis, offset: f32, rotation: bool) -> Mat4 {
    let (index, sign) = axis.parts();
    let target_location = target.w_axis.truncate();
    let turn = if rotation { split(target).1 } else { Mat3::IDENTITY };
    let mut local = (turn.transpose() * (owner.w_axis.truncate() - target_location)).to_array();
    if let Some(value) = local.get_mut(index) {
        // Positive axes keep the owner above the plane, negative ones below it.
        *value = if sign > 0.0 { value.max(offset) } else { value.min(offset) };
    }
    let mut out = owner;
    out.w_axis = (target_location + turn * Vec3::from_array(local)).extend(1.0);
    out
}

/// The mix a Transformation constraint uses for `to` when none is chosen (the reference
/// application's defaults: add for location and rotation, replace for scale).
pub fn default_transform_mix(to: TransformChannel) -> TransformMix {
    match to {
        TransformChannel::Location | TransformChannel::Rotation => TransformMix::Add,
        TransformChannel::Scale => TransformMix::Replace,
    }
}

/// Transformation: maps a channel of the target (location, rotation, or scale, each axis
/// through its range) onto a channel of the owner. `mapping[i]` is the target axis the owner's
/// axis `i` reads. Values outside the source range clamp unless `extrapolate`.
#[allow(clippy::too_many_arguments)]
pub fn transform(
    owner: Mat4,
    target: Mat4,
    from: TransformChannel,
    to: TransformChannel,
    from_range: (Vec3, Vec3),
    to_range: (Vec3, Vec3),
    mapping: [u8; 3],
    extrapolate: bool,
    mix: TransformMix,
) -> Mat4 {
    let source = channel_values(target, from).to_array();
    let (from_min, from_max) = (from_range.0.to_array(), from_range.1.to_array());
    let (to_min, to_max) = (to_range.0.to_array(), to_range.1.to_array());
    let mut out = [0.0f32; 3];
    for (axis, slot) in out.iter_mut().enumerate() {
        let read = usize::from(mapping.get(axis).copied().unwrap_or(axis as u8).min(2));
        let value = source.get(read).copied().unwrap_or(0.0);
        let (low, high) = (from_min.get(read).copied().unwrap_or(0.0), from_max.get(read).copied().unwrap_or(0.0));
        let mut t = if (high - low).abs() > 1e-12 { (value - low) / (high - low) } else { 0.0 };
        if !extrapolate {
            t = t.clamp(0.0, 1.0);
        }
        let (start, end) = (to_min.get(axis).copied().unwrap_or(0.0), to_max.get(axis).copied().unwrap_or(0.0));
        *slot = start + t * (end - start);
    }
    let out = Vec3::from_array(out);
    let (location, _, scale) = split(owner);
    let rotation = euler_matrix(euler_of(normalized(owner)));
    match to {
        TransformChannel::Location => {
            // Only the location changes; a sheared owner keeps its shear.
            let mut moved = owner;
            moved.w_axis = (if mix == TransformMix::Replace { out } else { location + out }).extend(1.0);
            moved
        }
        TransformChannel::Rotation => {
            let turn = euler_matrix(out);
            let rotation = match mix {
                TransformMix::Replace => turn,
                TransformMix::Before => turn * rotation,
                TransformMix::After => rotation * turn,
                _ => euler_matrix(euler_of(normalized(owner)) + out),
            };
            compose(location, rotation, scale)
        }
        TransformChannel::Scale => {
            let scale = if mix == TransformMix::Multiply { scale * out } else { out };
            compose(location, rotation, scale)
        }
    }
}

/// How far into the action a target value puts the owner: `value` mapped from `min`..`max`
/// to `start`..`end`, clamped.
pub fn action_frame(value: f32, min: f32, max: f32, start: f32, end: f32) -> f32 {
    let t = if (max - min).abs() > 1e-12 { ((value - min) / (max - min)).clamp(0.0, 1.0) } else { 0.0 };
    start + t * (end - start)
}

/// Action: combines the action's transform `action` with the owner's own `local` one (both in
/// the owner's local space).
pub fn action_mix(local: Mat4, action: Mat4, mix: ActionMix) -> Mat4 {
    // Location from the full product, rotations multiplied, scales multiplied per axis.
    let aligned = |parent: Mat4, child: Mat4| {
        let (_, parent_rotation, parent_scale) = split(parent);
        let (child_location, child_rotation, child_scale) = split(child);
        compose(parent.transform_point3(child_location), parent_rotation * child_rotation, parent_scale * child_scale)
    };
    // Locations added, rotations multiplied, scales multiplied.
    let channels = |first: Mat4, second: Mat4| {
        let (first_location, first_rotation, first_scale) = split(first);
        let (second_location, second_rotation, second_scale) = split(second);
        compose(first_location + second_location, first_rotation * second_rotation, first_scale * second_scale)
    };
    match mix {
        ActionMix::Replace => action,
        ActionMix::BeforeFull => action * local,
        ActionMix::Before => aligned(action, local),
        ActionMix::BeforeSplit => channels(action, local),
        ActionMix::AfterFull => local * action,
        ActionMix::After => aligned(local, action),
        ActionMix::AfterSplit => channels(local, action),
    }
}

/// Armature: the owner moved by bones as a mesh's vertex would be. Each entry is a weight, the
/// bone's posed world matrix, and its rest world matrix. Weights are normalised; with
/// `preserve_volume` the bones blend as dual quaternions.
pub fn armature(owner: Mat4, bones: &[(f32, Mat4, Mat4)], preserve_volume: bool) -> Option<Mat4> {
    let bones: Vec<&(f32, Mat4, Mat4)> = bones.iter().filter(|(weight, _, _)| *weight > 0.0).collect();
    let total: f32 = bones.iter().map(|(weight, _, _)| *weight).sum();
    if total <= 1e-9 {
        return None;
    }
    if preserve_volume {
        let parts: Vec<(f32, crate::dual_quat::Part)> =
            bones.iter().map(|(weight, pose, rest)| (*weight, crate::dual_quat::part(*pose * rest.inverse(), *rest))).collect();
        let blend = crate::dual_quat::blend(&parts)?;
        let location = (blend.rigid * blend.scale).transform_point3(owner.w_axis.truncate());
        let mut moved = blend.rigid * blend.linear * owner;
        moved.w_axis = location.extend(1.0);
        return Some(moved);
    }
    let mut sum = Mat4::ZERO;
    for (weight, pose, rest) in &bones {
        sum += *pose * rest.inverse() * owner * *weight;
    }
    Some(sum * (1.0 / total))
}

/// One bone of a Spline IK chain, root first: its posed world matrix and rest length (its
/// length times its own Y scale for `BoneOriginal`).
#[derive(Debug, Clone, Copy)]
pub struct ChainBone {
    pub world: Mat4,
    pub length: f32,
}

/// A path along a curve, in world space: points and the radius at each.
#[derive(Debug, Clone, Default)]
pub struct Path {
    pub points: Vec<Vec3>,
    pub radii: Vec<f32>,
}

impl Path {
    fn lengths(&self) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.points.len());
        let mut total = 0.0;
        for (index, point) in self.points.iter().enumerate() {
            if let Some(previous) = index.checked_sub(1).and_then(|before| self.points.get(before)) {
                total += point.distance(*previous);
            }
            out.push(total);
        }
        out
    }

    /// The point and radius `distance` along the path (past the end, along the last segment).
    fn at(&self, lengths: &[f32], distance: f32) -> (Vec3, f32) {
        let count = self.points.len();
        let point = |index: usize| self.points.get(index).copied().unwrap_or(Vec3::ZERO);
        let radius = |index: usize| self.radii.get(index).copied().unwrap_or(1.0);
        let length = |index: usize| lengths.get(index).copied().unwrap_or(0.0);
        if count < 2 {
            return (point(0), radius(0));
        }
        let slot = lengths.iter().rposition(|at| *at <= distance).unwrap_or(0).min(count - 2);
        let span = length(slot + 1) - length(slot);
        let t = if span > 1e-12 { (distance - length(slot)) / span } else { 0.0 };
        let t = if slot == 0 { t.max(0.0) } else { t };
        (point(slot).lerp(point(slot + 1), t), radius(slot) + (radius(slot + 1) - radius(slot)) * t.clamp(0.0, 1.0))
    }
}

/// Spline IK: fits a chain (root first) along a path. Joints sit at fractions of the path's
/// length (by bone length, or evenly), the root at its start (or, with `chain_offset`, where it
/// is now); each bone turns from its parent's new frame by the shortest arc onto its segment,
/// and its scale follows the modes. Returns the new world matrices.
#[allow(clippy::too_many_arguments)]
pub fn spline_ik(
    chain: &[ChainBone],
    path: &Path,
    even: bool,
    chain_offset: bool,
    curve_radius: bool,
    y_scale: SplineYScale,
    xz_scale: SplineXzScale,
    bulge: f32,
) -> Vec<Mat4> {
    if chain.is_empty() || path.points.len() < 2 {
        return chain.iter().map(|bone| bone.world).collect();
    }
    let lengths = path.lengths();
    let total_path = lengths.last().copied().unwrap_or(0.0);
    let total_chain: f32 = chain.iter().map(|bone| bone.length).sum();
    let count = chain.len() as f32;
    // Distances of each joint along the path.
    let mut joints = Vec::with_capacity(chain.len() + 1);
    let mut walked = 0.0;
    joints.push(0.0);
    for bone in chain {
        walked += match (y_scale, even) {
            (SplineYScale::FitCurve, true) => total_path / count,
            (SplineYScale::FitCurve, false) if total_chain > 1e-9 => bone.length / total_chain * total_path,
            (SplineYScale::FitCurve, false) => total_path / count,
            (_, true) if total_chain > 1e-9 => total_chain / count,
            (SplineYScale::BoneOriginal, _) => bone.length * split(bone.world).2.y.abs(),
            _ => bone.length,
        };
        joints.push(walked);
    }
    let shift =
        if chain_offset { chain.first().map_or(Vec3::ZERO, |bone| bone.world.w_axis.truncate()) - path.at(&lengths, 0.0).0 } else { Vec3::ZERO };
    let points: Vec<(Vec3, f32)> = joints
        .iter()
        .map(|distance| {
            let (point, radius) = path.at(&lengths, *distance);
            (point + shift, radius)
        })
        .collect();
    let mut out = Vec::with_capacity(chain.len());
    // Each bone starts from where its parent's new turn puts it, then turns onto its segment.
    let mut carried = Mat3::IDENTITY;
    for (index, bone) in chain.iter().enumerate() {
        let (Some((head, head_radius)), Some((tail, tail_radius))) = (points.get(index).copied(), points.get(index + 1).copied()) else { break };
        let segment = tail - head;
        let length = segment.length();
        let (_, own_rotation, own_scale) = split(bone.world);
        let start = carried * own_rotation;
        let direction = if length > 1e-9 { segment / length } else { start.y_axis };
        let frame = Mat3::from_quat(Quat::from_rotation_arc(start.y_axis.normalize_or_zero(), direction)) * start;
        carried = frame * own_rotation.transpose();
        let rest = if bone.length > 1e-9 { bone.length } else { 1.0 };
        let y = match y_scale {
            SplineYScale::None => 1.0,
            SplineYScale::BoneOriginal => own_scale.y,
            SplineYScale::FitCurve => length / rest * own_scale.y.signum(),
        };
        let stretch = if own_scale.y.abs() > 1e-9 { y / own_scale.y } else { y };
        let mut xz = match xz_scale {
            SplineXzScale::None => Vec3::new(1.0, 1.0, 1.0),
            SplineXzScale::BoneOriginal => Vec3::new(own_scale.x, 1.0, own_scale.z),
            SplineXzScale::InversePreserve => {
                let value = if stretch.abs() > 1e-9 { 1.0 / stretch } else { 1.0 };
                Vec3::new(value, 1.0, value)
            }
            SplineXzScale::VolumePreserve => {
                let value = if stretch.abs() > 1e-9 { (1.0 / stretch.abs()).powf(bulge).sqrt() } else { 1.0 };
                Vec3::new(value, 1.0, value)
            }
        };
        if curve_radius {
            let radius = (head_radius + tail_radius) * 0.5;
            xz.x *= radius;
            xz.z *= radius;
        }
        out.push(compose(head, frame, Vec3::new(xz.x, y, xz.z)));
    }
    out
}
