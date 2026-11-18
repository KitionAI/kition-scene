//! Physics settings and the baked results the scene shows: the rigid body world, each object's
//! rigid body, cloth, soft body, and collision settings (the reference application's defaults),
//! and the bake cache that `Scene::world_matrix`, `Object::evaluated_mesh`, and hair read at the
//! scene's frame.
//!
//! The settings are part of the scene (saved, merged); the cache is derived: the simulation in
//! `kition-scene-physics` makes it from the settings, and it is never saved (each replica bakes
//! its own; the solver is deterministic, so equal inputs give equal motion).

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::error::{Result, SceneError};
use crate::scene::{Id, Object, Scene};

pub use crate::physics_weights::EffectorWeights;
pub use kition_scene_particles::effector::{Effector, FieldKind, ForceField};

/// The rigid body world: gravity, steps, and the frames it simulates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PhysicsWorld {
    /// Gravity in m/s², for rigid bodies and cloth.
    pub gravity: Vec3,
    /// Solver steps per frame.
    pub substeps: u32,
    /// Constraint solver iterations per step.
    pub iterations: u32,
    /// How fast simulated time runs against the timeline.
    pub time_scale: f32,
    /// The frames the bake covers.
    pub frame_start: i32,
    pub frame_end: i32,
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self { gravity: Vec3::new(0.0, 0.0, -9.81), substeps: 10, iterations: 10, time_scale: 1.0, frame_start: 1, frame_end: 250 }
    }
}

impl PhysicsWorld {
    /// Whether every setting is a new scene's (then nothing is written for it).
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Checks the ranges.
    pub fn check(&self) -> Result<()> {
        if !self.gravity.is_finite() || self.gravity.length() > 1000.0 {
            return Err(SceneError::params("gravity must be finite and at most 1000 m/s²"));
        }
        if !(1..=100).contains(&self.substeps) || !(1..=100).contains(&self.iterations) {
            return Err(SceneError::params("substeps and iterations must be between 1 and 100"));
        }
        if !(self.time_scale.is_finite() && (0.0..=100.0).contains(&self.time_scale)) {
            return Err(SceneError::params("the time scale must be between 0 and 100"));
        }
        if self.frame_end < self.frame_start || self.frame_end - self.frame_start > 10_000 {
            return Err(SceneError::params("the frame range must run forwards and cover at most 10000 frames"));
        }
        Ok(())
    }
}

/// Whether a rigid body moves (active) or only collides (passive).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BodyKind {
    #[default]
    Active,
    Passive,
}

/// The shape a rigid body collides as.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CollisionShape {
    Box,
    Sphere,
    Capsule,
    Cylinder,
    #[default]
    ConvexHull,
    /// The mesh's triangles; passive bodies only (an active one collides as its convex hull).
    Mesh,
}

/// A rigid body's settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RigidBody {
    pub kind: BodyKind,
    /// Simulated (an active body that is not simulated stays put until enabled).
    pub enabled: bool,
    /// Moved by its animation, not by the simulation; pushes active bodies (passive bodies only).
    pub animated: bool,
    pub mass: f32,
    pub friction: f32,
    /// Bounciness, 0 to 1.
    pub restitution: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub shape: CollisionShape,
    /// The collision margin in metres (contacts begin this far apart).
    pub margin: f32,
    /// The body may fall asleep: once it moves slower than the speeds below for two seconds it
    /// stops being simulated until something knocks into it.
    pub deactivation: bool,
    /// The body starts asleep (it hangs where it is until something knocks into it).
    pub start_deactivated: bool,
    /// Below these speeds (m/s and rad/s) a body may fall asleep.
    pub deactivate_linear: f32,
    pub deactivate_angular: f32,
}

impl Default for RigidBody {
    fn default() -> Self {
        Self {
            kind: BodyKind::Active,
            enabled: true,
            animated: false,
            mass: 1.0,
            friction: 0.5,
            restitution: 0.0,
            linear_damping: 0.04,
            angular_damping: 0.1,
            shape: CollisionShape::ConvexHull,
            margin: 0.04,
            deactivation: false,
            start_deactivated: false,
            deactivate_linear: 0.4,
            deactivate_angular: 0.5,
        }
    }
}

