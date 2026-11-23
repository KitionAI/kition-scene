//! A `.kscene` file holds the scene's content, not its history (kition-sdk data model 7.3, rule
//! M16b; feature `scene.file.content-only`): its `content.kdm` holds one change (the seal) and none
//! of the deleted or overwritten content; an unchanged scene saves to the same bytes; a file opened
//! without a store is the same document and edits; a replica from the file and one with the whole
//! history converge; undo after a save takes back the last edit; and a file opens on top of a
//! stored replica, by its changes or by its state. Hosts seal before saving (`prepare_save`, the
//! direct path); the one fallback, a save nobody sealed for (the kit editor before its release that
//! calls `prepareSave`), seals a copy and `mark_saved` takes the seal in.
//!
//! The needles below are written once each and share no run of four characters with anything else
//! in the scene, so the engine's compression keeps each one literal in the bytes that hold it:
//! every test checks the session's full snapshot holds them (the control) before it checks the
//! file's content does not (the method of kition-sdk's `crates/kition-kdm/tests/content_snapshot.rs`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_kit::kdm::{ActorId, Origin, OriginKind, Replica, Version};
use kition_kit::session::{__export_changes, __snapshot, KitSessionDocument};
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

const FILE: &str = "Scene.kscene";
const TYPED_THEN_DELETED: &str = "Jx4Hb8Ty3Nc6";
const DELETED_OBJECT: &str = "Qv7Zp9Wm2Rk5";
const OVERWRITTEN: &str = "Ug5Le1Fo0Dw4";
const NEEDLES: [&str; 3] = [TYPED_THEN_DELETED, DELETED_OBJECT, OVERWRITTEN];

fn contains(bytes: &[u8], needle: &str) -> bool {
    let utf16: Vec<u8> = needle.encode_utf16().flat_map(u16::to_le_bytes).collect();
    bytes.windows(needle.len()).any(|window| window == needle.as_bytes()) || bytes.windows(utf16.len()).any(|window| window == utf16.as_slice())
}

/// Every part of a file, uncompressed: its content, its metadata and its editor state.
fn parts(bytes: &[u8]) -> Vec<u8> {
    let file = kition_scene_kscene::read(bytes).unwrap();
    let mut all = file.content.clone();
    all.extend_from_slice(file.meta.doc_id.as_bytes());
    all.extend_from_slice(serde_json::to_string(&file.editor).unwrap().as_bytes());
    all
}

/// The file as a host saves it: sealed first (`prepare_save`), then the bytes.
fn host_save(doc: &mut SceneDocument) -> Vec<u8> {
    doc.prepare_save().unwrap();
    doc.save(Some(FILE)).unwrap()
}

fn version(doc: &SceneDocument) -> Version {
    KitSessionDocument::session(doc).version()
}

/// The version a file's content holds.
fn file_version(bytes: &[u8]) -> Version {
    Replica::load(kition_scene_kdm::schema(), &kition_scene_kscene::read(bytes).unwrap().content, ActorId::random()).unwrap().version()
}

fn state(doc: &SceneDocument) -> Value {
    KitSessionDocument::session(doc).replica().state_json().unwrap()
}

fn id(doc: &SceneDocument, name: &str) -> u32 {
    doc.scene().object_named(name).unwrap_or_else(|| panic!("no object {name}")).id
}

fn ids(doc: &SceneDocument) -> Vec<(u32, String)> {
    doc.scene().objects.iter().map(|object| (object.id, object.name.clone())).collect()
}

fn body(doc: &SceneDocument, name: &str) -> String {
    match &doc.scene().object_named(name).unwrap().data {
        kition_scene_model::ObjectData::Text(text) => text.body.clone(),
        other => panic!("{name} is no text: {other:?}"),
    }
}

/// A scene whose history holds typed-then-deleted text (in a 3D text's body), a deleted object and
/// an overwritten name; the text `Words` and the cube `Kept` stay.
fn scene_with_history() -> SceneDocument {
    let mut doc = SceneDocument::create(FILE, 320, 180).unwrap();
    doc.execute("add.text", json!({ "name": "Words", "body": "Kept", "location": [0, 4, 0] })).unwrap();
    doc.execute("text.insert", json!({ "object": "Words", "text": TYPED_THEN_DELETED })).unwrap();
    doc.execute("text.set", json!({ "object": "Words", "body": "Kept" })).unwrap();
    doc.execute("add.cube", json!({ "name": DELETED_OBJECT, "location": [3, 0, 0] })).unwrap();
    doc.execute("edit.delete", json!({ "objects": [DELETED_OBJECT] })).unwrap();
    doc.execute("add.cube", json!({ "name": OVERWRITTEN, "location": [-3, 0, 0] })).unwrap();
    doc.execute("object.rename", json!({ "object": OVERWRITTEN, "name": "Kept" })).unwrap();
    assert_eq!(body(&doc, "Words"), "Kept");
    doc
}

