//! Particle statistics against the reference application's: an emitter with default settings on
//! the default 2 m plane (live particles and their mean height at frames 10 and 50) and hair
//! with default settings on the default UV sphere (strand count and mean length). Random
//! sequences differ, so statistics are compared, not particles. The numbers Blender 5.2.3 gave
//! are checked always; when `KITION_BLENDER` names a Blender executable, `tests/blender/
//! particles.py` makes them again there.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_scene_model::particles::{self, Cache, Kind, ParticleSystem, hair};
use kition_scene_model::{ObjectData, Primitive, Scene};
use serde_json::{Value, json};

/// What Blender 5.2.3 gave.
fn recorded() -> Value {
    json!({
        "emitter": { "10": { "alive": 46, "meanHeight": -0.034391692 }, "50": { "alive": 247, "meanHeight": -5.318739128 } },
        "hair": { "strands": 1000, "keys": 6, "meanLength": 4.0 }
    })
}

/// Our statistics, in the shape the script writes.
fn ours() -> Value {
    let mut scene = Scene::empty();
    let plane = scene.add_object("Plane", ObjectData::Mesh(Primitive::Plane { size: 2.0 }.build().unwrap()));
    scene.object_mut(plane).unwrap().particles.push(ParticleSystem::new("Particles", Kind::Emitter));
    let sphere = scene.add_object("Sphere", ObjectData::Mesh(Primitive::UvSphere { segments: 32, rings: 16, radius: 1.0 }.build().unwrap()));
    scene.object_mut(sphere).unwrap().location.x = 10.0;
    scene.object_mut(sphere).unwrap().particles.push(ParticleSystem::new("Hair", Kind::Hair));
    let mut cache = Cache::new();
    let mut emitter = serde_json::Map::new();
    for frame in [10, 50] {
        scene.go_to_frame(frame as f32);
        let live = particles::particles(&scene, plane, 0, &mut cache);
        let mean = live.iter().map(|particle| particle.position.z).sum::<f32>() / live.len().max(1) as f32;
        emitter.insert(frame.to_string(), json!({ "alive": live.len(), "meanHeight": mean }));
    }
    let strands = particles::strands(&scene, sphere, 0, false);
    let mean = strands.iter().map(|strand| hair::length(strand)).sum::<f32>() / strands.len().max(1) as f32;
    json!({ "emitter": emitter, "hair": { "strands": strands.len(), "keys": strands[0].len(), "meanLength": mean } })
}

fn compare(reference: &Value, ours: &Value) {
    for frame in ["10", "50"] {
        let (theirs, mine) = (&reference["emitter"][frame], &ours["emitter"][frame]);
        let (alive_theirs, alive_mine) = (theirs["alive"].as_i64().unwrap(), mine["alive"].as_i64().unwrap());
        let (height_theirs, height_mine) = (theirs["meanHeight"].as_f64().unwrap(), mine["meanHeight"].as_f64().unwrap());
        eprintln!("frame {frame}: alive {alive_mine} (reference {alive_theirs}), mean height {height_mine:.4} m (reference {height_theirs:.4} m)");
        assert!((alive_mine - alive_theirs).abs() <= 1, "frame {frame}: {alive_mine} alive against {alive_theirs}");
        assert!((height_mine - height_theirs).abs() <= 0.01 + 0.01 * height_theirs.abs(), "frame {frame}: {height_mine} against {height_theirs}");
    }
    let (theirs, mine) = (&reference["hair"], &ours["hair"]);
    eprintln!(
        "hair: {} strands of {} points, mean length {:.4} m (reference {} of {}, {:.4} m)",
        mine["strands"],
        mine["keys"],
        mine["meanLength"].as_f64().unwrap(),
        theirs["strands"],
        theirs["keys"],
        theirs["meanLength"].as_f64().unwrap()
    );
    assert_eq!(mine["strands"], theirs["strands"]);
    assert_eq!(mine["keys"], theirs["keys"]);
    assert!((mine["meanLength"].as_f64().unwrap() - theirs["meanLength"].as_f64().unwrap()).abs() < 1e-3);
}

#[test]
fn particles_match_the_recorded_reference() {
    compare(&recorded(), &ours());
}

#[test]
fn particles_match_the_reference_application() {
    let Some(blender) = std::env::var_os("KITION_BLENDER") else {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    };
    let output = std::env::temp_dir().join(format!("kition-scene-particles-{}.json", std::process::id()));
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/blender/particles.py");
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
