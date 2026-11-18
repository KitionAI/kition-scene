//! The NLA and F-curve modifiers ([`kition_scene_model::nla`], [`kition_scene_model::fmodifier`]).
//!
//! An object's NLA stacks tracks of action strips under its own keys: `nla.pushDown` makes the
//! object's keys an action played by a strip on a new track, `nla.addStrip` plays an action,
//! `nla.setStrip` changes a strip's frames, scale, repeat, blends, extrapolation, blend type,
//! and influence (or moves it to another track), `nla.setTrack` mutes, solos, and renames
//! tracks, and `nla.setAction` sets how the object's own keys go on top. `nla.tweak` edits a
//! strip's action in place (its keys become the object's own until `nla.tweakExit`).
//!
//! `graph.addModifier`, `graph.setModifier`, and `graph.removeModifier` edit a channel's
//! F-curve modifiers (Generator, Built-in Function, Envelope, Cycles, Noise, Stepped, Limits), with
//! their frame range, influence, and mute. Settings use the names `inspect()` shows them by.

use kition_kit::KitCommand;
use kition_scene_model::fmodifier::{FModifier, FModifierKind};
use kition_scene_model::nla::{ActionBlend, BlendType, Extrapolation, MIN_REPEAT, MIN_SCALE, NlaStrip, StripKind};
use kition_scene_model::{Id, Property, Scene};
use serde_json::{Map, Value, json};

use crate::params::{self, bad, schema as s};
use crate::{Mode, SceneDocument};

struct Spec {
    id: &'static str,
    title: &'static str,
    menu: &'static [&'static str],
    schema: fn() -> Value,
}

const NLA: &[&str] = &["Object", "Animation", "NLA"];
const MODIFIERS: &[&str] = &["Object", "Animation", "F-Curve Modifiers"];

const BLENDS: &[&str] = &["replace", "combine", "add", "subtract", "multiply"];
const EXTRAPOLATIONS: &[&str] = &["hold", "holdForward", "nothing"];
const KINDS: &[&str] = &["generator", "function", "envelope", "cycles", "noise", "stepped", "limits"];

fn object_field() -> (&'static str, Value) {
    ("object", s::object("The object; the active one when left out"))
}

fn track_field() -> (&'static str, Value) {
    ("track", s::required(s::integer("The track, from 0 at the bottom")))
}

fn strip_fields() -> Vec<(&'static str, Value)> {
    vec![object_field(), track_field(), ("strip", s::required(s::integer("The strip on the track, from 0 (frame order)")))]
}

fn strip_settings() -> Vec<(&'static str, Value)> {
    vec![
        ("name", s::string("The strip's name")),
        ("start", s::number("The scene frame it starts at")),
        ("end", s::number("Meta strips: move or stretch the children together to this end frame")),
        ("actionStart", s::number("The first action frame it plays")),
        ("actionEnd", s::number("The last action frame it plays")),
        ("scale", s::number("How many times slower it plays (0.0001 or more)")),
        ("repeat", s::number("How many times it plays (0.01 or more)")),
        ("reversed", s::boolean("Plays backwards")),
        ("blendIn", s::number("Frames its influence rises over at its start")),
        ("blendOut", s::number("Frames its influence falls over at its end")),
        (
            "autoBlend",
            s::boolean("Blends in and out over the strips of the tracks below and above it that overlap it (switching it off clears the blends)"),
        ),
        (
            "extrapolation",
            s::choice(
                "Outside its frames: hold (both ways; only the first strip of a track holds backwards), holdForward, or nothing",
                EXTRAPOLATIONS,
            ),
        ),
        ("blend", s::choice("How it mixes onto the tracks below", BLENDS)),
        ("influence", s::number("A fixed influence from 0 to 1 in place of the blends; null for the blends")),
        ("muted", s::boolean("Does nothing")),
    ]
}

fn channel_fields() -> Vec<(&'static str, Value)> {
    vec![
        ("target", s::required(s::integer("The channel's object (or material) id"))),
        ("property", s::required(s::string("The channel's property (location, boneRotation, shapeKey, ...)"))),
        ("index", s::integer("The component, from 0 (0)")),
        ("path", s::string("The bone or shape key, for their properties")),
    ]
}

