//! The Kition SDK contract, and a scene built the way the agent builds one.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn starter() -> Vec<u8> {
    let mut doc = SceneDocument::create("Untitled scene.kscene", 1280, 720).unwrap();
    doc.save_file(None).unwrap()
}

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

/// Runs a command as the agent does (its steps undo in the agent's scope, engine contract 2).
fn agent(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    use kition_kit::session::KitSessionDocument;
    doc.execute_as(command, params, &kition_kit::kdm::Origin::agent("agent")).unwrap_or_else(|error| panic!("{command}: {error}"))
}

#[test]
fn keeps_the_kit_sdk_contract() {
    // `check_contract` compares the reopened file's pixels with the edited document's: the new
    // sphere, selected, is outlined, and a selection is the person's view state, not the file's, so
    // the edit leaves nothing selected.
    kition_kit::testing::check_contract::<SceneDocument>(&starter(), "fixture.kscene", |doc| {
        doc.execute("add.uvSphere", json!({ "location": [3, 0, 1] }))?;
        doc.execute("select.none", json!({}))?;
        Ok(())
    })
    .unwrap();
}

#[test]
fn the_agent_builds_a_table() {
    let mut doc = SceneDocument::open("t.kscene", &starter()).unwrap();
    agent(&mut doc, "edit.delete", json!({ "objects": "Cube" }));
    let top = agent(&mut doc, "add.cube", json!({ "name": "Top", "location": [0, 0, 0.75], "scale": [1, 0.6, 0.04] }));
    assert_eq!(top["name"], "Top");
    for (index, [x, y]) in [[0.9, 0.5], [-0.9, 0.5], [0.9, -0.5], [-0.9, -0.5]].into_iter().enumerate() {
        agent(
            &mut doc,
            "add.cylinder",
            json!({ "name": format!("Leg {}", index + 1), "radius": 0.04, "depth": 0.72, "vertices": 16, "location": [x, y, 0.36] }),
        );
    }
    let wood = agent(&mut doc, "material.new", json!({ "name": "Wood", "baseColor": "#8b5a2b", "roughness": 0.6, "assign": false }));
    agent(&mut doc, "material.assign", json!({ "material": "Wood", "objects": ["Top", "Leg 1", "Leg 2", "Leg 3", "Leg 4"] }));
    agent(&mut doc, "object.parent", json!({ "objects": ["Leg 1", "Leg 2", "Leg 3", "Leg 4"], "parent": "Top" }));
    agent(&mut doc, "object.lookAt", json!({ "object": "Camera", "target": [0, 0, 0.5] }));

    let inspected = doc.inspect().unwrap();
    let objects = inspected["objects"].as_array().unwrap();
    assert_eq!(objects.iter().filter(|object| object["type"] == "mesh").count(), 5);
    let leg = objects.iter().find(|object| object["name"] == "Leg 1").unwrap();
    assert_eq!(leg["parent"], top["object"]);
    assert_eq!(leg["material"], wood["material"]);
    // Parenting kept the leg where it was.
    let world = leg["worldLocation"].as_array().unwrap();
    assert!((world[0].as_f64().unwrap() - 0.9).abs() < 1e-3 && (world[2].as_f64().unwrap() - 0.36).abs() < 1e-3, "{world:?}");
    let check = doc.check();
    assert!(check.iter().all(|diagnostic| diagnostic.code != "scene.outOfShot"), "{check:?}");

    // Export and render.
    let glb = agent(&mut doc, "file.export", json!({ "format": "glb" }));
    assert_eq!(glb["mime"], "model/gltf-binary");
    assert!(glb["base64"].as_str().unwrap().len() > 1000);
    agent(&mut doc, "render.set", json!({ "width": 320, "height": 180 }));
    let png = agent(&mut doc, "render.image", json!({ "samples": 1 }));
    assert_eq!(png["width"], 320);

    // Everything undoes back to the starter scene.
    while doc.inspect().unwrap()["canUndo"] == true {
        agent(&mut doc, "edit.undo", json!({ "scope": "agent" }));
    }
    let names: Vec<String> =
        doc.inspect().unwrap()["objects"].as_array().unwrap().iter().map(|object| object["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(names, ["Cube", "Light", "Camera"]);
}

#[test]
fn edit_mode_extrudes_insets_and_cuts() {
    let mut doc = SceneDocument::open("t.kscene", &starter()).unwrap();
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "mode.set", json!({ "mode": "edit" }));
    // The top face: the one whose corners all have z = 1.
    let top = 5;
    run(&mut doc, "mesh.select", json!({ "faces": [top] }));
    run(&mut doc, "mesh.inset", json!({ "thickness": 0.2 }));
    run(&mut doc, "mesh.extrude", json!({ "distance": 0.5 }));
    let mesh = &doc.inspect().unwrap()["objects"][0]["mesh"];
    assert_eq!((mesh["vertices"].as_u64(), mesh["faces"].as_u64()), (Some(16), Some(14)));
    run(&mut doc, "mesh.loopCut", json!({ "edge": [0, 1] }));
    assert!(doc.check().is_empty(), "{:?}", doc.check());
    run(&mut doc, "mode.set", json!({ "mode": "object" }));
    run(&mut doc, "modifier.add", json!({ "type": "subdivision", "levels": 2 }));
    let cube = &doc.inspect().unwrap()["objects"][0];
    assert!(cube["mesh"]["evaluated"]["faces"].as_u64().unwrap() > cube["mesh"]["faces"].as_u64().unwrap() * 10);
    run(&mut doc, "modifier.apply", json!({ "index": 0 }));
    assert!(doc.inspect().unwrap()["objects"][0].get("modifiers").is_none());
}

#[test]
fn bad_input_is_refused_and_changes_nothing() {
    let mut doc = SceneDocument::open("t.kscene", &starter()).unwrap();
    let before = doc.save_file(None).unwrap();
    for (command, params) in [
        ("add.uvSphere", json!({ "segments": 2 })),
        ("add.cube", json!({ "location": [0, 0] })),
        ("object.transform", json!({ "objects": "Nope", "location": [0, 0, 0] })),
        ("object.parent", json!({ "objects": ["Cube"], "parent": "Cube" })),
        ("mesh.extrude", json!({})),
        ("modifier.add", json!({ "objects": "Cube", "type": "explode" })),
        ("mesh.setGeometry", json!({ "object": "Cube", "positions": [[0, 0, 0]], "faces": [[0, 1, 2]] })),
        ("file.import", json!({ "base64": "AAAA", "format": "glb" })),
        ("render.set", json!({ "width": 0 })),
        ("view.resize", json!({ "width": 100000, "height": 10 })),
    ] {
        assert!(doc.execute(command, params.clone()).is_err(), "{command} {params} was accepted");
    }
    assert_eq!(doc.save_file(None).unwrap(), before);
    assert!(!doc.dirty());
}

#[test]
fn edge_mode_hidden_objects_and_materials() {
    let mut doc = SceneDocument::open("t.kscene", &starter()).unwrap();
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "mode.set", json!({ "mode": "edit" }));
    run(&mut doc, "mesh.selectMode", json!({ "mode": "edge" }));
    assert_eq!(doc.inspect().unwrap()["mode"]["select"], "edge");
    run(&mut doc, "mode.set", json!({ "mode": "object" }));
    run(&mut doc, "object.hide", json!({ "objects": ["Cube", "Light"] }));
    run(&mut doc, "object.reveal", json!({ "objects": "Light" }));
    let objects = doc.inspect().unwrap()["objects"].clone();
    assert_eq!(objects[0]["hidden"], true);
    assert!(objects[1].get("hidden").is_none());
    run(&mut doc, "object.reveal", json!({}));
    run(&mut doc, "material.clear", json!({ "objects": "Cube" }));
    assert!(doc.inspect().unwrap()["objects"][0].get("material").is_none());
}

