//! Defocus must convert its physical blur diameter through the fitted horizontal gate.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use kition_scene_model::{Camera, ObjectData, Scene, scene::SensorFit};

#[test]
fn defocus_receives_the_fitted_horizontal_gate() {
    for (width, height) in [(160, 90), (90, 160)] {
        for (fit, sensor_width) in [
            (SensorFit::Auto, if width > height { 39.0 } else { 39.0 * 90.0 / 160.0 }),
            (SensorFit::Horizontal, 39.0),
            (SensorFit::Vertical, 21.0 * width as f32 / height as f32),
        ] {
            let mut scene = Scene::empty();
            scene.render.width = width;
            scene.render.height = height;
            scene.camera = Some(scene.add_object(
                "Camera",
                ObjectData::Camera(Camera { lens: 47.0, sensor: 39.0, sensor_height: 21.0, sensor_fit: fit, ..Camera::default() }),
            ));
            let lens = super::lens(&scene).unwrap();
            assert!((lens.sensor_width - sensor_width).abs() < 1e-5, "{fit:?}: {width}x{height}: {} != {sensor_width}", lens.sensor_width);
            assert_eq!(lens.focal_length, 47.0);
        }
    }
}
