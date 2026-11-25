//! The agent's view of a 3D scene through `kition-scene` served headless: the preview is the kit's
//! render command (`agent.render` in kit.json), run as the agent, and what `view.describe`
//! answers (the `scene_inspect` details) leaves out the editor's state: orbiting the viewport,
//! selecting objects or entering edit mode changes none of it, and rendering changes nothing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_headless::{Headless, HostFolder, KitInfo};
use kition_kit::KitDocument;
use kition_kit_server::protocol::Origin;
use kition_kit_server::{KitBackend, OpenRequest};
use kition_scene_kit::SceneDocument;
use serde_json::{Map, Value, json};

const MANIFEST: &str = include_str!("../../../kit.json");

fn params(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

/// The starter scene at a 320 x 180 render size, the cube keyed to move over the first second,
/// saved in a fresh workspace.
fn workspace(name: &str) -> (std::path::PathBuf, HostFolder) {
    let root = std::env::temp_dir().join(format!("kition-scene-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let root = std::fs::canonicalize(root).unwrap();
    let mut doc = SceneDocument::create("Scene.kscene", 640, 360).unwrap();
    doc.execute("render.set", json!({ "width": 320, "height": 180 })).unwrap();
    doc.execute("animation.insertKey", json!({ "objects": "Cube", "properties": ["location"], "frame": 1 })).unwrap();
    doc.execute("frame.set", json!({ "frame": 25 })).unwrap();
    doc.execute("object.transform", json!({ "objects": "Cube", "location": [2, 0, 0] })).unwrap();
    doc.execute("animation.insertKey", json!({ "objects": "Cube", "properties": ["location"], "frame": 25 })).unwrap();
    doc.execute("frame.set", json!({ "frame": 1 })).unwrap();
    std::fs::write(root.join("Scene.kscene"), doc.save_file(None).unwrap()).unwrap();
    let folder = HostFolder::new(&root).unwrap();
    (root, folder)
}

/// The headless kit as the `kition-scene` executable makes it.
fn kit() -> Headless<SceneDocument> {
    let info = KitInfo::from_manifest(MANIFEST, kition_scene_kit::SANDBOXED, kition_scene_kit::EDITOR_STATE).unwrap();
    Headless::<SceneDocument>::with_replica_port(info)
}

fn run(kit: &mut Headless<SceneDocument>, id: &str, command: &str, value: Value, origin: Origin) -> Value {
    kit.execute(id, command, params(value), origin).unwrap_or_else(|error| panic!("{command}: {error:?}")).result.unwrap_or(Value::Null)
}

#[test]
fn the_manifest_names_the_render_command() {
    let manifest: Value = serde_json::from_str(MANIFEST).unwrap();
    assert_eq!(manifest["agent"]["render"], json!("file.render"));
}

#[test]
fn the_agent_renders_a_view_at_a_time_and_changes_nothing() {
    let (root, folder) = workspace("agent-render");
    let mut kit = kit();
    kit.set_host(folder.host());
    let id = kit.open(OpenRequest { file: &folder.file("Scene.kscene").unwrap(), read_only: false, create: None }).unwrap().id;
    let described = kit.describe(&id).unwrap().state;
    let done = kit
        .execute(&id, "file.render", params(json!({ "target": { "view": "front", "time": 0.5 }, "scale": 1, "maxSize": 1024 })), Origin::Agent)
        .unwrap();
    assert!(!done.changed, "rendering is no edit");
    let answer = done.result.unwrap();
    assert!(answer["png"].as_str().is_some_and(|png| !png.is_empty()), "{answer}");
    assert_eq!((answer["width"].as_u64(), answer["height"].as_u64()), (Some(320), Some(180)));
    assert_eq!(answer["frames"][0]["frame"], json!(13));
    assert!(!kit.document(&id).unwrap().dirty, "rendering leaves the document clean");
    assert_eq!(kit.describe(&id).unwrap().state, described);
    // Out of range, the agent is told the range, as invalid parameters at a JSON pointer.
    let error = kit.execute(&id, "file.render", params(json!({ "target": { "time": 99 } })), Origin::Agent).unwrap_err();
    let shown = format!("{error:?}");
    assert!(shown.contains("InvalidParams") || shown.contains("invalid_params"), "{shown}");
    assert!(shown.contains("/target/time") && shown.contains("10.417 s"), "{shown}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_viewport_mode_and_selection_are_no_part_of_the_description() {
    let (root, folder) = workspace("describe-editor");
    let mut kit = kit();
    kit.set_host(folder.host());
    let id = kit.open(OpenRequest { file: &folder.file("Scene.kscene").unwrap(), read_only: false, create: None }).unwrap().id;
    let before = kit.describe(&id).unwrap().state;
    // The keyframes are the scene's: the editor-state keys take none of them away.
    let channel = &before["animation"]["channels"][0];
    let frames: Vec<Value> = channel["keys"].as_array().unwrap().iter().map(|key| key["frame"].clone()).collect();
    assert_eq!(frames, [json!(1.0), json!(25.0)], "{before:?}");
    run(&mut kit, &id, "view.orbit", json!({ "turn": 40, "tilt": 10 }), Origin::User);
    run(&mut kit, &id, "view.wireframe", json!({}), Origin::User);
    run(&mut kit, &id, "select.objects", json!({ "objects": ["Cube", "Light"] }), Origin::User);
    run(&mut kit, &id, "mode.set", json!({ "mode": "edit", "object": "Cube" }), Origin::User);
    run(&mut kit, &id, "mesh.select", json!({ "vertices": [0, 1] }), Origin::User);
    let after = kit.describe(&id).unwrap().state;
    assert_eq!(after, before, "the editor's viewport, mode or selection changed what the agent reads of the scene");
    let text = serde_json::to_string(&after).unwrap();
    for key in kition_scene_kit::EDITOR_STATE {
        assert!(!text.contains(&format!("\"{key}\"")), "{key} is the editor's, not the scene's");
    }
    let _ = std::fs::remove_dir_all(root);
}
