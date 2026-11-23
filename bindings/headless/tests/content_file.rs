//! The headless `kition-scene` saves on the direct path (feature `scene.file.content-only`): the
//! host seals the session (`prepare_save`) before every save, so the file an agent's batch writes
//! holds the scene's content and, of its history, only that seal.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::*;
use kition_kit::kdm::{Origin, Replica};
use serde_json::json;

/// Written once, overwritten at once: in no file.
const OVERWRITTEN: &str = "Ug5Le1Fo0Dw4";

fn origins(file: &[u8]) -> Vec<Option<Origin>> {
    Replica::snapshot_origins(&kition_scene_kscene::read(file).unwrap().content).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_batches_save_sealed_content() {
    let f = Fixture::new(2);
    assert_eq!(origins(&std::fs::read(f.file()).unwrap()), vec![Some(Origin::system("content-seal"))], "the fixture's file");
    let (details, _) = f.inspect().await;
    let cube = ids_by_name(&details)["C0"];
    let first = f.ok("batch", json!({ "ops": [{ "op": "set", "target": cube, "field": "name", "value": OVERWRITTEN }] })).await;
    assert!(first.get("error").is_none(), "{first}");
    let second = f.ok("batch", json!({ "ops": [{ "op": "set", "target": cube, "field": "name", "value": "Final" }] })).await;
    assert!(second.get("error").is_none(), "{second}");
    let file = std::fs::read(f.file()).unwrap();
    assert_eq!(origins(&file), vec![Some(Origin::system("content-seal"))], "of the history, the file holds the seal");
    let content = kition_scene_kscene::read(&file).unwrap().content;
    assert!(!content.windows(OVERWRITTEN.len()).any(|window| window == OVERWRITTEN.as_bytes()), "the overwritten name is in no file");
    let named = on_disk(&f.file()).scene().object(u32::try_from(cube).unwrap()).unwrap().name.clone();
    assert_eq!(named, "Final");
    f.shutdown().await;
}
