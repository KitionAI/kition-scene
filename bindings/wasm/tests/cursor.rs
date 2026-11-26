//! The 3D cursor, the pivot point, the transform orientation and snapping (plan
//! scene-usable-by-hand, phase 5): session commands (`cursor.set`, `cursor.reset`,
//! `cursor.moveToSelected`, `cursor.moveToActive`, `transform.set`) that make no undo step, leave
//! the document clean and live in the view state, and the modal transform following them.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use kition_scene_model::glam::{Vec3, Vec4};
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

/// The starter scene seen from the front, the cube selected, saved.
fn starter() -> SceneDocument {
    let mut doc = SceneDocument::create("Cursor.kscene", 800, 600).unwrap();
    run(&mut doc, "view.front", json!({}));
    run(&mut doc, "view.frameAll", json!({}));
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    doc.mark_saved();
    doc
}

fn screen(doc: &SceneDocument, point: Vec3) -> [f64; 2] {
    let projector = doc.session().editor.view.projector(doc.scene(), doc.width(), doc.height());
    let at = projector.to_screen(projector.view_projection() * Vec4::new(point.x, point.y, point.z, 1.0));
    [f64::from(at.x), f64::from(at.y)]
}

fn cursor(doc: &SceneDocument) -> Vec3 {
    let value = &doc.inspect().unwrap()["transform"]["cursor"];
    Vec3::new(value[0].as_f64().unwrap() as f32, value[1].as_f64().unwrap() as f32, value[2].as_f64().unwrap() as f32)
}

fn near(a: Vec3, b: Vec3) -> bool {
    (a - b).length() < 1e-3
}

fn object(doc: &SceneDocument, name: &str) -> kition_scene_model::Object {
    doc.scene().object_named(name).unwrap().clone()
}

fn no_step(doc: &SceneDocument) {
    assert!(!doc.dirty(), "the document stays clean");
    assert_eq!(doc.inspect().unwrap()["canUndo"], false, "no undo step");
}

#[test]
fn the_cursor_is_view_state_and_lands_on_the_surface_under_a_point() {
    let mut doc = starter();
    assert!(near(cursor(&doc), Vec3::ZERO), "the cursor starts at the origin");
    // On the cube's front face, seen from the front.
    let at = screen(&doc, Vec3::new(0.3, -1.0, 0.2));
    let placed = run(&mut doc, "cursor.set", json!({ "at": at }));
    let on = cursor(&doc);
    assert!((on.y + 1.0).abs() < 1e-3 && (on.x - 0.3).abs() < 0.01 && (on.z - 0.2).abs() < 0.01, "on the front face: {placed}");
    // Over nothing: on the plane through the cursor facing the view (here y = -1).
    let sky = screen(&doc, Vec3::new(6.0, -1.0, 3.0));
    run(&mut doc, "cursor.set", json!({ "at": sky }));
    assert!(near(cursor(&doc), Vec3::new(6.0, -1.0, 3.0)), "{:?}", cursor(&doc));
    no_step(&doc);
    // It travels with the view state, and a state without it restores at the origin.
    let mut state = run(&mut doc, "view.state", json!({}))["state"].clone();
    assert_eq!(state["cursor"].as_array().map(Vec::len), Some(3), "{state}");
    let mut other = starter();
    run(&mut other, "view.restore", json!({ "state": state.clone() }));
    assert!(near(cursor(&other), cursor(&doc)), "the cursor comes back with the view state");
    state.as_object_mut().unwrap().remove("cursor");
    run(&mut other, "view.restore", json!({ "state": state }));
    assert!(near(cursor(&other), Vec3::ZERO));
    // Reset, to the selection, to the active object.
    run(&mut doc, "object.transform", json!({ "objects": ["Cube"], "location": [1.0, 2.0, 3.0] }));
    run(&mut doc, "cursor.moveToSelected", json!({}));
    assert!(near(cursor(&doc), Vec3::new(1.0, 2.0, 3.0)));
    run(&mut doc, "cursor.reset", json!({}));
    assert!(near(cursor(&doc), Vec3::ZERO));
    run(&mut doc, "cursor.moveToActive", json!({}));
    assert!(near(cursor(&doc), Vec3::new(1.0, 2.0, 3.0)));
    assert!(doc.execute("cursor.set", json!({ "location": [f64::MAX, 0.0, 0.0] })).is_err());
    assert!(doc.execute("cursor.set", json!({})).is_err());
}

#[test]
fn an_agents_add_without_a_location_keeps_the_origin() {
    let mut doc = starter();
    run(&mut doc, "cursor.set", json!({ "location": [4.0, 4.0, 0.0] }));
    run(&mut doc, "object.addUvSphere", json!({}));
    assert!(near(object(&doc, "Sphere").location, Vec3::ZERO), "the engine's default does not follow the cursor");
    run(&mut doc, "object.addCylinder", json!({ "location": [4.0, 4.0, 0.0] }));
    assert!(near(object(&doc, "Cylinder").location, Vec3::new(4.0, 4.0, 0.0)));
}

