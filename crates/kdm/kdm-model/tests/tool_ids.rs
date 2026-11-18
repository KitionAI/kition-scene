//! Tool ids (KitObjectModel OM9, task card C8-scene): every object, material, node group and node
//! holds the engine's id (the id `scene_inspect` lists and commands take) in its read-only `key`
//! field, and every image its content hash in `hash`, which the schema names as the type's tool
//! id. The ids are stored in the `.kscene` file's replica, so they survive saving, reopening and
//! another program rewriting the file. Objects, materials, node groups and nodes share one id
//! space (no prefix: the engine mints them from one counter per actor). The post-merge repair
//! numbers a record that has no key (a batch created it: the field is read-only) and gives
//! another key to an object or material whose key another object or material holds, the same on
//! every replica.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::kdm::{ActorId, DocId, ObjectId, Origin, Replica};
use kition_kit::session::MemoryBlobs;
use kition_scene_kdm::mesh_blob::MeshCache;
use kition_scene_kdm::read::read_scene;
use kition_scene_kdm::write::write_scene;
use kition_scene_kdm::{repair, root_id, schema};
use kition_scene_model::Scene;
use serde_json::{Map, Value, json};

fn create(scene: &Scene) -> (Replica, MemoryBlobs) {
    let mut blobs = MemoryBlobs::default();
    let (replica, written) =
        Replica::create_with(schema(), DocId::new(), ActorId::random(), &Origin::system("create"), |tx| write_scene(tx, scene)).unwrap();
    for blob in written.blobs {
        kition_kit::session::BlobSink::put(&mut blobs, &blob.bytes, "application/octet-stream");
    }
    (replica, blobs)
}

fn sync(a: &mut Replica, b: &mut Replica) {
    let to_b = a.export_changes(&b.version()).unwrap();
    let to_a = b.export_changes(&a.version()).unwrap();
    b.import_changes(&to_b).unwrap();
    a.import_changes(&to_a).unwrap();
    for replica in [&mut *a, &mut *b] {
        replica.transact(&Origin::system("repair"), |tx| Ok(repair::repair(tx).unwrap())).unwrap();
    }
}

/// The schema names `key` as the tool id of objects, materials, node groups and nodes (read-only
/// and optional, so a batch may create them), and `hash` as an image's; tool ids change nothing
/// stored, so the schema version stays.
#[test]
fn the_schema_declares_tool_ids() {
    let parsed: Value = serde_json::from_str(kition_scene_kdm::SCHEMA).unwrap();
    for ty in ["Object", "Material", "NodeGroup", "Node"] {
        assert_eq!(parsed["types"][ty]["toolId"], json!({ "field": "key" }), "{ty}");
        assert_eq!(
            parsed["types"][ty]["fields"]["key"],
            json!({ "kind": "integer", "optional": true, "readOnly": true,
            "minimum": 1, "maximum": u32::MAX, "multipleOf": 1 }),
            "{ty}"
        );
    }
    assert_eq!(parsed["types"]["Image"]["toolId"], json!({ "field": "hash" }));
    assert_eq!(parsed["types"]["Image"]["fields"]["hash"]["readOnly"], true);
    assert!(parsed["types"]["Link"].get("toolId").is_none(), "links are told under their node group");
    assert_eq!(schema().schema_version, 6, "tool ids change nothing stored");
}

/// The fields of `object` but `key`: a copy a batch could create.
fn without_key(replica: &Replica, object: &ObjectId) -> Map<String, Value> {
    let ty = replica.object_type(object).unwrap();
    let mut fields = Map::new();
    for name in schema().types[&ty].fields.keys() {
        if name == "key" {
            continue;
        }
        let value = replica.get(object, name).unwrap_or(Value::Null);
        if !value.is_null() {
            fields.insert(name.clone(), value);
        }
    }
    fields
}

/// An object and a material a peer's batch created without a key are numbered by the repair, the
/// same on both replicas, and the scene reads them with those keys (before, an object without a
/// key was left out of the scene).
#[test]
fn records_created_without_a_key_are_numbered_alike_on_every_replica() {
    let (mut a, blobs) = create(&Scene::starter());
    let mut b = Replica::load(schema(), &a.snapshot().unwrap(), ActorId::random()).unwrap();
    let cube = a.list(&root_id(), "objects").unwrap()[0].clone();
    let material = a.list(&root_id(), "materials").unwrap()[0].clone();
    let (object_fields, material_fields) = (without_key(&a, &cube), without_key(&a, &material));
    b.transact(&Origin::person("b"), |tx| {
        let made = tx.create("Object", object_fields.clone())?;
        tx.list_insert(&root_id(), "objects", 0, &made)?;
        let made = tx.create("Material", material_fields.clone())?;
        tx.list_insert(&root_id(), "materials", 1, &made)?;
        Ok(())
    })
    .unwrap();
    sync(&mut a, &mut b);
    assert_eq!(a.state_json().unwrap(), b.state_json().unwrap(), "both replicas write the same keys");
    let keys: Vec<Value> =
        ["objects", "materials"].iter().flat_map(|list| a.list(&root_id(), list).unwrap()).map(|object| a.get(&object, "key").unwrap()).collect();
    assert!(keys.iter().all(Value::is_u64), "every record numbered: {keys:?}");
    let mut unique = keys.clone();
    unique.sort_by_key(|key| key.as_u64());
    unique.dedup();
    assert_eq!(unique.len(), keys.len(), "every record has its own key: {keys:?}");
    let scene = read_scene(&a, &blobs, &mut MeshCache::default()).unwrap();
    assert_eq!(scene.objects.len(), 4, "the copy is read");
    assert_eq!(scene.materials.len(), 2);
}

/// A material whose key an object holds (two replicas that each numbered a new record alike) is
/// given another: objects and materials share one id space.
#[test]
fn a_material_holding_an_objects_key_is_given_another() {
    let (mut a, _) = create(&Scene::starter());
    let mut b = Replica::load(schema(), &a.snapshot().unwrap(), ActorId::random()).unwrap();
    let cube = a.list(&root_id(), "objects").unwrap()[0].clone();
    let material = a.list(&root_id(), "materials").unwrap()[0].clone();
    let taken = a.get(&cube, "key").unwrap();
    b.transact(&Origin::person("b"), |tx| tx.set(&material, "key", taken.clone())).unwrap();
    sync(&mut a, &mut b);
    let now = a.get(&material, "key").unwrap();
    assert_eq!(now, b.get(&material, "key").unwrap(), "the same on both replicas");
    assert_ne!(now, taken, "the material was given a key of its own");
    assert_eq!(a.get(&cube, "key").unwrap(), taken, "the object keeps its key");
}
