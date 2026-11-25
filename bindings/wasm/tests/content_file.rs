//! A `.kscene` file holds the scene's content, not its history (kition-sdk data model 7.3 and 7.4,
//! rule M16b; feature `scene.file.content-only`): its `content.json` holds the state, no version,
//! lineage, actor or origin, and none of the deleted or overwritten content; an unchanged scene
//! saves to the same bytes; a file opened without a store is the same document, with the same
//! object ids, and edits; undo after a save takes back the last edit; and a file opens on top of a
//! stored replica, as it is when both hold the same state and by the file's state otherwise. The
//! file holds nothing of a person's session: two sessions that open the same file and save it
//! without an edit write the same bytes, whatever each one's view, selection and frame (those are
//! the person's view state, which the host keeps).
//!
//! The needles below are written once each and share no run of four characters with anything else
//! in the scene: every test checks the session's change log holds them (the control) before it
//! checks the file's content does not.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_kit::kdm::Version;
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

/// Every part of a file, uncompressed: its content and its metadata.
fn parts(bytes: &[u8]) -> Vec<u8> {
    let file = kition_scene_kscene::read(bytes).unwrap();
    let mut all = file.content.clone();
    all.extend_from_slice(file.meta.doc_id.as_bytes());
    all
}

/// The file as a host saves it.
fn host_save(doc: &mut SceneDocument) -> Vec<u8> {
    doc.save(Some(FILE)).unwrap()
}

/// The session's changes since `since` (the version it was made at): its change log, which holds
/// what it deleted and overwrote (the control for the needles).
fn change_log(doc: &SceneDocument, since: &Version) -> Vec<u8> {
    __export_changes(doc, &since.encode()).unwrap()
}

fn version(doc: &SceneDocument) -> Version {
    KitSessionDocument::session(doc).version()
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
    scene_with_history_made_at().0
}

/// [`scene_with_history`] and the version it was made at.
fn scene_with_history_made_at() -> (SceneDocument, Version) {
    let mut doc = SceneDocument::create(FILE, 320, 180).unwrap();
    let made_at = version(&doc);
    doc.execute("object.addText", json!({ "name": "Words", "body": "Kept", "location": [0, 4, 0] })).unwrap();
    doc.execute("text.insert", json!({ "object": "Words", "text": TYPED_THEN_DELETED })).unwrap();
    doc.execute("text.set", json!({ "object": "Words", "body": "Kept" })).unwrap();
    doc.execute("object.addCube", json!({ "name": DELETED_OBJECT, "location": [3, 0, 0] })).unwrap();
    doc.execute("edit.delete", json!({ "objects": [DELETED_OBJECT] })).unwrap();
    doc.execute("object.addCube", json!({ "name": OVERWRITTEN, "location": [-3, 0, 0] })).unwrap();
    doc.execute("object.rename", json!({ "object": OVERWRITTEN, "name": "Kept" })).unwrap();
    assert_eq!(body(&doc, "Words"), "Kept");
    (doc, made_at)
}

#[test]
fn an_unchanged_scene_saves_to_the_same_bytes_and_its_content_holds_no_history() {
    let (mut doc, made_at) = scene_with_history_made_at();
    let log = change_log(&doc, &made_at);
    for needle in NEEDLES {
        assert!(contains(&log, needle), "the control: the session's change log holds {needle}");
    }
    let bytes = host_save(&mut doc);
    for needle in NEEDLES {
        assert!(!contains(&parts(&bytes), needle), "the file holds {needle}");
    }
    let content = String::from_utf8(kition_scene_kscene::read(&bytes).unwrap().content).unwrap();
    let header = content.lines().next().unwrap();
    assert!(header.starts_with(&format!("{{\"kdm\":1,\"kit\":\"kit.scene\",\"docId\":\"{}\",\"schemaVersion\":", doc.doc_id())), "{header}");
    for key in ["\"version\"", "\"lineage\"", "\"seq\"", "\"actor\"", "\"origin\""] {
        assert!(!content.contains(key), "the content holds no {key}");
    }
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
    let saved = version(&doc);
    assert_eq!(host_save(&mut doc), first, "two saves with nothing between");
    assert_eq!(version(&doc), saved, "a save writes no change");
    doc.mark_saved();
    assert_eq!(host_save(&mut doc), first, "and after the host stored the first");
    let mut opened = SceneDocument::open(FILE, &first).unwrap();
    assert_eq!(ids(&opened), ids(&doc), "the file opened again holds the same object ids");
    assert_eq!(state(&opened), state(&doc), "and the same state, KDM object ids included");
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
    assert_eq!(state(&back), state(&doc), "the same state, KDM object ids included");
    let kept = id(&back, "Kept");
    back.execute("object.rename", json!({ "object": kept, "name": "Edited" })).unwrap();
    back.execute("text.insert", json!({ "object": "Words", "text": "!" })).unwrap();
    back.execute("object.addCube", json!({ "name": "New", "location": [0, -4, 0] })).unwrap();
    let saved = host_save(&mut back);
    let again = SceneDocument::open(FILE, &saved).unwrap();
    assert_eq!(again.doc_id(), doc.doc_id());
    assert_eq!(again.scene().object(kept).unwrap().name, "Edited");
    assert_eq!(body(&again, "Words"), "Kept!");
    assert!(again.scene().object_named("New").is_some());
}

