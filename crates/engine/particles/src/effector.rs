//! Force fields: objects that push particles, cloth, and rigid bodies about, in the manner of the
//! reference application's (Force, Wind, Vortex, Turbulence, and Drag). A field's settings are
//! [`ForceField`]; placed in the world (its object's position and axis) it is an [`Effector`],
//! whose [`Effector::push`] gives the acceleration it gives a particle of unit mass.
//!
//! What each kind does, measured against the reference (`crates/engine/model/tests/
//! fields_blender.rs`): Force pushes away from the field's origin (pulls with a negative
//! strength); Wind pushes along the field's Z axis; Vortex carries particles round its Z axis
//! like a turning flow; Turbulence pushes along a smooth noise field; Drag slows particles down,
//! in proportion to their speed (`linear_drag`) and to its square (`quadratic_drag`). The
//! strength falls off with distance as `(1 + r)^-power`, measured from the origin (Force,
//! Turbulence, Drag) or along the Z axis (Wind, Vortex), and stops at the maximum distance when
//! one is set. Flow turns the push into a flow the particle is carried along with: the particle
//! also slows by `flow` × its speed each second.

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::rng;

/// What a field does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldKind {
    /// Away from (or, negative, towards) the origin.
    #[default]
    Force,
    /// Along the Z axis.
    Wind,
    /// Round the Z axis.
    Vortex,
    /// Along a smooth noise field.
    Turbulence,
    /// Against the motion.
    Drag,
}

impl FieldKind {
    pub const ALL: [FieldKind; 5] = [Self::Force, Self::Wind, Self::Vortex, Self::Turbulence, Self::Drag];

    /// The name commands and files use.
    pub fn name(self) -> &'static str {
        match self {
            Self::Force => "force",
            Self::Wind => "wind",
            Self::Vortex => "vortex",
            Self::Turbulence => "turbulence",
            Self::Drag => "drag",
        }
    }

    /// Whether distance is measured along the Z axis (a plane through the origin) rather than
    /// from the origin.
    pub fn planar(self) -> bool {
        matches!(self, Self::Wind | Self::Vortex)
    }
}

/// A force field's settings, with the reference's defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ForceField {
    pub kind: FieldKind,
    /// How hard it pushes (m/s² on a particle at the origin; unused by Drag).
    pub strength: f32,
    /// Particles are carried along: they also slow by this share of their speed each second.
    pub flow: f32,
    /// How fast the strength falls off with distance (0: not at all).
    pub falloff_power: f32,
    /// Beyond this distance the field does nothing (none: no limit).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_distance: Option<f32>,
    /// Within this distance the field is at full strength; falloff starts here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_distance: Option<f32>,
    /// Turbulence: the size of the noise's swirls in metres.
    pub size: f32,
    /// Turbulence: different seeds give different noise.
    pub seed: u32,
    /// Drag: slowing in proportion to speed, and to speed squared.
    pub linear_drag: f32,
    pub quadratic_drag: f32,
}

impl Default for ForceField {
    fn default() -> Self {
        Self::new(FieldKind::Force)
    }
}

impl ForceField {
    /// A field of `kind` with the reference's defaults (Wind's flow is 1, every other's 0).
    pub fn new(kind: FieldKind) -> Self {
        Self {
            kind,
            strength: 1.0,
            flow: if kind == FieldKind::Wind { 1.0 } else { 0.0 },
            falloff_power: 0.0,
            max_distance: None,
            min_distance: None,
            size: 1.0,
            seed: 0,
            linear_drag: 1.0,
            quadratic_drag: 1.0,
        }
    }

    /// Checks the ranges; an error names the setting.
    pub fn check(&self) -> Result<(), String> {
        let ranges = [
            ("strength", self.strength, -1.0e4, 1.0e4),
            ("flow", self.flow, 0.0, 100.0),
            ("falloffPower", self.falloff_power, 0.0, 10.0),
            ("size", self.size, 0.001, 1000.0),
            ("linearDrag", self.linear_drag, 0.0, 1000.0),
            ("quadraticDrag", self.quadratic_drag, 0.0, 1000.0),
        ];
        for (name, value, low, high) in ranges {
            if !(value.is_finite() && (low..=high).contains(&value)) {
                return Err(format!("`{name}` must be between {low} and {high}"));
            }
        }
        for (name, value) in [("maxDistance", self.max_distance), ("minDistance", self.min_distance)] {
            if value.is_some_and(|distance| !(distance.is_finite() && (0.0..=1.0e5).contains(&distance))) {
                return Err(format!("`{name}` must be between 0 and 100000"));
            }
        }
        Ok(())
    }
}