#[test]
fn the_file_holds_the_content_and_of_the_history_only_the_seal() {
    let mut doc = scene_with_history();
    let full = __snapshot(&doc).unwrap();
    for needle in NEEDLES {
        assert!(contains(&full, needle), "the control: the session's history holds {needle}");
    }
    let bytes = host_save(&mut doc);
    for needle in NEEDLES {
        assert!(!contains(&parts(&bytes), needle), "the file holds {needle}");
    }
    let held = Replica::snapshot_origins(&kition_scene_kscene::read(&bytes).unwrap().content).unwrap();
    assert_eq!(held.len(), 1, "of the history, the file holds one change: {held:?}");
    let origin = held[0].clone().unwrap();
    assert_eq!(origin, Origin::system("content-seal"));
    assert_eq!(origin.kind, OriginKind::System);
    assert!(Replica::snapshot_origins(&full).unwrap().len() > 1, "the control: the history holds every change");
    // The direct path: the seal is the session's own.
    assert_eq!(version(&doc), file_version(&bytes));
    let meta = kition_scene_kscene::read(&bytes).unwrap().meta;
    assert_eq!(meta.doc_id, doc.doc_id());
    assert_eq!(meta.schema_version, 1);
    // The state is all there.
    let back = SceneDocument::open(FILE, &bytes).unwrap();
    assert_eq!(state(&back), state(&doc));
    assert_eq!(body(&back, "Words"), "Kept");
    assert!(back.scene().object_named("Kept").is_some());
}

#[test]
fn an_unchanged_scene_saves_to_the_same_bytes() {
    let mut doc = scene_with_history();
    let first = host_save(&mut doc);
    let sealed = version(&doc);
    assert_eq!(host_save(&mut doc), first, "two saves with nothing between");
    assert_eq!(version(&doc), sealed, "a sealed scene seals nothing more");
    doc.mark_saved();
    assert_eq!(host_save(&mut doc), first, "and after the host stored the first");
    let mut opened = SceneDocument::open(FILE, &first).unwrap();
    assert_eq!(host_save(&mut opened), first, "and from the file opened again");
}

#[test]
fn a_file_opened_without_a_store_is_the_same_document_and_edits() {
    let mut doc = scene_with_history();
    let bytes = host_save(&mut doc);
    doc.mark_saved();
    let mut back = SceneDocument::open(FILE, &bytes).unwrap();
    assert_eq!(back.doc_id(), doc.doc_id());
    assert_eq!(ids(&back), ids(&doc), "the same object ids");
    assert_eq!(version(&back), version(&doc), "the same version: the seal is the session's");
    let kept = id(&back, "Kept");
    back.execute("object.rename", json!({ "object": kept, "name": "Edited" })).unwrap();
    back.execute("text.insert", json!({ "object": "Words", "text": "!" })).unwrap();
    back.execute("add.cube", json!({ "name": "New", "location": [0, -4, 0] })).unwrap();
    let saved = host_save(&mut back);
    let again = SceneDocument::open(FILE, &saved).unwrap();
    assert_eq!(again.doc_id(), doc.doc_id());
    assert_eq!(again.scene().object(kept).unwrap().name, "Edited");
    assert_eq!(body(&again, "Words"), "Kept!");
    assert!(again.scene().object_named("New").is_some());
}

#[test]
fn a_replica_from_the_file_and_one_with_the_whole_history_converge() {
    let mut full = scene_with_history();
    let bytes = host_save(&mut full);
    full.mark_saved();
    let mut file = SceneDocument::open(FILE, &bytes).unwrap();
    let kept = id(&full, "Kept");
    // Edits on both after the save.
    full.execute("object.translate", json!({ "objects": [kept], "offset": [0, 0, 2] })).unwrap();
    full.execute("add.cube", json!({ "name": "Full", "location": [6, 6, 0] })).unwrap();
    file.execute("object.rename", json!({ "object": kept, "name": "From the file" })).unwrap();
    file.execute("add.cube", json!({ "name": "File", "location": [-6, 6, 0] })).unwrap();
    for _ in 0..4 {
        let (to_file, to_full) =
            (__export_changes(&full, &version(&file).encode()).unwrap(), __export_changes(&file, &version(&full).encode()).unwrap());
        KitSessionDocument::import_changes(&mut full, &to_full).unwrap();
        KitSessionDocument::import_changes(&mut file, &to_file).unwrap();
        if version(&full) == version(&file) {
            break;
        }
    }
    assert_eq!(version(&full), version(&file));
    assert_eq!(state(&full), state(&file), "both replicas hold the same document");
    let object = full.scene().object(kept).unwrap();
    assert_eq!((object.name.as_str(), object.location.z), ("From the file", 2.0), "both edits survive");
    assert!(full.scene().object_named("Full").is_some() && full.scene().object_named("File").is_some());
}

