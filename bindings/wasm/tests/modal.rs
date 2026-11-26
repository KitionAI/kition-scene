//! The modal transform (plan scene-usable-by-hand, phase 2): G, R and S as session commands the
//! editor drives (`modal.begin`, `modal.pointer`, `modal.key`, `modal.confirm`, `modal.cancel`),
//! through the engine as the editor's keys and pointer reach it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_kit::tools::{KitTools, PointerAction, ToolMods, ToolPointer};
use kition_scene_kit::SceneDocument;
use kition_scene_model::glam::{Vec3, Vec4};
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

/// The starter scene seen from the front, the cube selected, saved.
fn starter() -> SceneDocument {
    let mut doc = SceneDocument::create("Modal.kscene", 800, 600).unwrap();
    run(&mut doc, "view.front", json!({}));
    run(&mut doc, "view.frameAll", json!({}));
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    doc.mark_saved();
    doc
}

fn cube(doc: &SceneDocument) -> &kition_scene_model::Object {
    doc.scene().object_named("Cube").unwrap()
}

fn undo_label(doc: &SceneDocument) -> Value {
    doc.inspect().unwrap()["undo"].clone()
}

/// A world point on the screen, in document pixels.
fn screen(doc: &SceneDocument, point: Vec3) -> (f64, f64) {
    let projector = doc.session().editor.view.projector(doc.scene(), doc.width(), doc.height());
    let at = projector.to_screen(projector.view_projection() * Vec4::new(point.x, point.y, point.z, 1.0));
    (f64::from(at.x), f64::from(at.y))
}

fn keys(doc: &mut SceneDocument, keys: &[&str]) {
    for key in keys {
        let answer = run(doc, "modal.key", json!({ "key": key }));
        assert_eq!(answer["taken"], true, "{key}: {answer}");
    }
}

fn near(a: Vec3, b: Vec3) -> bool {
    (a - b).length() < 1e-4
}

#[test]
fn typed_values_move_rotate_and_scale_exactly_in_one_step() {
    let mut doc = starter();
    let base = undo_label(&doc);
    let at = screen(&doc, Vec3::new(1.0, 0.0, 1.0));
    run(&mut doc, "modal.begin", json!({ "kind": "move", "at": [at.0, at.1] }));
    keys(&mut doc, &["z", "2"]);
    let answer = run(&mut doc, "modal.state", json!({}));
    assert_eq!(answer["header"], "D: [2|] = 2.0000 m (2.0000 m) along global Z", "{answer}");
    assert!(near(cube(&doc).location, Vec3::new(0.0, 0.0, 2.0)), "the scene shows the move");
    assert_eq!(undo_label(&doc), base, "a transform under way is no step");
    assert!(!doc.dirty(), "nor an unsaved change");
    run(&mut doc, "modal.confirm", json!({}));
    assert!(near(cube(&doc).location, Vec3::new(0.0, 0.0, 2.0)));
    assert_eq!(undo_label(&doc), "Move");
    assert!(doc.dirty());
    run(&mut doc, "edit.undo", json!({}));
    assert!(near(cube(&doc).location, Vec3::ZERO), "one undo takes it back");
    // Minus negates the whole number.
    run(&mut doc, "modal.begin", json!({ "kind": "move", "at": [at.0, at.1] }));
    keys(&mut doc, &["z", "-", "2"]);
    run(&mut doc, "modal.confirm", json!({}));
    assert!(near(cube(&doc).location, Vec3::new(0.0, 0.0, -2.0)), "{:?}", cube(&doc).location);
    run(&mut doc, "edit.undo", json!({}));
    // R Z 90: a quarter turn about Z.
    run(&mut doc, "modal.begin", json!({ "kind": "rotate", "at": [at.0, at.1] }));
    keys(&mut doc, &["z", "9", "0"]);
    run(&mut doc, "modal.confirm", json!({}));
    assert_eq!(undo_label(&doc), "Rotate");
    assert!((cube(&doc).rotation.z.to_degrees() - 90.0).abs() < 1e-3, "{:?}", cube(&doc).rotation);
    run(&mut doc, "edit.undo", json!({}));
    // S X 2 and S Z 0.05.
    run(&mut doc, "modal.begin", json!({ "kind": "scale", "at": [at.0, at.1] }));
    keys(&mut doc, &["x", "2"]);
    run(&mut doc, "modal.confirm", json!({}));
    assert!(near(cube(&doc).scale, Vec3::new(2.0, 1.0, 1.0)), "{:?}", cube(&doc).scale);
    assert_eq!(undo_label(&doc), "Scale");
    run(&mut doc, "edit.undo", json!({}));
    run(&mut doc, "modal.begin", json!({ "kind": "scale", "at": [at.0, at.1] }));
    keys(&mut doc, &["z", "0", ".", "0", "5"]);
    run(&mut doc, "modal.confirm", json!({}));
    assert!(near(cube(&doc).scale, Vec3::new(1.0, 1.0, 0.05)), "{:?}", cube(&doc).scale);
    // An untyped scale takes the number for every axis.
    run(&mut doc, "edit.undo", json!({}));
    run(&mut doc, "modal.begin", json!({ "kind": "scale", "at": [at.0, at.1] }));
    keys(&mut doc, &["3"]);
    run(&mut doc, "modal.confirm", json!({}));
    assert!(near(cube(&doc).scale, Vec3::splat(3.0)), "{:?}", cube(&doc).scale);
}

