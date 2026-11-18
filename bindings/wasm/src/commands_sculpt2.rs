//! Sculpt mode's topology and visibility tools ([`kition_scene_model::sculpt::dyntopo`],
//! [`kition_scene_model::sculpt::face_sets`], [`kition_scene_model::multires`]): dynamic
//! topology, face sets and hiding, the box and lasso gestures, and sculpting a Multiresolution
//! modifier's level.
//!
//! The box and lasso tools take the pointer in place of the brush: a drag draws the box or the
//! lasso, and letting go masks, hides, or makes a face set of what it encloses (Cmd held does
//! the opposite: unmask, show). H hides the face set under the pointer, Shift+H shows only it,
//! and Alt+H shows every face, as in the reference application (the commands' shortcuts; the
//! pointer is where it last moved over the viewport).
//!
//! With a Multiresolution modifier, sculpt mode shows and sculpts the modifier's sculpt level:
//! a stroke moves that level's vertices, and the level's offsets are stored back on the mesh
//! after every dab. Masks, face sets, and hiding stay on the base mesh: they are painted and
//! changed at level 0 and carried up to the level shown.

use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use glam::Vec3;
use kition_kit::KitDocument;
use kition_kit::tools::{PointerAction, ToolOverlay, ToolPointer, ToolResponse};
use kition_scene_model::multires::{self, Level};
use kition_scene_model::sculpt::dyntopo::{self, DetailMode, Dyntopo, Refine};
use kition_scene_model::sculpt::face_sets::{self, Gesture, Visibility};
use kition_scene_model::{Id, Mesh, ModifierKind};
use serde_json::{Value, json};

use crate::SceneDocument;
use crate::params::{self, bad};

/// What the pointer does in sculpt mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SculptTool {
    /// Sculpts with the brush.
    #[default]
    Brush,
    BoxMask,
    LassoMask,
    BoxHide,
    LassoHide,
    BoxFaceSet,
    LassoFaceSet,
}

impl SculptTool {
    pub const ALL: [SculptTool; 7] = [
        SculptTool::Brush,
        SculptTool::BoxMask,
        SculptTool::LassoMask,
        SculptTool::BoxHide,
        SculptTool::LassoHide,
        SculptTool::BoxFaceSet,
        SculptTool::LassoFaceSet,
    ];

    pub fn id(self) -> &'static str {
        match self {
            SculptTool::Brush => "brush",
            SculptTool::BoxMask => "boxMask",
            SculptTool::LassoMask => "lassoMask",
            SculptTool::BoxHide => "boxHide",
            SculptTool::LassoHide => "lassoHide",
            SculptTool::BoxFaceSet => "boxFaceSet",
            SculptTool::LassoFaceSet => "lassoFaceSet",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.id() == id)
    }

    fn lasso(self) -> bool {
        matches!(self, SculptTool::LassoMask | SculptTool::LassoHide | SculptTool::LassoFaceSet)
    }

    /// The gesture's action, or its opposite with Cmd held.
    fn action(self, opposite: bool) -> Option<&'static str> {
        match self {
            SculptTool::Brush => None,
            SculptTool::BoxMask | SculptTool::LassoMask => Some(if opposite { "unmask" } else { "mask" }),
            SculptTool::BoxHide | SculptTool::LassoHide => Some(if opposite { "show" } else { "hide" }),
            SculptTool::BoxFaceSet | SculptTool::LassoFaceSet => Some("faceSet"),
        }
    }
}

/// The level a stroke is sculpting, and its vertices as they are now.
#[derive(Debug, Clone)]
pub struct LiveLevel {
    pub at: u32,
    pub level: Level,
    pub mesh: Mesh,
}

/// What sculpt mode last showed at a multiresolution level, by the key of what it came from.
pub type ShownCache = Arc<Mutex<Option<(u64, Arc<Mesh>)>>>;

/// The level sculpt mode works on: the first enabled Multiresolution modifier's sculpt level (at
/// most the mesh's levels), else 0.
pub fn sculpt_level(doc: &SceneDocument, object: Id) -> u32 {
    let Ok(found) = doc.scene.object(object) else { return 0 };
    let Some(mesh) = found.mesh() else { return 0 };
    let wanted = found
        .modifiers
        .iter()
        .filter(|modifier| modifier.enabled)
        .find_map(|modifier| match modifier.kind {
            ModifierKind::Multires { sculpt_levels, .. } => Some(sculpt_levels),
            _ => None,
        })
        .unwrap_or(0);
    wanted.min(multires::levels(mesh))
}

