//! A particle system's settings, with the reference application's defaults (an emitter of 1000
//! particles from frame 1 to 200 that live 50 frames, halos 0.05 m wide; hair 4 m long in five
//! segments).

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// An object id (the scene's), named by colliders and instanced objects.
pub type Id = u32;

/// What a system makes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// Particles born over a frame range that fly under gravity.
    #[default]
    Emitter,
    /// Strands rooted on the surface.
    Hair,
}

/// Where particles start.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EmitFrom {
    /// On the faces, by area.
    #[default]
    Faces,
    /// On the vertices.
    Verts,
    /// Inside a closed mesh.
    Volume,
}

/// How start points spread.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Distribution {
    /// Evenly, each particle in its own share of the surface, jittered within it.
    #[default]
    Jittered,
    /// Independently at random.
    Random,
}

/// How an emitter's particles show.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RenderAs {
    /// A small blob at each particle.
    #[default]
    Halo,
    /// A copy of `instance_object` at each particle.
    Object,
    /// Nothing (the particles still simulate).
    None,
}

/// Child strands around (or between) the parent strands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Children {
    #[default]
    None,
    /// Copies of each parent, scattered round its root.
    Simple,
    /// New roots on the surface, shaped by the nearest parents.
    Interpolated,
}

/// Hair dynamics: parent strands simulated in the physics bake (roots held on the emitter,
/// bending back towards their rest shape), with the reference's defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HairDynamics {
    /// Steps per frame.
    pub quality: u32,
    /// The mass of each point, kg.
    pub mass: f32,
    /// How firmly strands bend back to their rest shape, and how fast their bending settles.
    pub stiffness: f32,
    pub damping: f32,
    pub air_damping: f32,
    /// How strongly points are drawn to where the rest shape puts them (0: not at all).
    pub pin_stiffness: f32,
    /// Strands collide with collision objects, keeping `collision_distance` metres away.
    pub collision: bool,
    pub collision_distance: f32,
}

impl Default for HairDynamics {
    fn default() -> Self {
        Self { quality: 5, mass: 0.3, stiffness: 0.5, damping: 0.5, air_damping: 1.0, pin_stiffness: 0.0, collision: true, collision_distance: 0.015 }
    }
}

impl HairDynamics {
    /// Checks the ranges; an error names the setting.
    pub fn check(&self) -> Result<(), String> {
        if !(1..=80).contains(&self.quality) {
            return Err("`dynamics.quality` must be between 1 and 80".into());
        }
        let ranges = [
            ("mass", self.mass, 0.0001, 1000.0),
            ("stiffness", self.stiffness, 0.0, 10_000.0),
            ("damping", self.damping, 0.0, 1000.0),
            ("airDamping", self.air_damping, 0.0, 10.0),
            ("pinStiffness", self.pin_stiffness, 0.0, 10_000.0),
            ("collisionDistance", self.collision_distance, 0.001, 1.0),
        ];
        for (name, value, low, high) in ranges {
            if !(value.is_finite() && (low..=high).contains(&value)) {
                return Err(format!("`dynamics.{name}` must be between {low} and {high}"));
            }
        }
        Ok(())
    }
}

fn one() -> f32 {
    1.0
}
fn count() -> u32 {
    1000
}
fn frame_start() -> f32 {
    1.0
}
fn frame_end() -> f32 {
    200.0
}
fn lifetime() -> f32 {
    50.0
}
fn timestep() -> f32 {
    0.04
}
fn bounce() -> f32 {
    0.5
}
fn size() -> f32 {
    0.05
}
fn hair_length() -> f32 {
    4.0
}
fn segments() -> u32 {
    5
}
fn child_count() -> u32 {
    10
}
fn child_radius() -> f32 {
    0.2
}
fn strand_width() -> f32 {
    0.01
}

