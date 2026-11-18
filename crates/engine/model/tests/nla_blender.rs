//! The NLA (strips, their timing, influence, extrapolation, blend types, mute and solo, the
//! active action on top, auto blends, tweak mode) and F-curve modifiers against the reference
//! application. `tests/fixtures/nla-cases.json` lists the cases; `tests/blender/nla.py` builds
//! each in Blender 5.2.3 and its results are recorded in `tests/fixtures/nla-reference.json`,
//! which this test always compares with. When `KITION_BLENDER` names a Blender executable the
//! script runs again and the recording must still match it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_scene_model::animation::{Action, ActionChannel, Interpolation, Keyframe, Property, read_at, write_at};
use kition_scene_model::fmodifier::FModifier;
use kition_scene_model::glam::Vec3;
use kition_scene_model::nla::{ActionBlend, Nla, NlaStrip, NlaTrack};
use kition_scene_model::{Armature, Bone, Channel, Id, ObjectData, Scene};
use serde_json::{Value, json};

/// Every value within this of the reference's.
const TOLERANCE: f64 = 1e-4;

fn cases() -> Value {
    serde_json::from_str(include_str!("fixtures/nla-cases.json")).unwrap()
}

/// The recording, checked once against a live run when Blender is at hand.
fn reference() -> Value {
    let recorded: Value = serde_json::from_str(include_str!("fixtures/nla-reference.json")).unwrap();
    if let Some(blender) = std::env::var_os("KITION_BLENDER") {
        let dir = std::env::temp_dir().join(format!("kition-scene-nla-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let output = dir.join("out.json");
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let status = std::process::Command::new(blender)
            .args(["-b", "--factory-startup", "--python"])
            .arg(root.join("tests/blender/nla.py"))
            .arg("--")
            .arg(root.join("tests/fixtures/nla-cases.json"))
            .arg(&output)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "Blender failed");
        let live: Value = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        same_numbers(&recorded, &live, "", 1e-5);
        let _ = std::fs::remove_dir_all(&dir);
    }
    recorded
}

fn same_numbers(a: &Value, b: &Value, at: &str, tolerance: f64) {
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(a.len(), b.len(), "{at}: keys differ");
            for (key, value) in a {
                same_numbers(value, &b[key], &format!("{at}/{key}"), tolerance);
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "{at}: lengths differ");
            for (index, (x, y)) in a.iter().zip(b).enumerate() {
                same_numbers(x, y, &format!("{at}/{index}"), tolerance);
            }
        }
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            assert!((x - y).abs() <= tolerance, "{at}: recorded {x}, Blender now {y}");
        }
        (x, y) => assert_eq!(x, y, "{at}"),
    }
}

/// A case's path (`location.0`, `bone:Bone:rotation_euler.2`, `hide_viewport`) as a channel.
fn path(text: &str) -> (Property, u8, String) {
    let (bone, rest) = match text.strip_prefix("bone:") {
        Some(rest) => {
            let (bone, rest) = rest.split_once(':').unwrap();
            (Some(bone.to_string()), rest)
        }
        None => (None, text),
    };
    let (name, index) = match rest.rsplit_once('.') {
        Some((name, index)) => (name, index.parse::<u8>().unwrap()),
        None => (rest, 0),
    };
    let property = match (name, bone.is_some()) {
        ("location", false) => Property::Location,
        ("rotation_euler", false) => Property::Rotation,
        ("scale", false) => Property::Scale,
        ("hide_viewport", false) => Property::Hidden,
        ("location", true) => Property::BoneLocation,
        ("rotation_euler", true) => Property::BoneRotation,
        ("scale", true) => Property::BoneScale,
        other => panic!("no property {other:?}"),
    };
    (property, index, bone.unwrap_or_default())
}

fn channels(spec: &Value) -> Vec<ActionChannel> {
    spec.as_object()
        .unwrap()
        .iter()
        .map(|(name, channel)| {
            let (property, index, path) = path(name);
            let interpolation = match channel["interpolation"].as_str().unwrap_or("bezier") {
                "linear" => Interpolation::Linear,
                "constant" => Interpolation::Constant,
                _ => Interpolation::Bezier,
            };
            let keys = channel["keys"]
                .as_array()
                .unwrap()
                .iter()
                .map(|key| Keyframe { frame: key[0].as_f64().unwrap() as f32, value: key[1].as_f64().unwrap() as f32, interpolation })
                .collect();
            let modifiers: Vec<FModifier> = serde_json::from_value(channel.get("modifiers").cloned().unwrap_or(json!([]))).unwrap();
            ActionChannel { property, index, path, keys, modifiers }
        })
        .collect()
}

fn strip(spec: &Value, actions: &[Action]) -> NlaStrip {
    let action = actions.iter().find(|action| action.name == spec["action"]).unwrap();
    let mut merged = serde_json::to_value(NlaStrip::new(action.name.clone(), action, spec["start"].as_f64().unwrap() as f32)).unwrap();
    for (key, value) in spec.as_object().unwrap() {
        merged[key.as_str()] = value.clone();
    }
    serde_json::from_value(merged).unwrap()
}

