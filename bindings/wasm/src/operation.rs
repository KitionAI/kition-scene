//! Adjust Last Operation: the person's last operation (an added object, an edit-mode operation, a
//! confirmed transform) with the values it ran with, which the editor shows as a panel in the
//! viewport's corner. Changing a value takes the operation back and runs it again with the new
//! value from the selection it started from: still one undo step. The panel's fields are the
//! operation's parameters as its command describes them (a transform's, as its replay does).
//!
//! It is the person's session state: nothing of it is saved, and it lasts only while it is the
//! last thing that happened. Any other change to the scene (another command, an undo, a
//! collaborator's change) or to the selection or the mode ends it.
//!
//! Two session commands: `operation.get` answers it, `operation.set` runs it again.

use std::collections::BTreeSet;

use kition_kit::KitCommand;
use kition_kit::kdm::UndoScope;
use kition_scene_model::Id;
use serde_json::{Map, Value, json};

use crate::modal::Replay;
use crate::params::{bad, schema as s};
use crate::{Mode, SceneDocument, SelectMode};

/// The commands the panel offers to adjust, by id or by the prefix of their ids: the operations a
/// person runs from a menu, a key or a tool. Another command adjusts once it is listed here.
const ADJUSTABLE: &[&str] = &[
    "object.add",
    "mesh.extrude",
    "mesh.inset",
    "mesh.bevel",
    "mesh.loopCut",
    "mesh.subdivide",
    "mesh.merge",
    "mesh.spin",
    "mesh.bisect",
    "mesh.symmetrize",
    "mesh.extrudeIndividual",
    "mesh.translate",
    "mesh.rotate",
    "mesh.scale",
    "mesh.edgeSlide",
    "edit.duplicate",
    "object.translate",
    "object.rotate",
    "object.scale",
    "object.shadeSmoothByAngle",
];

/// Parameters the panel never shows: what the operation acts on, rather than how.
const HIDDEN: &[&str] = &["object", "objects", "elements", "name", "pick", "edge"];

fn adjustable(command: &str) -> bool {
    ADJUSTABLE.iter().any(|id| command == *id || (id.ends_with(".add") && command.starts_with(id)))
}

/// The selection and the mode, as an operation started from them or left them.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Selected {
    selected: Vec<Id>,
    active: Option<Id>,
    mode: Mode,
    vertices: BTreeSet<u32>,
    select_mode: SelectMode,
}

impl Selected {
    pub(crate) fn of(doc: &SceneDocument) -> Self {
        let session = &doc.session;
        Self {
            selected: session.editor.selected.clone(),
            active: session.editor.active,
            mode: session.mode,
            vertices: session.vertices.clone(),
            select_mode: session.select_mode,
        }
    }

    fn put(&self, doc: &mut SceneDocument) {
        doc.session.editor.selected = self.selected.clone();
        doc.session.editor.active = self.active;
        doc.session.mode = self.mode;
        doc.session.vertices = self.vertices.clone();
        doc.session.select_mode = self.select_mode;
        doc.session.faces = None;
        doc.tidy_session();
    }
}

/// How the last operation runs again.
#[derive(Debug, Clone)]
enum Again {
    /// Its command, through the command route.
    Command(String),
    /// A confirmed transform, through the modal.
    Transform(Replay),
}

/// The person's last operation.
#[derive(Debug, Clone)]
pub(crate) struct LastOperation {
    title: String,
    again: Again,
    params: Value,
    /// The parameters' descriptions, the panel's fields.
    schema: Value,
    before: Selected,
    after: Selected,
    /// The document's content counter after it: any other change moves it.
    content: u64,
}

fn person(doc: &SceneDocument) -> bool {
    UndoScope::of(doc.origin.kind) == Some(UndoScope::Person)
}

/// A command the person ran made a step: it becomes the last operation when the panel adjusts it.
pub(crate) fn record_command(doc: &mut SceneDocument, command: &str, params: &Value, before: Selected, title: &str) {
    if !person(doc) || !adjustable(command) {
        doc.last_operation = None;
        return;
    }
    let Some(schema) = crate::commands::schema_of(doc, command).cloned() else {
        doc.last_operation = None;
        return;
    };
    let params = match params {
        // What it acted on stays out (the selection it started from is put back), but for the
        // edge a loop cut crosses.
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .filter(|(key, _)| !HIDDEN.contains(&key.as_str()) || key.as_str() == "edge")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ),
        _ => json!({}),
    };
    doc.last_operation = Some(LastOperation {
        title: title.to_owned(),
        again: Again::Command(command.to_owned()),
        params,
        schema,
        before,
        after: Selected::of(doc),
        content: doc.content,
    });
}

