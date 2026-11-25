//! Curve and text objects through a replica: written whole and read back.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::kdm::{ActorId, DocId, Origin, Replica};
use kition_kit::session::{BlobSink, MemoryBlobs};
use kition_scene_kdm::mesh_blob::MeshCache;
use kition_scene_kdm::read::read_scene;
use kition_scene_kdm::schema;
use kition_scene_kdm::write::{Written, write_scene};
use kition_scene_model::{CurvePrimitive, ObjectData, Scene, Text};

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

fn with_shapes() -> Scene {
    let mut scene = Scene::starter();
    let mut curve = CurvePrimitive::Circle.build(1.0);
    curve.bevel_depth = 0.1;
    scene.add_object("BezierCircle", ObjectData::Curve(curve));
    scene.add_object("Text", ObjectData::Text(Text { body: "Hi\nYou".into(), extrude: 0.05, ..Text::default() }));
    scene
}

#[test]
fn curves_and_texts_round_trip() {
    let scene = with_shapes();
    let (replica, blobs) = create(&scene);
    let back = read(&replica, &blobs);
    for name in ["BezierCircle", "Text"] {
        assert_eq!(back.object_named(name).unwrap().data, scene.object_named(name).unwrap().data, "{name}");
    }
    assert_eq!(back.object_named("BezierCircle").unwrap().data.kind(), "curve");
    assert_eq!(back.object_named("Text").unwrap().data.kind(), "text");
    back.validate().unwrap();
    // A loaded snapshot holds the same.
    let loaded = Replica::load(schema(), &replica.snapshot().unwrap(), ActorId::random()).unwrap();
    assert_eq!(read(&loaded, &blobs).object_named("Text").unwrap().data, scene.object_named("Text").unwrap().data);
}
