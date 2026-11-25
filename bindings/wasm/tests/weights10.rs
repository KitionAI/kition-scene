//! Typed field weights, independent caches, history, persistence, and explicit bake recovery.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use glam::Vec3;
use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};
fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}
fn setup() -> SceneDocument {
    let mut doc = SceneDocument::create("Weights.kscene", 640, 360).unwrap();
    run(&mut doc, "edit.selectAll", json!({}));
    run(&mut doc, "edit.delete", json!({}));
    run(&mut doc, "object.addPlane", json!({"name":"Cloth","size":1}));
    run(&mut doc, "physics.cloth.add", json!({"airDamping":0}));
    run(&mut doc, "object.addPlane", json!({"name":"Soft","size":1,"location":[3,0,0]}));
    run(&mut doc, "physics.softBody.add", json!({"goal":false,"edges":false,"friction":0}));
    run(&mut doc, "physics.field.new", json!({"name":"Wind","type":"wind","strength":2,"flow":0}));
    run(&mut doc, "physics.world.set", json!({"frameEnd":12}));
    doc
}
fn weights(doc: &SceneDocument, name: &str, system: &str) -> Value {
    let id = doc.scene().object_named(name).unwrap().id;
    doc.inspect().unwrap()["physics"]["objects"].as_array().unwrap().iter().find(|object| object["id"] == id).unwrap()[system]["effectorWeights"]
        .clone()
}
fn track(doc: &SceneDocument, name: &str) -> Vec<Vec<Vec3>> {
    let id = doc.scene().object_named(name).unwrap().id;
    doc.scene().physics_cache.baked.as_ref().unwrap().cloths[&id].clone()
}
fn state(doc: &SceneDocument) -> Value {
    doc.inspect().unwrap()["physics"].clone()
}

#[test]
fn partial_weights_are_typed_atomic_undoable_and_persistent() {
    let mut doc = setup();
    let initial = weights(&doc, "Cloth", "cloth");
    assert_eq!(initial.as_object().unwrap().len(), 7);
    assert!(initial.as_object().unwrap().values().all(|value| value.as_f64() == Some(1.0)));
    assert_eq!(initial, weights(&doc, "Soft", "softBody"));
    let before_multi = doc.save_file(None).unwrap();
    assert!(doc.execute("physics.cloth.set", json!({"objects":["Cloth","Soft"],"effectorWeights":{"wind":0}})).is_err());
    assert_eq!(doc.save_file(None).unwrap(), before_multi, "a mixed valid/invalid target list is one atomic edit");
    for id in ["physics.cloth.add", "physics.cloth.set", "physics.softBody.add", "physics.softBody.set"] {
        let command = doc.commands().into_iter().find(|command| command.id == id).unwrap();
        assert!(command.mutates);
        assert!(command.description.contains("effectorWeights"));
        let schema = command.schema.unwrap();
        kition_kit::schema::validate(&schema).unwrap();
        assert_eq!(schema["effectorWeights"]["properties"].as_object().unwrap().len(), 7);
        kition_kit::schema::check_params(&schema, json!({"effectorWeights":{"gravity":-200,"wind":200}}).as_object().unwrap()).unwrap();
    }
    for (name, system) in [("Cloth", "cloth"), ("Soft", "softBody")] {
        let command = format!("physics.{system}.set");
        run(&mut doc, &command, json!({"objects":name,"effectorWeights":{"gravity":-200,"wind":200}}));
        run(&mut doc, &command, json!({"objects":name,"effectorWeights":{"all":0}}));
        let changed = weights(&doc, name, system);
        assert_eq!(changed["gravity"].as_f64(), Some(-200.0));
        assert_eq!(changed["wind"].as_f64(), Some(200.0));
        assert_eq!(changed["all"].as_f64(), Some(0.0));
        assert_eq!(changed["force"].as_f64(), Some(1.0));
        let before = doc.save_file(None).unwrap();
        for invalid in [
            json!(null),
            json!([]),
            json!({"gravity":null}),
            json!({"gravity":"0"}),
            json!({"all":true}),
            json!({"wind":200.001}),
            json!({"gravity":-200.001}),
            json!({"force":1e100}),
            json!({"unknown":1}),
            json!({"gravity":0,"drag":false}),
        ] {
            let result = doc.execute(&command, json!({"objects":name,"effectorWeights":invalid}));
            assert!(result.is_err(), "{invalid}");
            assert_eq!(doc.save_file(None).unwrap(), before, "invalid edit must be atomic: {invalid}");
        }
        run(&mut doc, "edit.undo", json!({}));
        assert_eq!(weights(&doc, name, system)["all"].as_f64(), Some(1.0));
        run(&mut doc, "edit.redo", json!({}));
        assert_eq!(weights(&doc, name, system), changed);
    }
    let reopened = SceneDocument::open("Weights.kscene", &doc.save_file(None).unwrap()).unwrap();
    assert_eq!(weights(&doc, "Cloth", "cloth"), weights(&reopened, "Cloth", "cloth"));
    assert_eq!(weights(&doc, "Soft", "softBody"), weights(&reopened, "Soft", "softBody"));
}

