//! Constraints: rules that override where an object or a bone ends up, evaluated after its own
//! transform (and its parent's), in world space, each blended in by its influence. They follow
//! the reference application's defaults: Track To points −Z at the target with +Y up, Damped
//! Track turns +Y towards it by the shortest arc, Child Of keeps the owner where it was when it
//! was added (its inverse matrix), and Inverse Kinematics bends a bone chain until the tip's tail
//! reaches the target, starting from the current pose, then turns the chain about the line from
//! its root to the target so the root bone's X axis points at the pole (turned by the pole angle).
//! Stretch To, Floor, Transformation, Action, Armature, and Spline IK are worked out in
//! [`crate::constraint_math`]. Copy Location, Copy Rotation, Limit Rotation, Transformation, and
//! Action read their owner and target in a [`Space`]: the matrices are taken into it, the
//! constraint works there, and the result goes back to world space.
//!
//! Objects are evaluated through [`world_matrix`], which [`Scene::world_matrix`] calls; bones
//! through [`bone_matrices`]. A constraint whose target is missing does nothing. Dependencies
//! (parents, targets, bones in the same armature) must not form a cycle: [`check_cycles`]
//! refuses such a scene, and evaluation stops following targets past a fixed depth, so a cycle
//! that got in anyway cannot recurse forever.

use std::collections::{BTreeMap, BTreeSet};

use glam::{Mat3, Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::armature::{Armature, armature_of};
use crate::error::{Result, SceneError};
use crate::scene::{Id, ObjectData, Scene, euler_matrix, matrix_euler};

/// How deep evaluation follows targets of targets.
const MAX_DEPTH: u32 = 12;

/// What a constraint points at: an object, or a bone of an armature object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub object: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bone: Option<String>,
}

/// An axis of the owner, signed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackAxis {
    X,
    #[default]
    Y,
    Z,
    NegX,
    NegY,
    NegZ,
}

impl TrackAxis {
    /// The axis index (0 x, 1 y, 2 z) and its sign.
    pub fn parts(self) -> (usize, f32) {
        match self {
            Self::X => (0, 1.0),
            Self::Y => (1, 1.0),
            Self::Z => (2, 1.0),
            Self::NegX => (0, -1.0),
            Self::NegY => (1, -1.0),
            Self::NegZ => (2, -1.0),
        }
    }

    fn local(self) -> Vec3 {
        let (index, sign) = self.parts();
        unit(index) * sign
    }
}

/// An unsigned axis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpAxis {
    X,
    #[default]
    Y,
    Z,
}

impl UpAxis {
    fn index(self) -> usize {
        match self {
            Self::X => 0,
            Self::Y => 1,
            Self::Z => 2,
        }
    }
}

fn unit(index: usize) -> Vec3 {
    match index {
        0 => Vec3::X,
        1 => Vec3::Y,
        _ => Vec3::Z,
    }
}

/// The space a constraint reads its owner or target in. Pose and Local With Parent are bones'
/// own: Pose is the armature's space, Local With Parent the bone's rest frame in it (so the
/// parent's motion counts), and Local the bone's rest frame where its parent now holds it (just
/// the bone's own pose). An object's Local is its parent's space; with no parent, its own
/// rotation's frame (as the reference application defines it).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Space {
    #[default]
    World,
    Local,
    Pose,
    LocalWithParent,
}

impl Space {
    fn is_world(&self) -> bool {
        *self == Self::World
    }

    /// Whether only a bone has this space.
    pub fn bones_only(self) -> bool {
        matches!(self, Self::Pose | Self::LocalWithParent)
    }
}

/// Which of the owner's other axes Stretch To thins to keep its volume.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StretchVolume {
    #[default]
    Xzx,
    X,
    Z,
    None,
}

/// How Stretch To turns its owner onto the target.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeepAxis {
    /// The shortest turn of Y onto the target.
    #[default]
    SwingY,
    /// Keeps X in the plane it spans with the new Y.
    PlaneX,
    /// Keeps Z in the plane it spans with the new Y.
    PlaneZ,
}

/// A transform channel: location, rotation (XYZ Euler), or scale.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransformChannel {
    #[default]
    Location,
    Rotation,
    Scale,
}

/// How a Transformation constraint combines its result with the owner's channel: location
/// adds or replaces, rotation adds angles, replaces, or turns before or after the owner's own,
/// scale replaces or multiplies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransformMix {
    Add,
    Replace,
    Before,
    After,
    Multiply,
}

impl TransformMix {
    /// Whether the mix applies to `channel`.
    pub fn fits(self, channel: TransformChannel) -> bool {
        match channel {
            TransformChannel::Location => matches!(self, Self::Add | Self::Replace),
            TransformChannel::Rotation => matches!(self, Self::Add | Self::Replace | Self::Before | Self::After),
            TransformChannel::Scale => matches!(self, Self::Replace | Self::Multiply),
        }
    }
}

/// How an Action constraint combines the action's transform with the owner's: replacing it, or
/// before (as a parent would) or after (as a child would) it, fully, aligned (scale per axis),
/// or channel by channel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionMix {
    Replace,
    BeforeFull,
    Before,
    BeforeSplit,
    AfterFull,
    #[default]
    After,
    AfterSplit,
}

/// How Spline IK scales its bones' lengths.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SplineYScale {
    /// Bones keep their length.
    None,
    /// The chain stretches to fill the curve.
    #[default]
    FitCurve,
    /// Bones keep their pose's Y scale.
    BoneOriginal,
}

/// How Spline IK scales its bones' thickness.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SplineXzScale {
    #[default]
    None,
    BoneOriginal,
    InversePreserve,
    VolumePreserve,
}

/// A bone of an Armature constraint and its weight.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArmatureTarget {
    #[serde(flatten)]
    pub target: Target,
    #[serde(default = "one")]
    pub weight: f32,
}

