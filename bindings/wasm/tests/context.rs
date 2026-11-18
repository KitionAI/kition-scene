//! The editor-state hooks (`editor_state`, `set_context`, `restore_editor_state`) and an agent's
//! calls: a step names the objects, the time and the mesh elements it acts on, and leaves the
//! editor's selection, active object, mode, edited mesh elements, current frame and viewport as
//! they were (task card C4-scene).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::kdm::Origin;
use kition_kit::session::KitSessionDocument;
use kition_kit::{CallContext, KitDocument};
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

/// The starter scene (Cube selected and active) with two more cubes, `Left` and `Right`, frames 1
/// to 120 at 24 fps, at frame 7, the viewport orbited.
fn scene() -> SceneDocument {
    let mut doc = SceneDocument::create("Scene.kscene", 320, 180).unwrap();
    for (name, x) in [("Left", -3), ("Right", 3)] {
        doc.execute("add.cube", json!({ "name": name, "location": [x, 0, 0] })).unwrap();
    }
    doc.execute("frame.range", json!({ "start": 1, "end": 120, "fps": 24 })).unwrap();
    doc.execute("select.objects", json!({ "objects": "Cube" })).unwrap();
    doc.execute("frame.set", json!({ "frame": 7 })).unwrap();
    doc.execute("view.orbit", json!({ "turn": 30, "tilt": 5 })).unwrap();
    doc
}

fn agent() -> Origin {
    Origin::agent("agent:test")
}

fn id_of(doc: &SceneDocument, name: &str) -> u64 {
    doc.inspect().unwrap()["objects"].as_array().unwrap().iter().find(|object| object["name"] == name).unwrap()["id"].as_u64().unwrap()
}

fn object<'a>(inspected: &'a Value, name: &str) -> &'a Value {
    inspected["objects"].as_array().unwrap().iter().find(|object| object["name"] == name).unwrap()
}

/// What a person sees of the editor: the selection, the mode, the current frame, the viewport.
fn editor(doc: &SceneDocument) -> Value {
    let inspected = doc.inspect().unwrap();
    json!({ "selection": inspected["selection"], "mode": inspected["mode"], "frame": inspected["animation"]["frame"], "view": inspected["view"] })
}

fn context(value: Value) -> CallContext {
    serde_json::from_value(value).unwrap()
}

/// kition-sdk's check with contexts the document is already in (its `inspect()` includes the
/// editor state, so a context that moves it would change what inspect answers): setting them and
/// restoring change nothing.
#[test]
fn the_sdk_check_passes_with_the_current_state() {
    let mut doc = scene();
    let cube = id_of(&doc, "Cube");
    kition_kit::testing::check_context(&mut doc, &context(json!({ "on": [cube], "time": 0.25 })), kition_scene_kit::EDITOR_STATE).unwrap();
    kition_kit::testing::check_context(&mut doc, &context(json!({ "on": ["Cube"] })), kition_scene_kit::EDITOR_STATE).unwrap();
    kition_kit::testing::check_context(&mut doc, &CallContext::default(), kition_scene_kit::EDITOR_STATE).unwrap();
}

/// A real move: `on` becomes the selection (the last named the active object) and `time` the
/// current frame (frame 1 at 0 s, so 2 s at 24 fps is frame 49), without an edit; restoring puts
/// back the selection, the frame and the viewport exactly.
#[test]
fn setting_the_context_moves_only_the_editor_and_restoring_puts_it_back() {
    let mut doc = scene();
    let (left, right) = (id_of(&doc, "Left"), id_of(&doc, "Right"));
    let before = editor(&doc);
    let saved = doc.editor_state();
    let (revision, dirty) = (doc.revision(), doc.dirty());
    doc.set_context(&context(json!({ "on": ["Left", right], "time": 2.0 }))).unwrap();
    assert_eq!((doc.revision(), doc.dirty()), (revision, dirty), "setting the context is no edit");
    let inspected = doc.inspect().unwrap();
    assert_eq!(inspected["selection"]["objects"], json!([left, right]));
    assert_eq!(inspected["selection"]["active"], json!(right));
    assert_eq!(inspected["animation"]["frame"], json!(49));
    doc.restore_editor_state(&saved).unwrap();
    assert_eq!((doc.revision(), doc.dirty()), (revision, dirty), "restoring is no edit");
    assert_eq!(editor(&doc), before);
    assert_eq!(doc.editor_state(), saved);
}