/// A field placed in the world.
#[derive(Debug, Clone, PartialEq)]
pub struct Effector {
    pub field: ForceField,
    /// The field object's origin and Z axis (unit length) in world space.
    pub origin: Vec3,
    pub axis: Vec3,
    /// The object the field belongs to (it never pushes its own particles, cloth, or body).
    pub owner: u32,
}

/// What a field does at a point: the push (an acceleration on a unit mass, without flow) and
/// the flow slowing it adds.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Push {
    pub force: Vec3,
    pub flow: f32,
}

/// The amplitude that gives turbulence the reference's strength (mean speeds of particles
/// released into it match within the noise of a few hundred samples).
const TURBULENCE_SCALE: f32 = 0.39;

impl Effector {
    /// How much of its strength the field has at `point` (0 beyond its reach).
    pub fn falloff(&self, point: Vec3) -> f32 {
        let offset = point - self.origin;
        let distance = if self.field.kind.planar() { offset.dot(self.axis).abs() } else { offset.length() };
        if self.field.max_distance.is_some_and(|max| distance > max) {
            return 0.0;
        }
        if self.field.falloff_power <= 0.0 {
            return 1.0;
        }
        let beyond = (distance - self.field.min_distance.unwrap_or(0.0)).max(0.0);
        (1.0 + beyond).powf(-self.field.falloff_power)
    }

    /// The push on a particle at `point` moving at `velocity`.
    pub fn push(&self, point: Vec3, velocity: Vec3) -> Push {
        let reach = self.falloff(point);
        if reach <= 0.0 {
            return Push::default();
        }
        let strength = self.field.strength * reach;
        let offset = point - self.origin;
        let force = match self.field.kind {
            FieldKind::Force => offset.normalize_or_zero() * strength,
            FieldKind::Wind => self.axis * strength,
            FieldKind::Vortex => {
                // A flow turning round the axis at `strength` radians a second: the particle is
                // drawn to the flow's speed and turns with it.
                let across = offset - self.axis * offset.dot(self.axis);
                let flow = self.axis.cross(across) * strength;
                (flow - velocity) + self.axis.cross(flow) * strength
            }
            FieldKind::Turbulence => noise_vector(offset / self.field.size.max(1e-3), self.field.seed) * (strength * TURBULENCE_SCALE),
            FieldKind::Drag => -(velocity * self.field.linear_drag + velocity * velocity.length() * self.field.quadratic_drag) * reach,
        };
        let flow = match self.field.kind {
            FieldKind::Vortex | FieldKind::Drag => 0.0,
            _ => self.field.flow * reach.min(1.0),
        };
        Push { force, flow }
    }

    /// The acceleration the field gives a particle of unit mass: its push, less its flow slowing.
    pub fn acceleration(&self, point: Vec3, velocity: Vec3) -> Vec3 {
        let push = self.push(point, velocity);
        push.force - velocity * push.flow
    }
}

/// The sum of `effectors`' accelerations at a point.
pub fn acceleration(effectors: &[Effector], point: Vec3, velocity: Vec3) -> Vec3 {
    effectors.iter().map(|effector| effector.acceleration(point, velocity)).sum()
}

/// A random unit gradient for a lattice corner.
fn gradient(seed: u32, corner: [i32; 3], channel: u32) -> Vec3 {
    let index =
        (corner[0] as u32).wrapping_mul(73_856_093) ^ (corner[1] as u32).wrapping_mul(19_349_663) ^ (corner[2] as u32).wrapping_mul(83_492_791);
    rng::direction(seed, index, channel)
}

