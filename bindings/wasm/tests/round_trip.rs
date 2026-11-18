//! Editing a scene end to end: commands build and change a scene, it saves to `.kscene`, reopens
//! from those bytes, and describes itself exactly as before (every object, mesh, modifier,
//! material, the animation and the render settings) and renders the same picture. The undo
//! history is left out of the comparison because a file does not keep it.
#![allow(clippy::unwrap_used, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params.clone()).unwrap_or_else(|error| panic!("{command} {params}: {error}"))
}

fn described(doc: &SceneDocument) -> Value {
    let mut value = doc.inspect().unwrap();
    let object = value.as_object_mut().unwrap();
    for history in ["canUndo", "canRedo", "undo", "redo"] {
        object.remove(history);
    }
    value
}

fn object<'a>(described: &'a Value, name: &str) -> &'a Value {
    described["objects"].as_array().unwrap().iter().find(|object| object["name"] == name).unwrap_or_else(|| panic!("no {name}"))
}

/// Saves, reopens from the saved bytes, and checks the reopened scene is the one saved.
fn saves_and_reopens(mut doc: SceneDocument) -> SceneDocument {
    let before = described(&doc);
    let picture = doc.render(0.0, 0.0, 0.25, 160, 90).unwrap();
    let saved = doc.save(None).unwrap();
    doc.mark_saved();
    assert!(!doc.dirty());
    let reopened = SceneDocument::open("Table.kscene", &saved).unwrap();
    assert_eq!(described(&reopened), before);
    assert_eq!(reopened.render(0.0, 0.0, 0.25, 160, 90).unwrap(), picture);
    let again = SceneDocument::open("Table.kscene", &reopened.save(None).unwrap()).unwrap();
    assert_eq!(described(&again), before);
    reopened
}

/// A table: a top, two legs and a ball, the default cube gone.
fn table() -> SceneDocument {
    let mut doc = SceneDocument::create("Table.kscene", 640, 360).unwrap();
    run(&mut doc, "edit.delete", json!({ "objects": "Cube" }));
    run(&mut doc, "add.cube", json!({ "name": "Top", "location": [0, 0, 0.75], "scale": [1, 0.6, 0.04] }));
    run(&mut doc, "add.cylinder", json!({ "name": "Leg 1", "radius": 0.04, "depth": 0.72, "vertices": 16, "location": [0.9, 0.5, 0.36] }));
    run(&mut doc, "add.cylinder", json!({ "name": "Leg 2", "radius": 0.04, "depth": 0.72, "vertices": 16, "location": [-0.9, 0.5, 0.36] }));
    run(&mut doc, "add.uvSphere", json!({ "name": "Ball", "location": [0, 0, 1.2] }));
    doc
}

#[test]
fn objects_and_their_transforms_survive_save_and_reopen() {
    let mut doc = table();
    run(&mut doc, "object.transform", json!({ "objects": ["Ball"], "location": [0.2, 0, 1.3], "rotation": [0, 0, 30], "scale": [0.5, 0.5, 0.5] }));
    run(&mut doc, "object.transform", json!({ "objects": ["Leg 2"], "rotation": [0, 8, 0] }));
    let reopened = saves_and_reopens(doc);
    let scene = described(&reopened);
    let names: Vec<&str> = scene["objects"].as_array().unwrap().iter().filter_map(|object| object["name"].as_str()).collect();
    assert!(!names.contains(&"Cube"), "{names:?}");
    let ball = object(&scene, "Ball");
    assert_eq!(ball["location"], json!([0.2, 0.0, 1.3]));
    assert_eq!(ball["rotation"], json!([0.0, 0.0, 30.0]));
    assert_eq!(ball["scale"], json!([0.5, 0.5, 0.5]));
}