#[test]
fn bevel_boolean_and_screw_through_commands() {
    let mut doc = SceneDocument::open("t.kscene", &starter()).unwrap();
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "mode.set", json!({ "mode": "edit" }));
    run(&mut doc, "mesh.bevel", json!({ "width": 0.1 }));
    let mesh = &doc.inspect().unwrap()["objects"][0]["mesh"];
    assert_eq!((mesh["vertices"].as_u64(), mesh["faces"].as_u64()), (Some(24), Some(26)), "the reference's all-edges bevel");
    run(&mut doc, "edit.undo", json!({}));
    run(&mut doc, "mode.set", json!({ "mode": "object" }));

    run(&mut doc, "add.cube", json!({ "name": "Cutter", "location": [1, 1, 1] }));
    let added = run(&mut doc, "modifier.add", json!({ "object": "Cube", "type": "boolean", "operand": "Cutter" }));
    assert_eq!(added["object"], doc.inspect().unwrap()["objects"][0]["id"]);
    let cube = doc.inspect().unwrap()["objects"][0].clone();
    assert_eq!(cube["modifiers"][0]["operand"], doc.inspect().unwrap()["objects"][3]["id"]);
    assert_eq!(cube["mesh"]["evaluated"]["vertices"], 14, "{cube}");
    assert!(doc.execute("modifier.add", json!({ "object": "Cube", "type": "boolean", "operand": "Cube" })).is_err());

    run(&mut doc, "modifier.add", json!({ "object": "Cube", "type": "bevel", "angleLimit": 45, "segments": 2 }));
    assert_eq!(doc.inspect().unwrap()["objects"][0]["modifiers"][1]["angleLimit"], 45.0);
    run(&mut doc, "modifier.set", json!({ "object": "Cube", "index": 1, "angleLimit": 60 }));
    assert_eq!(doc.inspect().unwrap()["objects"][0]["modifiers"][1]["angleLimit"], 60.0);
}

