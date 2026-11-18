//! Sculpt Mode: a mesh object's surface shaped with brushes, as the reference application's
//! Sculpt Mode does ([`kition_scene_model::sculpt`]).
//!
//! Sculpt is a mode of a mesh object beside the others ([`SculptSession`]): every tool's pointer
//! sculpts with the brush, a stroke (press to release) is one transaction and one undo step, and
//! the brush circle follows the pointer. Agents sculpt reproducibly with `sculpt.stroke`, giving
//! the path in viewport pixels or as world points. Sculpting replaces the object's mesh: when two
//! people sculpt the same object at once, the last stroke to arrive wins (meshes merge per object,
//! like every other mesh edit).

use glam::{Mat4, Vec3};
use kition_kit::KitCommand;
use kition_kit::tools::{PointerAction, ToolOverlay, ToolPointer, ToolResponse};
use kition_scene_model::Id;
use kition_scene_model::proportional::Falloff;
use kition_scene_model::sculpt::dyntopo::Dyntopo;
use kition_scene_model::sculpt::{self, Brush, BrushKind, Dab, MaskAction, Stroke};
use kition_scene_raster::{NONE, Projector, SculptOverlay};
use serde_json::{Value, json};

use crate::commands_sculpt2::{LiveLevel, SculptTool, ShownCache};
use crate::params::{self, bad, schema as s};
use crate::{Mode, SceneDocument, Step};

/// A mesh in sculpt mode.
#[derive(Debug, Clone)]
pub struct SculptSession {
    pub object: Id,
    pub brush: Brush,
    /// Dynamic topology's settings (in the session, as the reference keeps them in its tool
    /// settings, not the file).
    pub dyntopo: Dyntopo,
    /// The brush, or a box or lasso tool.
    pub tool: SculptTool,
    stroke: Option<Live>,
    /// A box or lasso being drawn: its points in pixels.
    pub gesture: Option<Vec<(f64, f64)>>,
    /// Where the pointer last was (H hides the face set under it).
    pub hover: Option<(f64, f64)>,
    /// What sculpt mode last showed at a multiresolution level.
    pub shown: ShownCache,
}

impl PartialEq for SculptSession {
    fn eq(&self, other: &Self) -> bool {
        self.object == other.object && self.brush == other.brush && self.dyntopo == other.dyntopo && self.tool == other.tool
    }
}

impl SculptSession {
    /// The level a stroke under way is sculpting.
    pub fn live_level(&self) -> Option<&LiveLevel> {
        self.stroke.as_ref().and_then(|live| live.level.as_ref())
    }
}

/// A stroke under way.
#[derive(Debug, Clone)]
struct Live {
    before: Box<Step>,
    stroke: Stroke,
    /// What the viewport showed when the stroke began: where surfaces were, to find the brush's
    /// spot under the pointer without drawing again.
    picture: Picture,
    /// The pointer's last dab, in pixels.
    last: (f64, f64),
    /// Where the first dab touched (world), and its distance in front of the eye: grab and snake
    /// hook drag in the plane through it.
    anchor: Option<(Vec3, f32)>,
    /// Where the previous dab put the dragged point (snake hook).
    dragged: Option<Vec3>,
    /// The multiresolution level the stroke sculpts, when not the mesh itself.
    level: Option<LiveLevel>,
}

#[derive(Debug, Clone)]
pub(crate) struct Picture {
    projector: Projector,
    width: u32,
    height: u32,
    depth: Vec<f32>,
    object: Vec<u32>,
    face: Vec<u32>,
}

impl Picture {
    fn slot(&self, x: f64, y: f64) -> Option<usize> {
        if x < 0.0 || y < 0.0 || x >= f64::from(self.width) || y >= f64::from(self.height) {
            return None;
        }
        Some((y as usize) * self.width as usize + x as usize)
    }

    /// The distance in front of the eye of what is drawn at pixel (`x`, `y`).
    pub(crate) fn depth_at(&self, x: f64, y: f64) -> Option<f32> {
        self.depth.get(self.slot(x, y)?).copied().filter(|depth| depth.is_finite())
    }

    /// The face of `object`'s drawn mesh at pixel (`x`, `y`).
    pub(crate) fn face_at(&self, object: Id, x: f64, y: f64) -> Option<u32> {
        let slot = self.slot(x, y)?;
        if self.object.get(slot).copied().unwrap_or(NONE) != object {
            return None;
        }
        self.face.get(slot).copied().filter(|face| *face != NONE)
    }