/// Whether the object's mesh has multiresolution levels: dynamic topology never runs on it
/// (enabling it, a stroke, a detail flood), whatever level is sculpted.
pub fn has_levels(doc: &SceneDocument, object: Id) -> bool {
    doc.scene.object(object).ok().and_then(|found| found.mesh()).is_some_and(|mesh| multires::levels(mesh) > 0)
}

/// The level `at` of `mesh` to sculpt, with the base's mask, face sets, and hidden faces carried
/// up to it.
pub fn working(mesh: &Mesh, at: u32) -> Option<LiveLevel> {
    let level = multires::level(mesh, at)?;
    let mut shown = level.mesh.clone();
    let origins = multires::face_origins(mesh, at);
    if mesh.face_sets.len() == mesh.faces.len() {
        shown.face_sets = multires::carry_faces(&mesh.face_sets, &origins);
    }
    if mesh.hidden.len() == mesh.faces.len() {
        shown.hidden = multires::carry_faces(&mesh.hidden, &origins);
    }
    shown.mask = multires::carry_vertices(mesh, at, &mesh.mask);
    if shown.mask.iter().all(|value| *value == 0.0) {
        shown.mask.clear();
    }
    face_sets::tidy(&mut shown);
    Some(LiveLevel { at, level, mesh: shown })
}

fn shown_key(mesh: &Mesh, at: u32) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    at.hash(&mut hasher);
    for point in mesh.positions.iter().chain(mesh.multires.iter().flatten()) {
        point.to_array().map(f32::to_bits).hash(&mut hasher);
    }
    mesh.faces.hash(&mut hasher);
    mesh.face_sets.hash(&mut hasher);
    mesh.hidden.hash(&mut hasher);
    for value in &mesh.mask {
        value.to_bits().hash(&mut hasher);
    }
    mesh.smooth.hash(&mut hasher);
    hasher.finish()
}

/// What sculpt mode shows of the sculpted object when it is not its own mesh: the sculpt level
/// (the stroke's when one is under way).
pub fn shown(doc: &SceneDocument) -> Option<Arc<Mesh>> {
    let session = doc.session.sculpt.as_ref()?;
    if let Some(live) = session.live_level() {
        return Some(Arc::new(live.mesh.clone()));
    }
    let at = sculpt_level(doc, session.object);
    if at == 0 {
        return None;
    }
    let mesh = doc.scene.object(session.object).ok()?.mesh()?;
    let key = shown_key(mesh, at);
    let mut cache = session.shown.lock().ok()?;
    if let Some((stored, shown)) = cache.as_ref()
        && *stored == key
    {
        return Some(Arc::clone(shown));
    }
    let made = Arc::new(working(mesh, at)?.mesh);
    *cache = Some((key, Arc::clone(&made)));
    Some(made)
}

/// Stores a stroke's level back on the mesh: the offsets of the vertices it touched.
pub fn store(doc: &mut SceneDocument, object: Id, live: &LiveLevel, touched: &std::collections::BTreeSet<u32>) -> Result<(), String> {
    let mesh = doc.scene.object_mut(object).map_err(|error| error.to_string())?.mesh_mut().ok_or("unsupported: not a mesh")?;
    multires::store_some(mesh, live.at, &live.level, &live.mesh.positions, touched.iter().copied());
    Ok(())
}

/// Commands this module runs (all mutate but `sculpt.tool`).
pub const COMMANDS: &[&str] = &[
    "sculpt.tool",
    "sculpt.dyntopo",
    "sculpt.detailFlood",
    "sculpt.gesture",
    "sculpt.faceSetsFromMask",
    "sculpt.faceSetsFromVisible",
    "sculpt.faceSetsFromLooseParts",
    "sculpt.faceSetsClear",
    "sculpt.faceSetVisibility",
    "sculpt.hideFaceSet",
    "sculpt.isolateFaceSet",
    "sculpt.showAllFaces",
];

