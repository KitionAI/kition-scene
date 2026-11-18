//! Rigid body constraints and force fields through a replica: written, read back, and merged;
//! a constraint whose body a peer deleted lets go of it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::kdm::{ActorId, DocId, Origin, Replica};
use kition_kit::session::{BlobSink, MemoryBlobs};
use kition_scene_kdm::mesh_blob::MeshCache;
use kition_scene_kdm::read::read_scene;
use kition_scene_kdm::write::{Written, stored_form, write_changes, write_scene};
use kition_scene_kdm::{ids, schema};
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

#[test]
fn a_constraint_lets_go_of_a_body_a_peer_deleted() {
    let (mut a, mut blobs_a) = create(&joined());
    let mut b = Replica::load(schema(), &a.snapshot().unwrap(), ActorId::random()).unwrap();
    let mut blobs_b = blobs_a.clone();
    // A deletes the second body; B makes the first body heavier and turns the wind up. (An
    // object's physics is one value: two people changing the same object's at once keep one.)
    edit(&mut a, &mut blobs_a, |scene| {
        let second = scene.object_named("Second").unwrap().id;
        scene.remove_objects(&[second]).unwrap();
    });
    edit(&mut b, &mut blobs_b, |scene| {
        let first = scene.object_named("First").unwrap().id;
        scene.object_mut(first).unwrap().physics.as_mut().unwrap().rigid_body.as_mut().unwrap().mass = 4.0;
        let wind = scene.object_named("Wind").unwrap().id;
        scene.object_mut(wind).unwrap().physics.as_mut().unwrap().field.as_mut().unwrap().strength = 3.0;
    });
    sync(&mut a, &mut b, &mut blobs_a, &mut blobs_b);
    assert_eq!(a.state_json().unwrap(), b.state_json().unwrap());
    let scene = read(&a, &blobs_a);
    assert_eq!(read(&b, &blobs_b), scene);
    scene.validate().unwrap();
    let constraint = scene.object_named("Hinge").unwrap().physics.clone().unwrap().rigid_constraint.unwrap();
    assert_eq!(constraint.kind, JointKind::Hinge);
    assert_eq!(scene.object_named("First").unwrap().physics.clone().unwrap().rigid_body.unwrap().mass, 4.0, "B's change stands");
    assert_eq!(constraint.object1, Some(scene.object_named("First").unwrap().id));
    assert_eq!(constraint.object2, None, "the deleted body is let go");
    assert_eq!(scene.object_named("Wind").unwrap().physics.clone().unwrap().field.unwrap().strength, 3.0);
}
