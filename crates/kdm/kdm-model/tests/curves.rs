//! Curve and text objects through a replica: written whole, read back, and merged with a peer's
//! edits of other objects and of the same one.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::kdm::{ActorId, DocId, Origin, Replica};
use kition_kit::session::{BlobSink, MemoryBlobs};
use kition_scene_kdm::mesh_blob::MeshCache;
use kition_scene_kdm::read::read_scene;
use kition_scene_kdm::write::{Written, stored_form, write_changes, write_scene};
use kition_scene_kdm::{ids, schema};
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

#[test]
fn concurrent_curve_and_text_edits_converge() {
    let (mut a, mut blobs_a) = create(&with_shapes());
    let mut b = Replica::load(schema(), &a.snapshot().unwrap(), ActorId::random()).unwrap();
    let mut blobs_b = blobs_a.clone();
    // A bevels the circle deeper and adds a path; B retypes the text and moves the circle.
    edit(&mut a, &mut blobs_a, |scene| {
        let id = scene.object_named("BezierCircle").unwrap().id;
        if let ObjectData::Curve(curve) = &mut scene.object_mut(id).unwrap().data {
            curve.bevel_depth = 0.25;
        }
        scene.add_object("NurbsPath", ObjectData::Curve(CurvePrimitive::Path.build(1.0)));
    });
    edit(&mut b, &mut blobs_b, |scene| {
        let text = scene.object_named("Text").unwrap().id;
        if let ObjectData::Text(text) = &mut scene.object_mut(text).unwrap().data {
            text.body = "Kition".into();
        }
        let circle = scene.object_named("BezierCircle").unwrap().id;
        scene.object_mut(circle).unwrap().location.z = 2.0;
    });
    sync(&mut a, &mut b, &mut blobs_a, &mut blobs_b);
    assert_eq!(a.state_json().unwrap(), b.state_json().unwrap());
    let scene = read(&a, &blobs_a);
    assert_eq!(read(&b, &blobs_b), scene);
    let circle = scene.object_named("BezierCircle").unwrap();
    let ObjectData::Curve(curve) = &circle.data else { panic!("not a curve") };
    assert_eq!((curve.bevel_depth, circle.location.z), (0.25, 2.0), "both edits kept");
    let ObjectData::Text(text) = &scene.object_named("Text").unwrap().data else { panic!("not a text") };
    assert_eq!(text.body, "Kition");
    assert!(scene.object_named("NurbsPath").is_some());
    scene.validate().unwrap();
}
