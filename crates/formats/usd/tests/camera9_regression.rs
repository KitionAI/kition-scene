//! A portrait Auto camera must write its actual fitted aperture to USD.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use kition_scene_model::{Camera, ObjectData, Scene};

#[test]
fn portrait_auto_exports_the_fitted_aperture() {
    let mut scene = Scene::empty();
    scene.render.width = 90;
    scene.render.height = 160;
    scene.add_object("Camera", ObjectData::Camera(Camera::default()));
    let text = String::from_utf8(kition_scene_usd::write(&scene, None).unwrap()).unwrap();
    let aperture = |name: &str| text.lines().find(|line| line.contains(name)).unwrap().split('=').nth(1).unwrap().trim().parse::<f32>().unwrap();
    assert!((aperture("horizontalAperture") - 0.2025).abs() < 1e-6);
    assert!((aperture("verticalAperture") - 0.36).abs() < 1e-6);
}
