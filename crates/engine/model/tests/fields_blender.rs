//! Force fields on particles and density groups against the reference application: one particle
//! released at rest (no gravity) next to each kind of field, its position at frames 10 and 30;
//! 400 particles released into turbulence, their mean speed (the noise differs, so only its
//! strength is compared); and 1000 particles on a grid weighted by a density group, where they
//! start. The numbers Blender 5.2.3 gave are checked always; when `KITION_BLENDER` names a
//! Blender executable, `tests/blender/fields.py` makes them again there.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::f32::consts::FRAC_PI_2;

use kition_scene_model::glam::Vec3;
use kition_scene_model::particles::{self, Cache, EmitFrom, Kind, ParticleSystem};
use kition_scene_model::physics::{FieldKind, ForceField, ObjectPhysics};
use kition_scene_model::vertex_data::VertexGroup;
use kition_scene_model::{Id, ObjectData, Primitive, Scene};
use serde_json::{Value, json};

/// What Blender 5.2.3 gave (metres, metres per second).
fn recorded() -> Value {
    json!({"force": {"10": [-0.0644, 0.0004, 0.0], "30": [-0.6724, 0.0006, 0.0]},
        "forceFalloff": {"10": [-0.0068, 0.0004, 0.0], "30": [-0.0738, 0.0004, 0.0]},
        "wind": {"10": [0.0004, 0.0004, 0.0577], "30": [0.0004, 0.0004, 0.4736]},
        "windStill": {"10": [0.3244, 0.0004, 0.0], "30": [3.3644, 0.0004, 0.0]},
        "vortex": {"10": [0.0581, -0.0562, 0.0], "30": [0.4697, -0.3802, 0.0]},
        "vortexStrong": {"10": [0.2243, -0.106, 0.0], "30": [1.3487, -0.3346, 0.0]},
        "drag": {"10": [0.0004, 0.0004, 0.4725], "30": [0.0004, 0.0004, 0.8647]},
        "dragSquare": {"10": [0.0004, 0.0004, 0.8917], "30": [0.0004, 0.0004, 1.7327]},
        "turbulence": {"10": 0.0892, "30": 0.2834},
        "turbulenceStrong": {"10": 0.4467, "30": 1.1595},
        "turbulenceLarge": {"10": 0.5523, "30": 1.6169},
        "densityRamp": {"alive": 1000, "meanX": 0.3348, "negative": 233},
        "densityStep": {"alive": 1000, "meanX": 0.5011, "negative": 43},
        "densityVertices": {"alive": 1000, "meanX": 0.4338, "negative": 194}})
}

fn emitter(scene: &mut Scene, size: f32, count: u32, normal: f32) -> Id {
    let plane = scene.add_object("Plane", ObjectData::Mesh(Primitive::Plane { size }.build().unwrap()));
    let mut system = ParticleSystem::new("P", Kind::Emitter);
    system.count = count;
    system.frame_end = 1.0;
    system.lifetime = 200.0;
    system.normal_velocity = normal;
    system.gravity = 0.0;
    scene.object_mut(plane).unwrap().particles.push(system);
    plane
}

fn field(scene: &mut Scene, made: ForceField, location: Vec3, rotation: Vec3) {
    let id = scene.add_object("Field", ObjectData::Empty { size: 1.0 });
    let object = scene.object_mut(id).unwrap();
    object.location = location;
    object.rotation = rotation;
    object.physics = Some(ObjectPhysics { field: Some(made), ..ObjectPhysics::default() });
}

/// The live particles of `plane` at each of `frames`.
fn alive(scene: &mut Scene, plane: Id, frames: &[i32]) -> Vec<(i32, Vec<particles::Particle>)> {
    let mut cache = Cache::new();
    frames
        .iter()
        .map(|frame| {
            scene.go_to_frame(*frame as f32);
            (*frame, particles::particles(scene, plane, 0, &mut cache).to_vec())
        })
        .collect()
}