fn modifier_settings() -> Vec<(&'static str, Value)> {
    vec![
        ("muted", s::boolean("Does nothing (values of rotation channels are radians)")),
        ("influence", s::number("Scales its effect, 0 to 1; null for full")),
        (
            "range",
            json!({ "type": "object", "additionalProperties": true, "description": "Limits it to { start, end, blendIn, blendOut } frames; null for everywhere" }),
        ),
        ("mode", s::choice("generator: expanded (c0 + c1·x + …) or factorized ((c0·x + c1)(c2·x + c3)…)", &["expanded", "factorized"])),
        ("coefficients", json!({ "type": "array", "items": { "type": "number" }, "description": "generator: the coefficients" })),
        ("additive", s::boolean("generator, function: adds to the curve instead of replacing it")),
        (
            "function",
            s::choice(
                "function: the function of amplitude · f(phaseMultiplier · frame + phaseOffset) + valueOffset",
                &["sin", "cos", "tan", "sqrt", "ln", "sinc"],
            ),
        ),
        ("amplitude", s::number("function")),
        ("phaseMultiplier", s::number("function")),
        ("phaseOffset", s::number("function")),
        ("valueOffset", s::number("function")),
        ("reference", s::number("envelope: the reference value")),
        ("min", s::number("envelope: the low end, below the reference")),
        ("max", s::number("envelope: the high end, above the reference")),
        (
            "points",
            json!({ "type": "array", "items": { "type": "object", "additionalProperties": true }, "description": "envelope: control points { frame, min, max } in frame order" }),
        ),
        ("before", s::choice("cycles: before the first key", &["none", "repeat", "repeatOffset", "mirror"])),
        ("after", s::choice("cycles: after the last key", &["none", "repeat", "repeatOffset", "mirror"])),
        ("beforeCount", s::integer("cycles: cycles before (0 forever)")),
        ("afterCount", s::integer("cycles: cycles after (0 forever)")),
        ("blend", s::choice("noise: how it mixes in", &["replace", "add", "subtract", "multiply"])),
        ("scale", s::number("noise: frames per noise feature")),
        ("strength", s::number("noise")),
        ("phase", s::number("noise")),
        ("offset", s::number("noise, stepped: frame offset")),
        ("depth", s::integer("noise: octaves beyond the first (0 to 32)")),
        ("lacunarity", s::number("noise: frequency step between octaves")),
        ("roughness", s::number("noise: amplitude step between octaves")),
        ("minX", json!({ "oneOf": [{ "type": "number" }, { "type": "null" }], "description": "limits: minimum evaluated frame; null disables" })),
        ("maxX", json!({ "oneOf": [{ "type": "number" }, { "type": "null" }], "description": "limits: maximum evaluated frame; null disables" })),
        ("minY", json!({ "oneOf": [{ "type": "number" }, { "type": "null" }], "description": "limits: minimum curve value; null disables" })),
        ("maxY", json!({ "oneOf": [{ "type": "number" }, { "type": "null" }], "description": "limits: maximum curve value; null disables" })),
        ("step", s::number("stepped: frames each step holds")),
        ("start", s::number("stepped: steps only from this frame; null for always")),
        ("end", s::number("stepped: steps only up to this frame; null for always")),
    ]
}

