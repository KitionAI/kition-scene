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
