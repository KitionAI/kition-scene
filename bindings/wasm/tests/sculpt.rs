//! Sculpt Mode through commands: strokes from world points and from viewport pixels, one undo
//! step each, the mask, grab, remesh, and the sculpt surviving a save.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

/// The starter scene without its cube, and a 60 × 60 grid 2 m wide, being sculpted.
fn sculpting() -> SceneDocument {
    let made = SceneDocument::create("Untitled scene.kscene", 800, 600).unwrap();
    let mut doc = SceneDocument::open("t.kscene", &made.save(None).unwrap()).unwrap();
    run(&mut doc, "edit.delete", json!({ "objects": "Cube" }));
    run(&mut doc, "add.grid", json!({ "name": "Clay", "xSubdivisions": 60, "ySubdivisions": 60, "size": 2 }));
    run(&mut doc, "mode.sculpt", json!({}));
    doc
}

/// Every vertex height of the scene's meshes: the document saved, reopened, and exported to OBJ
/// (which is Y-up, so the grid's Z is the file's Y).
fn heights(doc: &SceneDocument) -> Vec<f32> {
    let mut copy = SceneDocument::open("t.kscene", &doc.save(None).unwrap()).unwrap();
    let exported = copy.execute("file.export", json!({ "format": "obj" })).unwrap();
    let text = decode(exported["base64"].as_str().unwrap());
    text.lines().filter_map(|line| line.strip_prefix("v ")).map(|rest| rest.split_whitespace().nth(1).unwrap().parse::<f32>().unwrap()).collect()
}

fn decode(base64: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let clean: Vec<u8> = base64.bytes().filter(|byte| *byte != b'=').collect();
    for chunk in clean.chunks(4) {
        let mut value = 0u32;
        for (index, byte) in chunk.iter().enumerate() {
            value |= (TABLE.iter().position(|item| item == byte).unwrap() as u32) << (18 - 6 * index);
        }
        out.extend(value.to_be_bytes().iter().skip(1).take(chunk.len() - 1));
    }
    String::from_utf8(out).unwrap()
}

fn peak(doc: &SceneDocument) -> f32 {
    heights(doc).into_iter().fold(f32::MIN, f32::max)
}

#[test]
fn sculpt_mode_is_a_mode_and_needs_a_mesh() {
    let mut doc = sculpting();
    let mode = &doc.inspect().unwrap()["mode"];
    assert_eq!(mode["mode"], "sculpt", "{mode}");
    assert_eq!(mode["sculpt"]["brush"]["kind"], "draw");
    run(&mut doc, "mode.sculpt", json!({ "on": false }));
    assert_eq!(doc.inspect().unwrap()["mode"]["mode"], "object");
    run(&mut doc, "select.objects", json!({ "objects": "Light" }));
    assert!(doc.execute("mode.sculpt", json!({})).is_err(), "a light cannot be sculpted");
    assert!(doc.execute("sculpt.stroke", json!({ "points": [[0, 0, 0]] })).is_err(), "not in sculpt mode");
}

#[test]
fn a_world_stroke_raises_the_surface_and_undoes_in_one_step() {
    let mut doc = sculpting();
    assert_eq!(peak(&doc), 0.0);
    let undo_before = doc.inspect().unwrap()["canUndo"].clone();
    let done = run(&mut doc, "sculpt.stroke", json!({ "points": [[-0.3, 0, 0], [0.3, 0, 0]], "worldRadius": 0.3, "strength": 1 }));
    assert!(done["touched"].as_u64().unwrap() > 50, "{done}");
    let raised = peak(&doc);
    assert!(raised > 0.01, "{raised}");
    assert!(doc.dirty());
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(peak(&doc), 0.0, "one stroke is one undo step");
    assert_eq!(doc.inspect().unwrap()["canUndo"], undo_before);
    run(&mut doc, "edit.redo", json!({}));
    assert_eq!(peak(&doc), raised);
    // Subtract goes in.
    run(&mut doc, "sculpt.stroke", json!({ "points": [[0.6, 0.6, 0]], "worldRadius": 0.2, "subtract": true, "strength": 1 }));
    assert!(heights(&doc).into_iter().fold(f32::MAX, f32::min) < 0.0);
}