/// What a constraint does, and its settings. Angles are radians.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ConstraintKind {
    /// Points `track_axis` at the target, with `up_axis` towards world +Z.
    TrackTo {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Target>,
        #[serde(default = "neg_z")]
        track_axis: TrackAxis,
        #[serde(default)]
        up_axis: UpAxis,
    },
    /// Turns `track_axis` towards the target by the shortest arc.
    DampedTrack {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Target>,
        #[serde(default)]
        track_axis: TrackAxis,
    },
    /// Takes the target's location on the chosen axes.
    CopyLocation {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Target>,
        #[serde(default = "all_axes")]
        axes: [bool; 3],
        #[serde(default)]
        invert: [bool; 3],
        /// Adds the target's location to the owner's instead of replacing it.
        #[serde(default)]
        offset: bool,
        #[serde(default, skip_serializing_if = "Space::is_world")]
        owner_space: Space,
        #[serde(default, skip_serializing_if = "Space::is_world")]
        target_space: Space,
    },
    /// Takes the target's rotation (all of it, or the chosen XYZ Euler angles).
    CopyRotation {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Target>,
        #[serde(default = "all_axes")]
        axes: [bool; 3],
        #[serde(default)]
        invert: [bool; 3],
        #[serde(default, skip_serializing_if = "Space::is_world")]
        owner_space: Space,
        #[serde(default, skip_serializing_if = "Space::is_world")]
        target_space: Space,
    },
    /// Keeps XYZ Euler angles within limits, read in the owner's `space`.
    LimitRotation {
        #[serde(default)]
        limit: [bool; 3],
        #[serde(default)]
        min: Vec3,
        #[serde(default)]
        max: Vec3,
        #[serde(default)]
        space: Space,
    },
    /// Follows the target as if it were the owner's parent from when the constraint was set.
    ChildOf {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Target>,
        /// The inverse of the target's world matrix when the constraint was set.
        #[serde(default = "identity")]
        inverse: Mat4,
        #[serde(default = "all_axes")]
        location: [bool; 3],
        #[serde(default = "all_axes")]
        rotation: [bool; 3],
        #[serde(default = "all_axes")]
        scale: [bool; 3],
    },
    /// Inverse kinematics (bones only): bends the chain so the bone's tail reaches the target.
    Ik {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Target>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pole: Option<Target>,
        #[serde(default)]
        pole_angle: f32,
        /// Bones in the chain, counting this one; 0 runs to the root.
        #[serde(default)]
        chain: u32,
        #[serde(default = "five_hundred")]
        iterations: u32,
    },
    /// Turns Y at the target and stretches to reach it, thinning to keep the volume.
    StretchTo {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Target>,
        /// The distance at which nothing stretches (set when the target is first set).
        #[serde(default)]
        rest_length: f32,
        /// How strongly the other axes thin, 0 to 100 (1 keeps the volume).
        #[serde(default = "one")]
        bulge: f32,
        #[serde(default)]
        volume: StretchVolume,
        #[serde(default)]
        keep_axis: KeepAxis,
    },
    /// Keeps the owner on one side of a plane through the target.
    Floor {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Target>,
        #[serde(default = "plus_z")]
        axis: TrackAxis,
        #[serde(default)]
        offset: f32,
        /// The plane turns with the target.
        #[serde(default)]
        use_rotation: bool,
    },
    /// Maps a channel of the target, through ranges, onto a channel of the owner.
    Transform {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Target>,
        #[serde(default)]
        map_from: TransformChannel,
        #[serde(default)]
        map_to: TransformChannel,
        #[serde(default)]
        from_min: Vec3,
        #[serde(default)]
        from_max: Vec3,
        #[serde(default)]
        to_min: Vec3,
        #[serde(default)]
        to_max: Vec3,
        /// For each owner axis, the target axis it reads.
        #[serde(default = "straight")]
        mapping: [u8; 3],
        #[serde(default)]
        extrapolate: bool,
        /// How the result combines with the owner (the channel's default when `None`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mix: Option<TransformMix>,
        #[serde(default, skip_serializing_if = "Space::is_world")]
        owner_space: Space,
        #[serde(default, skip_serializing_if = "Space::is_world")]
        target_space: Space,
    },
    /// Plays an action on the owner, at the frame a channel of the target picks.
    Action {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Target>,
        /// The action's name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        action: Option<String>,
        #[serde(default)]
        channel: TransformChannel,
        /// The channel's axis, 0 to 2.
        #[serde(default)]
        axis: u8,
        /// The channel's values that map to the first and last frames.
        #[serde(default)]
        min: f32,
        #[serde(default)]
        max: f32,
        #[serde(default)]
        frame_start: f32,
        #[serde(default)]
        frame_end: f32,
        #[serde(default)]
        mix: ActionMix,
        #[serde(default, skip_serializing_if = "Space::is_world")]
        target_space: Space,
    },
    /// Moves the owner with bones, as the Armature modifier moves a mesh.
    Armature {
        #[serde(default)]
        targets: Vec<ArmatureTarget>,
        #[serde(default)]
        preserve_volume: bool,
    },
    /// Spline IK (bones only): fits the chain ending at this bone along a curve object.
    SplineIk {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Target>,
        /// Bones in the chain, counting this one.
        #[serde(default = "one_count")]
        chain: u32,
        #[serde(default)]
        even_divisions: bool,
        /// Keeps the chain's root where it is (the curve moves to it).
        #[serde(default)]
        chain_offset: bool,
        /// The curve's radius scales the bones' thickness.
        #[serde(default = "yes")]
        curve_radius: bool,
        #[serde(default)]
        y_scale: SplineYScale,
        #[serde(default)]
        xz_scale: SplineXzScale,
        #[serde(default = "one")]
        bulge: f32,
    },
}

fn neg_z() -> TrackAxis {
    TrackAxis::NegZ
}
fn plus_z() -> TrackAxis {
    TrackAxis::Z
}
fn all_axes() -> [bool; 3] {
    [true; 3]
}
fn straight() -> [u8; 3] {
    [0, 1, 2]
}
fn identity() -> Mat4 {
    Mat4::IDENTITY
}
fn five_hundred() -> u32 {
    500
}
fn one_count() -> u32 {
    1
}
fn yes() -> bool {
    true
}
fn one() -> f32 {
    1.0
}

/// The constraint kinds `constraint.add` accepts, with their display names; IK and Spline IK
/// only on bones.
pub const KINDS: &[(&str, &str)] = &[
    ("trackTo", "Track To"),
    ("dampedTrack", "Damped Track"),
    ("copyLocation", "Copy Location"),
    ("copyRotation", "Copy Rotation"),
    ("limitRotation", "Limit Rotation"),
    ("childOf", "Child Of"),
    ("ik", "IK"),
    ("stretchTo", "Stretch To"),
    ("floor", "Floor"),
    ("transform", "Transformation"),
    ("action", "Action"),
    ("armature", "Armature"),
    ("splineIk", "Spline IK"),
];

impl ConstraintKind {
    /// The kind with its defaults.
    pub fn default_of(kind: &str) -> Result<Self> {
        serde_json::from_value(serde_json::json!({ "type": kind })).map_err(|_| SceneError::params(format!("unknown constraint type `{kind}`")))
    }

