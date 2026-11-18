//! Constraints and weight paint through commands, as the agent and the editor drive them.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_kit::tools::{KitTools, PointerAction, ToolMods, ToolPointer};
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn object<'a>(inspected: &'a Value, name: &str) -> &'a Value {
    inspected["objects"].as_array().unwrap().iter().find(|object| object["name"] == name).unwrap_or_else(|| panic!("no {name}"))
}

fn world(doc: &SceneDocument, name: &str) -> [f64; 3] {
    let inspected = doc.inspect().unwrap();
    let at = &object(&inspected, name)["worldLocation"];
    [at[0].as_f64().unwrap(), at[1].as_f64().unwrap(), at[2].as_f64().unwrap()]
}

#[test]
fn constraints_on_objects() {
    let mut doc = SceneDocument::create("C.kscene", 1280, 720).unwrap();
    run(&mut doc, "add.empty", json!({ "name": "Goal", "location": [3, 4, 0] }));
    // The camera tracks the goal.
    let added = run(&mut doc, "constraint.add", json!({ "object": "Camera", "type": "trackTo", "target": "Goal" }));
    assert_eq!(added["name"], "Track To");
    let camera = object(&doc.inspect().unwrap(), "Camera").clone();
    let reopened = SceneDocument::open("C.kscene", &doc.save(None).unwrap()).unwrap();
    assert_eq!(object(&reopened.inspect().unwrap(), "Camera")["constraints"], camera["constraints"], "constraints survive saving");
    assert_eq!(camera["constraints"][0]["target"]["name"], "Goal");
    assert_eq!(camera["constraints"][0]["trackAxis"], "-z");
    // A cycle is refused, and leaves the scene as it was.
    let before = doc.save(None).unwrap();
    assert!(doc.execute("constraint.add", json!({ "object": "Goal", "type": "dampedTrack", "target": "Camera" })).is_err());
    assert!(doc.execute("constraint.add", json!({ "object": "Goal", "type": "trackTo", "target": "Goal" })).is_err());
    assert!(doc.execute("constraint.add", json!({ "object": "Goal", "type": "ik", "target": "Camera" })).is_err());
    assert_eq!(doc.save(None).unwrap(), before);

    // Child Of keeps the cube where it is, then carries it.
    let start = world(&doc, "Cube");
    run(&mut doc, "constraint.add", json!({ "object": "Cube", "type": "childOf", "target": "Goal" }));
    let kept = world(&doc, "Cube");
    assert!(start.iter().zip(kept).all(|(a, b)| (a - b).abs() < 1e-4), "{start:?} {kept:?}");
    run(&mut doc, "object.transform", json!({ "objects": "Goal", "location": [3, 4, 2] }));
    assert!((world(&doc, "Cube")[2] - (start[2] + 2.0)).abs() < 1e-4);
    // Influence, then apply: the cube keeps the place the constraint gave it.
    run(&mut doc, "constraint.set", json!({ "object": "Cube", "index": 0, "influence": 0.5 }));
    assert!((world(&doc, "Cube")[2] - (start[2] + 1.0)).abs() < 1e-3);
    let carried = world(&doc, "Cube");
    run(&mut doc, "constraint.apply", json!({ "object": "Cube", "index": 0 }));
    assert!(object(&doc.inspect().unwrap(), "Cube").get("constraints").is_none());
    assert!(world(&doc, "Cube").iter().zip(carried).all(|(a, b)| (a - b).abs() < 1e-3));
    // Undo brings the constraint back; the file keeps the camera's.
    run(&mut doc, "edit.undo", json!({}));
    assert!(object(&doc.inspect().unwrap(), "Cube")["constraints"].is_array());
    let bytes = doc.save(None).unwrap();
    let back = SceneDocument::open("C.kscene", &bytes).unwrap();
    assert_eq!(object(&back.inspect().unwrap(), "Camera")["constraints"][0]["type"], "trackTo");
}

