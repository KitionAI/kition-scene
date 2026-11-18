//! Angle-based unwrapping and stitching, through commands.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn editing_cube() -> SceneDocument {
    let doc = SceneDocument::create("Untitled.kscene", 400, 300).unwrap();
    let mut doc = SceneDocument::open("t.kscene", &doc.save(None).unwrap()).unwrap();
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "mode.set", json!({ "mode": "edit" }));
    doc
}

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

/// The coordinates of every corner at `vertex`, over all faces.
fn corners_at(doc: &mut SceneDocument, vertex: u64) -> Vec<(f64, f64)> {
    let layout = run(doc, "uv.layout", json!({}));
    let mut out = Vec::new();
    for face in layout["faces"].as_array().unwrap() {
        for (corner, uv) in face["vertices"].as_array().unwrap().iter().zip(face["uvs"].as_array().unwrap()) {
            if corner.as_u64() == Some(vertex) {
                out.push((uv[0].as_f64().unwrap(), uv[1].as_f64().unwrap()));
            }
        }
    }
    out
}

#[test]
fn unwrap_methods_and_stitching_two_islands() {
    let mut doc = editing_cube();
    run(&mut doc, "mesh.markSeam", json!({}));
    // Angle-based is the default; conformal on request; anything else is refused.
    assert_eq!(run(&mut doc, "uv.unwrap", json!({}))["charts"], 6);
    assert_eq!(run(&mut doc, "uv.unwrap", json!({ "method": "conformal" }))["charts"], 6);
    assert!(doc.execute("uv.unwrap", json!({ "method": "smart" })).is_err());

    // Stitch along the edge between vertices 0 and 1 (shared by two of the cube's faces).
    let apart = corners_at(&mut doc, 0);
    assert!(apart.windows(2).any(|pair| (pair[0].0 - pair[1].0).abs() + (pair[0].1 - pair[1].1).abs() > 1e-4), "islands start apart");
    run(&mut doc, "mesh.select", json!({ "vertices": [0, 1] }));
    let stitched = run(&mut doc, "uv.stitch", json!({}));
    assert_eq!(stitched["edges"], 1);
    // On the two faces of that edge the corners now meet; seams stay marked.
    run(&mut doc, "select.all", json!({}));
    let layout = run(&mut doc, "uv.layout", json!({}));
    assert_eq!(layout["seams"].as_array().unwrap().len(), 12);
    let faces: Vec<&Value> = layout["faces"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|face| {
            let vertices = face["vertices"].as_array().unwrap();
            vertices.iter().any(|v| v.as_u64() == Some(0)) && vertices.iter().any(|v| v.as_u64() == Some(1))
        })
        .collect();
    assert_eq!(faces.len(), 2);
    for vertex in [0u64, 1] {
        let uv = |face: &Value| {
            let at = face["vertices"].as_array().unwrap().iter().position(|v| v.as_u64() == Some(vertex)).unwrap();
            (face["uvs"][at][0].as_f64().unwrap(), face["uvs"][at][1].as_f64().unwrap())
        };
        let (a, b) = (uv(faces[0]), uv(faces[1]));
        assert!((a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5, "{a:?} {b:?}");
    }
    // Nothing between islands to stitch now: refused, nothing changed.
    run(&mut doc, "mesh.select", json!({ "vertices": [0, 1] }));
    assert!(doc.execute("uv.stitch", json!({})).is_err());
    run(&mut doc, "select.all", json!({}));
    run(&mut doc, "edit.undo", json!({}));
    assert!(corners_at(&mut doc, 0).len() >= 3);
}
