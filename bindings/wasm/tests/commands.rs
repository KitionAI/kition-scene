//! Every command says what it does, and read commands declare what they answer
//! (`kition_kit::testing::check_commands`, kition-sdk W1-46 P2-1).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn sample() -> SceneDocument {
    SceneDocument::open("basic.kscene", include_bytes!("../../../conformance/basic.kscene")).unwrap()
}

#[test]
fn every_command_says_what_it_does() {
    if let Err(problems) = kition_kit::testing::check_commands(&sample().commands(), kition_scene_kit::described::NO_OUTPUT) {
        panic!("{}", problems.join("\n"));
    }
}

/// Parameters a read command needs on `basic.kscene`.
fn sample_params(id: &str) -> Value {
    match id {
        "select.objects" => json!({ "objects": [2] }),
        "mode.set" => json!({ "mode": "edit" }),
        "view.shading" => json!({ "mode": "solid" }),
        "view.zoom" => json!({ "factor": 1.5 }),
        "view.resize" => json!({ "width": 640, "height": 480 }),
        "view.scheme" => json!({ "scheme": "dark" }),
        "frame.set" => json!({ "frame": 10 }),
        "file.export" => json!({ "format": "obj" }),
        "presence.peers" => json!({ "peers": [] }),
        "view.restore" => json!({ "state": { "v": 1 } }),
        "mesh.select" => json!({ "all": true }),
        "mesh.selectMode" => json!({ "mode": "vertex" }),
        _ => json!({}),
    }
}

#[test]
fn every_schema_is_valid() {
    for command in sample().commands() {
        if let Some(schema) = &command.schema {
            kition_kit::schema::validate(schema).unwrap_or_else(|error| panic!("{}: {error}", command.id));
        }
        if let Some(schema) = &command.output_schema {
            kition_kit::schema::validate_output(schema).unwrap_or_else(|error| panic!("{}: {error}", command.id));
        }
    }
}

/// Read commands answer what they declare, on `basic.kscene` with its objects selected or in edit
/// mode when they need it. Commands that need objects `basic.kscene` lacks (armatures, curves,
/// lattices, texts, particles, physics, keyframes, a running path trace) are left out.
#[test]
fn read_commands_answer_what_they_declare() {
    let mut checked = 0;
    for command in sample().commands() {
        let Some(output) = command.output_schema.clone() else { continue };
        let run = |setup: &[(&str, Value)]| {
            let mut doc = sample();
            for (id, params) in setup {
                let _ = doc.execute(id, params.clone());
            }
            doc.execute(&command.id, sample_params(&command.id))
        };
        let edit = [("select.all", json!({})), ("mode.set", json!({ "mode": "edit" }))];
        let Ok(answer) = run(&[]).or_else(|_| run(&[("select.all", json!({}))])).or_else(|_| run(&edit)) else { continue };
        let schema = json!({ "answer": output });
        let answer = serde_json::Map::from_iter([("answer".to_owned(), answer)]);
        if let Err(error) = kition_kit::schema::check_params(&schema, &answer) {
            panic!("{}: {error}", command.id);
        }
        checked += 1;
    }
    assert!(checked >= 40, "only {checked} read commands could be checked");
}
