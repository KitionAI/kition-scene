//! Rigging: armature objects, their edit mode (adding, extruding, moving, and deleting bones)
//! and pose mode (posing and keying bones), parenting meshes to an armature with automatic or
//! empty vertex groups, vertex groups, and shape keys. Rotations are in degrees.
//!
//! An armature's edit and pose modes are kept beside the mesh modes ([`RigSession`]): while one
//! is on, the document is in object mode for everything else, but the object-level commands that
//! would act on the armature (delete, duplicate, select all, keying) act on its bones instead.

use std::collections::BTreeSet;

use glam::{Mat3, Vec3};
use kition_kit::KitCommand;
use kition_kit::tools::{PointerAction, ToolOverlay, ToolPointer, ToolResponse};
use kition_scene_model::armature::{self, Armature, Bone, BoneShape, armature_mut, armature_of};
use kition_scene_model::scene::{euler_matrix, matrix_euler};
use kition_scene_model::vertex_data::{self, ShapeKey};
use kition_scene_model::{Id, Modifier, ModifierKind, ObjectData, Property, Scene};
use kition_scene_raster::BoneOverlay;
use serde_json::{Value, json};

use crate::params::{self, bad, schema as s};
use crate::{Mode, SceneDocument, Step};

/// An armature in edit or pose mode.
#[derive(Debug, Clone, PartialEq)]
pub struct RigSession {
    pub object: Id,
    /// Pose mode; otherwise edit mode.
    pub pose: bool,
    pub selected: BTreeSet<String>,
    pub active: Option<String>,
    drag: Option<RigDrag>,
}

#[derive(Debug, Clone)]
struct RigDrag {
    start: (f64, f64),
    pivot: Vec3,
    pivot_screen: (f32, f32),
    before: Box<Step>,
    moved: bool,
    marquee: bool,
}

impl PartialEq for RigDrag {
    fn eq(&self, other: &Self) -> bool {
        self.start == other.start && self.moved == other.moved
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Needs {
    /// An armature active, or one in edit or pose mode.
    Armature,
    RigEdit,
    /// Edit mode with a bone selected.
    RigEditSelection,
    Pose,
    /// Pose mode with a bone selected.
    PoseSelection,
    /// Object mode with meshes and an armature selected.
    Parenting,
    /// A mesh to act on: in mesh edit mode, or active in object mode.
    Mesh,
    /// Mesh edit mode with vertices selected.
    MeshEditSelection,
    /// Object mode.
    ObjectMode,
    /// The scene has keys.
    Keys,
}

struct Spec {
    id: &'static str,
    title: &'static str,
    menu: &'static [&'static str],
    shortcut: Option<&'static str>,
    mutates: bool,
    needs: Needs,
    schema: fn() -> Value,
}

const fn spec(
    id: &'static str,
    title: &'static str,
    menu: &'static [&'static str],
    shortcut: Option<&'static str>,
    mutates: bool,
    needs: Needs,
    schema: fn() -> Value,
) -> Spec {
    Spec { id, title, menu, shortcut, mutates, needs, schema }
}

fn bones_param(description: &str) -> Value {
    json!({ "oneOf": [{ "type": "string", "description": "a bone's name" }, { "type": "array", "items": { "type": "string" }, "description": "bones' names" }], "description": description })
}

fn mesh_param() -> (&'static str, Value) {
    ("object", s::object("The mesh object; the edited or active one when left out"))
}

const ARMATURE: &[&str] = &["Armature"];
const POSE: &[&str] = &["Pose"];
const GROUPS: &[&str] = &["Object", "Vertex Groups"];
const SHAPES: &[&str] = &["Object", "Shape Keys"];

static SPECS: &[Spec] = &[
    spec("object.addArmature", "Armature", &["Add"], None, true, Needs::ObjectMode, || {
        s::of(vec![("location", s::vec3("Where to put it, in metres")), ("name", s::string("The object's name"))])
    }),
    spec("mode.armature", "Toggle Edit Mode", &["Armature"], Some("Tab"), false, Needs::Armature, || {
        s::of(vec![("object", s::object("The armature; the active one when left out"))])
    }),
    spec("mode.pose", "Pose Mode", &["Armature"], None, false, Needs::Armature, || {
        s::of(vec![
            ("on", s::boolean("Pose mode on or off; toggles when left out")),
            ("object", s::object("The armature; the active one when left out")),
        ])
    }),
    spec("bone.add", "Add Bone", ARMATURE, None, true, Needs::RigEdit, || {
        s::of(vec![
            ("name", s::string("The bone's name (Bone)")),
            ("head", s::vec3("Where it starts, in the armature's space (the origin)")),
            ("tail", s::vec3("Where it ends (a metre above the head)")),
            ("parent", s::string("The parent bone")),
            ("connected", s::boolean("The head sits on the parent's tail")),
        ])
    }),
    spec("bone.extrude", "Extrude Bone", ARMATURE, Some("E"), true, Needs::RigEditSelection, || {
        s::of(vec![
            ("tail", s::vec3("Where the new bone ends, in the armature's space")),
            ("length", s::number("Its length along the bone it grows from (the same length)")),
        ])
    }),
    spec("bone.set", "Bone Settings", &[], None, true, Needs::Armature, || {
        s::of(vec![
            ("object", s::object("The armature; the active one when left out")),
            ("bone", s::string("The bone; the active one when left out")),
            ("name", s::string("A new name")),
            ("head", s::vec3("Where it starts, armature space")),
            ("tail", s::vec3("Where it ends, armature space")),
            ("roll", s::number("Turn about its own axis, degrees")),
            ("parent", s::string("The parent bone; an empty string clears it")),
            ("connected", s::boolean("The head sits on the parent's tail")),
            ("deform", s::boolean("The bone deforms meshes")),
            ("headRadius", s::number("Envelope: the radius at the head, metres (0.1)")),
            ("tailRadius", s::number("Envelope: the radius at the tail, metres (0.05)")),
            ("envelopeDistance", s::number("Envelope: how far past the radii it fades out, metres (0.25)")),
            ("envelopeWeight", s::number("Envelope: its strength where full (1)")),
            ("envelopeMultiply", s::boolean("Multiply the bone's vertex group weights by its envelope")),
            ("shape", s::object("Custom shape: the object whose mesh draws the bone; null for the octahedron")),
            ("shapeTranslation", s::vec3("Custom shape: its offset in the bone's axes, metres")),
            ("shapeRotation", s::vec3("Custom shape: its turn in the bone's axes, degrees")),
            ("shapeScale", s::vec3("Custom shape: its scale")),
            ("shapeBoneSize", s::boolean("Custom shape: scaled by the bone's length too (true)")),
            ("shapeWire", s::boolean("Custom shape: drawn as its edges only")),
        ])
    }),
    spec("bone.select", "Select Bones", &[], None, false, Needs::Armature, || {
        s::of(vec![
            ("bones", s::required(bones_param("The bones to select"))),
            ("extend", s::boolean("Add to the selection")),
            ("deselect", s::boolean("Take them out of the selection")),
        ])
    }),
    spec("pose.set", "Pose Bones", &[], None, true, Needs::Pose, || {
        s::of(vec![
            ("bones", bones_param("The bones; the selected ones when left out")),
            ("location", s::vec3("Location in the bone's rest space, metres")),
            ("rotation", s::vec3("Rotation in degrees, XYZ Euler, in the bone's rest space")),
            ("scale", s::vec3("Scale")),
        ])
    }),
    spec("pose.clear", "Clear Pose", POSE, None, true, Needs::PoseSelection, || {
        s::of(vec![("bones", bones_param("The bones; the selected ones when left out")), ("all", s::boolean("Every bone"))])
    }),
    spec("pose.insertKey", "Insert Keyframe", POSE, Some("I"), true, Needs::PoseSelection, || {
        s::of(vec![
            ("bones", bones_param("The bones; the selected ones when left out")),
            (
                "properties",
                json!({ "type": "array", "items": { "type": "string", "enum": ["location", "rotation", "scale"] }, "description": "What to key (all three when left out)" }),
            ),
            ("frame", s::number("The frame; the current frame when left out")),
            ("keyingSet", s::string("Key what this keying set names instead (the active one is used when no properties are given)")),
        ])
    }),
    spec("pose.deleteKey", "Delete Keyframe", POSE, Some("Alt+I"), true, Needs::PoseSelection, || {
        s::of(vec![
            ("bones", bones_param("The bones; the selected ones when left out")),
            (
                "properties",
                json!({ "type": "array", "items": { "type": "string", "enum": ["location", "rotation", "scale"] }, "description": "Whose keys to delete (all three when left out)" }),
            ),
            ("frame", s::number("The frame; the current frame when left out")),
        ])
    }),
    spec("object.parentArmature", "Parent to Armature", &["Object", "Parent"], None, true, Needs::Parenting, || {
        s::of(vec![
            ("objects", s::objects("The meshes; the selected ones without the active armature when left out")),
            ("armature", s::object("The armature; the active object when left out")),
            ("weights", s::choice("How the vertex groups are made", &["automatic", "empty", "none"])),
        ])
    }),
    spec("vertexGroup.add", "Add Vertex Group", GROUPS, None, true, Needs::Mesh, || {
        s::of(vec![mesh_param(), ("name", s::string("The group's name (Group)"))])
    }),
    spec("vertexGroup.remove", "Remove Vertex Group", GROUPS, None, true, Needs::Mesh, || {
        s::of(vec![mesh_param(), ("name", s::required(s::string("The group")))])
    }),
    spec("vertexGroup.assign", "Assign to Group", GROUPS, None, true, Needs::Mesh, || {
        s::of(vec![
            mesh_param(),
            ("name", s::required(s::string("The group (made when missing)"))),
            ("weight", s::number("0 to 1 (1)")),
            ("vertices", s::indices("The vertices; the edit-mode selection when left out")),
        ])
    }),
    spec("vertexGroup.unassign", "Remove from Group", GROUPS, None, true, Needs::Mesh, || {
        s::of(vec![
            mesh_param(),
            ("name", s::required(s::string("The group"))),
            ("vertices", s::indices("The vertices; the edit-mode selection when left out")),
        ])
    }),
    spec("vertexGroup.select", "Select Group", GROUPS, None, false, Needs::MeshEditSelection, || {
        s::of(vec![("name", s::required(s::string("The group")))])
    }),
    spec("shapeKey.add", "Add Shape Key", SHAPES, None, true, Needs::Mesh, || {
        s::of(vec![
            mesh_param(),
            ("name", s::string("The key's name (Key 1, Key 2, ...)")),
            ("from", s::object("A mesh with the same vertex count whose shape the key takes")),
            (
                "offsets",
                json!({ "type": "array", "items": { "type": "json" }, "description": "An offset [x, y, z] per vertex, in the object's own axes" }),
            ),
        ])
    }),
    spec("shapeKey.remove", "Remove Shape Key", SHAPES, None, true, Needs::Mesh, || {
        s::of(vec![mesh_param(), ("name", s::required(s::string("The key")))])
    }),
    spec("shapeKey.set", "Shape Key Settings", &[], None, true, Needs::Mesh, || {
        s::of(vec![
            mesh_param(),
            ("name", s::required(s::string("The key"))),
            ("value", s::number("How much of the shape, usually 0 to 1")),
            ("rename", s::string("A new name")),
        ])
    }),
    spec("shapeKey.offset", "Move in Shape Key", SHAPES, None, true, Needs::Mesh, || {
        s::of(vec![
            mesh_param(),
            ("name", s::required(s::string("The key"))),
            ("offset", s::required(s::vec3("How far the vertices move in this shape, object axes"))),
            ("vertices", s::indices("The vertices; the edit-mode selection when left out")),
        ])
    }),
    spec("shapeKey.insertKey", "Key Shape Value", SHAPES, None, true, Needs::Mesh, || {
        s::of(vec![mesh_param(), ("name", s::required(s::string("The key"))), ("frame", s::number("The frame; the current frame when left out"))])
    }),
    spec("graph.moveKey", "Move Keyframe", &[], None, true, Needs::Keys, || {
        s::of(graph_key(vec![("toFrame", s::number("The key's new frame")), ("value", s::number("The key's new value (degrees for rotations)"))]))
    }),
    spec("graph.deleteKey", "Delete Keyframe", &[], None, true, Needs::Keys, || s::of(graph_key(vec![]))),
    spec("graph.setInterpolation", "Keyframe Interpolation", &[], None, true, Needs::Keys, || {
        s::of(graph_key(vec![("interpolation", s::required(s::choice("How the curve runs to the next key", &["bezier", "linear", "constant"])))]))
    }),
];