/// Cloth settings (a mesh simulated as cloth; it moves first in the mesh's modifier stack).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Cloth {
    pub effector_weights: EffectorWeights,
    /// Steps per frame.
    pub quality: u32,
    /// The mass of each vertex, in kg.
    pub mass: f32,
    pub air_damping: f32,
    pub tension: f32,
    pub compression: f32,
    pub shear: f32,
    pub bending: f32,
    /// Vertices of this vertex group (weight over one half) stay where the mesh puts them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin_group: Option<String>,
    /// How close the cloth comes to collision objects, in metres.
    pub collision_distance: f32,
    /// The cloth collides with itself, keeping `self_distance` apart.
    pub self_collision: bool,
    pub self_distance: f32,
}

impl Default for Cloth {
    fn default() -> Self {
        Self {
            effector_weights: EffectorWeights::default(),
            quality: 5,
            mass: 0.3,
            air_damping: 1.0,
            tension: 15.0,
            compression: 15.0,
            shear: 5.0,
            bending: 0.5,
            pin_group: None,
            collision_distance: 0.015,
            self_collision: false,
            self_distance: 0.015,
        }
    }
}

/// Soft body settings (a mesh whose vertices are simulated as masses on springs; it moves first
/// in the mesh's modifier stack). Time runs per frame, as in the reference application: a soft
/// body moves the same each frame whatever the frame rate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SoftBody {
    pub effector_weights: EffectorWeights,
    /// The mass of each vertex, in kg.
    pub mass: f32,
    /// Air friction.
    pub friction: f32,
    /// How fast simulated time runs.
    pub speed: f32,
    /// Vertices are drawn towards where the mesh puts them (the goal).
    pub goal: bool,
    /// The vertex group whose weights scale each vertex's goal (none: every vertex alike).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_group: Option<String>,
    /// A vertex's goal weight is `goal_min + (goal_max - goal_min) × group weight × goal_default`;
    /// weight 1 follows the mesh exactly.
    pub goal_default: f32,
    pub goal_min: f32,
    pub goal_max: f32,
    /// The goal springs' stiffness (below 1) and damping.
    pub goal_stiffness: f32,
    pub goal_damping: f32,
    /// Springs along the mesh's edges.
    pub edges: bool,
    /// Their stiffness when stretched and when squeezed (below 1), and their damping.
    pub pull: f32,
    pub push: f32,
    pub damping: f32,
    /// Springs between vertices two edges apart (0: none).
    pub bending: f32,
    /// Vertices keep apart from each other, as balls `ball_size` × their edges' mean length
    /// across.
    pub self_collision: bool,
    pub ball_size: f32,
    /// The frames simulated (before them the mesh is at rest; after them it holds).
    pub frame_start: i32,
    pub frame_end: i32,
}

impl Default for SoftBody {
    fn default() -> Self {
        Self {
            effector_weights: EffectorWeights::default(),
            mass: 1.0,
            friction: 0.5,
            speed: 1.0,
            goal: true,
            goal_group: None,
            goal_default: 0.7,
            goal_min: 0.0,
            goal_max: 1.0,
            goal_stiffness: 0.5,
            goal_damping: 0.0,
            edges: true,
            pull: 0.5,
            push: 0.5,
            damping: 0.5,
            bending: 0.0,
            self_collision: false,
            ball_size: 0.49,
            frame_start: 1,
            frame_end: 250,
        }
    }
}

impl SoftBody {
    /// Each vertex's goal weight, for a mesh of `count` vertices with vertex groups `groups`
    /// (a group the mesh lacks counts as none).
    pub fn goal_weights(&self, count: usize, groups: &[crate::vertex_data::VertexGroup]) -> Vec<f32> {
        let span = self.goal_max - self.goal_min;
        match self.goal_group.as_ref().and_then(|name| groups.iter().find(|group| &group.name == name)) {
            Some(group) => {
                (0..count).map(|index| self.goal_min + span * group.weights.get(index).copied().unwrap_or(0.0) * self.goal_default).collect()
            }
            None => vec![self.goal_min + span * self.goal_default; count],
        }
    }
}

/// A collision object for cloth and soft bodies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Collision {
    /// How far outside its surface cloth and soft bodies stay, in metres.
    pub thickness: f32,
    /// Friction against cloth and soft bodies.
    pub friction: f32,
    /// How much of a soft body's bounce it takes (0: the most bounce, 0.26 and up: none).
    pub damping: f32,
}

