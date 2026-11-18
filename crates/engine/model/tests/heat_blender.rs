//! Automatic (bone heat) weights against the reference application's on the same meshes and
//! bones, compared by correlation per bone. Runs when `KITION_BLENDER` names a Blender
//! executable; `tests/blender/heat.py` builds the meshes there and writes what it made.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_scene_model::glam::{Mat4, Vec3};
use kition_scene_model::{Mesh, heat};
use serde_json::Value;

fn correlation(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len() as f32;
    let (ma, mb) = (a.iter().sum::<f32>() / n, b.iter().sum::<f32>() / n);
    let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        cov += (x - ma) * (y - mb);
        va += (x - ma) * (x - ma);
        vb += (y - mb) * (y - mb);
    }
    cov / (va * vb).sqrt().max(1e-12)
}

fn vec3(value: &Value) -> Vec3 {
    let parts: Vec<f32> = value.as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
    Vec3::new(parts[0], parts[1], parts[2])
}

#[test]
fn heat_weights_follow_the_reference() {
    let Some(blender) = std::env::var_os("KITION_BLENDER") else {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    };
    let output = std::env::temp_dir().join(format!("kition-scene-heat-{}.json", std::process::id()));
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/blender/heat.py");
    let status = std::process::Command::new(blender)
        .args(["-b", "--factory-startup", "--python", script, "--"])
        .arg(&output)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let cases: Value = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    let _ = std::fs::remove_file(&output);
    for (name, case) in cases.as_object().unwrap() {
        let mesh = Mesh {
            positions: case["positions"].as_array().unwrap().iter().map(vec3).collect(),
            faces: case["faces"]
                .as_array()
                .unwrap()
                .iter()
                .map(|face| face.as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect())
                .collect(),
            ..Default::default()
        };
        let bones: Vec<(String, Vec3, Vec3)> =
            case["bones"].as_array().unwrap().iter().map(|bone| (bone[0].as_str().unwrap().to_string(), vec3(&bone[1]), vec3(&bone[2]))).collect();
        let segments: Vec<(Vec3, Vec3)> = bones.iter().map(|(_, head, tail)| (*head, *tail)).collect();
        let ours = heat::weights(&mesh, Mat4::IDENTITY, &segments).unwrap();
        for ((bone, _, _), weights) in bones.iter().zip(&ours) {
            let theirs: Vec<f32> = case["weights"][bone].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
            let r = correlation(weights, &theirs);
            let mean_difference = weights.iter().zip(&theirs).map(|(a, b)| (a - b).abs()).sum::<f32>() / theirs.len() as f32;
            eprintln!("{name} {bone}: correlation {r:.4}, mean difference {mean_difference:.4}");
            assert!(r > 0.999, "{name} {bone}: correlation {r}");
        }
    }
}
