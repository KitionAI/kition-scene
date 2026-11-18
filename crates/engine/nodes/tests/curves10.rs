//! Public-application spline-type fixtures. The recorded graph and results are both immutable;
//! KITION_BLENDER runs the public Python recorder again and compares it with the offline file.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_scene_model::modifiers::NodesCall;
use kition_scene_model::{HandleType, Mesh, NodeGroup, ObjectData, Scene};
use kition_scene_nodes::curves::CurveType;
use kition_scene_nodes::field::Scalar;
use kition_scene_nodes::geometry::{Attributes, Domain, Geometry};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const FIXTURE: &str = include_str!("fixtures/curves10-reference.json");

fn evaluate(group: &NodeGroup) -> Result<Geometry, String> {
    let mut scene = Scene::empty();
    let mesh = Mesh::default();
    let object = scene.add_object("Spline types", ObjectData::Mesh(mesh.clone()));
    scene.node_groups.push(group.clone());
    let values = BTreeMap::new();
    let call = NodesCall { mesh: &mesh, scene: &scene, object: scene.object(object).unwrap(), group: group.id, inputs: &values, depth: 0 };
    kition_scene_nodes::eval::Evaluator::new(&call, group, Geometry::default()).run()
}

fn scalar(value: &Scalar) -> Value {
    match value {
        Scalar::Float(value) => json!(value),
        Scalar::Int(value) => json!(value),
        Scalar::Bool(value) => json!(value),
        Scalar::Vector(value) => json!(value.to_array()),
    }
}
fn attributes(values: &Attributes) -> Value {
    let map: serde_json::Map<String, Value> = values.iter().filter(|(name, _)| !name.starts_with('.')).map(|(name, attr)| {
        (name.clone(), json!({"domain": if attr.domain == Domain::Curve { "curve" } else { "point" }, "values": attr.values.iter().map(scalar).collect::<Vec<_>>() }))
    }).collect();
    Value::Object(map)
}
fn handle(kind: &HandleType) -> u8 {
    match kind {
        HandleType::Free => 0,
        HandleType::Auto => 1,
        HandleType::Vector => 2,
        HandleType::Aligned => 3,
    }
}
fn record(geometry: &Geometry) -> Value {
    let mut out = json!({});
    if let Some(curves) = &geometry.curves {
        out["curves"] = json!({
            "positions": curves.positions.iter().map(|point| point.to_array()).collect::<Vec<_>>(),
            "offsets": curves.offsets, "cyclic": curves.cyclic, "resolution": curves.resolution,
            "types": curves.kinds.iter().map(|kind| match kind { CurveType::CatmullRom => 0, CurveType::Poly => 1, CurveType::Bezier => 2, CurveType::Nurbs => 3 }).collect::<Vec<_>>(),
            "radius": curves.radius, "tilt": curves.tilt,
            "handle_left": curves.handle_left.iter().map(|point| point.to_array()).collect::<Vec<_>>(),
            "handle_right": curves.handle_right.iter().map(|point| point.to_array()).collect::<Vec<_>>(),
            "handle_type_left": curves.handle_type_left.iter().map(handle).collect::<Vec<_>>(),
            "handle_type_right": curves.handle_type_right.iter().map(handle).collect::<Vec<_>>(),
            "nurbs_order": curves.nurbs_order, "nurbs_weight": curves.nurbs_weight,
            "knots_mode": curves.knots_mode.iter().map(|mode| match mode { kition_scene_nodes::spline_types::KnotsMode::Normal => 0, kition_scene_nodes::spline_types::KnotsMode::Endpoint => 1, kition_scene_nodes::spline_types::KnotsMode::Bezier => 2 }).collect::<Vec<_>>(),
            "attributes": attributes(&curves.attributes),
        });
    }
    if let Some(points) = &geometry.points {
        out["points"] = json!({"positions": points.positions.iter().map(|point| point.to_array()).collect::<Vec<_>>(), "radius": points.radius, "attributes": attributes(&points.attributes)});
    }
    if let Some(mesh) = &geometry.mesh {
        out["mesh"] = json!({"positions": mesh.mesh.positions.iter().map(|point| point.to_array()).collect::<Vec<_>>(), "faces": mesh.mesh.faces, "edges": mesh.mesh.edges().len()});
    }
    out
}