impl Default for Collision {
    fn default() -> Self {
        Self { thickness: 0.02, friction: 5.0, damping: 0.1 }
    }
}

/// How a rigid body constraint joins its two bodies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JointKind {
    /// Held together as one.
    #[default]
    Fixed,
    /// Joined at the constraint's origin, free to turn any way about it.
    Point,
    /// Turning about the constraint's Z axis only.
    Hinge,
    /// Sliding along the constraint's X axis only, without turning.
    Slider,
    /// Each axis free, limited, or sprung, as set.
    GenericSpring,
    /// Drives relative translation and spin along the constraint's X axis; other axes are free.
    Motor,
}

impl JointKind {
    pub const ALL: [JointKind; 6] = [Self::Fixed, Self::Point, Self::Hinge, Self::Slider, Self::GenericSpring, Self::Motor];

    pub fn name(self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::Point => "point",
            Self::Hinge => "hinge",
            Self::Slider => "slider",
            Self::GenericSpring => "genericSpring",
            Self::Motor => "motor",
        }
    }
}

/// A rigid body constraint: joins two rigid bodies at the owning object's origin, along its
/// axes (as they are when the simulation starts). Its owner is usually an empty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RigidConstraint {
    pub kind: JointKind,
    pub enabled: bool,
    /// The two bodies joined (the constraint does nothing until both are rigid bodies).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object1: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object2: Option<Id>,
    /// The joined bodies pass through each other.
    pub disable_collisions: bool,
    /// The constraint breaks for good when it must push harder than the threshold in one step
    /// (an impulse in newton seconds).
    pub breakable: bool,
    pub breaking_threshold: f32,
    /// Hinge: how far it turns about Z, lower and upper in radians (none: freely).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub angle_limit: Option<[f32; 2]>,
    /// Slider: how far it slides along X, lower and upper in metres (none: freely).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance_limit: Option<[f32; 2]>,
    /// Generic spring: per axis (X, Y, Z) the range it may move or turn in (none: free; equal
    /// ends lock the axis).
    pub linear_limits: [Option<[f32; 2]>; 3],
    pub angular_limits: [Option<[f32; 2]>; 3],
    /// Generic spring: per axis a spring's stiffness (N/m, N·m/rad) pulling back to the start
    /// (none: no spring), and the springs' damping.
    pub linear_springs: [Option<f32>; 3],
    pub angular_springs: [Option<f32>; 3],
    pub spring_damping: f32,
    /// Motor: independent drives along/about local X, disabled by default.
    pub linear_motor: bool,
    pub angular_motor: bool,
    /// Relative target velocity of body 1 minus body 2 (m/s and rad/s).
    pub linear_target_velocity: f32,
    pub angular_target_velocity: f32,
    /// Blender's Max Impulse controls: the solver bounds each step by this value times dt.
    pub linear_max_impulse: f32,
    pub angular_max_impulse: f32,
}

impl Default for RigidConstraint {
    fn default() -> Self {
        Self {
            kind: JointKind::Fixed,
            enabled: true,
            object1: None,
            object2: None,
            disable_collisions: true,
            breakable: false,
            breaking_threshold: 10.0,
            angle_limit: None,
            distance_limit: None,
            linear_limits: [None; 3],
            angular_limits: [None; 3],
            linear_springs: [None; 3],
            angular_springs: [None; 3],
            spring_damping: 0.5,
            linear_motor: false,
            angular_motor: false,
            linear_target_velocity: 1.0,
            angular_target_velocity: 1.0,
            linear_max_impulse: 1.0,
            angular_max_impulse: 1.0,
        }
    }
}

