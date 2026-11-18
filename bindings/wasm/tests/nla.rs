//! The NLA and F-curve modifiers through commands, as the agent and the editor drive them: push
//! down, strips and their settings, tweak mode, modifiers on a channel; each command one undo
//! step, everything surviving a save, and a released 0.11.0 file taking them.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn object<'a>(inspected: &'a Value, name: &str) -> &'a Value {
    inspected["objects"].as_array().unwrap().iter().find(|object| object["name"] == name).unwrap_or_else(|| panic!("no {name}"))
}

fn number(value: &Value) -> f64 {
    value.as_f64().unwrap_or_else(|| panic!("not a number: {value}"))
}

fn reopen(doc: &SceneDocument) -> SceneDocument {
    SceneDocument::open("A.kscene", &doc.save(None).unwrap()).unwrap()
}

/// The cube's location at `frame`.
fn at(doc: &mut SceneDocument, frame: f64) -> [f64; 3] {
    run(doc, "frame.set", json!({ "frame": frame }));
    let inspected = doc.inspect().unwrap();
    let location = &object(&inspected, "Cube")["location"];
    [number(&location[0]), number(&location[1]), number(&location[2])]
}

/// A cube rising from z 0 to 2 over frames 1 to 11 (linear), pushed down into the action Rise.
fn rising() -> SceneDocument {
    let mut doc = SceneDocument::create("A.kscene", 1280, 720).unwrap();
    run(&mut doc, "frame.set", json!({ "frame": 1 }));
    run(&mut doc, "anim.insertKey", json!({ "objects": "Cube", "properties": ["location"] }));
    run(&mut doc, "frame.set", json!({ "frame": 11 }));
    run(&mut doc, "object.transform", json!({ "objects": "Cube", "location": [0, 0, 2] }));
    run(&mut doc, "anim.insertKey", json!({ "objects": "Cube", "properties": ["location"] }));
    run(&mut doc, "anim.setInterpolation", json!({ "objects": "Cube", "interpolation": "linear" }));
    let pushed = run(&mut doc, "nla.pushDown", json!({ "object": "Cube", "name": "Rise" }));
    assert_eq!(pushed["action"], "Rise");
    doc
}

#[test]
fn strips_play_actions_and_survive_saving() {
    let mut doc = rising();
    let inspected = doc.inspect().unwrap();
    let cube = object(&inspected, "Cube");
    assert_eq!(cube["nla"]["tracks"][0]["strips"][0]["action"], "Rise");
    assert_eq!(cube["nla"]["tracks"][0]["strips"][0]["end"], 11.0);
    assert!(inspected["animation"]["channels"].as_array().unwrap().is_empty(), "push down takes the cube's own keys");
    assert!((at(&mut doc, 6.0)[2] - 1.0).abs() < 1e-4);

    // Twice as slow, twice over, from frame 21.
    run(&mut doc, "nla.setStrip", json!({ "object": "Cube", "track": 0, "strip": 0, "start": 21, "scale": 2, "repeat": 2 }));
    assert_eq!(object(&doc.inspect().unwrap(), "Cube")["nla"]["tracks"][0]["strips"][0]["end"], 61.0);
    assert!((at(&mut doc, 31.0)[2] - 1.0).abs() < 1e-4);
    assert!((at(&mut doc, 41.0)[2]).abs() < 1e-4, "the second repeat starts over");
    assert!((at(&mut doc, 61.0)[2] - 2.0).abs() < 1e-4);

    // A second strip adding a lift on a new track, at half influence.
    run(&mut doc, "add.empty", json!({ "name": "Other" }));
    run(&mut doc, "frame.set", json!({ "frame": 1 }));
    run(&mut doc, "object.transform", json!({ "objects": "Cube", "location": [3, 0, 0] }));
    run(&mut doc, "anim.insertKey", json!({ "objects": "Cube", "properties": ["location"] }));
    run(&mut doc, "nla.pushDown", json!({ "object": "Cube", "name": "Shift" }));
    run(&mut doc, "nla.setStrip", json!({ "object": "Cube", "track": 1, "strip": 0, "blend": "add", "influence": 0.5 }));
    let shifted = at(&mut doc, 31.0);
    assert!((shifted[0] - 1.5).abs() < 1e-4 && (shifted[2] - 1.0).abs() < 1e-4, "{shifted:?}");
    run(&mut doc, "nla.setTrack", json!({ "object": "Cube", "track": 1, "muted": true }));
    assert!((at(&mut doc, 31.0)[0]).abs() < 1e-4);
    run(&mut doc, "edit.undo", json!({}));
    assert!((at(&mut doc, 31.0)[0] - 1.5).abs() < 1e-4, "muting is one undo step");

    let mut back = reopen(&doc);
    assert_eq!(object(&back.inspect().unwrap(), "Cube")["nla"], object(&doc.inspect().unwrap(), "Cube")["nla"]);
    assert!((at(&mut back, 31.0)[0] - 1.5).abs() < 1e-4);
}

