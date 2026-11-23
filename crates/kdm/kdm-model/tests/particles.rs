//! Particle systems through a replica: settings and groomed hair are written, read back, and
//! merged; simulated particles are never stored.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::kdm::{ActorId, DocId, Origin, Replica};
use kition_kit::session::{BlobSink, MemoryBlobs};
use kition_scene_kdm::mesh_blob::MeshCache;
use kition_scene_kdm::read::{blob_hashes, read_scene};
use kition_scene_kdm::write::{Written, stored_form, write_changes, write_scene};
use kition_scene_kdm::{ids, schema};
use kition_scene_model::particles::{self, Kind, ParticleSystem, hair};
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

/// The starter scene with a floor, an emitter on the cube bouncing off it, and groomed hair on a
/// sphere.
fn with_particles() -> Scene {
    let mut scene = Scene::starter();
    let floor = scene.add_object("Floor", ObjectData::Mesh(Primitive::Plane { size: 10.0 }.build().unwrap()));
    let cube = scene.object_named("Cube").unwrap().id;
    let mut rain = ParticleSystem::new("Rain", Kind::Emitter);
    rain.colliders.push(floor);
    rain.seed = 7;
    scene.object_mut(cube).unwrap().particles.push(rain);
    let sphere = scene.add_object("Sphere", ObjectData::Mesh(Primitive::UvSphere { segments: 16, rings: 8, radius: 1.0 }.build().unwrap()));
    let mut fur = ParticleSystem::new("Fur", Kind::Hair);
    fur.count = 40;
    fur.hair_length = 0.3;
    scene.object_mut(sphere).unwrap().particles.push(fur);
    let surface = particles::surface(&scene, sphere, false).unwrap();
    let mut groomed = hair::parents(&scene.object(sphere).unwrap().particles[0], &surface);
    for strand in &mut groomed {
        let tip = strand.len() - 1;
        strand[tip].z += 0.1;
    }
    scene.object_mut(sphere).unwrap().particles[0].groomed = groomed;
    scene.validate().unwrap();
    scene
}

#[test]
fn settings_and_groomed_hair_round_trip() {
    let scene = with_particles();
    let (replica, blobs) = create(&scene);
    assert!(kition_scene_kdm::SCHEMA.contains("\"schemaVersion\": 1"));
    let back = read(&replica, &blobs);
    for name in ["Cube", "Sphere"] {
        assert_eq!(back.object_named(name).unwrap().particles, scene.object_named(name).unwrap().particles, "{name}");
    }
    assert_eq!(back.object_named("Sphere").unwrap().particles[0].groomed.len(), 40);
    // One mesh blob per mesh object and one hair blob.
    let meshes = scene.objects.iter().filter(|object| matches!(object.data, ObjectData::Mesh(_))).count();
    assert_eq!(blob_hashes(&replica).len(), meshes + 1);
    // Particles are not stored: nothing in the state names a particle's position.
    let state = replica.state_json().unwrap().to_string();
    assert!(state.contains("\"Rain\"") && !state.contains("velocity\":["), "{}", &state[..200]);
    let loaded = Replica::load(schema(), &replica.snapshot().unwrap(), ActorId::random()).unwrap();
    assert_eq!(read(&loaded, &blobs).object_named("Sphere").unwrap().particles, scene.object_named("Sphere").unwrap().particles);
}

#[test]
fn concurrent_particle_edits_converge() {
    let (mut a, mut blobs_a) = create(&with_particles());
    let mut b = Replica::load(schema(), &a.snapshot().unwrap(), ActorId::random()).unwrap();
    let mut blobs_b = blobs_a.clone();
    // A deletes the floor and changes the fur's count (its groom no longer fits); B grooms again
    // and raises the emitter's count.
    edit(&mut a, &mut blobs_a, |scene| {
        let floor = scene.object_named("Floor").unwrap().id;
        scene.remove_objects(&[floor]).unwrap();
        let sphere = scene.object_named("Sphere").unwrap().id;
        let fur = &mut scene.object_mut(sphere).unwrap().particles[0];
        fur.count = 60;
        fur.groomed.clear();
    });
    edit(&mut b, &mut blobs_b, |scene| {
        let sphere = scene.object_named("Sphere").unwrap().id;
        for strand in &mut scene.object_mut(sphere).unwrap().particles[0].groomed {
            strand[1].x += 0.05;
        }
        let cube = scene.object_named("Cube").unwrap().id;
        scene.object_mut(cube).unwrap().particles[0].count = 2000;
    });
    sync(&mut a, &mut b, &mut blobs_a, &mut blobs_b);
    assert_eq!(a.state_json().unwrap(), b.state_json().unwrap());
    let scene = read(&a, &blobs_a);
    assert_eq!(read(&b, &blobs_b), scene);
    scene.validate().unwrap();
    // The floor is gone and the emitter lets go of it; the fur has a count of 60 and B's groom,
    // made for 40 strands, is dropped.
    let rain = &scene.object_named("Cube").unwrap().particles[0];
    assert!(rain.colliders.is_empty());
    let fur = &scene.object_named("Sphere").unwrap().particles[0];
    assert_eq!(fur.count, 60);
    assert!(fur.groomed.is_empty());
}
