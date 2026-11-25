//! Envelopes, the Armature modifier's options, custom bone shapes, X-mirror painting, and the
//! Gradient and Sample Weight tools through commands and the pointer, as the agent and the
//! editor drive them.
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

fn event(action: PointerAction, x: f64, y: f64) -> ToolPointer {
    ToolPointer { action, x, y, mods: ToolMods::default(), pressure: 1.0, zoom: 1.0 }
}

/// The cube's dimensions as inspected (after its modifiers).
fn dimensions(doc: &SceneDocument) -> Vec<f64> {
    object(&doc.inspect().unwrap(), "Cube")["dimensions"].as_array().unwrap().iter().map(|value| value.as_f64().unwrap()).collect()
}

#[test]
fn envelopes_deform_a_mesh_without_groups() {
    let mut doc = SceneDocument::create("E.kscene", 1280, 720).unwrap();
    run(&mut doc, "object.addArmature", json!({ "location": [0, 0, -1] }));
    run(
        &mut doc,
        "bone.set",
        json!({ "bone": "Bone", "tail": [0, 0, 2], "headRadius": 0.5, "tailRadius": 0.5, "envelopeDistance": 1.0, "envelopeWeight": 0.8 }),
    );
    let inspected = doc.inspect().unwrap();
    let bones = &object(&inspected, "Armature")["armature"]["bones"][0];
    assert_eq!(bones["envelope"], json!({ "headRadius": 0.5, "tailRadius": 0.5, "distance": 1.0, "weight": 0.8, "multiply": false }));
    assert!(doc.execute("bone.set", json!({ "bone": "Bone", "envelopeWeight": -1 })).is_err(), "weights are not negative");
    run(&mut doc, "select.objects", json!({ "objects": ["Cube", "Armature"] }));
    run(&mut doc, "object.parentArmature", json!({ "objects": ["Cube"], "armature": "Armature", "weights": "none" }));
    run(&mut doc, "modifier.set", json!({ "object": "Cube", "index": 0, "envelopes": true, "vertexGroups": false }));
    let inspected = doc.inspect().unwrap();
    let modifier = &object(&inspected, "Cube")["modifiers"][0];
    assert_eq!((modifier["envelopes"].clone(), modifier["vertexGroups"].clone()), (json!(true), json!(false)));
    let rest = dimensions(&doc);
    run(&mut doc, "mode.pose", json!({ "object": "Armature", "on": true }));
    run(&mut doc, "pose.set", json!({ "bones": "Bone", "scale": [2, 1, 1] }));
    let posed = dimensions(&doc);
    assert!(posed[0] > rest[0] * 1.5, "the envelope stretches the cube: {rest:?} to {posed:?}");
    run(&mut doc, "mode.pose", json!({ "on": false }));
    // Its own vertex group: a missing one lets every vertex move; an inverted full group none.
    run(&mut doc, "modifier.set", json!({ "object": "Cube", "index": 0, "vertexGroup": "Mask" }));
    assert_eq!(dimensions(&doc), posed);
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "paint.vertices", json!({ "object": "Cube", "group": "Mask", "vertices": [0, 1, 2, 3, 4, 5, 6, 7], "weight": 1.0 }));
    run(&mut doc, "modifier.set", json!({ "object": "Cube", "index": 0, "invertVertexGroup": true }));
    assert_eq!(dimensions(&doc), rest);
    // Renaming the bone keeps the modifier's group, which is not the bone's.
    run(&mut doc, "bone.set", json!({ "object": "Armature", "bone": "Bone", "name": "Spine" }));
    assert_eq!(object(&doc.inspect().unwrap(), "Cube")["modifiers"][0]["vertexGroup"], "Mask");
    run(&mut doc, "edit.undo", json!({}));
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(dimensions(&doc), posed, "each change was one undo step");
}

#[test]
fn custom_shapes_draw_bones_and_let_go_of_deleted_objects() {
    let mut doc = SceneDocument::create("S.kscene", 640, 360).unwrap();
    run(&mut doc, "object.addArmature", json!({ "location": [0, 0, 1.2] }));
    let before = doc.render(0.0, 0.0, 1.0, 640, 360).unwrap();
    assert!(doc.execute("bone.set", json!({ "object": "Armature", "bone": "Bone", "shape": "Armature" })).is_err(), "not its own shape");
    assert!(doc.execute("bone.set", json!({ "object": "Armature", "bone": "Bone", "shapeScale": [2, 2, 2] })).is_err(), "no shape yet");
    run(
        &mut doc,
        "bone.set",
        json!({ "object": "Armature", "bone": "Bone", "shape": "Cube", "shapeTranslation": [0, 0.5, 0], "shapeRotation": [0, 45, 0], "shapeScale": [0.3, 0.3, 0.3] }),
    );
    let inspected = doc.inspect().unwrap();
    let shape = &object(&inspected, "Armature")["armature"]["bones"][0]["shape"];
    assert_eq!(shape["name"], "Cube");
    assert_eq!(shape["rotation"], json!([0.0, 45.0, 0.0]));
    assert_eq!((shape["boneSize"].clone(), shape["wire"].clone()), (json!(true), json!(false)));
    let shaped = doc.render(0.0, 0.0, 1.0, 640, 360).unwrap();
    assert_ne!(before, shaped, "the shape draws the bone");
    run(&mut doc, "bone.set", json!({ "object": "Armature", "bone": "Bone", "shapeWire": true }));
    let wire = doc.render(0.0, 0.0, 1.0, 640, 360).unwrap();
    assert_ne!(shaped, wire, "a wire shape leaves its faces out");
    // Saved and reopened, the shape stays; deleting its object gives the octahedron back.
    let reopened = SceneDocument::open("S.kscene", &doc.save_file(None).unwrap()).unwrap();
    assert_eq!(object(&reopened.inspect().unwrap(), "Armature")["armature"]["bones"][0]["shape"]["wire"], true);
    run(&mut doc, "edit.delete", json!({ "objects": "Cube" }));
    assert!(object(&doc.inspect().unwrap(), "Armature")["armature"]["bones"][0]["shape"].is_null());
    run(&mut doc, "bone.set", json!({ "object": "Armature", "bone": "Bone", "shape": null }));
}

