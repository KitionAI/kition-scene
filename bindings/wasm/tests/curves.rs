//! Curves and texts through commands, as the agent and the editor's menus drive them: added,
//! set, edited in their edit modes (points through commands and the pointer, letters through
//! typed text and keys), converted to meshes, exported, and kept by the file.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_kit::tools::{KitTools, PointerAction, ToolMods, ToolPointer};
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn object(doc: &SceneDocument, name: &str) -> Value {
    let inspected = doc.inspect().unwrap();
    inspected["objects"].as_array().unwrap().iter().find(|object| object["name"] == name).cloned().unwrap_or_else(|| panic!("no {name}"))
}

fn evaluated(doc: &SceneDocument, name: &str) -> (u64, u64) {
    let found = object(doc, name);
    let data = if found["curve"].is_object() { &found["curve"] } else { &found["text"] };
    (data["evaluated"]["vertices"].as_u64().unwrap(), data["evaluated"]["faces"].as_u64().unwrap())
}

#[test]
fn curves_are_added_set_and_kept_by_the_file() {
    let mut doc = SceneDocument::create("Curves.kscene", 1280, 720).unwrap();
    run(&mut doc, "object.addCurveBezier", json!({ "location": [0, 0, 1] }));
    run(&mut doc, "object.addCurveCircle", json!({ "radius": 2 }));
    run(&mut doc, "object.addCurvePath", json!({}));
    assert_eq!(object(&doc, "BezierCurve")["type"], "curve");
    assert_eq!(evaluated(&doc, "BezierCurve"), (13, 0));
    // The reference's bevelled circle: 576 vertices and faces.
    run(&mut doc, "curve.set", json!({ "object": "BezierCircle", "bevelDepth": 0.1 }));
    assert_eq!(evaluated(&doc, "BezierCircle"), (576, 576));
    run(&mut doc, "curve.set", json!({ "object": "BezierCircle", "dimensions": "2d", "bevelDepth": 0 }));
    assert_eq!(object(&doc, "BezierCircle")["curve"]["fillMode"], "both");
    assert!(evaluated(&doc, "BezierCircle").1 > 0, "a closed 2D curve fills");
    assert!(doc.execute("curve.set", json!({ "object": "BezierCircle", "resolution": 0 })).is_err());
    assert!(doc.execute("curve.set", json!({ "object": "Cube", "bevelDepth": 0.1 })).is_err(), "a mesh is no curve");
    // Agents can describe splines outright.
    run(
        &mut doc,
        "curve.setSplines",
        json!({ "object": "NurbsPath", "splines": [{ "type": "poly", "points": [{ "co": [0, 0, 0] }, { "co": [1, 0, 0] }, { "co": [1, 1, 0] }] }] }),
    );
    assert_eq!(evaluated(&doc, "NurbsPath"), (3, 0));

    let bytes = doc.save_file(None).unwrap();
    let mut back = SceneDocument::open("Curves.kscene", &bytes).unwrap();
    assert_eq!(back.save_file(None).unwrap(), bytes);
    assert_eq!(evaluated(&back, "NurbsPath"), (3, 0));
    assert_eq!(object(&back, "BezierCircle")["curve"], object(&doc, "BezierCircle")["curve"]);
    // Exports bake them to meshes.
    run(&mut back, "select.objects", json!({ "objects": "BezierCircle" }));
    for format in ["obj", "stl", "glb"] {
        let exported = run(&mut back, "file.export", json!({ "format": format, "selectedOnly": true }));
        assert!(exported["base64"].as_str().unwrap().len() > 2000, "{format} holds the filled circle");
    }
}

#[test]
fn curve_edit_mode_extrudes_subdivides_and_deletes_points() {
    let mut doc = SceneDocument::create("Edit.kscene", 1280, 720).unwrap();
    run(&mut doc, "object.addCurveBezier", json!({}));
    run(&mut doc, "mode.shape", json!({}));
    assert_eq!(doc.inspect().unwrap()["mode"]["mode"], "curveEdit");
    assert!(doc.execute("object.addCube", json!({})).is_err(), "objects wait for object mode");
    // Extrude the end point: a third point continues the spline.
    run(&mut doc, "curve.select", json!({ "points": [1] }));
    run(&mut doc, "curve.extrude", json!({}));
    let curve = object(&doc, "BezierCurve")["curve"].clone();
    assert_eq!(curve["splines"][0]["points"].as_array().unwrap().len(), 3);
    assert_eq!(doc.inspect().unwrap()["mode"]["points"], json!([2]));
    run(&mut doc, "curve.translate", json!({ "offset": [0, 1, 0] }));
    let moved = object(&doc, "BezierCurve")["curve"]["splines"][0]["points"][2]["co"].clone();
    assert!((moved[1].as_f64().unwrap() - 1.0).abs() < 1e-5, "{moved}");
    // Select all, subdivide both segments, then toggle cyclic.
    run(&mut doc, "edit.selectAll", json!({}));
    run(&mut doc, "curve.subdivide", json!({}));
    assert_eq!(object(&doc, "BezierCurve")["curve"]["splines"][0]["points"].as_array().unwrap().len(), 5);
    run(&mut doc, "curve.toggleCyclic", json!({}));
    assert_eq!(object(&doc, "BezierCurve")["curve"]["splines"][0]["cyclic"], true);
    run(&mut doc, "curve.select", json!({ "points": [0, 1] }));
    run(&mut doc, "curve.setHandleType", json!({ "type": "vector" }));
    assert_eq!(object(&doc, "BezierCurve")["curve"]["splines"][0]["points"][0]["leftType"], "vector");
    // Delete takes points in edit mode, not the object.
    run(&mut doc, "edit.delete", json!({}));
    assert_eq!(object(&doc, "BezierCurve")["curve"]["splines"][0]["points"].as_array().unwrap().len(), 3);
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(object(&doc, "BezierCurve")["curve"]["splines"][0]["points"].as_array().unwrap().len(), 5);
    run(&mut doc, "mode.shape", json!({}));
    assert_eq!(doc.inspect().unwrap()["mode"]["mode"], "object");
}

