//! Animation commands: keyframes (insert, delete, clear, interpolation), the current frame and
//! frame range, jumping between keys, and rendering a frame range. Rotations are keyed in
//! radians and shown in degrees.
//!
//! Objects hold their values at the current frame. Changing the frame writes every animated
//! value; editing an animated value without keying it lasts until the frame changes, as in the
//! reference application. Changing the frame is not an undo step and does not dirty the file.

use std::io::Write as _;

use kition_kit::KitCommand;
use kition_scene_model::{Id, Interpolation, Property, Scene};
use serde_json::{Value, json};

use crate::params::{self, bad, schema as s};
use crate::{Mode, SceneDocument};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Needs {
    Always,
    /// Object mode with objects to act on (selected, or named).
    Objects,
    ObjectMode,
    Camera,
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

fn targets() -> Vec<(&'static str, Value)> {
    vec![
        ("objects", s::objects("The objects; the selection when left out")),
        (
            "properties",
            json!({ "type": "array", "items": { "type": "string", "enum": ["location", "rotation", "scale", "hidden", "power", "color", "lens", "baseColor"] }, "description": "What to key: location, rotation, scale, hidden; power and color of a light; lens of a camera; baseColor of the object's material (location, rotation, and scale when left out)" }),
        ),
        ("frame", s::number("The frame; the current frame when left out")),
        ("keyingSet", s::string("insertKey: key what this keying set names instead (the active one is used when no properties are given)")),
    ]
}

const ANIMATION: &[&str] = &["Object", "Animation"];

static SPECS: &[Spec] = &[
    Spec {
        id: "anim.insertKey",
        title: "Insert Keyframe",
        menu: ANIMATION,
        shortcut: Some("I"),
        mutates: true,
        needs: Needs::Objects,
        schema: || s::of(targets()),
    },
    Spec {
        id: "anim.deleteKey",
        title: "Delete Keyframe",
        menu: ANIMATION,
        shortcut: Some("Alt+I"),
        mutates: true,
        needs: Needs::Objects,
        schema: || s::of(targets()),
    },
    Spec {
        id: "anim.clear",
        title: "Clear Keyframes",
        menu: ANIMATION,
        shortcut: Some("Shift+Alt+I"),
        mutates: true,
        needs: Needs::Objects,
        schema: || {
            s::of(vec![
                ("objects", s::objects("The objects; the selection when left out")),
                (
                    "properties",
                    json!({ "type": "array", "items": { "type": "string" }, "description": "Only these properties (every animated one when left out)" }),
                ),
            ])
        },
    },
    Spec {
        id: "anim.setInterpolation",
        title: "Interpolation",
        menu: ANIMATION,
        shortcut: None,
        mutates: true,
        needs: Needs::Objects,
        schema: || {
            let mut fields = targets();
            fields.push(("interpolation", s::required(s::choice("How the curve runs from a key to the next", &["bezier", "linear", "constant"]))));
            if let Some((_, frame)) = fields.iter_mut().find(|(key, _)| *key == "frame") {
                *frame = s::number("Only the keys on this frame (every key when left out)");
            }
            s::of(fields)
        },
    },
    Spec {
        id: "frame.set",
        title: "Go to Frame",
        menu: &[],
        shortcut: None,
        mutates: false,
        needs: Needs::Always,
        schema: || s::of(vec![("frame", s::required(s::number("The frame")))]),
    },
    Spec {
        id: "frame.next",
        title: "Next Frame",
        menu: &["View", "Frame"],
        shortcut: Some("ArrowRight"),
        mutates: false,
        needs: Needs::Always,
        schema: || s::of(vec![]),
    },
    Spec {
        id: "frame.previous",
        title: "Previous Frame",
        menu: &["View", "Frame"],
        shortcut: Some("ArrowLeft"),
        mutates: false,
        needs: Needs::Always,
        schema: || s::of(vec![]),
    },
    Spec {
        id: "frame.start",
        title: "Jump to Start",
        menu: &["View", "Frame"],
        shortcut: Some("Shift+ArrowLeft"),
        mutates: false,
        needs: Needs::Always,
        schema: || s::of(vec![]),
    },
    Spec {
        id: "frame.end",
        title: "Jump to End",
        menu: &["View", "Frame"],
        shortcut: Some("Shift+ArrowRight"),
        mutates: false,
        needs: Needs::Always,
        schema: || s::of(vec![]),
    },
    Spec {
        id: "anim.nextKey",
        title: "Jump to Next Keyframe",
        menu: &["View", "Frame"],
        shortcut: Some("ArrowUp"),
        mutates: false,
        needs: Needs::Always,
        schema: || s::of(vec![]),
    },
    Spec {
        id: "anim.previousKey",
        title: "Jump to Previous Keyframe",
        menu: &["View", "Frame"],
        shortcut: Some("ArrowDown"),
        mutates: false,
        needs: Needs::Always,
        schema: || s::of(vec![]),
    },
    Spec {
        id: "frame.range",
        title: "Frame Range",
        menu: &["Render"],
        shortcut: None,
        mutates: true,
        needs: Needs::Always,
        schema: || {
            s::of(vec![("start", s::integer("The first frame")), ("end", s::integer("The last frame")), ("fps", s::number("Frames per second"))])
        },
    },
    Spec {
        id: "render.animation",
        title: "Render Animation",
        menu: &["Render"],
        shortcut: Some("Cmd+F12"),
        mutates: false,
        needs: Needs::Camera,
        schema: || {
            s::of(vec![
                ("start", s::integer("The first frame (the range's start when left out)")),
                ("end", s::integer("The last frame (the range's end when left out)")),
                ("step", s::integer("Render every nth frame (1)")),
                ("samples", s::integer("Samples per pixel side, 1 to 4 (1)")),
            ])
        },
    },
];

fn spec_of(id: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|spec| spec.id == id)
}

