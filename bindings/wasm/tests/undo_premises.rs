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
    let mut fresh = SceneDocument::open("Undo.kscene", &doc.save(None).unwrap()).unwrap();
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
