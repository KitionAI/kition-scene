//! `kition-scene` served headless keeps the editing session (the selection and active object, the
//! mode and the edited mesh's selected vertices, the current frame, the viewport) out of what an
//! agent reads of the scene (`view.describe`, the `scene_inspect` details), and sets the editor
//! state one call reads and puts it back through the Kit Process Protocol's `context.get`,
//! `context.set` and `context.restore` (section 7a), so nothing an agent does depends on or leaves
//! behind the editor's selection, active object, mode, current frame or viewport (task card
//! C4-scene).
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

/// The headless kit as the `kition-scene` executable makes it.
fn kit() -> Headless<SceneDocument> {
    let info = KitInfo::from_manifest(MANIFEST, kition_scene_kit::SANDBOXED, kition_scene_kit::EDITOR_STATE).unwrap();
    Headless::<SceneDocument>::with_replica_port(info)
}

/// A workspace holding `Scene.kscene`: the starter scene (Cube selected and active) with two more
/// cubes, `Left` and `Right`, a Simple Deform modifier on the Cube (its `mode` is the scene's), a
/// 24 fps range of frames 1 to 120, and the Light's power keyed on frames 1 and 25, at frame 1.
fn scene() -> SceneDocument {
    let mut doc = SceneDocument::create("Scene.kscene", 320, 180).unwrap();
    for (name, x) in [("Left", -3), ("Right", 3)] {
        doc.execute("object.addCube", json!({ "name": name, "location": [x, 0, 0] })).unwrap();
    }
    doc.execute("modifier.add", json!({ "object": "Cube", "type": "simpleDeform", "mode": "bend" })).unwrap();
    doc.execute("frame.range", json!({ "start": 1, "end": 120, "fps": 24 })).unwrap();
    doc.execute("animation.insertKey", json!({ "objects": "Light", "properties": ["power"], "frame": 1 })).unwrap();
    doc.execute("animation.insertKey", json!({ "objects": "Light", "properties": ["power"], "frame": 25 })).unwrap();
    doc.execute("select.objects", json!({ "objects": "Cube" })).unwrap();
    doc.execute("frame.set", json!({ "frame": 1 })).unwrap();
    doc
}