#[test]
fn undo_after_a_save_takes_back_the_last_edit() {
    let mut doc = SceneDocument::create(FILE, 320, 180).unwrap();
    doc.execute("object.addCube", json!({ "name": "A", "location": [3, 0, 0] })).unwrap();
    let a = id(&doc, "A");
    doc.execute("object.translate", json!({ "objects": [a], "offset": [1, 0, 0] })).unwrap();
    host_save(&mut doc);
    doc.mark_saved();
    assert!(doc.inspect().unwrap()["canUndo"].as_bool().unwrap(), "a save is no undo step and clears none");
    doc.execute("edit.undo", json!({})).unwrap();
    assert_eq!(doc.scene().object(a).unwrap().location.x, 3.0, "the undo took back the move");
    doc.execute("edit.undo", json!({})).unwrap();
    assert!(doc.scene().object(a).is_err(), "then the cube");
    doc.execute("edit.redo", json!({})).unwrap();
    assert!(doc.scene().object(a).is_ok());
}

#[test]
fn a_file_opens_on_the_stored_replica_that_holds_its_state() {
    let mut doc = scene_with_history();
    let bytes = host_save(&mut doc);
    doc.mark_saved();
    let stored = __snapshot(&doc).unwrap();
    let mut opened = SceneDocument::open_with_replica(FILE, &bytes, &stored, &doc.blobs_bundle()).unwrap();
    assert_eq!(version(&opened), version(&doc), "the stored replica, as it was");
    assert_eq!(ids(&opened), ids(&doc));
    assert_eq!(host_save(&mut opened), bytes, "and saves the document it opened, writing nothing new");
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
    elsewhere.execute("object.addCube", json!({ "name": "New", "location": [0, -4, 0] })).unwrap();
    let edited = host_save(&mut elsewhere);
    let opened = SceneDocument::open_with_replica(FILE, &edited, &stored, &elsewhere.blobs_bundle()).unwrap();
    assert_eq!(opened.doc_id(), doc.doc_id());
    assert_eq!(state(&opened), state(&elsewhere), "the file's state, by object id");
    assert_eq!(ids(&opened), ids(&elsewhere));
    assert!(version(&opened).includes(&version(&doc)), "on the stored replica's line of changes");
    assert_ne!(version(&opened), version(&doc), "one change further: the file's state");
}

/// Two sessions open the same file, each moves its own view, selection, frame, shading, grid and
/// UV options, and saves without editing: both write the bytes of the file they opened. What each
/// session showed is its view state (`view.state`), which another session takes back
/// (`view.restore`) without an edit.
#[test]
fn two_sessions_that_open_the_same_file_and_save_without_editing_write_the_same_bytes() {
    let mut doc = scene_with_history();
    let file = host_save(&mut doc);
    let mut first = SceneDocument::open(FILE, &file).unwrap();
    let mut second = SceneDocument::open(FILE, &file).unwrap();
    let kept = id(&first, "Kept");
    for (command, params) in [
        ("view.orbit", json!({ "turn": 70, "tilt": -20 })),
        ("view.zoom", json!({ "factor": 0.4 })),
        ("view.wireframe", json!({})),
        ("view.grid", json!({ "on": false })),
        ("frame.set", json!({ "frame": 40 })),
        ("select.objects", json!({ "objects": [kept] })),
        ("view.resize", json!({ "width": 640, "height": 360 })),
    ] {
        first.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"));
    }
    for (command, params) in
        [("view.top", json!({})), ("view.overlays", json!({ "on": false })), ("edit.selectAll", json!({})), ("frame.set", json!({ "frame": 7 }))]
    {
        second.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"));
    }
    assert_ne!(first.view_state(), second.view_state(), "the control: the sessions show different things");
    assert!(!first.dirty() && !second.dirty(), "moving the view edits nothing");
    let (one, two) = (first.save_file(Some(FILE)).unwrap(), second.save_file(Some(FILE)).unwrap());
    assert!(one == two, "two sessions saved different bytes");
    assert!(one == file, "a session saved other bytes than the file it opened");
    // The view state travels apart from the file: a third session takes the first's back.
    let shown = first.execute("view.state", json!({})).unwrap()["state"].clone();
    assert_eq!(shown["v"], 1, "{shown}");
    let mut third = SceneDocument::open(FILE, &one).unwrap();
    let revision = third.revision();
    assert_eq!(third.execute("view.restore", json!({ "state": shown })).unwrap()["restored"], true);
    assert_eq!(third.view_state(), first.view_state(), "the first session's view, selection and frame");
    assert_eq!(third.inspect().unwrap()["animation"]["frame"], 40);
    assert!(third.revision() > revision && !third.dirty(), "a repaint, no edit");
    assert!(third.save_file(Some(FILE)).unwrap() == file, "and the file stays the same");
    // A state of another version, or one naming an object the scene no longer has.
    let mut other = shown.clone();
    other["v"] = json!(2);
    assert_eq!(third.execute("view.restore", json!({ "state": other })).unwrap()["restored"], false);
    let mut gone = shown;
    gone["selected"] = json!([kept, 999_999]);
    gone["active"] = json!(999_999);
    assert_eq!(third.execute("view.restore", json!({ "state": gone })).unwrap()["restored"], true);
    assert_eq!(third.view_state()["selected"], json!([kept]));
    assert_eq!(third.view_state()["active"], json!(kept));
}
