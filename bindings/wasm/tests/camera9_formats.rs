//! Exchange cameras preserve the authored frame at the same render aspect, not fit policy.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use kition_scene_model::{Camera, ObjectData, Scene, scene::SensorFit};
use serde_json::Value;

fn scene(width: u32, height: u32) -> Scene {
    let mut scene = Scene::empty();
    scene.render.width = width;
    scene.render.height = height;
    for orthographic in [false, true] {
        for (name, fit) in [("AUTO", SensorFit::Auto), ("HORIZONTAL", SensorFit::Horizontal), ("VERTICAL", SensorFit::Vertical)] {
            scene.add_object(
                &format!("{}_{name}", if orthographic { "ORTHO" } else { "PERSP" }),
                ObjectData::Camera(Camera {
                    lens: 47.0,
                    sensor: 39.0,
                    sensor_height: 21.0,
                    sensor_fit: fit,
                    orthographic,
                    ortho_scale: 7.0,
                    ..Camera::default()
                }),
            );
        }
    }
    scene
}

fn exports(scene: &Scene) -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("glb", kition_scene_exchange::export(scene, kition_scene_exchange::Format::Glb, None).unwrap()),
        ("usda", kition_scene_usd::write(scene, None).unwrap()),
        ("usdc", kition_scene_usd::write_binary(scene, None).unwrap()),
        ("fbx", kition_scene_fbx::write(scene, None).unwrap()),
    ]
}

#[test]
fn extreme_gltf_angles_preserve_the_recorded_frame() {
    let reference: Value = serde_json::from_str(include_str!("../../../crates/engine/model/tests/fixtures/camera9-extreme-reference.json")).unwrap();
    for case in reference["cases"].as_array().unwrap() {
        let mut scene = Scene::empty();
        scene.render.width = case["width"].as_u64().unwrap() as u32;
        scene.render.height = case["height"].as_u64().unwrap() as u32;
        let fit = match case["fit"].as_str().unwrap() {
            "AUTO" => SensorFit::Auto,
            "HORIZONTAL" => SensorFit::Horizontal,
            _ => SensorFit::Vertical,
        };
        scene.add_object(
            "Camera",
            ObjectData::Camera(Camera { lens: 1.0, sensor: 200.0, sensor_height: 200.0, sensor_fit: fit, ..Camera::default() }),
        );
        let bytes = kition_scene_exchange::export(&scene, kition_scene_exchange::Format::Gltf, None).unwrap();
        let json: Value = serde_json::from_slice(&bytes).unwrap();
        let half_height = (json["cameras"][0]["perspective"]["yfov"].as_f64().unwrap() / 2.0).tan();
        let expected_height = 1.0 / case["matrix"][1][1].as_f64().unwrap();
        assert!((half_height / expected_height - 1.0).abs() < 2e-6, "{case}");
        let mut actual = Scene::empty();
        kition_scene_exchange::place(&mut actual, kition_scene_exchange::import(&bytes, kition_scene_exchange::Format::Gltf).unwrap()).unwrap();
        let ObjectData::Camera(camera) = &actual.object_named("Camera").unwrap().data else { panic!("camera") };
        let (x, y) = camera.frame_half_size(scene.render.width as f32 / scene.render.height as f32);
        assert!((f64::from(y) / expected_height - 1.0).abs() < 2e-6);
        assert!((f64::from(x) * case["matrix"][0][0].as_f64().unwrap() - 1.0).abs() < 2e-6);
    }
}

#[test]
fn blend_import_retains_each_sensor_dimension_and_fit() {
    let read = kition_scene_blend::read(include_bytes!("../../../crates/formats/blend/tests/fixtures/camera9.blend")).unwrap();
    let mut actual = Scene::empty();
    kition_scene_blend::place(&mut actual, read).unwrap();
    let expected = scene(90, 160);
    for object in expected.objects {
        let ObjectData::Camera(expected) = object.data else { panic!("camera") };
        let ObjectData::Camera(actual) = &actual.object_named(&object.name).unwrap().data else { panic!("camera") };
        assert_eq!(actual.sensor_fit, expected.sensor_fit);
        assert_eq!(actual.sensor_height, expected.sensor_height);
        assert_eq!(actual.sensor, expected.sensor);
        assert_eq!(actual.lens, expected.lens);
        assert_eq!(actual.orthographic, expected.orthographic);
        assert_eq!(actual.ortho_scale, expected.ortho_scale);
    }
}

#[test]
fn native_camera_fields_round_trip_framing_at_the_authored_aspect() {
    for (width, height) in [(160, 90), (90, 160)] {
        let source = scene(width, height);
        for (format, bytes) in exports(&source) {
            let imported = match format {
                "glb" => kition_scene_exchange::import(&bytes, kition_scene_exchange::Format::Glb).unwrap(),
                "fbx" => kition_scene_fbx::read(&bytes).unwrap(),
                _ => kition_scene_usd::read(&bytes).unwrap(),
            };
            let mut actual = Scene::empty();
            kition_scene_exchange::place(&mut actual, imported).unwrap();
            for object in &source.objects {
                let ObjectData::Camera(expected) = &object.data else { panic!("camera") };
                let ObjectData::Camera(camera) = &actual.object_named(&object.name).unwrap().data else { panic!("camera") };
                assert_eq!(camera.orthographic, expected.orthographic, "{format}: {}", object.name);
                let aspect = width as f32 / height as f32;
                let (x, y) = camera.frame_half_size(aspect);
                let (ex, ey) = expected.frame_half_size(aspect);
                assert!((x - ex).abs() < 2e-5 && (y - ey).abs() < 2e-5, "{format}: {} {width}x{height}: ({x}, {y}) != ({ex}, {ey})", object.name);
            }
        }
    }
}

