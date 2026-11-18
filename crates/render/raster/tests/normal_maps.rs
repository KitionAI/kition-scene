//! Normal maps: a tangent-space map bends a plane's shading the way the map says, along the
//! texture's U direction, and a flat map changes nothing.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use kition_scene_model::glam::Vec3;
use kition_scene_model::{Camera, Image, Light, LightKind, ObjectData, Primitive, Scene};
use kition_scene_raster::{RenderOptions, render_camera_with};

fn png(rgb: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, 2, 2);
    encoder.set_color(png::ColorType::Rgb);
    encoder.write_header().unwrap().write_image_data(&rgb.repeat(4)).unwrap();
    out
}

/// A plane seen from straight above, lit by a sun slanting down from +X, with an optional
/// normal map of one colour. Answers the brightness at the centre.
fn brightness(map: Option<[u8; 3]>, flip_u: bool) -> f32 {
    let mut scene = Scene::empty();
    let mut mesh = Primitive::Plane { size: 2.0 }.build().unwrap();
    if flip_u {
        for uv in mesh.uvs.iter_mut().flatten() {
            uv.x = 1.0 - uv.x;
        }
    }
    let plane = scene.add_object("Plane", ObjectData::Mesh(mesh));
    let material = scene.add_material("Bumpy");
    if let Some(rgb) = map {
        let hash = scene.add_image(Image::new("normal.png", png(rgb)).unwrap());
        scene.material_mut(material).unwrap().normal_texture = Some(hash);
    }
    scene.object_mut(plane).unwrap().material = Some(material);
    let sun = scene.add_object("Sun", ObjectData::Light(Light { power: 3.0, ..Light::new(LightKind::Sun) }));
    // The sun's -Z turned 60° about Y towards -X: light arrives from +X, high up.
    scene.object_mut(sun).unwrap().rotation = Vec3::new(0.0, 60f32.to_radians(), 0.0);
    let camera = scene.add_object("Camera", ObjectData::Camera(Camera { orthographic: true, ortho_scale: 2.0, ..Camera::default() }));
    scene.object_mut(camera).unwrap().location = Vec3::new(0.0, 0.0, 5.0);
    scene.camera = Some(camera);
    scene.world.strength = 0.0;
    scene.render.width = 16;
    scene.render.height = 16;
    let canvas = render_camera_with(&scene, &RenderOptions { shadows: false, ambient_occlusion: false, samples: 1 }).unwrap();
    canvas.pixel(8, 8).unwrap()[0]
}

#[test]
fn the_map_bends_the_surface_along_u() {
    let plain = brightness(None, false);
    let flat = brightness(Some([128, 128, 255]), false);
    // Tilted towards +X (towards the sun) and towards -X.
    let towards = brightness(Some([218, 128, 218]), false);
    let away = brightness(Some([38, 128, 218]), false);
    assert!((plain - flat).abs() < 0.01 * plain.max(1e-3), "a flat map changes nothing: {plain} {flat}");
    assert!(towards > plain * 1.05, "towards the light is brighter: {towards} against {plain}");
    assert!(away < plain * 0.95, "away from it is darker: {away} against {plain}");
    // Mirrored U turns the tangent round: the same map now tilts away.
    let mirrored = brightness(Some([218, 128, 218]), true);
    assert!(mirrored < plain * 0.95, "mirrored coordinates tilt the other way: {mirrored}");
}
