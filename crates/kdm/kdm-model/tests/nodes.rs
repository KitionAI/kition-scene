//! Node groups through a replica: written whole, edited node by node and read back.
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