impl RigidConstraint {
    /// Checks the ranges.
    pub fn check(&self) -> Result<()> {
        if !(self.breaking_threshold.is_finite() && (0.0..=1.0e9).contains(&self.breaking_threshold)) {
            return Err(SceneError::params("the breaking threshold must be between 0 and 1000000000"));
        }
        if !(self.spring_damping.is_finite() && (0.0..=1.0e6).contains(&self.spring_damping)) {
            return Err(SceneError::params("spring damping must be between 0 and 1000000"));
        }
        for velocity in [self.linear_target_velocity, self.angular_target_velocity] {
            if !velocity.is_finite() || velocity.abs() > 1.0e5 {
                return Err(SceneError::params("motor target velocity must be finite and between -100000 and 100000"));
            }
        }
        for impulse in [self.linear_max_impulse, self.angular_max_impulse] {
            if !impulse.is_finite() || !(0.0..=1.0e9).contains(&impulse) {
                return Err(SceneError::params("motor maximum impulse must be between 0 and 1000000000"));
            }
        }
        let ranges = [self.angle_limit, self.distance_limit].into_iter().chain(self.linear_limits).chain(self.angular_limits);
        for range in ranges.flatten() {
            if !range.iter().all(|end| end.is_finite() && end.abs() <= 1.0e5) || range[0] > range[1] {
                return Err(SceneError::params("a limit's lower end must be a number at most its upper end"));
            }
        }
        for stiffness in self.linear_springs.iter().chain(&self.angular_springs).flatten() {
            if !(stiffness.is_finite() && (0.0..=1.0e7).contains(stiffness)) {
                return Err(SceneError::params("spring stiffness must be between 0 and 10000000"));
            }
        }
        Ok(())
    }

    /// The objects it names.
    pub fn objects(&self) -> impl Iterator<Item = Id> {
        self.object1.into_iter().chain(self.object2)
    }
}

/// An object's physics: any of a rigid body, cloth or a soft body, and collision.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectPhysics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rigid_body: Option<RigidBody>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloth: Option<Cloth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soft_body: Option<SoftBody>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collision: Option<Collision>,
    /// A force field around the object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<ForceField>,
    /// A rigid body constraint joining two other objects at this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rigid_constraint: Option<RigidConstraint>,
}

impl ObjectPhysics {
    /// Whether nothing is set (then the object has no physics).
    pub fn is_empty(&self) -> bool {
        self.rigid_body.is_none()
            && self.cloth.is_none()
            && self.soft_body.is_none()
            && self.collision.is_none()
            && self.field.is_none()
            && self.rigid_constraint.is_none()
    }

    /// Checks the ranges.
    pub fn check(&self) -> Result<()> {
        let finite_in = |name: &str, value: f32, low: f32, high: f32| {
            if value.is_finite() && (low..=high).contains(&value) {
                Ok(())
            } else {
                Err(SceneError::params(format!("{name} must be between {low} and {high}")))
            }
        };
        if let Some(body) = &self.rigid_body {
            finite_in("mass", body.mass, 0.001, 1.0e6)?;
            finite_in("friction", body.friction, 0.0, 10.0)?;
            finite_in("restitution", body.restitution, 0.0, 1.0)?;
            finite_in("linear damping", body.linear_damping, 0.0, 1.0)?;
            finite_in("angular damping", body.angular_damping, 0.0, 1.0)?;
            finite_in("margin", body.margin, 0.0, 1.0)?;
            finite_in("deactivation linear velocity", body.deactivate_linear, 0.0, 1000.0)?;
            finite_in("deactivation angular velocity", body.deactivate_angular, 0.0, 1000.0)?;
        }
        if let Some(cloth) = &self.cloth {
            cloth.effector_weights.check()?;
            if !(1..=80).contains(&cloth.quality) {
                return Err(SceneError::params("cloth quality must be between 1 and 80"));
            }
            finite_in("cloth mass", cloth.mass, 0.0001, 1000.0)?;
            finite_in("air damping", cloth.air_damping, 0.0, 10.0)?;
            for (name, value) in [("tension", cloth.tension), ("compression", cloth.compression), ("shear", cloth.shear), ("bending", cloth.bending)]
            {
                finite_in(name, value, 0.0, 10_000.0)?;
            }
            finite_in("collision distance", cloth.collision_distance, 0.0, 1.0)?;
            finite_in("self collision distance", cloth.self_distance, 0.001, 1.0)?;
        }
        if let Some(soft) = &self.soft_body {
            soft.effector_weights.check()?;
            finite_in("soft body mass", soft.mass, 0.001, 1000.0)?;
            finite_in("soft body friction", soft.friction, 0.0, 50.0)?;
            finite_in("soft body speed", soft.speed, 0.01, 100.0)?;
            for (name, value) in [("goal default", soft.goal_default), ("goal minimum", soft.goal_min), ("goal maximum", soft.goal_max)] {
                finite_in(name, value, 0.0, 1.0)?;
            }
            if soft.goal_min > soft.goal_max {
                return Err(SceneError::params("the goal minimum must be at most the goal maximum"));
            }
            for (name, value) in [("goal stiffness", soft.goal_stiffness), ("pull", soft.pull), ("push", soft.push)] {
                finite_in(name, value, 0.0, 0.999)?;
            }
            finite_in("goal damping", soft.goal_damping, 0.0, 50.0)?;
            finite_in("edge damping", soft.damping, 0.0, 50.0)?;
            finite_in("bending", soft.bending, 0.0, 10.0)?;
            finite_in("ball size", soft.ball_size, 0.001, 10.0)?;
            if soft.frame_end < soft.frame_start || soft.frame_end - soft.frame_start > 10_000 {
                return Err(SceneError::params("the soft body's frames must run forwards and cover at most 10000 frames"));
            }
        }
        if let Some(collision) = &self.collision {
            finite_in("thickness", collision.thickness, 0.0, 1.0)?;
            finite_in("friction", collision.friction, 0.0, 100.0)?;
            finite_in("collision damping", collision.damping, 0.0, 1.0)?;
        }
        if let Some(field) = &self.field {
            field.check().map_err(SceneError::params)?;
        }
        if let Some(constraint) = &self.rigid_constraint {
            constraint.check()?;
        }
        Ok(())
    }
}