/// [`scene`] saved in a fresh workspace, which the kit opens.
fn open(name: &str) -> (Headless<SceneDocument>, String, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("kition-scene-context-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let root = std::fs::canonicalize(root).unwrap();
    std::fs::write(root.join("Scene.kscene"), scene().save_file(None).unwrap()).unwrap();
    let mut kit = kit();
    let folder = HostFolder::new(&root).unwrap();
    kit.set_host(folder.host());
    let id = kit.open(OpenRequest { file: &folder.file("Scene.kscene").unwrap(), read_only: false, create: None }).unwrap().id;
    (kit, id, root)
}

fn run(kit: &mut Headless<SceneDocument>, id: &str, command: &str, value: Value, origin: Origin) -> Value {
    kit.execute(id, command, params(value), origin).unwrap_or_else(|error| panic!("{command}: {error:?}")).result.unwrap_or(Value::Null)
}

/// The id of the object named `name` in what the agent reads.
fn object_id(described: &Map<String, Value>, name: &str) -> u64 {
    described["objects"].as_array().unwrap().iter().find(|object| object["name"] == name).unwrap()["id"].as_u64().unwrap()
}

/// Every place `EDITOR_STATE` names is something the kit's `inspect()` answers and no part of what
/// an agent reads holds: a JSON Pointer at that place, a name (an animated object's `keyedNow`,
/// which is in each object's entry) wherever it is. The scene's own fields named like editor
/// state stay: the render's colour `view`, a modifier's `mode`, and the keyframes' frames.
#[test]
fn what_an_agent_reads_leaves_out_exactly_the_declared_editor_state() {
    let (mut kit, id, root) = open("declared");
    let described = Value::Object(kit.describe(&id).unwrap().state);
    let full = scene().inspect().unwrap();
    let text = serde_json::to_string(&described).unwrap();
    for key in kition_scene_kit::EDITOR_STATE {
        if key.starts_with('/') {
            assert!(full.pointer(key).is_some(), "{key} names nothing in inspect(): {full}");
            assert!(described.pointer(key).is_none(), "{key} is the editor's, not the scene's");
        } else {
            assert!(serde_json::to_string(&full).unwrap().contains(&format!("\"{key}\"")), "{key} names nothing in inspect(): {full}");
            assert!(!text.contains(&format!("\"{key}\"")), "{key} is the editor's, not the scene's");
        }
    }
    assert_eq!(described.pointer("/render/view"), Some(&json!("standard")), "the render's colour view is the scene's: {}", described["render"]);
    let cube = described["objects"].as_array().unwrap().iter().find(|object| object["name"] == "Cube").unwrap();
    assert_eq!(cube["modifiers"][0]["mode"], json!("bend"), "a modifier's mode is the scene's: {cube}");
    let keys = &described["animation"]["channels"][0]["keys"];
    assert_eq!(keys.as_array().map(|keys| keys.iter().map(|key| key["frame"].clone()).collect::<Vec<_>>()), Some(vec![json!(1.0), json!(25.0)]));
    let _ = std::fs::remove_dir_all(root);
}

/// A person orbiting the viewport, going to another frame, selecting objects or editing a mesh's
/// vertices changes nothing an agent reads of the scene.
#[test]
fn the_editors_state_changes_nothing_an_agent_reads() {
    let (mut kit, id, root) = open("unchanged");
    let before = kit.describe(&id).unwrap().state;
    run(&mut kit, &id, "view.orbit", json!({ "turn": 40, "tilt": 10 }), Origin::User);
    run(&mut kit, &id, "view.wireframe", json!({}), Origin::User);
    run(&mut kit, &id, "frame.set", json!({ "frame": 60 }), Origin::User);
    run(&mut kit, &id, "select.objects", json!({ "objects": ["Left", "Right"] }), Origin::User);
    run(&mut kit, &id, "mode.set", json!({ "mode": "edit", "object": "Left" }), Origin::User);
    run(&mut kit, &id, "mesh.select", json!({ "faces": [1] }), Origin::User);
    assert_eq!(kit.describe(&id).unwrap().state, before, "the editor's state changed what the agent reads of the scene");
    let _ = std::fs::remove_dir_all(root);
}

/// Through `context.set` and `context.restore`, an agent's `material.assign` acts on the three
/// objects `on` names (by name and by id), and afterwards the selection and the active object are
/// what the person left; neither request is an edit nor changes what the agent reads.
#[test]
fn a_step_on_named_objects_through_the_context_requests() {
    let (mut kit, id, root) = open("on");
    let described = kit.describe(&id).unwrap().state;
    let right = object_id(&described, "Right");
    run(&mut kit, &id, "material.new", json!({ "name": "Blue", "baseColor": [0, 0, 1], "assign": false }), Origin::Agent);
    let described = kit.describe(&id).unwrap().state;
    let before = kit.context_get(&id).unwrap();
    let dirty = kit.document(&id).unwrap().dirty;
    kit.context_set(&id, &params(json!({ "on": ["Cube", "Left", right] }))).unwrap();
    assert_eq!(kit.document(&id).unwrap().dirty, dirty, "setting the context is no edit");
    assert_eq!(kit.describe(&id).unwrap().state, described, "setting the context changed what the agent reads");
    run(&mut kit, &id, "material.assign", json!({ "material": "Blue" }), Origin::Agent);
    kit.context_restore(&id, &before).unwrap();
    assert_eq!(kit.context_get(&id).unwrap(), before, "the selection and the active object are what the person left");
    let after = kit.describe(&id).unwrap().state;
    let blue = after["materials"].as_array().unwrap().iter().find(|material| material["name"] == "Blue").unwrap()["id"].clone();
    let with_blue: Vec<&str> = after["objects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|object| object["material"] == blue)
        .map(|object| object["name"].as_str().unwrap())
        .collect();
    assert_eq!(with_blue, ["Cube", "Left", "Right"]);
    let _ = std::fs::remove_dir_all(root);
}

/// An agent's `animation.insertKey` at `at.time` 2 s keys frame 49 (frame 1 at 0 s, 24 fps, as
/// `file.render`'s `target.time` counts), and the current frame stays where the person left it.
#[test]
fn a_step_at_a_time_keys_there_and_leaves_the_current_frame() {
    let (mut kit, id, root) = open("time");
    run(&mut kit, &id, "frame.set", json!({ "frame": 7 }), Origin::User);
    let before = kit.context_get(&id).unwrap();
    kit.context_set(&id, &params(json!({ "time": 2.0 }))).unwrap();
    run(&mut kit, &id, "animation.insertKey", json!({ "objects": "Right", "properties": ["location"] }), Origin::Agent);
    kit.context_restore(&id, &before).unwrap();
    assert_eq!(kit.context_get(&id).unwrap(), before);
    let doc = kit.document(&id).unwrap();
    assert!(doc.dirty);
    let inspected = run(&mut kit, &id, "frame.set", json!({ "frame": 7 }), Origin::User);
    assert_eq!(inspected["frame"], json!(7));
    let described = kit.describe(&id).unwrap().state;
    let right = object_id(&described, "Right");
    let frames: Vec<Value> = described["animation"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|channel| channel["object"] == json!(right))
        .flat_map(|channel| channel["keys"].as_array().unwrap().iter().map(|key| key["frame"].clone()).collect::<Vec<_>>())
        .collect();
    assert_eq!(frames, [json!(49.0), json!(49.0), json!(49.0)], "{}", described["animation"]);
    let _ = std::fs::remove_dir_all(root);
}

/// An object, time or part the scene lacks is `invalid_params`, saying what is valid, and changes
/// nothing.
#[test]
fn what_the_scene_lacks_is_invalid_params() {
    let (mut kit, id, root) = open("refused");
    let before = kit.context_get(&id).unwrap();
    for (context, says) in [
        (json!({ "on": ["Nowhere"] }), "scene_inspect"),
        (json!({ "on": [9_999] }), "scene_inspect"),
        (json!({ "on": [true] }), "an object's `id` or `name`"),
        (json!({ "time": 5.0 }), "from 0 to 4.958 s"),
        (json!({ "part": { "slide": 1 } }), "has no parts"),
    ] {
        let error = format!("{:?}", kit.context_set(&id, &params(context.clone())).unwrap_err());
        assert!((error.contains("InvalidParams") || error.contains("invalid_params")) && error.contains(says), "{context}: {error}");
    }
    assert_eq!(kit.context_get(&id).unwrap(), before, "a refused context changes nothing");
    let _ = std::fs::remove_dir_all(root);
}

/// The agent's command list says what each command reads (`x-khp-reads`, Kit Process Protocol
/// section 6): the selection for a command that falls back to it, the time for one that falls back
/// to the current frame, nothing for one that takes everything it acts on.
#[test]
fn the_agent_sees_what_commands_read() {
    let (mut kit, id, root) = open("schemas");
    let commands = kit.agent_commands(Some(&id)).unwrap();
    let params = |wanted: &str| commands.iter().find(|command| command.id == wanted).map(|command| command.params.clone()).unwrap();
    assert_eq!(params("material.assign")["x-khp-reads"], json!(["selection"]));
    assert!(params("animation.insertKey")["x-khp-reads"].as_array().unwrap().contains(&json!("time")), "{}", params("animation.insertKey"));
    assert!(params("object.addCube").get("x-khp-reads").is_none(), "{}", params("object.addCube"));
    let _ = std::fs::remove_dir_all(root);
}
