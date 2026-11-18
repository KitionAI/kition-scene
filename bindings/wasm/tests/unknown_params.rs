//! A parameter a command does not take is an error that names it, never a call that silently
//! does something else: `modifier.add {objects: …}` used to add the modifier to the active object
//! because the key is `object` (kit requirement AX5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use kition_kit::KitDocument;
use kition_kit::testing::UNKNOWN_PARAM;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn doc() -> SceneDocument {
    SceneDocument::create("A.kscene", 640, 360).unwrap()
}

#[test]
fn a_misspelled_parameter_names_the_one_it_resembles() {
    let mut doc = doc();
    let error = doc.execute("modifier.add", json!({ "objects": ["Cube"], "type": "subdivision" })).unwrap_err();
    assert!(error.starts_with("invalid-params: "), "{error}");
    assert!(error.contains("`objects`") && error.contains("did you mean `object`"), "{error}");
    let inspected = doc.inspect().unwrap();
    let modifiers = inspected["objects"].as_array().unwrap().iter().map(|object| object["modifiers"].clone()).collect::<Vec<Value>>();
    assert!(modifiers.iter().all(|list| list.as_array().is_none_or(Vec::is_empty)), "the refused call changed the scene: {modifiers:?}");
}

#[test]
fn every_command_refuses_a_parameter_it_does_not_take() {
    let commands = doc().commands();
    let named = format!("`{UNKNOWN_PARAM}`");
    let mut checked = 0;
    for command in &commands {
        let Some(Value::Object(properties)) = &command.schema else { continue };
        assert!(!properties.contains_key(UNKNOWN_PARAM), "{} names {UNKNOWN_PARAM}", command.id);
        let mut doc = doc();
        match doc.execute(&command.id, json!({ UNKNOWN_PARAM: 1 })) {
            Ok(answer) => panic!("{} accepted an unknown parameter and answered {answer}", command.id),
            Err(error) => {
                assert!(error.starts_with("invalid-params: "), "{}: the refusal is not invalid-params: {error}", command.id);
                assert!(error.contains(&named), "{}: the error does not name the parameter: {error}", command.id);
            }
        }
        checked += 1;
    }
    assert!(checked > 300, "only {checked} commands with a schema");
    if let Err(problems) = kition_kit::testing::check_unknown_params(doc, &[]) {
        panic!("{} commands:\n{}", problems.len(), problems.join("\n"));
    }
}
