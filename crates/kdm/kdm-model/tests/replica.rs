//! The scene through a replica: written whole, read back and edited through the bridge; an
//! agent's draft merged while the person edited, and the repair after it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::kdm::{ActorId, DocId, Origin, Replica};
use kition_kit::session::{BlobSink, KitModel, MemoryBlobs};
use kition_scene_kdm::mesh_blob::MeshCache;
use kition_scene_kdm::read::read_scene;
use kition_scene_kdm::write::{Written, stored_form, write_changes, write_scene};
use kition_scene_kdm::{SceneModel, ids, root_id, schema};
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

/// The person's `live` replica and an agent's draft of it (a fork at its version, data model
/// 6.6): the draft takes `drafted`, the person meanwhile `meanwhile`, and the person merges the
/// draft as `KitSession::merge_changes` does: one change of the person's, the repair in it.
fn merge_draft(live: &mut Replica, blobs: &mut MemoryBlobs, drafted: impl FnOnce(&mut Scene), meanwhile: impl FnOnce(&mut Scene)) {
    let base = live.version();
    let mut draft = live.fork(ActorId::random()).unwrap();
    let mut draft_blobs = blobs.clone();
    edit(&mut draft, &mut draft_blobs, drafted);
    edit(live, blobs, meanwhile);
    for (hash, bytes) in draft_blobs.0 {
        blobs.0.entry(hash).or_insert(bytes);
    }
    live.merge_changes_repaired(&draft.export_changes(&base).unwrap(), &Origin::agent("agent:test"), |tx, _| {
        kition_scene_kdm::repair::repair(tx).map(drop)
    })
    .unwrap();
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
    let changes: serde_json::Value = serde_json::from_slice(&replica.export_changes(&version).unwrap()).unwrap();
    let changes = changes["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1, "one change: {changes:?}");
    let ops = changes[0]["ops"].as_array().unwrap();
    let registers: Vec<&serde_json::Value> = ops.iter().filter(|op| !(op["op"] == "mapSet" && op["field"] == "keys")).collect();
    assert!(
        registers.len() == 1 && registers[0]["op"] == "set" && registers[0]["field"] == "location",
        "a move writes one register, not the scene: {ops:?}"
    );
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

    // The light aims at the cube, then the cube is deleted: the light keeps its constraint,
    // untargeted.
    edit(&mut a, &mut blobs_a, |scene| {
        let (cube, light) = (scene.object_named("Cube").unwrap().id, scene.object_named("Light").unwrap().id);
        scene.object_mut(light).unwrap().constraints.push(track(cube));
    });
    edit(&mut a, &mut blobs_a, |scene| {
        let cube = scene.object_named("Cube").unwrap().id;
        scene.remove_objects(&[cube]).unwrap();
    });
    let scene = read(&a, &blobs_a);
    scene.validate().unwrap();
    let light = &scene.object_named("Light").unwrap().constraints;
    assert!(light.len() == 1 && light[0].kind.target().is_none(), "the target went");
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

/// What a merged draft can leave dangling, the repair clears: the agent parents the cube to the
/// light while the person deletes the light and the camera (the draft writes only the cube, so
/// the merge applies, rule D8); the cube's parent goes and no camera is left. And the agent
/// parents the light to the cube while the person parents the cube to the light (two registers,
/// each changed by one side, rule D3): the cycle is broken at its highest key.
#[test]
fn repair_clears_what_a_merge_left_dangling() {
    let (mut a, mut blobs_a) = create(&Scene::starter());
    merge_draft(
        &mut a,
        &mut blobs_a,
        |scene| {
            let light = scene.object_named("Light").unwrap().id;
            let cube = scene.object_named("Cube").unwrap().id;
            scene.set_parent(cube, Some(light)).unwrap();
        },
        |scene| {
            let light = scene.object_named("Light").unwrap().id;
            let camera = scene.object_named("Camera").unwrap().id;
            scene.remove_objects(&[light, camera]).unwrap();
        },
    );
    let scene = read(&a, &blobs_a);
    assert!(scene.object_named("Light").is_none() && scene.object_named("Camera").is_none());
    assert_eq!(scene.object_named("Cube").unwrap().parent, None, "the parent went");
    assert_eq!(scene.camera, None, "no camera is left");
    scene.validate().unwrap();

    // Each side parents one of two objects to the other: the cycle is broken.
    let (mut c, mut blobs_c) = create(&Scene::starter());
    let (cube, light) = {
        let scene = read(&c, &blobs_c);
        (scene.object_named("Cube").unwrap().id, scene.object_named("Light").unwrap().id)
    };
    merge_draft(&mut c, &mut blobs_c, |scene| scene.set_parent(light, Some(cube)).unwrap(), |scene| scene.set_parent(cube, Some(light)).unwrap());
    let scene = read(&c, &blobs_c);
    scene.validate().unwrap();
    let (highest, other) = if cube > light { (cube, light) } else { (light, cube) };
    assert_eq!(scene.object(highest).unwrap().parent, None, "the object with the highest key lets go of its parent");
    assert_eq!(scene.object(other).unwrap().parent, Some(highest), "the other keeps its own");
}

/// A record listed twice in the scene's objects (a state only a file written by another program
/// can hold: a merge takes one item both sides inserted at one place as one insertion and refuses
/// it at two places, rule D6, and minted ids never meet) keeps
/// its first place when the repair runs (after the next undo, redo, merge or import).
#[test]
fn repair_keeps_the_first_place_of_a_record_listed_twice() {
    let (mut replica, blobs) = create(&Scene::starter());
    let root = root_id();
    let listed = replica.list(&root, "objects").unwrap();
    let first = listed[0].clone();
    replica.transact(&Origin::system("import"), |tx| tx.list_insert(&root, "objects", listed.len(), &first)).unwrap();
    let mut doubled = listed.clone();
    doubled.push(first);
    assert_eq!(replica.list(&root, "objects").unwrap(), doubled, "the control: the state holds the record twice");
    replica.transact(&Origin::system("repair"), |tx| Ok(kition_scene_kdm::repair::repair(tx).unwrap())).unwrap();
    assert_eq!(replica.list(&root, "objects").unwrap(), listed);
    let scene = read(&replica, &blobs);
    assert_eq!(scene, stored_form(&Scene::starter(), &Scene::starter()));
}
