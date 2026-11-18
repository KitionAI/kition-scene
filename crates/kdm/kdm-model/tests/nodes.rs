//! Node groups through a replica: written whole, edited node by node, merged with a peer's
//! edits, and repaired.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::kdm::{ActorId, DocId, Origin, Replica};
use kition_kit::session::{BlobSink, KitModel, MemoryBlobs};
use kition_scene_kdm::mesh_blob::MeshCache;
use kition_scene_kdm::read::read_scene;
use kition_scene_kdm::write::{Written, stored_form, write_changes, write_scene};
use kition_scene_kdm::{SceneModel, ids, schema};
use kition_scene_model::{GroupSocket, Id, Link, Modifier, ModifierKind, Node, NodeGroup, Scene, SocketType};
use serde_json::json;

fn keep(blobs: &mut MemoryBlobs, written: Written) {
    for blob in written.blobs {
        blobs.put(&blob.bytes, "application/octet-stream");
    }
}

fn create(scene: &Scene) -> (Replica, MemoryBlobs) {
    let mut blobs = MemoryBlobs::default();
    let (replica, written) =
        Replica::create_with(schema(), DocId::new(), ActorId::random(), &Origin::system("create"), |tx| write_scene(tx, scene)).unwrap();
    keep(&mut blobs, written);
    (replica, blobs)
}

fn read(replica: &Replica, blobs: &MemoryBlobs) -> Scene {
    read_scene(replica, blobs, &mut MeshCache::default()).unwrap()
}

/// Runs `edit` on what `replica` holds and records it, as the document's bridge does.
fn edit(replica: &mut Replica, blobs: &mut MemoryBlobs, edit: impl FnOnce(&mut Scene)) {
    let before = read(replica, blobs);
    let mut after = before.clone();
    ids::allocate(replica, replica.actor(), &mut after);
    edit(&mut after);
    let next = after.allocator.0.map(|(next, _)| next);
    let after = stored_form(&after, &before);
    let key = ids::key_of(replica.actor());
    let written = replica.transact(&Origin::person("p"), |tx| write_changes(tx, &before, &after, next.map(|next| (key.as_str(), next)))).unwrap();
    keep(blobs, written);
}

fn sync(a: &mut Replica, b: &mut Replica, blobs_a: &mut MemoryBlobs, blobs_b: &mut MemoryBlobs) {
    let to_b = a.export_changes(&b.version()).unwrap();
    let to_a = b.export_changes(&a.version()).unwrap();
    b.import_changes(&to_b).unwrap();
    a.import_changes(&to_a).unwrap();
    for (hash, bytes) in blobs_a.0.clone() {
        blobs_b.0.entry(hash).or_insert(bytes);
    }
    for (hash, bytes) in blobs_b.0.clone() {
        blobs_a.0.entry(hash).or_insert(bytes);
    }
    for replica in [&mut *a, &mut *b] {
        replica.transact(&Origin::system("repair"), |tx| Ok(kition_scene_kdm::repair::repair(tx).unwrap())).unwrap();
    }
}

fn link(from: Id, from_socket: &str, to: Id, to_socket: &str) -> Link {
    Link { from, from_socket: from_socket.into(), to, to_socket: to_socket.into() }
}

/// The starter scene with a group (input, transform, output) on the cube.
fn with_group() -> (Scene, Id, [Id; 3]) {
    let mut scene = Scene::starter();
    let (group, input, output, transform) = (scene.take_id(), scene.take_id(), scene.take_id(), scene.take_id());
    let mut tree = NodeGroup::new(group, "Geometry Nodes", input, output);
    tree.links.clear();
    tree.nodes.push(Node { id: transform, kind: "transform".into(), location: [0.0, 0.0], values: Default::default() });
    tree.links.push(link(input, "Geometry", transform, "Geometry"));
    tree.links.push(link(transform, "Geometry", output, "Geometry"));
    scene.node_groups.push(tree);
    let cube = scene.object_named("Cube").unwrap().id;
    scene.object_mut(cube).unwrap().modifiers.push(Modifier {
        name: "GeometryNodes".into(),
        enabled: true,
        kind: ModifierKind::GeometryNodes { group: Some(group), inputs: Default::default() },
    });
    scene.validate().unwrap();
    (scene, group, [input, output, transform])
}