fn sculpted(doc: &SceneDocument) -> Result<Id, String> {
    doc.session.sculpt.as_ref().map(|session| session.object).ok_or_else(|| "not-allowed: not in sculpt mode".to_string())
}

fn base_mesh(doc: &mut SceneDocument, object: Id) -> Result<&mut Mesh, String> {
    doc.scene.object_mut(object).map_err(|error| error.to_string())?.mesh_mut().ok_or_else(|| "unsupported: not a mesh".to_string())
}

/// Refuses work that changes the base mesh's topology or masks while sculpting a level.
fn at_base(doc: &SceneDocument, object: Id, what: &str) -> Result<(), String> {
    if sculpt_level(doc, object) > 0 {
        return Err(format!("not-allowed: {what} works at sculpt level 0 (the Multires modifier's sculpt level)"));
    }
    Ok(())
}

/// `sculpt.tool` (no change to the document).
pub fn run_view(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let name = params::string(params, "tool")?.ok_or_else(|| bad("`tool` is required"))?;
    let tool = SculptTool::from_id(name).ok_or_else(|| bad(format!("unknown sculpt tool `{name}`")))?;
    let session = doc.session.sculpt.as_mut().ok_or("not-allowed: not in sculpt mode")?;
    session.tool = tool;
    session.gesture = None;
    Ok(json!({ "tool": tool.id() }))
}

/// Runs a mutating command of this module (inside the caller's transaction).
pub fn run_change(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    let object = sculpted(doc)?;
    match command {
        "sculpt.dyntopo" => dyntopo_command(doc, object, params),
        "sculpt.detailFlood" => detail_flood(doc, object),
        "sculpt.gesture" => gesture_command(doc, object, params),
        "sculpt.faceSetsFromMask" | "sculpt.faceSetsFromVisible" => {
            let mesh = base_mesh(doc, object)?;
            let (id, faces) = if command == "sculpt.faceSetsFromMask" { face_sets::from_mask(mesh) } else { face_sets::from_visible(mesh) }
                .map_err(|error| format!("not-allowed: {}", error.to_string().replace("invalid-params: ", "")))?;
            Ok(json!({ "object": object, "faceSet": id, "faces": faces }))
        }
        "sculpt.faceSetsFromLooseParts" => {
            let mesh = base_mesh(doc, object)?;
            Ok(json!({ "object": object, "faceSets": face_sets::from_loose_parts(mesh) }))
        }
        "sculpt.faceSetsClear" => {
            face_sets::clear(base_mesh(doc, object)?);
            Ok(json!({ "object": object }))
        }
        "sculpt.faceSetVisibility" => visibility_command(doc, object, params),
        "sculpt.hideFaceSet" => visibility_command(doc, object, &json!({ "action": "hide" })),
        "sculpt.isolateFaceSet" => visibility_command(doc, object, &json!({ "action": "isolate" })),
        "sculpt.showAllFaces" => visibility_command(doc, object, &json!({ "action": "showAll" })),
        other => Err(format!("unknown-command: {other}")),
    }
}

fn settings_json(settings: &Dyntopo) -> Value {
    serde_json::to_value(settings).unwrap_or(Value::Null)
}

/// `sculpt.dyntopo`: dynamic topology on or off and its settings; turning it on cuts the mesh
/// into triangles.
fn dyntopo_command(doc: &mut SceneDocument, object: Id, params: &Value) -> Result<Value, String> {
    let mut settings = doc.session.sculpt.as_ref().map(|session| session.dyntopo).unwrap_or_default();
    if let Some(on) = params::boolean(params, "enabled")? {
        settings.enabled = on;
    }
    if let Some(mode) = params::string(params, "mode")? {
        settings.mode = match mode {
            "relative" => DetailMode::Relative,
            "constant" => DetailMode::Constant,
            "brush" => DetailMode::Brush,
            other => return Err(bad(format!("unknown detail mode `{other}`"))),
        };
    }
    if let Some(refine) = params::string(params, "refine")? {
        settings.refine = match refine {
            "subdivideCollapse" => Refine::SubdivideCollapse,
            "subdivide" => Refine::Subdivide,
            "collapse" => Refine::Collapse,
            other => return Err(bad(format!("unknown refine method `{other}`"))),
        };
    }
    if let Some(size) = params::number(params, "detailSize")? {
        settings.detail_size = size;
    }
    if let Some(resolution) = params::number(params, "resolution")? {
        settings.resolution = resolution;
    }
    if let Some(percent) = params::number(params, "percent")? {
        settings.percent = percent;
    }
    settings.check().map_err(|error| error.to_string())?;
    let mut triangulated = false;
    if settings.enabled {
        if has_levels(doc, object) {
            return Err("not-allowed: dynamic topology does not work on a mesh with multiresolution levels".into());
        }
        triangulated = dyntopo::triangulate(base_mesh(doc, object)?);
    }
    if let Some(session) = doc.session.sculpt.as_mut() {
        session.dyntopo = settings;
    }
    Ok(json!({ "object": object, "dyntopo": settings_json(&settings), "triangulated": triangulated }))
}

