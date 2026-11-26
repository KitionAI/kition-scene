//! C8-scene-engine 5a: what an agent's `edit.undo` and `edit.redo` rest on (`KitDocument::under`).
//! The kit writes each command's changes itself and keeps a record per undo step of each scope of
//! the objects (by id) and the scene's own fields it wrote; an undo holds those of the steps it may
//! take back (the scope's undo steps; a redo its redo steps), every field and position, and the
//! scene's fields they wrote. With no step to take back it holds nothing (an undo later in the same
//! run takes back that run's own step, which that step holds). When the replica keeps steps the
//! kit recorded nothing of (a model batch's), what an undo takes back is not known: the whole scene.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use kition_kit::KitDocument;
use kition_kit::kdm::Origin;
use kition_kit::session::KitSessionDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

/// Each premise of `command`'s answer: (id, every field?, every position?).
fn held(doc: &SceneDocument, command: &str, params: Value) -> Vec<(Value, bool, bool)> {
    doc.under(command, &params)
        .unwrap()
        .map(|under| under.premises.into_iter().map(|premise| (premise.id, premise.fields.is_none(), premise.positions.is_none())).collect())
        .unwrap_or_default()
}

fn named(doc: &SceneDocument, name: &str) -> Value {
    json!(doc.scene().object_named(name).unwrap().id)
}

#[test]
fn an_undo_holds_the_objects_of_the_steps_it_takes_back() {
    let mut doc = SceneDocument::create("Undo.kscene", 320, 180).unwrap();
    let agent = Origin::agent("agent:test");
    doc.execute_as("object.addCube", json!({ "name": "A", "location": [3, 0, 0] }), &agent).unwrap();
    let mut fresh = SceneDocument::open("Undo.kscene", &doc.save_file(None).unwrap()).unwrap();
    // A document read again has no step to take back: the undo holds nothing.
    assert_eq!(held(&fresh, "edit.undo", json!({})), vec![], "a reopened document's undo");
    let (a, cube) = (named(&fresh, "A"), named(&fresh, "Cube"));
    fresh.execute_as("object.rename", json!({ "object": a, "name": "Mine" }), &agent).unwrap();
    assert_eq!(held(&fresh, "edit.undo", json!({})), vec![(a.clone(), true, true)], "the undo of the rename");
    assert_eq!(held(&fresh, "edit.undo", json!({ "scope": "agent" })), vec![(a.clone(), true, true)]);
    // The person's scope has no step.
    assert_eq!(held(&fresh, "edit.undo", json!({ "scope": "person" })), vec![]);
    assert_eq!(held(&fresh, "edit.redo", json!({})), vec![], "nothing to redo");
    fresh.execute_as("object.addCamera", json!({ "name": "Side", "location": [9, 0, 1] }), &agent).unwrap();
    fresh.execute_as("object.setCamera", json!({ "object": named(&fresh, "Side") }), &agent).unwrap();
    let undo = held(&fresh, "edit.undo", json!({}));
    assert!(
        undo.contains(&(a.clone(), true, true)) && undo.contains(&(json!("scene"), false, false)) && !undo.contains(&(cube.clone(), true, true)),
        "{undo:?}"
    );
    fresh.execute_as("edit.undo", json!({}), &agent).unwrap();
    let redo = held(&fresh, "edit.redo", json!({}));
    assert!(redo.contains(&(json!("scene"), false, false)) && !redo.contains(&(cube, true, true)), "{redo:?}");
    // A model batch's step is the replica's, not recorded by the kit: the whole scene.
    fresh.batch_applied();
    assert_eq!(held(&fresh, "edit.undo", json!({})), vec![(json!("scene"), true, true)]);
}