fn peers(scene: &Scene) -> (Replica, Replica, MemoryBlobs, MemoryBlobs) {
    let (a, blobs_a) = create(scene);
    let b = Replica::load(schema(), &a.snapshot().unwrap(), ActorId::random()).unwrap();
    let blobs_b = blobs_a.clone();
    (a, b, blobs_a, blobs_b)
}

#[test]
fn groups_round_trip_and_ids_stay_unique() {
    let (scene, group, _) = with_group();
    let (mut replica, mut blobs) = create(&scene);
    let back = read(&replica, &blobs);
    assert_eq!(back.node_groups, scene.node_groups);
    assert!(back.next_id > group);
    assert!(SceneModel::load(&replica, &blobs).is_ok());
    let mut added = 0;
    edit(&mut replica, &mut blobs, |scene| {
        added = scene.take_id();
        scene.node_groups[0].nodes.push(Node { id: added, kind: "meshCube".into(), location: [10.0, 20.0], values: Default::default() });
    });
    let back = read(&replica, &blobs);
    assert!(added >= ids::FIRST_RANGE);
    assert_eq!(back.node_groups[0].node(added).unwrap().location, [10.0, 20.0]);
    back.validate().unwrap();
}

#[test]
fn edits_of_different_nodes_and_values_merge() {
    let (scene, _, [_, _, transform]) = with_group();
    let (mut a, mut b, mut blobs_a, mut blobs_b) = peers(&scene);
    edit(&mut a, &mut blobs_a, |scene| {
        let node = scene.node_groups[0].node_mut(transform).unwrap();
        node.values.insert("Translation".into(), json!([1.0, 0.0, 0.0]));
        node.location = [5.0, 5.0];
    });
    edit(&mut b, &mut blobs_b, |scene| {
        let node = scene.node_groups[0].node_mut(transform).unwrap();
        node.values.insert("Scale".into(), json!([2.0, 2.0, 2.0]));
        let id = scene.take_id();
        scene.node_groups[0].nodes.push(Node { id, kind: "index".into(), location: [0.0, 0.0], values: Default::default() });
    });
    sync(&mut a, &mut b, &mut blobs_a, &mut blobs_b);
    assert_eq!(a.state_json().unwrap(), b.state_json().unwrap());
    let scene = read(&a, &blobs_a);
    let node = scene.node_groups[0].node(transform).unwrap();
    assert_eq!(node.values.len(), 2, "both values kept");
    assert_eq!(node.location, [5.0, 5.0]);
    assert_eq!(scene.node_groups[0].nodes.len(), 4);
    scene.validate().unwrap();
}

#[test]
fn repair_drops_links_that_no_longer_fit() {
    let (scene, group, [input, output, transform]) = with_group();
    let (mut a, mut b, mut blobs_a, mut blobs_b) = peers(&scene);
    // A deletes the transform (and links the input straight through) while B links a new cube
    // into the transform and into the output: the output takes one link.
    edit(&mut a, &mut blobs_a, |scene| {
        let tree = &mut scene.node_groups[0];
        tree.nodes.retain(|node| node.id != transform);
        tree.links = vec![link(input, "Geometry", output, "Geometry")];
    });
    let mut cube = 0;
    edit(&mut b, &mut blobs_b, |scene| {
        cube = scene.take_id();
        let tree = &mut scene.node_groups[0];
        tree.nodes.push(Node { id: cube, kind: "meshCube".into(), location: [0.0, 0.0], values: Default::default() });
        tree.links.push(link(cube, "Mesh", transform, "Translation"));
    });
    sync(&mut a, &mut b, &mut blobs_a, &mut blobs_b);
    assert_eq!(a.state_json().unwrap(), b.state_json().unwrap());
    let scene = read(&a, &blobs_a);
    let tree = scene.node(group);
    assert!(tree.node(transform).is_none() && tree.node(cube).is_some());
    assert_eq!(tree.links, vec![link(input, "Geometry", output, "Geometry")]);
    scene.validate().unwrap();
}

