//! Soft bodies and hair dynamics through commands, as the agent and the editor drive them: the
//! settings (one undo step each, refused when out of range, saved), a bake that moves a soft
//! body's mesh and dynamic hair's strands, the bake following hair alone, and a released 0.12.0
//! file taking both.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use kition_scene_model::Id;
use kition_scene_model::particles;
use serde_json::{Value, json};

fn empty() -> SceneDocument {
    let mut doc = SceneDocument::create("Untitled.kscene", 640, 360).unwrap();
    run(&mut doc, "select.all", json!({}));
    run(&mut doc, "edit.delete", json!({}));
    doc
}

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn reopen(doc: &SceneDocument) -> SceneDocument {
    SceneDocument::open("A.kscene", &doc.save(None).unwrap()).unwrap()
}

fn id(doc: &SceneDocument, name: &str) -> Id {
    let inspected = doc.inspect().unwrap();
    let object = inspected["objects"].as_array().unwrap().iter().find(|object| object["name"] == name).unwrap_or_else(|| panic!("no {name}")).clone();
    object["id"].as_u64().unwrap() as Id
}

fn physics_of(doc: &SceneDocument, name: &str) -> Value {
    let wanted = id(doc, name);
    let inspected = doc.inspect().unwrap();
    inspected["physics"]["objects"].as_array().unwrap().iter().find(|object| object["id"] == wanted).cloned().unwrap_or(Value::Null)
}

/// The lowest point of an object's mesh as the scene shows it now (after the bake).
fn lowest(doc: &SceneDocument, name: &str) -> f32 {
    let scene = doc.scene();
    let id = id(doc, name);
    let mesh = scene.object(id).unwrap().evaluated_mesh(scene).unwrap();
    let world = scene.world_matrix(id).unwrap();
    mesh.positions.iter().map(|point| world.transform_point3(*point).z).fold(f32::INFINITY, f32::min)
}

#[test]
fn soft_body_settings_are_undoable_checked_and_saved() {
    let mut doc = empty();
    run(&mut doc, "add.cube", json!({ "name": "Jelly", "size": 2, "location": [0, 0, 3] }));
    let added = run(&mut doc, "physics.softBody.add", json!({ "objects": "Jelly", "goalDefault": 0.9, "bending": 1 }));
    assert_eq!(added["objects"][0]["softBody"]["goalDefault"].as_f64().map(|value| (value * 100.0).round()), Some(90.0));
    run(&mut doc, "physics.softBody.set", json!({ "objects": "Jelly", "pull": 0.8, "goalGroup": "Pin", "frameStart": 5, "frameEnd": 40 }));
    let soft = physics_of(&doc, "Jelly")["softBody"].clone();
    assert_eq!((soft["goalGroup"].as_str(), soft["frameStart"].as_i64(), soft["frameEnd"].as_i64()), (Some("Pin"), Some(5), Some(40)));
    run(&mut doc, "edit.undo", json!({}));
    assert!(physics_of(&doc, "Jelly")["softBody"].get("goalGroup").is_none(), "one undo step");
    // Out of range, or cloth on a soft body: nothing changes.
    let before = physics_of(&doc, "Jelly");
    for params in
        [json!({ "pull": 1.0 }), json!({ "goalMin": 0.8, "goalMax": 0.2 }), json!({ "frameStart": 50, "frameEnd": 10 }), json!({ "mass": -1 })]
    {
        let mut params = params;
        params["objects"] = json!("Jelly");
        assert!(doc.execute("physics.softBody.set", params.clone()).is_err(), "refused: {params}");
    }
    assert!(doc.execute("physics.cloth.add", json!({ "objects": "Jelly" })).is_err(), "a soft body is not also cloth");
    assert_eq!(physics_of(&doc, "Jelly"), before);
    run(&mut doc, "add.plane", json!({ "name": "Floor", "size": 10 }));
    run(&mut doc, "physics.collision.add", json!({ "objects": "Floor", "damping": 0.5 }));
    assert!(doc.execute("physics.softBody.add", json!({ "objects": "Floor", "pull": 2 })).is_err());
    assert!(physics_of(&doc, "Floor").get("softBody").is_none(), "a failed add adds nothing");
    let back = reopen(&doc);
    assert_eq!(physics_of(&back, "Jelly"), before);
    assert_eq!(physics_of(&back, "Floor")["collision"]["damping"].as_f64(), Some(0.5));
    run(&mut doc, "physics.softBody.remove", json!({ "objects": "Jelly" }));
    assert!(physics_of(&doc, "Jelly").is_null(), "no physics left");
}

#[test]
fn a_baked_soft_body_falls_onto_a_floor_and_rests_on_it() {
    let mut doc = empty();
    run(&mut doc, "add.plane", json!({ "name": "Floor", "size": 10 }));
    run(&mut doc, "physics.collision.add", json!({ "objects": "Floor" }));
    run(&mut doc, "add.cube", json!({ "name": "Jelly", "size": 1, "location": [0, 0, 2] }));
    run(&mut doc, "physics.softBody.add", json!({ "objects": "Jelly", "goal": false }));
    run(&mut doc, "physics.world.set", json!({ "frameEnd": 72 }));
    let baked = run(&mut doc, "physics.bake", json!({}));
    assert_eq!((baked["state"].as_str(), baked["frames"].as_u64()), (Some("baked"), Some(72)));
    assert!((lowest(&doc, "Jelly") - 1.5).abs() < 1e-4, "frame 1 is the start");
    run(&mut doc, "frame.set", json!({ "frame": 72 }));
    let rest = lowest(&doc, "Jelly");
    assert!((rest - 0.02).abs() < 2e-3, "rests on the floor's thickness: {rest}");
    // A softer goal is another bake: it starts by itself.
    run(&mut doc, "physics.softBody.set", json!({ "objects": "Jelly", "friction": 2 }));
    assert_eq!(doc.inspect().unwrap()["physics"]["state"], "baking");
}

