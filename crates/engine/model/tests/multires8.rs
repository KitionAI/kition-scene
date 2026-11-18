//! Clean-room recovery against recorded Blender output, with adversarial topology and data.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use kition_scene_model::{
    Mesh, Primitive,
    glam::{Quat, Vec2, Vec3},
    modifiers::catmull_clark,
    multires,
};
use serde_json::Value;

fn recorded() -> Value {
    serde_json::from_str(include_str!("fixtures/multires8-reference.json")).unwrap()
}
fn mesh(value: &Value) -> Mesh {
    serde_json::from_value(value.clone()).unwrap()
}
fn cube(depth: u32) -> Mesh {
    let mut mesh = Primitive::Cube { size: 2.0 }.build().unwrap();
    for _ in 0..depth {
        mesh = catmull_clark(&mesh);
    }
    mesh
}
fn same_surface(actual: &Mesh, expected: &Mesh, tolerance: f32) {
    assert_eq!(actual.positions.len(), expected.positions.len());
    assert_eq!(actual.faces.len(), expected.faces.len());
    let mut mapping = Vec::new();
    let mut used = std::collections::BTreeSet::new();
    for point in &actual.positions {
        let (at, distance) =
            expected.positions.iter().enumerate().map(|(i, other)| (i, point.distance(*other))).min_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        assert!(distance < tolerance, "distance {distance} >= {tolerance}");
        assert!(used.insert(at), "vertex mapping must be one-to-one");
        mapping.push(at as u32);
    }
    let cycle = |face: &[u32]| {
        let start = face.iter().enumerate().min_by_key(|(_, v)| *v).unwrap().0;
        face.iter().cycle().skip(start).take(face.len()).copied().collect::<Vec<_>>()
    };
    let expected: std::collections::BTreeSet<_> = expected.faces.iter().map(|f| cycle(f)).collect();
    for face in &actual.faces {
        let mapped: Vec<_> = face.iter().map(|v| mapping[*v as usize]).collect();
        assert!(expected.contains(&cycle(&mapped)), "oriented face topology changed");
    }
}

#[test]
fn recovery_matches_blender_recordings_offline_and_optionally_live() {
    let reference = recorded();
    if let Some(blender) = std::env::var_os("KITION_BLENDER") {
        let output = std::env::temp_dir().join(format!("scene-multires8-{}.json", std::process::id()));
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/blender/multires8.py");
        let status =
            std::process::Command::new(blender).args(["-b", "--factory-startup", "--python"]).arg(script).arg("--").arg(&output).status().unwrap();
        assert!(status.success());
        let live: Value = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        assert_eq!(live, reference);
        std::fs::remove_file(output).unwrap();
    }
    for case in reference["cases"].as_array().unwrap() {
        let mut input = mesh(&case["input"]);
        for _ in 0..case["existing"].as_u64().unwrap() {
            multires::subdivide(&mut input).unwrap();
        }
        let original = input.clone();
        let highest = multires::evaluate(&input, multires::levels(&input));
        let result = if case["operation"] == "rebuild" { multires::rebuild_subdivisions(&mut input) } else { multires::unsubdivide(&mut input) };
        if case["status"] == serde_json::json!(["CANCELLED"]) {
            assert!(result.is_err(), "{}", case["name"]);
            assert_eq!(input, original);
        } else {
            assert_eq!(result.unwrap(), case["total"].as_u64().unwrap() as u32, "{}", case["name"]);
            same_surface(&input, &mesh(&case["base"]), 2e-6);
            same_surface(&multires::evaluate(&input, multires::levels(&input)), &highest, 2e-6);
            if case["existing"] == 0 {
                same_surface(&multires::evaluate(&input, multires::levels(&input)), &mesh(case["levels"].as_array().unwrap().last().unwrap()), 2e-6);
            }
        }
    }
}

fn permute(mesh: &Mesh) -> Mesh {
    let count = mesh.positions.len();
    let map = |v: u32| (count - 1 - v as usize) as u32;
    let mut out = mesh.clone();
    out.positions.reverse();
    out.faces.reverse();
    out.uvs.reverse();
    for (index, face) in out.faces.iter_mut().enumerate() {
        for v in face.iter_mut() {
            *v = map(*v);
        }
        face.rotate_left(index % 4);
        if let Some(uv) = out.uvs.get_mut(index) {
            uv.rotate_left(index % 4);
        }
    }
    out
}