/// The case's scene: one empty (or armature with a bone `Bone`) carrying the case's NLA.
fn build(case: &Value) -> (Scene, Id) {
    let mut scene = Scene::empty();
    let id = if case["armature"] == true {
        scene.add_object("Rig", ObjectData::Armature(Armature { bones: vec![Bone::new("Bone", Vec3::ZERO, Vec3::Z)] }))
    } else {
        scene.add_object("E", ObjectData::Empty { size: 1.0 })
    };
    if let Some(actions) = case["actions"].as_object() {
        for (name, spec) in actions {
            scene.animation.actions.push(Action { name: name.clone(), channels: channels(spec) });
        }
    }
    if !case["own"].is_null() {
        for channel in channels(&case["own"]) {
            scene.animation.channels.push(Channel {
                target: id,
                property: channel.property,
                index: channel.index,
                path: channel.path,
                keys: channel.keys,
                driver: None,
                modifiers: channel.modifiers,
            });
        }
    }
    let mut nla = Nla::default();
    for spec in case["tracks"].as_array().into_iter().flatten() {
        nla.tracks.push(NlaTrack {
            name: "NlaTrack".into(),
            muted: spec["muted"] == true,
            solo: spec["solo"] == true,
            strips: spec["strips"].as_array().unwrap().iter().map(|strip_spec| strip(strip_spec, &scene.animation.actions)).collect(),
        });
    }
    if !case["ownBlend"].is_null() {
        nla.action = serde_json::from_value::<ActionBlend>(case["ownBlend"].clone()).unwrap();
    }
    nla.settle();
    scene.object_mut(id).unwrap().nla = nla;
    scene.validate_animation().unwrap();
    if !case["tweak"].is_null() {
        let tweak = &case["tweak"];
        scene
            .nla_tweak(id, tweak["track"].as_u64().unwrap() as usize, tweak["strip"].as_u64().unwrap() as usize, tweak["upperStack"] == true)
            .unwrap();
        scene.validate_animation().unwrap();
    }
    for (name, value) in case["static"].as_object().into_iter().flatten() {
        let (property, index, path) = path(name);
        assert!(write_at(&mut scene, id, property, index, &path, value.as_f64().unwrap() as f32));
    }
    (scene, id)
}

fn sample(case: &Value) -> (Scene, Vec<Vec<f64>>) {
    let (mut scene, id) = build(case);
    let paths: Vec<(Property, u8, String)> = case["paths"].as_array().unwrap().iter().map(|text| path(text.as_str().unwrap())).collect();
    let values = case["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|frame| {
            scene.go_to_frame(frame.as_f64().unwrap() as f32);
            paths.iter().map(|(property, index, path)| f64::from(read_at(&scene, id, *property, *index, path).unwrap())).collect()
        })
        .collect();
    (scene, values)
}

fn mean_and_deviation(values: &[f64]) -> (f64, f64) {
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values.iter().map(|value| (value - mean).powi(2)).sum::<f64>() / values.len() as f64;
    (mean, variance.sqrt())
}

#[test]
fn nla_and_modifiers_match_the_reference() {
    let reference = reference();
    let cases = cases();
    let mut failures = Vec::new();
    let mut checked = 0;
    for case in cases["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let expected = &reference[name];
        let (scene, got) = sample(case);
        if case["statistics"] == true {
            // Noise: Kition's own noise, compared by its spread (the reference's noise function
            // is not reproduced sample by sample).
            let column = |rows: &[Vec<f64>]| rows.iter().map(|row| row[0]).collect::<Vec<_>>();
            let want: Vec<Vec<f64>> = serde_json::from_value(expected["values"].clone()).unwrap();
            let ((mine_mean, mine_spread), (their_mean, their_spread)) = (mean_and_deviation(&column(&got)), mean_and_deviation(&column(&want)));
            println!("{name}: mean {mine_mean:.4} (reference {their_mean:.4}), deviation {mine_spread:.4} (reference {their_spread:.4})");
            assert!(mine_mean.abs() < 0.1 * case_strength(case) && their_mean.abs() < 0.1 * case_strength(case), "{name}: noise is centred");
            assert!((mine_spread / their_spread - 1.0).abs() < 0.15, "{name}: spread {mine_spread} against {their_spread}");
            continue;
        }
        let mut worst = 0.0f64;
        for (row, (frame, want)) in case["frames"].as_array().unwrap().iter().zip(expected["values"].as_array().unwrap()).enumerate() {
            for (column, want) in want.as_array().unwrap().iter().enumerate() {
                let (want, have) = (want.as_f64().unwrap(), got[row][column]);
                let error = (want - have).abs();
                worst = worst.max(error);
                checked += 1;
                if error > TOLERANCE {
                    failures.push(format!("{name} at frame {frame} {}: {have} against {want}", case["paths"][column]));
                }
            }
        }
        if case["blends"] == true {
            let object = &scene.objects[0];
            for (slot, (track, want)) in object.nla.tracks.iter().zip(expected["blends"].as_array().unwrap()).enumerate() {
                for (strip, want) in track.strips.iter().zip(want.as_array().unwrap()) {
                    let (blend_in, blend_out) = (want[0].as_f64().unwrap(), want[1].as_f64().unwrap());
                    if (f64::from(strip.blend_in) - blend_in).abs() > TOLERANCE || (f64::from(strip.blend_out) - blend_out).abs() > TOLERANCE {
                        failures.push(format!("{name} track {slot}: blends {} {} against {blend_in} {blend_out}", strip.blend_in, strip.blend_out));
                    }
                }
            }
        }
        println!("{name}: largest difference {worst:.2e}");
    }
    assert!(failures.is_empty(), "{} of {checked} values differ:\n{}", failures.len(), failures.join("\n"));
}

fn case_strength(case: &Value) -> f64 {
    case["own"]["location.0"]["modifiers"][0]["strength"].as_f64().unwrap_or(1.0)
}