#[test]
fn the_pointer_drives_the_transform_and_axis_keys_keep_it_to_an_axis() {
    let mut doc = starter();
    let origin = screen(&doc, Vec3::ZERO);
    let from = (origin.0 + 100.0, origin.1 + 50.0);
    // Free: in the view's plane (the front view: X and Z).
    run(&mut doc, "modal.begin", json!({ "kind": "move", "at": [from.0, from.1] }));
    run(&mut doc, "modal.pointer", json!({ "at": [from.0 + 80.0, from.1 - 40.0] }));
    let free = cube(&doc).location;
    assert!(free.x > 0.1 && free.z > 0.1 && free.y.abs() < 1e-4, "{free:?}");
    let header = run(&mut doc, "modal.state", json!({}))["header"].as_str().unwrap().to_owned();
    assert!(header.starts_with("Dx: "), "{header}");
    // X: only X; the second press the object's own X; the third free again.
    keys(&mut doc, &["x"]);
    let along = cube(&doc).location;
    assert!(along.x > 0.1 && along.y == 0.0 && along.z == 0.0, "{along:?}");
    assert!(run(&mut doc, "modal.state", json!({}))["header"].as_str().unwrap().ends_with("along global X"));
    keys(&mut doc, &["x"]);
    assert!(run(&mut doc, "modal.state", json!({}))["header"].as_str().unwrap().ends_with("along local X"));
    keys(&mut doc, &["x"]);
    assert!(near(cube(&doc).location, free), "a third press frees it again");
    // Shift+Z: across Z (the view shows X and Z, so only X follows).
    run(&mut doc, "modal.key", json!({ "key": "z", "shift": true }));
    let locked = cube(&doc).location;
    assert!(locked.z == 0.0 && locked.x > 0.1, "{locked:?}");
    // Ctrl: whole metres.
    run(&mut doc, "modal.key", json!({ "key": "x" }));
    run(&mut doc, "modal.pointer", json!({ "at": [from.0 + 333.0, from.1], "step": true }));
    let stepped = cube(&doc).location.x;
    assert!(stepped >= 1.0 && (stepped - stepped.round()).abs() < 1e-5, "{stepped}");
    // Shift: a tenth of the pointer's motion from where it was when held.
    run(&mut doc, "modal.pointer", json!({ "at": [from.0 + 333.0, from.1] }));
    let held = cube(&doc).location.x;
    run(&mut doc, "modal.pointer", json!({ "at": [from.0 + 433.0, from.1], "precision": true }));
    let fine = cube(&doc).location.x - held;
    run(&mut doc, "modal.cancel", json!({}));
    let mut coarse = starter();
    run(&mut coarse, "modal.begin", json!({ "kind": "move", "at": [from.0, from.1] }));
    run(&mut coarse, "modal.key", json!({ "key": "x" }));
    run(&mut coarse, "modal.pointer", json!({ "at": [from.0 + 100.0, from.1] }));
    let whole = cube(&coarse).location.x;
    assert!((fine - whole * 0.1).abs() < 1e-3, "Shift moved {fine}, a tenth of {whole}");
}