/// A transform the person confirmed made a step: it becomes the last operation.
pub(crate) fn record_transform(doc: &mut SceneDocument, replay: Replay, params: Value, before: Selected) {
    if !person(doc) {
        doc.last_operation = None;
        return;
    }
    let title = match (replay.duplicate, replay.kind) {
        (true, _) => "Duplicate",
        (_, crate::modal::Kind::Rotate) => "Rotate",
        (_, crate::modal::Kind::Scale) => "Scale",
        _ => "Move",
    };
    doc.last_operation = Some(LastOperation {
        title: title.into(),
        schema: crate::modal::replay_schema(replay.kind),
        again: Again::Transform(replay),
        params,
        before,
        after: Selected::of(doc),
        content: doc.content,
    });
}

impl SceneDocument {
    /// The last operation, while it is still the last thing that happened.
    pub(crate) fn last_operation(&self) -> Option<&LastOperation> {
        self.last_operation.as_ref().filter(|last| last.content == self.content && last.after == Selected::of(self) && self.modal_started().is_none())
    }

    /// The last operation as the editor's panel shows it (`null` without one).
    pub(crate) fn operation_json(&self) -> Value {
        let Some(last) = self.last_operation() else { return Value::Null };
        json!({
            "title": last.title,
            "command": match &last.again { Again::Command(command) => Value::from(command.as_str()), Again::Transform(_) => Value::Null },
            "params": last.params,
            "fields": fields(&last.schema, &last.params),
        })
    }
}

/// A field's label from its parameter name: `majorSegments` is Major Segments.
fn label(key: &str) -> String {
    let mut text = String::new();
    for (index, character) in key.chars().enumerate() {
        if index == 0 {
            text.extend(character.to_uppercase());
        } else if character.is_ascii_uppercase() || (character.is_ascii_digit() && !text.ends_with(|c: char| c.is_ascii_digit())) {
            text.push(' ');
            text.push(character);
        } else {
            text.push(character);
        }
    }
    text
}

/// The default a description states at its end, as `(32)`, `(0.1)` or `(false)`.
fn stated_default(description: &str) -> Option<&str> {
    let trimmed = description.trim_end().strip_suffix(')')?;
    let open = trimmed.rfind('(')?;
    trimmed.get(open + 1..)
}

/// The panel's fields: each parameter that says how the operation runs (a number, a whole
/// number, a switch, a choice, a vector), with its value (the one it ran with, else the default
/// its description states).
fn fields(schema: &Value, params: &Value) -> Vec<Value> {
    let Some(properties) = schema.as_object() else { return Vec::new() };
    let mut out: Vec<(u8, Value)> = Vec::new();
    for (key, node) in properties {
        if HIDDEN.contains(&key.as_str()) {
            continue;
        }
        let description = node.get("description").and_then(Value::as_str).unwrap_or_default();
        let given = params.get(key).filter(|value| !value.is_null());
        let stated = stated_default(description);
        let kind = node.get("type").and_then(Value::as_str);
        let items = node.get("items").and_then(|items| items.get("type")).and_then(Value::as_str);
        let (field, value) = match (kind, node.get("enum").and_then(Value::as_array)) {
            (Some("string"), Some(choices)) => {
                let value = given.cloned().or_else(|| stated.filter(|text| choices.iter().any(|choice| choice == text)).map(Value::from));
                ("choice", value.unwrap_or(Value::Null))
            }
            (Some("number"), _) => {
                ("number", given.cloned().or_else(|| stated.and_then(|text| text.parse::<f64>().ok()).map(Value::from)).unwrap_or(Value::Null))
            }
            (Some("integer"), _) => {
                ("integer", given.cloned().or_else(|| stated.and_then(|text| text.parse::<i64>().ok()).map(Value::from)).unwrap_or(Value::Null))
            }
            (Some("boolean"), _) => (
                "boolean",
                given.cloned().or_else(|| stated.and_then(|text| text.parse::<bool>().ok()).map(Value::from)).unwrap_or(Value::Bool(false)),
            ),
            (Some("array"), _) if items == Some("number") && description.ends_with("[x, y, z]") => {
                let fallback = if key.contains("scale") || key == "factor" { json!([1.0, 1.0, 1.0]) } else { json!([0.0, 0.0, 0.0]) };
                ("vector", given.cloned().unwrap_or(fallback))
            }
            _ => continue,
        };
        let mut entry = Map::new();
        entry.insert("key".into(), Value::from(key.as_str()));
        entry.insert("label".into(), Value::from(label(key)));
        entry.insert("type".into(), Value::from(field));
        entry.insert("value".into(), value);
        entry.insert("description".into(), Value::from(description));
        if let Some(choices) = node.get("enum") {
            entry.insert("choices".into(), choices.clone());
        }
        // Placement last, as Blender lists it.
        let order = match key.as_str() {
            "location" => 1,
            "rotation" => 2,
            "scale" if field == "vector" && properties.contains_key("location") => 3,
            _ => 0,
        };
        out.push((order, Value::Object(entry)));
    }
    out.sort_by_key(|(order, _)| *order);
    out.into_iter().map(|(_, field)| field).collect()
}

