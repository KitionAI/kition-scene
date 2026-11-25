//! Boid settings are ordinary document commands, one register, and deterministic derived data.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};
fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|e| panic!("{command}: {e}"))
}
fn settings(doc: &SceneDocument) -> Value {
    doc.inspect().unwrap()["particles"][0].clone()
}
fn setup() -> SceneDocument {
    let mut doc = SceneDocument::create("Boids.kscene", 640, 360).unwrap();
    run(&mut doc, "object.addEmpty", json!({"name":"Goal","location":[8,0,8]}));
    run(
        &mut doc,
        "particle.add",
        json!({"object":"Cube","physics":"boids","count":8,"frameEnd":1,"lifetime":100,"emitFrom":"verts","boids":{"rules":[{"kind":"goal","target":"Goal"}]}}),
    );
    doc
}
#[test]
fn boid_settings_are_typed_undoable_saved_and_seen_by_rendering() {
    let mut doc = setup();
    for id in ["particle.add", "particle.set"] {
        let command = doc.commands().into_iter().find(|command| command.id == id).unwrap();
        assert!(command.mutates);
        assert!(!command.description.is_empty());
        kition_kit::schema::check_params(
            &command.schema.unwrap(),
            json!({"physics":"boids","boids":{"airSpeedMax":6,"rules":[{"kind":"goal","target":"Goal"}]}}).as_object().unwrap(),
        )
        .unwrap();
    }
    let initial = settings(&doc);
    run(&mut doc, "particle.set", json!({"object":"Cube","boids":{"airSpeedMax":6}}));
    let modified = settings(&doc);
    assert_eq!(modified["boids"]["airSpeedMax"].as_f64(), Some(6.0));
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(settings(&doc), initial);
    run(&mut doc, "edit.redo", json!({}));
    assert_eq!(settings(&doc), modified);
    let at31 = run(&mut doc, "particle.stats", json!({"object":"Cube","frame":31}));
    assert_eq!(at31["alive"], 8);
    assert!(at31["meanPosition"][0].as_f64().unwrap() > 1.0);
    assert!(at31["meanPosition"][2].as_f64().unwrap() > 1.0);
    run(&mut doc, "frame.set", json!({"frame":1}));
    let before = doc.render(0.0, 0.0, 1.0, 640, 360).unwrap();
    run(&mut doc, "frame.set", json!({"frame":31}));
    let after = doc.render(0.0, 0.0, 1.0, 640, 360).unwrap();
    assert_ne!(before, after);
    let mut back = SceneDocument::open("Boids.kscene", &doc.save_file(None).unwrap()).unwrap();
    assert_eq!(settings(&back), modified);
    assert_eq!(run(&mut back, "particle.stats", json!({"object":"Cube","frame":31})), at31);
    for frame in [90, 1, 11, 31] {
        run(&mut back, "frame.set", json!({"frame":frame}));
    }
    assert_eq!(run(&mut back, "particle.stats", json!({"object":"Cube","frame":31})), at31);
    run(&mut back, "object.transform", json!({"objects":"Goal","location":[-8,0,8]}));
    assert!(run(&mut back, "particle.stats", json!({"object":"Cube","frame":31}))["meanPosition"][0].as_f64().unwrap() < -1.0);
    let launch = initial["normalVelocity"].clone();
    run(&mut back, "particle.set", json!({"object":"Cube","physics":"newtonian"}));
    assert_eq!(settings(&back)["normalVelocity"], launch);
    run(&mut back, "particle.set", json!({"object":"Cube","physics":"boids"}));
    assert_eq!(settings(&back)["boids"], modified["boids"]);
}
#[test]
fn invalid_boid_edits_fail_atomically_and_deleting_targets_clears_only_references() {
    let mut doc = setup();
    let before = doc.save_file(None).unwrap();
    for patch in [
        json!({"physics":null}),
        json!({"emitFrom":"volume"}),
        json!({"boids":null}),
        json!({"boids":{"unknown":true}}),
        json!({"boids":{"airSpeedMax":-1}}),
        json!({"boids":{"airAngularMax":2}}),
        json!({"boids":{"rules":[{"kind":"fight"}]}}),
        json!({"boids":{"rules":[{"kind":"goal","target":"Missing"}]}}),
        json!({"boids":{"rules":[{"kind":"goal","target":"Cube"}]}}),
        json!({"count":2049}),
        json!({"count":1000,"lifetime":501}),
        json!({"count":1000,"frameEnd":200,"lifetime":250}),
        json!({"kind":"hair"}),
        json!({"colliders":["Goal"]}),
        json!({"dieOnCollision":true}),
        json!({"timestep":1}),
    ] {
        let mut patch = patch;
        patch["object"] = json!("Cube");
        assert!(doc.execute("particle.set", patch.clone()).is_err(), "{patch}");
        assert_eq!(doc.save_file(None).unwrap(), before, "{patch}");
    }
    run(&mut doc, "select.objects", json!({"objects":"Goal"}));
    run(&mut doc, "edit.delete", json!({}));
    let cleared = settings(&doc);
    assert!(cleared["boids"]["rules"][0].get("target").is_none());
    assert_eq!(cleared["physics"], "boids");
    run(&mut doc, "edit.undo", json!({}));
    assert!(settings(&doc)["boids"]["rules"][0]["target"].is_number());
}
#[test]
fn boid_targets_use_parented_world_positions_and_scene_effects_are_explicitly_inert() {
    let mut doc = setup();
    run(&mut doc, "object.addEmpty", json!({"name":"Parent","location":[3,2,1]}));
    // The hierarchy command preserves world space; the same target must keep the same flight.
    let before = run(&mut doc, "particle.stats", json!({"object":"Cube","frame":31}));
    run(&mut doc, "object.parent", json!({"objects":"Goal","parent":"Parent"}));
    assert_eq!(run(&mut doc, "particle.stats", json!({"object":"Cube","frame":31})), before);
    run(&mut doc, "object.addPlane", json!({"name":"Floor"}));
    run(&mut doc, "physics.collision.add", json!({}));
    run(&mut doc, "physics.field.add", json!({"type":"wind"}));
    assert_eq!(run(&mut doc, "particle.stats", json!({"object":"Cube","frame":31})), before);
}

