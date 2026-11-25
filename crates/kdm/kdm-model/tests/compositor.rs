//! Compositing groups through a replica: their tree and the render's choice of group written
//! and read back.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::kdm::{ActorId, DocId, Origin, Replica};
use kition_kit::session::{BlobSink, MemoryBlobs};
use kition_scene_kdm::mesh_blob::MeshCache;
use kition_scene_kdm::read::read_scene;
use kition_scene_kdm::schema;
use kition_scene_kdm::write::{Written, write_scene};
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