#[test]
fn bad_strip_settings_change_nothing() {
    let mut doc = rising();
    let before = doc.inspect().unwrap();
    for params in [
        json!({ "object": "Cube", "track": 0, "strip": 0, "scale": 0 }),
        json!({ "object": "Cube", "track": 0, "strip": 0, "repeat": -1 }),
        json!({ "object": "Cube", "track": 0, "strip": 0, "actionEnd": 0 }),
        json!({ "object": "Cube", "track": 0, "strip": 0, "influence": 2 }),
        json!({ "object": "Cube", "track": 0, "strip": 0, "start": 1e30 }),
        json!({ "object": "Cube", "track": 0, "strip": 0, "extrapolation": "sideways" }),
        json!({ "object": "Cube", "track": 4, "strip": 0 }),
    ] {
        let error = doc.execute("nla.setStrip", params.clone()).unwrap_err();
        assert!(error.starts_with("invalid-params") || error.starts_with("not-found"), "{params}: {error}");
    }
    let error = doc.execute("nla.addStrip", json!({ "object": "Cube", "action": "Rise", "track": 0, "start": 5 })).unwrap_err();
    assert!(error.starts_with("invalid-params"), "an overlapping strip: {error}");
    assert_eq!(doc.inspect().unwrap()["objects"], before["objects"]);
    // Without a track it goes on a new one.
    let placed = run(&mut doc, "nla.addStrip", json!({ "object": "Cube", "action": "Rise", "start": 5 }));
    assert_eq!(placed["track"], 1);
}

#[test]
fn tweak_mode_edits_the_action_through_the_strip() {
    let mut doc = rising();
    run(&mut doc, "nla.setStrip", json!({ "object": "Cube", "track": 0, "strip": 0, "start": 21, "scale": 2 }));
    run(&mut doc, "nla.tweak", json!({ "object": "Cube", "track": 0, "strip": 0 }));
    let inspected = doc.inspect().unwrap();
    assert_eq!(object(&inspected, "Cube")["nla"]["tweak"]["track"], 0);
    assert_eq!(inspected["animation"]["channels"].as_array().unwrap().len(), 3, "the action's keys are the cube's own");
    // A key at scene frame 31 lands on action frame 6.
    run(&mut doc, "frame.set", json!({ "frame": 31 }));
    run(&mut doc, "object.transform", json!({ "objects": "Cube", "location": [0, 0, 5] }));
    run(&mut doc, "anim.insertKey", json!({ "objects": "Cube", "properties": ["location"] }));
    let frames: Vec<f64> =
        doc.inspect().unwrap()["animation"]["channels"][2]["keys"].as_array().unwrap().iter().map(|key| number(&key["frame"])).collect();
    assert_eq!(frames, vec![1.0, 6.0, 11.0]);
    assert!(doc.inspect().unwrap()["animation"]["selectedKeyFrames"].as_array().is_some());
    assert!((at(&mut doc, 31.0)[2] - 5.0).abs() < 1e-4);
    assert!(doc.execute("nla.setStrip", json!({ "object": "Cube", "track": 0, "strip": 0, "scale": 1 })).unwrap_err().starts_with("not-allowed"));
    run(&mut doc, "nla.tweakExit", json!({ "object": "Cube" }));
    let inspected = doc.inspect().unwrap();
    assert!(inspected["animation"]["channels"].as_array().unwrap().is_empty());
    assert_eq!(inspected["animation"]["actions"][0]["name"], "Rise");
    assert!((at(&mut doc, 31.0)[2] - 5.0).abs() < 1e-4, "the action keeps the key");
    // Removing the action takes its strip.
    run(&mut doc, "action.remove", json!({ "name": "Rise" }));
    assert!(object(&doc.inspect().unwrap(), "Cube")["nla"]["tracks"][0]["strips"].as_array().unwrap().is_empty());
}

