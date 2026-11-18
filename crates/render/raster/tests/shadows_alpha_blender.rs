//! Transparent shadows in fast renders against the reference's fast renderer (EEVEE 5.2.3):
//! blended alpha lets (1 − alpha) of a sun's light through each surface it crosses, and clipped
//! alpha leaves the image's holes in the shadow. `fixtures/shadows-alpha.EEVEE.json` holds the
//! reference's shares of the open floor's light in each shadow (`blender/shadows_alpha.py`
//! writes it; with `KITION_BLENDER` naming a Blender executable the script runs again and must
//! give the same).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_scene_model::glam::Vec3;
use kition_scene_model::{AlphaMode, Image, Light, LightKind, ObjectData, Primitive, Scene};
use kition_scene_raster::cache::MeshCache;
use kition_scene_raster::{RenderOptions, render_camera_passes};
use serde_json::Value;

const REFERENCE: &str = include_str!("fixtures/shadows-alpha.EEVEE.json");
const WIDTH: u32 = 320;
const HEIGHT: u32 = 160;

/// Name, primitive (plane or cube), alpha, and x of each caster, 2 m up at y = −2.
const CASTERS: [(&str, bool, f32, f32); 5] = [
    ("plane50", false, 0.5, -6.0),
    ("plane25", false, 0.25, -3.0),
    ("cube50", true, 0.5, 0.0),
    ("cube25", true, 0.25, 3.0),
    ("checker", false, 1.0, 6.0),
];

/// An 8 × 8 RGBA PNG whose alpha is a checker of 0 and 1.
fn checker() -> Vec<u8> {
    let mut pixels = Vec::new();
    for y in 0..8u32 {
        for x in 0..8u32 {
            pixels.extend([255, 255, 255, if (x + y) % 2 == 1 { 255 } else { 0 }]);
        }
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 8, 8);
        encoder.set_color(png::ColorType::Rgba);
        encoder.write_header().unwrap().write_image_data(&pixels).unwrap();
    }
    out
}

fn scene() -> Scene {
    let mut scene = Scene::empty();
    scene.world.color = Vec3::ZERO;
    let floor = scene.add_material("Floor");
    let floor_object = scene.add_object("Floor", ObjectData::Mesh(Primitive::Plane { size: 40.0 }.build().unwrap()));
    scene.object_mut(floor_object).unwrap().material = Some(floor);
    let image = scene.add_image(Image::new("checker.png", checker()).unwrap());
    for (name, cube, alpha, x) in CASTERS {
        let material = scene.add_material(name);
        {
            let m = scene.material_mut(material).unwrap();
            m.base_color[3] = alpha;
            if name == "checker" {
                m.alpha_mode = AlphaMode::Clip;
                m.base_color_texture = Some(image.clone());
            } else {
                m.alpha_mode = AlphaMode::Blend;
            }
        }
        let mesh = if cube { Primitive::Cube { size: 1.0 } } else { Primitive::Plane { size: 1.0 } }.build().unwrap();
        let id = scene.add_object(name, ObjectData::Mesh(mesh));
        let object = scene.object_mut(id).unwrap();
        object.location = Vec3::new(x, -2.0, 2.0);
        object.material = Some(material);
    }
    let sun = scene.add_object("Sun", ObjectData::Light(Light { power: 3.0, radius: 0.0, ..Light::new(LightKind::Sun) }));
    scene.object_mut(sun).unwrap().rotation = Vec3::new(45f32.to_radians(), 0.0, 0.0);
    let camera = scene.add_object("Camera", ObjectData::Camera(Default::default()));
    {
        let object = scene.object_mut(camera).unwrap();
        object.location = Vec3::new(0.0, 0.0, 30.0);
        if let ObjectData::Camera(camera) = &mut object.data {
            camera.orthographic = true;
            camera.ortho_scale = 16.0;
        }
    }
    scene.camera = Some(camera);
    scene.render.width = WIDTH;
    scene.render.height = HEIGHT;
    scene
}