#[test]
fn numbering_rotation_boundaries_and_irregular_valence_do_not_change_recovery() {
    for base in [Primitive::Cube { size: 2.0 }.build().unwrap(), Primitive::Grid { x_subdivisions: 4, y_subdivisions: 6, size: 3.0 }.build().unwrap()]
    {
        let mut fine = catmull_clark(&catmull_clark(&base));
        let rotation = Quat::from_euler(kition_scene_model::glam::EulerRot::XYZ, 0.4, -0.6, 1.3);
        for p in &mut fine.positions {
            p.z += 0.03 * (p.x * 2.7 + p.y).sin();
            *p = rotation * *p + Vec3::new(3.0, -2.0, 4.0);
        }
        let mut shuffled = permute(&fine);
        assert_eq!(multires::unsubdivide(&mut shuffled).unwrap(), 1);
        same_surface(&multires::evaluate(&shuffled, 1), &fine, 2e-6);
        assert_eq!(multires::unsubdivide(&mut shuffled).unwrap(), 2);
        same_surface(&multires::evaluate(&shuffled, 2), &fine, 3e-6);
        assert_eq!(shuffled.positions.len(), base.positions.len());
    }
}

#[test]
fn existing_sculpt_detail_at_each_level_is_preserved_and_still_sculpts() {
    let mut input = cube(1);
    for at in 1..=2 {
        multires::subdivide(&mut input).unwrap();
        let level = multires::level(&input, at).unwrap();
        let positions: Vec<_> =
            level.mesh.positions.iter().enumerate().map(|(i, p)| *p + Vec3::new(0.05 * (i as f32).sin(), 0.02, 0.08 * (i as f32).cos())).collect();
        multires::store(&mut input, at, &level, &positions);
    }
    let before: Vec<_> = (0..=2).map(|at| multires::evaluate(&input, at)).collect();
    assert_eq!(multires::unsubdivide(&mut input).unwrap(), 3);
    for (i, old) in before.iter().enumerate() {
        same_surface(&multires::evaluate(&input, i as u32 + 1), old, 3e-6);
    }
    let level = multires::level(&input, 3).unwrap();
    let positions: Vec<_> = level.mesh.positions.iter().map(|p| *p + Vec3::Z * 0.1).collect();
    multires::store(&mut input, 3, &level, &positions);
    for (a, b) in multires::evaluate(&input, 3).positions.iter().zip(positions) {
        assert!(a.distance(b) < 2e-6);
    }
    let before_apply = multires::evaluate(&input, 3);
    multires::apply_base(&mut input).unwrap();
    same_surface(&multires::evaluate(&input, 3), &before_apply, 3e-6);
    assert_eq!(multires::levels(&input), 3);
    multires::delete_higher(&mut input, 1);
    assert_eq!(multires::levels(&input), 1);
}

#[test]
fn disconnected_components_stop_at_their_common_recoverable_depth() {
    let mut first = cube(2);
    let mut second = cube(1);
    for p in &mut second.positions {
        p.x += 5.0;
    }
    first.append(&second);
    let original = first.clone();
    assert_eq!(multires::rebuild_subdivisions(&mut first).unwrap(), 1);
    same_surface(&multires::evaluate(&first, 1), &original, 2e-6);
    let before = first.clone();
    assert!(multires::unsubdivide(&mut first).is_err());
    assert_eq!(first, before);
}