/// A person's undo whose last step's object an agent deleted since: the replica passes that step
/// over and takes back the one below in the same call (`KitSession::undo_in_outcome`). The kit's
/// labels take both steps off together and keep one redo step, named by the step taken back and
/// holding what both wrote, so they stay in line with the replica's steps: the next undo takes
/// back exactly the step below, and the redos bring them back in order. Before, the labels kept
/// a step the replica had already taken back, and every later undo was named by the step above
/// the one it took back.
#[test]
fn an_undo_that_passes_over_a_step_takes_the_one_below_with_it() {
    let mut doc = SceneDocument::create("Undo.kscene", 320, 180).unwrap();
    let agent = Origin::agent("agent:test");
    let label = |doc: &SceneDocument, key: &str| doc.inspect().unwrap()[key].clone();
    let x = |doc: &SceneDocument| doc.scene().object_named("A").map(|object| object.location.x);
    doc.execute("object.addCube", json!({ "name": "A", "location": [3, 0, 0] })).unwrap();
    let added = label(&doc, "undo");
    doc.execute("object.translate", json!({ "objects": "A", "offset": [1, 0, 0] })).unwrap();
    let moved = label(&doc, "undo");
    let (a, cube) = (named(&doc, "A"), named(&doc, "Cube"));
    doc.execute("object.rename", json!({ "object": cube.clone(), "name": "Gone" })).unwrap();
    doc.execute_as("edit.delete", json!({ "objects": "Gone" }), &agent).unwrap();
    assert_eq!(x(&doc), Some(4.0));

    doc.execute("edit.undo", json!({ "scope": "person" })).unwrap();
    assert_eq!(x(&doc), Some(3.0), "the step below the rename is taken back");
    assert!(doc.scene().object_named("Gone").is_none() && doc.scene().object_named("Cube").is_none(), "the agent's deletion stays");
    assert_eq!(label(&doc, "undo"), added, "the next undo is named by the step below both");
    assert_eq!(label(&doc, "redo"), moved, "the one redo step is named by the step taken back");
    let redo = held(&doc, "edit.redo", json!({ "scope": "person" }));
    assert!(redo.contains(&(a, true, true)), "the redo holds what the step taken back wrote: {redo:?}");

    // The next undo takes back the step below those, exactly.
    doc.execute("edit.undo", json!({ "scope": "person" })).unwrap();
    assert_eq!(x(&doc), None, "the cube added first is taken back");
    assert_eq!(label(&doc, "redo"), added);
    doc.execute("edit.redo", json!({ "scope": "person" })).unwrap();
    assert_eq!(x(&doc), Some(3.0), "the redo brings the cube back");
    doc.execute("edit.redo", json!({ "scope": "person" })).unwrap();
    assert_eq!(x(&doc), Some(4.0), "the redo brings the move back");
    assert_eq!(label(&doc, "redo"), Value::Null, "nothing of the person's is left to redo");
    assert_eq!(label(&doc, "undo"), moved);
}

/// Each object's name, place and parent, in scene order.
type View = Vec<(String, [f32; 3], Option<u32>)>;

/// The replica's state (its hash) and the engine's own view of it.
fn exact(doc: &SceneDocument) -> (String, View) {
    let hash = KitSessionDocument::session(doc).state_hash().unwrap();
    let view = doc.scene().objects.iter().map(|object| (object.name.clone(), object.location.to_array(), object.parent)).collect();
    (hash, view)
}

/// A draft an agent made (a second engine on the scene's snapshot, as the editor opens one),
/// merged into `doc` after the person added an object of their own meanwhile; answers the label of
/// that step.
fn merge_agent_draft(doc: &mut SceneDocument, agent: &Origin) -> Value {
    use kition_kit::session::{__export_changes, __snapshot};
    let base = KitSessionDocument::session(doc).version();
    let mut draft = SceneDocument::open_with_replica("Undo.kscene", &[], &__snapshot(doc).unwrap(), &doc.blobs_bundle()).unwrap();
    draft.execute_as("object.addEmpty", json!({ "name": "Drafted", "location": [0, 5, 0] }), agent).unwrap();
    doc.execute("object.addEmpty", json!({ "name": "Mine", "location": [0, -5, 0] })).unwrap();
    let mine = doc.inspect().unwrap()["undo"].clone();
    assert!(mine.is_string(), "{mine}");
    let changes = __export_changes(&draft, &base.encode()).unwrap();
    doc.add_blobs(&draft.blobs_bundle()).unwrap();
    KitSessionDocument::merge_changes(doc, &changes, agent).unwrap();
    mine
}

