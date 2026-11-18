//! Image textures on surfaces: where texels land, and mip levels far away.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use kition_scene_model::glam::Vec3;
use kition_scene_model::{Camera, Image, ObjectData, Primitive, Scene};
use kition_scene_raster::{RenderOptions, render_camera_with};

fn png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.write_header().unwrap().write_image_data(rgba).unwrap();
    out
}

/// A plane under a camera looking straight down, its material showing `image`.
fn textured_plane(image: Vec<u8>) -> Scene {
    let mut scene = Scene::empty();
    let plane = scene.add_object("Plane", ObjectData::Mesh(Primitive::Plane { size: 2.0 }.build().unwrap()));
    let material = scene.add_material("Checker");
    let hash = scene.add_image(Image::new("checker.png", image).unwrap());
    scene.material_mut(material).unwrap().base_color = [1.0; 4];
    scene.material_mut(material).unwrap().base_color_texture = Some(hash);
    scene.object_mut(plane).unwrap().material = Some(material);
    let camera = scene.add_object("Camera", ObjectData::Camera(Camera { orthographic: true, ortho_scale: 2.0, ..Camera::default() }));
    scene.object_mut(camera).unwrap().location = Vec3::new(0.0, 0.0, 5.0);
    scene.camera = Some(camera);
    scene.render.width = 64;
    scene.render.height = 64;
    scene
}

#[test]
fn texels_land_where_the_coordinates_put_them() {
    // Top row red, green; bottom row blue, white. V runs up, so the top row is at +Y.
    let image = png(2, 2, &[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255]);
    let scene = textured_plane(image);
    let canvas = render_camera_with(&scene, &RenderOptions { shadows: false, ambient_occlusion: false, samples: 1 }).unwrap();
    let rgba = canvas.to_rgba8();
    let at = |x: usize, y: usize| [rgba[(y * 64 + x) * 4], rgba[(y * 64 + x) * 4 + 1], rgba[(y * 64 + x) * 4 + 2]];
    let strongest = |pixel: [u8; 3]| (0..3).max_by_key(|channel| pixel[*channel]).unwrap();
    assert_eq!(strongest(at(16, 16)), 0, "top left is red: {:?}", at(16, 16));
    assert_eq!(strongest(at(48, 16)), 1, "top right is green: {:?}", at(48, 16));
    assert_eq!(strongest(at(16, 48)), 2, "bottom left is blue: {:?}", at(16, 48));
    let white = at(48, 48);
    assert!(white[0] > 100 && white[0].abs_diff(white[1]) < 8 && white[1].abs_diff(white[2]) < 8, "bottom right is white: {white:?}");
}

#[test]
fn far_away_a_fine_checker_averages_to_grey() {
    // 256 × 256 one-pixel checker on a plane 64 pixels wide: the mip level must average it.
    let mut rgba = Vec::with_capacity(256 * 256 * 4);
    for y in 0..256 {
        for x in 0..256 {
            let value = if (x + y) % 2 == 0 { 255 } else { 0 };
            rgba.extend([value, value, value, 255]);
        }
    }
    let scene = textured_plane(png(256, 256, &rgba));
    let canvas = render_camera_with(&scene, &RenderOptions { shadows: false, ambient_occlusion: false, samples: 1 }).unwrap();
    let bytes = canvas.to_rgba8();
    let values: Vec<u8> = (20..44).flat_map(|y| (20..44).map(move |x| (y, x))).map(|(y, x)| bytes[(y * 64 + x) * 4]).collect();
    let (low, high) = (values.iter().min().unwrap(), values.iter().max().unwrap());
    assert!(high - low < 24, "no moiré: {low}..{high}");
}