fn enabled(doc: &SceneDocument, needs: Needs) -> bool {
    let object_mode = doc.session.mode == Mode::Object && doc.session.rig.is_none();
    match needs {
        Needs::Always => true,
        Needs::Objects => object_mode && !doc.session.editor.selected.is_empty(),
        Needs::ObjectMode => object_mode,
        Needs::Camera => doc.scene.camera.is_some(),
    }
}

/// The animation commands, with whether each can run now.
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

/// The animation commands that cannot run now.
pub fn disabled(doc: &SceneDocument) -> Vec<String> {
    SPECS.iter().filter(|spec| !enabled(doc, spec.needs)).map(|spec| spec.id.to_string()).collect()
}

/// Runs an animation command, or answers `None` when `command` is not one.
pub fn execute(doc: &mut SceneDocument, command: &str, params: &Value) -> Option<Result<Value, String>> {
    let spec = spec_of(command)?;
    if !params.is_object() && !params.is_null() {
        return Some(Err(bad("the parameters must be an object")));
    }
    // Objects named by the command count as the ones to act on.
    let named = params.get("objects").is_some_and(|value| !value.is_null());
    let allowed = enabled(doc, spec.needs) || (spec.needs == Needs::Objects && named && enabled(doc, Needs::ObjectMode));
    if !allowed {
        return Some(Err(format!("not-allowed: {} cannot run now", spec.title)));
    }
    if !spec.mutates {
        return Some(run_view(doc, command, params));
    }
    let before = doc.step(spec.title);
    let session = doc.session.clone();
    Some(match run_change(doc, command, params) {
        Ok(value) => {
            doc.tidy_session();
            if let Err(error) = doc.scene.validate() {
                doc.restore(before);
                doc.session = session;
                return Some(Err(format!("failed: the change would break the scene ({error})")));
            }
            doc.committed(before).map(|()| value)
        }
        Err(error) => {
            doc.restore(before);
            doc.session = session;
            Err(error)
        }
    })
}

fn frame_param(doc: &SceneDocument, params: &Value) -> Result<f32, String> {
    Ok(params::number(params, "frame")?.unwrap_or(doc.scene.animation.timeline.frame as f32))
}

fn properties(params: &Value, fallback: &[Property]) -> Result<Vec<Property>, String> {
    match params.get("properties").filter(|value| !value.is_null()) {
        None => Ok(fallback.to_vec()),
        Some(Value::Array(items)) => {
            items.iter().map(|item| item.as_str().and_then(Property::parse).ok_or_else(|| bad(format!("unknown property {item}")))).collect()
        }
        Some(Value::String(name)) => Property::parse(name).map(|property| vec![property]).ok_or_else(|| bad(format!("unknown property {name}"))),
        Some(_) => Err(bad("`properties` is a list of property names")),
    }
}