/// Smooth gradient noise in about [-1, 1], zero at lattice corners, for `channel`.
pub fn noise(point: Vec3, seed: u32, channel: u32) -> f32 {
    let base = point.floor();
    let local = point - base;
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (u, v, w) = (fade(local.x), fade(local.y), fade(local.z));
    let corner = |dx: i32, dy: i32, dz: i32| {
        let lattice = [base.x as i32 + dx, base.y as i32 + dy, base.z as i32 + dz];
        gradient(seed, lattice, channel).dot(local - Vec3::new(dx as f32, dy as f32, dz as f32))
    };
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(corner(0, 0, 0), corner(1, 0, 0), u);
    let x10 = lerp(corner(0, 1, 0), corner(1, 1, 0), u);
    let x01 = lerp(corner(0, 0, 1), corner(1, 0, 1), u);
    let x11 = lerp(corner(0, 1, 1), corner(1, 1, 1), u);
    lerp(lerp(x00, x10, v), lerp(x01, x11, v), w) * 2.0
}

/// Three independent noises as a vector.
pub fn noise_vector(point: Vec3, seed: u32) -> Vec3 {
    Vec3::new(noise(point, seed, 101), noise(point, seed, 103), noise(point, seed, 107))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(kind: FieldKind, origin: Vec3) -> Effector {
        Effector { field: ForceField::new(kind), origin, axis: Vec3::Z, owner: 0 }
    }

    #[test]
    fn force_pushes_away_and_falls_off() {
        let mut force = at(FieldKind::Force, Vec3::new(2.0, 0.0, 0.0));
        assert!(force.acceleration(Vec3::ZERO, Vec3::ZERO).abs_diff_eq(Vec3::new(-1.0, 0.0, 0.0), 1e-6));
        force.field.falloff_power = 2.0;
        assert!((force.acceleration(Vec3::ZERO, Vec3::ZERO).x + 1.0 / 9.0).abs() < 1e-6, "(1 + 2)^-2");
        force.field.max_distance = Some(1.0);
        assert_eq!(force.acceleration(Vec3::ZERO, Vec3::ZERO), Vec3::ZERO);
    }

    #[test]
    fn wind_measures_distance_along_its_axis_and_flow_slows() {
        let mut wind = at(FieldKind::Wind, Vec3::new(5.0, 0.0, -3.0));
        wind.field.falloff_power = 2.0;
        assert!((wind.acceleration(Vec3::ZERO, Vec3::ZERO).z - 1.0 / 16.0).abs() < 1e-6);
        wind.field.falloff_power = 0.0;
        let moving = wind.acceleration(Vec3::ZERO, Vec3::new(0.0, 0.0, 0.5));
        assert!((moving.z - 0.5).abs() < 1e-6, "flow 1 slows by the speed: {moving}");
    }

    #[test]
    fn vortex_turns_round_its_axis_and_drag_slows() {
        let vortex = at(FieldKind::Vortex, Vec3::new(1.0, 0.0, 0.0));
        let push = vortex.acceleration(Vec3::ZERO, Vec3::ZERO);
        assert!(push.abs_diff_eq(Vec3::new(1.0, -1.0, 0.0), 1e-6), "round and in: {push}");
        let drag = at(FieldKind::Drag, Vec3::ZERO);
        let slowing = drag.acceleration(Vec3::ZERO, Vec3::new(0.0, 0.0, 2.0));
        assert!((slowing.z + 6.0).abs() < 1e-5, "2 + 2²: {slowing}");
    }

    #[test]
    fn noise_is_smooth_bounded_and_repeatable() {
        let mut largest = 0.0f32;
        let mut sum = 0.0f32;
        for step in 0..2000 {
            let point = Vec3::new(step as f32 * 0.137, step as f32 * 0.071, step as f32 * 0.029);
            let value = noise(point, 7, 0);
            largest = largest.max(value.abs());
            sum += value;
            let near = noise(point + Vec3::splat(1e-3), 7, 0);
            assert!((value - near).abs() < 0.02, "smooth");
        }
        assert!(largest <= 2.0 && largest > 0.3, "{largest}");
        assert!((sum / 2000.0).abs() < 0.1, "about zero on average: {}", sum / 2000.0);
        assert_eq!(noise(Vec3::new(0.3, 0.4, 0.5), 1, 2), noise(Vec3::new(0.3, 0.4, 0.5), 1, 2));
        assert_ne!(noise(Vec3::new(0.3, 0.4, 0.5), 1, 2), noise(Vec3::new(0.3, 0.4, 0.5), 2, 2));
    }
}
