//! `file.render`, the kit's agent preview (`agent.render` in kit.json; KHP A24, A39): the scene
//! seen from an explicit view at an explicit time, in an explicit pass, or a sheet of views (or of
//! times from one view), each labelled. It is a function of the scene and the call alone: it never
//! reads or moves the editor's viewport, the current frame, the mode or the selection, and never
//! makes the document dirty, so the same call answers the same picture whatever the editor does.
//!
//! Views: `camera` is the scene's active camera (a property of the scene, not the viewport);
//! `target.camera` names another; `front`, `back`, `left`, `right`, `top` and `bottom` look
//! orthographically along an axis, and `persp` from a fixed three-quarter angle, each framed to fit
//! the scene's visible meshes (or the object `objectId` names) by [`ViewState::fit`]. Pictures are
//! drawn by the software rasteriser the viewport uses (material shading, no overlays), which has
//! no sampling noise and no threads, never by the path tracer, so every machine answers the same
//! bytes.

use std::collections::BTreeMap;

use glam::Vec3;
use kition_kit::KitCommand;
use kition_kit::schema::{ParamError, refusal};
use kition_scene_model::{Id, ObjectData, Scene};
use kition_scene_raster::{Axis, Canvas, MeshCache, NONE, Overlays, Shading, ViewState};
use serde_json::{Map, Value, json};

use crate::SceneDocument;

/// The command's id.
pub const RENDER: &str = "file.render";

/// What the command does, for the catalogue.
pub const DESCRIPTION: &str = "Renders a preview of the scene as a PNG without changing anything: from `target.view` (`camera`, the scene's active camera; `front`, `back`, `left`, `right`, `top`, `bottom` or `persp`, framed to fit the scene or `objectId`) or `target.camera` (another camera, by id or name), at `target.time` (seconds from the first frame; 0 without it), in `target.pass` (`color`, `id` with a legend of object colours, `depth` or `wireframe`). `target.views` (2 to 6) draws one labelled sheet of views; `target.range` [from, to] with `target.frames` (2 to 12) one labelled sheet of times from one view. Without a target it answers four views (the camera or `persp`, then front, right and top). It never reads or moves the viewport, the current frame, the mode or the selection.";

/// The largest `scale` (picture pixels per render pixel), and `maxSize`'s range and default.
const MAX_SCALE: f64 = 2.0;
const MIN_SIZE: u64 = 16;
const MAX_SIZE: u64 = 4096;
const DEFAULT_SIZE: u64 = 1024;
/// How many frames a sheet over time holds, and how many views a sheet of views.
const MIN_FRAMES: u64 = 2;
const MAX_FRAMES: u64 = 12;
const SHEET_FRAMES: u64 = 6;
const MIN_VIEWS: usize = 2;
const MAX_VIEWS: usize = 6;
/// Black pixels between a sheet's cells.
const GAP: u32 = 2;
/// Seconds an agent may name (well beyond any animation).
const MAX_SECONDS: f64 = 1.0e7;
/// What a fitted view leaves round what it frames (a tenth on each side).
const FIT_MARGIN: f32 = 1.1;
/// The three-quarter view: tilted 60° from looking straight down, turned 45° from the front
/// towards the right (looking from front right, above).
const PERSP_TILT: f32 = 60.0;
const PERSP_TURN: f32 = 45.0;

/// The view names `target.view` takes.
const VIEWS: [&str; 8] = ["camera", "front", "back", "left", "right", "top", "bottom", "persp"];
/// The passes `target.pass` takes.
const PASSES: [&str; 4] = ["color", "id", "depth", "wireframe"];
/// The keys `target` takes.
const KEYS: [&str; 9] = ["view", "views", "camera", "time", "range", "frames", "pass", "objectId", "path"];

