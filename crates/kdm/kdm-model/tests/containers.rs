//! The scene's stacks and its 3D text are containers (schema freeze, schema version 1): an
//! object's modifiers, constraints, particle systems, keyframe channels, NLA tracks (and their
//! strips) and an armature's bones (and their constraints), a material's channels, the scene's
//! actions, keying sets and node groups are lists of records of their own, and a text object's
//! body is a `text` field. Two replicas editing one of them at once both keep their edits, and
//! end identical.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::kdm::{ActorId, DocId, Origin, Replica};
use kition_kit::session::{BlobSink, MemoryBlobs};
use kition_scene_kdm::mesh_blob::MeshCache;
use kition_scene_kdm::read::read_scene;
use kition_scene_kdm::write::{Written, stored_form, write_changes, write_scene};
use kition_scene_kdm::{ids, repair, root_id, schema};
use kition_scene_model::animation::{Action, KeyingSet, Property};
use kition_scene_model::glam::Vec3;
use kition_scene_model::nla::{NlaStrip, NlaTrack};
use kition_scene_model::particles::{Kind, ParticleSystem};
use kition_scene_model::{Armature, Bone, Constraint, Modifier, NodeGroup, ObjectData, Scene, Text};
use serde_json::json;

fn keep(blobs: &mut MemoryBlobs, written: Written) {
    for blob in written.blobs {
        blobs.put(&blob.bytes, "application/octet-stream");
    }
}

fn read(replica: &Replica, blobs: &MemoryBlobs) -> Scene {
    read_scene(replica, blobs, &mut MeshCache::default()).unwrap()
}