/// `object.parent` driven by `on` (the host sets the selection the command falls back to) parents
/// exactly those objects, and the selection and the active object are what the person left.
#[test]
fn a_selection_command_driven_by_on_changes_those_objects_and_leaves_the_selection() {
    let mut doc = scene();
    let cube = id_of(&doc, "Cube");
    let before = editor(&doc);
    let saved = doc.editor_state();
    doc.set_context(&context(json!({ "on": ["Left", "Right"] }))).unwrap();
    doc.execute_as("object.parent", json!({ "parent": "Cube" }), &agent()).unwrap();
    doc.restore_editor_state(&saved).unwrap();
    assert_eq!(editor(&doc), before);
    let inspected = doc.inspect().unwrap();
    assert_eq!(object(&inspected, "Left")["parent"], json!(cube));
    assert_eq!(object(&inspected, "Right")["parent"], json!(cube));
    assert!(object(&inspected, "Cube").get("parent").is_none());
}

/// `anim.insertKey` driven by `at.time` keys at that time (2 s: frame 49) and leaves the current
/// frame where the person left it.
#[test]
fn a_frame_command_driven_by_at_time_keys_there_and_leaves_the_current_frame() {
    let mut doc = scene();
    let saved = doc.editor_state();
    doc.set_context(&context(json!({ "time": 2.0 }))).unwrap();
    doc.execute_as("anim.insertKey", json!({ "objects": "Right", "properties": ["location"] }), &agent()).unwrap();
    doc.restore_editor_state(&saved).unwrap();
    let inspected = doc.inspect().unwrap();
    assert_eq!(inspected["animation"]["frame"], json!(7));
    let right = id_of(&doc, "Right");
    let frames: Vec<&Value> = inspected["animation"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|channel| channel["object"] == json!(right))
        .map(|channel| &channel["keys"][0]["frame"])
        .collect();
    assert_eq!(frames, [&json!(49.0), &json!(49.0), &json!(49.0)]);
}

/// An agent's command that names its objects leaves the selection and the active object as they
/// were (a person's duplicate selects the copies, as Blender's does), also when it fails.
#[test]
fn an_agents_command_given_its_objects_leaves_the_editor() {
    let mut doc = scene();
    let before = editor(&doc);
    let copied = doc.execute_as("edit.duplicate", json!({ "objects": ["Left", "Right"], "offset": [0, 4, 0] }), &agent()).unwrap();
    assert_eq!(copied["objects"].as_array().unwrap().len(), 2);
    assert_eq!(editor(&doc), before, "the agent's duplicate moved the selection");
    assert!(doc.execute_as("edit.duplicate", json!({ "objects": ["Left", "Nowhere"] }), &agent()).is_err());
    assert_eq!(editor(&doc), before);
    // A person's duplicate selects the copies.
    let mine = doc.execute("edit.duplicate", json!({ "objects": "Left" })).unwrap();
    assert_eq!(doc.inspect().unwrap()["selection"]["objects"], mine["objects"]);
    // Navigation keeps its effect for the agent too.
    doc.execute_as("select.objects", json!({ "objects": "Right" }), &agent()).unwrap();
    assert_eq!(doc.inspect().unwrap()["selection"]["active"], json!(id_of(&doc, "Right")));
}

/// An agent extrudes a face of `Left` in one step, naming the mesh and the face in `elements`, from
/// object mode: the mesh gains the faces, and the mode, the selection and the edited elements are
/// as they were.
#[test]
fn an_edit_mode_operation_named_with_elements_runs_in_one_step() {
    let mut doc = scene();
    let before = editor(&doc);
    let faces = object(&doc.inspect().unwrap(), "Left")["mesh"]["faces"].as_u64().unwrap();
    doc.execute_as("mesh.extrude", json!({ "elements": { "object": "Left", "faces": [0] }, "distance": 0.5 }), &agent()).unwrap();
    assert_eq!(editor(&doc), before);
    assert_eq!(object(&doc.inspect().unwrap(), "Left")["mesh"]["faces"].as_u64().unwrap(), faces + 4);
    // In the editor's own edit mode on another mesh, too.
    doc.execute("mode.set", json!({ "mode": "edit", "object": "Cube" })).unwrap();
    doc.execute("mesh.select", json!({ "vertices": [0, 1] })).unwrap();
    let editing = editor(&doc);
    doc.execute_as("mesh.extrude", json!({ "elements": { "object": "Right", "faces": [2] } }), &agent()).unwrap();
    assert_eq!(editor(&doc), editing);
    assert_eq!(object(&doc.inspect().unwrap(), "Right")["mesh"]["faces"].as_u64().unwrap(), faces + 4);
}

