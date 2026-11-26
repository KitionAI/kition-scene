//! Lattice objects: adding them, their settings, moving their points (for the agent, by index),
//! and lattice edit mode, which Tab enters through the curve module's edit modes
//! ([`crate::commands_curve::ShapeSession::Lattice`]): points are selected by clicking and box
//! dragging, and moved, turned, and scaled with the viewport tools, each drag one undo step.

use std::collections::BTreeSet;

use glam::Vec3;
use kition_kit::KitCommand;
use kition_kit::tools::{PointerAction, ToolOverlay, ToolPointer, ToolResponse};
use kition_scene_model::{Id, Lattice, LatticeInterpolation, ObjectData};
use kition_scene_raster::{CurveOverlay, CurvePointOverlay};
use serde_json::{Value, json};

use crate::commands_curve::{ShapeSession, change, projector, to_screen};
use crate::params::{self, bad, schema as s};
use crate::{Mode, SceneDocument};

/// A lattice's points being edited.
#[derive(Debug, Clone, PartialEq)]
pub struct LatticeEdit {
    pub object: Id,
    pub selected: BTreeSet<u32>,
    /// Where the Select tool's box was pressed, while it is dragged.
    marquee: Option<(f64, f64)>,
}

impl LatticeEdit {
    pub fn new(object: Id) -> Self {
        Self { object, selected: BTreeSet::new(), marquee: None }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Needs {
    ObjectMode,
    /// A lattice active, or being edited.
    Lattice,
    /// Lattice edit mode.
    Edit,
}

struct Spec {
    id: &'static str,
    title: &'static str,
    menu: &'static [&'static str],
    mutates: bool,
    needs: Needs,
    schema: fn() -> Value,
}

fn target() -> (&'static str, Value) {
    ("object", s::object("The lattice; the active or edited one when left out"))
}

fn resolution() -> (&'static str, Value) {
    (
        "resolution",
        json!({ "oneOf": [{ "type": "integer" }, { "type": "array", "items": { "type": "integer" } }], "description": "Points along each side, 1 to 64: one number for all three, or [u, v, w]" }),
    )
}

static SPECS: &[Spec] = &[
    Spec {
        id: "object.addLattice",
        title: "Lattice",
        menu: &["Add"],
        mutates: true,
        needs: Needs::ObjectMode,
        schema: || {
            s::of(vec![
                resolution(),
                ("location", s::vec3("Where to put it, in metres (the origin when left out)")),
                ("rotation", s::vec3("Rotation in degrees, XYZ Euler")),
                ("scale", s::vec3("Scale: the lattice spans this many metres along each axis (1)")),
                ("name", s::string("The object's name")),
            ])
        },
    },
    Spec {
        id: "lattice.set",
        title: "Lattice Settings",
        menu: &[],
        mutates: true,
        needs: Needs::Lattice,
        schema: || {
            s::of(vec![
                target(),
                resolution(),
                ("interpolation", s::choice("How the points' offsets blend (bspline)", &["bspline", "linear"])),
                ("reset", s::boolean("Put every point back where it rests")),
            ])
        },
    },
    Spec {
        id: "lattice.movePoints",
        title: "Move Lattice Points",
        menu: &[],
        mutates: true,
        needs: Needs::Lattice,
        schema: || {
            s::of(vec![
                target(),
                ("points", s::indices("Point indices (u + v × U + w × U × V); the selected points in edit mode when left out")),
                ("offset", s::required(s::vec3("How far to move them, in the lattice's own axes"))),
            ])
        },
    },
    Spec {
        id: "lattice.select",
        title: "Select Lattice Points",
        menu: &[],
        mutates: false,
        needs: Needs::Edit,
        schema: || {
            s::of(vec![
                ("points", s::indices("Point indices")),
                ("extend", s::boolean("Add to the selection")),
                ("deselect", s::boolean("Take them out of the selection")),
            ])
        },
    },
];

fn spec_of(id: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|spec| spec.id == id)
}