#[test]
fn each_system_invalidates_and_replays_its_weighted_bake_without_changing_the_other() {
    let mut doc = setup();
    run(&mut doc, "physics.bake", json!({}));
    let cloth = track(&doc, "Cloth");
    let soft = track(&doc, "Soft");
    assert_ne!(cloth.first(), cloth.last());
    assert_ne!(soft.first(), soft.last());
    run(&mut doc, "physics.cloth.set", json!({"objects":"Cloth","effectorWeights":{"gravity":0,"wind":0}}));
    assert_eq!(state(&doc)["state"], "baking");
    run(&mut doc, "physics.bake", json!({}));
    let stopped = track(&doc, "Cloth");
    assert!(stopped.iter().all(|frame| frame == &stopped[0]));
    assert_eq!(track(&doc, "Soft"), soft);
    for frame in [12, 1, 7, 12] {
        run(&mut doc, "frame.set", json!({"frame":frame}));
    }
    assert_eq!(track(&doc, "Cloth"), stopped);
    run(&mut doc, "edit.undo", json!({}));
    run(&mut doc, "physics.bake", json!({}));
    assert_eq!(track(&doc, "Cloth"), cloth);
    run(&mut doc, "edit.redo", json!({}));
    run(&mut doc, "physics.softBody.set", json!({"objects":"Soft","effectorWeights":{"gravity":0,"wind":0}}));
    let mut reopened = SceneDocument::open("Weights.kscene", &doc.save_file(None).unwrap()).unwrap();
    assert_eq!(state(&reopened)["state"], "none");
    run(&mut reopened, "physics.bake", json!({"wait":false}));
    for _ in 0..4 {
        run(&mut reopened, "physics.bakeStep", json!({"frames":3}));
    }
    assert_eq!(state(&reopened)["state"], "baked");
    assert_eq!(track(&reopened, "Cloth"), stopped);
    let stopped_soft = track(&reopened, "Soft");
    assert!(stopped_soft.iter().all(|frame| frame == &stopped_soft[0]));
}

#[test]
fn failed_bakes_clear_partial_results_stop_following_and_allow_explicit_recovery() {
    for system in ["cloth", "softBody"] {
        for wait in [true, false] {
            let mut doc = setup();
            run(&mut doc, "physics.bake", json!({}));
            run(&mut doc, "physics.world.set", json!({"gravity":[0,0,0]}));
            run(&mut doc, "physics.field.set", json!({"objects":"Wind","type":"vortex","strength":10000}));
            run(&mut doc, "object.transform", json!({"objects":"Wind","location":[6,0,0]}));
            let name = if system == "cloth" { "Cloth" } else { "Soft" };
            let (other_name, other_system) = if system == "cloth" { ("Soft", "softBody") } else { ("Cloth", "cloth") };
            run(&mut doc, &format!("physics.{other_system}.set"), json!({"objects":other_name,"effectorWeights":{"all":0,"gravity":0}}));
            run(&mut doc, &format!("physics.{system}.set"), json!({"objects":name,"effectorWeights":{"all":200,"vortex":200}}));
            let before = doc.save_file(None).unwrap();
            let result = if wait {
                doc.execute("physics.bake", json!({}))
            } else {
                run(&mut doc, "physics.bake", json!({"wait":false}));
                doc.execute("physics.bakeStep", json!({"frames":12}))
            };
            let error = result.unwrap_err();
            assert!(error.starts_with("failed:"), "{error}");
            assert!(error.contains("nonfinite"), "{error}");
            assert!(error.contains(&format!("object {} ", doc.scene().object_named(name).unwrap().id)), "{error}");
            assert_eq!(state(&doc)["state"], "failed");
            assert_eq!(state(&doc)["frames"], 0);
            assert_eq!(state(&doc)["error"], error);
            assert!(doc.scene().physics_cache.baked.is_none());
            assert_eq!(doc.save_file(None).unwrap(), before, "a failed derived bake does not edit the document");
            doc.scene().validate().unwrap();
            run(&mut doc, "frame.set", json!({"frame":12}));
            run(&mut doc, "physics.bakeStep", json!({}));
            assert_eq!(state(&doc)["state"], "failed");
            run(&mut doc, "physics.field.set", json!({"objects":"Wind","strength":0}));
            assert_eq!(state(&doc)["state"], "failed", "editing does not enter an endless automatic retry");
            run(&mut doc, "physics.bake", json!({}));
            assert_eq!(state(&doc)["state"], "baked");
            assert!(state(&doc).get("error").is_none());
            for name in ["Cloth", "Soft"] {
                assert!(track(&doc, name).iter().flatten().all(|point| point.is_finite()));
            }
            run(&mut doc, "physics.free", json!({}));
            assert_eq!(state(&doc)["state"], "none");
        }
    }
}

#[test]
fn failed_initial_recording_does_not_retain_or_restart_a_stale_bake() {
    for automatic in [false, true] {
        let mut doc = setup();
        if automatic {
            run(&mut doc, "physics.bake", json!({}));
        }
        run(&mut doc, "object.transform", json!({"objects":"Cloth","scale":[0,1,1]}));
        doc.scene().validate().unwrap();
        let before = doc.save_file(None).unwrap();
        if !automatic {
            let error = doc.execute("physics.bake", json!({"wait":false})).unwrap_err();
            assert!(error.starts_with("failed:"));
        }
        assert_eq!(state(&doc)["state"], "failed");
        assert_eq!(state(&doc)["frames"], 0);
        assert!(doc.scene().physics_cache.baked.is_none());
        assert_eq!(doc.save_file(None).unwrap(), before);
        run(&mut doc, "object.transform", json!({"objects":"Cloth","scale":[1,1,1]}));
        assert_eq!(state(&doc)["state"], "failed");
        run(&mut doc, "physics.bakeStep", json!({}));
        assert_eq!(state(&doc)["state"], "failed");
        run(&mut doc, "physics.bake", json!({}));
        assert_eq!(state(&doc)["state"], "baked");
        assert!(state(&doc).get("error").is_none());
    }
}
