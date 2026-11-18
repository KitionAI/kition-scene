//! Fixed actor and arrival orders exercise deletion repair and whole-register boid edits.
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
    particles::{
        Kind, ParticlePhysics, ParticleSystem,
        boids::{Rule, RuleKind},
    },
    physics::{Collision, FieldKind, ForceField, ObjectPhysics},
};
fn read(replica: &Replica, blobs: &MemoryBlobs) -> Scene {
    read_scene(replica, blobs, &mut MeshCache::default()).unwrap()
}
#[test]
fn boids_merge_with_target_deletion_and_scene_effects_in_both_orders_twenty_times() {
    for _ in 0..20 {
        for variant in 0..6 {
            for (left, right) in [(11, 22), (22, 11)] {
                let mut scene = Scene::starter();
                let cube = scene.object_named("Cube").unwrap().id;
                let target = scene.add_object("Goal", ObjectData::Empty { size: 1.0 });
                let mut settings = ParticleSystem::new("Birds", Kind::Emitter);
                settings.boids.rules = vec![Rule { kind: RuleKind::Goal, target: Some(target) }];
                scene.object_mut(cube).unwrap().particles.push(settings);
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
                aa.object_mut(cube).unwrap().particles[0].physics = ParticlePhysics::Boids;
                aa.object_mut(cube).unwrap().particles[0].boids.air_speed_max = 7.0;
                let mut bb = before.clone();
                match variant {
                    0 => bb.objects.retain(|object| object.id != target),
                    1 => {
                        bb.object_mut(cube).unwrap().particles[0].boids.air_speed_max = 3.0;
                    }
                    2 => {
                        bb.object_mut(cube).unwrap().physics = Some(ObjectPhysics { collision: Some(Collision::default()), ..Default::default() });
                    }
                    3 => {
                        bb.object_mut(target).unwrap().physics =
                            Some(ObjectPhysics { field: Some(ForceField::new(FieldKind::Wind)), ..Default::default() });
                    }
                    4 => bb.object_mut(target).unwrap().location = glam::Vec3::new(1e30, -1e30, 1e30),
                    _ => bb.object_mut(cube).unwrap().location = glam::Vec3::new(-1e30, 1e30, -1e30),
                }
                aa.validate().unwrap();
                if variant != 0 {
                    bb.validate().unwrap();
                }
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
                merged.validate().unwrap();
                assert_eq!(merged, read(&d, &blobs));
                let settings = &merged.object(cube).unwrap().particles[0];
                if variant == 0 {
                    assert!(settings.boids.rules[0].target.is_none());
                    assert_eq!(settings.boids.air_speed_max, 7.0);
                } else if variant == 1 {
                    assert!(settings == &aa.object(cube).unwrap().particles[0] || settings == &bb.object(cube).unwrap().particles[0]);
                } else {
                    assert_eq!(settings.physics, ParticlePhysics::Boids);
                    assert_eq!(settings.boids.air_speed_max, 7.0);
                }
            }
        }
    }
}
