//! Rigid body constraints, deactivation, and force fields on rigid bodies and cloth against the
//! reference application: pendulums on a hinge and a point, a hinge stopped by its limit, a
//! slider with and without a limit, a generic spring, two boxes held by a fixed constraint, a
//! breakable constraint that breaks and one that holds, a body that starts asleep (alone, and
//! knocked awake), rigid bodies in a Force field and a Wind, and cloth in both. The numbers
//! Blender 5.2.3 gave are checked always; with `KITION_BLENDER` naming a Blender executable,
//! `constraints.py` makes them again there.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic, clippy::approx_constant)]

use std::f32::consts::FRAC_PI_2;

use kition_scene_model::glam::Vec3;
use kition_scene_model::physics::{BodyKind, Cloth, FieldKind, ForceField, JointKind, ObjectPhysics, RigidBody, RigidConstraint};
use kition_scene_model::{Id, ObjectData, Primitive, Scene};
use serde_json::{Value, json};

/// What Blender 5.2.3 gave (positions in metres, rounded to 0.1 mm).
fn recorded() -> Value {
    json!({"hinge": {"10": [[1.8844, 0.0, -0.6708]], "20": [[0.3258, 0.0, -1.9741]], "30": [[-1.6799, 0.0, -1.0862]], "40": [[-1.9956, 0.0, -0.1334]], "60": [[-0.279, 0.0, -1.9812]], "80": [[1.9852, 0.0, -0.2428]], "100": [[0.119, 0.0, -1.9972]]},
        "point": {"10": [[1.8848, 0.0, -0.6709]], "20": [[0.3311, 0.0, -1.9735]], "30": [[-1.6774, 0.0, -1.0947]], "40": [[-1.9953, 0.0, -0.1366]], "60": [[-0.2918, 0.0, -1.9795]], "80": [[1.9845, 0.0, -0.2482]], "100": [[0.1371, 0.0, -1.996]]},
        "hingeLimit": {"10": [[1.8844, 0.0, -0.6708]], "20": [[1.7319, 0.0, -1.0002]], "30": [[1.7319, 0.0, -1.0002]], "40": [[1.7319, 0.0, -1.0002]], "60": [[1.7319, 0.0, -1.0002]], "80": [[1.7319, 0.0, -1.0002]], "100": [[1.732, 0.0, -1.0002]]},
        "slider": {"30": [[3.535, 0.0, -4.535]], "60": [[14.363, 0.0, -15.363]]},
        "sliderLimit": {"30": [[0.7071, 0.0, -1.7071]], "60": [[0.7071, 0.0, -1.7071]]},
        "spring": {"25": [[0.0, 0.0, -2.73]], "50": [[0.0, 0.0, -1.4163]], "100": [[0.0, 0.0, -1.6793]]},
        "fixed": {"15": [[0.0, 0.0, 1.3322], [1.2, 0.0, 1.8322]], "100": [[0.0654, -0.0025, 0.6416], [1.3654, -0.0116, 0.6416]]},
        "weak": {"40": [[1.9998, 0.0, -12.5628]]},
        "strong": {"40": [[2.0, 0.0, -0.0078]]},
        "asleep": {"60": [[0.0, 0.0, 3.0]], "80": [[0.0, 0.0, 3.0]]},
        "woken": {"60": [[-0.6321, -0.1236, 0.5001]], "80": [[-0.6321, -0.1236, 0.5001]]},
        "force": {"47": [[-0.0747, 0.0, 0.0]]},
        "forceHeavy": {"47": [[-0.0374, 0.0, 0.0]]},
        "wind": {"47": [[0.0436, 0.0, 0.0]]},
        "windStill": {"47": [[0.0747, 0.0, 0.0]]},
        "clothForce": {"24": -0.0048, "48": -0.016},
        "clothForceHeavy": {"24": -0.0017, "48": -0.0065},
        "clothWindFacing": {"24": 0.0048, "48": 0.0159},
        "clothWindFlat": {"24": 0.0, "48": 0.0}})
}

fn add(scene: &mut Scene, primitive: Primitive, location: Vec3, physics: ObjectPhysics) -> Id {
    let id = scene.add_object("Object", ObjectData::Mesh(primitive.build().unwrap()));
    let object = scene.object_mut(id).unwrap();
    object.location = location;
    object.physics = Some(physics);
    id
}

fn body(kind: BodyKind, settings: impl FnOnce(&mut RigidBody)) -> ObjectPhysics {
    let mut made = RigidBody { kind, ..RigidBody::default() };
    settings(&mut made);
    ObjectPhysics { rigid_body: Some(made), ..ObjectPhysics::default() }
}