fn objects(doc: &SceneDocument, params: &Value) -> Result<Vec<Id>, String> {
    match params::objects(&doc.scene, params, "objects")? {
        Some(ids) => Ok(ids),
        None if !doc.session.editor.selected.is_empty() => Ok(doc.session.editor.selected.clone()),
        None => Err("not-allowed: select an object first, or name `objects`".into()),
    }
}

/// The id a property of an object is keyed on: the object, or its material.
fn target_of(scene: &Scene, object: Id, property: Property) -> Result<Id, String> {
    if property.on_material() {
        return scene.object(object).map_err(|error| error.to_string())?.material.ok_or_else(|| bad("the object has no material to key"));
    }
    Ok(object)
}

const TRANSFORM: &[Property] = &[Property::Location, Property::Rotation, Property::Scale];

fn run_change(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    match command {
        "anim.insertKey" => {
            let frame = frame_param(doc, params)?;
            if let Some(set) = crate::commands_anim2::keying_set_for(doc, params)? {
                let objects = objects(doc, params)?;
                let keys = doc.scene.insert_keying_set(&set, &objects, None, frame).map_err(|error| error.to_string())?;
                return Ok(json!({ "frame": frame, "keys": keys }));
            }
            let wanted = properties(params, TRANSFORM)?;
            let mut keys = 0;
            for object in objects(doc, params)? {
                for property in &wanted {
                    let target = target_of(&doc.scene, object, *property)?;
                    keys += doc.scene.insert_key(target, *property, frame).map_err(|error| error.to_string())?;
                }
            }
            Ok(json!({ "frame": frame, "keys": keys }))
        }
        "anim.deleteKey" => {
            let frame = frame_param(doc, params)?;
            let wanted = properties(params, &Property::ALL)?;
            let mut keys = 0;
            for object in objects(doc, params)? {
                for property in &wanted {
                    if let Ok(target) = target_of(&doc.scene, object, *property) {
                        keys += doc.scene.delete_key(target, *property, frame);
                    }
                }
            }
            if keys == 0 {
                return Err(format!("not-allowed: there is no keyframe on frame {frame}"));
            }
            Ok(json!({ "frame": frame, "deleted": keys }))
        }
        "anim.clear" => {
            let only = match params.get("properties").filter(|value| !value.is_null()) {
                Some(_) => Some(properties(params, &[])?),
                None => None,
            };
            let mut channels = 0;
            for object in objects(doc, params)? {
                match &only {
                    None => {
                        channels += doc.scene.clear_animation(object, None);
                        if let Ok(target) = target_of(&doc.scene, object, Property::BaseColor) {
                            channels += doc.scene.clear_animation(target, Some(Property::BaseColor));
                        }
                    }
                    Some(list) => {
                        for property in list {
                            if let Ok(target) = target_of(&doc.scene, object, *property) {
                                channels += doc.scene.clear_animation(target, Some(*property));
                            }
                        }
                    }
                }
            }
            if channels == 0 {
                return Err("not-allowed: nothing is animated".into());
            }
            Ok(json!({ "cleared": channels }))
        }
        "anim.setInterpolation" => {
            let interpolation = params::string(params, "interpolation")?
                .and_then(Interpolation::parse)
                .ok_or_else(|| bad("`interpolation` must be bezier, linear, or constant"))?;
            let frame = params::number(params, "frame")?;
            let wanted = properties(params, &Property::ALL)?;
            let mut targets = Vec::new();
            for object in objects(doc, params)? {
                for property in &wanted {
                    if let Ok(target) = target_of(&doc.scene, object, *property) {
                        targets.push((target, *property));
                    }
                }
            }
            // In tweak mode `frame` is a scene frame: the key is at the strip's action frame, as
            // insertKey and deleteKey place and find it (material and shape key keys are not mapped).
            let frames: Vec<Option<f32>> = targets
                .iter()
                .map(|(target, property)| {
                    frame.map(
                        |frame| if property.on_material() || *property == Property::ShapeKey { frame } else { doc.scene.key_time(*target, frame) },
                    )
                })
                .collect();
            let mut changed = 0;
            for channel in doc.scene.animation.channels.iter_mut() {
                let Some(at) = targets.iter().position(|wanted| *wanted == (channel.target, channel.property)) else { continue };
                if channel.property == Property::Hidden {
                    continue;
                }
                let frame = frames.get(at).copied().flatten();
                for key in channel.keys.iter_mut().filter(|key| frame.is_none_or(|frame| (key.frame - frame).abs() <= 1e-4)) {
                    key.interpolation = interpolation;
                    changed += 1;
                }
            }
            if changed == 0 {
                return Err("not-allowed: no keyframe to change".into());
            }
            Ok(json!({ "changed": changed }))
        }
        "frame.range" => {
            let timeline = &mut doc.scene.animation.timeline;
            if let Some(start) = params::number(params, "start")? {
                timeline.start = start.round() as i32;
            }
            if let Some(end) = params::number(params, "end")? {
                timeline.end = end.round() as i32;
            }
            if let Some(fps) = params::number(params, "fps")? {
                timeline.fps = fps;
            }
            let timeline = doc.scene.animation.timeline;
            if timeline.start > timeline.end {
                return Err(bad("the range must start before it ends"));
            }
            if !(timeline.fps > 0.0 && timeline.fps <= 1000.0) {
                return Err(bad("`fps` must be between 0 and 1000"));
            }
            Ok(json!({ "start": timeline.start, "end": timeline.end, "fps": timeline.fps }))
        }
        other => Err(format!("unknown-command: {other}")),
    }
}

