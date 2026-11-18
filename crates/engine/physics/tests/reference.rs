//! The four comparison scenes against the reference application's own simulation: a cube dropped
//! on a plane, a stack of boxes, a line of dominoes, and a cloth draped over a sphere. The
//! reference numbers below were measured with its rigid body world and cloth at their defaults
//! (`KITION_BLENDER` naming its executable runs it again and prints both side by side).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::process::Command;

use kition_scene_model::glam::{Mat4, Vec3};
use kition_scene_model::physics::{BodyKind, Cloth, Collision, ObjectPhysics, RigidBody};
use kition_scene_model::{Id, ObjectData, Primitive, Scene};
use serde_json::Value;

fn add(scene: &mut Scene, name: &str, primitive: Primitive, location: Vec3, rotation: Vec3, scale: Vec3, physics: ObjectPhysics) -> Id {
    let id = scene.add_object(name, ObjectData::Mesh(primitive.build().unwrap()));
    let object = scene.object_mut(id).unwrap();
    object.location = location;
    object.rotation = rotation;
    object.scale = scale;
    object.physics = Some(physics);
    id
}

fn rigid(kind: BodyKind) -> ObjectPhysics {
    ObjectPhysics { rigid_body: Some(RigidBody { kind, ..RigidBody::default() }), ..ObjectPhysics::default() }
}

fn ground(scene: &mut Scene, size: f32) {
    add(scene, "Ground", Primitive::Plane { size }, Vec3::ZERO, Vec3::ZERO, Vec3::ONE, rigid(BodyKind::Passive));
}

/// The world matrices of `ids` at every frame of the bake.
fn baked_worlds(mut scene: Scene, ids: &[Id]) -> Vec<Vec<Mat4>> {
    let baked = kition_scene_physics::bake(&scene).unwrap();
    let (start, frames) = (baked.start, baked.frames);
    scene.physics_cache.baked = Some(std::sync::Arc::new(baked));
    assert!(scene.refresh_physics());
    (0..frames).map(|frame| ids.iter().map(|id| scene.at_frame((start + frame as i32) as f32).world_matrix(*id).unwrap()).collect()).collect()
}

/// The frame (1-based) from which a series stays within `tolerance` of its last value.
fn settle_frame(values: &[f32], tolerance: f32) -> usize {
    let last = *values.last().unwrap();
    values.iter().rposition(|value| (value - last).abs() >= tolerance).map_or(1, |index| index + 2)
}

/// How far a piece leans over towards +X, in degrees (its up axis against the world's).
fn lean(world: &Mat4) -> f32 {
    let up = world.transform_vector3(Vec3::Z).normalize();
    up.x.atan2(up.z).to_degrees()
}

struct Drop {
    rest: f32,
    settle: usize,
    lowest: f32,
}

fn drop() -> Drop {
    let mut scene = Scene::empty();
    scene.physics.frame_end = 100;
    ground(&mut scene, 20.0);
    let cube = add(&mut scene, "Cube", Primitive::Cube { size: 2.0 }, Vec3::new(0.0, 0.0, 4.0), Vec3::ZERO, Vec3::ONE, rigid(BodyKind::Active));
    let heights: Vec<f32> = baked_worlds(scene, &[cube]).iter().map(|frame| frame[0].w_axis.z).collect();
    Drop { rest: *heights.last().unwrap(), settle: settle_frame(&heights, 0.001), lowest: heights.iter().copied().fold(f32::INFINITY, f32::min) }
}

fn stack() -> Vec<Vec3> {
    let mut scene = Scene::empty();
    scene.physics.frame_end = 120;
    ground(&mut scene, 20.0);
    let boxes: Vec<Id> = (0..5)
        .map(|level| {
            let location = Vec3::new(0.0, 0.0, 0.5 + level as f32 * 1.01);
            add(&mut scene, "Box", Primitive::Cube { size: 1.0 }, location, Vec3::ZERO, Vec3::ONE, rigid(BodyKind::Active))
        })
        .collect();
    baked_worlds(scene, &boxes).last().unwrap().iter().map(|world| world.w_axis.truncate()).collect()
}

fn domino() -> Vec<(f32, f32)> {
    let mut scene = Scene::empty();
    scene.physics.frame_end = 120;
    ground(&mut scene, 40.0);
    let pieces: Vec<Id> = (0..8)
        .map(|index| {
            let tilt = if index == 0 { 15f32.to_radians() } else { 0.0 };
            let location = Vec3::new(index as f32, 0.0, 1.0);
            add(
                &mut scene,
                "Domino",
                Primitive::Cube { size: 1.0 },
                location,
                Vec3::new(0.0, tilt, 0.0),
                Vec3::new(0.2, 1.0, 2.0),
                rigid(BodyKind::Active),
            )
        })
        .collect();
    baked_worlds(scene, &pieces).last().unwrap().iter().map(|world| (lean(world), world.w_axis.x)).collect()
}

struct Drape {
    low: Vec3,
    high: Vec3,
    mean: f32,
    count: usize,
}