#[test]
fn ik_on_a_bone_through_commands() {
    let mut doc = SceneDocument::create("Ik.kscene", 1280, 720).unwrap();
    run(&mut doc, "add.armature", json!({}));
    run(&mut doc, "mode.armature", json!({}));
    run(&mut doc, "bone.select", json!({ "bones": "Bone" }));
    run(&mut doc, "bone.extrude", json!({ "tail": [0, 0.2, 2] }));
    run(&mut doc, "mode.armature", json!({}));
    run(&mut doc, "add.empty", json!({ "name": "Goal", "location": [0, 0.6, 1.4] }));
    run(&mut doc, "mode.pose", json!({ "object": "Armature" }));
    run(&mut doc, "bone.select", json!({ "bones": "Bone.001" }));
    let added = run(&mut doc, "constraint.add", json!({ "type": "ik", "target": "Goal", "chain": 2 }));
    assert_eq!(added["bone"], "Bone.001");
    let inspected = doc.inspect().unwrap();
    assert_eq!(object(&inspected, "Armature")["boneConstraints"]["Bone.001"][0]["type"], "ik");
    // An IK target inside its own chain is refused.
    assert!(
        doc.execute("constraint.add", json!({ "object": "Armature", "bone": "Bone", "type": "ik", "target": "Armature", "targetBone": "Bone.001" }))
            .is_err()
    );
    run(&mut doc, "mode.pose", json!({ "on": false }));
}

#[test]
fn weight_paint_paints_strokes_one_step_each() {
    let mut doc = SceneDocument::create("W.kscene", 1280, 720).unwrap();
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "mode.weightPaint", json!({}));
    assert_eq!(doc.inspect().unwrap()["mode"]["mode"], "weightPaint");
    run(&mut doc, "paint.group", json!({ "name": "Glow" }));
    run(&mut doc, "paint.brush", json!({ "radius": 400, "falloff": "constant", "weight": 0.75 }));
    // A stroke over the middle of the view, where the cube is.
    let (w, h) = (doc.width() as f64, doc.height() as f64);
    let event = |action, x: f64, y: f64| ToolPointer { action, x, y, mods: ToolMods::default(), pressure: 1.0, zoom: 1.0 };
    doc.pointer(&event(PointerAction::Down, w / 2.0, h / 2.0)).unwrap();
    doc.pointer(&event(PointerAction::Drag, w / 2.0 + 10.0, h / 2.0)).unwrap();
    doc.pointer(&event(PointerAction::Up, w / 2.0 + 10.0, h / 2.0)).unwrap();
    let inspected = doc.inspect().unwrap();
    let groups = object(&inspected, "Cube")["vertexGroups"].as_array().unwrap().clone();
    assert_eq!(groups[0]["name"], "Glow");
    assert_eq!(inspected["undo"], "Paint Weights");
    // The weights survive saving and reopening.
    let reopened = SceneDocument::open("W.kscene", &doc.save(None).unwrap()).unwrap();
    assert_eq!(object(&reopened.inspect().unwrap(), "Cube")["vertexGroups"], Value::Array(groups.clone()));
    // Agents set weights exactly, and normalise.
    run(&mut doc, "paint.vertices", json!({ "group": "Other", "vertices": [0, 1], "weight": 0.75 }));
    run(&mut doc, "paint.normalizeAll", json!({ "lockActive": false }));
    run(&mut doc, "edit.undo", json!({}));
    run(&mut doc, "edit.undo", json!({}));
    run(&mut doc, "edit.undo", json!({}));
    assert!(object(&doc.inspect().unwrap(), "Cube").get("vertexGroups").is_none_or(|groups| groups.as_array().unwrap().is_empty()));
    // Leaving the mode by another mode switch.
    run(&mut doc, "mode.set", json!({ "mode": "edit" }));
    assert_eq!(doc.inspect().unwrap()["mode"]["mode"], "edit");
}
