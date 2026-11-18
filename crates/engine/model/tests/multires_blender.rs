//! Multiresolution levels against the reference application's Multiresolution modifier: levels 1
//! to 3 of its default cube, plane, and UV sphere before any sculpting (vertex counts and where
//! the vertices are). The statistics Blender 5.2.3 gave are checked always; when
//! `KITION_BLENDER` names a Blender executable, `tests/blender/multires.py` writes every vertex
//! there and each of ours must lie on one of its.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_scene_model::glam::Vec3;
use kition_scene_model::{Mesh, Primitive, multires};
use serde_json::{Value, json};

/// What Blender 5.2.3 gave: per shape and level, the vertex count and the mean, largest, and
/// smallest distance from the origin.
fn recorded() -> Value {
    json!({
        "cube": {
            "1": { "vertices": 26, "meanRadius": 0.8580751, "maxRadius": 0.8660256, "minRadius": 0.8395062 },
            "2": { "vertices": 98, "meanRadius": 0.8565201, "maxRadius": 0.8660256, "minRadius": 0.8395062 },
            "3": { "vertices": 386, "meanRadius": 0.8563104, "maxRadius": 0.8660256, "minRadius": 0.8395062 }
        },
        "plane": {
            "1": { "vertices": 9, "meanRadius": 0.8272522, "maxRadius": 0.9446505, "minRadius": 0.0 },
            "2": { "vertices": 25, "meanRadius": 0.7657583, "maxRadius": 0.9446505, "minRadius": 0.0 },
            "3": { "vertices": 81, "meanRadius": 0.716056, "maxRadius": 0.9446505, "minRadius": 0.0 }
        },
        "sphere": {
            "1": { "vertices": 1986, "meanRadius": 0.9904452, "maxRadius": 0.9975755, "minRadius": 0.9872265 },
            "2": { "vertices": 7938, "meanRadius": 0.9904427, "maxRadius": 0.9975755, "minRadius": 0.9872265 },
            "3": { "vertices": 31746, "meanRadius": 0.9904433, "maxRadius": 0.9975755, "minRadius": 0.9872265 }
        }
    })
}

/// How far apart our vertices and the reference's may be. On a closed mesh both sit on the exact
/// limit surface; at an open mesh's boundary corners the reference approximates the limit
/// (its Quality setting, 4 by default) and lands up to about 2 mm away on a 2 m plane.
fn tolerance(name: &str) -> f32 {
    if name == "plane" { 2.5e-3 } else { 1e-4 }
}

fn base(name: &str) -> Mesh {
    match name {
        "cube" => Primitive::Cube { size: 2.0 }.build().unwrap(),
        "plane" => Primitive::Plane { size: 2.0 }.build().unwrap(),
        _ => Primitive::UvSphere { segments: 32, rings: 16, radius: 1.0 }.build().unwrap(),
    }
}

/// Our levels 1 to 3 of `name`.
fn ours(name: &str) -> Vec<Vec<Vec3>> {
    let mut mesh = base(name);
    (1..=3)
        .map(|_| {
            multires::subdivide(&mut mesh).unwrap();
            multires::evaluate(&mesh, multires::levels(&mesh)).positions
        })
        .collect()
}

fn statistics(points: &[Vec3]) -> Value {
    let radii: Vec<f32> = points.iter().map(|point| point.length()).collect();
    json!({
        "vertices": points.len(),
        "meanRadius": radii.iter().sum::<f32>() / radii.len() as f32,
        "maxRadius": radii.iter().copied().fold(f32::MIN, f32::max),
        "minRadius": radii.iter().copied().fold(f32::MAX, f32::min),
    })
}

#[test]
fn levels_match_the_recorded_reference() {
    let reference = recorded();
    for name in ["cube", "plane", "sphere"] {
        for (index, points) in ours(name).iter().enumerate() {
            let level = (index + 1).to_string();
            let (theirs, mine) = (&reference[name][&level], statistics(points));
            eprintln!("{name} level {level}: {mine} (reference {theirs})");
            assert_eq!(mine["vertices"], theirs["vertices"], "{name} {level}");
            for key in ["meanRadius", "maxRadius", "minRadius"] {
                let (a, b) = (mine[key].as_f64().unwrap(), theirs[key].as_f64().unwrap());
                assert!((a - b).abs() < f64::from(tolerance(name)), "{name} level {level} {key}: {a} against {b}");
            }
        }
    }
}

#[test]
fn levels_match_the_reference_application() {
    let Some(blender) = std::env::var_os("KITION_BLENDER") else {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    };
    let output = std::env::temp_dir().join(format!("kition-scene-multires-{}.json", std::process::id()));
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/blender/multires.py");
    let status = std::process::Command::new(blender)
        .args(["-b", "--factory-startup", "--python", script, "--"])
        .arg(&output)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let theirs: Value = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    let _ = std::fs::remove_file(&output);
    for name in ["cube", "plane", "sphere"] {
        for (index, points) in ours(name).iter().enumerate() {
            let level = (index + 1).to_string();
            let reference: Vec<Vec3> = theirs[name][&level]
                .as_array()
                .unwrap()
                .iter()
                .map(|point| Vec3::new(point[0].as_f64().unwrap() as f32, point[1].as_f64().unwrap() as f32, point[2].as_f64().unwrap() as f32))
                .collect();
            assert_eq!(points.len(), reference.len(), "{name} level {level}");
            // Every one of our vertices lies on one of the reference's, and the other way round.
            let farthest = |from: &[Vec3], to: &[Vec3]| {
                from.iter().map(|point| to.iter().map(|other| point.distance(*other)).fold(f32::MAX, f32::min)).fold(0f32, f32::max)
            };
            let gap = farthest(points, &reference).max(farthest(&reference, points));
            eprintln!("{name} level {level}: every vertex within {gap:.2e} m of the reference's");
            assert!(gap < tolerance(name), "{name} level {level}: {gap}");
        }
    }
}