fn go(doc: &mut SceneDocument, frame: f32) -> Result<Value, String> {
    if !frame.is_finite() || frame.abs() > kition_scene_model::animation::MAX_FRAME as f32 {
        return Err(bad("the frame is out of range"));
    }
    let frame = frame.round();
    doc.scene.go_to_frame(frame);
    doc.touched();
    Ok(json!({ "frame": doc.scene.animation.timeline.frame }))
}

fn run_view(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    let timeline = doc.scene.animation.timeline;
    let current = timeline.frame as f32;
    match command {
        "frame.set" => go(doc, params::need_number(params, "frame")?),
        // Stepping past the end wraps to the start, as playback does.
        "frame.next" => go(doc, if timeline.frame >= timeline.end { timeline.start as f32 } else { current + 1.0 }),
        "frame.previous" => go(doc, if timeline.frame <= timeline.start { timeline.end as f32 } else { current - 1.0 }),
        "frame.start" => go(doc, timeline.start as f32),
        "frame.end" => go(doc, timeline.end as f32),
        "anim.nextKey" | "anim.previousKey" => {
            let keys = doc.scene.key_frames(&[]);
            let found = if command == "anim.nextKey" {
                keys.into_iter().find(|frame| *frame > current + 1e-4)
            } else {
                keys.into_iter().rev().find(|frame| *frame < current - 1e-4)
            };
            match found {
                Some(frame) => go(doc, frame),
                None => Err("not-allowed: there is no keyframe that way".into()),
            }
        }
        "render.animation" => render_animation(doc, params),
        other => Err(format!("unknown-command: {other}")),
    }
}

/// The most pixels a frame-range render may make altogether.
const MAX_ANIMATION_PIXELS: u64 = 400_000_000;
/// The most frames a frame-range render may make.
const MAX_ANIMATION_FRAMES: i64 = 1000;

fn render_animation(doc: &SceneDocument, params: &Value) -> Result<Value, String> {
    let timeline = doc.scene.animation.timeline;
    let start = params::number(params, "start")?.map_or(timeline.start, |frame| frame.round() as i32);
    let end = params::number(params, "end")?.map_or(timeline.end, |frame| frame.round() as i32);
    let step = params::integer(params, "step")?.unwrap_or(1).max(1);
    let samples = params::integer(params, "samples")?.unwrap_or(1).clamp(1, 4);
    if start > end {
        return Err(bad("the range must start before it ends"));
    }
    let frames: Vec<i32> = (start..=end).step_by(step as usize).collect();
    let pixels = u64::from(doc.scene.render.width) * u64::from(doc.scene.render.height) * u64::from(samples * samples);
    if frames.len() as i64 > MAX_ANIMATION_FRAMES || pixels.saturating_mul(frames.len() as u64) > MAX_ANIMATION_PIXELS {
        return Err(bad("the range is too long to render at once; render fewer frames or a smaller picture"));
    }
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored).last_modified_time(zip::DateTime::default());
    for frame in &frames {
        let png = doc.render_png_at(samples, Some(*frame as f32))?;
        zip.start_file(format!("frame-{frame:04}.png"), options).map_err(|error| format!("failed: {error}"))?;
        zip.write_all(&png).map_err(|error| format!("failed: {error}"))?;
    }
    let bytes = zip.finish().map_err(|error| format!("failed: {error}"))?.into_inner();
    Ok(json!({ "mime": "application/zip", "extension": "zip", "frames": frames.len(), "base64": crate::commands::base64(&bytes) }))
}