fn cube(scene: &mut Scene, size: f32, location: Vec3, kind: BodyKind) -> Id {
    add(scene, Primitive::Cube { size }, location, body(kind, |_| {}))
}

fn empty(scene: &mut Scene, location: Vec3, rotation: Vec3, physics: ObjectPhysics) -> Id {
    let id = scene.add_object("Empty", ObjectData::Empty { size: 1.0 });
    let object = scene.object_mut(id).unwrap();
    object.location = location;
    object.rotation = rotation;
    object.physics = Some(physics);
    id
}

fn joint(scene: &mut Scene, kind: JointKind, location: Vec3, rotation: Vec3, first: Id, second: Id, settings: impl FnOnce(&mut RigidConstraint)) {
    let mut constraint = RigidConstraint { kind, object1: Some(first), object2: Some(second), ..RigidConstraint::default() };
    settings(&mut constraint);
    empty(scene, location, rotation, ObjectPhysics { rigid_constraint: Some(constraint), ..ObjectPhysics::default() });
}

fn field(scene: &mut Scene, kind: FieldKind, location: Vec3, rotation: Vec3, flow: Option<f32>) {
    let mut made = ForceField::new(kind);
    if let Some(flow) = flow {
        made.flow = flow;
    }
    empty(scene, location, rotation, ObjectPhysics { field: Some(made), ..ObjectPhysics::default() });
}

fn scene(end: i32) -> Scene {
    let mut scene = Scene::empty();
    scene.physics.frame_end = end;
    scene
}

/// World positions of `ids` at each of `frames`, after baking.
fn at(mut scene: Scene, ids: &[Id], frames: &[i32]) -> Value {
    let baked = kition_scene_physics::bake(&scene).unwrap();
    scene.physics_cache.baked = Some(std::sync::Arc::new(baked));
    assert!(scene.refresh_physics());
    let mut out = serde_json::Map::new();
    for frame in frames {
        let shown = scene.at_frame(*frame as f32);
        let points: Vec<Value> = ids.iter().map(|id| json!(shown.world_matrix(*id).unwrap().w_axis.truncate().to_array())).collect();
        out.insert(frame.to_string(), json!(points));
    }
    Value::Object(out)
}

fn pendulum(kind: JointKind, settings: impl FnOnce(&mut RigidConstraint)) -> Value {
    let mut scene = scene(100);
    let anchor = cube(&mut scene, 0.2, Vec3::ZERO, BodyKind::Passive);
    let bob = cube(&mut scene, 0.5, Vec3::new(2.0, 0.0, 0.0), BodyKind::Active);
    joint(&mut scene, kind, Vec3::ZERO, Vec3::new(FRAC_PI_2, 0.0, 0.0), anchor, bob, settings);
    at(scene, &[bob], &[10, 20, 30, 40, 60, 80, 100])
}

fn slider(settings: impl FnOnce(&mut RigidConstraint)) -> Value {
    let mut scene = scene(60);
    let anchor = cube(&mut scene, 0.2, Vec3::ZERO, BodyKind::Passive);
    let bob = cube(&mut scene, 0.5, Vec3::new(0.0, 0.0, -1.0), BodyKind::Active);
    joint(&mut scene, JointKind::Slider, Vec3::ZERO, Vec3::new(0.0, 45f32.to_radians(), 0.0), anchor, bob, settings);
    at(scene, &[bob], &[30, 60])
}

fn spring() -> Value {
    let mut scene = scene(100);
    let anchor = cube(&mut scene, 0.2, Vec3::ZERO, BodyKind::Passive);
    let bob = cube(&mut scene, 0.5, Vec3::new(0.0, 0.0, -1.0), BodyKind::Active);
    joint(&mut scene, JointKind::GenericSpring, Vec3::ZERO, Vec3::ZERO, anchor, bob, |c| {
        c.linear_limits = [Some([0.0, 0.0]), Some([0.0, 0.0]), None];
        c.angular_limits = [Some([0.0, 0.0]); 3];
        c.linear_springs = [None, None, Some(10.0)];
    });
    at(scene, &[bob], &[25, 50, 100])
}

fn ground(scene: &mut Scene) {
    add(scene, Primitive::Plane { size: 20.0 }, Vec3::ZERO, body(BodyKind::Passive, |_| {}));
}

