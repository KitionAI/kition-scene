//! Airborne boids with averaged Goal, Avoid, Separate and Flock rules. The steering and
//! arrival law were measured through Blender's public API; see tests/blender/boids9.md.

use std::collections::BTreeMap;

use glam::{DVec3, Vec3};
use serde::{Deserialize, Serialize};

use crate::settings::{Id, ParticleSystem};
use crate::sim::{Field, Particle};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Physics {
    #[default]
    Newtonian,
    Boids,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuleKind {
    Goal,
    Avoid,
    Separate,
    Flock,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Rule {
    pub kind: RuleKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Id>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct Boids {
    pub air_speed_max: f32,
    pub air_acc_max: f32,
    pub air_angular_max: f32,
    pub air_personal_space: f32,
    pub rules: Vec<Rule>,
}

impl Default for Boids {
    fn default() -> Self {
        Self {
            air_speed_max: 10.0,
            air_acc_max: 0.5,
            air_angular_max: 0.5,
            air_personal_space: 1.0,
            rules: vec![Rule { kind: RuleKind::Separate, target: None }, Rule { kind: RuleKind::Flock, target: None }],
        }
    }
}

impl Boids {
    pub fn check(&self, system: &ParticleSystem) -> Result<(), String> {
        for (name, value, high) in [
            ("airSpeedMax", self.air_speed_max, 100.0),
            ("airAccMax", self.air_acc_max, 1.0),
            ("airAngularMax", self.air_angular_max, 1.0),
            ("airPersonalSpace", self.air_personal_space, 10.0),
        ] {
            if !value.is_finite() || !(0.0..=high).contains(&value) {
                return Err(format!("boids.{name} must be between 0 and {high}"));
            }
        }
        if self.rules.len() > 8 {
            return Err("boids supports at most eight averaged rules".into());
        }
        if self.rules.iter().any(|rule| matches!(rule.kind, RuleKind::Separate | RuleKind::Flock) && rule.target.is_some()) {
            return Err("only Goal and Avoid rules name a target".into());
        }
        if system.physics == Physics::Boids {
            let span = system.frame_end - system.frame_start + system.lifetime;
            if system.kind != crate::settings::Kind::Emitter {
                return Err("boids physics is only for emitter systems".into());
            }
            if system.frame_start.abs() > 100_000.0
                || system.frame_end.abs() > 100_000.0
                || system.count > 2048
                || !span.is_finite()
                || span > 1000.0
                || span.ceil() * system.count as f32 > 500_000.0
            {
                return Err("boids allows 2048 particles, 1000 emission-plus-lifetime frames, and 500000 particle-frames".into());
            }
            if system.timestep > 0.1 || system.size > 10.0 {
                return Err("boids timestep must be at most 0.1 s and particle size at most 10 m".into());
            }
            if system.emit_from == crate::settings::EmitFrom::Volume {
                return Err("boids emit from faces or vertices; volume emission is not supported".into());
            }
            if event_work(system) > 500_000 {
                return Err("boids birth/death events exceed 500000 particle updates; reduce count, lifetime, or emission duration".into());
            }
            if !system.colliders.is_empty() || system.die_on_collision {
                return Err("boids collision and ground movement are not supported".into());
            }
        }
        Ok(())
    }
}

/// Births, deaths and frame ticks share one deterministic clock. A requested fractional
/// frame can split one interval, so validation reserves one additional update per particle.
fn events(system: &ParticleSystem) -> Vec<(f32, i32)> {
    let mut events = Vec::with_capacity(system.count as usize * 2 + 1002);
    for index in 0..system.count {
        let (birth, life) = crate::sim::birth_and_life(system, index);
        events.push((birth, 1));
        events.push((birth + life, -1));
    }
    let span = (system.frame_end - system.frame_start + system.lifetime).ceil() as u32;
    for tick in 0..=span {
        events.push((system.frame_start + tick as f32, 0));
    }
    events.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
    events
}

fn event_work(system: &ParticleSystem) -> u64 {
    let mut active = 0_i32;
    let mut previous = system.frame_start;
    let mut work = u64::from(system.count);
    for (time, delta) in events(system) {
        if time > previous {
            work += active.max(0) as u64;
            previous = time;
        }
        active += delta;
    }
    work
}

/// Deterministic spatial search. Each query visits at most 128 nodes and keeps 32 neighbors;
/// neither a dense cluster nor coincident points can make evaluation quadratic.
struct Tree {
    index: usize,
    axis: usize,
    left: Option<Box<Tree>>,
    right: Option<Box<Tree>>,
}

fn component(v: Vec3, axis: usize) -> f32 {
    match axis {
        0 => v.x,
        1 => v.y,
        _ => v.z,
    }
}

impl Tree {
    fn build(indices: &mut [usize], particles: &[Particle], depth: usize) -> Option<Box<Self>> {
        if indices.is_empty() {
            return None;
        }
        let axis = depth % 3;
        let middle = indices.len() / 2;
        let (left, index, right) = indices.select_nth_unstable_by(middle, |a, b| {
            let value = |index: usize| particles.get(index).map_or(0.0, |p| component(p.position, axis));
            value(*a).total_cmp(&value(*b)).then(a.cmp(b))
        });
        Some(Box::new(Self { index: *index, axis, left: Self::build(left, particles, depth + 1), right: Self::build(right, particles, depth + 1) }))
    }

    fn near(&self, point: Vec3, own: usize, particles: &[Particle], visited: &mut usize, best: &mut Vec<(f64, usize)>) {
        if *visited >= 128 {
            return;
        }
        *visited += 1;
        let Some(particle) = particles.get(self.index) else {
            return;
        };
        let distance = point.as_dvec3().distance_squared(particle.position.as_dvec3());
        if self.index != own {
            let at = best.binary_search_by(|other| other.0.total_cmp(&distance).then(other.1.cmp(&self.index))).unwrap_or_else(|at| at);
            if at < 32 {
                if best.len() == 32 {
                    best.pop();
                }
                best.insert(at, (distance, self.index));
            }
        }
        let delta = f64::from(component(point, self.axis)) - f64::from(component(particle.position, self.axis));
        let (near, far) = if delta < 0.0 { (&self.left, &self.right) } else { (&self.right, &self.left) };
        if let Some(near) = near {
            near.near(point, own, particles, visited, best);
        }
        if (best.len() < 32 || best.last().is_some_and(|last| delta * delta <= last.0))
            && let Some(far) = far
        {
            far.near(point, own, particles, visited, best);
        }
    }
}

/// Advance initialized live boids together. Reading the old array for every rule makes the
/// result independent of which boid is visited first. Targets are world-space static points.
pub fn step(settings: &Boids, targets: &BTreeMap<Id, Vec3>, particles: &[Particle], dt: f32) -> Vec<Particle> {
    let interacts = settings.rules.iter().any(|rule| matches!(rule.kind, RuleKind::Separate | RuleKind::Flock));
    let tree = interacts.then(|| Tree::build(&mut (0..particles.len()).collect::<Vec<_>>(), particles, 0)).flatten();
    particles
        .iter()
        .enumerate()
        .map(|(index, particle)| {
            let mut near = if interacts { Vec::with_capacity(32) } else { Vec::new() };
            if let Some(tree) = &tree {
                tree.near(particle.position, index, particles, &mut 0, &mut near);
            }
            let mut desired = DVec3::ZERO;
            let mut count = 0;
            let mut requested_speed = 0.0;
            let mut separate = DVec3::ZERO;
            for rule in &settings.rules {
                let response = match rule.kind {
                    RuleKind::Goal | RuleKind::Avoid => {
                        rule.target.and_then(|target| targets.get(&target)).filter(|target| target.is_finite()).map(|target| {
                            let delta = target.as_dvec3() - particle.position.as_dvec3();
                            let speed = particle.velocity.length();
                            let arrival = if rule.kind == RuleKind::Goal && speed > 1e-8 {
                                (delta.length() / (2.0 * f64::from(speed))).min(1.0).powi(3) as f32
                            } else {
                                1.0
                            };
                            (delta * if rule.kind == RuleKind::Avoid { -1.0 } else { 1.0 }, settings.air_speed_max * arrival)
                        })
                    }
                    RuleKind::Separate => {
                        let mut away = DVec3::ZERO;
                        for (_, neighbor) in &near {
                            let Some(other) = particles.get(*neighbor) else {
                                continue;
                            };
                            let delta = particle.position.as_dvec3() - other.position.as_dvec3();
                            let distance = delta.length();
                            if distance < f64::from((particle.size + other.size) * settings.air_personal_space) && distance > 1e-8 {
                                away += delta / distance * (f64::from((particle.size + other.size) * settings.air_personal_space) - distance);
                            }
                        }
                        if away.length_squared() > 1e-12 {
                            separate += away;
                            Some((away, settings.air_speed_max))
                        } else {
                            None
                        }
                    }
                    RuleKind::Flock => {
                        let mut center = DVec3::ZERO;
                        let mut velocity = DVec3::ZERO;
                        let mut total = 0;
                        for (_, neighbor) in &near {
                            if let Some(other) = particles.get(*neighbor) {
                                center += other.position.as_dvec3();
                                velocity += other.velocity.as_dvec3();
                                total += 1;
                            }
                        }
                        (total > 0).then(|| {
                            let desired =
                                center / total as f64 - particle.position.as_dvec3() + velocity / total as f64 - particle.velocity.as_dvec3();
                            (desired, desired.length().min(f64::from(settings.air_speed_max)) as f32)
                        })
                    }
                };
                if let Some((response, speed)) = response {
                    desired += response;
                    requested_speed += speed;
                    count += 1;
                }
            }
            if count > 0 {
                desired /= count as f64;
                requested_speed /= count as f32;
            }
            let speed = particle.velocity.length();
            let wanted_speed = requested_speed.min(settings.air_speed_max).min(speed + settings.air_speed_max * settings.air_acc_max);
            let direction = if speed <= 1e-8 {
                if separate.length_squared() > 1e-12 { separate.normalize_or_zero().as_vec3() } else { Vec3::Z }
            } else {
                turn(particle.velocity / speed, desired.normalize_or_zero().as_vec3(), settings.air_angular_max * std::f32::consts::PI)
            };
            let velocity = particle.velocity.lerp(direction * wanted_speed, dt);
            Particle { position: particle.position + (particle.velocity + velocity) * (0.5 * dt), velocity, ..*particle }
        })
        .collect()
}

fn turn(current: Vec3, desired: Vec3, limit: f32) -> Vec3 {
    if desired == Vec3::ZERO {
        return current;
    }
    let dot = current.dot(desired).clamp(-1.0, 1.0);
    let angle = dot.acos();
    if angle <= limit {
        return desired;
    }
    let side = desired - current * dot;
    if side.length_squared() <= 1e-12 {
        return current;
    }
    let side = side.normalize_or_zero();
    current * limit.cos() + side * limit.sin()
}

/// Replay from emission with simultaneous updates. Settings bound the coupled replay work;
/// surface preparation remains linear in mesh size. Seeking and rewinding are history-free.
pub fn particles_at(system: &ParticleSystem, field: &Field, frame: f32) -> Vec<Particle> {
    if !frame.is_finite() || frame < system.frame_start || frame >= system.frame_end + system.lifetime || system.check().is_err() {
        return Vec::new();
    }
    let starts = crate::surface::starts(&field.surface, system.emit_from, system.distribution, system.count, system.seed);
    let lifetimes: Vec<(f32, f32)> = (0..starts.len())
        .map(|index| {
            let (birth, life) = crate::sim::birth_and_life(system, index as u32);
            (birth, birth + life)
        })
        .collect();
    let mut times = events(system).into_iter().peekable();
    let mut particles = Vec::new();
    let mut next_birth = 0;
    let mut time = system.frame_start;
    loop {
        // Births are ordered by emission index. Each is visited once; a deleted or
        // nonfinite start does not make the other particles wait for a whole frame.
        while let Some((birth, death)) = lifetimes.get(next_birth).copied() {
            if birth > time {
                break;
            }
            let index = next_birth as u32;
            let start = starts.get(next_birth);
            next_birth += 1;
            if let Some(start) = start.filter(|start| start.position.is_finite() && time < death) {
                particles.push(Particle {
                    index,
                    position: start.position,
                    velocity: Vec3::ZERO,
                    age: time - birth,
                    size: system.size * (1.0 - system.size_random * crate::rng::unit(system.seed, index, 24)),
                    rotation: (Vec3::new(
                        crate::rng::unit(system.seed, index, 25),
                        crate::rng::unit(system.seed, index, 26),
                        crate::rng::unit(system.seed, index, 27),
                    ) * 2.0
                        - Vec3::ONE)
                        * (system.rotation_random * std::f32::consts::PI),
                });
            }
        }
        particles.retain(|particle| lifetimes.get(particle.index as usize).is_some_and(|(_, death)| time < *death));
        if time >= frame {
            break;
        }
        while times.peek().is_some_and(|(event, _)| *event <= time) {
            times.next();
        }
        let next = times.peek().map_or(frame, |(event, _)| event.min(frame));
        particles = step(&system.boids, &field.boid_targets, &particles, (next - time) * system.timestep);
        time = next;
    }
    for particle in &mut particles {
        if let Some((birth, _)) = lifetimes.get(particle.index as usize) {
            particle.age = frame - birth;
        }
    }
    particles
}
