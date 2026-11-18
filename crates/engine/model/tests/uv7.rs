//! Independent Blender application recordings; no Blender installation needed for normal tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_scene_model::{
    Mesh,
    glam::{Vec2, Vec3},
    unwrap::{self, Method},
};
use serde_json::Value;
use std::collections::BTreeSet;

fn cases() -> Vec<Value> {
    let mut cases: Vec<Value> = serde_json::from_str(include_str!("fixtures/uv7-cases.json")).unwrap();
    cases.extend(serde_json::from_str::<Vec<Value>>(include_str!("fixtures/uv7-complex-cases.json")).unwrap());
    cases
}
fn mesh(case: &Value) -> Mesh {
    Mesh {
        positions: serde_json::from_value::<Vec<[f32; 3]>>(case["positions"].clone()).unwrap().into_iter().map(Vec3::from_array).collect(),
        faces: serde_json::from_value(case["faces"].clone()).unwrap(),
        uvs: coordinates(&case["uvs"]),
        seams: serde_json::from_value(case["seams"].clone()).unwrap(),
        pinned: serde_json::from_value(case["pinned"].clone()).unwrap(),
        ..Default::default()
    }
}
fn coordinates(value: &Value) -> Vec<Vec<Vec2>> {
    serde_json::from_value::<Vec<Vec<[f32; 2]>>>(value.clone()).unwrap().into_iter().map(|f| f.into_iter().map(Vec2::from_array).collect()).collect()
}
/// Symmetric Dirichlet distortion with the best uniform UV scale factored out. Four is
/// isometric; this measures the stated area-and-angle objective independently of shelf packing.
fn energy(mesh: &Mesh) -> f64 {
    let (mut direct, mut inverse, mut area) = (0.0, 0.0, 0.0);
    for (face, uv) in mesh.faces.iter().zip(&mesh.uvs) {
        let (a, b, c) =
            (mesh.positions[face[0] as usize].as_dvec3(), mesh.positions[face[1] as usize].as_dvec3(), mesh.positions[face[2] as usize].as_dvec3());
        let length = a.distance(b);
        let twice_area = (b - a).cross(c - a).length();
        let (x, y) = ((c - a).dot(b - a) / length, twice_area / length);
        let first = (uv[1] - uv[0]).as_dvec2() / length;
        let second = ((uv[2] - uv[0]).as_dvec2() - first * x) / y;
        let det = first.perp_dot(second);
        assert!(det > 0.0, "flipped triangle");
        let norm = first.length_squared() + second.length_squared();
        direct += twice_area * norm;
        inverse += twice_area * norm / (det * det);
        area += twice_area;
    }
    2.0 * (direct * inverse).sqrt() / area
}

#[test]
fn minimum_stretch_matches_recorded_reference() {
    let mut reference: Value = serde_json::from_str(include_str!("fixtures/uv7-reference.json")).unwrap();
    let complex: Value = serde_json::from_str(include_str!("fixtures/uv7-complex-reference.json")).unwrap();
    reference["cases"].as_array_mut().unwrap().extend(complex["cases"].as_array().unwrap().clone());
    for case in cases() {
        let mut actual = mesh(&case);
        let before = actual.clone();
        let mut expected = actual.clone();
        expected.uvs = coordinates(&reference["cases"].as_array().unwrap().iter().find(|r| r["name"] == case["name"]).unwrap()["uvs"]);
        let result = unwrap::minimum_stretch(&mut actual, &BTreeSet::new(), 0.02, 50).unwrap();
        assert_eq!(result.charts, if case["name"] == "seamed" { 2 } else { 1 });
        actual.validate().unwrap();
        let (ours, theirs) = (energy(&actual), energy(&expected));
        let (angle, area) = unwrap::distortion(&actual, &BTreeSet::new());
        let (reference_angle, reference_area) = unwrap::distortion(&expected, &BTreeSet::new());
        eprintln!("{}: energy {ours:.7}/{theirs:.7}, angle {angle:.6}/{reference_angle:.6}, area {area:.6}/{reference_area:.6}", case["name"]);
        assert!(ours <= theirs * 1.03, "symmetric Dirichlet objective exceeds the reference by more than 3%");
        assert!((angle - reference_angle).abs() <= 0.03, "angle distortion differs by more than 0.03 radians");
        assert!((area - reference_area).abs() <= 0.05, "log-area distortion differs by more than 0.05");
        if !actual.pinned.is_empty() {
            let max_error = actual.uvs.iter().flatten().zip(expected.uvs.iter().flatten()).map(|(a, b)| a.distance(*b)).fold(0.0f32, f32::max);
            eprintln!("pinned max coordinate difference {max_error}");
            assert!(max_error < 0.03, "pinned chart differs by more than 0.03 UV units");
        }
        for (face, vertices) in actual.faces.iter().enumerate() {
            for (corner, vertex) in vertices.iter().enumerate() {
                if actual.pinned.contains(vertex) {
                    assert_eq!(actual.uvs[face][corner], before.uvs[face][corner]);
                }
            }
        }
    }
}

