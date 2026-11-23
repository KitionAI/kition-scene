//! Sensor-fit commands, GPU projection, atomic failure and previous-release persistence.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use kition_scene_model::{Camera, ObjectData, scene::SensorFit};
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}
fn camera(doc: &SceneDocument, name: &str) -> Camera {
    let ObjectData::Camera(camera) = &doc.scene().object_named(name).unwrap().data else { panic!("camera") };
    camera.clone()
}
fn info(doc: &SceneDocument, name: &str) -> Value {
    doc.inspect().unwrap()["objects"].as_array().unwrap().iter().find(|object| object["name"] == name).unwrap()["camera"].clone()
}

#[test]
fn released_cameras_keep_their_auto_frames_and_new_settings_round_trip() {
    let mut doc = SceneDocument::open("Released.kscene", include_bytes!("fixtures/released-0.16.0.kscene")).unwrap();
    assert_eq!(kition_scene_kdm::schema().schema_version, 1);
    assert_eq!((doc.scene().render.width, doc.scene().render.height), (200, 320));
    let before = camera(&doc, "Camera");
    let ortho = camera(&doc, "Ortho");
    assert_eq!((before.sensor_fit, before.sensor_height), (SensorFit::Auto, 24.0));
    assert_eq!(before.lens, 43.0);
    assert_eq!(before.sensor, 32.0);
    let (x, y) = before.frame_half_size(200.0 / 320.0);
    assert!((x - (16.0 / 43.0) * 0.625).abs() < 1e-7);
    assert!((y - 16.0 / 43.0).abs() < 1e-7);
    assert_eq!(ortho.sensor_fit, SensorFit::Auto);
    assert_eq!(ortho.frame_half_size(0.625), (2.1875, 3.5));
    let particles = doc.scene().object_named("Cube").unwrap().particles.clone();
    run(&mut doc, "camera.set", json!({"object":"Camera", "sensorFit":"vertical", "sensorHeight":19.0}));
    let changed = camera(&doc, "Camera");
    assert_eq!((changed.sensor_fit, changed.sensor_height), (SensorFit::Vertical, 19.0));
    assert_eq!(info(&doc, "Camera")["sensorFit"], "vertical");
    assert_eq!(info(&doc, "Camera")["sensorHeight"], 19.0);
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(camera(&doc, "Camera"), before);
    run(&mut doc, "edit.redo", json!({}));
    assert_eq!(camera(&doc, "Camera"), changed);
    let reopened = SceneDocument::open("Saved.kscene", &doc.save(None).unwrap()).unwrap();
    assert_eq!(camera(&reopened, "Camera"), changed);
    assert_eq!(camera(&reopened, "Ortho"), ortho);
    assert_eq!(reopened.scene().object_named("Cube").unwrap().particles, particles);
}

#[test]
fn invalid_fit_and_height_are_atomic_and_described() {
    let mut doc = SceneDocument::create("Camera.kscene", 400, 300).unwrap();
    for params in [
        json!({"sensorFit":"diagonal"}),
        json!({"sensorFit":null}),
        json!({"sensorHeight":0}),
        json!({"sensorHeight":201}),
        json!({"sensorHeight":"wide"}),
    ] {
        let mut params = params;
        params["object"] = json!("Camera");
        params["lens"] = json!(77);
        let bytes = doc.save(None).unwrap();
        let details = doc.inspect().unwrap();
        assert!(doc.execute("camera.set", params).is_err());
        assert_eq!(doc.save(None).unwrap(), bytes);
        assert_eq!(doc.inspect().unwrap(), details);
    }
    let command = doc.commands().into_iter().find(|command| command.id == "camera.set").unwrap();
    assert!(command.mutates);
    assert!(command.description.contains("sensorFit"));
    assert!(command.schema.unwrap()["sensorFit"]["enum"].as_array().unwrap().iter().any(|value| value == "vertical"));
}

#[test]
fn gpu_camera_matrix_matches_blender_and_responds_to_render_aspect() {
    let reference: Value = serde_json::from_str(include_str!("../../../crates/engine/model/tests/fixtures/camera9-reference.json")).unwrap();
    let mut doc = SceneDocument::create("Camera.kscene", 400, 300).unwrap();
    run(&mut doc, "view.resize", json!({"width":400, "height":300}));
    run(&mut doc, "view.camera", json!({}));
    for case in reference["cases"].as_array().unwrap() {
        run(&mut doc, "render.set", json!({"width":case["width"], "height":case["height"]}));
        run(
            &mut doc,
            "camera.set",
            json!({"object":"Camera", "lens":47, "sensor":39, "sensorHeight":21, "sensorFit":case["fit"].as_str().unwrap().to_ascii_lowercase(), "orthographic":case["kind"] == "ORTHO", "orthoScale":7}),
        );
        let frame = doc.gl_scene();
        let c = &frame["camera"];
        let rect = c["frame"].as_array().unwrap();
        let width = (rect[2].as_f64().unwrap() - rect[0].as_f64().unwrap()) as f32;
        let height = (rect[3].as_f64().unwrap() - rect[1].as_f64().unwrap()) as f32;
        let projection = c["projection"].as_array().unwrap();
        assert!((projection[0].as_f64().unwrap() as f32 * 400.0 / width - case["matrix"][0][0].as_f64().unwrap() as f32).abs() < 2e-6);
        assert!((projection[5].as_f64().unwrap() as f32 * 300.0 / height - case["matrix"][1][1].as_f64().unwrap() as f32).abs() < 2e-6);
    }
}
