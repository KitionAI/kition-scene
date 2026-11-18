//! Emitter particles: born evenly over the frame range at their start points, flying under
//! gravity and drag, bouncing off colliders. Nothing carries over between frames: a particle at
//! a frame is worked out from its birth with the seed, so any frame can be shown at once and
//! every replica gets the same particles. A [`Cache`] keeps recent frames for playback.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use glam::{Mat3, Vec3};

use crate::effector::{self, Effector};
use crate::rng;
use crate::settings::{Kind, ParticleSystem};
use crate::surface::{self, Start, Surface};

/// The scene's gravity, metres per second squared.
pub const GRAVITY: Vec3 = Vec3::new(0.0, 0.0, -9.81);

/// One live particle at a frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Particle {
    pub index: u32,
    pub position: Vec3,
    pub velocity: Vec3,
    /// Frames since birth.
    pub age: f32,
    pub size: f32,
    /// XYZ Euler angles in radians (instances turn by it).
    pub rotation: Vec3,
}

/// What an emitter flies through, in world space.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Field {
    /// The emitter's surface in world space.
    pub surface: Surface,
    /// The emitter's world rotation (its own axes, for object-aligned velocity).
    pub axes: Mat3,
    /// Colliders' triangles in world space.
    pub colliders: Vec<[Vec3; 3]>,
    /// The force fields that push the particles (as they are at the frame shown).
    pub effectors: Vec<Effector>,
    /// Static world-space points named by boid rules.
    pub boid_targets: std::collections::BTreeMap<crate::settings::Id, Vec3>,
}

// Random channels.
const LIFE: u32 = 20;
const SPEED: u32 = 21;
const SIZE: u32 = 24;
const TURN: u32 = 25;

/// When particle `index` is born (frames), and how long it lives.
pub fn birth_and_life(system: &ParticleSystem, index: u32) -> (f32, f32) {
    let span = system.frame_end - system.frame_start;
    let birth = system.frame_start + if system.count > 1 { span * index as f32 / system.count as f32 } else { 0.0 };
    let life = system.lifetime * (1.0 - system.lifetime_random * rng::unit(system.seed, index, LIFE));
    (birth, life.max(1e-3))
}

fn first_hit(from: Vec3, to: Vec3, colliders: &[[Vec3; 3]]) -> Option<(f32, Vec3)> {
    let step = to - from;
    let length = step.length();
    if length <= 1e-9 {
        return None;
    }
    let direction = step / length;
    let mut best: Option<(f32, Vec3)> = None;
    for [a, b, c] in colliders {
        if let Some(t) = surface::ray_hits(from, direction, *a, *b, *c)
            && t <= length
            && best.is_none_or(|(nearest, _)| t < nearest)
        {
            best = Some((t, (*b - *a).cross(*c - *a).normalize_or_zero()));
        }
    }
    best.map(|(t, normal)| (t / length, normal))
}

/// Particle `index` from `start` flown for `age` frames; `None` when it died on a collider.
fn fly(system: &ParticleSystem, field: &Field, index: u32, start: Start, age: f32) -> Option<Particle> {
    let mut velocity = start.normal * system.normal_velocity
        + field.axes * system.object_velocity
        + rng::direction(system.seed, index, SPEED) * system.random_velocity;
    let mut position = start.position;
    let seconds = age * system.timestep;
    let steps = age.ceil().clamp(1.0, 10_000.0) as u32;
    let h = seconds / steps as f32;
    let gravity = GRAVITY * system.gravity;
    for _ in 0..steps {
        let (next, next_velocity) = if field.effectors.is_empty() {
            let acceleration = gravity - velocity * system.drag;
            (position + velocity * h + acceleration * (0.5 * h * h), velocity + acceleration * h)
        } else {
            // Fields depend on where the particle is and how fast it goes: a Runge-Kutta step.
            let accelerate = |point: Vec3, speed: Vec3| gravity - speed * system.drag + effector::acceleration(&field.effectors, point, speed);
            let (p1, v1) = (position, velocity);
            let a1 = accelerate(p1, v1);
            let (p2, v2) = (position + v1 * (h * 0.5), velocity + a1 * (h * 0.5));
            let a2 = accelerate(p2, v2);
            let (p3, v3) = (position + v2 * (h * 0.5), velocity + a2 * (h * 0.5));
            let a3 = accelerate(p3, v3);
            let (p4, v4) = (position + v3 * h, velocity + a3 * h);
            let a4 = accelerate(p4, v4);
            let _ = p4;
            (position + (v1 + v2 * 2.0 + v3 * 2.0 + v4) * (h / 6.0), velocity + (a1 + a2 * 2.0 + a3 * 2.0 + a4) * (h / 6.0))
        };
        match first_hit(position, next, &field.colliders) {
            Some((share, normal)) => {
                if system.die_on_collision {
                    return None;
                }
                // Bounce: the hit point, the rest of the step's travel mirrored, speed kept by `bounce`.
                let hit = position + (next - position) * share;
                let normal = if normal.dot(next - position) > 0.0 { -normal } else { normal };
                let mirror = |vector: Vec3| vector - normal * (2.0 * vector.dot(normal));
                velocity = mirror(next_velocity) * system.bounce;
                position = hit + normal * 1e-4 + mirror(next - hit) * system.bounce;
            }
            None => {
                position = next;
                velocity = next_velocity;
            }
        }
    }
    let size = system.size * (1.0 - system.size_random * rng::unit(system.seed, index, SIZE));
    let turn = system.rotation_random * std::f32::consts::PI;
    let rotation = Vec3::new(rng::unit(system.seed, index, TURN), rng::unit(system.seed, index, TURN + 1), rng::unit(system.seed, index, TURN + 2))
        * 2.0
        - Vec3::ONE;
    Some(Particle { index, position, velocity, age, size, rotation: rotation * turn })
}