/// The command as the catalogue lists it.
pub fn command() -> KitCommand {
    let view = format!("One of {}", VIEWS.join(", "));
    KitCommand {
        id: RENDER.to_owned(),
        title: "Render Preview".to_owned(),
        menu: None,
        shortcut: None,
        params: Some(
            "{target?: {view?: camera|front|back|left|right|top|bottom|persp, views?: [2-6 views or {camera}], camera?: id or name, time?: seconds, range?: [from, to] seconds, frames?: 2-12, pass?: color|id|depth|wireframe, objectId?: id or name, path?: \"object:<id>\"}, scale?: >0-2 (default 1), maxSize?: 16-4096 (default 1024)} → {png (base64), mimeType, width, height, scale, frames: [{time, frame, label, view, camera?}], columns, rows, legend?: [{id, name, color}]}".to_owned(),
        ),
        schema: Some(json!({
            "target": {
                "type": "object",
                "additionalProperties": false,
                "description": "What to show. One view (`view` or `camera`) at one `time`; a sheet of `views`; or a sheet over `range` from one view. Without a target, four views at the first frame.",
                "properties": {
                    "view": { "type": "string", "enum": VIEWS, "description": format!("{view}. `camera` is the scene's active camera; the others are framed to fit the scene's visible meshes (or `objectId`): the six along the axes orthographically, `persp` from front right, above.") },
                    "views": { "type": "array", "items": { "type": "json" }, "description": "2 to 6 views for one sheet, each a view name or {\"camera\": id or name}; each cell is labelled with its view." },
                    "camera": { "oneOf": [{ "type": "integer" }, { "type": "string" }], "description": "A camera object's id or name, to look through it instead of the scene's active camera. Not with `view`." },
                    "time": { "type": "number", "description": "Seconds from the first frame of the animation (the scene's start frame at 0 s, at its fps), from 0 to before its end. The frame that starts at or before that time is drawn. 0 when left out." },
                    "range": { "type": "array", "items": { "type": "number" }, "description": "[from, to]: seconds from the first frame, from < to, within the animation. A sheet of `frames` frames spread evenly from `from` to `to`, both included, each labelled with its time and frame. Not with `views` or `time`." },
                    "frames": { "type": "integer", "description": "How many frames the sheet of `range` shows, 2 to 12 (default 6)." },
                    "pass": { "type": "string", "enum": PASSES, "description": "color (default): the materials lit by the scene's lights. id: each object in a flat colour of its own on black, named in `legend`. depth: grey, nearer is lighter, nothing is black. wireframe: the edges." },
                    "objectId": { "oneOf": [{ "type": "integer" }, { "type": "string" }], "description": "An object (id, a diagnostic's string id, or name) a named view frames instead of the whole scene. Not with the camera's view: a camera's framing is its own." },
                    "path": { "type": "string", "description": "`object:<id or name>`, the same as `objectId`." },
                },
            },
            "scale": { "type": "number", "description": "Picture pixels per render pixel (the scene's render size), above 0 and at most 2 (default 1). Each cell of a sheet is drawn at it. Reduced so the picture fits `maxSize`." },
            "maxSize": { "type": "integer", "description": "The most pixels on the picture's longer side, 16 to 4096 (default 1024)." },
        })),
        output_schema: Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "png": { "type": "string", "required": true, "description": "The picture as base64 PNG." },
                "mimeType": { "type": "string", "required": true },
                "width": { "type": "integer", "required": true },
                "height": { "type": "integer", "required": true },
                "scale": { "type": "number", "required": true, "description": "The scale each cell was drawn at, after `maxSize`." },
                "frames": { "type": "array", "required": true, "items": { "type": "json" }, "description": "Each cell, in reading order: {time (seconds from the first frame), frame (the scene's frame number), label, view, camera (the camera's id, when looking through one)}." },
                "columns": { "type": "integer", "required": true },
                "rows": { "type": "integer", "required": true },
                "legend": { "type": "array", "items": { "type": "json" }, "description": "With `pass: \"id\"`: {id, name, color (#rrggbb)} for each object the pass can draw." },
            },
        })),
        mutates: false,
        enabled: true,
        ..Default::default()
    }
}

/// A refusal of the parameter at `path` (a JSON Pointer), typed for the host (`invalid_params`)
/// and prefixed with this kit's reason code.
fn wrong(path: &str, message: String, expected: &str) -> String {
    format!("invalid-params: {}", refusal(&[ParamError { path: path.to_owned(), message, expected: expected.to_owned() }]))
}

/// Seconds as a short label: at most three decimals, no trailing zeros.
fn seconds(value: f64) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".to_owned() } else { text.to_owned() }
}

/// The parameter's name for messages: `/target/time` is `target.time`.
fn dotted(path: &str) -> String {
    path.trim_start_matches('/').replace('/', ".")
}

fn number(value: &Value, path: &str, what: &str) -> Result<f64, String> {
    value
        .as_f64()
        .filter(|n| n.is_finite() && n.abs() <= MAX_SECONDS)
        .ok_or_else(|| wrong(path, format!("`{}` is {value}: {what}", dotted(path)), what))
}