fn compare(path: &str, ours: &Value, expected: &Value) -> Result<(), String> {
    match expected {
        Value::Object(map) => {
            for (key, value) in map {
                compare(&format!("{path}/{key}"), &ours[key], value)?;
            }
        }
        Value::Array(values) => {
            let mine = ours.as_array().ok_or_else(|| format!("{path}: expected array, got {ours}"))?;
            if mine.len() != values.len() {
                return Err(format!("{path}: {} != {} elements", mine.len(), values.len()));
            }
            for (index, (mine, value)) in mine.iter().zip(values).enumerate() {
                compare(&format!("{path}/{index}"), mine, value)?;
            }
        }
        Value::Number(number) if number.is_f64() => {
            let mine = ours.as_f64().ok_or_else(|| format!("{path}: expected number, got {ours}"))?;
            let expected = number.as_f64().unwrap();
            if !mine.is_finite() || (mine - expected).abs() > 1.0e-5 {
                return Err(format!("{path}: {mine} != {expected}"));
            }
        }
        _ => {
            if ours != expected {
                return Err(format!("{path}: {ours} != {expected}"));
            }
        }
    }
    Ok(())
}

fn normalized(mut expected: Value) -> Value {
    if let Some(curves) = expected.get_mut("curves") {
        let count = curves["positions"].as_array().unwrap().len();
        let splines = curves["offsets"].as_array().unwrap().len() - 1;
        for (name, value, size) in [
            ("types", json!(0), splines),
            ("cyclic", json!(false), splines),
            ("resolution", json!(12), splines),
            ("radius", json!(1.0), count),
            ("tilt", json!(0.0), count),
        ] {
            if curves[name].is_null() {
                curves[name] = Value::Array(vec![value; size]);
            }
        }
        // Unused handles and NURBS settings are not semantically defined on other spline types.
        let types = curves["types"].as_array().unwrap().clone();
        let offsets = curves["offsets"].as_array().unwrap().clone();
        let point_types: Vec<i64> = types
            .iter()
            .enumerate()
            .flat_map(|(curve, kind)| {
                std::iter::repeat_n(kind.as_i64().unwrap(), offsets[curve + 1].as_u64().unwrap() as usize - offsets[curve].as_u64().unwrap() as usize)
            })
            .collect();
        for key in ["handle_left", "handle_right", "handle_type_left", "handle_type_right"] {
            if curves[key].is_null() || !point_types.contains(&2) {
                curves.as_object_mut().unwrap().remove(key);
            }
        }
        for key in ["nurbs_order", "knots_mode", "nurbs_weight"] {
            if curves[key].is_null() || !types.iter().any(|kind| kind == 3) {
                curves.as_object_mut().unwrap().remove(key);
            }
        }
    }
    if let Some(points) = expected.get_mut("points")
        && points["radius"].is_null()
    {
        points["radius"] = Value::Array(vec![json!(1.0); points["positions"].as_array().unwrap().len()]);
    }
    expected
}

#[test]
fn spline_types_match_public_blender_recordings() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    let mut failures = Vec::new();
    for (name, case) in fixture["cases"].as_object().unwrap() {
        let group: NodeGroup = serde_json::from_value(case["group"].clone()).unwrap();
        let ours = evaluate(&group).unwrap_or_else(|error| panic!("{name}: {error}"));
        let expected = normalized(case["result"].clone());
        if let Err(error) = compare(name, &record(&ours), &expected) {
            failures.push(error);
        }
    }
    assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn optional_live_recording_is_the_committed_reference() {
    let Ok(blender) = std::env::var("KITION_BLENDER") else { return };
    let path = std::env::temp_dir().join(format!("scene-curves10-{}.json", std::process::id()));
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/blender/curves10.py");
    let status = std::process::Command::new(blender)
        .args(["-b", "--factory-startup", "--python-exit-code", "1", "--python", script, "--"])
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success());
    let live: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let saved: Value = serde_json::from_str(FIXTURE).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(live, saved);
}

#[test]
fn unsupported_nurbs_fitting_is_explicit_and_evaluation_is_bounded() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    let mut group: NodeGroup = serde_json::from_value(fixture["cases"]["convert-nurbs-nurbs-open-controls"]["group"].clone()).unwrap();
    group.nodes.iter_mut().rev().find(|node| node.kind == "setSplineType").unwrap().values.insert("splineType".into(), json!("bezier"));
    assert!(evaluate(&group).unwrap_err().contains("NURBS to Bezier requires"));
    let mut group: NodeGroup = serde_json::from_value(fixture["cases"]["nurbs-open-order-6"]["group"].clone()).unwrap();
    group.nodes.iter_mut().find(|node| node.kind == "setNurbsOrder").unwrap().values.insert("Order".into(), json!(17));
    assert!(evaluate(&group).unwrap_err().contains("limited to 16"));
    let mut group: NodeGroup = serde_json::from_value(fixture["cases"]["weight-open-2-evaluated"]["group"].clone()).unwrap();
    group.nodes.iter_mut().find(|node| node.kind == "setNurbsWeight").unwrap().values.insert("Weight".into(), json!(1e9));
    assert!(evaluate(&group).unwrap_err().contains("at most 100000000"));
}