/// The fields that name one key of one channel (as inspect's `animation.channels` lists them).
fn graph_key(mut extra: Vec<(&'static str, Value)>) -> Vec<(&'static str, Value)> {
    let mut fields = vec![
        ("target", s::required(s::integer("The channel's object (or material) id"))),
        ("property", s::required(s::string("The channel's property (location, boneRotation, shapeKey, ...)"))),
        ("index", s::integer("The component, from 0 (0)")),
        ("path", s::string("The bone or shape key, for their properties")),
        ("frame", s::required(s::number("The key's frame"))),
    ];
    fields.append(&mut extra);
    fields
}

/// Object-level commands that act on the bones while an armature is in edit or pose mode.
const TAKEN_OVER: &[&str] = &["edit.delete", "edit.duplicate", "edit.selectAll", "edit.deselectAll"];

fn spec_of(id: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|spec| spec.id == id)
}

fn active_armature(doc: &SceneDocument) -> Option<Id> {
    doc.session.editor.active.filter(|id| armature_of(&doc.scene, *id).is_some())
}

fn edited_mesh(doc: &SceneDocument) -> Option<Id> {
    match doc.session.mode {
        Mode::Edit(id) => Some(id),
        Mode::Object => doc.session.editor.active.filter(|id| doc.scene.object(*id).is_ok_and(|object| object.mesh().is_some())),
    }
}

fn enabled(doc: &SceneDocument, needs: Needs) -> bool {
    let rig = doc.session.rig.as_ref();
    let object_mode = doc.session.mode == Mode::Object && rig.is_none();
    match needs {
        Needs::Armature => rig.is_some() || (doc.session.mode == Mode::Object && active_armature(doc).is_some()),
        Needs::RigEdit => rig.is_some_and(|rig| !rig.pose),
        Needs::RigEditSelection => rig.is_some_and(|rig| !rig.pose && !rig.selected.is_empty()),
        Needs::Pose => rig.is_some_and(|rig| rig.pose),
        Needs::PoseSelection => rig.is_some_and(|rig| rig.pose && !rig.selected.is_empty()),
        Needs::Parenting => {
            object_mode
                && active_armature(doc).is_some()
                && doc.session.editor.selected.iter().any(|id| doc.scene.object(*id).is_ok_and(|object| object.mesh().is_some()))
        }
        Needs::Mesh => rig.is_none() && edited_mesh(doc).is_some(),
        Needs::MeshEditSelection => matches!(doc.session.mode, Mode::Edit(_)) && !doc.session.vertices.is_empty(),
        Needs::ObjectMode => object_mode,
        Needs::Keys => !doc.scene.animation.channels.is_empty(),
    }
}

/// The rigging commands, with whether each can run now.
pub fn catalogue(doc: &SceneDocument) -> Vec<KitCommand> {
    SPECS
        .iter()
        .map(|spec| KitCommand {
            id: spec.id.into(),
            title: spec.title.into(),
            menu: (!spec.menu.is_empty()).then(|| spec.menu.iter().map(|part| (*part).to_string()).collect()),
            shortcut: spec.shortcut.map(Into::into),
            params: None,
            schema: Some((spec.schema)()),
            mutates: spec.mutates,
            enabled: enabled(doc, spec.needs),
            ..Default::default()
        })
        .collect()
}

/// The rigging commands that cannot run now.
pub fn disabled(doc: &SceneDocument) -> Vec<String> {
    SPECS.iter().filter(|spec| !enabled(doc, spec.needs)).map(|spec| spec.id.to_string()).collect()
}

/// Runs a rigging command (or an object-level one an armature mode takes over), or answers
/// `None` when it is neither.
pub fn execute(doc: &mut SceneDocument, command: &str, params: &Value) -> Option<Result<Value, String>> {
    if !params.is_object() && !params.is_null() {
        return spec_of(command).map(|_| Err(bad("the parameters must be an object")));
    }
    if doc.session.rig.is_some() && TAKEN_OVER.contains(&command) {
        return Some(match command {
            "edit.selectAll" => select_all(doc, params::string(params, "action").ok().flatten().unwrap_or("select")),
            "edit.deselectAll" => select_all(doc, "deselect"),
            "edit.delete" => change(doc, "Delete Bones", delete_bones),
            _ => change(doc, "Duplicate Bones", duplicate_bones),
        });
    }
    // Objects are added in object mode, as in the reference application.
    if doc.session.rig.is_some() && command.starts_with("object.add") {
        return Some(Err("not-allowed: leave the armature's edit or pose mode to add objects".into()));
    }
    // Leaving the armature's modes: any mesh mode change or another object's edit mode.
    if command == "mode.set" && doc.session.rig.is_some() {
        doc.session.rig = None;
        doc.touched();
    }
    let spec = spec_of(command)?;
    // A named armature counts as the active one.
    if matches!(spec.needs, Needs::Armature)
        && let Some(object) = params.get("object").filter(|value| !value.is_null())
        && let Ok(id) = params::resolve(&doc.scene, object)
        && armature_of(&doc.scene, id).is_some()
        && doc.session.rig.is_none()
    {
        doc.session.editor.active = Some(id);
        if !doc.session.editor.selected.contains(&id) {
            doc.session.editor.selected.push(id);
        }
    }
    let named_mesh = matches!(spec.needs, Needs::Mesh) && params.get("object").is_some_and(|value| !value.is_null());
    if !(enabled(doc, spec.needs) || named_mesh && doc.session.rig.is_none() || names_targets(doc, spec.needs, params)) {
        return Some(Err(format!("not-allowed: {} cannot run now", spec.title)));
    }
    if !spec.mutates {
        let result = run_view(doc, command, params);
        if result.is_ok() {
            doc.touched();
        }
        return Some(result);
    }
    Some(change(doc, spec.title, |doc| run_change(doc, command, params)))
}

