//! The scene through a replica: written whole, read back, edited through the bridge, merged with
//! a peer's edits, and repaired.
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

#[test]
fn concurrent_edits_of_one_object_and_of_others_converge() {
    let (mut a, mut blobs_a) = create(&Scene::starter());
    let mut b = Replica::load(schema(), &a.snapshot().unwrap(), ActorId::random()).unwrap();
    let mut blobs_b = blobs_a.clone();
    // One moves the cube while the other turns it, and each adds an object.
    edit(&mut a, &mut blobs_a, |scene| {
        let cube = scene.object_named("Cube").unwrap().id;
        scene.object_mut(cube).unwrap().location.x = 4.0;
        scene.add_object(
            "Cone",
            ObjectData::Mesh(
                Primitive::Cone { vertices: 8, radius1: 1.0, radius2: 0.0, depth: 2.0, fill: kition_scene_model::Fill::Ngon }.build().unwrap(),
            ),
        );
    });
    edit(&mut b, &mut blobs_b, |scene| {
        let cube = scene.object_named("Cube").unwrap().id;
        scene.object_mut(cube).unwrap().rotation.z = 1.0;
        scene.add_object(
            "Torus",
            ObjectData::Mesh(Primitive::Torus { major_segments: 8, minor_segments: 4, major_radius: 1.0, minor_radius: 0.25 }.build().unwrap()),
        );
    });
    sync(&mut a, &mut b, &mut blobs_a, &mut blobs_b);
    assert_eq!(a.state_json().unwrap(), b.state_json().unwrap());
    let scene = read(&a, &blobs_a);
    let cube = scene.object_named("Cube").unwrap();
    assert_eq!((cube.location.x, cube.rotation.z), (4.0, 1.0), "both edits kept");
    assert!(scene.object_named("Cone").is_some() && scene.object_named("Torus").is_some());
    let keys: std::collections::BTreeSet<_> = scene.objects.iter().map(|object| object.id).collect();
    assert_eq!(keys.len(), scene.objects.len(), "no key minted twice");
    scene.validate().unwrap();
}