/// How the animation runs: its first and last frames and its frames per second.
#[derive(Clone, Copy)]
struct Timeline {
    start: i32,
    end: i32,
    fps: f64,
}

impl Timeline {
    fn of(scene: &Scene) -> Self {
        let timeline = scene.animation.timeline;
        let fps = f64::from(timeline.fps);
        Self { start: timeline.start, end: timeline.end.max(timeline.start), fps: if fps.is_finite() && fps > 0.0 { fps } else { 24.0 } }
    }

    /// Seconds from the first frame to the end of the last.
    fn duration(self) -> f64 {
        f64::from(self.end - self.start + 1) / self.fps
    }

    /// Where frame `frame` starts, in seconds from the first frame.
    fn time(self, frame: i32) -> f64 {
        f64::from(frame - self.start) / self.fps
    }

    /// The frame showing at `t` seconds (the one that starts at or before it; the last at the end).
    fn frame(self, t: f64) -> i32 {
        let offset = (t * self.fps + 1e-4).floor().clamp(0.0, f64::from(self.end - self.start));
        self.start + offset as i32
    }

    fn describe(self) -> String {
        format!("the animation runs from 0 to {} s (frames {} to {} at {} fps)", seconds(self.duration()), self.start, self.end, seconds(self.fps))
    }
}

/// The frame showing at `t` seconds from the first frame, counted as `target.time` counts them
/// (frame `start` at 0 s, the frame that starts at or before `t`), for a call's `at.time` (named
/// `at` in the refusal); a time outside the animation is refused, giving its range.
pub(crate) fn frame_at_time(scene: &Scene, t: f64, at: &str) -> Result<i32, String> {
    let timeline = Timeline::of(scene);
    if !(t.is_finite() && (0.0..timeline.duration()).contains(&t)) {
        return Err(format!(
            "invalid-params: `{at}` is {} s, outside the animation: {}; give a time from 0 to {} s (where its last frame, {}, starts)",
            seconds(t),
            timeline.describe(),
            seconds(timeline.time(timeline.end)),
            timeline.end
        ));
    }
    Ok(timeline.frame(t))
}

