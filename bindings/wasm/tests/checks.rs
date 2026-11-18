//! `scene_check` against its catalogue (`agent/checks.json`, KHP A38): each fixture in
//! `conformance/` (written by `examples/conformance.rs`) makes the check report its code and only
//! catalogued codes, and `basic.kscene`, the scene the conformance suite edits, is clean.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::path::Path;

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::Value;

fn repo() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn codes(fixture: &str) -> Vec<String> {
    let bytes = std::fs::read(repo().join("conformance").join(fixture)).unwrap_or_else(|error| panic!("{fixture}: {error}"));
    SceneDocument::open(fixture, &bytes).unwrap().check().into_iter().map(|diagnostic| diagnostic.code).collect()
}

#[test]
fn every_fixture_reports_its_code_and_basic_is_clean() {
    let catalogue: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(repo().join("agent/checks.json")).unwrap()).unwrap();
    let known: Vec<&str> = catalogue.iter().map(|entry| entry["code"].as_str().unwrap()).collect();
    let mut failures = Vec::new();
    for entry in &catalogue {
        let (code, fixture) = (entry["code"].as_str().unwrap(), entry["fixture"].as_str().unwrap());
        let found = codes(fixture);
        if !found.iter().any(|found| found == code) {
            failures.push(format!("{fixture} does not report {code}: {found:?}"));
        }
        if let Some(other) = found.iter().find(|found| !known.contains(&found.as_str())) {
            failures.push(format!("{fixture} reports {other} outside the catalogue"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    assert_eq!(codes("basic.kscene"), Vec::<String>::new());
}

/// Every diagnostic a fixture raises that carries a `fix` is cleared by running it (KHP A21: a
/// fix is a command the model can run as given). The unused-material note must carry one.
#[test]
fn every_fix_clears_its_code() {
    let catalogue: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(repo().join("agent/checks.json")).unwrap()).unwrap();
    let mut failures = Vec::new();
    for entry in &catalogue {
        let (code, fixture) = (entry["code"].as_str().unwrap(), entry["fixture"].as_str().unwrap());
        let bytes = std::fs::read(repo().join("conformance").join(fixture)).unwrap();
        let mut doc = SceneDocument::open(fixture, &bytes).unwrap();
        let fixes: Vec<_> = doc.check().into_iter().filter(|diagnostic| diagnostic.code == code).filter_map(|diagnostic| diagnostic.fix).collect();
        if code == "scene.unusedMaterial" && fixes.is_empty() {
            failures.push(format!("{code} carries no fix"));
        }
        for fix in fixes {
            if let Err(error) = doc.execute(&fix.command, fix.params.clone().unwrap_or(Value::Null)) {
                failures.push(format!("{code}: {} failed: {error}", fix.command));
            }
        }
        if doc.check().iter().any(|diagnostic| diagnostic.code == code) && doc.check().iter().any(|d| d.code == code && d.fix.is_some()) {
            failures.push(format!("{code} is still reported after its fixes"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn material_delete_refuses_a_used_material_unless_forced_and_undoes() {
    let mut doc = SceneDocument::create("Scene.kscene", 64, 64).unwrap();
    let made = doc.execute("material.new", serde_json::json!({ "name": "Red" })).unwrap();
    let material = made["material"].clone();
    let count = |doc: &SceneDocument| -> Vec<String> {
        doc.inspect().unwrap()["materials"].as_array().unwrap().iter().filter_map(|material| material["name"].as_str().map(str::to_owned)).collect()
    };
    assert!(!made["assigned"].as_array().unwrap().is_empty(), "the new scene's selection takes the material: {made}");
    assert!(doc.execute("material.delete", serde_json::json!({ "material": material })).is_err(), "Red is used by the selection");
    doc.execute("material.delete", serde_json::json!({ "material": "Red", "force": true })).unwrap();
    assert!(!count(&doc).contains(&"Red".to_owned()));
    doc.execute("edit.undo", serde_json::json!({})).unwrap();
    assert!(count(&doc).contains(&"Red".to_owned()), "the delete is one undo step");
}

#[test]
fn edit_delete_offers_its_objects() {
    let doc = SceneDocument::create("Scene.kscene", 64, 64).unwrap();
    let delete = doc.commands().into_iter().find(|command| command.id == "edit.delete").unwrap();
    let schema = delete.schema.unwrap();
    assert!(schema.get("objects").is_some(), "{schema}");
    let material_delete = doc.commands().into_iter().find(|command| command.id == "material.delete").unwrap();
    let schema = material_delete.schema.unwrap();
    assert!(schema.get("material").is_some() && schema.get("force").is_some(), "{schema}");
}
