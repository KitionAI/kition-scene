//! UV live preview, transactions, preferences and saved files through the public command surface.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}
fn chart() -> SceneDocument {
    let mut doc = SceneDocument::create("UV.kscene", 400, 300).unwrap();
    let cases: Value = serde_json::from_str(include_str!("../../../crates/engine/model/tests/fixtures/uv7-cases.json")).unwrap();
    run(&mut doc, "select.objects", json!({"objects":"Cube"}));
    run(&mut doc, "mesh.setGeometry", json!({"object":"Cube","positions":cases[0]["positions"],"faces":cases[0]["faces"]}));
    run(&mut doc, "mode.set", json!({"mode":"edit"}));
    run(&mut doc, "uv.unwrap", json!({"method":"minimumStretch","iterations":30}));
    run(&mut doc, "mesh.select", json!({"vertices":[0,4]}));
    run(&mut doc, "uv.pin", json!({}));
    run(&mut doc, "mesh.select", json!({"vertices":[4]}));
    run(&mut doc, "uv.liveUnwrap", json!({"on":true}));
    doc
}
fn layout(doc: &mut SceneDocument) -> Value {
    run(doc, "uv.layout", json!({"all":true}))
}
fn vertex(layout: &Value, vertex: u32) -> [f64; 2] {
    for face in layout["faces"].as_array().unwrap() {
        if let Some(i) = face["vertices"].as_array().unwrap().iter().position(|v| v.as_u64() == Some(u64::from(vertex))) {
            return [face["uvs"][i][0].as_f64().unwrap(), face["uvs"][i][1].as_f64().unwrap()];
        }
    }
    panic!("missing vertex")
}

#[test]
fn live_preview_is_readonly_and_commit_is_one_undo_step() {
    let mut doc = chart();
    let before = layout(&mut doc);
    let bytes = doc.save_file(None).unwrap();
    let inspected = doc.inspect().unwrap();
    let mut preview = run(&mut doc, "uv.preview", json!({"offset":[0.08,0.04]}));
    assert_eq!(doc.save_file(None).unwrap(), bytes);
    assert_eq!(doc.inspect().unwrap(), inspected);
    assert_eq!(vertex(&preview, 0), vertex(&before, 0));
    let a = vertex(&preview, 4);
    let b = vertex(&before, 4);
    assert!((a[0] - b[0] - 0.08).abs() < 1e-6 && (a[1] - b[1] - 0.04).abs() < 1e-6);
    assert_ne!(vertex(&preview, 12), vertex(&before, 12));
    let done = run(&mut doc, "uv.translate", json!({"offset":[0.08,0.04]}));
    assert_eq!(done["charts"], 1);
    let committed = layout(&mut doc);
    assert_ne!(committed["source"], preview["source"]);
    preview["source"] = committed["source"].clone();
    assert_eq!(committed, preview);
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(layout(&mut doc)["faces"], before["faces"]);
    run(&mut doc, "edit.redo", json!({}));
    assert_eq!(layout(&mut doc)["faces"], committed["faces"]);
    let mut reopened = doc.reopen("Saved.kscene").unwrap();
    run(&mut reopened, "mode.set", json!({"mode":"edit"}));
    let reopened = layout(&mut reopened);
    assert_eq!(reopened["liveUnwrap"], true);
    assert_eq!(reopened["method"], "minimumStretch");
    for (a, b) in reopened["faces"].as_array().unwrap().iter().zip(committed["faces"].as_array().unwrap()) {
        assert_eq!(a["uvs"], b["uvs"]);
    }
}