/// A rigid body's motion: its position and orientation at each baked frame, from the first.
#[derive(Debug, Clone, PartialEq)]
pub struct BodyTrack {
    /// The object's world scale, kept as it was at the start.
    pub scale: Vec3,
    pub frames: Vec<(Vec3, Quat)>,
}

/// A bake: every simulated object's motion over the frame range.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Baked {
    /// [`Scene::physics_key`] of the scene it was baked from.
    pub key: u64,
    pub start: i32,
    /// Frames baked so far (a bake in progress shows what it has).
    pub frames: usize,
    pub bodies: BTreeMap<Id, BodyTrack>,
    /// Each cloth's and soft body's vertex positions in its object's own space, per baked frame.
    pub cloths: BTreeMap<Id, Vec<Vec<Vec3>>>,
    /// Each dynamic hair system's parent strands (by object and system index) in the object's
    /// own space, per baked frame.
    pub hair: BTreeMap<(Id, usize), Vec<Vec<Vec<Vec3>>>>,
}

impl Baked {
    /// The (fractional) index into the frames for scene frame `frame`, when the bake has it.
    fn index(&self, frame: f32) -> Option<(usize, usize, f32)> {
        if self.frames == 0 || !frame.is_finite() {
            return None;
        }
        let at = (frame - self.start as f32).max(0.0);
        let last = self.frames - 1;
        let low = (at.floor() as usize).min(last);
        let high = (low + 1).min(last);
        Some((low, high, (at - low as f32).clamp(0.0, 1.0)))
    }
}

/// The bake the scene shows, and the frame it shows it at. Session state: never saved, and two
/// scenes that differ only in it are equal.
#[derive(Debug, Clone, Default)]
pub struct PhysicsCache {
    pub baked: Option<Arc<Baked>>,
    /// The frame `go_to_frame` last went to (the timeline's frame when it never did).
    pub frame: Option<f32>,
    /// The bake matches the scene's physics now (the document checks after every change).
    pub current: bool,
}

impl PartialEq for PhysicsCache {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Scene {
    fn shown_bake(&self) -> Option<(&Baked, f32)> {
        let cache = &self.physics_cache;
        let baked = cache.baked.as_deref().filter(|_| cache.current)?;
        let frame = cache.frame.unwrap_or(self.animation.timeline.frame as f32);
        Some((baked, frame))
    }

    /// The world matrix the bake gives a simulated rigid body now, if any.
    pub fn simulated_world(&self, id: Id) -> Option<Mat4> {
        let (baked, frame) = self.shown_bake()?;
        if frame < baked.start as f32 {
            return None;
        }
        let track = baked.bodies.get(&id)?;
        let (low, high, t) = baked.index(frame)?;
        let (a, b) = (track.frames.get(low)?, track.frames.get(high)?);
        let position = a.0.lerp(b.0, t);
        let rotation = a.1.slerp(b.1, t);
        Some(Mat4::from_scale_rotation_translation(track.scale, rotation, position))
    }

