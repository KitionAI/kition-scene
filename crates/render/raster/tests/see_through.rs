//! Blended and clipped alpha, glass, and the world image in the fast renderer.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use kition_scene_model::glam::Vec3;
use kition_scene_model::hdr;
use kition_scene_model::{AlphaMode, Image, ObjectData, Primitive, Scene};
use kition_scene_raster::{RenderOptions, render_camera_with};

/// The starter scene at a small size with its cube's material set by `edit`, and a red plane
/// behind the cube as seen from the camera.
fn scene(edit: impl FnOnce(&mut kition_scene_model::Material)) -> Scene {
    let mut scene = Scene::starter();
    scene.render.width = 160;
    scene.render.height = 90;
    let cube = scene.object_named("Cube").unwrap().id;
    let material = scene.object(cube).unwrap().material.unwrap();
    edit(scene.material_mut(material).unwrap());
    let wall = scene.add_object("Wall", ObjectData::Mesh(Primitive::Plane { size: 30.0 }.build().unwrap()));
    let red = scene.add_material("Red");
    scene.material_mut(red).unwrap().base_color = [0.8, 0.02, 0.02, 1.0];
    let object = scene.object_mut(wall).unwrap();
    object.material = Some(red);
    // Behind the cube, facing the camera (which looks from +X −Y +Z towards the origin).
    object.location = Vec3::new(-6.0, 6.0, 0.0);
    object.rotation = Vec3::new(std::f32::consts::FRAC_PI_2, 0.0, 0.814_93);
    scene
}

fn centre(scene: &Scene) -> [f32; 4] {
    let canvas = render_camera_with(scene, &RenderOptions { samples: 1, shadows: false, ambient_occlusion: false }).unwrap();
    canvas.pixel(80, 45).unwrap()
}

#[test]
fn blended_alpha_shows_what_is_behind_and_clipping_removes_it() {
    let opaque = centre(&scene(|_| {}));
    let blended = centre(&scene(|material| {
        material.base_color[3] = 0.3;
        material.alpha_mode = AlphaMode::Blend;
    }));
    let clipped = centre(&scene(|material| {
        material.base_color[3] = 0.3;
        material.alpha_mode = AlphaMode::Clip;
    }));
    let kept = centre(&scene(|material| {
        material.base_color[3] = 0.7;
        material.alpha_mode = AlphaMode::Clip;
    }));
    assert!((opaque[0] - opaque[1]).abs() < 0.05, "the grey cube: {opaque:?}");
    assert!(blended[0] > blended[1] + 0.05, "red shows through: {blended:?}");
    assert!(clipped[0] > clipped[1] * 4.0, "the wall alone: {clipped:?}");
    assert_eq!(kept, opaque);
}

#[test]
fn glass_lets_the_wall_through() {
    let glass = centre(&scene(|material| material.transmission = 1.0));
    assert!(glass[0] > glass[1] + 0.05, "{glass:?}");
}

#[test]
fn a_world_image_is_the_background_and_lights_the_scene() {
    // A sky that is blue above the horizon and dark below.
    let (width, height) = (64, 32);
    let pixels: Vec<[f32; 3]> =
        (0..width * height).map(|index| if index / width < height / 2 { [0.2, 0.4, 1.0] } else { [0.02, 0.02, 0.02] }).collect();
    let mut scene = Scene::starter();
    scene.render.width = 160;
    scene.render.height = 90;
    let light = scene.object_named("Light").unwrap().id;
    scene.remove_objects(&[light]).unwrap();
    let hash = scene.add_image(Image::new("Sky.hdr", hdr::encode(width, height, &pixels)).unwrap());
    scene.world.image = Some(hash);
    let canvas = render_camera_with(&scene, &RenderOptions { samples: 1, shadows: false, ambient_occlusion: false }).unwrap();
    // The cube's top faces the blue sky; the picture's corner looks past the cube at the world
    // (the camera looks down, so it sees blue sky or the dark ground there).
    let top = canvas.pixel(80, 30).unwrap();
    assert!(top[2] > top[0] * 1.5, "the cube's top is lit blue: {top:?}");
    let corner = canvas.pixel(2, 2).unwrap();
    assert!(corner[2] > 0.3 || corner[0] < 0.05, "the background is the world: {corner:?}");
}
