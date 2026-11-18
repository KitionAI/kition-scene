//! Group nodes and zones through commands, as the agent and the node editor drive them: a group
//! used inside another with its inputs and outputs as sockets, a group refused where it would use
//! itself, repeat and simulation zones added in pairs with their items, and the file and undo
//! keeping it all.
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

fn height(doc: &SceneDocument) -> f64 {
    cube(doc)["dimensions"][2].as_f64().unwrap()
}

fn add(doc: &mut SceneDocument, group: u64, kind: &str, values: Value) -> Value {
    run(doc, "nodes.addNode", json!({ "group": group, "kind": kind, "values": values }))
}

fn link(doc: &mut SceneDocument, group: u64, from: u64, from_socket: &str, to: u64, to_socket: &str) {
    run(doc, "nodes.link", json!({ "group": group, "from": from, "fromSocket": from_socket, "to": to, "toSocket": to_socket }));
}

fn node_of(doc: &SceneDocument, group: u64, node: u64) -> Value {
    let inspected = doc.inspect().unwrap();
    let group = inspected["nodeGroups"].as_array().unwrap().iter().find(|item| item["id"] == group).cloned().unwrap();
    group["nodes"].as_array().unwrap().iter().find(|item| item["id"] == node).cloned().unwrap()
}

fn ids(sockets: &Value) -> Vec<String> {
    sockets.as_array().unwrap().iter().map(|socket| socket["id"].as_str().unwrap().to_owned()).collect()
}

/// The cube's group ("Outer", on the cube) and a second group ("Inner") scaling its geometry by
/// its Height input along Z and giving Height back as its Lift output.
fn two_groups(doc: &mut SceneDocument) -> ((u64, u64, u64), (u64, u64, u64)) {
    let outer = run(doc, "nodes.newGroup", json!({ "object": "Cube", "name": "Outer" }));
    let inner = run(doc, "nodes.newGroup", json!({ "name": "Inner" }));
    let part = |made: &Value| (made["group"].as_u64().unwrap(), made["input"].as_u64().unwrap(), made["output"].as_u64().unwrap());
    let (outer, inner) = (part(&outer), part(&inner));
    run(doc, "nodes.addInput", json!({ "group": inner.0, "name": "Height", "type": "float", "default": 1.0 }));
    run(doc, "nodes.addOutput", json!({ "group": inner.0, "name": "Lift", "type": "float" }));
    let combine = add(doc, inner.0, "combineXyz", json!({ "X": 1.0, "Y": 1.0 }))["node"].as_u64().unwrap();
    let transform = add(doc, inner.0, "transform", json!({}))["node"].as_u64().unwrap();
    run(doc, "nodes.unlink", json!({ "group": inner.0, "to": inner.2, "toSocket": "Geometry" }));
    link(doc, inner.0, inner.1, "Height", combine, "Z");
    link(doc, inner.0, inner.1, "Geometry", transform, "Geometry");
    link(doc, inner.0, combine, "Vector", transform, "Scale");
    link(doc, inner.0, transform, "Geometry", inner.2, "Geometry");
    link(doc, inner.0, inner.1, "Height", inner.2, "Lift");
    (outer, inner)
}