fn fixed_pair() -> Value {
    let mut scene = scene(100);
    ground(&mut scene);
    let first = cube(&mut scene, 1.0, Vec3::new(0.0, 0.0, 3.0), BodyKind::Active);
    let second = cube(&mut scene, 1.0, Vec3::new(1.2, 0.0, 3.5), BodyKind::Active);
    joint(&mut scene, JointKind::Fixed, Vec3::new(0.6, 0.0, 3.2), Vec3::ZERO, first, second, |_| {});
    at(scene, &[first, second], &[15, 100])
}

fn breaking(threshold: f32) -> Value {
    let mut scene = scene(40);
    let anchor = cube(&mut scene, 0.2, Vec3::ZERO, BodyKind::Passive);
    let bob = cube(&mut scene, 0.5, Vec3::new(2.0, 0.0, 0.0), BodyKind::Active);
    joint(&mut scene, JointKind::Fixed, Vec3::ZERO, Vec3::ZERO, anchor, bob, |c| {
        c.breakable = true;
        c.breaking_threshold = threshold;
    });
    at(scene, &[bob], &[40])
}

fn asleep(woken: bool) -> Value {
    let mut scene = scene(80);
    ground(&mut scene);
    let sleeper = add(
        &mut scene,
        Primitive::Cube { size: 1.0 },
        Vec3::new(0.0, 0.0, 3.0),
        body(BodyKind::Active, |b| {
            b.deactivation = true;
            b.start_deactivated = true;
        }),
    );
    if woken {
        cube(&mut scene, 1.0, Vec3::new(0.2, 0.0, 6.0), BodyKind::Active);
    }
    at(scene, &[sleeper], &[60, 80])
}

fn pushed(kind: FieldKind, mass: f32, flow: Option<f32>) -> Value {
    let mut scene = scene(48);
    scene.physics.gravity = Vec3::ZERO;
    let id = add(&mut scene, Primitive::Cube { size: 1.0 }, Vec3::ZERO, body(BodyKind::Active, |b| b.mass = mass));
    match kind {
        FieldKind::Force => field(&mut scene, kind, Vec3::new(3.0, 0.0, 0.0), Vec3::ZERO, flow),
        _ => field(&mut scene, kind, Vec3::new(-3.0, 0.0, 0.0), Vec3::new(0.0, FRAC_PI_2, 0.0), flow),
    }
    at(scene, &[id], &[47])
}

fn cloth(kind: FieldKind, facing: bool, mass: f32) -> Value {
    let mut scene = scene(48);
    scene.physics.gravity = Vec3::ZERO;
    let physics = ObjectPhysics { cloth: Some(Cloth { mass, ..Cloth::default() }), ..ObjectPhysics::default() };
    let id = add(&mut scene, Primitive::Grid { x_subdivisions: 10, y_subdivisions: 10, size: 1.0 }, Vec3::ZERO, physics);
    if facing {
        scene.object_mut(id).unwrap().rotation = Vec3::new(0.0, FRAC_PI_2, 0.0);
    }
    match kind {
        FieldKind::Force => field(&mut scene, kind, Vec3::new(3.0, 0.0, 0.0), Vec3::ZERO, None),
        _ => field(&mut scene, kind, Vec3::new(-3.0, 0.0, 0.0), Vec3::new(0.0, FRAC_PI_2, 0.0), None),
    }
    let baked = kition_scene_physics::bake(&scene).unwrap();
    let world = scene.world_matrix(id).unwrap();
    let frames = &baked.cloths[&id];
    let mean = |frame: usize| {
        let points = &frames[frame - 1];
        points.iter().map(|point| world.transform_point3(*point).x).sum::<f32>() / points.len() as f32
    };
    json!({ "24": mean(24), "48": mean(48) })
}

fn ours() -> Value {
    json!({
        "hinge": pendulum(JointKind::Hinge, |_| {}),
        "point": pendulum(JointKind::Point, |_| {}),
        "hingeLimit": pendulum(JointKind::Hinge, |c| c.angle_limit = Some([-30f32.to_radians(), 30f32.to_radians()])),
        "slider": slider(|_| {}),
        "sliderLimit": slider(|c| c.distance_limit = Some([-1.0, 1.0])),
        "spring": spring(),
        "fixed": fixed_pair(),
        "weak": breaking(0.05),
        "strong": breaking(0.2),
        "asleep": asleep(false),
        "woken": asleep(true),
        "force": pushed(FieldKind::Force, 1.0, None),
        "forceHeavy": pushed(FieldKind::Force, 2.0, None),
        "wind": pushed(FieldKind::Wind, 1.0, None),
        "windStill": pushed(FieldKind::Wind, 1.0, Some(0.0)),
        "clothForce": cloth(FieldKind::Force, false, 0.3),
        "clothForceHeavy": cloth(FieldKind::Force, false, 1.0),
        "clothWindFacing": cloth(FieldKind::Wind, true, 0.3),
        "clothWindFlat": cloth(FieldKind::Wind, false, 0.3),
    })
}