/// Whether the call names what the gate would look for in the selection: pose commands naming
/// their `bones` (or `all`) in pose mode, and Parent to Armature naming its `armature` and its
/// `objects` (meshes) in object mode.
fn names_targets(doc: &SceneDocument, needs: Needs, params: &Value) -> bool {
    let given = |key: &str| params.get(key).is_some_and(|value| !value.is_null());
    match needs {
        Needs::PoseSelection => {
            doc.session.rig.as_ref().is_some_and(|rig| rig.pose) && (given("bones") || params::boolean(params, "all").ok().flatten().unwrap_or(false))
        }
        Needs::Parenting => {
            let armature = match params.get("armature").filter(|value| !value.is_null()) {
                Some(value) => params::resolve(&doc.scene, value).ok().filter(|id| armature_of(&doc.scene, *id).is_some()),
                None => active_armature(doc),
            };
            let meshes = match params::objects(&doc.scene, params, "objects").ok().flatten() {
                Some(ids) => ids.into_iter().any(|id| doc.scene.object(id).is_ok_and(|object| object.mesh().is_some())),
                None => doc.session.editor.selected.iter().any(|id| doc.scene.object(*id).is_ok_and(|object| object.mesh().is_some())),
            };
            enabled(doc, Needs::ObjectMode) && armature.is_some() && meshes
        }
        _ => false,
    }
}

/// Runs a change as one undo step, rolling everything back when it fails.
fn change(doc: &mut SceneDocument, label: &str, work: impl FnOnce(&mut SceneDocument) -> Result<Value, String>) -> Result<Value, String> {
    let before = doc.step(label);
    let session = doc.session.clone();
    match work(doc) {
        Ok(value) => {
            doc.tidy_session();
            if let Err(error) = doc.scene.validate() {
                doc.restore(before);
                doc.session = session;
                return Err(format!("failed: the change would break the scene ({error})"));
            }
            doc.committed(before)?;
            Ok(value)
        }
        Err(error) => {
            doc.restore(before);
            doc.session = session;
            Err(error)
        }
    }
}

/// Drops bones that are gone from the armature mode's selection, and the mode when its armature
/// is gone.
pub fn tidy(doc: &mut SceneDocument) {
    let Some(rig) = doc.session.rig.as_mut() else { return };
    let Some(armature) = armature_of(&doc.scene, rig.object) else {
        doc.session.rig = None;
        return;
    };
    rig.selected.retain(|name| armature.bone(name).is_some());
    if rig.active.as_ref().is_some_and(|name| armature.bone(name).is_none()) {
        rig.active = rig.selected.iter().next_back().cloned();
    }
}

/// What the viewport shows of the armature being edited or posed.
pub fn bone_overlay(doc: &SceneDocument) -> Option<BoneOverlay> {
    let rig = doc.session.rig.as_ref()?;
    Some(BoneOverlay { object: rig.object, selected: rig.selected.clone(), active: rig.active.clone(), pose: rig.pose })
}

fn rig_of(doc: &SceneDocument) -> Result<&RigSession, String> {
    doc.session.rig.as_ref().ok_or_else(|| "not-allowed: no armature is in edit or pose mode".to_string())
}

fn names(value: &Value) -> Result<Vec<String>, String> {
    match value {
        Value::String(name) => Ok(vec![name.clone()]),
        Value::Array(items) => items.iter().map(|item| item.as_str().map(str::to_string).ok_or_else(|| bad("bones are named by strings"))).collect(),
        _ => Err(bad("bones are named by a string or a list of strings")),
    }
}

/// The bones a command acts on: `bones` when given, else the selection.
fn chosen_bones(doc: &SceneDocument, params: &Value) -> Result<Vec<String>, String> {
    let rig = rig_of(doc)?;
    let armature = armature_of(&doc.scene, rig.object).ok_or("not-found: the armature")?;
    let list = match params.get("bones").filter(|value| !value.is_null()) {
        Some(value) => names(value)?,
        None => rig.selected.iter().cloned().collect(),
    };
    for name in &list {
        if armature.bone(name).is_none() {
            return Err(format!("not-found: no bone named {name}"));
        }
    }
    if list.is_empty() {
        return Err("not-allowed: select a bone first, or name `bones`".into());
    }
    Ok(list)
}

fn armature_target(doc: &SceneDocument, params: &Value) -> Result<Id, String> {
    if let Some(rig) = &doc.session.rig {
        return Ok(rig.object);
    }
    let id = match params.get("object").filter(|value| !value.is_null()) {
        Some(value) => params::resolve(&doc.scene, value)?,
        None => active_armature(doc).ok_or("not-allowed: there is no active armature")?,
    };
    armature_of(&doc.scene, id).ok_or_else(|| bad("that object is not an armature"))?;
    Ok(id)
}

/// The armature a command changes: `object` when given (an explicit armature wins over the
/// armature in edit or pose mode), else the one in edit or pose mode, else the active one.
fn named_armature(doc: &SceneDocument, params: &Value) -> Result<Id, String> {
    match params.get("object").filter(|value| !value.is_null()) {
        Some(value) => {
            let id = params::resolve(&doc.scene, value)?;
            armature_of(&doc.scene, id).ok_or_else(|| bad("that object is not an armature"))?;
            Ok(id)
        }
        None => armature_target(doc, params),
    }
}

fn select_all(doc: &mut SceneDocument, action: &str) -> Result<Value, String> {
    let rig = rig_of(doc)?.clone();
    let armature = armature_of(&doc.scene, rig.object).ok_or("not-found: the armature")?;
    let every: BTreeSet<String> = armature.bones.iter().map(|bone| bone.name.clone()).collect();
    let selected = match action {
        "select" => every,
        "deselect" => BTreeSet::new(),
        "invert" => every.difference(&rig.selected).cloned().collect(),
        other => return Err(bad(format!("unknown action {other}"))),
    };
    if let Some(session) = doc.session.rig.as_mut() {
        session.active = session.active.clone().filter(|name| selected.contains(name)).or_else(|| selected.iter().next_back().cloned());
        session.selected = selected;
    }
    doc.touched();
    Ok(json!({ "bones": doc.session.rig.as_ref().map(|rig| rig.selected.len()).unwrap_or(0) }))
}

fn set_mode(doc: &mut SceneDocument, object: Id, pose: Option<bool>) {
    let current = doc.session.rig.as_ref().filter(|rig| rig.object == object).map(|rig| rig.pose);
    doc.session.mode = Mode::Object;
    doc.session.vertices.clear();
    doc.session.rig = match pose {
        // Toggling edit mode: from object mode into edit mode; from edit or pose back out.
        None => match current {
            Some(_) => None,
            None => Some(RigSession { object, pose: false, selected: BTreeSet::new(), active: None, drag: None }),
        },
        Some(true) => Some(RigSession { object, pose: true, selected: BTreeSet::new(), active: None, drag: None }),
        Some(false) => None,
    };
    doc.session.editor.active = Some(object);
    if !doc.session.editor.selected.contains(&object) {
        doc.session.editor.selected.push(object);
    }
}

fn run_view(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    match command {
        "mode.armature" => {
            let object = armature_target(doc, params)?;
            let leaving = doc.session.rig.as_ref().is_some_and(|rig| !rig.pose);
            set_mode(doc, object, if leaving { Some(false) } else { None });
            if !leaving && doc.session.rig.is_none() {
                // Tab from pose mode goes to edit mode.
                set_mode(doc, object, None);
            }
        }
        "mode.pose" => {
            let object = armature_target(doc, params)?;
            let on = params::boolean(params, "on")?.unwrap_or_else(|| !doc.session.rig.as_ref().is_some_and(|rig| rig.pose));
            set_mode(doc, object, Some(on));
        }
        "bone.select" => {
            let wanted = names(params.get("bones").ok_or_else(|| bad("`bones` is required"))?)?;
            let rig = rig_of(doc)?.clone();
            let armature = armature_of(&doc.scene, rig.object).ok_or("not-found: the armature")?;
            if let Some(missing) = wanted.iter().find(|name| armature.bone(name).is_none()) {
                return Err(format!("not-found: no bone named {missing}"));
            }
            let session = doc.session.rig.as_mut().ok_or("not-allowed: no armature mode")?;
            if params::boolean(params, "deselect")?.unwrap_or(false) {
                session.selected.retain(|name| !wanted.contains(name));
            } else if params::boolean(params, "extend")?.unwrap_or(false) {
                session.selected.extend(wanted.iter().cloned());
            } else {
                session.selected = wanted.iter().cloned().collect();
            }
            session.active =
                wanted.last().cloned().filter(|name| session.selected.contains(name)).or_else(|| session.selected.iter().next_back().cloned());
        }
        "vertexGroup.select" => {
            let name = params::string(params, "name")?.ok_or_else(|| bad("`name` is required"))?;
            let Mode::Edit(id) = doc.session.mode else { return Err("not-allowed: not in edit mode".into()) };
            let mesh = doc.scene.object(id).ok().and_then(|object| object.mesh()).ok_or("not-found: the mesh")?;
            let group = vertex_data::group(mesh, name).ok_or_else(|| format!("not-found: no vertex group named {name}"))?;
            let picked: BTreeSet<u32> =
                group.weights.iter().enumerate().filter(|(_, weight)| **weight > 0.0).map(|(index, _)| index as u32).collect();
            doc.session.vertices.extend(picked);
        }
        other => return Err(format!("unknown-command: {other}")),
    }
    Ok(json!({ "changed": true }))
}