#[test]
fn typed_rules_require_their_kind_and_explicit_null_clears_a_configured_target() {
    let mut doc = setup();
    let schema = doc.commands().into_iter().find(|command| command.id == "particle.set").unwrap().schema.unwrap();
    kition_kit::schema::validate(&schema).unwrap();
    let target_types = schema["boids"]["properties"]["rules"]["items"]["properties"]["target"]["oneOf"].as_array().unwrap();
    assert!(target_types.iter().any(|branch| branch["type"] == "null"));
    for rule in [json!({}), json!({"kind":null})] {
        let params = json!({"boids":{"rules":[rule]}});
        assert!(kition_kit::schema::check_params(&schema, params.as_object().unwrap()).is_err());
    }
    let original = settings(&doc);
    assert!(original["boids"]["rules"][0]["target"].is_number());
    let clear = json!({"object":"Cube","boids":{"rules":[{"kind":"goal","target":null}]}});
    kition_kit::schema::check_params(&schema, clear.as_object().unwrap()).unwrap();
    run(&mut doc, "particle.set", clear);
    let cleared = settings(&doc);
    assert_eq!(cleared["boids"]["rules"][0]["kind"], "goal");
    assert!(cleared["boids"]["rules"][0].get("target").is_none());
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(settings(&doc), original);

    let eight = vec![json!({"kind":"goal","target":"Goal"}); 8];
    run(&mut doc, "particle.set", json!({"object":"Cube","boids":{"rules":eight}}));
    assert_eq!(settings(&doc)["boids"]["rules"].as_array().unwrap().len(), 8);
    let before = doc.save_file(None).unwrap();
    let nine = vec![json!({"kind":"goal","target":"Goal"}); 9];
    assert!(doc.execute("particle.set", json!({"object":"Cube","boids":{"rules":nine}})).is_err());
    assert_eq!(doc.save_file(None).unwrap(), before);
}
