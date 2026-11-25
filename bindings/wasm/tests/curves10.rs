//! The node editor and agent share the typed node catalogue, undo, diagnostics, and saved graph.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, id: &str, params: Value) -> Value {
    doc.execute(id, params).unwrap_or_else(|error| panic!("{id}: {error}"))
}
fn add(doc: &mut SceneDocument, kind: &str, values: Value) -> u64 {
    run(doc, "node.addNode", json!({"kind": kind, "values": values}))["node"].as_u64().unwrap()
}
fn link(doc: &mut SceneDocument, from: u64, from_socket: &str, to: u64, to_socket: &str) {
    run(doc, "node.link", json!({"from": from, "fromSocket": from_socket, "to": to, "toSocket": to_socket}));
}
fn vertices(doc: &SceneDocument) -> u64 {
    doc.inspect().unwrap()["objects"].as_array().unwrap().iter().find(|object| object["name"] == "Cube").unwrap()["mesh"]["evaluated"]["vertices"]
        .as_u64()
        .unwrap()
}
fn group(doc: &SceneDocument) -> Value {
    doc.inspect().unwrap()["nodeGroups"][0].clone()
}

#[test]
fn spline_type_nodes_are_discoverable_editable_undoable_and_saved() {
    let mut doc = SceneDocument::create("Splines.kscene", 640, 480).unwrap();
    for kind in ["setSplineType", "setNurbsOrder", "setNurbsWeight"] {
        let described = run(&mut doc, "node.describe", json!({"kind": kind}));
        assert!(described.to_string().contains(kind), "{described}");
    }
    let created = run(&mut doc, "node.newGroup", json!({"object": "Cube", "name": "Spline types"}));
    let output = created["output"].as_u64().unwrap();
    let source = add(&mut doc, "curveSpiral", json!({"Resolution": 6, "Rotations": 1.0}));
    let resolution = add(&mut doc, "setSplineResolution", json!({"Resolution": 4}));
    let kind = add(&mut doc, "setSplineType", json!({"splineType": "catmullRom"}));
    let order = add(&mut doc, "setNurbsOrder", json!({"Order": 3}));
    let weight = add(&mut doc, "setNurbsWeight", json!({"Weight": 0.5}));
    link(&mut doc, source, "Curve", resolution, "Geometry");
    link(&mut doc, resolution, "Geometry", kind, "Curve");
    link(&mut doc, kind, "Curve", order, "Curves");
    link(&mut doc, order, "Curves", weight, "Curves");
    link(&mut doc, weight, "Curves", output, "Geometry");
    assert_eq!(vertices(&doc), 25);
    run(&mut doc, "node.setValue", json!({"node": kind, "id": "splineType", "value": "nurbs"}));
    assert_eq!(vertices(&doc), 21);
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(vertices(&doc), 25);
    run(&mut doc, "edit.redo", json!({}));
    assert_eq!(vertices(&doc), 21);
    let expected = group(&doc);
    let bytes = doc.save_file(None).unwrap();
    let opened = SceneDocument::open("Splines.kscene", &bytes).unwrap();
    assert_eq!(group(&opened), expected);
    assert_eq!(vertices(&opened), 21);
    assert!(doc.check().iter().all(|entry| entry.code != "scene.nodesError"));
    let before = doc.save_file(None).unwrap();
    assert!(doc.execute("node.setValue", json!({"node": kind, "id": "splineType", "value": "unknown"})).is_err());
    assert_eq!(doc.save_file(None).unwrap(), before, "invalid settings are atomic");
}

#[test]
fn unsupported_conversion_reports_the_existing_node_diagnostic_and_undo_recovers() {
    let mut doc = SceneDocument::create("Splines.kscene", 640, 480).unwrap();
    let created = run(&mut doc, "node.newGroup", json!({"object": "Cube"}));
    let source = add(&mut doc, "curveSpiral", json!({"Resolution": 6, "Rotations": 1.0}));
    let nurbs = add(&mut doc, "setSplineType", json!({"splineType": "nurbs"}));
    let convert = add(&mut doc, "setSplineType", json!({"splineType": "poly"}));
    link(&mut doc, source, "Curve", nurbs, "Curve");
    link(&mut doc, nurbs, "Curve", convert, "Curve");
    link(&mut doc, convert, "Curve", created["output"].as_u64().unwrap(), "Geometry");
    assert_eq!(vertices(&doc), 7);
    run(&mut doc, "node.setValue", json!({"node": convert, "id": "splineType", "value": "bezier"}));
    let errors = doc.check();
    assert!(errors.iter().any(|entry| entry.code == "scene.nodesError" && entry.message.contains("NURBS to Bezier requires")));
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(vertices(&doc), 7);
    assert!(doc.check().iter().all(|entry| entry.code != "scene.nodesError"));
}