#[test]
fn materials_survive_save_and_reopen() {
    let mut doc = table();
    run(&mut doc, "material.set", json!({ "material": "Material", "name": "Wood", "baseColor": "#8b5a2b", "roughness": 0.6 }));
    run(&mut doc, "material.assign", json!({ "material": "Wood", "objects": ["Top", "Leg 1", "Leg 2"] }));
    run(&mut doc, "select.objects", json!({ "objects": "Ball" }));
    let glass = run(&mut doc, "material.new", json!({}));
    run(
        &mut doc,
        "material.set",
        json!({ "material": glass["material"], "name": "Glass", "baseColor": "#d8f0ff", "roughness": 0.05, "transmission": 1, "ior": 1.45 }),
    );
    let reopened = saves_and_reopens(doc);
    let scene = described(&reopened);
    let materials = scene["materials"].as_array().unwrap();
    let wood = materials.iter().find(|material| material["name"] == "Wood").unwrap();
    assert_eq!((wood["roughness"].clone(), wood["users"].clone()), (json!(0.6), json!(3)));
    let glass = materials.iter().find(|material| material["name"] == "Glass").unwrap();
    assert_eq!((glass["transmission"].clone(), glass["ior"].clone()), (json!(1.0), json!(1.45)));
    assert_eq!(object(&scene, "Ball")["material"], glass["id"]);
}

#[test]
fn modifier_stacks_survive_save_and_reopen() {
    let mut doc = table();
    run(&mut doc, "modifier.add", json!({ "object": "Ball", "type": "subdivision", "levels": 2 }));
    run(&mut doc, "modifier.add", json!({ "object": "Top", "type": "bevel", "width": 0.02, "segments": 3 }));
    run(&mut doc, "modifier.set", json!({ "object": "Ball", "name": "Subdivision", "levels": 3 }));
    run(&mut doc, "modifier.add", json!({ "object": "Ball", "type": "mirror" }));
    run(&mut doc, "modifier.move", json!({ "object": "Ball", "name": "Mirror", "to": 0 }));
    let reopened = saves_and_reopens(doc);
    let scene = described(&reopened);
    let stack: Vec<&str> =
        object(&scene, "Ball")["modifiers"].as_array().unwrap().iter().map(|modifier| modifier["name"].as_str().unwrap()).collect();
    assert_eq!(stack, ["Mirror", "Subdivision"]);
    assert_eq!(object(&scene, "Ball")["modifiers"][1]["levels"], 3);
    assert_eq!(object(&scene, "Top")["modifiers"][0]["name"], "Bevel");
}

#[test]
fn mesh_edits_survive_save_and_reopen() {
    let mut doc = table();
    let before = object(&described(&doc), "Top")["mesh"].clone();
    run(&mut doc, "select.objects", json!({ "objects": "Top" }));
    run(&mut doc, "mode.set", json!({ "mode": "edit" }));
    run(&mut doc, "mesh.select", json!({ "faces": [5] }));
    run(&mut doc, "mesh.inset", json!({ "thickness": 0.05 }));
    run(&mut doc, "mesh.extrude", json!({ "distance": 0.1 }));
    run(&mut doc, "mode.set", json!({ "mode": "object" }));
    let edited = object(&described(&doc), "Top")["mesh"].clone();
    assert_ne!(edited["faces"], before["faces"], "the edits changed nothing: {edited}");
    let reopened = saves_and_reopens(doc);
    assert_eq!(object(&described(&reopened), "Top")["mesh"], edited);
}

#[test]
fn keyframed_motion_survives_save_and_reopen() {
    let mut doc = table();
    run(&mut doc, "select.objects", json!({ "objects": "Ball" }));
    run(&mut doc, "anim.insertKey", json!({ "properties": ["location"], "frame": 1 }));
    run(&mut doc, "object.transform", json!({ "objects": ["Ball"], "location": [0, 0, 2.4] }));
    run(&mut doc, "anim.insertKey", json!({ "properties": ["location"], "frame": 24 }));
    run(&mut doc, "frame.set", json!({ "frame": 12 }));
    let reopened = saves_and_reopens(doc);
    let scene = described(&reopened);
    assert_eq!(object(&scene, "Ball")["animated"], json!(["location"]));
}
