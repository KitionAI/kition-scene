//! Edit mode's operations as modals, and Adjust Last Operation (plan scene-usable-by-hand, phase
//! 3): E extrudes and moves the extrusion along the faces' normal, I insets by the pointer (Ctrl:
//! the depth), Ctrl+B bevels by the pointer with the wheel's segments, Ctrl+R shows a loop, cuts
//! it and slides it; the person's last operation runs again with other values in place of its own
//! undo step.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use kition_scene_model::glam::{Vec3, Vec4};
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

/// The starter scene from the front, the cube in edit mode.
fn editing() -> SceneDocument {
    let mut doc = SceneDocument::create("Ops.kscene", 800, 600).unwrap();
    run(&mut doc, "view.front", json!({}));
    run(&mut doc, "view.frameAll", json!({}));
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "mode.set", json!({ "mode": "edit" }));
    doc.mark_saved();
    doc
}

fn mesh(doc: &SceneDocument) -> &kition_scene_model::Mesh {
    doc.scene().object_named("Cube").unwrap().mesh().unwrap()
}

fn inspect(doc: &SceneDocument) -> Value {
    doc.inspect().unwrap()
}

/// The face whose corners all have `z` (the cube's top at 1).
fn face_at_height(doc: &SceneDocument, z: f32) -> usize {
    let mesh = mesh(doc);
    mesh.faces.iter().position(|face| face.iter().all(|vertex| (mesh.positions[*vertex as usize].z - z).abs() < 1e-4)).unwrap()
}

fn select_top(doc: &mut SceneDocument) {
    let top = face_at_height(doc, 1.0);
    run(doc, "mesh.select", json!({ "faces": [top] }));
}

fn screen(doc: &SceneDocument, point: Vec3) -> [f64; 2] {
    let projector = doc.session().editor.view.projector(doc.scene(), doc.width(), doc.height());
    let at = projector.to_screen(projector.view_projection() * Vec4::new(point.x, point.y, point.z, 1.0));
    [f64::from(at.x), f64::from(at.y)]
}

fn keys(doc: &mut SceneDocument, keys: &[&str]) {
    for key in keys {
        let answer = run(doc, "modal.key", json!({ "key": key }));
        assert_eq!(answer["taken"], true, "{key}: {answer}");
    }
}

fn highest(doc: &SceneDocument) -> f32 {
    mesh(doc).positions.iter().map(|point| point.z).fold(f32::MIN, f32::max)
}

#[test]
fn extrude_moves_along_the_normal_and_takes_a_negative_value() {
    let mut doc = editing();
    select_top(&mut doc);
    let before = inspect(&doc)["undo"].clone();
    let at = screen(&doc, Vec3::new(0.0, 0.0, 1.0));
    run(&mut doc, "modal.begin", json!({ "kind": "extrude", "at": at }));
    assert_eq!(mesh(&doc).positions.len(), 12, "the extrusion is made at once");
    let answer = run(&mut doc, "modal.state", json!({}));
    assert!(answer["header"].as_str().unwrap().ends_with(" normal"), "{answer}");
    // The pointer up the screen pulls it out along the normal.
    run(&mut doc, "modal.pointer", json!({ "at": [at[0], at[1] - 60.0] }));
    assert!(highest(&doc) > 1.05, "the pointer moves it ({})", highest(&doc));
    keys(&mut doc, &["-", "0", ".", "5"]);
    let answer = run(&mut doc, "modal.state", json!({}));
    assert_eq!(answer["header"], "D: [-(0.5|)] = -0.5000 m (0.5000 m) normal", "{answer}");
    assert_eq!(inspect(&doc)["undo"], before, "no step while it runs");
    run(&mut doc, "modal.confirm", json!({}));
    let mesh_now = mesh(&doc);
    assert_eq!(mesh_now.positions.len(), 12);
    let top = face_at_height(&doc, 0.5);
    assert_eq!(mesh(&doc).faces[top].len(), 4, "the top is down at 0.5 m: the mug's hollow");
    assert_eq!(inspect(&doc)["undo"], "Extrude Faces");
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(mesh(&doc).positions.len(), 8, "one undo takes the whole extrusion back");
}

