//! NLA groups/transitions and Limits through the document transaction and file boundaries.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};
fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|e| panic!("{command}: {e}"))
}
fn released() -> SceneDocument {
    SceneDocument::open("Released.kscene", include_bytes!("fixtures/released-0.13.0.kscene")).unwrap()
}
fn cube(doc: &SceneDocument) -> Value {
    doc.inspect().unwrap()["objects"].as_array().unwrap().iter().find(|o| o["name"] == "Cube").unwrap().clone()
}
fn at(doc: &mut SceneDocument, frame: f64) -> f64 {
    run(doc, "frame.set", json!({"frame":frame}));
    cube(doc)["location"][0].as_f64().unwrap()
}
fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-4, "{actual} != {expected}");
}

#[test]
fn released_file_takes_transition_meta_and_round_trip_undo() {
    let mut doc = released();
    let before = cube(&doc)["nla"].clone();
    assert_eq!(before["tracks"][0]["strips"].as_array().unwrap().len(), 2);
    near(at(&mut doc, 16.0), 10.0);
    run(&mut doc, "nla.addTransition", json!({"object":"Cube","track":0,"strip":0}));
    assert_eq!(cube(&doc)["nla"]["tracks"][0]["strips"][1]["kind"], "transition");
    near(at(&mut doc, 16.0), 5.0);
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(cube(&doc)["nla"], before);
    run(&mut doc, "edit.redo", json!({}));
    near(at(&mut doc, 16.0), 5.0);
    run(&mut doc, "nla.makeMeta", json!({"object":"Cube","track":0,"first":0,"last":2}));
    assert_eq!(cube(&doc)["nla"]["tracks"][0]["strips"][0]["children"].as_array().unwrap().len(), 3);
    near(at(&mut doc, 16.0), 5.0);
    run(&mut doc, "nla.setStrip", json!({"object":"Cube","track":0,"strip":0,"start":11,"end":71}));
    near(at(&mut doc, 41.0), 5.0);
    near(at(&mut doc, 21.0), 5.0);
    run(&mut doc, "edit.undo", json!({}));
    near(at(&mut doc, 16.0), 5.0);
    run(&mut doc, "edit.redo", json!({}));
    let saved = doc.save_file(None).unwrap();
    let mut back = SceneDocument::open("Saved.kscene", &saved).unwrap();
    assert_eq!(cube(&back)["nla"], cube(&doc)["nla"]);
    near(at(&mut back, 41.0), 5.0);
    run(&mut back, "nla.dissolveMeta", json!({"object":"Cube","track":0,"strip":0}));
    assert_eq!(cube(&back)["nla"]["tracks"][0]["strips"].as_array().unwrap().len(), 3);
    near(at(&mut back, 41.0), 5.0);
    run(&mut back, "edit.undo", json!({}));
    assert_eq!(cube(&back)["nla"]["tracks"][0]["strips"][0]["kind"], "meta");
}

#[test]
fn limits_are_transactional_and_survive_action_tweaking_and_save() {
    let mut doc = released();
    run(&mut doc, "nla.tweak", json!({"object":"Cube","track":0,"strip":0}));
    let target = cube(&doc)["id"].clone();
    let params = json!({"target":target,"property":"location","index":0,"type":"limits","minX":3,"maxX":8,"minY":4,"maxY":6});
    let schema = doc.commands().into_iter().find(|command| command.id == "graph.setModifier").unwrap().schema.unwrap();
    kition_kit::schema::check_params(
        &schema,
        json!({"target":target,"property":"location","index":0,"modifier":0,"minX":null,"maxX":null,"minY":null,"maxY":null}).as_object().unwrap(),
    )
    .unwrap();
    run(&mut doc, "graph.addModifier", params);
    near(at(&mut doc, 1.0), 4.0);
    near(at(&mut doc, 11.0), 6.0);
    run(&mut doc, "edit.undo", json!({}));
    near(at(&mut doc, 11.0), 10.0);
    run(&mut doc, "edit.redo", json!({}));
    run(&mut doc, "graph.setModifier", json!({"target":target,"property":"location","index":0,"modifier":0,"minY":null}));
    near(at(&mut doc, 1.0), 2.0);
    run(&mut doc, "nla.tweakExit", json!({"object":"Cube"}));
    let mut back = SceneDocument::open("Saved.kscene", &doc.save_file(None).unwrap()).unwrap();
    near(at(&mut back, 1.0), 2.0);
    near(at(&mut back, 11.0), 6.0);
}