fn lattice_of(doc: &SceneDocument, id: Id) -> Option<&Lattice> {
    match doc.scene.object(id).ok().map(|object| &object.data) {
        Some(ObjectData::Lattice(lattice)) => Some(lattice),
        _ => None,
    }
}

fn edited(doc: &SceneDocument) -> Option<&LatticeEdit> {
    match &doc.session.shape {
        Some(ShapeSession::Lattice(edit)) => Some(edit),
        _ => None,
    }
}

fn edited_mut(doc: &mut SceneDocument) -> Option<&mut LatticeEdit> {
    match doc.session.shape.as_mut() {
        Some(ShapeSession::Lattice(edit)) => Some(edit),
        _ => None,
    }
}

/// The lattice the commands act on: the edited one, else the active one.
pub fn active_lattice(doc: &SceneDocument) -> Option<Id> {
    match &doc.session.shape {
        Some(ShapeSession::Lattice(edit)) => Some(edit.object),
        Some(_) => None,
        None => doc.session.editor.active.filter(|id| lattice_of(doc, *id).is_some()),
    }
}

fn enabled(doc: &SceneDocument, needs: Needs) -> bool {
    let object_mode = doc.session.mode == Mode::Object && doc.session.rig.is_none() && doc.session.paint.is_none();
    match needs {
        Needs::ObjectMode => object_mode && doc.session.shape.is_none(),
        Needs::Lattice => object_mode && active_lattice(doc).is_some(),
        Needs::Edit => edited(doc).is_some(),
    }
}

/// The lattice commands, with whether each can run now.
pub fn catalogue(doc: &SceneDocument) -> Vec<KitCommand> {
    SPECS
        .iter()
        .map(|spec| KitCommand {
            id: spec.id.into(),
            title: spec.title.into(),
            menu: (!spec.menu.is_empty()).then(|| spec.menu.iter().map(|part| (*part).to_string()).collect()),
            schema: Some((spec.schema)()),
            mutates: spec.mutates,
            enabled: enabled(doc, spec.needs),
            ..Default::default()
        })
        .collect()
}

/// The lattice commands that cannot run now.
pub fn disabled(doc: &SceneDocument) -> Vec<String> {
    SPECS.iter().filter(|spec| !enabled(doc, spec.needs)).map(|spec| spec.id.to_string()).collect()
}

/// Runs a lattice command, or answers `None` when it is not one.
pub fn execute(doc: &mut SceneDocument, command: &str, params: &Value) -> Option<Result<Value, String>> {
    let spec = spec_of(command)?;
    if !params.is_object() && !params.is_null() {
        return Some(Err(bad("the parameters must be an object")));
    }
    // A named lattice passes the gate as the active one would; it is not made active or selected.
    let object_mode = doc.session.mode == Mode::Object
        && doc.session.rig.is_none()
        && doc.session.paint.is_none()
        && (doc.session.shape.is_none() || edited(doc).is_some());
    let names_lattice = spec.needs == Needs::Lattice
        && object_mode
        && params
            .get("object")
            .filter(|value| !value.is_null())
            .and_then(|value| params::resolve(&doc.scene, value).ok())
            .is_some_and(|id| lattice_of(doc, id).is_some());
    if !(enabled(doc, spec.needs) || names_lattice) {
        return Some(Err(format!("not-allowed: {} cannot run now", spec.title)));
    }
    if !spec.mutates {
        let result = select(doc, params);
        if result.is_ok() {
            doc.touched();
        }
        return Some(result);
    }
    Some(change(doc, spec.title, |doc| run_change(doc, command, params)))
}

