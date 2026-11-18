//! Glass, alpha, depth of field, and the world image in the path tracer.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use kition_scene_model::glam::Vec3;
use kition_scene_model::hdr;
use kition_scene_model::{AlphaMode, Image, ObjectData, Primitive, Scene};
use kition_scene_trace::{TraceOptions, render};

fn small(mut scene: Scene) -> Scene {
    scene.render.width = 64;
    scene.render.height = 36;
    scene
}

fn options(samples: u32) -> TraceOptions {
    TraceOptions { samples, bounces: 6, denoise: false, seed: 1, clamp_indirect: 0.0, volume_bounces: 0, passes: false }
}

fn pixel(image: &[u8], width: u32, x: u32, y: u32) -> [u8; 3] {
    let at = ((y * width + x) * 4) as usize;
    [image[at], image[at + 1], image[at + 2]]
}

/// The starter scene with the cube's material changed and a red wall behind it.
fn with_wall(edit: impl FnOnce(&mut kition_scene_model::Material)) -> Scene {
    let mut scene = small(Scene::starter());
    let cube = scene.object_named("Cube").unwrap().id;
    let material = scene.object(cube).unwrap().material.unwrap();
    edit(scene.material_mut(material).unwrap());
    let wall = scene.add_object("Wall", ObjectData::Mesh(Primitive::Plane { size: 30.0 }.build().unwrap()));
    let red = scene.add_material("Red");
    scene.material_mut(red).unwrap().base_color = [0.8, 0.02, 0.02, 1.0];
    scene.material_mut(red).unwrap().emission = Vec3::new(1.0, 0.0, 0.0);
    scene.material_mut(red).unwrap().emission_strength = 1.0;
    let object = scene.object_mut(wall).unwrap();
    object.material = Some(red);
    object.location = Vec3::new(-6.0, 6.0, 0.0);
    object.rotation = Vec3::new(std::f32::consts::FRAC_PI_2, 0.0, 0.814_93);
    scene
}

#[test]
fn glass_and_alpha_show_the_wall_through_the_cube() {
    let (opaque, width, _) = render(&with_wall(|_| {}), options(16)).unwrap();
    let (glass, ..) = render(
        &with_wall(|material| {
            material.transmission = 1.0;
            material.roughness = 0.0;
        }),
        options(16),
    )
    .unwrap();
    let (clipped, ..) = render(
        &with_wall(|material| {
            material.base_color[3] = 0.2;
            material.alpha_mode = AlphaMode::Clip;
        }),
        options(16),
    )
    .unwrap();
    let [r, g, _] = pixel(&opaque, width, 32, 18);
    assert!(r.abs_diff(g) < 20, "a grey cube: {r} {g}");
    let [r, g, _] = pixel(&glass, width, 32, 18);
    assert!(r > g + 30, "the red wall through the glass: {r} {g}");
    let [r, g, _] = pixel(&clipped, width, 32, 18);
    assert!(r > g + 60, "the wall alone where the cube is clipped away: {r} {g}");
}

#[test]
fn depth_of_field_keeps_the_focused_object_sharp_and_blurs_the_rest() {
    let sharp = small(Scene::starter());
    let focused = |focus_on_cube: bool| {
        let mut scene = sharp.clone();
        let cube = scene.object_named("Cube").unwrap().id;
        let camera = scene.camera.unwrap();
        if let ObjectData::Camera(camera) = &mut scene.object_mut(camera).unwrap().data {
            camera.dof.enabled = true;
            camera.dof.f_stop = 0.2;
            camera.dof.focus_distance = 2.0;
            camera.dof.focus_object = focus_on_cube.then_some(cube);
        }
        scene
    };
    let (pinhole, ..) = render(&sharp, options(64)).unwrap();
    let difference = |image: &[u8]| -> f64 { image.iter().zip(&pinhole).map(|(a, b)| f64::from(a.abs_diff(*b))).sum::<f64>() / image.len() as f64 };
    let (on_cube, ..) = render(&focused(true), options(64)).unwrap();
    let (near, ..) = render(&focused(false), options(64)).unwrap();
    let (in_focus, out_of_focus) = (difference(&on_cube), difference(&near));
    assert!(out_of_focus > in_focus * 3.0, "focused on the cube {in_focus:.2}, focused 2 m away {out_of_focus:.2}");
}

#[test]
fn a_world_image_lights_the_scene() {
    let (width, height) = (32, 16);
    let pixels: Vec<[f32; 3]> = (0..width * height).map(|index| if index / width < height / 2 { [0.1, 0.3, 1.0] } else { [0.0, 0.0, 0.0] }).collect();
    let mut scene = small(Scene::starter());
    let light = scene.object_named("Light").unwrap().id;
    scene.remove_objects(&[light]).unwrap();
    let hash = scene.add_image(Image::new("Sky.hdr", hdr::encode(width, height, &pixels)).unwrap());
    scene.world.image = Some(hash);
    let (image, w, _) = render(&scene, options(32)).unwrap();
    let [r, _, b] = pixel(&image, w, 32, 12);
    assert!(b > r + 20, "the cube's top is lit by the blue sky: {r} {b}");
}