#[test]
fn invalid_group_edits_leave_the_document_unchanged() {
    let mut doc = released();
    run(&mut doc, "nla.addTransition", json!({"object":"Cube","track":0,"strip":0}));
    let before = cube(&doc)["nla"].clone();
    for (command, params) in [
        ("nla.addTransition", json!({"object":"Cube","track":0,"strip":0})),
        ("nla.makeMeta", json!({"object":"Cube","track":0,"first":1,"last":2})),
        ("nla.makeMeta", json!({"object":"Cube","track":0,"first":0,"last":99})),
        ("nla.dissolveMeta", json!({"object":"Cube","track":0,"strip":0})),
        ("nla.setStrip", json!({"object":"Cube","track":0,"strip":1,"start":12})),
    ] {
        assert!(doc.execute(command, params).is_err(), "{command}");
        assert_eq!(cube(&doc)["nla"], before);
    }
    run(&mut doc, "nla.makeMeta", json!({"object":"Cube","track":0,"first":0,"last":2}));
    let before = cube(&doc)["nla"].clone();
    for values in [json!({"end":0}), json!({"end":1e30}), json!({"scale":2}), json!({"repeat":2}), json!({"reversed":true})] {
        let mut params = json!({"object":"Cube","track":0,"strip":0});
        for (key, value) in values.as_object().unwrap() {
            params[key] = value.clone();
        }
        assert!(doc.execute("nla.setStrip", params).is_err());
        assert_eq!(cube(&doc)["nla"], before);
    }
    for _ in 0..15 {
        run(&mut doc, "nla.makeMeta", json!({"object":"Cube","track":0,"first":0,"last":0}));
    }
    let before = cube(&doc)["nla"].clone();
    assert!(doc.execute("nla.makeMeta", json!({"object":"Cube","track":0,"first":0,"last":0})).is_err());
    assert_eq!(cube(&doc)["nla"], before);
}

#[test]
fn transition_and_group_commands_count_nested_descendants() {
    use kition_kit::kdm::{ActorId, DocId, Origin, Replica};
    use kition_scene_model::animation::Action;
    use kition_scene_model::nla::{NlaStrip, NlaTrack};
    use kition_scene_model::{ObjectData, Scene};
    let mut scene = Scene::empty();
    let object = scene.add_object("Cube", ObjectData::Empty { size: 1.0 });
    let action = Action { name: "Move".into(), channels: Vec::new() };
    scene.animation.actions.push(action.clone());
    let strips = (0..1023).map(|i| NlaStrip::new(format!("Strip {i}"), &action, 1.0 + i as f32 * 3.0)).collect();
    scene.object_mut(object).unwrap().nla.tracks.push(NlaTrack { name: "Track".into(), strips, muted: false, solo: false });
    scene.nla_make_meta(object, 0, 0, 1021).unwrap();
    scene.validate().unwrap();
    let (replica, _) =
        Replica::create_with(kition_scene_kdm::schema(), DocId::new(), ActorId::from_u64(1).unwrap(), &Origin::system("create"), |tx| {
            kition_scene_kdm::write::write_scene(tx, &scene)
        })
        .unwrap();
    let file = kition_scene_kscene::File {
        meta: kition_scene_kscene::Meta { kdm: kition_kit::kdm::KDM_VERSION, schema_version: 1, doc_id: replica.doc_id().to_string() },
        content: replica.content_snapshot().unwrap(),
        ..Default::default()
    };
    let mut doc = SceneDocument::open("Full.kscene", &kition_scene_kscene::write(&file).unwrap()).unwrap();
    let before = cube(&doc)["nla"].clone();
    for (command, params) in
        [("nla.addTransition", json!({"object":"Cube","track":0,"strip":0})), ("nla.makeMeta", json!({"object":"Cube","track":0,"first":0,"last":1}))]
    {
        assert!(doc.execute(command, params).is_err(), "{command}");
        assert_eq!(cube(&doc)["nla"], before);
    }
}

#[test]
fn transitions_follow_moved_and_resized_neighbors() {
    fn bounds(doc: &SceneDocument) -> Value {
        json!(
            cube(doc)["nla"]["tracks"][0]["strips"]
                .as_array()
                .unwrap()
                .iter()
                .map(|strip| { json!({"kind":strip["kind"],"start":strip["start"],"end":strip["end"]}) })
                .collect::<Vec<_>>()
        )
    }
    let reference: Value = serde_json::from_str(include_str!("../../../crates/engine/model/tests/fixtures/nla6-move-reference.json")).unwrap();
    if let Some(blender) = std::env::var_os("KITION_BLENDER") {
        let output = std::env::temp_dir().join(format!("scene-nla6-move-{}.json", std::process::id()));
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/engine/model/tests/blender/nla6_move.py");
        let status =
            std::process::Command::new(blender).args(["-b", "--factory-startup", "--python"]).arg(script).arg("--").arg(&output).status().unwrap();
        assert!(status.success());
        let live: Value = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        assert_eq!(live, reference);
        std::fs::remove_file(output).unwrap();
    }
    let mut doc = released();
    run(&mut doc, "nla.setStrip", json!({"object":"Cube","track":0,"strip":1,"start":31}));
    run(&mut doc, "nla.addTransition", json!({"object":"Cube","track":0,"strip":0}));
    assert_eq!(bounds(&doc), reference["states"]["initial"]);
    run(&mut doc, "nla.setStrip", json!({"object":"Cube","track":0,"strip":0,"start":12}));
    assert_eq!(bounds(&doc), reference["states"]["leftMoved"]);
    run(&mut doc, "nla.setStrip", json!({"object":"Cube","track":0,"strip":0,"scale":0.5}));
    assert_eq!(bounds(&doc), reference["states"]["leftScaled"]);
    run(&mut doc, "nla.setStrip", json!({"object":"Cube","track":0,"strip":2,"start":24}));
    assert_eq!(bounds(&doc), reference["states"]["rightMoved"]);
    run(&mut doc, "nla.makeMeta", json!({"object":"Cube","track":0,"first":0,"last":0}));
    run(&mut doc, "nla.setStrip", json!({"object":"Cube","track":0,"strip":0,"start":18}));
    assert_eq!(bounds(&doc), reference["states"]["metaMoved"]);
}