#[test]
fn settings_are_local_and_invalid_commands_preserve_all_state() {
    let mut doc = chart();
    let mut doc = doc.reopen("Clean.kscene").unwrap();
    run(&mut doc, "mode.set", json!({"mode":"edit"}));
    run(&mut doc, "uv.liveUnwrap", json!({"on":false,"method":"conformal","iterations":12}));
    assert!(!doc.dirty());
    assert_eq!(layout(&mut doc)["method"], "conformal");
    for (command, params) in [
        ("uv.liveUnwrap", json!({"on":true,"method":"bad"})),
        ("uv.liveUnwrap", json!({"on":true,"iterations":0})),
        ("uv.unwrap", json!({"method":"minimumStretch","iterations":101})),
        ("uv.preview", json!({"offset":[1e300,0]})),
        ("uv.translate", json!({"offset":[1e300,0]})),
    ] {
        let before = doc.save_file(None).unwrap();
        let view = doc.inspect().unwrap();
        assert!(doc.execute(command, params).is_err(), "{command}");
        assert_eq!(doc.save_file(None).unwrap(), before);
        assert_eq!(doc.inspect().unwrap(), view);
    }
    for id in ["uv.preview", "uv.liveUnwrap"] {
        let command = doc.commands().into_iter().find(|c| c.id == id).unwrap();
        assert!(!command.mutates);
        assert!(!command.description.is_empty());
        assert!(command.output_schema.is_some());
    }
}

#[test]
fn rotation_and_scale_relax_without_moving_stationary_pins() {
    let mut doc = chart();
    for (command, params) in [("uv.rotate", json!({"angle":10,"pivot":[0.5,0.5]})), ("uv.scale", json!({"factor":1.1,"pivot":[0.5,0.5]}))] {
        let before = layout(&mut doc);
        run(&mut doc, command, params);
        let after = layout(&mut doc);
        assert_eq!(vertex(&before, 0), vertex(&after, 0));
        assert_ne!(vertex(&before, 12), vertex(&after, 12));
        run(&mut doc, "edit.undo", json!({}));
        assert_eq!(layout(&mut doc)["faces"], before["faces"]);
    }
}

#[test]
fn source_tokens_ignore_display_changes_and_reject_changed_mesh_or_selection() {
    let mut doc = chart();
    let token = layout(&mut doc)["source"].clone();
    run(&mut doc, "view.orbit", json!({"turn":5}));
    assert_eq!(layout(&mut doc)["source"], token);
    let preview = run(&mut doc, "uv.preview", json!({"offset":[0.03,0.02],"source":token}));
    assert_eq!(preview["source"], token);
    run(&mut doc, "mesh.select", json!({"vertices":[0]}));
    assert_ne!(layout(&mut doc)["source"], token);
    for command in ["uv.preview", "uv.translate"] {
        let before = doc.save_file(None).unwrap();
        assert!(doc.execute(command, json!({"offset":[0.03,0.02],"source":token})).is_err());
        assert_eq!(doc.save_file(None).unwrap(), before);
    }
}

#[test]
fn minimum_stretch_unwrap_is_one_saved_undoable_command() {
    let mut doc = chart();
    run(&mut doc, "uv.liveUnwrap", json!({"on":false}));
    run(&mut doc, "mesh.select", json!({"vertices":(0..25).collect::<Vec<_>>()}));
    run(&mut doc, "uv.scale", json!({"factor":[1.4,0.7]}));
    let before = layout(&mut doc);
    run(&mut doc, "uv.unwrap", json!({"method":"minimumStretch","iterations":30}));
    let after = layout(&mut doc);
    assert_ne!(before["faces"], after["faces"]);
    assert_eq!(vertex(&before, 0), vertex(&after, 0));
    assert_eq!(vertex(&before, 4), vertex(&after, 4));
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(layout(&mut doc)["faces"], before["faces"]);
    run(&mut doc, "edit.redo", json!({}));
    assert_eq!(layout(&mut doc)["faces"], after["faces"]);
    let mut back = doc.reopen("Unwrapped.kscene").unwrap();
    run(&mut back, "mode.set", json!({"mode":"edit"}));
    let reopened = layout(&mut back);
    for (a, b) in reopened["faces"].as_array().unwrap().iter().zip(after["faces"].as_array().unwrap()) {
        assert_eq!(a["uvs"], b["uvs"]);
    }
}
