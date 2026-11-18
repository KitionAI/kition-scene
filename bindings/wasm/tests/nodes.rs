//! Geometry nodes through commands, as the agent and the node editor drive them: a group made
//! on an object, nodes added, linked, set, moved, and removed, inputs exposed as the modifier's
//! settings, undo, diagnostics, and the file keeping it all.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn cube(doc: &SceneDocument) -> Value {
    let inspected = doc.inspect().unwrap();
    inspected["objects"].as_array().unwrap().iter().find(|object| object["name"] == "Cube").cloned().unwrap()
}

fn evaluated(doc: &SceneDocument) -> (u64, u64) {
    let mesh = &cube(doc)["mesh"]["evaluated"];
    (mesh["vertices"].as_u64().unwrap(), mesh["faces"].as_u64().unwrap())
}

fn add(doc: &mut SceneDocument, kind: &str, values: Value) -> u64 {
    run(doc, "nodes.addNode", json!({ "kind": kind, "values": values }))["node"].as_u64().unwrap()
}

fn link(doc: &mut SceneDocument, from: u64, from_socket: &str, to: u64, to_socket: &str) {
    run(doc, "nodes.link", json!({ "from": from, "fromSocket": from_socket, "to": to, "toSocket": to_socket }));
}

#[test]
fn a_scatter_tree_is_built_saved_and_undone() {
    let mut doc = SceneDocument::create("Nodes.kscene", 1280, 720).unwrap();
    let made = run(&mut doc, "nodes.newGroup", json!({ "object": "Cube" }));
    let (input, output) = (made["input"].as_u64().unwrap(), made["output"].as_u64().unwrap());
    assert_eq!(cube(&doc)["modifiers"][0]["type"], "geometryNodes");
    assert_eq!(evaluated(&doc), (8, 6), "a new group passes the mesh through");
    // Grid → Distribute Points → Instance on Points (a small cube) → Realize, joined with the input.
    let grid = add(&mut doc, "meshGrid", json!({ "Size X": 4.0, "Size Y": 4.0 }));
    let scatter = add(&mut doc, "distributePointsOnFaces", json!({ "Density": 10.0 }));
    let small = add(&mut doc, "meshCube", json!({ "Size": [0.1, 0.1, 0.1] }));
    let instance = add(&mut doc, "instanceOnPoints", json!({}));
    let realize = add(&mut doc, "realizeInstances", json!({}));
    let join = add(&mut doc, "joinGeometry", json!({}));
    link(&mut doc, grid, "Mesh", scatter, "Mesh");
    link(&mut doc, scatter, "Points", instance, "Points");
    link(&mut doc, small, "Mesh", instance, "Instance");
    link(&mut doc, instance, "Instances", realize, "Geometry");
    link(&mut doc, realize, "Geometry", join, "Geometry");
    link(&mut doc, input, "Geometry", join, "Geometry");
    link(&mut doc, join, "Geometry", output, "Geometry");
    let (vertices, faces) = evaluated(&doc);
    assert_eq!((vertices - 8) % 8, 0);
    assert_eq!(faces - 6, (vertices - 8) / 8 * 6);
    assert!(vertices > 8 * 100);
    // Inspect shows the group with each node's shown sockets.
    let inspected = doc.inspect().unwrap();
    let group = &inspected["nodeGroups"][0];
    assert_eq!(group["nodes"].as_array().unwrap().len(), 8);
    assert_eq!(group["links"].as_array().unwrap().len(), 7);
    // A cycle and a mismatched link are refused.
    assert!(doc.execute("nodes.link", json!({ "from": join, "fromSocket": "Geometry", "to": instance, "toSocket": "Points" })).is_err());
    assert!(doc.execute("nodes.link", json!({ "from": scatter, "fromSocket": "Normal", "to": realize, "toSocket": "Geometry" })).is_err());
    // The file keeps the group, and the modifier still uses it.
    let bytes = doc.save(None).unwrap();
    let back = SceneDocument::open("Nodes.kscene", &bytes).unwrap();
    assert_eq!(evaluated(&back), (vertices, faces));
    // Removing the realize node unlinks it; undo puts both back.
    run(&mut doc, "nodes.removeNodes", json!({ "nodes": [realize] }));
    assert_eq!(evaluated(&doc), (8, 6));
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(evaluated(&doc), (vertices, faces));
}

