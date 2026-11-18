//! Soft bodies against the reference application: the default cube wobbling on its goal, a grid
//! hanging from a pinned edge (with and without bending), a chain of two edges below a goal and
//! a bent one, a cube dropped onto a collision plane, a small body bouncing off planes of three
//! dampings, a grid in a Force field (two masses) and a Wind, and a cube simulated only over
//! frames 10 to 30. What Blender 5.2.3 gave is in `fixtures/soft-reference.json` and is checked
//! always; with `KITION_BLENDER` naming a Blender executable, `soft.py` makes it again there.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::f32::consts::FRAC_PI_2;

use kition_scene_model::glam::Vec3;
use kition_scene_model::physics::{Collision, FieldKind, ForceField, ObjectPhysics, SoftBody};
use kition_scene_model::vertex_data::VertexGroup;
use kition_scene_model::{Id, Mesh, ObjectData, Primitive, Scene};
use serde_json::{Value, json};

fn scene(end: i32) -> Scene {
    let mut scene = Scene::empty();
    scene.physics.frame_end = end;
    scene
}

fn add(scene: &mut Scene, mesh: Mesh, location: Vec3, physics: ObjectPhysics) -> Id {
    let id = scene.add_object("Object", ObjectData::Mesh(mesh));
    let object = scene.object_mut(id).unwrap();
    object.location = location;
    object.physics = Some(physics);
    id
}

fn soft(settings: impl FnOnce(&mut SoftBody)) -> ObjectPhysics {
    let mut made = SoftBody::default();
    settings(&mut made);
    ObjectPhysics { soft_body: Some(made), ..ObjectPhysics::default() }
}

fn collider(damping: f32) -> ObjectPhysics {
    ObjectPhysics { collision: Some(Collision { damping, ..Collision::default() }), ..ObjectPhysics::default() }
}

fn pin(mesh: &mut Mesh, pinned: impl Fn(Vec3) -> bool) {
    let weights = mesh.positions.iter().map(|point| if pinned(*point) { 1.0 } else { 0.0 }).collect();
    mesh.groups.push(VertexGroup { name: "Pin".into(), weights });
}

fn r(value: f32) -> f32 {
    (value * 1e5).round() / 1e5
}

/// The body's vertices in world space at every frame, summarised as the script does.
fn summary(mut scene: Scene, id: Id, full: &[usize]) -> Value {
    let baked = kition_scene_physics::bake(&scene).unwrap();
    scene.physics_cache.baked = Some(std::sync::Arc::new(baked));
    assert!(scene.refresh_physics());
    let world = scene.world_matrix(id).unwrap();
    let frames: Vec<Vec<Vec3>> = scene.physics_cache.baked.as_ref().unwrap().cloths[&id]
        .iter()
        .map(|frame| frame.iter().map(|point| world.transform_point3(*point)).collect())
        .collect();
    let centroid: Vec<Value> = frames
        .iter()
        .map(|frame| {
            let mean = frame.iter().copied().sum::<Vec3>() / frame.len() as f32;
            json!([r(mean.x), r(mean.y), r(mean.z)])
        })
        .collect();
    let lowest: Vec<f32> = frames.iter().map(|frame| r(frame.iter().map(|point| point.z).fold(f32::INFINITY, f32::min))).collect();
    let mut out = json!({ "centroid": centroid, "lowest": lowest });
    if !full.is_empty() {
        let points = |frame: &[Vec3]| frame.iter().map(|point| json!([r(point.x), r(point.y), r(point.z)])).collect::<Vec<_>>();
        out["start"] = json!(points(&frames[0]));
        out["frames"] = full.iter().map(|frame| (frame.to_string(), json!(points(&frames[frame - 1])))).collect::<serde_json::Map<_, _>>().into();
    }
    out
}

fn jelly() -> Value {
    let mut scene = scene(60);
    let id = add(&mut scene, Primitive::Cube { size: 2.0 }.build().unwrap(), Vec3::ZERO, soft(|_| {}));
    summary(scene, id, &[])
}

fn hang(bending: f32) -> Value {
    let mut scene = scene(120);
    let mut mesh = Primitive::Grid { x_subdivisions: 10, y_subdivisions: 10, size: 2.0 }.build().unwrap();
    pin(&mut mesh, |point| point.y < -0.99);
    let physics = soft(|s| {
        s.goal_group = Some("Pin".into());
        s.goal_default = 1.0;
        s.bending = bending;
    });
    let id = add(&mut scene, mesh, Vec3::ZERO, physics);
    summary(scene, id, &[11, 31, 61, 120])
}

