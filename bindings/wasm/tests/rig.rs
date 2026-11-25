//! Rigging through commands, as the agent and the editor's menus drive it: an armature built in
//! edit mode, a column parented with automatic weights, a keyed pose, shape keys, and the file
//! keeping all of it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_kit::tools::{KitTools, PointerAction, ToolMods, ToolPointer};
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn object<'a>(inspected: &'a Value, name: &str) -> &'a Value {
    inspected["objects"].as_array().unwrap().iter().find(|object| object["name"] == name).unwrap_or_else(|| panic!("no {name}"))
}

fn rigged() -> SceneDocument {
    let mut doc = SceneDocument::create("Rig.kscene", 1280, 720).unwrap();
    run(&mut doc, "edit.delete", json!({ "objects": "Cube" }));
    run(&mut doc, "object.addCylinder", json!({ "name": "Column", "vertices": 8, "radius": 0.2, "depth": 2, "location": [0, 0, 1] }));
    run(&mut doc, "object.addArmature", json!({}));
    run(&mut doc, "mode.armature", json!({}));
    run(&mut doc, "bone.select", json!({ "bones": "Bone" }));
    run(&mut doc, "bone.extrude", json!({}));
    run(&mut doc, "mode.armature", json!({}));
    run(&mut doc, "select.objects", json!({ "objects": ["Column", "Armature"] }));
    run(&mut doc, "object.parentArmature", json!({ "weights": "automatic" }));
    doc
}

#[test]
fn an_armature_is_built_bound_and_posed() {
    let mut doc = rigged();
    let inspected = doc.inspect().unwrap();
    let bones = object(&inspected, "Armature")["armature"]["bones"].as_array().unwrap().clone();
    assert_eq!(bones.len(), 2);
    assert_eq!(bones[1]["name"], "Bone.001");
    assert_eq!(bones[1]["parent"], "Bone");
    assert_eq!(bones[1]["connected"], true);
    assert_eq!(bones[1]["tail"], json!([0.0, 0.0, 2.0]));
    let column = object(&inspected, "Column");
    assert_eq!(column["parent"], object(&inspected, "Armature")["id"]);
    assert_eq!(column["modifiers"][0]["type"], "armature");
    assert_eq!(column["vertexGroups"].as_array().unwrap().len(), 2);

    // Pose the upper bone and key it on two frames.
    run(&mut doc, "mode.pose", json!({ "object": "Armature" }));
    assert_eq!(doc.inspect().unwrap()["mode"]["mode"], "pose");
    run(&mut doc, "bone.select", json!({ "bones": "Bone.001" }));
    run(&mut doc, "pose.insertKey", json!({ "frame": 1 }));
    run(&mut doc, "pose.set", json!({ "rotation": [90, 0, 0] }));
    run(&mut doc, "pose.insertKey", json!({ "frame": 21 }));
    let channels = doc.inspect().unwrap()["animation"]["channels"].as_array().unwrap().clone();
    assert_eq!(channels.iter().filter(|channel| channel["path"] == "Bone.001").count(), 9);
    let posed_top = |doc: &mut SceneDocument, frame: i32| {
        run(doc, "frame.set", json!({ "frame": frame }));
        let dimensions = object(&doc.inspect().unwrap(), "Column")["dimensions"].clone();
        dimensions[1].as_f64().unwrap()
    };
    let rest = posed_top(&mut doc, 1);
    let bent = posed_top(&mut doc, 21);
    assert!(rest < 0.5 && bent > 0.8, "the column bends along Y: {rest} then {bent}");
    // Keying in pose mode does not reach the object-level keys; object commands wait.
    assert!(doc.execute("animation.insertKey", json!({})).is_err());
    assert!(doc.execute("object.addCube", json!({})).is_err());
    run(&mut doc, "mode.pose", json!({ "on": false }));
    assert_eq!(doc.inspect().unwrap()["mode"]["mode"], "object");

    // The file keeps the bones, pose keys, groups, and modifier.
    let bytes = doc.save_file(None).unwrap();
    let mut back = SceneDocument::open("Rig.kscene", &bytes).unwrap();
    assert_eq!(back.save_file(None).unwrap(), bytes);
    run(&mut back, "frame.set", json!({ "frame": 21 }));
    assert!(object(&back.inspect().unwrap(), "Column")["dimensions"][1].as_f64().unwrap() > 0.8);
}