#[test]
fn a_group_runs_inside_another_with_its_interface_as_sockets() {
    let mut doc = SceneDocument::create("Groups.kscene", 1280, 720).unwrap();
    let (outer, inner) = two_groups(&mut doc);
    assert!((height(&doc) - 2.0).abs() < 1e-4, "the cube's own group passes it through");
    let node = add(&mut doc, outer.0, "group", json!({ "group": inner.0, "Height": 3.0 }))["node"].as_u64().unwrap();
    run(&mut doc, "nodes.unlink", json!({ "group": outer.0, "to": outer.2, "toSocket": "Geometry" }));
    link(&mut doc, outer.0, outer.1, "Geometry", node, "Geometry");
    link(&mut doc, outer.0, node, "Geometry", outer.2, "Geometry");
    assert!((height(&doc) - 6.0).abs() < 1e-4, "the inner group scales by its Height");
    // The Group node shows the used group's name and interface.
    let shown = node_of(&doc, outer.0, node);
    assert_eq!(shown["title"], "Inner");
    assert_eq!(ids(&shown["inputs"]), ["Geometry", "Height"]);
    assert_eq!(ids(&shown["outputs"]), ["Geometry", "Lift"]);
    // Its output feeds another node: a second use of the group, its Height from the first's Lift.
    let second = add(&mut doc, outer.0, "group", json!({ "group": inner.0 }))["node"].as_u64().unwrap();
    run(&mut doc, "nodes.unlink", json!({ "group": outer.0, "to": outer.2, "toSocket": "Geometry" }));
    link(&mut doc, outer.0, node, "Geometry", second, "Geometry");
    link(&mut doc, outer.0, node, "Lift", second, "Height");
    link(&mut doc, outer.0, second, "Geometry", outer.2, "Geometry");
    assert!((height(&doc) - 18.0).abs() < 1e-3, "{}", height(&doc));
    // A group that would use itself is refused, directly or through another.
    assert!(doc.execute("nodes.addNode", json!({ "group": outer.0, "kind": "group", "values": { "group": outer.0 } })).is_err());
    assert!(doc.execute("nodes.addNode", json!({ "group": inner.0, "kind": "group", "values": { "group": outer.0 } })).is_err());
    let loose = add(&mut doc, inner.0, "group", json!({}))["node"].as_u64().unwrap();
    assert!(doc.execute("nodes.setValue", json!({ "group": inner.0, "node": loose, "id": "group", "value": outer.0 })).is_err());
    // The file keeps both groups and the outputs.
    let bytes = doc.save(None).unwrap();
    let back = SceneDocument::open("Groups.kscene", &bytes).unwrap();
    assert!((height(&back) - 18.0).abs() < 1e-3);
    assert_eq!(back.inspect().unwrap()["nodeGroups"][1]["outputs"][0]["name"], "Lift");
    // Removing the output unlinks it where the groups use it; undo puts it back.
    run(&mut doc, "nodes.removeOutput", json!({ "group": inner.0, "name": "Lift" }));
    assert!((height(&doc) - 6.0).abs() < 1e-3, "the second use takes its default Height again");
    run(&mut doc, "edit.undo", json!({}));
    assert!((height(&doc) - 18.0).abs() < 1e-3);
}

#[test]
fn removing_a_group_lets_the_group_nodes_using_it_go() {
    let mut doc = SceneDocument::create("Groups.kscene", 1280, 720).unwrap();
    let (outer, inner) = two_groups(&mut doc);
    let node = add(&mut doc, outer.0, "group", json!({ "group": inner.0, "Height": 3.0 }))["node"].as_u64().unwrap();
    run(&mut doc, "nodes.unlink", json!({ "group": outer.0, "to": outer.2, "toSocket": "Geometry" }));
    link(&mut doc, outer.0, outer.1, "Geometry", node, "Geometry");
    link(&mut doc, outer.0, node, "Geometry", outer.2, "Geometry");
    assert!((height(&doc) - 6.0).abs() < 1e-4);
    run(&mut doc, "nodes.removeGroup", json!({ "group": inner.0 }));
    let shown = node_of(&doc, outer.0, node);
    assert!(shown["values"].get("group").is_none());
    assert!(ids(&shown["outputs"]).is_empty());
    let inspected = doc.inspect().unwrap();
    assert!(!inspected["nodeGroups"][0]["links"].as_array().unwrap().iter().any(|link| link["from"] == node || link["to"] == node));
    assert!(cube(&doc)["mesh"]["evaluated"]["vertices"].as_u64() == Some(0), "nothing reaches the output");
    run(&mut doc, "edit.undo", json!({}));
    assert!((height(&doc) - 6.0).abs() < 1e-4);
}