fn chain(bent: bool) -> Value {
    let mut scene = scene(240);
    let positions = if bent {
        vec![Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0)]
    } else {
        vec![Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0), Vec3::new(0.0, 0.0, -2.0)]
    };
    let mut mesh = Mesh { positions, loose_edges: vec![[0, 1], [1, 2]], ..Mesh::default() };
    pin(&mut mesh, |point| if bent { point.x < 1.5 } else { point.z > -0.5 });
    let physics = soft(|s| {
        s.goal_group = Some("Pin".into());
        s.friction = 5.0;
        if bent {
            s.bending = 2.0;
        }
    });
    let id = add(&mut scene, mesh, Vec3::ZERO, physics);
    summary(scene, id, &[240])
}

fn drop() -> Value {
    let mut scene = scene(72);
    add(&mut scene, Primitive::Plane { size: 10.0 }.build().unwrap(), Vec3::ZERO, collider(0.1));
    let id = add(&mut scene, Primitive::Cube { size: 1.0 }.build().unwrap(), Vec3::new(0.0, 0.0, 2.0), soft(|s| s.goal = false));
    summary(scene, id, &[])
}

fn bounce(damping: f32) -> Value {
    let mut scene = scene(96);
    add(&mut scene, Primitive::Plane { size: 10.0 }.build().unwrap(), Vec3::ZERO, collider(damping));
    let physics = soft(|s| {
        s.goal = false;
        s.edges = false;
    });
    let id = add(&mut scene, Primitive::Plane { size: 0.2 }.build().unwrap(), Vec3::new(0.0, 0.0, 1.0), physics);
    summary(scene, id, &[])
}

fn field(kind: FieldKind, mass: f32) -> Value {
    let mut scene = scene(48);
    scene.physics.gravity = Vec3::ZERO;
    let grid = Primitive::Grid { x_subdivisions: 4, y_subdivisions: 4, size: 1.0 }.build().unwrap();
    let id = add(
        &mut scene,
        grid,
        Vec3::ZERO,
        soft(|s| {
            s.goal = false;
            s.mass = mass;
        }),
    );
    let owner = scene.add_object("Field", ObjectData::Empty { size: 1.0 });
    let object = scene.object_mut(owner).unwrap();
    if kind == FieldKind::Force {
        object.location = Vec3::new(3.0, 0.0, 0.0);
    } else {
        object.location = Vec3::new(-3.0, 0.0, 0.0);
        object.rotation = Vec3::new(0.0, FRAC_PI_2, 0.0);
    }
    object.physics = Some(ObjectPhysics { field: Some(ForceField::new(kind)), ..ObjectPhysics::default() });
    summary(scene, id, &[])
}

fn frame_range() -> Value {
    let mut scene = scene(48);
    let physics = soft(|s| {
        s.frame_start = 10;
        s.frame_end = 30;
    });
    let id = add(&mut scene, Primitive::Cube { size: 2.0 }.build().unwrap(), Vec3::ZERO, physics);
    summary(scene, id, &[])
}

fn ours() -> Value {
    json!({
        "jelly": jelly(),
        "hang": hang(0.0),
        "hangBend": hang(1.0),
        "pair": chain(false),
        "bend": chain(true),
        "drop": drop(),
        "bounce": bounce(0.0),
        "bounceDefault": bounce(0.1),
        "bounceDead": bounce(0.5),
        "force": field(FieldKind::Force, 1.0),
        "forceHeavy": field(FieldKind::Force, 2.0),
        "wind": field(FieldKind::Wind, 1.0),
        "range": frame_range(),
    })
}

fn point(value: &Value) -> Vec3 {
    let numbers: Vec<f32> = value.as_array().unwrap().iter().map(|number| number.as_f64().unwrap() as f32).collect();
    Vec3::new(numbers[0], numbers[1], numbers[2])
}

fn series(value: &Value) -> Vec<Vec3> {
    value.as_array().unwrap().iter().map(point).collect()
}

fn numbers(value: &Value) -> Vec<f32> {
    value.as_array().unwrap().iter().map(|number| number.as_f64().unwrap() as f32).collect()
}

/// The largest distance between two centroid paths over `frames` (1-based, inclusive).
fn path_error(reference: &Value, ours: &Value, case: &str, frames: std::ops::RangeInclusive<usize>) -> f32 {
    let (theirs, mine) = (series(&reference[case]["centroid"]), series(&ours[case]["centroid"]));
    assert_eq!(theirs.len(), mine.len(), "{case}: frames");
    frames.map(|frame| theirs[frame - 1].distance(mine[frame - 1])).fold(0.0, f32::max)
}

