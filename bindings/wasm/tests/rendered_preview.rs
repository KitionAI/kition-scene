//! Regression: the rendered preview can be traced at the viewport's
//! full size (`maxSide`, 512 by default for a live view) and denoised (`denoise`), and the
//! overlays (grid, lights, cameras, origins) can be switched off with view.overlays, so a
//! rendered camera view shows only the render.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn rendered(width: u32, height: u32) -> SceneDocument {
    let mut doc = SceneDocument::create("Preview.kscene", width, height).unwrap();
    run(&mut doc, "view.rendered", json!({}));
    doc
}

/// How much neighbouring pixels differ across the picture (noise and edges).
fn roughness(px: &[u8], width: usize) -> f64 {
    let mut total = 0.0;
    for (i, p) in px.as_chunks::<4>().0.iter().enumerate() {
        if i % width + 1 < width {
            let q = &px[(i + 1) * 4..(i + 1) * 4 + 3];
            total += (0..3).map(|c| f64::from(p[c].abs_diff(q[c]))).sum::<f64>();
        }
    }
    total / (px.len() / 4) as f64
}

#[test]
fn the_preview_can_be_full_size_and_denoised() {
    let mut doc = rendered(1024, 576);
    let preview = run(&mut doc, "view.renderedStep", json!({ "samples": 1, "wait": true }));
    assert_eq!((preview["width"].clone(), preview["height"].clone()), (json!(512), json!(288)), "a live preview: {preview}");
    let full = run(&mut doc, "view.renderedStep", json!({ "samples": 1, "wait": true, "maxSide": 1024 }));
    assert_eq!((full["width"].clone(), full["height"].clone()), (json!(1024), json!(576)), "full size: {full}");
    assert!(doc.execute("view.renderedStep", json!({ "maxSide": 10 })).is_err(), "64 to 4096");

    // The noise, without the grid's lines and the outlines in the way.
    let mut noisy = rendered(256, 144);
    run(&mut noisy, "view.overlays", json!({ "on": false }));
    run(&mut noisy, "view.renderedStep", json!({ "samples": 2, "wait": true }));
    let mut smooth = rendered(256, 144);
    run(&mut smooth, "view.overlays", json!({ "on": false }));
    run(&mut smooth, "view.renderedStep", json!({ "samples": 2, "wait": true, "denoise": true }));
    let (a, b) = (roughness(&noisy.render(0.0, 0.0, 1.0, 256, 144).unwrap(), 256), roughness(&smooth.render(0.0, 0.0, 1.0, 256, 144).unwrap(), 256));
    assert!(b < a * 0.9, "denoised is smoother: {b} vs {a}");
}

#[test]
fn overlays_can_be_switched_off() {
    let mut doc = rendered(160, 90);
    run(&mut doc, "view.renderedStep", json!({ "samples": 1, "wait": true }));
    let with = doc.render(0.0, 0.0, 1.0, 160, 90).unwrap();
    run(&mut doc, "view.overlays", json!({ "on": false }));
    assert_eq!(doc.inspect().unwrap()["view"]["overlays"], json!(false));
    run(&mut doc, "view.renderedStep", json!({ "samples": 1, "wait": true }));
    let without = doc.render(0.0, 0.0, 1.0, 160, 90).unwrap();
    assert_ne!(with, without, "the light, the camera and the selection outline are gone");
    // The overlays off draw what tracing the same view without any overlay draws.
    run(&mut doc, "view.overlays", json!({}));
    assert_eq!(doc.inspect().unwrap()["view"]["overlays"], json!(true), "toggles");
}