fn degrees(value: Vec3) -> Vec3 {
    Vec3::new(value.x.to_radians(), value.y.to_radians(), value.z.to_radians())
}

fn mesh_target(doc: &SceneDocument, params: &Value) -> Result<Id, String> {
    let id = match params.get("object").filter(|value| !value.is_null()) {
        Some(value) => params::resolve(&doc.scene, value)?,
        None => edited_mesh(doc).ok_or("not-allowed: there is no mesh to act on")?,
    };
    doc.scene.object(id).ok().and_then(|object| object.mesh()).ok_or_else(|| bad("that object is not a mesh"))?;
    Ok(id)
}

/// The vertices a command acts on: `vertices` when given, else the edit-mode selection.
fn chosen_vertices(doc: &SceneDocument, id: Id, params: &Value) -> Result<Vec<u32>, String> {
    let count = doc.scene.object(id).ok().and_then(|object| object.mesh()).map_or(0, |mesh| mesh.vertex_count());
    let list: Vec<u32> = match params::indices(params, "vertices")? {
        Some(list) => list,
        None if doc.session.mode == Mode::Edit(id) => doc.session.vertices.iter().copied().collect(),
        None => return Err(bad("name `vertices`, or select them in edit mode")),
    };
    if let Some(bad_vertex) = list.iter().find(|vertex| **vertex >= count) {
        return Err(format!("not-found: vertex {bad_vertex}"));
    }
    Ok(list)
}

fn mesh_mut(scene: &mut Scene, id: Id) -> Result<&mut kition_scene_model::Mesh, String> {
    scene.object_mut(id).map_err(|error| error.to_string())?.mesh_mut().ok_or_else(|| bad("that object is not a mesh"))
}