static SPECS: &[Spec] = &[
    Spec {
        id: "nla.pushDown",
        title: "Push Down",
        menu: NLA,
        schema: || s::of(vec![object_field(), ("name", s::string("The new action's name (<object>Action)"))]),
    },
    Spec {
        id: "nla.addTrack",
        title: "Add NLA Track",
        menu: NLA,
        schema: || {
            s::of(vec![
                object_field(),
                ("name", s::string("Its name (NlaTrack)")),
                ("index", s::integer("Where, from 0 at the bottom; on top when left out")),
            ])
        },
    },
    Spec { id: "nla.removeTrack", title: "Remove NLA Track", menu: &[], schema: || s::of(vec![object_field(), track_field()]) },
    Spec {
        id: "nla.setTrack",
        title: "NLA Track Settings",
        menu: &[],
        schema: || {
            s::of(vec![
                object_field(),
                track_field(),
                ("name", s::string("Its name")),
                ("muted", s::boolean("Does nothing")),
                ("solo", s::boolean("The only track evaluated (one at a time)")),
                ("moveTo", s::integer("Moves it to this position, from 0 at the bottom")),
            ])
        },
    },
    Spec {
        id: "nla.addStrip",
        title: "Add Action Strip",
        menu: NLA,
        schema: || {
            s::of(vec![
                object_field(),
                ("action", s::required(s::string("The action it plays"))),
                ("track", s::integer("The track; the top one when it has room, else a new one")),
                ("start", s::number("The frame it starts at; the current frame when left out")),
            ])
        },
    },
    Spec {
        id: "nla.setStrip",
        title: "Action Strip Settings",
        menu: &[],
        schema: || {
            let mut fields = strip_fields();
            fields.extend(strip_settings());
            fields.push(("toTrack", s::integer("Moves it to this track")));
            s::of(fields)
        },
    },
    Spec { id: "nla.addTransition", title: "Add Transition Strip", menu: NLA, schema: || s::of(strip_fields()) },
    Spec {
        id: "nla.makeMeta",
        title: "Make Meta Strip",
        menu: NLA,
        schema: || {
            s::of(vec![
                object_field(),
                track_field(),
                ("first", s::required(s::integer("First strip in the contiguous group"))),
                ("last", s::required(s::integer("Last strip in the contiguous group, inclusive"))),
            ])
        },
    },
    Spec { id: "nla.dissolveMeta", title: "Dissolve Meta Strip", menu: NLA, schema: || s::of(strip_fields()) },
    Spec { id: "nla.removeStrip", title: "Remove Action Strip", menu: &[], schema: || s::of(strip_fields()) },
    Spec {
        id: "nla.setAction",
        title: "Active Action Blending",
        menu: &[],
        schema: || {
            s::of(vec![
                object_field(),
                ("blend", s::choice("How the object's own keys mix onto the tracks", BLENDS)),
                ("influence", s::number("Their influence, 0 to 1")),
                ("extrapolation", s::choice("Outside their keyed frames", EXTRAPOLATIONS)),
            ])
        },
    },
    Spec {
        id: "nla.tweak",
        title: "Start Tweaking Strip Action",
        menu: NLA,
        schema: || {
            let mut fields = strip_fields();
            fields.push(("upperStack", s::boolean("Evaluates the tracks above too (false: the lower stack only)")));
            s::of(fields)
        },
    },
    Spec { id: "nla.tweakExit", title: "Stop Tweaking Strip Action", menu: NLA, schema: || s::of(vec![object_field()]) },
    Spec {
        id: "graph.addModifier",
        title: "Add F-Curve Modifier",
        menu: MODIFIERS,
        schema: || {
            let mut fields = channel_fields();
            fields.push(("type", s::required(s::choice("The modifier", KINDS))));
            fields.extend(modifier_settings());
            s::of(fields)
        },
    },
    Spec {
        id: "graph.setModifier",
        title: "F-Curve Modifier Settings",
        menu: &[],
        schema: || {
            let mut fields = channel_fields();
            fields.push(("modifier", s::required(s::integer("The modifier's place in the stack, from 0"))));
            fields.extend(modifier_settings());
            fields.push(("moveTo", s::integer("Moves it to this place in the stack")));
            s::of(fields)
        },
    },
    Spec {
        id: "graph.removeModifier",
        title: "Remove F-Curve Modifier",
        menu: &[],
        schema: || {
            let mut fields = channel_fields();
            fields.push(("modifier", s::required(s::integer("The modifier's place in the stack, from 0"))));
            s::of(fields)
        },
    },
];

fn enabled(doc: &SceneDocument) -> bool {
    doc.session.mode == Mode::Object
}

/// The commands, with whether each can run now.
pub fn catalogue(doc: &SceneDocument) -> Vec<KitCommand> {
    SPECS
        .iter()
        .map(|spec| KitCommand {
            id: spec.id.into(),
            title: spec.title.into(),
            menu: (!spec.menu.is_empty()).then(|| spec.menu.iter().map(|part| (*part).to_string()).collect()),
            shortcut: None,
            params: None,
            schema: Some((spec.schema)()),
            mutates: true,
            enabled: enabled(doc),
            ..Default::default()
        })
        .collect()
}