/// The open floor's light, and our shares of it in each shadow, read as the script reads the
/// reference's.
fn shares() -> (f64, Vec<(&'static str, f64)>) {
    // Light brighter than white is kept, so shares are of the light itself.
    let canvas =
        render_camera_passes(&scene(), &RenderOptions { shadows: true, ambient_occlusion: false, samples: 2 }, &mut MeshCache::new()).unwrap();
    // KITION_SCENE_SNAPSHOTS=<dir> writes the picture for a person to look at.
    if let Some(dir) = std::env::var_os("KITION_SCENE_SNAPSHOTS") {
        let file = std::fs::File::create(std::path::Path::new(&dir).join("shadows-alpha.png")).unwrap();
        let mut encoder = png::Encoder::new(file, WIDTH, HEIGHT);
        encoder.set_color(png::ColorType::Rgba);
        encoder.write_header().unwrap().write_image_data(&canvas.to_rgba8()).unwrap();
    }
    // World (x, y) to a pixel: rows run down from the top here.
    let at = |x: f32, y: f32| {
        let column = ((x + 8.0) / 16.0 * WIDTH as f32) as u32;
        let row = HEIGHT - 1 - ((y + 4.0) / 8.0 * HEIGHT as f32) as u32;
        f64::from(canvas.pixel(column, row).unwrap()[0])
    };
    let mean = |x0: f32, y0: f32, size: f32| {
        let steps = 16;
        let mut sum = 0.0;
        for i in 0..steps {
            for j in 0..steps {
                sum += at(x0 + (i as f32 + 0.5) / steps as f32 * size - size / 2.0, y0 + (j as f32 + 0.5) / steps as f32 * size - size / 2.0);
            }
        }
        sum / f64::from(steps * steps)
    };
    let lit = mean(0.0, 2.5, 1.0);
    (lit, CASTERS.iter().map(|(name, _, _, x)| (*name, mean(*x, 0.0, if *name == "checker" { 0.9 } else { 0.5 }) / lit)).collect())
}

/// A sun of strength 3 W/m² at 45° on a floor of albedo 0.8 gives 3 × cos 45° × 0.8 / π: 0.540,
/// the reference 0.536 (its diffuse loses a little to its specular layer). The fast renderers
/// had left out the π, making suns π times too bright (1.70).
#[test]
fn a_sun_lights_the_floor_as_the_reference_s_does() {
    let reference: Value = serde_json::from_str(REFERENCE).unwrap();
    let theirs = reference["lit"].as_f64().unwrap();
    let (lit, _) = shares();
    assert!((lit / theirs - 1.0).abs() < 0.02, "{lit:.4} against {theirs:.4}");
}

#[test]
fn transparent_shadows_let_through_what_the_reference_s_do() {
    let reference: Value = serde_json::from_str(REFERENCE).unwrap();
    // Each share within 0.02 of the reference's: plane 0.5 0.500 (0.508), plane 0.25 0.750
    // (0.758), cube 0.5 0.250 (0.246), cube 0.25 0.563 (0.558), checker 0.500 (0.501).
    let (_, ours) = shares();
    for (name, ours) in ours {
        let theirs = reference["shares"][name].as_f64().unwrap();
        eprintln!("{name}: {ours:.3} against {theirs:.3}");
        assert!((ours - theirs).abs() < 0.02, "{name}: {ours:.3} against {theirs:.3}");
    }
}

#[test]
fn the_fixture_is_the_reference_s() {
    let Some(blender) = std::env::var_os("KITION_BLENDER") else {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    };
    let dir = std::env::temp_dir().join(format!("kition-scene-shadows-alpha-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let output = dir.join("shadows-alpha.json");
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/blender/shadows_alpha.py");
    let status = std::process::Command::new(blender)
        .args(["-b", "--factory-startup", "--python"])
        .arg(&script)
        .arg("--")
        .arg(&output)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "Blender failed");
    let fresh: Value = serde_json::from_str(&std::fs::read_to_string(&output).unwrap()).unwrap();
    let fixture: Value = serde_json::from_str(REFERENCE).unwrap();
    for (name, ..) in CASTERS {
        let (a, b) = (fresh["shares"][name].as_f64().unwrap(), fixture["shares"][name].as_f64().unwrap());
        assert!((a - b).abs() < 1e-6, "{name}: {a} against the fixture's {b}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
