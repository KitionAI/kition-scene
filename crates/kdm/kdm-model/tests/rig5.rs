//! Envelopes, the Armature modifier's options, and custom bone shapes through a replica: written
//! and read back.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::kdm::{ActorId, DocId, Origin, Replica};
use kition_kit::session::{BlobSink, MemoryBlobs};
use kition_scene_kdm::mesh_blob::MeshCache;
use kition_scene_kdm::read::read_scene;
use kition_scene_kdm::schema;
use kition_scene_kdm::write::{Written, write_scene};
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

/// A replica of the rigged scene.
fn create() -> (Replica, MemoryBlobs) {
    let mut blobs = MemoryBlobs::default();
    let (a, written) =
        Replica::create_with(schema(), DocId::new(), ActorId::from_u64(1).unwrap(), &Origin::system("create"), |tx| write_scene(tx, &rigged()))
            .unwrap();
    keep(&mut blobs, written);
    (a, blobs)
}

#[test]
fn envelopes_options_and_shapes_round_trip() {
    let (a, blobs) = create();
    let scene = rigged();
    let back = read(&a, &blobs);
    assert_eq!(back.object_named("Armature").unwrap().data, scene.object_named("Armature").unwrap().data);
    assert_eq!(back.object_named("Cube").unwrap().modifiers, scene.object_named("Cube").unwrap().modifiers);
}