#[test]
fn the_move_tool_drags_a_curve_point_in_one_step() {
    let mut doc = SceneDocument::create("Drag.kscene", 1280, 720).unwrap();
    run(&mut doc, "edit.delete", json!({ "objects": "Cube" }));
    run(&mut doc, "object.addCurveBezier", json!({}));
    run(&mut doc, "view.resize", json!({ "width": 640, "height": 480 }));
    run(&mut doc, "view.top", json!({}));
    run(&mut doc, "view.frameAll", json!({}));
    run(&mut doc, "mode.shape", json!({}));
    run(&mut doc, "curve.select", json!({ "points": [0] }));
    let before = object(&doc, "BezierCurve")["curve"]["splines"][0]["points"][0]["co"].clone();
    doc.select_tool("move", 1.0).unwrap();
    let event = |action, x: f64, y: f64| ToolPointer { action, x, y, mods: ToolMods::default(), pressure: 1.0, zoom: 1.0 };
    doc.pointer(&event(PointerAction::Down, 300.0, 240.0)).unwrap();
    doc.pointer(&event(PointerAction::Drag, 340.0, 200.0)).unwrap();
    doc.pointer(&event(PointerAction::Up, 340.0, 200.0)).unwrap();
    let after = object(&doc, "BezierCurve")["curve"]["splines"][0]["points"][0]["co"].clone();
    assert_ne!(before, after);
    assert!(after[0].as_f64().unwrap() > before[0].as_f64().unwrap(), "{before} → {after}");
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(object(&doc, "BezierCurve")["curve"]["splines"][0]["points"][0]["co"], before);
    // A click with the select tool picks the nearest point.
    doc.select_tool("select", 1.0).unwrap();
    run(&mut doc, "curve.select", json!({ "points": [] }));
    doc.pointer(&event(PointerAction::Down, 2.0, 2.0)).unwrap();
    doc.pointer(&event(PointerAction::Up, 638.0, 478.0)).unwrap();
    assert_eq!(doc.inspect().unwrap()["mode"]["points"], json!([0, 1]), "a box takes every point inside");
}

#[test]
fn texts_are_typed_into_and_converted() {
    let mut doc = SceneDocument::create("Text.kscene", 1280, 720).unwrap();
    run(&mut doc, "object.addText", json!({}));
    assert_eq!(object(&doc, "Text")["text"]["body"], "Text");
    run(&mut doc, "text.set", json!({ "extrude": 0.1, "align": "center" }));
    let (vertices, faces) = evaluated(&doc, "Text");
    assert!(vertices > 900 && faces > 0, "{vertices} {faces}");
    // Text edit mode takes typed text and editing keys.
    run(&mut doc, "mode.shape", json!({}));
    assert_eq!(doc.inspect().unwrap()["mode"]["mode"], "textEdit");
    assert!(doc.state(0.0, 0.0, ToolMods::default(), 1.0).wants_text);
    KitTools::text(&mut doc, "!", 1.0).unwrap();
    doc.key("Enter", ToolMods::default(), 1.0).unwrap();
    KitTools::text(&mut doc, "Hi", 1.0).unwrap();
    assert_eq!(object(&doc, "Text")["text"]["body"], "Text!\nHi");
    doc.key("ArrowUp", ToolMods::default(), 1.0).unwrap();
    doc.key("Backspace", ToolMods::default(), 1.0).unwrap();
    assert_eq!(object(&doc, "Text")["text"]["body"], "Txt!\nHi");
    run(&mut doc, "text.caret", json!({ "place": "end" }));
    run(&mut doc, "text.delete", json!({ "count": 3 }));
    assert_eq!(object(&doc, "Text")["text"]["body"], "Txt!");
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(object(&doc, "Text")["text"]["body"], "Txt!\nHi");
    doc.key("Escape", ToolMods::default(), 1.0).unwrap();
    assert_eq!(doc.inspect().unwrap()["mode"]["mode"], "object");
    assert!(!doc.state(0.0, 0.0, ToolMods::default(), 1.0).wants_text);
    // An agent types without the edit mode.
    run(&mut doc, "text.insert", json!({ "object": "Text", "text": "!" }));
    assert_eq!(object(&doc, "Text")["text"]["body"], "Txt!\nHi!");

    let bytes = doc.save_file(None).unwrap();
    let back = SceneDocument::open("Text.kscene", &bytes).unwrap();
    assert_eq!(object(&back, "Text")["text"], object(&doc, "Text")["text"]);

    // Convert to mesh keeps the shape.
    let before = evaluated(&doc, "Text");
    run(&mut doc, "select.objects", json!({ "objects": "Text" }));
    run(&mut doc, "object.convertToMesh", json!({}));
    let converted = object(&doc, "Text");
    assert_eq!(converted["type"], "mesh");
    assert_eq!(converted["mesh"]["vertices"].as_u64().unwrap(), before.0);
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(object(&doc, "Text")["type"], "text");
}