#[test]
fn minimum_stretch_is_distinct_and_more_iterations_lower_its_energy() {
    let case = cases().remove(0);
    let mut early = mesh(&case);
    let mut solved = early.clone();
    let mut conformal = early.clone();
    unwrap::minimum_stretch(&mut early, &BTreeSet::new(), 0.02, 1).unwrap();
    unwrap::minimum_stretch(&mut solved, &BTreeSet::new(), 0.02, 50).unwrap();
    unwrap::unwrap_with(&mut conformal, &BTreeSet::new(), 0.02, Method::Conformal).unwrap();
    assert!(energy(&solved) < energy(&early));
    assert!(energy(&solved) < energy(&conformal));
    assert_ne!(solved.uvs, conformal.uvs);
}

#[test]
fn live_unwrap_holds_every_pin_and_leaves_other_islands_alone() {
    let mut mesh = mesh(&cases().remove(2));
    unwrap::minimum_stretch(&mut mesh, &BTreeSet::new(), 0.02, 30).unwrap();
    mesh.pinned = vec![0, 20];
    let before = mesh.clone();
    for (face, uvs) in mesh.faces.iter().zip(&mut mesh.uvs) {
        for (vertex, uv) in face.iter().zip(uvs) {
            if *vertex == 20 {
                uv.x += 0.1;
            }
        }
    }
    let transformed = mesh.clone();
    let changed = unwrap::live_unwrap(&mut mesh, &before, &BTreeSet::from([20]), Method::MinimumStretch, 30).unwrap();
    assert_eq!(changed, 1);
    let mut relaxed = false;
    for (face, vertices) in mesh.faces.iter().enumerate() {
        for (corner, vertex) in vertices.iter().enumerate() {
            if mesh.pinned.contains(vertex) {
                assert_eq!(mesh.uvs[face][corner], transformed.uvs[face][corner]);
            }
            if vertices.iter().all(|v| v % 5 >= 2) {
                assert_eq!(mesh.uvs[face][corner], before.uvs[face][corner]);
            } else if !mesh.pinned.contains(vertex) {
                relaxed |= mesh.uvs[face][corner].distance(before.uvs[face][corner]) > 1e-4;
            }
        }
    }
    assert!(relaxed);
}

#[test]
fn hostile_inputs_fail_atomically() {
    let original = mesh(&cases().remove(0));
    for margin in [f32::NAN, f32::INFINITY, -0.1, 0.5] {
        let mut bad = original.clone();
        assert!(unwrap::minimum_stretch(&mut bad, &BTreeSet::new(), margin, 30).is_err());
        assert_eq!(bad.uvs, original.uvs);
    }
    for iterations in [0, 101, u32::MAX] {
        let mut bad = original.clone();
        assert!(unwrap::minimum_stretch(&mut bad, &BTreeSet::new(), 0.02, iterations).is_err());
        assert_eq!(bad.uvs, original.uvs);
    }
    let mut bad = original.clone();
    bad.positions[1] = bad.positions[0];
    assert!(unwrap::minimum_stretch(&mut bad, &BTreeSet::new(), 0.02, 30).is_err());
    assert_eq!(bad.uvs, original.uvs);
    let mut bad = original.clone();
    assert!(unwrap::minimum_stretch(&mut bad, &BTreeSet::from([u32::MAX]), 0.02, 30).is_err());
    assert_eq!(bad.uvs, original.uvs);
}