fn parse_resolution(params: &Value) -> Result<Option<[u32; 3]>, String> {
    let sides = match params.get("resolution") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Number(_)) => [params::integer(params, "resolution")?.unwrap_or(2); 3],
        Some(Value::Array(list)) if list.len() == 3 => {
            let mut sides = [2; 3];
            for (slot, value) in sides.iter_mut().zip(list) {
                *slot = value.as_u64().and_then(|side| u32::try_from(side).ok()).ok_or_else(|| bad("`resolution` must hold whole numbers"))?;
            }
            sides
        }
        Some(_) => return Err(bad("`resolution` must be a number or [u, v, w]")),
    };
    if sides.iter().any(|side| !(1..=kition_scene_model::lattice::MAX_SIDE).contains(side)) {
        return Err(bad(format!("`resolution` must be 1 to {} on each side", kition_scene_model::lattice::MAX_SIDE)));
    }
    if sides.iter().map(|side| u64::from(*side)).product::<u64>() > 32_768 {
        return Err(bad("a lattice has at most 32768 points"));
    }
    Ok(Some(sides))
}

fn lattice_mut(doc: &mut SceneDocument, id: Id) -> Result<&mut Lattice, String> {
    match &mut doc.scene.object_mut(id).map_err(|error| error.to_string())?.data {
        ObjectData::Lattice(lattice) => Ok(lattice),
        _ => Err(bad("that object is not a lattice")),
    }
}

fn target_id(doc: &SceneDocument, params: &Value) -> Result<Id, String> {
    if let Some(value) = params.get("object").filter(|value| !value.is_null()) {
        return params::resolve(&doc.scene, value);
    }
    active_lattice(doc).ok_or_else(|| "not-allowed: there is no active lattice".into())
}

fn run_change(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    match command {
        "object.addLattice" => {
            let lattice = Lattice::new(parse_resolution(params)?.unwrap_or([2, 2, 2]));
            crate::commands_curve::add(doc, "Lattice", ObjectData::Lattice(lattice), params)
        }
        "lattice.set" => {
            let id = target_id(doc, params)?;
            let resolution = parse_resolution(params)?;
            let interpolation = match params::string(params, "interpolation")? {
                None => None,
                Some("bspline") => Some(LatticeInterpolation::Bspline),
                Some("linear") => Some(LatticeInterpolation::Linear),
                Some(other) => return Err(bad(format!("`interpolation` must be bspline or linear, not {other}"))),
            };
            let reset = params::boolean(params, "reset")?.unwrap_or(false);
            let lattice = lattice_mut(doc, id)?;
            if let Some(interpolation) = interpolation {
                lattice.interpolation = interpolation;
            }
            if let Some(resolution) = resolution
                && resolution != lattice.resolution
            {
                *lattice = lattice.resized(resolution);
            }
            if reset {
                lattice.points = lattice.rest_points();
            }
            let count = lattice.point_count();
            // Selected points that no longer exist go (only the edited lattice's points are selected).
            if let Some(edit) = edited_mut(doc).filter(|edit| edit.object == id) {
                edit.selected.retain(|point| (*point as usize) < count);
            }
            Ok(json!({ "object": id, "points": count }))
        }
        "lattice.movePoints" => {
            let id = target_id(doc, params)?;
            let offset = params::vec3(params, "offset")?.ok_or_else(|| bad("`offset` is required"))?;
            if !offset.is_finite() {
                return Err(bad("`offset` must be numbers"));
            }
            let chosen: Vec<u32> = match params::indices(params, "points")? {
                Some(points) => points,
                None => match edited(doc) {
                    Some(edit) if edit.object == id => edit.selected.iter().copied().collect(),
                    _ => return Err(bad("`points` is required outside lattice edit mode")),
                },
            };
            let lattice = lattice_mut(doc, id)?;
            for point in &chosen {
                let slot = lattice.points.get_mut(*point as usize).ok_or_else(|| format!("not-found: point {point}"))?;
                *slot += offset;
            }
            Ok(json!({ "object": id, "moved": chosen.len() }))
        }
        other => Err(format!("unknown-command: {other}")),
    }
}

fn select(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let edit = edited(doc).cloned().ok_or("not-allowed: no lattice is in edit mode")?;
    let count = lattice_of(doc, edit.object).map_or(0, Lattice::point_count);
    let mut chosen = BTreeSet::new();
    for point in params::indices(params, "points")?.unwrap_or_default() {
        if point as usize >= count {
            return Err(format!("not-found: point {point}"));
        }
        chosen.insert(point);
    }
    let selected = if params::boolean(params, "deselect")?.unwrap_or(false) {
        edit.selected.difference(&chosen).copied().collect()
    } else if params::boolean(params, "extend")?.unwrap_or(false) {
        edit.selected.union(&chosen).copied().collect()
    } else {
        chosen
    };
    if let Some(edit) = edited_mut(doc) {
        edit.selected = selected;
    }
    Ok(json!({ "changed": true }))
}

