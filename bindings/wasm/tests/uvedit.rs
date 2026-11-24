//! UV editing, the knife, and edge slide, through commands.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn editing_cube() -> SceneDocument {
    let mut doc = SceneDocument::create("Untitled.kscene", 400, 300).unwrap();
    let mut doc = SceneDocument::open("t.kscene", &doc.save_file(None).unwrap()).unwrap();
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "mode.set", json!({ "mode": "edit" }));
    doc
}

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn mesh(doc: &SceneDocument) -> Value {
    doc.inspect().unwrap()["objects"][0]["mesh"].clone()
}

#[test]
fn seams_unwrap_and_pins() {
    let mut doc = editing_cube();
    // Every edge a seam: six islands.
    let marked = run(&mut doc, "mesh.markSeam", json!({}));
    assert_eq!(marked["seams"], 12);
    let done = run(&mut doc, "uv.unwrap", json!({ "margin": 0.05 }));
    assert_eq!(done["charts"], 6);
    let layout = run(&mut doc, "uv.layout", json!({}));
    assert_eq!(layout["faces"].as_array().unwrap().len(), 6);
    assert_eq!(layout["seams"].as_array().unwrap().len(), 12);
    for uv in layout["faces"].as_array().unwrap().iter().flat_map(|face| face["uvs"].as_array().unwrap().clone()) {
        let (u, v) = (uv[0].as_f64().unwrap(), uv[1].as_f64().unwrap());
        assert!((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v));
    }
    run(&mut doc, "mesh.clearSeam", json!({}));
    assert_eq!(run(&mut doc, "uv.layout", json!({}))["seams"].as_array().unwrap().len(), 0);
    run(&mut doc, "mesh.select", json!({ "vertices": [0, 1] }));
    run(&mut doc, "uv.pin", json!({}));
    assert_eq!(run(&mut doc, "uv.layout", json!({ "all": true }))["pinned"], json!([0, 1]));
    run(&mut doc, "mesh.select", json!({ "vertices": [0] }));
    run(&mut doc, "uv.unpin", json!({}));
    assert_eq!(run(&mut doc, "uv.layout", json!({ "all": true }))["pinned"], json!([1]));
    // The file keeps seams and pins.
    run(&mut doc, "mesh.select", json!({ "vertices": [0, 1, 2, 3] }));
    run(&mut doc, "mesh.markSeam", json!({}));
    let mut reopened = SceneDocument::open("t.kscene", &doc.save_file(None).unwrap()).unwrap();
    run(&mut reopened, "select.objects", json!({ "objects": "Cube" }));
    run(&mut reopened, "mode.set", json!({ "mode": "edit" }));
    let layout = run(&mut reopened, "uv.layout", json!({ "all": true }));
    assert_eq!(layout["seams"].as_array().unwrap().len(), 4);
    assert_eq!(layout["pinned"], json!([1]));
}

#[test]
fn moving_turning_and_scaling_coordinates() {
    let mut doc = editing_cube();
    run(&mut doc, "mesh.select", json!({ "faces": [5] }));
    let before = run(&mut doc, "uv.layout", json!({}))["faces"][0]["uvs"].clone();
    run(&mut doc, "uv.translate", json!({ "offset": [0.1, -0.2] }));
    let after = run(&mut doc, "uv.layout", json!({}))["faces"][0]["uvs"].clone();
    for (old, new) in before.as_array().unwrap().iter().zip(after.as_array().unwrap()) {
        assert!((new[0].as_f64().unwrap() - old[0].as_f64().unwrap() - 0.1).abs() < 1e-5);
        assert!((new[1].as_f64().unwrap() - old[1].as_f64().unwrap() + 0.2).abs() < 1e-5);
    }
    // A half turn about the island's centre, twice, comes back.
    run(&mut doc, "uv.rotate", json!({ "angle": 180 }));
    run(&mut doc, "uv.rotate", json!({ "angle": 180 }));
    run(&mut doc, "uv.scale", json!({ "factor": 2 }));
    run(&mut doc, "uv.scale", json!({ "factor": [0.5, 0.5] }));
    let back = run(&mut doc, "uv.layout", json!({}))["faces"][0]["uvs"].clone();
    for (a, b) in after.as_array().unwrap().iter().zip(back.as_array().unwrap()) {
        assert!((a[0].as_f64().unwrap() - b[0].as_f64().unwrap()).abs() < 1e-4 && (a[1].as_f64().unwrap() - b[1].as_f64().unwrap()).abs() < 1e-4);
    }
    // Every step undoes.
    for _ in 0..5 {
        run(&mut doc, "edit.undo", json!({}));
    }
    assert_eq!(run(&mut doc, "uv.layout", json!({}))["faces"][0]["uvs"], before);
    assert!(doc.execute("uv.translate", json!({ "offset": [1] })).is_err());
}