#[test]
fn cancelling_the_extrusions_move_keeps_its_geometry() {
    let mut doc = editing();
    select_top(&mut doc);
    let at = screen(&doc, Vec3::new(0.0, 0.0, 1.0));
    run(&mut doc, "modal.begin", json!({ "kind": "extrude", "at": at }));
    run(&mut doc, "modal.pointer", json!({ "at": [at[0], at[1] - 60.0] }));
    run(&mut doc, "modal.cancel", json!({}));
    assert_eq!(mesh(&doc).positions.len(), 12, "the extrusion stays, as Blender leaves it");
    assert!((highest(&doc) - 1.0).abs() < 1e-5, "where it was made");
    assert_eq!(inspect(&doc)["undo"], "Extrude Faces");
}

#[test]
fn inset_takes_the_pointer_typed_values_and_a_depth() {
    let mut doc = editing();
    select_top(&mut doc);
    let centre = screen(&doc, Vec3::new(0.0, 0.0, 1.0));
    let at = [centre[0] + 150.0, centre[1]];
    run(&mut doc, "modal.begin", json!({ "kind": "inset", "at": at }));
    // Towards the centre is thicker.
    let answer = run(&mut doc, "modal.pointer", json!({ "at": [centre[0] + 100.0, centre[1]] }));
    let thickness = answer["values"]["thickness"].as_f64().unwrap();
    assert!(thickness > 0.05, "{answer}");
    assert!(answer["header"].as_str().unwrap().starts_with("Thickness: "), "{answer}");
    // Ctrl: the pointer sizes the depth, the thickness stays.
    let answer = run(&mut doc, "modal.pointer", json!({ "at": [centre[0] + 140.0, centre[1]], "step": true }));
    assert!((answer["values"]["thickness"].as_f64().unwrap() - thickness).abs() < 1e-6, "{answer}");
    let answer = run(&mut doc, "modal.pointer", json!({ "at": [centre[0] + 180.0, centre[1]], "step": true }));
    assert!(answer["values"]["depth"].as_f64().unwrap() > 0.05, "{answer}");
    keys(&mut doc, &["0", ".", "2", "Tab", "-", "0", ".", "3"]);
    let answer = run(&mut doc, "modal.state", json!({}));
    assert_eq!(answer["values"]["thickness"].as_f64().unwrap() as f32, 0.2, "{answer}");
    assert_eq!(answer["values"]["depth"].as_f64().unwrap() as f32, -0.3, "{answer}");
    run(&mut doc, "modal.confirm", json!({}));
    assert_eq!(inspect(&doc)["undo"], "Inset Faces");
    assert_eq!(mesh(&doc).positions.len(), 12);
    let inner = face_at_height(&doc, 0.7);
    let corners: Vec<Vec3> = mesh(&doc).faces[inner].iter().map(|vertex| mesh(&doc).positions[*vertex as usize]).collect();
    assert!(corners.iter().all(|corner| (corner.x.abs() - 0.8).abs() < 1e-4), "0.2 m in: {corners:?}");
    // Escape on a new inset leaves nothing.
    run(&mut doc, "modal.begin", json!({ "kind": "inset", "at": at }));
    run(&mut doc, "modal.cancel", json!({}));
    assert_eq!(mesh(&doc).positions.len(), 12);
    assert_eq!(inspect(&doc)["undo"], "Inset Faces");
}