fn single(kind: FieldKind, location: Vec3, rotation: Vec3, normal: f32, settings: impl FnOnce(&mut ForceField)) -> Value {
    let mut scene = Scene::empty();
    let plane = emitter(&mut scene, 0.001, 1, normal);
    let mut made = ForceField::new(kind);
    settings(&mut made);
    field(&mut scene, made, location, rotation);
    let mut out = serde_json::Map::new();
    for (frame, live) in alive(&mut scene, plane, &[10, 30]) {
        out.insert(frame.to_string(), json!(live[0].position.to_array()));
    }
    Value::Object(out)
}

fn turbulence(strength: f32, size: f32) -> Value {
    let mut scene = Scene::empty();
    let plane = emitter(&mut scene, 2.0, 400, 0.0);
    let mut made = ForceField::new(FieldKind::Turbulence);
    made.strength = strength;
    made.size = size;
    field(&mut scene, made, Vec3::ZERO, Vec3::ZERO);
    let mut out = serde_json::Map::new();
    for (frame, live) in alive(&mut scene, plane, &[10, 30]) {
        out.insert(frame.to_string(), json!(live.iter().map(|particle| particle.velocity.length()).sum::<f32>() / live.len() as f32));
    }
    Value::Object(out)
}

fn density(weight: fn(f32) -> f32, from: EmitFrom) -> Value {
    let mut scene = Scene::empty();
    let mut mesh = Primitive::Grid { x_subdivisions: 11, y_subdivisions: 11, size: 2.0 }.build().unwrap();
    let weights = mesh.positions.iter().map(|point| weight(point.x)).collect();
    mesh.groups.push(VertexGroup { name: "Density".into(), weights });
    let plane = scene.add_object("Plane", ObjectData::Mesh(mesh));
    let mut system = ParticleSystem::new("P", Kind::Emitter);
    system.frame_end = 1.0;
    system.lifetime = 100.0;
    system.normal_velocity = 0.0;
    system.gravity = 0.0;
    system.emit_from = from;
    system.density_group = Some("Density".into());
    scene.object_mut(plane).unwrap().particles.push(system);
    let (_, live) = alive(&mut scene, plane, &[2]).remove(0);
    let xs: Vec<f32> = live.iter().map(|particle| particle.position.x).collect();
    json!({ "alive": xs.len(), "meanX": xs.iter().sum::<f32>() / xs.len() as f32, "negative": xs.iter().filter(|x| **x < 0.0).count() })
}

fn ours() -> Value {
    let along_x = Vec3::new(0.0, FRAC_PI_2, 0.0);
    json!({
        "force": single(FieldKind::Force, Vec3::new(2.0, 0.0, 0.0), Vec3::ZERO, 0.0, |_| {}),
        "forceFalloff": single(FieldKind::Force, Vec3::new(2.0, 0.0, 0.0), Vec3::ZERO, 0.0, |f| f.falloff_power = 2.0),
        "wind": single(FieldKind::Wind, Vec3::new(0.0, 0.0, -3.0), Vec3::ZERO, 0.0, |_| {}),
        "windStill": single(FieldKind::Wind, Vec3::new(-3.0, 0.0, 0.0), along_x, 0.0, |f| {
            f.flow = 0.0;
            f.strength = 5.0;
        }),
        "vortex": single(FieldKind::Vortex, Vec3::new(1.0, 0.0, 0.0), Vec3::ZERO, 0.0, |_| {}),
        "vortexStrong": single(FieldKind::Vortex, Vec3::new(1.0, 0.0, 0.0), Vec3::ZERO, 0.0, |f| f.strength = 2.0),
        "drag": single(FieldKind::Drag, Vec3::ZERO, Vec3::ZERO, 2.0, |_| {}),
        // The reference's `linear_drag` 1 and `quadratic_drag` 0: slowing with the speed squared.
        "dragSquare": single(FieldKind::Drag, Vec3::ZERO, Vec3::ZERO, 4.0, |f| {
            f.linear_drag = 0.0;
            f.quadratic_drag = 1.0;
        }),
        "turbulence": turbulence(1.0, 1.0),
        "turbulenceStrong": turbulence(5.0, 1.0),
        "turbulenceLarge": turbulence(5.0, 2.0),
        "densityRamp": density(|x| (x + 1.0) / 2.0, EmitFrom::Faces),
        "densityStep": density(|x| if x > 0.0 { 1.0 } else { 0.0 }, EmitFrom::Faces),
        "densityVertices": density(|x| (x + 1.0) / 2.0, EmitFrom::Verts),
    })
}

