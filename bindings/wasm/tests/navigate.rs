//! The viewport's navigation commands (plan scene-usable-by-hand, phase 4): the back, left and
//! bottom views, and the camera view's frame zoomed and moved without leaving the camera. All of
//! them are the person's view state: no undo step, nothing unsaved.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn starter() -> SceneDocument {
    let mut doc = SceneDocument::create("View.kscene", 96, 64).unwrap();
    doc.reopen("View.kscene").unwrap()
}

fn view(doc: &SceneDocument) -> Value {
    doc.inspect().unwrap()["view"].clone()
}

#[test]
fn back_left_and_bottom_look_along_their_axes_orthographically() {
    let mut doc = starter();
    for (command, tilt, turn) in [("view.back", 90.0, 180.0), ("view.left", 90.0, 270.0), ("view.bottom", 180.0, 0.0)] {
        run(&mut doc, command, json!({}));
        let now = view(&doc);
        assert_eq!(
            (now["tilt"].as_f64(), now["turn"].as_f64(), now["perspective"].as_bool()),
            (Some(tilt), Some(turn), Some(false)),
            "{command}: {now}"
        );
    }
    assert!(!doc.dirty());
    assert_eq!(doc.inspect().unwrap()["canUndo"], false);
}

#[test]
fn the_camera_frame_zooms_and_moves_in_the_camera_view() {
    let mut doc = starter();
    run(&mut doc, "view.camera", json!({}));
    let camera = view(&doc)["camera"].clone();
    assert!(camera.is_number());
    let before = doc.gl_scene()["camera"]["frame"].clone();
    run(&mut doc, "view.set", json!({ "cameraZoom": 2.0, "cameraOffset": [0.25, -0.5] }));
    let now = view(&doc);
    assert_eq!(now["camera"], camera, "zooming the frame keeps the camera view");
    assert_eq!((now["cameraZoom"].as_f64(), now["cameraOffset"].clone()), (Some(2.0), json!([0.25, -0.5])));
    let after = doc.gl_scene()["camera"]["frame"].clone();
    let width = |frame: &Value| frame[2].as_f64().unwrap() - frame[0].as_f64().unwrap();
    assert!((width(&after) - 2.0 * width(&before)).abs() < 1e-3, "{before} → {after}");
    // Out of range: clamped.
    run(&mut doc, "view.set", json!({ "cameraZoom": 1000.0 }));
    assert_eq!(view(&doc)["cameraZoom"].as_f64(), Some(16.0));
    // An orbit leaves the camera view; the frame's zoom is kept for the next time.
    run(&mut doc, "view.set", json!({ "turn": 10.0 }));
    assert!(view(&doc)["camera"].is_null());
    assert!(!doc.dirty());
    assert_eq!(doc.inspect().unwrap()["canUndo"], false);
    // A view state from before the frame could zoom still restores.
    let mut state = run(&mut doc, "view.state", json!({}))["state"].clone();
    let view_state = state["view"].as_object_mut().unwrap();
    view_state.remove("cameraZoom");
    view_state.remove("cameraOffset");
    assert_eq!(run(&mut doc, "view.restore", json!({ "state": state }))["restored"], true);
    assert_eq!(view(&doc)["cameraZoom"].as_f64(), Some(1.0));
}