#[test]
fn bevel_follows_the_pointer_and_the_wheel_adds_segments() {
    let mut doc = editing();
    let mesh_now = mesh(&doc);
    let top: Vec<u32> = (0..mesh_now.positions.len() as u32)
        .filter(|vertex| {
            let p = mesh_now.positions[*vertex as usize];
            p.z > 0.9 && p.y < -0.9
        })
        .collect();
    assert_eq!(top.len(), 2, "the front top edge");
    run(&mut doc, "mesh.select", json!({ "vertices": top }));
    let centre = screen(&doc, Vec3::new(0.0, -1.0, 1.0));
    run(&mut doc, "modal.begin", json!({ "kind": "bevel", "at": [centre[0], centre[1] - 20.0] }));
    let answer = run(&mut doc, "modal.pointer", json!({ "at": [centre[0], centre[1] - 60.0] }));
    assert!(answer["values"]["width"].as_f64().unwrap() > 0.05, "{answer}");
    let answer = run(&mut doc, "modal.wheel", json!({ "delta": -100 }));
    assert_eq!(answer["taken"], true);
    assert_eq!(answer["values"]["segments"], 2, "{answer}");
    keys(&mut doc, &["0", ".", "1"]);
    let answer = run(&mut doc, "modal.state", json!({}));
    assert_eq!(answer["header"], "Offset: [0.1|] = 0.1000 m, Segments: 2", "{answer}");
    run(&mut doc, "modal.confirm", json!({}));
    assert_eq!(inspect(&doc)["undo"], "Bevel Edges");
    let operation = inspect(&doc)["operation"].clone();
    assert_eq!(operation["command"], "mesh.bevel", "{operation}");
    assert_eq!(operation["params"]["segments"], 2, "{operation}");
}

#[test]
fn loop_cut_shows_cuts_then_slides_and_the_right_button_centres() {
    let mut doc = editing();
    run(&mut doc, "edit.deselectAll", json!({}));
    // A vertical edge at the front.
    let at = screen(&doc, Vec3::new(1.0, -1.0, 0.0));
    let answer = run(&mut doc, "modal.begin", json!({ "kind": "loopCut", "at": at }));
    assert_eq!(answer["kind"], "loopCut", "{answer}");
    assert!(answer["edge"].is_array(), "the edge under the pointer: {answer}");
    let answer = run(&mut doc, "modal.wheel", json!({ "delta": -100 }));
    assert_eq!(answer["header"], "Cuts: 2", "{answer}");
    assert_eq!(mesh(&doc).positions.len(), 8, "nothing is cut yet");
    let answer = run(&mut doc, "modal.confirm", json!({}));
    assert_eq!(answer["active"], true, "the click goes on to the slide: {answer}");
    assert_eq!(answer["kind"], "slide", "{answer}");
    assert_eq!(mesh(&doc).positions.len(), 16, "two loops of four");
    run(&mut doc, "modal.cancel", json!({}));
    assert_eq!(mesh(&doc).positions.len(), 16, "the right button keeps the cut");
    assert_eq!(inspect(&doc)["undo"], "Loop Cut");
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(mesh(&doc).positions.len(), 8, "in one step");
    // One cut, slid: off the middle.
    run(&mut doc, "modal.begin", json!({ "kind": "loopCut", "at": at }));
    run(&mut doc, "modal.confirm", json!({}));
    let answer = run(&mut doc, "modal.pointer", json!({ "at": [at[0], at[1] - 80.0] }));
    let factor = answer["values"]["factor"].as_f64().unwrap();
    assert!((factor - 0.5).abs() > 0.05, "{answer}");
    run(&mut doc, "modal.confirm", json!({}));
    assert_eq!(mesh(&doc).positions.len(), 12);
    let heights: Vec<f32> = mesh(&doc).positions[8..].iter().map(|point| point.z).collect();
    assert!(heights.iter().all(|z| (z.abs() - (factor as f32 * 2.0 - 1.0).abs()).abs() < 1e-3), "{heights:?} for {factor}");
    // Escape before the click cuts nothing.
    run(&mut doc, "modal.begin", json!({ "kind": "loopCut", "at": at }));
    run(&mut doc, "modal.cancel", json!({}));
    assert_eq!(mesh(&doc).positions.len(), 12);
}