fn run_change(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    match command {
        "object.addArmature" => {
            let name = params::string(params, "name")?.unwrap_or("Armature");
            let id = doc.scene.add_object(name, ObjectData::Armature(Armature::starter()));
            if let Some(location) = params::vec3(params, "location")? {
                doc.scene.object_mut(id).map_err(|error| error.to_string())?.location = location;
            }
            doc.session.editor.selected = vec![id];
            doc.session.editor.active = Some(id);
            let name = doc.scene.object(id).map(|object| object.name.clone()).unwrap_or_default();
            Ok(json!({ "object": id, "name": name }))
        }
        "bone.add" => {
            let rig = rig_of(doc)?.object;
            let name = params::string(params, "name")?.unwrap_or("Bone").to_string();
            let head = params::vec3(params, "head")?.unwrap_or(Vec3::ZERO);
            let tail = params::vec3(params, "tail")?.unwrap_or(head + Vec3::Z);
            let parent = params::string(params, "parent")?.map(str::to_string);
            let connected = params::boolean(params, "connected")?.unwrap_or(false);
            let armature = armature_mut(&mut doc.scene, rig).ok_or("not-found: the armature")?;
            if parent.as_ref().is_some_and(|parent| armature.bone(parent).is_none()) {
                return Err(bad("the parent bone does not exist"));
            }
            let name = armature.unique_name(&name);
            let mut bone = Bone::new(name.clone(), head, tail);
            bone.parent = parent;
            bone.connected = connected && bone.parent.is_some();
            if bone.connected
                && let Some(parent_tail) = bone.parent.as_deref().and_then(|parent| armature.bone(parent)).map(|parent| parent.tail)
            {
                bone.head = parent_tail;
            }
            armature.bones.push(bone);
            select_bone(doc, &name);
            Ok(json!({ "bone": name }))
        }
        "bone.extrude" => {
            let rig = rig_of(doc)?.clone();
            let tail = params::vec3(params, "tail")?;
            let length = params::number(params, "length")?;
            let armature = armature_mut(&mut doc.scene, rig.object).ok_or("not-found: the armature")?;
            let mut made = Vec::new();
            for name in &rig.selected {
                let Some(from) = armature.bone(name).cloned() else { continue };
                let direction = (from.tail - from.head).normalize_or(Vec3::Z);
                let end = tail.unwrap_or(from.tail + direction * length.unwrap_or(from.length()));
                let new_name = armature.unique_name(&from.name);
                let mut bone = Bone::new(new_name.clone(), from.tail, end);
                bone.roll = from.roll;
                bone.parent = Some(from.name.clone());
                bone.connected = true;
                armature.bones.push(bone);
                made.push(new_name);
            }
            if let Some(session) = doc.session.rig.as_mut() {
                session.selected = made.iter().cloned().collect();
                session.active = made.last().cloned();
            }
            Ok(json!({ "bones": made }))
        }
        "bone.set" => {
            let object = named_armature(doc, params)?;
            let bone_name = match params::string(params, "bone")? {
                Some(name) => name.to_string(),
                // The active bone is the session's armature's.
                None => doc
                    .session
                    .rig
                    .as_ref()
                    .filter(|rig| rig.object == object)
                    .and_then(|rig| rig.active.clone())
                    .ok_or_else(|| bad("name the `bone`"))?,
            };
            let new_name = params::string(params, "name")?.map(str::trim).filter(|name| !name.is_empty()).map(str::to_string);
            let head = params::vec3(params, "head")?;
            let tail = params::vec3(params, "tail")?;
            let roll = params::number(params, "roll")?;
            let parent = params::string(params, "parent")?.map(str::to_string);
            let connected = params::boolean(params, "connected")?;
            let deform = params::boolean(params, "deform")?;
            let envelope = [
                params::number(params, "headRadius")?,
                params::number(params, "tailRadius")?,
                params::number(params, "envelopeDistance")?,
                params::number(params, "envelopeWeight")?,
            ];
            if envelope.iter().flatten().any(|value| !(0.0..=armature::MAX_ENVELOPE).contains(value)) {
                return Err(bad(format!("envelope radii, distance, and weight go from 0 to {}", armature::MAX_ENVELOPE)));
            }
            let multiply = params::boolean(params, "envelopeMultiply")?;
            let shape = match params.get("shape") {
                None => None,
                Some(Value::Null) => Some(None),
                Some(value) => {
                    let id = params::resolve(&doc.scene, value)?;
                    let found = doc.scene.object(id).map_err(|error| error.to_string())?;
                    if id == object || !found.has_geometry() {
                        return Err(bad("a custom shape is another object with a mesh, curve, or text"));
                    }
                    Some(Some(id))
                }
            };
            let shape_translation = params::vec3(params, "shapeTranslation")?;
            let shape_rotation = params::vec3(params, "shapeRotation")?;
            let shape_scale = params::vec3(params, "shapeScale")?;
            let shape_bone_size = params::boolean(params, "shapeBoneSize")?;
            let shape_wire = params::boolean(params, "shapeWire")?;
            let armature = armature_mut(&mut doc.scene, object).ok_or("not-found: the armature")?;
            armature.bone(&bone_name).ok_or_else(|| format!("not-found: no bone named {bone_name}"))?;
            if let Some(parent) = parent.as_deref().filter(|parent| !parent.is_empty())
                && armature.bone(parent).is_none()
            {
                return Err(bad(format!("no bone named {parent}")));
            }
            let mut renamed = bone_name.clone();
            if let Some(name) = new_name
                && name != bone_name
            {
                renamed = armature.unique_name(&name);
                armature.rename(&bone_name, &renamed);
                rename_groups(&mut doc.scene, object, &bone_name, &renamed);
                rename_channels(&mut doc.scene, object, &bone_name, &renamed);
            }
            let armature = armature_mut(&mut doc.scene, object).ok_or("not-found: the armature")?;
            let bone = armature.bone_mut(&renamed).ok_or("not-found: the bone")?;
            if let Some(head) = head {
                bone.head = head;
            }
            if let Some(tail) = tail {
                bone.tail = tail;
            }
            if let Some(roll) = roll {
                bone.roll = roll.to_radians();
            }
            if let Some(parent) = parent {
                bone.parent = (!parent.is_empty()).then_some(parent);
                if bone.parent.is_none() {
                    bone.connected = false;
                }
            }
            if let Some(connected) = connected {
                bone.connected = connected && bone.parent.is_some();
            }
            if let Some(deform) = deform {
                bone.deform = deform;
            }
            let [head_radius, tail_radius, distance, weight] = envelope;
            bone.head_radius = head_radius.unwrap_or(bone.head_radius);
            bone.tail_radius = tail_radius.unwrap_or(bone.tail_radius);
            bone.envelope_distance = distance.unwrap_or(bone.envelope_distance);
            bone.envelope_weight = weight.unwrap_or(bone.envelope_weight);
            bone.envelope_multiply = multiply.unwrap_or(bone.envelope_multiply);
            match shape {
                Some(None) => bone.shape = None,
                Some(Some(id)) => bone.shape = Some(bone.shape.take().map_or_else(|| BoneShape::new(id), |shape| BoneShape { object: id, ..shape })),
                None => {}
            }
            let shaped =
                [shape_translation, shape_rotation, shape_scale].iter().any(Option::is_some) || shape_bone_size.is_some() || shape_wire.is_some();
            if shaped {
                let shape = bone.shape.as_mut().ok_or_else(|| bad("the bone has no custom shape; give `shape` first"))?;
                shape.translation = shape_translation.unwrap_or(shape.translation);
                shape.rotation = shape_rotation.map_or(shape.rotation, |degrees| degrees * std::f32::consts::PI / 180.0);
                shape.scale = shape_scale.unwrap_or(shape.scale);
                shape.bone_size = shape_bone_size.unwrap_or(shape.bone_size);
                shape.wire = shape_wire.unwrap_or(shape.wire);
            }
            follow_connections(armature);
            if let Some(session) = doc.session.rig.as_mut()
                && renamed != bone_name
            {
                if session.selected.remove(&bone_name) {
                    session.selected.insert(renamed.clone());
                }
                if session.active.as_deref() == Some(bone_name.as_str()) {
                    session.active = Some(renamed.clone());
                }
            }
            Ok(json!({ "bone": renamed }))
        }
        "pose.set" => {
            let bones = chosen_bones(doc, params)?;
            let rig = rig_of(doc)?.object;
            let location = params::vec3(params, "location")?;
            let rotation = params::vec3(params, "rotation")?;
            let scale = params::vec3(params, "scale")?;
            let armature = armature_mut(&mut doc.scene, rig).ok_or("not-found: the armature")?;
            for name in &bones {
                let bone = armature.bone_mut(name).ok_or("not-found: the bone")?;
                if let Some(location) = location {
                    bone.pose.location = location;
                }
                if let Some(rotation) = rotation {
                    bone.pose.rotation = degrees(rotation);
                }
                if let Some(scale) = scale {
                    bone.pose.scale = scale;
                }
            }
            Ok(json!({ "bones": bones }))
        }
        "pose.clear" => {
            let rig = rig_of(doc)?.object;
            let bones = if params::boolean(params, "all")?.unwrap_or(false) {
                armature_of(&doc.scene, rig).map(|armature| armature.bones.iter().map(|bone| bone.name.clone()).collect()).unwrap_or_default()
            } else {
                chosen_bones(doc, params)?
            };
            let armature = armature_mut(&mut doc.scene, rig).ok_or("not-found: the armature")?;
            for name in &bones {
                if let Some(bone) = armature.bone_mut(name) {
                    bone.pose = Default::default();
                }
            }
            Ok(json!({ "bones": bones }))
        }
        "pose.insertKey" | "pose.deleteKey" => {
            let bones = chosen_bones(doc, params)?;
            let rig = rig_of(doc)?.object;
            let frame = params::number(params, "frame")?.unwrap_or(doc.scene.animation.timeline.frame as f32);
            if command == "pose.insertKey"
                && let Some(set) = crate::commands_anim2::keying_set_for(doc, params)?
            {
                let keys = doc.scene.insert_keying_set(&set, &[rig], Some((rig, &bones)), frame).map_err(|error| error.to_string())?;
                return Ok(json!({ "frame": frame, "keys": keys }));
            }
            let wanted: Vec<Property> = match params.get("properties").filter(|value| !value.is_null()) {
                None => vec![Property::BoneLocation, Property::BoneRotation, Property::BoneScale],
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|item| match item.as_str() {
                        Some("location") => Ok(Property::BoneLocation),
                        Some("rotation") => Ok(Property::BoneRotation),
                        Some("scale") => Ok(Property::BoneScale),
                        _ => Err(bad(format!("unknown bone property {item}"))),
                    })
                    .collect::<Result<_, _>>()?,
                Some(_) => return Err(bad("`properties` is a list")),
            };
            let mut keys = 0;
            for name in &bones {
                for property in &wanted {
                    if command == "pose.insertKey" {
                        keys += doc.scene.insert_key_at(rig, *property, name, frame).map_err(|error| error.to_string())?;
                    } else {
                        keys += doc.scene.delete_key_at(rig, *property, Some(name), frame);
                    }
                }
            }
            if keys == 0 {
                return Err("not-allowed: there is no key on this frame".into());
            }
            Ok(json!({ "keys": keys, "frame": frame }))
        }
        "object.parentArmature" => {
            let rig = match params.get("armature").filter(|value| !value.is_null()) {
                Some(value) => params::resolve(&doc.scene, value)?,
                None => active_armature(doc).ok_or("not-allowed: make the armature the active object")?,
            };
            armature_of(&doc.scene, rig).ok_or_else(|| bad("`armature` is not an armature"))?;
            let meshes: Vec<Id> = match params::objects(&doc.scene, params, "objects")? {
                Some(ids) => ids,
                None => doc.session.editor.selected.iter().copied().filter(|id| *id != rig).collect(),
            };
            let meshes: Vec<Id> = meshes.into_iter().filter(|id| doc.scene.object(*id).is_ok_and(|object| object.mesh().is_some())).collect();
            if meshes.is_empty() {
                return Err("not-allowed: select the meshes, then the armature".into());
            }
            let weights = params::string(params, "weights")?.unwrap_or("automatic");
            if !["automatic", "empty", "none"].contains(&weights) {
                return Err(bad("`weights` is automatic, empty, or none"));
            }
            let mut groups = 0;
            for mesh in &meshes {
                doc.scene.set_parent(*mesh, Some(rig)).map_err(|error| format!("invalid-params: {error}"))?;
                let object = doc.scene.object_mut(*mesh).map_err(|error| error.to_string())?;
                if !object.modifiers.iter().any(|modifier| matches!(modifier.kind, ModifierKind::Armature { object: Some(id), .. } if id == rig)) {
                    object
                        .modifiers
                        .insert(0, Modifier::new(ModifierKind::Armature { object: Some(rig), preserve_volume: false, deform: Default::default() }));
                }
                if weights != "none" {
                    groups += armature::bind(&mut doc.scene, *mesh, rig, weights == "automatic").map_err(|error| error.to_string())?.len();
                }
            }
            Ok(json!({ "armature": rig, "objects": meshes, "groups": groups }))
        }
        "vertexGroup.add" => {
            let id = mesh_target(doc, params)?;
            let mesh = mesh_mut(&mut doc.scene, id)?;
            let base = params::string(params, "name")?.unwrap_or("Group");
            let name = kition_scene_model::scene::unique_name(base, |candidate| mesh.groups.iter().any(|group| group.name == candidate));
            vertex_data::group_mut(mesh, &name);
            Ok(json!({ "object": id, "group": name }))
        }
        "vertexGroup.remove" => {
            let id = mesh_target(doc, params)?;
            let name = params::string(params, "name")?.ok_or_else(|| bad("`name` is required"))?.to_string();
            let mesh = mesh_mut(&mut doc.scene, id)?;
            let before = mesh.groups.len();
            mesh.groups.retain(|group| group.name != name);
            if mesh.groups.len() == before {
                return Err(format!("not-found: no vertex group named {name}"));
            }
            Ok(json!({ "object": id, "removed": name }))
        }
        "vertexGroup.assign" | "vertexGroup.unassign" => {
            let id = mesh_target(doc, params)?;
            let name = params::string(params, "name")?.ok_or_else(|| bad("`name` is required"))?.to_string();
            let weight = if command == "vertexGroup.assign" { params::number(params, "weight")?.unwrap_or(1.0) } else { 0.0 };
            if !(0.0..=1.0).contains(&weight) {
                return Err(bad("`weight` is between 0 and 1"));
            }
            let vertices = chosen_vertices(doc, id, params)?;
            let mesh = mesh_mut(&mut doc.scene, id)?;
            if command == "vertexGroup.unassign" && vertex_data::group(mesh, &name).is_none() {
                return Err(format!("not-found: no vertex group named {name}"));
            }
            let group = vertex_data::group_mut(mesh, &name).ok_or("failed: the group")?;
            for vertex in &vertices {
                if let Some(slot) = group.weights.get_mut(*vertex as usize) {
                    *slot = weight;
                }
            }
            Ok(json!({ "object": id, "group": name, "vertices": vertices.len() }))
        }
        "shapeKey.add" => {
            let id = mesh_target(doc, params)?;
            let from = match params.get("from").filter(|value| !value.is_null()) {
                Some(value) => {
                    let other = params::resolve(&doc.scene, value)?;
                    let shape = doc.scene.object(other).ok().and_then(|object| object.mesh()).ok_or_else(|| bad("`from` is not a mesh"))?;
                    Some(shape.positions.clone())
                }
                None => None,
            };
            let offsets = match params.get("offsets").filter(|value| !value.is_null()) {
                Some(Value::Array(items)) => Some(
                    items
                        .iter()
                        .enumerate()
                        .map(|(index, item)| params::vec3(&json!({ "o": item }), "o")?.ok_or_else(|| bad(format!("offset {index} is not [x, y, z]"))))
                        .collect::<Result<Vec<_>, _>>()?,
                ),
                Some(_) => return Err(bad("`offsets` is a list of [x, y, z]")),
                None => None,
            };
            let base = params::string(params, "name")?.map(str::to_string);
            let mesh = mesh_mut(&mut doc.scene, id)?;
            let count = mesh.positions.len();
            let offsets = match (from, offsets) {
                (Some(shape), _) => {
                    if shape.len() != count {
                        return Err(bad("`from` must have as many vertices as the mesh"));
                    }
                    shape.iter().zip(&mesh.positions).map(|(target, basis)| *target - *basis).collect()
                }
                (None, Some(offsets)) => {
                    if offsets.len() != count {
                        return Err(bad("`offsets` must have one offset per vertex"));
                    }
                    offsets
                }
                (None, None) => vec![Vec3::ZERO; count],
            };
            let base = base.unwrap_or_else(|| format!("Key {}", mesh.shape_keys.len() + 1));
            let name = kition_scene_model::scene::unique_name(&base, |candidate| mesh.shape_keys.iter().any(|key| key.name == candidate));
            mesh.shape_keys.push(ShapeKey { name: name.clone(), offsets, value: 0.0 });
            Ok(json!({ "object": id, "shapeKey": name }))
        }
        "shapeKey.remove" => {
            let id = mesh_target(doc, params)?;
            let name = params::string(params, "name")?.ok_or_else(|| bad("`name` is required"))?.to_string();
            let mesh = mesh_mut(&mut doc.scene, id)?;
            let before = mesh.shape_keys.len();
            mesh.shape_keys.retain(|key| key.name != name);
            if mesh.shape_keys.len() == before {
                return Err(format!("not-found: no shape key named {name}"));
            }
            doc.scene.prune_animation();
            Ok(json!({ "object": id, "removed": name }))
        }
        "shapeKey.set" => {
            let id = mesh_target(doc, params)?;
            let name = params::string(params, "name")?.ok_or_else(|| bad("`name` is required"))?.to_string();
            let value = params::number(params, "value")?;
            let rename = params::string(params, "rename")?.map(str::trim).filter(|name| !name.is_empty()).map(str::to_string);
            let mesh = mesh_mut(&mut doc.scene, id)?;
            if rename.as_ref().is_some_and(|new| new != &name && mesh.shape_keys.iter().any(|key| &key.name == new)) {
                return Err(bad("a shape key already has that name"));
            }
            let key = mesh.shape_keys.iter_mut().find(|key| key.name == name).ok_or_else(|| format!("not-found: no shape key named {name}"))?;
            if let Some(value) = value {
                key.value = value.clamp(vertex_data::SHAPE_KEY_RANGE.0, vertex_data::SHAPE_KEY_RANGE.1);
            }
            if let Some(new) = &rename {
                key.name = new.clone();
                for channel in doc
                    .scene
                    .animation
                    .channels
                    .iter_mut()
                    .filter(|channel| channel.target == id && channel.property == Property::ShapeKey && channel.path == name)
                {
                    channel.path = new.clone();
                }
            }
            Ok(json!({ "object": id, "shapeKey": rename.unwrap_or(name) }))
        }
        "shapeKey.offset" => {
            let id = mesh_target(doc, params)?;
            let name = params::string(params, "name")?.ok_or_else(|| bad("`name` is required"))?.to_string();
            let offset = params::vec3(params, "offset")?.ok_or_else(|| bad("`offset` is required"))?;
            let vertices = chosen_vertices(doc, id, params)?;
            let mesh = mesh_mut(&mut doc.scene, id)?;
            let key = mesh.shape_keys.iter_mut().find(|key| key.name == name).ok_or_else(|| format!("not-found: no shape key named {name}"))?;
            for vertex in &vertices {
                if let Some(slot) = key.offsets.get_mut(*vertex as usize) {
                    *slot += offset;
                }
            }
            Ok(json!({ "object": id, "vertices": vertices.len() }))
        }
        "shapeKey.insertKey" => {
            let id = mesh_target(doc, params)?;
            let name = params::string(params, "name")?.ok_or_else(|| bad("`name` is required"))?.to_string();
            let frame = params::number(params, "frame")?.unwrap_or(doc.scene.animation.timeline.frame as f32);
            let keys = doc.scene.insert_key_at(id, Property::ShapeKey, &name, frame).map_err(|error| error.to_string())?;
            Ok(json!({ "object": id, "keys": keys }))
        }
        "graph.moveKey" | "graph.deleteKey" | "graph.setInterpolation" => graph(doc, command, params),
        other => Err(format!("unknown-command: {other}")),
    }
}