/// Where a cell looks from.
#[derive(Clone, Copy, PartialEq)]
enum Look {
    Axis(Axis, &'static str),
    Persp,
    /// Through a camera; whether it is the scene's active one.
    Camera(Id, bool),
}

/// Which picture of the scene a cell is.
#[derive(Clone, Copy, PartialEq)]
enum Pass {
    Color,
    Id,
    Depth,
    Wireframe,
}

/// What a call asks for.
struct Request {
    looks: Vec<Look>,
    frames: Vec<i32>,
    pass: Pass,
    /// The object named views frame, instead of the scene.
    fit: Option<Id>,
}

/// The scene's cameras, by id and name, for a refusal.
fn cameras(scene: &Scene) -> String {
    let listed: Vec<String> = scene
        .objects
        .iter()
        .filter(|object| matches!(object.data, ObjectData::Camera(_)))
        .map(|object| format!("{} \"{}\"", object.id, object.name))
        .collect();
    if listed.is_empty() {
        "the scene has no cameras: add one with object.addCamera, or use a named view (front, back, left, right, top, bottom or persp)".to_owned()
    } else {
        format!("the scene's cameras are {}", listed.join(", "))
    }
}

/// An object named by id (a number or a string of digits) or by name.
fn object_named(scene: &Scene, value: &Value) -> Option<Id> {
    match value {
        Value::Number(number) => number.as_u64().and_then(|id| u32::try_from(id).ok()).filter(|id| scene.object(*id).is_ok()),
        Value::String(text) => {
            text.parse::<u32>().ok().filter(|id| scene.object(*id).is_ok()).or_else(|| scene.object_named(text).map(|object| object.id))
        }
        _ => None,
    }
}

/// A camera named by id or name.
fn camera_named(scene: &Scene, value: &Value, path: &str) -> Result<Look, String> {
    let id = object_named(scene, value).filter(|id| matches!(scene.object(*id).map(|object| &object.data), Ok(ObjectData::Camera(_))));
    match id {
        Some(id) => Ok(Look::Camera(id, scene.camera == Some(id))),
        None => {
            Err(wrong(path, format!("`{}` is {value}, which is no camera of the scene: {}", dotted(path), cameras(scene)), "a camera's id or name"))
        }
    }
}

/// A view by name.
fn view_named(scene: &Scene, value: &Value, path: &str) -> Result<Look, String> {
    let name = value.as_str().unwrap_or_default();
    match name {
        "camera" => match scene.camera.filter(|id| matches!(scene.object(*id).map(|object| &object.data), Ok(ObjectData::Camera(_)))) {
            Some(id) => Ok(Look::Camera(id, true)),
            None => Err(wrong(
                path,
                format!(
                    "the scene has no active camera to look through: {}; pass `target.camera` with one, make one active with object.setCamera, or use a named view",
                    cameras(scene)
                ),
                "a named view",
            )),
        },
        "front" => Ok(Look::Axis(Axis::Front, "front")),
        "back" => Ok(Look::Axis(Axis::Back, "back")),
        "left" => Ok(Look::Axis(Axis::Left, "left")),
        "right" => Ok(Look::Axis(Axis::Right, "right")),
        "top" => Ok(Look::Axis(Axis::Top, "top")),
        "bottom" => Ok(Look::Axis(Axis::Bottom, "bottom")),
        "persp" => Ok(Look::Persp),
        _ => Err(wrong(path, format!("`{}` is {value}: a view is one of {}", dotted(path), VIEWS.join(", ")), "a view name")),
    }
}

/// The view a call without one shows: the active camera, else the three-quarter view.
fn default_look(scene: &Scene) -> Look {
    view_named(scene, &json!("camera"), "/target/view").unwrap_or(Look::Persp)
}

fn request(scene: &Scene, params: &Value) -> Result<Request, String> {
    let target = match params.get("target") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(target)) => target.clone(),
        Some(other) => {
            return Err(wrong("/target", format!("`target` is {other}: an object such as {{\"view\": \"front\", \"time\": 1.5}}"), "an object"));
        }
    };
    if let Some(key) = target.keys().find(|key| !KEYS.contains(&key.as_str())) {
        return Err(wrong(
            &format!("/target/{key}"),
            format!("this kit shows a 3D scene and takes no `target.{key}`: it takes {}", KEYS.map(|key| format!("`{key}`")).join(", ")),
            "view, views, camera, time, range, frames, pass, objectId or path",
        ));
    }
    let given = |key: &str| target.get(key).filter(|value| !value.is_null());
    let timeline = Timeline::of(scene);
    // Which objects a named view frames.
    let by_id = given("objectId").map(|value| (value.clone(), "/target/objectId"));
    let by_path = match given("path") {
        Some(Value::String(path)) => match path.strip_prefix("object:") {
            Some(object) => Some((json!(object), "/target/path")),
            None => {
                return Err(wrong(
                    "/target/path",
                    format!("`target.path` is {path:?}: this kit frames objects, named `object:<id or name>` (or pass `target.objectId`)"),
                    "object:<id or name>",
                ));
            }
        },
        Some(other) => return Err(wrong("/target/path", format!("`target.path` is {other}: a string such as \"object:2\""), "object:<id or name>")),
        None => None,
    };
    let mut fit = None;
    for (value, path) in by_id.iter().chain(by_path.iter()) {
        let Some(id) = object_named(scene, value) else {
            let listed: Vec<String> = scene.objects.iter().take(20).map(|object| format!("{} \"{}\"", object.id, object.name)).collect();
            return Err(wrong(
                path,
                format!("`{}` is {value}, which names no object: the scene's objects are {}", dotted(path), listed.join(", ")),
                "an object's id or name",
            ));
        };
        if fit.is_some_and(|other| other != id) {
            return Err(wrong(path, "`target.objectId` and `target.path` name different objects".to_owned(), "one object"));
        }
        fit = Some(id);
    }
    let fit_path = if by_id.is_some() { "/target/objectId" } else { "/target/path" };
    // Where to look from.
    let (view, views, camera) = (given("view"), given("views"), given("camera"));
    if view.is_some() && camera.is_some() {
        return Err(wrong(
            "/target/camera",
            "give `view` (a named view, or \"camera\" for the scene's active camera) or `camera` (a camera to look through), not both".to_owned(),
            "view or camera",
        ));
    }
    if views.is_some() && (view.is_some() || camera.is_some()) {
        return Err(wrong(
            "/target/views",
            "give `views` for a sheet of views, or `view`/`camera` for one, not both".to_owned(),
            "views, or view or camera",
        ));
    }
    let (time, range, frames) = (given("time"), given("range"), given("frames"));
    if views.is_some() && range.is_some() {
        return Err(wrong(
            "/target/range",
            "a sheet shows one axis: several `views` at one time, or one view over a `range` of times; give one view with `range`".to_owned(),
            "views or range",
        ));
    }
    if time.is_some() && range.is_some() {
        return Err(wrong(
            "/target/time",
            "give `time` for one moment, or `range` (with `frames`) for a sheet over time, not both".to_owned(),
            "time or range",
        ));
    }
    if frames.is_some() && range.is_none() {
        return Err(wrong("/target/frames", "`frames` counts the frames of a `range`: give `range` [from, to] too".to_owned(), "range"));
    }
    let looks = match (view, views, camera) {
        (Some(view), _, _) => vec![view_named(scene, view, "/target/view")?],
        (_, _, Some(camera)) => vec![camera_named(scene, camera, "/target/camera")?],
        (_, Some(views), _) => {
            let items = views.as_array().filter(|items| (MIN_VIEWS..=MAX_VIEWS).contains(&items.len())).ok_or_else(|| {
                wrong(
                    "/target/views",
                    format!("`target.views` is {views}: a list of 2 to 6 views, each a view name or {{\"camera\": id or name}}"),
                    "2 to 6 views",
                )
            })?;
            let mut looks = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                let path = format!("/target/views/{index}");
                looks.push(match item {
                    Value::Object(fields) if fields.len() == 1 && fields.contains_key("camera") => {
                        camera_named(scene, fields.get("camera").unwrap_or(&Value::Null), &format!("{path}/camera"))?
                    }
                    Value::String(_) => view_named(scene, item, &path)?,
                    _ => {
                        return Err(wrong(
                            &path,
                            format!("`{}` is {item}: a view name ({}) or {{\"camera\": id or name}}", dotted(&path), VIEWS.join(", ")),
                            "a view",
                        ));
                    }
                });
            }
            looks
        }
        // A sheet over time looks through one view; without a target at all, four views.
        _ if range.is_some() => vec![if fit.is_some() { Look::Persp } else { default_look(scene) }],
        _ => {
            let first = if fit.is_some() { Look::Persp } else { default_look(scene) };
            vec![first, Look::Axis(Axis::Front, "front"), Look::Axis(Axis::Right, "right"), Look::Axis(Axis::Top, "top")]
        }
    };
    if fit.is_some() && looks.iter().any(|look| matches!(look, Look::Camera(..))) {
        return Err(wrong(
            fit_path,
            "a camera's framing is its own and cannot fit an object: frame the object with a named view (front, back, left, right, top, bottom or persp), or leave out the object to look through the camera".to_owned(),
            "a named view",
        ));
    }
    // When.
    let frames = if let Some(time) = time {
        let t = number(time, "/target/time", "seconds from the first frame")?;
        if !(0.0..timeline.duration()).contains(&t) {
            let last = timeline.time(timeline.end);
            return Err(wrong(
                "/target/time",
                format!(
                    "{} s is outside the animation: {}; pass a time from 0 to {} s (where its last frame, {}, starts)",
                    seconds(t),
                    timeline.describe(),
                    seconds(last),
                    timeline.end
                ),
                "seconds from the first frame",
            ));
        }
        vec![timeline.frame(t)]
    } else if let Some(range) = range {
        let ends = range
            .as_array()
            .filter(|ends| ends.len() == 2)
            .ok_or_else(|| wrong("/target/range", format!("`target.range` is {range}: [from, to] in seconds from the first frame"), "[from, to]"))?;
        let from = number(ends.first().unwrap_or(&Value::Null), "/target/range", "[from, to] in seconds")?;
        let to = number(ends.get(1).unwrap_or(&Value::Null), "/target/range", "[from, to] in seconds")?;
        let duration = timeline.duration();
        if !(from >= 0.0 && from < to && to <= duration + 1e-9) {
            return Err(wrong(
                "/target/range",
                format!(
                    "`target.range` [{}, {}] is not a span of the animation: {}; pass 0 <= from < to <= {}",
                    seconds(from),
                    seconds(to),
                    timeline.describe(),
                    seconds(duration)
                ),
                "[from, to] within the animation",
            ));
        }
        let n = match frames {
            Some(frames) => frames
                .as_u64()
                .ok_or_else(|| wrong("/target/frames", format!("`target.frames` is {frames}: a whole number from 2 to 12"), "2 to 12"))?,
            None => SHEET_FRAMES,
        };
        if !(MIN_FRAMES..=MAX_FRAMES).contains(&n) {
            return Err(wrong(
                "/target/frames",
                format!("`target.frames` is {n}: a sheet over time shows {MIN_FRAMES} to {MAX_FRAMES} frames"),
                "2 to 12",
            ));
        }
        let steps = (n - 1) as f64;
        (0..n).map(|i| timeline.frame(from + (to - from) * i as f64 / steps)).collect()
    } else {
        vec![timeline.start]
    };
    let pass = match given("pass") {
        None => Pass::Color,
        Some(value) => match value.as_str() {
            Some("color") => Pass::Color,
            Some("id") => Pass::Id,
            Some("depth") => Pass::Depth,
            Some("wireframe") => Pass::Wireframe,
            _ => {
                return Err(wrong(
                    "/target/pass",
                    format!("`target.pass` is {value}: this kit draws {}", PASSES.join(", ")),
                    "color, id, depth or wireframe",
                ));
            }
        },
    };
    Ok(Request { looks, frames, pass, fit })
}