    /// The frames over which the shown bake moves a rigid body (exporters sample them), or
    /// `None` when it does not move one.
    pub fn simulated_range(&self, id: Id) -> Option<(f32, f32)> {
        let (baked, _) = self.shown_bake()?;
        let track = baked.bodies.get(&id)?;
        let first = track.frames.first()?;
        if track.frames.iter().all(|pose| pose == first) {
            return None;
        }
        let start = baked.start as f32;
        Some((start, start + track.frames.len().saturating_sub(1) as f32))
    }

    /// The parent strands the bake gives hair system `index` of object `id` now (object space),
    /// if any.
    pub fn simulated_hair(&self, id: Id, index: usize) -> Option<Vec<Vec<Vec3>>> {
        let (baked, frame) = self.shown_bake()?;
        if frame < baked.start as f32 {
            return None;
        }
        let frames = baked.hair.get(&(id, index))?;
        let (low, high, t) = baked.index(frame)?;
        let (a, b) = (frames.get(low)?, frames.get(high)?);
        Some(a.iter().zip(b).map(|(a, b)| a.iter().zip(b).map(|(a, b)| a.lerp(*b, t)).collect()).collect())
    }

    /// The vertex positions the bake gives a cloth or soft body now (object space), if any.
    pub fn simulated_cloth(&self, id: Id) -> Option<Vec<Vec3>> {
        let (baked, frame) = self.shown_bake()?;
        if frame < baked.start as f32 {
            return None;
        }
        let frames = baked.cloths.get(&id)?;
        let (low, high, t) = baked.index(frame)?;
        let (a, b) = (frames.get(low)?, frames.get(high)?);
        Some(a.iter().zip(b).map(|(a, b)| a.lerp(*b, t)).collect())
    }

