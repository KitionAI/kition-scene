//! Binary USD is an export, so it leaves revision, dirty state, selection and undo alone.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::json;

#[test]
fn binary_usd_export_is_read_only_and_selected_only_round_trips() {
    let mut created = SceneDocument::create("scene.kscene", 320, 240).unwrap();
    let mut doc = created.reopen("scene.kscene").unwrap();
    let snapshot = doc.save_file(None).unwrap();
    let before = doc.inspect().unwrap();
    let revision = doc.revision();
    let dirty = doc.dirty();
    let exported = doc.execute("file.export", json!({"format":"usdc","selectedOnly":true})).unwrap();
    assert_eq!(exported["mime"], "application/octet-stream");
    assert_eq!(exported["extension"], "usdc");
    assert!(exported["base64"].as_str().unwrap().starts_with("UFhSLVVTREM"));
    assert_eq!(doc.inspect().unwrap(), before);
    assert_eq!(doc.revision(), revision);
    assert_eq!(doc.dirty(), dirty);
    assert_eq!(doc.save_file(None).unwrap(), snapshot);
    let imported = doc.execute("file.import", json!({"format":"usdc","base64":exported["base64"]})).unwrap();
    assert_eq!(imported["objects"].as_array().unwrap().len(), 1);
    assert_eq!(doc.inspect().unwrap()["objects"].as_array().unwrap().len(), 4);
    doc.execute("edit.undo", json!({})).unwrap();
    assert_eq!(doc.inspect().unwrap()["objects"], before["objects"]);
}
