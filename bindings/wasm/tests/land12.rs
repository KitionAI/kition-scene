//! C8-land-12-scene: JSON bounds, union declarations and direct-session cache refresh.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
use kition_kit::{
    KitDocument,
    session::{
        KitSessionDocument,
        model::{Exchange, session_batch},
    },
};
use kition_scene_kit::SceneDocument;
use serde_json::json;

#[test]
fn material_json_members_refuse_out_of_range_path_and_whole_writes_atomically() {
    let mut doc = SceneDocument::create("Scene.kscene", 320, 180).unwrap();
    let material = doc.execute("material.new", json!({"name":"M","assign":false})).unwrap()["material"].clone();
    let record = KitSessionDocument::session(&doc)
        .replica()
        .objects_of_type("Material")
        .into_iter()
        .find(|id| KitSessionDocument::session(&doc).replica().get(id, "key").unwrap() == material)
        .unwrap();
    let mut exchange = Exchange::new();
    for (member, values) in [
        ("roughness", vec![json!(-0.1), json!(2)]),
        ("metallic", vec![json!(-0.1), json!(2)]),
        ("transmission", vec![json!(-0.1), json!(2)]),
        ("alphaCutoff", vec![json!(-0.1), json!(2)]),
        ("ior", vec![json!(0), json!(11)]),
        ("normalStrength", vec![json!(-11), json!(11)]),
        ("emissionStrength", vec![json!(-1)]),
    ] {
        for value in values {
            for whole in [false, true] {
                let before = doc.save_file(None).unwrap();
                let data = KitSessionDocument::session(&doc).replica().get(&record, "data").unwrap();
                let (field, value) = if whole {
                    let mut data = data;
                    data[member] = value.clone();
                    ("data".to_owned(), data)
                } else {
                    (format!("data.{member}"), value.clone())
                };
                let error = exchange
                    .batch(
                        &mut doc,
                        &json!({"ops":[
                            {"op":"set","target":"root","field":"world.strength","value":2},
                            {"op":"set","target":record.to_string(),"field":field,"value":value}
                        ]}),
                        false,
                    )
                    .unwrap_err();
                assert_eq!(error.code, kition_kit::kdm::batch::BatchErrorCode::InvalidValue, "{error}");
                assert_eq!(doc.save_file(None).unwrap(), before, "{member}, whole={whole}");
            }
        }
    }
}

#[test]
fn direct_batch_delete_refreshes_the_cache_before_the_next_command() {
    let mut doc = SceneDocument::create("Scene.kscene", 320, 180).unwrap();
    let group = doc.execute("nodes.newGroup", json!({"name":"G","tree":"shader"})).unwrap()["group"].clone();
    let record = KitSessionDocument::session(&doc)
        .replica()
        .objects_of_type("NodeGroup")
        .into_iter()
        .find(|id| KitSessionDocument::session(&doc).replica().get(id, "key").unwrap() == group)
        .unwrap();
    session_batch(&mut doc, &json!({"ops":[{"op":"delete","target":record.to_string()}]}), false).unwrap();
    assert!(doc.scene().node_groups.is_empty(), "the cached scene still holds the deleted group");
    let replacement = doc.execute("nodes.newGroup", json!({"name":"G","tree":"shader"})).unwrap()["group"].as_u64().unwrap() as u32;
    assert_eq!(doc.scene().node_group(replacement).unwrap().name, "G", "name allocation must read the refreshed cache");
    let reopened = SceneDocument::open("Scene.kscene", &doc.save_file(None).unwrap()).unwrap();
    assert_eq!(reopened.scene().node_groups.len(), 1);
    assert_eq!(reopened.scene().node_group(replacement).unwrap().name, "G");
}

#[test]
fn point_or_object_unions_declare_ordinary_typed_names() {
    let doc = SceneDocument::create("Scene.kscene", 320, 180).unwrap();
    let commands = doc.commands();
    for (id, path) in [
        ("camera.move", "target"),
        ("camera.move", "to"),
        ("light.studio", "target"),
        ("object.hop", "to"),
        ("object.gather", "center"),
        ("object.wave", "origin"),
    ] {
        let command = commands.iter().find(|command| command.id == id).unwrap();
        assert!(command.name_params.contains(&(path, "name")), "{id} {path}");
        assert!(command.object_types.contains(&(path, &["Object"])), "{id} {path}");
        assert!(command.schema.as_ref().unwrap()[path].get("x-khp-name").is_none(), "{id} {path}");
    }
}

#[test]
fn every_declared_json_member_bound_accepts_its_edges_and_refuses_values_beyond_them() {
    let schema = kition_scene_kdm::schema();
    let mut count = 0;
    for (ty, definition) in &schema.types {
        for (field, definition) in &definition.fields {
            for (member, limits) in &definition.members {
                count += 1;
                for (edge, outside) in [(limits.minimum.as_ref(), -1.0), (limits.maximum.as_ref(), 1.0)] {
                    if let Some(edge) = edge {
                        assert!(schema.check_value(ty, field, &json!({member:edge})).is_ok(), "{ty}.{field}.{member} edge {edge}");
                        let outside = edge.as_f64().unwrap() + outside;
                        assert!(schema.check_value(ty, field, &json!({member:outside})).is_err(), "{ty}.{field}.{member}: {outside}");
                    }
                }
                if limits.multiple_of.is_some() {
                    assert!(schema.check_value(ty, field, &json!({member:1.5})).is_err(), "{ty}.{field}.{member}: fractional count/id");
                }
            }
        }
    }
    assert_eq!(count, 47);
}

#[test]
fn root_json_limits_refuse_invalid_counts_and_factors_in_direct_batches() {
    let mut doc = SceneDocument::create("Scene.kscene", 320, 180).unwrap();
    for (field, invalid) in [
        ("render.width", json!(0)),
        ("render.height", json!(8193)),
        ("render.width", json!(1.5)),
        ("timeline.start", json!(-1048575)),
        ("timeline.end", json!(1048575)),
        ("timeline.fps", json!(0)),
        ("timeline.fps", json!(1001)),
        ("world.strength", json!(-1)),
    ] {
        let before = doc.save_file(None).unwrap();
        let error = session_batch(&mut doc, &json!({"ops":[{"op":"set","target":"root","field":field,"value":invalid}]}), false).unwrap_err();
        assert_eq!(error.code, kition_kit::kdm::batch::BatchErrorCode::InvalidValue, "{error}");
        assert_eq!(doc.save_file(None).unwrap(), before, "{field}");
    }
}
