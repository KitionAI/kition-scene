//! A surface object (`regressions/issue-object-kind-surface`): the engine gives its kind as
//! `surface`, which the schema's `Object.kind` enum must declare, or writing it fails.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::kdm::{ActorId, DocId, Origin, Replica};
use kition_kit::session::{BlobSink, MemoryBlobs};
use kition_scene_kdm::mesh_blob::MeshCache;
use kition_scene_kdm::read::read_scene;
use kition_scene_kdm::write::{stored_form, write_changes, write_scene};
use kition_scene_kdm::{root_id, schema};
use kition_scene_model::surface::SurfacePrimitive;
use kition_scene_model::{ObjectData, Scene};
use serde_json::Value;

#[test]
fn the_schema_declares_every_kind_the_engine_writes() {
    let parsed: Value = serde_json::from_str(kition_scene_kdm::SCHEMA).unwrap();
    let kinds = parsed["types"]["Object"]["fields"]["kind"]["values"].as_array().unwrap().clone();
    for kind in ["mesh", "light", "camera", "empty", "armature", "curve", "text", "lattice", "surface"] {
        assert!(kinds.contains(&Value::from(kind)), "Object.kind lacks `{kind}`");
    }
}

#[test]
fn a_surface_object_is_written_and_read_back() {
    let mut scene = Scene::starter();
    scene.add_object("SurfSphere", ObjectData::Surface(SurfacePrimitive::Sphere.build(1.0)));
    let mut blobs = MemoryBlobs::default();
    let (mut replica, written) =
        Replica::create_with(schema(), DocId::new(), ActorId::random(), &Origin::system("create"), |tx| write_scene(tx, &scene)).unwrap();
    for blob in written.blobs {
        blobs.put(&blob.bytes, "application/octet-stream");
    }
    let back = read_scene(&replica, &blobs, &mut MeshCache::default()).unwrap();
    assert_eq!(back.object_named("SurfSphere").unwrap().data, scene.object_named("SurfSphere").unwrap().data);
    let object = replica.list(&root_id(), "objects").unwrap().last().cloned().unwrap();
    assert_eq!(replica.get(&object, "kind").unwrap(), "surface");

    // Added by a command to a scene the replica holds.
    let before = stored_form(&back, &back);
    let mut after = before.clone();
    after.add_object("SurfTorus", ObjectData::Surface(SurfacePrimitive::Torus.build(1.0)));
    let after = stored_form(&after, &before);
    replica.transact(&Origin::person("p"), |tx| write_changes(tx, &before, &after, None)).unwrap();
    let back = read_scene(&replica, &blobs, &mut MeshCache::default()).unwrap();
    assert_eq!(back.object_named("SurfTorus").unwrap().data.kind(), "surface");
}
