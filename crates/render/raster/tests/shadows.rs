//! Shadows and ambient occlusion in final renders, and shadows in the material viewport.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::collections::BTreeSet;

use kition_scene_model::glam::Vec3;
use kition_scene_model::{Light, LightKind, ObjectData, Primitive, Scene};
use kition_scene_raster::view::camera_view;
use kition_scene_raster::{Canvas, Overlays, RenderOptions, Shading, ViewState, render_camera_with, render_viewport};

fn save(name: &str, canvas: &Canvas) {
    // KITION_SCENE_SNAPSHOTS=<dir> writes the pictures for a person to look at.
    let Some(dir) = std::env::var_os("KITION_SCENE_SNAPSHOTS") else { return };
    let file = std::fs::File::create(std::path::Path::new(&dir).join(format!("{name}.png"))).unwrap();
    let mut encoder = png::Encoder::new(file, canvas.width, canvas.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.write_header().unwrap().write_image_data(&canvas.to_rgba8()).unwrap();
}

/// The starter cube resting on a 12 m floor, lit by a sun from the upper left, seen from the
/// starter camera; and the floor point straight down-sun of the cube.
fn cube_on_a_floor() -> (Scene, Vec3) {
    let mut scene = Scene::starter();
    let cube = scene.object_named("Cube").unwrap().id;
    scene.object_mut(cube).unwrap().location = Vec3::new(0.0, 0.0, 1.0);
    let floor = scene.add_object("Floor", ObjectData::Mesh(Primitive::Plane { size: 12.0 }.build().unwrap()));
    let _ = floor;
    let light = scene.object_named("Light").unwrap().id;
    scene.remove_objects(&[light]).unwrap();
    let sun = scene.add_object("Sun", ObjectData::Light(Light { power: 1.0, ..Light::new(LightKind::Sun) }));
    // Tilted -45° about X: it shines down and towards -Y, towards the camera's side.
    scene.object_mut(sun).unwrap().rotation = Vec3::new(-45f32.to_radians(), 0.0, 0.0);
    scene.render.width = 480;
    scene.render.height = 270;
    scene.world.strength = 1.0;
    // The sun shines along (0, -0.707, -0.707): the cube's shadow falls on -Y of it.
    (scene, Vec3::new(0.0, -2.5, 0.0))
}

fn brightness(canvas: &Canvas, x: u32, y: u32) -> f32 {
    let pixel = canvas.pixel(x, y).unwrap();
    pixel[0] + pixel[1] + pixel[2]
}

fn screen(scene: &Scene, point: Vec3) -> (u32, u32) {
    let projector =
        camera_view(scene, scene.camera.unwrap()).unwrap().fit(scene.render.width, scene.render.height, scene.render.width, scene.render.height);
    let at = projector.to_screen(projector.view_projection() * point.extend(1.0));
    (at.x as u32, at.y as u32)
}

#[test]
fn a_sun_casts_the_cube_s_shadow_on_the_floor() {
    let (scene, behind) = cube_on_a_floor();
    let lit_floor = Vec3::new(3.5, 0.0, 0.0);
    let (bx, by) = screen(&scene, behind);
    let (lx, ly) = screen(&scene, lit_floor);
    let without = render_camera_with(&scene, &RenderOptions { shadows: false, ambient_occlusion: false, samples: 1 }).unwrap();
    let with = render_camera_with(&scene, &RenderOptions { shadows: true, ambient_occlusion: false, samples: 1 }).unwrap();
    save("render-no-shadows", &without);
    save("render-shadows", &with);
    // Without shadows the floor behind the cube is as bright as the open floor.
    assert!(
        (brightness(&without, bx, by) - brightness(&without, lx, ly)).abs() < 0.15,
        "{} {}",
        brightness(&without, bx, by),
        brightness(&without, lx, ly)
    );
    // With them it is much darker than the open floor, which stays lit.
    assert!(brightness(&with, bx, by) < brightness(&with, lx, ly) * 0.5, "shadowed {} lit {}", brightness(&with, bx, by), brightness(&with, lx, ly));
    assert!((brightness(&with, lx, ly) - brightness(&without, lx, ly)).abs() < 0.1, "the open floor does not darken");
}

#[test]
fn ambient_occlusion_darkens_where_the_cube_meets_the_floor() {
    let (mut scene, _) = cube_on_a_floor();
    // Only the world lights it: what the sky reaches is what shows.
    let sun = scene.object_named("Sun").unwrap().id;
    scene.remove_objects(&[sun]).unwrap();
    scene.world.color = Vec3::splat(0.6);
    let corner = Vec3::new(1.05, -1.05, 0.0);
    let open = Vec3::new(3.0, -2.0, 0.0);
    let (cx, cy) = screen(&scene, corner);
    let (ox, oy) = screen(&scene, open);
    let plain = render_camera_with(&scene, &RenderOptions { shadows: false, ambient_occlusion: false, samples: 1 }).unwrap();
    let occluded = render_camera_with(&scene, &RenderOptions { shadows: false, ambient_occlusion: true, samples: 1 }).unwrap();
    save("render-ao", &occluded);
    assert!(brightness(&occluded, cx, cy) < brightness(&plain, cx, cy) * 0.9, "the crease darkens");
    assert!(brightness(&occluded, ox, oy) > brightness(&plain, ox, oy) * 0.95, "the open floor keeps its light");
}

#[test]
fn the_material_viewport_shows_shadows() {
    let (scene, behind) = cube_on_a_floor();
    let view = ViewState { camera: scene.camera, ..ViewState::default() };
    let overlays = Overlays {
        grid: false,
        selected: BTreeSet::new(),
        active: None,
        edit: None,
        bones: None,
        weights: None,
        curve: None,
        caret: None,
        peers: Vec::new(),
        sculpt: None,
        hide_extras: false,
    };
    let material = render_viewport(&scene, &view, 480, 270, Shading::Material, &overlays);
    save("viewport-material-shadows", &material);
    let (bx, by) = screen(&scene, behind);
    let (lx, ly) = screen(&scene, Vec3::new(3.5, 0.0, 0.0));
    assert!(brightness(&material, bx, by) < brightness(&material, lx, ly) * 0.6);
}
