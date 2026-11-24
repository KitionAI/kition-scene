//! Motor settings use the existing physics register, transactions and derived bake lifecycle.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}
fn setup() -> SceneDocument {
    let mut doc = SceneDocument::create("Motors.kscene", 640, 360).unwrap();
    run(&mut doc, "select.all", json!({}));
    run(&mut doc, "edit.delete", json!({}));
    run(&mut doc, "add.cube", json!({"name":"Anchor", "size":1, "location":[-2,0,0]}));
    run(&mut doc, "physics.rigidBody.add", json!({"type":"passive"}));
    run(&mut doc, "add.cube", json!({"name":"Body", "size":1, "location":[2,0,0]}));
    run(&mut doc, "physics.rigidBody.add", json!({"linearDamping":0, "angularDamping":0}));
    run(&mut doc, "add.empty", json!({"name":"Motor"}));
    run(&mut doc, "physics.constraint.add", json!({"type":"motor", "object1":"Anchor", "object2":"Body"}));
    run(&mut doc, "physics.world.set", json!({"gravity":[0,0,0], "frameEnd":25}));
    doc
}
fn settings(doc: &SceneDocument) -> Value {
    doc.inspect().unwrap()["physics"]["objects"].as_array().unwrap().iter().find_map(|object| object.get("rigidConstraint")).unwrap().clone()
}
fn x(doc: &SceneDocument) -> f64 {
    doc.inspect().unwrap()["objects"].as_array().unwrap().iter().find(|object| object["name"] == "Body").unwrap()["worldLocation"][0]
        .as_f64()
        .unwrap()
}

#[test]
fn motor_commands_are_described_typed_and_mutating() {
    let doc = setup();
    for id in ["physics.constraint.add", "physics.constraint.set"] {
        let command = doc.commands().into_iter().find(|c| c.id == id).unwrap();
        assert!(command.mutates);
        assert!(!command.description.is_empty());
        let schema = command.schema.unwrap();
        let params = json!({"type":"motor", "linearMotor":true,"angularMotor":false,"linearTargetVelocity":-2,"angularTargetVelocity":3,"linearMaxImpulse":4,"angularMaxImpulse":5});
        kition_kit::schema::check_params(&schema, params.as_object().unwrap()).unwrap();
        for key in ["linearMotor", "angularMotor", "linearTargetVelocity", "angularTargetVelocity", "linearMaxImpulse", "angularMaxImpulse"] {
            assert!(schema[key].is_object(), "{key}");
        }
    }
}

#[test]
fn motor_settings_undo_redo_and_reopen_then_rebake() {
    let mut doc = setup();
    let defaults = settings(&doc);
    assert_eq!(defaults["linearMotor"], false);
    assert_eq!(defaults["angularMotor"], false);
    run(
        &mut doc,
        "physics.constraint.set",
        json!({"objects":"Motor", "linearMotor":true,"angularMotor":true,"linearTargetVelocity":-2,"angularTargetVelocity":-1,"linearMaxImpulse":100,"angularMaxImpulse":10}),
    );
    let driven = settings(&doc);
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(settings(&doc), defaults);
    run(&mut doc, "edit.redo", json!({}));
    assert_eq!(settings(&doc), driven);
    run(&mut doc, "physics.bake", json!({"wait":false}));
    assert_eq!(doc.inspect().unwrap()["physics"]["state"], "baking");
    for _ in 0..10 {
        if run(&mut doc, "physics.bakeStep", json!({"frames":4}))["state"] == "baked" {
            break;
        }
    }
    assert_eq!(doc.inspect().unwrap()["physics"]["state"], "baked");
    run(&mut doc, "frame.set", json!({"frame":25}));
    assert!((x(&doc) - 4.0).abs() < 0.03);
    run(&mut doc, "physics.constraint.set", json!({"objects":"Motor", "linearTargetVelocity":2}));
    assert_eq!(doc.inspect().unwrap()["physics"]["state"], "baking");
    run(&mut doc, "physics.bake", json!({}));
    assert!(x(&doc) < 0.03);
    let saved_settings = settings(&doc);
    let mut back = doc.reopen("Saved.kscene").unwrap();
    assert_eq!(settings(&back), saved_settings);
    assert_eq!(back.inspect().unwrap()["physics"]["state"], "none", "bakes are derived, never stored");
    run(&mut back, "physics.bake", json!({}));
    assert!((x(&back) - x(&doc)).abs() < 1e-6);
    run(&mut back, "physics.constraint.remove", json!({"objects":"Motor"}));
    run(&mut back, "edit.undo", json!({}));
    assert_eq!(settings(&back), saved_settings);
}

#[test]
fn invalid_motor_edits_are_atomic_including_multi_object_operations() {
    let mut doc = setup();
    let before = doc.save_file(None).unwrap();
    for patch in [
        json!({"linearMaxImpulse":-1}),
        json!({"angularMaxImpulse":1e30}),
        json!({"linearTargetVelocity":1e30}),
        json!({"angularTargetVelocity":-1e30}),
        json!({"linearMotor":"yes"}),
        json!({"angularMotor":null}),
        json!({"object2":"Motor"}),
    ] {
        let mut params = json!({"objects":"Motor", "linearMotor":true});
        params.as_object_mut().unwrap().extend(patch.as_object().unwrap().clone());
        assert!(doc.execute("physics.constraint.set", params).is_err());
        assert_eq!(doc.save_file(None).unwrap(), before);
    }
    assert!(doc.execute("physics.constraint.set", json!({"objects":["Motor","Body"],"linearMotor":true})).is_err());
    assert_eq!(doc.save_file(None).unwrap(), before);
}

#[test]
fn released_015_file_takes_motor_settings_without_new_kdm_fields() {
    let mut doc = SceneDocument::open("Released.kscene", include_bytes!("fixtures/released-0.15.0.kscene")).unwrap();
    let original = doc.scene().object_named("Cube").unwrap().data.clone();
    run(&mut doc, "add.empty", json!({"name":"Motor"}));
    run(
        &mut doc,
        "physics.constraint.add",
        json!({"type":"motor","object1":"Anchor","object2":"Cube","linearMotor":true,"angularMotor":true,"linearTargetVelocity":-3,"angularTargetVelocity":2,"linearMaxImpulse":1,"angularMaxImpulse":0.5}),
    );
    let driven = settings(&doc);
    run(&mut doc, "edit.undo", json!({}));
    assert!(doc.scene().object_named("Motor").unwrap().physics.as_ref().is_none_or(|p| p.rigid_constraint.is_none()));
    run(&mut doc, "edit.redo", json!({}));
    assert_eq!(settings(&doc), driven);
    let mut back = doc.reopen("Saved.kscene").unwrap();
    assert_eq!(settings(&back), driven);
    assert_eq!(back.scene().object_named("Cube").unwrap().data, original);
    run(&mut back, "physics.bake", json!({}));
    run(&mut back, "frame.set", json!({"frame":12}));
    let cube = back.inspect().unwrap()["objects"].as_array().unwrap().iter().find(|o| o["name"] == "Cube").unwrap().clone();
    assert!(cube["worldLocation"][0].as_f64().unwrap() > 0.04);
}
