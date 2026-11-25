//! Particle systems through a replica: settings and groomed hair are written and read back;
//! simulated particles are never stored.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::kdm::{ActorId, DocId, Origin, Replica};
use kition_kit::session::{BlobSink, MemoryBlobs};
use kition_scene_kdm::mesh_blob::MeshCache;
use kition_scene_kdm::read::{blob_hashes, read_scene};
use kition_scene_kdm::schema;
use kition_scene_kdm::write::{Written, write_scene};
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
