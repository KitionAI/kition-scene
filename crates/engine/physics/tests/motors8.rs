//! Clean-room motor trajectories recorded by tests/blender/motors8.py in Blender 5.2.3.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use kition_scene_model::glam::{EulerRot, Quat, Vec3};
use kition_scene_model::physics::{BodyKind, CollisionShape, JointKind, ObjectPhysics, RigidBody, RigidConstraint};
use kition_scene_model::{ObjectData, Primitive, Scene};
use serde_json::Value;

fn recording() -> Value {
    serde_json::from_str(include_str!("fixtures/motors8-reference.json")).unwrap()
}
fn vector(value: &Value, fallback: Vec3) -> Vec3 {
    value.as_array().map_or(fallback, |a| Vec3::new(a[0].as_f64().unwrap() as f32, a[1].as_f64().unwrap() as f32, a[2].as_f64().unwrap() as f32))
}
fn number(value: &Value, fallback: f32) -> f32 {
    value.as_f64().map_or(fallback, |v| v as f32)
}
fn scene(case: &Value) -> (Scene, [kition_scene_model::Id; 2]) {
    let options = &case["options"];
    let settings = &options["settings"];
    let mut scene = Scene::empty();
    scene.physics.gravity = vector(&options["gravity"], Vec3::ZERO);
    scene.physics.frame_end = options["end"].as_i64().unwrap_or(25) as i32;
    scene.physics.substeps = options["substeps"].as_u64().unwrap_or(10) as u32;
    scene.physics.iterations = options["iterations"].as_u64().unwrap_or(10) as u32;
    let mut ids = Vec::new();
    for (index, location) in [Vec3::new(-2.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0)].into_iter().enumerate() {
        let id = scene.add_object("Body", ObjectData::Mesh(Primitive::Cube { size: 1.0 }.build().unwrap()));
        let object = scene.object_mut(id).unwrap();
        object.location = location;
        object.rotation = vector(&options["bodyRotation"], Vec3::ZERO);
        object.physics = Some(ObjectPhysics {
            rigid_body: Some(RigidBody {
                kind: if index == 0 && options["twoBodies"] != true { BodyKind::Passive } else { BodyKind::Active },
                mass: number(&options[if index == 0 { "massA" } else { "massB" }], 1.0),
                shape: CollisionShape::Box,
                linear_damping: 0.0,
                angular_damping: 0.0,
                deactivation: index == 1 && options["deactivation"] == true,
                start_deactivated: index == 1 && options["startDeactivated"] == true,
                ..RigidBody::default()
            }),
            ..ObjectPhysics::default()
        });
        ids.push(id);
    }
    let id = scene.add_object("Motor", ObjectData::Empty { size: 1.0 });
    let motor = scene.object_mut(id).unwrap();
    motor.location = vector(&options["pivot"], Vec3::ZERO);
    motor.rotation = vector(&options["rotation"], Vec3::ZERO);
    motor.physics = Some(ObjectPhysics {
        rigid_constraint: Some(RigidConstraint {
            kind: JointKind::Motor,
            object1: Some(ids[0]),
            object2: Some(ids[1]),
            linear_motor: settings["use_motor_lin"] == true,
            angular_motor: settings["use_motor_ang"] == true,
            linear_target_velocity: number(&settings["motor_lin_target_velocity"], 1.0),
            angular_target_velocity: number(&settings["motor_ang_target_velocity"], 1.0),
            linear_max_impulse: number(&settings["motor_lin_max_impulse"], 1.0),
            angular_max_impulse: number(&settings["motor_ang_max_impulse"], 1.0),
            enabled: settings["enabled"] != false,
            breakable: settings["use_breaking"] == true,
            breaking_threshold: number(&settings["breaking_threshold"], 10.0),
            ..RigidConstraint::default()
        }),
        ..ObjectPhysics::default()
    });
    (scene, [ids[0], ids[1]])
}

#[test]
fn motor_defaults_match_the_recorded_application() {
    let defaults = &recording()["cases"]["linear"]["defaults"];
    let settings = RigidConstraint::default();
    assert_eq!(settings.linear_motor, defaults["use_motor_lin"].as_bool().unwrap());
    assert_eq!(settings.angular_motor, defaults["use_motor_ang"].as_bool().unwrap());
    assert_eq!(settings.linear_target_velocity, number(&defaults["motor_lin_target_velocity"], 0.0));
    assert_eq!(settings.angular_target_velocity, number(&defaults["motor_ang_target_velocity"], 0.0));
    assert_eq!(settings.linear_max_impulse, number(&defaults["motor_lin_max_impulse"], 0.0));
    assert_eq!(settings.angular_max_impulse, number(&defaults["motor_ang_max_impulse"], 0.0));
}