#[test]
fn spline_consumers_match_additional_public_recordings() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/curves10-consumers-reference.json")).unwrap();
    let mut failures = Vec::new();
    for (name, case) in fixture["cases"].as_object().unwrap() {
        let group: NodeGroup = serde_json::from_value(case["group"].clone()).unwrap();
        let ours = evaluate(&group).unwrap_or_else(|error| panic!("{name}: {error}"));
        if let Err(error) = compare(name, &record(&ours), &normalized(case["result"].clone())) {
            failures.push(error);
        }
    }
    assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn optional_live_consumers_are_the_committed_reference() {
    let Ok(blender) = std::env::var("KITION_BLENDER") else { return };
    let path = std::env::temp_dir().join(format!("scene-curves10-consumers-{}.json", std::process::id()));
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/blender/curves10.py");
    let status = std::process::Command::new(blender)
        .args(["-b", "--factory-startup", "--python-exit-code", "1", "--python", script, "--"])
        .arg(&path)
        .arg("more")
        .status()
        .unwrap();
    assert!(status.success());
    let live: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let saved: Value = serde_json::from_str(include_str!("fixtures/curves10-consumers-reference.json")).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(live, saved);
}

#[test]
fn rational_nurbs_cannot_silently_drop_weights_when_converted_to_bezier() {
    use kition_scene_model::glam::Vec3;
    use kition_scene_nodes::curves::{ControlPoint, CurvesComponent};
    use kition_scene_nodes::spline_types::{KnotsMode, convert};
    let mut curves = CurvesComponent::default();
    curves.push(CurveType::Nurbs, false, 4, &(0..6).map(|at| ControlPoint::at(Vec3::new(at as f32, 0.0, 0.0))).collect::<Vec<_>>());
    curves.knots_mode[0] = KnotsMode::Bezier;
    curves.nurbs_weight[2] = 2.0;
    assert!(convert(&curves, &[true], CurveType::Bezier).unwrap_err().contains("equal positive weights"));
    curves.nurbs_weight.fill(0.0);
    assert!(convert(&curves, &[true], CurveType::Bezier).unwrap_err().contains("equal positive weights"));
    curves.nurbs_weight.fill(0.5);
    let bezier = convert(&curves, &[true], CurveType::Bezier).unwrap();
    assert_eq!(bezier.points_len(), 2);
    for (left, right) in curves.evaluate(0).positions.iter().zip(bezier.evaluate(0).positions) {
        assert!(left.distance(right) < 1e-6);
    }
}

#[test]
fn spline_boundaries_match_additional_public_recordings() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/curves10-boundary-reference.json")).unwrap();
    let mut failures = Vec::new();
    for (name, case) in fixture["cases"].as_object().unwrap() {
        let group: NodeGroup = serde_json::from_value(case["group"].clone()).unwrap();
        let ours = evaluate(&group).unwrap_or_else(|error| panic!("{name}: {error}"));
        if let Err(error) = compare(name, &record(&ours), &normalized(case["result"].clone())) {
            failures.push(error);
        }
    }
    assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn optional_live_boundaries_are_the_committed_reference() {
    let Ok(blender) = std::env::var("KITION_BLENDER") else { return };
    let path = std::env::temp_dir().join(format!("scene-curves10-boundaries-{}.json", std::process::id()));
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/blender/curves10.py");
    let status = std::process::Command::new(blender)
        .args(["-b", "--factory-startup", "--python-exit-code", "1", "--python", script, "--"])
        .arg(&path)
        .arg("boundaries")
        .status()
        .unwrap();
    assert!(status.success());
    let live: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let saved: Value = serde_json::from_str(include_str!("fixtures/curves10-boundary-reference.json")).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(live, saved);
}

fn bounded_group(kind: &str) -> Value {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    fixture["cases"][format!("{kind}-open-4-controls")]["group"].clone()
}

