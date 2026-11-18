//! Actual traced silhouettes follow the public-API camera matrices for every fitted frame.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use kition_scene_model::{Camera, ObjectData, Primitive, Scene, glam::Vec3, scene::SensorFit};
use kition_scene_trace::{TraceOptions, Tracer};
use serde_json::Value;

#[test]
fn traced_asymmetric_plane_matches_the_recorded_projection() {
    let reference: Value = serde_json::from_str(include_str!("../../../engine/model/tests/fixtures/camera9-reference.json")).unwrap();
    for case in reference["cases"].as_array().unwrap() {
        let mut scene = Scene::empty();
        scene.world.color = Vec3::ZERO;
        scene.render.width = case["width"].as_u64().unwrap() as u32;
        scene.render.height = case["height"].as_u64().unwrap() as u32;
        let camera = Camera {
            lens: 47.0,
            sensor: 39.0,
            sensor_height: 21.0,
            sensor_fit: match case["fit"].as_str().unwrap() {
                "AUTO" => SensorFit::Auto,
                "HORIZONTAL" => SensorFit::Horizontal,
                _ => SensorFit::Vertical,
            },
            orthographic: case["kind"] == "ORTHO",
            ortho_scale: 7.0,
            ..Camera::default()
        };
        let ortho = camera.orthographic;
        scene.camera = Some(scene.add_object("Camera", ObjectData::Camera(camera)));
        let material = scene.add_material("Glow");
        let glow = scene.material_mut(material).unwrap();
        glow.base_color = [0.0, 0.0, 0.0, 1.0];
        glow.emission = Vec3::ONE;
        glow.emission_strength = 1.0;
        let plane = scene.add_object("Plane", ObjectData::Mesh(Primitive::Plane { size: 1.0 }.build().unwrap()));
        let plane = scene.object_mut(plane).unwrap();
        plane.location = Vec3::new(0.2, -0.15, -4.0);
        plane.scale = Vec3::new(0.6, 0.8, 1.0);
        plane.material = Some(material);
        let mut tracer = Tracer::new(&scene, TraceOptions { samples: 1, bounces: 0, ..Default::default() }).unwrap();
        tracer.finish();
        let image = tracer.linear();
        let w = scene.render.width as f32;
        let h = scene.render.height as f32;
        let distance = if ortho { 1.0 } else { 4.0 };
        let sx = case["matrix"][0][0].as_f64().unwrap() as f32 / distance;
        let sy = case["matrix"][1][1].as_f64().unwrap() as f32 / distance;
        let left = (-0.1 * sx + 1.0) * 0.5 * w;
        let right = (0.5 * sx + 1.0) * 0.5 * w;
        let top = (1.0 - 0.25 * sy) * 0.5 * h;
        let bottom = (1.0 + 0.55 * sy) * 0.5 * h;
        let mut inside = 0;
        for (index, color) in image.iter().enumerate() {
            let x = (index % scene.render.width as usize) as f32;
            let y = (index / scene.render.width as usize) as f32;
            // A sample lies anywhere in its pixel. Only pixels wholly inside or outside
            // the measured silhouette have an unambiguous one-sample classification.
            if x > left && x + 1.0 < right && y > top && y + 1.0 < bottom {
                assert!(color.min_element() > 0.9, "{case}: missing plane at {x},{y}");
                inside += 1;
            }
            if x + 1.0 < left || x > right || y + 1.0 < top || y > bottom {
                assert!(color.max_element() < 1e-6, "{case}: plane outside frame bounds at {x},{y}");
            }
        }
        assert!(inside > 0);
    }
}
