//! NLA tracks and F-curve modifiers through a replica: written, read back, edited, and merged;
//! a strip whose action a peer removed goes, and tweak mode on it ends with the object's own
//! keys back.
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
    assert!(kition_scene_kdm::SCHEMA.contains("\"schemaVersion\": 6"));
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

#[test]
fn a_strip_whose_action_a_peer_removed_goes() {
    let (mut a, mut blobs_a) = create(&layered());
    let mut b = Replica::load(schema(), &a.snapshot().unwrap(), ActorId::random()).unwrap();
    let mut blobs_b = blobs_a.clone();
    // A removes the action Lift; B tweaks the strip playing it, keying a new frame.
    edit(&mut a, &mut blobs_a, |scene| {
        scene.animation.actions.retain(|action| action.name != "Lift");
        scene.nla_follow_action("Lift", None);
    });
    edit(&mut b, &mut blobs_b, |scene| {
        let cube = scene.object_named("Cube").unwrap().id;
        scene.object_mut(cube).unwrap().nla.tracks[0].strips[0].repeat = 3.0;
        scene.insert_key(cube, Property::Scale, 1.0).unwrap();
        scene.nla_tweak(cube, 1, 0, false).unwrap();
    });
    sync(&mut a, &mut b, &mut blobs_a, &mut blobs_b);
    assert_eq!(a.state_json().unwrap(), b.state_json().unwrap());
    let scene = read(&a, &blobs_a);
    assert_eq!(read(&b, &blobs_b), scene);
    scene.validate().unwrap();
    let cube = scene.object_named("Cube").unwrap();
    assert!(cube.nla.tweak.is_none(), "tweak mode on the removed action's strip ends");
    assert!(cube.nla.tracks.iter().all(|track| track.strips.iter().all(|strip| strip.action != "Lift")));
    let own: Vec<Property> = scene.animation.channels.iter().filter(|channel| channel.target == cube.id).map(|channel| channel.property).collect();
    assert_eq!(own, vec![Property::Scale; 3], "the keys put aside come back");
}

/// Two replicas of the layered scene with fixed actors, so a test can force who wins a register
/// both ways.
fn pair(first: u64, second: u64) -> ((Replica, MemoryBlobs), (Replica, MemoryBlobs)) {
    let mut blobs = MemoryBlobs::default();
    let (a, written) =
        Replica::create_with(schema(), DocId::new(), ActorId::from_u64(first).unwrap(), &Origin::system("create"), |tx| write_scene(tx, &layered()))
            .unwrap();
    keep(&mut blobs, written);
    let b = Replica::load(schema(), &a.snapshot().unwrap(), ActorId::from_u64(second).unwrap()).unwrap();
    let other = blobs.clone();
    ((a, blobs), (b, other))
}

#[test]
fn tweak_mode_ends_the_same_under_either_merge_order() {
    for (first, second) in [(1, 2), (2, 1), (7, 0x7fff_ffff_ffff_fff0), (0x7fff_ffff_ffff_fff0, 7)] {
        let ((mut a, mut blobs_a), (mut b, mut blobs_b)) = pair(first, second);
        // A removes the action Lift (and with it its strip); B tweaks that strip after keying scale.
        edit(&mut a, &mut blobs_a, |scene| {
            scene.animation.actions.retain(|action| action.name != "Lift");
            scene.nla_follow_action("Lift", None);
        });
        edit(&mut b, &mut blobs_b, |scene| {
            let cube = scene.object_named("Cube").unwrap().id;
            scene.object_mut(cube).unwrap().nla.tracks[0].strips[0].repeat = 3.0;
            scene.insert_key(cube, Property::Scale, 1.0).unwrap();
            scene.nla_tweak(cube, 1, 0, false).unwrap();
        });
        sync(&mut a, &mut b, &mut blobs_a, &mut blobs_b);
        assert_eq!(a.state_json().unwrap(), b.state_json().unwrap(), "actors {first} and {second}");
        let scene = read(&a, &blobs_a);
        scene.validate().unwrap();
        let cube = scene.object_named("Cube").unwrap();
        assert!(cube.nla.tweak.is_none(), "actors {first} and {second}");
        let own: Vec<Property> =
            scene.animation.channels.iter().filter(|channel| channel.target == cube.id).map(|channel| channel.property).collect();
        assert_eq!(own, vec![Property::Scale; 3], "actors {first} and {second}: the keys put aside come back");
    }
}

#[test]
fn tweak_mode_follows_its_strip_when_a_peer_removes_a_track_below() {
    for (first, second) in [(1, 2), (2, 1)] {
        let ((mut a, mut blobs_a), (mut b, mut blobs_b)) = pair(first, second);
        // A removes the bottom track (Slide); B tweaks Lift's strip on the track above it.
        edit(&mut a, &mut blobs_a, |scene| {
            let cube = scene.object_named("Cube").unwrap().id;
            scene.object_mut(cube).unwrap().nla.tracks.remove(0);
        });
        edit(&mut b, &mut blobs_b, |scene| {
            let cube = scene.object_named("Cube").unwrap().id;
            scene.nla_tweak(cube, 1, 0, false).unwrap();
        });
        sync(&mut a, &mut b, &mut blobs_a, &mut blobs_b);
        assert_eq!(a.state_json().unwrap(), b.state_json().unwrap(), "actors {first} and {second}");
        let scene = read(&a, &blobs_a);
        scene.validate().unwrap();
        let cube = scene.object_named("Cube").unwrap();
        assert_eq!(cube.nla.tweaked().map(|strip| strip.action.as_str()), Some("Lift"), "actors {first} and {second}");
        let own: Vec<Property> =
            scene.animation.channels.iter().filter(|channel| channel.target == cube.id).map(|channel| channel.property).collect();
        assert_eq!(own, vec![Property::Location; 3], "actors {first} and {second}: Lift's keys are being tweaked");
    }
}
