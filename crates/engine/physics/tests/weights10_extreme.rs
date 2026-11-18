#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use glam::Vec3;
use kition_scene_model::physics::{Cloth, EffectorWeights, FieldKind, ForceField, ObjectPhysics};
use kition_scene_model::{ObjectData, Primitive, Scene};

#[test]
fn an_unstable_weighted_cloth_bake_returns_an_error_instead_of_nonfinite_vertices() {
    let mut scene = Scene::empty();
    scene.physics.frame_end = 12;
    scene.physics.gravity = Vec3::ZERO;
    let cloth = scene.add_object("Cloth", ObjectData::Mesh(Primitive::Plane { size: 1.0 }.build().unwrap()));
    scene.object_mut(cloth).unwrap().physics = Some(ObjectPhysics {
        cloth: Some(Cloth { effector_weights: EffectorWeights { all: 200.0, vortex: 200.0, ..Default::default() }, ..Default::default() }),
        ..Default::default()
    });
    let field = scene.add_object("Vortex", ObjectData::Empty { size: 1.0 });
    scene.object_mut(field).unwrap().location = Vec3::new(3.0, 0.0, 0.0);
    scene.object_mut(field).unwrap().physics =
        Some(ObjectPhysics { field: Some(ForceField { strength: 10000.0, ..ForceField::new(FieldKind::Vortex) }), ..Default::default() });
    scene.validate().unwrap();
    let result = kition_scene_physics::bake(&scene);
    assert!(result.is_err(), "an unstable weighted simulation must not publish a broken bake");
}
