//! FBX and USD through `file.export` and `file.import`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn doc() -> SceneDocument {
    let mut doc = SceneDocument::create("Untitled scene.kscene", 1280, 720).unwrap();
    SceneDocument::open("t.kscene", &doc.save_file(None).unwrap()).unwrap()
}

fn names(doc: &SceneDocument) -> Vec<String> {
    doc.inspect().unwrap()["objects"].as_array().unwrap().iter().map(|object| object["name"].as_str().unwrap().to_string()).collect()
}

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

#[test]
fn fbx_usda_and_usdz_export_and_import_back_into_the_scene() {
    for (format, mime, magic) in [
        ("fbx", "application/octet-stream", "S2F5ZGFyYSBGQlggQmluYXJ5"),
        ("usda", "model/vnd.usda", "I3VzZGEgMS4w"),
        ("usdz", "model/vnd.usdz+zip", "UEsDB"),
    ] {
        let mut doc = doc();
        let exported = run(&mut doc, "file.export", json!({ "format": format }));
        assert_eq!(exported["mime"], mime);
        assert_eq!(exported["extension"], format);
        let base64 = exported["base64"].as_str().unwrap().to_string();
        assert!(base64.starts_with(magic), "{format}: {}", &base64[..24]);
        let imported = run(&mut doc, "file.import", json!({ "base64": base64, "format": format }));
        assert_eq!(imported["objects"].as_array().unwrap().len(), 3, "{format}");
        // The copies sit beside the originals under new names, and are selected.
        assert_eq!(names(&doc).len(), 6, "{format}: {:?}", names(&doc));
        assert_eq!(doc.inspect().unwrap()["selection"]["objects"].as_array().map(Vec::len), Some(3));
        run(&mut doc, "edit.undo", json!({}));
        assert_eq!(names(&doc), ["Cube", "Light", "Camera"]);
    }
}

#[test]
fn binary_usd_exports_succeed_and_unknown_or_malformed_imports_are_refused() {
    let mut doc = doc();
    let exported = run(&mut doc, "file.export", json!({ "format": "usdc" }));
    assert_eq!(exported["mime"], "application/octet-stream");
    assert_eq!(exported["extension"], "usdc");
    assert!(exported["base64"].as_str().unwrap().starts_with("UFhSLVVTREM"));
    let imported = run(&mut doc, "file.import", json!({ "base64": exported["base64"], "format": "usdc" }));
    assert_eq!(imported["objects"].as_array().unwrap().len(), 3);
    assert_eq!(names(&doc).len(), 6);
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(names(&doc), ["Cube", "Light", "Camera"]);
    let error = doc.execute("file.import", json!({ "base64": "AAAA", "format": "dae" })).unwrap_err().to_string();
    assert!(error.contains("fbx") && error.contains("usdz"), "{error}");
    // "PXR-USDC" and padding.
    let error = doc.execute("file.import", json!({ "base64": "UFhSLVVTREMAAAAA", "format": "usd" })).unwrap_err().to_string();
    assert!(error.contains("unsupported"), "{error}");
    assert_eq!(names(&doc), ["Cube", "Light", "Camera"]);
}

#[test]
fn curves_and_texts_go_out_to_fbx_and_usda_as_meshes() {
    for format in ["fbx", "usda"] {
        let mut doc = doc();
        run(&mut doc, "edit.delete", json!({ "objects": ["Cube", "Light", "Camera"] }));
        run(&mut doc, "object.addCurveCircle", json!({}));
        run(&mut doc, "object.addText", json!({}));
        let exported = run(&mut doc, "file.export", json!({ "format": format }));
        let base64 = exported["base64"].as_str().unwrap().to_string();
        let imported = run(&mut doc, "file.import", json!({ "base64": base64, "format": format }));
        let ids: Vec<u64> = imported["objects"].as_array().unwrap().iter().filter_map(Value::as_u64).collect();
        let objects = doc.inspect().unwrap()["objects"].as_array().unwrap().clone();
        let back: Vec<&Value> = objects.iter().filter(|object| object["id"].as_u64().is_some_and(|id| ids.contains(&id))).collect();
        // The text always makes faces; the circle has none until it is bevelled or filled, so it
        // may come back with no geometry.
        assert!(!back.is_empty(), "{format}: nothing imported");
        assert!(back.iter().all(|object| object["type"] == "mesh"), "{format}: {back:?}");
        assert!(back.iter().any(|object| object["mesh"]["faces"].as_u64().unwrap_or(0) > 0), "{format}: the text's faces came back");
    }
}