/// Select all, none, or the others, in lattice edit mode.
pub fn select_all(doc: &mut SceneDocument, action: &str) -> Result<Value, String> {
    let edit = edited(doc).cloned().ok_or("not-allowed: no lattice is in edit mode")?;
    let count = u32::try_from(lattice_of(doc, edit.object).map_or(0, Lattice::point_count)).unwrap_or(0);
    let all: BTreeSet<u32> = (0..count).collect();
    let selected = match action {
        "select" => all,
        "deselect" => BTreeSet::new(),
        "invert" => all.difference(&edit.selected).copied().collect(),
        other => return Err(bad(format!("unknown action {other}"))),
    };
    if let Some(edit) = edited_mut(doc) {
        edit.selected = selected;
    }
    doc.touched();
    Ok(json!({ "changed": true }))
}

/// Drops lattice edit mode when its object is gone or no longer a lattice, and selected points
/// that no longer exist.
pub fn tidy(doc: &mut SceneDocument) {
    let Some(edit) = edited(doc) else { return };
    match lattice_of(doc, edit.object).map(Lattice::point_count) {
        Some(count) => {
            if let Some(edit) = edited_mut(doc) {
                edit.selected.retain(|point| (*point as usize) < count);
            }
        }
        None => doc.session.shape = None,
    }
}

/// The edited lattice's points for the viewport (drawn as a curve's points are).
pub fn overlay(doc: &SceneDocument) -> Option<CurveOverlay> {
    let edit = edited(doc)?;
    let lattice = lattice_of(doc, edit.object)?;
    let points = lattice
        .points
        .iter()
        .enumerate()
        .map(|(index, co)| {
            let chosen = edit.selected.contains(&u32::try_from(index).unwrap_or(u32::MAX));
            CurvePointOverlay { co: *co, handles: None, selected: [chosen, false, false] }
        })
        .collect();
    Some(CurveOverlay { object: edit.object, points })
}

/// What inspect shows of a lattice: its settings, and its points when there are not many.
pub fn lattice_json(lattice: &Lattice) -> Value {
    let rest = lattice.rest_points();
    let moved: Vec<usize> = lattice
        .points
        .iter()
        .zip(&rest)
        .enumerate()
        .filter(|(_, (point, at_rest))| point.distance(**at_rest) > 1e-6)
        .map(|(index, _)| index)
        .collect();
    let mut value = json!({
        "resolution": lattice.resolution,
        "interpolation": match lattice.interpolation {
            LatticeInterpolation::Bspline => "bspline",
            LatticeInterpolation::Linear => "linear",
        },
        "pointCount": lattice.point_count(),
        "moved": moved,
    });
    if lattice.point_count() <= 512
        && let Some(map) = value.as_object_mut()
    {
        let round = |value: f32| (f64::from(value) * 10_000.0).round() / 10_000.0;
        map.insert("points".into(), json!(lattice.points.iter().map(|point| [round(point.x), round(point.y), round(point.z)]).collect::<Vec<_>>()));
    }
    value
}

/// The session's state for inspect.
pub fn extend_inspect(edit: &LatticeEdit, mode: &mut serde_json::Map<String, Value>) {
    mode.insert("mode".into(), json!("latticeEdit"));
    mode.insert("object".into(), json!(edit.object));
    mode.insert("points".into(), json!(edit.selected));
}

