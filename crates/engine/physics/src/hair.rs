//! Hair dynamics: each parent strand a chain of points with mass, its root held on the emitter,
//! its segments keeping their length, and each joint bending back towards the strand's rest
//! shape (as groomed or grown, carried along by the emitter), solved as extended
//! position-based dynamics with damping, air damping, a pin towards the rest shape, gravity,
//! force fields, and collisions with collision objects.
//!
//! Bending is the angular kind: at each joint, the next segment is held at its rest direction
//! turned the way the segment before it has turned (at the root, the rest direction itself).
//! Its stiffness is per unit of the strand's length, so a strand droops into the same shape
//! whatever its length, as in the reference application.

use glam::{Quat, Vec3};
use kition_scene_model::physics::{Effector, FieldKind};

use crate::cloth::ColliderSurface;

/// Hair settings the solver uses (the reference's names).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HairSettings {
    /// Each point's mass, kg.
    pub mass: f32,
    /// Bending stiffness, and the damping of bending.
    pub stiffness: f32,
    pub damping: f32,
    pub air_damping: f32,
    /// How strongly points are drawn to their rest place (0: not at all).
    pub pin_stiffness: f32,
    /// How close strands come to collision objects (none: they pass through).
    pub collision_distance: Option<f32>,
    pub gravity: Vec3,
}

/// Bending stiffness per unit of the setting (newtons per metre of deviation, times the segment's
/// length), measured against the reference's hair drooping under gravity: 1 m strands of four
/// segments held out level settle with their tips 0.993, 0.660, and 0.088 m down at stiffness
/// 0.05, 0.5, and 5 (the reference: 0.996, 0.661, 0.088).
pub const BEND_SCALE: f32 = 50.0;
/// Damping of bending per unit of the setting (newton seconds per metre).
pub const BEND_DAMPING: f32 = 1.0;
/// The pull towards the rest shape per unit of pin stiffness, newtons per metre (the reference's
/// level strands with pin stiffness 1 settle 0.175 m down).
pub const PIN_SCALE: f32 = 18.0;
/// Air damping: the share of speed a point of 1 kg loses each second per unit (fitted with the
/// bending damping to the reference's strands falling with air damping 1 and 10).
const AIR: f32 = 0.36;
/// How hard fields push a point, newtons per unit of strength (the reference's upright strands
/// in a Force field and a Wind, of 0.3 and 1 kg points).
pub const FIELD_SCALE: f32 = 0.4;
/// The time a frame stands for: the reference's hair moves the same each frame whatever the
/// frame rate (1/√1000 s, as its soft bodies).
pub const FRAME_TIME: f32 = 0.031_622_775;
/// Segments keep their length this firmly (metres per newton).
const STRETCH_COMPLIANCE: f32 = 1e-7;

/// Simulated parent strands, in world coordinates, root first.
#[derive(Debug, Clone)]
pub struct HairSim {
    pub strands: Vec<Vec<Vec3>>,
    velocities: Vec<Vec<Vec3>>,
    /// Each segment's rest length.
    lengths: Vec<Vec<f32>>,
    settings: HairSettings,
}

impl HairSim {
    /// Strands at rest at `rest` (world positions, root first).
    pub fn new(rest: Vec<Vec<Vec3>>, settings: HairSettings) -> Self {
        let lengths =
            rest.iter().map(|strand| strand.windows(2).map(|pair| if let [a, b] = pair { a.distance(*b) } else { 0.0 }).collect()).collect();
        let velocities = rest.iter().map(|strand| vec![Vec3::ZERO; strand.len()]).collect();
        Self { strands: rest, velocities, lengths, settings }
    }

