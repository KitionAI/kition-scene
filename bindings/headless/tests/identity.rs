//! `document.identity` and `document.fork` (Kit Process Protocol section 5.6, KitModel M7a)
//! through the real `kition-scene` process: a `.kscene` embeds its replica as the document's
//! content (`content.kdm`, a content snapshot with no history, `docs/design.md` section 7.5), so a
//! byte copy of a file is the same document. A host reads the identity a file carries without parsing the scene, and forks a file
//! into a new document: a new docId, the same objects, ids and values, no history, only the blobs
//! the scene references, and `editor.json` as it was.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use kition_kit::KitDocument;
use kition_kit_host::{ClientError, HeadlessKit, LaunchOptions};
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const LIMIT: Duration = Duration::from_secs(30);

/// The name of an object deleted before the save: in the session's history, in neither file.
const SECRET: &str = "SECRET-DELETED-BEFORE-THE-FORK";

/// A blob no object references (another program left it in the file).
const STRAY: &[u8] = b"bytes no object of the scene references";

fn workspace(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kition-scene-identity-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::canonicalize(dir).unwrap()
}

async fn launch(dir: &Path) -> HeadlessKit {
    let mut options = LaunchOptions::new(env!("CARGO_BIN_EXE_kition-scene"));
    options.workspace = Some(dir.to_path_buf());
    HeadlessKit::launch(options).await.unwrap()
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A scene saved twice: first with two more cubes, one named [`SECRET`], then with that cube
/// deleted; then a stray blob added to the file.
fn scene() -> Vec<u8> {
    let mut doc = SceneDocument::create("Plan.kscene", 320, 180).unwrap();
    doc.execute("add.cube", json!({ "name": "Kept", "location": [3, 0, 0] })).unwrap();
    doc.execute("add.uvSphere", json!({ "name": SECRET, "location": [6, 0, 0] })).unwrap();
    let first = doc.save(None).unwrap();
    let mut doc = SceneDocument::open("Plan.kscene", &first).unwrap();
    doc.execute("edit.delete", json!({ "objects": [SECRET] })).unwrap();
    let saved = doc.save(None).unwrap();
    assert!(holds(&kition_kit::session::__snapshot(&doc).unwrap(), SECRET), "the control: the session's history holds the deleted name");
    let mut parts = kition_scene_kscene::read(&saved).unwrap();
    parts.blobs.push((sha256(STRAY), STRAY.to_vec()));
    kition_scene_kscene::write(&parts).unwrap()
}

/// Every object of the scene with its fields, by id: what must be equal in a fork.
async fn objects(kit: &HeadlessKit, path: &str) -> Value {
    let state = kit.open(path, false, LIMIT).await.unwrap();
    let document = state["documentId"].clone();
    let batch = |request: Value| {
        let mut request = request;
        request["documentId"] = document.clone();
        kit.request("model.batch", request, LIMIT)
    };
    let inventory = batch(json!({ "ops": [], "objects": true })).await.unwrap();
    let ids: Vec<Value> = inventory["objects"].as_array().unwrap().iter().map(|object| object["id"].clone()).collect();
    assert!(ids.len() >= 5, "the scene, Cube, Light, Camera and Kept: {inventory}");
    let load: Vec<Value> = ids.iter().map(|id| json!({ "target": id })).collect();
    let loaded = batch(json!({ "ops": [], "load": load })).await.unwrap();
    kit.request("document.close", json!({ "documentId": document }), LIMIT).await.unwrap();
    json!({ "objects": inventory["objects"], "loaded": loaded["loaded"] })
}

fn holds(bytes: &[u8], needle: &str) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle.as_bytes())
}

/// What the kit said about a refusal (`error.data.detail`).
fn detail(error: &ClientError) -> String {
    error.data().and_then(|data| data["detail"].as_str()).unwrap_or_default().to_owned()
}