/// The longest edge, in the object's own space, a flood fill leaves.
fn flood_length(doc: &SceneDocument, object: Id, settings: &Dyntopo, brush_radius: f32) -> Result<f32, String> {
    let world = doc.scene.world_matrix(object).map_err(|error| error.to_string())?;
    let scale = ((world.x_axis.truncate().length() + world.y_axis.truncate().length() + world.z_axis.truncate().length()) / 3.0).max(1e-6);
    let mesh = doc.scene.object(object).ok().and_then(|found| found.mesh()).ok_or("unsupported: not a mesh")?;
    let center = world.transform_point3(mesh.bounds(glam::Mat4::IDENTITY).center());
    let pixel = doc.session.editor.view.projector(&doc.scene, doc.session.width, doc.session.height).pixel_size_at(center) / scale;
    Ok(match settings.mode {
        DetailMode::Constant => 1.0 / settings.resolution,
        DetailMode::Relative => pixel * settings.detail_size,
        DetailMode::Brush => pixel * brush_radius * settings.percent / 100.0,
    })
}

/// `sculpt.detailFlood`: the whole mesh refined to the detail size.
fn detail_flood(doc: &mut SceneDocument, object: Id) -> Result<Value, String> {
    at_base(doc, object, "detail flood fill")?;
    if has_levels(doc, object) {
        return Err("not-allowed: dynamic topology does not work on a mesh with multiresolution levels".into());
    }
    let (settings, radius) = doc.session.sculpt.as_ref().map(|session| (session.dyntopo, session.brush.radius)).unwrap_or_default();
    if !settings.enabled {
        return Err("not-allowed: turn dynamic topology on first".into());
    }
    let longest = flood_length(doc, object, &settings, radius)?;
    let mesh = base_mesh(doc, object)?;
    dyntopo::triangulate(mesh);
    let bounds = mesh.bounds(glam::Mat4::IDENTITY);
    let (center, reach) = (bounds.center(), bounds.size().length());
    let (mut split, mut collapsed) = (0, 0);
    for _ in 0..16 {
        let refined = dyntopo::refine(mesh, center, reach, longest, settings.refine);
        split += refined.split;
        collapsed += refined.collapsed;
        if !refined.changed() {
            break;
        }
    }
    Ok(json!({ "object": object, "split": split, "collapsed": collapsed, "vertices": mesh.positions.len(), "faces": mesh.faces.len() }))
}

