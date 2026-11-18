//! Arbitrary viewport projections against Blender's public API renders. Fixtures run offline;
//! KITION_BLENDER checks newly observed output against the same immutable references.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use kition_scene_model::{Light, LightKind, ObjectData, Primitive, Scene, glam::Vec3};
use kition_scene_raster::{MeshCache, Overlays, Prepared, ViewState};
use kition_scene_trace::{TraceOptions, Tracer};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

fn scene() -> Scene {
    let mut scene = Scene::empty();
    scene.world.color = Vec3::splat(0.05);
    scene.world.strength = 1.0;
    for (name, primitive, at, color, roughness) in [
        ("Blue", Primitive::Cube { size: 2.0 }, Vec3::Z, [0.1, 0.25, 0.7, 1.0], 0.35),
        ("Floor", Primitive::Plane { size: 12.0 }, Vec3::ZERO, [0.65, 0.65, 0.65, 1.0], 0.7),
    ] {
        let material = scene.add_material(name);
        scene.material_mut(material).unwrap().base_color = color;
        scene.material_mut(material).unwrap().roughness = roughness;
        let id = scene.add_object(name, ObjectData::Mesh(primitive.build().unwrap()));
        scene.object_mut(id).unwrap().location = at;
        scene.object_mut(id).unwrap().material = Some(material);
    }
    let mut light = Light::new(LightKind::Area);
    light.power = 400.0;
    light.radius = 2.0;
    let id = scene.add_object("Light", ObjectData::Light(light));
    scene.object_mut(id).unwrap().location = Vec3::new(0.0, 0.0, 5.0);
    scene
}
fn meshes(scene: &Scene) -> HashMap<u32, Arc<Prepared>> {
    kition_scene_raster::viewport_meshes(scene, &Overlays::default(), &mut MeshCache::new()).into_iter().map(|(id, mesh, _)| (id, mesh)).collect()
}
fn view(perspective: bool) -> ViewState {
    ViewState { target: Vec3::Z, distance: 8.0, tilt: 60f32.to_radians(), turn: 30f32.to_radians(), perspective, ..Default::default() }
}
fn picture(path: &Path) -> Vec<u8> {
    let mut reader = png::Decoder::new(std::fs::File::open(path).unwrap()).read_info().unwrap();
    let mut data = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut data).unwrap();
    assert_eq!((info.width, info.height, info.color_type), (64, 48, png::ColorType::Rgba));
    data.truncate(info.buffer_size());
    data
}
fn reference(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/viewport7").join(name)
}
fn compare(ours: &[u8], other: &[u8]) -> f64 {
    assert_eq!(ours.len(), other.len());
    let mut difference = 0.0;
    for y in (0..48).step_by(4) {
        for x in (0..64).step_by(4) {
            for channel in 0..3 {
                let mean = |image: &[u8]| -> f64 {
                    (0..4).flat_map(|dy| (0..4).map(move |dx| image[((y + dy) * 64 + x + dx) * 4 + channel] as f64)).sum::<f64>() / 16.0
                };
                difference += (mean(ours) - mean(other)).abs();
            }
        }
    }
    difference / (12 * 16 * 3) as f64
}
#[test]
fn orbit_views_use_scene_lighting_like_blender() {
    let scene = scene();
    let meshes = meshes(&scene);
    let facts: serde_json::Value = serde_json::from_slice(&std::fs::read(reference("reference.json")).unwrap()).unwrap();
    assert_eq!(facts["renderedUsesSceneLights"], true);
    assert_eq!(facts["renderedUsesSceneWorld"], true);
    let live = std::env::var_os("KITION_BLENDER").map(|blender| {
        let out = std::env::temp_dir().join(format!("scene-viewport7-reference-{}", std::process::id()));
        let status = std::process::Command::new(blender)
            .args(["--background", "--threads", "2", "--python"])
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/blender/viewport7.py"))
            .arg("--")
            .arg(&out)
            .status()
            .unwrap();
        assert!(status.success());
        out
    });
    for (name, perspective) in [("perspective", true), ("orthographic", false)] {
        let projector = view(perspective).projector(&scene, 64, 48);
        let mut tracer = Tracer::viewport(&scene, projector, None, TraceOptions { samples: 128, ..Default::default() }, &meshes).unwrap();
        tracer.finish();
        let expected = picture(&reference(&format!("{name}.RENDER.CYCLES.png")));
        let error = compare(&tracer.image(), &expected);
        println!("{name}: mean block error {error:.3} levels");
        // Independent Monte Carlo samplers and finite samples differ at edges and in shadows.
        assert!(error < 4.0, "{name}: {error} levels");
        if let Some(out) = &live {
            assert!(compare(&picture(&out.join(format!("{name}.RENDER.CYCLES.png"))), &expected) < 0.2);
        }
    }
}
#[test]
fn viewport_sample_targets_reuse_the_scene_and_match_fresh_accumulation() {
    let scene = scene();
    let meshes = meshes(&scene);
    let make = |samples| {
        Tracer::viewport(&scene, view(true).projector(&scene, 32, 24), None, TraceOptions { samples, ..Default::default() }, &meshes).unwrap()
    };
    let mut growing = make(2);
    growing.finish();
    growing.set_samples(4);
    while !growing.step_rows(3).done {}
    let mut fresh = make(4);
    fresh.finish();
    assert_eq!(growing.image(), fresh.image());
    growing.set_samples(1);
    growing.finish();
    let mut one = make(1);
    one.finish();
    assert_eq!(growing.image(), one.image());
}
#[test]
fn a_viewport_needs_no_camera_and_refuses_invalid_projection_or_size() {
    let scene = scene();
    let meshes = meshes(&scene);
    assert!(scene.camera.is_none());
    let mut projector = view(true).projector(&scene, 32, 24);
    projector.width = 0;
    assert!(Tracer::viewport(&scene, projector, None, TraceOptions::default(), &meshes).is_err());
    let mut projector = view(true).projector(&scene, 32, 24);
    projector.view = kition_scene_model::glam::Mat4::from_cols_array(&[f32::NAN; 16]);
    assert!(Tracer::viewport(&scene, projector, None, TraceOptions::default(), &meshes).is_err());
}

#[test]
fn preview_steps_yield_for_budget_rows_and_completed_passes() {
    let scene = scene();
    let mut tracer =
        Tracer::viewport(&scene, view(true).projector(&scene, 8, 64), None, TraceOptions { samples: 2, ..Default::default() }, &meshes(&scene))
            .unwrap();
    let tick = std::cell::Cell::new(0.0);
    let progress = tracer.step_preview(8.0, &|| {
        let now = tick.get();
        tick.set(now + 8.0);
        now
    });
    assert_eq!(progress.samples, 0);
    assert!((progress.fraction - 1.0 / 128.0).abs() < 1e-6);
    let capped = tracer.step_preview(8.0, &|| 0.0);
    assert!((capped.fraction - 33.0 / 128.0).abs() < 1e-6);
    let boundary = tracer.step_preview(8.0, &|| 0.0);
    assert_eq!(boundary.samples, 1);
    assert_eq!(boundary.fraction, 0.5);
}