#[test]
fn adjust_last_operation_runs_it_again_in_its_own_step() {
    let mut doc = editing();
    select_top(&mut doc);
    run(&mut doc, "mesh.extrude", json!({ "distance": 1.0 }));
    let operation = run(&mut doc, "operation.get", json!({}))["operation"].clone();
    assert_eq!(operation["title"], "Extrude Faces", "{operation}");
    let fields: Vec<&str> = operation["fields"].as_array().unwrap().iter().map(|field| field["key"].as_str().unwrap()).collect();
    assert_eq!(fields, ["distance", "offset"], "{operation}");
    assert_eq!(operation["fields"][0]["value"], 1.0);
    run(&mut doc, "operation.set", json!({ "params": { "distance": -0.2 } }));
    assert!((highest(&doc) - 1.0).abs() < 1e-5 && face_at_height(&doc, 0.8) < mesh(&doc).faces.len(), "the top went in instead");
    assert_eq!(mesh(&doc).positions.len(), 12, "one extrusion, not two");
    assert_eq!(inspect(&doc)["operation"]["params"]["distance"], -0.2);
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(mesh(&doc).positions.len(), 8, "still one undo step");
    assert!(inspect(&doc)["operation"].is_null(), "an undo ends it");

    // A confirmed move shows its vector and axis, and runs again along another.
    run(&mut doc, "mode.set", json!({ "mode": "object" }));
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    let at = screen(&doc, Vec3::ZERO);
    run(&mut doc, "modal.begin", json!({ "kind": "move", "at": at }));
    keys(&mut doc, &["z", "2"]);
    run(&mut doc, "modal.confirm", json!({}));
    let operation = inspect(&doc)["operation"].clone();
    assert_eq!(operation["title"], "Move", "{operation}");
    assert_eq!(operation["params"]["axis"], "z");
    assert_eq!(operation["params"]["move"], json!([0.0, 0.0, 2.0]));
    run(&mut doc, "operation.set", json!({ "params": { "axis": "x", "move": [3.0, 0.0, 0.0] } }));
    let cube = doc.scene().object_named("Cube").unwrap().location;
    assert!((cube - Vec3::new(3.0, 0.0, 0.0)).length() < 1e-4, "{cube:?}");
    assert_eq!(inspect(&doc)["undo"], "Move");
    run(&mut doc, "edit.undo", json!({}));
    assert!(doc.scene().object_named("Cube").unwrap().location.length() < 1e-4, "one undo");
    // Selecting something ends it.
    run(&mut doc, "modal.begin", json!({ "kind": "move", "at": at }));
    keys(&mut doc, &["x", "1"]);
    run(&mut doc, "modal.confirm", json!({}));
    assert!(!inspect(&doc)["operation"].is_null());
    run(&mut doc, "edit.deselectAll", json!({}));
    assert!(inspect(&doc)["operation"].is_null(), "another change ends it");
    assert!(doc.execute("operation.set", json!({ "params": { "move": [1, 1, 1] } })).is_err());
}

#[test]
fn an_added_object_shows_its_size_and_segments() {
    let mut doc = SceneDocument::create("Add.kscene", 800, 600).unwrap();
    run(&mut doc, "object.addCylinder", json!({}));
    let operation = inspect(&doc)["operation"].clone();
    assert_eq!(operation["title"], "Cylinder", "{operation}");
    let field = |key: &str| operation["fields"].as_array().unwrap().iter().find(|field| field["key"] == key).cloned().unwrap();
    assert_eq!(field("vertices")["value"], 32, "the default its description states");
    assert_eq!(field("radius")["value"], 1.0);
    assert_eq!(field("location")["type"], "vector");
    let keys: Vec<String> = operation["fields"].as_array().unwrap().iter().map(|field| field["key"].as_str().unwrap().to_owned()).collect();
    assert_eq!(keys.last().map(String::as_str), Some("scale"), "placement last: {keys:?}");
    let count = doc.scene().objects.len();
    run(&mut doc, "operation.set", json!({ "params": { "vertices": 8, "radius": 0.5 } }));
    assert_eq!(doc.scene().objects.len(), count, "the cylinder is made again, not twice");
    let cylinder = doc.scene().object_named("Cylinder").unwrap().mesh().unwrap();
    assert!(matches!(cylinder.positions.len(), 16 | 18), "eight around, two rings: {}", cylinder.positions.len());
    assert!(!doc.inspect().unwrap()["operation"].is_null(), "and the panel stays");
}