fn vector(value: &Value) -> Vec3 {
    let numbers: Vec<f32> = value.as_array().unwrap().iter().map(|number| number.as_f64().unwrap() as f32).collect();
    Vec3::new(numbers[0], numbers[1], numbers[2])
}

fn number(value: &Value) -> f32 {
    value.as_f64().unwrap() as f32
}

fn compare(reference: &Value, ours: &Value) {
    for case in ["force", "forceFalloff", "wind", "windStill", "vortex", "vortexStrong", "drag", "dragSquare"] {
        for frame in ["10", "30"] {
            let (theirs, mine) = (vector(&reference[case][frame]), vector(&ours[case][frame]));
            eprintln!("{case} frame {frame}: {mine:.4} (reference {theirs:.4})");
            // Within 1 mm and 2 % of the way travelled (the integrators differ a little).
            let travelled = theirs.length();
            assert!(mine.distance(theirs) <= 0.001 + 0.02 * travelled, "{case} at frame {frame}: {mine} against {theirs}");
        }
    }
    for case in ["turbulence", "turbulenceStrong", "turbulenceLarge"] {
        for frame in ["10", "30"] {
            let (theirs, mine) = (number(&reference[case][frame]), number(&ours[case][frame]));
            eprintln!("{case} frame {frame}: mean speed {mine:.4} (reference {theirs:.4})");
            // Different noise: its strength within a third.
            assert!((mine / theirs - 1.0).abs() < 0.34, "{case} at frame {frame}: {mine} against {theirs}");
        }
    }
    // Faces: each face weighted by its corners' mean weight (ours by triangle, the reference's by
    // face, which moves the straddling column's share a little). Vertices: ours exactly in
    // proportion to the weights (a mean x of 0.394 on this grid); the reference leans a little
    // further to the heavy vertices (0.434).
    for (case, spread) in [("densityRamp", 25), ("densityStep", 25), ("densityVertices", 40)] {
        let (theirs, mine) = (&reference[case], &ours[case]);
        eprintln!(
            "{case}: {} alive, mean x {:.4}, {} on the left (reference {}, {:.4}, {})",
            mine["alive"],
            number(&mine["meanX"]),
            mine["negative"],
            theirs["alive"],
            number(&theirs["meanX"]),
            theirs["negative"]
        );
        assert_eq!(mine["alive"], theirs["alive"], "{case}: the count stays");
        assert!((number(&mine["meanX"]) - number(&theirs["meanX"])).abs() < 0.05, "{case}: {mine} against {theirs}");
        let (left_theirs, left_mine) = (theirs["negative"].as_i64().unwrap(), mine["negative"].as_i64().unwrap());
        assert!((left_mine - left_theirs).abs() <= spread, "{case}: {left_mine} on the left against {left_theirs}");
    }
}

#[test]
fn fields_and_densities_match_the_recorded_reference() {
    compare(&recorded(), &ours());
}

#[test]
fn fields_and_densities_match_the_reference_application() {
    let Some(blender) = std::env::var_os("KITION_BLENDER") else {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    };
    let output = std::env::temp_dir().join(format!("kition-scene-fields-{}.json", std::process::id()));
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/blender/fields.py");
    let status = std::process::Command::new(blender)
        .args(["-b", "--factory-startup", "--python", script, "--"])
        .arg(&output)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let reference: Value = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    let _ = std::fs::remove_file(&output);
    compare(&reference, &ours());
}