#[test]
fn x_mirror_paints_the_other_side_in_the_mirrored_group() {
    let mut doc = SceneDocument::create("M.kscene", 1280, 720).unwrap();
    run(&mut doc, "view.front", json!({}));
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "mode.weightPaint", json!({}));
    run(&mut doc, "paint.group", json!({ "name": "Side.L" }));
    let brush = run(&mut doc, "paint.brush", json!({ "radius": 120, "falloff": "constant", "weight": 0.6, "xMirror": true }));
    assert_eq!((brush["xMirror"].clone(), brush["mirrorGroups"].clone()), (json!(true), json!(true)));
    // A dab on the cube's right half paints its left half in Side.R.
    let (w, h) = (doc.width() as f64, doc.height() as f64);
    run(&mut doc, "paint.stroke", json!({ "points": [[w / 2.0 + 150.0, h / 2.0]] }));
    let inspected = doc.inspect().unwrap();
    let groups = object(&inspected, "Cube")["vertexGroups"].as_array().unwrap().clone();
    let count = |name: &str| groups.iter().find(|group| group["name"] == name).map_or(0, |group| group["vertices"].as_u64().unwrap());
    assert!(count("Side.L") > 0 && count("Side.L") == count("Side.R"), "{groups:?}");
    assert_eq!(inspected["mode"]["paint"]["mirror"], json!({ "x": true, "topology": false, "groups": true }));
    run(&mut doc, "edit.undo", json!({}));
    assert!(object(&doc.inspect().unwrap(), "Cube").get("vertexGroups").is_none_or(|groups| groups.as_array().unwrap().is_empty()));
}

#[test]
fn the_gradient_and_sample_weight_tools() {
    let mut doc = SceneDocument::create("G.kscene", 1280, 720).unwrap();
    run(&mut doc, "view.front", json!({}));
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "mode.weightPaint", json!({}));
    run(&mut doc, "paint.group", json!({ "name": "Fade" }));
    let (w, h) = (doc.width() as f64, doc.height() as f64);
    // A command fills the gradient across the cube, left (full) to right (none).
    run(&mut doc, "paint.gradient", json!({ "from": [w / 2.0 - 50.0, h / 2.0], "to": [w / 2.0 + 50.0, h / 2.0] }));
    let inspected = doc.inspect().unwrap();
    assert_eq!(inspected["undo"], "Gradient");
    let painted = object(&inspected, "Cube")["vertexGroups"][0]["vertices"].as_u64().unwrap();
    assert!(painted > 0 && painted < 8, "the right side stays empty ({painted} of 8)");
    run(&mut doc, "edit.undo", json!({}));
    // The pointer: a radial gradient dragged out from the middle, one undo step.
    run(&mut doc, "paint.tool", json!({ "tool": "radialGradient" }));
    assert_eq!(doc.inspect().unwrap()["mode"]["paint"]["tool"], "radialGradient");
    doc.pointer(&event(PointerAction::Down, w / 2.0, h / 2.0)).unwrap();
    doc.pointer(&event(PointerAction::Drag, w / 2.0 + 100.0, h / 2.0)).unwrap();
    doc.pointer(&event(PointerAction::Drag, w / 2.0 + 600.0, h / 2.0)).unwrap();
    doc.pointer(&event(PointerAction::Up, w / 2.0 + 600.0, h / 2.0)).unwrap();
    let inspected = doc.inspect().unwrap();
    assert_eq!(inspected["undo"], "Gradient");
    assert_eq!(object(&inspected, "Cube")["vertexGroups"][0]["vertices"], 8, "every corner within reach");
    // Sample Weight takes the brush's weight from under the pointer.
    run(&mut doc, "paint.vertices", json!({ "group": "Fade", "vertices": [0, 1, 2, 3, 4, 5, 6, 7], "weight": 0.35 }));
    let sampled = run(&mut doc, "paint.sample", json!({ "at": [w / 2.0, h / 2.0] }))["weight"].as_f64().unwrap();
    assert!((sampled - 0.35).abs() < 1e-6, "{sampled}");
    run(&mut doc, "paint.brush", json!({ "weight": 1.0 }));
    run(&mut doc, "paint.tool", json!({ "tool": "sampleWeight" }));
    doc.pointer(&event(PointerAction::Down, w / 2.0, h / 2.0)).unwrap();
    doc.pointer(&event(PointerAction::Up, w / 2.0, h / 2.0)).unwrap();
    let brush = &doc.inspect().unwrap()["mode"]["paint"]["brush"];
    assert!((brush["weight"].as_f64().unwrap() - 0.35).abs() < 1e-6, "{brush}");
    assert!(doc.execute("paint.sample", json!({ "at": [2.0, 2.0] })).is_err(), "nothing to sample in the corner");
}