/// `sculpt.gesture`: masks, hides, shows, or makes a face set of what a box or lasso (in
/// viewport pixels) encloses.
fn gesture_command(doc: &mut SceneDocument, object: Id, params: &Value) -> Result<Value, String> {
    at_base(doc, object, "the box and lasso tools")?;
    let action = params::string(params, "action")?.ok_or_else(|| bad("`action` is required"))?;
    let value = params::number(params, "value")?;
    let gesture = match action {
        "mask" => Gesture::Mask(value.unwrap_or(1.0)),
        "unmask" => Gesture::Mask(0.0),
        "hide" => Gesture::Hide,
        "show" => Gesture::Show,
        "faceSet" => Gesture::FaceSet,
        other => return Err(bad(format!("unknown gesture action `{other}`"))),
    };
    let points: Vec<(f32, f32)> = params
        .get("points")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("`points` is a list of [x, y] viewport pixels"))?
        .iter()
        .map(|point| {
            let pair: Vec<f64> = point.as_array().map(|items| items.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
            match pair.as_slice() {
                [x, y] if x.is_finite() && y.is_finite() => Ok((*x as f32, *y as f32)),
                _ => Err(bad("each point is [x, y] viewport pixels")),
            }
        })
        .collect::<Result<_, _>>()?;
    let shape = params::string(params, "shape")?.unwrap_or(if points.len() == 2 { "box" } else { "lasso" });
    let outline: Vec<(f32, f32)> = match (shape, points.as_slice()) {
        ("box", [a, b]) => {
            vec![(a.0.min(b.0), a.1.min(b.1)), (a.0.max(b.0), a.1.min(b.1)), (a.0.max(b.0), a.1.max(b.1)), (a.0.min(b.0), a.1.max(b.1))]
        }
        ("box", _) => return Err(bad("a box is two corners")),
        ("lasso", list) if list.len() >= 3 && list.len() <= 20_000 => list.to_vec(),
        ("lasso", _) => return Err(bad("a lasso is 3 to 20000 points")),
        (other, _) => return Err(bad(format!("unknown shape `{other}` (box or lasso)"))),
    };
    let outside = params::boolean(params, "outside")?.unwrap_or(false);
    let front = params::boolean(params, "frontFacesOnly")?.unwrap_or(false);
    let projector = doc.session.editor.view.projector(&doc.scene, doc.session.width, doc.session.height);
    let world = doc.scene.world_matrix(object).map_err(|error| error.to_string())?;
    let matrix = projector.view_projection() * world;
    let eye_matrix = projector.view * world;
    let seen = front.then(|| crate::commands_sculpt::picture(doc));
    let inside = |point: Vec3| -> bool {
        let clip = matrix * point.extend(1.0);
        if clip.w <= 0.0 {
            return false;
        }
        let screen = projector.to_screen(clip);
        if !face_sets::in_polygon(&outline, (screen.x, screen.y)) {
            return false;
        }
        match &seen {
            Some(picture) => {
                let depth = -eye_matrix.transform_point3(point).z;
                picture.depth_at(f64::from(screen.x), f64::from(screen.y)).is_some_and(|front| depth <= front + 1e-3 + front * 0.01)
            }
            None => true,
        }
    };
    let mesh = base_mesh(doc, object)?;
    let changed = face_sets::gesture(mesh, gesture, outside, &inside)
        .map_err(|error| format!("not-allowed: {}", error.to_string().replace("invalid-params: ", "")))?;
    Ok(json!({ "object": object, "action": action, "changed": changed }))
}

/// `sculpt.faceSetVisibility`: hides, isolates, or shows face sets, by id or by the face under a
/// viewport pixel.
fn visibility_command(doc: &mut SceneDocument, object: Id, params: &Value) -> Result<Value, String> {
    let action = match params::string(params, "action")?.unwrap_or("toggle") {
        "hide" => Visibility::Hide,
        "isolate" => Visibility::Isolate,
        "showAll" => Visibility::ShowAll,
        "toggle" => Visibility::Toggle,
        other => return Err(bad(format!("unknown action `{other}`"))),
    };
    let id = match (params::integer(params, "faceSet")?, params::number(params, "x")?, params::number(params, "y")?) {
        (Some(id), _, _) => id,
        _ if action == Visibility::ShowAll => 0,
        (None, Some(x), Some(y)) => face_set_at(doc, object, f64::from(x), f64::from(y)).ok_or("not-found: no face of the sculpted mesh there")?,
        _ => {
            let (x, y) = doc
                .session
                .sculpt
                .as_ref()
                .and_then(|session| session.hover)
                .ok_or_else(|| bad("give `faceSet`, or `x` and `y` in viewport pixels"))?;
            face_set_at(doc, object, x, y).ok_or("not-found: no face of the sculpted mesh under the pointer")?
        }
    };
    let mesh = base_mesh(doc, object)?;
    let hidden = face_sets::visibility(mesh, id, action);
    Ok(json!({ "object": object, "faceSet": id, "hiddenFaces": hidden }))
}