#[test]
fn bones_are_edited_deleted_and_undone() {
    let mut doc = rigged();
    run(&mut doc, "select.objects", json!({ "objects": "Armature" }));
    run(&mut doc, "mode.armature", json!({}));
    run(&mut doc, "bone.set", json!({ "bone": "Bone.001", "name": "Upper", "roll": 30 }));
    let inspected = doc.inspect().unwrap();
    assert_eq!(object(&inspected, "Armature")["armature"]["bones"][1]["roll"], 30.0);
    // The column's group follows the bone's new name.
    let groups: Vec<String> =
        object(&inspected, "Column")["vertexGroups"].as_array().unwrap().iter().map(|group| group["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(groups, ["Bone", "Upper"]);
    run(&mut doc, "bone.select", json!({ "bones": "Upper" }));
    run(&mut doc, "edit.duplicate", json!({}));
    assert_eq!(object(&doc.inspect().unwrap(), "Armature")["armature"]["bones"].as_array().unwrap().len(), 3);
    run(&mut doc, "edit.delete", json!({}));
    assert_eq!(object(&doc.inspect().unwrap(), "Armature")["armature"]["bones"].as_array().unwrap().len(), 2);
    // Undo brings the copy back; the armature object itself was never touched.
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(object(&doc.inspect().unwrap(), "Armature")["armature"]["bones"].as_array().unwrap().len(), 3);
    assert!(doc.execute("bone.set", json!({ "bone": "Upper", "parent": "Nope" })).is_err());
    assert!(doc.execute("bone.add", json!({ "head": [0, 0, 0], "tail": [0, 0, 0] })).is_err(), "a bone without length is refused");
}

#[test]
fn the_pose_tool_rotates_a_bone_in_one_step() {
    let mut doc = rigged();
    run(&mut doc, "view.resize", json!({ "width": 640, "height": 480 }));
    run(&mut doc, "view.front", json!({}));
    run(&mut doc, "view.frameAll", json!({}));
    run(&mut doc, "mode.pose", json!({ "object": "Armature" }));
    run(&mut doc, "bone.select", json!({ "bones": "Bone.001" }));
    doc.select_tool("rotate", 1.0).unwrap();
    let undo_before = doc.inspect().unwrap()["undo"].clone();
    let event = |action, x: f64, y: f64| ToolPointer { action, x, y, mods: ToolMods::default(), pressure: 1.0, zoom: 1.0 };
    doc.pointer(&event(PointerAction::Down, 420.0, 240.0)).unwrap();
    doc.pointer(&event(PointerAction::Drag, 400.0, 160.0)).unwrap();
    doc.pointer(&event(PointerAction::Up, 400.0, 160.0)).unwrap();
    let inspected = doc.inspect().unwrap();
    let rotation = object(&inspected, "Armature")["armature"]["bones"][1]["pose"]["rotation"].clone();
    assert!(rotation.as_array().unwrap().iter().any(|angle| angle.as_f64().unwrap().abs() > 5.0), "{rotation}");
    assert_ne!(inspected["undo"], undo_before);
    run(&mut doc, "edit.undo", json!({}));
    let rotation = object(&doc.inspect().unwrap(), "Armature")["armature"]["bones"][1]["pose"]["rotation"].clone();
    assert_eq!(rotation, json!([0.0, 0.0, 0.0]));
}

#[test]
fn shape_keys_and_vertex_groups() {
    let mut doc = SceneDocument::create("Shapes.kscene", 1280, 720).unwrap();
    run(&mut doc, "shapeKey.add", json!({ "object": "Cube", "name": "Tall" }));
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "mode.set", json!({ "mode": "edit" }));
    run(&mut doc, "mesh.select", json!({ "faces": [5] }));
    run(&mut doc, "shapeKey.offset", json!({ "name": "Tall", "offset": [0, 0, 1] }));
    run(&mut doc, "vertexGroup.assign", json!({ "name": "Top", "weight": 0.5 }));
    run(&mut doc, "mode.set", json!({ "mode": "object" }));
    run(&mut doc, "shapeKey.set", json!({ "object": "Cube", "name": "Tall", "value": 1 }));
    let cube = object(&doc.inspect().unwrap(), "Cube").clone();
    assert_eq!(cube["dimensions"][2], 3.0);
    assert_eq!(cube["vertexGroups"][0], json!({ "name": "Top", "vertices": 4 }));
    run(&mut doc, "shapeKey.insertKey", json!({ "object": "Cube", "name": "Tall", "frame": 10 }));
    run(&mut doc, "shapeKey.set", json!({ "object": "Cube", "name": "Tall", "value": 0 }));
    run(&mut doc, "shapeKey.insertKey", json!({ "object": "Cube", "name": "Tall", "frame": 1 }));
    run(&mut doc, "frame.set", json!({ "frame": 10 }));
    assert_eq!(object(&doc.inspect().unwrap(), "Cube")["dimensions"][2], 3.0);
    run(&mut doc, "frame.set", json!({ "frame": 1 }));
    assert_eq!(object(&doc.inspect().unwrap(), "Cube")["dimensions"][2], 2.0);
    // Shape keys and groups survive an edit that adds vertices.
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "mode.set", json!({ "mode": "edit" }));
    run(&mut doc, "mesh.select", json!({ "faces": [5] }));
    run(&mut doc, "mesh.extrude", json!({ "distance": 1 }));
    run(&mut doc, "mode.set", json!({ "mode": "object" }));
    let cube = object(&doc.inspect().unwrap(), "Cube").clone();
    assert_eq!(cube["mesh"]["vertices"], 12);
    assert_eq!(cube["vertexGroups"][0]["vertices"], 8);
    run(&mut doc, "shapeKey.remove", json!({ "object": "Cube", "name": "Tall" }));
    assert!(doc.inspect().unwrap()["animation"]["channels"].as_array().unwrap().is_empty(), "the key's channel went with it");
    assert!(doc.execute("shapeKey.add", json!({ "object": "Cube", "offsets": [[0, 0, 1]] })).is_err());
}