#[test]
fn optional_live_blender_recording_matches_committed_observations() {
    let Some(executable) = std::env::var_os("KITION_BLENDER") else { return };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    for (name, script, input, recording, extra) in [
        ("basic", "uv7.py", "uv7-cases.json", include_str!("fixtures/uv7-reference.json"), None),
        ("complex", "uv7.py", "uv7-complex-cases.json", include_str!("fixtures/uv7-complex-reference.json"), None),
        ("live", "uv7_live.py", "uv7-cases.json", include_str!("fixtures/uv7-live-reference.json"), None),
        ("onepin", "uv7_live.py", "uv7-cases.json", include_str!("fixtures/uv7-onepin-reference.json"), Some("single")),
    ] {
        let output = std::env::temp_dir().join(format!("kition-scene-uv7-{}-{name}.json", std::process::id()));
        let mut command = std::process::Command::new(&executable);
        command
            .args(["-b", "--factory-startup", "--python-exit-code", "1", "--python"])
            .arg(root.join("blender").join(script))
            .arg("--")
            .arg(root.join("fixtures").join(input))
            .arg(&output);
        if let Some(extra) = extra {
            command.arg(extra);
        }
        assert!(command.status().unwrap().success());
        let actual: Value = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        let expected: Value = serde_json::from_str(recording).unwrap();
        assert_eq!(actual["version"], expected["version"]);
        assert_eq!(actual["cases"].as_array().unwrap().len(), expected["cases"].as_array().unwrap().len());
        for (a, b) in actual["cases"].as_array().unwrap().iter().zip(expected["cases"].as_array().unwrap()) {
            assert_eq!(a["name"], b["name"]);
            for (a, b) in coordinates(&a["uvs"]).iter().flatten().zip(coordinates(&b["uvs"]).iter().flatten()) {
                assert!(a.distance(*b) < 2e-5);
            }
        }
        std::fs::remove_file(output).unwrap();
    }
}

#[test]
fn pinned_minimum_stretch_is_independent_of_model_units() {
    let case = cases().remove(3);
    let mut base = mesh(&case);
    let mut scaled = base.clone();
    for point in &mut scaled.positions {
        *point *= 100.0;
    }
    unwrap::minimum_stretch(&mut base, &BTreeSet::new(), 0.02, 50).unwrap();
    unwrap::minimum_stretch(&mut scaled, &BTreeSet::new(), 0.02, 50).unwrap();
    for (a, b) in base.uvs.iter().flatten().zip(scaled.uvs.iter().flatten()) {
        assert!(a.distance(*b) < 1e-5);
    }
}

#[test]
fn live_relaxation_matches_the_uv_editor_observation() {
    for (data, pins) in
        [(include_str!("fixtures/uv7-live-reference.json"), vec![0, 4]), (include_str!("fixtures/uv7-onepin-reference.json"), vec![4])]
    {
        let reference: Value = serde_json::from_str(data).unwrap();
        let mut mesh = mesh(&cases().remove(3));
        mesh.pinned = pins;
        mesh.uvs = coordinates(&reference["cases"][0]["uvs"]);
        let mut original = mesh.clone();
        // The disabled reference contains only the transformed pin. Recover the starting
        // chart using the exact documented operator offset for the one-pin frame.
        for (face, uvs) in original.faces.iter().zip(&mut original.uvs) {
            for (vertex, uv) in face.iter().zip(uvs) {
                if *vertex == 4 {
                    *uv -= Vec2::new(0.12, 0.07);
                }
            }
        }
        unwrap::live_unwrap(&mut mesh, &original, &BTreeSet::from([4]), Method::AngleBased, 30).unwrap();
        let expected = coordinates(&reference["cases"][1]["uvs"]);
        let maximum = mesh.uvs.iter().flatten().zip(expected.iter().flatten()).map(|(a, b)| a.distance(*b)).fold(0.0f32, f32::max);
        eprintln!("{}-pin live reference max error {maximum}", mesh.pinned.len());
        assert!(maximum < if mesh.pinned.len() == 1 { 2e-5 } else { 0.03 });
    }
}