#[test]
fn zones_come_in_pairs_with_their_items() {
    let mut doc = SceneDocument::create("Zones.kscene", 1280, 720).unwrap();
    let made = run(&mut doc, "nodes.newGroup", json!({ "object": "Cube" }));
    let (group, input, output) = (made["group"].as_u64().unwrap(), made["input"].as_u64().unwrap(), made["output"].as_u64().unwrap());
    // Adding the zone's input adds its output, holding one geometry item, linked through.
    let zone = add(&mut doc, group, "repeatInput", json!({ "Iterations": 3 }));
    let (start, end) = (zone["node"].as_u64().unwrap(), zone["output"].as_u64().unwrap());
    let shown = node_of(&doc, group, start);
    assert_eq!(shown["zone"]["pair"], end);
    assert_eq!(ids(&shown["inputs"]), ["Iterations", "Geometry"]);
    assert_eq!(ids(&shown["outputs"]), ["Iteration", "Geometry"]);
    assert!(doc.execute("nodes.addNode", json!({ "group": group, "kind": "repeatOutput" })).is_err(), "an output alone is refused");
    // The zone doubles the cube's height each time: a scale inside it.
    let scale = add(&mut doc, group, "transform", json!({ "Scale": [1.0, 1.0, 2.0] }))["node"].as_u64().unwrap();
    run(&mut doc, "nodes.unlink", json!({ "group": group, "to": output, "toSocket": "Geometry" }));
    link(&mut doc, group, input, "Geometry", start, "Geometry");
    link(&mut doc, group, start, "Geometry", scale, "Geometry");
    link(&mut doc, group, scale, "Geometry", end, "Geometry");
    link(&mut doc, group, end, "Geometry", output, "Geometry");
    assert!((height(&doc) - 16.0).abs() < 1e-3, "{}", height(&doc));
    // Items: added on either node, shown on both; a repeated or reserved name is refused.
    run(&mut doc, "nodes.addZoneItem", json!({ "group": group, "node": start, "name": "Count", "type": "int" }));
    assert_eq!(ids(&node_of(&doc, group, end)["inputs"]), ["Geometry", "Count"]);
    assert!(doc.execute("nodes.addZoneItem", json!({ "group": group, "node": end, "name": "Count", "type": "float" })).is_err());
    assert!(doc.execute("nodes.addZoneItem", json!({ "group": group, "node": end, "name": "Iterations", "type": "float" })).is_err());
    assert!(doc.execute("nodes.setValue", json!({ "group": group, "node": end, "id": "items", "value": [] })).is_err());
    let more = add(&mut doc, group, "math", json!({ "Value_001": 1.0 }))["node"].as_u64().unwrap();
    link(&mut doc, group, start, "Count", more, "Value");
    link(&mut doc, group, more, "Value", end, "Count");
    run(&mut doc, "nodes.removeZoneItem", json!({ "group": group, "node": end, "name": "Count" }));
    let inspected = doc.inspect().unwrap();
    assert!(!inspected["nodeGroups"][0]["links"].as_array().unwrap().iter().any(|link| link["fromSocket"] == "Count" || link["toSocket"] == "Count"));
    // The file keeps the zone; removing one of its nodes removes both.
    let back = SceneDocument::open("Zones.kscene", &doc.save(None).unwrap()).unwrap();
    assert!((height(&back) - 16.0).abs() < 1e-3);
    run(&mut doc, "nodes.removeNodes", json!({ "group": group, "nodes": [end] }));
    let inspected = doc.inspect().unwrap();
    assert!(!inspected["nodeGroups"][0]["nodes"].as_array().unwrap().iter().any(|node| node["id"] == start || node["id"] == end));
}

#[test]
fn a_simulation_zone_carries_its_state_from_frame_to_frame() {
    let mut doc = SceneDocument::create("Simulation.kscene", 1280, 720).unwrap();
    let made = run(&mut doc, "nodes.newGroup", json!({ "object": "Cube" }));
    let (group, input, output) = (made["group"].as_u64().unwrap(), made["input"].as_u64().unwrap(), made["output"].as_u64().unwrap());
    let zone = add(&mut doc, group, "simulationInput", json!({}));
    let (start, end) = (zone["node"].as_u64().unwrap(), zone["output"].as_u64().unwrap());
    let scale = add(&mut doc, group, "transform", json!({ "Scale": [1.0, 1.0, 1.5] }))["node"].as_u64().unwrap();
    run(&mut doc, "nodes.unlink", json!({ "group": group, "to": output, "toSocket": "Geometry" }));
    link(&mut doc, group, input, "Geometry", start, "Geometry");
    link(&mut doc, group, start, "Geometry", scale, "Geometry");
    link(&mut doc, group, scale, "Geometry", end, "Geometry");
    link(&mut doc, group, end, "Geometry", output, "Geometry");
    // Frame 1 (the start) runs the zone once; each later frame once more; before the start the
    // zone passes its input through; back at the start it starts again.
    let at = |doc: &mut SceneDocument, frame: f64| {
        run(doc, "frame.set", json!({ "frame": frame }));
        height(doc)
    };
    assert!((at(&mut doc, 1.0) - 3.0).abs() < 1e-3);
    assert!((at(&mut doc, 3.0) - 2.0 * 1.5f64.powi(3)).abs() < 1e-3);
    assert!((at(&mut doc, 0.0) - 2.0).abs() < 1e-3);
    assert!((at(&mut doc, 2.0) - 2.0 * 1.5f64.powi(2)).abs() < 1e-3);
    assert!((at(&mut doc, 1.0) - 3.0).abs() < 1e-3);
}
