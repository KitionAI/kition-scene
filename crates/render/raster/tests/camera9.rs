//! Camera framing remains the same inside mismatched viewport rectangles and rotated frames.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use kition_scene_model::{
    Camera,
    glam::{Mat4, Quat, Vec3},
    scene::SensorFit,
};
use kition_scene_raster::view::CameraView;
use serde_json::Value;

#[test]
fn extreme_perspective_extents_avoid_angle_round_trip_error() {
    let reference: Value = serde_json::from_str(include_str!("../../../engine/model/tests/fixtures/camera9-extreme-reference.json")).unwrap();
    for case in reference["cases"].as_array().unwrap() {
        let fit = match case["fit"].as_str().unwrap() {
            "AUTO" => SensorFit::Auto,
            "HORIZONTAL" => SensorFit::Horizontal,
            _ => SensorFit::Vertical,
        };
        let camera = Camera { lens: 1.0, sensor: 200.0, sensor_height: 200.0, sensor_fit: fit, ..Camera::default() };
        let width = case["width"].as_u64().unwrap() as u32;
        let height = case["height"].as_u64().unwrap() as u32;
        let projector = CameraView { view: Mat4::IDENTITY, camera }.fit(width, height, width, height);
        let sx = case["matrix"][0][0].as_f64().unwrap() as f32;
        let sy = case["matrix"][1][1].as_f64().unwrap() as f32;
        assert!((projector.projection.x_axis.x / sx - 1.0).abs() < 2e-6, "{case}");
        assert!((projector.projection.y_axis.y / sy - 1.0).abs() < 2e-6, "{case}");
        let point = Vec3::new(0.5 / sx, -0.5 / sy, -1.0);
        let screen = projector.to_screen(projector.view_projection() * point.extend(1.0));
        assert!((screen.x - width as f32 * 0.75).abs() < 0.001);
        assert!((screen.y - height as f32 * 0.75).abs() < 0.001);
        let (origin, direction) = projector.ray(screen.x, screen.y);
        let on_plane = origin + direction * ((-1.0 - origin.z) / direction.z);
        assert!(((on_plane.x - point.x) * sx).abs() < 2e-6);
        assert!(((on_plane.y - point.y) * sy).abs() < 2e-6);
    }
}

#[test]
fn camera_projector_matches_the_recorded_frame_inside_every_viewport() {
    let reference: Value = serde_json::from_str(include_str!("../../../engine/model/tests/fixtures/camera9-reference.json")).unwrap();
    for case in reference["cases"].as_array().unwrap() {
        let fit = match case["fit"].as_str().unwrap() {
            "AUTO" => SensorFit::Auto,
            "HORIZONTAL" => SensorFit::Horizontal,
            _ => SensorFit::Vertical,
        };
        let camera = Camera {
            lens: 47.0,
            sensor: 39.0,
            sensor_height: 21.0,
            sensor_fit: fit,
            orthographic: case["kind"] == "ORTHO",
            ortho_scale: 7.0,
            clip_start: 0.2,
            clip_end: 80.0,
            ..Camera::default()
        };
        let render_width = case["width"].as_u64().unwrap() as u32;
        let render_height = case["height"].as_u64().unwrap() as u32;
        for world in
            [Mat4::IDENTITY, Mat4::from_rotation_translation(Quat::from_euler(glam::EulerRot::XYZ, 0.3, -0.9, 1.2), Vec3::new(2.0, -1.0, 5.0))]
        {
            let camera = CameraView { view: world.inverse(), camera: camera.clone() };
            for (width, height) in [(render_width, render_height), (400, 300), (200, 500)] {
                let projector = camera.fit(width, height, render_width, render_height);
                let [left, top, right, bottom] = projector.frame.unwrap();
                assert!(((right - left) / (bottom - top) - render_width as f32 / render_height as f32).abs() < 1e-6);
                for (point, expected) in reference["points"].as_array().unwrap().iter().zip(case["projected"].as_array().unwrap()) {
                    let point = world.transform_point3(Vec3::new(
                        point[0].as_f64().unwrap() as f32,
                        point[1].as_f64().unwrap() as f32,
                        point[2].as_f64().unwrap() as f32,
                    ));
                    let screen = projector.to_screen(projector.view_projection() * point.extend(1.0));
                    assert!(((screen.x - left) / (right - left) - expected[0].as_f64().unwrap() as f32).abs() < 1e-6);
                    assert!((1.0 - (screen.y - top) / (bottom - top) - expected[1].as_f64().unwrap() as f32).abs() < 1e-6);
                    let (origin, direction) = projector.ray(screen.x, screen.y);
                    assert!(
                        (point - origin).cross(direction).length() < 2e-5,
                        "picking uses the same frame: error {}, case {case}, size {width}x{height}, point {point}, origin {origin}, direction {direction}",
                        (point - origin).cross(direction).length()
                    );
                }
            }
        }
    }
}
