//! The two round-eight operations share a scene saved by the published 0.15.0 engine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use kition_scene_model::{Mesh, multires};
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn cube_mesh(doc: &SceneDocument) -> Mesh {
    doc.scene().object_named("Cube").unwrap().mesh().unwrap().clone()
}

fn same_surface(a: &Mesh, b: &Mesh) {
    assert_eq!(a.positions.len(), b.positions.len());
    assert_eq!(a.faces.len(), b.faces.len());
    let mut remaining: Vec<_> = b.positions.iter().copied().enumerate().collect();
    let mut mapping = Vec::new();
    for point in &a.positions {
        let nearest =
            remaining.iter().enumerate().min_by(|(_, x), (_, y)| x.1.distance_squared(*point).total_cmp(&y.1.distance_squared(*point))).unwrap();
        assert!(nearest.1.1.distance(*point) < 2e-5, "recovery changed a sculpted point: {point}");
        mapping.push(nearest.1.0 as u32);
        remaining.swap_remove(nearest.0);
    }
    assert!(remaining.is_empty());
    let cycle = |face: &[u32]| {
        let start = face.iter().enumerate().min_by_key(|(_, vertex)| *vertex).unwrap().0;
        face.iter().cycle().skip(start).take(face.len()).copied().collect::<Vec<_>>()
    };
    let mut actual: Vec<_> = a.faces.iter().map(|face| cycle(&face.iter().map(|vertex| mapping[*vertex as usize]).collect::<Vec<_>>())).collect();
    let mut expected: Vec<_> = b.faces.iter().map(|face| cycle(face)).collect();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected, "recovery preserves oriented face topology");
    assert!(a.has_uvs() && b.has_uvs(), "the released sculpt retains per-corner UVs");
    for (face_index, face) in a.faces.iter().enumerate() {
        let mapped: Vec<_> = face.iter().map(|vertex| mapping[*vertex as usize]).collect();
        let (other_index, other) = b.faces.iter().enumerate().find(|(_, other)| cycle(other) == cycle(&mapped)).unwrap();
        for (corner, vertex) in mapped.iter().enumerate() {
            let other_corner = other.iter().position(|other_vertex| other_vertex == vertex).unwrap();
            assert!(a.corner_uv(face_index, corner).distance(b.corner_uv(other_index, other_corner)) < 1e-6, "recovery preserves corner UVs");
        }
    }
}

fn cube_world_x(doc: &SceneDocument) -> f64 {
    doc.inspect().unwrap()["objects"].as_array().unwrap().iter().find(|object| object["name"] == "Cube").unwrap()["worldLocation"][0]
        .as_f64()
        .unwrap()
}

#[test]
fn recovering_a_released_sculpt_keeps_its_motor_and_rebuilds_derived_physics() {
    let mut doc = SceneDocument::open("Released.kscene", include_bytes!("fixtures/released-0.15.0.kscene")).unwrap();
    let original = cube_mesh(&doc);
    assert_eq!((original.positions.len(), original.faces.len(), multires::levels(&original)), (26, 24, 1));
    assert!(original.has_uvs(), "the released fixture must exercise UV preservation");
    let finest = multires::evaluate(&original, 1);
    let body = doc.scene().object_named("Cube").unwrap().physics.clone();
    run(&mut doc, "add.empty", json!({"name": "Motor"}));
    run(
        &mut doc,
        "physics.constraint.add",
        json!({"objects": "Motor", "type": "motor", "object1": "Anchor", "object2": "Cube", "linearMotor": true,
            "linearTargetVelocity": 1, "linearMaxImpulse": 1000}),
    );
    let motor = doc.scene().object_named("Motor").unwrap().physics.clone();
    run(&mut doc, "physics.bake", json!({}));
    run(&mut doc, "frame.set", json!({"frame": 12}));
    let position = cube_world_x(&doc);
    assert!(position < -0.4, "the motor must drive the released body");

    run(&mut doc, "multires.unsubdivide", json!({"object": "Cube"}));
    let recovered = cube_mesh(&doc);
    assert_eq!((recovered.positions.len(), recovered.faces.len(), multires::levels(&recovered)), (8, 6, 2));
    same_surface(&finest, &multires::evaluate(&recovered, 2));
    assert_eq!(doc.scene().object_named("Cube").unwrap().physics, body);
    assert_eq!(doc.scene().object_named("Motor").unwrap().physics, motor);
    assert_eq!(doc.inspect().unwrap()["physics"]["state"], "baking", "geometry recovery must invalidate the old bake");

    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(cube_mesh(&doc), original, "one undo restores the complete released sculpt");
    assert_eq!(doc.scene().object_named("Cube").unwrap().physics, body);
    assert_eq!(doc.scene().object_named("Motor").unwrap().physics, motor);
    run(&mut doc, "edit.redo", json!({}));
    assert_eq!(cube_mesh(&doc), recovered);
    run(&mut doc, "physics.bake", json!({}));
    assert!((cube_world_x(&doc) - position).abs() < 1e-6);

    let mut reopened = SceneDocument::open("Saved.kscene", &doc.save(None).unwrap()).unwrap();
    assert_eq!(cube_mesh(&reopened), recovered);
    assert_eq!(reopened.scene().object_named("Cube").unwrap().physics, body);
    assert_eq!(reopened.scene().object_named("Motor").unwrap().physics, motor);
    assert_eq!(reopened.inspect().unwrap()["physics"]["state"], "none", "the bake must stay derived");
    run(&mut reopened, "physics.bake", json!({}));
    assert!((cube_world_x(&reopened) - position).abs() < 1e-6);
}
