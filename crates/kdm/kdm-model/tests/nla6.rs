//! Nested NLA strips: the raw limits are checked before a read and a repair.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use kition_kit::kdm::{ActorId, DocId, Origin, Replica};
use kition_kit::session::MemoryBlobs;
use kition_scene_kdm::mesh_blob::MeshCache;
use kition_scene_kdm::read::read_scene;
use kition_scene_kdm::write::write_scene;
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
        // The track and its strip as their records hold them: the strip's children are its data.
        let mut data = strip.clone();
        data.as_object_mut().unwrap().remove("name");
        replica
            .transact(&Origin::person("p"), |tx| {
                let track = tx.create("NlaTrack", json!({ "name": "Track" }).as_object().unwrap().clone())?;
                let held = tx.create("NlaStrip", json!({ "name": "Meta", "data": data }).as_object().unwrap().clone())?;
                tx.list_insert(&track, "strips", 0, &held)?;
                tx.list_insert(&id, "nla", 0, &track)
            })
            .unwrap();
        assert!(read_scene(&replica, &MemoryBlobs::default(), &mut MeshCache::default()).is_err());
        assert!(replica.transact(&Origin::system("repair"), kition_scene_kdm::repair::repair).is_err());
    }
}
