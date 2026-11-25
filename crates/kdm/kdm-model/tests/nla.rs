//! NLA tracks and F-curve modifiers through a replica: written, read back and edited.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::kdm::{ActorId, DocId, Origin, Replica};
use kition_kit::session::{BlobSink, MemoryBlobs};
use kition_scene_kdm::mesh_blob::MeshCache;
use kition_scene_kdm::read::read_scene;
use kition_scene_kdm::write::{Written, stored_form, write_changes, write_scene};
use kition_scene_kdm::{ids, schema};
use kition_scene_model::Scene;
use kition_scene_model::animation::Property;
use kition_scene_model::fmodifier::{FModifier, FModifierKind};
use kition_scene_model::nla::BlendType;

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

/// The starter scene with the cube's keys pushed down into a strip, a second action on a track
/// above it, and a Cycles modifier on the first action's location X.
fn layered() -> Scene {
    let mut scene = Scene::starter();
    let cube = scene.object_named("Cube").unwrap().id;
    scene.insert_key(cube, Property::Location, 1.0).unwrap();
    scene.object_mut(cube).unwrap().location.x = 4.0;
    scene.insert_key(cube, Property::Location, 21.0).unwrap();
    scene.animation.channels[0].modifiers.push(FModifier::new(FModifierKind::new("cycles").unwrap()));
    let rise = scene.nla_push_down(cube, Some("Slide")).unwrap();
    assert_eq!(rise, "Slide");
    scene.object_mut(cube).unwrap().location.z = 2.0;
    scene.insert_key(cube, Property::Location, 1.0).unwrap();
    scene.nla_push_down(cube, Some("Lift")).unwrap();
    let nla = &mut scene.object_mut(cube).unwrap().nla;
    nla.tracks[1].strips[0].blend = BlendType::Add;
    nla.tracks[1].strips[0].influence = Some(0.5);
    scene.validate().unwrap();
    scene
}

#[test]
fn tracks_and_modifiers_round_trip() {
    assert!(kition_scene_kdm::SCHEMA.contains("\"schemaVersion\": 1"));
    let scene = layered();
    let (replica, blobs) = create(&scene);
    let back = read(&replica, &blobs);
    let cube = scene.object_named("Cube").unwrap();
    assert_eq!(back.object_named("Cube").unwrap().nla, cube.nla);
    assert_eq!(back.animation.actions, scene.animation.actions);
    assert_eq!(back.animation.actions[0].channels[0].modifiers.len(), 1);
    assert_eq!(back.at_frame(31.0).object_named("Cube").unwrap().location, scene.at_frame(31.0).object_named("Cube").unwrap().location);
}

#[test]
fn an_nla_edit_is_one_register_and_frames_stay_local() {
    let (mut replica, mut blobs) = create(&layered());
    edit(&mut replica, &mut blobs, |scene| {
        let cube = scene.object_named("Cube").unwrap().id;
        scene.object_mut(cube).unwrap().nla.tracks[0].strips[0].scale = 2.0;
        // The view shows the NLA's values at the local frame; the replica keeps the stored ones.
        scene.go_to_frame(11.0);
    });
    let back = read(&replica, &blobs);
    let cube = back.object_named("Cube").unwrap();
    assert_eq!(cube.nla.tracks[0].strips[0].scale, 2.0);
    assert_eq!(cube.location, layered().object_named("Cube").unwrap().location, "animated values are stored as they were");
}
