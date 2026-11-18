//! Object references a peer's delete leaves behind (C8-scene-engine group 3): a modifier's
//! object, a curve's bevel and taper objects and the curve a text follows let go of an object
//! a peer deleted, as deleting it locally does. Case `regressions/issue-delete-object-references`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::kdm::{ActorId, DocId, Origin, Replica};
use kition_kit::session::{BlobSink, MemoryBlobs};
use kition_scene_kdm::mesh_blob::MeshCache;
use kition_scene_kdm::read::read_scene;
use kition_scene_kdm::write::{Written, stored_form, write_changes, write_scene};
use kition_scene_kdm::{ids, schema};
use kition_scene_model::{Curve, Modifier, ModifierKind, ObjectData, Primitive, Scene, Text};

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

/// A cube, a tool mesh, a profile curve, a path curve and a text.
fn start() -> Scene {
    let mut scene = Scene::starter();
    scene.add_object("Tool", ObjectData::Mesh(Primitive::Cube { size: 1.0 }.build().unwrap()));
    scene.add_object("Profile", ObjectData::Curve(Curve::default()));
    scene.add_object("Path", ObjectData::Curve(Curve::default()));
    scene.add_object("Words", ObjectData::Text(Text::default()));
    scene
}

#[test]
fn references_let_go_of_an_object_a_peer_deleted() {
    let (mut a, mut blobs_a) = create(&start());
    let mut b = Replica::load(schema(), &a.snapshot().unwrap(), ActorId::random()).unwrap();
    let mut blobs_b = blobs_a.clone();
    // A deletes the tool and the profile; B, at the same time, makes them a boolean's operand,
    // the path's bevel and taper and the curve the text follows.
    edit(&mut a, &mut blobs_a, |scene| {
        let gone = [scene.object_named("Tool").unwrap().id, scene.object_named("Profile").unwrap().id];
        scene.remove_objects(&gone).unwrap();
    });
    edit(&mut b, &mut blobs_b, |scene| {
        let (tool, profile) = (scene.object_named("Tool").unwrap().id, scene.object_named("Profile").unwrap().id);
        let cube = scene.object_named("Cube").unwrap().id;
        scene.object_mut(cube).unwrap().modifiers.push(Modifier::new(ModifierKind::Boolean { operation: Default::default(), object: Some(tool) }));
        let path = scene.object_named("Path").unwrap().id;
        if let ObjectData::Curve(curve) = &mut scene.object_mut(path).unwrap().data {
            curve.bevel_object = Some(profile);
            curve.taper_object = Some(profile);
        }
        let words = scene.object_named("Words").unwrap().id;
        if let ObjectData::Text(text) = &mut scene.object_mut(words).unwrap().data {
            text.follow_curve = Some(profile);
        }
    });
    sync(&mut a, &mut b, &mut blobs_a, &mut blobs_b);
    assert_eq!(a.state_json().unwrap(), b.state_json().unwrap());
    let scene = read(&a, &blobs_a);
    assert_eq!(read(&b, &blobs_b), scene);
    scene.validate().unwrap();
    let cube = scene.object_named("Cube").unwrap();
    assert_eq!(cube.modifiers.len(), 1, "B's modifier stands");
    assert_eq!(cube.modifiers[0].kind.object(), None, "the deleted tool is let go");
    let ObjectData::Curve(curve) = &scene.object_named("Path").unwrap().data else { panic!() };
    assert_eq!((curve.bevel_object, curve.taper_object), (None, None));
    let ObjectData::Text(text) = &scene.object_named("Words").unwrap().data else { panic!() };
    assert_eq!(text.follow_curve, None);
}