#[test]
fn concurrent_links_into_one_input_and_cycles_resolve() {
    let (scene, group, [input, output, transform]) = with_group();
    let (mut a, mut b, mut blobs_a, mut blobs_b) = peers(&scene);
    // Both add a second transform; A links transform → second, B links second → transform:
    // together a cycle. Both also link something new into the output.
    let second = 9_000;
    let mut start = scene.clone();
    start.node_groups[0].nodes.push(Node { id: second, kind: "transform".into(), location: [0.0, 0.0], values: Default::default() });
    let (mut a2, mut b2, mut blobs_a2, mut blobs_b2) = peers(&start);
    edit(&mut a2, &mut blobs_a2, |scene| scene.node_groups[0].links.push(link(transform, "Geometry", second, "Geometry")));
    edit(&mut b2, &mut blobs_b2, |scene| {
        let tree = &mut scene.node_groups[0];
        tree.links.retain(|link| link.to != transform);
        tree.links.push(link(second, "Geometry", transform, "Geometry"));
    });
    sync(&mut a2, &mut b2, &mut blobs_a2, &mut blobs_b2);
    assert_eq!(a2.state_json().unwrap(), b2.state_json().unwrap());
    let merged = read(&a2, &blobs_a2);
    let tree = merged.node(group);
    assert!(!tree.links.iter().any(|link| kition_scene_kdm::nodes::reaches(&tree.links, link.to, link.from)), "no cycle");
    assert_eq!(tree.links.iter().filter(|link| link.to == transform || link.to == second).count(), 1, "one of the two links went");
    assert_eq!(kition_scene_kdm::nodes::fitting_links(tree, &merged.node_groups), tree.links);
    merged.validate().unwrap();
    // Two links into the output at once: one stays.
    edit(&mut a, &mut blobs_a, |scene| {
        let tree = &mut scene.node_groups[0];
        tree.links.retain(|link| link.to != output);
        tree.links.push(link(input, "Geometry", output, "Geometry"));
    });
    edit(&mut b, &mut blobs_b, |_| {});
    sync(&mut a, &mut b, &mut blobs_a, &mut blobs_b);
    let scene = read(&b, &blobs_b);
    assert_eq!(scene.node(group).links.iter().filter(|link| link.to == output).count(), 1);
}