#[tokio::test]
async fn a_scene_tells_its_identity_and_forks_into_a_new_document_without_history() {
    let dir = workspace("embedded");
    std::fs::write(dir.join("Plan.kscene"), scene()).unwrap();
    let kit = launch(&dir).await;
    assert!(kit.serves("document.identity"), "{}", kit.capabilities());

    // The identity the file carries, read without opening it, and the same for a byte copy.
    let source_id = kit.identity("Plan.kscene", LIMIT).await.unwrap().doc_id.expect("a scene file has an identity");
    let source_bytes = std::fs::read(dir.join("Plan.kscene")).unwrap();
    assert_eq!(SceneDocument::open("Plan.kscene", &source_bytes).unwrap().doc_id(), source_id, "the replica's own id");
    std::fs::copy(dir.join("Plan.kscene"), dir.join("Copy.kscene")).unwrap();
    assert_eq!(kit.identity("Copy.kscene", LIMIT).await.unwrap().doc_id.as_deref(), Some(source_id.as_str()), "a byte copy is the same document");

    // A fork is a new document with the same objects, ids and values.
    let fork = kit.fork("Plan.kscene", "Fork.kscene", LIMIT).await.unwrap();
    let fork_id = fork.doc_id.clone().expect("the fork's identity");
    assert_ne!(fork_id, source_id);
    assert_eq!(fork.name, "Fork.kscene");
    assert!(!fork.version.is_empty());
    assert_eq!(kit.identity("Fork.kscene", LIMIT).await.unwrap().doc_id.as_deref(), Some(fork_id.as_str()), "the forked file carries the new id");
    assert_eq!(kit.identity("Plan.kscene", LIMIT).await.unwrap().doc_id.as_deref(), Some(source_id.as_str()), "the source is unchanged");
    assert_eq!(std::fs::read(dir.join("Plan.kscene")).unwrap(), source_bytes, "the source's bytes are unchanged");
    assert_eq!(objects(&kit, "Fork.kscene").await, objects(&kit, "Plan.kscene").await, "the same objects, ids and values");

    // The fork's replica travels as a whole (an agent's draft of it: `model.export`, then
    // `model.open`), though it starts at the fork and has no earlier changes.
    let state = kit.open("Fork.kscene", true, LIMIT).await.unwrap();
    let exported = kit.request("model.export", json!({ "documentId": state["documentId"] }), LIMIT).await.unwrap();
    let draft = kit.request("model.open", json!({ "name": "Fork.kscene", "snapshot": exported["changes"] }), LIMIT).await.unwrap();
    assert_ne!(draft["documentId"], state["documentId"], "{draft}");
    kit.request("document.close", json!({ "documentId": state["documentId"] }), LIMIT).await.unwrap();

    // The fork's file: the editor's state kept, the referenced blobs and no others.
    let fork_bytes = std::fs::read(dir.join("Fork.kscene")).unwrap();
    let (source, forked) = (kition_scene_kscene::read(&source_bytes).unwrap(), kition_scene_kscene::read(&fork_bytes).unwrap());
    assert_eq!(forked.editor, source.editor, "editor.json is kept");
    assert_eq!(forked.meta.doc_id, fork_id, "meta.json's docId is the fork's");
    assert_eq!(forked.meta.schema_version, source.meta.schema_version);
    assert!(
        kition_kit::kdm::Replica::load(kition_scene_kdm::schema(), &forked.content, kition_kit::kdm::ActorId::random()).unwrap().is_sealed(),
        "the fork's content is sealed"
    );
    let stray = sha256(STRAY);
    assert!(source.blobs.iter().any(|(hash, _)| *hash == stray));
    let referenced: Vec<_> = source.blobs.iter().filter(|(hash, _)| *hash != stray).cloned().collect();
    assert!(!referenced.is_empty(), "the cubes' meshes are blobs");
    assert_eq!(forked.blobs, referenced, "the blobs the scene references, and no others");

    // The fork edits, saves and opens again as any scene, under its own id.
    let mut opened = SceneDocument::open("Fork.kscene", &fork_bytes).unwrap();
    assert_eq!(opened.doc_id(), fork_id);
    let count = opened.scene().objects.len();
    opened.execute("add.cube", json!({ "name": "New", "location": [0, 6, 0] })).unwrap();
    let edited = opened.save(None).unwrap();
    let reopened = SceneDocument::open("Fork.kscene", &edited).unwrap();
    assert_eq!((reopened.doc_id(), reopened.scene().objects.len()), (fork_id.clone(), count + 1));

    // No history: the deleted object's name is in neither file.
    assert!(!holds(&source.content, SECRET), "the source's file holds no deleted content");
    assert!(!holds(&forked.content, SECRET), "deleted content does not travel");

    // A fork of a fork is a third document with the same content.
    let again = kit.fork("Fork.kscene", "Again.kscene", LIMIT).await.unwrap().doc_id.expect("the second fork's identity");
    assert!(again != source_id && again != fork_id, "{again}");
    assert_eq!(objects(&kit, "Again.kscene").await, objects(&kit, "Plan.kscene").await);

    // A fork never replaces a file.
    let taken = kit.fork("Plan.kscene", "Fork.kscene", LIMIT).await.unwrap_err();
    assert_eq!(taken.reason(), Some("file_conflict"), "{taken}");
    assert_eq!(std::fs::read(dir.join("Fork.kscene")).unwrap(), fork_bytes, "the first fork is kept");
    kit.shutdown(Duration::from_secs(5)).await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn an_unsaved_scene_has_no_identity_and_bytes_that_are_no_scene_are_refused() {
    let dir = workspace("plain");
    let kit = launch(&dir).await;
    // An empty `.kscene` (a file made for the kit before it saved one) holds no replica.
    std::fs::write(dir.join("New.kscene"), b"").unwrap();
    assert_eq!(kit.identity("New.kscene", LIMIT).await.unwrap().doc_id, None);
    let fork = kit.fork("New.kscene", "NewCopy.kscene", LIMIT).await.unwrap();
    assert_eq!(fork.doc_id, None);
    assert_eq!(std::fs::read(dir.join("NewCopy.kscene")).unwrap(), b"", "a file without a replica forks unchanged");

    // Bytes that are not a scene, and a scene whose replica is damaged, are refused, never
    // answered with no identity.
    std::fs::write(dir.join("Text.kscene"), b"words a person wrote elsewhere").unwrap();
    let damaged = kition_scene_kscene::File {
        meta: kition_scene_kscene::Meta { kdm: 1, schema_version: 1, doc_id: "damaged".into() },
        content: b"not a replica".to_vec(),
        blobs: Vec::new(),
        editor: None,
    };
    std::fs::write(dir.join("Damaged.kscene"), kition_scene_kscene::write(&damaged).unwrap()).unwrap();
    for name in ["Text", "Damaged"] {
        let (file, copy) = (format!("{name}.kscene"), format!("{name}Copy.kscene"));
        for refused in [kit.identity(&file, LIMIT).await.unwrap_err(), kit.fork(&file, &copy, LIMIT).await.unwrap_err()] {
            assert_eq!(refused.reason(), Some("unsupported_format"), "{name}: {refused}: {}", detail(&refused));
        }
        assert!(!dir.join(copy).exists());
    }
    kit.shutdown(Duration::from_secs(5)).await;
    let _ = std::fs::remove_dir_all(dir);
}
