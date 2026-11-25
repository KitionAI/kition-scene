//! Session continuity on reread (kition-sdk call-context.md, "Session Continuity On Reread"): a
//! clean scene whose file another program saved takes the file's difference into its live replica,
//! keeping both undo histories, the editor state and the blobs; an unchanged file writes nothing;
//! a reread that removed what the editor state names is refused by name and changes nothing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use kition_kit::KitDocument;
use kition_kit::kdm::Origin;
use kition_kit::session::KitSessionDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

const FILE: &str = "Scene.kscene";

fn agent() -> Origin {
    Origin::agent("agent:test")
}

/// The starter scene with two more cubes, `Mine` and `Theirs` (selected and active, as the last
/// one added), and their ids.
fn scene() -> (SceneDocument, u32, u32) {
    let mut doc = SceneDocument::create(FILE, 320, 180).unwrap();
    doc.execute("object.addCube", json!({ "name": "Mine", "location": [3, 0, 0] })).unwrap();
    doc.execute("object.addCube", json!({ "name": "Theirs", "location": [6, 0, 0] })).unwrap();
    let id = |doc: &SceneDocument, name: &str| doc.scene().object_named(name).unwrap().id;
    let (mine, theirs) = (id(&doc, "Mine"), id(&doc, "Theirs"));
    (doc, mine, theirs)
}

fn name(doc: &SceneDocument, id: u32) -> String {
    doc.scene().object(id).unwrap().name.clone()
}

/// The scene saved and marked saved, as a host stores it; answers the bytes.
fn store(doc: &mut SceneDocument) -> Vec<u8> {
    let bytes = doc.save_file(Some(FILE)).unwrap();
    doc.mark_saved();
    bytes
}

/// Another program opens `bytes`, runs `steps` and saves; answers the file read again.
fn outside(bytes: &[u8], steps: &[(&str, Value)]) -> SceneDocument {
    let mut other = SceneDocument::open(FILE, bytes).unwrap();
    for (command, params) in steps {
        other.execute(command, params.clone()).unwrap();
    }
    SceneDocument::open(FILE, &other.save_file(Some(FILE)).unwrap()).unwrap()
}

#[test]
fn an_outside_save_keeps_the_agents_undo_and_the_files_edit() {
    let (mut live, mine, theirs) = scene();
    live.execute_as("object.rename", json!({ "object": mine, "name": "Mine edited" }), &agent()).unwrap();
    let bytes = store(&mut live);
    let mut fresh = outside(&bytes, &[("object.rename", json!({ "object": theirs, "name": "Theirs edited" }))]);
    let state = live.editor_state();
    fresh.restore_editor_state(&state).unwrap();
    let revision = live.revision();

    assert_eq!(live.reconcile_reread(&fresh), Ok(true));
    live.mark_saved();
    assert!(!live.dirty(), "a reconciled session is what the file holds");
    assert!(live.revision() > revision, "the views see the file's difference");
    assert_eq!((name(&live, mine), name(&live, theirs)), ("Mine edited".into(), "Theirs edited".into()));
    assert_eq!(live.editor_state(), state, "the editor state stays");

    assert_eq!(live.execute_as("edit.undo", json!({}), &agent()).map(|_| ()), Ok(()));
    assert_eq!(name(&live, mine), "Mine", "the agent's own step is still undoable");
    assert_eq!(name(&live, theirs), "Theirs edited", "undo keeps the file's edit");

    // The next save holds both writers' work.
    let saved = SceneDocument::open(FILE, &live.save_file(Some(FILE)).unwrap()).unwrap();
    assert_eq!((name(&saved, mine), name(&saved, theirs)), ("Mine".into(), "Theirs edited".into()));
}

#[test]
fn an_unchanged_reread_writes_nothing_and_keeps_the_history() {
    let (mut live, mine, _) = scene();
    live.execute("object.rename", json!({ "object": mine, "name": "Mine edited" })).unwrap();
    let bytes = store(&mut live);
    let version = KitSessionDocument::session(&live).version();
    let revision = live.revision();

    let fresh = SceneDocument::open(FILE, &bytes).unwrap();
    assert_eq!(live.reconcile_reread(&fresh), Ok(true));
    assert_eq!(KitSessionDocument::session(&live).version(), version, "nothing is written");
    assert_eq!(live.revision(), revision);
    assert!(!live.dirty());
    assert_eq!(live.save_file(Some(FILE)).unwrap(), bytes, "an unchanged save writes the file read again");
    live.execute("edit.undo", json!({})).unwrap();
    assert_eq!(name(&live, mine), "Mine");
}

#[test]
fn a_reread_that_deleted_the_selected_object_is_refused_by_name() {
    let (mut live, mine, theirs) = scene();
    live.execute("object.rename", json!({ "object": mine, "name": "Mine edited" })).unwrap();
    let bytes = store(&mut live);
    let state = live.editor_state();
    let before = live.save_file(Some(FILE)).unwrap();

    let mut fresh = outside(&bytes, &[("edit.delete", json!({ "objects": [theirs] }))]);
    let refused = fresh.restore_editor_state(&state).unwrap_err();
    assert!(refused.contains(&format!("deleted selected object {theirs}")), "{refused}");
    assert_eq!(live.reconcile_reread(&fresh), Err(format!("deleted selected object {theirs}")));
    assert_eq!(live.save_file(Some(FILE)).unwrap(), before, "a refused reread changes nothing");
    assert_eq!(live.editor_state(), state);
    assert_eq!(name(&live, theirs), "Theirs");

    // A file that keeps the selected object restores it.
    let mut fresh = outside(&bytes, &[("edit.delete", json!({ "objects": [mine] }))]);
    fresh.restore_editor_state(&state).unwrap();
    assert_eq!(fresh.editor_state()["selected"], json!([theirs]));
}

#[test]
fn a_file_of_another_scene_is_not_reconciled() {
    let (mut live, mine, _) = scene();
    live.execute("object.rename", json!({ "object": mine, "name": "Mine edited" })).unwrap();
    store(&mut live);
    let mut other = SceneDocument::create(FILE, 320, 180).unwrap();
    let other = SceneDocument::open(FILE, &other.save_file(Some(FILE)).unwrap()).unwrap();
    let before = live.save_file(Some(FILE)).unwrap();
    assert_eq!(live.reconcile_reread(&other), Ok(false));
    assert_eq!(live.save_file(Some(FILE)).unwrap(), before);
}
