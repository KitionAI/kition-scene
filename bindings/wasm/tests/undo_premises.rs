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
    doc.execute_as("add.cube", json!({ "name": "A", "location": [3, 0, 0] }), &agent).unwrap();
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
    fresh.execute_as("add.camera", json!({ "name": "Side", "location": [9, 0, 1] }), &agent).unwrap();
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
    doc.execute("add.cube", json!({ "name": "A", "location": [3, 0, 0] })).unwrap();
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

/// The replica's state (its hash) and the engine's own view of it: each object's name, place and
/// parent, in scene order.
fn exact(doc: &SceneDocument) -> (String, Vec<(String, [f32; 3], Option<u32>)>) {
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
    draft.execute_as("add.empty", json!({ "name": "Drafted", "location": [0, 5, 0] }), agent).unwrap();
    doc.execute("add.empty", json!({ "name": "Mine", "location": [0, -5, 0] })).unwrap();
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
    doc.execute("add.cube", json!({ "name": "A", "location": [3, 0, 0] })).unwrap();
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