#[test]
fn the_graph_editor_moves_retimes_and_deletes_keys() {
    let mut doc = SceneDocument::create("Graph.kscene", 1280, 720).unwrap();
    run(&mut doc, "animation.insertKey", json!({ "objects": "Cube", "properties": ["location"], "frame": 1 }));
    run(&mut doc, "object.transform", json!({ "objects": "Cube", "location": [4, 0, 0] }));
    run(&mut doc, "animation.insertKey", json!({ "objects": "Cube", "properties": ["location"], "frame": 21 }));
    let cube = object(&doc.inspect().unwrap(), "Cube")["id"].clone();
    run(&mut doc, "graph.moveKey", json!({ "target": cube, "property": "location", "index": 0, "frame": 21, "toFrame": 11, "value": 2 }));
    run(&mut doc, "graph.setInterpolation", json!({ "target": cube, "property": "location", "index": 0, "frame": 1, "interpolation": "linear" }));
    run(&mut doc, "frame.set", json!({ "frame": 6 }));
    assert_eq!(object(&doc.inspect().unwrap(), "Cube")["location"][0], 1.0);
    assert!(
        doc.execute("graph.moveKey", json!({ "target": cube, "property": "location", "frame": 11, "toFrame": 1 })).is_err(),
        "frames stay distinct"
    );
    run(&mut doc, "graph.deleteKey", json!({ "target": cube, "property": "location", "index": 0, "frame": 11 }));
    let channels = doc.inspect().unwrap()["animation"]["channels"].clone();
    assert_eq!(channels[0]["keys"].as_array().unwrap().len(), 1);
}