/// A replica of `scene`, and a second one of the same document.
fn pair(scene: &Scene) -> (Replica, Replica, MemoryBlobs) {
    let mut blobs = MemoryBlobs::default();
    let (a, written) =
        Replica::create_with(schema(), DocId::new(), ActorId::random(), &Origin::system("create"), |tx| write_scene(tx, scene)).unwrap();
    keep(&mut blobs, written);
    let b = Replica::load(schema(), &a.snapshot().unwrap(), ActorId::random()).unwrap();
    (a, b, blobs)
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

/// Both replicas take each other's changes and repair; they hold the same state.
fn sync(a: &mut Replica, b: &mut Replica) {
    let to_b = a.export_changes(&b.version()).unwrap();
    let to_a = b.export_changes(&a.version()).unwrap();
    b.import_changes(&to_b).unwrap();
    a.import_changes(&to_a).unwrap();
    for replica in [&mut *a, &mut *b] {
        replica.transact(&Origin::system("repair"), |tx| Ok(repair::repair(tx).unwrap())).unwrap();
    }
    // A repair's own writes reach the other replica too (they are the same writes).
    let to_b = a.export_changes(&b.version()).unwrap();
    let to_a = b.export_changes(&a.version()).unwrap();
    b.import_changes(&to_b).unwrap();
    a.import_changes(&to_a).unwrap();
    assert_eq!(a.state_json().unwrap(), b.state_json().unwrap(), "the replicas converge");
}

/// Two people each edit `scene` once (`left` on one replica, `right` on the other) at the same
/// time; the merged scene, the same on both.
fn concurrently(scene: &Scene, left: impl FnOnce(&mut Scene), right: impl FnOnce(&mut Scene)) -> Scene {
    let (mut a, mut b, mut blobs) = pair(scene);
    edit(&mut a, &mut blobs, left);
    edit(&mut b, &mut blobs, right);
    sync(&mut a, &mut b);
    let merged = read(&a, &blobs);
    assert_eq!(read(&b, &blobs), merged, "both replicas read one scene");
    merged
}

fn modifier(name: &str) -> Modifier {
    serde_json::from_value(json!({ "name": name, "type": "mirror" })).unwrap()
}

fn constraint(name: &str) -> Constraint {
    serde_json::from_value(json!({ "name": name, "type": "copyLocation" })).unwrap()
}

fn cube(scene: &Scene) -> u32 {
    scene.object_named("Cube").unwrap().id
}

#[test]
fn modifiers_added_and_edited_at_once_all_stay() {
    let mut scene = Scene::starter();
    let id = cube(&scene);
    scene.object_mut(id).unwrap().modifiers = vec![modifier("First"), modifier("Second")];
    let merged = concurrently(
        &scene,
        |scene| {
            let object = scene.object_mut(id).unwrap();
            object.modifiers[0].enabled = false;
            object.modifiers.push(modifier("Left"));
        },
        |scene| {
            let object = scene.object_mut(id).unwrap();
            object.modifiers[1].enabled = false;
            object.modifiers.insert(0, modifier("Right"));
        },
    );
    let modifiers = &merged.object(id).unwrap().modifiers;
    let names: Vec<&str> = modifiers.iter().map(|modifier| modifier.name.as_str()).collect();
    assert_eq!(names, ["Right", "First", "Second", "Left"]);
    assert!(!modifiers[1].enabled && !modifiers[2].enabled, "both edits stay: {modifiers:?}");
}

#[test]
fn modifiers_of_one_name_added_at_once_are_both_kept_under_two_names() {
    let scene = Scene::starter();
    let id = cube(&scene);
    let merged = concurrently(
        &scene,
        |scene| scene.object_mut(id).unwrap().modifiers.push(modifier("Mirror")),
        |scene| scene.object_mut(id).unwrap().modifiers.push(modifier("Mirror")),
    );
    let mut names: Vec<String> = merged.object(id).unwrap().modifiers.iter().map(|modifier| modifier.name.clone()).collect();
    names.sort();
    assert_eq!(names, ["Mirror", "Mirror.001"]);
}

#[test]
fn constraints_added_at_once_all_stay() {
    let scene = Scene::starter();
    let id = cube(&scene);
    let merged = concurrently(
        &scene,
        |scene| scene.object_mut(id).unwrap().constraints.push(constraint("Left")),
        |scene| scene.object_mut(id).unwrap().constraints.push(constraint("Right")),
    );
    let mut names: Vec<String> = merged.object(id).unwrap().constraints.iter().map(|constraint| constraint.name.clone()).collect();
    names.sort();
    assert_eq!(names, ["Left", "Right"]);
}

#[test]
fn particle_systems_added_at_once_all_stay() {
    let scene = Scene::starter();
    let id = cube(&scene);
    let merged = concurrently(
        &scene,
        |scene| scene.object_mut(id).unwrap().particles.push(ParticleSystem::new("Rain", Kind::Emitter)),
        |scene| scene.object_mut(id).unwrap().particles.push(ParticleSystem::new("Fur", Kind::Hair)),
    );
    let mut names: Vec<String> = merged.object(id).unwrap().particles.iter().map(|system| system.name.clone()).collect();
    names.sort();
    assert_eq!(names, ["Fur", "Rain"]);
}

#[test]
fn keys_of_one_object_inserted_at_once_all_stay() {
    let scene = Scene::starter();
    let id = cube(&scene);
    let merged = concurrently(
        &scene,
        |scene| {
            scene.object_mut(id).unwrap().location = Vec3::new(1.0, 0.0, 0.0);
            scene.insert_key(id, Property::Location, 1.0).unwrap();
        },
        |scene| {
            scene.object_mut(id).unwrap().location = Vec3::new(0.0, 5.0, 0.0);
            scene.insert_key(id, Property::Location, 10.0).unwrap();
            scene.insert_key(id, Property::Scale, 10.0).unwrap();
        },
    );
    let channels: Vec<_> = merged.animation.channels.iter().filter(|channel| channel.target == id).collect();
    assert_eq!(channels.iter().filter(|channel| channel.property == Property::Location).count(), 3, "one channel per component");
    assert_eq!(channels.iter().filter(|channel| channel.property == Property::Scale).count(), 3);
    let x = channels.iter().find(|channel| channel.property == Property::Location && channel.index == 0).unwrap();
    let frames: Vec<f32> = x.keys.iter().map(|key| key.frame).collect();
    assert_eq!(frames, [1.0, 10.0], "both people's keys");
    let y = channels.iter().find(|channel| channel.property == Property::Location && channel.index == 1).unwrap();
    assert_eq!((y.keys[0].value, y.keys[1].value), (0.0, 5.0));
}

#[test]
fn keys_of_one_material_inserted_at_once_all_stay() {
    let scene = Scene::starter();
    let material = scene.materials[0].id;
    let merged = concurrently(
        &scene,
        |scene| {
            scene.insert_key(material, Property::BaseColor, 1.0).unwrap();
        },
        |scene| {
            scene.insert_key(material, Property::BaseColor, 20.0).unwrap();
        },
    );
    let channels: Vec<_> = merged.animation.channels.iter().filter(|channel| channel.target == material).collect();
    assert_eq!(channels.len(), 4, "one channel per component: {channels:?}");
    assert!(channels.iter().all(|channel| channel.keys.iter().map(|key| key.frame).collect::<Vec<_>>() == [1.0, 20.0]));
}

fn action(name: &str) -> Action {
    serde_json::from_value(json!({ "name": name, "channels": [{ "property": "location", "index": 0, "keys": [{ "frame": 1, "value": 0 }] }] }))
        .unwrap()
}

#[test]
fn actions_and_nla_tracks_and_strips_added_at_once_all_stay() {
    let mut scene = Scene::starter();
    let id = cube(&scene);
    scene.animation.actions = vec![action("Walk"), action("Run")];
    let walk = scene.animation.actions[0].clone();
    scene.object_mut(id).unwrap().nla.tracks = vec![NlaTrack { name: "Base".into(), muted: false, solo: false, strips: Vec::new() }];
    let merged = concurrently(
        &scene,
        |scene| {
            scene.animation.actions.push(action("Jump"));
            let nla = &mut scene.object_mut(id).unwrap().nla;
            nla.tracks[0].strips.push(NlaStrip::new("Left", &walk, 1.0));
            nla.tracks.push(NlaTrack { name: "Left".into(), muted: true, solo: false, strips: Vec::new() });
        },
        |scene| {
            scene.animation.actions.push(action("Sit"));
            let nla = &mut scene.object_mut(id).unwrap().nla;
            nla.tracks[0].strips.push(NlaStrip::new("Right", &walk, 100.0));
            nla.tracks.push(NlaTrack { name: "Right".into(), muted: false, solo: false, strips: Vec::new() });
        },
    );
    let mut actions: Vec<&str> = merged.animation.actions.iter().map(|action| action.name.as_str()).collect();
    actions.sort_unstable();
    assert_eq!(actions, ["Jump", "Run", "Sit", "Walk"]);
    let nla = &merged.object(id).unwrap().nla;
    let mut tracks: Vec<&str> = nla.tracks.iter().map(|track| track.name.as_str()).collect();
    tracks.sort_unstable();
    assert_eq!(tracks, ["Base", "Left", "Right"]);
    let mut strips: Vec<&str> = nla.tracks[0].strips.iter().map(|strip| strip.name.as_str()).collect();
    strips.sort_unstable();
    assert_eq!(strips, ["Left", "Right"], "both strips on the shared track");
}

#[test]
fn keying_sets_added_at_once_all_stay() {
    let scene = Scene::starter();
    let set = |name: &str| -> KeyingSet { serde_json::from_value(json!({ "name": name, "paths": [] })).unwrap() };
    let merged =
        concurrently(&scene, |scene| scene.animation.keying_sets.sets.push(set("Left")), |scene| scene.animation.keying_sets.sets.push(set("Right")));
    let mut names: Vec<&str> = merged.animation.keying_sets.sets.iter().map(|set| set.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["Left", "Right"]);
}

#[test]
fn bones_and_bone_constraints_added_at_once_all_stay() {
    let mut scene = Scene::starter();
    let rig = scene.add_object("Rig", ObjectData::Armature(Armature::starter()));
    let bones = |scene: &mut Scene| -> &mut Vec<Bone> {
        let ObjectData::Armature(armature) = &mut scene.object_mut(rig).unwrap().data else { panic!("not an armature") };
        &mut armature.bones
    };
    let merged = concurrently(
        &scene,
        |scene| {
            let bones = bones(scene);
            bones[0].constraints.push(constraint("Left"));
            bones.push(Bone::new("Arm", Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 1.0)));
        },
        |scene| {
            let bones = bones(scene);
            bones[0].constraints.push(constraint("Right"));
            bones.push(Bone::new("Leg", Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0)));
        },
    );
    let ObjectData::Armature(armature) = &merged.object(rig).unwrap().data else { panic!("not an armature") };
    let mut names: Vec<&str> = armature.bones.iter().map(|bone| bone.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["Arm", "Bone", "Leg"]);
    let mut constraints: Vec<&str> = armature.bone("Bone").unwrap().constraints.iter().map(|constraint| constraint.name.as_str()).collect();
    constraints.sort_unstable();
    assert_eq!(constraints, ["Left", "Right"]);
}