    /// The kind's tag, as commands name it.
    pub fn tag(&self) -> &'static str {
        match self {
            Self::TrackTo { .. } => "trackTo",
            Self::DampedTrack { .. } => "dampedTrack",
            Self::CopyLocation { .. } => "copyLocation",
            Self::CopyRotation { .. } => "copyRotation",
            Self::LimitRotation { .. } => "limitRotation",
            Self::ChildOf { .. } => "childOf",
            Self::Ik { .. } => "ik",
            Self::StretchTo { .. } => "stretchTo",
            Self::Floor { .. } => "floor",
            Self::Transform { .. } => "transform",
            Self::Action { .. } => "action",
            Self::Armature { .. } => "armature",
            Self::SplineIk { .. } => "splineIk",
        }
    }

    /// The display name a new constraint of this kind gets.
    pub fn display_name(&self) -> &'static str {
        let tag = self.tag();
        KINDS.iter().find(|(id, _)| *id == tag).map_or("Constraint", |(_, name)| name)
    }

    /// Whether the kind solves a bone chain (and so goes only on bones).
    pub fn is_chain(&self) -> bool {
        matches!(self, Self::Ik { .. } | Self::SplineIk { .. })
    }

    /// The constraint's target, if it has one.
    pub fn target(&self) -> Option<&Target> {
        match self {
            Self::TrackTo { target, .. }
            | Self::DampedTrack { target, .. }
            | Self::CopyLocation { target, .. }
            | Self::CopyRotation { target, .. }
            | Self::ChildOf { target, .. }
            | Self::Ik { target, .. }
            | Self::StretchTo { target, .. }
            | Self::Floor { target, .. }
            | Self::Transform { target, .. }
            | Self::Action { target, .. }
            | Self::SplineIk { target, .. } => target.as_ref(),
            Self::LimitRotation { .. } | Self::Armature { .. } => None,
        }
    }

    /// Every object this constraint reads (target, pole, and an Armature's bones).
    pub fn targets(&self) -> Vec<&Target> {
        let mut out: Vec<&Target> = self.target().into_iter().collect();
        match self {
            Self::Ik { pole: Some(pole), .. } => out.push(pole),
            Self::Armature { targets, .. } => out.extend(targets.iter().map(|entry| &entry.target)),
            _ => {}
        }
        out
    }

    /// Drops the target and pole that name an object in `gone` (as the reference application
    /// leaves a constraint whose target was deleted, with no target).
    pub fn forget(&mut self, gone: &[Id]) {
        let lost = |slot: &mut Option<Target>| {
            if slot.as_ref().is_some_and(|target| gone.contains(&target.object)) {
                *slot = None;
            }
        };
        match self {
            Self::TrackTo { target, .. }
            | Self::DampedTrack { target, .. }
            | Self::CopyLocation { target, .. }
            | Self::CopyRotation { target, .. }
            | Self::ChildOf { target, .. }
            | Self::StretchTo { target, .. }
            | Self::Floor { target, .. }
            | Self::Transform { target, .. }
            | Self::Action { target, .. }
            | Self::SplineIk { target, .. } => lost(target),
            Self::Ik { target, pole, .. } => {
                lost(target);
                lost(pole);
            }
            Self::Armature { targets, .. } => targets.retain(|entry| !gone.contains(&entry.target.object)),
            Self::LimitRotation { .. } => {}
        }
    }

    /// The spaces the constraint reads its owner and target in.
    pub fn spaces(&self) -> (Space, Space) {
        match self {
            Self::CopyLocation { owner_space, target_space, .. }
            | Self::CopyRotation { owner_space, target_space, .. }
            | Self::Transform { owner_space, target_space, .. } => (*owner_space, *target_space),
            Self::LimitRotation { space, .. } => (*space, Space::World),
            Self::Action { target_space, .. } => (Space::Local, *target_space),
            _ => (Space::World, Space::World),
        }
    }

    /// Checks the numbers.
    pub fn check(&self) -> Result<()> {
        let finite3 = |values: &[Vec3]| values.iter().all(|value| value.is_finite());
        let fine = match self {
            Self::LimitRotation { min, max, .. } => min.is_finite() && max.is_finite(),
            Self::ChildOf { inverse, .. } => inverse.is_finite(),
            Self::Ik { pole_angle, iterations, chain, .. } => pole_angle.is_finite() && *iterations <= 10_000 && *chain <= 255,
            Self::TrackTo { track_axis, up_axis, .. } => track_axis.parts().0 != up_axis.index(),
            Self::StretchTo { rest_length, bulge, .. } => rest_length.is_finite() && *rest_length >= 0.0 && (0.0..=100.0).contains(bulge),
            Self::Floor { offset, .. } => offset.is_finite(),
            Self::Transform { from_min, from_max, to_min, to_max, mapping, mix, map_to, .. } => {
                finite3(&[*from_min, *from_max, *to_min, *to_max]) && mapping.iter().all(|axis| *axis < 3) && mix.is_none_or(|mix| mix.fits(*map_to))
            }
            Self::Action { axis, min, max, frame_start, frame_end, .. } => {
                *axis < 3
                    && [*min, *max, *frame_start, *frame_end].iter().all(|value| value.is_finite())
                    && frame_start.abs() <= crate::animation::MAX_FRAME as f32
                    && frame_end.abs() <= crate::animation::MAX_FRAME as f32
            }
            Self::Armature { targets, .. } => {
                targets.len() <= 64 && targets.iter().all(|entry| (0.0..=1.0).contains(&entry.weight) && entry.target.bone.is_some())
            }
            Self::SplineIk { chain, bulge, .. } => (1..=255).contains(chain) && (0.0..=100.0).contains(bulge),
            Self::CopyLocation { .. } | Self::CopyRotation { .. } | Self::DampedTrack { .. } => true,
        };
        if fine {
            Ok(())
        } else {
            Err(SceneError::params(format!(
                "{} has a setting out of range (Track To's track and up axes must differ; weights run 0 to 1 and name a bone; a Transformation's mix must suit its channel)",
                self.display_name()
            )))
        }
    }
}

/// One constraint on an object or a bone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Constraint {
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// How much of the constraint's result is used, 0 to 1.
    #[serde(default = "one")]
    pub influence: f32,
    #[serde(flatten)]
    pub kind: ConstraintKind,
}

impl Constraint {
    /// A constraint of `kind` named after it.
    pub fn new(kind: ConstraintKind) -> Self {
        Self { name: kind.display_name().to_string(), enabled: true, influence: 1.0, kind }
    }

    /// Checks the influence and the kind's numbers.
    pub fn check(&self) -> Result<()> {
        if !(self.influence.is_finite() && (0.0..=1.0).contains(&self.influence)) {
            return Err(SceneError::params("influence must be between 0 and 1"));
        }
        self.kind.check()
    }
}

/// Checks a stack: numbers, names, that IK and Spline IK appear only on bones, and that only
/// bones read in bones' spaces.
pub fn check_stack(stack: &[Constraint], on_bone: bool) -> Result<()> {
    let mut names = BTreeSet::new();
    for constraint in stack {
        constraint.check().map_err(|error| SceneError::InvalidFile(error.to_string()))?;
        if !names.insert(constraint.name.as_str()) {
            return Err(SceneError::InvalidFile(format!("constraint `{}` is repeated", constraint.name)));
        }
        if !on_bone && constraint.kind.is_chain() {
            return Err(SceneError::InvalidFile(format!("{} constraints go on bones", constraint.kind.display_name())));
        }
        if !on_bone && constraint.kind.spaces().0.bones_only() {
            return Err(SceneError::InvalidFile(format!("{} on an object reads in world or local space", constraint.name)));
        }
    }
    Ok(())
}

/// Linear blend of two transforms: locations and scales mixed, rotations slerped (the
/// reference application blends a constraint's result by its influence this way).
pub fn blend(from: Mat4, to: Mat4, t: f32) -> Mat4 {
    if t >= 1.0 {
        return to;
    }
    if t <= 0.0 {
        return from;
    }
    let (scale_a, rotation_a, location_a) = from.to_scale_rotation_translation();
    let (scale_b, rotation_b, location_b) = to.to_scale_rotation_translation();
    Mat4::from_scale_rotation_translation(scale_a.lerp(scale_b, t), rotation_a.slerp(rotation_b, t), location_a.lerp(location_b, t))
}