/// One particle system on a mesh object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParticleSystem {
    pub name: String,
    #[serde(default)]
    pub kind: Kind,
    #[serde(default)]
    pub physics: crate::boids::Physics,
    #[serde(default)]
    pub boids: crate::boids::Boids,
    /// Different seeds give different (but each repeatable) particles.
    #[serde(default)]
    pub seed: u32,
    #[serde(default = "count")]
    pub count: u32,
    /// Emitters: the frames particles are born over.
    #[serde(default = "frame_start")]
    pub frame_start: f32,
    #[serde(default = "frame_end")]
    pub frame_end: f32,
    /// Emitters: frames a particle lives, and how much of it may randomly be cut, 0 to 1.
    #[serde(default = "lifetime")]
    pub lifetime: f32,
    #[serde(default)]
    pub lifetime_random: f32,
    #[serde(default)]
    pub emit_from: EmitFrom,
    #[serde(default)]
    pub distribution: Distribution,
    /// Start speed along the surface normal, metres per second.
    #[serde(default = "one")]
    pub normal_velocity: f32,
    /// Start velocity along the emitter's own axes, metres per second.
    #[serde(default)]
    pub object_velocity: Vec3,
    /// Start speed in a random direction, metres per second.
    #[serde(default)]
    pub random_velocity: f32,
    /// How strongly the scene's gravity (9.81 m/s² down) pulls.
    #[serde(default = "one")]
    pub gravity: f32,
    /// Air drag: the share of speed lost per second.
    #[serde(default)]
    pub drag: f32,
    /// Seconds of simulation per frame (the reference's default is 0.04).
    #[serde(default = "timestep")]
    pub timestep: f32,
    /// Objects particles bounce off (their surfaces, after modifiers).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub colliders: Vec<Id>,
    /// The share of speed kept on a bounce, 0 to 1.
    #[serde(default = "bounce")]
    pub bounce: f32,
    /// A particle that hits a collider dies there.
    #[serde(default)]
    pub die_on_collision: bool,
    #[serde(default)]
    pub render_as: RenderAs,
    /// The object copied at each particle when rendering as objects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_object: Option<Id>,
    /// A particle's size in metres, and how much of it may randomly be taken off, 0 to 1.
    #[serde(default = "size")]
    pub size: f32,
    #[serde(default)]
    pub size_random: f32,
    /// How far instances turn at random, 0 (not at all) to 1 (any way).
    #[serde(default)]
    pub rotation_random: f32,
    /// Hair: a strand's length in metres (the object's own units) and its segments.
    #[serde(default = "hair_length")]
    pub hair_length: f32,
    #[serde(default = "segments")]
    pub segments: u32,
    #[serde(default)]
    pub children: Children,
    /// Children per parent strand.
    #[serde(default = "child_count")]
    pub child_count: u32,
    /// How far simple children spread round their parent's root, metres.
    #[serde(default = "child_radius")]
    pub child_radius: f32,
    /// Hair: a strand's width at its root in metres (it tapers to nothing at the tip).
    #[serde(default = "strand_width")]
    pub strand_width: f32,
    /// The vertex group whose weights set how densely particles and strands start (none: all
    /// alike).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density_group: Option<String>,
    /// Hair: the parent strands are simulated in the physics bake (none: they hold their shape).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamics: Option<HairDynamics>,
    /// Groomed parent strands in the object's own space (each `segments + 1` points, root first);
    /// empty while the hair is as generated. Kept in the hair blob, not with the settings.
    #[serde(skip)]
    pub groomed: Vec<Vec<Vec3>>,
}

/// The most particles or strands a system makes.
pub const MAX_COUNT: u32 = 100_000;
/// The most points a strand has.
pub const MAX_SEGMENTS: u32 = 50;
/// The most children per strand.
pub const MAX_CHILDREN: u32 = 100;

impl ParticleSystem {
    /// A system of `kind` with the reference's defaults.
    pub fn new(name: impl Into<String>, kind: Kind) -> Self {
        Self {
            name: name.into(),
            kind,
            physics: crate::boids::Physics::Newtonian,
            boids: crate::boids::Boids::default(),
            seed: 0,
            count: count(),
            frame_start: frame_start(),
            frame_end: frame_end(),
            lifetime: lifetime(),
            lifetime_random: 0.0,
            emit_from: EmitFrom::Faces,
            distribution: Distribution::Jittered,
            normal_velocity: one(),
            object_velocity: Vec3::ZERO,
            random_velocity: 0.0,
            gravity: one(),
            drag: 0.0,
            timestep: timestep(),
            colliders: Vec::new(),
            bounce: bounce(),
            die_on_collision: false,
            render_as: RenderAs::Halo,
            instance_object: None,
            size: size(),
            size_random: 0.0,
            rotation_random: 0.0,
            hair_length: hair_length(),
            segments: segments(),
            children: Children::None,
            child_count: child_count(),
            child_radius: child_radius(),
            strand_width: strand_width(),
            density_group: None,
            dynamics: None,
            groomed: Vec::new(),
        }
    }