/// The commands that cannot run now.
pub fn disabled(doc: &SceneDocument) -> Vec<String> {
    if enabled(doc) { Vec::new() } else { SPECS.iter().map(|spec| spec.id.to_string()).collect() }
}

/// Runs one of these commands, or answers `None` when `command` is not one.
pub fn execute(doc: &mut SceneDocument, command: &str, params: &Value) -> Option<Result<Value, String>> {
    let spec = SPECS.iter().find(|spec| spec.id == command)?;
    if !params.is_object() && !params.is_null() {
        return Some(Err(bad("the parameters must be an object")));
    }
    if !enabled(doc) {
        return Some(Err(format!("not-allowed: {} runs in object mode", spec.title)));
    }
    let before = doc.step(spec.title);
    let session = doc.session.clone();
    Some(match run(doc, command, params) {
        Ok(value) => {
            let frame = doc.scene.animation.timeline.frame as f32;
            doc.scene.go_to_frame(frame);
            doc.tidy_session();
            if let Err(error) = doc.scene.validate() {
                doc.restore(before);
                doc.session = session;
                return Some(Err(format!("not-allowed: {}", error.to_string().replace("invalid-file: ", ""))));
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

fn object_of(doc: &SceneDocument, params: &Value) -> Result<Id, String> {
    match params.get("object").filter(|value| !value.is_null()) {
        Some(value) => params::resolve(&doc.scene, value),
        None => doc.session.editor.active.ok_or_else(|| "not-allowed: there is no active object; name `object`".to_string()),
    }
}

fn index(params: &Value, key: &str) -> Result<usize, String> {
    params::integer(params, key)?.map(|value| value as usize).ok_or_else(|| bad(format!("`{key}` is required")))
}

fn not_tweaking(scene: &Scene, object: Id) -> Result<(), String> {
    if scene.tweaked_strip(object).is_some() {
        return Err("not-allowed: leave tweak mode first".into());
    }
    Ok(())
}

fn model(error: kition_scene_model::SceneError) -> String {
    error.to_string()
}

/// `fields` of `params` laid over `base` (a serialized setting), read back as `T`.
fn merged<T: serde::de::DeserializeOwned>(base: Value, params: &Value, fields: &[&str], what: &str) -> Result<T, String> {
    let mut map = match base {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    for field in fields {
        if let Some(value) = params.get(*field) {
            map.insert((*field).to_string(), value.clone());
        }
    }
    serde_json::from_value(Value::Object(map)).map_err(|error| bad(format!("{what}: {error}")))
}

const STRIP_FIELDS: &[&str] = &[
    "name",
    "start",
    "actionStart",
    "actionEnd",
    "scale",
    "repeat",
    "reversed",
    "blendIn",
    "blendOut",
    "autoBlend",
    "extrapolation",
    "blend",
    "influence",
    "muted",
];

const MODIFIER_FIELDS: &[&str] = &[
    "muted",
    "influence",
    "range",
    "mode",
    "coefficients",
    "additive",
    "function",
    "amplitude",
    "phaseMultiplier",
    "phaseOffset",
    "valueOffset",
    "reference",
    "min",
    "max",
    "points",
    "before",
    "after",
    "beforeCount",
    "afterCount",
    "blend",
    "scale",
    "strength",
    "phase",
    "offset",
    "depth",
    "lacunarity",
    "roughness",
    "minX",
    "maxX",
    "minY",
    "maxY",
    "step",
    "start",
    "end",
];

fn check_strip(strip: &NlaStrip) -> Result<(), String> {
    let finite = [strip.start, strip.action_start, strip.action_end, strip.scale, strip.repeat, strip.blend_in, strip.blend_out]
        .iter()
        .all(|value| value.is_finite() && value.abs() <= kition_scene_model::animation::MAX_FRAME as f32);
    if !finite || strip.action_end <= strip.action_start {
        return Err(bad("a strip's frames are finite, its action end after its action start"));
    }
    if strip.scale < MIN_SCALE
        || strip.repeat < MIN_REPEAT
        || !strip.end().is_finite()
        || strip.end().abs() > kition_scene_model::animation::MAX_FRAME as f32
    {
        return Err(bad(format!("a strip's scale is at least {MIN_SCALE} and its repeat at least {MIN_REPEAT}, and it ends in the frame range")));
    }
    if strip.blend_in < 0.0 || strip.blend_out < 0.0 || strip.name.trim().is_empty() || strip.name.len() > 128 {
        return Err(bad("blends are 0 or more, and a strip has a name"));
    }
    if strip.influence.is_some_and(|influence| !(0.0..=1.0).contains(&influence)) {
        return Err(bad("an influence is from 0 to 1"));
    }
    Ok(())
}

fn run(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    if command.starts_with("graph.") {
        return modifiers(doc, command, params);
    }
    let object = object_of(doc, params)?;
    let scene = &mut doc.scene;
    match command {
        "nla.pushDown" => {
            let action = scene.nla_push_down(object, params::string(params, "name")?).map_err(model)?;
            Ok(json!({ "object": object, "action": action, "track": scene.object(object).map_err(model)?.nla.tracks.len().saturating_sub(1) }))
        }
        "nla.addTrack" => {
            let at = params::integer(params, "index")?.map(|at| at as usize);
            let track = scene.nla_add_track(object, params::string(params, "name")?, at).map_err(model)?;
            Ok(json!({ "object": object, "track": track }))
        }
        "nla.removeTrack" => {
            not_tweaking(scene, object)?;
            let track = index(params, "track")?;
            let nla = &mut scene.object_mut(object).map_err(model)?.nla;
            if track >= nla.tracks.len() {
                return Err(format!("not-found: no track {track}"));
            }
            nla.tracks.remove(track);
            nla.settle();
            Ok(json!({ "object": object, "tracks": nla.tracks.len() }))
        }
        "nla.setTrack" => {
            not_tweaking(scene, object)?;
            let track = index(params, "track")?;
            let nla = &mut scene.object_mut(object).map_err(model)?.nla;
            let count = nla.tracks.len();
            let taken: Vec<String> = nla.tracks.iter().enumerate().filter(|(slot, _)| *slot != track).map(|(_, other)| other.name.clone()).collect();
            let found = nla.tracks.get_mut(track).ok_or_else(|| format!("not-found: no track {track}"))?;
            if let Some(name) = params::string(params, "name")? {
                let name = name.trim();
                if name.is_empty() || name.len() > 128 || taken.iter().any(|other| other == name) {
                    return Err(bad("a track's name is not empty, at most 128 bytes, and its own"));
                }
                found.name = name.to_string();
            }
            if let Some(muted) = params::boolean(params, "muted")? {
                found.muted = muted;
            }
            let solo = params::boolean(params, "solo")?;
            if let Some(solo) = solo {
                found.solo = solo;
            }
            if solo == Some(true) {
                for (slot, other) in nla.tracks.iter_mut().enumerate() {
                    other.solo = slot == track;
                }
            }
            let mut at = track;
            if let Some(to) = params::integer(params, "moveTo")? {
                let to = (to as usize).min(count.saturating_sub(1));
                if track < nla.tracks.len() {
                    let moved = nla.tracks.remove(track);
                    nla.tracks.insert(to, moved);
                    at = to;
                }
            }
            nla.settle();
            Ok(json!({ "object": object, "track": at }))
        }
        "nla.addStrip" => {
            let action = params::string(params, "action")?.ok_or_else(|| bad("`action` is required"))?;
            let start = params::number(params, "start")?.unwrap_or(scene.animation.timeline.frame as f32);
            let track = params::integer(params, "track")?.map(|track| track as usize);
            let (track, strip) = scene.nla_add_strip(object, track, action, start).map_err(model)?;
            Ok(json!({ "object": object, "track": track, "strip": strip }))
        }
        "nla.addTransition" => {
            let track = index(params, "track")?;
            let strip = scene.nla_add_transition(object, track, index(params, "strip")?).map_err(model)?;
            Ok(json!({ "object": object, "track": track, "strip": strip }))
        }
        "nla.makeMeta" => {
            let track = index(params, "track")?;
            let strip = scene.nla_make_meta(object, track, index(params, "first")?, index(params, "last")?).map_err(model)?;
            Ok(json!({ "object": object, "track": track, "strip": strip }))
        }
        "nla.dissolveMeta" => {
            let (track, strip) = (index(params, "track")?, index(params, "strip")?);
            scene.nla_dissolve_meta(object, track, strip).map_err(model)?;
            Ok(json!({ "object": object, "track": track, "strip": strip }))
        }
        "nla.setStrip" => {
            not_tweaking(scene, object)?;
            let (track, slot) = (index(params, "track")?, index(params, "strip")?);
            let nla = &mut scene.object_mut(object).map_err(model)?.nla;
            let old = nla
                .tracks
                .get(track)
                .and_then(|found| found.strips.get(slot))
                .cloned()
                .ok_or_else(|| format!("not-found: no strip {slot} on track {track}"))?;
            let mut strip: NlaStrip = merged(serde_json::to_value(&old).map_err(|error| error.to_string())?, params, STRIP_FIELDS, "the strip")?;
            if old.kind == StripKind::Transition {
                if ["start", "end", "actionStart", "actionEnd", "scale", "repeat", "toTrack"].iter().any(|field| params.get(*field).is_some()) {
                    return Err(bad("a transition follows its two neighbors; move those strips instead"));
                }
            } else if old.kind == StripKind::Meta {
                if ["actionStart", "actionEnd", "scale", "repeat", "reversed"].iter().any(|field| params.get(*field).is_some()) {
                    return Err(bad("a meta's timing is set with start and end"));
                }
                let start = params::number(params, "start")?.unwrap_or(old.start);
                let end = params::number(params, "end")?.unwrap_or(start + old.length());
                strip.start = old.start;
                strip.resize_meta(start, end).map_err(model)?;
            } else if params.get("end").is_some() {
                return Err(bad("end applies to meta strips; action strips use scale and action range"));
            }
            if params::boolean(params, "autoBlend")? == Some(false) {
                strip.blend_in = 0.0;
                strip.blend_out = 0.0;
            }
            check_strip(&strip)?;
            let to = params::integer(params, "toTrack")?.map_or(track, |to| to as usize);
            if to >= nla.tracks.len() {
                return Err(format!("not-found: no track {to}"));
            }
            let except = (to == track).then_some(slot);
            if !nla.tracks.get(to).is_some_and(|found| {
                found.strips.iter().enumerate().all(|(at, other)| {
                    Some(at) == except
                        || (to == track && other.kind == StripKind::Transition)
                        || strip.end() <= other.start
                        || strip.start >= other.end()
                })
            }) {
                return Err(bad("the strip would overlap another on its track"));
            }
            if let Some(found) = nla.tracks.get_mut(track)
                && slot < found.strips.len()
            {
                found.strips.remove(slot);
            }
            let found = nla.tracks.get_mut(to).ok_or_else(|| format!("not-found: no track {to}"))?;
            if found.strips.iter().any(|other| other.name == strip.name) {
                strip.name = kition_scene_model::scene::unique_name(&strip.name, |taken| found.strips.iter().any(|other| other.name == taken));
            }
            let name = strip.name.clone();
            // Keep the original neighbors until transitions have followed the edited bounds.
            // Sorting against their old frame ranges first can orphan a valid transition.
            let at = if to == track { slot } else { found.strips.iter().position(|other| other.start > strip.start).unwrap_or(found.strips.len()) };
            found.strips.insert(at, strip);
            nla.settle();
            if to == track {
                if let Some(found) = nla.tracks.get_mut(to) {
                    found.strips.sort_by(|a, b| a.start.total_cmp(&b.start));
                }
                nla.settle();
            }
            let at = nla.tracks.get(to).and_then(|found| found.strips.iter().position(|strip| strip.name == name)).unwrap_or(at);
            let strip = nla.tracks.get(to).and_then(|found| found.strips.get(at)).map(strip_json).unwrap_or(Value::Null);
            Ok(json!({ "object": object, "track": to, "strip": at, "settings": strip }))
        }
        "nla.removeStrip" => {
            not_tweaking(scene, object)?;
            let (track, slot) = (index(params, "track")?, index(params, "strip")?);
            let nla = &mut scene.object_mut(object).map_err(model)?.nla;
            let found = nla
                .tracks
                .get_mut(track)
                .filter(|found| slot < found.strips.len())
                .ok_or_else(|| format!("not-found: no strip {slot} on track {track}"))?;
            found.strips.remove(slot);
            nla.settle();
            Ok(json!({ "object": object }))
        }
        "nla.setAction" => {
            let nla = &mut scene.object_mut(object).map_err(model)?.nla;
            let blend: ActionBlend = merged(
                serde_json::to_value(nla.action).map_err(|error| error.to_string())?,
                params,
                &["blend", "influence", "extrapolation"],
                "the active action's blending",
            )?;
            if !(blend.influence.is_finite() && (0.0..=1.0).contains(&blend.influence)) {
                return Err(bad("`influence` is from 0 to 1"));
            }
            nla.action = blend;
            Ok(json!({ "object": object, "blend": blend.blend.name(), "influence": blend.influence, "extrapolation": blend.extrapolation.name() }))
        }
        "nla.tweak" => {
            let (track, slot) = (index(params, "track")?, index(params, "strip")?);
            let upper = params::boolean(params, "upperStack")?.unwrap_or(false);
            scene.nla_tweak(object, track, slot, upper).map_err(model)?;
            Ok(json!({ "object": object, "track": track, "strip": slot, "upperStack": upper }))
        }
        "nla.tweakExit" => {
            scene.nla_tweak_exit(object).map_err(model)?;
            Ok(json!({ "object": object }))
        }
        other => Err(format!("unknown-command: {other}")),
    }
}

fn modifiers(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    let target = params::integer(params, "target")?.ok_or_else(|| bad("`target` is required"))?;
    let property = params::string(params, "property")?.and_then(Property::parse).ok_or_else(|| bad("`property` is not a property"))?;
    let component = u8::try_from(params::integer(params, "index")?.unwrap_or(0)).map_err(|_| bad("`index` is out of range"))?;
    let path = params::string(params, "path")?.unwrap_or("").to_string();
    let channel = doc
        .scene
        .animation
        .channels
        .iter_mut()
        .find(|channel| channel.target == target && channel.property == property && channel.index == component && channel.path == path)
        .ok_or("not-found: no such channel")?;
    let stack = &mut channel.modifiers;
    let slot = match command {
        "graph.addModifier" => {
            let kind = params::string(params, "type")?
                .and_then(FModifierKind::new)
                .ok_or_else(|| bad("`type` is generator, function, envelope, cycles, noise, stepped, or limits"))?;
            if stack.len() >= kition_scene_model::animation::MAX_MODIFIERS {
                return Err(bad("a channel takes at most 64 modifiers"));
            }
            let base = serde_json::to_value(FModifier::new(kind)).map_err(|error| error.to_string())?;
            let modifier: FModifier = merged(base, params, MODIFIER_FIELDS, "the modifier")?;
            modifier.check().map_err(bad)?;
            stack.push(modifier);
            stack.len() - 1
        }
        "graph.setModifier" => {
            let slot = index(params, "modifier")?;
            let old = stack.get(slot).cloned().ok_or_else(|| format!("not-found: no modifier {slot}"))?;
            let base = serde_json::to_value(&old).map_err(|error| error.to_string())?;
            let modifier: FModifier = merged(base, params, MODIFIER_FIELDS, "the modifier")?;
            if modifier.kind.name() != old.kind.name() {
                return Err(bad("a modifier keeps its type"));
            }
            modifier.check().map_err(bad)?;
            if let Some(found) = stack.get_mut(slot) {
                *found = modifier;
            }
            match params::integer(params, "moveTo")? {
                Some(to) => {
                    let to = (to as usize).min(stack.len().saturating_sub(1));
                    let moved = stack.remove(slot);
                    stack.insert(to, moved);
                    to
                }
                None => slot,
            }
        }
        _ => {
            let slot = index(params, "modifier")?;
            if slot >= stack.len() {
                return Err(format!("not-found: no modifier {slot}"));
            }
            stack.remove(slot);
            slot
        }
    };
    Ok(json!({ "modifier": slot, "modifiers": stack.len() }))
}

/// How many steps the graph editor's preview of a modified curve takes.
const SAMPLES: usize = 240;

fn round(value: f32) -> f64 {
    (f64::from(value) * 10_000.0).round() / 10_000.0
}

fn strip_json(strip: &NlaStrip) -> Value {
    let mut value = serde_json::to_value(strip).unwrap_or(Value::Null);
    if let Some(map) = value.as_object_mut() {
        for (key, default) in [
            ("scale", json!(1.0)),
            ("repeat", json!(1.0)),
            ("reversed", json!(false)),
            ("blendIn", json!(0.0)),
            ("blendOut", json!(0.0)),
            ("autoBlend", json!(false)),
            ("extrapolation", json!(Extrapolation::Hold.name())),
            ("blend", json!(BlendType::Replace.name())),
            ("influence", Value::Null),
            ("muted", json!(false)),
        ] {
            map.entry(key).or_insert(default);
        }
        map.insert("kind".into(), serde_json::to_value(strip.kind).unwrap_or(Value::Null));
        map.insert("children".into(), json!(strip.children.iter().map(strip_json).collect::<Vec<_>>()));
        map.insert("end".into(), json!(strip.end()));
    }
    value
}

/// Each object's NLA (`objects[].nla`) and each channel's modifiers in `inspect()`.
pub fn extend_inspect(doc: &SceneDocument, inspected: &mut Value) {
    let scene = &doc.scene;
    if let Some(objects) = inspected.get_mut("objects").and_then(Value::as_array_mut) {
        for entry in objects {
            let Some(id) = entry.get("id").and_then(Value::as_u64).and_then(|id| u32::try_from(id).ok()) else { continue };
            let Ok(object) = scene.object(id) else { continue };
            if object.nla.is_empty() {
                continue;
            }
            let nla = &object.nla;
            let tracks: Vec<Value> = nla
                .tracks
                .iter()
                .map(|track| json!({ "name": track.name, "muted": track.muted, "solo": track.solo, "strips": track.strips.iter().map(strip_json).collect::<Vec<_>>() }))
                .collect();
            let tweak = nla.tweak.as_ref().map_or(Value::Null, |tweak| {
                let strip = nla.tweaked();
                json!({
                    "track": tweak.track,
                    "strip": tweak.strip,
                    "upperStack": tweak.upper_stack,
                    "start": strip.map(|strip| strip.start),
                    "actionStart": strip.map(|strip| strip.action_start),
                    "scale": strip.map(|strip| strip.scale),
                    "reversed": strip.map(|strip| strip.reversed),
                    "actionEnd": strip.map(|strip| strip.action_end),
                })
            });
            if let Some(fields) = entry.as_object_mut() {
                fields.insert(
                    "nla".into(),
                    json!({
                        "tracks": tracks,
                        "action": { "blend": nla.action.blend.name(), "influence": nla.action.influence, "extrapolation": nla.action.extrapolation.name() },
                        "tweak": tweak,
                    }),
                );
            }
        }
    }
    // Modifiers beside each channel's keys (the channels are listed in the same order).
    if let Some(channels) = inspected.pointer_mut("/animation/channels").and_then(Value::as_array_mut) {
        for (entry, channel) in channels.iter_mut().zip(&scene.animation.channels) {
            if channel.modifiers.is_empty() {
                continue;
            }
            // The modified curve, sampled over the timeline (rotations in degrees, as keys are).
            let timeline = scene.animation.timeline;
            let (start, end) = (timeline.start as f32, timeline.end.max(timeline.start + 1) as f32);
            let degrees = matches!(channel.property, Property::Rotation | Property::BoneRotation);
            let samples: Vec<Value> = (0..=SAMPLES)
                .filter_map(|step| {
                    let frame = start + (end - start) * step as f32 / SAMPLES as f32;
                    let value = channel.evaluate(frame)?;
                    let value = if degrees { value.to_degrees() } else { value };
                    value.is_finite().then(|| json!([round(frame), round(value)]))
                })
                .collect();
            if let Some(fields) = entry.as_object_mut() {
                fields.insert("modifiers".into(), serde_json::to_value(&channel.modifiers).unwrap_or(Value::Null));
                fields.insert("samples".into(), Value::Array(samples));
            }
        }
    }
}