#[test]
fn repair_clears_what_a_merge_left_dangling() {
    let (mut a, mut blobs_a) = create(&Scene::starter());
    let mut b = Replica::load(schema(), &a.snapshot().unwrap(), ActorId::random()).unwrap();
    let mut blobs_b = blobs_a.clone();
    // A deletes the light and the camera while B parents the cube to the light.
    edit(&mut a, &mut blobs_a, |scene| {
        let light = scene.object_named("Light").unwrap().id;
        let camera = scene.object_named("Camera").unwrap().id;
        scene.remove_objects(&[light, camera]).unwrap();
    });
    edit(&mut b, &mut blobs_b, |scene| {
        let light = scene.object_named("Light").unwrap().id;
        let cube = scene.object_named("Cube").unwrap().id;
        scene.set_parent(cube, Some(light)).unwrap();
    });
    sync(&mut a, &mut b, &mut blobs_a, &mut blobs_b);
    assert_eq!(a.state_json().unwrap(), b.state_json().unwrap());
    let scene = read(&a, &blobs_a);
    assert_eq!(scene.object_named("Cube").unwrap().parent, None, "the parent went");
    assert_eq!(scene.camera, None, "no camera is left");
    scene.validate().unwrap();

    // Two people parent two objects to each other at once: the cycle is broken.
    let (mut c, mut blobs_c) = create(&Scene::starter());
    let mut d = Replica::load(schema(), &c.snapshot().unwrap(), ActorId::random()).unwrap();
    let mut blobs_d = blobs_c.clone();
    edit(&mut c, &mut blobs_c, |scene| {
        let (cube, light) = (scene.object_named("Cube").unwrap().id, scene.object_named("Light").unwrap().id);
        scene.set_parent(cube, Some(light)).unwrap();
    });
    edit(&mut d, &mut blobs_d, |scene| {
        let (cube, light) = (scene.object_named("Cube").unwrap().id, scene.object_named("Light").unwrap().id);
        scene.set_parent(light, Some(cube)).unwrap();
    });
    sync(&mut c, &mut d, &mut blobs_c, &mut blobs_d);
    assert_eq!(c.state_json().unwrap(), d.state_json().unwrap());
    read(&c, &blobs_c).validate().unwrap();
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
fn glass_depth_of_field_and_the_world_image_round_trip_and_merge() {
    let scene = rendered_scene();
    let (mut a, mut blobs_a) = create(&scene);
    let back = read(&a, &blobs_a);
    assert_eq!(back.materials, scene.materials);
    assert_eq!(back.world, scene.world);
    assert_eq!(back.object_named("Camera").unwrap().data, scene.object_named("Camera").unwrap().data);
    let image = back.image(back.world.image.as_deref().unwrap()).unwrap();
    assert_eq!(image.format, kition_scene_model::ImageFormat::Hdr);
    assert_eq!(image.bytes.as_slice(), sky().as_slice());

    // A deletes the focused cube and clears the world image while B turns the world and refocuses
    // the camera at f/2: both let go of what A removed.
    let mut b = Replica::load(schema(), &a.snapshot().unwrap(), ActorId::random()).unwrap();
    let mut blobs_b = blobs_a.clone();
    edit(&mut a, &mut blobs_a, |scene| {
        let cube = scene.object_named("Cube").unwrap().id;
        scene.remove_objects(&[cube]).unwrap();
        scene.world.image = None;
        scene.prune_images();
    });
    edit(&mut b, &mut blobs_b, |scene| {
        scene.world.rotation = 2.5;
        let camera = scene.camera.unwrap();
        if let ObjectData::Camera(camera) = &mut scene.object_mut(camera).unwrap().data {
            camera.dof.f_stop = 2.0;
        }
    });
    sync(&mut a, &mut b, &mut blobs_a, &mut blobs_b);
    assert_eq!(a.state_json().unwrap(), b.state_json().unwrap());
    let merged = read(&a, &blobs_a);
    merged.validate().unwrap();
    assert_eq!(merged.world.image, None, "the world image went with its record");
    assert!(merged.images.is_empty());
    let ObjectData::Camera(camera) = &merged.object_named("Camera").unwrap().data else { panic!() };
    assert_eq!(camera.dof.focus_object, None, "the camera let go of the deleted cube");
    assert!(camera.dof.enabled);
}

#[test]
fn constraints_and_weights_merge_and_cycles_are_broken() {
    use kition_scene_model::constraints::{ConstraintKind, TrackAxis, UpAxis};
    use kition_scene_model::{Constraint, Target};
    let track = |target| Constraint {
        name: "Track To".into(),
        enabled: true,
        influence: 1.0,
        kind: ConstraintKind::TrackTo { target: Some(Target { object: target, bone: None }), track_axis: TrackAxis::NegZ, up_axis: UpAxis::Y },
    };
    let (mut a, mut blobs_a) = create(&Scene::starter());
    let mut b = Replica::load(schema(), &a.snapshot().unwrap(), ActorId::random()).unwrap();
    let mut blobs_b = blobs_a.clone();
    // A makes the camera track the cube and paints a group; B makes the cube track the camera.
    edit(&mut a, &mut blobs_a, |scene| {
        let (cube, camera) = (scene.object_named("Cube").unwrap().id, scene.object_named("Camera").unwrap().id);
        scene.object_mut(camera).unwrap().constraints.push(track(cube));
        let ObjectData::Mesh(mesh) = &mut scene.object_mut(cube).unwrap().data else { panic!("the cube is a mesh") };
        kition_scene_model::weight_paint::dab(mesh, "Group", &[(0, 1.0)], &Default::default()).unwrap();
    });
    let scene = read(&a, &blobs_a);
    assert_eq!(scene.object_named("Camera").unwrap().constraints.len(), 1, "constraints read back");
    edit(&mut b, &mut blobs_b, |scene| {
        let (cube, camera) = (scene.object_named("Cube").unwrap().id, scene.object_named("Camera").unwrap().id);
        scene.object_mut(cube).unwrap().constraints.push(track(camera));
    });
    sync(&mut a, &mut b, &mut blobs_a, &mut blobs_b);
    assert_eq!(a.state_json().unwrap(), b.state_json().unwrap());
    let scene = read(&a, &blobs_a);
    scene.validate().unwrap();
    let left = scene.objects.iter().map(|object| object.constraints.len()).sum::<usize>();
    assert_eq!(left, 1, "one constraint of the cycle went");
    let ObjectData::Mesh(mesh) = &scene.object_named("Cube").unwrap().data else { panic!("the cube is a mesh") };
    assert_eq!(mesh.groups.iter().find(|group| group.name == "Group").unwrap().weights[0], 1.0, "the weights merged in");

    // A deletes the cube while B aims the light at it: the light keeps its constraint, untargeted.
    edit(&mut b, &mut blobs_b, |scene| {
        let (cube, light) = (scene.object_named("Cube").unwrap().id, scene.object_named("Light").unwrap().id);
        scene.object_mut(light).unwrap().constraints.push(track(cube));
    });
    edit(&mut a, &mut blobs_a, |scene| {
        let cube = scene.object_named("Cube").unwrap().id;
        scene.remove_objects(&[cube]).unwrap();
    });
    sync(&mut a, &mut b, &mut blobs_a, &mut blobs_b);
    assert_eq!(a.state_json().unwrap(), b.state_json().unwrap());
    let scene = read(&a, &blobs_a);
    let light = &scene.object_named("Light").unwrap().constraints;
    assert!(light.len() == 1 && light[0].kind.target().is_none(), "the target went");
}

#[test]
fn physics_settings_merge_and_the_bake_stays_out_of_the_replica() {
    use kition_scene_model::physics::{BodyKind, ObjectPhysics, RigidBody};
    let (mut a, mut blobs_a) = create(&Scene::starter());
    let mut b = Replica::load(schema(), &a.snapshot().unwrap(), ActorId::random()).unwrap();
    let mut blobs_b = blobs_a.clone();
    // A makes the cube a rigid body; B shortens the simulation.
    edit(&mut a, &mut blobs_a, |scene| {
        let cube = scene.object_named("Cube").unwrap().id;
        let body = RigidBody { kind: BodyKind::Active, mass: 2.0, ..RigidBody::default() };
        scene.object_mut(cube).unwrap().physics = Some(ObjectPhysics { rigid_body: Some(body), ..ObjectPhysics::default() });
    });
    edit(&mut b, &mut blobs_b, |scene| scene.physics.frame_end = 80);
    sync(&mut a, &mut b, &mut blobs_a, &mut blobs_b);
    assert_eq!(a.state_json().unwrap(), b.state_json().unwrap());
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
