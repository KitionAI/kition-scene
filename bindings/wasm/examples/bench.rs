//! Times the viewport render of a busy scene: `cargo run --release -p kition-scene-kit --example bench`.
#![allow(clippy::indexing_slicing)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::json;

fn main() -> Result<(), String> {
    let mut doc = SceneDocument::create("Bench.kscene", 1280, 720)?;
    doc.execute("modifier.add", json!({ "objects": "Cube", "type": "subdivision", "levels": 3 }))?;
    for index in 0..20 {
        doc.execute("add.uvSphere", json!({ "location": [(index % 5) as f32 * 2.5 - 5.0, (index / 5) as f32 * 2.5 - 4.0, 0.5], "radius": 0.8 }))?;
    }
    let faces: u64 = doc.inspect()?["objects"]
        .as_array()
        .map(|objects| objects.iter().filter_map(|object| object["mesh"]["evaluated"]["faces"].as_u64()).sum())
        .unwrap_or(0);
    let time = |doc: &mut SceneDocument, label: &str| -> Result<(), String> {
        let start = std::time::Instant::now();
        for turn in 0..10 {
            doc.execute("view.orbit", json!({ "turn": turn as f32 }))?;
            doc.render(0.0, 0.0, 1.0, doc.width(), doc.height())?;
        }
        println!("  {label}: {:.1} ms", start.elapsed().as_secs_f64() * 100.0);
        Ok(())
    };
    time(&mut doc, "solid, grid, selection")?;
    doc.execute("view.grid", json!({ "on": false }))?;
    time(&mut doc, "no grid")?;
    doc.execute("select.none", json!({}))?;
    time(&mut doc, "no grid, no selection")?;
    doc.execute("view.zoom", json!({ "factor": 20 }))?;
    time(&mut doc, "far away (few pixels)")?;
    doc.execute("view.zoom", json!({ "factor": 0.05 }))?;
    let mut empty = SceneDocument::create("Empty.kscene", 1280, 720)?;
    empty.execute("select.all", json!({}))?;
    empty.execute("edit.delete", json!({}))?;
    empty.execute("view.grid", json!({ "on": false }))?;
    time(&mut empty, "empty scene, no grid")?;
    doc.execute("view.grid", json!({ "on": true }))?;
    for shading in ["solid", "wireframe", "material"] {
        doc.execute("view.shading", json!({ "mode": shading }))?;
        let start = std::time::Instant::now();
        let runs = 10;
        for turn in 0..runs {
            doc.execute("view.orbit", json!({ "turn": turn as f32 }))?;
            doc.render(0.0, 0.0, 1.0, doc.width(), doc.height())?;
        }
        println!("{shading}: {faces} faces, {:.1} ms per 1280x720 frame", start.elapsed().as_secs_f64() * 1000.0 / runs as f64);
    }
    Ok(())
}
