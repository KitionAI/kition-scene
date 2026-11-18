//! Hair dynamics against the reference application: four 1 m strands of four segments grown
//! level from a 10 cm plane and drooping under gravity (stiffness 0.05, 0.5, and 5, twice the
//! length, a heavier point, more bending and air damping, a pin towards the rest shape), upright
//! strands standing, bending in a Force field (two masses) and a Wind, level strands landing on a
//! collision plane, and upright strands on an emitter that slides 1 m. Strands start at
//! different places on the plane in the two applications, so the strands' mean tip (from its
//! root) is compared. What Blender 5.2.3 gave is in `fixtures/hair-reference.json` and is checked
//! always; with `KITION_BLENDER` naming a Blender executable, `hair.py` makes it again there.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::f32::consts::FRAC_PI_2;

use kition_scene_model::animation::{Interpolation, Property};
use kition_scene_model::glam::Vec3;
use kition_scene_model::particles::{HairDynamics, Kind, ParticleSystem};
use kition_scene_model::physics::{Collision, FieldKind, ForceField, ObjectPhysics};
use kition_scene_model::{ObjectData, Primitive, Scene};
use serde_json::{Value, json};

#[derive(Default)]
struct Case {
    upright: bool,
    length: Option<f32>,
    end: Option<i32>,
    weightless: bool,
    floor: Option<f32>,
    field: Option<FieldKind>,
    moving: bool,
}

fn r(value: f32) -> f32 {
    (value * 1e5).round() / 1e5
}

fn strands(case: Case, settings: impl FnOnce(&mut HairDynamics)) -> Value {
    let end = case.end.unwrap_or(200);
    let mut scene = Scene::empty();
    scene.physics.frame_end = end;
    if case.weightless {
        scene.physics.gravity = Vec3::ZERO;
    }
    if let Some(height) = case.floor {
        let floor = scene.add_object("Floor", ObjectData::Mesh(Primitive::Plane { size: 10.0 }.build().unwrap()));
        let object = scene.object_mut(floor).unwrap();
        object.location = Vec3::new(0.0, 0.0, height);
        object.physics = Some(ObjectPhysics { collision: Some(Collision::default()), ..ObjectPhysics::default() });
    }
    if let Some(kind) = case.field {
        let owner = scene.add_object("Field", ObjectData::Empty { size: 1.0 });
        let object = scene.object_mut(owner).unwrap();
        if kind == FieldKind::Force {
            object.location = Vec3::new(3.0, 0.0, 0.0);
        } else {
            object.location = Vec3::new(-3.0, 0.0, 0.0);
            object.rotation = Vec3::new(0.0, FRAC_PI_2, 0.0);
        }
        object.physics = Some(ObjectPhysics { field: Some(ForceField::new(kind)), ..ObjectPhysics::default() });
    }
    let id = scene.add_object("Emitter", ObjectData::Mesh(Primitive::Plane { size: 0.1 }.build().unwrap()));
    let mut dynamics = HairDynamics::default();
    settings(&mut dynamics);
    let mut system = ParticleSystem::new("Hair", Kind::Hair);
    system.count = 4;
    system.hair_length = case.length.unwrap_or(1.0);
    system.segments = 4;
    system.dynamics = Some(dynamics);
    let object = scene.object_mut(id).unwrap();
    object.particles.push(system);
    if !case.upright {
        object.rotation = Vec3::new(FRAC_PI_2, 0.0, 0.0);
    }
    if case.moving {
        scene.insert_key(id, Property::Location, 1.0).unwrap();
        scene.object_mut(id).unwrap().location = Vec3::new(1.0, 0.0, 0.0);
        scene.insert_key(id, Property::Location, 25.0).unwrap();
        scene.object_mut(id).unwrap().location = Vec3::ZERO;
        for channel in scene.animation.channels.iter_mut().filter(|channel| channel.target == id) {
            channel.keys.iter_mut().for_each(|key| key.interpolation = Interpolation::Linear);
        }
    }
    let baked = kition_scene_physics::bake(&scene).unwrap();
    let frames = &baked.hair[&(id, 0)];
    let (mut tips, mut roots, mut lowest, mut lengths) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (index, frame) in frames.iter().enumerate() {
        let world = scene.at_frame((baked.start + index as i32) as f32).world_matrix(id).unwrap();
        let points: Vec<Vec<Vec3>> = frame.iter().map(|strand| strand.iter().map(|point| world.transform_point3(*point)).collect()).collect();
        let count = points.len() as f32;
        let tip = points.iter().map(|strand| *strand.last().unwrap() - strand[0]).sum::<Vec3>() / count;
        let root = points.iter().map(|strand| strand[0]).sum::<Vec3>() / count;
        tips.push(json!([r(tip.x), r(tip.y), r(tip.z)]));
        roots.push(json!([r(root.x), r(root.y), r(root.z)]));
        lowest.push(r(points.iter().flatten().map(|point| point.z).fold(f32::INFINITY, f32::min)));
        let length = points.iter().map(|strand| strand.windows(2).map(|pair| pair[0].distance(pair[1])).sum::<f32>()).sum::<f32>() / count;
        lengths.push(r(length));
    }
    json!({ "tip": tips, "root": roots, "lowest": lowest, "length": lengths })
}