#[test]
fn deleting_a_group_or_its_targets_lets_go_of_them() {
    let (scene, group, _) = with_group();
    let (mut a, mut b, mut blobs_a, mut blobs_b) = peers(&scene);
    let mut info = 0;
    // A deletes the group and the light; B points an Object Info at the light.
    edit(&mut a, &mut blobs_a, |scene| {
        scene.node_groups.clear();
        let cube = scene.object_named("Cube").unwrap().id;
        let light = scene.object_named("Light").unwrap().id;
        scene.object_mut(cube).unwrap().modifiers[0].kind = ModifierKind::GeometryNodes { group: Some(group), inputs: Default::default() };
        scene.remove_objects(&[light]).unwrap();
    });
    edit(&mut b, &mut blobs_b, |scene| {
        info = scene.take_id();
        let light = scene.object_named("Light").unwrap().id;
        scene.node_groups[0].nodes.push(Node {
            id: info,
            kind: "objectInfo".into(),
            location: [0.0, 0.0],
            values: [("Object".to_owned(), json!(light))].into(),
        });
    });
    sync(&mut a, &mut b, &mut blobs_a, &mut blobs_b);
    assert_eq!(a.state_json().unwrap(), b.state_json().unwrap());
    let scene = read(&a, &blobs_a);
    assert!(scene.node_groups.is_empty());
    let cube = scene.object_named("Cube").unwrap();
    assert_eq!(cube.modifiers[0].kind, ModifierKind::GeometryNodes { group: None, inputs: Default::default() });
    scene.validate().unwrap();
    // The other way: the group stays, the light goes, the Object Info lets go of it.
    let (scene, _, _) = with_group();
    let (mut a, mut b, mut blobs_a, mut blobs_b) = peers(&scene);
    edit(&mut a, &mut blobs_a, |scene| {
        let light = scene.object_named("Light").unwrap().id;
        scene.remove_objects(&[light]).unwrap();
    });
    edit(&mut b, &mut blobs_b, |scene| {
        info = scene.take_id();
        let light = scene.object_named("Light").unwrap().id;
        scene.node_groups[0].nodes.push(Node {
            id: info,
            kind: "objectInfo".into(),
            location: [0.0, 0.0],
            values: [("Object".to_owned(), json!(light))].into(),
        });
    });
    sync(&mut a, &mut b, &mut blobs_a, &mut blobs_b);
    let scene = read(&a, &blobs_a);
    assert!(scene.node_groups[0].node(info).unwrap().values.is_empty());
}

/// The starter scene with the cube's group (`with_group`) and a second group, "Inner", with a
/// Height input and a Lift output; returns the inner group's id.
fn with_inner() -> (Scene, Id, Id) {
    let (mut scene, group, _) = with_group();
    let (inner, input, output) = (scene.take_id(), scene.take_id(), scene.take_id());
    let mut tree = NodeGroup::new(inner, "Inner", input, output);
    tree.inputs.push(GroupSocket { name: "Height".into(), kind: SocketType::Float, default: json!(0.5) });
    tree.outputs.push(GroupSocket { name: "Lift".into(), kind: SocketType::Float, default: serde_json::Value::Null });
    tree.links.push(link(input, "Height", output, "Lift"));
    scene.node_groups.push(tree);
    (scene, group, inner)
}

fn group_node(id: Id, used: Id) -> Node {
    Node { id, kind: "group".into(), location: [0.0, 0.0], values: [("group".to_owned(), json!(used))].into() }
}

#[test]
fn group_outputs_group_nodes_and_zones_round_trip() {
    let (mut scene, group, inner) = with_inner();
    let (node, zone_in, zone_out) = (scene.take_id(), scene.take_id(), scene.take_id());
    let tree = scene.node_group_mut(group).unwrap();
    tree.nodes.push(group_node(node, inner));
    tree.nodes.push(Node {
        id: zone_out,
        kind: "repeatOutput".into(),
        location: [0.0, 0.0],
        values: [("items".to_owned(), json!([{ "name": "Geometry", "type": "geometry" }, { "name": "Count", "type": "int" }]))].into(),
    });
    tree.nodes.push(Node {
        id: zone_in,
        kind: "repeatInput".into(),
        location: [0.0, 0.0],
        values: [("output".to_owned(), json!(zone_out)), ("Iterations".to_owned(), json!(3))].into(),
    });
    let first = tree.nodes.iter().find(|node| node.kind == "transform").unwrap().id;
    tree.links.push(link(node, "Lift", first, "Translation"));
    tree.links.push(link(zone_in, "Count", zone_out, "Count"));
    scene.validate().unwrap();
    let (replica, blobs) = create(&scene);
    assert!(kition_scene_kdm::SCHEMA.contains("\"outputs\""));
    let back = read(&replica, &blobs);
    assert_eq!(back.node_groups, scene.node_groups);
    back.validate().unwrap();
}

