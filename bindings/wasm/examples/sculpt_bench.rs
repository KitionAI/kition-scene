//! Times sculpt strokes through the engine on a 200k-vertex grid (the replica commit included):
//! `cargo run --release -p kition-scene-kit --example sculpt_bench`.
#![allow(clippy::indexing_slicing)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::json;

fn main() -> Result<(), String> {
    let mut doc = SceneDocument::create("Bench.kscene", 1280, 720)?;
    doc.execute("edit.delete", json!({ "objects": "Cube" }))?;
    let start = std::time::Instant::now();
    doc.execute("object.addGrid", json!({ "name": "Clay", "xSubdivisions": 446, "ySubdivisions": 446, "size": 2 }))?;
    println!("add a 200k-vertex grid: {:.0} ms", start.elapsed().as_secs_f64() * 1000.0);
    doc.execute("mode.sculpt", json!({}))?;
    for (kind, radius) in [("draw", 0.1), ("clayStrips", 0.1), ("smooth", 0.1), ("grab", 0.2)] {
        let points: Vec<[f32; 3]> = (0..20).map(|step| [-0.5 + step as f32 * 0.05, 0.0, 0.0]).collect();
        let start = std::time::Instant::now();
        let done = doc.execute("sculpt.stroke", json!({ "kind": kind, "points": points, "worldRadius": radius, "drag": [0, 0, 0.2] }))?;
        let stroke = start.elapsed().as_secs_f64() * 1000.0;
        println!("{kind} stroke (20 points, r={radius} m): {stroke:.0} ms in all, {} vertices touched", done["touched"]);
    }
    let start = std::time::Instant::now();
    doc.render(0.0, 0.0, 1.0, doc.width(), doc.height())?;
    println!("viewport frame in sculpt mode: {:.0} ms", start.elapsed().as_secs_f64() * 1000.0);
    doc.execute("view.orbit", json!({ "turn": 1 }))?;
    let start = std::time::Instant::now();
    doc.render(0.0, 0.0, 1.0, doc.width(), doc.height())?;
    println!("same mesh, view turned (prepared mesh cached): {:.0} ms", start.elapsed().as_secs_f64() * 1000.0);
    doc.execute("sculpt.stroke", json!({ "points": [[0.2, 0.2, 0.0]], "worldRadius": 0.1 }))?;
    let start = std::time::Instant::now();
    doc.render(0.0, 0.0, 1.0, doc.width(), doc.height())?;
    println!("frame after a dab (vertices moved): {:.0} ms", start.elapsed().as_secs_f64() * 1000.0);
    doc.execute("mode.sculpt", json!({ "on": false }))?;
    doc.execute("view.orbit", json!({ "turn": 1 }))?;
    let start = std::time::Instant::now();
    doc.render(0.0, 0.0, 1.0, doc.width(), doc.height())?;
    println!("object mode, first frame: {:.0} ms", start.elapsed().as_secs_f64() * 1000.0);
    doc.execute("view.orbit", json!({ "turn": 1 }))?;
    let start = std::time::Instant::now();
    doc.render(0.0, 0.0, 1.0, doc.width(), doc.height())?;
    println!("object mode, view turned: {:.0} ms", start.elapsed().as_secs_f64() * 1000.0);
    Ok(())
}
