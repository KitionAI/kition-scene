//! Nested NLA references converge regardless of actor ordering or change arrival order.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use kition_kit::kdm::{ActorId, DocId, Origin, Replica};
use kition_kit::session::MemoryBlobs;
use kition_scene_kdm::mesh_blob::MeshCache;
use kition_scene_kdm::read::read_scene;
use kition_scene_kdm::write::{write_changes, write_scene};
use kition_scene_kdm::{root_id, schema};
use kition_scene_model::animation::{Action, ActionChannel, Property};
use kition_scene_model::nla::Nla;
use kition_scene_model::{Interpolation, Keyframe, ObjectData, Scene};
use serde_json::json;
fn scene() -> Scene {
    let mut scene = Scene::empty();
    let id = scene.add_object("E", ObjectData::Empty { size: 1.0 });
    for (name, value) in [("A", 0.0), ("B", 30.0)] {
        scene.animation.actions.push(Action {
            name: name.into(),
            channels: vec![ActionChannel {
                property: Property::Location,
                index: 0,
                path: String::new(),
                keys: vec![
                    Keyframe { frame: 1.0, value, interpolation: Interpolation::Linear },
                    Keyframe { frame: 11.0, value: value + 10.0, interpolation: Interpolation::Linear },
                ],
                modifiers: Vec::new(),
            }],
        });
    }
    scene.nla_add_strip(id, None, "A", 1.0).unwrap();
    scene.nla_add_strip(id, Some(0), "B", 21.0).unwrap();
    scene.nla_add_transition(id, 0, 0).unwrap();
    scene
}
fn read(replica: &Replica) -> Scene {
    read_scene(replica, &MemoryBlobs::default(), &mut MeshCache::default()).unwrap()
}
fn edit(replica: &mut Replica, f: impl FnOnce(&mut Scene)) {
    let before = read(replica);
    let mut after = before.clone();
    f(&mut after);
    replica.transact(&Origin::person("p"), |tx| write_changes(tx, &before, &after, None)).unwrap();
}
fn repair(replica: &mut Replica) {
    replica.transact(&Origin::system("repair"), kition_scene_kdm::repair::repair).unwrap();
}
#[test]
fn nested_action_removal_repairs_in_both_orders_twenty_times() {
    for _ in 0..20 {
        for (first, second) in [(1, 2), (2, 1)] {
            let (mut a, _) = Replica::create_with(schema(), DocId::new(), ActorId::from_u64(first).unwrap(), &Origin::system("create"), |tx| {
                write_scene(tx, &scene())
            })
            .unwrap();
            let base = a.snapshot().unwrap();
            let version = a.version();
            let mut b = Replica::load(schema(), &base, ActorId::from_u64(second).unwrap()).unwrap();
            edit(&mut a, |scene| {
                scene.animation.actions.retain(|action| action.name != "A");
                scene.nla_follow_action("A", None);
            });
            edit(&mut b, |scene| {
                let id = scene.objects[0].id;
                scene.nla_make_meta(id, 0, 0, 2).unwrap();
                scene.nla_make_meta(id, 0, 0, 0).unwrap();
                scene.object_mut(id).unwrap().nla.tracks[0].strips[0].resize_meta(11.0, 71.0).unwrap();
            });
            let changes_a = a.export_changes(&version).unwrap();
            let changes_b = b.export_changes(&version).unwrap();
            let mut c = Replica::load(schema(), &base, ActorId::from_u64(100).unwrap()).unwrap();
            let mut d = Replica::load(schema(), &base, ActorId::from_u64(200).unwrap()).unwrap();
            c.import_changes(&changes_a).unwrap();
            c.import_changes(&changes_b).unwrap();
            repair(&mut c);
            d.import_changes(&changes_b).unwrap();
            d.import_changes(&changes_a).unwrap();
            repair(&mut d);
            assert_eq!(c.state_json().unwrap(), d.state_json().unwrap(), "actors {first}, {second}");
            let scene = read(&c);
            scene.validate().unwrap();
            let mut played = Vec::new();
            for track in &scene.objects[0].nla.tracks {
                for strip in &track.strips {
                    strip.visit_actions(&mut |name| played.push(name.to_string()));
                }
            }
            assert_eq!(played, vec!["B"]);
            assert_eq!(read(&d), scene);
        }
    }
}
#[test]
fn raw_nested_limits_are_checked_before_read_and_repair() {
    for excessive_depth in [false, true] {
        let (mut replica, _) =
            Replica::create_with(schema(), DocId::new(), ActorId::from_u64(1).unwrap(), &Origin::system("create"), |tx| write_scene(tx, &scene()))
                .unwrap();
        let mut strip = json!({"name":"Meta","kind":"meta","action":"","start":1,"actionStart":1,"actionEnd":31,"children":[]});
        if excessive_depth {
            for _ in 0..18 {
                strip = json!({"name":"Meta","kind":"meta","action":"","start":1,"actionStart":1,"actionEnd":31,"children":[strip]});
            }
        } else {
            strip["children"] = json!(vec![json!({"name":"A","action":"A","start":1,"actionStart":1,"actionEnd":11}); 1025]);
        }
        let nla = json!({"tracks":[{"name":"Track","strips":[strip]}]});
        assert!(Nla::check_json_structure(&nla).is_err());
        let id = replica.list(&root_id(), "objects").unwrap()[0].clone();
        replica.transact(&Origin::person("p"), |tx| tx.set(&id, "nla", nla)).unwrap();
        assert!(read_scene(&replica, &MemoryBlobs::default(), &mut MeshCache::default()).is_err());
        assert!(replica.transact(&Origin::system("repair"), kition_scene_kdm::repair::repair).is_err());
    }
}
