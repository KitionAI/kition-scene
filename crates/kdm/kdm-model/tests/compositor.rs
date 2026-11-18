//! Compositing groups through a replica: their tree and the render's choice of group written
//! and read back, a peer's edits to their nodes merged, and a group a peer deleted let go of.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::kdm::{ActorId, DocId, Origin, Replica};
use kition_kit::session::{BlobSink, MemoryBlobs};
use kition_scene_kdm::mesh_blob::MeshCache;
use kition_scene_kdm::read::read_scene;
use kition_scene_kdm::write::{Written, stored_form, write_changes, write_scene};
use kition_scene_kdm::{ids, schema};
use kition_scene_model::node_tree::Tree;
use kition_scene_model::{Id, Link, Node, NodeGroup, Scene};
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

fn peers(scene: &Scene) -> (Replica, Replica, MemoryBlobs, MemoryBlobs) {
    let (a, blobs_a) = create(scene);
    let b = Replica::load(schema(), &a.snapshot().unwrap(), ActorId::random()).unwrap();
    let blobs_b = blobs_a.clone();
    (a, b, blobs_a, blobs_b)
}

/// The starter scene with a compositing group (Render Layers, Glare, output) renders use.
fn with_compositor() -> (Scene, Id, Id) {
    let mut scene = Scene::starter();
    let (group, input, output, glare) = (scene.take_id(), scene.take_id(), scene.take_id(), scene.take_id());
    let mut tree = NodeGroup::compositor(group, "Compositing", input, output);
    tree.nodes.push(Node { id: glare, kind: "glare".into(), location: [0.0, 0.0], values: [("Type".to_owned(), json!("bloom"))].into() });
    tree.links = vec![
        Link { from: input, from_socket: "Image".into(), to: glare, to_socket: "Image".into() },
        Link { from: glare, from_socket: "Image".into(), to: output, to_socket: "Image".into() },
    ];
    scene.node_groups.push(tree);
    scene.render.compositor = Some(group);
    scene.validate().unwrap();
    (scene, group, glare)
}

#[test]
fn a_compositing_group_and_the_renders_choice_of_it_round_trip() {
    let (mut scene, group, glare) = with_compositor();
    scene.render.compositing = false;
    let curves = json!({ "c": [[0.05, 0.1], [0.4, 0.6], [0.95, 0.9]] });
    scene.node_group_mut(group).unwrap().node_mut(glare).unwrap().values.insert("Tint".into(), json!([0.9, 0.5, 0.25, 0.75]));
    let id = scene.take_id();
    scene.node_group_mut(group).unwrap().nodes.push(Node {
        id,
        kind: "rgbCurves".into(),
        location: [10.5, 0.0],
        values: [("curves".to_owned(), curves)].into(),
    });
    let (replica, blobs) = create(&scene);
    let back = read(&replica, &blobs);
    assert_eq!(back.node_groups, scene.node_groups);
    assert_eq!(back.node_groups[0].tree, Tree::Compositor);
    assert_eq!(back.render, scene.render);
    // A geometry group stays without a tree in the replica, as files before compositing have it.
    let state = replica.state_json().unwrap().to_string();
    assert!(state.contains("\"tree\":\"compositor\""), "{state}");
}

#[test]
fn peers_edit_different_compositing_nodes_and_both_edits_stay() {
    let (scene, group, glare) = with_compositor();
    let (mut a, mut b, mut blobs_a, mut blobs_b) = peers(&scene);
    edit(&mut a, &mut blobs_a, |scene| {
        scene.node_group_mut(group).unwrap().node_mut(glare).unwrap().values.insert("Strength".into(), json!(0.5));
    });
    let mut exposure = 0;
    edit(&mut b, &mut blobs_b, |scene| {
        exposure = scene.take_id();
        scene.node_group_mut(group).unwrap().nodes.push(Node {
            id: exposure,
            kind: "exposure".into(),
            location: [0.0, 0.0],
            values: Default::default(),
        });
        scene.render.compositing = false;
    });
    sync(&mut a, &mut b, &mut blobs_a, &mut blobs_b);
    assert_eq!(a.state_json().unwrap(), b.state_json().unwrap());
    let scene = read(&a, &blobs_a);
    let tree = scene.node_group(group).unwrap();
    assert_eq!(tree.node(glare).unwrap().values["Strength"], json!(0.5));
    assert!(tree.node(exposure).is_some());
    assert!(!scene.render.compositing);
    scene.validate().unwrap();
}

#[test]
fn renders_let_go_of_a_compositing_group_a_peer_deleted() {
    let (mut scene, group, glare) = with_compositor();
    scene.render.compositor = None;
    let (mut a, mut b, mut blobs_a, mut blobs_b) = peers(&scene);
    // A deletes the group; B, at the same time, makes renders use it and edits a node in it.
    edit(&mut a, &mut blobs_a, |scene| scene.node_groups.clear());
    edit(&mut b, &mut blobs_b, |scene| {
        scene.render.compositor = Some(group);
        scene.node_group_mut(group).unwrap().node_mut(glare).unwrap().values.insert("Size".into(), json!(0.8));
    });
    sync(&mut a, &mut b, &mut blobs_a, &mut blobs_b);
    assert_eq!(a.state_json().unwrap(), b.state_json().unwrap());
    let scene = read(&a, &blobs_a);
    assert!(scene.node_groups.is_empty());
    assert_eq!(scene.render.compositor, None);
    scene.validate().unwrap();
}