    /// Checks the settings' ranges; an error names the setting.
    pub fn check(&self) -> Result<(), String> {
        let finite = [
            ("frameStart", self.frame_start),
            ("frameEnd", self.frame_end),
            ("lifetime", self.lifetime),
            ("lifetimeRandom", self.lifetime_random),
            ("normalVelocity", self.normal_velocity),
            ("randomVelocity", self.random_velocity),
            ("gravity", self.gravity),
            ("drag", self.drag),
            ("timestep", self.timestep),
            ("bounce", self.bounce),
            ("size", self.size),
            ("sizeRandom", self.size_random),
            ("rotationRandom", self.rotation_random),
            ("hairLength", self.hair_length),
            ("childRadius", self.child_radius),
            ("strandWidth", self.strand_width),
        ];
        if let Some((name, _)) = finite.iter().find(|(_, value)| !value.is_finite()) {
            return Err(format!("`{name}` must be a number"));
        }
        if !self.object_velocity.is_finite() {
            return Err("`objectVelocity` must be numbers".into());
        }
        if self.count > MAX_COUNT {
            return Err(format!("`count` must be at most {MAX_COUNT}"));
        }
        if !(1..=MAX_SEGMENTS).contains(&self.segments) {
            return Err(format!("`segments` must be between 1 and {MAX_SEGMENTS}"));
        }
        if self.child_count > MAX_CHILDREN {
            return Err(format!("`childCount` must be at most {MAX_CHILDREN}"));
        }
        if self.frame_end < self.frame_start {
            return Err("`frameEnd` must not come before `frameStart`".into());
        }
        if self.lifetime <= 0.0 || self.timestep <= 0.0 || self.timestep > 10.0 {
            return Err("`lifetime` and `timestep` must be positive (timestep at most 10 s)".into());
        }
        if self.size < 0.0 || self.hair_length < 0.0 || self.strand_width < 0.0 || self.child_radius < 0.0 {
            return Err("sizes and lengths must not be negative".into());
        }
        for (name, value) in [
            ("lifetimeRandom", self.lifetime_random),
            ("sizeRandom", self.size_random),
            ("rotationRandom", self.rotation_random),
            ("bounce", self.bounce),
        ] {
            if !(0.0..=1.0).contains(&value) {
                return Err(format!("`{name}` must be between 0 and 1"));
            }
        }
        if self.drag < 0.0 {
            return Err("`drag` must not be negative".into());
        }
        self.boids.check(self)?;
        if let Some(dynamics) = &self.dynamics {
            dynamics.check()?;
        }
        let points = self.segments as usize + 1;
        if !self.groomed.is_empty()
            && (self.groomed.len() != self.count as usize
                || self.groomed.iter().any(|strand| strand.len() != points || strand.iter().any(|point| !point.is_finite())))
        {
            return Err("the groomed strands do not match the count and segments".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_parse_from_a_name_alone() {
        let parsed: ParticleSystem = serde_json::from_value(serde_json::json!({ "name": "Rain" })).unwrap();
        assert_eq!(parsed, ParticleSystem::new("Rain", Kind::Emitter));
        assert!(parsed.check().is_ok());
    }

    #[test]
    fn bad_settings_are_refused() {
        let mut system = ParticleSystem::new("P", Kind::Emitter);
        system.lifetime = f32::NAN;
        assert!(system.check().is_err());
        let mut system = ParticleSystem::new("P", Kind::Hair);
        system.segments = 0;
        assert!(system.check().is_err());
        let mut system = ParticleSystem::new("P", Kind::Emitter);
        system.count = MAX_COUNT + 1;
        assert!(system.check().is_err());
    }
}
