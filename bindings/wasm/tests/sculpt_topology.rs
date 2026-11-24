//! Sculpt mode's topology and visibility tools through commands and the pointer: multiresolution
//! levels (subdivide, sculpt a level, delete higher, apply base, reshape), dynamic topology, face
//! sets, hiding, and the box and lasso tools; each change one undo step, and all of it kept
//! through a save.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_kit::tools::{KitTools, PointerAction, ToolMods, ToolPointer};
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

/// The starter scene, its cube selected, in an 800 × 600 view from the top.
fn scene() -> SceneDocument {
    let mut made = SceneDocument::create("Untitled scene.kscene", 800, 600).unwrap();
    let mut doc = SceneDocument::open("t.kscene", &made.save_file(None).unwrap()).unwrap();
    run(&mut doc, "view.resize", json!({ "width": 800, "height": 600 }));
    doc
}

fn object(doc: &SceneDocument, name: &str) -> Value {
    doc.inspect().unwrap()["objects"].as_array().unwrap().iter().find(|object| object["name"] == name).unwrap().clone()
}

fn sculpt_state(doc: &SceneDocument) -> Value {
    doc.inspect().unwrap()["mode"]["sculpt"].clone()
}

fn reopen(doc: &mut SceneDocument) -> SceneDocument {
    SceneDocument::open("t.kscene", &doc.save_file(None).unwrap()).unwrap()
}

#[test]
fn multires_levels_subdivide_sculpt_and_save() {
    let mut doc = scene();
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    let first = run(&mut doc, "multires.subdivide", json!({}));
    assert_eq!(first["levels"], 1);
    run(&mut doc, "multires.subdivide", json!({}));
    let cube = object(&doc, "Cube");
    assert_eq!(cube["mesh"]["multiresLevels"], 2);
    assert_eq!(cube["mesh"]["evaluated"]["vertices"], 98, "{cube}");
    assert_eq!(cube["modifiers"][0]["type"], "multires");
    assert_eq!(cube["modifiers"][0]["sculptLevels"], 2);
    // Sculpting works on level 2: the base mesh keeps its 8 vertices.
    run(&mut doc, "mode.sculpt", json!({}));
    assert_eq!(sculpt_state(&doc)["level"], 2);
    let before = object(&doc, "Cube")["dimensions"].clone();
    let done =
        run(&mut doc, "sculpt.stroke", json!({ "points": [[0, 0, 1.0]], "worldRadius": 0.6, "strength": 1, "symmetry": [false, false, false] }));
    assert_eq!(done["level"], 2);
    assert!(done["touched"].as_u64().unwrap() > 5, "{done}");
    let after = object(&doc, "Cube");
    assert_eq!(after["mesh"]["vertices"], 8);
    assert!(after["dimensions"][2].as_f64().unwrap() > before[2].as_f64().unwrap() + 0.01, "{} vs {}", after["dimensions"], before);
    // The mask is painted at level 0.
    assert!(doc.execute("sculpt.stroke", json!({ "kind": "mask", "points": [[0, 0, 1.0]] })).is_err());
    // One stroke, one undo step.
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(object(&doc, "Cube")["dimensions"], before);
    run(&mut doc, "edit.redo", json!({}));
    let sculpted = object(&doc, "Cube")["dimensions"].clone();
    // Kept through a save.
    let reopened = reopen(&mut doc);
    let cube = object(&reopened, "Cube");
    assert_eq!(cube["mesh"]["multiresLevels"], 2);
    assert_eq!(cube["dimensions"], sculpted);
    // A lower sculpt level, then Delete Higher keeps it.
    run(&mut doc, "mode.sculpt", json!({ "on": false }));
    run(&mut doc, "modifier.set", json!({ "object": "Cube", "index": 0, "sculptLevels": 1 }));
    run(&mut doc, "multires.deleteHigher", json!({}));
    let cube = object(&doc, "Cube");
    assert_eq!(cube["mesh"]["multiresLevels"], 1);
    assert_eq!(cube["modifiers"][0]["levels"], 1);
    assert_eq!(cube["mesh"]["evaluated"]["vertices"], 26);
    // A second Multires modifier is refused.
    assert!(doc.execute("modifier.add", json!({ "object": "Cube", "type": "multires" })).is_err());
}

