//! The scene through a replica: written whole, read back and edited through the bridge.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::kdm::{ActorId, DocId, Origin, Replica};
use kition_kit::session::{BlobSink, KitModel, MemoryBlobs};
use kition_scene_kdm::mesh_blob::MeshCache;
use kition_scene_kdm::read::read_scene;
use kition_scene_kdm::write::{Written, stored_form, write_changes, write_scene};
use kition_scene_kdm::{SceneModel, ids, schema};
use kition_scene_model::glam::Vec3;
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

#[test]
fn the_starter_scene_round_trips() {
    let mut starter = Scene::starter();
    starter.add_object("Ball", ObjectData::Mesh(Primitive::UvSphere { segments: 8, rings: 4, radius: 1.0 }.build().unwrap()));
    let (replica, blobs) = create(&starter);
    let back = read(&replica, &blobs);
    assert_eq!(back, stored_form(&starter, &starter));
    assert!(SceneModel::load(&replica, &blobs).is_ok());
}

#[test]
fn edits_write_only_what_changed_and_read_back() {
    let (mut replica, mut blobs) = create(&Scene::starter());
    let version = replica.version();
    edit(&mut replica, &mut blobs, |scene| {
        let cube = scene.object_named("Cube").unwrap().id;
        scene.object_mut(cube).unwrap().location = Vec3::new(1.0, 2.0, 3.0);
    });
    let changes = replica.export_changes(&version).unwrap();
    assert!(changes.len() < 400, "a move writes one register, not the scene ({} bytes)", changes.len());
    let mut added = 0;
    edit(&mut replica, &mut blobs, |scene| {
        added = scene.add_object("Ball", ObjectData::Mesh(Primitive::IcoSphere { subdivisions: 2, radius: 1.0 }.build().unwrap()));
        let light = scene.object_named("Light").unwrap().id;
        scene.remove_objects(&[light]).unwrap();
    });
    let back = read(&replica, &blobs);
    assert_eq!(back.object_named("Cube").unwrap().location, Vec3::new(1.0, 2.0, 3.0));
    assert!(back.object_named("Light").is_none());
    let ball = back.object_named("Ball").unwrap();
    assert_eq!(ball.id, added);
    assert!(ball.id >= ids::FIRST_RANGE, "minted from the actor's range");
    assert_eq!(ball.mesh().unwrap().faces.len(), 80);
}

/// A small Radiance HDR file (flat RGBE pixels), as the world image.
fn sky() -> Vec<u8> {
    let mut bytes = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 2 +X 4\n".to_vec();
    for _ in 0..4 {
        bytes.extend([64, 128, 255, 129]);
    }
    for _ in 0..4 {
        bytes.extend([32, 32, 32, 126]);
    }
    bytes
}

/// The starter scene with a glass cube, a camera focused on it, and an HDR world image.
fn rendered_scene() -> Scene {
    let mut scene = Scene::starter();
    let cube = scene.object_named("Cube").unwrap().id;
    let material = scene.object(cube).unwrap().material.unwrap();
    let found = scene.material_mut(material).unwrap();
    found.transmission = 1.0;
    found.ior = 1.33;
    found.alpha_mode = kition_scene_model::AlphaMode::Clip;
    found.alpha_cutoff = 0.25;
    let camera = scene.camera.unwrap();
    if let ObjectData::Camera(camera) = &mut scene.object_mut(camera).unwrap().data {
        camera.dof.enabled = true;
        camera.dof.f_stop = 1.4;
        camera.dof.focus_object = Some(cube);
    }
    let hash = scene.add_image(kition_scene_model::Image::new("Sky.hdr", sky()).unwrap());
    scene.world.image = Some(hash);
    scene.world.rotation = 1.25;
    scene
}

#[test]
fn glass_depth_of_field_and_the_world_image_round_trip() {
    let scene = rendered_scene();
    let (a, blobs_a) = create(&scene);
    let back = read(&a, &blobs_a);
    assert_eq!(back.materials, scene.materials);
    assert_eq!(back.world, scene.world);
    assert_eq!(back.object_named("Camera").unwrap().data, scene.object_named("Camera").unwrap().data);
    let image = back.image(back.world.image.as_deref().unwrap()).unwrap();
    assert_eq!(image.format, kition_scene_model::ImageFormat::Hdr);
    assert_eq!(image.bytes.as_slice(), sky().as_slice());
}

#[test]
fn constraints_and_weights_read_back() {
    use kition_scene_model::constraints::{ConstraintKind, TrackAxis, UpAxis};
    use kition_scene_model::{Constraint, Target};
    let track = |target| Constraint {
        name: "Track To".into(),
        enabled: true,
        influence: 1.0,
        kind: ConstraintKind::TrackTo { target: Some(Target { object: target, bone: None }), track_axis: TrackAxis::NegZ, up_axis: UpAxis::Y },
    };
    let (mut a, mut blobs_a) = create(&Scene::starter());
    // The camera tracks the cube, and a group is painted on the cube.
    edit(&mut a, &mut blobs_a, |scene| {
        let (cube, camera) = (scene.object_named("Cube").unwrap().id, scene.object_named("Camera").unwrap().id);
        scene.object_mut(camera).unwrap().constraints.push(track(cube));
        let ObjectData::Mesh(mesh) = &mut scene.object_mut(cube).unwrap().data else { panic!("the cube is a mesh") };
        kition_scene_model::weight_paint::dab(mesh, "Group", &[(0, 1.0)], &Default::default()).unwrap();
    });
    let scene = read(&a, &blobs_a);
    scene.validate().unwrap();
    assert_eq!(scene.object_named("Camera").unwrap().constraints.len(), 1, "constraints read back");
    let ObjectData::Mesh(mesh) = &scene.object_named("Cube").unwrap().data else { panic!("the cube is a mesh") };
    assert_eq!(mesh.groups.iter().find(|group| group.name == "Group").unwrap().weights[0], 1.0, "the weights read back");
}

#[test]
fn physics_settings_read_back_and_the_bake_stays_out_of_the_replica() {
    use kition_scene_model::physics::{BodyKind, ObjectPhysics, RigidBody};
    let (mut a, mut blobs_a) = create(&Scene::starter());
    // The cube becomes a rigid body, then the simulation is shortened.
    edit(&mut a, &mut blobs_a, |scene| {
        let cube = scene.object_named("Cube").unwrap().id;
        let body = RigidBody { kind: BodyKind::Active, mass: 2.0, ..RigidBody::default() };
        scene.object_mut(cube).unwrap().physics = Some(ObjectPhysics { rigid_body: Some(body), ..ObjectPhysics::default() });
    });
    edit(&mut a, &mut blobs_a, |scene| scene.physics.frame_end = 80);
    let scene = read(&a, &blobs_a);
    scene.validate().unwrap();
    assert_eq!(scene.physics.frame_end, 80);
    let body = scene.object_named("Cube").unwrap().physics.as_ref().and_then(|physics| physics.rigid_body.clone()).unwrap();
    assert_eq!(body.mass, 2.0);
    assert!(!a.state_json().unwrap().to_string().contains("baked"), "the cache is derived, never stored");
    // Removing the physics clears the register.
    edit(&mut a, &mut blobs_a, |scene| {
        let cube = scene.object_named("Cube").unwrap().id;
        scene.object_mut(cube).unwrap().physics = None;
        scene.physics = Default::default();
    });
    let scene = read(&a, &blobs_a);
    assert!(scene.object_named("Cube").unwrap().physics.is_none() && scene.physics.is_default());
}
