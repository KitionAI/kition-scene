//! Curve and text objects: adding them, their settings, converting them to meshes, curve edit
//! mode (selecting, moving, extruding, subdividing, and deleting control points and handles) and
//! text edit mode (typing into the text in the viewport).
//!
//! Like an armature's modes, the two edit modes are kept beside the mesh modes ([`ShapeSession`]):
//! while one is on, the document is in object mode for everything else, but Tab, select all, and
//! delete act on the curve's points or the text's letters.

use std::collections::BTreeSet;

use glam::{Mat3, Vec3};
use kition_kit::KitCommand;
use kition_kit::tools::{PointerAction, ToolOverlay, ToolPointer, ToolResponse};
use kition_scene_model::curve::{Dimensions, FillMode};
use kition_scene_model::{BezierPoint, ControlPoint, Curve, CurvePrimitive, HandleType, Id, Mesh, ObjectData, Spline, Text, TextAlign};
use kition_scene_raster::{CaretOverlay, CurveOverlay, CurvePointOverlay};
use serde_json::{Value, json};

use crate::params::{self, bad, schema as s};
use crate::{Mode, SceneDocument, Step};

/// A curve or a text in its edit mode.
#[derive(Debug, Clone, PartialEq)]
pub enum ShapeSession {
    Curve(CurveEdit),
    Text(TextEdit),
    Lattice(crate::commands_lattice::LatticeEdit),
}

/// A curve's points being edited: selected parts as `point × 3 + part` (0 the point, 1 its left
/// handle, 2 its right handle), points counted through the splines in order.
#[derive(Debug, Clone, PartialEq)]
pub struct CurveEdit {
    pub object: Id,
    pub selected: BTreeSet<u32>,
    drag: Option<CurveDrag>,
}

#[derive(Debug, Clone)]
struct CurveDrag {
    start: (f64, f64),
    before: Box<Step>,
    original: Curve,
    pivot: Vec3,
    pivot_screen: (f32, f32),
    moved: bool,
    marquee: bool,
}

impl PartialEq for CurveDrag {
    fn eq(&self, other: &Self) -> bool {
        self.start == other.start && self.moved == other.moved
    }
}