/// The points under (or in the box to) a pixel.
fn points_at(doc: &SceneDocument, edit: &LatticeEdit, start: (f64, f64), end: (f64, f64)) -> BTreeSet<u32> {
    let Some(lattice) = lattice_of(doc, edit.object) else { return BTreeSet::new() };
    let Ok(world) = doc.scene.world_matrix(edit.object) else { return BTreeSet::new() };
    let projector = projector(doc);
    let boxed = (end.0 - start.0).abs() > 4.0 || (end.1 - start.1).abs() > 4.0;
    let (left, right, top, bottom) = (start.0.min(end.0) as f32, start.0.max(end.0) as f32, start.1.min(end.1) as f32, start.1.max(end.1) as f32);
    let mut found = BTreeSet::new();
    let mut nearest: Option<(f32, u32)> = None;
    for (index, point) in lattice.points.iter().enumerate() {
        let index = u32::try_from(index).unwrap_or(u32::MAX);
        let Some((x, y)) = to_screen(&projector, world.transform_point3(*point)) else { continue };
        if boxed {
            if x >= left && x <= right && y >= top && y <= bottom {
                found.insert(index);
            }
        } else {
            let distance = (x - end.0 as f32).hypot(y - end.1 as f32);
            if distance <= 12.0 && nearest.is_none_or(|(best, _)| distance < best) {
                nearest = Some((distance, index));
            }
        }
    }
    found.extend(nearest.map(|(_, index)| index));
    found
}

pub(crate) fn pivot(doc: &SceneDocument, edit: &LatticeEdit) -> Option<Vec3> {
    let lattice = lattice_of(doc, edit.object)?;
    let world = doc.scene.world_matrix(edit.object).ok()?;
    let chosen: Vec<Vec3> =
        edit.selected.iter().filter_map(|index| lattice.points.get(*index as usize)).map(|point| world.transform_point3(*point)).collect();
    (!chosen.is_empty()).then(|| chosen.iter().copied().sum::<Vec3>() / chosen.len() as f32)
}

/// The viewport tools in lattice edit mode: the Select tool picks and boxes points; Move, Rotate
/// and Scale start the transform on a press (the point under the pointer first, when none is
/// selected).
pub fn pointer(doc: &mut SceneDocument, tool: &str, event: &ToolPointer) -> ToolResponse {
    let cursor = match tool {
        "move" => "move",
        "select" => "default",
        _ => "crosshair",
    };
    let idle = |busy: bool| ToolResponse { cursor: cursor.into(), busy, ..Default::default() };
    let Some(edit) = edited(doc).cloned() else { return idle(false) };
    let at = (event.x, event.y);
    match event.action {
        PointerAction::Down => {
            if let Some(kind) = crate::modal::Kind::parse(tool) {
                if edit.selected.is_empty() {
                    let points = points_at(doc, &edit, at, at);
                    if let Some(edit) = edited_mut(doc) {
                        edit.selected = points;
                    }
                }
                let _ = doc.modal_begin(kind, Some(at), crate::modal::Started::Press { select: false }, None, false);
                return idle(true);
            }
            if let Some(edit) = edited_mut(doc) {
                edit.marquee = Some(at);
            }
            idle(true)
        }
        PointerAction::Drag => {
            let Some((x0, y0)) = edit.marquee else { return idle(false) };
            ToolResponse {
                overlays: vec![ToolOverlay::Marquee { x0, y0, x1: at.0, y1: at.1 }],
                cursor: cursor.into(),
                busy: true,
                ..Default::default()
            }
        }
        PointerAction::Up => {
            let Some(start) = edit.marquee else { return idle(false) };
            let points = points_at(doc, &edit, start, at);
            let selected = if event.mods.shift {
                if points.is_subset(&edit.selected) && !points.is_empty() {
                    edit.selected.difference(&points).copied().collect()
                } else {
                    edit.selected.union(&points).copied().collect()
                }
            } else {
                points
            };
            doc.session.shape = Some(ShapeSession::Lattice(LatticeEdit { selected, marquee: None, ..edit }));
            doc.touched();
            idle(false)
        }
        _ => idle(false),
    }
}

/// Escape during a box of the Select tool drops it; answers whether there was one.
pub fn cancel(doc: &mut SceneDocument) -> bool {
    edited_mut(doc).and_then(|edit| edit.marquee.take()).is_some()
}