fn ours() -> Value {
    let level = || Case::default();
    let upright = || Case { upright: true, end: Some(48), ..Case::default() };
    json!({
        "level": strands(level(), |_| {}),
        "levelSoft": strands(level(), |d| d.stiffness = 0.05),
        "levelStiff": strands(level(), |d| d.stiffness = 5.0),
        "long": strands(Case { length: Some(2.0), ..level() }, |_| {}),
        "heavy": strands(level(), |d| d.mass = 1.0),
        "damped": strands(level(), |d| d.damping = 5.0),
        "air": strands(level(), |d| d.air_damping = 10.0),
        "pinned": strands(level(), |d| d.pin_stiffness = 1.0),
        "upright": strands(upright(), |_| {}),
        "force": strands(Case { weightless: true, field: Some(FieldKind::Force), ..upright() }, |_| {}),
        "forceHeavy": strands(Case { weightless: true, field: Some(FieldKind::Force), ..upright() }, |d| d.mass = 1.0),
        "wind": strands(Case { weightless: true, field: Some(FieldKind::Wind), ..upright() }, |_| {}),
        "floor": strands(Case { end: Some(96), floor: Some(-0.3), ..level() }, |_| {}),
        "moving": strands(Case { moving: true, ..upright() }, |_| {}),
    })
}

fn point(value: &Value) -> Vec3 {
    let numbers: Vec<f32> = value.as_array().unwrap().iter().map(|number| number.as_f64().unwrap() as f32).collect();
    Vec3::new(numbers[0], numbers[1], numbers[2])
}

fn tips(summary: &Value, case: &str) -> Vec<Vec3> {
    summary[case]["tip"].as_array().unwrap().iter().map(point).collect()
}

fn numbers(summary: &Value, case: &str, key: &str) -> Vec<f32> {
    summary[case][key].as_array().unwrap().iter().map(|number| number.as_f64().unwrap() as f32).collect()
}

/// The mean tip (from its root) over the last 50 frames, where the strands have settled.
fn settled(summary: &Value, case: &str) -> Vec3 {
    let all = tips(summary, case);
    all[all.len() - 50..].iter().copied().sum::<Vec3>() / 50.0
}

/// The root-mean-square distance between the two tip paths, and the largest.
fn path(reference: &Value, ours: &Value, case: &str) -> (f32, f32) {
    let (theirs, mine) = (tips(reference, case), tips(ours, case));
    assert_eq!(theirs.len(), mine.len(), "{case}: frames");
    let distances: Vec<f32> = theirs.iter().zip(&mine).map(|(a, b)| a.distance(*b)).collect();
    ((distances.iter().map(|d| d * d).sum::<f32>() / distances.len() as f32).sqrt(), distances.iter().copied().fold(0.0, f32::max))
}