/// The emitter's live particles at `frame`.
pub fn particles_at(system: &ParticleSystem, field: &Field, frame: f32) -> Vec<Particle> {
    if system.kind != Kind::Emitter || system.count == 0 {
        return Vec::new();
    }
    if system.physics == crate::boids::Physics::Boids {
        return crate::boids::particles_at(system, field, frame);
    }
    let starts = surface::starts(&field.surface, system.emit_from, system.distribution, system.count, system.seed);
    starts
        .iter()
        .enumerate()
        .filter_map(|(index, start)| {
            let index = u32::try_from(index).ok()?;
            let (birth, life) = birth_and_life(system, index);
            let age = frame - birth;
            (age >= 0.0 && age < life).then(|| fly(system, field, index, *start, age)).flatten()
        })
        .collect()
}

/// Recent frames' particles, by what they depend on.
#[derive(Debug, Default)]
pub struct Cache {
    frames: HashMap<u64, Arc<Vec<Particle>>>,
    order: VecDeque<u64>,
}

/// Frames a cache keeps.
const CACHED: usize = 600;

fn key(system: &ParticleSystem, field: &Field, frame: f32) -> u64 {
    let mut hasher = DefaultHasher::new();
    format!("{system:?}").hash(&mut hasher);
    frame.to_bits().hash(&mut hasher);
    for point in field.surface.positions.iter().chain(field.surface.normals.iter()) {
        point.to_array().map(f32::to_bits).hash(&mut hasher);
    }
    field.surface.triangles.hash(&mut hasher);
    field.axes.to_cols_array().map(f32::to_bits).hash(&mut hasher);
    for triangle in &field.colliders {
        for point in triangle {
            point.to_array().map(f32::to_bits).hash(&mut hasher);
        }
    }
    field.surface.weights.iter().map(|weight| weight.to_bits()).for_each(|bits| bits.hash(&mut hasher));
    format!("{:?}", field.effectors).hash(&mut hasher);
    format!("{:?}", field.boid_targets).hash(&mut hasher);
    hasher.finish()
}

impl Cache {
    pub fn new() -> Self {
        Self::default()
    }

    /// [`particles_at`], from the cache when this frame of this system was worked out before.
    pub fn particles_at(&mut self, system: &ParticleSystem, field: &Field, frame: f32) -> Arc<Vec<Particle>> {
        let key = key(system, field, frame);
        if let Some(found) = self.frames.get(&key) {
            return found.clone();
        }
        let made = Arc::new(particles_at(system, field, frame));
        self.frames.insert(key, made.clone());
        self.order.push_back(key);
        while self.order.len() > CACHED {
            if let Some(old) = self.order.pop_front() {
                self.frames.remove(&old);
            }
        }
        made
    }

    /// Frames held.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effector::{FieldKind, ForceField};

    fn plane_field() -> Field {
        Field {
            surface: Surface {
                positions: vec![Vec3::new(-1.0, -1.0, 0.0), Vec3::new(1.0, -1.0, 0.0), Vec3::new(1.0, 1.0, 0.0), Vec3::new(-1.0, 1.0, 0.0)],
                normals: vec![Vec3::Z; 4],
                triangles: vec![[0, 1, 2], [0, 2, 3]],
                weights: Vec::new(),
            },
            axes: Mat3::IDENTITY,
            colliders: Vec::new(),
            effectors: Vec::new(),
            boid_targets: Default::default(),
        }
    }