/// The face set of the sculpted mesh's face under pixel (`x`, `y`).
fn face_set_at(doc: &SceneDocument, object: Id, x: f64, y: f64) -> Option<u32> {
    let face = crate::commands_sculpt::picture(doc).face_at(object, x, y)?;
    let mesh = doc.scene.object(object).ok()?.mesh()?;
    let at = sculpt_level(doc, object);
    let base_face = if at > 0 { multires::face_origins(mesh, at).get(face as usize).copied()? } else { face };
    face_sets::face_sets(mesh).get(base_face as usize).copied()
}

/// The box or lasso tool's pointer; `None` when the brush has it.
pub fn pointer(doc: &mut SceneDocument, event: &ToolPointer) -> Option<ToolResponse> {
    let session = doc.session.sculpt.as_ref()?;
    let tool = session.tool;
    if tool == SculptTool::Brush {
        return None;
    }
    let at = (event.x, event.y);
    let opposite = event.mods.command;
    let mut points = session.gesture.clone().unwrap_or_default();
    match event.action {
        PointerAction::Down => points = vec![at],
        PointerAction::Drag if !points.is_empty() => {
            if tool.lasso() {
                let far = points.last().is_none_or(|last| (last.0 - at.0).hypot(last.1 - at.1) >= 2.0);
                if far {
                    points.push(at);
                }
            } else {
                points.truncate(1);
                points.push(at);
            }
        }
        PointerAction::Up if !points.is_empty() => {
            if let Some(session) = doc.session.sculpt.as_mut() {
                session.gesture = None;
            }
            if !tool.lasso() {
                points.truncate(1);
                points.push(at);
            }
            let enough = if tool.lasso() {
                points.len() >= 3
            } else {
                points.first().is_some_and(|first| (first.0 - at.0).abs() >= 1.0 && (first.1 - at.1).abs() >= 1.0)
            };
            if enough && let Some(action) = tool.action(opposite) {
                let shape = if tool.lasso() { "lasso" } else { "box" };
                let list: Vec<Value> = points.iter().map(|(x, y)| json!([x, y])).collect();
                let _ = doc.execute("sculpt.gesture", json!({ "action": action, "shape": shape, "points": list }));
            }
            return Some(ToolResponse { cursor: "crosshair".into(), ..Default::default() });
        }
        _ => {
            return Some(ToolResponse { cursor: "crosshair".into(), ..Default::default() });
        }
    }
    let path = outline_path(&points, tool.lasso());
    if let Some(session) = doc.session.sculpt.as_mut() {
        session.gesture = Some(points);
    }
    Some(ToolResponse {
        overlays: vec![ToolOverlay::Path { d: path, color: [255, 255, 255, 230], width: 1.5, dashed: opposite }],
        cursor: "crosshair".into(),
        busy: true,
        ..Default::default()
    })
}

/// The SVG path of a box (two corners) or a closed lasso.
fn outline_path(points: &[(f64, f64)], lasso: bool) -> String {
    match (lasso, points) {
        (false, [a, b]) => format!("M {} {} L {} {} L {} {} L {} {} Z", a.0, a.1, b.0, a.1, b.0, b.1, a.0, b.1),
        _ => {
            let mut d = String::new();
            for (index, (x, y)) in points.iter().enumerate() {
                d.push_str(&format!("{}{x} {y} ", if index == 0 { "M " } else { "L " }));
            }
            if lasso && points.len() > 2 {
                d.push('Z');
            }
            d
        }
    }
}

/// Sculpt mode's part of `inspect()`: the tool, dynamic topology, the sculpt level, face sets,
/// and hidden faces.
pub fn inspect(doc: &SceneDocument) -> Option<serde_json::Map<String, Value>> {
    let session = doc.session.sculpt.as_ref()?;
    let mesh = doc.scene.object(session.object).ok()?.mesh()?;
    let mut out = serde_json::Map::new();
    out.insert("tool".into(), json!(session.tool.id()));
    out.insert("dyntopo".into(), settings_json(&session.dyntopo));
    out.insert("level".into(), json!(sculpt_level(doc, session.object)));
    out.insert("levels".into(), json!(multires::levels(mesh)));
    out.insert("faceSets".into(), json!(face_sets::ids(mesh).into_iter().collect::<Vec<_>>()));
    out.insert("hiddenFaces".into(), json!(face_sets::hidden(mesh).iter().filter(|flag| **flag).count()));
    out.insert("triangles".into(), json!(mesh.faces.iter().all(|face| face.len() == 3)));
    Some(out)
}
