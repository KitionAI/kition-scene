//! What each command acts on and the editor state it reads besides its parameters
//! (`KitCommand::targets`, `KitCommand::reads`), for kition-sdk's explicit call context: a host
//! sets the selection and the current frame a command reads from a run step's `on` and `at.time`
//! and puts them back after the call (task card C4-scene).
//!
//! `reads` is what the kit's code reads, not what descriptions say: `reads.json` records it (with
//! `mode`, which kition-sdk's `EditorState` has no kind for: the mode and what it selects, the
//! edited mesh's vertices among them), and `tests/acts.rs` follows each command's code through the
//! kit to the accessors of the selection, the current frame and the mode, and fails when the record
//! says otherwise.
use std::collections::HashMap;
use std::sync::OnceLock;

use kition_kit::EditorState::{self, Selection, Time};
use serde_json::Value;

/// Every combination `reads` can be, in one order (selection, time).
const COMBINATIONS: [&[EditorState]; 3] = [&[Selection], &[Time], &[Selection, Time]];

/// The recorded table: command id to the states it reads (`selection`, `time`, `mode`).
fn table() -> &'static HashMap<String, Vec<String>> {
    static TABLE: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();
    TABLE.get_or_init(|| serde_json::from_str(include_str!("reads.json")).unwrap_or_default())
}

fn recorded(id: &str, state: &str) -> bool {
    table().get(id).is_some_and(|states| states.iter().any(|recorded| recorded == state))
}

/// The editor state command `id` reads besides its parameters that a host can set for a call: the
/// object selection and active object (`on`), and the current frame (`at.time`).
pub fn reads(id: &str) -> &'static [EditorState] {
    let wanted: Vec<EditorState> =
        [(Selection, "selection"), (Time, "time")].into_iter().filter(|(_, name)| recorded(id, name)).map(|(state, _)| state).collect();
    COMBINATIONS.iter().find(|combination| **combination == wanted.as_slice()).copied().unwrap_or(&[])
}

/// Whether command `id` reads the mode or what it selects (the edited mesh, its selected vertices,
/// edges or faces, the armature's bones, a curve's points): no host can set it for a call, so an
/// agent enters the mode inside one run (or names the mesh and its elements in an edit-mode
/// command's `elements`), and the host puts the mode back after the run.
pub fn reads_mode(id: &str) -> bool {
    recorded(id, "mode")
}

/// The parameter that names what command `id` acts on, by the ids `scene_inspect` lists: `objects`,
/// else `object`, only where kition-sdk's `schema::check_targets` takes its schema. The kit's
/// object parameters take a name or an id (`oneOf` integer and string), which it refuses, so an
/// agent's `on` reaches those commands through the selection they read.
pub fn targets(schema: Option<&Value>) -> Option<&'static str> {
    let schema = schema?;
    let properties = schema.get("properties").unwrap_or(schema);
    ["objects", "object"]
        .into_iter()
        .find(|name| properties.get(*name).is_some())
        .filter(|name| kition_kit::schema::check_targets(properties, name).is_ok())
}

/// Commands whose work is moving the editor (the selection, the mode and what it selects, the
/// current frame, the viewport): given what they act on, they keep what they set, where every
/// other agent command given its objects or its frame leaves the editor as it was
/// ([`crate::SceneDocument`]'s `execute_as`). A host still puts an agent's run back afterwards.
pub fn moves_the_editor(id: &str) -> bool {
    ["select.", "view.", "frame.", "mode."].iter().any(|prefix| id.starts_with(prefix))
        || id.contains(".select")
        || matches!(id, "animation.nextKey" | "animation.previousKey" | "edit.undo" | "edit.redo")
}

/// Whether a call names what it acts on (`objects`, `object`, the mesh `elements`, a world point
/// `pick`) or when (`frame`, `time`), the keys a command reads in place of the selection, the
/// active object, the edited mesh or the current frame.
pub fn names_what_or_when(params: &Value) -> bool {
    ["objects", "object", "elements", "pick", "frame", "time"].iter().any(|key| params.get(*key).is_some_and(|value| !value.is_null()))
}