#[test]
fn unsupported_topology_and_attributes_are_atomic_and_uvs_are_retained() {
    let valid = cube(1);
    let mut recovered = valid.clone();
    multires::unsubdivide(&mut recovered).unwrap();
    let restored = multires::evaluate(&recovered, 1);
    for (face, uvs) in restored.faces.iter().zip(&restored.uvs) {
        let mapped: Vec<_> =
            face.iter().map(|v| valid.positions.iter().position(|p| p.distance(restored.positions[*v as usize]) < 2e-6).unwrap() as u32).collect();
        let index = valid.faces.iter().position(|f| mapped.iter().all(|v| f.contains(v))).unwrap();
        for (v, uv) in mapped.iter().zip(uvs) {
            let corner = valid.faces[index].iter().position(|p| p == v).unwrap();
            assert!(uv.distance(valid.uvs[index][corner]) < 1e-6);
        }
    }
    let mut bad = vec![Mesh { positions: vec![Vec3::ZERO; 7], faces: vec![vec![0, 1, 2, 3], vec![0, 4, 5, 6]], ..Default::default() }];
    let mut m = valid.clone();
    m.faces.pop();
    m.uvs.pop();
    bad.push(m);
    let mut m = valid.clone();
    m.faces.push(m.faces[0].clone());
    m.uvs.push(m.uvs[0].clone());
    bad.push(m);
    let mut m = valid.clone();
    m.faces[0].reverse();
    bad.push(m);
    let mut m = valid.clone();
    m.positions.push(Vec3::ZERO);
    bad.push(m);
    let mut m = valid.clone();
    m.loose_edges.push([0, 1]);
    bad.push(m);
    let mut m = valid.clone();
    m.faces[0].pop();
    m.uvs[0].pop();
    bad.push(m);
    let mut m = valid.clone();
    m.mask = vec![0.0; m.positions.len()];
    bad.push(m);
    let mut m = valid.clone();
    m.groups.push(kition_scene_model::vertex_data::VertexGroup { name: "Group".into(), weights: vec![0.0; m.positions.len()] });
    bad.push(m);
    let mut m = valid.clone();
    m.shape_keys.push(kition_scene_model::vertex_data::ShapeKey { name: "Shape".into(), offsets: vec![Vec3::ZERO; m.positions.len()], value: 0.0 });
    bad.push(m);
    let mut m = valid.clone();
    m.face_sets = vec![1; m.faces.len()];
    bad.push(m);
    let mut m = valid.clone();
    m.hidden = vec![false; m.faces.len()];
    bad.push(m);
    let mut m = valid.clone();
    m.seams = vec![m.edges()[0]];
    bad.push(m);
    let mut m = valid.clone();
    m.pinned = vec![0];
    bad.push(m);
    let mut m = valid.clone();
    m.uvs[0][1] += Vec2::X * 0.1;
    bad.push(m);
    let mut m = valid.clone();
    m.multires = vec![vec![Vec3::ZERO]];
    bad.push(m);
    for mut input in bad {
        let before = input.clone();
        assert!(multires::unsubdivide(&mut input).is_err(), "{before:?}");
        assert_eq!(input, before);
    }
    let before = recovered.clone();
    assert!(multires::rebuild_subdivisions(&mut recovered).is_err());
    assert_eq!(before, recovered);
}

#[test]
fn ambiguous_regular_components_and_resource_limits_are_rejected_atomically() {
    let torus = Primitive::Torus { major_segments: 8, minor_segments: 6, major_radius: 2.0, minor_radius: 0.5 }.build().unwrap();
    let mut ambiguous = catmull_clark(&torus);
    let before = ambiguous.clone();
    let error = multires::unsubdivide(&mut ambiguous).unwrap_err().to_string();
    assert!(error.contains("subdivision phase"), "{error}");
    assert_eq!(ambiguous, before);

    let mut at_cap = cube(0);
    for count in multires::vertex_counts(&at_cap, 6) {
        at_cap.multires.push(vec![Vec3::ZERO; count]);
    }
    let before = at_cap.clone();
    assert!(multires::unsubdivide(&mut at_cap).unwrap_err().to_string().contains("at most 6"));
    assert_eq!(at_cap, before);

    let mut too_many = Mesh { positions: vec![Vec3::ZERO; 4], faces: vec![vec![0, 1, 2, 3]; 100_001], ..Default::default() };
    let before = too_many.clone();
    assert!(multires::unsubdivide(&mut too_many).unwrap_err().to_string().contains("100000 base faces"));
    assert_eq!(too_many, before);

    // Reject the high-level allocation before evaluating or copying any high-resolution meshes.
    let mut high = cube(4);
    for count in multires::vertex_counts(&high, 4) {
        high.multires.push(vec![Vec3::ZERO; count]);
    }
    let before = high.clone();
    assert!(multires::unsubdivide(&mut high).unwrap_err().to_string().contains("250000 faces"));
    assert_eq!(high, before);
}