    #[test]
    fn births_spread_over_the_range_and_particles_die_at_their_lifetime() {
        let system = ParticleSystem::new("P", Kind::Emitter);
        let field = plane_field();
        // 1000 over 199 frames: about five a frame, each living 50 frames.
        assert_eq!(particles_at(&system, &field, 0.5).len(), 0);
        let at_10 = particles_at(&system, &field, 10.0).len();
        assert!((44..=47).contains(&at_10), "{at_10}");
        let at_100 = particles_at(&system, &field, 100.0).len();
        assert!((250..=252).contains(&at_100), "{at_100}");
        assert_eq!(particles_at(&system, &field, 260.0).len(), 0);
    }

    #[test]
    fn flight_follows_the_ballistic_curve() {
        let system = ParticleSystem::new("P", Kind::Emitter);
        let field = plane_field();
        for particle in particles_at(&system, &field, 30.0) {
            let t = particle.age * system.timestep;
            let height = t - 0.5 * 9.81 * t * t;
            assert!((particle.position.z - height).abs() < 1e-4, "{} against {height}", particle.position.z);
        }
    }

    #[test]
    fn a_floor_bounces_and_kills() {
        let mut system = ParticleSystem::new("P", Kind::Emitter);
        let mut field = plane_field();
        let floor = |z: f32| {
            [
                [Vec3::new(-9.0, -9.0, z), Vec3::new(9.0, -9.0, z), Vec3::new(9.0, 9.0, z)],
                [Vec3::new(-9.0, -9.0, z), Vec3::new(9.0, 9.0, z), Vec3::new(-9.0, 9.0, z)],
            ]
        };
        field.colliders = floor(-0.5).to_vec();
        let lowest = particles_at(&system, &field, 120.0).iter().map(|particle| particle.position.z).fold(f32::INFINITY, f32::min);
        assert!(lowest > -0.5 - 1e-3, "nothing falls through: {lowest}");
        system.die_on_collision = true;
        let alive = particles_at(&system, &field, 120.0).len();
        let free = particles_at(&system, &plane_field(), 120.0).len();
        assert!(alive < free, "{alive} {free}");
    }

    #[test]
    fn fields_push_particles_as_the_reference_does() {
        // One particle at rest on a tiny plane, no gravity: the reference's numbers at frame 30
        // (1.16 s after birth) for a Force field 2 m away (pushed 0.672 m), a Wind with flow 1
        // (carried 0.474 m), and Drag on a particle leaving at 2 m/s (0.865 m, now 0.265 m/s).
        let mut system = ParticleSystem::new("P", Kind::Emitter);
        system.count = 1;
        system.frame_end = 1.0;
        system.lifetime = 200.0;
        system.normal_velocity = 0.0;
        system.gravity = 0.0;
        let mut field = plane_field();
        // A plane 1 mm wide, as the reference's.
        field.surface.positions.iter_mut().for_each(|point| *point *= 0.0005);
        let place = |kind: FieldKind, origin: Vec3| Effector { field: ForceField::new(kind), origin, axis: Vec3::Z, owner: 0 };
        field.effectors = vec![place(FieldKind::Force, Vec3::new(2.0, 0.0, 0.0))];
        let start = particles_at(&system, &field, 1.0)[0].position;
        let pushed = particles_at(&system, &field, 30.0)[0].position - start;
        assert!((pushed.x + 0.672).abs() < 0.005, "{pushed}");
        field.effectors = vec![place(FieldKind::Wind, Vec3::new(0.0, 0.0, -3.0))];
        let carried = particles_at(&system, &field, 30.0)[0].position - start;
        assert!((carried.z - 0.4736).abs() < 0.005, "{carried}");
        system.normal_velocity = 2.0;
        field.effectors = vec![place(FieldKind::Drag, Vec3::ZERO)];
        let slowed = particles_at(&system, &field, 30.0)[0];
        assert!((slowed.position.z - start.z - 0.8647).abs() < 0.01 && (slowed.velocity.z - 0.2651).abs() < 0.005, "{slowed:?}");
    }

    #[test]
    fn the_cache_answers_the_same() {
        let system = ParticleSystem::new("P", Kind::Emitter);
        let field = plane_field();
        let mut cache = Cache::new();
        let first = cache.particles_at(&system, &field, 42.0);
        assert_eq!(*first, particles_at(&system, &field, 42.0));
        let again = cache.particles_at(&system, &field, 42.0);
        assert!(Arc::ptr_eq(&first, &again));
        assert_eq!(cache.len(), 1);
    }
}