/// Edits one key of one channel, then puts the scene at the current frame again.
fn graph(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    let target = params::integer(params, "target")?.ok_or_else(|| bad("`target` is required"))?;
    let property = params::string(params, "property")?.and_then(Property::parse).ok_or_else(|| bad("`property` is not a property"))?;
    let index = u8::try_from(params::integer(params, "index")?.unwrap_or(0)).map_err(|_| bad("`index` is out of range"))?;
    let path = params::string(params, "path")?.unwrap_or("").to_string();
    let frame = params::number(params, "frame")?.ok_or_else(|| bad("`frame` is required"))?;
    let degrees = matches!(property, Property::Rotation | Property::BoneRotation);
    let channels = &mut doc.scene.animation.channels;
    let slot = channels
        .iter()
        .position(|channel| channel.target == target && channel.property == property && channel.index == index && channel.path == path)
        .ok_or("not-found: no such channel")?;
    let channel = channels.get_mut(slot).ok_or("not-found: no such channel")?;
    let key = channel.keys.iter().position(|key| (key.frame - frame).abs() < 1e-3).ok_or("not-found: no key on that frame")?;
    match command {
        "graph.deleteKey" => {
            channel.keys.remove(key);
            if channel.keys.is_empty() {
                channels.remove(slot);
            }
        }
        "graph.setInterpolation" => {
            let interpolation = params::string(params, "interpolation")?
                .and_then(kition_scene_model::Interpolation::parse)
                .ok_or_else(|| bad("`interpolation` is bezier, linear, or constant"))?;
            if let Some(found) = channel.keys.get_mut(key) {
                found.interpolation = interpolation;
            }
        }
        _ => {
            let old = channel.keys.get(key).copied().ok_or("not-found: the key")?;
            let to_frame = params::number(params, "toFrame")?.map(|frame| (frame * 1000.0).round() / 1000.0).unwrap_or(old.frame);
            let value = params::number(params, "value")?.map(|value| if degrees { value.to_radians() } else { value }).unwrap_or(old.value);
            if channel.keys.iter().enumerate().any(|(other, found)| other != key && (found.frame - to_frame).abs() < 1e-3) {
                return Err(bad("another key is on that frame"));
            }
            channel.keys.remove(key);
            channel.insert(to_frame, value, old.interpolation);
        }
    }
    let now = doc.scene.animation.timeline.frame as f32;
    doc.scene.go_to_frame(now);
    Ok(json!({ "changed": true }))
}

fn select_bone(doc: &mut SceneDocument, name: &str) {
    if let Some(session) = doc.session.rig.as_mut() {
        session.selected = BTreeSet::from([name.to_string()]);
        session.active = Some(name.to_string());
    }
}

