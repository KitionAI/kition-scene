//! Keyframes through commands: keying, scrubbing, interpolation, renders of frames, saving.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn doc() -> SceneDocument {
    SceneDocument::create("Untitled cube.kscene", 1280, 720).unwrap().reopen("t.kscene").unwrap()
}

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn cube_x(doc: &SceneDocument) -> f64 {
    doc.inspect().unwrap()["objects"][0]["location"][0].as_f64().unwrap()
}

#[test]
fn keys_move_the_cube_between_frames() {
    let mut doc = doc();
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "anim.insertKey", json!({}));
    run(&mut doc, "frame.set", json!({ "frame": 21 }));
    run(&mut doc, "object.transform", json!({ "location": [4, 0, 0] }));
    let keyed = run(&mut doc, "anim.insertKey", json!({ "properties": ["location"] }));
    assert_eq!(keyed["keys"], 3);
    let inspected = doc.inspect().unwrap();
    assert_eq!(inspected["animation"]["selectedKeyFrames"], json!([1.0, 21.0]));
    assert_eq!(inspected["objects"][0]["animated"], json!(["location", "rotation", "scale"]));
    // Halfway the eased curve is at the middle.
    run(&mut doc, "frame.set", json!({ "frame": 11 }));
    assert!((cube_x(&doc) - 2.0).abs() < 1e-3);
    // Frame changes are not undo steps and leave the file clean.
    assert_eq!(doc.inspect().unwrap()["undo"], "Insert Keyframe");
    run(&mut doc, "anim.setInterpolation", json!({ "interpolation": "constant", "frame": 1 }));
    run(&mut doc, "frame.set", json!({ "frame": 15 }));
    assert_eq!(cube_x(&doc), 0.0);
    // An edit without a key lasts until the frame changes.
    run(&mut doc, "object.transform", json!({ "location": [9, 0, 0] }));
    assert_eq!(cube_x(&doc), 9.0);
    run(&mut doc, "frame.next", json!({}));
    assert_eq!(cube_x(&doc), 0.0);
    run(&mut doc, "anim.nextKey", json!({}));
    assert_eq!(doc.inspect().unwrap()["animation"]["frame"], 21);
    run(&mut doc, "anim.previousKey", json!({}));
    assert_eq!(doc.inspect().unwrap()["animation"]["frame"], 1);
    assert!(doc.execute("anim.previousKey", json!({})).is_err());
}

#[test]
fn animation_saves_and_reopens_at_its_frame() {
    let mut doc = doc();
    run(&mut doc, "anim.insertKey", json!({ "objects": "Light", "properties": ["power", "color"], "frame": 1 }));
    run(&mut doc, "anim.insertKey", json!({ "objects": "Cube", "properties": ["baseColor", "hidden"], "frame": 1 }));
    run(&mut doc, "anim.insertKey", json!({ "objects": "Camera", "properties": "lens", "frame": 1 }));
    run(&mut doc, "frame.range", json!({ "start": 1, "end": 48, "fps": 30 }));
    run(&mut doc, "frame.set", json!({ "frame": 12 }));
    // The range and the rate are the scene's, in the file; the current frame is the person's view
    // state, which the host keeps beside it: the file alone opens at the first frame, and with the
    // view state at the frame it was saved at.
    let alone = SceneDocument::open("t.kscene", &doc.save_file(None).unwrap()).unwrap();
    let animation = alone.inspect().unwrap()["animation"].clone();
    assert_eq!((animation["frame"].as_i64(), animation["end"].as_i64(), animation["fps"].as_f64()), (Some(1), Some(48), Some(30.0)));
    let reopened = doc.reopen("t.kscene").unwrap();
    let animation = reopened.inspect().unwrap()["animation"].clone();
    assert_eq!((animation["frame"].as_i64(), animation["end"].as_i64(), animation["fps"].as_f64()), (Some(12), Some(48), Some(30.0)));
    assert_eq!(animation["channels"].as_array().unwrap().len(), 1 + 3 + 4 + 1 + 1);
    assert!(doc.execute("anim.insertKey", json!({ "objects": "Light", "properties": ["lens"] })).is_err());
    assert!(doc.execute("frame.range", json!({ "start": 10, "end": 2 })).is_err());
    // Deleting the cube takes its channels; its material keeps its own.
    run(&mut doc, "edit.delete", json!({ "objects": "Cube" }));
    let channels = doc.inspect().unwrap()["animation"]["channels"].as_array().unwrap().len();
    assert_eq!(channels, 1 + 3 + 4 + 1);
}

#[test]
fn renders_follow_the_frame() {
    let mut doc = doc();
    run(&mut doc, "render.set", json!({ "width": 64, "height": 36 }));
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "anim.insertKey", json!({ "frame": 1 }));
    run(&mut doc, "object.transform", json!({ "location": [0, 0, 30] }));
    run(&mut doc, "anim.insertKey", json!({ "frame": 10 }));
    let first = run(&mut doc, "render.image", json!({ "frame": 1, "samples": 1 }));
    let last = run(&mut doc, "render.image", json!({ "frame": 10, "samples": 1 }));
    assert_ne!(first["base64"], last["base64"], "the cube left the shot");
    let zip = run(&mut doc, "render.animation", json!({ "start": 1, "end": 10, "step": 3 }));
    assert_eq!(zip["frames"], 4);
    run(&mut doc, "edit.duplicate", json!({}));
    let channels = doc.inspect().unwrap()["animation"]["channels"].as_array().unwrap().len();
    assert_eq!(channels, 18, "the duplicate has its own keys");
}