fn compose(location: Vec3, rotation: Mat3, scale: Vec3) -> Mat4 {
    Mat4::from_translation(location) * Mat4::from_mat3(rotation) * Mat4::from_scale(scale)
}

fn parts(matrix: Mat4) -> (Vec3, Mat3, Vec3) {
    let (scale, rotation, location) = matrix.to_scale_rotation_translation();
    (location, Mat3::from_quat(rotation), scale)
}

/// An object's Local space: its parent's (parent world matrix times parent inverse) or, with no
/// parent, its own rotation.
fn object_local_space(scene: &Scene, id: Id, depth: u32) -> Option<Mat4> {
    let object = scene.object(id).ok()?;
    match object.parent {
        Some(parent) => Some(world_matrix(scene, parent, depth).ok()? * object.parent_inverse),
        None => Some(Mat4::from_mat3(euler_matrix(object.rotation))),
    }
}

/// The spaces of an owner: (local, pose, local with parent), each as a matrix to world.
#[derive(Debug, Clone, Copy)]
struct Spaces {
    local: Mat4,
    pose: Mat4,
    with_parent: Mat4,
}

impl Spaces {
    fn of(&self, space: Space) -> Mat4 {
        match space {
            Space::World => Mat4::IDENTITY,
            Space::Local => self.local,
            Space::Pose => self.pose,
            Space::LocalWithParent => self.with_parent,
        }
    }
}

/// The owner a stack runs on, for constraints that read the owner's own animation.
#[derive(Debug, Clone, Copy)]
enum Owner<'a> {
    Object,
    /// A bone, by name, and whether its head sits on its parent's tail.
    Bone(&'a str, bool),
}

/// What a stack reads besides its owner.
trait Lookup {
    fn scene(&self) -> &Scene;
    /// The target's world matrix.
    fn world(&mut self, target: &Target) -> Option<Mat4>;
    /// The matrix of the target's `space` (to world).
    fn space(&mut self, target: &Target, space: Space) -> Option<Mat4>;
    /// A target bone's rest matrix in world space.
    fn rest(&mut self, target: &Target) -> Option<Mat4>;
}

/// Objects' lookups (and other armatures' bones).
struct Objects<'a> {
    scene: &'a Scene,
    depth: u32,
}

/// A bone's spaces in its armature: (local, local with parent), from the evaluated parent pose.
fn bone_spaces(armature: &Armature, name: &str, parent_pose: Option<Mat4>) -> (Mat4, Mat4) {
    let Some(bone) = armature.bone(name) else { return (Mat4::IDENTITY, Mat4::IDENTITY) };
    let local = match (bone.parent.as_deref().and_then(|parent| armature.bone(parent)), parent_pose) {
        (Some(parent), Some(pose)) => pose * parent.rest_matrix().inverse() * bone.rest_matrix(),
        _ => bone.rest_matrix(),
    };
    (local, bone.rest_matrix())
}

impl Lookup for Objects<'_> {
    fn scene(&self) -> &Scene {
        self.scene
    }

    fn world(&mut self, target: &Target) -> Option<Mat4> {
        target_matrix(self.scene, target, self.depth)
    }

    fn space(&mut self, target: &Target, space: Space) -> Option<Mat4> {
        if space == Space::World {
            return Some(Mat4::IDENTITY);
        }
        if self.depth >= MAX_DEPTH {
            return None;
        }
        match &target.bone {
            None => object_local_space(self.scene, target.object, self.depth),
            Some(bone) => {
                let armature = armature_of(self.scene, target.object)?;
                let rig_world = world_matrix(self.scene, target.object, self.depth).ok()?;
                let poses = bone_matrices(self.scene, target.object, self.depth);
                let parent = armature.bone(bone)?.parent.as_deref().and_then(|parent| poses.get(parent).copied());
                let (local, with_parent) = bone_spaces(armature, bone, parent);
                Some(
                    rig_world
                        * if space == Space::Local {
                            local
                        } else if space == Space::Pose {
                            Mat4::IDENTITY
                        } else {
                            with_parent
                        },
                )
            }
        }
    }

    fn rest(&mut self, target: &Target) -> Option<Mat4> {
        let bone = armature_of(self.scene, target.object)?.bone(target.bone.as_deref()?)?;
        Some(world_matrix(self.scene, target.object, self.depth).ok()? * bone.rest_matrix())
    }
}

/// The object's world matrix with its parents and constraints (`depth` follows targets).
pub fn world_matrix(scene: &Scene, id: Id, depth: u32) -> Result<Mat4> {
    // A parent cycle is an error, as before constraints.
    let mut chain = Vec::new();
    let mut current = Some(id);
    while let Some(at) = current {
        let object = scene.object(at)?;
        chain.push(at);
        current = object.parent;
        if chain.len() > scene.objects.len() {
            return Err(SceneError::InvalidFile("the parent hierarchy has a cycle".into()));
        }
    }
    let mut matrix = Mat4::IDENTITY;
    for at in chain.iter().rev() {
        let object = scene.object(*at)?;
        // A baked rigid body is where the simulation put it.
        if let Some(simulated) = scene.simulated_world(*at) {
            matrix = simulated;
            continue;
        }
        let parent_space = if object.parent.is_some() { matrix * object.parent_inverse } else { Mat4::IDENTITY };
        matrix = parent_space * object.local_matrix();
        if !object.constraints.is_empty() && depth < MAX_DEPTH {
            let local = if object.parent.is_some() { parent_space } else { Mat4::from_mat3(euler_matrix(object.rotation)) };
            let spaces = Spaces { local, pose: Mat4::IDENTITY, with_parent: Mat4::IDENTITY };
            let mut lookup = Objects { scene, depth: depth + 1 };
            matrix = apply_stack(matrix, &object.constraints, &mut lookup, &spaces, Owner::Object);
        }
    }
    Ok(matrix)
}

/// The world matrix of a target: an object's, or a bone's (its head and axes).
fn target_matrix(scene: &Scene, target: &Target, depth: u32) -> Option<Mat4> {
    if depth >= MAX_DEPTH {
        return None;
    }
    let world = world_matrix(scene, target.object, depth).ok()?;
    match &target.bone {
        None => Some(world),
        Some(bone) => {
            armature_of(scene, target.object)?;
            Some(world * *bone_matrices(scene, target.object, depth).get(bone)?)
        }
    }
}

/// The world matrix of an object or a bone, constraints applied.
pub fn target_world(scene: &Scene, target: &Target) -> Option<Mat4> {
    target_matrix(scene, target, 0)
}

/// The matrix (to world) of an object's or a bone's `space`.
pub fn target_space(scene: &Scene, target: &Target, space: Space) -> Option<Mat4> {
    Objects { scene, depth: 0 }.space(target, space)
}

/// Runs a stack on a world matrix (chain solvers excepted: they run after every bone).
fn apply_stack(mut matrix: Mat4, stack: &[Constraint], lookup: &mut dyn Lookup, spaces: &Spaces, owner: Owner<'_>) -> Mat4 {
    for constraint in stack.iter().filter(|constraint| constraint.enabled && constraint.influence > 0.0) {
        if constraint.kind.is_chain() {
            continue;
        }
        if let Some(result) = apply_one(&constraint.kind, matrix, lookup, spaces, owner) {
            matrix = blend(matrix, result, constraint.influence);
        }
    }
    matrix
}