/// A merged draft (an agent's) is one undo step of the person, named "Merge Draft"; a restored
/// version is one named "Restore Version". Before, neither pushed a label: the next undo took
/// back the merge (or the restore) but was named by the person's step below it, and every label
/// below was one step off. Undo gives back the exact state before each, redo the exact state
/// after, and the labels stay in line with the replica's steps both ways.
#[test]
fn a_merge_and_a_restore_are_named_steps_of_the_person() {
    use kition_kit::kdm::UndoScope;
    let mut doc = SceneDocument::create("Undo.kscene", 320, 180).unwrap();
    let agent = Origin::agent("agent:test");
    let label = |doc: &SceneDocument, key: &str| doc.inspect().unwrap()[key].clone();
    let names = |doc: &SceneDocument| doc.scene().objects.iter().map(|object| object.name.clone()).collect::<Vec<_>>();
    doc.execute("object.addCube", json!({ "name": "A", "location": [3, 0, 0] })).unwrap();
    let checkpoint = kition_kit::session::__snapshot(&doc).unwrap();
    doc.execute("object.translate", json!({ "objects": "A", "offset": [1, 0, 0] })).unwrap();
    let moved = label(&doc, "undo");

    // The merge, with the person's own edit made meanwhile: both objects are there.
    let added_mine = merge_agent_draft(&mut doc, &agent);
    let merged = exact(&doc);
    assert!(names(&doc).contains(&"Drafted".to_owned()) && names(&doc).contains(&"Mine".to_owned()), "{:?}", names(&doc));
    assert_eq!(label(&doc, "undo"), json!(kition_scene_kit::MERGE_DRAFT_LABEL));
    // The agent's undo does not take it back: the merge is the person's.
    assert!(!KitSessionDocument::session(&doc).can_undo_in(UndoScope::Agent));
    assert!(doc.execute_as("edit.undo", json!({}), &agent).is_err(), "the agent has no step to undo");
    assert_eq!(exact(&doc), merged);

    let undone = doc.execute("edit.undo", json!({ "scope": "person" })).unwrap();
    assert_eq!(undone["undone"], json!(kition_scene_kit::MERGE_DRAFT_LABEL), "the undo is named by the merge");
    let mine = exact(&doc);
    assert!(!names(&doc).contains(&"Drafted".to_owned()) && names(&doc).contains(&"Mine".to_owned()));
    assert_eq!(label(&doc, "undo"), added_mine, "the next undo is named by the person's step below the merge");
    assert_eq!(label(&doc, "redo"), json!(kition_scene_kit::MERGE_DRAFT_LABEL));
    let undone = doc.execute("edit.undo", json!({ "scope": "person" })).unwrap();
    assert_eq!(undone["undone"], added_mine);
    assert!(!names(&doc).contains(&"Mine".to_owned()));
    assert_eq!(label(&doc, "undo"), moved);
    doc.execute("edit.redo", json!({ "scope": "person" })).unwrap();
    assert_eq!(exact(&doc), mine, "the redo gives back the person's step exactly");
    let redone = doc.execute("edit.redo", json!({ "scope": "person" })).unwrap();
    assert_eq!(redone["redone"], json!(kition_scene_kit::MERGE_DRAFT_LABEL));
    assert_eq!(exact(&doc), merged, "the redo gives back the merged state exactly");
    assert_eq!(label(&doc, "redo"), Value::Null);
    doc.execute("edit.undo", json!({ "scope": "person" })).unwrap();
    assert_eq!(exact(&doc), mine, "the undo gives back the state before the merge exactly");
    doc.execute("edit.redo", json!({ "scope": "person" })).unwrap();
    assert_eq!(exact(&doc), merged);

    // The restore of the version saved before the move: one step of the person.
    let before = exact(&doc);
    let person = Origin::person("person:test");
    KitSessionDocument::restore_state(&mut doc, &checkpoint, &person).unwrap();
    let restored = exact(&doc);
    assert_eq!(doc.scene().object_named("A").map(|object| object.location.x), Some(3.0));
    assert!(doc.scene().object_named("Drafted").is_none() && doc.scene().object_named("Mine").is_none());
    assert_eq!(label(&doc, "undo"), json!(kition_scene_kit::RESTORE_VERSION_LABEL));
    let undone = doc.execute("edit.undo", json!({ "scope": "person" })).unwrap();
    assert_eq!(undone["undone"], json!(kition_scene_kit::RESTORE_VERSION_LABEL), "the undo is named by the restore");
    assert_eq!(exact(&doc), before, "the undo gives back the state before the restore exactly");
    assert_eq!(label(&doc, "undo"), json!(kition_scene_kit::MERGE_DRAFT_LABEL), "the next undo is named by the merge below");
    assert_eq!(label(&doc, "redo"), json!(kition_scene_kit::RESTORE_VERSION_LABEL));
    let redone = doc.execute("edit.redo", json!({ "scope": "person" })).unwrap();
    assert_eq!(redone["redone"], json!(kition_scene_kit::RESTORE_VERSION_LABEL));
    assert_eq!(exact(&doc), restored, "the redo gives back the restored state exactly");
}

