//! Shade Smooth by Angle through commands: the angle is kept on the mesh (undone, saved, carried
//! through modifiers and Apply), and the GPU viewport's geometry, renders, and exports split the
//! normals at edges sharper than it, so a capped cylinder's sides stay round and its caps flat.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn starter() -> SceneDocument {
    let made = SceneDocument::create("Untitled.kscene", 1280, 720).unwrap();
    SceneDocument::open("t.kscene", &made.save(None).unwrap()).unwrap()
}

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn mesh(doc: &SceneDocument, name: &str) -> Value {
    doc.inspect().unwrap()["objects"].as_array().unwrap().iter().find(|object| object["name"] == name).unwrap()["mesh"].clone()
}

fn id(doc: &SceneDocument, name: &str) -> u32 {
    doc.inspect().unwrap()["objects"].as_array().unwrap().iter().find(|object| object["name"] == name).unwrap()["id"].as_u64().unwrap() as u32
}

/// Each drawn corner's normal from the GPU viewport's geometry buffer.
fn drawn_normals(doc: &SceneDocument, name: &str) -> Vec<[f32; 3]> {
    let bytes = doc.gl_mesh(id(doc, name), false).unwrap();
    let (corners, ..) = kition_scene_kit::gl::mesh_header(&bytes).unwrap();
    let float = |at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    (0..corners as usize)
        .map(|corner| {
            let at = 20 + corner * 9 * 4 + 3 * 4;
            [float(at), float(at + 4), float(at + 8)]
        })
        .collect()
}

/// Corners whose normal leans part of the way between the side and a cap (the smear).
fn leaning(normals: &[[f32; 3]]) -> usize {
    normals.iter().filter(|normal| normal[2].abs() > 0.05 && normal[2].abs() < 0.95).count()
}

fn with_cylinder() -> SceneDocument {
    let mut doc = starter();
    run(&mut doc, "edit.delete", json!({ "objects": "Cube" }));
    run(&mut doc, "add.cylinder", json!({ "name": "Can", "vertices": 24 }));
    doc
}

#[test]
fn the_angle_is_a_mesh_setting() {
    let mut doc = with_cylinder();
    run(&mut doc, "object.shadeSmoothByAngle", json!({ "objects": "Can" }));
    let can = mesh(&doc, "Can");
    assert_eq!((can["smooth"].as_bool(), can["smoothAngle"].as_f64()), (Some(true), Some(30.0)), "30° by default");
    run(&mut doc, "object.shadeSmoothByAngle", json!({ "objects": "Can", "angle": 45 }));
    assert_eq!(mesh(&doc, "Can")["smoothAngle"].as_f64(), Some(45.0));
    for refused in [json!({ "objects": "Can", "angle": 181 }), json!({ "objects": "Can", "angle": -1 }), json!({ "objects": "Light" })] {
        assert!(doc.execute("object.shadeSmoothByAngle", refused.clone()).is_err(), "{refused}");
    }
    let reopened = SceneDocument::open("t.kscene", &doc.save(None).unwrap()).unwrap();
    assert_eq!(mesh(&reopened, "Can")["smoothAngle"].as_f64(), Some(45.0));
    // Shade Smooth smooths every edge again; Shade Flat is flat.
    run(&mut doc, "object.shadeSmooth", json!({ "objects": "Can" }));
    assert_eq!((mesh(&doc, "Can")["smooth"].as_bool(), mesh(&doc, "Can")["smoothAngle"].clone()), (Some(true), Value::Null));
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(mesh(&doc, "Can")["smoothAngle"].as_f64(), Some(45.0));
    run(&mut doc, "object.shadeFlat", json!({ "objects": "Can" }));
    assert_eq!((mesh(&doc, "Can")["smooth"].as_bool(), mesh(&doc, "Can")["smoothAngle"].clone()), (Some(false), Value::Null));
}

#[test]
fn sharp_rims_split_the_normals_the_viewport_draws() {
    let mut doc = with_cylinder();
    run(&mut doc, "object.shadeSmooth", json!({ "objects": "Can" }));
    assert!(leaning(&drawn_normals(&doc, "Can")) > 0, "plain smooth shading smears the rims");
    run(&mut doc, "object.shadeSmoothByAngle", json!({ "objects": "Can" }));
    let normals = drawn_normals(&doc, "Can");
    assert_eq!(leaning(&normals), 0, "sides horizontal, caps straight up or down");
    assert!(normals.iter().any(|normal| normal[2] > 0.999) && normals.iter().any(|normal| normal[2].abs() < 1e-4));
    // A modifier and applying it keep the angle.
    run(&mut doc, "modifier.add", json!({ "object": "Can", "type": "array" }));
    assert_eq!(leaning(&drawn_normals(&doc, "Can")), 0, "through a modifier");
    run(&mut doc, "modifier.apply", json!({ "object": "Can", "index": 0 }));
    assert_eq!(mesh(&doc, "Can")["smoothAngle"].as_f64(), Some(30.0));
    assert_eq!(leaning(&drawn_normals(&doc, "Can")), 0, "after Apply");
}

#[test]
fn renders_and_exports_use_the_split_normals() {
    let mut doc = with_cylinder();
    run(&mut doc, "render.set", json!({ "width": 48, "height": 27 }));
    run(&mut doc, "object.shadeSmooth", json!({ "objects": "Can" }));
    let smooth = run(&mut doc, "render.image", json!({ "samples": 1 }))["base64"].clone();
    let traced_smooth = run(&mut doc, "render.trace.start", json!({ "samples": 2, "wait": true }))["base64"].clone();
    let usda_smooth = String::from_utf8(kition_scene_usd::write(doc.scene(), None).unwrap()).unwrap();
    run(&mut doc, "object.shadeSmoothByAngle", json!({ "objects": "Can" }));
    assert_ne!(run(&mut doc, "render.image", json!({ "samples": 1 }))["base64"], smooth, "the fast render changes");
    assert_ne!(run(&mut doc, "render.trace.start", json!({ "samples": 2, "wait": true }))["base64"], traced_smooth, "the path tracer's too");
    let usda = String::from_utf8(kition_scene_usd::write(doc.scene(), None).unwrap()).unwrap();
    // The caps' corners point straight up and down in the export, as they are drawn.
    let up = |text: &str| {
        let line = text.lines().find(|line| line.contains("normal3f[] normals")).unwrap();
        line.split('(')
            .skip(1)
            .filter(|tuple| tuple.split([',', ')']).nth(2).and_then(|z| z.trim().parse::<f32>().ok()).is_some_and(|z| z.abs() > 0.999))
            .count()
    };
    assert!(up(&usda) >= 48 && up(&usda_smooth) == 0, "{} against {}", up(&usda), up(&usda_smooth));
}