fn cloth() -> Drape {
    let mut scene = Scene::empty();
    scene.physics.frame_end = 60;
    let collider = ObjectPhysics { collision: Some(Collision::default()), ..ObjectPhysics::default() };
    let sphere = Primitive::UvSphere { segments: 32, rings: 16, radius: 1.0 };
    add(&mut scene, "Sphere", sphere, Vec3::ZERO, Vec3::ZERO, Vec3::ONE, collider);
    let sheet = ObjectPhysics { cloth: Some(Cloth::default()), ..ObjectPhysics::default() };
    let grid = Primitive::Grid { x_subdivisions: 20, y_subdivisions: 20, size: 3.0 };
    let id = add(&mut scene, "Cloth", grid, Vec3::new(0.0, 0.0, 1.5), Vec3::ZERO, Vec3::ONE, sheet);
    let start: Vec<Vec3> = scene.object(id).unwrap().mesh().unwrap().positions.iter().map(|point| *point + Vec3::new(0.0, 0.0, 1.5)).collect();
    let baked = kition_scene_physics::bake(&scene).unwrap();
    let end: Vec<Vec3> = baked.cloths[&id].last().unwrap().iter().map(|point| *point + Vec3::new(0.0, 0.0, 1.5)).collect();
    let low = end.iter().copied().fold(Vec3::splat(f32::INFINITY), Vec3::min);
    let high = end.iter().copied().fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
    let mean = start.iter().zip(&end).map(|(a, b)| a.distance(*b)).sum::<f32>() / end.len() as f32;
    Drape { low, high, mean, count: end.len() }
}

#[test]
fn a_dropped_cube_rests_where_and_when_the_reference_does() {
    // Reference: rests at z 1.0001, settles (within 1 mm) from frame 27, lowest 0.989.
    let ours = drop();
    println!("drop: rest {:.4} (ref 1.0001), settled from frame {} (ref 27), lowest {:.4} (ref 0.989)", ours.rest, ours.settle, ours.lowest);
    assert!((ours.rest - 1.0).abs() < 0.02, "rest height {}", ours.rest);
    assert!((20..=40).contains(&ours.settle), "settles from frame {}", ours.settle);
    assert!(ours.lowest > 0.95);
}

#[test]
fn a_stack_of_five_boxes_stands() {
    // Reference: every box within 2 mm of its column, at 0.5, 1.5, ... 4.5 (+0.5 mm).
    let ours = stack();
    for (level, centre) in ours.iter().enumerate() {
        println!("stack: box {level} at {centre:.4} (ref z {:.4})", 0.5 + level as f32);
        assert!(centre.truncate().length() < 0.02, "box {level} drifted to {centre}");
        assert!((centre.z - (0.5 + level as f32)).abs() < 0.03, "box {level} at {centre}");
    }
}

#[test]
fn a_line_of_dominoes_falls() {
    // Reference leans (degrees towards +X) at frame 120: 90, 77.5, 77.5, 78.9, 78.8, 78.6, 77.7,
    // 90 (the first lies flat, the middle ones rest on their neighbours, the last lies flat).
    let reference = [90.0, 77.5, 77.5, 78.9, 78.8, 78.6, 77.7, 90.0];
    let ours = domino();
    for ((index, (lean, x)), expected) in ours.iter().enumerate().zip(reference) {
        println!("domino {index}: lean {lean:.1}° (ref {expected}°), x {x:.2}");
    }
    for (index, (lean, _)) in ours.iter().enumerate().skip(1).take(6) {
        assert!((60.0..=85.0).contains(lean), "domino {index} leans {lean}° on its neighbour");
    }
    let last = ours.last().unwrap().0;
    assert!(last > 80.0, "the last one falls flat: {last}°");
}

#[test]
fn a_cloth_drapes_over_a_sphere_like_the_reference() {
    // Reference at frame 60: bounds (-1.284, -1.276, -0.467) to (1.270, 1.281, 1.034), mean
    // displacement 1.116 m, 441 vertices.
    let ours = cloth();
    println!(
        "cloth: bounds {:.3} to {:.3} (ref -1.28,-1.28,-0.47 to 1.27,1.28,1.03), mean displacement {:.4} (ref 1.1158), {} vertices",
        ours.low, ours.high, ours.mean, ours.count
    );
    assert_eq!(ours.count, 441);
    assert!((ours.high.z - 1.034).abs() < 0.03, "the top rests on the sphere: {}", ours.high.z);
    assert!((ours.low.z - -0.467).abs() < 0.25, "the hem hangs to {}", ours.low.z);
    assert!((ours.high.x - 1.27).abs() < 0.2 && (ours.low.x + 1.28).abs() < 0.2, "the drape's width {} to {}", ours.low.x, ours.high.x);
    assert!((ours.mean - 1.116).abs() < 0.15, "mean displacement {}", ours.mean);
}

const SCRIPT: &str = include_str!("reference.py");

/// Runs the reference again (with `KITION_BLENDER`) and prints its numbers.
#[test]
fn the_reference_numbers_hold() {
    let Some(executable) = std::env::var_os("KITION_BLENDER") else {
        eprintln!("skipped: set KITION_BLENDER to the reference modeller's executable");
        return;
    };
    let dir = std::env::temp_dir().join(format!("kition-scene-physics-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("physics.py");
    std::fs::write(&script, SCRIPT).unwrap();
    let mut results = serde_json::Map::new();
    for which in ["drop", "stack", "domino", "cloth"] {
        let out = dir.join(format!("{which}.json"));
        let status = Command::new(&executable)
            .args(["-b", "--factory-startup", "--python"])
            .arg(&script)
            .arg("--")
            .arg(which)
            .arg(&out)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        let value: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
        results.insert(which.into(), value);
    }
    let heights: Vec<f32> = results["drop"].as_array().unwrap().iter().map(|point| point[2].as_f64().unwrap() as f32).collect();
    let rest = *heights.last().unwrap();
    println!("reference drop: rest {rest:.4}, settled from frame {}", settle_frame(&heights, 0.001));
    assert!((rest - 1.0).abs() < 0.01);
    let _ = std::fs::remove_dir_all(&dir);
}