    /// One step of `dt`. `rest` is the strands' rest shape where the emitter is now (world, the
    /// same points); roots go to its roots.
    pub fn step(&mut self, dt: f32, iterations: u32, rest: &[Vec<Vec3>], colliders: &[ColliderSurface], effectors: &[Effector]) {
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        let settings = self.settings;
        let mass = settings.mass.max(1e-6);
        let inverse_mass = 1.0 / mass;
        let damping = 1.0 / (1.0 + settings.air_damping * AIR / mass * dt);
        let bend_damping = settings.damping * BEND_DAMPING * dt * inverse_mass;
        let alpha_stretch = STRETCH_COMPLIANCE / (dt * dt);
        let alpha_pin = if settings.pin_stiffness > 0.0 { Some(1.0 / (settings.pin_stiffness * PIN_SCALE * dt * dt)) } else { None };
        for (index, ((strand, velocities), lengths)) in self.strands.iter_mut().zip(&mut self.velocities).zip(&self.lengths).enumerate() {
            let Some(rest) = rest.get(index).filter(|rest| rest.len() == strand.len()) else { continue };
            let previous = strand.clone();
            // Move: the root to its place, the rest under gravity, fields, and air damping.
            for (point, (position, velocity)) in strand.iter_mut().zip(velocities.iter_mut()).enumerate() {
                if point == 0 {
                    if let Some(root) = rest.first() {
                        *position = *root;
                    }
                    continue;
                }
                let mut acceleration = settings.gravity;
                for effector in effectors {
                    let push = effector.push(*position, *velocity);
                    let force = if effector.field.kind == FieldKind::Drag { push.force * mass } else { push.force * FIELD_SCALE };
                    acceleration += force * inverse_mass;
                }
                *velocity = (*velocity + acceleration * dt) * damping;
                *position += *velocity * dt;
            }
            let rest_directions: Vec<Vec3> =
                rest.windows(2).map(|pair| if let [a, b] = pair { (*b - *a).normalize_or_zero() } else { Vec3::ZERO }).collect();
            let segments = strand.len().saturating_sub(1);
            let mut stretch = vec![0.0f32; segments];
            let mut bend = vec![Vec3::ZERO; segments];
            let mut pin = vec![Vec3::ZERO; strand.len()];
            let weight = |point: usize| if point == 0 { 0.0 } else { inverse_mass };
            for _ in 0..iterations.max(1) {
                for segment in 0..segments {
                    let (Some(&a), Some(&b), Some(&length)) = (strand.get(segment), strand.get(segment + 1), lengths.get(segment)) else { continue };
                    // Bending: the segment towards its rest direction turned as its parent turned.
                    let direction = match segment {
                        0 => rest_directions.first().copied().unwrap_or(Vec3::ZERO),
                        _ => {
                            let (Some(&before), Some(&rest_before), Some(&rest_here)) =
                                (strand.get(segment - 1), rest_directions.get(segment - 1), rest_directions.get(segment))
                            else {
                                continue;
                            };
                            let turned = Quat::from_rotation_arc(rest_before, (a - before).normalize_or(rest_before));
                            turned * rest_here
                        }
                    };
                    if settings.stiffness > 0.0 && direction != Vec3::ZERO {
                        let alpha = length.max(1e-6) / (settings.stiffness * BEND_SCALE) / (dt * dt);
                        let target = a + direction * length;
                        let (wa, wb) = (weight(segment), weight(segment + 1));
                        if let Some(lambda) = bend.get_mut(segment) {
                            let change = (-(b - target) - *lambda * alpha) / (wa + wb + alpha);
                            *lambda += change;
                            if let Some(point) = strand.get_mut(segment + 1) {
                                *point += change * wb;
                            }
                            if let Some(point) = strand.get_mut(segment) {
                                *point -= change * wa;
                            }
                        }
                    }
                    // Stretch: the segment keeps its length.
                    let (Some(&a), Some(&b)) = (strand.get(segment), strand.get(segment + 1)) else { continue };
                    let delta = b - a;
                    let distance = delta.length();
                    if distance < 1e-9 {
                        continue;
                    }
                    let gradient = delta / distance;
                    let (wa, wb) = (weight(segment), weight(segment + 1));
                    if let Some(lambda) = stretch.get_mut(segment) {
                        let change = (-(distance - length) - alpha_stretch * *lambda) / (wa + wb + alpha_stretch);
                        *lambda += change;
                        if let Some(point) = strand.get_mut(segment + 1) {
                            *point += gradient * (change * wb);
                        }
                        if let Some(point) = strand.get_mut(segment) {
                            *point -= gradient * (change * wa);
                        }
                    }
                }
                if let Some(alpha) = alpha_pin {
                    for (point, lambda) in pin.iter_mut().enumerate().skip(1) {
                        let (Some(position), Some(goal)) = (strand.get(point).copied(), rest.get(point)) else { continue };
                        let change = (-(position - *goal) - *lambda * alpha) / (inverse_mass + alpha);
                        *lambda += change;
                        if let Some(slot) = strand.get_mut(point) {
                            *slot += change * inverse_mass;
                        }
                    }
                }
            }
            if let Some(distance) = settings.collision_distance {
                for (point, position) in strand.iter_mut().enumerate().skip(1) {
                    let before = previous.get(point).copied().unwrap_or(*position);
                    for collider in colliders {
                        if let Some((corrected, normal)) = collider.contact(*position, before, distance) {
                            *position = collider.with_friction(corrected, before, normal);
                        }
                    }
                }
            }
            for ((velocity, position), before) in velocities.iter_mut().zip(strand.iter()).zip(&previous) {
                *velocity = (*position - *before) / dt;
            }
            // Bending damping: each segment's tip slows towards its base's motion.
            for segment in 0..segments {
                let base = velocities.get(segment).copied().unwrap_or(Vec3::ZERO);
                if let Some(tip) = velocities.get_mut(segment + 1) {
                    *tip = base + (*tip - base) / (1.0 + bend_damping);
                }
            }
            if let Some(root) = velocities.first_mut() {
                *root = Vec3::ZERO;
            }
            if strand.iter().any(|point| !point.is_finite()) {
                *strand = rest.clone();
                velocities.iter_mut().for_each(|velocity| *velocity = Vec3::ZERO);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> HairSettings {
        HairSettings {
            mass: 0.3,
            stiffness: 0.5,
            damping: 0.5,
            air_damping: 1.0,
            pin_stiffness: 0.0,
            collision_distance: Some(0.015),
            gravity: Vec3::new(0.0, 0.0, -9.81),
        }
    }

    fn strand(direction: Vec3, length: f32, segments: usize) -> Vec<Vec3> {
        (0..=segments).map(|step| direction * (length * step as f32 / segments as f32)).collect()
    }

    fn run(sim: &mut HairSim, rest: &[Vec<Vec3>], frames: usize, colliders: &[ColliderSurface]) {
        for _ in 0..frames * 20 {
            sim.step(FRAME_TIME / 20.0, 4, rest, colliders, &[]);
        }
    }

    #[test]
    fn upright_hair_stands_and_keeps_its_length() {
        let rest = vec![strand(Vec3::Z, 1.0, 4)];
        let mut sim = HairSim::new(rest.clone(), settings());
        run(&mut sim, &rest, 48, &[]);
        let tip = sim.strands[0].last().unwrap();
        assert!((tip.z - 1.0).abs() < 2e-3 && tip.x.abs() < 1e-6, "{tip}");
        assert_eq!(sim.strands[0][0], Vec3::ZERO, "the root stays");
    }

    #[test]
    fn stiffer_hair_droops_less_and_shape_does_not_depend_on_length() {
        let droop = |stiffness: f32, length: f32| {
            let rest = vec![strand(Vec3::NEG_Y, length, 4)];
            let mut sim = HairSim::new(rest.clone(), HairSettings { stiffness, damping: 5.0, ..settings() });
            run(&mut sim, &rest, 240, &[]);
            sim.strands[0].last().unwrap().z / length
        };
        let (soft, medium, stiff) = (droop(0.05, 1.0), droop(0.5, 1.0), droop(5.0, 1.0));
        assert!(soft < medium && medium < stiff && stiff < 0.0, "{soft} {medium} {stiff}");
        assert!((droop(0.5, 2.0) - medium).abs() < 0.02, "the same shape at twice the length");
    }

    #[test]
    fn hair_lies_on_a_collision_object() {
        let floor = ColliderSurface::new(
            vec![
                [Vec3::new(-5.0, -5.0, -0.3), Vec3::new(5.0, -5.0, -0.3), Vec3::new(5.0, 5.0, -0.3)],
                [Vec3::new(-5.0, -5.0, -0.3), Vec3::new(5.0, 5.0, -0.3), Vec3::new(-5.0, 5.0, -0.3)],
            ],
            0.02,
            5.0,
        );
        let rest = vec![strand(Vec3::NEG_Y, 1.0, 4)];
        let mut sim = HairSim::new(rest.clone(), HairSettings { stiffness: 0.05, ..settings() });
        run(&mut sim, &rest, 96, std::slice::from_ref(&floor));
        let lowest = sim.strands[0].iter().map(|point| point.z).fold(f32::INFINITY, f32::min);
        assert!(lowest > -0.3 + 0.03 && lowest < -0.3 + 0.04, "{lowest}");
    }
}