#[test]
fn cancelling_leaves_no_step_and_nothing_unsaved() {
    let mut doc = starter();
    let base = undo_label(&doc);
    let revision = doc.revision();
    let origin = screen(&doc, Vec3::ZERO);
    for kind in ["move", "rotate", "scale"] {
        run(&mut doc, "modal.begin", json!({ "kind": kind, "at": [origin.0 + 80.0, origin.1] }));
        run(&mut doc, "modal.pointer", json!({ "at": [origin.0 + 40.0, origin.1 - 90.0] }));
        assert!(run(&mut doc, "modal.state", json!({}))["changed"] == true, "{kind}");
        let answer = run(&mut doc, "modal.cancel", json!({}));
        assert_eq!(answer["cancelled"], true);
        let object = cube(&doc);
        assert!(near(object.location, Vec3::ZERO) && near(object.rotation, Vec3::ZERO) && near(object.scale, Vec3::ONE), "{kind}: put back");
        assert_eq!(undo_label(&doc), base, "{kind}: no step");
        assert!(!doc.dirty(), "{kind}: nothing unsaved");
    }
    assert!(doc.revision() > revision, "the picture is drawn again after a transform ends");
    // Nothing to end: no error.
    assert_eq!(run(&mut doc, "modal.cancel", json!({}))["cancelled"], false);
    assert_eq!(run(&mut doc, "modal.confirm", json!({}))["confirmed"], false);
    // Nothing selected: nothing to transform.
    run(&mut doc, "edit.deselectAll", json!({}));
    assert!(doc.execute("modal.begin", json!({ "kind": "move" })).unwrap_err().starts_with("not-allowed"));
}

#[test]
fn the_quick_step_leaves_the_revision_alone() {
    // The editor's pointer and keys go through `modalStep`: no inspect, no software picture.
    let mut doc = starter();
    let origin = screen(&doc, Vec3::ZERO);
    run(&mut doc, "modal.begin", json!({ "kind": "move", "at": [origin.0 + 60.0, origin.1] }));
    let revision = doc.revision();
    let answer: Value =
        serde_json::from_str(&doc.modal_step("modal.pointer", &json!({ "at": [origin.0 + 160.0, origin.1] }).to_string(), true).unwrap()).unwrap();
    assert_eq!(answer["active"], true);
    assert!(answer["frame"]["objects"].as_array().is_some_and(|objects| !objects.is_empty()), "the GPU frame comes with it");
    let answer: Value = serde_json::from_str(&doc.modal_step("modal.key", &json!({ "key": "x" }).to_string(), false).unwrap()).unwrap();
    assert_eq!(answer["taken"], true);
    assert!(answer.get("frame").is_none());
    assert_eq!(doc.revision(), revision, "the revision stays");
    assert!(cube(&doc).location.x > 0.1);
    run(&mut doc, "modal.confirm", json!({}));
    assert!(doc.revision() > revision);
}

#[test]
fn shift_d_moves_the_copies_in_one_duplicate_step() {
    let mut doc = starter();
    let origin = screen(&doc, Vec3::ZERO);
    run(&mut doc, "modal.begin", json!({ "kind": "move", "duplicate": true, "at": [origin.0, origin.1] }));
    run(&mut doc, "modal.pointer", json!({ "at": [origin.0 + 120.0, origin.1] }));
    run(&mut doc, "modal.confirm", json!({}));
    assert_eq!(undo_label(&doc), "Duplicate");
    let copy = doc.scene().object_named("Cube.001").unwrap();
    assert!(copy.location.x > 0.3, "{:?}", copy.location);
    assert!(near(cube(&doc).location, Vec3::ZERO), "the original stays");
    run(&mut doc, "edit.undo", json!({}));
    assert!(doc.scene().object_named("Cube.001").is_none(), "one undo takes the copy away");
    // Cancelled, the copy stays where it was made, its own step.
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "modal.begin", json!({ "kind": "move", "duplicate": true, "at": [origin.0, origin.1] }));
    run(&mut doc, "modal.pointer", json!({ "at": [origin.0 + 120.0, origin.1] }));
    run(&mut doc, "modal.cancel", json!({}));
    assert_eq!(undo_label(&doc), "Duplicate");
    assert!(near(doc.scene().object_named("Cube.001").unwrap().location, Vec3::ZERO));
}