#[test]
fn modifiers_reshape_a_channel_and_survive_saving() {
    let mut doc = SceneDocument::create("A.kscene", 1280, 720).unwrap();
    run(&mut doc, "frame.set", json!({ "frame": 1 }));
    run(&mut doc, "anim.insertKey", json!({ "objects": "Cube", "properties": ["location"] }));
    run(&mut doc, "frame.set", json!({ "frame": 11 }));
    run(&mut doc, "object.transform", json!({ "objects": "Cube", "location": [2, 0, 0] }));
    run(&mut doc, "anim.insertKey", json!({ "objects": "Cube", "properties": ["location"] }));
    run(&mut doc, "anim.setInterpolation", json!({ "objects": "Cube", "interpolation": "linear" }));
    let cube = object(&doc.inspect().unwrap(), "Cube")["id"].as_u64().unwrap();
    let channel = json!({ "target": cube, "property": "location", "index": 0 });
    let with = |extra: Value| {
        let mut params = channel.clone();
        params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        params
    };
    run(&mut doc, "graph.addModifier", with(json!({ "type": "cycles", "after": "repeatOffset" })));
    assert!((at(&mut doc, 16.0)[0] - 3.0).abs() < 1e-4, "the next cycle starts at 2");
    run(
        &mut doc,
        "graph.addModifier",
        with(json!({ "type": "generator", "coefficients": [1.0], "additive": true, "range": { "start": 0, "end": 5 } })),
    );
    assert!((at(&mut doc, 3.0)[0] - 1.4).abs() < 1e-4);
    assert!((at(&mut doc, 6.0)[0] - 1.0).abs() < 1e-4, "outside its range");
    run(&mut doc, "graph.setModifier", with(json!({ "modifier": 1, "influence": 0.5 })));
    assert!((at(&mut doc, 3.0)[0] - 0.9).abs() < 1e-4);
    let inspected = doc.inspect().unwrap();
    let listed = &inspected["animation"]["channels"][0];
    assert_eq!(listed["modifiers"][0]["type"], "cycles");
    assert_eq!(listed["modifiers"][1]["influence"], 0.5);
    assert!(listed["samples"].as_array().unwrap().len() > 100);
    let error = doc.execute("graph.setModifier", with(json!({ "modifier": 1, "coefficients": [] }))).unwrap_err();
    assert!(error.starts_with("invalid-params"), "{error}");
    let mut back = reopen(&doc);
    assert_eq!(back.inspect().unwrap()["animation"]["channels"][0]["modifiers"], listed["modifiers"]);
    assert!((at(&mut back, 16.0)[0] - 3.0).abs() < 1e-4);
    run(&mut doc, "graph.removeModifier", with(json!({ "modifier": 0 })));
    assert!((at(&mut doc, 16.0)[0] - 2.0).abs() < 1e-4, "without cycles the curve holds");
    run(&mut doc, "edit.undo", json!({}));
    assert!((at(&mut doc, 16.0)[0] - 3.0).abs() < 1e-4);
}

#[test]
fn a_released_0_11_0_file_takes_tracks_and_modifiers() {
    let mut doc = SceneDocument::open("Room.kscene", include_bytes!("fixtures/released-0.11.0.kscene")).unwrap();
    run(&mut doc, "nla.addStrip", json!({ "object": "Ball", "action": "Rise", "start": 31 }));
    run(&mut doc, "nla.setStrip", json!({ "object": "Ball", "track": 0, "strip": 0, "extrapolation": "nothing", "blendIn": 5 }));
    run(&mut doc, "nla.pushDown", json!({ "object": "Cube" }));
    let cube = object(&doc.inspect().unwrap(), "Cube")["id"].as_u64().unwrap();
    run(&mut doc, "nla.tweak", json!({ "object": "Cube", "track": 0, "strip": 0 }));
    run(&mut doc, "graph.addModifier", json!({ "target": cube, "property": "location", "index": 2, "type": "noise", "strength": 0.5 }));
    run(&mut doc, "nla.tweakExit", json!({ "object": "Cube" }));
    let mut back = reopen(&doc);
    let inspected = back.inspect().unwrap();
    assert_eq!(object(&inspected, "Ball")["nla"]["tracks"][0]["strips"][0]["blendIn"], 5.0);
    assert_eq!(object(&inspected, "Cube")["nla"]["tracks"][0]["strips"][0]["action"], "CubeAction");
    assert_eq!(inspected["animation"]["actions"].as_array().unwrap().len(), 2);
    run(&mut back, "frame.set", json!({ "frame": 36 }));
    let inspected = back.inspect().unwrap();
    let ball = &object(&inspected, "Ball")["location"];
    assert!(number(&ball[2]) > 0.0 && number(&ball[2]) < 2.0, "the ball rises, blending in: {ball}");
}