#[test]
fn groups_put_into_each_other_at_once_resolve() {
    let (scene, group, inner) = with_inner();
    let (mut a, mut b, mut blobs_a, mut blobs_b) = peers(&scene);
    // A puts Inner into the cube's group; B puts the cube's group into Inner: together each
    // would use itself.
    let (mut in_outer, mut in_inner) = (0, 0);
    edit(&mut a, &mut blobs_a, |scene| {
        in_outer = scene.take_id();
        scene.node_group_mut(group).unwrap().nodes.push(group_node(in_outer, inner));
    });
    edit(&mut b, &mut blobs_b, |scene| {
        in_inner = scene.take_id();
        let tree = scene.node_group_mut(inner).unwrap();
        tree.nodes.push(group_node(in_inner, group));
        let output = tree.nodes.iter().find(|node| node.kind == "groupOutput").unwrap().id;
        tree.links.retain(|link| link.to != output || link.to_socket != "Geometry");
        tree.links.push(link(in_inner, "Geometry", output, "Geometry"));
    });
    sync(&mut a, &mut b, &mut blobs_a, &mut blobs_b);
    assert_eq!(a.state_json().unwrap(), b.state_json().unwrap());
    let scene = read(&a, &blobs_a);
    scene.validate().unwrap();
    // The cube's group comes first in key order: its use stays, Inner's lets go (and its link
    // from the Group node's geometry goes with the socket).
    assert_eq!(scene.node(group).node(in_outer).unwrap().values.get("group"), Some(&json!(inner)));
    assert!(!scene.node(inner).node(in_inner).unwrap().values.contains_key("group"));
    assert!(!scene.node(inner).links.iter().any(|link| link.from == in_inner));
}

#[test]
fn group_nodes_and_zones_let_go_of_what_is_gone() {
    let (mut scene, group, inner) = with_inner();
    let (zone_in, zone_out) = (scene.take_id(), scene.take_id());
    let tree = scene.node_group_mut(group).unwrap();
    tree.nodes.push(Node { id: zone_out, kind: "simulationOutput".into(), location: [0.0, 0.0], values: Default::default() });
    tree.nodes.push(Node {
        id: zone_in,
        kind: "simulationInput".into(),
        location: [0.0, 0.0],
        values: [("output".to_owned(), json!(zone_out))].into(),
    });
    tree.links.push(link(zone_in, "Geometry", zone_out, "Geometry"));
    let (mut a, mut b, mut blobs_a, mut blobs_b) = peers(&scene);
    // A removes Inner and the zone's output; B adds a Group node using Inner, linked.
    let mut node = 0;
    edit(&mut a, &mut blobs_a, |scene| {
        scene.node_groups.retain(|other| other.id != inner);
        let tree = scene.node_group_mut(group).unwrap();
        tree.nodes.retain(|node| node.id != zone_out);
        tree.links.retain(|link| link.to != zone_out);
    });
    edit(&mut b, &mut blobs_b, |scene| {
        node = scene.take_id();
        let tree = scene.node_group_mut(group).unwrap();
        tree.nodes.push(group_node(node, inner));
        let transform = tree.nodes.iter().find(|node| node.kind == "transform").unwrap().id;
        tree.links.push(link(node, "Lift", transform, "Translation"));
    });
    sync(&mut a, &mut b, &mut blobs_a, &mut blobs_b);
    assert_eq!(a.state_json().unwrap(), b.state_json().unwrap());
    let scene = read(&a, &blobs_a);
    scene.validate().unwrap();
    let tree = scene.node(group);
    assert!(!tree.node(node).unwrap().values.contains_key("group"));
    assert!(!tree.links.iter().any(|link| link.from == node));
    assert!(!tree.node(zone_in).unwrap().values.contains_key("output"));
}

trait GroupOf {
    fn node(&self, group: Id) -> &NodeGroup;
}

impl GroupOf for Scene {
    fn node(&self, group: Id) -> &NodeGroup {
        self.node_group(group).unwrap()
    }
}
