//! Motor settings are one physics JSON register; deletion repair preserves drive settings.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use kition_kit::{
    kdm::{ActorId, DocId, Origin, Replica},
    session::{BlobSink, MemoryBlobs},
};
use kition_scene_kdm::{
    mesh_blob::MeshCache,
    read::read_scene,
    schema,
    write::{write_changes, write_scene},
};
use kition_scene_model::{
    ObjectData, Scene,
    physics::{JointKind, ObjectPhysics, RigidBody, RigidConstraint},
};

fn scene() -> Scene {
    let mut scene = Scene::starter();
    let body = scene.object_named("Cube").unwrap().id;
    scene.object_mut(body).unwrap().physics = Some(ObjectPhysics { rigid_body: Some(RigidBody::default()), ..ObjectPhysics::default() });
    let id = scene.add_object("Motor", ObjectData::Empty { size: 1.0 });
    scene.object_mut(id).unwrap().physics = Some(ObjectPhysics {
        rigid_constraint: Some(RigidConstraint { kind: JointKind::Motor, object1: Some(body), linear_motor: true, ..RigidConstraint::default() }),
        ..ObjectPhysics::default()
    });
    scene
}
fn read(replica: &Replica, blobs: &MemoryBlobs) -> Scene {
    read_scene(replica, blobs, &mut MeshCache::default()).unwrap()
}
fn settings(scene: &Scene) -> RigidConstraint {
    scene.object_named("Motor").unwrap().physics.as_ref().unwrap().rigid_constraint.clone().unwrap()
}
#[test]
fn motor_register_merges_and_body_deletion_repairs_in_both_orders_twenty_times() {
    for _ in 0..20 {
        for remove_body in [false, true] {
            for (left, right) in [(11, 22), (22, 11)] {
                let mut blobs = MemoryBlobs::default();
                let (mut a, written) =
                    Replica::create_with(schema(), DocId::new(), ActorId::from_u64(left).unwrap(), &Origin::system("create"), |tx| {
                        write_scene(tx, &scene())
                    })
                    .unwrap();
                for blob in written.blobs {
                    blobs.put(&blob.bytes, "application/octet-stream");
                }
                let base = a.snapshot().unwrap();
                let version = a.version();
                let mut b = Replica::load(schema(), &base, ActorId::from_u64(right).unwrap()).unwrap();
                let before = read(&a, &blobs);
                let mut after_a = before.clone();
                let motor =
                    after_a.objects.iter_mut().find(|o| o.name == "Motor").unwrap().physics.as_mut().unwrap().rigid_constraint.as_mut().unwrap();
                motor.linear_target_velocity = -7.0;
                motor.angular_motor = true;
                motor.angular_max_impulse = 3.0;
                let mut after_b = before.clone();
                if remove_body {
                    let body = after_b.object_named("Cube").unwrap().id;
                    after_b.objects.retain(|o| o.id != body);
                } else {
                    let motor =
                        after_b.objects.iter_mut().find(|o| o.name == "Motor").unwrap().physics.as_mut().unwrap().rigid_constraint.as_mut().unwrap();
                    motor.linear_target_velocity = 4.0;
                    motor.linear_max_impulse = 0.25;
                }
                a.transact(&Origin::person("left"), |tx| write_changes(tx, &before, &after_a, None)).unwrap();
                b.transact(&Origin::person("right"), |tx| write_changes(tx, &before, &after_b, None)).unwrap();
                let ca = a.export_changes(&version).unwrap();
                let cb = b.export_changes(&version).unwrap();
                let mut c = Replica::load(schema(), &base, ActorId::from_u64(100).unwrap()).unwrap();
                let mut d = Replica::load(schema(), &base, ActorId::from_u64(200).unwrap()).unwrap();
                c.import_changes(&ca).unwrap();
                c.import_changes(&cb).unwrap();
                d.import_changes(&cb).unwrap();
                d.import_changes(&ca).unwrap();
                for replica in [&mut c, &mut d] {
                    replica.transact(&Origin::system("repair"), kition_scene_kdm::repair::repair).unwrap();
                }
                assert_eq!(c.state_json().unwrap(), d.state_json().unwrap());
                let merged = read(&c, &blobs);
                merged.validate().unwrap();
                assert_eq!(merged, read(&d, &blobs));
                let motor = settings(&merged);
                if remove_body {
                    assert!(motor.object1.is_none());
                    assert_eq!(motor.linear_target_velocity, -7.0);
                    assert!(motor.angular_motor);
                    assert_eq!(motor.angular_max_impulse, 3.0);
                } else {
                    assert!(motor == settings(&after_a) || motor == settings(&after_b), "a complete physics register wins");
                }
            }
        }
    }
}
