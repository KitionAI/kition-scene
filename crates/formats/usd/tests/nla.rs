//! The NLA and F-curve modifiers go out baked: every frame of a layered scene written as
//! USD and read back moves the same.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_scene_model::fmodifier::{FModifier, FModifierKind};
use kition_scene_model::nla::BlendType;
use kition_scene_model::{Property, Scene};

/// The starter scene with layered animation: the cube's keys pushed down into a strip played
/// twice as slow from frame 5, a lift added on a track above at half influence, a turn of its
/// own repeating under a Cycles modifier; the light stepping along under a Stepped modifier.
fn layered() -> Scene {
    let mut scene = Scene::starter();
    let cube = scene.object_named("Cube").unwrap().id;
    let light = scene.object_named("Light").unwrap().id;
    scene.insert_key(cube, Property::Location, 1.0).unwrap();
    scene.object_mut(cube).unwrap().location.x = 4.0;
    scene.insert_key(cube, Property::Location, 11.0).unwrap();
    scene.nla_push_down(cube, Some("Slide")).unwrap();
    scene.object_mut(cube).unwrap().nla.tracks[0].strips[0].scale = 2.0;
    scene.object_mut(cube).unwrap().nla.tracks[0].strips[0].start = 5.0;
    scene.object_mut(cube).unwrap().location.z = 1.0;
    scene.insert_key(cube, Property::Location, 1.0).unwrap();
    scene.nla_push_down(cube, Some("Lift")).unwrap();
    scene.object_mut(cube).unwrap().nla.tracks[1].strips[0].blend = BlendType::Add;
    scene.object_mut(cube).unwrap().nla.tracks[1].strips[0].influence = Some(0.5);
    scene.insert_key(cube, Property::Rotation, 1.0).unwrap();
    scene.object_mut(cube).unwrap().rotation.z = 1.0;
    scene.insert_key(cube, Property::Rotation, 9.0).unwrap();
    for channel in scene.animation.channels.iter_mut().filter(|channel| channel.target == cube) {
        channel.modifiers.push(FModifier::new(FModifierKind::new("cycles").unwrap()));
    }
    scene.insert_key(light, Property::Location, 1.0).unwrap();
    scene.object_mut(light).unwrap().location.y += 3.0;
    scene.insert_key(light, Property::Location, 25.0).unwrap();
    for channel in scene.animation.channels.iter_mut().filter(|channel| channel.target == light) {
        channel.modifiers.push(FModifier::new(FModifierKind::Stepped { step: 4.0, offset: 0.0, start: None, end: None }));
    }
    scene.animation.timeline.end = 40;
    scene.validate().unwrap();
    scene.go_to_frame(1.0);
    scene
}

/// Every frame of the timeline matches between the layered scene and what a file brought back.
fn same_motion(scene: &Scene, back: &Scene) {
    for name in ["Cube", "Light"] {
        for frame in 1..=40 {
            let (a, b) = (scene.at_frame(frame as f32), back.at_frame(frame as f32));
            let (a, b) = (a.world_matrix(a.object_named(name).unwrap().id).unwrap(), b.world_matrix(b.object_named(name).unwrap().id).unwrap());
            assert!(a.abs_diff_eq(b, 1e-4), "{name} at frame {frame}:\n{a}\n{b}");
        }
    }
}

#[test]
fn nla_and_modifiers_go_out_baked() {
    let scene = layered();
    let bytes = kition_scene_usd::write(&scene, None).unwrap();
    let mut back = Scene::empty();
    kition_scene_exchange::place(&mut back, kition_scene_usd::read(&bytes).unwrap()).unwrap();
    back.validate().unwrap();
    same_motion(&scene, &back);
}