#[test]
fn apply_base_and_reshape_keep_the_shape() {
    let mut doc = scene();
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "multires.subdivide", json!({}));
    run(&mut doc, "mode.sculpt", json!({}));
    run(&mut doc, "sculpt.stroke", json!({ "kind": "inflate", "points": [[0, 0, 1.0], [0, 0, -1.0]], "worldRadius": 2, "strength": 1 }));
    run(&mut doc, "mode.sculpt", json!({ "on": false }));
    let shaped = object(&doc, "Cube")["dimensions"].clone();
    run(&mut doc, "multires.applyBase", json!({}));
    let cube = object(&doc, "Cube");
    for axis in 0..3 {
        let (a, b) = (cube["dimensions"][axis].as_f64().unwrap(), shaped[axis].as_f64().unwrap());
        assert!((a - b).abs() < 1e-3, "{a} vs {b}");
    }
    // Reshape from a copy with the level's vertices: the cube applied at level 1.
    run(&mut doc, "edit.duplicate", json!({ "objects": "Cube" }));
    let copy = doc.inspect().unwrap()["selection"]["active"].clone();
    run(&mut doc, "modifier.apply", json!({ "object": copy, "index": 0 }));
    let copy_object = doc.inspect().unwrap()["objects"].as_array().unwrap().iter().find(|object| object["id"] == copy).unwrap().clone();
    assert_eq!(copy_object["mesh"]["vertices"], 26);
    let reshaped = run(&mut doc, "multires.reshape", json!({ "object": "Cube", "from": copy }));
    assert_eq!(reshaped["levels"], 1);
    let cube = object(&doc, "Cube");
    for axis in 0..3 {
        let (a, b) = (cube["dimensions"][axis].as_f64().unwrap(), shaped[axis].as_f64().unwrap());
        assert!((a - b).abs() < 1e-3, "reshaped from the same shape: {a} vs {b}");
    }
    assert!(doc.execute("multires.reshape", json!({ "object": "Cube", "from": "Light" })).is_err());
}

#[test]
fn dynamic_topology_refines_under_strokes() {
    let mut doc = scene();
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "mode.sculpt", json!({}));
    let on = run(&mut doc, "sculpt.dyntopo", json!({ "enabled": true, "mode": "brush", "percent": 10 }));
    assert_eq!(on["triangulated"], true);
    assert_eq!(object(&doc, "Cube")["mesh"]["faces"], 12);
    assert_eq!(sculpt_state(&doc)["dyntopo"]["enabled"], true);
    let done = run(&mut doc, "sculpt.stroke", json!({ "points": [[0, 0, 1.0], [0.4, 0, 1.0]], "worldRadius": 0.5, "strength": 1 }));
    assert!(done["split"].as_u64().unwrap() > 20, "{done}");
    let faces = object(&doc, "Cube")["mesh"]["faces"].as_u64().unwrap();
    assert!(faces > 60, "{faces}");
    // One stroke, one undo step (its new faces too).
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(object(&doc, "Cube")["mesh"]["faces"], 12);
    run(&mut doc, "edit.redo", json!({}));
    // Detail flood fill at a constant resolution.
    run(&mut doc, "sculpt.dyntopo", json!({ "mode": "constant", "resolution": 4 }));
    let flood = run(&mut doc, "sculpt.detailFlood", json!({}));
    assert!(flood["faces"].as_u64().unwrap() > 100, "{flood}");
    assert_eq!(
        reopen(&mut doc).inspect().unwrap()["objects"].as_array().unwrap().iter().find(|object| object["name"] == "Cube").unwrap()["mesh"]["faces"],
        flood["faces"]
    );
    // Not with multiresolution levels.
    assert!(doc.execute("multires.subdivide", json!({})).is_err(), "dynamic topology is on");
    run(&mut doc, "sculpt.dyntopo", json!({ "enabled": false }));
    assert!(doc.execute("sculpt.dyntopo", json!({ "detailSize": 0 })).is_err());
}

