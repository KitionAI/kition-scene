//! Constraints on objects and bones, and weight paint. Angles are in degrees.
//!
//! A constraint goes on an object, or with `bone` on a bone of an armature object (in pose
//! mode the active bone is the default owner). Child Of sets its inverse when it is added, so
//! the owner stays where it is, as in the reference application.
//!
//! Weight paint is a mode of a mesh object beside the others ([`PaintSession`]): the viewport
//! shows one vertex group's weights from blue (0) to red (1), and the pointer paints them with
//! the brush, draws a linear or radial gradient, or samples the weight under it. X mirror paints
//! the vertex across X = 0 too (by position or by topology), in the group of the mirrored name.
//! Each stroke and each gradient is one undo step.

use glam::{Mat3, Vec3};
use kition_kit::KitCommand;
use kition_kit::tools::{PointerAction, ToolOverlay, ToolPointer, ToolResponse};
use kition_scene_model::constraints::{
    self, ActionMix, ArmatureTarget, Constraint, ConstraintKind, KeepAxis, Space, SplineXzScale, SplineYScale, StretchVolume, Target, TrackAxis,
    TransformChannel, TransformMix, UpAxis,
};
use kition_scene_model::proportional::Falloff;
use kition_scene_model::weight_paint::{self, Brush, BrushMode, GradientKind, Mirror};
use kition_scene_model::{Id, ObjectData};
use kition_scene_raster::WeightOverlay;
use serde_json::{Value, json};

use crate::params::{self, bad, schema as s};
use crate::{Mode, SceneDocument, Step};

/// A mesh in weight paint mode.
#[derive(Debug, Clone, PartialEq)]
pub struct PaintSession {
    pub object: Id,
    /// The group painted; the first group (or a new `Group`) when none is chosen.
    pub group: Option<String>,
    pub brush: Brush,
    pub mirror: PaintMirror,
    pub tool: PaintTool,
    stroke: Option<Stroke>,
}

/// Weight paint's symmetry (the reference keeps it on the mesh; here it is session state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaintMirror {
    /// Paint the vertex across X = 0 too.
    pub x: bool,
    /// Find that vertex by the edges, not by position.
    pub topology: bool,
    /// Paint it in the group of the mirrored name (`Arm.L`, `Arm.R`).
    pub groups: bool,
}

impl Default for PaintMirror {
    fn default() -> Self {
        Self { x: false, topology: false, groups: true }
    }
}

/// What the pointer does in weight paint.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PaintTool {
    #[default]
    Brush,
    Gradient(GradientKind),
    Sample,
}

impl PaintTool {
    fn name(self) -> &'static str {
        match self {
            Self::Brush => "brush",
            Self::Gradient(GradientKind::Linear) => "linearGradient",
            Self::Gradient(GradientKind::Radial) => "radialGradient",
            Self::Sample => "sampleWeight",
        }
    }
}

#[derive(Debug, Clone)]
struct Stroke {
    before: Box<Step>,
    /// Where the stroke or gradient started.
    start: (f64, f64),
    last: (f64, f64),
    painted: bool,
}

