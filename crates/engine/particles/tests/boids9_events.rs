//! Fractional birth/death events must contribute only over their actual live durations.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
use glam::Vec3;
use kition_scene_particles::{
    EmitFrom, Field, Kind, ParticlePhysics, ParticleSystem, Surface,
    boids::{Rule, RuleKind},
    sim,
};
use std::collections::BTreeMap;
fn system() -> (ParticleSystem, Field) {
    let mut s = ParticleSystem::new("Birds", Kind::Emitter);
    s.physics = ParticlePhysics::Boids;
    s.count = 2;
    s.frame_start = 1.0;
    s.frame_end = 2.0;
    s.lifetime = 1.5;
    s.emit_from = EmitFrom::Verts;
    s.boids.rules = vec![Rule { kind: RuleKind::Goal, target: Some(9) }];
    let f = Field {
        surface: Surface { positions: vec![Vec3::ZERO], normals: vec![Vec3::Z], ..Default::default() },
        boid_targets: BTreeMap::from([(9, Vec3::new(0.0, 0.0, 100.0))]),
        ..Default::default()
    };
    (s, f)
}
#[test]
fn fractional_births_move_for_their_actual_age_before_the_next_whole_frame() {
    let (s, f) = system();
    let particles = sim::particles_at(&s, &f, 2.0);
    assert_eq!(particles.len(), 2);
    let second = particles.iter().find(|p| p.index == 1).unwrap();
    assert_eq!(second.age, 0.5);
    assert!((second.velocity.z - 0.1).abs() < 1e-6);
    assert!((second.position.z - 0.001).abs() < 1e-6);
}
#[test]
fn a_death_event_removes_the_neighbor_before_the_rest_of_the_step() {
    let (mut s, mut f) = system();
    s.boids.rules = vec![Rule { kind: RuleKind::Flock, target: None }];
    s.lifetime = 0.75;
    f.surface.positions = vec![Vec3::new(-1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)];
    f.surface.normals = vec![Vec3::Z; 2];
    let alive = sim::particles_at(&s, &f, 1.7);
    assert_eq!(alive.len(), 2);
    assert!(alive[1].velocity.length() > 0.0);
    let after = sim::particles_at(&s, &f, 1.9);
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].index, 1);
    let at_death = sim::particles_at(&s, &f, 1.75);
    assert_eq!(at_death.len(), 1);
    let expected = at_death[0].velocity * (1.0 - (1.9 - 1.75) * s.timestep);
    assert!(after[0].velocity.distance(expected) < 1e-6, "the departed boid no longer steers its neighbor");
}