#[test]
fn another_command_ends_the_transform_first() {
    let mut doc = starter();
    let origin = screen(&doc, Vec3::ZERO);
    run(&mut doc, "modal.begin", json!({ "kind": "move", "at": [origin.0 + 60.0, origin.1] }));
    run(&mut doc, "modal.pointer", json!({ "at": [origin.0 + 200.0, origin.1] }));
    assert!(cube(&doc).location.x > 0.5);
    // The view leaves it running.
    run(&mut doc, "view.orbit", json!({ "turn": 10 }));
    assert_eq!(run(&mut doc, "modal.state", json!({}))["active"], true);
    // A change ends it: the scene it changes is the scene as it was.
    run(&mut doc, "object.rename", json!({ "object": "Cube", "name": "Box" }));
    assert_eq!(run(&mut doc, "modal.state", json!({}))["active"], false);
    let renamed = doc.scene().object_named("Box").unwrap();
    assert!(near(renamed.location, Vec3::ZERO), "the transform did not ride along with the rename");
    let saved = doc.save_file(None).unwrap();
    let back = SceneDocument::open("Modal.kscene", &saved).unwrap();
    assert!(near(back.scene().object_named("Box").unwrap().location, Vec3::ZERO), "a transform is never saved");
}

#[test]
fn edit_mode_moves_the_selected_face() {
    let mut doc = starter();
    run(&mut doc, "mode.set", json!({ "mode": "edit" }));
    run(&mut doc, "mesh.select", json!({ "faces": [0] }));
    let selected: Vec<u32> = doc.session().vertices.iter().copied().collect();
    let before: Vec<Vec3> = cube(&doc).mesh().unwrap().positions.clone();
    let origin = screen(&doc, Vec3::ZERO);
    run(&mut doc, "modal.begin", json!({ "kind": "move", "at": [origin.0 + 50.0, origin.1] }));
    keys(&mut doc, &["z", "1"]);
    run(&mut doc, "modal.confirm", json!({}));
    assert_eq!(undo_label(&doc), "Move");
    let after = &cube(&doc).mesh().unwrap().positions;
    for (index, (was, now)) in before.iter().zip(after).enumerate() {
        let expected = if selected.contains(&(index as u32)) { *was + Vec3::Z } else { *was };
        assert!(near(*now, expected), "vertex {index}: {was:?} → {now:?}");
    }
}

#[test]
fn a_tool_press_runs_the_transform_until_the_release() {
    let mut doc = starter();
    doc.select_tool("move", 1.0).unwrap();
    let origin = screen(&doc, Vec3::ZERO);
    let event = |action, at: (f64, f64)| ToolPointer { action, x: at.0, y: at.1, mods: ToolMods::default(), pressure: 1.0, zoom: 1.0 };
    doc.pointer(&event(PointerAction::Down, origin)).unwrap();
    doc.pointer(&event(PointerAction::Drag, (origin.0 + 40.0, origin.1 - 30.0))).unwrap();
    // X during the drag keeps it to X (it deleted the cube before).
    doc.key("x", ToolMods::default(), 1.0).unwrap();
    doc.pointer(&event(PointerAction::Drag, (origin.0 + 120.0, origin.1 - 60.0))).unwrap();
    doc.pointer(&event(PointerAction::Up, (origin.0 + 120.0, origin.1 - 60.0))).unwrap();
    let location = cube(&doc).location;
    assert!(location.x > 0.3 && location.y == 0.0 && location.z == 0.0, "{location:?}");
    assert_eq!(undo_label(&doc), "Move");
    // A key's transform: the left button confirms it, and its release does nothing more.
    run(&mut doc, "edit.undo", json!({}));
    doc.select_tool("select", 1.0).unwrap();
    run(&mut doc, "modal.begin", json!({ "kind": "rotate", "at": [origin.0 + 80.0, origin.1] }));
    doc.pointer(&event(PointerAction::Move, (origin.0 + 80.0, origin.1 - 80.0))).unwrap();
    doc.pointer(&event(PointerAction::Down, (origin.0 + 80.0, origin.1 - 80.0))).unwrap();
    doc.pointer(&event(PointerAction::Up, (origin.0 + 80.0, origin.1 - 80.0))).unwrap();
    assert_eq!(undo_label(&doc), "Rotate");
    let turned = cube(&doc).rotation.y.to_degrees();
    assert!((turned.abs() - 45.0).abs() < 1.0, "a quarter of the way round the pivot turns 45° about the view's axis ({turned})");
    assert_eq!(doc.inspect().unwrap()["selection"]["objects"].as_array().unwrap().len(), 1, "the release selected nothing");
}
