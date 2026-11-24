//! Physics through commands: settings (undoable, saved, inspectable), the bake the scene shows
//! at each frame, stepping a bake, out-of-date bakes starting again, and freeing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn empty() -> SceneDocument {
    let mut doc = SceneDocument::create("Untitled.kscene", 640, 360).unwrap();
    run(&mut doc, "select.all", json!({}));
    run(&mut doc, "edit.delete", json!({}));
    doc
}

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn z_of(doc: &SceneDocument, name: &str) -> f64 {
    let inspected = doc.inspect().unwrap();
    let object = inspected["objects"].as_array().unwrap().iter().find(|object| object["name"] == name).unwrap().clone();
    object["worldLocation"][2].as_f64().unwrap()
}

/// A ground plane and a cube 3 m up, both rigid bodies.
fn drop_scene() -> SceneDocument {
    let mut doc = empty();
    run(&mut doc, "add.plane", json!({ "size": 10, "name": "Ground" }));
    run(&mut doc, "physics.rigidBody.add", json!({ "type": "passive" }));
    run(&mut doc, "add.cube", json!({ "size": 1, "location": [0, 0, 3], "name": "Box" }));
    run(&mut doc, "physics.rigidBody.add", json!({ "objects": "Box", "mass": 2 }));
    run(&mut doc, "physics.world.set", json!({ "frameEnd": 60 }));
    doc
}

#[test]
fn settings_are_undoable_saved_and_inspected() {
    let mut doc = drop_scene();
    let physics = &doc.inspect().unwrap()["physics"];
    assert_eq!(physics["world"]["frameEnd"], 60);
    assert_eq!(physics["state"], "none");
    let objects = physics["objects"].as_array().unwrap();
    assert_eq!(objects.len(), 2);
    assert_eq!(objects[1]["rigidBody"]["mass"].as_f64(), Some(2.0));
    assert_eq!(objects[0]["rigidBody"]["kind"], "passive");
    assert!(doc.execute("physics.rigidBody.set", json!({ "objects": "Box", "restitution": 2 })).is_err(), "out of range");
    assert!(doc.execute("physics.cloth.set", json!({ "objects": "Box", "mass": 1 })).is_err(), "no cloth to set");
    run(&mut doc, "physics.cloth.add", json!({ "objects": "Box", "pinGroup": "Pins" }));
    run(&mut doc, "edit.undo", json!({}));
    assert!(doc.inspect().unwrap()["physics"]["objects"][1].get("cloth").is_none(), "undo took the cloth back");
    let reopened = SceneDocument::open("t.kscene", &doc.save_file(None).unwrap()).unwrap();
    let physics = &reopened.inspect().unwrap()["physics"];
    assert_eq!(physics["world"]["frameEnd"], 60);
    assert_eq!(physics["objects"][1]["rigidBody"]["mass"].as_f64(), Some(2.0));
    assert_eq!(physics["state"], "none", "the bake is never saved");
    run(&mut doc, "physics.rigidBody.remove", json!({ "objects": "Box" }));
    assert_eq!(doc.inspect().unwrap()["physics"]["objects"].as_array().unwrap().len(), 1);
}

#[test]
fn a_bake_shows_on_the_timeline_and_follows_changes() {
    let mut doc = drop_scene();
    let baked = run(&mut doc, "physics.bake", json!({}));
    assert_eq!((baked["state"].as_str(), baked["frames"].as_u64()), (Some("baked"), Some(60)));
    assert!((z_of(&doc, "Box") - 3.0).abs() < 1e-3, "frame 1 is the start");
    run(&mut doc, "frame.set", json!({ "frame": 60 }));
    let rest = z_of(&doc, "Box");
    assert!((rest - 0.5).abs() < 0.05, "the box rests on the ground: {rest}");
    // Moving the box makes the bake out of date; a new bake starts and steps to the end.
    run(&mut doc, "frame.set", json!({ "frame": 1 }));
    run(&mut doc, "object.transform", json!({ "objects": "Box", "location": [1, 0, 2] }));
    let state = doc.inspect().unwrap()["physics"].clone();
    assert_eq!(state["state"], "baking");
    let mut steps = 0;
    while run(&mut doc, "physics.bakeStep", json!({ "frames": 7 }))["state"] == "baking" {
        steps += 1;
        assert!(steps < 20);
    }
    assert_eq!(doc.inspect().unwrap()["physics"]["state"], "baked");
    run(&mut doc, "frame.set", json!({ "frame": 60 }));
    assert!((z_of(&doc, "Box") - 0.5).abs() < 0.05);
    // Undo comes back to the old scene; its bake starts again.
    run(&mut doc, "edit.undo", json!({}));
    run(&mut doc, "physics.bake", json!({}));
    assert!((z_of(&doc, "Box") - rest).abs() < 1e-5, "the solver is deterministic");
    run(&mut doc, "physics.free", json!({}));
    assert_eq!(doc.inspect().unwrap()["physics"]["state"], "none");
    assert!((z_of(&doc, "Box") - 3.0).abs() < 1e-3, "without the bake the box stays where it is");
}

#[test]
fn baked_motion_exports_to_gltf() {
    let mut doc = drop_scene();
    run(&mut doc, "physics.bake", json!({}));
    let exported = run(&mut doc, "file.export", json!({ "format": "gltf" }));
    let base64 = exported["base64"].as_str().unwrap();
    assert!(base64.len() > 100);
    let mut unbaked = drop_scene();
    let plain = run(&mut unbaked, "file.export", json!({ "format": "gltf" }));
    assert!(plain["base64"].as_str().unwrap().len() < base64.len(), "the baked fall adds animation samples");
}