#[test]
fn undo_after_a_save_takes_back_the_last_edit() {
    let mut doc = SceneDocument::create(FILE, 320, 180).unwrap();
    doc.execute("add.cube", json!({ "name": "A", "location": [3, 0, 0] })).unwrap();
    let a = id(&doc, "A");
    doc.execute("object.translate", json!({ "objects": [a], "offset": [1, 0, 0] })).unwrap();
    host_save(&mut doc);
    doc.mark_saved();
    assert!(doc.inspect().unwrap()["canUndo"].as_bool().unwrap(), "the seal is no undo step and clears none");
    doc.execute("edit.undo", json!({})).unwrap();
    assert_eq!(doc.scene().object(a).unwrap().location.x, 3.0, "the undo took back the move");
    doc.execute("edit.undo", json!({})).unwrap();
    assert!(doc.scene().object(a).is_err(), "then the cube");
    doc.execute("edit.redo", json!({})).unwrap();
    assert!(doc.scene().object(a).is_ok());
}

#[test]
fn a_file_opens_on_the_stored_replica_that_holds_its_version() {
    let mut doc = scene_with_history();
    let bytes = host_save(&mut doc);
    doc.mark_saved();
    let stored = __snapshot(&doc).unwrap();
    let mut opened = SceneDocument::open_with_replica(FILE, &bytes, &stored, &doc.blobs_bundle()).unwrap();
    assert_eq!(version(&opened), version(&doc), "the stored replica, as it was");
    assert!(!KitSessionDocument::session(&opened).replica().is_shallow(), "with its whole history");
    for needle in NEEDLES {
        assert!(contains(&__snapshot(&opened).unwrap(), needle), "the history keeps {needle}");
    }
    assert_eq!(file_version(&host_save(&mut opened)), file_version(&bytes), "and saves the document it opened, sealing nothing new");
}

#[test]
fn a_file_saved_elsewhere_opens_on_the_stored_replica_by_its_state() {
    let mut doc = scene_with_history();
    let stored = __snapshot(&doc).unwrap();
    let bytes = host_save(&mut doc);
    doc.mark_saved();
    // Elsewhere, from the file alone: an edit, a new cube, a save.
    let mut elsewhere = SceneDocument::open(FILE, &bytes).unwrap();
    let kept = id(&elsewhere, "Kept");
    elsewhere.execute("object.rename", json!({ "object": kept, "name": "Elsewhere" })).unwrap();
    elsewhere.execute("add.cube", json!({ "name": "New", "location": [0, -4, 0] })).unwrap();
    let edited = host_save(&mut elsewhere);
    let opened = SceneDocument::open_with_replica(FILE, &edited, &stored, &elsewhere.blobs_bundle()).unwrap();
    assert_eq!(opened.doc_id(), doc.doc_id());
    assert_eq!(state(&opened), state(&elsewhere), "the file's state, by object id");
    assert_eq!(ids(&opened), ids(&elsewhere));
    assert!(!KitSessionDocument::session(&opened).replica().is_shallow(), "on top of the stored history");
    assert!(contains(&__snapshot(&opened).unwrap(), DELETED_OBJECT), "which it keeps");
}

/// The fallback (delete with it once the kit editor calls `prepareSave`): a save nobody sealed for
/// writes the content of a sealed copy, the same bytes for the same version, and `mark_saved` takes
/// the seal into the session, which then exchanges changes with a replica of the file.
#[test]
fn a_save_nobody_sealed_for_seals_a_copy_and_mark_saved_takes_the_seal_in() {
    let mut doc = scene_with_history();
    let before = version(&doc);
    let first = doc.save(Some(FILE)).unwrap();
    assert_eq!(version(&doc), before, "the session is not sealed by a save");
    assert_eq!(doc.save(Some(FILE)).unwrap(), first, "the same version seals the same way");
    for needle in NEEDLES {
        assert!(!contains(&parts(&first), needle), "the file holds {needle}");
    }
    let held = Replica::snapshot_origins(&kition_scene_kscene::read(&first).unwrap().content).unwrap();
    assert_eq!(held, vec![Some(Origin::system("content-seal"))]);
    doc.mark_saved();
    assert_eq!(version(&doc), file_version(&first), "mark_saved took the seal in");
    let mut file = SceneDocument::open(FILE, &first).unwrap();
    doc.execute("add.cube", json!({ "name": "Session", "location": [0, 9, 0] })).unwrap();
    file.execute("add.cube", json!({ "name": "File", "location": [9, 9, 0] })).unwrap();
    let (to_file, to_doc) = (__export_changes(&doc, &version(&file).encode()).unwrap(), __export_changes(&file, &version(&doc).encode()).unwrap());
    KitSessionDocument::import_changes(&mut doc, &to_doc).unwrap();
    KitSessionDocument::import_changes(&mut file, &to_file).unwrap();
    assert_eq!(state(&doc), state(&file));
}
