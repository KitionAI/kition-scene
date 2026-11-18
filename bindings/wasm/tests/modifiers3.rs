//! Shrinkwrap, Lattice, Decimate's un-subdivide and planar ways, and Displace with an image
//! through commands, as the agent and the editor drive them: lattices added, resized, and their
//! points moved by index and in lattice edit mode; the modifiers set by `operand`; and all of it
//! kept by the file, including files the released kits wrote.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_kit::tools::{KitTools, PointerAction, ToolMods, ToolPointer};
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn object(doc: &SceneDocument, name: &str) -> Value {
    let inspected = doc.inspect().unwrap();
    inspected["objects"].as_array().unwrap().iter().find(|object| object["name"] == name).cloned().unwrap_or_else(|| panic!("no {name}"))
}

fn dimensions(doc: &SceneDocument, name: &str) -> [f64; 3] {
    let found = object(doc, name);
    [0, 1, 2].map(|axis| found["dimensions"][axis].as_f64().unwrap())
}

/// A small grey PNG, base64 encoded: every pixel `value`.
fn png_base64(value: u8) -> String {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(&[value; 4]).unwrap();
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let triple = chunk.iter().enumerate().fold(0u32, |sum, (index, byte)| sum | (u32::from(*byte) << (16 - 8 * index)));
        for slot in 0..4 {
            if slot <= chunk.len() {
                out.push(char::from(ALPHABET[((triple >> (18 - 6 * slot)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[test]
fn a_lattice_bends_a_mesh_and_the_file_keeps_it() {
    let mut doc = SceneDocument::create("Lattice.kscene", 1280, 720).unwrap();
    run(&mut doc, "add.lattice", json!({ "resolution": 3, "scale": [2, 2, 2] }));
    let lattice = object(&doc, "Lattice");
    assert_eq!(lattice["type"], "lattice");
    assert_eq!(lattice["lattice"]["pointCount"], 27);
    run(&mut doc, "modifier.add", json!({ "object": "Cube", "type": "lattice", "operand": "Lattice" }));
    assert_eq!(object(&doc, "Cube")["modifiers"][0]["operand"], lattice["id"]);
    assert_eq!(dimensions(&doc, "Cube"), [2.0, 2.0, 2.0], "a lattice at rest changes nothing");
    // The top layer up by a quarter of the lattice: the cube's top moves up.
    let top: Vec<u32> = (18..27).collect();
    run(&mut doc, "lattice.movePoints", json!({ "object": "Lattice", "points": top, "offset": [0, 0, 0.25] }));
    let size = dimensions(&doc, "Cube");
    assert!(size[2] > 2.2, "{size:?}");
    assert!(doc.execute("modifier.add", json!({ "object": "Cube", "type": "lattice", "operand": "Camera" })).is_err(), "only lattices");
    // Resizing keeps the shape; linear blending reaches the cube's top fully.
    run(&mut doc, "lattice.set", json!({ "object": "Lattice", "resolution": [2, 2, 2], "interpolation": "linear" }));
    assert_eq!(object(&doc, "Lattice")["lattice"]["pointCount"], 8);
    let size = dimensions(&doc, "Cube");
    assert!((size[2] - 2.5).abs() < 1e-3, "{size:?}");

    let bytes = doc.save(None).unwrap();
    let mut back = SceneDocument::open("Lattice.kscene", &bytes).unwrap();
    assert_eq!(back.save(None).unwrap(), bytes);
    assert_eq!(object(&back, "Lattice")["lattice"], object(&doc, "Lattice")["lattice"]);
    assert_eq!(dimensions(&back, "Cube"), dimensions(&doc, "Cube"));
    run(&mut back, "lattice.set", json!({ "object": "Lattice", "reset": true }));
    assert_eq!(dimensions(&back, "Cube"), [2.0, 2.0, 2.0]);
    run(&mut back, "edit.undo", json!({}));
    assert_eq!(dimensions(&back, "Cube"), dimensions(&doc, "Cube"));
}

#[test]
fn lattice_edit_mode_selects_and_drags_points() {
    let mut doc = SceneDocument::create("Edit.kscene", 1280, 720).unwrap();
    run(&mut doc, "edit.delete", json!({ "objects": "Cube" }));
    run(&mut doc, "add.lattice", json!({}));
    run(&mut doc, "mode.shape", json!({}));
    assert_eq!(doc.inspect().unwrap()["mode"]["mode"], "latticeEdit");
    assert!(doc.execute("add.cube", json!({})).is_err(), "objects wait for object mode");
    run(&mut doc, "select.all", json!({}));
    assert_eq!(doc.inspect().unwrap()["mode"]["points"].as_array().unwrap().len(), 8);
    run(&mut doc, "lattice.select", json!({ "points": [7] }));
    run(&mut doc, "lattice.movePoints", json!({ "offset": [0.5, 0, 0] }));
    assert_eq!(object(&doc, "Lattice")["lattice"]["points"][7], json!([1.0, 0.5, 0.5]));
    assert_eq!(object(&doc, "Lattice")["lattice"]["moved"], json!([7]));
    run(&mut doc, "edit.delete", json!({}));
    assert_eq!(object(&doc, "Lattice")["lattice"]["pointCount"], 8, "delete leaves a lattice's points");
    // The move tool drags the selection in one step.
    run(&mut doc, "view.resize", json!({ "width": 640, "height": 480 }));
    run(&mut doc, "view.top", json!({}));
    run(&mut doc, "view.frameAll", json!({}));
    run(&mut doc, "lattice.select", json!({ "points": [0] }));
    let before = object(&doc, "Lattice")["lattice"]["points"][0].clone();
    doc.select_tool("move", 1.0).unwrap();
    let event = |action, x: f64, y: f64| ToolPointer { action, x, y, mods: ToolMods::default(), pressure: 1.0, zoom: 1.0 };
    doc.pointer(&event(PointerAction::Down, 300.0, 240.0)).unwrap();
    doc.pointer(&event(PointerAction::Drag, 340.0, 240.0)).unwrap();
    doc.pointer(&event(PointerAction::Up, 340.0, 240.0)).unwrap();
    let after = object(&doc, "Lattice")["lattice"]["points"][0].clone();
    assert!(after[0].as_f64().unwrap() > before[0].as_f64().unwrap(), "{before} → {after}");
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(object(&doc, "Lattice")["lattice"]["points"][0], before);
    // A box with the select tool takes every point inside it.
    doc.select_tool("select", 1.0).unwrap();
    run(&mut doc, "lattice.select", json!({ "points": [] }));
    doc.pointer(&event(PointerAction::Down, 2.0, 2.0)).unwrap();
    doc.pointer(&event(PointerAction::Up, 638.0, 478.0)).unwrap();
    assert_eq!(doc.inspect().unwrap()["mode"]["points"].as_array().unwrap().len(), 8);
    run(&mut doc, "mode.shape", json!({}));
    assert_eq!(doc.inspect().unwrap()["mode"]["mode"], "object");
}

#[test]
fn shrinkwrap_decimate_and_an_image_displace_through_commands() {
    let mut doc = SceneDocument::create("Mods.kscene", 1280, 720).unwrap();
    run(&mut doc, "add.uvSphere", json!({ "name": "Ball", "radius": 1.5 }));
    run(&mut doc, "modifier.add", json!({ "object": "Ball", "type": "shrinkwrap", "operand": "Cube" }));
    let size = dimensions(&doc, "Ball");
    assert!(size.iter().all(|side| (side - 2.0).abs() < 1e-3), "onto the cube: {size:?}");
    run(&mut doc, "modifier.set", json!({ "object": "Ball", "index": 0, "wrapMethod": "project", "negative": true, "offset": 0.1 }));
    let modifier = object(&doc, "Ball")["modifiers"][0].clone();
    assert_eq!((modifier["wrapMethod"].as_str(), modifier["negative"].as_bool()), (Some("project"), Some(true)));
    let size = dimensions(&doc, "Ball");
    assert!(size.iter().all(|side| (side - 2.2).abs() < 1e-3), "projected and kept 0.1 off: {size:?}");
    assert!(doc.execute("modifier.set", json!({ "object": "Ball", "index": 0, "operand": "Ball" })).is_err());

    // Decimate: planar joins a subdivided cube's faces back into six.
    run(&mut doc, "modifier.add", json!({ "object": "Cube", "type": "subdivision", "levels": 2 }));
    run(&mut doc, "modifier.apply", json!({ "object": "Cube", "index": 0 }));
    run(&mut doc, "modifier.add", json!({ "object": "Cube", "type": "decimate", "mode": "planar", "angleLimit": 20 }));
    let evaluated = object(&doc, "Cube")["mesh"]["evaluated"].clone();
    assert!(evaluated["faces"].as_u64().unwrap() < 96, "a smoothed cube is curved, so planar keeps some: {evaluated}");
    assert_eq!(object(&doc, "Cube")["modifiers"][0]["angleLimit"], 20.0);
    run(&mut doc, "modifier.set", json!({ "object": "Cube", "index": 0, "mode": "unsubdivide", "iterations": 2 }));
    assert_eq!(object(&doc, "Cube")["mesh"]["evaluated"], json!({ "vertices": 26, "faces": 24 }));
    assert!(doc.execute("modifier.set", json!({ "object": "Cube", "index": 0, "iterations": 99 })).is_err());

    // Displace by an image: white pushes out by strength × (1 − mid level).
    run(&mut doc, "add.plane", json!({ "name": "Ground", "size": 2, "location": [5, 0, 0] }));
    run(&mut doc, "modifier.add", json!({ "object": "Ground", "type": "displace", "strength": 1 }));
    let set = run(&mut doc, "modifier.setImage", json!({ "object": "Ground", "index": 0, "base64": png_base64(255), "imageName": "White.png" }));
    assert_eq!((set["width"].as_u64(), set["height"].as_u64()), (Some(2), Some(2)));
    let size = dimensions(&doc, "Ground");
    assert!((size[2]).abs() < 1e-4, "a flat push keeps it flat: {size:?}");
    let ground = object(&doc, "Ground");
    assert!((ground["worldLocation"][2].as_f64().unwrap()).abs() < 1e-6);
    let bytes = doc.save(None).unwrap();
    let back = SceneDocument::open("Mods.kscene", &bytes).unwrap();
    assert_eq!(object(&back, "Ground")["modifiers"][0]["image"], set["image"]);
    assert!(back.inspect().unwrap()["images"].as_array().is_some_and(|images| images.iter().any(|image| image["hash"] == set["image"])));
    run(&mut doc, "modifier.setImage", json!({ "object": "Ground", "index": 0, "clear": true }));
    assert!(object(&doc, "Ground")["modifiers"][0]["image"].is_null());
}

#[test]
fn released_files_take_lattices_and_the_new_modifiers() {
    for (name, bytes) in
        [("0.7.0", &include_bytes!("fixtures/released-0.7.0.kscene")[..]), ("0.8.0", &include_bytes!("fixtures/released-0.8.0.kscene")[..])]
    {
        let mut doc = SceneDocument::open("Room.kscene", bytes).unwrap();
        run(&mut doc, "add.lattice", json!({ "name": "Cage", "resolution": [2, 3, 2], "scale": [3, 3, 3] }));
        run(&mut doc, "lattice.movePoints", json!({ "object": "Cage", "points": [11], "offset": [0.2, 0, 0] }));
        run(&mut doc, "modifier.add", json!({ "object": "Cube", "type": "lattice", "operand": "Cage" }));
        run(&mut doc, "add.uvSphere", json!({ "name": "Wrapped", "location": [4, 0, 0] }));
        run(&mut doc, "modifier.add", json!({ "object": "Wrapped", "type": "shrinkwrap", "operand": "Cube" }));
        run(&mut doc, "modifier.add", json!({ "object": "Wrapped", "type": "decimate", "mode": "unsubdivide", "iterations": 1 }));
        let saved = doc.save(None).unwrap();
        doc.mark_saved();
        let back = SceneDocument::open("Room.kscene", &saved).unwrap();
        let cage = object(&back, "Cage");
        assert_eq!(cage["lattice"]["resolution"], json!([2, 3, 2]), "{name}");
        assert_eq!(cage["lattice"]["moved"], json!([11]), "{name}");
        assert_eq!(object(&back, "Cube")["modifiers"][0]["type"], "lattice", "{name}");
        let wrapped: Vec<Value> = object(&back, "Wrapped")["modifiers"].as_array().unwrap().clone();
        assert_eq!((wrapped[0]["type"].as_str(), wrapped[1]["mode"].as_str()), (Some("shrinkwrap"), Some("unsubdivide")), "{name}");
        assert_eq!(object(&back, "Cube")["dimensions"], object(&doc, "Cube")["dimensions"], "{name}");
    }
}
