//! Coupled replay has explicit work limits; dense neighborhoods and remote targets stay finite.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use glam::Vec3;
use kition_scene_particles::{
    Boids, EmitFrom, Field, Kind, Particle, ParticlePhysics, ParticleSystem, Surface,
    boids::{self, Rule, RuleKind},
    sim,
};
use std::collections::BTreeMap;

#[test]
fn the_default_emitter_fits_the_event_budget_but_long_overlapping_emissions_do_not() {
    let mut system = ParticleSystem::new("Birds", Kind::Emitter);
    system.physics = ParticlePhysics::Boids;
    assert!(system.check().is_ok());
    system.lifetime = 250.0;
    let error = system.check().unwrap_err();
    assert!(error.contains("birth/death events"), "{error}");
    system.frame_end = system.frame_start;
    assert!(system.check().is_ok());
}

#[test]
fn a_dense_maximum_count_default_flock_moves_and_repeats_with_finite_instance_rotations() {
    let mut system = ParticleSystem::new("Birds", Kind::Emitter);
    system.physics = ParticlePhysics::Boids;
    system.count = 2048;
    system.frame_end = 1.0;
    system.lifetime = 100.0;
    system.emit_from = EmitFrom::Verts;
    system.rotation_random = 1.0;
    system.boids.air_speed_max = 100.0;
    system.boids.air_acc_max = 1.0;
    system.boids.air_angular_max = 1.0;
    system.boids.air_personal_space = 10.0;
    system.timestep = 0.1;
    system.size = 10.0;
    let field = Field {
        surface: Surface { positions: (0..2048).map(|index| Vec3::new(index as f32 * 0.00002, 0.0, 0.0)).collect(), ..Default::default() },
        ..Default::default()
    };
    assert!(system.check().is_ok());
    let born = sim::particles_at(&system, &field, 1.0);
    let moved = sim::particles_at(&system, &field, 3.0);
    assert_eq!(moved.len(), 2048);
    assert_eq!(moved, sim::particles_at(&system, &field, 3.0));
    assert!(moved.iter().all(|p| p.position.is_finite() && p.velocity.is_finite() && p.rotation.is_finite()));
    assert!(moved.iter().zip(&born).any(|(p, initial)| p.position != initial.position));
    assert!(moved.iter().all(|p| p.velocity.length() <= 100.00001));
    assert_ne!(moved[0].rotation, moved[1].rotation);
    assert_eq!(born[0].rotation, moved[0].rotation);
}

#[test]
fn finite_targets_and_particles_need_no_cross_register_coordinate_limit() {
    let settings =
        Boids { rules: vec![Rule { kind: RuleKind::Goal, target: Some(1) }, Rule { kind: RuleKind::Flock, target: None }], ..Default::default() };
    let far = f32::MAX * 0.25;
    let initial = vec![
        Particle { index: 0, position: Vec3::splat(-far), velocity: Vec3::ZERO, age: 0.0, size: 0.05, rotation: Vec3::ZERO },
        Particle { index: 1, position: Vec3::splat(far), velocity: Vec3::ZERO, age: 0.0, size: 0.05, rotation: Vec3::ZERO },
    ];
    let targets = BTreeMap::from([(1, Vec3::splat(far))]);
    let after = boids::step(&settings, &targets, &initial, 0.04);
    assert_eq!(after.len(), 2);
    assert!(after.iter().all(|p| p.position.is_finite() && p.velocity.is_finite()));
    assert!(after.iter().any(|p| p.velocity.length() > 0.0));
}