#[test]
fn rotations_turn_about_the_pivot_point() {
    let mut doc = starter();
    run(&mut doc, "object.addCube", json!({ "name": "Right", "location": [3.0, 0.0, 0.0], "size": 1.0 }));
    run(&mut doc, "select.objects", json!({ "objects": ["Cube"] }));
    run(&mut doc, "select.objects", json!({ "objects": ["Right"], "extend": true }));
    let rotate = |doc: &mut SceneDocument| {
        run(doc, "modal.begin", json!({ "kind": "rotate", "at": [500.0, 250.0] }));
        for key in ["y", "9", "0"] {
            run(doc, "modal.key", json!({ "key": key }));
        }
        run(doc, "modal.confirm", json!({}));
    };
    // The 3D cursor.
    run(&mut doc, "transform.set", json!({ "pivot": "cursor" }));
    run(&mut doc, "cursor.set", json!({ "location": [0.0, 0.0, 2.0] }));
    rotate(&mut doc);
    assert!(
        near(object(&doc, "Cube").location, Vec3::new(2.0, 0.0, 2.0)) || near(object(&doc, "Cube").location, Vec3::new(-2.0, 0.0, 2.0)),
        "{:?}",
        object(&doc, "Cube").location
    );
    run(&mut doc, "edit.undo", json!({}));
    // Individual origins: each turns in place.
    run(&mut doc, "transform.set", json!({ "pivot": "individual" }));
    rotate(&mut doc);
    assert!(near(object(&doc, "Cube").location, Vec3::ZERO) && near(object(&doc, "Right").location, Vec3::new(3.0, 0.0, 0.0)));
    assert!((object(&doc, "Cube").rotation.y.abs() - 90f32.to_radians()).abs() < 1e-3);
    run(&mut doc, "edit.undo", json!({}));
    // The bounding box's middle (1.5, 0, 0) against the median of the origins (1.5, 0, 0) is the
    // same here; the active object's origin is Right's.
    run(&mut doc, "transform.set", json!({ "pivot": "active" }));
    rotate(&mut doc);
    assert!(near(object(&doc, "Right").location, Vec3::new(3.0, 0.0, 0.0)), "the active object turns in place");
    assert!(!near(object(&doc, "Cube").location, Vec3::ZERO));
    let settings = run(&mut doc, "transform.set", json!({ "pivot": "median" }));
    assert_eq!(settings["pivot"], "median");
    assert!(doc.execute("transform.set", json!({ "pivot": "elsewhere" })).is_err());
}

#[test]
fn axis_keys_follow_the_orientation_and_the_gizmo_its_axes() {
    let mut doc = starter();
    run(&mut doc, "object.transform", json!({ "objects": ["Cube"], "rotation": [0.0, 0.0, 30.0] }));
    run(&mut doc, "transform.set", json!({ "orientation": "local" }));
    let gizmo = doc.inspect().unwrap()["transform"]["gizmo"].clone();
    assert!((gizmo["axes"][0][1].as_f64().unwrap() - 0.5).abs() < 1e-4, "the gizmo's X is the cube's own: {gizmo}");
    run(&mut doc, "modal.begin", json!({ "kind": "move", "at": [400.0, 300.0] }));
    let answer = run(&mut doc, "modal.key", json!({ "key": "x" }));
    assert_eq!(answer["constraint"]["orientation"], "local", "{answer}");
    let answer = run(&mut doc, "modal.key", json!({ "key": "x" }));
    assert_eq!(answer["constraint"]["orientation"], "global", "again: global");
    run(&mut doc, "modal.cancel", json!({}));
    run(&mut doc, "transform.set", json!({ "orientation": "view" }));
    run(&mut doc, "modal.begin", json!({ "kind": "move", "at": [400.0, 300.0] }));
    run(&mut doc, "modal.key", json!({ "key": "z" }));
    run(&mut doc, "modal.key", json!({ "key": "2" }));
    run(&mut doc, "modal.confirm", json!({}));
    // Seen from the front the view's Z points at the eye (-Y).
    assert!(near(object(&doc, "Cube").location, Vec3::new(0.0, -2.0, 0.0)), "{:?}", object(&doc, "Cube").location);
    // A gizmo handle's drag in the normal orientation in edit mode keeps to the faces' normal.
    run(&mut doc, "edit.undo", json!({}));
    run(&mut doc, "object.transform", json!({ "objects": ["Cube"], "rotation": [0.0, 0.0, 0.0] }));
    run(&mut doc, "mode.set", json!({ "mode": "edit" }));
    run(&mut doc, "mesh.selectMode", json!({ "mode": "face" }));
    let faces = doc.scene().object_named("Cube").unwrap().mesh().unwrap().faces.clone();
    let mesh = doc.scene().object_named("Cube").unwrap().mesh().unwrap().clone();
    let front = faces.iter().position(|face| mesh.face_normal(face).y < -0.9).unwrap();
    run(&mut doc, "mesh.select", json!({ "faces": [front] }));
    run(&mut doc, "modal.begin", json!({ "kind": "move", "at": [400.0, 300.0], "axis": "z", "orientation": "normal", "press": true }));
    run(&mut doc, "modal.key", json!({ "key": "1" }));
    let answer = run(&mut doc, "modal.state", json!({}));
    assert_eq!(answer["header"], "D: [1|] = 1.0000 m (1.0000 m) normal", "{answer}");
    run(&mut doc, "modal.confirm", json!({}));
    let moved = doc.scene().object_named("Cube").unwrap().mesh().unwrap().clone();
    let face = &moved.faces[front];
    let y = face.iter().map(|vertex| moved.positions[*vertex as usize].y).sum::<f32>() / face.len() as f32;
    assert!((y + 2.0).abs() < 1e-3, "the front face moved 1 m out along its normal: {y}");
}