impl SceneDocument {
    /// A PNG of the scene rendered through its camera at `frame` (the current frame when `None`).
    pub fn render_png_at(&self, samples: u32, frame: Option<f32>) -> Result<Vec<u8>, String> {
        let scene = match frame {
            Some(frame) if frame.is_finite() => crate::commands_particles::render_scene(self, self.scene.at_frame(frame)),
            Some(_) => return Err(bad("the frame is not a number")),
            None => return self.render_png(samples),
        };
        crate::commands_compositor::camera_png(self, &scene, samples)
    }
}

fn round(value: f32) -> f64 {
    (f64::from(value) * 10_000.0).round() / 10_000.0
}

/// Adds the timeline, the channels, and each object's animated properties to inspect.
pub fn extend_inspect(doc: &SceneDocument, inspected: &mut Value) {
    let scene = &doc.scene;
    let timeline = scene.animation.timeline;
    let channels: Vec<Value> = scene
        .animation
        .channels
        .iter()
        .map(|channel| {
            let degrees = matches!(channel.property, Property::Rotation | Property::BoneRotation);
            let keys: Vec<Value> = channel
                .keys
                .iter()
                .map(|key| {
                    let value = if degrees { key.value.to_degrees() } else { key.value };
                    json!({ "frame": round(key.frame), "value": round(value), "interpolation": match key.interpolation { Interpolation::Bezier => "bezier", Interpolation::Linear => "linear", Interpolation::Constant => "constant" } })
                })
                .collect();
            let owner = if channel.property.on_material() { "material" } else { "object" };
            let mut entry = json!({ "property": channel.property.name(), "index": channel.index, "keys": keys });
            if let Some(fields) = entry.as_object_mut() {
                fields.insert(owner.into(), json!(channel.target));
                if !channel.path.is_empty() {
                    fields.insert("path".into(), json!(channel.path));
                }
            }
            entry
        })
        .collect();
    let selected = doc.session.editor.selected.clone();
    let mut keyed: Vec<Id> = selected.clone();
    for id in &selected {
        if let Some(material) = scene.object(*id).ok().and_then(|object| object.material) {
            keyed.push(material);
        }
    }
    let Some(map) = inspected.as_object_mut() else { return };
    map.insert(
        "animation".into(),
        json!({
            "frame": timeline.frame,
            "start": timeline.start,
            "end": timeline.end,
            "fps": round(timeline.fps),
            "selectedKeyFrames": scene.key_frames(&keyed).into_iter().map(round).collect::<Vec<_>>(),
            "keyFrames": scene.key_frames(&[]).into_iter().map(round).collect::<Vec<_>>(),
            "channels": channels,
        }),
    );
    if let Some(objects) = map.get_mut("objects").and_then(Value::as_array_mut) {
        for entry in objects {
            let Some(id) = entry.get("id").and_then(Value::as_u64).and_then(|id| u32::try_from(id).ok()) else { continue };
            let material = scene.object(id).ok().and_then(|object| object.material);
            let mut animated: Vec<&str> = Vec::new();
            for channel in &scene.animation.channels {
                let mine = if channel.property.on_material() { Some(channel.target) == material } else { channel.target == id };
                if mine && !animated.contains(&channel.property.name()) {
                    animated.push(channel.property.name());
                }
            }
            // Properties with a key on the current frame.
            let mut keyed_now: Vec<&str> = Vec::new();
            for channel in &scene.animation.channels {
                let mine = if channel.property.on_material() { Some(channel.target) == material } else { channel.target == id };
                if mine
                    && channel.keys.iter().any(|key| (key.frame - timeline.frame as f32).abs() <= 1e-4)
                    && !keyed_now.contains(&channel.property.name())
                {
                    keyed_now.push(channel.property.name());
                }
            }
            if let (false, Some(fields)) = (animated.is_empty(), entry.as_object_mut()) {
                fields.insert("animated".into(), json!(animated));
                fields.insert("keyedNow".into(), json!(keyed_now));
            }
        }
    }
}