/// An object-model batch (`model/batch`) is one step of the replica, its commands and raw writes
/// included (an undo group): the labels keep one step for it, named "Batch Edit", by the replica's
/// `undo_depth` and `in_undo_group`. Before, each command of the batch pushed a label of its own
/// and a raw write none, so the labels drifted from the replica's steps: the undo of the batch
/// was named by its last command and the next one by the command before it. Undo gives back the
/// exact state before the batch and names it; the next undo is named by the step before it; redo
/// gives the batch's state back exactly.
#[test]
fn a_model_batch_is_one_named_step_of_the_labels() {
    use kition_kit::kdm::UndoScope;
    use kition_kit::session::model::session_batch;
    let mut doc = SceneDocument::create("Undo.kscene", 320, 180).unwrap();
    let label = |doc: &SceneDocument, key: &str| doc.inspect().unwrap()[key].clone();
    let depth = |doc: &SceneDocument| KitSessionDocument::session(doc).undo_depth(UndoScope::Person);
    let names = |doc: &SceneDocument| doc.scene().objects.iter().map(|object| object.name.clone()).collect::<Vec<_>>();
    doc.execute("object.addCube", json!({ "name": "A", "location": [3, 0, 0] })).unwrap();
    let added = label(&doc, "undo");
    assert!(added.is_string(), "{added}");
    let (before, steps) = (exact(&doc), depth(&doc));

    session_batch(
        &mut doc,
        &json!({ "ops": [
            { "op": "command", "id": "object.addEmpty", "params": { "name": "B", "location": [0, 5, 0] } },
            { "op": "command", "id": "object.translate", "params": { "objects": "A", "offset": [1, 0, 0] } },
            { "op": "set", "target": "root", "field": "world.strength", "value": 2 }
        ] }),
        false,
    )
    .unwrap();
    assert!(!KitSessionDocument::session(&doc).in_undo_group());
    assert_eq!(depth(&doc), steps + 1, "the batch is one step of the replica");
    assert!(names(&doc).contains(&"B".to_owned()));
    assert_eq!(doc.scene().object_named("A").map(|object| object.location.x), Some(4.0));
    let batched = exact(&doc);
    assert_eq!(label(&doc, "undo"), json!(kition_scene_kit::MODEL_BATCH_LABEL));

    let undone = doc.execute("edit.undo", json!({ "scope": "person" })).unwrap();
    assert_eq!(undone["undone"], json!(kition_scene_kit::MODEL_BATCH_LABEL), "the undo is named by the batch");
    assert_eq!(exact(&doc), before, "the undo gives back the state before the batch exactly");
    assert!(!names(&doc).contains(&"B".to_owned()));
    assert_eq!(label(&doc, "undo"), added, "the next undo is named by the step before the batch");
    assert_eq!(label(&doc, "redo"), json!(kition_scene_kit::MODEL_BATCH_LABEL));
    let redone = doc.execute("edit.redo", json!({ "scope": "person" })).unwrap();
    assert_eq!(redone["redone"], json!(kition_scene_kit::MODEL_BATCH_LABEL));
    assert_eq!(exact(&doc), batched, "the redo gives back the batch's state exactly");
    doc.execute("edit.undo", json!({ "scope": "person" })).unwrap();
    let undone = doc.execute("edit.undo", json!({ "scope": "person" })).unwrap();
    assert_eq!(undone["undone"], added, "then the step before the batch");
    assert!(names(&doc).iter().all(|name| name != "A" && name != "B"), "{:?}", names(&doc));
}