#[test]
fn the_knife_cuts_across_the_top() {
    let mut doc = editing_cube();
    run(&mut doc, "view.top", json!({}));
    run(&mut doc, "select.all", json!({ "action": "deselect" }));
    let cut = run(&mut doc, "mesh.knife", json!({ "points": [[-2, 0, 1], [2, 0, 1]] }));
    assert_eq!(cut["vertices"].as_array().unwrap().len(), 2);
    let after = mesh(&doc);
    assert_eq!((after["vertices"].as_u64(), after["faces"].as_u64()), (Some(10), Some(7)));
    assert!(doc.check().is_empty(), "{:?}", doc.check());
    // A line that misses the cube cuts nothing and changes nothing.
    assert!(doc.execute("mesh.knife", json!({ "points": [[5, 5, 1], [6, 5, 1]] })).is_err());
    assert_eq!(mesh(&doc)["faces"], 7);
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(mesh(&doc)["faces"], 6);
    // Through the cube: the bottom is cut too.
    run(&mut doc, "mesh.knife", json!({ "points": [[-2, 0, 1], [2, 0, 1]], "through": true }));
    assert_eq!(mesh(&doc)["faces"], 10);
}

#[test]
fn edge_slide_moves_a_loop() {
    let mut doc = editing_cube();
    let cut = run(&mut doc, "mesh.loopCut", json!({ "edge": [0, 1] }));
    let ring = cut["vertices"].clone();
    run(&mut doc, "mesh.select", json!({ "vertices": ring }));
    let slid = run(&mut doc, "mesh.edgeSlide", json!({ "factor": 0.5 }));
    assert_eq!(slid["moved"], 4);
    assert!(doc.execute("mesh.edgeSlide", json!({ "factor": 3 })).is_err());
    assert!(doc.check().is_empty());
}

#[test]
fn the_knife_and_slide_tools() {
    use kition_kit::tools::{KitTools, PointerAction, ToolMods, ToolPointer};
    let pointer = |action, x: f64, y: f64| ToolPointer { action, x, y, mods: ToolMods::default(), pressure: 1.0, zoom: 1.0 };
    let mut doc = editing_cube();
    run(&mut doc, "view.resize", json!({ "width": 400, "height": 300 }));
    run(&mut doc, "view.top", json!({}));
    run(&mut doc, "select.all", json!({ "action": "deselect" }));
    assert!(doc.tools().iter().any(|tool| tool.id == "knife"));
    doc.select_tool("knife", 1.0).unwrap();
    // Across the whole view through its middle: the top face's two side edges.
    doc.pointer(&pointer(PointerAction::Down, 5.0, 150.0)).unwrap();
    let response = doc.pointer(&pointer(PointerAction::Move, 395.0, 150.0)).unwrap();
    assert!(response.busy && !response.overlays.is_empty());
    doc.pointer(&pointer(PointerAction::Down, 395.0, 150.0)).unwrap();
    doc.key("Enter", ToolMods::default(), 1.0).unwrap();
    assert_eq!(mesh(&doc)["faces"], 7);
    // Escape forgets an unfinished line.
    doc.pointer(&pointer(PointerAction::Down, 5.0, 100.0)).unwrap();
    doc.key("Escape", ToolMods::default(), 1.0).unwrap();
    assert_eq!(doc.inspect().unwrap()["tool"]["knifePoints"], 0);

    // Slide the loop a loop cut makes: one undo step per drag.
    let mut doc = editing_cube();
    let ring = run(&mut doc, "mesh.loopCut", json!({ "edge": [0, 1] }))["vertices"].clone();
    run(&mut doc, "mesh.select", json!({ "vertices": ring }));
    doc.select_tool("slide", 1.0).unwrap();
    let before = doc.save_file(None).unwrap();
    doc.pointer(&pointer(PointerAction::Down, 100.0, 100.0)).unwrap();
    doc.pointer(&pointer(PointerAction::Drag, 200.0, 100.0)).unwrap();
    doc.pointer(&pointer(PointerAction::Up, 200.0, 100.0)).unwrap();
    assert_ne!(doc.save_file(None).unwrap(), before);
    run(&mut doc, "edit.undo", json!({}));
    let back = doc.save_file(None).unwrap();
    assert_eq!(
        SceneDocument::open("a.kscene", &back).unwrap().inspect().unwrap()["objects"],
        SceneDocument::open("b.kscene", &before).unwrap().inspect().unwrap()["objects"]
    );
}
