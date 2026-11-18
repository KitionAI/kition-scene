//! Baked motion leaves through the exporters: glTF and FBX sample the simulated bodies.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::sync::Arc;

use kition_scene_exchange::{Format, export};
use kition_scene_model::glam::Vec3;
use kition_scene_model::physics::{BodyKind, ObjectPhysics, RigidBody};
use kition_scene_model::{ObjectData, Primitive, Scene};
use serde_json::Value;

fn dropped() -> Scene {
    let mut scene = Scene::empty();
    scene.physics.frame_end = 48;
    for (name, primitive, z, kind) in
        [("Ground", Primitive::Plane { size: 10.0 }, 0.0, BodyKind::Passive), ("Cube", Primitive::Cube { size: 1.0 }, 3.0, BodyKind::Active)]
    {
        let id = scene.add_object(name, ObjectData::Mesh(primitive.build().unwrap()));
        let object = scene.object_mut(id).unwrap();
        object.location = Vec3::new(0.0, 0.0, z);
        object.physics = Some(ObjectPhysics { rigid_body: Some(RigidBody { kind, ..RigidBody::default() }), ..ObjectPhysics::default() });
    }
    let baked = kition_scene_physics::bake(&scene).unwrap();
    scene.physics_cache.baked = Some(Arc::new(baked));
    assert!(scene.refresh_physics());
    scene.go_to_frame(1.0);
    scene
}

#[test]
fn gltf_carries_the_fall_as_animation_samples() {
    let scene = dropped();
    let bytes = export(&scene, Format::Gltf, None).unwrap();
    let gltf: Value = serde_json::from_slice(&bytes).unwrap();
    let animation = &gltf["animations"][0];
    let channels = animation["channels"].as_array().unwrap();
    assert!(channels.iter().any(|channel| channel["target"]["path"] == "translation"), "the cube's fall: {animation}");
    let sampler = &animation["samplers"][0];
    let input = &gltf["accessors"][sampler["input"].as_u64().unwrap() as usize];
    assert_eq!(input["count"], 48, "one sample per baked frame");
}

#[test]
fn fbx_carries_the_fall_as_curves() {
    let scene = dropped();
    let bytes = kition_scene_fbx::write(&scene, None).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("AnimationCurveNode"), "an animation stack with curve nodes");
    let mut unbaked = scene.clone();
    unbaked.physics_cache = Default::default();
    let bytes = kition_scene_fbx::write(&unbaked, None).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("AnimationCurveNode"), "without the bake nothing moves");
}