fn compare(reference: &Value, ours: &Value) {
    // Every case: the roots stay on the emitter (exactly), the strands keep their length.
    for case in ["level", "levelSoft", "levelStiff", "long", "heavy", "damped", "air", "pinned", "upright", "force", "forceHeavy", "wind", "floor"] {
        let roots: Vec<Vec3> = ours[case]["root"].as_array().unwrap().iter().map(point).collect();
        assert!(roots.iter().all(|root| root.distance(roots[0]) < 1e-5), "{case}: the roots move");
        let rest = if case == "long" { 2.0 } else { 1.0 };
        let (theirs, mine) = (numbers(reference, case, "length"), numbers(ours, case, "length"));
        let stretch = |lengths: &[f32]| lengths.iter().map(|length| (length - rest).abs()).fold(0.0, f32::max);
        eprintln!("{case}: lengths within {:.4} m of {rest} m (reference {:.4})", stretch(&mine), stretch(&theirs));
        assert!(stretch(&mine) < 2e-3 * rest, "{case}: stretched {}", stretch(&mine));
    }
    // Level strands droop into the same settled shape (twice as long: twice the size).
    for (case, scale) in [("level", 1.0), ("levelSoft", 1.0), ("levelStiff", 1.0), ("long", 2.0), ("heavy", 1.0), ("pinned", 1.0)] {
        let (theirs, mine) = (settled(reference, case), settled(ours, case));
        let (rms, largest) = path(reference, ours, case);
        eprintln!("{case}: settles with the tip at {mine:.3} (reference {theirs:.3}); path within {rms:.3} m rms, {largest:.3} m at most");
        assert!(mine.distance(theirs) < 0.03 * scale, "{case}: settles at {mine} against {theirs}");
    }
    // How they get there: falling, the swing, and how fast it dies.
    for case in ["level", "levelSoft", "levelStiff", "long", "heavy", "damped", "air"] {
        let (rms, largest) = path(reference, ours, case);
        eprintln!("{case}: tip path within {rms:.3} m rms, {largest:.3} m at most");
        assert!(rms < 0.1, "{case}: path {rms}");
    }
    // The first frames, before the strands' shapes part (more bending damping slows the
    // reference's first frames more than ours).
    for (case, within) in [("level", 0.005), ("heavy", 0.005), ("damped", 0.025)] {
        let (theirs, mine) = (tips(reference, case), tips(ours, case));
        let early = (0..6).map(|frame| theirs[frame].distance(mine[frame])).fold(0.0, f32::max);
        eprintln!("{case}: first six frames within {early:.4} m");
        assert!(early < within, "{case}: first frames {early}");
    }
    // Upright strands stand.
    let standing = tips(ours, "upright");
    assert!(standing.iter().all(|tip| (tip.z - 1.0).abs() < 2e-3 && tip.truncate().length() < 1e-4), "upright strands stand");
    // Fields bend upright strands (no gravity).
    for case in ["force", "forceHeavy", "wind"] {
        let (theirs, mine) = (tips(reference, case), tips(ours, case));
        for frame in [12, 24, 36, 48] {
            let (theirs, mine) = (theirs[frame - 1].x, mine[frame - 1].x);
            eprintln!("{case} frame {frame}: tip x {mine:.4} m (reference {theirs:.4})");
            assert!((mine - theirs).abs() < 0.01 + 0.15 * theirs.abs(), "{case} at frame {frame}: {mine} against {theirs}");
        }
    }
    // Level strands land on a collision plane 0.3 m down and lie on it.
    let (theirs, mine) = (numbers(reference, "floor", "lowest"), numbers(ours, "floor", "lowest"));
    let (their_rest, my_rest) =
        (theirs[60..].iter().copied().fold(f32::INFINITY, f32::min), mine[60..].iter().copied().fold(f32::INFINITY, f32::min));
    eprintln!("floor: the lowest point rests at {my_rest:.4} m (reference {their_rest:.4})");
    assert!(my_rest > -0.3 + 0.015 && (my_rest - their_rest).abs() < 0.01, "floor: {my_rest} against {their_rest}");
    // A sliding emitter: the roots go with it exactly; the tips trail and then swing past.
    let (their_roots, my_roots): (Vec<Vec3>, Vec<Vec3>) = (
        reference["moving"]["root"].as_array().unwrap().iter().map(point).collect(),
        ours["moving"]["root"].as_array().unwrap().iter().map(point).collect(),
    );
    for frame in [1, 13, 25, 48] {
        let (theirs, mine) = (their_roots[frame - 1].x - their_roots[0].x, my_roots[frame - 1].x - my_roots[0].x);
        assert!((mine - theirs).abs() < 1e-4, "moving: the root at frame {frame} has gone {mine} (reference {theirs})");
    }
    let (theirs, mine) = (tips(reference, "moving"), tips(ours, "moving"));
    for frame in [12, 24, 36, 48] {
        eprintln!("moving frame {frame}: tip {:.3} (reference {:.3})", mine[frame - 1], theirs[frame - 1]);
    }
    assert!(mine[23].x < -0.3 && theirs[23].x < -0.3, "the tips trail while the emitter slides");
    assert!(mine[47].x > 0.2 && theirs[47].x > 0.2, "and swing past when it stops");
    let (rms, _) = path(reference, ours, "moving");
    eprintln!("moving: tip path within {rms:.3} m rms");
    assert!(rms < 0.08, "moving: {rms}");
}

fn recorded() -> Value {
    serde_json::from_str(include_str!("fixtures/hair-reference.json")).unwrap()
}

#[test]
fn hair_dynamics_match_the_recorded_reference() {
    compare(&recorded(), &ours());
}

#[test]
fn hair_dynamics_match_the_reference_application() {
    let Some(blender) = std::env::var_os("KITION_BLENDER") else {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    };
    let output = std::env::temp_dir().join(format!("kition-scene-hair-{}.json", std::process::id()));
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/hair.py");
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