/// Elements the mesh lacks, or a mesh that is not one, are refused saying what is valid, and change
/// nothing.
#[test]
fn elements_the_mesh_lacks_are_refused() {
    let mut doc = scene();
    let (before, scene_before) = (editor(&doc), doc.inspect().unwrap());
    for (elements, says) in [
        (json!({ "object": "Left", "faces": [99] }), "faces 0 to 5"),
        (json!({ "object": "Left", "vertices": [8] }), "vertices 0 to 7"),
        (json!({ "object": "Light", "faces": [0] }), "not a mesh"),
        (json!({ "object": "Nowhere" }), "no object named Nowhere"),
        (json!({ "faces": [0] }), "`elements.object`"),
    ] {
        let error = doc.execute_as("mesh.extrude", json!({ "elements": elements }), &agent()).unwrap_err();
        assert!(error.contains(says), "{elements}: {error}");
        assert_eq!(editor(&doc), before);
    }
    assert_eq!(doc.inspect().unwrap(), scene_before, "a refused step changed the scene");
}

/// The run an agent makes of a mode command (enter edit mode, select, extrude) is put back by the
/// host after the run, also when a later step fails: mode, element selection and object selection
/// are as they were.
#[test]
fn an_edit_mode_run_is_put_back_when_a_later_step_fails() {
    let mut doc = scene();
    let before = editor(&doc);
    let saved = doc.editor_state();
    doc.execute_as("mode.set", json!({ "mode": "edit", "object": "Left" }), &agent()).unwrap();
    doc.execute_as("mesh.select", json!({ "faces": [1] }), &agent()).unwrap();
    doc.execute_as("mesh.extrude", json!({ "distance": 0.25 }), &agent()).unwrap();
    assert!(doc.execute_as("mesh.select", json!({ "vertices": [9_999] }), &agent()).is_err());
    doc.restore_editor_state(&saved).unwrap();
    assert_eq!(editor(&doc), before);
    assert_eq!(doc.editor_state(), saved);
}

/// Restoring drops what the run deleted: the selection keeps the objects that are still there.
#[test]
fn restoring_drops_what_the_run_deleted() {
    let mut doc = scene();
    doc.execute("select.objects", json!({ "objects": ["Left", "Right"] })).unwrap();
    let right = id_of(&doc, "Right");
    let saved = doc.editor_state();
    doc.execute_as("edit.delete", json!({ "objects": "Left" }), &agent()).unwrap();
    doc.restore_editor_state(&saved).unwrap();
    let inspected = doc.inspect().unwrap();
    assert_eq!(inspected["selection"]["objects"], json!([right]));
    assert_eq!(inspected["selection"]["active"], json!(right));
}

/// An object or a time the scene lacks, and a part (a scene has none), are refused saying what is
/// valid; nothing changes.
#[test]
fn what_the_scene_lacks_is_refused_and_changes_nothing() {
    let mut doc = scene();
    let saved = doc.editor_state();
    for (given, says) in [
        (json!({ "on": ["Nowhere"] }), "no object named Nowhere"),
        (json!({ "on": [9_999] }), "no object 9999"),
        (json!({ "on": ["Left", 9_999], "time": 1.0 }), "no object 9999"),
        (json!({ "time": 5.0 }), "from 0 to 4.958 s"),
        (json!({ "time": -1.0 }), "from 0 to 4.958 s"),
        (json!({ "part": { "slide": 1 } }), "has no parts"),
    ] {
        let error = doc.set_context(&context(given.clone())).unwrap_err();
        assert!(error.contains(says) && error.contains("invalid-params"), "{given}: {error}");
        assert_eq!(doc.editor_state(), saved, "{given} changed the editor");
    }
}
