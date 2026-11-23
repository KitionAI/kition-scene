//! Recovery through public commands: one transaction, saved detail, and exact failures.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use kition_scene_model::{Mesh, multires};
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|e| panic!("{command}: {e}"))
}
fn setup() -> SceneDocument {
    let reference: Value = serde_json::from_str(include_str!("../../../crates/engine/model/tests/fixtures/multires8-reference.json")).unwrap();
    let input = &reference["cases"][2]["input"];
    let mut doc = SceneDocument::create("Recovery.kscene", 400, 300).unwrap();
    run(&mut doc, "select.objects", json!({"objects":"Cube"}));
    run(&mut doc, "mesh.setGeometry", json!({"object":"Cube", "positions":input["positions"], "faces":input["faces"]}));
    run(&mut doc, "modifier.add", json!({"object":"Cube", "type":"multires"}));
    doc
}
fn mesh(doc: &SceneDocument) -> &Mesh {
    doc.scene().objects.iter().find(|o| o.name == "Cube").unwrap().mesh().unwrap()
}
fn cube(doc: &SceneDocument) -> Value {
    doc.inspect().unwrap()["objects"].as_array().unwrap().iter().find(|o| o["name"] == "Cube").unwrap().clone()
}
fn close_positions(a: &Mesh, b: &Mesh) {
    assert_eq!(a.positions.len(), b.positions.len());
    for p in &a.positions {
        assert!(b.positions.iter().any(|q| p.distance(*q) < 3e-6));
    }
}

#[test]
fn each_recovery_operator_undoes_redoes_and_survives_save() {
    for command in ["multires.unsubdivide", "multires.rebuildSubdivisions"] {
        let mut doc = setup();
        let before = mesh(&doc).clone();
        let expected = if command == "multires.unsubdivide" { 1 } else { 2 };
        let result = run(&mut doc, command, json!({"object":"Cube"}));
        assert_eq!(result["levels"], expected);
        assert_eq!(result["modifier"], 0);
        let after = mesh(&doc).clone();
        close_positions(&multires::evaluate(&after, expected), &before);
        assert_eq!(cube(&doc)["modifiers"][0]["levels"], if expected == 1 { 0 } else { 2 });
        assert_eq!(cube(&doc)["modifiers"][0]["sculptLevels"], if expected == 1 { 0 } else { 2 });
        run(&mut doc, "edit.undo", json!({}));
        assert_eq!(mesh(&doc), &before);
        run(&mut doc, "edit.redo", json!({}));
        assert_eq!(mesh(&doc), &after);
        let reopened = SceneDocument::open("Saved.kscene", &doc.save(None).unwrap()).unwrap();
        assert_eq!(mesh(&reopened), &after);
        assert_eq!(cube(&reopened)["modifiers"], cube(&doc)["modifiers"]);
        assert_eq!(kition_scene_kdm::schema().schema_version, 1);
    }
}

#[test]
fn sculpted_levels_survive_recovery_and_subsequent_sculpting() {
    let mut doc = setup();
    run(&mut doc, "multires.subdivide", json!({}));
    run(&mut doc, "mode.sculpt", json!({}));
    run(&mut doc, "sculpt.stroke", json!({"points":[[0,0,1]],"worldRadius":0.8,"strength":1}));
    let before = multires::evaluate(mesh(&doc), 1);
    run(&mut doc, "multires.unsubdivide", json!({}));
    assert_eq!(multires::levels(mesh(&doc)), 2);
    close_positions(&multires::evaluate(mesh(&doc), 2), &before);
    run(&mut doc, "modifier.set", json!({"object":"Cube", "index":0, "levels":2,"sculptLevels":2}));
    let sculpted = run(&mut doc, "sculpt.stroke", json!({"points":[[0,0,1]],"worldRadius":0.8,"strength":1}));
    assert_eq!(sculpted["level"], 2);
    assert!(sculpted["touched"].as_u64().unwrap() > 0);
    let after = mesh(&doc).clone();
    assert_ne!(after.multires[1], vec![kition_scene_model::glam::Vec3::ZERO; after.multires[1].len()]);
    let reopened = SceneDocument::open("Sculpted.kscene", &doc.save(None).unwrap()).unwrap();
    assert_eq!(mesh(&reopened), &after);
}

#[test]
fn invalid_recovery_preserves_document_session_history_and_serialized_bytes() {
    let mut doc = setup();
    run(&mut doc, "multires.rebuildSubdivisions", json!({}));
    for (command, params) in [
        ("multires.rebuildSubdivisions", json!({})),
        ("multires.unsubdivide", json!({})),
        ("multires.unsubdivide", json!({"object":"Light"})),
        ("multires.unsubdivide", json!({"object":"Absent"})),
        ("multires.rebuildSubdivisions", json!([1, 2])),
    ] {
        let bytes = doc.save(None).unwrap();
        let inspect = doc.inspect().unwrap();
        assert!(doc.execute(command, params).is_err());
        assert_eq!(doc.save(None).unwrap(), bytes);
        assert_eq!(doc.inspect().unwrap(), inspect);
    }
    for id in ["multires.unsubdivide", "multires.rebuildSubdivisions"] {
        let command = doc.commands().into_iter().find(|c| c.id == id).unwrap();
        assert!(command.mutates);
        assert!(!command.description.is_empty());
        assert!(command.schema.is_some());
    }
}