    /// What a cloth's shown positions depend on, for evaluation keys.
    pub fn cloth_key(&self, id: Id) -> Option<u64> {
        let (baked, frame) = self.shown_bake()?;
        if !baked.cloths.contains_key(&id) || frame < baked.start as f32 {
            return None;
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        baked.key.hash(&mut hasher);
        baked.frames.hash(&mut hasher);
        frame.to_bits().hash(&mut hasher);
        Some(hasher.finish())
    }

    /// The force fields in the scene now, in world space (hidden objects' fields do nothing).
    pub fn effectors(&self) -> Vec<Effector> {
        self.objects
            .iter()
            .filter(|object| !object.hidden)
            .filter_map(|object| {
                let field = object.physics.as_ref()?.field.clone()?;
                let world = self.world_matrix(object.id).ok()?;
                let origin = world.transform_point3(Vec3::ZERO);
                let axis = world.transform_vector3(Vec3::Z).normalize_or(Vec3::Z);
                Some(Effector { field, origin, axis, owner: object.id })
            })
            .collect()
    }

    /// Checks that rigid body constraints name objects that exist (and not their owner).
    pub fn check_physics_references(&self) -> Result<()> {
        for object in &self.objects {
            let Some(constraint) = object.physics.as_ref().and_then(|physics| physics.rigid_constraint.as_ref()) else { continue };
            if constraint.objects().any(|id| id == object.id || self.object(id).is_err()) {
                return Err(SceneError::InvalidFile(format!("{}'s rigid body constraint names a missing object", object.name)));
            }
        }
        Ok(())
    }

    /// Lets go of deleted objects in rigid body constraints.
    pub fn forget_physics(&mut self, ids: &[Id]) {
        for object in &mut self.objects {
            let Some(constraint) = object.physics.as_mut().and_then(|physics| physics.rigid_constraint.as_mut()) else { continue };
            for slot in [&mut constraint.object1, &mut constraint.object2] {
                if slot.is_some_and(|id| ids.contains(&id)) {
                    *slot = None;
                }
            }
        }
    }

    /// World-space line segments the viewports draw for an object's force field (its shape:
    /// rings for Force, a plane and arrow for Wind, a spiral for Vortex, a wavy ring for
    /// Turbulence, a ring with inward ticks for Drag) and rigid body constraint (lines to the
    /// joined bodies, and the hinge's or slider's axis).
    pub fn physics_lines(&self, id: Id) -> Vec<[Vec3; 2]> {
        let Ok(object) = self.object(id) else { return Vec::new() };
        let Some(physics) = &object.physics else { return Vec::new() };
        let Ok(world) = self.world_matrix(id) else { return Vec::new() };
        // Unscaled: the drawing keeps its size whatever the object's scale.
        let (_, rotation, origin) = world.to_scale_rotation_translation();
        let place = |point: Vec3| origin + rotation * point;
        let mut lines = Vec::new();
        let mut path = |points: &[Vec3], closed: bool| {
            for (a, b) in points.iter().zip(points.iter().skip(1)) {
                lines.push([place(*a), place(*b)]);
            }
            if closed && let (Some(first), Some(last)) = (points.first(), points.last()) {
                lines.push([place(*last), place(*first)]);
            }
        };
        let ring = |radius: f32, z: f32, wave: f32| -> Vec<Vec3> {
            (0..32)
                .map(|step| {
                    let angle = std::f32::consts::TAU * step as f32 / 32.0;
                    let r = radius * (1.0 + wave * (angle * 6.0).sin());
                    Vec3::new(r * angle.cos(), r * angle.sin(), z)
                })
                .collect()
        };
        if let Some(field) = &physics.field {
            match field.kind {
                FieldKind::Force => {
                    for radius in [0.25, 0.5, 0.75] {
                        path(&ring(radius, 0.0, 0.0), true);
                    }
                }
                FieldKind::Wind => {
                    path(&[Vec3::new(-0.5, -0.5, 0.0), Vec3::new(0.5, -0.5, 0.0), Vec3::new(0.5, 0.5, 0.0), Vec3::new(-0.5, 0.5, 0.0)], true);
                    path(&[Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0)], false);
                    path(&[Vec3::new(-0.1, 0.0, 0.85), Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.1, 0.0, 0.85)], false);
                }
                FieldKind::Vortex => {
                    let spiral: Vec<Vec3> = (0..=48)
                        .map(|step| {
                            let t = step as f32 / 48.0;
                            let angle = t * std::f32::consts::TAU * 2.0;
                            Vec3::new(0.75 * t * angle.cos(), 0.75 * t * angle.sin(), 0.0)
                        })
                        .collect();
                    path(&spiral, false);
                }
                FieldKind::Turbulence => path(&ring(0.5, 0.0, 0.15), true),
                FieldKind::Drag => {
                    path(&ring(0.5, 0.0, 0.0), true);
                    for step in 0..8 {
                        let angle = std::f32::consts::TAU * step as f32 / 8.0;
                        let out = Vec3::new(angle.cos(), angle.sin(), 0.0);
                        path(&[out * 0.5, out * 0.3], false);
                    }
                }
            }
        }
        if let Some(constraint) = &physics.rigid_constraint {
            for joined in constraint.objects() {
                if let Ok(other) = self.world_matrix(joined) {
                    lines.push([origin, other.w_axis.truncate()]);
                }
            }
            match constraint.kind {
                JointKind::Hinge => lines.push([place(Vec3::new(0.0, 0.0, -0.5)), place(Vec3::new(0.0, 0.0, 0.5))]),
                JointKind::Slider => lines.push([place(Vec3::new(-0.5, 0.0, 0.0)), place(Vec3::new(0.5, 0.0, 0.0))]),
                _ => {}
            }
        }
        lines
    }

    /// The objects with physics, in scene order.
    pub fn physics_objects(&self) -> impl Iterator<Item = &Object> {
        self.objects.iter().filter(|object| object.physics.as_ref().is_some_and(|physics| !physics.is_empty()))
    }

    /// The objects a bake simulates or collides with: those with physics, and those with
    /// dynamic hair, in scene order.
    pub fn simulated_objects(&self) -> impl Iterator<Item = &Object> {
        self.objects.iter().filter(|object| {
            object.physics.as_ref().is_some_and(|physics| !physics.is_empty()) || object.particles.iter().any(|system| system.dynamics.is_some())
        })
    }