/// Every vertex at `frame` against the reference's vertex that started at the same place.
fn vertex_error(reference: &Value, ours: &Value, case: &str, frame: &str) -> f32 {
    let (their_start, my_start) = (series(&reference[case]["start"]), series(&ours[case]["start"]));
    let (theirs, mine) = (series(&reference[case]["frames"][frame]), series(&ours[case]["frames"][frame]));
    assert_eq!(their_start.len(), my_start.len(), "{case}: vertices");
    my_start
        .iter()
        .zip(&mine)
        .map(|(start, now)| {
            let twin = their_start.iter().position(|other| other.distance(*start) < 1e-4).unwrap_or_else(|| panic!("{case}: no twin for {start}"));
            now.distance(theirs[twin])
        })
        .fold(0.0, f32::max)
}

/// Where a hanging body's centroid is on average over frames 60 to 120, how far it swings along
/// Y (the range), and its mean lowest point.
fn hanging(summary: &Value, case: &str) -> (Vec3, f32, f32) {
    let centroid = series(&summary[case]["centroid"]);
    let lowest = numbers(&summary[case]["lowest"]);
    let window = &centroid[59..120];
    let mean = window.iter().copied().sum::<Vec3>() / window.len() as f32;
    let (low, high) = window.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), point| (low.min(point.y), high.max(point.y)));
    let deepest = lowest[59..120].iter().sum::<f32>() / 61.0;
    (mean, high - low, deepest)
}

/// The first rebound's height: the highest after the lowest of the first `within` frames.
fn rebound(lowest: &[f32], within: usize) -> f32 {
    let touch = (0..within).min_by(|a, b| lowest[*a].total_cmp(&lowest[*b])).unwrap();
    lowest[touch..].iter().take(30).copied().fold(f32::NEG_INFINITY, f32::max)
}