impl PartialEq for Stroke {
    fn eq(&self, other: &Self) -> bool {
        self.last == other.last && self.painted == other.painted
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Needs {
    /// Something to constrain: an object, or a bone in pose mode.
    Owner,
    /// A mesh in object mode, or weight paint on.
    Paintable,
    Paint,
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

fn owner_fields() -> Vec<(&'static str, Value)> {
    vec![
        ("object", s::object("The owner; the active object when left out")),
        ("bone", s::string("A bone of the owner (an armature) to constrain instead of the object; the active bone in pose mode")),
    ]
}

fn reference_fields() -> Vec<(&'static str, Value)> {
    let mut fields = owner_fields();
    fields.push(("index", s::integer("The constraint's position in the stack, from 0")));
    fields.push(("name", s::string("The constraint's name (instead of index)")));
    fields
}

fn settings() -> Vec<(&'static str, Value)> {
    vec![
        ("target", s::object("The target object")),
        ("targetBone", s::string("A bone of the target (an armature) to use instead of the object")),
        ("influence", s::number("How much of the result is used, 0 to 1 (1)")),
        ("trackAxis", s::choice("trackTo and dampedTrack: the axis pointed at the target", &["x", "y", "z", "-x", "-y", "-z"])),
        ("upAxis", s::choice("trackTo: the axis kept up", &["x", "y", "z"])),
        ("axes", json!({ "type": "array", "items": { "type": "boolean" }, "description": "copyLocation and copyRotation: [x, y, z] axes copied" })),
        (
            "invert",
            json!({ "type": "array", "items": { "type": "boolean" }, "description": "copyLocation and copyRotation: [x, y, z] axes inverted" }),
        ),
        ("offset", s::boolean("copyLocation: add to the owner's location instead of replacing it")),
        ("limit", json!({ "type": "array", "items": { "type": "boolean" }, "description": "limitRotation: [x, y, z] angles limited" })),
        (
            "min",
            json!({ "oneOf": [{ "type": "array", "items": { "type": "number" } }, { "type": "number" }], "description": "limitRotation: lowest angles, degrees [x, y, z]; action: the channel value that plays frameStart (degrees for rotation)" }),
        ),
        (
            "max",
            json!({ "oneOf": [{ "type": "array", "items": { "type": "number" } }, { "type": "number" }], "description": "limitRotation: highest angles, degrees [x, y, z]; action: the channel value that plays frameEnd" }),
        ),
        ("space", s::choice("limitRotation: the space the angles are read in (pose and localWithParent on bones)", SPACES)),
        (
            "ownerSpace",
            s::choice("copyLocation, copyRotation, transform: the space the owner is read in (pose and localWithParent on bones)", SPACES),
        ),
        ("targetSpace", s::choice("copyLocation, copyRotation, transform, action: the space the target is read in", SPACES)),
        ("restLength", s::number("stretchTo: the distance, in the owner's Y units, at which it does not stretch; 0 takes the distance now")),
        ("bulge", s::number("stretchTo and splineIk: how strongly the other axes thin as it stretches, 0 to 100 (1 keeps the volume)")),
        ("volume", s::choice("stretchTo: the axes that thin", &["xzx", "x", "z", "none"])),
        ("keepAxis", s::choice("stretchTo: how Y turns to the target", &["swingY", "planeX", "planeZ"])),
        ("axis", s::choice("floor: the plane's side the owner stays on", &["x", "y", "z", "-x", "-y", "-z"])),
        ("offset", s::number("copyLocation: true adds to the owner's location; floor: metres beyond the plane")),
        ("useRotation", s::boolean("floor: the plane turns with the target")),
        ("mapFrom", s::choice("transform: the target's channel read", &["location", "rotation", "scale"])),
        ("mapTo", s::choice("transform: the owner's channel set", &["location", "rotation", "scale"])),
        ("fromMin", s::vec3("transform: the source range's low end (degrees for rotation)")),
        ("fromMax", s::vec3("transform: the source range's high end")),
        ("toMin", s::vec3("transform: the destination range's low end (degrees for rotation)")),
        ("toMax", s::vec3("transform: the destination range's high end")),
        (
            "mapping",
            json!({ "type": "array", "items": { "type": "string", "enum": ["x", "y", "z"] }, "description": "transform: for the owner's x, y, z, the target axis each reads" }),
        ),
        ("extrapolate", s::boolean("transform: map values beyond the source range too")),
        (
            "mix",
            s::choice(
                "transform: add or replace (location), add, replace, before, or after (rotation), replace or multiply (scale); action: how the action combines with the owner",
                &["add", "replace", "before", "after", "multiply", "beforeFull", "beforeSplit", "afterFull", "afterSplit"],
            ),
        ),
        ("action", s::string("action: the action's name")),
        ("channel", s::choice("action: the target's channel that picks the frame", ACTION_CHANNELS)),
        ("frameStart", s::number("action: the frame the channel's minimum plays")),
        ("frameEnd", s::number("action: the frame the channel's maximum plays")),
        (
            "targets",
            json!({ "type": "array", "items": { "type": "object", "additionalProperties": true, "properties": { "object": s::object("The armature object by id or name"), "bone": s::string("The bone name"), "weight": s::number("The bone weight") } }, "description": "armature: the bones that move the owner, as { object, bone, weight }" }),
        ),
        ("preserveVolume", s::boolean("armature: blend the bones as dual quaternions")),
        ("evenDivisions", s::boolean("splineIk: share the curve evenly between the bones")),
        ("chainOffset", s::boolean("splineIk: keep the chain's root where it is")),
        ("curveRadius", s::boolean("splineIk: the curve's radius scales the bones' thickness (true)")),
        ("yScale", s::choice("splineIk: the bones' lengths", &["none", "fitCurve", "boneOriginal"])),
        ("xzScale", s::choice("splineIk: the bones' thickness", &["none", "boneOriginal", "inversePreserve", "volumePreserve"])),
        ("pole", s::object("ik: the pole target object")),
        ("poleBone", s::string("ik: a bone of the pole target")),
        ("poleAngle", s::number("ik: degrees the chain turns from the pole")),
        ("chain", s::integer("ik: bones in the chain counting this one, 0 runs to the root; splineIk: bones fitted to the curve (1)")),
        ("iterations", s::integer("ik: solver iterations (500)")),
    ]
}

const CONSTRAINT: &[&str] = &["Object", "Constraints"];
const SPACES: &[&str] = &["world", "local", "pose", "localWithParent"];
const ACTION_CHANNELS: &[&str] = &["locationX", "locationY", "locationZ", "rotationX", "rotationY", "rotationZ", "scaleX", "scaleY", "scaleZ"];
const KIND_NAMES: &[&str] = &[
    "trackTo",
    "dampedTrack",
    "copyLocation",
    "copyRotation",
    "limitRotation",
    "childOf",
    "ik",
    "stretchTo",
    "floor",
    "transform",
    "action",
    "armature",
    "splineIk",
];
const PAINT: &[&str] = &["Weights"];

static SPECS: &[Spec] = &[
    spec("constraint.add", "Add Constraint", CONSTRAINT, None, true, Needs::Owner, || {
        let mut fields = owner_fields();
        fields.push(("type", s::required(s::choice("The constraint (ik and splineIk on bones)", KIND_NAMES))));
        fields.push(("rename", s::string("Its name")));
        fields.extend(settings());
        s::of(fields)
    }),
    spec("constraint.set", "Change Constraint", &[], None, true, Needs::Owner, || {
        let mut fields = reference_fields();
        fields.push(("enabled", s::boolean("On or off")));
        fields.push(("rename", s::string("A new name")));
        fields.extend(settings());
        s::of(fields)
    }),
    spec("constraint.remove", "Remove Constraint", &[], None, true, Needs::Owner, || s::of(reference_fields())),
    spec("constraint.move", "Move Constraint", &[], None, true, Needs::Owner, || {
        let mut fields = reference_fields();
        fields.push(("to", s::required(s::integer("The new position, from 0"))));
        s::of(fields)
    }),
    spec("constraint.apply", "Apply Constraint", &[], None, true, Needs::Owner, || s::of(reference_fields())),
    spec("constraint.setInverse", "Set Inverse", &[], None, true, Needs::Owner, || s::of(reference_fields())),
    spec("constraint.clearInverse", "Clear Inverse", &[], None, true, Needs::Owner, || s::of(reference_fields())),
    spec("mode.weightPaint", "Weight Paint", &["Object"], None, false, Needs::Paintable, || {
        s::of(vec![
            ("on", s::boolean("Weight paint on or off; toggles when left out")),
            ("object", s::object("The mesh; the active one when left out")),
        ])
    }),
    spec("paint.group", "Paint Group", &[], None, false, Needs::Paint, || {
        s::of(vec![("name", s::required(s::string("The vertex group to paint (made on the first stroke)")))])
    }),
    spec("paint.brush", "Brush Settings", &[], None, false, Needs::Paint, || {
        s::of(vec![
            ("mode", s::choice("What a stroke does", &["draw", "add", "subtract", "blur"])),
            ("weight", s::number("The weight painted, 0 to 1 (1)")),
            ("strength", s::number("How much one dab changes, 0 to 1 (1)")),
            ("radius", s::number("Radius in pixels (50)")),
            ("falloff", s::choice("How the brush fades to its edge", &["smooth", "sphere", "root", "sharp", "linear", "constant"])),
            ("frontFacesOnly", s::boolean("Only vertices facing the view")),
            ("xMirror", s::boolean("Paint the vertex across X = 0 too (false)")),
            ("topologyMirror", s::boolean("X mirror finds that vertex by the edges, for sides that differ in shape (false)")),
            ("mirrorGroups", s::boolean("X mirror paints in the group of the mirrored name, Arm.L and Arm.R (true)")),
        ])
    }),
    spec("paint.tool", "Weight Tool", &[], None, false, Needs::Paint, || {
        s::of(vec![("tool", s::required(s::choice("What the pointer does", &["brush", "linearGradient", "radialGradient", "sampleWeight"])))])
    }),
    spec("paint.sample", "Sample Weight", &[], None, false, Needs::Paint, || {
        s::of(vec![("at", s::required(json!({ "type": "array", "items": { "type": "number" }, "description": "Viewport pixels [x, y]" })))])
    }),
    spec("paint.gradient", "Gradient", &[], None, true, Needs::Paint, || {
        s::of(vec![
            (
                "from",
                s::required(
                    json!({ "type": "array", "items": { "type": "number" }, "description": "Where the full weight is, viewport pixels [x, y]" }),
                ),
            ),
            (
                "to",
                s::required(
                    json!({ "type": "array", "items": { "type": "number" }, "description": "Where it has faded out, viewport pixels [x, y]" }),
                ),
            ),
            ("kind", s::choice("Along the line, or around the start (linear)", &["linear", "radial"])),
            ("group", s::string("The group; the paint group when left out")),
        ])
    }),
    spec("paint.stroke", "Paint Stroke", &[], None, true, Needs::Paint, || {
        s::of(vec![
            (
                "points",
                s::required(
                    json!({ "type": "array", "items": { "type": "json" }, "description": "The stroke's path in viewport pixels, as [x, y] pairs" }),
                ),
            ),
            ("group", s::string("The group to paint; the paint group when left out")),
        ])
    }),
    spec("paint.vertices", "Set Weights", &[], None, true, Needs::Paintable, || {
        s::of(vec![
            ("object", s::object("The mesh; the painted or active one when left out")),
            ("group", s::string("The group; the paint group when left out")),
            ("vertices", s::required(s::indices("Vertex indices"))),
            ("weight", s::number("The weight (1)")),
            ("mode", s::choice("How the weight is used (draw replaces)", &["draw", "add", "subtract"])),
        ])
    }),
    spec("paint.normalizeAll", "Normalize All", PAINT, None, true, Needs::Paintable, || {
        s::of(vec![
            ("object", s::object("The mesh; the painted or active one when left out")),
            ("lockActive", s::boolean("Keep the paint group's weights and share the rest (true)")),
        ])
    }),
];

fn spec_of(id: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|spec| spec.id == id)
}

fn active_mesh(doc: &SceneDocument) -> Option<Id> {
    doc.session.editor.active.filter(|id| doc.scene.object(*id).is_ok_and(|object| object.mesh().is_some()))
}

fn enabled(doc: &SceneDocument, needs: Needs) -> bool {
    let object_mode = doc.session.mode == Mode::Object;
    match needs {
        Needs::Owner => doc.session.editor.active.is_some() || doc.session.rig.as_ref().is_some_and(|rig| rig.pose && rig.active.is_some()),
        Needs::Paintable => doc.session.paint.is_some() || (object_mode && doc.session.rig.is_none() && active_mesh(doc).is_some()),
        Needs::Paint => doc.session.paint.is_some(),
    }
}

/// Every constraint and weight paint command, with whether it can run now.
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

/// The commands here that cannot run now.
pub fn disabled(doc: &SceneDocument) -> Vec<String> {
    SPECS.iter().filter(|spec| !enabled(doc, spec.needs)).map(|spec| spec.id.to_string()).collect()
}

/// Mode changes elsewhere end weight paint.
const ENDS_PAINT: &[&str] = &["mode.set", "mode.toggle", "mode.armature", "mode.pose"];

/// Runs a constraint or weight paint command, or answers `None` when it is neither.
pub fn execute(doc: &mut SceneDocument, command: &str, params: &Value) -> Option<Result<Value, String>> {
    if doc.session.paint.is_some() && ENDS_PAINT.contains(&command) {
        doc.session.paint = None;
        doc.touched();
    }
    let spec = spec_of(command)?;
    if !params.is_object() && !params.is_null() {
        return Some(Err(bad("the parameters must be an object")));
    }
    let named = params.get("object").is_some_and(|value| !value.is_null());
    if !(enabled(doc, spec.needs) || (named && spec.needs != Needs::Paint)) {
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
fn change(doc: &mut SceneDocument, label: &str, work: impl FnOnce(&mut SceneDocument) -> Result<Value, String>) -> Result<Value, String> {
    let before = doc.step(label);
    let session = doc.session.clone();
    match work(doc) {
        Ok(value) => {
            doc.tidy_session();
            if let Err(error) = doc.scene.validate() {
                doc.restore(before);
                doc.session = session;
                return Err(format!("not-allowed: {}", error.to_string().replace("invalid-file: ", "")));
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

/// Ends weight paint when its mesh is gone or no longer a mesh.
pub fn tidy(doc: &mut SceneDocument) {
    if let Some(paint) = &doc.session.paint
        && !doc.scene.object(paint.object).is_ok_and(|object| object.mesh().is_some())
    {
        doc.session.paint = None;
    }
}

/// The group weight paint shows and paints.
fn paint_group(doc: &SceneDocument) -> Option<String> {
    let paint = doc.session.paint.as_ref()?;
    if let Some(name) = &paint.group {
        return Some(name.clone());
    }
    let mesh = doc.scene.object(paint.object).ok()?.mesh()?;
    mesh.groups.first().map(|group| group.name.clone())
}

/// [`paint_group`] when `object` is the mesh in weight paint; another mesh has no paint group
/// (the session's group is the painted mesh's, not this one's).
fn paint_group_of(doc: &SceneDocument, object: Id) -> Option<String> {
    doc.session.paint.as_ref().filter(|paint| paint.object == object)?;
    paint_group(doc)
}

/// What the viewport shows in weight paint: the paint group's weights.
pub fn weight_overlay(doc: &SceneDocument) -> Option<WeightOverlay> {
    let paint = doc.session.paint.as_ref()?;
    let mesh = doc.scene.object(paint.object).ok()?.mesh()?;
    let group = paint_group(doc);
    let weights = group.and_then(|name| mesh.groups.iter().find(|group| group.name == name).map(|group| group.weights.clone())).unwrap_or_default();
    let mut weights = weights;
    weights.resize(mesh.positions.len(), 0.0);
    Some(WeightOverlay { object: paint.object, weights })
}

fn run_view(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    match command {
        "mode.weightPaint" => {
            let target = match params.get("object").filter(|value| !value.is_null()) {
                Some(value) => params::resolve(&doc.scene, value)?,
                None => {
                    doc.session.paint.as_ref().map(|paint| paint.object).or_else(|| active_mesh(doc)).ok_or("not-allowed: select a mesh to paint")?
                }
            };
            if doc.scene.object(target).map_err(|error| error.to_string())?.mesh().is_none() {
                return Err(bad("weight paint works on meshes"));
            }
            let on = params::boolean(params, "on")?.unwrap_or(doc.session.paint.as_ref().is_none_or(|paint| paint.object != target));
            if on {
                // Weight paint is a mode of its own: mesh edit and armature modes end.
                doc.session.mode = Mode::Object;
                doc.session.vertices.clear();
                doc.session.rig = None;
                let (brush, mirror, tool) = doc.session.paint.as_ref().map(|paint| (paint.brush, paint.mirror, paint.tool)).unwrap_or_default();
                doc.session.paint = Some(PaintSession { object: target, group: None, brush, mirror, tool, stroke: None });
                doc.session.editor.active = Some(target);
                if !doc.session.editor.selected.contains(&target) {
                    doc.session.editor.selected.push(target);
                }
            } else {
                doc.session.paint = None;
            }
            Ok(json!({ "weightPaint": on, "object": target }))
        }
        "paint.group" => {
            let name = params::string(params, "name")?.map(str::trim).filter(|name| !name.is_empty()).ok_or_else(|| bad("`name` is required"))?;
            let paint = doc.session.paint.as_mut().ok_or("not-allowed: not in weight paint")?;
            paint.group = Some(name.to_string());
            Ok(json!({ "group": name }))
        }
        "paint.brush" => {
            let paint = doc.session.paint.as_mut().ok_or("not-allowed: not in weight paint")?;
            let mut brush = paint.brush;
            if let Some(mode) = params::string(params, "mode")? {
                brush.mode = match mode {
                    "draw" => BrushMode::Draw,
                    "add" => BrushMode::Add,
                    "subtract" => BrushMode::Subtract,
                    "blur" => BrushMode::Blur,
                    other => return Err(bad(format!("unknown brush mode {other}"))),
                };
            }
            if let Some(weight) = params::number(params, "weight")? {
                brush.weight = weight;
            }
            if let Some(strength) = params::number(params, "strength")? {
                brush.strength = strength;
            }
            if let Some(radius) = params::number(params, "radius")? {
                brush.radius = radius;
            }
            if let Some(falloff) = params::string(params, "falloff")? {
                brush.falloff = serde_json::from_value::<Falloff>(json!(falloff)).map_err(|_| bad(format!("unknown falloff {falloff}")))?;
            }
            if let Some(front) = params::boolean(params, "frontFacesOnly")? {
                brush.front_faces_only = front;
            }
            let mut mirror = paint.mirror;
            if let Some(x) = params::boolean(params, "xMirror")? {
                mirror.x = x;
            }
            if let Some(topology) = params::boolean(params, "topologyMirror")? {
                mirror.topology = topology;
            }
            if let Some(groups) = params::boolean(params, "mirrorGroups")? {
                mirror.groups = groups;
            }
            brush.check().map_err(|error| error.to_string())?;
            paint.brush = brush;
            paint.mirror = mirror;
            let mut out = serde_json::to_value(brush).unwrap_or(Value::Null);
            if let Some(map) = out.as_object_mut() {
                map.insert("xMirror".into(), json!(mirror.x));
                map.insert("topologyMirror".into(), json!(mirror.topology));
                map.insert("mirrorGroups".into(), json!(mirror.groups));
            }
            Ok(out)
        }
        "paint.tool" => {
            let tool = match params::string(params, "tool")?.ok_or_else(|| bad("`tool` is required"))? {
                "brush" => PaintTool::Brush,
                "linearGradient" => PaintTool::Gradient(GradientKind::Linear),
                "radialGradient" => PaintTool::Gradient(GradientKind::Radial),
                "sampleWeight" => PaintTool::Sample,
                other => return Err(bad(format!("unknown weight tool {other}"))),
            };
            let paint = doc.session.paint.as_mut().ok_or("not-allowed: not in weight paint")?;
            paint.tool = tool;
            Ok(json!({ "tool": tool.name() }))
        }
        "paint.sample" => {
            let (x, y) = pixel(params, "at")?;
            let weight = sample(doc, x, y).ok_or("not-found: the painted mesh is not under that point")?;
            let paint = doc.session.paint.as_mut().ok_or("not-allowed: not in weight paint")?;
            paint.brush.weight = weight.clamp(0.0, 1.0);
            Ok(json!({ "weight": paint.brush.weight }))
        }
        other => Err(format!("unknown-command: {other}")),
    }
}

/// The owner a constraint command names: (object, bone).
fn owner(doc: &SceneDocument, params: &Value) -> Result<(Id, Option<String>), String> {
    let pose = doc.session.rig.as_ref().filter(|rig| rig.pose);
    let object = match params.get("object").filter(|value| !value.is_null()) {
        Some(value) => params::resolve(&doc.scene, value)?,
        None => pose.map(|rig| rig.object).or(doc.session.editor.active).ok_or("not-allowed: there is no active object; name `object`")?,
    };
    let bone = match params::string(params, "bone")? {
        Some(bone) => Some(bone.to_string()),
        None if pose.is_some_and(|rig| rig.object == object) => pose.and_then(|rig| rig.active.clone()),
        None => None,
    };
    if let Some(name) = &bone {
        let armature = kition_scene_model::armature::armature_of(&doc.scene, object).ok_or_else(|| bad("only an armature has bones"))?;
        armature.bone(name).ok_or_else(|| format!("not-found: no bone {name}"))?;
    }
    Ok((object, bone))
}

fn stack_mut<'a>(scene: &'a mut kition_scene_model::Scene, object: Id, bone: Option<&str>) -> Result<&'a mut Vec<Constraint>, String> {
    match bone {
        Some(name) => kition_scene_model::armature::armature_mut(scene, object)
            .and_then(|armature| armature.bone_mut(name))
            .map(|bone| &mut bone.constraints)
            .ok_or_else(|| format!("not-found: no bone {name}")),
        None => scene.object_mut(object).map(|object| &mut object.constraints).map_err(|error| error.to_string()),
    }
}

fn stack(scene: &kition_scene_model::Scene, object: Id, bone: Option<&str>) -> Result<Vec<Constraint>, String> {
    match bone {
        Some(name) => kition_scene_model::armature::armature_of(scene, object)
            .and_then(|armature| armature.bone(name))
            .map(|bone| bone.constraints.clone())
            .ok_or_else(|| format!("not-found: no bone {name}")),
        None => scene.object(object).map(|object| object.constraints.clone()).map_err(|error| error.to_string()),
    }
}

/// Which constraint in the stack `params` names.
fn index_of(list: &[Constraint], params: &Value) -> Result<usize, String> {
    match (params::integer(params, "index")?, params::string(params, "name")?) {
        (Some(index), _) if (index as usize) < list.len() => Ok(index as usize),
        (Some(index), _) => Err(format!("not-found: there is no constraint {index}")),
        (None, Some(name)) => {
            list.iter().position(|constraint| constraint.name == name).ok_or_else(|| format!("not-found: no constraint named {name}"))
        }
        (None, None) => Err(bad("name the constraint by `index` or `name`")),
    }
}

fn target_param(doc: &SceneDocument, params: &Value, object_key: &str, bone_key: &str) -> Result<Option<Option<Target>>, String> {
    let Some(value) = params.get(object_key) else { return Ok(None) };
    if value.is_null() {
        return Ok(Some(None));
    }
    let object = params::resolve(&doc.scene, value)?;
    let bone = params::string(params, bone_key)?.map(str::to_string);
    if let Some(name) = &bone {
        let armature =
            kition_scene_model::armature::armature_of(&doc.scene, object).ok_or_else(|| bad(format!("`{bone_key}` needs an armature target")))?;
        armature.bone(name).ok_or_else(|| format!("not-found: no bone {name}"))?;
    }
    Ok(Some(Some(Target { object, bone })))
}

fn track_axis(text: &str) -> Result<TrackAxis, String> {
    Ok(match text {
        "x" => TrackAxis::X,
        "y" => TrackAxis::Y,
        "z" => TrackAxis::Z,
        "-x" => TrackAxis::NegX,
        "-y" => TrackAxis::NegY,
        "-z" => TrackAxis::NegZ,
        other => return Err(bad(format!("unknown axis {other}"))),
    })
}

fn up_axis(text: &str) -> Result<UpAxis, String> {
    Ok(match text {
        "x" => UpAxis::X,
        "y" => UpAxis::Y,
        "z" => UpAxis::Z,
        other => return Err(bad(format!("unknown axis {other}"))),
    })
}

fn flags(params: &Value, key: &str) -> Result<Option<[bool; 3]>, String> {
    let Some(value) = params.get(key).filter(|value| !value.is_null()) else { return Ok(None) };
    let items = value.as_array().filter(|items| items.len() == 3).ok_or_else(|| bad(format!("`{key}` is three true/false values")))?;
    let mut out = [false; 3];
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = item.as_bool().ok_or_else(|| bad(format!("`{key}` is three true/false values")))?;
    }
    Ok(Some(out))
}

fn radians(value: Vec3) -> Vec3 {
    Vec3::new(value.x.to_radians(), value.y.to_radians(), value.z.to_radians())
}

/// Applies the settings in `params` to a constraint.
fn configure(doc: &SceneDocument, constraint: &mut Constraint, params: &Value) -> Result<(), String> {
    if let Some(influence) = params::number(params, "influence")? {
        constraint.influence = influence;
    }
    if let Some(enabled) = params::boolean(params, "enabled")? {
        constraint.enabled = enabled;
    }
    let target = target_param(doc, params, "target", "targetBone")?;
    match &mut constraint.kind {
        ConstraintKind::TrackTo { target: slot, track_axis: track, up_axis: up } => {
            if let Some(target) = target {
                *slot = target;
            }
            if let Some(text) = params::string(params, "trackAxis")? {
                *track = track_axis(text)?;
            }
            if let Some(text) = params::string(params, "upAxis")? {
                *up = up_axis(text)?;
            }
        }
        ConstraintKind::DampedTrack { target: slot, track_axis: track } => {
            if let Some(target) = target {
                *slot = target;
            }
            if let Some(text) = params::string(params, "trackAxis")? {
                *track = track_axis(text)?;
            }
        }
        ConstraintKind::CopyLocation { target: slot, axes, invert, offset, owner_space, target_space } => {
            if let Some(target) = target {
                *slot = target;
            }
            spaces(params, owner_space, target_space)?;
            if let Some(value) = flags(params, "axes")? {
                *axes = value;
            }
            if let Some(value) = flags(params, "invert")? {
                *invert = value;
            }
            if let Some(value) = params::boolean(params, "offset")? {
                *offset = value;
            }
        }
        ConstraintKind::CopyRotation { target: slot, axes, invert, owner_space, target_space } => {
            if let Some(target) = target {
                *slot = target;
            }
            spaces(params, owner_space, target_space)?;
            if let Some(value) = flags(params, "axes")? {
                *axes = value;
            }
            if let Some(value) = flags(params, "invert")? {
                *invert = value;
            }
        }
        ConstraintKind::LimitRotation { limit, min, max, space } => {
            if let Some(value) = flags(params, "limit")? {
                *limit = value;
            }
            if let Some(value) = params::vec3(params, "min")? {
                *min = radians(value);
            }
            if let Some(value) = params::vec3(params, "max")? {
                *max = radians(value);
            }
            if let Some(text) = params::string(params, "space")? {
                *space = space_of(text)?;
            }
        }
        ConstraintKind::ChildOf { target: slot, .. } => {
            if let Some(target) = target {
                *slot = target;
            }
        }
        ConstraintKind::Ik { target: slot, pole, pole_angle, chain, iterations } => {
            if let Some(target) = target {
                *slot = target;
            }
            if let Some(value) = target_param(doc, params, "pole", "poleBone")? {
                *pole = value;
            }
            if let Some(value) = params::number(params, "poleAngle")? {
                *pole_angle = value.to_radians();
            }
            if let Some(value) = params::integer(params, "chain")? {
                *chain = value;
            }
            if let Some(value) = params::integer(params, "iterations")? {
                *iterations = value;
            }
        }
        ConstraintKind::StretchTo { target: slot, rest_length, bulge, volume, keep_axis } => {
            if let Some(target) = target {
                *slot = target;
            }
            if let Some(value) = params::number(params, "restLength")? {
                *rest_length = value;
            }
            if let Some(value) = params::number(params, "bulge")? {
                *bulge = value;
            }
            if let Some(text) = params::string(params, "volume")? {
                *volume = parse_choice(
                    text,
                    &[("xzx", StretchVolume::Xzx), ("x", StretchVolume::X), ("z", StretchVolume::Z), ("none", StretchVolume::None)],
                )?;
            }
            if let Some(text) = params::string(params, "keepAxis")? {
                *keep_axis = parse_choice(text, &[("swingY", KeepAxis::SwingY), ("planeX", KeepAxis::PlaneX), ("planeZ", KeepAxis::PlaneZ)])?;
            }
        }
        ConstraintKind::Floor { target: slot, axis, offset, use_rotation } => {
            if let Some(target) = target {
                *slot = target;
            }
            if let Some(text) = params::string(params, "axis")? {
                *axis = track_axis(text)?;
            }
            if let Some(value) = params::number(params, "offset")? {
                *offset = value;
            }
            if let Some(value) = params::boolean(params, "useRotation")? {
                *use_rotation = value;
            }
        }
        ConstraintKind::Transform {
            target: slot,
            map_from,
            map_to,
            from_min,
            from_max,
            to_min,
            to_max,
            mapping,
            extrapolate,
            mix,
            owner_space,
            target_space,
        } => {
            if let Some(target) = target {
                *slot = target;
            }
            spaces(params, owner_space, target_space)?;
            // A new channel starts from its own neutral range (scale from 1).
            let neutral = |channel: TransformChannel| if channel == TransformChannel::Scale { Vec3::ONE } else { Vec3::ZERO };
            if let Some(text) = params::string(params, "mapFrom")? {
                let channel = transform_channel(text)?;
                if channel != *map_from {
                    (*from_min, *from_max) = (neutral(channel), neutral(channel));
                }
                *map_from = channel;
            }
            if let Some(text) = params::string(params, "mapTo")? {
                let channel = transform_channel(text)?;
                if channel != *map_to {
                    (*to_min, *to_max) = (neutral(channel), neutral(channel));
                    *mix = None;
                }
                *map_to = channel;
            }
            let angle = |channel: TransformChannel, value: Vec3| if channel == TransformChannel::Rotation { radians(value) } else { value };
            for (key, slot, channel) in [
                ("fromMin", &mut *from_min, *map_from),
                ("fromMax", &mut *from_max, *map_from),
                ("toMin", &mut *to_min, *map_to),
                ("toMax", &mut *to_max, *map_to),
            ] {
                if let Some(value) = params::vec3(params, key)? {
                    *slot = angle(channel, value);
                }
            }
            if let Some(value) = params.get("mapping").filter(|value| !value.is_null()) {
                let items = value.as_array().filter(|items| items.len() == 3).ok_or_else(|| bad("`mapping` is three of x, y, z"))?;
                for (slot, item) in mapping.iter_mut().zip(items) {
                    *slot = match item.as_str() {
                        Some("x") => 0,
                        Some("y") => 1,
                        Some("z") => 2,
                        _ => return Err(bad("`mapping` is three of x, y, z")),
                    };
                }
            }
            if let Some(value) = params::boolean(params, "extrapolate")? {
                *extrapolate = value;
            }
            if let Some(text) = params::string(params, "mix")? {
                *mix = Some(parse_choice(
                    text,
                    &[
                        ("add", TransformMix::Add),
                        ("replace", TransformMix::Replace),
                        ("before", TransformMix::Before),
                        ("after", TransformMix::After),
                        ("multiply", TransformMix::Multiply),
                    ],
                )?);
            }
        }
        ConstraintKind::Action { target: slot, action, channel, axis, min, max, frame_start, frame_end, mix, target_space } => {
            if let Some(target) = target {
                *slot = target;
            }
            let mut ignored = Space::World;
            spaces(params, &mut ignored, target_space)?;
            match params.get("action") {
                Some(Value::Null) => *action = None,
                Some(_) => {
                    let name = params::string(params, "action")?.unwrap_or_default();
                    if doc.scene.animation.action(name).is_none() {
                        return Err(format!("not-found: no action {name}"));
                    }
                    *action = Some(name.to_string());
                }
                None => {}
            }
            if let Some(text) = params::string(params, "channel")? {
                (*channel, *axis) = action_channel(text)?;
            }
            let angle = |value: f32| if *channel == TransformChannel::Rotation { value.to_radians() } else { value };
            if let Some(value) = params.get("min").and_then(Value::as_f64) {
                *min = angle(value as f32);
            }
            if let Some(value) = params.get("max").and_then(Value::as_f64) {
                *max = angle(value as f32);
            }
            if let Some(value) = params::number(params, "frameStart")? {
                *frame_start = value;
            }
            if let Some(value) = params::number(params, "frameEnd")? {
                *frame_end = value;
            }
            if let Some(text) = params::string(params, "mix")? {
                *mix = parse_choice(
                    text,
                    &[
                        ("replace", ActionMix::Replace),
                        ("beforeFull", ActionMix::BeforeFull),
                        ("before", ActionMix::Before),
                        ("beforeSplit", ActionMix::BeforeSplit),
                        ("afterFull", ActionMix::AfterFull),
                        ("after", ActionMix::After),
                        ("afterSplit", ActionMix::AfterSplit),
                    ],
                )?;
            }
        }
        ConstraintKind::Armature { targets, preserve_volume } => {
            if let Some(value) = params.get("targets").filter(|value| !value.is_null()) {
                let items = value.as_array().ok_or_else(|| bad("`targets` is a list of { object, bone, weight }"))?;
                let mut list = Vec::with_capacity(items.len());
                for item in items {
                    let object = params::resolve(&doc.scene, item.get("object").unwrap_or(&Value::Null))?;
                    let bone = item.get("bone").and_then(Value::as_str).ok_or_else(|| bad("each armature target names a `bone`"))?;
                    let armature = kition_scene_model::armature::armature_of(&doc.scene, object)
                        .ok_or_else(|| bad("an armature target's object must be an armature"))?;
                    armature.bone(bone).ok_or_else(|| format!("not-found: no bone {bone}"))?;
                    let weight = item.get("weight").and_then(Value::as_f64).unwrap_or(1.0) as f32;
                    list.push(ArmatureTarget { target: Target { object, bone: Some(bone.to_string()) }, weight });
                }
                *targets = list;
            }
            if let Some(value) = params::boolean(params, "preserveVolume")? {
                *preserve_volume = value;
            }
        }
        ConstraintKind::SplineIk { target: slot, chain, even_divisions, chain_offset, curve_radius, y_scale, xz_scale, bulge } => {
            if let Some(target) = target {
                if let Some(target) = &target
                    && !matches!(doc.scene.object(target.object).map(|object| &object.data), Ok(ObjectData::Curve(_)))
                {
                    return Err(bad("Spline IK's target must be a curve"));
                }
                *slot = target;
            }
            if let Some(value) = params::integer(params, "chain")? {
                *chain = value;
            }
            if let Some(value) = params::boolean(params, "evenDivisions")? {
                *even_divisions = value;
            }
            if let Some(value) = params::boolean(params, "chainOffset")? {
                *chain_offset = value;
            }
            if let Some(value) = params::boolean(params, "curveRadius")? {
                *curve_radius = value;
            }
            if let Some(text) = params::string(params, "yScale")? {
                *y_scale = parse_choice(
                    text,
                    &[("none", SplineYScale::None), ("fitCurve", SplineYScale::FitCurve), ("boneOriginal", SplineYScale::BoneOriginal)],
                )?;
            }
            if let Some(text) = params::string(params, "xzScale")? {
                *xz_scale = parse_choice(
                    text,
                    &[
                        ("none", SplineXzScale::None),
                        ("boneOriginal", SplineXzScale::BoneOriginal),
                        ("inversePreserve", SplineXzScale::InversePreserve),
                        ("volumePreserve", SplineXzScale::VolumePreserve),
                    ],
                )?;
            }
            if let Some(value) = params::number(params, "bulge")? {
                *bulge = value;
            }
        }
    }
    constraint.check().map_err(|error| error.to_string())
}

fn parse_choice<T: Copy>(text: &str, choices: &[(&str, T)]) -> Result<T, String> {
    choices.iter().find(|(name, _)| *name == text).map(|(_, value)| *value).ok_or_else(|| bad(format!("unknown choice {text}")))
}

fn space_of(text: &str) -> Result<Space, String> {
    parse_choice(text, &[("world", Space::World), ("local", Space::Local), ("pose", Space::Pose), ("localWithParent", Space::LocalWithParent)])
}

fn space_name(space: Space) -> &'static str {
    match space {
        Space::World => "world",
        Space::Local => "local",
        Space::Pose => "pose",
        Space::LocalWithParent => "localWithParent",
    }
}

/// Reads `ownerSpace` and `targetSpace`.
fn spaces(params: &Value, owner: &mut Space, target: &mut Space) -> Result<(), String> {
    if let Some(text) = params::string(params, "ownerSpace")? {
        *owner = space_of(text)?;
    }
    if let Some(text) = params::string(params, "targetSpace")? {
        *target = space_of(text)?;
    }
    Ok(())
}

fn transform_channel(text: &str) -> Result<TransformChannel, String> {
    parse_choice(text, &[("location", TransformChannel::Location), ("rotation", TransformChannel::Rotation), ("scale", TransformChannel::Scale)])
}

/// `locationX` to (location, 0).
fn action_channel(text: &str) -> Result<(TransformChannel, u8), String> {
    let index = ACTION_CHANNELS.iter().position(|name| *name == text).ok_or_else(|| bad(format!("unknown channel {text}")))?;
    let channel = [TransformChannel::Location, TransformChannel::Rotation, TransformChannel::Scale].get(index / 3).copied().unwrap_or_default();
    Ok((channel, (index % 3) as u8))
}

/// Stretch To with no rest length takes the distance to its target now (in the owner's Y
/// units), so it starts unstretched, as the reference application does.
fn settle_stretch(scene: &kition_scene_model::Scene, object: Id, bone: Option<&str>, constraint: &mut Constraint) {
    if let ConstraintKind::StretchTo { target: Some(target), rest_length, .. } = &mut constraint.kind
        && *rest_length <= 0.0
        && let (Ok(owner), Some(goal)) = (owner_world(scene, object, bone), target_world(scene, target))
    {
        let y = owner.y_axis.truncate().length().max(1e-6);
        *rest_length = owner.w_axis.truncate().distance(goal.w_axis.truncate()) / y;
    }
}

/// The owner's world matrix now (an object's, or a bone's in world space), constraints applied.
fn owner_world(scene: &kition_scene_model::Scene, object: Id, bone: Option<&str>) -> Result<glam::Mat4, String> {
    let world = scene.world_matrix(object).map_err(|error| error.to_string())?;
    match bone {
        Some(name) => Ok(world * *constraints::bone_matrices(scene, object, 0).get(name).ok_or_else(|| format!("not-found: no bone {name}"))?),
        None => Ok(world),
    }
}

/// The world matrix of a constraint's target, if it has one now.
fn target_world(scene: &kition_scene_model::Scene, target: &Target) -> Option<glam::Mat4> {
    owner_world(scene, target.object, target.bone.as_deref()).ok()
}

fn run_change(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    match command {
        "constraint.add" => {
            let (object, bone) = owner(doc, params)?;
            let kind = params::string(params, "type")?.ok_or_else(|| bad("`type` is required"))?;
            let kind = ConstraintKind::default_of(kind).map_err(|error| error.to_string())?;
            if bone.is_none() && kind.is_chain() {
                return Err(bad(format!("{} goes on a bone: name `bone` (or pose a bone)", kind.display_name())));
            }
            let mut constraint = Constraint::new(kind);
            configure(doc, &mut constraint, params)?;
            // Child Of starts where the target is now, so the owner does not move.
            if let ConstraintKind::ChildOf { target: Some(target), inverse, .. } = &mut constraint.kind
                && let Some(world) = target_world(&doc.scene, target)
            {
                *inverse = world.inverse();
            }
            settle_stretch(&doc.scene, object, bone.as_deref(), &mut constraint);
            let existing = stack(&doc.scene, object, bone.as_deref())?;
            let base = params::string(params, "rename")?.unwrap_or(&constraint.name).to_string();
            constraint.name = kition_scene_model::scene::unique_name(&base, |name| existing.iter().any(|other| other.name == name));
            let name = constraint.name.clone();
            let list = stack_mut(&mut doc.scene, object, bone.as_deref())?;
            list.push(constraint);
            Ok(json!({ "object": object, "bone": bone, "index": list.len() - 1, "name": name }))
        }
        "constraint.set" => {
            let (object, bone) = owner(doc, params)?;
            let list = stack(&doc.scene, object, bone.as_deref())?;
            let index = index_of(&list, params)?;
            let mut constraint = list.get(index).cloned().ok_or("not-found: the constraint")?;
            configure(doc, &mut constraint, params)?;
            settle_stretch(&doc.scene, object, bone.as_deref(), &mut constraint);
            if let Some(name) = params::string(params, "rename")? {
                if list.iter().enumerate().any(|(other, item)| other != index && item.name == name) {
                    return Err(bad(format!("a constraint is already called {name}")));
                }
                constraint.name = name.to_string();
            }
            if let Some(slot) = stack_mut(&mut doc.scene, object, bone.as_deref())?.get_mut(index) {
                *slot = constraint;
            }
            Ok(json!({ "object": object, "bone": bone, "index": index }))
        }
        "constraint.remove" => {
            let (object, bone) = owner(doc, params)?;
            let index = index_of(&stack(&doc.scene, object, bone.as_deref())?, params)?;
            stack_mut(&mut doc.scene, object, bone.as_deref())?.remove(index);
            Ok(json!({ "object": object, "bone": bone, "removed": index }))
        }
        "constraint.move" => {
            let (object, bone) = owner(doc, params)?;
            let list = stack(&doc.scene, object, bone.as_deref())?;
            let index = index_of(&list, params)?;
            let to = params::integer(params, "to")?.ok_or_else(|| bad("`to` is required"))? as usize;
            if to >= list.len() {
                return Err(bad(format!("`to` must be below {}", list.len())));
            }
            let stack = stack_mut(&mut doc.scene, object, bone.as_deref())?;
            let constraint = stack.remove(index);
            stack.insert(to, constraint);
            Ok(json!({ "object": object, "bone": bone, "index": to }))
        }
        "constraint.setInverse" | "constraint.clearInverse" => {
            let (object, bone) = owner(doc, params)?;
            let list = stack(&doc.scene, object, bone.as_deref())?;
            let index = index_of(&list, params)?;
            let world = match list.get(index).map(|constraint| &constraint.kind) {
                Some(ConstraintKind::ChildOf { target: Some(target), .. }) if command == "constraint.setInverse" => {
                    target_world(&doc.scene, target).ok_or("not-found: the target")?.inverse()
                }
                Some(ConstraintKind::ChildOf { .. }) => glam::Mat4::IDENTITY,
                _ => return Err(bad("only Child Of has an inverse")),
            };
            if let Some(ConstraintKind::ChildOf { inverse, .. }) =
                stack_mut(&mut doc.scene, object, bone.as_deref())?.get_mut(index).map(|constraint| &mut constraint.kind)
            {
                *inverse = world;
            }
            Ok(json!({ "object": object, "bone": bone, "index": index }))
        }
        "constraint.apply" => {
            let (object, bone) = owner(doc, params)?;
            let index = index_of(&stack(&doc.scene, object, bone.as_deref())?, params)?;
            // This constraint's effect alone, on the owner's own transform, goes into its fields
            // (as a modifier applied out of order acts on the base alone): the constraints left
            // on the stack still act on top, each once.
            let mut trimmed = doc.scene.clone();
            let alone = stack_mut(&mut trimmed, object, bone.as_deref())?;
            let applied = alone.get(index).cloned().ok_or("not-found: the constraint")?;
            *alone = vec![applied];
            let world = owner_world(&trimmed, object, bone.as_deref())?;
            stack_mut(&mut doc.scene, object, bone.as_deref())?.remove(index);
            match bone.as_deref() {
                None => {
                    let parent_space = doc.scene.parent_frame(object).map_err(|error| error.to_string())?;
                    let owner = doc.scene.object_mut(object).map_err(|error| error.to_string())?;
                    owner.set_local_matrix(parent_space.inverse() * world);
                }
                Some(name) => {
                    let rig_world = doc.scene.world_matrix(object).map_err(|error| error.to_string())?;
                    let mut plain = doc.scene.clone();
                    if let Some(armature) = kition_scene_model::armature::armature_mut(&mut plain, object)
                        && let Some(bone) = armature.bone_mut(name)
                    {
                        bone.constraints.clear();
                        bone.pose = Default::default();
                    }
                    // The bone's rest frame as its (evaluated) parent puts it.
                    let rest = rig_world * *constraints::bone_matrices(&plain, object, 0).get(name).ok_or("not-found: the bone")?;
                    let pose = rest.inverse() * world;
                    let (scale, rotation, location) = pose.to_scale_rotation_translation();
                    if let Some(bone) =
                        kition_scene_model::armature::armature_mut(&mut doc.scene, object).and_then(|armature| armature.bone_mut(name))
                    {
                        bone.pose.location = location;
                        bone.pose.rotation = kition_scene_model::scene::matrix_euler(Mat3::from_quat(rotation));
                        bone.pose.scale = scale;
                    }
                }
            }
            Ok(json!({ "object": object, "bone": bone, "applied": index }))
        }
        "paint.stroke" => {
            let points = params.get("points").and_then(Value::as_array).ok_or_else(|| bad("`points` is a list of [x, y]"))?;
            let mut path = Vec::with_capacity(points.len());
            for point in points {
                let pair = point.as_array().filter(|pair| pair.len() == 2).ok_or_else(|| bad("each point is [x, y]"))?;
                let (Some(x), Some(y)) = (pair.first().and_then(Value::as_f64), pair.get(1).and_then(Value::as_f64)) else {
                    return Err(bad("each point is [x, y]"));
                };
                if !(x.is_finite() && y.is_finite()) {
                    return Err(bad("each point is [x, y]"));
                }
                path.push((x, y));
            }
            if let Some(group) = params::string(params, "group")?
                && let Some(paint) = doc.session.paint.as_mut()
            {
                paint.group = Some(group.to_string());
            }
            let mut last = None;
            for point in path {
                paint_along(doc, last, point)?;
                last = Some(point);
            }
            Ok(json!({ "group": paint_group(doc) }))
        }
        "paint.gradient" => {
            let from = pixel(params, "from")?;
            let to = pixel(params, "to")?;
            let kind = match params::string(params, "kind")?.unwrap_or("linear") {
                "linear" => GradientKind::Linear,
                "radial" => GradientKind::Radial,
                other => return Err(bad(format!("unknown gradient {other}"))),
            };
            if let Some(group) = params::string(params, "group")?
                && let Some(paint) = doc.session.paint.as_mut()
            {
                paint.group = Some(group.to_string());
            }
            gradient(doc, from, to, kind)?;
            Ok(json!({ "group": paint_group(doc) }))
        }
        "paint.vertices" => {
            let object = match params.get("object").filter(|value| !value.is_null()) {
                Some(value) => params::resolve(&doc.scene, value)?,
                None => doc.session.paint.as_ref().map(|paint| paint.object).or_else(|| active_mesh(doc)).ok_or("not-allowed: name a mesh")?,
            };
            let group = match params::string(params, "group")? {
                Some(group) => group.to_string(),
                None => paint_group_of(doc, object).unwrap_or_else(|| "Group".into()),
            };
            let vertices = params::indices(params, "vertices")?.ok_or_else(|| bad("`vertices` is required"))?;
            let weight = params::number(params, "weight")?.unwrap_or(1.0);
            let mode = match params::string(params, "mode")?.unwrap_or("draw") {
                "draw" => BrushMode::Draw,
                "add" => BrushMode::Add,
                "subtract" => BrushMode::Subtract,
                other => return Err(bad(format!("unknown mode {other}"))),
            };
            let mesh = doc.scene.object_mut(object).map_err(|error| error.to_string())?.mesh_mut().ok_or_else(|| bad("that object is not a mesh"))?;
            if let Some(bad_vertex) = vertices.iter().find(|vertex| **vertex as usize >= mesh.positions.len()) {
                return Err(format!("not-found: vertex {bad_vertex}"));
            }
            let touches: Vec<(u32, f32)> = vertices.iter().map(|vertex| (*vertex, 1.0)).collect();
            weight_paint::dab(mesh, &group, &touches, &Brush { mode, weight, ..Brush::default() }).map_err(|error| error.to_string())?;
            Ok(json!({ "object": object, "group": group, "vertices": vertices.len() }))
        }
        "paint.normalizeAll" => {
            let object = match params.get("object").filter(|value| !value.is_null()) {
                Some(value) => params::resolve(&doc.scene, value)?,
                None => doc.session.paint.as_ref().map(|paint| paint.object).or_else(|| active_mesh(doc)).ok_or("not-allowed: name a mesh")?,
            };
            let locked = if params::boolean(params, "lockActive")?.unwrap_or(true) { paint_group_of(doc, object) } else { None };
            let mesh = doc.scene.object_mut(object).map_err(|error| error.to_string())?.mesh_mut().ok_or_else(|| bad("that object is not a mesh"))?;
            if mesh.groups.is_empty() {
                return Err("not-allowed: the mesh has no vertex groups".into());
            }
            weight_paint::normalize_all(mesh, locked.as_deref());
            Ok(json!({ "object": object }))
        }
        other => Err(format!("unknown-command: {other}")),
    }
}

/// Dabs the brush along the segment from `from` to `to` (viewport pixels), a dab every quarter
/// radius.
fn paint_along(doc: &mut SceneDocument, from: Option<(f64, f64)>, to: (f64, f64)) -> Result<(), String> {
    let paint = doc.session.paint.clone().ok_or("not-allowed: not in weight paint")?;
    let group = paint_group(doc).unwrap_or_else(|| "Group".into());
    let spacing = f64::from(paint.brush.radius * 0.25).max(1.0);
    let dabs: Vec<(f64, f64)> = match from {
        None => vec![to],
        Some(from) => {
            let length = (to.0 - from.0).hypot(to.1 - from.1);
            let steps = (length / spacing).ceil().clamp(1.0, 500.0) as usize;
            (1..=steps).map(|step| step as f64 / steps as f64).map(|t| (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t)).collect()
        }
    };
    let screen = vertex_screen(doc, paint.object);
    let mirror = paint
        .mirror
        .x
        .then(|| {
            let mesh = doc.scene.object(paint.object).ok()?.mesh()?;
            Some(Mirror { partners: weight_paint::mirror_map(mesh, paint.mirror.topology), names: paint.mirror.groups })
        })
        .flatten();
    for (x, y) in dabs {
        let touches: Vec<(u32, f32)> = screen
            .iter()
            .enumerate()
            .filter_map(|(vertex, place)| {
                let (sx, sy, facing) = (*place)?;
                if paint.brush.front_faces_only && !facing {
                    return None;
                }
                let touch = paint.brush.touch(((sx - x as f32).hypot(sy - y as f32)).abs());
                (touch > 0.0).then_some((vertex as u32, touch))
            })
            .collect();
        let mesh = doc.scene.object_mut(paint.object).map_err(|error| error.to_string())?.mesh_mut().ok_or("unsupported: not a mesh")?;
        match &mirror {
            Some(mirror) => weight_paint::dab_mirrored(mesh, &group, &touches, &paint.brush, mirror),
            None => weight_paint::dab(mesh, &group, &touches, &paint.brush),
        }
        .map_err(|error| error.to_string())?;
    }
    doc.touched();
    Ok(())
}

/// A point in viewport pixels from `[x, y]`.
fn pixel(params: &Value, key: &str) -> Result<(f64, f64), String> {
    let pair = params.get(key).and_then(Value::as_array).filter(|pair| pair.len() == 2).ok_or_else(|| bad(format!("`{key}` is [x, y]")))?;
    match (pair.first().and_then(Value::as_f64), pair.get(1).and_then(Value::as_f64)) {
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() => Ok((x, y)),
        _ => Err(bad(format!("`{key}` is [x, y]"))),
    }
}

/// The gradient tool from `from` to `to` (viewport pixels) on the paint group, with the brush's
/// weight, strength, and falloff.
fn gradient(doc: &mut SceneDocument, from: (f64, f64), to: (f64, f64), kind: GradientKind) -> Result<(), String> {
    let paint = doc.session.paint.clone().ok_or("not-allowed: not in weight paint")?;
    let group = paint_group(doc).unwrap_or_else(|| "Group".into());
    let screen: Vec<Option<(f32, f32)>> = vertex_screen(doc, paint.object)
        .into_iter()
        .map(|place| place.filter(|(_, _, facing)| !paint.brush.front_faces_only || *facing).map(|(x, y, _)| (x, y)))
        .collect();
    let mesh = doc.scene.object_mut(paint.object).map_err(|error| error.to_string())?.mesh_mut().ok_or("unsupported: not a mesh")?;
    weight_paint::gradient(mesh, &group, &screen, (from.0 as f32, from.1 as f32), (to.0 as f32, to.1 as f32), kind, &paint.brush)
        .map_err(|error| error.to_string())?;
    doc.touched();
    Ok(())
}

/// The paint group's weight under pixel (`x`, `y`): of the vertex nearest the point on the face
/// drawn there.
fn sample(doc: &SceneDocument, x: f64, y: f64) -> Option<f32> {
    let paint = doc.session.paint.as_ref()?;
    let hit = doc.with_frame(|cached| kition_scene_raster::hit(&cached.canvas, x.max(0.0) as u32, y.max(0.0) as u32))?;
    if hit.object != paint.object {
        return None;
    }
    let object = doc.scene.object(paint.object).ok()?;
    let own = object.mesh()?;
    let evaluated = object.evaluated_mesh(&doc.scene).filter(|mesh| mesh.positions.len() == own.positions.len());
    let face = evaluated.as_ref().unwrap_or(own).faces.get(hit.face as usize)?.clone();
    let screen = vertex_screen(doc, paint.object);
    let nearest = face.iter().copied().filter_map(|vertex| {
        let (sx, sy, _) = screen.get(vertex as usize).copied().flatten()?;
        Some((f64::from(sx - x as f32).hypot(f64::from(sy - y as f32)), vertex))
    });
    let (_, vertex) = nearest.min_by(|a, b| a.0.total_cmp(&b.0))?;
    let group = paint_group(doc)?;
    Some(own.groups.iter().find(|candidate| candidate.name == group)?.weights.get(vertex as usize).copied().unwrap_or(0.0))
}

/// Each vertex of the object's mesh on screen (as drawn: deformed when its modifiers keep the
/// vertices), and whether its normal faces the eye.
fn vertex_screen(doc: &SceneDocument, object: Id) -> Vec<Option<(f32, f32, bool)>> {
    let Ok(found) = doc.scene.object(object) else { return Vec::new() };
    let Some(own) = found.mesh() else { return Vec::new() };
    let evaluated = found.evaluated_mesh(&doc.scene).filter(|mesh| mesh.positions.len() == own.positions.len());
    let mesh = evaluated.as_ref().unwrap_or(own);
    let Ok(world) = doc.scene.world_matrix(object) else { return Vec::new() };
    let projector = doc.session.editor.view.projector(&doc.scene, doc.session.width, doc.session.height);
    let matrix = projector.view_projection() * world;
    let normals = mesh.vertex_normals();
    let normal_matrix = Mat3::from_mat4(world).inverse().transpose();
    let eye = projector.eye();
    mesh.positions
        .iter()
        .zip(normals.iter())
        .map(|(point, normal)| {
            let clip = matrix * point.extend(1.0);
            if clip.w <= 0.0 || clip.z < 0.0 {
                return None;
            }
            let on_screen = projector.to_screen(clip);
            let world_point = world.transform_point3(*point);
            let toward = if projector.perspective { eye - world_point } else { projector.view.inverse().transform_vector3(Vec3::Z) };
            Some((on_screen.x, on_screen.y, (normal_matrix * *normal).dot(toward) > 0.0))
        })
        .collect()
}

/// The pointer in weight paint. The brush: Down starts a stroke (one undo step), Drag paints
/// along the path, Move shows the brush. A gradient: Down sets its start, Drag fills it again
/// from the weights before it to the pointer (one undo step on Up). Sample Weight: Down sets the
/// brush's weight from the weight under the pointer.
pub fn pointer(doc: &mut SceneDocument, event: &ToolPointer) -> Option<ToolResponse> {
    let paint = doc.session.paint.clone()?;
    let at = (event.x, event.y);
    let circle = |busy: bool, line: Option<(f64, f64)>| {
        let radius = f64::from(paint.brush.radius);
        let d = match (paint.tool, line) {
            (PaintTool::Gradient(_), Some(start)) => format!("M {} {} L {} {}", start.0, start.1, at.0, at.1),
            (PaintTool::Brush, _) => {
                format!("M {} {} a {radius} {radius} 0 1 0 {} 0 a {radius} {radius} 0 1 0 {} 0", at.0 - radius, at.1, radius * 2.0, -radius * 2.0)
            }
            _ => String::new(),
        };
        let overlays = if d.is_empty() { Vec::new() } else { vec![ToolOverlay::Path { d, color: [255, 255, 255, 220], width: 1.5, dashed: false }] };
        ToolResponse { overlays, cursor: "crosshair".into(), busy, ..Default::default() }
    };
    match (event.action, paint.tool) {
        (PointerAction::Down, PaintTool::Sample) => {
            if let (Some(weight), Some(session)) = (sample(doc, at.0, at.1), doc.session.paint.as_mut()) {
                session.brush.weight = weight.clamp(0.0, 1.0);
                doc.touched();
            }
            Some(circle(false, None))
        }
        (PointerAction::Down, tool) => {
            let before = Box::new(doc.step(if tool == PaintTool::Brush { "Paint Weights" } else { "Gradient" }));
            if let Some(session) = doc.session.paint.as_mut() {
                session.stroke = Some(Stroke { before, start: at, last: at, painted: false });
            }
            let painted = tool == PaintTool::Brush && paint_along(doc, None, at).is_ok();
            if let Some(stroke) = doc.session.paint.as_mut().and_then(|session| session.stroke.as_mut()) {
                stroke.painted = painted;
            }
            Some(circle(true, Some(at)))
        }
        (PointerAction::Drag, tool) => {
            let (start, last) = doc.session.paint.as_ref().and_then(|session| session.stroke.as_ref()).map(|stroke| (stroke.start, stroke.last))?;
            let painted = match tool {
                PaintTool::Gradient(kind) => {
                    // Each move fills the gradient again from the weights before it.
                    if let Some(before) =
                        doc.session.paint.as_ref().and_then(|session| session.stroke.as_ref()).map(|stroke| stroke.before.scene.clone())
                    {
                        doc.scene = before;
                    }
                    let far = (at.0 - start.0).hypot(at.1 - start.1) > 1.0;
                    far && gradient(doc, start, at, kind).is_ok()
                }
                _ => paint_along(doc, Some(last), at).is_ok(),
            };
            if let Some(stroke) = doc.session.paint.as_mut().and_then(|session| session.stroke.as_mut()) {
                stroke.last = at;
                stroke.painted |= painted;
            }
            doc.touched();
            Some(circle(true, Some(start)))
        }
        (PointerAction::Up, _) => {
            let stroke = doc.session.paint.as_mut().and_then(|session| session.stroke.take())?;
            if stroke.painted && doc.scene != stroke.before.scene {
                if doc.scene.validate().is_ok() {
                    // One transaction per stroke.
                    let _ = doc.committed(*stroke.before);
                } else {
                    let paint = doc.session.paint.clone();
                    doc.restore(*stroke.before);
                    doc.session.paint = paint;
                    doc.touched();
                }
            }
            Some(circle(false, None))
        }
        (PointerAction::Move | PointerAction::DoubleClick, _) => Some(circle(false, None)),
    }
}

/// Escape during a stroke takes it back.
pub fn cancel(doc: &mut SceneDocument) -> bool {
    let Some(stroke) = doc.session.paint.as_mut().and_then(|session| session.stroke.take()) else { return false };
    let paint = doc.session.paint.clone();
    doc.restore(*stroke.before);
    doc.session.paint = paint;
    doc.touched();
    true
}

fn constraint_json(scene: &kition_scene_model::Scene, constraint: &Constraint) -> Value {
    let mut value = serde_json::to_value(constraint).unwrap_or(Value::Null);
    let name_of = |target: &Target| scene.object(target.object).map(|object| object.name.clone()).unwrap_or_default();
    if let Some(fields) = value.as_object_mut() {
        for key in ["target", "pole"] {
            let target = match (key, &constraint.kind) {
                ("pole", ConstraintKind::Ik { pole, .. }) => pole.as_ref(),
                ("target", kind) => kind.target(),
                _ => None,
            };
            if let Some(target) = target {
                fields.insert(key.into(), json!({ "object": target.object, "name": name_of(target), "bone": target.bone }));
            }
        }
        let degrees = |radians: f64| (radians.to_degrees() * 1000.0).round() / 1000.0;
        if let Some(angle) = fields.get("poleAngle").and_then(Value::as_f64) {
            fields.insert("poleAngle".into(), json!(degrees(angle)));
        }
        for key in ["min", "max"] {
            if let (true, Some(list)) = (matches!(constraint.kind, ConstraintKind::LimitRotation { .. }), fields.get(key).and_then(Value::as_array)) {
                let converted: Vec<f64> = list.iter().filter_map(Value::as_f64).map(degrees).collect();
                fields.insert(key.into(), json!(converted));
            }
        }
        fields.remove("inverse");
        let degrees3 = |value: &Value| value.as_array().map(|list| json!(list.iter().filter_map(Value::as_f64).map(degrees).collect::<Vec<_>>()));
        match &constraint.kind {
            ConstraintKind::Transform { map_from, map_to, mapping, .. } => {
                for (keys, channel) in [(["fromMin", "fromMax"], map_from), (["toMin", "toMax"], map_to)] {
                    if *channel == TransformChannel::Rotation {
                        for key in keys {
                            if let Some(converted) = fields.get(key).and_then(degrees3) {
                                fields.insert(key.into(), converted);
                            }
                        }
                    }
                }
                fields.insert("mapping".into(), json!(mapping.map(|axis| ["x", "y", "z"].get(usize::from(axis)).copied().unwrap_or("x"))));
            }
            ConstraintKind::Action { channel, axis, min, max, .. } => {
                let index = match channel {
                    TransformChannel::Location => 0,
                    TransformChannel::Rotation => 3,
                    TransformChannel::Scale => 6,
                } + usize::from(*axis);
                fields.insert("channel".into(), json!(ACTION_CHANNELS.get(index).copied().unwrap_or("locationX")));
                fields.remove("axis");
                let shown = |value: f32| if *channel == TransformChannel::Rotation { degrees(f64::from(value)) } else { f64::from(value) };
                fields.insert("min".into(), json!(shown(*min)));
                fields.insert("max".into(), json!(shown(*max)));
            }
            ConstraintKind::Armature { targets, .. } => {
                let entries: Vec<Value> = targets
                    .iter()
                    .map(|entry| json!({ "object": entry.target.object, "name": name_of(&entry.target), "bone": entry.target.bone, "weight": entry.weight }))
                    .collect();
                fields.insert("targets".into(), json!(entries));
            }
            _ => {}
        }
        let (owner_space, target_space) = constraint.kind.spaces();
        if matches!(constraint.kind, ConstraintKind::CopyLocation { .. } | ConstraintKind::CopyRotation { .. } | ConstraintKind::Transform { .. }) {
            fields.insert("ownerSpace".into(), json!(space_name(owner_space)));
        }
        if matches!(
            constraint.kind,
            ConstraintKind::CopyLocation { .. }
                | ConstraintKind::CopyRotation { .. }
                | ConstraintKind::Transform { .. }
                | ConstraintKind::Action { .. }
        ) {
            fields.insert("targetSpace".into(), json!(space_name(target_space)));
        }
        let track = match &constraint.kind {
            ConstraintKind::TrackTo { track_axis, .. } | ConstraintKind::DampedTrack { track_axis, .. } => Some(("trackAxis", *track_axis)),
            ConstraintKind::Floor { axis, .. } => Some(("axis", *axis)),
            _ => None,
        };
        if let Some((key, track)) = track {
            let text = match track {
                TrackAxis::X => "x",
                TrackAxis::Y => "y",
                TrackAxis::Z => "z",
                TrackAxis::NegX => "-x",
                TrackAxis::NegY => "-y",
                TrackAxis::NegZ => "-z",
            };
            fields.insert(key.into(), json!(text));
        }
    }
    value
}

/// Adds constraints (objects' and bones') and weight paint to `inspect()`.
pub fn extend_inspect(doc: &SceneDocument, inspected: &mut Value) {
    if let Some(objects) = inspected.get_mut("objects").and_then(Value::as_array_mut) {
        for entry in objects.iter_mut() {
            let Some(id) = entry.get("id").and_then(Value::as_u64).and_then(|id| u32::try_from(id).ok()) else { continue };
            let Ok(object) = doc.scene.object(id) else { continue };
            let Some(map) = entry.as_object_mut() else { continue };
            if !object.constraints.is_empty() {
                map.insert(
                    "constraints".into(),
                    Value::Array(object.constraints.iter().map(|constraint| constraint_json(&doc.scene, constraint)).collect()),
                );
            }
            if let ObjectData::Armature(armature) = &object.data {
                let constrained: serde_json::Map<String, Value> = armature
                    .bones
                    .iter()
                    .filter(|bone| !bone.constraints.is_empty())
                    .map(|bone| {
                        (bone.name.clone(), Value::Array(bone.constraints.iter().map(|constraint| constraint_json(&doc.scene, constraint)).collect()))
                    })
                    .collect();
                if !constrained.is_empty() {
                    map.insert("boneConstraints".into(), Value::Object(constrained));
                }
            }
        }
    }
    if let (Some(paint), Some(mode)) = (&doc.session.paint, inspected.get_mut("mode").and_then(Value::as_object_mut)) {
        mode.insert("mode".into(), json!("weightPaint"));
        mode.insert("object".into(), json!(paint.object));
        mode.insert("paint".into(), json!({ "group": paint_group(doc), "brush": paint.brush, "mirror": paint.mirror, "tool": paint.tool.name() }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_schema_is_well_formed() {
        for spec in SPECS {
            kition_kit::schema::validate(&(spec.schema)()).unwrap_or_else(|error| panic!("{}: {error}", spec.id));
        }
    }
}