    /// A hash of everything a bake depends on: the world's settings, and each simulated object's
    /// settings, transform, parent, geometry, animation, and dynamic hair. A bake whose key
    /// differs is out of date.
    pub fn physics_key(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_string(&self.physics).unwrap_or_default().hash(&mut hasher);
        for object in self.simulated_objects() {
            object.id.hash(&mut hasher);
            serde_json::to_string(&object.physics).unwrap_or_default().hash(&mut hasher);
            if object.particles.iter().any(|system| system.dynamics.is_some()) {
                serde_json::to_string(&object.particles).unwrap_or_default().hash(&mut hasher);
                for point in object.particles.iter().flat_map(|system| system.groomed.iter().flatten()) {
                    point.to_array().map(f32::to_bits).hash(&mut hasher);
                }
            }
            for value in [object.location, object.rotation, object.scale] {
                value.to_array().map(f32::to_bits).hash(&mut hasher);
            }
            object.parent.hash(&mut hasher);
            object.parent_inverse.to_cols_array().map(f32::to_bits).hash(&mut hasher);
            object.hidden.hash(&mut hasher);
            if object.has_geometry() {
                object.rest_key(self).hash(&mut hasher);
            }
            for channel in self.animation.channels.iter().filter(|channel| channel.target == object.id) {
                serde_json::to_string(channel).unwrap_or_default().hash(&mut hasher);
            }
        }
        // Parents of simulated objects move them too.
        let parents: Vec<Id> = self.simulated_objects().filter_map(|object| object.parent).collect();
        for parent in parents {
            if let Ok(world) = self.world_matrix(parent) {
                world.to_cols_array().map(f32::to_bits).hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    /// Whether the shown bake (if any) matches the scene's physics now; marks it.
    pub fn refresh_physics(&mut self) -> bool {
        let key = self.physics_key();
        let current = self.physics_cache.baked.as_ref().is_some_and(|baked| baked.key == key);
        self.physics_cache.current = current;
        current
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::ObjectData;

    #[test]
    fn goal_weights_scale_between_the_minimum_and_maximum() {
        let soft = SoftBody { goal_group: Some("Pin".into()), goal_min: 0.2, goal_max: 0.6, ..SoftBody::default() };
        let group = crate::vertex_data::VertexGroup { name: "Pin".into(), weights: vec![1.0, 0.5] };
        let weights = soft.goal_weights(3, std::slice::from_ref(&group));
        // 0.2 + 0.4 × weight × 0.7 (the reference's 0.5 weight gives 0.340); outside: the minimum.
        for (got, expected) in weights.iter().zip([0.48, 0.34, 0.2]) {
            assert!((got - expected).abs() < 1e-6, "{weights:?}");
        }
        let missing = SoftBody { goal_group: Some("Gone".into()), ..SoftBody::default() };
        assert_eq!(missing.goal_weights(2, &[group]), vec![0.7, 0.7], "a group the mesh lacks counts as none");
        let mut physics = ObjectPhysics { soft_body: Some(SoftBody::default()), ..ObjectPhysics::default() };
        assert!(physics.check().is_ok());
        if let Some(soft) = physics.soft_body.as_mut() {
            soft.pull = 1.0;
        }
        assert!(physics.check().is_err(), "pull stays below 1");
    }

    #[test]
    fn fields_and_constraints_are_drawn_and_their_objects_let_go() {
        let mut scene = Scene::empty();
        let first = scene.add_object("First", ObjectData::Empty { size: 1.0 });
        let second = scene.add_object("Second", ObjectData::Empty { size: 1.0 });
        scene.object_mut(second).unwrap().location = Vec3::new(2.0, 0.0, 0.0);
        let hinge = scene.add_object("Hinge", ObjectData::Empty { size: 1.0 });
        let constraint = RigidConstraint { kind: JointKind::Hinge, object1: Some(first), object2: Some(second), ..RigidConstraint::default() };
        scene.object_mut(hinge).unwrap().physics = Some(ObjectPhysics { rigid_constraint: Some(constraint), ..ObjectPhysics::default() });
        let lines = scene.physics_lines(hinge);
        assert_eq!(lines.len(), 3, "a line to each body and the hinge's axis");
        assert!(lines.iter().any(|[a, b]| *a == Vec3::ZERO && *b == Vec3::new(2.0, 0.0, 0.0)));
        let wind = scene.add_object("Wind", ObjectData::Empty { size: 1.0 });
        scene.object_mut(wind).unwrap().physics = Some(ObjectPhysics { field: Some(ForceField::new(FieldKind::Wind)), ..ObjectPhysics::default() });
        assert!(scene.physics_lines(wind).iter().any(|[_, b]| (*b - Vec3::Z).length() < 1e-6), "the arrow along Z");
        assert_eq!(scene.effectors().len(), 1);
        scene.validate().unwrap();
        scene.remove_objects(&[second]).unwrap();
        let kept = scene.object(hinge).unwrap().physics.clone().unwrap().rigid_constraint.unwrap();
        assert_eq!((kept.object1, kept.object2), (Some(first), None));
        scene.validate().unwrap();
    }
}