#[test]
fn group_inputs_are_the_modifiers_settings() {
    let mut doc = SceneDocument::create("Nodes.kscene", 1280, 720).unwrap();
    let made = run(&mut doc, "nodes.newGroup", json!({ "object": "Cube", "name": "Lift" }));
    let (input, output) = (made["input"].as_u64().unwrap(), made["output"].as_u64().unwrap());
    run(&mut doc, "nodes.addInput", json!({ "group": "Lift", "name": "Height", "type": "float", "default": 1.0 }));
    let combine = add(&mut doc, "combineXyz", json!({}));
    let transform = add(&mut doc, "transform", json!({}));
    link(&mut doc, input, "Height", combine, "Z");
    link(&mut doc, input, "Geometry", transform, "Geometry");
    link(&mut doc, combine, "Vector", transform, "Translation");
    link(&mut doc, transform, "Geometry", output, "Geometry");
    let top = |doc: &SceneDocument| cube(doc)["dimensions"][2].as_f64().unwrap();
    assert!((top(&doc) - 2.0).abs() < 1e-4, "translation keeps the size");
    run(&mut doc, "modifier.set", json!({ "object": "Cube", "index": 0, "inputs": { "Height": 3.0 } }));
    assert_eq!(cube(&doc)["modifiers"][0]["inputs"]["Height"], 3.0);
    // Setting values: a setting's choice, a socket's value; bad ones are refused.
    let math = add(&mut doc, "math", json!({ "operation": "sqrt" }));
    run(&mut doc, "nodes.setValue", json!({ "node": math, "id": "Value", "value": 9.0 }));
    assert!(doc.execute("nodes.setValue", json!({ "node": math, "id": "operation", "value": "cube_root" })).is_err());
    assert!(doc.execute("nodes.setValue", json!({ "node": math, "id": "Value", "value": "nine" })).is_err());
    run(&mut doc, "nodes.moveNodes", json!({ "nodes": [math], "offset": [10, 20] }));
    let inspected = doc.inspect().unwrap();
    let node = inspected["nodeGroups"][0]["nodes"].as_array().unwrap().iter().find(|node| node["id"] == math).cloned().unwrap();
    assert_eq!(node["location"], json!([10.0, 20.0]));
    assert_eq!(node["inputs"].as_array().unwrap().len(), 1, "sqrt shows one input");
    // Removing the input unlinks it; removing the group lets the modifier go of it.
    run(&mut doc, "nodes.removeInput", json!({ "name": "Height" }));
    run(&mut doc, "nodes.removeGroup", json!({}));
    assert!(cube(&doc)["modifiers"][0].get("group").is_none());
    assert_eq!(evaluated(&doc), (8, 6));
}

#[test]
fn a_broken_group_is_a_diagnostic_and_describe_lists_kinds() {
    let mut doc = SceneDocument::create("Nodes.kscene", 1280, 720).unwrap();
    let made = run(&mut doc, "nodes.newGroup", json!({ "object": "Cube" }));
    run(&mut doc, "nodes.removeNodes", json!({ "nodes": [made["output"]] }));
    let codes: Vec<String> = doc.check().into_iter().map(|diagnostic| diagnostic.code).collect();
    assert!(codes.contains(&"scene.nodesError".to_owned()), "{codes:?}");
    let described = run(&mut doc, "nodes.describe", json!({}));
    let kinds = described["kinds"].as_array().unwrap();
    assert!(kinds.len() >= 38);
    let math = run(&mut doc, "nodes.describe", json!({ "kind": "math" }));
    assert_eq!(math["settings"][0]["id"], "operation");
    let catalogue = doc.commands();
    assert!(catalogue.iter().any(|command| command.id == "nodes.link" && command.schema.is_some()));
}