#[test]
fn hair_dynamics_bake_alone_and_droop() {
    let mut doc = empty();
    run(&mut doc, "add.plane", json!({ "name": "Head", "size": 0.1, "rotation": [90, 0, 0] }));
    run(&mut doc, "particles.add", json!({ "object": "Head", "kind": "hair", "count": 4, "hairLength": 1, "segments": 4, "dynamics": true }));
    let inspected = doc.inspect().unwrap();
    assert_eq!(inspected["particles"][0]["dynamics"]["stiffness"].as_f64(), Some(0.5));
    assert_eq!(inspected["physics"]["bakeable"], true, "hair alone can be baked");
    assert!(inspected["physics"]["objects"].as_array().unwrap().is_empty());
    run(&mut doc, "particles.set", json!({ "object": "Head", "dynamics": { "stiffness": 5, "mass": 0.5 } }));
    let dynamics = doc.inspect().unwrap()["particles"][0]["dynamics"].clone();
    assert_eq!((dynamics["stiffness"].as_f64(), dynamics["mass"].as_f64(), dynamics["quality"].as_u64()), (Some(5.0), Some(0.5), Some(5)));
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(doc.inspect().unwrap()["particles"][0]["dynamics"]["stiffness"].as_f64(), Some(0.5), "one undo step");
    assert!(doc.execute("particles.set", json!({ "object": "Head", "dynamics": { "stiffness": -1 } })).is_err());
    assert!(doc.execute("particles.set", json!({ "object": "Head", "dynamics": { "springs": 1 } })).is_err(), "no such setting");
    assert!(doc.execute("particles.set", json!({ "object": "Head", "dynamics": 3 })).is_err());
    run(&mut doc, "physics.world.set", json!({ "frameEnd": 48 }));
    let tips = |doc: &SceneDocument| -> f32 {
        let strands = particles::strands(doc.scene(), id(doc, "Head"), 0, true);
        strands.iter().map(|strand| strand.last().unwrap().z - strand[0].z).sum::<f32>() / strands.len() as f32
    };
    assert!(tips(&doc).abs() < 1e-4, "level before the bake");
    assert_eq!(run(&mut doc, "physics.bake", json!({}))["state"], "baked");
    run(&mut doc, "frame.set", json!({ "frame": 48 }));
    let drooped = tips(&doc);
    assert!(drooped < -0.3, "the strands droop: {drooped}");
    // Stats see the baked strands; the length stays.
    let stats = run(&mut doc, "particles.stats", json!({ "object": "Head" }));
    assert!((stats["meanWorldLength"].as_f64().unwrap() - 1.0).abs() < 5e-3, "{stats}");
    let back = reopen(&doc);
    assert_eq!(back.inspect().unwrap()["particles"][0]["dynamics"]["stiffness"].as_f64(), Some(0.5));
    // Emitters have no strands to simulate.
    run(&mut doc, "particles.add", json!({ "object": "Head", "name": "Sparks" }));
    assert!(doc.execute("particles.set", json!({ "object": "Head", "system": "Sparks", "dynamics": true })).is_err());
    run(&mut doc, "particles.set", json!({ "object": "Head", "system": 0, "kind": "emitter" }));
    assert!(doc.inspect().unwrap()["particles"][0].get("dynamics").is_none(), "an emitter drops its hair dynamics");
}

#[test]
fn a_released_0_12_0_file_takes_soft_bodies_and_hair_dynamics() {
    let mut doc = SceneDocument::open("Lab.kscene", include_bytes!("fixtures/released-0.12.0.kscene")).unwrap();
    run(&mut doc, "physics.softBody.add", json!({ "objects": "Sheet", "goalGroup": "Pin", "goalDefault": 1, "bending": 0.5 }));
    run(&mut doc, "physics.collision.add", json!({ "objects": "Floor", "damping": 0.3 }));
    run(&mut doc, "particles.set", json!({ "object": "Head", "dynamics": { "stiffness": 2, "pinStiffness": 0.5 } }));
    run(&mut doc, "physics.world.set", json!({ "frameEnd": 12 }));
    assert_eq!(run(&mut doc, "physics.bake", json!({}))["state"], "baked");
    let back = reopen(&doc);
    let soft = physics_of(&back, "Sheet")["softBody"].clone();
    assert_eq!((soft["goalGroup"].as_str(), soft["goalDefault"].as_f64(), soft["bending"].as_f64()), (Some("Pin"), Some(1.0), Some(0.5)));
    assert_eq!(physics_of(&back, "Floor")["collision"]["damping"].as_f64().map(|value| (value * 10.0).round()), Some(3.0));
    let dynamics = back.inspect().unwrap()["particles"][0]["dynamics"].clone();
    assert_eq!((dynamics["stiffness"].as_f64(), dynamics["pinStiffness"].as_f64()), (Some(2.0), Some(0.5)));
    assert_eq!(back.inspect().unwrap()["physics"]["state"], "none", "the bake is never saved");
}
