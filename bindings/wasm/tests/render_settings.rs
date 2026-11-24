//! Glass and alpha on materials, depth of field on cameras, and the world image, through
//! commands: settings, undo, saving, and what inspect reports.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use kition_scene_model::hdr;
use serde_json::{Value, json};

fn starter() -> SceneDocument {
    let mut made = SceneDocument::create("Untitled.kscene", 1280, 720).unwrap();
    SceneDocument::open("t.kscene", &made.save_file(None).unwrap()).unwrap()
}

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let triple = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        for index in 0..4 {
            out.push(if index <= chunk.len() { TABLE[((triple >> (18 - 6 * index)) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

#[test]
fn glass_and_alpha_are_material_settings() {
    let mut doc = starter();
    run(&mut doc, "material.set", json!({ "material": "Material", "transmission": 1, "ior": 1.33, "alphaMode": "clip", "alphaCutoff": 0.3 }));
    let material = &doc.inspect().unwrap()["materials"][0];
    assert_eq!((material["transmission"].as_f64(), material["ior"].as_f64()), (Some(1.0), Some(1.33)));
    assert_eq!((material["alphaMode"].as_str(), material["alphaCutoff"].as_f64()), (Some("clip"), Some(0.3)));
    assert!(doc.execute("material.set", json!({ "material": "Material", "alphaMode": "dither" })).is_err());
    let reopened = SceneDocument::open("t.kscene", &doc.save_file(None).unwrap()).unwrap();
    assert_eq!(reopened.inspect().unwrap()["materials"][0]["ior"].as_f64(), Some(1.33));
}

#[test]
fn depth_of_field_focuses_on_an_object_until_it_is_gone() {
    let mut doc = starter();
    run(&mut doc, "camera.set", json!({ "object": "Camera", "depthOfField": true, "fStop": 1.4, "focusObject": "Cube" }));
    let camera = |doc: &SceneDocument| {
        doc.inspect().unwrap()["objects"].as_array().unwrap().iter().find(|object| object["name"] == "Camera").unwrap()["camera"].clone()
    };
    let cube = doc.inspect().unwrap()["objects"][0]["id"].clone();
    assert_eq!(camera(&doc)["depthOfField"], true);
    assert_eq!(camera(&doc)["fStop"].as_f64(), Some(1.4));
    assert_eq!(camera(&doc)["focusObject"], cube);
    assert!(doc.execute("camera.set", json!({ "object": "Camera", "focusObject": "Camera" })).is_err());
    run(&mut doc, "edit.delete", json!({ "objects": "Cube" }));
    assert_eq!(camera(&doc)["focusObject"], Value::Null);
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(camera(&doc)["focusObject"], cube);
}

#[test]
fn the_world_image_is_kept_saved_and_cleared() {
    let mut doc = starter();
    let sky = hdr::encode(8, 4, &[[0.2, 0.4, 2.0]; 32]);
    let set = run(&mut doc, "world.setImage", json!({ "base64": base64(&sky), "name": "Sky.hdr", "rotation": 90, "strength": 1.5 }));
    assert_eq!((set["width"].as_u64(), set["height"].as_u64()), (Some(8), Some(4)));
    let world = doc.inspect().unwrap()["world"].clone();
    assert_eq!(world["image"], set["image"]);
    assert_eq!((world["rotation"].as_f64(), world["strength"].as_f64()), (Some(90.0), Some(1.5)));
    // A render through the world image works, and the file keeps the image.
    run(&mut doc, "render.set", json!({ "width": 32, "height": 18 }));
    run(&mut doc, "render.image", json!({ "samples": 1 }));
    let bytes = doc.save_file(None).unwrap();
    let reopened = SceneDocument::open("t.kscene", &bytes).unwrap();
    assert_eq!(reopened.inspect().unwrap()["world"]["image"], set["image"]);
    run(&mut doc, "world.setImage", json!({ "clear": true }));
    assert_eq!(doc.inspect().unwrap()["world"]["image"], Value::Null);
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(doc.inspect().unwrap()["world"]["image"], set["image"]);
    assert!(doc.execute("world.setImage", json!({ "base64": base64(b"not an image") })).is_err());
    assert!(doc.execute("world.setImage", json!({})).is_err());
}
