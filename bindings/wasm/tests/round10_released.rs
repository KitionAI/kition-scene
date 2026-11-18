//! New spline evaluation and independent effector weights share a scene saved by Scene 0.17.0.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use glam::Vec3;
use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn positions(doc: &SceneDocument, name: &str) -> Vec<Vec3> {
    let scene = doc.scene();
    let object = scene.object_named(name).unwrap();
    let world = scene.world_matrix(object.id).unwrap();
    object.evaluated_mesh(scene).unwrap().positions.iter().map(|point| world.transform_point3(*point)).collect()
}

fn settings(doc: &SceneDocument, name: &str, kind: &str) -> Value {
    let id = doc.scene().object_named(name).unwrap().id;
    let inspected = doc.inspect().unwrap();
    inspected["physics"]["objects"].as_array().unwrap().iter().find(|object| object["id"] == id).unwrap()[kind].clone()
}

fn assert_positions(actual: &[Vec3], expected: &[Vec3]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert!(actual.is_finite());
        assert!(actual.distance(*expected) < 1e-5, "{actual:?} differs from {expected:?}");
    }
}

fn bake(doc: &mut SceneDocument) {
    let baked = run(doc, "physics.bake", json!({}));
    assert_eq!(baked["state"], "baked");
    assert_eq!(baked["frames"], 12);
}

#[test]
fn a_released_curve_and_deformable_bodies_keep_independent_settings_and_history() {
    let mut doc = SceneDocument::open("Released.kscene", include_bytes!("fixtures/released-0.17.0.kscene")).unwrap();
    let curve_before = positions(&doc, "Cube");
    let cloth_rest = positions(&doc, "Cloth");
    let soft_rest = positions(&doc, "Soft");
    let released_groups = doc.inspect().unwrap()["nodeGroups"].clone();
    assert_eq!((curve_before.len(), cloth_rest.len(), soft_rest.len()), (9, 9, 8));
    let cloth_before = settings(&doc, "Cloth", "cloth");
    let soft_before = settings(&doc, "Soft", "softBody");
    for settings in [&cloth_before, &soft_before] {
        for key in ["all", "gravity", "force", "wind", "vortex", "turbulence", "drag"] {
            assert_eq!(settings["effectorWeights"][key].as_f64(), Some(1.0), "released settings default {key} to one");
        }
    }
    bake(&mut doc);
    let cloth_moving = positions(&doc, "Cloth");
    let soft_moving = positions(&doc, "Soft");
    assert!(cloth_moving.iter().zip(&cloth_rest).any(|(a, b)| a.distance(*b) > 1e-4));
    assert!(soft_moving.iter().zip(&soft_rest).any(|(a, b)| a.distance(*b) > 1e-4));

    run(&mut doc, "physics.cloth.set", json!({"objects":"Cloth", "effectorWeights":{"gravity":0, "wind":0}}));
    let cloth_weighted = settings(&doc, "Cloth", "cloth");
    assert_ne!(cloth_weighted, cloth_before);
    assert_eq!(settings(&doc, "Soft", "softBody"), soft_before);
    bake(&mut doc);
    assert_positions(&positions(&doc, "Cloth"), &cloth_rest);
    assert_eq!(positions(&doc, "Soft"), soft_moving, "one body's weights cannot affect the other body");
    assert_eq!(positions(&doc, "Cube"), curve_before);

    let inspected = doc.inspect().unwrap();
    let group = inspected["nodeGroups"].as_array().unwrap().iter().find(|group| group["name"] == "ReleasedCurves").unwrap();
    let nodes = group["nodes"].as_array().unwrap();
    let source = nodes.iter().find(|node| node["kind"] == "curveBezierSegment").unwrap()["id"].clone();
    let output = nodes.iter().find(|node| node["kind"] == "groupOutput").unwrap()["id"].clone();
    let converted =
        run(&mut doc, "nodes.addNode", json!({"group":"ReleasedCurves", "kind":"setSplineType", "values":{"splineType":"nurbs"}}))["node"].clone();
    run(&mut doc, "nodes.link", json!({"group":"ReleasedCurves", "from":source, "fromSocket":"Curve", "to":converted, "toSocket":"Curve"}));
    run(&mut doc, "nodes.link", json!({"group":"ReleasedCurves", "from":converted, "fromSocket":"Curve", "to":output, "toSocket":"Geometry"}));
    let converted_groups = doc.inspect().unwrap()["nodeGroups"].clone();
    assert_ne!(converted_groups, released_groups);
    assert_positions(&positions(&doc, "Cube"), &curve_before);
    assert_eq!(settings(&doc, "Cloth", "cloth"), cloth_weighted);
    bake(&mut doc);
    assert_positions(&positions(&doc, "Cloth"), &cloth_rest);
    assert_eq!(positions(&doc, "Soft"), soft_moving);

    for _ in 0..3 {
        run(&mut doc, "edit.undo", json!({}));
    }
    assert_eq!(doc.inspect().unwrap()["nodeGroups"], released_groups);
    assert_eq!(positions(&doc, "Cube"), curve_before);
    assert_eq!(settings(&doc, "Cloth", "cloth"), cloth_weighted, "undoing the three node edits retains the weight edit");
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(settings(&doc, "Cloth", "cloth"), cloth_before);
    bake(&mut doc);
    assert_eq!(positions(&doc, "Cloth"), cloth_moving);
    assert_eq!(positions(&doc, "Soft"), soft_moving);
    for _ in 0..4 {
        run(&mut doc, "edit.redo", json!({}));
    }
    assert_eq!(doc.inspect().unwrap()["nodeGroups"], converted_groups);
    assert_positions(&positions(&doc, "Cube"), &curve_before);
    assert_eq!(settings(&doc, "Cloth", "cloth"), cloth_weighted);

    run(&mut doc, "physics.softBody.set", json!({"objects":"Soft", "effectorWeights":{"gravity":0, "wind":0}}));
    let soft_weighted = settings(&doc, "Soft", "softBody");
    bake(&mut doc);
    assert_positions(&positions(&doc, "Cloth"), &cloth_rest);
    assert_positions(&positions(&doc, "Soft"), &soft_rest);
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(settings(&doc, "Soft", "softBody"), soft_before);
    assert_eq!(settings(&doc, "Cloth", "cloth"), cloth_weighted);
    run(&mut doc, "edit.redo", json!({}));

    let mut reopened = SceneDocument::open("Saved.kscene", &doc.save(None).unwrap()).unwrap();
    assert_eq!(settings(&reopened, "Cloth", "cloth"), cloth_weighted);
    assert_eq!(settings(&reopened, "Soft", "softBody"), soft_weighted);
    assert_eq!(reopened.inspect().unwrap()["physics"]["state"], "none", "bakes are derived, never saved");
    assert_eq!(reopened.inspect().unwrap()["nodeGroups"], doc.inspect().unwrap()["nodeGroups"]);
    assert_eq!(reopened.inspect().unwrap()["nodeGroups"], converted_groups);
    assert_positions(&positions(&reopened, "Cube"), &curve_before);
    bake(&mut reopened);
    assert_positions(&positions(&reopened, "Cloth"), &cloth_rest);
    assert_positions(&positions(&reopened, "Soft"), &soft_rest);
}