/// One constraint on a world matrix; `None` when it does nothing (no target, say).
fn apply_one(kind: &ConstraintKind, owner: Mat4, lookup: &mut dyn Lookup, spaces: &Spaces, who: Owner<'_>) -> Option<Mat4> {
    let (owner_space, target_space) = kind.spaces();
    let into = spaces.of(owner_space);
    let local = into.inverse() * owner;
    match kind {
        ConstraintKind::LimitRotation { limit, min, max, .. } => return Some(into * limit_rotation(local, *limit, *min, *max)),
        ConstraintKind::Armature { targets, preserve_volume } => {
            let mut bones = Vec::with_capacity(targets.len());
            for entry in targets {
                if let (Some(pose), Some(rest)) = (lookup.world(&entry.target), lookup.rest(&entry.target)) {
                    bones.push((entry.weight, pose, rest));
                }
            }
            return crate::constraint_math::armature(owner, &bones, *preserve_volume);
        }
        _ => {}
    }
    let target = kind.target()?;
    let target_world = lookup.world(target)?;
    let target_local = match target_space {
        Space::World => target_world,
        space => lookup.space(target, space)?.inverse() * target_world,
    };
    let result = match kind {
        ConstraintKind::CopyLocation { .. } | ConstraintKind::CopyRotation { .. } => aim(kind, local, target_local)?,
        ConstraintKind::Transform { map_from, map_to, from_min, from_max, to_min, to_max, mapping, extrapolate, mix, .. } => {
            crate::constraint_math::transform(
                local,
                target_local,
                *map_from,
                *map_to,
                (*from_min, *from_max),
                (*to_min, *to_max),
                *mapping,
                *extrapolate,
                mix.unwrap_or_else(|| crate::constraint_math::default_transform_mix(*map_to)),
            )
        }
        ConstraintKind::Action { action, channel, axis, min, max, frame_start, frame_end, mix, .. } => {
            let action = lookup.scene().animation.action(action.as_deref()?)?;
            let value = crate::constraint_math::channel_values(target_local, *channel).to_array().get(usize::from(*axis)).copied().unwrap_or(0.0);
            let frame = crate::constraint_math::action_frame(value, *min, *max, *frame_start, *frame_end);
            let played = match who {
                Owner::Object => action.matrix_at(frame, None),
                Owner::Bone(name, connected) => {
                    let mut played = action.matrix_at(frame, Some(name));
                    // A connected bone's head stays on its parent's tail: the action's location
                    // does not move it.
                    if connected {
                        played.w_axis = Vec3::ZERO.extend(1.0);
                    }
                    played
                }
            };
            crate::constraint_math::action_mix(local, played, *mix)
        }
        ConstraintKind::StretchTo { rest_length, bulge, volume, keep_axis, .. } => {
            crate::constraint_math::stretch_to(owner, target_world.w_axis.truncate(), *rest_length, *bulge, *volume, *keep_axis)?
        }
        ConstraintKind::Floor { axis, offset, use_rotation, .. } => crate::constraint_math::floor(owner, target_world, *axis, *offset, *use_rotation),
        _ => return aim(kind, owner, target_world),
    };
    Some(into * result)
}

/// The constraints that point, copy, or follow: Track To, Damped Track, Copy Location, Copy
/// Rotation, Child Of.
fn aim(kind: &ConstraintKind, owner: Mat4, target: Mat4) -> Option<Mat4> {
    let (location, rotation, scale) = parts(owner);
    let target_location = target.w_axis.truncate();
    match kind {
        ConstraintKind::TrackTo { track_axis, up_axis, .. } => {
            let direction = (target_location - location).normalize_or_zero();
            if direction == Vec3::ZERO {
                return None;
            }
            let (track_index, sign) = track_axis.parts();
            let up_index = up_axis.index();
            if track_index == up_index {
                return None;
            }
            let track = direction * sign;
            let mut up = Vec3::Z - track * Vec3::Z.dot(track);
            if up.length_squared() < 1e-10 {
                // Looking straight up or down: keep the owner's own up axis as well as can be.
                let own = rotation * unit(up_index);
                up = own - track * own.dot(track);
            }
            let up = up.normalize_or_zero();
            if up == Vec3::ZERO {
                return None;
            }
            let mut columns = [Vec3::ZERO; 3];
            let third = 3 - track_index - up_index;
            set(&mut columns, track_index, track);
            set(&mut columns, up_index, up);
            // The third axis completes a right-handed frame.
            let (a, b) = ((third + 1) % 3, (third + 2) % 3);
            let (column_a, column_b) = (pick(&columns, a), pick(&columns, b));
            set(&mut columns, third, column_a.cross(column_b));
            Some(compose(location, Mat3::from_cols(columns[0], columns[1], columns[2]), scale))
        }
        ConstraintKind::DampedTrack { track_axis, .. } => {
            let direction = (target_location - location).normalize_or_zero();
            if direction == Vec3::ZERO {
                return None;
            }
            let axis = (rotation * track_axis.local()).normalize_or_zero();
            let turn = Quat::from_rotation_arc(axis, direction);
            Some(compose(location, Mat3::from_quat(turn) * rotation, scale))
        }
        ConstraintKind::CopyLocation { axes, invert, offset, .. } => {
            let mut out = location.to_array();
            for (index, component) in out.iter_mut().enumerate() {
                if !axes.get(index).copied().unwrap_or(false) {
                    continue;
                }
                let mut value = target_location.to_array().get(index).copied().unwrap_or(0.0);
                if invert.get(index).copied().unwrap_or(false) {
                    value = -value;
                }
                *component = if *offset { *component + value } else { value };
            }
            // Only the location changes; a sheared owner keeps its shear.
            let mut moved = owner;
            moved.w_axis = Vec3::from_array(out).extend(1.0);
            Some(moved)
        }
        ConstraintKind::CopyRotation { axes, invert, .. } => {
            let (_, target_rotation, _) = parts(target);
            if axes.iter().all(|on| *on) && invert.iter().all(|on| !*on) {
                return Some(compose(location, target_rotation, scale));
            }
            let mut mine = matrix_euler(rotation).to_array();
            let theirs = matrix_euler(target_rotation).to_array();
            for (index, component) in mine.iter_mut().enumerate() {
                if axes.get(index).copied().unwrap_or(false) {
                    let value = theirs.get(index).copied().unwrap_or(0.0);
                    *component = if invert.get(index).copied().unwrap_or(false) { -value } else { value };
                }
            }
            Some(compose(location, euler_matrix(Vec3::from_array(mine)), scale))
        }
        ConstraintKind::ChildOf { inverse, location: use_location, rotation: use_rotation, scale: use_scale, .. } => {
            let parent = if use_location.iter().all(|on| *on) && use_rotation.iter().all(|on| *on) && use_scale.iter().all(|on| *on) {
                target
            } else {
                let (target_location, target_rotation, target_scale) = parts(target);
                let mut euler = matrix_euler(target_rotation).to_array();
                let mut location = target_location.to_array();
                let mut scale_parts = target_scale.to_array();
                for index in 0..3 {
                    if !use_location.get(index).copied().unwrap_or(true)
                        && let Some(value) = location.get_mut(index)
                    {
                        *value = 0.0;
                    }
                    if !use_rotation.get(index).copied().unwrap_or(true)
                        && let Some(value) = euler.get_mut(index)
                    {
                        *value = 0.0;
                    }
                    if !use_scale.get(index).copied().unwrap_or(true)
                        && let Some(value) = scale_parts.get_mut(index)
                    {
                        *value = 1.0;
                    }
                }
                compose(Vec3::from_array(location), euler_matrix(Vec3::from_array(euler)), Vec3::from_array(scale_parts))
            };
            Some(parent * *inverse * owner)
        }
        _ => None,
    }
}