fn compare(reference: &Value, ours: &Value) {
    // The default cube on its goal (weight 0.7, every vertex alike): it sags and wobbles as one;
    // the whole path matches within half a millimetre.
    let jelly = path_error(reference, ours, "jelly", 1..=60);
    eprintln!("jelly: centroid path within {jelly:.5} m");
    assert!(jelly < 1e-3, "jelly: {jelly}");
    // A grid hanging from a pinned edge: the pinned row stays exactly; the first frames match
    // vertex by vertex; later the swing differs (compare the centroids).
    for case in ["hang", "hangBend"] {
        let (theirs, mine) = (series(&reference[case]["frames"]["120"]), series(&ours[case]["frames"]["120"]));
        let (their_start, my_start) = (series(&reference[case]["start"]), series(&ours[case]["start"]));
        for (start, now) in my_start.iter().zip(&mine).filter(|(start, _)| start.y < -0.99) {
            assert!(now.distance(*start) < 1e-5, "{case}: the pinned vertex at {start} moved to {now}");
        }
        for (start, now) in their_start.iter().zip(&theirs).filter(|(start, _)| start.y < -0.99) {
            assert!(now.distance(*start) < 1e-5, "{case}: the reference's pinned vertex moved");
        }
        let early = vertex_error(reference, ours, case, "11");
        let falling = path_error(reference, ours, case, 1..=31);
        let (their_hang, my_hang) = (hanging(reference, case), hanging(ours, case));
        eprintln!(
            "{case}: every vertex within {early:.4} m at frame 11, centroid within {falling:.4} m to frame 31; over frames 60 to 120 the centroid hangs at {:.3}, {:.3} m swinging {:.3} m (reference {:.3}, {:.3}, {:.3}) and the lowest point at {:.3} m (reference {:.3})",
            my_hang.0.y, my_hang.0.z, my_hang.1, their_hang.0.y, their_hang.0.z, their_hang.1, my_hang.2, their_hang.2
        );
        assert!(early < 0.01, "{case}: frame 11 {early}");
        assert!(falling < 0.02, "{case}: centroid to frame 31 {falling}");
        assert!(my_hang.0.distance(their_hang.0) < 0.05, "{case}: hangs at {} against {}", my_hang.0, their_hang.0);
        assert!((my_hang.1 - their_hang.1).abs() < 0.3 * their_hang.1, "{case}: swings {} against {}", my_hang.1, their_hang.1);
        assert!((my_hang.2 - their_hang.2).abs() < 0.1, "{case}: lowest {} against {}", my_hang.2, their_hang.2);
    }
    // Chains: a vertex on a goal (weight 0.7) holding two below it on edges, and a bent one.
    for case in ["pair", "bend"] {
        let error = vertex_error(reference, ours, case, "240");
        let path = path_error(reference, ours, case, 1..=240);
        eprintln!("{case}: every vertex within {error:.4} m at frame 240; centroid path within {path:.4} m");
        assert!(error < 5e-3, "{case}: {error}");
        assert!(path < 0.02, "{case}: {path}");
    }
    // A cube dropped onto a plane: falls as the reference's until it lands, bounces, and rests
    // (rebounds are the centroid's).
    let fall = path_error(reference, ours, "drop", 1..=18);
    let (their_low, my_low) = (numbers(&reference["drop"]["lowest"]), numbers(&ours["drop"]["lowest"]));
    let (their_rest, my_rest) = (series(&reference["drop"]["centroid"])[71], series(&ours["drop"]["centroid"])[71]);
    let heights = |summary: &Value, case: &str| series(&summary[case]["centroid"]).iter().map(|point| point.z).collect::<Vec<f32>>();
    let (their_bounce, my_bounce) = (rebound(&heights(reference, "drop"), 30), rebound(&heights(ours, "drop"), 30));
    eprintln!(
        "drop: falling within {fall:.5} m; first rebound to {my_bounce:.3} m (reference {their_bounce:.3}); rests at {:.4} m, lowest {:.4} (reference {:.4}, {:.4})",
        my_rest.z, my_low[71], their_rest.z, their_low[71]
    );
    assert!(fall < 1e-3, "drop: {fall}");
    assert!((my_bounce - their_bounce).abs() < 0.08, "drop: rebound {my_bounce} against {their_bounce}");
    assert!((my_rest.z - their_rest.z).abs() < 0.01 && (my_low[71] - their_low[71]).abs() < 0.005, "drop: rest {my_rest} against {their_rest}");
    // Bounces off planes of collision damping 0, 0.1, and 0.5.
    for case in ["bounce", "bounceDefault", "bounceDead"] {
        let (theirs, mine) = (heights(reference, case), heights(ours, case));
        let (their_bounce, my_bounce) = (rebound(&theirs, 20), rebound(&mine, 20));
        let (their_rest, my_rest) = (*theirs.last().unwrap(), *mine.last().unwrap());
        eprintln!("{case}: rebound to {my_bounce:.4} m (reference {their_bounce:.4}), rests at {my_rest:.4} m (reference {their_rest:.4})");
        assert!((my_bounce - their_bounce).abs() < 0.01, "{case}: rebound {my_bounce} against {their_bounce}");
        assert!((my_rest - their_rest).abs() < 2e-3, "{case}: rest {my_rest} against {their_rest}");
    }
    // Fields: the grid's centroid along X.
    for case in ["force", "forceHeavy", "wind"] {
        let (theirs, mine) = (series(&reference[case]["centroid"]), series(&ours[case]["centroid"]));
        for frame in [12, 24, 48] {
            let (theirs, mine) = (theirs[frame - 1].x, mine[frame - 1].x);
            eprintln!("{case} frame {frame}: {mine:.4} m (reference {theirs:.4})");
            assert!((mine - theirs).abs() <= 0.02 * theirs.abs() + 2e-3, "{case} at frame {frame}: {mine} against {theirs}");
        }
    }
    // Frames 10 to 30 only: at rest before, simulated through, held after.
    let range = path_error(reference, ours, "range", 1..=48);
    let mine = series(&ours["range"]["centroid"]);
    eprintln!("range: centroid path within {range:.5} m");
    assert!(mine[..10].iter().all(|point| point.length() < 1e-6), "at rest before frame 10");
    assert!(mine[30..].iter().all(|point| *point == mine[29]), "held after frame 30");
    assert!(range < 1e-3, "range: {range}");
}

fn recorded() -> Value {
    serde_json::from_str(include_str!("fixtures/soft-reference.json")).unwrap()
}

#[test]
fn soft_bodies_match_the_recorded_reference() {
    compare(&recorded(), &ours());
}

#[test]
fn soft_bodies_match_the_reference_application() {
    let Some(blender) = std::env::var_os("KITION_BLENDER") else {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    };
    let output = std::env::temp_dir().join(format!("kition-scene-soft-{}.json", std::process::id()));
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/soft.py");
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
