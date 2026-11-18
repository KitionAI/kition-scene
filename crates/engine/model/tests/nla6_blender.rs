//! Offline clean-room samples from Blender 5.2.3. Set KITION_BLENDER to check the recording live.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use kition_scene_model::animation::{Action, ActionChannel, Property};
use kition_scene_model::fmodifier::FModifier;
use kition_scene_model::{Channel, Interpolation, Keyframe, ObjectData, Scene};
use serde_json::{Value, json};

fn channel(spec: &Value) -> ActionChannel {
    ActionChannel {
        property: Property::parse(spec["property"].as_str().unwrap()).unwrap(),
        index: spec["index"].as_u64().unwrap() as u8,
        path: String::new(),
        keys: spec["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|key| Keyframe {
                frame: key[0].as_f64().unwrap() as f32,
                value: key[1].as_f64().unwrap() as f32,
                interpolation: Interpolation::Linear,
            })
            .collect(),
        modifiers: Vec::new(),
    }
}
fn settings(strip: &mut kition_scene_model::nla::NlaStrip, spec: &Value) {
    let mut value = serde_json::to_value(&strip).unwrap();
    for field in ["scale", "repeat", "reversed", "blend", "influence", "blendIn", "blendOut", "muted", "extrapolation"] {
        if let Some(v) = spec.get(field) {
            value[field] = v.clone();
        }
    }
    *strip = serde_json::from_value(value).unwrap();
}
fn measured(case: &Value) -> Value {
    let mut scene = Scene::empty();
    let id = scene.add_object("E", ObjectData::Empty { size: 1.0 });
    for (name, channels) in case["actions"].as_object().unwrap() {
        scene.animation.actions.push(Action { name: name.clone(), channels: channels.as_array().unwrap().iter().map(channel).collect() });
    }
    let frames = case["frames"].as_array().unwrap();
    if let Some(modifiers) = case.get("modifiers") {
        let ch = scene.animation.action("A").unwrap().channels[0].clone();
        let modifiers: Vec<FModifier> = serde_json::from_value(modifiers.clone()).unwrap();
        for modifier in &modifiers {
            modifier.check().unwrap();
        }
        let ch = Channel { target: id, property: ch.property, index: ch.index, path: ch.path, keys: ch.keys, driver: None, modifiers };
        return json!(frames.iter().map(|f| ch.evaluate(f.as_f64().unwrap() as f32).unwrap()).collect::<Vec<_>>());
    }
    for (track, row) in case["tracks"].as_array().unwrap().iter().enumerate() {
        scene.nla_add_track(id, None, None).unwrap();
        for spec in row.as_array().unwrap() {
            let (_, slot) = scene.nla_add_strip(id, Some(track), spec["action"].as_str().unwrap(), spec["start"].as_f64().unwrap() as f32).unwrap();
            settings(&mut scene.object_mut(id).unwrap().nla.tracks[track].strips[slot], spec);
        }
    }
    for op in case["operations"].as_array().unwrap() {
        let track = op["track"].as_u64().unwrap() as usize;
        let selected = op["strips"].as_array();
        match op["type"].as_str().unwrap() {
            "transition" => {
                scene.nla_add_transition(id, track, selected.unwrap()[0].as_u64().unwrap() as usize).unwrap();
            }
            "meta" => {
                let selected = selected.unwrap();
                scene.nla_make_meta(id, track, selected[0].as_u64().unwrap() as usize, selected.last().unwrap().as_u64().unwrap() as usize).unwrap();
            }
            "dissolve" => {
                scene.nla_dissolve_meta(id, track, selected.unwrap()[0].as_u64().unwrap() as usize).unwrap();
            }
            "set" => {
                let strip = &mut scene.object_mut(id).unwrap().nla.tracks[track].strips[op["strip"].as_u64().unwrap() as usize];
                if let Some(start) = op["start"].as_f64() {
                    strip.resize_meta(start as f32, op["end"].as_f64().unwrap() as f32).unwrap();
                }
                settings(strip, op);
            }
            other => panic!("{other}"),
        }
    }
    scene.object_mut(id).unwrap().nla.settle();
    scene.validate_animation().unwrap();
    json!(
        frames
            .iter()
            .map(|frame| {
                let scene = scene.at_frame(frame.as_f64().unwrap() as f32);
                let ob = scene.object(id).unwrap();
                [ob.location.x, ob.location.y, ob.location.z, ob.scale.x, ob.scale.y, ob.scale.z]
            })
            .collect::<Vec<_>>()
    )
}
fn compare(actual: &Value, expected: &Value, at: &str) {
    match (actual, expected) {
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "{at}");
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                compare(a, b, &format!("{at}/{i}"));
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(a.len(), b.len(), "{at}");
            for (key, value) in a {
                compare(value, &b[key], &format!("{at}/{key}"));
            }
        }
        (Value::Number(a), Value::Number(b)) => {
            let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
            assert!((a - b).abs() < 1e-4, "{at}: Cube {a}, Blender {b}");
        }
        _ => assert_eq!(actual, expected, "{at}"),
    }
}
#[test]
fn transitions_meta_and_limits_match_blender() {
    let cases: Value = serde_json::from_str(include_str!("fixtures/nla6-cases.json")).unwrap();
    let reference: Value = serde_json::from_str(include_str!("fixtures/nla6-reference.json")).unwrap();
    if let Some(blender) = std::env::var_os("KITION_BLENDER") {
        let output = std::env::temp_dir().join(format!("scene-nla6-{}.json", std::process::id()));
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let status = std::process::Command::new(blender)
            .args(["-b", "--factory-startup", "--python"])
            .arg(root.join("tests/blender/nla6.py"))
            .arg("--")
            .arg(root.join("tests/fixtures/nla6-cases.json"))
            .arg(&output)
            .status()
            .unwrap();
        assert!(status.success());
        let live: Value = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        compare(&live, &reference, "recording");
        std::fs::remove_file(output).unwrap();
    }
    for case in cases["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        compare(&measured(case), &reference["cases"][name], name);
    }
}