fn set(columns: &mut [Vec3; 3], index: usize, value: Vec3) {
    if let Some(slot) = columns.get_mut(index) {
        *slot = value;
    }
}

fn pick(columns: &[Vec3; 3], index: usize) -> Vec3 {
    columns.get(index).copied().unwrap_or(Vec3::ZERO)
}

/// Clamps XYZ Euler angles of a matrix (already in the space the limits read).
fn limit_rotation(owner: Mat4, limit: [bool; 3], min: Vec3, max: Vec3) -> Mat4 {
    let (location, _, scale) = parts(owner);
    let mut euler = crate::constraint_math::euler_of(crate::constraint_math::normalized(owner)).to_array();
    let (low, high) = (min.to_array(), max.to_array());
    for (index, angle) in euler.iter_mut().enumerate() {
        if limit.get(index).copied().unwrap_or(false) {
            let (lo, hi) = (low.get(index).copied().unwrap_or(0.0), high.get(index).copied().unwrap_or(0.0));
            *angle = angle.clamp(lo.min(hi), hi.max(lo));
        }
    }
    compose(location, euler_matrix(Vec3::from_array(euler)), scale)
}

/// Every bone's pose matrix in the armature's space, with its constraints (and IK and Spline
/// IK) applied; the plain [`Armature::pose_matrices`] when no bone has any.
pub fn bone_matrices(scene: &Scene, rig: Id, depth: u32) -> BTreeMap<String, Mat4> {
    let Some(armature) = armature_of(scene, rig) else { return BTreeMap::new() };
    if armature.bones.iter().all(|bone| bone.constraints.is_empty()) || depth >= MAX_DEPTH {
        return armature.pose_matrices();
    }
    let rig_world = world_matrix(scene, rig, depth + 1).unwrap_or(Mat4::IDENTITY);
    let mut evaluator = Bones { scene, rig, armature, rig_world, depth, done: BTreeMap::new(), visiting: BTreeSet::new() };
    for bone in &armature.bones {
        evaluator.eval(&bone.name);
    }
    evaluator.chains();
    evaluator.done
}

/// Bone evaluation for one armature.
struct Bones<'a> {
    scene: &'a Scene,
    rig: Id,
    armature: &'a Armature,
    rig_world: Mat4,
    depth: u32,
    /// Pose matrices (armature space) worked out so far.
    done: BTreeMap<String, Mat4>,
    visiting: BTreeSet<String>,
}

impl Lookup for Bones<'_> {
    fn scene(&self) -> &Scene {
        self.scene
    }

    fn world(&mut self, target: &Target) -> Option<Mat4> {
        match &target.bone {
            Some(bone) if target.object == self.rig => {
                self.armature.bone(bone)?;
                let matrix = self.eval(bone);
                Some(self.rig_world * matrix)
            }
            _ => target_matrix(self.scene, target, self.depth + 1),
        }
    }

    fn space(&mut self, target: &Target, space: Space) -> Option<Mat4> {
        match &target.bone {
            Some(bone) if target.object == self.rig => {
                let parent = self.armature.bone(bone)?.parent.clone().map(|parent| self.eval(&parent));
                let (local, with_parent) = bone_spaces(self.armature, bone, parent);
                Some(match space {
                    Space::World => Mat4::IDENTITY,
                    Space::Pose => self.rig_world,
                    Space::Local => self.rig_world * local,
                    Space::LocalWithParent => self.rig_world * with_parent,
                })
            }
            _ => Objects { scene: self.scene, depth: self.depth + 1 }.space(target, space),
        }
    }

    fn rest(&mut self, target: &Target) -> Option<Mat4> {
        if target.object == self.rig {
            return Some(self.rig_world * self.armature.bone(target.bone.as_deref()?)?.rest_matrix());
        }
        Objects { scene: self.scene, depth: self.depth + 1 }.rest(target)
    }
}