fn scale_of(params: &Value) -> Result<f64, String> {
    match params.get("scale") {
        None | Some(Value::Null) => Ok(1.0),
        Some(value) => value.as_f64().filter(|s| s.is_finite() && *s > 0.0 && *s <= MAX_SCALE).ok_or_else(|| {
            wrong("/scale", format!("`scale` is {value}: picture pixels per render pixel, above 0 and at most {MAX_SCALE}"), "above 0, at most 2")
        }),
    }
}

fn max_size_of(params: &Value) -> Result<u64, String> {
    match params.get("maxSize") {
        None | Some(Value::Null) => Ok(DEFAULT_SIZE),
        Some(value) => value.as_u64().filter(|size| (MIN_SIZE..=MAX_SIZE).contains(size)).ok_or_else(|| {
            wrong("/maxSize", format!("`maxSize` is {value}: the most pixels on the picture's longer side, {MIN_SIZE} to {MAX_SIZE}"), "16 to 4096")
        }),
    }
}

/// The corners of what named views frame in `scene`: the object `fit` names, else every visible
/// mesh (else every visible object, else the origin).
fn corners(scene: &Scene, fit: Option<Id>) -> Vec<Vec3> {
    let bounds = match fit {
        Some(id) => {
            let bounds = scene.bounds(Some(std::slice::from_ref(&id)));
            if bounds.is_empty() {
                scene.world_matrix(id).map(|world| world.transform_point3(Vec3::ZERO)).map(|at| bounds.with(at)).unwrap_or(bounds)
            } else {
                bounds
            }
        }
        None => {
            let meshes: Vec<Id> = scene.objects.iter().filter(|object| !object.hidden && object.has_geometry()).map(|object| object.id).collect();
            let bounds = scene.bounds(Some(&meshes));
            if bounds.is_empty() { scene.bounds(None) } else { bounds }
        }
    };
    if bounds.is_empty() {
        return vec![Vec3::ZERO];
    }
    let (low, high) = (bounds.min, bounds.max);
    (0..8)
        .map(|i| Vec3::new(if i & 1 == 0 { low.x } else { high.x }, if i & 2 == 0 { low.y } else { high.y }, if i & 4 == 0 { low.z } else { high.z }))
        .collect()
}

