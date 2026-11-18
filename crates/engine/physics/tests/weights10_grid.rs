//! Strong but valid weighted fields must not overflow self-collision's integer cell index.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use glam::Vec3;
use kition_scene_model::{
    ObjectData, Primitive, Scene,
    physics::{EffectorWeights, FieldKind, ForceField, ObjectPhysics, SoftBody},
};
#[test]
fn finite_extreme_motion_with_self_collision_returns_a_result_without_panicking() {
    let mut scene = Scene::empty();
    scene.physics.frame_end = 12;
    scene.physics.gravity = Vec3::ZERO;
    let id = scene.add_object("Soft", ObjectData::Mesh(Primitive::Plane { size: 1.0 }.build().unwrap()));
    scene.object_mut(id).unwrap().physics = Some(ObjectPhysics {
        soft_body: Some(SoftBody {
            goal: false,
            self_collision: true,
            effector_weights: EffectorWeights { all: 200.0, vortex: 200.0, ..Default::default() },
            ..Default::default()
        }),
        ..Default::default()
    });
    let field = scene.add_object("Vortex", ObjectData::Empty { size: 1.0 });
    scene.object_mut(field).unwrap().location = Vec3::new(3.0, 0.0, 0.0);
    scene.object_mut(field).unwrap().physics =
        Some(ObjectPhysics { field: Some(ForceField { strength: 10000.0, ..ForceField::new(FieldKind::Vortex) }), ..Default::default() });
    scene.validate().unwrap();
    match kition_scene_physics::bake(&scene) {
        Ok(baked) => assert!(baked.cloths[&id].iter().flatten().all(|point| point.is_finite())),
        Err(error) => assert!(error.starts_with("failed:"), "{error}"),
    }
}