#[test]
fn motor_bakes_follow_blender_positions_and_rotations() {
    let reference = recording();
    let mut maximum_position_error = 0.0f32;
    let mut maximum_angle_error = 0.0f32;
    for (name, case) in reference["cases"].as_object().unwrap() {
        let (mut scene, ids) = scene(case);
        scene.physics_cache.baked = Some(std::sync::Arc::new(kition_scene_physics::bake(&scene).unwrap()));
        assert!(scene.refresh_physics());
        for (frame, bodies) in case["frames"].as_object().unwrap() {
            let shown = scene.at_frame(frame.parse().unwrap());
            for (index, id) in ids.iter().enumerate() {
                let (_, rotation, position) = shown.world_matrix(*id).unwrap().to_scale_rotation_translation();
                let expected = &bodies[index];
                let location = vector(&expected["position"], Vec3::ZERO);
                let q = expected["rotation"].as_array().unwrap();
                let target = Quat::from_xyzw(number(&q[1], 0.0), number(&q[2], 0.0), number(&q[3], 0.0), number(&q[0], 1.0));
                maximum_position_error = maximum_position_error.max((position - location).length());
                maximum_angle_error = maximum_angle_error.max(rotation.angle_between(target).abs());
                assert!((position - location).length() < 0.002, "{name} frame {frame} body {index}: {position} != {location}");
                assert!(
                    rotation.angle_between(target).abs() < 0.003,
                    "{name} frame {frame} body {index}: {:?} != {:?}",
                    rotation.to_euler(EulerRot::XYZ),
                    target.to_euler(EulerRot::XYZ)
                );
            }
        }
    }
    eprintln!("motor reference maximum position error: {maximum_position_error} m; angle error: {maximum_angle_error} rad");
}

#[test]
fn optional_live_blender_matches_committed_motor_observations() {
    let Some(blender) = std::env::var_os("KITION_BLENDER") else { return };
    let output = std::env::temp_dir().join(format!("kition-motors8-{}.json", std::process::id()));
    let status = std::process::Command::new(blender)
        .args(["-b", "--factory-startup", "--python"])
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/blender/motors8.py"))
        .arg("--")
        .arg(&output)
        .status()
        .unwrap();
    assert!(status.success());
    let live: Value = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    std::fs::remove_file(output).unwrap();
    fn compare(actual: &Value, expected: &Value) {
        match (actual, expected) {
            (Value::Number(a), Value::Number(b)) => assert!((a.as_f64().unwrap() - b.as_f64().unwrap()).abs() < 1e-5),
            (Value::Array(a), Value::Array(b)) => {
                assert_eq!(a.len(), b.len());
                for (x, y) in a.iter().zip(b) {
                    compare(x, y);
                }
            }
            (Value::Object(a), Value::Object(b)) => {
                assert_eq!(a.len(), b.len());
                for (key, value) in b {
                    compare(&a[key], value);
                }
            }
            _ => assert_eq!(actual, expected),
        }
    }
    compare(&live, &recording());
}

#[test]
fn accumulated_impulse_caps_do_not_multiply_with_iterations_and_conserve_momentum() {
    use kition_scene_physics::{
        joint::Joint,
        rigid::{Body, Motion, World},
        shape::{Poly, Shape},
    };
    for iterations in [1, 10, 100] {
        let mut world = World::new(Vec3::ZERO, iterations);
        for (mass, x) in [(2.0, -2.0), (3.0, 2.0)] {
            let shape = Shape::Poly(Poly::cuboid(Vec3::ZERO, Vec3::splat(0.5)).unwrap());
            let mut body = Body::new(shape, Motion::Dynamic, Vec3::new(x, 0.0, 0.0), Quat::IDENTITY, mass);
            body.linear_damping = 0.0;
            body.angular_damping = 0.0;
            world.bodies.push(body);
        }
        let settings = RigidConstraint {
            kind: JointKind::Motor,
            linear_motor: true,
            angular_motor: true,
            linear_target_velocity: 10.0,
            angular_target_velocity: -10.0,
            linear_max_impulse: 2.0,
            angular_max_impulse: 3.0,
            ..RigidConstraint::default()
        };
        world.joints.push(Joint::new(&world, 0, 1, Vec3::ZERO, Quat::IDENTITY, settings).unwrap());
        world.step(0.01);
        let (a, b) = (&world.bodies[0], &world.bodies[1]);
        assert!((a.velocity.x - 0.01).abs() < 1e-6);
        assert!((b.velocity.x + 0.02 / 3.0).abs() < 1e-6);
        assert!((a.velocity * 2.0 + b.velocity * 3.0).length() < 1e-6);
        assert!((a.spin.x + 0.09).abs() < 1e-6);
        assert!((b.spin.x - 0.06).abs() < 1e-6);
        assert!((a.spin * (2.0 / 6.0) + b.spin * (3.0 / 6.0)).length() < 1e-6);
    }
}

#[test]
fn motor_model_rejects_nonfinite_or_unbounded_settings() {
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.0e30] {
        for angular in [false, true] {
            let mut motor = RigidConstraint::default();
            if angular {
                motor.angular_target_velocity = value;
            } else {
                motor.linear_target_velocity = value;
            }
            assert!(motor.check().is_err());
            let mut motor = RigidConstraint::default();
            if angular {
                motor.angular_max_impulse = value;
            } else {
                motor.linear_max_impulse = value;
            }
            assert!(motor.check().is_err());
        }
    }
}
