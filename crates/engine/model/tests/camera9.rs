//! Camera fit is measured through Blender's public API; offline recordings are authoritative.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use kition_scene_model::{Camera, scene::SensorFit};
use serde_json::Value;

const REFERENCE: &str = include_str!("fixtures/camera9-reference.json");

#[test]
fn fitted_frames_match_blender_in_both_projections_and_every_aspect() {
    let reference: Value = serde_json::from_str(REFERENCE).unwrap();
    for case in reference["cases"].as_array().unwrap() {
        let camera = Camera {
            lens: 47.0,
            sensor: 39.0,
            sensor_height: 21.0,
            sensor_fit: match case["fit"].as_str().unwrap() {
                "AUTO" => SensorFit::Auto,
                "HORIZONTAL" => SensorFit::Horizontal,
                "VERTICAL" => SensorFit::Vertical,
                other => panic!("unknown reference fit {other}"),
            },
            orthographic: case["kind"] == "ORTHO",
            ortho_scale: 7.0,
            ..Camera::default()
        };
        let aspect = case["width"].as_f64().unwrap() as f32 / case["height"].as_f64().unwrap() as f32;
        let (x, y) = camera.frame_half_size(aspect);
        assert!((x - 1.0 / case["matrix"][0][0].as_f64().unwrap() as f32).abs() < 1e-6, "{case}");
        assert!((y - 1.0 / case["matrix"][1][1].as_f64().unwrap() as f32).abs() < 1e-6, "{case}");
        for (point, projected) in reference["points"].as_array().unwrap().iter().zip(case["projected"].as_array().unwrap()) {
            let distance = if camera.orthographic { 1.0 } else { -point[2].as_f64().unwrap() as f32 };
            let u = (point[0].as_f64().unwrap() as f32 / distance / x + 1.0) * 0.5;
            let v = (point[1].as_f64().unwrap() as f32 / distance / y + 1.0) * 0.5;
            assert!((u - projected[0].as_f64().unwrap() as f32).abs() < 1e-6);
            assert!((v - projected[1].as_f64().unwrap() as f32).abs() < 1e-6);
        }
        let mut other = camera.clone();
        if camera.sensor_fit == SensorFit::Vertical {
            other.sensor = 91.0;
        } else {
            other.sensor_height = 91.0;
        }
        assert_eq!(camera.frame_half_size(aspect), other.frame_half_size(aspect), "the unused sensor dimension has no effect");
    }
}

#[test]
fn optional_live_blender_rechecks_the_committed_observation() {
    let Some(blender) = std::env::var_os("KITION_BLENDER") else { return };
    let path = std::env::temp_dir().join(format!("kition-camera9-{}.json", std::process::id()));
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/blender/camera9.py");
    let output = std::process::Command::new(blender)
        .args(["--background", "--factory-startup", "--python"])
        .arg(script)
        .arg("--")
        .arg(&path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let recorded: Value = serde_json::from_str(REFERENCE).unwrap();
    let observed: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(observed, recorded);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn optional_live_blender_rechecks_extreme_frames() {
    let Some(blender) = std::env::var_os("KITION_BLENDER") else { return };
    let path = std::env::temp_dir().join(format!("kition-camera9-extreme-{}.json", std::process::id()));
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/blender/camera9_extreme.py");
    let output = std::process::Command::new(blender)
        .args(["--background", "--factory-startup", "--python"])
        .arg(script)
        .arg("--")
        .arg(&path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let recorded: Value = serde_json::from_str(include_str!("fixtures/camera9-extreme-reference.json")).unwrap();
    let observed: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(observed, recorded);
    std::fs::remove_file(path).unwrap();
}