/// A draft's preview runs the scene's repair, as accepting the draft does: the agent parents the
/// cube to the light while the person deletes the light (the draft writes only the cube, so the
/// merge applies, rule D8), and the repair clears the cube's dangling parent. The preview's merged
/// state is the state after the merge, its comparison what the merge changed, the engine's
/// `compareChanges` answers that comparison, and the preview changes nothing. Before (kition-sdk
/// 68dd08b) the preview was the unrepaired merge, with the cube's parent naming a deleted object.
/// One Undo gives back the exact state before the merge, one Redo the repaired state.
#[test]
fn a_drafts_preview_is_the_repaired_merge() {
    use kition_kit::session::{__compare_changes, __export_changes, __snapshot};
    let mut doc = SceneDocument::create("Preview.kscene", 320, 180).unwrap();
    let agent = Origin::agent("agent:test");
    let base = KitSessionDocument::session(&doc).version();
    let mut draft = SceneDocument::open_with_replica("Preview.kscene", &[], &__snapshot(&doc).unwrap(), &doc.blobs_bundle()).unwrap();
    draft.execute_as("object.parent", json!({ "objects": ["Cube"], "parent": "Light" }), &agent).unwrap();
    assert!(draft.scene().object_named("Cube").unwrap().parent.is_some(), "the control: the draft parents the cube");
    let changes = __export_changes(&draft, &base.encode()).unwrap();
    doc.execute("edit.delete", json!({ "objects": "Light" })).unwrap();
    doc.add_blobs(&draft.blobs_bundle()).unwrap();

    let session = KitSessionDocument::session(&doc);
    let (version, before) = (session.version(), exact(&doc));
    let unrepaired = session.replica().merged_state(&changes).unwrap();
    let preview = session.merged_state(&changes).unwrap();
    let compared = session.compare_changes(&changes).unwrap().to_json();
    let engine: Value = serde_json::from_str(&__compare_changes(&doc, &changes).unwrap()).unwrap();
    assert_eq!(engine, compared, "compareChanges answers the repaired comparison");
    assert_ne!(preview, unrepaired, "the control: the repair changes this merge");
    assert_eq!((KitSessionDocument::session(&doc).version(), exact(&doc)), (version.clone(), before.clone()), "a preview changes nothing");

    KitSessionDocument::merge_changes(&mut doc, &changes, &agent).unwrap();
    assert!(doc.scene().object_named("Light").is_none());
    assert_eq!(doc.scene().object_named("Cube").unwrap().parent, None, "the repair cleared the dangling parent");
    let session = KitSessionDocument::session(&doc);
    assert_eq!(session.replica().state_json().unwrap(), preview, "the preview is the merge");
    assert_eq!(session.replica().diff_since(&version).unwrap().to_json(), compared, "the comparison is what the merge changed");
    let merged = exact(&doc);
    doc.execute("edit.undo", json!({ "scope": "person" })).unwrap();
    assert_eq!(exact(&doc), before, "one undo: the exact state before the merge");
    doc.execute("edit.redo", json!({ "scope": "person" })).unwrap();
    assert_eq!(exact(&doc), merged, "one redo: the repaired state");
}