#[test]
fn typing_in_one_text_at_once_keeps_both() {
    let mut scene = Scene::starter();
    let id = scene.add_object("Sign", ObjectData::Text(Text { body: "Hello world".into(), ..Text::default() }));
    let body = |scene: &mut Scene| -> &mut String {
        let ObjectData::Text(text) = &mut scene.object_mut(id).unwrap().data else { panic!("not a text") };
        &mut text.body
    };
    let merged = concurrently(&scene, |scene| body(scene).insert_str(5, ","), |scene| body(scene).push_str("!\n\u{4e2d}\u{6587}"));
    let ObjectData::Text(text) = &merged.object(id).unwrap().data else { panic!("not a text") };
    assert_eq!(text.body, "Hello, world!\n\u{4e2d}\u{6587}");
    // One retypes the first letter while the other deletes the last word.
    let merged = concurrently(&merged, |scene| body(scene).replace_range(0..1, "J"), |scene| body(scene).replace_range(5..12, ""));
    let ObjectData::Text(text) = &merged.object(id).unwrap().data else { panic!("not a text") };
    assert_eq!(text.body, "Jello!\n\u{4e2d}\u{6587}");
}

#[test]
fn node_groups_added_at_once_all_stay_in_the_scenes_list() {
    let scene = Scene::starter();
    let (mut a, mut b, mut blobs) = pair(&scene);
    let group = |scene: &mut Scene, name: &str| {
        let (id, input, output) = (scene.take_id(), scene.take_id(), scene.take_id());
        scene.node_groups.push(NodeGroup::new(id, name, input, output));
    };
    edit(&mut a, &mut blobs, |scene| group(scene, "Left"));
    edit(&mut b, &mut blobs, |scene| group(scene, "Right"));
    sync(&mut a, &mut b);
    let merged = read(&a, &blobs);
    let mut names: Vec<&str> = merged.node_groups.iter().map(|group| group.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["Left", "Right"]);
    assert_eq!(a.list(&root_id(), "nodeGroups").unwrap().len(), 2, "the scene lists its groups");
}

/// A record a stack no longer lists (its owner deleted at the same time as a peer added to it)
/// goes with the repair.
#[test]
fn an_object_deleted_while_a_peer_adds_to_its_stack_leaves_nothing_behind() {
    let scene = Scene::starter();
    let id = cube(&scene);
    let (mut a, mut b, mut blobs) = pair(&scene);
    edit(&mut a, &mut blobs, |scene| scene.remove_objects(&[id]).unwrap());
    edit(&mut b, &mut blobs, |scene| {
        scene.object_mut(id).unwrap().modifiers.push(modifier("Late"));
        scene.object_mut(id).unwrap().location.x = 2.0;
        scene.insert_key(id, Property::Location, 3.0).unwrap();
    });
    sync(&mut a, &mut b);
    assert!(read(&a, &blobs).object(id).is_err());
    let left = a.state_json().unwrap().to_string();
    assert!(!left.contains("\"Modifier\"") && !left.contains("\"Channel\""), "no held record is left: {left}");
}