#[test]
fn the_mask_protects_and_saves() {
    let mut doc = sculpting();
    run(&mut doc, "sculpt.maskFill", json!({}));
    assert!(doc.inspect().unwrap()["mode"]["sculpt"]["masked"].as_u64().unwrap() > 3000);
    run(&mut doc, "sculpt.stroke", json!({ "points": [[0, 0, 0]], "worldRadius": 0.3, "strength": 1 }));
    assert_eq!(peak(&doc), 0.0, "masked vertices stay");
    run(&mut doc, "sculpt.maskInvert", json!({}));
    assert_eq!(doc.inspect().unwrap()["mode"]["sculpt"]["masked"], 0);
    // A painted mask comes back after a save.
    run(&mut doc, "sculpt.stroke", json!({ "kind": "mask", "points": [[0, 0, 0]], "worldRadius": 0.4 }));
    let masked = doc.inspect().unwrap()["mode"]["sculpt"]["masked"].as_u64().unwrap();
    assert!(masked > 20);
    let bytes = doc.save(None).unwrap();
    let mut reopened = SceneDocument::open("t.kscene", &bytes).unwrap();
    run(&mut reopened, "select.objects", json!({ "objects": "Clay" }));
    run(&mut reopened, "mode.sculpt", json!({}));
    assert_eq!(reopened.inspect().unwrap()["mode"]["sculpt"]["masked"].as_u64().unwrap(), masked);
}

#[test]
fn a_pixel_stroke_sculpts_where_the_view_shows_the_surface() {
    let mut doc = sculpting();
    run(&mut doc, "view.resize", json!({ "width": 800, "height": 600 }));
    run(&mut doc, "view.top", json!({}));
    // Draw through the middle of the view, where the grid is.
    run(&mut doc, "sculpt.stroke", json!({ "points": [[360, 300], [440, 300]], "radius": 60, "strength": 1 }));
    assert!(peak(&doc) > 0.0, "the stroke landed on the grid");
    // Off the grid (the corner of the view) nothing happens.
    let before = heights(&doc);
    run(&mut doc, "sculpt.stroke", json!({ "points": [[2, 2]], "radius": 10 }));
    assert_eq!(heights(&doc), before);
}

#[test]
fn grab_drags_and_remesh_rebuilds() {
    let mut doc = sculpting();
    run(
        &mut doc,
        "sculpt.stroke",
        json!({ "kind": "grab", "points": [[0, 0, 0]], "worldRadius": 0.3, "drag": [0, 0, 0.5], "symmetry": [false, false, false] }),
    );
    let top = peak(&doc);
    assert!((top - 0.5).abs() < 0.02, "{top}");
    // Remesh a closed shape.
    run(&mut doc, "mode.sculpt", json!({ "on": false }));
    run(&mut doc, "add.cube", json!({ "name": "Block" }));
    let done = run(&mut doc, "sculpt.remesh", json!({ "object": "Block", "voxelSize": 0.1 }));
    assert!(done["faces"].as_u64().unwrap() > 1500, "{done}");
    let block = doc.inspect().unwrap()["objects"].as_array().unwrap().iter().find(|object| object["name"] == "Block").unwrap().clone();
    assert_eq!(block["mesh"]["faces"], done["faces"]);
    assert!(doc.execute("sculpt.remesh", json!({ "object": "Block", "voxelSize": 0.0001 })).is_err());
}

#[test]
fn bad_strokes_are_refused_and_change_nothing() {
    let mut doc = sculpting();
    let before = doc.save(None).unwrap();
    for params in [
        json!({ "points": [] }),
        json!({ "points": [[0, 0, 0], [1, 1]] }),
        json!({ "points": [["a", 0]] }),
        json!({ "points": [[0, 0, 0]], "kind": "chisel" }),
        json!({ "points": [[0, 0, 0]], "strength": 3 }),
        json!({ "points": [[0, 0, 0]], "worldRadius": -1 }),
        json!({ "points": [[0, 0, 0]], "symmetry": [true] }),
    ] {
        assert!(doc.execute("sculpt.stroke", params.clone()).is_err(), "{params}");
    }
    assert_eq!(doc.save(None).unwrap(), before);
}
