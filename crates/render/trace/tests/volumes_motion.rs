//! Volumes and motion blur on their own: what is known exactly.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::expect_used)]

use std::collections::BTreeMap;

use kition_scene_model::glam::Vec3;
use kition_scene_model::node_tree::{Link, Node, NodeGroup, Tree};
use kition_scene_model::{ObjectData, Primitive, Property, Scene};
use kition_scene_trace::{TraceOptions, Tracer};
use serde_json::json;

/// An orthographic camera looking along +Y at a white emitting wall 4 m away.
fn wall() -> Scene {
    let mut scene = Scene::empty();
    scene.world.color = Vec3::ZERO;
    let glow = scene.add_material("Glow");
    {
        let m = scene.material_mut(glow).unwrap();
        m.base_color = [0.0, 0.0, 0.0, 1.0];
        m.emission = Vec3::ONE;
        m.emission_strength = 1.0;
    }
    let wall = scene.add_object("Wall", ObjectData::Mesh(Primitive::Plane { size: 20.0 }.build().unwrap()));
    let object = scene.object_mut(wall).unwrap();
    object.location = Vec3::new(0.0, 4.0, 0.0);
    object.rotation = Vec3::new(std::f32::consts::FRAC_PI_2, 0.0, 0.0);
    object.material = Some(glow);
    let camera = scene.add_object("Camera", ObjectData::Camera(Default::default()));
    let object = scene.object_mut(camera).unwrap();
    object.location = Vec3::new(0.0, -6.0, 0.0);
    object.rotation = Vec3::new(std::f32::consts::FRAC_PI_2, 0.0, 0.0);
    if let ObjectData::Camera(camera) = &mut object.data {
        camera.orthographic = true;
        camera.ortho_scale = 4.0;
    }
    scene.camera = Some(camera);
    scene.render.width = 16;
    scene.render.height = 16;
    scene
}

/// A shader group with `kind` (its values) in the Material Output's Volume.
fn volume_group(scene: &mut Scene, kind: &str, values: serde_json::Value) -> u32 {
    let (id, output, node) = (scene.take_id(), scene.take_id(), scene.take_id());
    let values: BTreeMap<String, serde_json::Value> = values.as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    scene.node_groups.push(NodeGroup {
        id,
        name: kind.into(),
        tree: Tree::Shader,
        inputs: Vec::new(),
        outputs: Vec::new(),
        nodes: vec![
            Node { id: output, kind: "materialOutput".into(), location: [0.0, 0.0], values: BTreeMap::new() },
            Node { id: node, kind: kind.into(), location: [0.0, 0.0], values },
        ],
        links: vec![Link { from: node, from_socket: "Volume".into(), to: output, to_socket: "Volume".into() }],
    });
    id
}

fn render(scene: &Scene, samples: u32) -> Vec<Vec3> {
    let mut tracer = Tracer::new(scene, TraceOptions { samples, bounces: 4, clamp_indirect: 0.0, ..Default::default() }).unwrap();
    tracer.finish();
    tracer.linear()
}

#[test]
fn an_absorbing_volume_lets_through_e_to_the_minus_density_times_length() {
    let mut scene = wall();
    // Volume Absorption: (1 − colour) × density per metre, through a 2 m cube.
    let group = volume_group(&mut scene, "volumeAbsorption", json!({ "Color": [0.5, 0.2, 0.8, 1.0], "Density": 0.5 }));
    let ink = scene.add_material("Ink");
    scene.material_mut(ink).unwrap().shader = Some(group);
    let cube = scene.add_object("Ink", ObjectData::Mesh(Primitive::Cube { size: 2.0 }.build().unwrap()));
    scene.object_mut(cube).unwrap().material = Some(ink);
    scene.validate().unwrap();
    let picture = render(&scene, 4);
    let centre = picture[8 * 16 + 8];
    let expected = Vec3::new((-0.5f32).exp(), (-0.8f32).exp(), (-0.2f32).exp());
    assert!((centre - expected).abs().max_element() < 1e-4, "{centre} against {expected}");
    // Outside the cube the wall shows as it is: the volume has no surface.
    assert!((picture[8 * 16].x - 1.0).abs() < 1e-4, "{}", picture[8 * 16]);
}

#[test]
fn a_world_volume_hides_the_background_and_dims_what_is_behind_it() {
    let mut scene = wall();
    let group = volume_group(&mut scene, "volumeAbsorption", json!({ "Color": [0.0, 0.0, 0.0, 1.0], "Density": 0.1 }));
    scene.world.volume = Some(group);
    scene.world.color = Vec3::ONE;
    scene.validate().unwrap();
    let picture = render(&scene, 2);
    // Rays start at the camera's clip start, 9.9 m from the wall: e^(−0.1 × 9.9).
    let centre = picture[8 * 16 + 8];
    assert!((centre.x - (-0.99f32).exp()).abs() < 1e-4, "{centre}");
}

#[test]
fn a_scattering_volume_glows_with_its_emission_and_scatters_light() {
    let mut scene = wall();
    scene.objects.retain(|object| object.name != "Wall");
    // Principled Volume sending out 2 per metre over a 2 m cube, seen against black: it adds
    // 2 × (1 − e^(−σt·2)) / σt where σt is its density (it absorbs or scatters all of it).
    let group = volume_group(&mut scene, "principledVolume", json!({ "Density": 0.5, "Emission Strength": 2.0 }));
    let glow = scene.add_material("Glow");
    scene.material_mut(glow).unwrap().shader = Some(group);
    let cube = scene.add_object("Glow", ObjectData::Mesh(Primitive::Cube { size: 2.0 }.build().unwrap()));
    scene.object_mut(cube).unwrap().material = Some(glow);
    scene.validate().unwrap();
    let picture = render(&scene, 4);
    let expected = 2.0 * (1.0 - (-1.0f32).exp()) / 0.5;
    assert!((picture[8 * 16 + 8].x - expected).abs() < 1e-3, "{} against {expected}", picture[8 * 16 + 8]);
}

#[test]
fn motion_blur_leaves_still_scenes_alone_and_smears_what_moves() {
    let mut scene = Scene::starter();
    scene.render.width = 48;
    scene.render.height = 27;
    let still = render(&scene, 4);
    scene.render.motion_blur.enabled = true;
    assert_eq!(render(&scene, 4), still, "nothing moves: the same picture, sample for sample");
    // The cube slides 2 m along X between frames 1 and 3; rendered at frame 2 with a shutter of a
    // whole frame it covers a wider stretch, less solidly.
    let cube = scene.object_named("Cube").unwrap().id;
    scene.object_mut(cube).unwrap().location = Vec3::new(-1.0, 0.0, 0.0);
    scene.insert_key(cube, Property::Location, 1.0).unwrap();
    scene.object_mut(cube).unwrap().location = Vec3::new(1.0, 0.0, 0.0);
    scene.insert_key(cube, Property::Location, 3.0).unwrap();
    scene.go_to_frame(2.0);
    scene.render.motion_blur.shutter = 1.0;
    let blurred = render(&scene, 16);
    scene.render.motion_blur.enabled = false;
    let sharp = render(&scene, 16);
    let differs = blurred.iter().zip(&sharp).filter(|(a, b)| (**a - **b).abs().max_element() > 0.05).count();
    assert!(differs > 20, "{differs} pixels changed");
}
