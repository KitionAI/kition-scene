//! Regression: material.new says what it does: it gives the new
//! material to the selected objects unless `assign: false`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

#[test]
fn material_new_says_it_assigns_by_default() {
    let mut doc = SceneDocument::create("Materials.kscene", 320, 240).unwrap();
    let spec = doc.commands().into_iter().find(|c| c.id == "material.new").unwrap();
    assert!(spec.description.contains("unless `assign: false`"), "{}", spec.description);
    let params = serde_json::to_string(&spec).unwrap();
    assert!(params.contains("unless false"), "{params}");
    // The starter cube is selected: a new material goes on it, unless asked not to.
    let kept = run(&mut doc, "material.new", json!({ "name": "Spare", "assign": false }));
    let made = run(&mut doc, "material.new", json!({ "name": "Clay" }));
    let cube = doc.inspect().unwrap()["objects"].as_array().unwrap().iter().find(|o| o["name"] == "Cube").cloned().unwrap();
    assert_eq!(cube["material"], made["material"], "{cube} {made} {kept}");
}