/// Keeps connected bones' heads on their parents' tails.
fn follow_connections(armature: &mut Armature) {
    for _ in 0..armature.bones.len().min(64) {
        let tails: Vec<(String, Vec3)> = armature.bones.iter().map(|bone| (bone.name.clone(), bone.tail)).collect();
        let mut changed = false;
        for bone in &mut armature.bones {
            if !bone.connected {
                continue;
            }
            if let Some((_, tail)) = bone.parent.as_ref().and_then(|parent| tails.iter().find(|(name, _)| name == parent))
                && bone.head != *tail
            {
                bone.head = *tail;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

/// Renames the vertex groups of meshes deformed by armature `rig` after a bone's new name.
fn rename_groups(scene: &mut Scene, rig: Id, from: &str, to: &str) {
    for object in &mut scene.objects {
        let deformed = object.modifiers.iter().any(|modifier| matches!(modifier.kind, ModifierKind::Armature { object: Some(id), .. } if id == rig));
        if !deformed {
            continue;
        }
        if let Some(mesh) = object.mesh_mut() {
            for group in &mut mesh.groups {
                if group.name == from {
                    group.name = to.to_string();
                }
            }
        }
        // Armature modifiers mixing by the renamed group follow it.
        for modifier in &mut object.modifiers {
            if let ModifierKind::Armature { deform, .. } = &mut modifier.kind
                && deform.vertex_group.as_deref() == Some(from)
            {
                deform.vertex_group = Some(to.to_string());
            }
        }
    }
}

fn rename_channels(scene: &mut Scene, rig: Id, from: &str, to: &str) {
    for channel in scene.animation.channels.iter_mut().filter(|channel| channel.target == rig && channel.property.named() && channel.path == from) {
        channel.path = to.to_string();
    }
}

fn delete_bones(doc: &mut SceneDocument) -> Result<Value, String> {
    let rig = rig_of(doc)?.clone();
    if rig.pose {
        return Err("not-allowed: bones are deleted in edit mode".into());
    }
    if rig.selected.is_empty() {
        return Err("not-allowed: select bones to delete".into());
    }
    let armature = armature_mut(&mut doc.scene, rig.object).ok_or("not-found: the armature")?;
    armature.remove(&rig.selected);
    doc.scene.prune_animation();
    if let Some(session) = doc.session.rig.as_mut() {
        session.selected.clear();
        session.active = None;
    }
    Ok(json!({ "deleted": rig.selected.len() }))
}

fn duplicate_bones(doc: &mut SceneDocument) -> Result<Value, String> {
    let rig = rig_of(doc)?.clone();
    if rig.pose || rig.selected.is_empty() {
        return Err("not-allowed: select bones in edit mode to duplicate".into());
    }
    let armature = armature_mut(&mut doc.scene, rig.object).ok_or("not-found: the armature")?;
    let mut renames = Vec::new();
    for name in &rig.selected {
        let new_name = armature.unique_name(name);
        let Some(bone) = armature.bone(name).cloned() else { continue };
        armature.bones.push(Bone { name: new_name.clone(), ..bone });
        renames.push((name.clone(), new_name));
    }
    // Copies keep their parents among the copies.
    for bone in &mut armature.bones {
        if renames.iter().any(|(_, new)| *new == bone.name)
            && let Some(parent) = bone.parent.clone()
            && let Some((_, new_parent)) = renames.iter().find(|(old, _)| *old == parent)
        {
            bone.parent = Some(new_parent.clone());
        }
    }
    let made: Vec<String> = renames.into_iter().map(|(_, new)| new).collect();
    if let Some(session) = doc.session.rig.as_mut() {
        session.selected = made.iter().cloned().collect();
        session.active = made.last().cloned();
    }
    Ok(json!({ "bones": made }))
}

/// The armature mode's part of `inspect()`: every armature's bones, the mode and its selection,
/// and every mesh's vertex groups and shape keys.
pub fn extend_inspect(doc: &SceneDocument, inspected: &mut Value) {
    let round = |value: f32| (f64::from(value) * 10_000.0).round() / 10_000.0;
    let triple = |value: Vec3| json!([round(value.x), round(value.y), round(value.z)]);
    let Some(map) = inspected.as_object_mut() else { return };
    if let Some(objects) = map.get_mut("objects").and_then(Value::as_array_mut) {
        for entry in objects {
            let Some(id) = entry.get("id").and_then(Value::as_u64).and_then(|id| u32::try_from(id).ok()) else { continue };
            let Ok(object) = doc.scene.object(id) else { continue };
            let Some(fields) = entry.as_object_mut() else { continue };
            match &object.data {
                ObjectData::Armature(armature) => {
                    let bones: Vec<Value> = armature
                        .bones
                        .iter()
                        .map(|bone| {
                            json!({
                                "name": bone.name,
                                "head": triple(bone.head),
                                "tail": triple(bone.tail),
                                "roll": round(bone.roll.to_degrees()),
                                "parent": bone.parent,
                                "connected": bone.connected,
                                "deform": bone.deform,
                                "pose": {
                                    "location": triple(bone.pose.location),
                                    "rotation": triple(Vec3::new(bone.pose.rotation.x.to_degrees(), bone.pose.rotation.y.to_degrees(), bone.pose.rotation.z.to_degrees())),
                                    "scale": triple(bone.pose.scale),
                                },
                                "envelope": {
                                    "headRadius": round(bone.head_radius),
                                    "tailRadius": round(bone.tail_radius),
                                    "distance": round(bone.envelope_distance),
                                    "weight": round(bone.envelope_weight),
                                    "multiply": bone.envelope_multiply,
                                },
                                "shape": bone.shape.as_ref().map(|shape| json!({
                                    "object": shape.object,
                                    "name": doc.scene.object(shape.object).map(|other| other.name.clone()).ok(),
                                    "translation": triple(shape.translation),
                                    "rotation": triple(shape.rotation * (180.0 / std::f32::consts::PI)),
                                    "scale": triple(shape.scale),
                                    "boneSize": shape.bone_size,
                                    "wire": shape.wire,
                                })),
                            })
                        })
                        .collect();
                    fields.insert("armature".into(), json!({ "bones": bones }));
                }
                ObjectData::Mesh(mesh) => {
                    if !mesh.groups.is_empty() {
                        let groups: Vec<Value> = mesh
                            .groups
                            .iter()
                            .map(|group| json!({ "name": group.name, "vertices": group.weights.iter().filter(|weight| **weight > 0.0).count() }))
                            .collect();
                        fields.insert("vertexGroups".into(), Value::Array(groups));
                    }
                    if !mesh.shape_keys.is_empty() {
                        let keys: Vec<Value> = mesh.shape_keys.iter().map(|key| json!({ "name": key.name, "value": round(key.value) })).collect();
                        fields.insert("shapeKeys".into(), Value::Array(keys));
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(rig) = &doc.session.rig
        && let Some(mode) = map.get_mut("mode").and_then(Value::as_object_mut)
    {
        mode.insert("mode".into(), json!(if rig.pose { "pose" } else { "armatureEdit" }));
        mode.insert("object".into(), json!(rig.object));
        mode.insert("bones".into(), json!(rig.selected.iter().collect::<Vec<_>>()));
        mode.insert("activeBone".into(), json!(rig.active));
    }
}

// ----- The viewport tools in an armature mode -----

fn projector(doc: &SceneDocument) -> kition_scene_raster::Projector {
    doc.session.editor.view.projector(&doc.scene, doc.session.width, doc.session.height)
}

/// Each bone's head and tail in world space as drawn now (posed in pose mode, at rest in edit mode).
fn bone_segments(doc: &SceneDocument, rig: &RigSession) -> Vec<(String, Vec3, Vec3)> {
    let (Some(armature), Ok(world)) = (armature_of(&doc.scene, rig.object), doc.scene.world_matrix(rig.object)) else { return Vec::new() };
    if rig.pose {
        let poses = kition_scene_model::constraints::bone_matrices(&doc.scene, rig.object, 0);
        armature
            .bones
            .iter()
            .filter_map(|bone| {
                let matrix = world * *poses.get(&bone.name)?;
                Some((bone.name.clone(), matrix.transform_point3(Vec3::ZERO), matrix.transform_point3(Vec3::new(0.0, bone.length(), 0.0))))
            })
            .collect()
    } else {
        armature.bones.iter().map(|bone| (bone.name.clone(), world.transform_point3(bone.head), world.transform_point3(bone.tail))).collect()
    }
}

fn to_screen(projector: &kition_scene_raster::Projector, point: Vec3) -> Option<(f32, f32)> {
    let clip = projector.view_projection() * point.extend(1.0);
    if clip.w <= 0.0 {
        return None;
    }
    let screen = projector.to_screen(clip);
    Some((screen.x, screen.y))
}

/// The bone drawn nearest pixel (`x`, `y`), within a few pixels.
fn bone_at(doc: &SceneDocument, rig: &RigSession, x: f32, y: f32) -> Option<String> {
    let projector = projector(doc);
    let mut best: Option<(f32, String)> = None;
    for (name, head, tail) in bone_segments(doc, rig) {
        let (Some(a), Some(b)) = (to_screen(&projector, head), to_screen(&projector, tail)) else { continue };
        let distance = armature::distance_to_segment(Vec3::new(x, y, 0.0), Vec3::new(a.0, a.1, 0.0), Vec3::new(b.0, b.1, 0.0));
        if distance < 10.0 && best.as_ref().is_none_or(|(nearest, _)| distance < *nearest) {
            best = Some((distance, name));
        }
    }
    best.map(|(_, name)| name)
}

fn pick(doc: &mut SceneDocument, start: (f64, f64), end: (f64, f64), toggle: bool, marquee: bool) {
    let Some(rig) = doc.session.rig.clone() else { return };
    let found: Vec<String> = if marquee {
        let projector = projector(doc);
        let (left, right) = (start.0.min(end.0) as f32, start.0.max(end.0) as f32);
        let (top, bottom) = (start.1.min(end.1) as f32, start.1.max(end.1) as f32);
        let inside = |point: Option<(f32, f32)>| point.is_some_and(|(x, y)| x >= left && x <= right && y >= top && y <= bottom);
        bone_segments(doc, &rig)
            .into_iter()
            .filter(|(_, head, tail)| inside(to_screen(&projector, *head)) || inside(to_screen(&projector, *tail)))
            .map(|(name, _, _)| name)
            .collect()
    } else {
        bone_at(doc, &rig, end.0 as f32, end.1 as f32).into_iter().collect()
    };
    let Some(session) = doc.session.rig.as_mut() else { return };
    if toggle {
        for name in found {
            if !session.selected.insert(name.clone()) && (marquee || session.active.as_deref() == Some(name.as_str())) {
                session.selected.remove(&name);
                if session.active.as_deref() == Some(name.as_str()) {
                    session.active = session.selected.iter().next_back().cloned();
                }
            } else {
                session.active = Some(name);
            }
        }
    } else {
        session.active = found.last().cloned();
        session.selected = found.into_iter().collect();
    }
    doc.touched();
}

/// The world-space centre of the selected bones (heads in pose mode; heads and tails in edit mode).
fn pivot(doc: &SceneDocument, rig: &RigSession) -> Option<Vec3> {
    let points: Vec<Vec3> = bone_segments(doc, rig)
        .into_iter()
        .filter(|(name, _, _)| rig.selected.contains(name))
        .flat_map(|(_, head, tail)| if rig.pose { vec![head] } else { vec![head, tail] })
        .collect();
    (!points.is_empty()).then(|| points.iter().copied().sum::<Vec3>() / points.len() as f32)
}

/// A world-space change of the drag: what it does to a point, as a rotation about the view axis,
/// a move, or a scale about the pivot.
enum Gesture {
    Move(Vec3),
    Rotate(Mat3),
    Scale(f32),
}

fn gesture(doc: &SceneDocument, tool: &str, drag: &RigDrag, at: (f64, f64), shift: bool) -> Option<(Gesture, String)> {
    let projector = projector(doc);
    let fine = if shift { 0.1 } else { 1.0 };
    let (dx, dy) = ((at.0 - drag.start.0) as f32 * fine, (at.1 - drag.start.1) as f32 * fine);
    let (sx, sy) = drag.pivot_screen;
    let depth = projector.to_screen(projector.view_projection() * drag.pivot.extend(1.0)).z;
    match tool {
        "move" => {
            let delta = projector.unproject(sx + dx, sy + dy, depth) - projector.unproject(sx, sy, depth);
            Some((Gesture::Move(delta), format!("D: {:.3}  {:.3}  {:.3} m", delta.x, delta.y, delta.z)))
        }
        "rotate" => {
            let from = (drag.start.1 as f32 - sy).atan2(drag.start.0 as f32 - sx);
            let to = (at.1 as f32 - sy).atan2(at.0 as f32 - sx);
            let angle = -(to - from) * fine;
            let axis = if projector.perspective {
                (projector.eye() - drag.pivot).normalize_or_zero()
            } else {
                Mat3::from_mat4(projector.view.inverse()) * Vec3::Z
            };
            Some((Gesture::Rotate(Mat3::from_axis_angle(axis, angle)), format!("Rotation: {:.1}°", angle.to_degrees())))
        }
        "scale" => {
            let from = (drag.start.0 as f32 - sx).hypot(drag.start.1 as f32 - sy).max(1.0);
            let to = (at.0 as f32 - sx).hypot(at.1 as f32 - sy);
            let factor = 1.0 + (to / from - 1.0) * fine;
            Some((Gesture::Scale(factor), format!("Scale: {factor:.3}")))
        }
        _ => None,
    }
}

fn orthonormal(matrix: Mat3) -> Mat3 {
    let x = matrix.x_axis.normalize_or_zero();
    let y = (matrix.y_axis - x * x.dot(matrix.y_axis)).normalize_or_zero();
    Mat3::from_cols(x, y, x.cross(y))
}

/// Applies a drag to the bones, from the state before it began.
fn apply(doc: &mut SceneDocument, rig: &RigSession, drag: &RigDrag, gesture: &Gesture) {
    let mut scene = drag.before.scene.clone();
    let Ok(world) = scene.world_matrix(rig.object) else { return };
    let inverse = world.inverse();
    let Some(armature) = armature_mut(&mut scene, rig.object) else { return };
    if rig.pose {
        let poses = armature.pose_matrices();
        for bone in armature.bones.iter_mut().filter(|bone| rig.selected.contains(&bone.name)) {
            let Some(pose) = poses.get(&bone.name) else { continue };
            // The bone's frame before its own pose: world × armature pose × own pose⁻¹.
            let base = world * *pose * bone.pose.matrix(bone.connected).inverse();
            match gesture {
                Gesture::Move(delta) => {
                    if !bone.connected {
                        bone.pose.location += base.inverse().transform_vector3(*delta);
                    }
                }
                Gesture::Rotate(rotation) => {
                    let frame = orthonormal(Mat3::from_mat4(base));
                    let turned = frame.transpose() * *rotation * frame * euler_matrix(bone.pose.rotation);
                    bone.pose.rotation = matrix_euler(orthonormal(turned));
                }
                Gesture::Scale(factor) => bone.pose.scale *= *factor,
            }
        }
    } else {
        let pivot = inverse.transform_point3(drag.pivot);
        let change = |point: Vec3| -> Vec3 {
            match gesture {
                Gesture::Move(delta) => point + inverse.transform_vector3(*delta),
                Gesture::Rotate(rotation) => {
                    let local = Mat3::from_mat4(inverse) * *rotation * Mat3::from_mat4(world);
                    pivot + local * (point - pivot)
                }
                Gesture::Scale(factor) => pivot + (point - pivot) * *factor,
            }
        };
        let moving_heads: Vec<(String, Option<String>, bool)> = armature
            .bones
            .iter()
            .filter(|bone| rig.selected.contains(&bone.name))
            .map(|bone| (bone.name.clone(), bone.parent.clone(), bone.connected))
            .collect();
        for bone in armature.bones.iter_mut().filter(|bone| rig.selected.contains(&bone.name)) {
            bone.head = change(bone.head);
            bone.tail = change(bone.tail);
        }
        // A connected bone moved without its parent takes the parent's tail along.
        for (_, parent, connected) in &moving_heads {
            if let (true, Some(parent)) = (*connected, parent)
                && !rig.selected.contains(parent)
                && let Some(parent) = armature.bone_mut(parent)
            {
                parent.tail = change(parent.tail);
            }
        }
        follow_connections(armature);
    }
    doc.scene = scene;
    doc.touched();
}

/// The armature mode's handling of a pointer event, or `None` when no armature mode is on.
pub fn pointer(doc: &mut SceneDocument, tool: &str, event: &ToolPointer) -> Option<ToolResponse> {
    let rig = doc.session.rig.clone()?;
    let at = (event.x, event.y);
    let cursor = if tool == "move" {
        "move"
    } else if tool == "select" {
        "default"
    } else {
        "crosshair"
    };
    let idle = |busy: bool| ToolResponse { cursor: cursor.into(), busy, ..Default::default() };
    match event.action {
        PointerAction::Down => {
            let selecting = tool == "select" || !["move", "rotate", "scale"].contains(&tool);
            if !selecting && rig.selected.is_empty() {
                pick(doc, at, at, false, false);
            }
            let rig = doc.session.rig.clone()?;
            let pivot = pivot(doc, &rig).unwrap_or(Vec3::ZERO);
            let screen = to_screen(&projector(doc), pivot).unwrap_or((0.0, 0.0));
            let drag = RigDrag {
                start: at,
                pivot,
                pivot_screen: screen,
                before: Box::new(doc.step(if selecting { "Select" } else { "Transform Bones" })),
                moved: false,
                marquee: selecting,
            };
            if let Some(session) = doc.session.rig.as_mut() {
                session.drag = Some(drag);
            }
            Some(idle(true))
        }
        PointerAction::Drag => {
            let drag = rig.drag.clone()?;
            if drag.marquee {
                return Some(ToolResponse {
                    overlays: vec![ToolOverlay::Marquee { x0: drag.start.0, y0: drag.start.1, x1: at.0, y1: at.1 }],
                    cursor: cursor.into(),
                    busy: true,
                    ..Default::default()
                });
            }
            let (change, status) = gesture(doc, tool, &drag, at, event.mods.shift)?;
            apply(doc, &rig, &drag, &change);
            if let Some(session) = doc.session.rig.as_mut()
                && let Some(drag) = session.drag.as_mut()
            {
                drag.moved = true;
            }
            Some(ToolResponse {
                overlays: vec![ToolOverlay::Measure { x: at.0, y: at.1, text: status }],
                cursor: cursor.into(),
                busy: true,
                ..Default::default()
            })
        }
        PointerAction::Up => {
            let drag = doc.session.rig.as_mut().and_then(|session| session.drag.take())?;
            if drag.marquee {
                let boxed = (at.0 - drag.start.0).abs() > 4.0 || (at.1 - drag.start.1).abs() > 4.0;
                pick(doc, drag.start, at, event.mods.shift, boxed);
            } else if drag.moved && doc.scene != drag.before.scene {
                if doc.scene.validate().is_ok() {
                    // A refused change was put back by `committed`; the picture follows.
                    let _ = doc.committed(*drag.before);
                } else {
                    let rig = doc.session.rig.clone();
                    doc.restore(*drag.before);
                    doc.session.rig = rig;
                    doc.touched();
                }
            }
            Some(idle(false))
        }
        PointerAction::DoubleClick => {
            pick(doc, at, at, false, false);
            Some(idle(false))
        }
        PointerAction::Move => Some(idle(false)),
    }
}

/// Escape during a bone drag puts everything back.
pub fn cancel(doc: &mut SceneDocument) -> bool {
    let Some(drag) = doc.session.rig.as_mut().and_then(|session| session.drag.take()) else { return false };
    if drag.moved {
        let rig = doc.session.rig.clone();
        doc.restore(*drag.before);
        doc.session.rig = rig;
        doc.touched();
    }
    true
}