    /// The world point of `object`'s surface under pixel (`x`, `y`), if it is there.
    fn surface(&self, object: Id, x: f64, y: f64) -> Option<(Vec3, f32)> {
        if x < 0.0 || y < 0.0 || x >= f64::from(self.width) || y >= f64::from(self.height) {
            return None;
        }
        let slot = (y as usize) * self.width as usize + x as usize;
        if self.object.get(slot).copied().unwrap_or(NONE) != object {
            return None;
        }
        let depth = self.depth.get(slot).copied().filter(|depth| depth.is_finite())?;
        Some((self.at_depth(x, y, depth), depth))
    }

    /// The world point under pixel (`x`, `y`) at `depth` in front of the eye.
    fn at_depth(&self, x: f64, y: f64, depth: f32) -> Vec3 {
        let (origin, direction) = self.projector.ray(x as f32 + 0.5, y as f32 + 0.5);
        let start = self.projector.view.transform_point3(origin).z;
        let step = self.projector.view.transform_vector3(direction).z;
        let t = if step.abs() > 1e-9 { (-depth - start) / step } else { 0.0 };
        origin + direction * t
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Needs {
    /// A mesh in object mode, or sculpt on.
    Sculptable,
    Sculpt,
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

const BRUSHES: &[&str] = &["draw", "clayStrips", "smooth", "grab", "inflate", "crease", "flatten", "pinch", "snakeHook", "mask", "drawFaceSets"];
const TOOLS: &[&str] = &["brush", "boxMask", "lassoMask", "boxHide", "lassoHide", "boxFaceSet", "lassoFaceSet"];
const GESTURES: &[&str] = &["mask", "unmask", "hide", "show", "faceSet"];
const FALLOFFS: &[&str] = &["smooth", "sphere", "root", "sharp", "linear", "constant"];
const SCULPT: &[&str] = &["Sculpt"];

fn brush_fields() -> Vec<(&'static str, Value)> {
    vec![
        ("kind", s::choice("The brush", BRUSHES)),
        ("radius", s::number("Radius in viewport pixels (50)")),
        ("strength", s::number("Strength at the centre, 0 to 1 (0.5; 1 for grab, snake hook, crease, mask, draw face sets)")),
        ("falloff", s::choice("How the brush fades to its edge", FALLOFFS)),
        ("frontFacesOnly", s::boolean("Only vertices facing the view")),
        ("autoSmooth", s::number("Smoothing after each dab, 0 to 1")),
        ("spacing", s::number("Distance between dabs as a share of the radius (0.1)")),
        ("subtract", s::boolean("The other direction: in instead of out, unmask instead of mask")),
        (
            "symmetry",
            json!({ "type": "array", "items": { "type": "boolean" }, "description": "Mirror dabs across the object's own [x, y, z] (x on)" }),
        ),
        ("usePressure", s::boolean("Pen pressure scales the strength (true)")),
    ]
}

static SPECS: &[Spec] = &[
    spec("mode.sculpt", "Sculpt Mode", &["Object"], None, false, Needs::Sculptable, || {
        s::of(vec![("on", s::boolean("Sculpt on or off; toggles when left out")), ("object", s::object("The mesh; the active one when left out"))])
    }),
    spec("sculpt.brush", "Brush Settings", &[], None, false, Needs::Sculpt, || s::of(brush_fields())),
    spec("sculpt.stroke", "Sculpt Stroke", &[], None, true, Needs::Sculpt, || {
        let mut fields = brush_fields();
        fields.push((
            "points",
            s::required(json!({
                "type": "array",
                "items": { "type": "json" },
                "description": "The stroke's path: [x, y] viewport pixels, or [x, y, z] world points on the surface"
            })),
        ));
        fields.push(("pressures", json!({ "type": "array", "items": { "type": "number" }, "description": "Pen pressure at each point, 0 to 1" })));
        fields.push(("worldRadius", s::number("With world points: the radius in metres (0.25)")));
        fields.push(("drag", s::vec3("Grab and snake hook with world points: how far the last point drags, in metres")));
        s::of(fields)
    }),
    spec("sculpt.maskInvert", "Invert Mask", SCULPT, None, true, Needs::Sculpt, || s::of(vec![])),
    spec("sculpt.maskClear", "Clear Mask", SCULPT, None, true, Needs::Sculpt, || s::of(vec![])),
    spec("sculpt.maskFill", "Fill Mask", SCULPT, None, true, Needs::Sculpt, || s::of(vec![])),
    spec("sculpt.maskSmooth", "Smooth Mask", SCULPT, None, true, Needs::Sculpt, || s::of(vec![])),
    spec("sculpt.tool", "Sculpt Tool", &[], None, false, Needs::Sculpt, || {
        s::of(vec![("tool", s::required(s::choice("The brush, or a box or lasso tool that masks, hides, or makes a face set", TOOLS)))])
    }),
    spec("sculpt.dyntopo", "Dynamic Topology", &[], None, true, Needs::Sculpt, || {
        s::of(vec![
            ("enabled", s::boolean("Dynamic topology on (cuts the mesh into triangles) or off")),
            ("mode", s::choice("How the detail size is measured", &["relative", "constant", "brush"])),
            ("detailSize", s::number("Relative: the longest edge in screen pixels (12)")),
            ("resolution", s::number("Constant: edges per metre (12)")),
            ("percent", s::number("Brush: the longest edge as a percentage of the brush radius (25)")),
            ("refine", s::choice("What a dab does to the edges under it", &["subdivideCollapse", "subdivide", "collapse"])),
        ])
    }),
    spec("sculpt.detailFlood", "Detail Flood Fill", SCULPT, None, true, Needs::Sculpt, || s::of(vec![])),
    spec("sculpt.gesture", "Box or Lasso", &[], None, true, Needs::Sculpt, || {
        s::of(vec![
            ("action", s::required(s::choice("What happens to what the shape encloses", GESTURES))),
            ("shape", s::choice("A box (two corners) or a lasso (the outline)", &["box", "lasso"])),
            (
                "points",
                s::required(
                    json!({ "type": "array", "items": { "type": "json" }, "description": "[x, y] viewport pixels: a box's two corners or a lasso's outline" }),
                ),
            ),
            ("outside", s::boolean("Work on what lies outside the shape instead")),
            ("frontFacesOnly", s::boolean("Only what is seen, not what is behind it")),
            ("value", s::number("mask: the mask value, 0 to 1 (1)")),
        ])
    }),
    spec("sculpt.faceSetsFromMask", "Face Set from Masked", SCULPT, None, true, Needs::Sculpt, || s::of(vec![])),
    spec("sculpt.faceSetsFromVisible", "Face Set from Visible", SCULPT, None, true, Needs::Sculpt, || s::of(vec![])),
    spec("sculpt.faceSetsFromLooseParts", "Face Sets from Loose Parts", SCULPT, None, true, Needs::Sculpt, || s::of(vec![])),
    spec("sculpt.faceSetsClear", "Clear Face Sets", SCULPT, None, true, Needs::Sculpt, || s::of(vec![])),
    spec("sculpt.faceSetVisibility", "Face Set Visibility", &[], None, true, Needs::Sculpt, || {
        s::of(vec![
            ("action", s::choice("Hide the set, show only it, show every face, or toggle", &["hide", "isolate", "showAll", "toggle"])),
            ("faceSet", s::integer("The face set's id (0 for faces in none)")),
            ("x", s::number("Or the face set under this viewport pixel: x (the pointer's when left out)")),
            ("y", s::number("and y")),
        ])
    }),
    spec("sculpt.hideFaceSet", "Hide Face Set", SCULPT, Some("H"), true, Needs::Sculpt, || s::of(vec![])),
    spec("sculpt.isolateFaceSet", "Show Only Face Set", SCULPT, Some("Shift+H"), true, Needs::Sculpt, || s::of(vec![])),
    spec("sculpt.showAllFaces", "Show All Faces", SCULPT, Some("Alt+H"), true, Needs::Sculpt, || s::of(vec![])),
    spec("sculpt.remesh", "Voxel Remesh", SCULPT, None, true, Needs::Sculptable, || {
        s::of(vec![
            ("voxelSize", s::number("The size of the voxels the surface is rebuilt from, metres (0.1)")),
            ("object", s::object("The mesh; the sculpted or active one when left out")),
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
    let object_mode = doc.session.mode == Mode::Object && doc.session.rig.is_none() && doc.session.paint.is_none() && doc.session.shape.is_none();
    match needs {
        Needs::Sculptable => doc.session.sculpt.is_some() || (object_mode && active_mesh(doc).is_some()),
        Needs::Sculpt => doc.session.sculpt.is_some(),
    }
}

/// Every sculpt command, with whether it can run now.
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

/// The sculpt commands that cannot run now.
pub fn disabled(doc: &SceneDocument) -> Vec<String> {
    SPECS.iter().filter(|spec| !enabled(doc, spec.needs)).map(|spec| spec.id.to_string()).collect()
}

/// Mode changes elsewhere end sculpt.
const ENDS_SCULPT: &[&str] = &["mode.set", "mode.toggle", "mode.armature", "mode.pose", "mode.weightPaint"];

/// Runs a sculpt command, or answers `None` when it is not one.
pub fn execute(doc: &mut SceneDocument, command: &str, params: &Value) -> Option<Result<Value, String>> {
    if doc.session.sculpt.is_some() && ENDS_SCULPT.contains(&command) {
        doc.session.sculpt = None;
        doc.touched();
    }
    let spec = spec_of(command)?;
    if !params.is_object() && !params.is_null() {
        return Some(Err(bad("the parameters must be an object")));
    }
    // A named mesh stands in for the active one in object mode (or while sculpting), not in another
    // mode: edit mode's selection indexes the mesh's vertices.
    let named = params.get("object").is_some_and(|value| !value.is_null());
    let object_mode = doc.session.mode == Mode::Object && doc.session.rig.is_none() && doc.session.paint.is_none() && doc.session.shape.is_none();
    if !(enabled(doc, spec.needs) || (named && spec.needs == Needs::Sculptable && (object_mode || doc.session.sculpt.is_some()))) {
        return Some(Err(format!("not-allowed: {} cannot run now", spec.title)));
    }
    if command == "sculpt.tool" {
        let result = crate::commands_sculpt2::run_view(doc, params);
        if result.is_ok() {
            doc.touched();
        }
        return Some(result);
    }
    if crate::commands_sculpt2::COMMANDS.contains(&command) {
        return Some(change(doc, spec.title, |doc| crate::commands_sculpt2::run_change(doc, command, params)));
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

/// Ends sculpt when its mesh is gone or no longer a mesh.
pub fn tidy(doc: &mut SceneDocument) {
    if let Some(session) = &doc.session.sculpt
        && !doc.scene.object(session.object).is_ok_and(|object| object.mesh().is_some())
    {
        doc.session.sculpt = None;
    }
}

/// What sculpt mode shows: the sculpted mesh as it is (no modifiers, or its Multires modifier's
/// sculpt level), darkened where masked, coloured by face set, without its hidden faces.
pub fn overlay(doc: &SceneDocument) -> Option<SculptOverlay> {
    let session = doc.session.sculpt.as_ref()?;
    let own = doc.scene.object(session.object).ok()?.mesh()?;
    let shown = crate::commands_sculpt2::shown(doc);
    let mesh = shown.as_deref().unwrap_or(own);
    let faces = mesh.faces.len();
    Some(SculptOverlay {
        object: session.object,
        mask: mesh.mask.clone(),
        face_sets: if mesh.face_sets.len() == faces { mesh.face_sets.clone() } else { Vec::new() },
        hidden: if mesh.hidden.len() == faces { mesh.hidden.clone() } else { Vec::new() },
        mesh: shown,
    })
}

/// Reads brush settings from `params` onto `brush`; a new kind brings its default strength
/// unless one is given.
fn configure(brush: &mut Brush, params: &Value) -> Result<(), String> {
    if let Some(kind) = params::string(params, "kind")? {
        let kind = BrushKind::from_id(kind).ok_or_else(|| bad(format!("unknown brush `{kind}`")))?;
        if kind != brush.kind {
            *brush = Brush { kind, strength: kind.default_strength(), ..*brush };
        }
    }
    if let Some(radius) = params::number(params, "radius")? {
        brush.radius = radius;
    }
    if let Some(strength) = params::number(params, "strength")? {
        brush.strength = strength;
    }
    if let Some(falloff) = params::string(params, "falloff")? {
        brush.falloff = match falloff {
            "smooth" => Falloff::Smooth,
            "sphere" => Falloff::Sphere,
            "root" => Falloff::Root,
            "sharp" => Falloff::Sharp,
            "linear" => Falloff::Linear,
            "constant" => Falloff::Constant,
            other => return Err(bad(format!("unknown falloff `{other}`"))),
        };
    }
    if let Some(on) = params::boolean(params, "frontFacesOnly")? {
        brush.front_faces_only = on;
    }
    if let Some(amount) = params::number(params, "autoSmooth")? {
        brush.auto_smooth = amount;
    }
    if let Some(spacing) = params::number(params, "spacing")? {
        brush.spacing = spacing;
    }
    if let Some(on) = params::boolean(params, "subtract")? {
        brush.subtract = on;
    }
    if let Some(on) = params::boolean(params, "usePressure")? {
        brush.use_pressure = on;
    }
    if let Some(list) = params.get("symmetry").filter(|value| !value.is_null()) {
        let axes: Vec<bool> = list.as_array().map(|items| items.iter().filter_map(Value::as_bool).collect()).unwrap_or_default();
        let [x, y, z] = axes.as_slice() else { return Err(bad("`symmetry` is [x, y, z] booleans")) };
        brush.symmetry = [*x, *y, *z];
    }
    brush.check().map_err(|error| error.to_string())
}

fn run_view(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    match command {
        "mode.sculpt" => {
            let on = params::boolean(params, "on")?.unwrap_or(doc.session.sculpt.is_none());
            if !on {
                doc.session.sculpt = None;
                return Ok(json!({ "mode": "object" }));
            }
            let object = match params.get("object").filter(|value| !value.is_null()) {
                Some(value) => params::resolve(&doc.scene, value)?,
                None => active_mesh(doc).ok_or("not-allowed: select a mesh to sculpt")?,
            };
            if doc.scene.object(object).ok().and_then(|found| found.mesh()).is_none() {
                return Err(bad("only a mesh can be sculpted"));
            }
            doc.session.mode = Mode::Object;
            doc.session.paint = None;
            doc.session.rig = None;
            doc.session.shape = None;
            doc.session.editor.selected = vec![object];
            doc.session.editor.active = Some(object);
            let (brush, dyntopo) = doc.session.sculpt.as_ref().map(|session| (session.brush, session.dyntopo)).unwrap_or_default();
            doc.session.sculpt = Some(SculptSession {
                object,
                brush,
                dyntopo,
                tool: SculptTool::Brush,
                stroke: None,
                gesture: None,
                hover: None,
                shown: ShownCache::default(),
            });
            Ok(json!({ "mode": "sculpt", "object": object }))
        }
        "sculpt.brush" => {
            let session = doc.session.sculpt.as_mut().ok_or("not-allowed: not in sculpt mode")?;
            let mut brush = session.brush;
            configure(&mut brush, params)?;
            session.brush = brush;
            Ok(json!({ "brush": brush_json(&brush) }))
        }
        other => Err(format!("unknown-command: {other}")),
    }
}

fn run_change(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    let mask_action = match command {
        "sculpt.maskInvert" => Some(MaskAction::Invert),
        "sculpt.maskClear" => Some(MaskAction::Clear),
        "sculpt.maskFill" => Some(MaskAction::Fill),
        "sculpt.maskSmooth" => Some(MaskAction::Smooth),
        _ => None,
    };
    if let Some(action) = mask_action {
        let object = doc.session.sculpt.as_ref().map(|session| session.object).ok_or("not-allowed: not in sculpt mode")?;
        let mesh = doc.scene.object_mut(object).map_err(|error| error.to_string())?.mesh_mut().ok_or("unsupported: not a mesh")?;
        sculpt::mask(mesh, action);
        return Ok(json!({ "object": object, "masked": mesh.mask.iter().filter(|value| **value > 0.0).count() }));
    }
    match command {
        "sculpt.remesh" => {
            let object = match params.get("object").filter(|value| !value.is_null()) {
                Some(value) => params::resolve(&doc.scene, value)?,
                None => doc.session.sculpt.as_ref().map(|session| session.object).or_else(|| active_mesh(doc)).ok_or("not-allowed: select a mesh")?,
            };
            let voxel = params::number(params, "voxelSize")?.unwrap_or(0.1);
            let mesh = doc.scene.object(object).map_err(|error| error.to_string())?.mesh().ok_or_else(|| bad("only a mesh can be remeshed"))?;
            let rebuilt = sculpt::remesh::voxel_remesh(mesh, voxel).map_err(|error| error.to_string())?;
            let (vertices, faces) = (rebuilt.positions.len(), rebuilt.faces.len());
            if let Some(target) = doc.scene.object_mut(object).map_err(|error| error.to_string())?.mesh_mut() {
                *target = rebuilt;
            }
            Ok(json!({ "object": object, "vertices": vertices, "faces": faces }))
        }
        "sculpt.stroke" => stroke_command(doc, params),
        other => Err(format!("unknown-command: {other}")),
    }
}

fn brush_json(brush: &Brush) -> Value {
    let mut value = serde_json::to_value(brush).unwrap_or(Value::Null);
    if let Some(fields) = value.as_object_mut() {
        fields.insert("kind".into(), json!(brush.kind.id()));
    }
    value
}

/// The object's world matrix and how much it scales lengths on average.
fn placement(doc: &SceneDocument, object: Id) -> Result<(Mat4, f32), String> {
    let world = doc.scene.world_matrix(object).map_err(|error| error.to_string())?;
    let scale = (world.x_axis.truncate().length() + world.y_axis.truncate().length() + world.z_axis.truncate().length()) / 3.0;
    Ok((world, scale.max(1e-6)))
}

/// What the viewport shows now, for finding the brush's spot under the pointer.
pub(crate) fn picture(doc: &SceneDocument) -> Picture {
    let projector = doc.session.editor.view.projector(&doc.scene, doc.session.width, doc.session.height);
    doc.with_frame(|cached| Picture {
        projector: projector.clone(),
        width: cached.width,
        height: cached.height,
        depth: cached.canvas.depth.clone(),
        object: cached.canvas.object.clone(),
        face: cached.canvas.face.clone(),
    })
}

/// Starts a stroke on the sculpted mesh (or its Multires modifier's sculpt level) with `brush`.
fn begin(doc: &mut SceneDocument, brush: Brush, label: &str) -> Result<Live, String> {
    let session = doc.session.sculpt.as_ref().ok_or("not-allowed: not in sculpt mode")?;
    let (object, dyntopo) = (session.object, session.dyntopo);
    let at = crate::commands_sculpt2::sculpt_level(doc, object);
    if at > 0 && matches!(brush.kind, BrushKind::Mask | BrushKind::DrawFaceSets) {
        return Err("not-allowed: the mask and face sets are painted at sculpt level 0 (the Multires modifier's sculpt level)".into());
    }
    let mesh = doc.scene.object(object).map_err(|error| error.to_string())?.mesh().ok_or("unsupported: not a mesh")?;
    let level = if at > 0 { Some(crate::commands_sculpt2::working(mesh, at).ok_or("unsupported: the sculpt level cannot be made")?) } else { None };
    let target = level.as_ref().map_or(mesh, |level| &level.mesh);
    let bounds = target.bounds(Mat4::IDENTITY);
    // A cell about a brush wide for a typical brush on this mesh.
    let cell = (bounds.size().max_element() / 20.0).max(1e-3);
    let mut stroke = Stroke::new(target, brush, cell);
    if at == 0 && dyntopo.enabled && !crate::commands_sculpt2::has_levels(doc, object) {
        stroke.dyntopo = Some(dyntopo);
    }
    let before = Box::new(doc.step(label));
    Ok(Live { before, stroke, picture: picture(doc), last: (0.0, 0.0), anchor: None, dragged: None, level })
}

/// Applies one dab to what the stroke sculpts: the mesh, or the level (whose offsets go back on
/// the mesh).
fn apply_dab(doc: &mut SceneDocument, object: Id, live: &mut Live, dab: &Dab) -> Result<(), String> {
    match live.level.as_mut() {
        Some(level) => {
            live.stroke.dab(&mut level.mesh, dab);
            crate::commands_sculpt2::store(doc, object, level, &live.stroke.touched)
        }
        None => {
            let mesh = doc.scene.object_mut(object).map_err(|error| error.to_string())?.mesh_mut().ok_or("unsupported: not a mesh")?;
            live.stroke.dab(mesh, dab);
            Ok(())
        }
    }
}

fn stroke_result(object: Id, live: &Live) -> Value {
    json!({
        "object": object,
        "touched": live.stroke.touched.len(),
        "level": live.level.as_ref().map_or(0, |level| level.at),
        "split": live.stroke.refined.0,
        "collapsed": live.stroke.refined.1,
    })
}

/// One dab at pixel (`x`, `y`) of a stroke under way.
fn dab_at_pixel(doc: &mut SceneDocument, live: &mut Live, x: f64, y: f64, pressure: f32) -> Result<bool, String> {
    let Some(object) = doc.session.sculpt.as_ref().map(|session| session.object) else { return Ok(false) };
    let brush = live.stroke.brush;
    let (world, scale) = placement(doc, object)?;
    let inverse = world.inverse();
    let surface = live.picture.surface(object, x, y);
    // Grab and snake hook drag in the plane through where they took hold; the others need the
    // surface under the pointer.
    let point = match (brush.kind.drags(), surface, live.anchor) {
        (_, Some((point, depth)), None) => {
            live.anchor = Some((point, depth));
            point
        }
        (true, _, Some((_, depth))) => live.picture.at_depth(x, y, depth),
        (false, Some((point, _)), Some(_)) => point,
        _ => return Ok(false),
    };
    let radius_world = live.picture.projector.pixel_size_at(point) * brush.radius;
    let eye = live.picture.projector.eye();
    let toward_eye = if live.picture.projector.perspective { eye - point } else { live.picture.projector.view.inverse().transform_vector3(Vec3::Z) };
    let strength = brush.strength * if brush.use_pressure { pressure.clamp(0.0, 1.0) } else { 1.0 };
    let (center_world, drag_world) = match brush.kind {
        BrushKind::Grab => {
            let start = live.anchor.map(|(start, _)| start).unwrap_or(point);
            (start, point - start)
        }
        BrushKind::SnakeHook => {
            let previous = live.dragged.unwrap_or(point);
            live.dragged = Some(point);
            (previous, point - previous)
        }
        _ => (point, Vec3::ZERO),
    };
    let dab = Dab {
        center: inverse.transform_point3(center_world),
        radius: radius_world / scale,
        strength,
        toward_eye: inverse.transform_vector3(toward_eye).normalize_or_zero(),
        drag: inverse.transform_vector3(drag_world),
    };
    apply_dab(doc, object, live, &dab)?;
    Ok(true)
}

/// Dabs along the pixels from `from` to `to`, `spacing` of the radius apart.
fn dab_along(doc: &mut SceneDocument, live: &mut Live, from: Option<(f64, f64)>, to: (f64, f64), pressure: f32) -> Result<bool, String> {
    let brush = live.stroke.brush;
    let step = f64::from(brush.radius * brush.spacing).max(1.0);
    let mut any = false;
    match from {
        None => any |= dab_at_pixel(doc, live, to.0, to.1, pressure)?,
        Some(from) => {
            let length = (to.0 - from.0).hypot(to.1 - from.1);
            let steps = (length / step).floor().clamp(0.0, 2000.0) as usize;
            if steps == 0 {
                return Ok(false);
            }
            for index in 1..=steps {
                let t = (index as f64 * step / length).min(1.0);
                any |= dab_at_pixel(doc, live, from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t, pressure)?;
            }
        }
    }
    live.last = match from {
        Some(from) => {
            let length = (to.0 - from.0).hypot(to.1 - from.1).max(1e-9);
            let travelled = (length / step).floor() * step;
            (from.0 + (to.0 - from.0) * travelled / length, from.1 + (to.1 - from.1) * travelled / length)
        }
        None => to,
    };
    Ok(any)
}

/// `sculpt.stroke`: a whole stroke from points, as one undo step.
fn stroke_command(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let mut brush = doc.session.sculpt.as_ref().map(|session| session.brush).ok_or("not-allowed: not in sculpt mode")?;
    configure(&mut brush, params)?;
    let points = params.get("points").and_then(Value::as_array).ok_or_else(|| bad("`points` is a list of [x, y] or [x, y, z]"))?;
    if points.is_empty() || points.len() > 10_000 {
        return Err(bad("a stroke has 1 to 10000 points"));
    }
    let pressures: Vec<f32> = params
        .get("pressures")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_f64).map(|value| value as f32).collect())
        .unwrap_or_default();
    let mut parsed: Vec<Vec<f64>> = Vec::with_capacity(points.len());
    for point in points {
        let numbers: Vec<f64> = point.as_array().map(|items| items.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
        if !(numbers.len() == 2 || numbers.len() == 3) || numbers.iter().any(|value| !value.is_finite()) {
            return Err(bad("each point is [x, y] pixels or [x, y, z] world"));
        }
        parsed.push(numbers);
    }
    let world_points = parsed.first().is_some_and(|first| first.len() == 3);
    if parsed.iter().any(|point| (point.len() == 3) != world_points) {
        return Err(bad("the points are all pixels or all world points"));
    }
    let object = doc.session.sculpt.as_ref().map(|session| session.object).ok_or("not-allowed: not in sculpt mode")?;
    let mut live = begin(doc, brush, "Sculpt")?;
    let pressure_at = |index: usize| pressures.get(index).copied().unwrap_or(1.0);
    if world_points {
        // World points: dabs at each point (and along the way, `spacing` apart).
        let (world, scale) = placement(doc, object)?;
        let inverse = world.inverse();
        let radius = params::number(params, "worldRadius")?.unwrap_or(0.25);
        if !(radius > 0.0 && radius.is_finite()) {
            return Err(bad("`worldRadius` must be positive"));
        }
        let drag = params::vec3(params, "drag")?.unwrap_or(Vec3::ZERO);
        let eye = doc.session.editor.view.projector(&doc.scene, doc.session.width, doc.session.height).eye();
        let mut centers = Vec::new();
        let at = |point: &Vec<f64>| {
            Vec3::new(
                point.first().copied().unwrap_or(0.0) as f32,
                point.get(1).copied().unwrap_or(0.0) as f32,
                point.get(2).copied().unwrap_or(0.0) as f32,
            )
        };
        for (index, point) in parsed.iter().enumerate() {
            let here = at(point);
            if let Some(previous) = index.checked_sub(1).and_then(|previous| parsed.get(previous)).map(at) {
                let length = previous.distance(here);
                let steps = (length / (radius * brush.spacing).max(1e-4)).floor().clamp(0.0, 2000.0) as usize;
                for step in 1..steps {
                    centers.push((previous.lerp(here, step as f32 / steps as f32), pressure_at(index)));
                }
            }
            centers.push((here, pressure_at(index)));
        }
        let total = centers.len().max(1);
        for (number, (center, pressure)) in centers.into_iter().enumerate() {
            let strength = brush.strength * if brush.use_pressure { pressure.clamp(0.0, 1.0) } else { 1.0 };
            // Grab drags the whole way by the last point; snake hook a share at every dab.
            let drag_here = match brush.kind {
                BrushKind::Grab => drag * ((number + 1) as f32 / total as f32),
                BrushKind::SnakeHook => drag / total as f32,
                _ => Vec3::ZERO,
            };
            let dab = Dab {
                center: inverse.transform_point3(center),
                radius: radius / scale,
                strength,
                toward_eye: inverse.transform_vector3(eye - center).normalize_or_zero(),
                drag: inverse.transform_vector3(drag_here),
            };
            apply_dab(doc, object, &mut live, &dab)?;
        }
        return Ok(stroke_result(object, &live));
    }
    // Viewport pixels: the same path the pointer takes.
    let mut previous = None;
    for (index, point) in parsed.iter().enumerate() {
        let at = (point.first().copied().unwrap_or(0.0), point.get(1).copied().unwrap_or(0.0));
        dab_along(doc, &mut live, previous, at, pressure_at(index))?;
        previous = Some(live.last);
    }
    Ok(stroke_result(object, &live))
}

/// The pointer in sculpt mode: every tool sculpts. Down starts a stroke (one undo step), Drag
/// dabs along the path, Up commits, Move shows the brush.
pub fn pointer(doc: &mut SceneDocument, event: &ToolPointer) -> Option<ToolResponse> {
    if let Some(current) = doc.session.sculpt.as_mut() {
        current.hover = Some((event.x, event.y));
    }
    if let Some(response) = crate::commands_sculpt2::pointer(doc, event) {
        return Some(response);
    }
    let session = doc.session.sculpt.clone()?;
    let at = (event.x, event.y);
    let subtract = event.mods.command;
    let circle = |busy: bool| {
        let radius = f64::from(session.brush.radius);
        let d = format!("M {} {} a {radius} {radius} 0 1 0 {} 0 a {radius} {radius} 0 1 0 {} 0", at.0 - radius, at.1, radius * 2.0, -radius * 2.0);
        let color = if session.brush.kind == BrushKind::Mask { [40, 40, 40, 230] } else { [255, 255, 255, 230] };
        ToolResponse {
            overlays: vec![ToolOverlay::Path { d, color, width: 1.5, dashed: subtract }],
            cursor: "crosshair".into(),
            busy,
            ..Default::default()
        }
    };
    let pressure = if event.pressure.is_finite() && event.pressure > 0.0 { event.pressure } else { 1.0 };
    match event.action {
        PointerAction::Down => {
            let mut brush = session.brush;
            // Cmd held flips the brush's direction for this stroke.
            brush.subtract ^= subtract;
            let mut live = begin(doc, brush, "Sculpt").ok()?;
            let _ = dab_along(doc, &mut live, None, at, pressure);
            doc.touched();
            if let Some(current) = doc.session.sculpt.as_mut() {
                current.stroke = Some(live);
            }
            Some(circle(true))
        }
        PointerAction::Drag => {
            let mut live = doc.session.sculpt.as_mut().and_then(|current| current.stroke.take())?;
            let last = live.last;
            if dab_along(doc, &mut live, Some(last), at, pressure).unwrap_or(false) {
                doc.touched();
            }
            if let Some(current) = doc.session.sculpt.as_mut() {
                current.stroke = Some(live);
            }
            Some(circle(true))
        }
        PointerAction::Up => {
            let live = doc.session.sculpt.as_mut().and_then(|current| current.stroke.take())?;
            if doc.scene != live.before.scene {
                if doc.scene.validate().is_ok() {
                    // One transaction per stroke.
                    let _ = doc.committed(*live.before);
                } else {
                    let keep = doc.session.sculpt.clone();
                    doc.restore(*live.before);
                    doc.session.sculpt = keep;
                    doc.touched();
                }
            }
            Some(circle(false))
        }
        PointerAction::Move | PointerAction::DoubleClick => Some(circle(false)),
    }
}

/// Escape during a stroke takes it back.
pub fn cancel(doc: &mut SceneDocument) -> bool {
    let Some(live) = doc.session.sculpt.as_mut().and_then(|current| current.stroke.take()) else { return false };
    let keep = doc.session.sculpt.clone();
    doc.restore(*live.before);
    doc.session.sculpt = keep;
    doc.touched();
    true
}

/// Adds sculpt mode to `inspect()`.
pub fn extend_inspect(doc: &SceneDocument, inspected: &mut Value) {
    let (Some(session), Some(mode)) = (&doc.session.sculpt, inspected.get_mut("mode").and_then(Value::as_object_mut)) else { return };
    mode.insert("mode".into(), json!("sculpt"));
    mode.insert("object".into(), json!(session.object));
    let masked = doc
        .scene
        .object(session.object)
        .ok()
        .and_then(|object| object.mesh())
        .map_or(0, |mesh| mesh.mask.iter().filter(|value| **value > 0.0).count());
    let mut sculpt = serde_json::Map::new();
    sculpt.insert("brush".into(), brush_json(&session.brush));
    sculpt.insert("masked".into(), json!(masked));
    sculpt.insert("stroking".into(), json!(session.stroke.is_some()));
    sculpt.extend(crate::commands_sculpt2::inspect(doc).unwrap_or_default());
    mode.insert("sculpt".into(), Value::Object(sculpt));
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
