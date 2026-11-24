//! C8-land-11c: typed names, field-aware carries and object-model limits.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
mod common;
use common::*;
use kition_kit::KitDocument;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn a_material_taking_an_objects_name_does_not_relocate_the_object() {
    let mut doc = scene_doc(2);
    let material = doc.execute("material.new", json!({"name":"M","assign":false})).unwrap()["material"].clone();
    let f = Fixture::from_bytes(&doc.save_file(None).unwrap(), Default::default());
    let (details, basis) = f.inspect().await;
    let cube = ids_by_name(&details)["C0"];
    let other = f.call_by("other", "batch", json!({"ops":[{"op":"set","target":material,"field":"name","value":"C0"}]})).await;
    assert!(other.get("error").is_none(), "{other}");
    f.ok("run", json!({"basis":basis,"steps":[{"command":"object.hide","params":{"objects":["C0"]}}]})).await;
    assert_eq!(object_on_disk(&f.file(), cube).unwrap()["hidden"], true);
    f.shutdown().await;
}

async fn disappeared_material_name(deleted: bool) {
    let mut doc = scene_doc(2);
    let material = doc.execute("material.new", json!({"name":"M","assign":false})).unwrap()["material"].clone();
    let f = Fixture::from_bytes(&doc.save_file(None).unwrap(), Default::default());
    let (_, basis) = f.inspect().await;
    let op = if deleted { json!({"op":"delete","target":material}) } else { json!({"op":"set","target":material,"field":"name","value":"Away"}) };
    let other = f.call_by("other", "batch", json!({"ops":[op]})).await;
    assert!(other.get("error").is_none(), "{other}");
    let before = std::fs::read(f.file()).unwrap();
    let refused = f.refused("run", json!({"basis":basis,"steps":[{"command":"material.set","params":{"material":"M","roughness":0.9}}]})).await;
    assert_conflict(&refused, &material.to_string(), if deleted { "deleted" } else { "relocated" });
    assert_eq!(std::fs::read(f.file()).unwrap(), before);
    f.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_renamed_non_target_name_is_relocated() {
    disappeared_material_name(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_deleted_non_target_name_is_deleted() {
    disappeared_material_name(true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn scalar_names_in_point_or_object_unions_are_guarded() {
    for deleted in [false, true] {
        let f = Fixture::new(2);
        let (details, basis) = f.inspect().await;
        let cube = ids_by_name(&details)["C0"];
        if deleted {
            f.colleague(&[("edit.delete", json!({"objects":[cube]}))]);
        } else {
            f.colleague(&[("object.rename", json!({"object":cube,"name":"Away"}))]);
        }
        let before = std::fs::read(f.file()).unwrap();
        let refused = f.refused("run", json!({"basis":basis,"steps":[{"command":"light.studio","params":{"target":"C0"}}]})).await;
        assert_conflict(&refused, &cube.to_string(), if deleted { "deleted" } else { "relocated" });
        assert_eq!(std::fs::read(f.file()).unwrap(), before);
        f.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_parent_rename_carries_no_world_transforms() {
    for outside in [false, true] {
        let mut doc = scene_doc(3);
        let names = ids_by_name(&doc.inspect().unwrap());
        let parent = names["C1"];
        doc.execute("object.parent", json!({"objects":[names["C0"]],"parent":parent})).unwrap();
        let f = Fixture::from_bytes(&doc.save_file(None).unwrap(), Default::default());
        let (_, basis) = f.inspect().await;
        let params = json!({"object":parent,"name":"Renamed"});
        if outside {
            f.colleague(&[("object.rename", params)]);
        } else {
            let renamed = f.call_by("other", "run", json!({"steps":[{"command":"object.rename","params":params}]})).await;
            assert!(entry(&renamed["changes"]["changed"], &parent.to_string()).get("carries").is_none(), "{renamed}");
        }
        let ran = f.ok("run", json!({"basis":basis,"steps":[{"command":"object.hide","on":[names["C2"]]}]})).await;
        assert!(entry(&ran["others"]["changed"], &parent.to_string()).get("carries").is_none(), "{ran}");
        f.shutdown().await;
    }
}
