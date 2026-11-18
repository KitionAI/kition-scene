//! Weight-transfer identities recorded in the public app isolate weighting from solver fidelity.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use glam::Vec3;
use kition_scene_model::physics::{Cloth, EffectorWeights, FieldKind, ForceField, ObjectPhysics, SoftBody};
use kition_scene_model::{Mesh, ObjectData, Scene};
use serde_json::Value;

fn reference() -> Value {
    serde_json::from_str(include_str!("fixtures/weights10-reference.json")).unwrap()
}
fn vector(value: &Value) -> Vec3 {
    Vec3::new(value[0].as_f64().unwrap() as f32, value[1].as_f64().unwrap() as f32, value[2].as_f64().unwrap() as f32)
}
fn recorded(case: &Value) -> Vec<Vec3> {
    case["samples"].as_array().unwrap().iter().map(|sample| vector(&sample["mean"])).collect()
}
fn simulated(case: &Value) -> Vec<Vec3> {
    let mut scene = Scene::empty();
    scene.physics.frame_end = 12;
    let name = case["kind"].as_str().unwrap();
    scene.physics.gravity = if name == "gravity" || name == "drag" { Vec3::new(0.0, 0.0, -9.81) } else { Vec3::ZERO };
    let mesh = Mesh {
        positions: vec![Vec3::new(-0.5, -0.5, 0.0), Vec3::new(0.5, -0.5, 0.0), Vec3::new(0.5, 0.5, 0.0), Vec3::new(-0.5, 0.5, 0.0)],
        faces: vec![vec![0, 1, 2, 3]],
        ..Default::default()
    };
    let id = scene.add_object("Probe", ObjectData::Mesh(mesh));
    let weights: EffectorWeights = serde_json::from_value(case["weights"].clone()).unwrap();
    let physics = if case["system"] == "cloth" {
        ObjectPhysics { cloth: Some(Cloth { air_damping: 0.0, effector_weights: weights, ..Default::default() }), ..Default::default() }
    } else {
        ObjectPhysics {
            soft_body: Some(SoftBody { goal: false, edges: false, friction: 0.0, effector_weights: weights, ..Default::default() }),
            ..Default::default()
        }
    };
    scene.object_mut(id).unwrap().physics = Some(physics);
    if name != "gravity" {
        let kind: FieldKind = serde_json::from_value(case["kind"].clone()).unwrap();
        let mut field = ForceField { strength: 2.0, flow: 0.0, ..ForceField::new(kind) };
        if kind == FieldKind::Drag {
            field.linear_drag = 1.0;
            field.quadratic_drag = 0.0;
        }
        if let Some(overrides) = case["overrides"].as_object() {
            for (key, value) in overrides {
                let slot = match key.as_str() {
                    "strength" => &mut field.strength,
                    "flow" => &mut field.flow,
                    "linear_drag" => &mut field.linear_drag,
                    "quadratic_drag" => &mut field.quadratic_drag,
                    _ => panic!("unknown probe setting"),
                };
                *slot = value.as_f64().unwrap() as f32;
            }
        }
        let owner = scene.add_object("Field", ObjectData::Empty { size: 1.0 });
        let object = scene.object_mut(owner).unwrap();
        object.location = Vec3::new(3.0, 0.0, -2.0);
        object.physics = Some(ObjectPhysics { field: Some(field), ..Default::default() });
    }
    let baked = kition_scene_physics::bake(&scene).unwrap();
    assert_eq!(baked.cloths[&id].len(), 12);
    for frame in &baked.cloths[&id] {
        assert_eq!(frame.len(), 4);
        assert!(frame.iter().all(|point| point.is_finite()));
    }
    case["samples"]
        .as_array()
        .unwrap()
        .iter()
        .map(|sample| {
            let frame = sample["frame"].as_u64().unwrap() as usize - 1;
            baked.cloths[&id][frame].iter().copied().sum::<Vec3>() / 4.0
        })
        .collect()
}
fn close(a: &[Vec3], b: &[Vec3], scale: f32) {
    assert_eq!(a.len(), b.len());
    for (a, b) in a.iter().zip(b) {
        assert!(a.distance(*b * scale) <= 2e-5, "{a} versus {b} times {scale}");
    }
}
#[test]
fn recorded_weight_transfers_hold_for_both_baked_solvers() {
    let data = reference();
    for system in ["cloth", "soft"] {
        for key in ["all", "gravity", "force", "wind", "vortex", "turbulence", "drag"] {
            assert_eq!(data["defaults"][system][key]["default"].as_f64(), Some(1.0));
            assert_eq!(data["defaults"][system][key]["min"].as_f64(), Some(-200.0));
            assert_eq!(data["defaults"][system][key]["max"].as_f64(), Some(200.0));
        }
        for kind in ["gravity", "force", "wind", "vortex", "turbulence", "drag"] {
            let find = |list: &str, kind: &str, label: &str| {
                data[list].as_array().unwrap().iter().find(|case| case["system"] == system && case["kind"] == kind && case["label"] == label).unwrap()
            };
            if kind != "gravity" && kind != "drag" {
                let weighted = recorded(find("controls", kind, "weight"));
                let strength_only = recorded(find("controls", kind, "strength"));
                assert!(
                    weighted.last().unwrap().distance(*strength_only.last().unwrap()) > 2e-5,
                    "the public control distinguishes scaling flow from strength alone: {system} {kind}"
                );
            }
            for evaluate in [recorded as fn(&Value) -> Vec<Vec3>, simulated] {
                let get = |label| evaluate(find("cases", kind, label));
                let baseline = get("default");
                assert!(baseline.last().unwrap().length() > 1e-5, "{system} {kind} must exercise motion");
                if kind == "gravity" {
                    close(&get("all_zero"), &baseline, 1.0);
                    close(&get("all_half"), &baseline, 1.0);
                    close(&get("kind_half"), &baseline, 0.5);
                    close(&get("quarter"), &baseline, 0.5);
                    close(&get("negative"), &baseline, -1.0);
                    close(&get("double_negative"), &baseline, -1.0);
                    close(&get("kind_zero"), &baseline, 0.0);
                } else {
                    close(&get("all_half"), &get("kind_half"), 1.0);
                    close(&get("all_zero"), &get("kind_zero"), 1.0);
                    close(&get("negative"), &get("kind_zero"), 1.0);
                    close(&get("double_negative"), &baseline, 1.0);
                    close(&get("product"), &get("product_control"), 1.0);
                    if kind == "drag" {
                        close(&get("kind_zero"), &evaluate(find("cases", "gravity", "default")), 1.0);
                    } else {
                        close(&get("kind_zero"), &baseline, 0.0);
                    }
                    let weighted = evaluate(find("controls", kind, "weight"));
                    let control = evaluate(find("controls", kind, if kind == "drag" { "drag" } else { "strength_flow" }));
                    close(&weighted, &control, 1.0);
                }
            }
        }
    }
}

#[test]
fn optional_public_application_rechecks_the_immutable_recording() {
    let Some(blender) = std::env::var_os("KITION_BLENDER") else { return };
    let output = std::env::temp_dir().join(format!("scene-weights10-{}.json", std::process::id()));
    let status = std::process::Command::new(blender)
        .args(["-b", "--factory-startup", "--python"])
        .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/blender/weights10.py"))
        .arg("--")
        .arg(&output)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(serde_json::from_slice::<Value>(&std::fs::read(&output).unwrap()).unwrap(), reference());
    std::fs::remove_file(output).unwrap();
}