#[test]
fn a_move_snaps_to_a_vertex_a_face_and_steps_with_increments() {
    let mut doc = starter();
    run(&mut doc, "object.addCube", json!({ "name": "Other", "location": [4.0, 0.0, 0.0] }));
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "transform.set", json!({ "snap": true, "snapTarget": "vertex" }));
    // Seen from the front, the other cube's near top corner.
    let corner = screen(&doc, Vec3::new(3.0, -1.0, 1.0));
    let start = screen(&doc, Vec3::new(0.0, -1.0, 0.0));
    run(&mut doc, "modal.begin", json!({ "kind": "move", "at": start }));
    let answer = run(&mut doc, "modal.pointer", json!({ "at": [corner[0] + 4.0, corner[1] - 3.0] }));
    assert_eq!(answer["snap"]["on"], true, "{answer}");
    let point = &answer["snap"]["point"];
    assert!(
        near(
            Vec3::new(point[0].as_f64().unwrap() as f32, point[1].as_f64().unwrap() as f32, point[2].as_f64().unwrap() as f32),
            Vec3::new(3.0, -1.0, 1.0)
        ),
        "{answer}"
    );
    // Ctrl inverts it: off while held.
    let off = run(&mut doc, "modal.pointer", json!({ "at": [corner[0] + 4.0, corner[1] - 3.0], "step": true }));
    assert_eq!(off["snap"]["on"], false);
    assert!(off["snap"]["point"].is_null());
    run(&mut doc, "modal.pointer", json!({ "at": [corner[0] + 4.0, corner[1] - 3.0] }));
    run(&mut doc, "modal.confirm", json!({}));
    // The corner of the moved cube nearest the target is on the other cube's (3, -1, 1).
    let at = object(&doc, "Cube").location;
    let corners = (0..8).map(|bit: u32| {
        at + Vec3::new(if bit & 1 == 0 { -1.0 } else { 1.0 }, if bit & 2 == 0 { -1.0 } else { 1.0 }, if bit & 4 == 0 { -1.0 } else { 1.0 })
    });
    assert!(corners.clone().any(|corner| near(corner, Vec3::new(3.0, -1.0, 1.0))), "{at:?}");
    run(&mut doc, "edit.undo", json!({}));
    // Kept to an axis, only the axis's part of the snap counts.
    run(&mut doc, "modal.begin", json!({ "kind": "move", "at": start, "axis": "z" }));
    run(&mut doc, "modal.pointer", json!({ "at": [corner[0] + 4.0, corner[1] - 3.0] }));
    run(&mut doc, "modal.confirm", json!({}));
    let kept = object(&doc, "Cube").location;
    assert!(
        kept.x == 0.0 && kept.y == 0.0 && ((kept.z - 2.0).abs() < 1e-3 || kept.z.abs() < 1e-3),
        "only Z: a corner level with the target's: {kept:?}"
    );
    // A face: the other cube's front face.
    run(&mut doc, "transform.set", json!({ "snapTarget": "face" }));
    let face = screen(&doc, Vec3::new(4.2, -1.0, 0.3));
    run(&mut doc, "modal.begin", json!({ "kind": "move", "at": start }));
    let answer = run(&mut doc, "modal.pointer", json!({ "at": face }));
    assert!((answer["snap"]["point"][1].as_f64().unwrap() + 1.0).abs() < 1e-3, "{answer}");
    run(&mut doc, "modal.cancel", json!({}));
    // Increments: 1 m steps.
    run(&mut doc, "transform.set", json!({ "snapTarget": "increment" }));
    run(&mut doc, "modal.begin", json!({ "kind": "move", "at": [400.0, 300.0] }));
    let answer = run(&mut doc, "modal.pointer", json!({ "at": [437.0, 300.0] }));
    let x = answer["values"][0].as_f64().unwrap();
    assert!((x - x.round()).abs() < 1e-6, "{answer}");
    run(&mut doc, "modal.cancel", json!({}));
    // Shift+Tab turns it off for the next transform too.
    run(&mut doc, "modal.begin", json!({ "kind": "move", "at": [400.0, 300.0] }));
    let answer = run(&mut doc, "modal.key", json!({ "key": "Tab", "shift": true }));
    assert_eq!(answer["snap"]["on"], false, "{answer}");
    run(&mut doc, "modal.cancel", json!({}));
    assert_eq!(doc.inspect().unwrap()["transform"]["snap"], false);
}
