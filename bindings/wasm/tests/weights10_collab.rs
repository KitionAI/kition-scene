//! Whole physics-register edits converge before every peer deterministically rebuilds its cache.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use glam::Vec3;
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
    ObjectData, Primitive, Scene,
    physics::{Cloth, EffectorWeights, FieldKind, ForceField, ObjectPhysics, SoftBody},
};
fn read(replica: &Replica, blobs: &MemoryBlobs) -> Scene {
    read_scene(replica, blobs, &mut MeshCache::default()).unwrap()
}
#[test]
fn weights_and_fields_converge_in_both_actor_and_import_orders_twenty_times() {
    for _ in 0..20 {
        for variant in 0..4 {
            for (left, right) in [(11, 22), (22, 11)] {
                let mut scene = Scene::empty();
                scene.physics.frame_end = 6;
                scene.physics.gravity = Vec3::ZERO;
                let mesh = Primitive::Plane { size: 1.0 }.build().unwrap();
                let cloth = scene.add_object("Cloth", ObjectData::Mesh(mesh.clone()));
                let soft = scene.add_object("Soft", ObjectData::Mesh(mesh));
                scene.object_mut(cloth).unwrap().physics = Some(ObjectPhysics { cloth: Some(Cloth::default()), ..Default::default() });
                scene.object_mut(soft).unwrap().physics =
                    Some(ObjectPhysics { soft_body: Some(SoftBody { goal: false, edges: false, ..Default::default() }), ..Default::default() });
                let field = scene.add_object("Field", ObjectData::Empty { size: 1.0 });
                scene.object_mut(field).unwrap().location = Vec3::new(3.0, 0.0, 0.0);
                scene.object_mut(field).unwrap().physics =
                    Some(ObjectPhysics { field: Some(ForceField::new(FieldKind::Vortex)), ..Default::default() });
                let mut blobs = MemoryBlobs::default();
                let (mut a, written) =
                    Replica::create_with(schema(), DocId::new(), ActorId::from_u64(left).unwrap(), &Origin::system("create"), |tx| {
                        write_scene(tx, &scene)
                    })
                    .unwrap();
                for blob in written.blobs {
                    blobs.put(&blob.bytes, "application/octet-stream");
                }
                let base = a.snapshot().unwrap();
                let version = a.version();
                let mut b = Replica::load(schema(), &base, ActorId::from_u64(right).unwrap()).unwrap();
                let before = read(&a, &blobs);
                let mut aa = before.clone();
                let mut bb = before.clone();
                aa.object_mut(cloth).unwrap().physics.as_mut().unwrap().cloth.as_mut().unwrap().effector_weights = if variant == 3 {
                    EffectorWeights { all: 200.0, vortex: 200.0, ..Default::default() }
                } else {
                    EffectorWeights { gravity: 0.0, vortex: 0.0, ..Default::default() }
                };
                match variant {
                    0 => bb.object_mut(soft).unwrap().physics.as_mut().unwrap().soft_body.as_mut().unwrap().effector_weights.gravity = -0.5,
                    1 => bb.object_mut(cloth).unwrap().physics.as_mut().unwrap().cloth.as_mut().unwrap().effector_weights.wind = 0.3,
                    2 => bb.objects.retain(|object| object.id != field),
                    _ => bb.object_mut(field).unwrap().physics.as_mut().unwrap().field.as_mut().unwrap().strength = 10000.0,
                }
                aa.validate().unwrap();
                bb.validate().unwrap();
                a.transact(&Origin::person("left"), |tx| write_changes(tx, &before, &aa, None)).unwrap();
                b.transact(&Origin::person("right"), |tx| write_changes(tx, &before, &bb, None)).unwrap();
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
                let reverse = read(&d, &blobs);
                merged.validate().unwrap();
                reverse.validate().unwrap();
                assert_eq!(merged, reverse);
                let merged_cloth = merged.object(cloth).unwrap().physics.as_ref().unwrap().cloth.as_ref().unwrap();
                if variant == 1 {
                    assert!(
                        merged_cloth == aa.object(cloth).unwrap().physics.as_ref().unwrap().cloth.as_ref().unwrap()
                            || merged_cloth == bb.object(cloth).unwrap().physics.as_ref().unwrap().cloth.as_ref().unwrap()
                    );
                } else {
                    assert_eq!(merged_cloth, aa.object(cloth).unwrap().physics.as_ref().unwrap().cloth.as_ref().unwrap());
                }
                if variant == 0 {
                    assert_eq!(merged.object(soft).unwrap().physics, bb.object(soft).unwrap().physics);
                }
                if variant == 2 {
                    assert!(merged.object(field).is_err());
                }
                let result = kition_scene_physics::bake(&merged);
                let reverse_result = kition_scene_physics::bake(&reverse);
                if variant == 3 {
                    assert!(result.is_err(), "valid concurrent settings may diverge only in derived simulation");
                    assert_eq!(result.unwrap_err(), reverse_result.unwrap_err());
                } else {
                    let result = result.unwrap();
                    let reverse_result = reverse_result.unwrap();
                    assert_eq!(result.cloths, reverse_result.cloths);
                    assert!(result.cloths.values().flatten().flatten().all(|point| point.is_finite()));
                }
                let reopened = Replica::load(schema(), &c.snapshot().unwrap(), ActorId::from_u64(300).unwrap()).unwrap();
                assert_eq!(read(&reopened, &blobs), merged);
            }
        }
    }
}