fn point(value: &Value) -> Vec3 {
    let numbers: Vec<f32> = value.as_array().unwrap().iter().map(|number| number.as_f64().unwrap() as f32).collect();
    Vec3::new(numbers[0], numbers[1], numbers[2])
}

/// Compares every position of `case` (each object at each frame) within `tolerance` metres.
fn positions(reference: &Value, ours: &Value, case: &str, tolerance: f32) {
    for (frame, theirs) in reference[case].as_object().unwrap() {
        for (slot, expected) in theirs.as_array().unwrap().iter().enumerate() {
            let (expected, got) = (point(expected), point(&ours[case][frame][slot]));
            eprintln!("{case} frame {frame}: {got:.4} (reference {expected:.4})");
            assert!(got.distance(expected) <= tolerance, "{case} at frame {frame}: {got} against {expected}");
        }
    }
}

fn compare(reference: &Value, ours: &Value) {
    // Pendulums swing through the same arc at the same pace (the period matches within a frame
    // over two swings: 0.15 m along a 2 m arm).
    positions(reference, ours, "hinge", 0.15);
    positions(reference, ours, "point", 0.15);
    positions(reference, ours, "hingeLimit", 0.03);
    positions(reference, ours, "slider", 0.05);
    positions(reference, ours, "sliderLimit", 0.02);
    positions(reference, ours, "spring", 0.08);
    positions(reference, ours, "weak", 0.2);
    positions(reference, ours, "strong", 0.02);
    positions(reference, ours, "asleep", 1e-3);
    // The falling pair lands and tips as one; where it comes to rest depends on how it bounces.
    for frame in ["15", "100"] {
        let (a, b) = (point(&ours["fixed"][frame][0]), point(&ours["fixed"][frame][1]));
        assert!((a.distance(b) - 1.3).abs() < 0.02, "the pair stays joined at frame {frame}: {a} {b}");
    }
    let (theirs, mine) = (point(&reference["fixed"]["15"][0]), point(&ours["fixed"]["15"][0]));
    assert!((theirs.z - mine.z).abs() < 0.05, "the pair falls together: {mine} (reference {theirs})");
    for slot in 0..2 {
        let rest = point(&ours["fixed"]["100"][slot]);
        assert!(rest.z > 0.45 && rest.z < 1.2, "the pair rests on the ground: {rest}");
    }
    // Knocked awake, the sleeper falls to the ground (where it ends depends on the knock).
    let woken = point(&ours["woken"]["80"][0]);
    assert!((woken.z - 0.5).abs() < 0.02, "the sleeper falls when knocked: {woken}");
    // Fields on rigid bodies.
    for case in ["force", "forceHeavy", "wind", "windStill"] {
        let (theirs, mine) = (point(&reference[case]["47"][0]).x, point(&ours[case]["47"][0]).x);
        eprintln!("{case}: {mine:.4} (reference {theirs:.4})");
        assert!((mine - theirs).abs() <= 0.1 * theirs.abs() + 2e-4, "{case}: {mine} against {theirs}");
    }
    // Fields on cloth: a fifth either way (the cloth solvers differ).
    for case in ["clothForce", "clothForceHeavy", "clothWindFacing", "clothWindFlat"] {
        for frame in ["24", "48"] {
            let (theirs, mine) = (reference[case][frame].as_f64().unwrap() as f32, ours[case][frame].as_f64().unwrap() as f32);
            eprintln!("{case} frame {frame}: {mine:.5} (reference {theirs:.5})");
            assert!((mine - theirs).abs() <= 0.2 * theirs.abs() + 2e-4, "{case} at frame {frame}: {mine} against {theirs}");
        }
    }
}

#[test]
fn constraints_and_fields_match_the_recorded_reference() {
    compare(&recorded(), &ours());
}

#[test]
fn constraints_and_fields_match_the_reference_application() {
    let Some(blender) = std::env::var_os("KITION_BLENDER") else {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    };
    let output = std::env::temp_dir().join(format!("kition-scene-constraints-{}.json", std::process::id()));
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/constraints.py");
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
