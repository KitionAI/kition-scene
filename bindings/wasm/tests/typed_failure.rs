//! Why a command did not run is typed (`kition_kit::failure`, task card C8-scene), so a host
//! answers its error code without reading the wording: an unknown command is
//! `command_not_found`; one that cannot run now (nothing selected, the wrong mode)
//! `command_disabled`; wrong parameters, an object the scene lacks and a call context the scene
//! lacks `invalid_params`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use kition_kit::KitDocument;
use kition_kit::failure::{CommandFailure, parse_failure};
use kition_kit::kdm::Origin;
use kition_kit::session::KitSessionDocument;
use kition_scene_kit::SceneDocument;
use serde_json::json;

fn reason(error: &str) -> CommandFailure {
    parse_failure(error).unwrap_or_else(|| panic!("no typed reason in {error:?}")).1
}

#[test]
fn a_commands_failure_carries_its_reason() {
    let mut doc = SceneDocument::create("Scene.kscene", 320, 180).unwrap();
    assert_eq!(reason(&doc.execute("object.nothing", json!({})).unwrap_err()), CommandFailure::NotFound);
    assert_eq!(reason(&doc.execute("object.translate", json!({ "offset": "far" })).unwrap_err()), CommandFailure::InvalidParams);
    assert_eq!(reason(&doc.execute("object.translate", json!({ "ofset": [1, 0, 0] })).unwrap_err()), CommandFailure::InvalidParams);
    assert_eq!(reason(&doc.execute("object.rename", json!({ "object": 999_999, "name": "X" })).unwrap_err()), CommandFailure::InvalidParams);
    assert_eq!(reason(&doc.execute("object.rename", json!({ "object": "Nobody", "name": "X" })).unwrap_err()), CommandFailure::InvalidParams);
    // Edit mode is off: the command cannot run now.
    assert_eq!(reason(&doc.execute("mesh.extrude", json!({})).unwrap_err()), CommandFailure::Disabled);
    let agent = Origin::agent("user:test");
    assert_eq!(reason(&doc.execute_as("object.nothing", json!({}), &agent).unwrap_err()), CommandFailure::NotFound);
    assert_eq!(
        reason(&doc.execute_as("object.rename", json!({ "object": 999_999, "name": "X" }), &agent).unwrap_err()),
        CommandFailure::InvalidParams
    );
}

#[test]
fn a_call_context_the_scene_lacks_is_invalid_params() {
    let mut doc = SceneDocument::create("Scene.kscene", 320, 180).unwrap();
    for context in
        [json!({ "time": 600 }), json!({ "on": [9_999] }), json!({ "on": ["Nobody"] }), json!({ "part": { "slide": 1 } }), json!({ "on": [true] })]
    {
        let context: kition_kit::CallContext = serde_json::from_value(context.clone()).unwrap();
        let error = doc.set_context(&context).unwrap_err();
        assert_eq!(reason(&error), CommandFailure::InvalidParams, "{error}");
    }
}