/// `params` over `base`: a given value replaces the one it ran with, `null` takes it out.
fn merged(base: &Value, params: &Value) -> Value {
    let mut out = base.as_object().cloned().unwrap_or_default();
    if let Some(given) = params.as_object() {
        for (key, value) in given {
            if value.is_null() {
                out.remove(key);
            } else {
                out.insert(key.clone(), value.clone());
            }
        }
    }
    Value::Object(out)
}

/// `operation.set`: takes the last operation back and runs it again with `params` over the values
/// it ran with, from the selection it started from. One undo step stays one. When it cannot run
/// with them, the operation is put back as it was.
fn set(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let given = params.get("params").cloned().unwrap_or_else(|| json!({}));
    if !given.is_object() {
        return Err(bad("`params` must be an object of the operation's parameters"));
    }
    let Some(last) = doc.last_operation().cloned() else {
        return Err("not-allowed: there is no last operation to adjust".into());
    };
    let values = merged(&last.params, &given);
    for (key, _) in values.as_object().into_iter().flatten() {
        let known = last.schema.get(key).is_some() || key == "edge";
        if !known {
            return Err(bad(format!("{} has no parameter `{key}`", last.title)));
        }
    }
    crate::commands::execute(doc, "edit.undo", &json!({ "scope": "person" }))?;
    last.before.put(doc);
    let ran = match &last.again {
        Again::Command(command) => crate::commands::execute(doc, command, &values).map(|_| ()),
        Again::Transform(replay) => doc.modal_replay(replay, &values),
    };
    match ran {
        Ok(()) => {
            doc.touched();
            Ok(json!({ "operation": doc.operation_json() }))
        }
        Err(error) => {
            // Back as it was: the operation's step again, and its panel.
            let _ = crate::commands::execute(doc, "edit.redo", &json!({ "scope": "person" }));
            last.after.put(doc);
            doc.last_operation = Some(LastOperation { content: doc.content, ..last });
            doc.touched();
            Err(error)
        }
    }
}

fn command(id: &str, title: &str, schema: Value) -> KitCommand {
    KitCommand { id: id.into(), title: title.into(), schema: Some(schema), mutates: false, enabled: true, ..Default::default() }
}

/// Adjust Last Operation's commands: session commands for the editor (`operation.set` makes the
/// operation's own undo step in place of the one it takes back).
pub fn catalogue(_doc: &SceneDocument) -> Vec<KitCommand> {
    vec![
        command("operation.get", "Last Operation", s::of(vec![])),
        command(
            "operation.set",
            "Adjust Last Operation",
            s::of(vec![(
                "params",
                s::required(
                    json!({ "type": "object", "additionalProperties": true, "description": "The parameters to change, by name; null takes one back to its default" }),
                ),
            )]),
        ),
    ]
}

/// Runs one of these commands; `None` for any other.
pub fn execute(doc: &mut SceneDocument, command: &str, params: &Value) -> Option<Result<Value, String>> {
    match command {
        "operation.get" => Some(Ok(json!({ "operation": doc.operation_json() }))),
        "operation.set" => Some(set(doc, params)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_stated_defaults_read_from_the_parameters() {
        assert_eq!(label("majorSegments"), "Major Segments");
        assert_eq!(label("radius1"), "Radius 1");
        assert_eq!(stated_default("Vertices around (32)"), Some("32"));
        assert_eq!(stated_default("How far in, in metres (0.1)"), Some("0.1"));
        assert_eq!(stated_default("Where to put it"), None);
        assert!(adjustable("object.addCylinder") && adjustable("mesh.inset") && !adjustable("object.transform"));
    }
}
