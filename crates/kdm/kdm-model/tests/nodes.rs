//! Node groups through a replica: written whole, edited node by node and read back; an agent's
//! draft merged while the person edited, and the repair after it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::kdm::{ActorId, DocId, Origin, Replica};
use kition_kit::session::{BlobSink, KitModel, MemoryBlobs};
use kition_scene_kdm::mesh_blob::MeshCache;
use kition_scene_kdm::read::read_scene;
use kition_scene_kdm::write::{Written, stored_form, write_changes, write_scene};
use kition_scene_kdm::{SceneModel, ids, root_id, schema};
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

/// The person's `live` replica and an agent's draft of it (a fork at its version, data model
/// 6.6): the draft takes `drafted`, the person meanwhile `meanwhile`, and the person merges the
/// draft as `KitSession::merge_changes` does: one change of the person's, then the repair.
fn merge_draft(live: &mut Replica, blobs: &mut MemoryBlobs, drafted: impl FnOnce(&mut Scene), meanwhile: impl FnOnce(&mut Scene)) {
    let base = live.version();
    let mut draft = live.fork(ActorId::random()).unwrap();
    let mut draft_blobs = blobs.clone();
    edit(&mut draft, &mut draft_blobs, drafted);
    edit(live, blobs, meanwhile);
    for (hash, bytes) in draft_blobs.0 {
        blobs.0.entry(hash).or_insert(bytes);
    }
    live.merge_changes(&draft.export_changes(&base).unwrap(), &Origin::agent("agent:test")).unwrap();
    live.transact(&Origin::system("repair"), |tx| Ok(kition_scene_kdm::repair::repair(tx).unwrap())).unwrap();
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

/// Links a merge puts together that do not fit go. The agent links the transform into a second
/// transform while the person links the second into the first (each inserts its own link record
/// into the group's links, rule D6): together a cycle, and two links into the transform's
/// geometry; one of them goes. Then a link the draft puts into the output's single input
/// replaces the one there.
#[test]
fn links_a_merge_closes_into_a_cycle_resolve() {
    let (scene, group, [input, output, transform]) = with_group();
    let second = 9_000;
    let mut start = scene.clone();
    start.node_groups[0].nodes.push(Node { id: second, kind: "transform".into(), location: [0.0, 0.0], values: Default::default() });
    let (mut a, mut blobs_a) = create(&start);
    merge_draft(
        &mut a,
        &mut blobs_a,
        |scene| scene.node_groups[0].links.push(link(transform, "Geometry", second, "Geometry")),
        |scene| {
            let tree = &mut scene.node_groups[0];
            tree.links.retain(|link| link.to != transform);
            tree.links.push(link(second, "Geometry", transform, "Geometry"));
        },
    );
    let merged = read(&a, &blobs_a);
    let tree = merged.node(group);
    assert!(!tree.links.iter().any(|link| kition_scene_kdm::nodes::reaches(&tree.links, link.to, link.from)), "no cycle");
    assert_eq!(tree.links.iter().filter(|link| link.to == transform || link.to == second).count(), 1, "one of the two links went");
    assert_eq!(kition_scene_kdm::nodes::fitting_links(tree, &merged.node_groups), tree.links);
    merged.validate().unwrap();
    // A link into the output's one input, drafted: one stays.
    let (mut b, mut blobs_b) = create(&scene);
    merge_draft(
        &mut b,
        &mut blobs_b,
        |scene| {
            let tree = &mut scene.node_groups[0];
            tree.links.retain(|link| link.to != output);
            tree.links.push(link(input, "Geometry", output, "Geometry"));
        },
        |_| {},
    );
    let scene = read(&b, &blobs_b);
    assert_eq!(scene.node(group).links.iter().filter(|link| link.to == output).count(), 1);
    scene.validate().unwrap();
}

/// An Object Info node the agent's draft adds, reading the light, lets go of it when the person
/// deleted the light meanwhile: the draft writes only its own node (rule D8 lets it merge), and
/// the repair clears the setting. (The other half of the old concurrent test, deleting the group
/// the draft adds the node to, cannot arise any more: the merge refuses a draft that places an
/// object in one the document deleted, rule D8.)
#[test]
fn an_object_info_node_lets_go_of_an_object_deleted_meanwhile() {
    let (scene, group, _) = with_group();
    let (mut a, mut blobs_a) = create(&scene);
    let mut info = 0;
    merge_draft(
        &mut a,
        &mut blobs_a,
        |scene| {
            info = scene.take_id();
            let light = scene.object_named("Light").unwrap().id;
            scene.node_groups[0].nodes.push(Node {
                id: info,
                kind: "objectInfo".into(),
                location: [0.0, 0.0],
                values: [("Object".to_owned(), json!(light))].into(),
            });
        },
        |scene| {
            let light = scene.object_named("Light").unwrap().id;
            scene.remove_objects(&[light]).unwrap();
        },
    );
    let scene = read(&a, &blobs_a);
    assert!(scene.object_named("Light").is_none());
    assert!(scene.node(group).node(info).unwrap().values.is_empty());
    scene.validate().unwrap();
}

/// Groups the draft and the person put into each other at once (each adds a Group node to a
/// different group, so the merge applies) would each use itself: the repair breaks the loop.
#[test]
fn groups_put_into_each_other_by_a_merge_resolve() {
    let (scene, group, inner) = with_inner();
    let (mut a, mut blobs_a) = create(&scene);
    let (mut in_outer, mut in_inner) = (0, 0);
    merge_draft(
        &mut a,
        &mut blobs_a,
        |scene| {
            in_outer = scene.take_id();
            scene.node_group_mut(group).unwrap().nodes.push(group_node(in_outer, inner));
        },
        |scene| {
            in_inner = scene.take_id();
            let tree = scene.node_group_mut(inner).unwrap();
            tree.nodes.push(group_node(in_inner, group));
            let output = tree.nodes.iter().find(|node| node.kind == "groupOutput").unwrap().id;
            tree.links.retain(|link| link.to != output || link.to_socket != "Geometry");
            tree.links.push(link(in_inner, "Geometry", output, "Geometry"));
        },
    );
    let scene = read(&a, &blobs_a);
    scene.validate().unwrap();
    // The cube's group comes first in key order: its use stays, Inner's lets go (and its link
    // from the Group node's geometry goes with the socket).
    assert_eq!(scene.node(group).node(in_outer).unwrap().values.get("group"), Some(&json!(inner)));
    assert!(!scene.node(inner).node(in_inner).unwrap().values.contains_key("group"));
    assert!(!scene.node(inner).links.iter().any(|link| link.from == in_inner));
}

/// A node listed twice in its group (a state only a file written by another program can hold:
/// a merge refuses two insertions of one item, rule D6) keeps its first place when the repair
/// runs.
#[test]
fn repair_keeps_the_first_place_of_a_node_listed_twice() {
    let (scene, group, _) = with_group();
    let (mut replica, blobs) = create(&scene);
    let root = root_id();
    let record = replica.list(&root, "nodeGroups").unwrap()[0].clone();
    let listed = replica.list(&record, "nodes").unwrap();
    let first = listed[0].clone();
    replica.transact(&Origin::system("import"), |tx| tx.list_insert(&record, "nodes", listed.len(), &first)).unwrap();
    assert_eq!(replica.list(&record, "nodes").unwrap().len(), listed.len() + 1, "the control: the state holds the node twice");
    replica.transact(&Origin::system("repair"), |tx| Ok(kition_scene_kdm::repair::repair(tx).unwrap())).unwrap();
    assert_eq!(replica.list(&record, "nodes").unwrap(), listed);
    assert_eq!(read(&replica, &blobs).node(group), scene.node(group));
}

trait GroupOf {
    fn node(&self, group: Id) -> &NodeGroup;
}

impl GroupOf for Scene {
    fn node(&self, group: Id) -> &NodeGroup {
        self.node_group(group).unwrap()
    }
}