/// Removing an action takes away the strips that play it inside meta strips too, with the
/// transition beside them (`Nla::follow_action`): the cube's two strips, A then B with a
/// transition between, are grouped into a meta strip and that one into another, and the person
/// removes action A by command. Only B plays; undo gives back the nested strips exactly, redo
/// takes them away again. (Before the plain model this was a concurrent removal and grouping,
/// merged; one person now reaches the same state through the command.)
#[test]
fn removing_an_action_takes_its_nested_strips_away() {
    use kition_kit::kdm::{ActorId, DocId, Origin, Replica};
    use kition_scene_model::animation::{Action, ActionChannel, Property};
    use kition_scene_model::{Interpolation, Keyframe, ObjectData, Scene};
    let mut scene = Scene::empty();
    let id = scene.add_object("Cube", ObjectData::Empty { size: 1.0 });
    for (name, value) in [("A", 0.0), ("B", 30.0)] {
        scene.animation.actions.push(Action {
            name: name.into(),
            channels: vec![ActionChannel {
                property: Property::Location,
                index: 0,
                path: String::new(),
                keys: vec![
                    Keyframe { frame: 1.0, value, interpolation: Interpolation::Linear },
                    Keyframe { frame: 11.0, value: value + 10.0, interpolation: Interpolation::Linear },
                ],
                modifiers: Vec::new(),
            }],
        });
    }
    scene.nla_add_strip(id, None, "A", 1.0).unwrap();
    scene.nla_add_strip(id, Some(0), "B", 21.0).unwrap();
    scene.nla_add_transition(id, 0, 0).unwrap();
    scene.nla_make_meta(id, 0, 0, 2).unwrap();
    scene.nla_make_meta(id, 0, 0, 0).unwrap();
    scene.validate().unwrap();
    let (replica, _) =
        Replica::create_with(kition_scene_kdm::schema(), DocId::new(), ActorId::from_u64(1).unwrap(), &Origin::system("create"), |tx| {
            kition_scene_kdm::write::write_scene(tx, &scene)
        })
        .unwrap();
    let file = kition_scene_kscene::File {
        meta: kition_scene_kscene::Meta { kdm: kition_kit::kdm::KDM_VERSION, schema_version: 1, doc_id: replica.doc_id().to_string() },
        content: replica.content_snapshot().unwrap(),
        ..Default::default()
    };
    let mut doc = SceneDocument::open("Nested.kscene", &kition_scene_kscene::write(&file).unwrap()).unwrap();
    let played = |doc: &SceneDocument| {
        let mut played = Vec::new();
        for track in &doc.scene().object_named("Cube").unwrap().nla.tracks {
            for strip in &track.strips {
                strip.visit_actions(&mut |name| played.push(name.to_string()));
            }
        }
        played
    };
    assert_eq!(played(&doc), ["A", "B"], "the control: both play, nested");
    let before = cube(&doc)["nla"].clone();
    run(&mut doc, "action.remove", json!({ "name": "A" }));
    doc.scene().validate().unwrap();
    assert_eq!(played(&doc), ["B"]);
    let removed = cube(&doc)["nla"].clone();
    let back = SceneDocument::open("Saved.kscene", &doc.save_file(None).unwrap()).unwrap();
    assert_eq!(played(&back), ["B"], "the saved file plays B alone");
    assert_eq!(cube(&back)["nla"], removed);
    back.scene().validate().unwrap();
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(cube(&doc)["nla"], before, "undo gives back the nested strips");
    assert_eq!(played(&doc), ["A", "B"]);
    run(&mut doc, "edit.redo", json!({}));
    assert_eq!(cube(&doc)["nla"], removed);
    assert_eq!(played(&doc), ["B"]);
}