#[test]
fn path_traced_renders_run_in_steps_and_change_nothing() {
    let mut doc = SceneDocument::open("t.kscene", &starter()).unwrap();
    run(&mut doc, "render.set", json!({ "width": 64, "height": 36 }));
    let saved = doc.save_file(None).unwrap();
    let revision = doc.revision();
    let undo = doc.inspect().unwrap()["undo"].clone();
    assert!(doc.execute("render.trace.step", json!({})).is_err(), "no render yet");
    let started = run(&mut doc, "render.trace.start", json!({ "samples": 3 }));
    assert_eq!((started["width"].as_u64(), started["target"].as_u64(), started["done"].as_bool()), (Some(64), Some(3), Some(false)));
    let mut steps = 0;
    loop {
        let progress = run(&mut doc, "render.trace.step", json!({ "budgetMs": 1, "preview": steps == 0 }));
        if steps == 0 {
            assert!(progress["base64"].as_str().unwrap().len() > 100);
        }
        steps += 1;
        if progress["done"] == true {
            break;
        }
        assert!(steps < 10_000);
    }
    let image = run(&mut doc, "render.trace.image", json!({}));
    assert_eq!(image["mime"], "image/png");
    // A whole render at once matches the stepped one (the same seed and samples).
    let waited = run(&mut doc, "render.trace.start", json!({ "samples": 3, "wait": true }));
    assert_eq!(waited["base64"], image["base64"]);
    assert_eq!(run(&mut doc, "render.trace.cancel", json!({}))["cancelled"], true);
    assert!(doc.execute("render.trace.image", json!({})).is_err());
    assert_eq!(doc.inspect().unwrap()["undo"], undo);
    assert_eq!(doc.revision(), revision);
    assert_eq!(doc.save_file(None).unwrap(), saved);
    assert!(doc.execute("render.trace.start", json!({ "samples": 0 })).is_err());

    // Split across workers: each opens the snapshot and traces a shard; the collector merges
    // their sums into the same picture.
    let snapshot = doc.save_file(None).unwrap();
    run(&mut doc, "render.trace.start", json!({ "samples": 3, "collect": true }));
    for index in 0..3 {
        let mut worker = SceneDocument::open("Scene.kscene", &snapshot).unwrap();
        let shard = worker.trace_shard(&json!({ "index": index, "count": 3, "samples": 3 }).to_string()).unwrap();
        assert_eq!(shard["done"], false);
        while worker.trace_step(50.0).unwrap()["done"] != true {}
        let merged = doc.trace_merge(&worker.trace_sums().unwrap()).unwrap();
        assert_eq!(merged["done"], index == 2);
    }
    assert_eq!(run(&mut doc, "render.trace.image", json!({}))["base64"], image["base64"]);
    assert!(doc.trace_merge(b"KCTS").is_err());
    assert!(doc.trace_shard(&json!({ "index": 3, "count": 3 }).to_string()).is_err());
    assert_eq!(doc.revision(), revision);
    assert_eq!(doc.save_file(None).unwrap(), saved);
}
