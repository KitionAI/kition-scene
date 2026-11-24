//! The camera's opening, motion blur, and volumes through commands: settings, refusals, undo,
//! saving, what inspect reports, and a path-traced render with each.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn starter() -> SceneDocument {
    let mut made = SceneDocument::create("Untitled.kscene", 1280, 720).unwrap();
    SceneDocument::open("t.kscene", &made.save_file(None).unwrap()).unwrap()
}

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn camera(doc: &SceneDocument) -> Value {
    doc.inspect().unwrap()["objects"].as_array().unwrap().iter().find(|object| object["name"] == "Camera").unwrap()["camera"].clone()
}

fn trace(doc: &mut SceneDocument) -> Value {
    run(doc, "render.set", json!({ "width": 24, "height": 14 }));
    run(doc, "render.trace.start", json!({ "samples": 2, "wait": true, "volumeBounces": 1 }))
}

#[test]
fn the_opening_takes_blades_a_turn_and_a_ratio() {
    let mut doc = starter();
    run(&mut doc, "camera.set", json!({ "object": "Camera", "depthOfField": true, "fStop": 0.5, "blades": 6, "bladeRotation": 15, "ratio": 1.4 }));
    let settings = camera(&doc);
    assert_eq!((settings["blades"].as_u64(), settings["bladeRotation"].as_f64(), settings["ratio"].as_f64()), (Some(6), Some(15.0), Some(1.4)));
    for blades in [1, 2, 65] {
        assert!(doc.execute("camera.set", json!({ "object": "Camera", "blades": blades })).is_err(), "{blades} blades");
    }
    assert_eq!(camera(&doc)["blades"].as_u64(), Some(6), "a refused change changes nothing");
    let reopened = SceneDocument::open("t.kscene", &doc.save_file(None).unwrap()).unwrap();
    assert_eq!(camera(&reopened)["ratio"].as_f64(), Some(1.4));
    assert_eq!(trace(&mut doc)["done"], true);
    run(&mut doc, "edit.undo", json!({}));
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(camera(&doc)["blades"].as_u64(), Some(0));
}

#[test]
fn motion_blur_is_a_render_setting() {
    let mut doc = starter();
    run(&mut doc, "render.set", json!({ "motionBlur": true, "shutter": 1.5, "shutterPosition": "start" }));
    let render = doc.inspect().unwrap()["render"].clone();
    assert_eq!(
        (render["motionBlur"].as_bool(), render["shutter"].as_f64(), render["shutterPosition"].as_str()),
        (Some(true), Some(1.5), Some("start"))
    );
    assert!(doc.execute("render.set", json!({ "shutter": 0 })).is_err());
    assert!(doc.execute("render.set", json!({ "shutterPosition": "middle" })).is_err());
    assert_eq!(doc.inspect().unwrap()["render"]["shutter"].as_f64(), Some(1.5));
    let reopened = SceneDocument::open("t.kscene", &doc.save_file(None).unwrap()).unwrap();
    assert_eq!(reopened.inspect().unwrap()["render"]["shutterPosition"], "start");
    // A keyed cube renders blurred, and the render leaves the scene as it was.
    run(&mut doc, "anim.insertKey", json!({ "objects": "Cube", "properties": ["location"], "frame": 1 }));
    run(&mut doc, "object.transform", json!({ "objects": "Cube", "location": [2, 0, 0] }));
    run(&mut doc, "anim.insertKey", json!({ "objects": "Cube", "properties": ["location"], "frame": 3 }));
    assert_eq!(trace(&mut doc)["done"], true);
}

#[test]
fn shader_groups_give_materials_and_the_world_their_volumes() {
    let mut doc = starter();
    let made = run(&mut doc, "nodes.newGroup", json!({ "tree": "shader", "material": "Material" }));
    let group = made["group"].clone();
    let material = doc.inspect().unwrap()["materials"][0].clone();
    assert_eq!(material["shader"], group);
    // A Volume Absorption into the Volume, the Principled BSDF out of the Surface: only a volume.
    let absorb = run(&mut doc, "nodes.addNode", json!({ "group": group, "kind": "volumeAbsorption", "values": { "Density": 2.0 } }))["node"].clone();
    run(&mut doc, "nodes.link", json!({ "group": group, "from": absorb, "fromSocket": "Volume", "to": made["output"], "toSocket": "Volume" }));
    run(&mut doc, "nodes.unlink", json!({ "group": group, "to": made["output"], "toSocket": "Surface" }));
    let world = run(&mut doc, "nodes.newGroup", json!({ "tree": "shader", "world": true, "name": "Mist" }));
    assert_eq!(doc.inspect().unwrap()["world"]["volume"], world["group"]);
    assert!(doc.execute("nodes.newGroup", json!({ "tree": "shader", "world": true, "material": "Material" })).is_err());
    assert_eq!(trace(&mut doc)["done"], true);
    // Only shader groups: a geometry group is refused.
    let geometry = run(&mut doc, "nodes.newGroup", json!({}))["group"].clone();
    assert!(doc.execute("world.set", json!({ "volume": geometry })).is_err());
    assert!(doc.execute("material.set", json!({ "material": "Material", "shader": geometry })).is_err());
    run(&mut doc, "world.set", json!({ "volume": null }));
    assert_eq!(doc.inspect().unwrap()["world"]["volume"], Value::Null);
    run(&mut doc, "world.set", json!({ "volume": "Mist" }));
    // Removing the group lets go of it.
    run(&mut doc, "nodes.removeGroup", json!({ "group": "Mist" }));
    assert_eq!(doc.inspect().unwrap()["world"]["volume"], Value::Null);
    run(&mut doc, "material.set", json!({ "material": "Material", "shader": null }));
    assert_eq!(doc.inspect().unwrap()["materials"][0]["shader"], Value::Null);
    let reopened = SceneDocument::open("t.kscene", &doc.save_file(None).unwrap()).unwrap();
    assert_eq!(reopened.inspect().unwrap()["nodeGroups"].as_array().unwrap().len(), 2);
}