/// The viewport's view for `look`, framing `points` in a picture of `aspect`.
fn view_state(look: Look, points: &[Vec3], aspect: f32) -> ViewState {
    let mut view = ViewState::default();
    match look {
        Look::Camera(id, _) => view.camera = Some(id),
        Look::Axis(axis, _) => {
            view.align(axis);
            view.fit(points, aspect, FIT_MARGIN);
        }
        Look::Persp => {
            view.tilt = PERSP_TILT.to_radians();
            view.turn = PERSP_TURN.to_radians();
            view.perspective = true;
            view.fit(points, aspect, FIT_MARGIN);
        }
    }
    view
}

/// A flat, distinct colour for the `index`th object of the id pass (sRGB): hues a golden angle
/// apart, so neighbours in the list differ most.
fn id_colour(index: usize) -> [u8; 3] {
    let hue = (index as f64 * 0.618_033_988_749_895).fract() * 6.0;
    let (saturation, value) = (0.8, 0.95);
    let chroma = value * saturation;
    let x = chroma * (1.0 - (hue % 2.0 - 1.0).abs());
    let (r, g, b) = match hue as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = value - chroma;
    let byte = |c: f64| ((c + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    [byte(r), byte(g), byte(b)]
}

/// The objects the id pass can draw (visible meshes, curves, texts and surfaces), in scene order,
/// with their colours.
fn legend(scene: &Scene) -> Vec<(Id, String, [u8; 3])> {
    scene
        .objects
        .iter()
        .filter(|object| !object.hidden && object.has_geometry())
        .enumerate()
        .map(|(index, object)| (object.id, object.name.clone(), id_colour(index)))
        .collect()
}

/// `canvas`'s object buffer in the legend's colours (particles in their emitter's), black where
/// nothing is.
fn id_picture(doc: &SceneDocument, canvas: &Canvas, colours: &BTreeMap<Id, [u8; 3]>) -> Vec<u8> {
    let mut owner: BTreeMap<u32, Option<[u8; 3]>> = BTreeMap::new();
    let mut out = Vec::with_capacity(canvas.object.len() * 4);
    for id in &canvas.object {
        let colour = if *id == NONE {
            None
        } else {
            *owner.entry(*id).or_insert_with(|| {
                colours
                    .get(id)
                    .copied()
                    .or_else(|| crate::commands_particles::owners(doc, vec![*id]).first().and_then(|owner| colours.get(owner).copied()))
            })
        };
        let [r, g, b] = colour.unwrap_or([0, 0, 0]);
        out.extend([r, g, b, 255]);
    }
    out
}

/// `canvas`'s depth as grey: the nearest surface white, the farthest a tenth of white, nothing
/// black.
fn depth_picture(canvas: &Canvas) -> Vec<u8> {
    let drawn = canvas.depth.iter().zip(&canvas.object).filter(|(depth, object)| depth.is_finite() && **object != NONE).map(|(depth, _)| *depth);
    let (near, far) = drawn.fold((f32::INFINITY, f32::NEG_INFINITY), |(near, far), depth| (near.min(depth), far.max(depth)));
    let span = (far - near).max(1e-6);
    let mut out = Vec::with_capacity(canvas.depth.len() * 4);
    for (depth, object) in canvas.depth.iter().zip(&canvas.object) {
        let level =
            if depth.is_finite() && *object != NONE { (255.0 * (1.0 - 0.9 * (depth - near) / span)).round().clamp(0.0, 255.0) as u8 } else { 0 };
        out.extend([level, level, level, 255]);
    }
    out
}

/// One cell: `scene` from `view` at `width` × `height` in `pass`, as 8-bit sRGB RGBA.
fn cell(
    doc: &SceneDocument,
    scene: &Scene,
    view: &ViewState,
    size: (u32, u32),
    pass: Pass,
    colours: &BTreeMap<Id, [u8; 3]>,
    cache: &mut MeshCache,
) -> Vec<u8> {
    let overlays = Overlays { hide_extras: true, ..Overlays::default() };
    let shading = match pass {
        Pass::Color => Shading::Material,
        Pass::Wireframe => Shading::Wireframe,
        Pass::Id | Pass::Depth => Shading::Solid,
    };
    let canvas = kition_scene_raster::render_viewport_picture(scene, view, size.0, size.1, shading, &overlays, cache, None);
    match pass {
        Pass::Color | Pass::Wireframe => canvas.to_rgba8(),
        Pass::Id => id_picture(doc, &canvas, colours),
        Pass::Depth => depth_picture(&canvas),
    }
}

/// Runs `file.render` on `doc`'s scene: see the module.
pub fn render(doc: &SceneDocument, params: &Value) -> Result<Value, String> {
    let scale = scale_of(params)?;
    let max_size = max_size_of(params)? as f64;
    let scene = &doc.scene;
    let request = request(scene, params)?;
    let timeline = Timeline::of(scene);
    let (rw, rh) = (f64::from(scene.render.width.max(1)), f64::from(scene.render.height.max(1)));
    let count = request.looks.len().max(request.frames.len()).max(1);
    let sheet = count > 1;
    let columns = if sheet { (count as f64).sqrt().ceil().max(1.0) as usize } else { 1 };
    let rows = count.div_ceil(columns);
    // Room for the gaps, and a pixel per cell for rounding, within maxSize.
    let room = |cells: usize| max_size - f64::from(GAP) * cells.saturating_sub(1) as f64 - if sheet { cells as f64 } else { 0.0 };
    let fitted = scale.min(room(columns) / (columns as f64 * rw)).min(room(rows) / (rows as f64 * rh));
    if !(fitted.is_finite() && fitted > 0.0) {
        return Err(wrong("/maxSize", format!("`maxSize` {max_size} is too small for {count} cells: give a larger one"), "a larger maxSize"));
    }
    let (cw, ch) = (((rw * fitted).round() as u32).max(1), ((rh * fitted).round() as u32).max(1));
    // The scene at each frame asked for, as a render sees it (the current frame stays as it is).
    let mut scenes: BTreeMap<i32, Scene> = BTreeMap::new();
    for frame in &request.frames {
        scenes.entry(*frame).or_insert_with(|| crate::commands_particles::render_scene(doc, scene.at_frame(*frame as f32)));
    }
    // One framing for the whole picture (over every frame of a sheet over time), so motion shows.
    let points: Vec<Vec3> = scenes.values().flat_map(|at| corners(at, request.fit)).collect();
    let aspect = cw as f32 / ch as f32;
    let legend = legend(scene);
    let colours: BTreeMap<Id, [u8; 3]> = legend.iter().map(|(id, _, colour)| (*id, *colour)).collect();
    let mut cache = MeshCache::new();
    let mut infos = Vec::with_capacity(count);
    let mut cells = Vec::with_capacity(count);
    for index in 0..count {
        let look = *request.looks.get(index).or(request.looks.first()).unwrap_or(&Look::Persp);
        let frame = *request.frames.get(index).or(request.frames.first()).unwrap_or(&timeline.start);
        let Some(at) = scenes.get(&frame) else { continue };
        let time = (timeline.time(frame) * 1e6).round() / 1e6;
        let (view_name, camera, view_label) = match look {
            Look::Axis(_, name) => (name.to_owned(), None, name.to_owned()),
            Look::Persp => ("persp".to_owned(), None, "persp".to_owned()),
            Look::Camera(id, active) => {
                let name = if active { "camera".to_owned() } else { scene.object(id).map(|object| object.name.clone()).unwrap_or_default() };
                ("camera".to_owned(), Some(id), name)
            }
        };
        let when = format!("{} s, frame {frame}", seconds(time));
        let label = if !sheet {
            format!("{view_label}, {when}")
        } else if request.looks.len() > 1 {
            view_label
        } else {
            when
        };
        let mut pixels = cell(doc, at, &view_state(look, &points, aspect), (cw, ch), request.pass, &colours, &mut cache);
        if sheet {
            kition_scene_raster::label::burn_label(&mut pixels, cw, ch, &label);
        }
        let mut info = json!({ "time": time, "frame": frame, "label": label, "view": view_name });
        if let (Some(id), Some(fields)) = (camera, info.as_object_mut()) {
            fields.insert("camera".into(), json!(id));
        }
        infos.push(info);
        cells.push(pixels);
    }
    let width = cw.saturating_mul(columns as u32).saturating_add(GAP.saturating_mul(columns as u32 - 1));
    let height = ch.saturating_mul(rows as u32).saturating_add(GAP.saturating_mul(rows as u32 - 1));
    let rgba = if sheet {
        let mut picture = [0u8, 0, 0, 255].repeat(width as usize * height as usize);
        for (index, pixels) in cells.iter().enumerate() {
            let (x0, y0) = ((index % columns) as u32 * (cw + GAP), (index / columns) as u32 * (ch + GAP));
            for y in 0..ch.min(height.saturating_sub(y0)) {
                let row = (y * cw * 4) as usize;
                let take = (cw.min(width.saturating_sub(x0)) * 4) as usize;
                let at = (((y0 + y) * width + x0) * 4) as usize;
                if let (Some(to), Some(from)) = (picture.get_mut(at..at + take), pixels.get(row..row + take)) {
                    to.copy_from_slice(from);
                }
            }
        }
        picture
    } else {
        cells.into_iter().next().unwrap_or_default()
    };
    let png = crate::encode_png(width, height, &rgba)?;
    let mut answer = json!({
        "png": crate::commands::base64(&png),
        "mimeType": "image/png",
        "width": width,
        "height": height,
        "scale": fitted,
        "frames": infos,
        "columns": columns,
        "rows": rows,
    });
    if let (Pass::Id, Some(fields)) = (request.pass, answer.as_object_mut()) {
        let entries: Vec<Value> =
            legend.iter().map(|(id, name, [r, g, b])| json!({ "id": id, "name": name, "color": format!("#{r:02x}{g:02x}{b:02x}") })).collect();
        fields.insert("legend".into(), Value::Array(entries));
    }
    Ok(answer)
}