impl Bones<'_> {
    /// The bone's forward pose from its parent's (evaluated) matrix, before its constraints.
    fn forward(&self, name: &str, parent_matrix: Option<Mat4>) -> Mat4 {
        let Some(bone) = self.armature.bone(name) else { return Mat4::IDENTITY };
        match (bone.parent.as_deref().and_then(|parent| self.armature.bone(parent)), parent_matrix) {
            (Some(parent), Some(parent_matrix)) => {
                parent_matrix * parent.rest_matrix().inverse() * bone.rest_matrix() * bone.pose.matrix(bone.connected)
            }
            _ => bone.rest_matrix() * bone.pose.matrix(false),
        }
    }

    /// Runs a bone's stack on its forward pose (armature space in and out).
    fn constrain(&mut self, name: &str, base: Mat4, parent: Option<Mat4>) -> Mat4 {
        let Some(bone) = self.armature.bone(name) else { return base };
        if bone.constraints.is_empty() {
            return base;
        }
        let connected = bone.connected;
        let (local, with_parent) = bone_spaces(self.armature, name, parent);
        let spaces = Spaces { local: self.rig_world * local, pose: self.rig_world, with_parent: self.rig_world * with_parent };
        let stack = bone.constraints.clone();
        let world = self.rig_world * base;
        let rig_inverse = self.rig_world.inverse();
        rig_inverse * apply_stack(world, &stack, self, &spaces, Owner::Bone(name, connected))
    }

    /// Evaluates a bone (its parent first, and bones of this armature it targets).
    fn eval(&mut self, name: &str) -> Mat4 {
        if let Some(matrix) = self.done.get(name) {
            return *matrix;
        }
        let Some(bone) = self.armature.bone(name) else { return Mat4::IDENTITY };
        if !self.visiting.insert(name.to_string()) {
            // A cycle through targets: the bone goes unconstrained.
            let parent = bone.parent.as_deref().and_then(|parent| self.done.get(parent).copied());
            return self.forward(name, parent);
        }
        let parent = bone.parent.clone().map(|parent| self.eval(&parent));
        let base = self.forward(name, parent);
        let matrix = self.constrain(name, base, parent);
        self.visiting.remove(name);
        self.done.insert(name.to_string(), matrix);
        matrix
    }

    /// Solves every IK and Spline IK constraint, in bone order, then moves the bones below each
    /// chain.
    fn chains(&mut self) {
        let solves: Vec<(String, Constraint)> = self
            .armature
            .bones
            .iter()
            .flat_map(|bone| {
                bone.constraints.iter().filter(|c| c.enabled && c.influence > 0.0 && c.kind.is_chain()).map(|c| (bone.name.clone(), c.clone()))
            })
            .collect();
        for (tip, constraint) in solves {
            let solved = match &constraint.kind {
                ConstraintKind::Ik { target: Some(target), pole, pole_angle, chain, iterations } => {
                    let Some(goal) = self.world(target) else { continue };
                    let pole = pole.as_ref().and_then(|pole| self.world(pole)).map(|matrix| matrix.w_axis.truncate());
                    let names = self.chain(&tip, *chain);
                    let worlds: Vec<Mat4> =
                        names.iter().map(|name| self.rig_world * self.done.get(name).copied().unwrap_or(Mat4::IDENTITY)).collect();
                    let length = self.armature.bone(&tip).map_or(0.0, |bone| bone.length());
                    let after = crate::ik::solve_chain(&worlds, length, goal.w_axis.truncate(), pole, *pole_angle, (*iterations).clamp(1, 10_000));
                    (names, worlds, after)
                }
                ConstraintKind::SplineIk { target: Some(target), chain, even_divisions, chain_offset, curve_radius, y_scale, xz_scale, bulge } => {
                    let Some(path) = curve_path(self.scene, target.object, self.depth) else { continue };
                    let names = self.chain(&tip, (*chain).max(1));
                    let worlds: Vec<Mat4> =
                        names.iter().map(|name| self.rig_world * self.done.get(name).copied().unwrap_or(Mat4::IDENTITY)).collect();
                    let bones: Vec<crate::constraint_math::ChainBone> = names
                        .iter()
                        .zip(&worlds)
                        .map(|(name, world)| crate::constraint_math::ChainBone {
                            world: *world,
                            length: self.armature.bone(name).map_or(0.0, |bone| bone.length()),
                        })
                        .collect();
                    let after =
                        crate::constraint_math::spline_ik(&bones, &path, *even_divisions, *chain_offset, *curve_radius, *y_scale, *xz_scale, *bulge);
                    (names, worlds, after)
                }
                _ => continue,
            };
            let (names, worlds, after) = solved;
            if names.is_empty() {
                continue;
            }
            let inverse = self.rig_world.inverse();
            for (name, (before, after)) in names.iter().zip(worlds.iter().zip(after)) {
                self.done.insert(name.clone(), inverse * blend(*before, after, constraint.influence));
            }
            self.refresh_below(&names);
        }
    }

    /// The chain ending at `tip`, root first: `count` bones (0 = up to the root).
    fn chain(&self, tip: &str, count: u32) -> Vec<String> {
        let mut out = Vec::new();
        let mut current = Some(tip.to_string());
        while let Some(name) = current {
            if out.len() > self.armature.bones.len() || (count > 0 && out.len() >= count as usize) {
                break;
            }
            current = self.armature.bone(&name).and_then(|bone| bone.parent.clone());
            out.push(name);
        }
        out.reverse();
        out
    }

    /// Re-evaluates every bone below the chain (not in it) from its moved parent.
    fn refresh_below(&mut self, chain: &[String]) {
        let mut moved: BTreeSet<String> = chain.iter().cloned().collect();
        let mut changed = true;
        let mut guard = 0;
        while changed && guard <= self.armature.bones.len() {
            changed = false;
            guard += 1;
            for bone in &self.armature.bones {
                if moved.contains(&bone.name) {
                    continue;
                }
                let Some(parent) = bone.parent.as_deref() else { continue };
                if !moved.contains(parent) {
                    continue;
                }
                let parent_matrix = self.done.get(parent).copied();
                let base = self.forward(&bone.name, parent_matrix);
                let matrix = self.constrain(&bone.name, base, parent_matrix);
                self.done.insert(bone.name.clone(), matrix);
                moved.insert(bone.name.clone());
                changed = true;
            }
        }
    }
}

/// The path a curve object's first spline makes, in world space, with its radii.
fn curve_path(scene: &Scene, id: Id, depth: u32) -> Option<crate::constraint_math::Path> {
    let object = scene.object(id).ok()?;
    let ObjectData::Curve(curve) = &object.data else { return None };
    let (points, radii) = curve.path()?;
    let world = world_matrix(scene, id, depth + 1).ok()?;
    Some(crate::constraint_math::Path { points: points.iter().map(|point| world.transform_point3(*point)).collect(), radii })
}

/// Dependencies between objects through parents and constraint targets (an armature depends
/// on the targets of its bones' constraints): refuses a cycle.
pub fn check_cycles(scene: &Scene) -> Result<()> {
    let mut edges: BTreeMap<Id, BTreeSet<Id>> = BTreeMap::new();
    for object in &scene.objects {
        let entry = edges.entry(object.id).or_default();
        if let Some(parent) = object.parent {
            entry.insert(parent);
        }
        let mut stacks: Vec<(&Vec<Constraint>, bool)> = vec![(&object.constraints, true)];
        if let ObjectData::Armature(armature) = &object.data {
            stacks.extend(armature.bones.iter().map(|bone| (&bone.constraints, false)));
        }
        for (stack, on_object) in stacks {
            for constraint in stack {
                for target in constraint.kind.targets() {
                    if target.object != object.id {
                        entry.insert(target.object);
                    } else if on_object {
                        return Err(SceneError::params(format!("{} cannot target {} itself", constraint.name, object.name)));
                    }
                    // A bone targeting its own armature is checked inside the armature below.
                }
            }
        }
        if let ObjectData::Armature(armature) = &object.data {
            check_bone_cycles(armature, object.id)?;
        }
    }
    // Depth-first search for a back edge.
    let mut state: BTreeMap<Id, u8> = BTreeMap::new();
    for start in edges.keys().copied().collect::<Vec<_>>() {
        if state.get(&start).copied().unwrap_or(0) != 0 {
            continue;
        }
        let mut stack: Vec<(Id, Vec<Id>)> = vec![(start, edges.get(&start).map(|set| set.iter().copied().collect()).unwrap_or_default())];
        state.insert(start, 1);
        while let Some((node, pending)) = stack.last_mut() {
            match pending.pop() {
                Some(next) => match state.get(&next).copied().unwrap_or(0) {
                    1 => {
                        let name = scene.object(next).map(|object| object.name.clone()).unwrap_or_default();
                        return Err(SceneError::params(format!("constraints and parents make {name} depend on itself")));
                    }
                    0 => {
                        state.insert(next, 1);
                        let children = edges.get(&next).map(|set| set.iter().copied().collect()).unwrap_or_default();
                        stack.push((next, children));
                    }
                    _ => {}
                },
                None => {
                    state.insert(*node, 2);
                    stack.pop();
                }
            }
        }
    }
    Ok(())
}

