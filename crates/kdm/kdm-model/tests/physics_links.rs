//! Rigid body constraints and force fields through a replica: written and read back.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::kdm::{ActorId, DocId, Origin, Replica};
use kition_kit::session::{BlobSink, MemoryBlobs};
use kition_scene_kdm::mesh_blob::MeshCache;
use kition_scene_kdm::read::read_scene;
use kition_scene_kdm::schema;
use kition_scene_kdm::write::{Written, write_scene};
use kition_scene_model::physics::{FieldKind, ForceField, JointKind, ObjectPhysics, RigidBody, RigidConstraint};
use kition_scene_model::{ObjectData, Primitive, Scene};

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

/// Two rigid bodies joined by a hinge on an empty, and a wind.
fn joined() -> Scene {
    let mut scene = Scene::starter();
    let body = || Some(ObjectPhysics { rigid_body: Some(RigidBody::default()), ..ObjectPhysics::default() });
    let first = scene.add_object("First", ObjectData::Mesh(Primitive::Cube { size: 1.0 }.build().unwrap()));
    scene.object_mut(first).unwrap().physics = body();
    let second = scene.add_object("Second", ObjectData::Mesh(Primitive::Cube { size: 1.0 }.build().unwrap()));
    scene.object_mut(second).unwrap().physics = body();
    let hinge = scene.add_object("Hinge", ObjectData::Empty { size: 1.0 });
    let constraint = RigidConstraint { kind: JointKind::Hinge, object1: Some(first), object2: Some(second), ..RigidConstraint::default() };
    scene.object_mut(hinge).unwrap().physics = Some(ObjectPhysics { rigid_constraint: Some(constraint), ..ObjectPhysics::default() });
    let wind = scene.add_object("Wind", ObjectData::Empty { size: 1.0 });
    scene.object_mut(wind).unwrap().physics = Some(ObjectPhysics { field: Some(ForceField::new(FieldKind::Wind)), ..ObjectPhysics::default() });
    scene
}

#[test]
fn constraints_and_fields_round_trip() {
    let scene = joined();
    let (replica, blobs) = create(&scene);
    let back = read(&replica, &blobs);
    for name in ["Hinge", "Wind"] {
        assert_eq!(back.object_named(name).unwrap().physics, scene.object_named(name).unwrap().physics, "{name}");
    }
}