#[test]
fn face_sets_hiding_and_gestures() {
    let mut doc = scene();
    run(&mut doc, "edit.delete", json!({ "objects": "Cube" }));
    run(&mut doc, "add.grid", json!({ "name": "Clay", "xSubdivisions": 20, "ySubdivisions": 20, "size": 2 }));
    run(&mut doc, "view.top", json!({}));
    run(&mut doc, "view.frameSelected", json!({}));
    run(&mut doc, "mode.sculpt", json!({}));
    // A box over the left half of the view masks the left of the grid.
    let masked = run(&mut doc, "sculpt.gesture", json!({ "action": "mask", "points": [[0, 0], [400, 600]] }));
    assert!(masked["changed"].as_u64().unwrap() > 150, "{masked}");
    let from_mask = run(&mut doc, "sculpt.faceSetsFromMask", json!({}));
    assert_eq!(from_mask["faceSet"], 1);
    // H over the left half hides that set (where the pointer last moved); Alt+H shows everything
    // again.
    let hover = ToolPointer { action: PointerAction::Move, x: 200.0, y: 300.0, mods: ToolMods::default(), pressure: 1.0, zoom: 1.0 };
    doc.pointer(&hover).unwrap();
    run(&mut doc, "sculpt.hideFaceSet", json!({}));
    let hidden = object(&doc, "Clay")["mesh"]["hiddenFaces"].as_u64().unwrap();
    assert_eq!(hidden, from_mask["faces"].as_u64().unwrap());
    // Hidden faces are not sculpted.
    let before = object(&doc, "Clay")["dimensions"].clone();
    run(&mut doc, "sculpt.maskClear", json!({}));
    run(&mut doc, "sculpt.stroke", json!({ "points": [[-0.6, 0, 0]], "worldRadius": 0.2, "strength": 1, "symmetry": [false, false, false] }));
    assert_eq!(object(&doc, "Clay")["dimensions"], before);
    // Kept through a save.
    let reopened = reopen(&mut doc);
    assert_eq!(object(&reopened, "Clay")["mesh"]["hiddenFaces"].as_u64().unwrap(), hidden);
    assert_eq!(object(&reopened, "Clay")["mesh"]["faceSets"], 1);
    let mut doc = doc;
    run(&mut doc, "sculpt.showAllFaces", json!({}));
    assert!(object(&doc, "Clay")["mesh"].get("hiddenFaces").is_none());
    // The lasso tool through the pointer: a triangle on the right half makes a face set.
    run(&mut doc, "sculpt.tool", json!({ "tool": "lassoFaceSet" }));
    let event = |action, x: f64, y: f64| ToolPointer { action, x, y, mods: ToolMods::default(), pressure: 1.0, zoom: 1.0 };
    doc.pointer(&event(PointerAction::Down, 450.0, 150.0)).unwrap();
    doc.pointer(&event(PointerAction::Drag, 780.0, 300.0)).unwrap();
    doc.pointer(&event(PointerAction::Drag, 450.0, 450.0)).unwrap();
    doc.pointer(&event(PointerAction::Up, 450.0, 450.0)).unwrap();
    assert_eq!(sculpt_state(&doc)["faceSets"], json!([1, 2]));
    // The box hide tool, then Cmd to show again.
    run(&mut doc, "sculpt.tool", json!({ "tool": "boxHide" }));
    doc.pointer(&event(PointerAction::Down, 0.0, 0.0)).unwrap();
    doc.pointer(&event(PointerAction::Drag, 800.0, 300.0)).unwrap();
    doc.pointer(&event(PointerAction::Up, 800.0, 300.0)).unwrap();
    assert!(sculpt_state(&doc)["hiddenFaces"].as_u64().unwrap() > 100);
    let show = |action| ToolPointer { mods: ToolMods { command: true, ..ToolMods::default() }, ..event(action, 0.0, 0.0) };
    doc.pointer(&show(PointerAction::Down)).unwrap();
    doc.pointer(&ToolPointer { x: 800.0, y: 600.0, ..show(PointerAction::Drag) }).unwrap();
    doc.pointer(&ToolPointer { x: 800.0, y: 600.0, ..show(PointerAction::Up) }).unwrap();
    assert_eq!(sculpt_state(&doc)["hiddenFaces"], 0);
    // Draw Face Sets paints a third set; loose parts make one; clear drops them.
    run(&mut doc, "sculpt.stroke", json!({ "kind": "drawFaceSets", "points": [[0, 0.5, 0]], "worldRadius": 0.3 }));
    assert_eq!(sculpt_state(&doc)["faceSets"], json!([1, 2, 3]));
    run(&mut doc, "sculpt.faceSetsFromLooseParts", json!({}));
    assert_eq!(sculpt_state(&doc)["faceSets"], json!([1]));
    run(&mut doc, "sculpt.faceSetsClear", json!({}));
    assert_eq!(sculpt_state(&doc)["faceSets"], json!([]));
    // Bad gestures change nothing.
    for params in [
        json!({ "action": "mask", "points": [[0, 0]] }),
        json!({ "action": "melt", "points": [[0, 0], [1, 1]] }),
        json!({ "action": "mask", "shape": "lasso", "points": [[0, 0], [1, 1]] }),
    ] {
        assert!(doc.execute("sculpt.gesture", params.clone()).is_err(), "{params}");
    }
}

#[test]
fn the_gpu_viewport_draws_face_sets_and_hiding() {
    let mut doc = scene();
    run(&mut doc, "edit.delete", json!({ "objects": "Cube" }));
    run(&mut doc, "add.grid", json!({ "name": "Clay", "xSubdivisions": 20, "ySubdivisions": 20, "size": 2 }));
    run(&mut doc, "view.top", json!({}));
    run(&mut doc, "view.frameSelected", json!({}));
    run(&mut doc, "mode.sculpt", json!({}));
    run(&mut doc, "sculpt.gesture", json!({ "action": "faceSet", "points": [[400, 0], [800, 600]] }));
    run(&mut doc, "sculpt.gesture", json!({ "action": "hide", "points": [[0, 0], [300, 600]] }));
    let frame = doc.gl_scene();
    assert_eq!(frame["supported"], true);
    assert_eq!(frame["objects"][0]["paint"], 1);
}