fn append_node(group: &mut Value, kind: &str, values: Value, socket: &str, output_socket: &str) {
    let output = group["nodes"].as_array().unwrap().iter().find(|node| node["kind"] == "groupOutput").unwrap()["id"].clone();
    let previous = group["links"].as_array_mut().unwrap().iter_mut().find(|link| link["to"] == output).unwrap();
    let input = json!({"from": previous["from"], "fromSocket": previous["fromSocket"], "to": 999, "toSocket": socket});
    previous["from"] = json!(999);
    previous["fromSocket"] = json!(output_socket);
    group["links"].as_array_mut().unwrap().push(input);
    group["nodes"].as_array_mut().unwrap().push(json!({"id": 999, "kind": kind, "values": values}));
}

#[test]
fn spline_limits_count_all_splines_and_sweep_products_before_evaluation() {
    for kind in ["catmullRom", "nurbs"] {
        let mut group = bounded_group(kind);
        let resolution = group["nodes"].as_array_mut().unwrap().iter_mut().find(|node| node["kind"] == "setSplineResolution").unwrap();
        resolution["values"]["Resolution"] = json!(1024);
        append_node(&mut group, "joinGeometry", json!({}), "Geometry", "Geometry");
        let link = group["links"].as_array().unwrap().iter().find(|link| link["to"] == 999).unwrap().clone();
        group["links"].as_array_mut().unwrap().extend(std::iter::repeat_n(link, 4095));
        assert!(evaluate(&serde_json::from_value(group).unwrap()).unwrap_err().contains("more than 4000000"), "{kind}: total budget");
        let mut group = bounded_group(kind);
        let resolution = group["nodes"].as_array_mut().unwrap().iter_mut().find(|node| node["kind"] == "setSplineResolution").unwrap();
        resolution["values"]["Resolution"] = json!(1024);
        append_node(&mut group, "curveToMesh", json!({}), "Curve", "Mesh");
        group["nodes"].as_array_mut().unwrap().push(json!({"id": 998, "kind": "curveCircle", "values": {"Resolution": 4096}}));
        group["links"].as_array_mut().unwrap().push(json!({"from": 998, "fromSocket": "Curve", "to": 999, "toSocket": "Profile Curve"}));
        assert!(evaluate(&serde_json::from_value(group).unwrap()).unwrap_err().contains("more than 4000000"), "{kind}: sweep budget");
    }
}

#[test]
fn spline_coordinates_and_consumer_products_reject_nonfinite_geometry() {
    for kind in ["catmullRom", "nurbs"] {
        let mut group = bounded_group(kind);
        append_node(&mut group, "transform", json!({"Scale": [1e30, 1e30, 1e30]}), "Geometry", "Geometry");
        assert!(evaluate(&serde_json::from_value(group).unwrap()).unwrap_err().contains("1000000000000"), "{kind}: coordinates");
        let mut group = bounded_group(kind);
        append_node(&mut group, "curveToMesh", json!({"Scale": 1e30}), "Curve", "Mesh");
        group["nodes"].as_array_mut().unwrap().push(json!({"id": 998, "kind": "curveCircle", "values": {"Resolution": 4, "Radius": 1e30}}));
        group["links"].as_array_mut().unwrap().push(json!({"from": 998, "fromSocket": "Curve", "to": 999, "toSocket": "Profile Curve"}));
        assert!(evaluate(&serde_json::from_value(group).unwrap()).unwrap_err().contains("non-finite"), "{kind}: sweep product");
    }
}

#[test]
fn vector_handle_conversion_trim_matches_public_recordings() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/curves10-vector-reference.json")).unwrap();
    let mut failures = Vec::new();
    for (name, case) in fixture["cases"].as_object().unwrap() {
        let group: NodeGroup = serde_json::from_value(case["group"].clone()).unwrap();
        let ours = evaluate(&group).unwrap_or_else(|error| panic!("{name}: {error}"));
        if let Err(error) = compare(name, &record(&ours), &normalized(case["result"].clone())) {
            failures.push(error);
        }
    }
    assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn optional_live_vector_trim_is_the_committed_reference() {
    let Ok(blender) = std::env::var("KITION_BLENDER") else { return };
    let path = std::env::temp_dir().join(format!("scene-curves10-vectors-{}.json", std::process::id()));
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/blender/curves10.py");
    let status = std::process::Command::new(blender)
        .args(["-b", "--factory-startup", "--python-exit-code", "1", "--python", script, "--"])
        .arg(&path)
        .arg("vectors")
        .status()
        .unwrap();
    assert!(status.success());
    let live: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let saved: Value = serde_json::from_str(include_str!("fixtures/curves10-vector-reference.json")).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(live, saved);
}