/// A text being typed into: the caret is a character index into its body.
#[derive(Debug, Clone, PartialEq)]
pub struct TextEdit {
    pub object: Id,
    pub caret: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Needs {
    ObjectMode,
    /// A curve active (or being edited).
    Curve,
    /// A text active (or being typed into).
    Text,
    /// A curve or a text active.
    Shape,
    /// Curve edit mode.
    CurveEdit,
    /// Curve edit mode with something selected.
    CurveSelection,
    /// Text edit mode.
    TextEdit,
    /// Object mode with a curve or a text selected.
    Convertible,
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

fn none() -> Value {
    s::of(vec![])
}

fn placement(mut fields: Vec<(&'static str, Value)>) -> Value {
    fields.extend([
        ("location", s::vec3("Where to put it, in metres (the origin when left out)")),
        ("rotation", s::vec3("Rotation in degrees, XYZ Euler")),
        ("scale", s::vec3("Scale")),
        ("name", s::string("The object's name")),
    ]);
    s::of(fields)
}

fn target() -> (&'static str, Value) {
    ("object", s::object("The curve or text; the active or edited one when left out"))
}

fn curve_settings() -> Vec<(&'static str, Value)> {
    vec![
        target(),
        ("bevelDepth", s::number("The radius of the round profile swept along the curve, metres")),
        ("bevelResolution", s::integer("Segments in each quarter of the bevel's profile (4)")),
        ("extrude", s::number("Half the height the curve is extruded to, metres")),
        ("offset", s::number("Moves the curve along its normals, metres")),
        ("fillMode", s::choice("3D: full or half profile; 2D: which faces fill a closed curve", &["full", "half", "both", "front", "back", "none"])),
        ("dimensions", s::choice("2d keeps the curve flat and fills closed splines", &["2d", "3d"])),
        ("fillCaps", s::boolean("Close the ends of an open bevelled curve")),
        ("resolution", s::integer("Samples per segment, for every spline (12)")),
        ("cyclic", s::boolean("Close every spline on itself")),
    ]
}

fn text_settings() -> Vec<(&'static str, Value)> {
    vec![
        target(),
        ("body", s::string("The text; a newline starts a new line")),
        ("size", s::number("The text's size (1: the font's full height is one metre)")),
        ("extrude", s::number("Half the thickness the letters are extruded to, metres")),
        ("bevelDepth", s::number("The round edge's radius, metres")),
        ("bevelResolution", s::integer("Segments in the round edge (4)")),
        ("align", s::choice("Horizontal alignment of each line", &["left", "center", "right"])),
        ("lineSpacing", s::number("Line distance as a multiple of the size (1)")),
        ("characterSpacing", s::number("Letter spacing as a multiple of each advance (1)")),
        ("wordSpacing", s::number("Space width as a multiple (1)")),
        ("offsetX", s::number("Moves the text sideways, metres")),
        ("offsetY", s::number("Moves the text up, metres")),
        ("shear", s::number("Slants the letters: x moves by shear × y")),
        ("resolution", s::integer("Points per curved segment of each letter (12)")),
        ("font", s::string("The font's name (only the bundled Inter is available)")),
    ]
}

const ADD_CURVE: &[&str] = &["Add", "Curve"];
const CURVE: &[&str] = &["Curve"];
const TEXT: &[&str] = &["Text"];

static SPECS: &[Spec] = &[
    spec("object.addCurveBezier", "Bezier", ADD_CURVE, None, true, Needs::ObjectMode, || {
        placement(vec![("radius", s::number("Size: the curve spans twice this (1)"))])
    }),
    spec("object.addCurveCircle", "Circle", ADD_CURVE, None, true, Needs::ObjectMode, || {
        placement(vec![("radius", s::number("Radius in metres (1)"))])
    }),
    spec("object.addCurvePath", "Path", ADD_CURVE, None, true, Needs::ObjectMode, || {
        placement(vec![("radius", s::number("Size: the path spans four times this (1)"))])
    }),
    spec("object.addText", "Text", &["Add"], None, true, Needs::ObjectMode, || {
        placement(vec![("body", s::string("The text (\"Text\")")), ("size", s::number("The text's size (1)"))])
    }),
    spec("mode.shape", "Toggle Edit Mode", &["Curve"], Some("Tab"), false, Needs::Shape, none),
    spec("curve.set", "Curve Settings", &[], None, true, Needs::Curve, || s::of(curve_settings())),
    spec("text.set", "Text Settings", &[], None, true, Needs::Text, || s::of(text_settings())),
    spec("object.convertToMesh", "Convert to Mesh", &["Object", "Convert"], None, true, Needs::Convertible, || {
        s::of(vec![("objects", s::objects("The curves and texts; the selected ones when left out"))])
    }),
    spec("curve.setSplines", "Set Splines", &[], None, true, Needs::Curve, || {
        s::of(vec![
            target(),
            (
                "splines",
                s::required(
                    json!({ "type": "array", "items": { "type": "json" }, "description": "Splines as {type: bezier|poly|nurbs, points: [{co: [x,y,z], left?, right?, leftType?, rightType?: auto|vector|aligned|free, radius?, tilt?, weight?}], cyclic?, resolution?, order?, endpoint?}; Bézier handles left out are automatic" }),
                ),
            ),
        ])
    }),
    spec("curve.select", "Select Points", &[], None, false, Needs::CurveEdit, || {
        s::of(vec![
            ("points", s::indices("Control points, counted through the splines in order (handles go with their point)")),
            ("extend", s::boolean("Add to the selection")),
            ("deselect", s::boolean("Take them out of the selection")),
        ])
    }),
    spec("curve.translate", "Move Points", &["Curve", "Transform"], None, true, Needs::CurveSelection, || {
        s::of(vec![("offset", s::required(s::vec3("How far, in the curve's own axes, metres")))])
    }),
    spec("curve.extrude", "Extrude Points", CURVE, Some("E"), true, Needs::CurveSelection, || {
        s::of(vec![("offset", s::vec3("Where the new point goes from the end point; continues the spline when left out"))])
    }),
    spec("curve.subdivide", "Subdivide", CURVE, None, true, Needs::CurveSelection, none),
    spec("curve.toggleCyclic", "Toggle Cyclic", CURVE, Some("Alt+C"), true, Needs::CurveEdit, none),
    spec("curve.setHandleType", "Set Handle Type", CURVE, None, true, Needs::CurveSelection, || {
        s::of(vec![("type", s::required(s::choice("The handles' type", &["auto", "vector", "aligned", "free"])))])
    }),
    spec("curve.deletePoints", "Delete Points", CURVE, None, true, Needs::CurveSelection, none),
    spec("text.insert", "Type Text", TEXT, None, true, Needs::Text, || {
        s::of(vec![target(), ("text", s::required(s::string("What to type at the caret (or the end)")))])
    }),
    spec("text.delete", "Delete Characters", TEXT, None, true, Needs::TextEdit, || {
        s::of(vec![("count", s::integer("How many (1)")), ("forward", s::boolean("Delete after the caret instead of before"))])
    }),
    spec("text.caret", "Move Caret", &[], None, false, Needs::TextEdit, || {
        s::of(vec![
            ("to", s::integer("A character index")),
            ("by", s::integer("Characters to move (negative: back)")),
            ("place", s::choice("A place to go", &["start", "end", "lineStart", "lineEnd", "up", "down"])),
        ])
    }),
];

/// Commands the edit modes take over from object mode.
const TAKEN_OVER: &[&str] = &["edit.selectAll", "edit.deselectAll", "edit.delete"];

fn spec_of(id: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|spec| spec.id == id)
}

fn data_of(doc: &SceneDocument, id: Id) -> Option<&ObjectData> {
    doc.scene.object(id).ok().map(|object| &object.data)
}

fn curve_object(doc: &SceneDocument) -> Option<Id> {
    match &doc.session.shape {
        Some(ShapeSession::Curve(edit)) => Some(edit.object),
        Some(_) => None,
        None => doc.session.editor.active.filter(|id| matches!(data_of(doc, *id), Some(ObjectData::Curve(_)))),
    }
}

fn text_object(doc: &SceneDocument) -> Option<Id> {
    match &doc.session.shape {
        Some(ShapeSession::Text(edit)) => Some(edit.object),
        Some(_) => None,
        None => doc.session.editor.active.filter(|id| matches!(data_of(doc, *id), Some(ObjectData::Text(_)))),
    }
}

fn enabled(doc: &SceneDocument, needs: Needs) -> bool {
    let object_mode = doc.session.mode == Mode::Object && doc.session.rig.is_none() && doc.session.paint.is_none();
    match needs {
        Needs::ObjectMode => object_mode && doc.session.shape.is_none(),
        Needs::Curve => object_mode && curve_object(doc).is_some(),
        Needs::Text => object_mode && text_object(doc).is_some(),
        Needs::Shape => {
            object_mode && (curve_object(doc).is_some() || text_object(doc).is_some() || crate::commands_lattice::active_lattice(doc).is_some())
        }
        Needs::CurveEdit => matches!(doc.session.shape, Some(ShapeSession::Curve(_))),
        Needs::CurveSelection => matches!(&doc.session.shape, Some(ShapeSession::Curve(edit)) if !edit.selected.is_empty()),
        Needs::TextEdit => matches!(doc.session.shape, Some(ShapeSession::Text(_))),
        Needs::Convertible => {
            object_mode
                && doc.session.shape.is_none()
                && doc.session.editor.selected.iter().any(|id| matches!(data_of(doc, *id), Some(ObjectData::Curve(_) | ObjectData::Text(_))))
        }
    }
}

/// Whether the call names what the gate would look for in the active object or the selection: a
/// curve or text command's `object` of its kind, Convert to Mesh's `objects` holding a curve or a
/// text, in object mode. The named object is not made active or selected.
fn names_targets(doc: &SceneDocument, needs: Needs, params: &Value) -> bool {
    let object_mode = doc.session.mode == Mode::Object && doc.session.rig.is_none() && doc.session.paint.is_none() && doc.session.shape.is_none();
    let is = |id: Id, curve: bool, text: bool| match data_of(doc, id) {
        Some(ObjectData::Curve(_)) => curve,
        Some(ObjectData::Text(_)) => text,
        _ => false,
    };
    let named = || params.get("object").filter(|value| !value.is_null()).and_then(|value| params::resolve(&doc.scene, value).ok());
    object_mode
        && match needs {
            Needs::Curve => named().is_some_and(|id| is(id, true, false)),
            Needs::Text => named().is_some_and(|id| is(id, false, true)),
            Needs::Convertible => {
                params::objects(&doc.scene, params, "objects").ok().flatten().is_some_and(|ids| ids.into_iter().any(|id| is(id, true, true)))
            }
            _ => false,
        }
}

/// The curve and text commands, with whether each can run now.
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

/// The curve and text commands that cannot run now.
pub fn disabled(doc: &SceneDocument) -> Vec<String> {
    SPECS.iter().filter(|spec| !enabled(doc, spec.needs)).map(|spec| spec.id.to_string()).collect()
}

/// Runs a curve or text command (or an object-level one an edit mode takes over), or answers
/// `None` when it is neither.
pub fn execute(doc: &mut SceneDocument, command: &str, params: &Value) -> Option<Result<Value, String>> {
    if !params.is_object() && !params.is_null() {
        return spec_of(command).map(|_| Err(bad("the parameters must be an object")));
    }
    if let Some(session) = doc.session.shape.clone()
        && TAKEN_OVER.contains(&command)
    {
        return Some(match (session, command) {
            (ShapeSession::Curve(edit), "edit.selectAll") => {
                select_all(doc, &edit, params::string(params, "action").ok().flatten().unwrap_or("select"))
            }
            (ShapeSession::Curve(edit), "edit.deselectAll") => select_all(doc, &edit, "deselect"),
            (ShapeSession::Curve(_), _) => change(doc, "Delete Points", delete_points),
            (ShapeSession::Text(_), "edit.delete") => change(doc, "Delete Characters", |doc| delete_characters(doc, 1, true)),
            (ShapeSession::Text(_), _) => Ok(json!({ "changed": false })),
            (ShapeSession::Lattice(_), "edit.selectAll") => {
                crate::commands_lattice::select_all(doc, params::string(params, "action").ok().flatten().unwrap_or("select"))
            }
            (ShapeSession::Lattice(_), "edit.deselectAll") => crate::commands_lattice::select_all(doc, "deselect"),
            (ShapeSession::Lattice(_), _) => Ok(json!({ "changed": false })),
        });
    }
    // Leaving the edit modes: any other mode change.
    if doc.session.shape.is_some() && ["mode.set", "mode.toggle", "mode.armature", "mode.pose"].contains(&command) {
        doc.session.shape = None;
        doc.touched();
    }
    if doc.session.shape.is_some() && command.starts_with("object.add") {
        return Some(Err("not-allowed: leave the edit mode to add objects".into()));
    }
    let spec = spec_of(command)?;
    if !(enabled(doc, spec.needs) || names_targets(doc, spec.needs, params)) {
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

/// Runs a change as one undo step, rolling everything back when it fails.
pub(crate) fn change(doc: &mut SceneDocument, label: &str, work: impl FnOnce(&mut SceneDocument) -> Result<Value, String>) -> Result<Value, String> {
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

/// Drops the edit mode when its object is gone or no longer a curve or text, and selected parts
/// that no longer exist.
pub fn tidy(doc: &mut SceneDocument) {
    let Some(session) = doc.session.shape.clone() else { return };
    match session {
        ShapeSession::Curve(edit) => match data_of(doc, edit.object) {
            Some(ObjectData::Curve(curve)) => {
                let parts = u32::try_from(point_count(curve) * 3).unwrap_or(u32::MAX);
                if let Some(ShapeSession::Curve(edit)) = doc.session.shape.as_mut() {
                    edit.selected.retain(|part| *part < parts);
                }
            }
            _ => doc.session.shape = None,
        },
        ShapeSession::Text(edit) => match data_of(doc, edit.object) {
            Some(ObjectData::Text(text)) => {
                let length = text.body.chars().count();
                if let Some(ShapeSession::Text(edit)) = doc.session.shape.as_mut() {
                    edit.caret = edit.caret.min(length);
                }
            }
            _ => doc.session.shape = None,
        },
        ShapeSession::Lattice(_) => crate::commands_lattice::tidy(doc),
    }
}

/// The edited curve's points for the viewport.
pub fn curve_overlay(doc: &SceneDocument) -> Option<CurveOverlay> {
    if let Some(ShapeSession::Lattice(_)) = &doc.session.shape {
        return crate::commands_lattice::overlay(doc);
    }
    let Some(ShapeSession::Curve(edit)) = &doc.session.shape else { return None };
    let Some(ObjectData::Curve(curve)) = data_of(doc, edit.object) else { return None };
    let mut points = Vec::new();
    for (index, (co, handles)) in control_points(curve).into_iter().enumerate() {
        let base = u32::try_from(index * 3).unwrap_or(u32::MAX);
        points.push(CurvePointOverlay {
            co,
            handles,
            selected: [edit.selected.contains(&base), edit.selected.contains(&(base + 1)), edit.selected.contains(&(base + 2))],
        });
    }
    Some(CurveOverlay { object: edit.object, points })
}

/// The text edit caret for the viewport.
pub fn caret_overlay(doc: &SceneDocument) -> Option<CaretOverlay> {
    let Some(ShapeSession::Text(edit)) = &doc.session.shape else { return None };
    let Some(ObjectData::Text(text)) = data_of(doc, edit.object) else { return None };
    Some(CaretOverlay { object: edit.object, at: kition_scene_model::text::caret_position(text, edit.caret), height: text.size * 0.75 })
}

/// What inspect shows of a curve.
pub fn curve_json(curve: &Curve, evaluated: &Mesh) -> Value {
    let splines: Vec<Value> = curve
        .splines
        .iter()
        .map(|spline| {
            let kind = match spline {
                Spline::Bezier { .. } => "bezier",
                Spline::Poly { .. } => "poly",
                Spline::Nurbs { .. } => "nurbs",
            };
            json!({ "type": kind, "points": spline.len(), "cyclic": spline.cyclic() })
        })
        .collect();
    let mut value = serde_json::to_value(curve).unwrap_or(Value::Null);
    if let Some(map) = value.as_object_mut() {
        map.insert("summary".into(), Value::Array(splines));
        map.insert(
            "evaluated".into(),
            json!({ "vertices": evaluated.positions.len(), "edges": evaluated.edges().len(), "faces": evaluated.faces.len() }),
        );
    }
    value
}

/// What inspect shows of a text.
pub fn text_json(text: &Text, evaluated: &Mesh) -> Value {
    let mut value = serde_json::to_value(text).unwrap_or(Value::Null);
    if let Some(map) = value.as_object_mut() {
        map.insert("evaluated".into(), json!({ "vertices": evaluated.positions.len(), "faces": evaluated.faces.len() }));
    }
    value
}

/// The session's state for inspect.
pub fn extend_inspect(doc: &SceneDocument, inspected: &mut Value) {
    let Some(mode) = inspected.get_mut("mode").and_then(Value::as_object_mut) else { return };
    match &doc.session.shape {
        Some(ShapeSession::Curve(edit)) => {
            mode.insert("mode".into(), json!("curveEdit"));
            mode.insert("object".into(), json!(edit.object));
            let points: BTreeSet<u32> = edit.selected.iter().map(|part| part / 3).collect();
            mode.insert("points".into(), json!(points));
        }
        Some(ShapeSession::Text(edit)) => {
            mode.insert("mode".into(), json!("textEdit"));
            mode.insert("object".into(), json!(edit.object));
            mode.insert("caret".into(), json!(edit.caret));
        }
        Some(ShapeSession::Lattice(edit)) => crate::commands_lattice::extend_inspect(edit, mode),
        None => {}
    }
}

fn point_count(curve: &Curve) -> usize {
    curve.splines.iter().map(Spline::len).sum()
}

/// Every control point (and a Bézier point's handles), through the splines in order.
fn control_points(curve: &Curve) -> Vec<(Vec3, Option<(Vec3, Vec3)>)> {
    let mut out = Vec::new();
    for spline in &curve.splines {
        match spline {
            Spline::Bezier { points, .. } => out.extend(points.iter().map(|point| (point.co, Some((point.left, point.right))))),
            Spline::Poly { points, .. } | Spline::Nurbs { points, .. } => out.extend(points.iter().map(|point| (point.co, None))),
        }
    }
    out
}

/// The spline and point a counted point is.
fn locate(curve: &Curve, index: u32) -> Option<(usize, usize)> {
    let mut remaining = index as usize;
    for (spline_index, spline) in curve.splines.iter().enumerate() {
        if remaining < spline.len() {
            return Some((spline_index, remaining));
        }
        remaining -= spline.len();
    }
    None
}

fn first_index(curve: &Curve, spline: usize) -> u32 {
    u32::try_from(curve.splines.iter().take(spline).map(Spline::len).sum::<usize>()).unwrap_or(u32::MAX)
}

fn curve_mut(doc: &mut SceneDocument, id: Id) -> Result<&mut Curve, String> {
    match &mut doc.scene.object_mut(id).map_err(|error| error.to_string())?.data {
        ObjectData::Curve(curve) => Ok(curve),
        _ => Err(bad("that object is not a curve")),
    }
}

fn text_mut(doc: &mut SceneDocument, id: Id) -> Result<&mut Text, String> {
    match &mut doc.scene.object_mut(id).map_err(|error| error.to_string())?.data {
        ObjectData::Text(text) => Ok(text),
        _ => Err(bad("that object is not a text")),
    }
}

fn edited_curve(doc: &SceneDocument) -> Result<CurveEdit, String> {
    match &doc.session.shape {
        Some(ShapeSession::Curve(edit)) => Ok(edit.clone()),
        _ => Err("not-allowed: no curve is in edit mode".into()),
    }
}

fn set_selected(doc: &mut SceneDocument, selected: BTreeSet<u32>) {
    if let Some(ShapeSession::Curve(edit)) = doc.session.shape.as_mut() {
        edit.selected = selected;
    }
}

fn select_all(doc: &mut SceneDocument, edit: &CurveEdit, action: &str) -> Result<Value, String> {
    let Some(ObjectData::Curve(curve)) = data_of(doc, edit.object) else { return Err("not-found: the curve".into()) };
    let all: BTreeSet<u32> =
        (0..u32::try_from(point_count(curve)).unwrap_or(0)).flat_map(|point| [point * 3, point * 3 + 1, point * 3 + 2]).collect();
    let selected = match action {
        "select" => all,
        "deselect" => BTreeSet::new(),
        "invert" => all.difference(&edit.selected).copied().collect(),
        other => return Err(bad(format!("unknown action {other}"))),
    };
    set_selected(doc, selected);
    doc.touched();
    Ok(json!({ "changed": true }))
}

fn degrees(value: Vec3) -> Vec3 {
    Vec3::new(value.x.to_radians(), value.y.to_radians(), value.z.to_radians())
}

pub(crate) fn add(doc: &mut SceneDocument, name: &str, data: ObjectData, params: &Value) -> Result<Value, String> {
    let location = params::vec3(params, "location")?;
    let rotation = params::vec3(params, "rotation")?;
    let scale = params::vec3(params, "scale")?;
    let name = params::string(params, "name")?.unwrap_or(name);
    let id = doc.scene.add_object(name, data);
    let object = doc.scene.object_mut(id).map_err(|error| error.to_string())?;
    if let Some(location) = location {
        object.location = location;
    }
    if let Some(rotation) = rotation {
        object.rotation = degrees(rotation);
    }
    if let Some(scale) = scale {
        object.scale = scale;
    }
    let name = object.name.clone();
    doc.session.editor.selected = vec![id];
    doc.session.editor.active = Some(id);
    Ok(json!({ "object": id, "name": name }))
}

fn handle_type(name: &str) -> Result<HandleType, String> {
    match name {
        "auto" => Ok(HandleType::Auto),
        "vector" => Ok(HandleType::Vector),
        "aligned" => Ok(HandleType::Aligned),
        "free" => Ok(HandleType::Free),
        other => Err(bad(format!("unknown handle type {other}"))),
    }
}

/// Splines from an agent's JSON: Bézier handles left out are automatic.
fn parse_splines(value: &Value) -> Result<Vec<Spline>, String> {
    let items = value.as_array().ok_or_else(|| bad("`splines` is a list"))?;
    if items.len() > 1000 {
        return Err(bad("too many splines"));
    }
    let mut out = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let kind = item.get("type").and_then(Value::as_str).unwrap_or("bezier");
        let points = item.get("points").and_then(Value::as_array).ok_or_else(|| bad(format!("spline {index} has no points")))?;
        if points.is_empty() || points.len() > 100_000 {
            return Err(bad(format!("spline {index} needs between 1 and 100000 points")));
        }
        let cyclic = item.get("cyclic").and_then(Value::as_bool).unwrap_or(false);
        let resolution = params::integer(item, "resolution")?.unwrap_or(12);
        let co = |point: &Value, key: &str| params::vec3(point, key);
        let spline = match kind {
            "bezier" => {
                let mut list = Vec::with_capacity(points.len());
                for point in points {
                    let position = co(point, "co")?.ok_or_else(|| bad(format!("a point of spline {index} has no `co`")))?;
                    let mut made = BezierPoint::auto(position);
                    if let Some(left) = co(point, "left")? {
                        made.left = left;
                        made.left_type = HandleType::Free;
                    }
                    if let Some(right) = co(point, "right")? {
                        made.right = right;
                        made.right_type = HandleType::Free;
                    }
                    if let Some(name) = params::string(point, "leftType")? {
                        made.left_type = handle_type(name)?;
                    }
                    if let Some(name) = params::string(point, "rightType")? {
                        made.right_type = handle_type(name)?;
                    }
                    made.radius = params::number(point, "radius")?.unwrap_or(1.0);
                    made.tilt = params::number(point, "tilt")?.unwrap_or(0.0).to_radians();
                    list.push(made);
                }
                Spline::Bezier { points: list, cyclic, resolution }
            }
            "poly" | "nurbs" => {
                let mut list = Vec::with_capacity(points.len());
                for point in points {
                    let position = co(point, "co")?.ok_or_else(|| bad(format!("a point of spline {index} has no `co`")))?;
                    let mut made = ControlPoint::at(position);
                    made.weight = params::number(point, "weight")?.unwrap_or(1.0);
                    made.radius = params::number(point, "radius")?.unwrap_or(1.0);
                    made.tilt = params::number(point, "tilt")?.unwrap_or(0.0).to_radians();
                    list.push(made);
                }
                if kind == "poly" {
                    Spline::Poly { points: list, cyclic }
                } else {
                    Spline::Nurbs {
                        points: list,
                        cyclic,
                        order: params::integer(item, "order")?.unwrap_or(4),
                        endpoint: item.get("endpoint").and_then(Value::as_bool).unwrap_or(true),
                        resolution,
                    }
                }
            }
            other => return Err(bad(format!("unknown spline type {other}"))),
        };
        out.push(spline);
    }
    Ok(out)
}

fn target_id(doc: &SceneDocument, params: &Value, curve: bool) -> Result<Id, String> {
    if let Some(value) = params.get("object").filter(|value| !value.is_null()) {
        return params::resolve(&doc.scene, value);
    }
    let found = if curve { curve_object(doc) } else { text_object(doc) };
    found.ok_or_else(|| format!("not-allowed: there is no active {}", if curve { "curve" } else { "text" }))
}

fn run_view(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    match command {
        "mode.shape" => {
            if doc.session.shape.take().is_some() {
                return Ok(json!({ "mode": "object" }));
            }
            let id = doc.session.editor.active.ok_or("not-allowed: there is no active curve, text, or lattice")?;
            match data_of(doc, id) {
                Some(ObjectData::Curve(_)) => {
                    doc.session.shape = Some(ShapeSession::Curve(CurveEdit { object: id, selected: BTreeSet::new(), drag: None }));
                    Ok(json!({ "mode": "curveEdit" }))
                }
                Some(ObjectData::Text(text)) => {
                    let caret = text.body.chars().count();
                    doc.session.shape = Some(ShapeSession::Text(TextEdit { object: id, caret }));
                    Ok(json!({ "mode": "textEdit" }))
                }
                Some(ObjectData::Lattice(_)) => {
                    doc.session.shape = Some(ShapeSession::Lattice(crate::commands_lattice::LatticeEdit::new(id)));
                    Ok(json!({ "mode": "latticeEdit" }))
                }
                _ => Err(bad("the active object is not a curve, a text, or a lattice")),
            }
        }
        "curve.select" => {
            let edit = edited_curve(doc)?;
            let Some(ObjectData::Curve(curve)) = data_of(doc, edit.object) else { return Err("not-found: the curve".into()) };
            let count = u32::try_from(point_count(curve)).unwrap_or(0);
            let mut chosen = BTreeSet::new();
            for point in params::indices(params, "points")?.unwrap_or_default() {
                if point >= count {
                    return Err(format!("not-found: point {point}"));
                }
                chosen.extend([point * 3, point * 3 + 1, point * 3 + 2]);
            }
            let selected = if params::boolean(params, "deselect")?.unwrap_or(false) {
                edit.selected.difference(&chosen).copied().collect()
            } else if params::boolean(params, "extend")?.unwrap_or(false) {
                edit.selected.union(&chosen).copied().collect()
            } else {
                chosen
            };
            set_selected(doc, selected);
            Ok(json!({ "changed": true }))
        }
        "text.caret" => {
            let Some(ShapeSession::Text(edit)) = doc.session.shape.clone() else { return Err("not-allowed: no text is being typed into".into()) };
            let Some(ObjectData::Text(text)) = data_of(doc, edit.object) else { return Err("not-found: the text".into()) };
            let caret = move_caret(text, edit.caret, params)?;
            if let Some(ShapeSession::Text(edit)) = doc.session.shape.as_mut() {
                edit.caret = caret;
            }
            Ok(json!({ "caret": caret }))
        }
        other => Err(format!("unknown-command: {other}")),
    }
}

fn move_caret(text: &Text, caret: usize, params: &Value) -> Result<usize, String> {
    let characters: Vec<char> = text.body.chars().collect();
    let length = characters.len();
    if let Some(to) = params::integer(params, "to")? {
        return Ok((to as usize).min(length));
    }
    if let Some(by) = params.get("by").and_then(Value::as_i64) {
        return Ok(usize::try_from(i64::try_from(caret).unwrap_or(0).saturating_add(by).clamp(0, i64::try_from(length).unwrap_or(0))).unwrap_or(0));
    }
    // Lines as (start, end) character ranges.
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, character) in characters.iter().enumerate() {
        if *character == '\n' {
            lines.push((start, index));
            start = index + 1;
        }
    }
    lines.push((start, length));
    let line = lines.iter().position(|(from, to)| caret >= *from && caret <= *to).unwrap_or(0);
    let (from, to) = lines.get(line).copied().unwrap_or((0, length));
    Ok(match params::string(params, "place")?.unwrap_or("end") {
        "start" => 0,
        "end" => length,
        "lineStart" => from,
        "lineEnd" => to,
        "up" | "down" => {
            let column = caret - from;
            let target =
                if params::string(params, "place")? == Some("up") { line.checked_sub(1) } else { (line + 1 < lines.len()).then_some(line + 1) };
            match target.and_then(|target| lines.get(target)) {
                Some((start, end)) => (*start + column).min(*end),
                None => caret,
            }
        }
        other => return Err(bad(format!("unknown place {other}"))),
    })
}

fn insert_text(doc: &mut SceneDocument, id: Id, typed: &str) -> Result<Value, String> {
    if typed.contains('\0') {
        return Err(bad("the text cannot hold a null character"));
    }
    let caret = match &doc.session.shape {
        Some(ShapeSession::Text(edit)) if edit.object == id => Some(edit.caret),
        _ => None,
    };
    let text = text_mut(doc, id)?;
    let mut characters: Vec<char> = text.body.chars().collect();
    let at = caret.unwrap_or(characters.len()).min(characters.len());
    let added: Vec<char> = typed.chars().filter(|character| *character == '\n' || !character.is_control()).collect();
    if characters.len() + added.len() > kition_scene_model::text::MAX_LENGTH {
        return Err(bad("the text would be too long"));
    }
    let count = added.len();
    characters.splice(at..at, added);
    text.body = characters.into_iter().collect();
    if let Some(ShapeSession::Text(edit)) = doc.session.shape.as_mut()
        && edit.object == id
    {
        edit.caret = at + count;
    }
    Ok(json!({ "object": id, "caret": at + count }))
}

fn delete_characters(doc: &mut SceneDocument, count: usize, forward: bool) -> Result<Value, String> {
    let Some(ShapeSession::Text(edit)) = doc.session.shape.clone() else { return Err("not-allowed: no text is being typed into".into()) };
    let text = text_mut(doc, edit.object)?;
    let mut characters: Vec<char> = text.body.chars().collect();
    let caret = edit.caret.min(characters.len());
    let (from, to) = if forward { (caret, (caret + count).min(characters.len())) } else { (caret.saturating_sub(count), caret) };
    if from == to {
        return Err("not-allowed: there is nothing to delete there".into());
    }
    characters.drain(from..to);
    text.body = characters.into_iter().collect();
    if let Some(ShapeSession::Text(edit)) = doc.session.shape.as_mut() {
        edit.caret = from;
    }
    Ok(json!({ "caret": from }))
}

fn apply_curve_settings(curve: &mut Curve, params: &Value) -> Result<(), String> {
    if let Some(value) = params::number(params, "bevelDepth")? {
        curve.bevel_depth = value.max(0.0);
    }
    if let Some(value) = params::integer(params, "bevelResolution")? {
        curve.bevel_resolution = value.min(64);
    }
    if let Some(value) = params::number(params, "extrude")? {
        curve.extrude = value.max(0.0);
    }
    if let Some(value) = params::number(params, "offset")? {
        curve.offset = value;
    }
    if let Some(value) = params::string(params, "fillMode")? {
        curve.fill_mode = match value {
            "full" => FillMode::Full,
            "half" => FillMode::Half,
            "both" => FillMode::Both,
            "front" => FillMode::Front,
            "back" => FillMode::Back,
            "none" => FillMode::None,
            other => return Err(bad(format!("unknown fill mode {other}"))),
        };
    }
    if let Some(value) = params::string(params, "dimensions")? {
        let flat = match value {
            "2d" => true,
            "3d" => false,
            other => return Err(bad(format!("dimensions are 2d or 3d, not {other}"))),
        };
        curve.dimensions = if flat { Dimensions::Flat } else { Dimensions::Spatial };
        // A fill mode of the other kind turns into the new kind's default.
        let spatial_mode = matches!(curve.fill_mode, FillMode::Full | FillMode::Half);
        if flat && spatial_mode {
            curve.fill_mode = FillMode::Both;
        } else if !flat && !spatial_mode {
            curve.fill_mode = FillMode::Full;
        }
    }
    if let Some(value) = params::boolean(params, "fillCaps")? {
        curve.fill_caps = value;
    }
    if let Some(value) = params::integer(params, "resolution")? {
        if !(1..=1024).contains(&value) {
            return Err(bad("`resolution` must be between 1 and 1024"));
        }
        curve.resolution = value;
        for spline in &mut curve.splines {
            if let Spline::Bezier { resolution, .. } | Spline::Nurbs { resolution, .. } = spline {
                *resolution = value;
            }
        }
    }
    if let Some(value) = params::boolean(params, "cyclic")? {
        for spline in &mut curve.splines {
            spline.set_cyclic(value);
        }
    }
    curve.validate().map_err(|error| bad(error.to_string().replace("invalid-file: ", "")))
}

fn apply_text_settings(text: &mut Text, params: &Value) -> Result<(), String> {
    if let Some(value) = params::string(params, "body")? {
        if value.chars().count() > kition_scene_model::text::MAX_LENGTH {
            return Err(bad("the text is too long"));
        }
        text.body = value.chars().filter(|character| *character == '\n' || !character.is_control()).collect();
    }
    for (key, slot) in [
        ("size", &mut text.size),
        ("extrude", &mut text.extrude),
        ("bevelDepth", &mut text.bevel_depth),
        ("lineSpacing", &mut text.line_spacing),
        ("characterSpacing", &mut text.character_spacing),
        ("wordSpacing", &mut text.word_spacing),
        ("offsetX", &mut text.offset_x),
        ("offsetY", &mut text.offset_y),
        ("shear", &mut text.shear),
    ] {
        if let Some(value) = params::number(params, key)? {
            *slot = value;
        }
    }
    if let Some(value) = params::integer(params, "bevelResolution")? {
        text.bevel_resolution = value;
    }
    if let Some(value) = params::integer(params, "resolution")? {
        text.resolution = value;
    }
    if let Some(value) = params::string(params, "align")? {
        text.align = match value {
            "left" => TextAlign::Left,
            "center" => TextAlign::Center,
            "right" => TextAlign::Right,
            other => return Err(bad(format!("unknown alignment {other}"))),
        };
    }
    if let Some(value) = params::string(params, "font")? {
        text.font = value.to_string();
    }
    text.validate().map_err(|error| bad(error.to_string().replace("invalid-file: ", "")))
}

fn run_change(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    match command {
        "object.addCurveBezier" | "object.addCurveCircle" | "object.addCurvePath" => {
            let primitive = match command {
                "object.addCurveBezier" => CurvePrimitive::Bezier,
                "object.addCurveCircle" => CurvePrimitive::Circle,
                _ => CurvePrimitive::Path,
            };
            let radius = params::number(params, "radius")?.unwrap_or(1.0);
            if radius.is_nan() || radius <= 0.0 {
                return Err(bad("`radius` must be positive"));
            }
            add(doc, primitive.display_name(), ObjectData::Curve(primitive.build(radius)), params)
        }
        "object.addText" => {
            let mut text = Text::default();
            apply_text_settings(&mut text, params)?;
            add(doc, "Text", ObjectData::Text(text), params)
        }
        "curve.set" => {
            let id = target_id(doc, params, true)?;
            apply_curve_settings(curve_mut(doc, id)?, params)?;
            Ok(json!({ "object": id }))
        }
        "text.set" => {
            let id = target_id(doc, params, false)?;
            apply_text_settings(text_mut(doc, id)?, params)?;
            tidy(doc);
            Ok(json!({ "object": id }))
        }
        "object.convertToMesh" => {
            let ids = match params::objects(&doc.scene, params, "objects")? {
                Some(ids) => ids,
                None => doc.session.editor.selected.clone(),
            };
            let mut converted = Vec::new();
            for id in ids {
                let scene = doc.scene.clone();
                let object = doc.scene.object(id).map_err(|error| error.to_string())?;
                if !matches!(object.data, ObjectData::Curve(_) | ObjectData::Text(_)) {
                    continue;
                }
                // The mesh as it shows: with its modifiers, which it no longer needs.
                let mesh = object.evaluated_mesh(&scene).unwrap_or_default();
                let object = doc.scene.object_mut(id).map_err(|error| error.to_string())?;
                object.data = ObjectData::Mesh(mesh);
                object.modifiers.clear();
                converted.push(id);
            }
            if converted.is_empty() {
                return Err(bad("select curves or texts to convert"));
            }
            Ok(json!({ "objects": converted }))
        }
        "curve.setSplines" => {
            let id = target_id(doc, params, true)?;
            let splines = parse_splines(params.get("splines").ok_or_else(|| bad("`splines` is required"))?)?;
            let curve = curve_mut(doc, id)?;
            curve.splines = splines;
            curve.recalculate_handles();
            curve.validate().map_err(|error| bad(error.to_string().replace("invalid-file: ", "")))?;
            Ok(json!({ "object": id, "points": point_count(curve) }))
        }
        "curve.translate" => {
            let offset = params::vec3(params, "offset")?.ok_or_else(|| bad("`offset` is required"))?;
            let edit = edited_curve(doc)?;
            let curve = curve_mut(doc, edit.object)?;
            transform_parts(curve, &edit.selected, |point| point + offset);
            Ok(json!({ "moved": edit.selected.len() }))
        }
        "curve.extrude" => {
            let offset = params::vec3(params, "offset")?;
            let edit = edited_curve(doc)?;
            let curve = curve_mut(doc, edit.object)?;
            let made = extrude_points(curve, &edit.selected, offset)?;
            set_selected(doc, made.iter().flat_map(|point| [point * 3, point * 3 + 1, point * 3 + 2]).collect());
            Ok(json!({ "points": made }))
        }
        "curve.subdivide" => {
            let edit = edited_curve(doc)?;
            let curve = curve_mut(doc, edit.object)?;
            let added = subdivide(curve, &edit.selected)?;
            set_selected(doc, BTreeSet::new());
            Ok(json!({ "added": added }))
        }
        "curve.toggleCyclic" => {
            let edit = edited_curve(doc)?;
            let curve = curve_mut(doc, edit.object)?;
            let chosen: BTreeSet<usize> = edit.selected.iter().filter_map(|part| locate(curve, part / 3)).map(|(spline, _)| spline).collect();
            let all = chosen.is_empty();
            for (index, spline) in curve.splines.iter_mut().enumerate() {
                if all || chosen.contains(&index) {
                    let cyclic = spline.cyclic();
                    spline.set_cyclic(!cyclic);
                }
            }
            curve.recalculate_handles();
            Ok(json!({ "changed": true }))
        }
        "curve.setHandleType" => {
            let kind = handle_type(params::string(params, "type")?.ok_or_else(|| bad("`type` is required"))?)?;
            let edit = edited_curve(doc)?;
            let curve = curve_mut(doc, edit.object)?;
            for part in &edit.selected {
                let Some((spline, point)) = locate(curve, part / 3) else { continue };
                if let Some(Spline::Bezier { points, .. }) = curve.splines.get_mut(spline)
                    && let Some(point) = points.get_mut(point)
                {
                    match part % 3 {
                        1 => point.left_type = kind,
                        2 => point.right_type = kind,
                        _ => {
                            point.left_type = kind;
                            point.right_type = kind;
                        }
                    }
                }
            }
            curve.recalculate_handles();
            Ok(json!({ "changed": true }))
        }
        "curve.deletePoints" => delete_points(doc),
        "text.insert" => {
            let id = target_id(doc, params, false)?;
            let typed = params::string(params, "text")?.ok_or_else(|| bad("`text` is required"))?.to_string();
            insert_text(doc, id, &typed)
        }
        "text.delete" => {
            let count = params::integer(params, "count")?.unwrap_or(1) as usize;
            let forward = params::boolean(params, "forward")?.unwrap_or(false);
            delete_characters(doc, count.max(1), forward)
        }
        other => Err(format!("unknown-command: {other}")),
    }
}

/// Moves the selected points (their handles along) and selected handles by `change`; a moved
/// handle stops being automatic, and an aligned partner turns to stay in line.
fn transform_parts(curve: &mut Curve, selected: &BTreeSet<u32>, change: impl Fn(Vec3) -> Vec3) {
    let points: BTreeSet<u32> = selected.iter().filter(|part| *part % 3 == 0).map(|part| part / 3).collect();
    let mut index = 0u32;
    for spline in &mut curve.splines {
        match spline {
            Spline::Bezier { points: list, .. } => {
                for point in list.iter_mut() {
                    if points.contains(&index) {
                        point.co = change(point.co);
                        point.left = change(point.left);
                        point.right = change(point.right);
                    } else {
                        for (part, left) in [(1u32, true), (2u32, false)] {
                            if !selected.contains(&(index * 3 + part)) {
                                continue;
                            }
                            let (moved, kind) =
                                if left { (&mut point.left, &mut point.left_type) } else { (&mut point.right, &mut point.right_type) };
                            *moved = change(*moved);
                            *kind = match *kind {
                                HandleType::Auto => HandleType::Aligned,
                                HandleType::Vector => HandleType::Free,
                                other => other,
                            };
                            // The partner of an aligned handle turns to stay in line.
                            let (handle, partner, partner_kind) =
                                if left { (point.left, &mut point.right, point.right_type) } else { (point.right, &mut point.left, point.left_type) };
                            if matches!(partner_kind, HandleType::Aligned | HandleType::Auto) {
                                let along = (point.co - handle).normalize_or_zero();
                                if along != Vec3::ZERO {
                                    *partner = point.co + along * (*partner - point.co).length();
                                }
                            }
                            if left && point.right_type == HandleType::Auto {
                                point.right_type = HandleType::Aligned;
                            } else if !left && point.left_type == HandleType::Auto {
                                point.left_type = HandleType::Aligned;
                            }
                        }
                    }
                    index += 1;
                }
            }
            Spline::Poly { points: list, .. } | Spline::Nurbs { points: list, .. } => {
                for point in list.iter_mut() {
                    if points.contains(&index) {
                        point.co = change(point.co);
                    }
                    index += 1;
                }
            }
        }
    }
    // Automatic and vector handles follow the points that moved.
    curve.recalculate_handles();
}

/// Adds a point beyond each selected end point of an open spline; answers the new points.
fn extrude_points(curve: &mut Curve, selected: &BTreeSet<u32>, offset: Option<Vec3>) -> Result<Vec<u32>, String> {
    let chosen: BTreeSet<u32> = selected.iter().map(|part| part / 3).collect();
    let mut made = Vec::new();
    // Work from the last spline back so earlier counts stay right while points are added.
    for spline_index in (0..curve.splines.len()).rev() {
        let first = first_index(curve, spline_index);
        let Some(spline) = curve.splines.get_mut(spline_index) else { continue };
        if spline.cyclic() {
            continue;
        }
        let count = spline.len();
        let positions = spline.positions();
        let ends = [
            (0usize, chosen.contains(&first)),
            (count.saturating_sub(1), count > 1 && chosen.contains(&(first + u32::try_from(count - 1).unwrap_or(0)))),
        ];
        for (end, picked) in ends.into_iter().rev() {
            if !picked {
                continue;
            }
            let here = positions.get(end).copied().unwrap_or(Vec3::ZERO);
            let neighbour = if end == 0 { positions.get(1) } else { positions.get(end.saturating_sub(1)) }.copied().unwrap_or(here - Vec3::X);
            let step = offset.unwrap_or(here - neighbour);
            let co = here + step;
            match spline {
                Spline::Bezier { points, .. } => {
                    let point = BezierPoint::auto(co);
                    if end == 0 { points.insert(0, point) } else { points.push(point) }
                }
                Spline::Poly { points, .. } | Spline::Nurbs { points, .. } => {
                    let point = ControlPoint::at(co);
                    if end == 0 { points.insert(0, point) } else { points.push(point) }
                }
            }
            made.push((spline_index, end == 0));
        }
    }
    if made.is_empty() {
        return Err("not-allowed: select an end point of an open spline to extrude".into());
    }
    curve.recalculate_handles();
    // The new points' counts, now that every insertion is done.
    let mut out = Vec::new();
    for (spline_index, at_start) in made {
        let first = first_index(curve, spline_index);
        let length = curve.splines.get(spline_index).map_or(0, Spline::len);
        out.push(if at_start { first } else { first + u32::try_from(length.saturating_sub(1)).unwrap_or(0) });
    }
    out.sort_unstable();
    Ok(out)
}

/// Adds a point in the middle of each segment between two selected neighbours; answers how many.
fn subdivide(curve: &mut Curve, selected: &BTreeSet<u32>) -> Result<usize, String> {
    let chosen: BTreeSet<u32> = selected.iter().map(|part| part / 3).collect();
    let mut added = 0;
    for spline_index in 0..curve.splines.len() {
        let first = first_index(curve, spline_index);
        let Some(spline) = curve.splines.get_mut(spline_index) else { continue };
        let count = spline.len();
        let cyclic = spline.cyclic();
        let segments: Vec<usize> = (0..if cyclic { count } else { count.saturating_sub(1) })
            .filter(|segment| {
                let a = first + u32::try_from(*segment).unwrap_or(0);
                let b = first + u32::try_from((segment + 1) % count.max(1)).unwrap_or(0);
                chosen.contains(&a) && chosen.contains(&b)
            })
            .collect();
        for segment in segments.into_iter().rev() {
            let next = (segment + 1) % count;
            match spline {
                Spline::Bezier { points, .. } => {
                    let (Some(a), Some(b)) = (points.get(segment).cloned(), points.get(next).cloned()) else { continue };
                    // de Casteljau at the middle keeps the curve's shape.
                    let (p0, p1, p2, p3) = (a.co, a.right, b.left, b.co);
                    let (q0, q1, q2) = ((p0 + p1) * 0.5, (p1 + p2) * 0.5, (p2 + p3) * 0.5);
                    let (r0, r1) = ((q0 + q1) * 0.5, (q1 + q2) * 0.5);
                    let middle = (r0 + r1) * 0.5;
                    if let Some(point) = points.get_mut(segment) {
                        point.right = q0;
                        if point.right_type == HandleType::Auto {
                            point.right_type = HandleType::Aligned;
                        }
                    }
                    if let Some(point) = points.get_mut(next) {
                        point.left = q2;
                        if point.left_type == HandleType::Auto {
                            point.left_type = HandleType::Aligned;
                        }
                    }
                    let point = BezierPoint {
                        co: middle,
                        left: r0,
                        right: r1,
                        left_type: HandleType::Aligned,
                        right_type: HandleType::Aligned,
                        radius: (a.radius + b.radius) / 2.0,
                        tilt: (a.tilt + b.tilt) / 2.0,
                    };
                    points.insert(segment + 1, point);
                }
                Spline::Poly { points, .. } | Spline::Nurbs { points, .. } => {
                    let (Some(a), Some(b)) = (points.get(segment).cloned(), points.get(next).cloned()) else { continue };
                    points.insert(
                        segment + 1,
                        ControlPoint {
                            co: (a.co + b.co) * 0.5,
                            weight: (a.weight + b.weight) / 2.0,
                            radius: (a.radius + b.radius) / 2.0,
                            tilt: (a.tilt + b.tilt) / 2.0,
                        },
                    );
                }
            }
            added += 1;
        }
    }
    if added == 0 {
        return Err("not-allowed: select two neighbouring points to subdivide between".into());
    }
    curve.recalculate_handles();
    Ok(added)
}

fn delete_points(doc: &mut SceneDocument) -> Result<Value, String> {
    let edit = edited_curve(doc)?;
    let chosen: BTreeSet<u32> = edit.selected.iter().filter(|part| *part % 3 == 0).map(|part| part / 3).collect();
    if chosen.is_empty() {
        return Err("not-allowed: select points to delete".into());
    }
    let curve = curve_mut(doc, edit.object)?;
    let mut index = 0u32;
    for spline in &mut curve.splines {
        match spline {
            Spline::Bezier { points, .. } => {
                let mut slot = 0;
                points.retain(|_| {
                    let keep = !chosen.contains(&(index + slot));
                    slot += 1;
                    keep
                });
                index += slot;
            }
            Spline::Poly { points, .. } | Spline::Nurbs { points, .. } => {
                let mut slot = 0;
                points.retain(|_| {
                    let keep = !chosen.contains(&(index + slot));
                    slot += 1;
                    keep
                });
                index += slot;
            }
        }
    }
    curve.splines.retain(|spline| !spline.is_empty());
    // A NURBS spline cannot keep an order higher than its points.
    for spline in &mut curve.splines {
        if let Spline::Nurbs { points, order, cyclic, .. } = spline {
            let count = u32::try_from(points.len()).unwrap_or(0);
            if count < 2 {
                *cyclic = false;
            }
            *order = (*order).min(count.max(2));
        }
        if spline.len() < 3 {
            spline.set_cyclic(false);
        }
    }
    curve.recalculate_handles();
    set_selected(doc, BTreeSet::new());
    Ok(json!({ "deleted": chosen.len() }))
}

pub(crate) fn projector(doc: &SceneDocument) -> kition_scene_raster::Projector {
    doc.session.editor.view.projector(&doc.scene, doc.session.width, doc.session.height)
}

pub(crate) fn to_screen(projector: &kition_scene_raster::Projector, point: Vec3) -> Option<(f32, f32)> {
    let clip = projector.view_projection() * point.extend(1.0);
    if clip.w <= 0.0 {
        return None;
    }
    let screen = projector.to_screen(clip);
    Some((screen.x, screen.y))
}

/// The selectable parts under (or in the box to) a pixel: `point × 3 + part`.
fn parts_at(doc: &SceneDocument, edit: &CurveEdit, start: (f64, f64), end: (f64, f64)) -> BTreeSet<u32> {
    let Some(ObjectData::Curve(curve)) = data_of(doc, edit.object) else { return BTreeSet::new() };
    let Ok(world) = doc.scene.world_matrix(edit.object) else { return BTreeSet::new() };
    let projector = projector(doc);
    let boxed = (end.0 - start.0).abs() > 4.0 || (end.1 - start.1).abs() > 4.0;
    let (left, right, top, bottom) = (start.0.min(end.0) as f32, start.0.max(end.0) as f32, start.1.min(end.1) as f32, start.1.max(end.1) as f32);
    let mut found = BTreeSet::new();
    let mut nearest: Option<(f32, u32)> = None;
    for (index, (co, handles)) in control_points(curve).into_iter().enumerate() {
        let base = u32::try_from(index * 3).unwrap_or(u32::MAX);
        let mut parts = vec![(co, base)];
        if let Some((left_handle, right_handle)) = handles {
            parts.push((left_handle, base + 1));
            parts.push((right_handle, base + 2));
        }
        for (position, part) in parts {
            let Some((x, y)) = to_screen(&projector, world.transform_point3(position)) else { continue };
            if boxed {
                // A box selects whole points.
                if part % 3 == 0 && x >= left && x <= right && y >= top && y <= bottom {
                    found.extend([part, part + 1, part + 2]);
                }
            } else {
                let distance = (x - end.0 as f32).hypot(y - end.1 as f32);
                if distance <= 12.0 && nearest.is_none_or(|(best, _)| distance < best) {
                    nearest = Some((distance, part));
                }
            }
        }
    }
    if let Some((_, part)) = nearest {
        // A point brings its handles; a handle comes alone.
        if part % 3 == 0 {
            found.extend([part, part + 1, part + 2]);
        } else {
            found.insert(part);
        }
    }
    found
}

fn pivot(doc: &SceneDocument, edit: &CurveEdit) -> Option<Vec3> {
    let Some(ObjectData::Curve(curve)) = data_of(doc, edit.object) else { return None };
    let world = doc.scene.world_matrix(edit.object).ok()?;
    let mut sum = Vec3::ZERO;
    let mut count = 0.0;
    for (index, (co, handles)) in control_points(curve).into_iter().enumerate() {
        let base = u32::try_from(index * 3).unwrap_or(u32::MAX);
        for (position, part) in [(Some(co), base), (handles.map(|pair| pair.0), base + 1), (handles.map(|pair| pair.1), base + 2)] {
            if let Some(position) = position
                && edit.selected.contains(&part)
                && (part % 3 == 0 || !edit.selected.contains(&(part - part % 3)))
            {
                sum += world.transform_point3(position);
                count += 1.0;
            }
        }
    }
    (count > 0.0).then(|| sum / count)
}

/// The viewport tools in curve edit mode (select, move, rotate, scale on points and handles);
/// `None` when no curve is being edited, so the other modes get the event.
pub fn pointer(doc: &mut SceneDocument, tool: &str, event: &ToolPointer) -> Option<ToolResponse> {
    let edit = match &doc.session.shape {
        Some(ShapeSession::Curve(edit)) => edit.clone(),
        Some(ShapeSession::Text(_)) => return Some(ToolResponse { cursor: "text".into(), wants_text: true, ..Default::default() }),
        Some(ShapeSession::Lattice(_)) => return Some(crate::commands_lattice::pointer(doc, tool, event)),
        None => return None,
    };
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
            let selecting = !["move", "rotate", "scale"].contains(&tool);
            if !selecting && edit.selected.is_empty() {
                let parts = parts_at(doc, &edit, at, at);
                set_selected(doc, parts);
            }
            let Some(ShapeSession::Curve(edit)) = doc.session.shape.clone() else { return Some(idle(false)) };
            let Some(ObjectData::Curve(curve)) = data_of(doc, edit.object).cloned() else { return Some(idle(false)) };
            let pivot = pivot(doc, &edit).unwrap_or(Vec3::ZERO);
            let pivot_screen = to_screen(&projector(doc), pivot).unwrap_or((0.0, 0.0));
            let drag = CurveDrag {
                start: at,
                before: Box::new(doc.step(if selecting { "Select" } else { "Transform Points" })),
                original: curve,
                pivot,
                pivot_screen,
                moved: false,
                marquee: selecting,
            };
            if let Some(ShapeSession::Curve(edit)) = doc.session.shape.as_mut() {
                edit.drag = Some(drag);
            }
            doc.touched();
            Some(idle(true))
        }
        PointerAction::Drag => {
            let Some(ShapeSession::Curve(edit)) = doc.session.shape.clone() else { return Some(idle(false)) };
            let Some(mut drag) = edit.drag.clone() else { return Some(idle(false)) };
            drag.moved = true;
            if drag.marquee {
                if let Some(ShapeSession::Curve(edit)) = doc.session.shape.as_mut() {
                    edit.drag = Some(drag.clone());
                }
                let (x0, y0) = drag.start;
                return Some(ToolResponse {
                    overlays: vec![ToolOverlay::Marquee { x0, y0, x1: at.0, y1: at.1 }],
                    cursor: cursor.into(),
                    busy: true,
                    ..Default::default()
                });
            }
            let Ok(world) = doc.scene.world_matrix(edit.object) else { return Some(idle(true)) };
            let inverse = world.inverse();
            let projector = projector(doc);
            let fine = if event.mods.shift { 0.1 } else { 1.0 };
            let (sx, sy) = drag.pivot_screen;
            let (dx, dy) = ((at.0 - drag.start.0) as f32 * fine, (at.1 - drag.start.1) as f32 * fine);
            let depth = projector.to_screen(projector.view_projection() * drag.pivot.extend(1.0)).z;
            let pivot = drag.pivot;
            let world_change: Box<dyn Fn(Vec3) -> Vec3> = match tool {
                "move" => {
                    let delta = projector.unproject(sx + dx, sy + dy, depth) - projector.unproject(sx, sy, depth);
                    Box::new(move |point| point + delta)
                }
                "rotate" => {
                    let angle = -(((at.1 as f32 - sy).atan2(at.0 as f32 - sx)) - ((drag.start.1 as f32 - sy).atan2(drag.start.0 as f32 - sx))) * fine;
                    let axis = (projector.eye() - pivot).normalize_or_zero();
                    let rotation = Mat3::from_axis_angle(if axis == Vec3::ZERO { Vec3::Z } else { axis }, angle);
                    Box::new(move |point| pivot + rotation * (point - pivot))
                }
                _ => {
                    let from = ((drag.start.0 as f32 - sx).hypot(drag.start.1 as f32 - sy)).max(1.0);
                    let factor = 1.0 + ((at.0 as f32 - sx).hypot(at.1 as f32 - sy) / from - 1.0) * fine;
                    Box::new(move |point| pivot + (point - pivot) * factor)
                }
            };
            let mut curve = drag.original.clone();
            transform_parts(&mut curve, &edit.selected, |local| inverse.transform_point3(world_change(world.transform_point3(local))));
            if let Ok(object) = doc.scene.object_mut(edit.object) {
                object.data = ObjectData::Curve(curve);
            }
            if let Some(ShapeSession::Curve(edit)) = doc.session.shape.as_mut() {
                edit.drag = Some(drag);
            }
            doc.touched();
            Some(idle(true))
        }
        PointerAction::Up => {
            let Some(ShapeSession::Curve(mut edit)) = doc.session.shape.clone() else { return Some(idle(false)) };
            let Some(drag) = edit.drag.take() else { return Some(idle(false)) };
            if drag.marquee {
                let parts = parts_at(doc, &edit, drag.start, at);
                let selected = if event.mods.shift {
                    if parts.is_subset(&edit.selected) && !parts.is_empty() {
                        edit.selected.difference(&parts).copied().collect()
                    } else {
                        edit.selected.union(&parts).copied().collect()
                    }
                } else {
                    parts
                };
                edit.selected = selected;
                doc.session.shape = Some(ShapeSession::Curve(edit));
                doc.touched();
            } else {
                doc.session.shape = Some(ShapeSession::Curve(edit));
                if drag.moved && doc.scene != drag.before.scene {
                    if doc.scene.validate().is_ok() {
                        let _ = doc.committed(*drag.before);
                    } else {
                        doc.restore(*drag.before);
                        doc.touched();
                    }
                }
            }
            Some(idle(false))
        }
        _ => Some(idle(false)),
    }
}

/// Escape during a curve drag puts the points back; answers whether it did anything.
pub fn cancel(doc: &mut SceneDocument) -> bool {
    if let Some(ShapeSession::Lattice(_)) = &doc.session.shape {
        return crate::commands_lattice::cancel(doc);
    }
    let Some(ShapeSession::Curve(edit)) = doc.session.shape.as_mut() else { return false };
    let Some(drag) = edit.drag.take() else { return false };
    if drag.moved && !drag.marquee {
        doc.restore(*drag.before);
    }
    doc.touched();
    true
}

/// Whether text edit mode is on (the tools then take typed text).
pub fn typing(doc: &SceneDocument) -> bool {
    matches!(doc.session.shape, Some(ShapeSession::Text(_)))
}

/// Typed text in text edit mode: inserted at the caret as one step.
pub fn type_text(doc: &mut SceneDocument, typed: &str) -> Result<(), String> {
    let Some(ShapeSession::Text(edit)) = doc.session.shape.clone() else { return Ok(()) };
    change(doc, "Type Text", |doc| insert_text(doc, edit.object, typed)).map(|_| ())
}

/// An editing key in text edit mode; answers whether it was taken.
pub fn text_key(doc: &mut SceneDocument, key: &str) -> Result<bool, String> {
    if !typing(doc) {
        return Ok(false);
    }
    let caret = |doc: &mut SceneDocument, params: Value| -> Result<(), String> {
        run_view(doc, "text.caret", &params)?;
        doc.touched();
        Ok(())
    };
    match key {
        "Backspace" => {
            let _ = change(doc, "Delete Characters", |doc| delete_characters(doc, 1, false));
        }
        "Delete" => {
            let _ = change(doc, "Delete Characters", |doc| delete_characters(doc, 1, true));
        }
        "Enter" => {
            type_text(doc, "\n")?;
        }
        "ArrowLeft" => caret(doc, json!({ "by": -1 }))?,
        "ArrowRight" => caret(doc, json!({ "by": 1 }))?,
        "ArrowUp" => caret(doc, json!({ "place": "up" }))?,
        "ArrowDown" => caret(doc, json!({ "place": "down" }))?,
        "Home" => caret(doc, json!({ "place": "lineStart" }))?,
        "End" => caret(doc, json!({ "place": "lineEnd" }))?,
        "Escape" | "Tab" => {
            doc.session.shape = None;
            doc.touched();
        }
        _ => return Ok(false),
    }
    Ok(true)
}
