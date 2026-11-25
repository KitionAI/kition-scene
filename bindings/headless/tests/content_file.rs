//! The headless `kition-scene` saves on the direct path (feature `scene.file.content-only`): the
//! file an agent's batch writes holds the scene's content (`content.json`) and nothing of its
//! history: no version, lineage, actor or origin, and no overwritten value.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::*;
use serde_json::json;

/// Written once, overwritten at once: in no file.
const OVERWRITTEN: &str = "Ug5Le1Fo0Dw4";

/// Whether a file's content is the state alone: the content encoding's header with no version,
/// and no lineage, actor or origin anywhere (kition-sdk data model 7.4, C10).
fn holds_no_history(file: &[u8]) -> bool {
    let content = String::from_utf8(kition_scene_kscene::read(file).unwrap().content).unwrap();
    content.starts_with("{\"kdm\":1,\"kit\":\"kit.scene\",\"docId\":")
        && ["\"version\"", "\"lineage\"", "\"actor\"", "\"origin\""].iter().all(|key| !content.contains(key))
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_batches_save_content_without_history() {
    let f = Fixture::new(2);
    assert!(holds_no_history(&std::fs::read(f.file()).unwrap()), "the fixture's file");
    let (details, _) = f.inspect().await;
    let cube = ids_by_name(&details)["C0"];
    let first = f.ok("batch", json!({ "ops": [{ "op": "set", "target": cube, "field": "name", "value": OVERWRITTEN }] })).await;
    assert!(first.get("error").is_none(), "{first}");
    let second = f.ok("batch", json!({ "ops": [{ "op": "set", "target": cube, "field": "name", "value": "Final" }] })).await;
    assert!(second.get("error").is_none(), "{second}");
    let file = std::fs::read(f.file()).unwrap();
    assert!(holds_no_history(&file), "the file holds no history");
    let content = kition_scene_kscene::read(&file).unwrap().content;
    assert!(!content.windows(OVERWRITTEN.len()).any(|window| window == OVERWRITTEN.as_bytes()), "the overwritten name is in no file");
    let named = on_disk(&f.file()).scene().object(u32::try_from(cube).unwrap()).unwrap().name.clone();
    assert_eq!(named, "Final");
    f.shutdown().await;
}