#[test]
fn expanded_fbx_gates_preserve_wide_portrait_and_landscape_frames() {
    for (width, height) in [(1600, 90), (90, 1600), (8192, 64), (64, 8192)] {
        let source = scene(width, height);
        let bytes = kition_scene_fbx::write(&source, None).unwrap();
        let imported = kition_scene_fbx::read(&bytes).unwrap();
        let mut actual = Scene::empty();
        kition_scene_exchange::place(&mut actual, imported).unwrap();
        for object in &source.objects {
            let ObjectData::Camera(expected) = &object.data else { panic!("camera") };
            let ObjectData::Camera(camera) = &actual.object_named(&object.name).unwrap().data else { panic!("camera") };
            let aspect = width as f32 / height as f32;
            let (x, y) = camera.frame_half_size(aspect);
            let (ex, ey) = expected.frame_half_size(aspect);
            assert!((x - ex).abs() < 2e-5 && (y - ey).abs() < 2e-5, "FBX: {} {width}x{height}: ({x}, {y}) != ({ex}, {ey})", object.name);
        }
    }
}

#[test]
fn optional_blender_reads_the_exported_frames_at_the_same_aspect() {
    let Some(blender) = std::env::var_os("KITION_BLENDER") else { return };
    let dir = std::env::temp_dir().join(format!("kition-camera9-exports-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (width, height) in [(160, 90), (90, 160)] {
        let source = scene(width, height);
        for (format, bytes) in exports(&source) {
            let path = dir.join(format!("{width}-{height}.{format}"));
            std::fs::write(&path, bytes).unwrap();
            let result = dir.join("result.json");
            let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/engine/model/tests/blender/camera9_read.py");
            let output = std::process::Command::new(&blender)
                .args(["--background", "--factory-startup", "--python"])
                .arg(script)
                .arg("--")
                .arg(&path)
                .arg(width.to_string())
                .arg(height.to_string())
                .arg(&result)
                .output()
                .unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            let actual: Value = serde_json::from_slice(&std::fs::read(result).unwrap()).unwrap();
            for object in &source.objects {
                let ObjectData::Camera(expected) = &object.data else { panic!("camera") };
                let (x, y) = expected.frame_half_size(width as f32 / height as f32);
                let matrix = &actual[&object.name];
                assert!(matrix.is_array(), "{format} omitted {}: {actual}", object.name);
                let matrix = if matches!(format, "usda" | "usdc") && expected.orthographic {
                    // Blender 5.2.3's importer treats orthographic apertures as scene units;
                    // USD specifies tenths of a scene unit. Preserve that measured boundary
                    // while using the public USD frustum to check the exported frame.
                    assert!((matrix[0][0].as_f64().unwrap() * f64::from(x) * 10.0 - 1.0).abs() < 1e-6);
                    assert!((matrix[1][1].as_f64().unwrap() * f64::from(y) * 10.0 - 1.0).abs() < 1e-6);
                    &actual["_usd"][&object.name]
                } else {
                    matrix
                };
                let actual_x = 1.0 / matrix[0][0].as_f64().unwrap() as f32;
                let actual_y = 1.0 / matrix[1][1].as_f64().unwrap() as f32;
                assert!(
                    (actual_x - x).abs() < 2e-5 && (actual_y - y).abs() < 2e-5,
                    "{format} {}: ({actual_x}, {actual_y}) != ({x}, {y})",
                    object.name
                );
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn optional_blender_reads_wide_fbx_frames() {
    let Some(blender) = std::env::var_os("KITION_BLENDER") else { return };
    let dir = std::env::temp_dir().join(format!("kition-camera9-wide-fbx-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (width, height) in [(1600, 90), (90, 1600), (8192, 64), (64, 8192)] {
        let source = scene(width, height);
        for (format, bytes) in [("fbx", kition_scene_fbx::write(&source, None).unwrap())] {
            let path = dir.join(format!("{width}-{height}.{format}"));
            std::fs::write(&path, bytes).unwrap();
            let result = dir.join("result.json");
            let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/engine/model/tests/blender/camera9_read.py");
            let output = std::process::Command::new(&blender)
                .args(["--background", "--factory-startup", "--python"])
                .arg(script)
                .arg("--")
                .arg(&path)
                .arg(width.to_string())
                .arg(height.to_string())
                .arg(&result)
                .output()
                .unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            let actual: Value = serde_json::from_slice(&std::fs::read(result).unwrap()).unwrap();
            for object in &source.objects {
                let ObjectData::Camera(expected) = &object.data else { panic!("camera") };
                let (x, y) = expected.frame_half_size(width as f32 / height as f32);
                let matrix = &actual[&object.name];
                assert!(matrix.is_array(), "{format} omitted {}: {actual}", object.name);
                let actual_x = 1.0 / matrix[0][0].as_f64().unwrap() as f32;
                let actual_y = 1.0 / matrix[1][1].as_f64().unwrap() as f32;
                assert!(
                    (matrix[0][0].as_f64().unwrap() * f64::from(x) - 1.0).abs() < 1e-6
                        && (matrix[1][1].as_f64().unwrap() * f64::from(y) - 1.0).abs() < 1e-6,
                    "{format} {}: ({actual_x}, {actual_y}) != ({x}, {y})",
                    object.name
                );
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
