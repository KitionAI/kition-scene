//! Envelopes, the Armature modifier's options, and custom bone shapes through a replica: written,
//! read back, and merged; a bone drawn by an object a peer deleted gets its octahedron back under
//! either merge order.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::kdm::{ActorId, DocId, Origin, Replica};
use kition_kit::session::{BlobSink, MemoryBlobs};
use kition_scene_kdm::mesh_blob::MeshCache;
use kition_scene_kdm::read::read_scene;
use kition_scene_kdm::write::{Written, stored_form, write_changes, write_scene};
use kition_scene_kdm::{ids, schema};
use kition_scene_model::glam::Vec3;
use kition_scene_model::{Armature, ArmatureDeform, Bone, BoneShape, Modifier, ModifierKind, ObjectData, Scene};

fn keep(blobs: &mut MemoryBlobs, written: Written) {
    for blob in written.blobs {
        blobs.put(&blob.bytes, "application/octet-stream");
    }
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

/// The starter scene with an armature whose bone has an envelope and is drawn by the cube, and
/// the cube deformed by envelopes through an Armature modifier with its own vertex group.
fn rigged() -> Scene {
    let mut scene = Scene::starter();
    let cube = scene.object_named("Cube").unwrap().id;
    let mut bone = Bone::new("Bone", Vec3::ZERO, Vec3::Z);
    bone.head_radius = 0.3;
    bone.tail_radius = 0.2;
    bone.envelope_distance = 0.6;
    bone.envelope_weight = 0.8;
    bone.envelope_multiply = true;
    bone.shape = Some(BoneShape { translation: Vec3::new(0.0, 0.5, 0.0), rotation: Vec3::new(0.2, 0.0, 0.0), wire: true, ..BoneShape::new(cube) });
    let rig = scene.add_object("Armature", ObjectData::Armature(Armature { bones: vec![bone] }));
    let deform = ArmatureDeform { envelopes: true, vertex_group: Some("Mask".into()), invert_vertex_group: true, ..ArmatureDeform::default() };
    scene.object_mut(cube).unwrap().modifiers.push(Modifier::new(ModifierKind::Armature { object: Some(rig), preserve_volume: true, deform }));
    scene
}

/// Two replicas of the rigged scene with fixed actors, so a test can force who wins a register
/// both ways.
fn pair(first: u64, second: u64) -> ((Replica, MemoryBlobs), (Replica, MemoryBlobs)) {
    let mut blobs = MemoryBlobs::default();
    let (a, written) =
        Replica::create_with(schema(), DocId::new(), ActorId::from_u64(first).unwrap(), &Origin::system("create"), |tx| write_scene(tx, &rigged()))
            .unwrap();
    keep(&mut blobs, written);
    let b = Replica::load(schema(), &a.snapshot().unwrap(), ActorId::from_u64(second).unwrap()).unwrap();
    let other = blobs.clone();
    ((a, blobs), (b, other))
}

#[test]
fn envelopes_options_and_shapes_round_trip() {
    let ((a, blobs), _) = pair(1, 2);
    let scene = rigged();
    let back = read(&a, &blobs);
    assert_eq!(back.object_named("Armature").unwrap().data, scene.object_named("Armature").unwrap().data);
    assert_eq!(back.object_named("Cube").unwrap().modifiers, scene.object_named("Cube").unwrap().modifiers);
}

#[test]
fn a_shape_a_peer_deleted_lets_go_under_either_merge_order() {
    let mut kept_edits = 0;
    for (first, second) in [(1, 2), (2, 1), (7, 0x7fff_ffff_ffff_fff0), (0x7fff_ffff_ffff_fff0, 7)] {
        // Whichever side made more edits writes the armature last; both ways round.
        for busy_b in [false, true] {
            let ((mut a, mut blobs_a), (mut b, mut blobs_b)) = pair(first, second);
            let (busy, busy_blobs) = if busy_b { (&mut b, &mut blobs_b) } else { (&mut a, &mut blobs_a) };
            for step in 0..4 {
                edit(busy, busy_blobs, |scene| {
                    let light = scene.object_named("Light").unwrap().id;
                    scene.object_mut(light).unwrap().location.x += 0.1 * (step + 1) as f32;
                });
            }
            // A deletes the cube that draws the bone; B moves the shape and widens the envelope.
            edit(&mut a, &mut blobs_a, |scene| {
                let cube = scene.object_named("Cube").unwrap().id;
                scene.remove_objects(&[cube]).unwrap();
            });
            edit(&mut b, &mut blobs_b, |scene| {
                let rig = scene.object_named("Armature").unwrap().id;
                let ObjectData::Armature(armature) = &mut scene.object_mut(rig).unwrap().data else { panic!() };
                let bone = armature.bone_mut("Bone").unwrap();
                bone.head_radius = 0.5;
                bone.shape.as_mut().unwrap().scale = Vec3::splat(2.0);
            });
            sync(&mut a, &mut b, &mut blobs_a, &mut blobs_b);
            let label = format!("actors {first} and {second}, {} busier", if busy_b { "B" } else { "A" });
            assert_eq!(a.state_json().unwrap(), b.state_json().unwrap(), "{label}");
            let scene = read(&a, &blobs_a);
            scene.validate().unwrap();
            assert!(scene.object_named("Cube").is_none(), "{label}");
            let ObjectData::Armature(armature) = &scene.object_named("Armature").unwrap().data else { panic!() };
            let bone = armature.bone("Bone").unwrap();
            assert!(bone.shape.is_none(), "{label}: the bone lets go of the deleted cube");
            kept_edits += usize::from(bone.head_radius == 0.5);
        }
    }
    assert!(kept_edits > 0, "B's edit of the bone won some merges (the repair had a shape to clear)");
}