#[test]
fn a_mixed_pinned_and_free_layout_does_not_pack_over_the_pins() {
    let mut mesh = mesh(&cases().remove(2));
    mesh.pinned = vec![0, 20];
    let before = mesh.clone();
    unwrap::minimum_stretch(&mut mesh, &BTreeSet::new(), 0.02, 30).unwrap();
    let mut fixed = Vec::new();
    let mut free = Vec::new();
    for (face, vertices) in mesh.faces.iter().enumerate() {
        for (corner, vertex) in vertices.iter().enumerate() {
            if mesh.pinned.contains(vertex) {
                assert_eq!(mesh.uvs[face][corner], before.uvs[face][corner]);
            }
            if vertices.iter().all(|v| v % 5 >= 2) {
                free.push(mesh.uvs[face][corner]);
            } else {
                fixed.push(mesh.uvs[face][corner]);
            }
        }
    }
    let bounds =
        |uvs: &[Vec2]| uvs.iter().fold((Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)), |(low, high), uv| (low.min(*uv), high.max(*uv)));
    let (a, b) = bounds(&fixed);
    let (c, d) = bounds(&free);
    assert!(b.x < c.x || d.x < a.x || b.y < c.y || d.y < a.y);
}

#[test]
fn a_later_degenerate_chart_cannot_partially_write_an_earlier_chart() {
    let mut mesh = mesh(&cases().remove(0));
    let base = mesh.positions.len() as u32;
    mesh.positions.extend([Vec3::ZERO, Vec3::X, Vec3::X * 2.0]);
    mesh.faces.push(vec![base, base + 1, base + 2]);
    mesh.uvs.push(vec![Vec2::ZERO, Vec2::X, Vec2::Y]);
    let before = mesh.uvs.clone();
    assert!(unwrap::minimum_stretch(&mut mesh, &BTreeSet::new(), 0.02, 30).is_err());
    assert_eq!(mesh.uvs, before);
}

#[test]
fn closed_and_excessive_islands_are_explicit_errors() {
    let mut closed = kition_scene_model::Primitive::Cube { size: 2.0 }.build().unwrap();
    let before = closed.uvs.clone();
    assert!(unwrap::minimum_stretch(&mut closed, &BTreeSet::new(), 0.02, 30).is_err());
    assert_eq!(closed.uvs, before);
    let mut many = Mesh::default();
    for i in 0..129 {
        let base = many.positions.len() as u32;
        let offset = Vec3::X * i as f32 * 2.0;
        many.positions.extend([offset, offset + Vec3::X, offset + Vec3::Y]);
        many.faces.push(vec![base, base + 1, base + 2]);
        many.uvs.push(vec![Vec2::ZERO, Vec2::X, Vec2::Y]);
    }
    let before = many.uvs.clone();
    assert!(unwrap::minimum_stretch(&mut many, &BTreeSet::new(), 0.02, 30).unwrap_err().to_string().contains("128"));
    assert_eq!(many.uvs, before);
}

#[test]
fn one_pinned_minimum_stretch_keeps_its_uv_scale_when_model_units_change() {
    let mut base = mesh(&cases().remove(0));
    base.pinned = vec![0];
    let mut scaled = base.clone();
    for point in &mut scaled.positions {
        *point *= 100.0;
    }
    unwrap::minimum_stretch(&mut base, &BTreeSet::new(), 0.02, 50).unwrap();
    unwrap::minimum_stretch(&mut scaled, &BTreeSet::new(), 0.02, 50).unwrap();
    for (a, b) in base.uvs.iter().flatten().zip(scaled.uvs.iter().flatten()) {
        assert!(a.distance(*b) < 1e-5);
    }
}

#[test]
fn live_unwrap_respects_existing_uv_islands_after_mesh_seams_are_cleared() {
    let mut mesh = mesh(&cases().remove(2));
    unwrap::minimum_stretch(&mut mesh, &BTreeSet::new(), 0.02, 30).unwrap();
    mesh.seams.clear();
    mesh.pinned = vec![0, 20];
    let before = mesh.clone();
    for (face, uvs) in mesh.faces.iter().zip(&mut mesh.uvs) {
        for (vertex, uv) in face.iter().zip(uvs) {
            if *vertex == 20 {
                uv.x += 0.1;
            }
        }
    }
    assert_eq!(unwrap::live_unwrap(&mut mesh, &before, &BTreeSet::from([20]), Method::MinimumStretch, 30).unwrap(), 1);
    assert!(mesh.seams.is_empty());
    for (face, vertices) in mesh.faces.iter().enumerate() {
        if vertices.iter().all(|v| v % 5 >= 2) {
            assert_eq!(mesh.uvs[face], before.uvs[face]);
        }
    }
}