/// Bones of one armature: a bone depends on its parent and on bones of the same armature its
/// constraints target (an IK chain's bones depend on the target too).
fn check_bone_cycles(armature: &Armature, rig: Id) -> Result<()> {
    let mut edges: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for bone in &armature.bones {
        let entry = edges.entry(bone.name.as_str()).or_default();
        if let Some(parent) = bone.parent.as_deref() {
            entry.insert(parent);
        }
        for constraint in &bone.constraints {
            for target in constraint.kind.targets() {
                if target.object == rig
                    && let Some(other) = target.bone.as_deref()
                {
                    if other == bone.name {
                        return Err(SceneError::params(format!("{} on bone {} targets the bone itself", constraint.name, bone.name)));
                    }
                    entry.insert(other);
                }
            }
        }
    }
    // An IK target inside its own chain is a cycle.
    for bone in &armature.bones {
        for constraint in &bone.constraints {
            let ConstraintKind::Ik { target: Some(target), chain, .. } = &constraint.kind else { continue };
            let Some(goal) = target.bone.as_deref().filter(|_| target.object == rig) else { continue };
            let mut current = Some(bone.name.as_str());
            let mut steps = 0u32;
            while let Some(name) = current {
                if *chain > 0 && steps >= *chain {
                    break;
                }
                if name == goal || is_below(armature, goal, name) {
                    return Err(SceneError::params(format!("the IK target {goal} moves with the chain it drives")));
                }
                current = armature.bone(name).and_then(|bone| bone.parent.as_deref());
                steps += 1;
                if steps as usize > armature.bones.len() {
                    break;
                }
            }
        }
    }
    let mut state: BTreeMap<&str, u8> = BTreeMap::new();
    fn visit<'a>(node: &'a str, edges: &BTreeMap<&'a str, BTreeSet<&'a str>>, state: &mut BTreeMap<&'a str, u8>, depth: usize) -> bool {
        if depth > 4096 {
            return false;
        }
        match state.get(node).copied().unwrap_or(0) {
            1 => return false,
            2 => return true,
            _ => {}
        }
        state.insert(node, 1);
        for next in edges.get(node).into_iter().flatten() {
            if !visit(next, edges, state, depth + 1) {
                return false;
            }
        }
        state.insert(node, 2);
        true
    }
    for bone in &armature.bones {
        if !visit(bone.name.as_str(), &edges, &mut state, 0) {
            return Err(SceneError::params("bone constraints make a bone depend on itself"));
        }
    }
    Ok(())
}

/// Whether `bone` is `ancestor`'s descendant.
fn is_below(armature: &Armature, bone: &str, ancestor: &str) -> bool {
    let mut current = armature.bone(bone).and_then(|bone| bone.parent.as_deref());
    let mut steps = 0;
    while let Some(name) = current {
        if name == ancestor {
            return true;
        }
        current = armature.bone(name).and_then(|bone| bone.parent.as_deref());
        steps += 1;
        if steps > armature.bones.len() {
            return false;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::armature::Bone;
    use crate::scene::ObjectData;

    fn empty(scene: &mut Scene, name: &str, location: Vec3) -> Id {
        let id = scene.add_object(name, ObjectData::Empty { size: 1.0 });
        scene.object_mut(id).unwrap().location = location;
        id
    }

    #[test]
    fn track_to_points_minus_z_at_the_target_with_y_up() {
        let mut scene = Scene::empty();
        let target = empty(&mut scene, "Target", Vec3::new(3.0, 4.0, 0.0));
        let owner = empty(&mut scene, "Owner", Vec3::ZERO);
        scene.object_mut(owner).unwrap().constraints.push(Constraint::new(ConstraintKind::TrackTo {
            target: Some(Target { object: target, bone: None }),
            track_axis: TrackAxis::NegZ,
            up_axis: UpAxis::Y,
        }));
        let world = scene.world_matrix(owner).unwrap();
        let minus_z = -world.z_axis.truncate();
        assert!(minus_z.abs_diff_eq(Vec3::new(0.6, 0.8, 0.0), 1e-5), "{minus_z}");
        assert!(world.y_axis.truncate().abs_diff_eq(Vec3::Z, 1e-5));
    }

    #[test]
    fn influence_blends_and_child_of_keeps_the_owner_in_place() {
        let mut scene = Scene::empty();
        let parent = empty(&mut scene, "Parent", Vec3::new(1.0, 2.0, 3.0));
        let owner = empty(&mut scene, "Owner", Vec3::new(5.0, 0.0, 0.0));
        let inverse = scene.world_matrix(parent).unwrap().inverse();
        scene.object_mut(owner).unwrap().constraints.push(Constraint::new(ConstraintKind::ChildOf {
            target: Some(Target { object: parent, bone: None }),
            inverse,
            location: [true; 3],
            rotation: [true; 3],
            scale: [true; 3],
        }));
        assert!(scene.world_matrix(owner).unwrap().w_axis.truncate().abs_diff_eq(Vec3::new(5.0, 0.0, 0.0), 1e-5));
        scene.object_mut(parent).unwrap().location += Vec3::Z;
        assert!(scene.world_matrix(owner).unwrap().w_axis.truncate().abs_diff_eq(Vec3::new(5.0, 0.0, 1.0), 1e-5));
        let copy = Constraint {
            influence: 0.5,
            ..Constraint::new(ConstraintKind::CopyLocation {
                target: Some(Target { object: parent, bone: None }),
                axes: [true; 3],
                invert: [false; 3],
                offset: false,
                owner_space: Space::World,
                target_space: Space::World,
            })
        };
        scene.object_mut(owner).unwrap().constraints = vec![copy];
        assert!(scene.world_matrix(owner).unwrap().w_axis.truncate().abs_diff_eq(Vec3::new(3.0, 1.0, 2.0), 1e-5));
    }

    #[test]
    fn cycles_are_refused() {
        let mut scene = Scene::empty();
        let a = empty(&mut scene, "A", Vec3::ZERO);
        let b = empty(&mut scene, "B", Vec3::X);
        let track = |object| Constraint::new(ConstraintKind::DampedTrack { target: Some(Target { object, bone: None }), track_axis: TrackAxis::Y });
        scene.object_mut(a).unwrap().constraints.push(track(b));
        assert!(check_cycles(&scene).is_ok());
        scene.object_mut(b).unwrap().constraints.push(track(a));
        assert!(check_cycles(&scene).is_err());
        // Evaluation still ends.
        assert!(scene.world_matrix(a).is_ok());
    }

    #[test]
    fn ik_reaches_a_reachable_target() {
        let mut scene = Scene::empty();
        let goal = empty(&mut scene, "Goal", Vec3::new(0.0, 0.6, 1.4));
        let mut lower = Bone::new("B0", Vec3::ZERO, Vec3::Z);
        lower.roll = 0.0;
        let mut upper = Bone::new("B1", Vec3::Z, Vec3::new(0.0, 0.2, 2.0));
        upper.parent = Some("B0".into());
        upper.connected = true;
        upper.constraints.push(Constraint::new(ConstraintKind::Ik {
            target: Some(Target { object: goal, bone: None }),
            pole: None,
            pole_angle: 0.0,
            chain: 0,
            iterations: 500,
        }));
        let rig = scene.add_object("Rig", ObjectData::Armature(Armature { bones: vec![lower, upper] }));
        let posed = bone_matrices(&scene, rig, 0);
        let tip = posed.get("B1").unwrap().transform_point3(Vec3::new(0.0, Vec3::new(0.0, 0.2, 1.0).length(), 0.0));
        assert!(tip.abs_diff_eq(Vec3::new(0.0, 0.6, 1.4), 1e-3), "{tip}");
    }
}
