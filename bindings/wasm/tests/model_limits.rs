//! C8-land-11c: scalar limits, opaque JSON limits, and truthful model ownership.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
use kition_kit::{KitDocument, session::model::Exchange};
use kition_scene_kit::SceneDocument;
use serde_json::json;

#[test]
fn numeric_model_fields_declare_the_engine_id_range() {
    let schema = kition_scene_kdm::schema();
    for (ty, def) in &schema.types {
        for (name, field) in &def.fields {
            if field.kind != kition_kit::kdm::FieldKind::Integer {
                continue;
            }
            assert_eq!(field.minimum, Some(1.into()), "{ty}.{name}");
            assert_eq!(field.maximum, Some(u32::MAX.into()), "{ty}.{name}");
            assert_eq!(field.multiple_of, Some(1.into()), "{ty}.{name}");
            for invalid in [json!(-1), json!(0), json!(u64::from(u32::MAX) + 1), json!(1.5)] {
                assert!(schema.check_value(ty, name, &invalid).is_err(), "{ty}.{name}: {invalid}");
            }
        }
    }
}

#[test]
fn batches_refuse_out_of_range_scalars_atomically() {
    let mut doc = SceneDocument::create("Scene.kscene", 320, 180).unwrap();
    let mut exchange = Exchange::new();
    let before = doc.save_file(None).unwrap();
    for value in [json!(0), json!(-1), json!(u64::from(u32::MAX) + 1), json!(1.5)] {
        let error = exchange
            .batch(
                &mut doc,
                &json!({"ops":[
                    {"op":"set","target":"root","field":"world.strength","value":2},
                    {"op":"set","target":"root","field":"camera","value":value}
                ]}),
                false,
            )
            .unwrap_err();
        assert_eq!(error.code, kition_kit::kdm::batch::BatchErrorCode::InvalidValue, "{error}");
        assert_eq!(doc.save_file(None).unwrap(), before);
    }
}

#[test]
fn node_group_lists_own_nodes_and_links_but_references_own_nothing() {
    let schema = kition_scene_kdm::schema();
    assert!(schema.types.values().flat_map(|ty| ty.fields.values()).all(|field| !field.owns), "Kition Scene has no owning ref or map fields");
    let mut doc = SceneDocument::create("Scene.kscene", 320, 180).unwrap();
    let material = doc.execute("material.new", json!({"name":"Shared","assign":false})).unwrap()["material"].as_u64().unwrap() as u32;
    doc.execute("material.assign", json!({"material":material,"objects":["Cube"]})).unwrap();
    let group = doc.execute("node.newGroup", json!({"name":"Owned","object":"Cube"})).unwrap()["group"].clone();
    let mut exchange = Exchange::new();
    let replica = exchange.replica(&mut doc).unwrap();
    let record = replica.objects_of_type("NodeGroup").into_iter().find(|id| replica.get(id, "key").unwrap() == group).unwrap();
    let children: Vec<_> = ["nodes", "links"].into_iter().flat_map(|field| replica.list(&record, field).unwrap()).collect();
    assert!(!children.is_empty());
    doc.execute("node.removeGroup", json!({"group":group})).unwrap();
    let replica = exchange.replica(&mut doc).unwrap();
    assert!(!replica.exists(&record));
    assert!(children.iter().all(|id| !replica.exists(id)), "the engine deletes both owning lists");
    doc.execute("edit.undo", json!({})).unwrap();
    assert_eq!(doc.scene().node_groups.len(), 1);
    let before_batch = doc.save_file(None).unwrap();
    exchange.batch(&mut doc, &json!({"ops":[{"op":"delete","target":record.to_string()}]}), false).unwrap();
    let replica = exchange.replica(&mut doc).unwrap();
    assert!(!replica.exists(&record));
    assert!(children.iter().all(|id| !replica.exists(id)));
    assert!(doc.scene().node_groups.is_empty());
    assert!(doc.scene().object_named("Cube").is_some(), "a group never owns its modifier's object");
    let mut doc = SceneDocument::open("Scene.kscene", &before_batch).unwrap();
    assert_eq!(doc.scene().node_groups.len(), 1);
    doc.execute("edit.delete", json!({"objects":["Cube"]})).unwrap();
    assert_eq!(doc.scene().node_groups.len(), 1, "an object never owns its node group");
    assert!(doc.scene().material(material).is_ok(), "an object never owns its material");
}
