//! Promo staging for product shots: preset camera moves that chain into one take
//! (`camera.move`), shake and handheld float on top of them (`camera.shake`, `camera.handheld`),
//! a three-point studio light rig (`light.studio`), and object motion (`object.drop`,
//! `object.pop`, `object.hop`, `object.gather`, `object.wave`).
//!
//! Each command works the motion out and bakes it into keyframes as one undo step. Keys here
//! carry no handles of their own (curves use automatic clamped handles), so an ease such as
//! cubic-bezier(0.33, 0, 0.15, 1) is sampled: one key per frame, linear between. Rendered
//! frames show the eased curve exactly, in fast and path-traced renders alike; motion blur's
//! sub-frame samples run straight between frames. Re-running a command over the same frames
//! replaces its keys there. Camera aim is keyed rotation (no constraint), so the move saves,
//! exports and renders as plain keys.

use std::f32::consts::{PI, TAU};

use glam::{Mat3, Mat4, Quat, Vec3};
use kition_kit::KitCommand;
use kition_scene_model::scene::{euler_matrix, matrix_euler};
use kition_scene_model::{Channel, Id, Interpolation, Light, LightKind, ObjectData, Property, Scene};
use serde_json::{Value, json};

use crate::params::{self, bad, schema as s};
use crate::{Mode, SceneDocument};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Needs {
    ObjectMode,
    /// Object mode with objects to act on (selected, or named).
    Objects,
}

struct Spec {
    id: &'static str,
    title: &'static str,
    needs: Needs,
    schema: fn() -> Value,
}

/// The camera move presets.
const PRESETS: &[&str] = &["pushIn", "pushHover", "orbit", "craneRise", "reveal", "pullBack", "whip", "follow", "zoomThrough", "pullOut"];

fn target_schema(description: &str) -> Value {
    json!({ "oneOf": [
        { "type": "integer", "description": "an object's id (its bounding box's centre)" },
        { "type": "string", "description": "an object's name (its bounding box's centre)" },
        { "type": "array", "items": { "type": "number" }, "description": "a point [x, y, z] in metres" }
    ], "description": description })
}

static SPECS: &[Spec] = &[
    Spec {
        id: "camera.move",
        title: "Camera Move",
        needs: Needs::ObjectMode,
        schema: || {
            s::of(vec![
                ("preset", s::required(s::choice("The move", PRESETS))),
                (
                    "target",
                    target_schema(
                        "What the camera looks at and moves around (required, except for whip from where the camera is); follow: the object it follows",
                    ),
                ),
                (
                    "from",
                    s::choice(
                        "Where the move starts: current, where the camera is and how it looks at `start` (the default), so moves chain without a jump; position, round `position` aimed at the target from the first frame (the default when `position` is given)",
                        &["current", "position"],
                    ),
                ),
                ("camera", s::object("The camera; the scene's camera when left out")),
                ("start", s::number("The first frame of the move (the current frame)")),
                (
                    "duration",
                    s::number(
                        "Frames the move takes, 1 to 10000 (pushIn 45, pushHover 90, orbit 90, craneRise 110, reveal 60, pullBack 45, whip 10, follow 90)",
                    ),
                ),
                ("position", s::vec3("Where the camera is, instead of where it is at `start` (implies `from: \"position\"`)")),
                ("lens", s::number("Focal length in millimetres to reach by the end, 1 to 5000 (unchanged)")),
                (
                    "ease",
                    json!({ "type": "array", "items": { "type": "number" }, "description": "A cubic-bezier [x1, y1, x2, y2] instead of the preset's main ease (x1 and x2 from 0 to 1)" }),
                ),
                (
                    "amount",
                    s::number(
                        "pushIn: share of the distance closed (0.3); pushHover: share closed by the push (0.22); pullBack: how many times farther it ends (1.6)",
                    ),
                ),
                ("yaw", s::number("pushHover: degrees the camera swings round the target during the push (34)")),
                ("tilt", s::number("pushHover: degrees it rises during the push (8)")),
                ("creep", s::number("pushHover: share of the distance it drifts in while hovering, 0 to 0.015 (0.015)")),
                ("degrees", s::number("orbit: degrees round the target, counter-clockwise seen from above; negative turns clockwise (90)")),
                ("radius", s::number("orbit: horizontal distance from the target in metres (where the camera is)")),
                ("height", s::number("orbit: height above the target in metres (where the camera is)")),
                (
                    "easeEnds",
                    s::choice("orbit: speed up and slow down only at the ends (none: constant speed throughout)", &["none", "in", "out", "inOut"]),
                ),
                ("scale", s::number("craneRise and reveal: how many times larger the target looks at the start (3, 2.5)")),
                ("rise", s::number("craneRise: degrees the camera rises over the target (25)")),
                ("to", target_schema("whip: what the camera swings to")),
                ("motionBlur", s::boolean("whip: turn the scene's motion blur on with a 0.5-frame shutter (true); it shows in path-traced renders")),
                (
                    "through",
                    s::boolean("zoomThrough: arrive at full speed instead of slowing to rest, for a cut on the colour that fills the frame (false)"),
                ),
                (
                    "gap",
                    s::number(
                        "zoomThrough and pullOut: metres from the surface where the move ends or starts (1.5 times the camera's near clipping distance)",
                    ),
                ),
                (
                    "distance",
                    s::number(
                        "pullOut: metres from the target to the wide shot it ends on, back along the camera's line of sight (the camera's distance, or 8 times the target's half-size when it is closer than its size)",
                    ),
                ),
                ("lead", s::number("follow: seconds ahead of the object the camera aims, 0 to 10 (0.1)")),
                ("offset", s::vec3("follow: where the camera keeps to, from the object's centre, in metres (the camera's offset at `start`)")),
                ("smoothing", s::number("follow: seconds the camera lags behind (a critically damped spring), 0 to 10; 0 sticks rigidly (0.3)")),
            ])
        },
    },
    Spec {
        id: "camera.shake",
        title: "Camera Shake",
        needs: Needs::ObjectMode,
        schema: || {
            s::of(vec![
                ("camera", s::object("The camera; the scene's camera when left out")),
                ("frame", s::number("The frame the shake hits, at full strength (the current frame)")),
                ("amplitude", s::number("Metres the camera jolts sideways and up at first, 0 to 10 (0.03)")),
                ("rotation", s::number("Degrees it jolts in pitch and yaw at first (half that in roll), 0 to 45 (0.6)")),
                ("decay", s::number("How fast it dies away, per second: the strength falls by e every 1/decay seconds, 0.1 to 1000 (9)")),
                ("frequency", s::number("Jolts per second, 0.1 to 120 (14)")),
                ("seed", s::integer("Picks a different, repeatable shake (0)")),
            ])
        },
    },
    Spec {
        id: "camera.handheld",
        title: "Handheld Camera",
        needs: Needs::ObjectMode,
        schema: || {
            s::of(vec![
                ("camera", s::object("The camera; the scene's camera when left out")),
                ("start", s::number("The first frame of the float (the scene's first frame)")),
                ("end", s::number("The last frame of the float (the scene's last frame)")),
                ("amount", s::number("Metres the camera drifts at most along its own right and up, 0 to 10 (0.01)")),
                ("rotation", s::number("Degrees it drifts at most in pitch and yaw (half that in roll), 0 to 45 (0.25)")),
                (
                    "period",
                    json!({ "oneOf": [
                        { "type": "number", "description": "seconds of the slower sway; the other is 1.52 times it" },
                        { "type": "array", "items": { "type": "number" }, "description": "[a, b]: seconds of the two sways added together" }
                    ], "description": "How slowly it sways: two sine waves added together ([2.7, 4.1] seconds)" }),
                ),
                (
                    "holds",
                    json!({ "type": "array", "items": { "type": "array", "items": { "type": "number" } }, "description": "Frame ranges [[first, last], ...] where the camera is dead still; the float fades out over the 6 frames before each and back in over the 6 after" }),
                ),
                ("seed", s::integer("Picks a different, repeatable sway (0)")),
            ])
        },
    },
    Spec {
        id: "light.studio",
        title: "Studio Lights",
        needs: Needs::ObjectMode,
        schema: || {
            s::of(vec![
                ("target", s::required(target_schema("What the lights point at"))),
                ("preset", s::choice("The rig's mood (product)", &["product", "dramatic", "soft"])),
                ("distance", s::number("Metres from the target to the key and rim lights (five times half the target's largest side, at least 1)")),
            ])
        },
    },
    Spec {
        id: "object.drop",
        title: "Drop In",
        needs: Needs::Objects,
        schema: || {
            s::of(vec![
                ("objects", s::objects("The objects; the selection when left out")),
                ("start", s::number("The frame the fall starts (the current frame)")),
                ("duration", s::number("Frames the fall takes, 1 to 10000 (12)")),
                ("height", s::number("Metres above where they rest that they fall from (2)")),
                ("bounce", s::number("Height of the bounce as a share of the fall, 0 to 0.5 (0.12; 0 for none)")),
                ("squash", s::boolean("Stretch as they land, squash and settle (scale 1.06, 0.97, 1 over 4 frames) (true)")),
            ])
        },
    },
    Spec {
        id: "object.hop",
        title: "Hop",
        needs: Needs::Objects,
        schema: || {
            s::of(vec![
                ("objects", s::objects("The objects; the selection when left out. They hop together, keeping their places relative to one another")),
                ("to", s::required(target_schema("Where they land: a point their centre lands on, or an object they land on top of"))),
                ("start", s::number("The frame they take off (the current frame)")),
                ("duration", s::number("Frames in the air, 1 to 10000 (18)")),
                ("height", s::number("Metres the arc rises above the higher of its two ends, 0 to 10000 (0.6)")),
                ("spin", s::vec3("Degrees they turn about X, Y and Z in the air")),
                (
                    "impact",
                    json!({ "oneOf": [
                        { "type": "boolean", "description": "false: land without squashing" },
                        { "type": "object", "additionalProperties": false, "properties": {
                            "squash": { "type": "number", "description": "share of their height they squash by on landing, 0 to 0.9 (0.12)" },
                            "frames": { "type": "number", "description": "frames the squash rings out over, 1 to 1000 (8)" }
                        } }
                    ], "description": "The squash and stretch as they land ({squash: 0.12, frames: 8})" }),
                ),
            ])
        },
    },
    Spec {
        id: "object.gather",
        title: "Gather",
        needs: Needs::Objects,
        schema: || {
            s::of(vec![
                ("objects", s::objects("The objects, in the order they set off; the selection when left out")),
                ("center", s::required(target_schema("What they gather round: a point, or an object's bounding-box centre"))),
                (
                    "radius",
                    s::number(
                        "Metres from the centre to the ring they end on, 0 to 10000; 0 packs them into a cluster just touching (half their mean distance from the centre)",
                    ),
                ),
                ("level", s::number("Height in metres of the ring (each keeps its own height)")),
                ("start", s::number("The frame the first sets off (the current frame)")),
                ("duration", s::number("Frames each takes, 1 to 10000 (30)")),
                ("stagger", s::number("Frames between one setting off and the next (2; 4 with accelerate)")),
                ("accelerate", s::boolean("The gaps shrink from `stagger` to `minStagger`, so they come faster and faster (false)")),
                ("minStagger", s::number("accelerate: the last gap in frames (0.2)")),
                (
                    "arc",
                    s::number("Degrees each swirls round the centre on the way, counter-clockwise seen from above; negative turns clockwise (90)"),
                ),
                ("spin", s::vec3("Degrees each turns about X, Y and Z on the way")),
            ])
        },
    },
    Spec {
        id: "object.wave",
        title: "Wave",
        needs: Needs::Objects,
        schema: || {
            s::of(vec![
                ("objects", s::objects("The objects; the selection when left out")),
                ("origin", s::required(target_schema("Where the wave starts: a point, or an object's bounding-box centre"))),
                ("speed", s::number("Metres a second the wave travels, 0.001 to 100000 (4)")),
                (
                    "property",
                    s::choice(
                        "What the wave does: scale, a bump in size; color, switches each one's material base colour to `color` and leaves it there (scale)",
                        &["scale", "color"],
                    ),
                ),
                ("amount", s::number("scale: how much larger at the top of the bump, -0.9 to 100 (0.25)")),
                ("color", s::color("color: the colour each switches to (#rrggbb or linear [r, g, b])")),
                ("start", s::number("The frame the wave sets off from `origin` (the current frame)")),
                ("duration", s::number("Frames each object's bump or switch takes, 1 to 10000 (10)")),
            ])
        },
    },
    Spec {
        id: "object.pop",
        title: "Pop In",
        needs: Needs::Objects,
        schema: || {
            s::of(vec![
                ("objects", s::objects("The objects, in the order they pop; the selection when left out")),
                ("start", s::number("The frame the first one starts (the current frame)")),
                ("duration", s::number("Frames each takes to grow, 1 to 10000 (12)")),
                ("stagger", s::number("Frames between one starting and the next (3; 4 with accelerate)")),
                ("accelerate", s::boolean("The gaps shrink from `stagger` to `minStagger`, so they come faster and faster (false)")),
                ("minStagger", s::number("accelerate: the last gap in frames (0.2)")),
            ])
        },
    },
];

fn enabled(doc: &SceneDocument, needs: Needs) -> bool {
    let object_mode = doc.session.mode == Mode::Object && doc.session.rig.is_none();
    match needs {
        Needs::ObjectMode => object_mode,
        Needs::Objects => object_mode && !doc.session.editor.selected.is_empty(),
    }
}

/// The promo commands, with whether each can run now.
pub fn catalogue(doc: &SceneDocument) -> Vec<KitCommand> {
    SPECS
        .iter()
        .map(|spec| KitCommand {
            id: spec.id.into(),
            title: spec.title.into(),
            menu: None,
            shortcut: None,
            params: None,
            schema: Some((spec.schema)()),
            mutates: true,
            enabled: enabled(doc, spec.needs),
            ..Default::default()
        })
        .collect()
}

/// The promo commands that cannot run now.
pub fn disabled(doc: &SceneDocument) -> Vec<String> {
    SPECS.iter().filter(|spec| !enabled(doc, spec.needs)).map(|spec| spec.id.to_string()).collect()
}

/// Runs a promo command, or answers `None` when `command` is not one.
pub fn execute(doc: &mut SceneDocument, command: &str, params: &Value) -> Option<Result<Value, String>> {
    let spec = SPECS.iter().find(|spec| spec.id == command)?;
    if !params.is_object() && !params.is_null() {
        return Some(Err(bad("the parameters must be an object")));
    }
    let named = params.get("objects").is_some_and(|value| !value.is_null());
    let allowed = enabled(doc, spec.needs) || (spec.needs == Needs::Objects && named && enabled(doc, Needs::ObjectMode));
    if !allowed {
        return Some(Err(format!("not-allowed: {} cannot run now", spec.title)));
    }
    let before = doc.step(spec.title);
    let session = doc.session.clone();
    let result = match command {
        "camera.move" => camera_move(doc, params),
        "camera.shake" => shake(doc, params),
        "camera.handheld" => handheld(doc, params),
        "light.studio" => studio(doc, params),
        "object.drop" => drop_in(doc, params),
        "object.pop" => pop_in(doc, params),
        "object.hop" => hop(doc, params),
        "object.gather" => gather(doc, params),
        "object.wave" => wave(doc, params),
        other => Err(format!("unknown-command: {other}")),
    };
    Some(match result {
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

// Easing.

/// A CSS-style cubic Bézier ease from (0, 0) to (1, 1) through (x1, y1) and (x2, y2).
#[derive(Clone, Copy)]
struct Ease(f32, f32, f32, f32);

impl Ease {
    /// The eased value at `x` (0 to 1); y may leave 0..1 when the curve overshoots.
    fn at(self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        let cubic = |a: f32, b: f32, t: f32| {
            let u = 1.0 - t;
            3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
        };
        // x(t) rises monotonically when x1 and x2 are in 0..1.
        let (mut low, mut high) = (0.0_f32, 1.0_f32);
        for _ in 0..32 {
            let middle = 0.5 * (low + high);
            if cubic(self.0, self.2, middle) < x {
                low = middle;
            } else {
                high = middle;
            }
        }
        cubic(self.1, self.3, 0.5 * (low + high))
    }
}

/// A push: slow out of the start, a long soft landing.
const PUSH: Ease = Ease(0.33, 0.0, 0.15, 1.0);
/// A crane: its speed peaks about a fifth of the way through.
const CRANE: Ease = Ease(0.2, 0.0, 0.25, 1.0);
/// A whip pan: nearly all the turn happens in the middle.
const WHIP: Ease = Ease(0.75, 0.0, 0.25, 1.0);
/// A pop: past full size, then back.
const POP: Ease = Ease(0.2, 1.25, 0.3, 1.0);

/// Ease in and out along half a cosine.
fn sine(t: f32) -> f32 {
    0.5 - 0.5 * (PI * t.clamp(0.0, 1.0)).cos()
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Progress along a move at constant speed whose first `ramp_in` and last `ramp_out` shares
/// speed up from and slow down to rest (constant acceleration).
fn trapezoid(t: f32, ramp_in: f32, ramp_out: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let top = 1.0 / (1.0 - 0.5 * ramp_in - 0.5 * ramp_out);
    if ramp_in > 0.0 && t < ramp_in {
        top * t * t / (2.0 * ramp_in)
    } else if ramp_out > 0.0 && t > 1.0 - ramp_out {
        1.0 - top * (1.0 - t) * (1.0 - t) / (2.0 * ramp_out)
    } else {
        top * (t - 0.5 * ramp_in)
    }
}

// Parameters.

fn frames(params: &Value, key: &str, default: f32) -> Result<f32, String> {
    let value = params::number(params, key)?.unwrap_or(default);
    if !(1.0..=10_000.0).contains(&value) {
        return Err(bad(format!("`{key}` must be between 1 and 10000 frames")));
    }
    Ok(value)
}

fn start_frame(doc: &SceneDocument, params: &Value) -> Result<f32, String> {
    let start = params::number(params, "start")?.unwrap_or(doc.scene.animation.timeline.frame as f32);
    if start.abs() > (kition_scene_model::animation::MAX_FRAME - 20_000) as f32 {
        return Err(bad("`start` is out of range"));
    }
    Ok(start)
}

fn bounded(params: &Value, key: &str, default: f32, low: f32, high: f32) -> Result<f32, String> {
    let value = params::number(params, key)?.unwrap_or(default);
    if !(low..=high).contains(&value) {
        return Err(bad(format!("`{key}` must be between {low} and {high}")));
    }
    Ok(value)
}

/// The point `key` names (an object's bounding-box centre, or a point) and the half-size of what
/// it names (0.5 m for a point), in `scene`.
fn target_point(scene: &Scene, params: &Value, key: &str) -> Result<Option<(Vec3, f32)>, String> {
    let Some(value) = params.get(key).filter(|value| !value.is_null()) else { return Ok(None) };
    if value.is_array() || value.is_object() {
        return Ok(params::vec3(params, key)?.map(|point| (point, 0.5)));
    }
    let id = params::resolve(scene, value)?;
    let bounds = scene.bounds(Some(&[id]));
    if bounds.is_empty() {
        let world = scene.world_matrix(id).map_err(|error| error.to_string())?;
        return Ok(Some((world.transform_point3(Vec3::ZERO), 0.5)));
    }
    Ok(Some((bounds.center(), (bounds.size().max_element() / 2.0).max(1e-3))))
}

/// The objects named, or the selection.
fn chosen(doc: &SceneDocument, params: &Value) -> Result<Vec<Id>, String> {
    match params::objects(&doc.scene, params, "objects")? {
        Some(ids) if !ids.is_empty() => Ok(ids),
        Some(_) => Err(bad("`objects` names no object")),
        None if !doc.session.editor.selected.is_empty() => Ok(doc.session.editor.selected.clone()),
        None => Err("not-allowed: select an object first, or name `objects`".into()),
    }
}

// Keys.

/// The frames to key between `from` and `to`: both ends and every whole frame between.
fn sample_times(from: f32, to: f32) -> Vec<f32> {
    let mut times = vec![from];
    let mut frame = from.floor() + 1.0;
    while frame < to - 1e-3 {
        if frame > from + 1e-3 {
            times.push(frame);
        }
        frame += 1.0;
    }
    if to > from + 1e-3 {
        times.push(to);
    }
    times
}

/// Removes an object's keys of `property` from `from` to `to`.
fn clear_keys(scene: &mut Scene, target: Id, property: Property, from: f32, to: f32) {
    let (from, to) = (scene.key_time(target, from), scene.key_time(target, to));
    for channel in
        scene.animation.channels.iter_mut().filter(|channel| channel.target == target && channel.property == property && channel.path.is_empty())
    {
        channel.keys.retain(|key| key.frame < from - 1e-4 || key.frame > to + 1e-4);
    }
}

/// Keys component `index` of an object's `property` at `frame`, linear to the next key.
fn put_key(scene: &mut Scene, target: Id, property: Property, index: u8, frame: f32, value: f32) -> Result<(), String> {
    if scene.driver_of(target, property, index, "").is_some() {
        return Err(bad(format!("a driver sets this {}; it takes no keys", property.name())));
    }
    let frame = scene.key_time(target, frame);
    let channels = &mut scene.animation.channels;
    let found = channels
        .iter()
        .position(|channel| channel.target == target && channel.property == property && channel.index == index && channel.path.is_empty());
    let slot = match found {
        Some(slot) => slot,
        None => {
            channels.push(Channel { target, property, index, path: String::new(), keys: Vec::new(), driver: None, modifiers: Vec::new() });
            channels.len() - 1
        }
    };
    let channel = channels.get_mut(slot).ok_or_else(|| "failed: the channel went missing".to_string())?;
    channel.insert(frame, value, Interpolation::Linear);
    if let Some(key) = channel.keys.iter_mut().find(|key| (key.frame - frame).abs() <= 1e-4) {
        key.interpolation = Interpolation::Linear;
    }
    Ok(())
}

fn put_vector(scene: &mut Scene, target: Id, property: Property, frame: f32, value: Vec3) -> Result<(), String> {
    for (index, component) in (0u8..).zip(value.to_array()) {
        put_key(scene, target, property, index, frame, component)?;
    }
    Ok(())
}

/// Shows the keys just written at the current frame, and widens the frame range to `end`.
fn finish(scene: &mut Scene, end: f32) {
    let timeline = &mut scene.animation.timeline;
    let last = end.ceil() as i32;
    if last > timeline.end {
        timeline.end = last;
    }
    let frame = scene.animation.timeline.frame as f32;
    scene.go_to_frame(frame);
}

/// `angles` with each component moved by whole turns to lie within half a turn of `previous`.
fn unwrap(angles: Vec3, previous: Vec3) -> Vec3 {
    let near = |angle: f32, previous: f32| angle - TAU * ((angle - previous) / TAU).round();
    Vec3::new(near(angles.x, previous.x), near(angles.y, previous.y), near(angles.z, previous.z))
}

// camera.move

/// A place round a target: distance, angle about the up axis, and angle above the horizon.
#[derive(Clone, Copy)]
struct Around {
    radius: f32,
    azimuth: f32,
    elevation: f32,
}

impl Around {
    fn of(offset: Vec3) -> Self {
        let radius = offset.length();
        let elevation = if radius > 0.0 { (offset.z / radius).clamp(-1.0, 1.0).asin() } else { 0.0 };
        Self { radius, azimuth: offset.y.atan2(offset.x), elevation }
    }

    fn offset(self) -> Vec3 {
        let flat = self.radius * self.elevation.cos();
        Vec3::new(flat * self.azimuth.cos(), flat * self.azimuth.sin(), self.radius * self.elevation.sin())
    }

    fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            radius: lerp(self.radius, other.radius, t),
            azimuth: lerp(self.azimuth, other.azimuth, t),
            elevation: lerp(self.elevation, other.elevation, t),
        }
    }

    fn scaled(self, factor: f32) -> Self {
        Self { radius: self.radius * factor, ..self }
    }
}

const MAX_ELEVATION: f32 = 85.0 * PI / 180.0;

/// The camera's place and aim at a share `t` of the move, and the share of the lens change.
type Pose = Box<dyn Fn(f32) -> (Vec3, Vec3, f32)>;

/// One baked camera sample: frame, place, aim point, share of the lens change.
type Sample = (f32, Vec3, Vec3, f32);

/// The camera `camera` names, or the scene's.
fn the_camera(doc: &SceneDocument, params: &Value) -> Result<Id, String> {
    match params.get("camera").filter(|value| !value.is_null()) {
        Some(value) => params::resolve(&doc.scene, value),
        None => doc.scene.camera.ok_or_else(|| "not-allowed: the scene has no camera; add one or name `camera`".to_string()),
    }
}

/// An object's bounding-box centre in `scene` (its origin when it has no box).
fn object_center(scene: &Scene, id: Id) -> Result<Vec3, String> {
    let bounds = scene.bounds(Some(&[id]));
    if bounds.is_empty() {
        return scene.world_matrix(id).map(|world| world.transform_point3(Vec3::ZERO)).map_err(|error| error.to_string());
    }
    Ok(bounds.center())
}

/// A CSS-style ease given as `ease: [x1, y1, x2, y2]`, if any.
fn custom_ease(params: &Value) -> Result<Option<Ease>, String> {
    let Some(value) = params.get("ease").filter(|value| !value.is_null()) else { return Ok(None) };
    let numbers: Vec<f32> =
        value.as_array().map(|items| items.iter().filter_map(Value::as_f64).map(|number| number as f32).collect()).unwrap_or_default();
    match numbers.as_slice() {
        [x1, y1, x2, y2] if (0.0..=1.0).contains(x1) && (0.0..=1.0).contains(x2) && y1.abs() <= 4.0 && y2.abs() <= 4.0 => {
            Ok(Some(Ease(*x1, *y1, *x2, *y2)))
        }
        _ => Err(bad("`ease` is [x1, y1, x2, y2] with x1 and x2 from 0 to 1 and y1, y2 from -4 to 4")),
    }
}

/// Moves `place` and its velocity one step of `dt` seconds towards `goal` as a critically damped
/// spring of angular frequency `omega` (exact for a goal that holds still over the step).
fn damp(place: &mut Vec3, velocity: &mut Vec3, goal: Vec3, omega: f32, dt: f32) {
    let offset = *place - goal;
    let decay = (-omega * dt).exp();
    let push = (*velocity + offset * omega) * dt;
    *velocity = (*velocity - push * omega) * decay;
    *place = goal + (offset + push) * decay;
}

/// What the camera knows of itself at the first frame of a move.
struct Now {
    camera: Id,
    start: f32,
    duration: f32,
    /// Where it is, in the world.
    here: Vec3,
    /// The way it looks, in the world (a unit vector).
    facing: Vec3,
}

/// `follow`: keeps the camera at an offset from a moving object, aimed a little ahead of it, with
/// critically damped lag. Starts from where the camera is and how fast it moves at `start`.
fn follow(doc: &SceneDocument, params: &Value, now: &Now, times: &[f32]) -> Result<Vec<Sample>, String> {
    let value = params.get("target").filter(|value| !value.is_null()).ok_or_else(|| bad("follow needs `target`, an object"))?;
    if value.is_array() || value.is_object() {
        return Err(bad("follow's `target` is an object; a point does not move"));
    }
    let target = params::resolve(&doc.scene, value)?;
    let fps = doc.scene.animation.timeline.fps.max(1.0);
    let lead = bounded(params, "lead", 0.1, 0.0, 10.0)? * fps;
    let smoothing = bounded(params, "smoothing", 0.3, 0.0, 10.0)?;
    let mut probe = doc.scene.clone();
    let point_at = |probe: &mut Scene, frame: f32| -> Result<Vec3, String> {
        probe.go_to_frame(frame);
        object_center(probe, target)
    };
    let first = point_at(&mut probe, now.start)?;
    let offset = params::vec3(params, "offset")?.unwrap_or(now.here - first);
    if offset.length() < 1e-4 {
        return Err(bad("the camera is at the target; give `offset`"));
    }
    let first_aim = point_at(&mut probe, now.start + lead)?;
    // The camera carries on at the speed it had coming into `start`.
    probe.go_to_frame(now.start - 1.0);
    let before = probe.world_matrix(now.camera).map_err(|error| error.to_string())?.transform_point3(Vec3::ZERO);
    let mut place = now.here;
    let mut velocity = if smoothing > 0.0 { (now.here - before) * fps } else { Vec3::ZERO };
    let mut aim = now.here + now.facing * (first_aim - now.here).length().max(1e-3);
    let mut aim_velocity = Vec3::ZERO;
    let omega = if smoothing > 0.0 { 2.0 / smoothing } else { 0.0 };
    let mut samples = Vec::with_capacity(times.len());
    let mut last = now.start;
    for (index, frame) in times.iter().enumerate() {
        let goal = point_at(&mut probe, *frame)? + offset;
        let goal_aim = point_at(&mut probe, frame + lead)?;
        if index > 0 {
            if smoothing > 0.0 {
                let dt = (frame - last) / fps;
                damp(&mut place, &mut velocity, goal, omega, dt);
                damp(&mut aim, &mut aim_velocity, goal_aim, omega, dt);
            } else {
                (place, aim) = (goal, goal_aim);
            }
        }
        last = *frame;
        samples.push((*frame, place, aim, sine((frame - now.start) / now.duration)));
    }
    Ok(samples)
}

fn camera_move(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let preset = params::string(params, "preset")?.ok_or_else(|| bad("`preset` is required"))?.to_string();
    let default_duration = match preset.as_str() {
        "pushIn" | "pullBack" => 45.0,
        "pushHover" | "orbit" | "follow" => 90.0,
        "zoomThrough" | "pullOut" => 40.0,
        "craneRise" => 110.0,
        "reveal" => 60.0,
        "whip" => 10.0,
        other => return Err(bad(format!("`preset` is one of {}, not {other}", PRESETS.join(", ")))),
    };
    let camera = the_camera(doc, params)?;
    let start = start_frame(doc, params)?;
    let duration = frames(params, "duration", default_duration)?;
    let end = start + duration;
    let evaluated = doc.scene.at_frame(start);
    let object = evaluated.object(camera).map_err(|error| error.to_string())?;
    let ObjectData::Camera(lens_now) = &object.data else { return Err(bad(format!("{} is not a camera", object.name))) };
    let lens_from = lens_now.lens;
    let clip = lens_now.clip_start;
    let lens_to = match params::number(params, "lens")? {
        Some(lens) if (1.0..=5000.0).contains(&lens) => Some(lens),
        Some(_) => return Err(bad("`lens` must be between 1 and 5000 mm")),
        None => None,
    };
    let rotation_now = object.rotation;
    let world = evaluated.world_matrix(camera).map_err(|error| error.to_string())?;
    // The parent's frame at `start` (never the camera's own constraints).
    let parent_inverse = evaluated.parent_frame(camera).map_err(|error| error.to_string())?.inverse();
    if !parent_inverse.is_finite() {
        return Err(bad("the camera's parent has no inverse"));
    }
    let here = world.transform_point3(Vec3::ZERO);
    let facing = world.transform_vector3(Vec3::NEG_Z).normalize_or_zero();
    let given = params::vec3(params, "position")?;
    let current = match params::string(params, "from")? {
        None => given.is_none(),
        Some("current") if given.is_some() => {
            return Err(bad("`position` goes with `from: \"position\"`; `from: \"current\"` starts where the camera is"));
        }
        Some("current") => true,
        Some("position") => false,
        Some(other) => return Err(bad(format!("`from` is current or position, not {other}"))),
    };
    let position = given.unwrap_or(here);
    let custom = custom_ease(params)?;
    let times = sample_times(start, end);
    let now = Now { camera, start, duration, here, facing };
    let samples: Vec<Sample> = if preset == "follow" {
        follow(doc, params, &now, &times)?
    } else {
        let target = target_point(&evaluated, params, "target")?.map(|(point, _)| point);
        let input = PoseInput { evaluated: &evaluated, target, position, here, current, facing, duration, custom, clip };
        let pose = match preset.as_str() {
            "zoomThrough" | "pullOut" => zoom_pose(params, &preset, input)?,
            _ => preset_pose(doc, params, &preset, input)?,
        };
        let mut samples: Vec<Sample> = times
            .iter()
            .map(|frame| {
                let (place, aim, lens) = pose((frame - start) / duration);
                (*frame, place, aim, lens)
            })
            .collect();
        if current && preset != "whip" && facing != Vec3::ZERO {
            // The aim turns from where the camera looks at `start` to the move's own aim.
            let blend = (duration * 0.35).clamp(8.0, 30.0).min(duration);
            for (frame, place, aim, _) in &mut samples {
                let wanted = (*aim - *place).normalize_or_zero();
                if wanted == Vec3::ZERO {
                    continue;
                }
                let weight = sine((*frame - start) / blend);
                let turn = Quat::IDENTITY.slerp(Quat::from_rotation_arc(facing, wanted), weight);
                *aim = *place + turn * facing * (*aim - *place).length();
            }
        }
        samples
    };
    let scene = &mut doc.scene;
    clear_keys(scene, camera, Property::Location, start, end);
    clear_keys(scene, camera, Property::Rotation, start, end);
    if lens_to.is_some() {
        clear_keys(scene, camera, Property::Lens, start, end);
    }
    let mut previous = rotation_now;
    for (index, (frame, place, aim, lens_share)) in samples.iter().enumerate() {
        let look = crate::commands::look_rotation(*place, *aim).unwrap_or(previous);
        let world = Mat4::from_rotation_translation(Quat::from_mat3(&euler_matrix(look)), *place);
        let (_, rotation, location) = (parent_inverse * world).to_scale_rotation_translation();
        // The first key keeps the rotation the camera has (a roll, a shake) when it starts there.
        let kept = index == 0 && current && preset != "pullOut";
        let rotation = if kept { rotation_now } else { unwrap(matrix_euler(Mat3::from_quat(rotation)), previous) };
        let location = if kept { object.location } else { location };
        previous = rotation;
        if !(location.is_finite() && rotation.is_finite()) {
            return Err(bad("the move does not work out to finite places"));
        }
        put_vector(scene, camera, Property::Location, *frame, location)?;
        put_vector(scene, camera, Property::Rotation, *frame, rotation)?;
        if let Some(lens_to) = lens_to {
            put_key(scene, camera, Property::Lens, 0, *frame, lerp(lens_from, lens_to, *lens_share))?;
        }
    }
    finish(scene, end);
    Ok(
        json!({ "camera": camera, "preset": preset, "from": if current { "current" } else { "position" }, "start": start, "end": end, "keys": samples.len() }),
    )
}

/// What a preset's pose works from.
struct PoseInput<'a> {
    evaluated: &'a Scene,
    target: Option<Vec3>,
    /// Where the camera is (or `position`).
    position: Vec3,
    /// Where the camera is at `start`.
    here: Vec3,
    /// The move starts where the camera is and as it looks (`from: "current"`).
    current: bool,
    facing: Vec3,
    duration: f32,
    custom: Option<Ease>,
    /// The camera's near clipping distance.
    clip: f32,
}

fn preset_pose(doc: &mut SceneDocument, params: &Value, preset: &str, input: PoseInput<'_>) -> Result<Pose, String> {
    let PoseInput { evaluated, target, position, current, facing, duration, custom, .. } = input;
    if preset == "whip" {
        let (to, _) = target_point(evaluated, params, "to")?.ok_or_else(|| bad("whip needs `to`"))?;
        let from_direction = match (current, target) {
            (true, _) => facing,
            (false, Some(target)) => (target - position).normalize_or_zero(),
            (false, None) => return Err(bad("`target` is required")),
        };
        let to_direction = (to - position).normalize_or_zero();
        if from_direction == Vec3::ZERO || to_direction == Vec3::ZERO {
            return Err(bad("the camera is at `target` or `to`"));
        }
        let turn = Quat::from_rotation_arc(from_direction, to_direction);
        let distance = match (current, target) {
            (false, Some(target)) => lerp((target - position).length(), (to - position).length(), 0.5),
            _ => (to - position).length(),
        };
        let ease = custom.unwrap_or(WHIP);
        if params::boolean(params, "motionBlur")?.unwrap_or(true) {
            let blur = &mut doc.scene.render.motion_blur;
            blur.enabled = true;
            blur.shutter = 0.5;
        }
        return Ok(Box::new(move |t| {
            let e = ease.at(t);
            let direction = Quat::IDENTITY.slerp(turn, e) * from_direction;
            (position, position + direction * distance, e)
        }));
    }
    let target = target.ok_or_else(|| bad("`target` is required"))?;
    if (position - target).length() < 1e-4 {
        return Err(bad("the camera is at the target; move it or give `position`"));
    }
    let base = Around::of(position - target);
    let place = move |around: Around| target + around.offset();
    Ok(match preset {
        "pushIn" => {
            let amount = bounded(params, "amount", 0.3, 0.0, 0.95)?;
            let ease = custom.unwrap_or(PUSH);
            let to = base.scaled(1.0 - amount);
            Box::new(move |t| {
                let e = ease.at(t);
                (place(base.lerp(to, e)), target, e)
            })
        }
        "pullBack" => {
            let amount = bounded(params, "amount", 1.6, 1.0, 100.0)?;
            let ease = custom.unwrap_or(PUSH);
            let to = base.scaled(amount);
            Box::new(move |t| {
                let e = ease.at(t);
                (place(base.lerp(to, e)), target, e)
            })
        }
        "pushHover" => {
            let amount = bounded(params, "amount", 0.22, 0.0, 0.9)?;
            let yaw = bounded(params, "yaw", 34.0, -180.0, 180.0)?.to_radians();
            let tilt = bounded(params, "tilt", 8.0, -60.0, 60.0)?.to_radians();
            let creep = bounded(params, "creep", 0.015, 0.0, 0.015)?;
            let ease = custom.unwrap_or(PUSH);
            // Lock, push, hover, settle; the push and settle shrink when the move is short.
            let (lock, push, settle) = (4.0_f32, 18.0_f32, 18.0_f32);
            let fit = (duration / (lock + push + settle)).min(1.0);
            let (lock, push, settle) = (lock * fit, push * fit, settle * fit);
            let hover = (duration - lock - push - settle).max(0.0);
            let pushed = Around {
                radius: base.radius * (1.0 - amount),
                azimuth: base.azimuth + yaw,
                elevation: (base.elevation + tilt).clamp(-MAX_ELEVATION, MAX_ELEVATION),
            };
            let crept = pushed.scaled(1.0 - creep);
            // Settling gives back a little of the push.
            let settled = Around { radius: lerp(crept.radius, base.radius, 0.12), azimuth: lerp(pushed.azimuth, base.azimuth, 0.06), ..crept };
            Box::new(move |t| {
                let frame = t * duration;
                let (around, lens) = if frame <= lock {
                    (base, 0.0)
                } else if frame <= lock + push {
                    let e = ease.at((frame - lock) / push.max(1e-3));
                    (base.lerp(pushed, e), e)
                } else if frame <= lock + push + hover {
                    (pushed.lerp(crept, sine((frame - lock - push) / hover.max(1e-3))), 1.0)
                } else {
                    (crept.lerp(settled, sine((frame - lock - push - hover) / settle.max(1e-3))), 1.0)
                };
                (place(around), target, lens)
            })
        }
        "orbit" => {
            let degrees = bounded(params, "degrees", 90.0, -3600.0, 3600.0)?.to_radians();
            let offset = position - target;
            let flat = params::number(params, "radius")?.unwrap_or_else(|| offset.truncate().length());
            let height = params::number(params, "height")?.unwrap_or(offset.z);
            if flat.is_nan() || flat <= 1e-3 {
                return Err(bad("`radius` must be more than 0; the camera is straight above or below the target"));
            }
            let azimuth = offset.y.atan2(offset.x);
            let (ramp_in, ramp_out) = match params::string(params, "easeEnds")?.unwrap_or("none") {
                "none" => (0.0, 0.0),
                "in" => (0.25, 0.0),
                "out" => (0.0, 0.25),
                "inOut" => (0.25, 0.25),
                other => return Err(bad(format!("`easeEnds` is none, in, out, or inOut, not {other}"))),
            };
            Box::new(move |t| {
                let turn = match custom {
                    Some(ease) => ease.at(t),
                    None => trapezoid(t, ramp_in, ramp_out),
                };
                let angle = azimuth + degrees * turn;
                (target + Vec3::new(flat * angle.cos(), flat * angle.sin(), height), target, turn)
            })
        }
        "craneRise" => {
            let scale = bounded(params, "scale", 3.0, 1.0, 100.0)?;
            let rise = bounded(params, "rise", 25.0, 0.0, 170.0)?.to_radians();
            let ease = custom.unwrap_or(CRANE);
            // From where it is: the camera rises and pulls back from there. From `position`: that
            // place is the wide shot the crane ends on.
            let (low, high) = if current {
                (base, Around { radius: base.radius * scale, elevation: (base.elevation + rise).min(MAX_ELEVATION), ..base })
            } else {
                (Around { radius: base.radius / scale, elevation: (base.elevation - rise).max(-MAX_ELEVATION), ..base }, base)
            };
            let anticipated = low.scaled(1.0 - 0.015);
            let lead = (duration * 8.0 / 110.0).min(8.0);
            Box::new(move |t| {
                let frame = t * duration;
                if frame <= lead {
                    return (place(low.lerp(anticipated, sine(frame / lead.max(1e-3)))), target, 0.0);
                }
                let e = ease.at((frame - lead) / (duration - lead).max(1e-3));
                (place(anticipated.lerp(high, e)), target, e)
            })
        }
        "reveal" => {
            let scale = bounded(params, "scale", 2.5, 1.0, 100.0)?;
            let ease = custom.unwrap_or(PUSH);
            // From where it is: out to `scale` times the distance. From `position`: that place is
            // the wide shot the reveal ends on.
            let (close, wide) = if current { (base, base.scaled(scale)) } else { (base.scaled(1.0 / scale), base) };
            Box::new(move |t| {
                let e = ease.at(t);
                (place(close.lerp(wide, e)), target, e)
            })
        }
        other => return Err(bad(format!("`preset` is one of {}, not {other}", PRESETS.join(", ")))),
    })
}

/// How far along the ray from `origin` in unit `direction` it first meets object `id`'s surface.
fn surface_hit(scene: &Scene, id: Id, origin: Vec3, direction: Vec3) -> Option<f32> {
    let object = scene.object(id).ok()?;
    let mesh = object.evaluated_mesh(scene)?;
    let world = scene.world_matrix(id).ok()?;
    let points: Vec<Vec3> = mesh.positions.iter().map(|point| world.transform_point3(*point)).collect();
    let mut nearest: Option<f32> = None;
    for ([a, b, c], _) in mesh.triangles() {
        let (Some(a), Some(b), Some(c)) = (points.get(a as usize), points.get(b as usize), points.get(c as usize)) else { continue };
        // Möller and Trumbore's ray and triangle test.
        let (edge1, edge2) = (*b - *a, *c - *a);
        let cross = direction.cross(edge2);
        let det = edge1.dot(cross);
        if det.abs() < 1e-12 {
            continue;
        }
        let inverse = 1.0 / det;
        let along = origin - *a;
        let u = along.dot(cross) * inverse;
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let q = along.cross(edge1);
        let v = direction.dot(q) * inverse;
        if v < 0.0 || u + v > 1.0 {
            continue;
        }
        let distance = edge2.dot(q) * inverse;
        if distance > 1e-5 && nearest.is_none_or(|nearest| distance < nearest) {
            nearest = Some(distance);
        }
    }
    nearest
}

/// Ease into a constant speed (zoomThrough with `through`): no slowing at the end.
const ZOOM_IN: Ease = Ease(0.4, 0.0, 1.0, 1.0);
/// Ease in and out (zoomThrough stopping at the surface).
const ZOOM_STOP: Ease = Ease(0.4, 0.0, 0.6, 1.0);
/// Out at speed, slowing to rest (pullOut).
const ZOOM_OUT: Ease = Ease(0.0, 0.0, 0.58, 1.0);

/// `zoomThrough` and `pullOut`: straight along the line of sight to (or from) the target's
/// surface, the distance to it changing by the same factor every frame (log space), so the
/// approach looks steady instead of rushing at the end.
fn zoom_pose(params: &Value, preset: &str, input: PoseInput<'_>) -> Result<Pose, String> {
    let PoseInput { evaluated, target, position, here, current, facing, custom, clip, .. } = input;
    let target = target.ok_or_else(|| bad("`target` is required"))?;
    let named =
        params.get("target").filter(|value| !(value.is_array() || value.is_object())).map(|value| params::resolve(evaluated, value)).transpose()?;
    // Close enough to the surface that it fills the frame, but past the near clipping plane.
    let gap = bounded(params, "gap", (clip * 1.5).max(0.01), 0.001, 1000.0)?;
    let lens_share = |t: f32| sine((t - 0.6) / 0.4);
    let half = target_point(evaluated, params, "target")?.map_or(0.5, |(_, half)| half);
    if preset == "pullOut" {
        // The wide shot it ends on: `position`, or back along the camera's line of sight.
        let wide = match params::number(params, "distance")? {
            Some(distance) if (0.001..=1e6).contains(&distance) => {
                let direction = if current && facing != Vec3::ZERO { facing } else { (target - position).normalize_or_zero() };
                target - direction * distance
            }
            Some(_) => return Err(bad("`distance` must be between 0.001 and 1000000 metres")),
            None if !current => position,
            None if (here - target).length() > 2.0 * half && facing != Vec3::ZERO => target - facing * (here - target).length(),
            None => target - facing.normalize_or(Vec3::NEG_Y) * 8.0 * half,
        };
        let direction = (target - wide).normalize_or_zero();
        if direction == Vec3::ZERO {
            return Err(bad("the wide shot is at the target; give `distance` or `position`"));
        }
        let surface = named.and_then(|id| surface_hit(evaluated, id, wide, direction)).map_or(target, |distance| wide + direction * distance);
        let far = (surface - wide).length().max(gap * 1.001);
        let ease = custom.unwrap_or(ZOOM_OUT);
        return Ok(Box::new(move |t| {
            let distance = gap * (far / gap).powf(ease.at(t));
            (surface - direction * distance, target, lens_share(t))
        }));
    }
    let direction = (target - position).normalize_or_zero();
    if direction == Vec3::ZERO {
        return Err(bad("the camera is at the target; move it or give `position`"));
    }
    let through = params::boolean(params, "through")?.unwrap_or(false);
    let surface = named.and_then(|id| surface_hit(evaluated, id, position, direction)).map_or(target, |distance| position + direction * distance);
    // The distance to the surface runs down to `gap`. Not past it: rasterized renders cull the
    // inside of a closed surface, so a camera inside one sees straight through it.
    let goal = surface;
    let near = (goal - position).length();
    if near <= gap {
        return Err(bad("the camera is already at the surface"));
    }
    let ease = custom.unwrap_or(if through { ZOOM_IN } else { ZOOM_STOP });
    Ok(Box::new(move |t| {
        let distance = near * (gap / near).powf(ease.at(t));
        (goal - direction * distance, goal + direction, lens_share(t))
    }))
}

// camera.shake and camera.handheld

/// A repeatable pseudo-random number from -1 to 1 for (`seed`, `channel`, `step`).
fn hashed(seed: u32, channel: u32, step: i64) -> f32 {
    let mut x = u64::from(seed).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ u64::from(channel).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (step as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    x = x.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    x ^= x >> 33;
    (x >> 40) as f32 / (1u64 << 23) as f32 - 1.0
}

/// Smooth value noise from -1 to 1 along `x`: random at whole numbers, eased between.
fn noise(seed: u32, channel: u32, x: f32) -> f32 {
    let step = x.floor();
    let t = x - step;
    let step = step as i64;
    lerp(hashed(seed, channel, step), hashed(seed, channel, step + 1), t * t * (3.0 - 2.0 * t))
}

/// An offset to a camera: metres along its own right and up, and radians of pitch, yaw and
/// roll in its own frame.
#[derive(Clone, Copy, Default)]
struct Nudge {
    right: f32,
    up: f32,
    pitch: f32,
    yaw: f32,
    roll: f32,
}

impl Nudge {
    fn is_zero(self) -> bool {
        [self.right, self.up, self.pitch, self.yaw, self.roll].iter().all(|value| *value == 0.0)
    }
}

/// Adds `nudge(frame)` on top of what the camera does from `from` to `to`, as keys on every whole
/// frame. The frames out to the camera's nearest keys on either side are keyed with what they
/// show now, so the curves beyond the range stay as they were. Answers how many frames it keyed.
fn overlay(scene: &mut Scene, camera: Id, from: f32, to: f32, nudge: &dyn Fn(f32) -> Nudge) -> Result<usize, String> {
    let keyed = |property: Property| {
        scene
            .animation
            .channels
            .iter()
            .filter(move |channel| channel.target == camera && channel.property == property && channel.path.is_empty())
            .flat_map(|channel| channel.keys.iter().map(|key| key.frame))
    };
    let frames: Vec<f32> = keyed(Property::Location).chain(keyed(Property::Rotation)).collect();
    let reach = 10_000.0;
    let low = frames.iter().copied().filter(|frame| *frame <= from).reduce(f32::max).unwrap_or(from).max(from - reach);
    let high = frames.iter().copied().filter(|frame| *frame >= to).reduce(f32::min).unwrap_or(to).min(to + reach);
    let mut probe = scene.clone();
    let mut planned = Vec::new();
    for frame in sample_times(low, high) {
        probe.go_to_frame(frame);
        let object = probe.object(camera).map_err(|error| error.to_string())?;
        let (location, rotation) = (object.location, object.rotation);
        let nudge = nudge(frame);
        if nudge.is_zero() {
            planned.push((frame, location, rotation));
            continue;
        }
        let turn = euler_matrix(rotation);
        let location = location + turn * Vec3::new(nudge.right, nudge.up, 0.0);
        let turned = turn * Mat3::from_euler(glam::EulerRot::XYZ, nudge.pitch, nudge.yaw, nudge.roll);
        let rotation = unwrap(matrix_euler(turned), rotation);
        if !(location.is_finite() && rotation.is_finite()) {
            return Err(bad("the offsets do not work out to finite places"));
        }
        planned.push((frame, location, rotation));
    }
    clear_keys(scene, camera, Property::Location, low, high);
    clear_keys(scene, camera, Property::Rotation, low, high);
    for (frame, location, rotation) in &planned {
        put_vector(scene, camera, Property::Location, *frame, *location)?;
        put_vector(scene, camera, Property::Rotation, *frame, *rotation)?;
    }
    Ok(planned.len())
}

fn camera_of(doc: &SceneDocument, params: &Value) -> Result<Id, String> {
    let camera = the_camera(doc, params)?;
    match doc.scene.object(camera).map_err(|error| error.to_string())?.data {
        ObjectData::Camera(_) => Ok(camera),
        _ => Err(bad("`camera` is not a camera")),
    }
}

fn shake(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let camera = camera_of(doc, params)?;
    let frame = match params::number(params, "frame")? {
        Some(frame) if frame.abs() <= (kition_scene_model::animation::MAX_FRAME - 20_000) as f32 => frame,
        Some(_) => return Err(bad("`frame` is out of range")),
        None => doc.scene.animation.timeline.frame as f32,
    };
    let amplitude = bounded(params, "amplitude", 0.03, 0.0, 10.0)?;
    let rotation = bounded(params, "rotation", 0.6, 0.0, 45.0)?.to_radians();
    let decay = bounded(params, "decay", 9.0, 0.1, 1000.0)?;
    let frequency = bounded(params, "frequency", 14.0, 0.1, 120.0)?;
    let seed = params::integer(params, "seed")?.unwrap_or(0);
    let fps = doc.scene.animation.timeline.fps.max(1.0);
    // It lasts until it is down to 1/500 of its strength, and falls to exactly nothing there.
    let lasts = (500.0_f32.ln() / decay).min(10_000.0 / fps);
    let end = frame + (lasts * fps).ceil();
    let floor = (-decay * lasts).exp();
    let nudge = move |at: f32| {
        let t = (at - frame) / fps;
        if t < 0.0 || t >= lasts {
            return Nudge::default();
        }
        let strength = (((-decay * t).exp() - floor) / (1.0 - floor)).max(0.0);
        let x = t * frequency;
        let wobble = |channel: u32| noise(seed, channel, x) * strength;
        Nudge {
            right: amplitude * wobble(0),
            up: amplitude * wobble(1),
            pitch: rotation * wobble(2),
            yaw: rotation * wobble(3),
            roll: 0.5 * rotation * wobble(4),
        }
    };
    let keys = overlay(&mut doc.scene, camera, frame, end, &nudge)?;
    finish(&mut doc.scene, end);
    Ok(json!({ "camera": camera, "frame": frame, "end": end, "keys": keys }))
}

/// Frames the handheld float takes to fade at a hold's edges and at its own ends.
const HOLD_FADE: f32 = 6.0;

fn handheld(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let camera = camera_of(doc, params)?;
    let timeline = &doc.scene.animation.timeline;
    let (first, last, fps) = (timeline.start as f32, timeline.end as f32, timeline.fps.max(1.0));
    let limit = (kition_scene_model::animation::MAX_FRAME - 20_000) as f32;
    let start = params::number(params, "start")?.unwrap_or(first);
    let end = params::number(params, "end")?.unwrap_or(last);
    if start.abs() > limit || end.abs() > limit {
        return Err(bad("`start` or `end` is out of range"));
    }
    if end <= start || end - start > 100_000.0 {
        return Err(bad("`end` must come after `start`, at most 100000 frames later"));
    }
    let amount = bounded(params, "amount", 0.01, 0.0, 10.0)?;
    let rotation = bounded(params, "rotation", 0.25, 0.0, 45.0)?.to_radians();
    let seed = params::integer(params, "seed")?.unwrap_or(0);
    let periods = match params.get("period").filter(|value| !value.is_null()) {
        None => (2.7, 4.1),
        Some(Value::Number(number)) => {
            let slow = number.as_f64().unwrap_or(0.0) as f32;
            (slow, slow * 1.52)
        }
        Some(Value::Array(items)) => match items.iter().map(Value::as_f64).collect::<Vec<_>>().as_slice() {
            [Some(a), Some(b)] => (*a as f32, *b as f32),
            _ => return Err(bad("`period` is a number of seconds or [a, b]")),
        },
        Some(_) => return Err(bad("`period` is a number of seconds or [a, b]")),
    };
    if !((0.05..=1000.0).contains(&periods.0) && (0.05..=1000.0).contains(&periods.1)) {
        return Err(bad("`period` must be between 0.05 and 1000 seconds"));
    }
    let mut holds = Vec::new();
    if let Some(value) = params.get("holds").filter(|value| !value.is_null()) {
        let items = value.as_array().ok_or_else(|| bad("`holds` is a list of [first, last] frame ranges"))?;
        if items.len() > 10_000 {
            return Err(bad("at most 10000 holds"));
        }
        for item in items {
            match item.as_array().map(|pair| pair.iter().map(Value::as_f64).collect::<Vec<_>>()).as_deref() {
                Some([Some(a), Some(b)]) if a <= b && a.is_finite() && b.is_finite() => holds.push((*a as f32, *b as f32)),
                _ => return Err(bad("each hold is [first, last] with first no later than last")),
            }
        }
    }
    // A fade from 0 to 1 over HOLD_FADE frames, eased.
    let fade = |frames: f32| sine((frames / HOLD_FADE).clamp(0.0, 1.0));
    let weight = move |frame: f32| {
        if frame < start || frame > end {
            return 0.0;
        }
        let mut weight = fade(frame - start) * fade(end - frame);
        for (from, to) in &holds {
            let outside = if frame < *from {
                from - frame
            } else if frame > *to {
                frame - to
            } else {
                0.0
            };
            weight *= fade(outside);
        }
        weight
    };
    // Each channel sways with its own phases.
    let phase = |channel: u32, wave: u32| PI * hashed(seed, channel * 2 + wave, 7);
    let phases: Vec<(f32, f32)> = (0..5).map(|channel| (phase(channel, 0), phase(channel, 1))).collect();
    let nudge = move |frame: f32| {
        let weight = weight(frame);
        if weight == 0.0 {
            return Nudge::default();
        }
        let t = (frame - start) / fps;
        let sway = |channel: usize| {
            let (a, b) = phases.get(channel).copied().unwrap_or_default();
            weight * (0.62 * (TAU * t / periods.0 + a).sin() + 0.38 * (TAU * t / periods.1 + b).sin())
        };
        Nudge { right: amount * sway(0), up: amount * sway(1), pitch: rotation * sway(2), yaw: rotation * sway(3), roll: 0.5 * rotation * sway(4) }
    };
    let keys = overlay(&mut doc.scene, camera, start, end, &nudge)?;
    finish(&mut doc.scene, end);
    Ok(json!({ "camera": camera, "start": start, "end": end, "keys": keys }))
}

// light.studio

struct Rig {
    /// Key: degrees from the camera's direction (positive towards camera-right), elevation,
    /// cone, soft-shadow radius as a share of the distance.
    key: (f32, f32, f32, f32),
    fill_ratio: f32,
    rim_ratio: f32,
    world: f32,
    look: kition_scene_model::color::Look,
}

fn studio(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    use kition_scene_model::color::{Look, ViewTransform};
    let preset = params::string(params, "preset")?.unwrap_or("product");
    let rig = match preset {
        "product" => Rig { key: (-45.0, 45.0, 70.0, 0.08), fill_ratio: 0.2, rim_ratio: 1.2, world: 0.25, look: Look::Punchy },
        "dramatic" => Rig { key: (-65.0, 50.0, 50.0, 0.03), fill_ratio: 0.07, rim_ratio: 1.6, world: 0.08, look: Look::Punchy },
        "soft" => Rig { key: (-40.0, 40.0, 100.0, 0.4), fill_ratio: 0.5, rim_ratio: 0.6, world: 0.4, look: Look::None },
        other => return Err(bad(format!("`preset` is product, dramatic, or soft, not {other}"))),
    };
    let (target, half) = target_point(&doc.scene, params, "target")?.ok_or_else(|| bad("`target` is required"))?;
    let distance = match params::number(params, "distance")? {
        Some(distance) if (0.01..=10_000.0).contains(&distance) => distance,
        Some(_) => return Err(bad("`distance` must be between 0.01 and 10000 metres")),
        None => (half * 5.0).max(1.0),
    };
    // Directions are measured from the camera's side of the target.
    let toward_camera = doc
        .scene
        .camera
        .and_then(|camera| doc.scene.world_matrix(camera).ok())
        .map(|world| (world.transform_point3(Vec3::ZERO) - target).truncate().extend(0.0).normalize_or_zero())
        .filter(|direction| *direction != Vec3::ZERO)
        .unwrap_or(Vec3::NEG_Y);
    let at = |turn: f32, elevation: f32, reach: f32| {
        let (turn, elevation) = (turn.to_radians(), elevation.to_radians());
        let flat = toward_camera * turn.cos() + Vec3::Z.cross(toward_camera) * turn.sin();
        target + (flat * elevation.cos() + Vec3::Z * elevation.sin()) * reach
    };
    let key_power = 100.0 * distance * distance;
    let (turn, elevation, cone, softness) = rig.key;
    let lights = [
        ("Key Light", LightKind::Spot, at(turn, elevation, distance), Vec3::new(1.0, 0.96, 0.9), key_power, distance * softness, cone),
        ("Fill Light", LightKind::Area, at(60.0, 25.0, distance * 1.1), Vec3::new(0.72, 0.84, 1.0), key_power * rig.fill_ratio, distance * 0.8, 45.0),
        ("Rim Light", LightKind::Spot, at(155.0, 50.0, distance), Vec3::new(1.0, 0.76, 0.5), key_power * rig.rim_ratio, distance * 0.05, 45.0),
    ];
    let mut made = Vec::new();
    for (name, kind, place, color, power, radius, cone) in lights {
        let existing = doc
            .scene
            .object_named(name)
            .filter(|object| matches!(object.data, ObjectData::Light(_)) && object.parent.is_none())
            .map(|object| object.id);
        let id = match existing {
            Some(id) => id,
            None => doc.scene.add_object(name, ObjectData::Light(Light::new(kind))),
        };
        let rotation = crate::commands::look_rotation(place, target).unwrap_or(Vec3::ZERO);
        let object = doc.scene.object_mut(id).map_err(|error| error.to_string())?;
        object.location = place;
        object.rotation = rotation;
        object.scale = Vec3::ONE;
        object.hidden = false;
        object.data = ObjectData::Light(Light { kind, color, power, radius, spot_size: cone.to_radians() });
        made.push(json!({ "name": object.name, "object": id }));
    }
    doc.scene.world.strength = rig.world;
    let occlusion = &mut doc.scene.render.ambient_occlusion;
    occlusion.enabled = true;
    occlusion.distance = Some((half * 0.5).clamp(0.05, 2.0));
    let display = &mut doc.scene.render.color_management;
    display.view = ViewTransform::Agx;
    display.look = rig.look;

    Ok(json!({ "lights": made, "distance": distance, "target": target.to_array() }))
}

// object.drop and object.pop

fn drop_in(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let ids = chosen(doc, params)?;
    let start = start_frame(doc, params)?;
    let fall = frames(params, "duration", 12.0)?;
    let height = bounded(params, "height", 2.0, 0.0, 10_000.0)?;
    let bounce = bounded(params, "bounce", 0.12, 0.0, 0.5)?;
    let squash = params::boolean(params, "squash")?.unwrap_or(true);
    // A bounce up to a share b of the fall takes 2·√b of the fall's time, as under gravity.
    let hop = fall * 2.0 * bounce.sqrt();
    let landed = start + fall;
    let end = landed + hop.max(if squash { 4.0 } else { 0.0 });
    let height_at = |frame: f32| {
        if frame <= landed {
            let t = ((frame - start) / fall).clamp(0.0, 1.0);
            height * (1.0 - t * t)
        } else if hop > 0.0 && frame < landed + hop {
            let u = (frame - landed) / hop;
            height * bounce * (1.0 - (2.0 * u - 1.0).powi(2))
        } else {
            0.0
        }
    };
    // Stretched along Z as it lands, squashed the frame after, settled by the fourth.
    let stretch_at = |frame: f32| {
        if !squash {
            return 1.0;
        }
        let before = landed - frame;
        if (0.0..3.0).contains(&before) {
            lerp(1.06, 1.0, before / 3.0)
        } else if frame > landed && frame <= landed + 1.0 {
            lerp(1.06, 0.97, frame - landed)
        } else if frame > landed + 1.0 && frame < landed + 4.0 {
            lerp(0.97, 1.0, sine((frame - landed - 1.0) / 3.0))
        } else {
            1.0
        }
    };
    let evaluated = doc.scene.at_frame(start);
    let times = sample_times(start, end);
    for id in &ids {
        let object = evaluated.object(*id).map_err(|error| error.to_string())?;
        let (location, scale) = (object.location, object.scale);
        clear_keys(&mut doc.scene, *id, Property::Location, start, end);
        if squash {
            clear_keys(&mut doc.scene, *id, Property::Scale, start, end);
        }
        for frame in &times {
            put_vector(&mut doc.scene, *id, Property::Location, *frame, location + Vec3::Z * height_at(*frame))?;
            if squash {
                let z = stretch_at(*frame);
                put_vector(&mut doc.scene, *id, Property::Scale, *frame, scale * Vec3::new(1.0 / z.sqrt(), 1.0 / z.sqrt(), z))?;
            }
        }
    }
    finish(&mut doc.scene, end);
    Ok(json!({ "objects": ids, "start": start, "landed": landed, "end": end }))
}

// object.hop

/// The squash and stretch of a landing: the Z scale factor `after` frames after it (negative:
/// before), ringing out over `frames`.
fn landing_stretch(squash: f32, frames: f32, after: f32) -> f32 {
    if after < 0.0 {
        // Stretched by its speed over the last 3 frames of the fall.
        return 1.0 + 0.5 * squash * sine((after + 3.0) / 3.0);
    }
    if after >= frames {
        return 1.0;
    }
    let u = after / frames;
    // A damped spring back from the squash, overshooting once, at rest at `frames`.
    1.0 - squash * (TAU * 1.25 * u).cos() * (-3.0 * u).exp() * (1.0 - u)
}

fn hop(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let ids = chosen(doc, params)?;
    let start = start_frame(doc, params)?;
    let duration = frames(params, "duration", 18.0)?;
    let height = bounded(params, "height", 0.6, 0.0, 10_000.0)?;
    let spin = params::vec3(params, "spin")?.map(|degrees| degrees * (PI / 180.0));
    let (squash, ring) = match params.get("impact").filter(|value| !value.is_null()) {
        None | Some(Value::Bool(true)) => (0.12, 8.0),
        Some(Value::Bool(false)) => (0.0, 0.0),
        Some(impact @ Value::Object(_)) => (bounded(impact, "squash", 0.12, 0.0, 0.9)?, bounded(impact, "frames", 8.0, 1.0, 1000.0)?),
        Some(_) => return Err(bad("`impact` is {squash, frames} or false")),
    };
    let landed = start + duration;
    let end = landed + if squash > 0.0 { ring } else { 0.0 };
    let evaluated = doc.scene.at_frame(start);
    let group = evaluated.bounds(Some(&ids));
    if group.is_empty() {
        return Err(bad("the objects have no size to hop with"));
    }
    // Where the group's centre (or, onto an object, the middle of its bottom) lands.
    let shift = match params.get("to").filter(|value| !value.is_null()).ok_or_else(|| bad("`to` is required"))? {
        value if value.is_array() || value.is_object() => params::vec3(params, "to")?.unwrap_or(Vec3::ZERO) - group.center(),
        value => {
            let onto = params::resolve(&doc.scene, value)?;
            if ids.contains(&onto) {
                return Err(bad("an object cannot hop onto itself"));
            }
            let there = doc.scene.at_frame(landed).bounds(Some(&[onto]));
            if there.is_empty() {
                return Err(bad("`to` has no surface to land on"));
            }
            let top = Vec3::new(there.center().x, there.center().y, there.max.z);
            top - Vec3::new(group.center().x, group.center().y, group.min.z)
        }
    };
    for id in &ids {
        let object = evaluated.object(*id).map_err(|error| error.to_string())?;
        let (location, rotation, scale) = (object.location, object.rotation, object.scale);
        // The arc is planned in the world (the shift is a world one) and keyed in the parent's
        // frame at `start`.
        let frame_of = evaluated.parent_frame(*id).map_err(|error| error.to_string())?;
        let into = frame_of.inverse();
        if !into.is_finite() {
            return Err(bad("the object's parent has no inverse"));
        }
        let from = frame_of.transform_point3(location);
        // How far its origin sits above its bottom, kept planted as it squashes.
        let own = evaluated.bounds(Some(&[*id]));
        let above = if own.is_empty() {
            0.0
        } else {
            (evaluated.world_matrix(*id).map_err(|error| error.to_string())?.transform_point3(Vec3::ZERO).z - own.min.z).max(0.0)
        };
        let to = from + shift;
        // A parabola in time through both ends whose top is `height` above the higher end.
        let (rise, apex) = (to.z - from.z, from.z.max(to.z) + height - from.z);
        let gravity = if apex > 1e-6 { 2.0 * (2.0 * apex - rise + (4.0 * apex * (apex - rise)).max(0.0).sqrt()) } else { 0.0 };
        let launch = rise + 0.5 * gravity;
        clear_keys(&mut doc.scene, *id, Property::Location, start, end);
        if spin.is_some() {
            clear_keys(&mut doc.scene, *id, Property::Rotation, start, landed);
        }
        if squash > 0.0 {
            clear_keys(&mut doc.scene, *id, Property::Scale, (landed - 3.0).max(start), end);
        }
        for frame in sample_times(start, end) {
            let t = ((frame - start) / duration).clamp(0.0, 1.0);
            let mut place = from.lerp(to, t);
            place.z = from.z + launch * t - 0.5 * gravity * t * t;
            if frame >= landed {
                place.z = to.z;
            }
            let stretch = if squash > 0.0 { landing_stretch(squash, ring, frame - landed) } else { 1.0 };
            if frame >= landed {
                place.z += above * (stretch - 1.0);
            }
            put_vector(&mut doc.scene, *id, Property::Location, frame, into.transform_point3(place))?;
            if let Some(spin) = spin
                && frame <= landed
            {
                put_vector(&mut doc.scene, *id, Property::Rotation, frame, rotation + spin * t)?;
            }
            if squash > 0.0 && frame >= landed - 3.0 {
                let across = 1.0 / stretch.max(0.05).sqrt();
                put_vector(&mut doc.scene, *id, Property::Scale, frame, scale * Vec3::new(across, across, stretch))?;
            }
        }
    }
    finish(&mut doc.scene, end);
    Ok(json!({ "objects": ids, "start": start, "landed": landed, "end": end }))
}

/// The frames `count` objects start at from `start`: `stagger` frames apart, or with
/// `accelerate` gaps shrinking geometrically from `stagger` to `minStagger`.
fn staggered(params: &Value, start: f32, count: usize, default: f32) -> Result<Vec<f32>, String> {
    let accelerate = params::boolean(params, "accelerate")?.unwrap_or(false);
    let stagger = bounded(params, "stagger", if accelerate { 4.0 } else { default }, 0.0, 1000.0)?;
    let least = bounded(params, "minStagger", 0.2, 0.0, 1000.0)?;
    if count > 10_000 {
        return Err(bad("at most 10000 objects at once"));
    }
    let gaps = count.saturating_sub(1);
    let mut starts = Vec::with_capacity(count);
    let mut at = start;
    for slot in 0..count {
        starts.push(at);
        let gap = if accelerate && gaps > 1 {
            // Geometric: each gap the same share of the one before.
            let share = slot as f32 / (gaps - 1) as f32;
            if stagger > 0.0 && least > 0.0 { stagger * (least / stagger).powf(share) } else { lerp(stagger, least, share) }
        } else {
            stagger
        };
        at += gap;
    }
    Ok(starts)
}

// object.gather

/// Each object's way into the ring: cubic-bezier(0.3, 0, 0.25, 1).
const GATHER: Ease = Ease(0.3, 0.0, 0.25, 1.0);

/// `angle` moved by whole turns into -π..π.
fn wrapped(angle: f32) -> f32 {
    angle - TAU * (angle / TAU).round()
}

fn gather(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let ids = chosen(doc, params)?;
    let start = start_frame(doc, params)?;
    let duration = frames(params, "duration", 30.0)?;
    let starts = staggered(params, start, ids.len(), 2.0)?;
    let arc = bounded(params, "arc", 90.0, -3600.0, 3600.0)?.to_radians();
    let spin = params::vec3(params, "spin")?.map(|degrees| degrees * (PI / 180.0));
    let level = params::number(params, "level")?;
    if level.is_some_and(|level| !level.is_finite() || level.abs() > 1e6) {
        return Err(bad("`level` must be a finite height"));
    }
    let evaluated = doc.scene.at_frame(start);
    let (center, _) = target_point(&evaluated, params, "center")?.ok_or_else(|| bad("`center` is required"))?;
    // Each object's world place (its fields through its parent's frame at `start`): the ring is
    // planned in the world and keyed back in the parent's frame.
    let mut places = Vec::with_capacity(ids.len());
    let mut frames_of = Vec::with_capacity(ids.len());
    for id in &ids {
        let object = evaluated.object(*id).map_err(|error| error.to_string())?;
        let frame_of = evaluated.parent_frame(*id).map_err(|error| error.to_string())?;
        let into = frame_of.inverse();
        if !into.is_finite() {
            return Err(bad(format!("{}'s parent has no inverse", object.name)));
        }
        places.push((frame_of.transform_point3(object.location), object.rotation));
        frames_of.push(into);
    }
    let count = ids.len();
    let away: Vec<Vec3> = places.iter().map(|(location, _)| (*location - center) * Vec3::new(1.0, 1.0, 0.0)).collect();
    let radius = match params::number(params, "radius")? {
        Some(radius) if !(0.0..=10_000.0).contains(&radius) => return Err(bad("`radius` must be between 0 and 10000 metres")),
        Some(radius) if radius > 0.0 => radius,
        Some(_) => {
            // Just touching: neighbours a largest size apart round the ring.
            let size = ids
                .iter()
                .map(|id| evaluated.bounds(Some(&[*id])))
                .filter(|bounds| !bounds.is_empty())
                .map(|bounds| bounds.size().max_element())
                .fold(0.0_f32, f32::max);
            if count > 1 { 0.5 * size / (PI / count as f32).sin() } else { 0.0 }
        }
        None => 0.5 * away.iter().map(|offset| offset.length()).sum::<f32>() / count.max(1) as f32,
    };
    // Slots round the ring in the order the objects stand round the centre (after the swirl), so
    // their paths do not cross; the ring is turned to suit them best.
    let step = TAU / count.max(1) as f32;
    let angle_of = |offset: Vec3| if offset.length() > 1e-5 { offset.y.atan2(offset.x) } else { 0.0 };
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by(|a, b| {
        let (a, b) = (away.get(*a).copied().map_or(0.0, angle_of), away.get(*b).copied().map_or(0.0, angle_of));
        a.total_cmp(&b)
    });
    let pull = order.iter().enumerate().fold(Vec3::ZERO, |sum, (slot, index)| {
        let angle = away.get(*index).copied().map_or(0.0, angle_of) + arc - slot as f32 * step;
        sum + Vec3::new(angle.cos(), angle.sin(), 0.0)
    });
    let turn = if pull.length() > 1e-5 { pull.y.atan2(pull.x) } else { 0.0 };
    let mut slots = vec![0.0_f32; count];
    for (slot, index) in order.iter().enumerate() {
        if let Some(angle) = slots.get_mut(*index) {
            *angle = turn + slot as f32 * step;
        }
    }
    let mut end = start;
    for (index, id) in ids.iter().enumerate() {
        let (Some((location, rotation)), Some(offset), Some(slot), Some(begin), Some(into)) =
            (places.get(index), away.get(index), slots.get(index), starts.get(index), frames_of.get(index))
        else {
            continue;
        };
        let (from_radius, from_angle) = (offset.length(), if offset.length() > 1e-5 { angle_of(*offset) } else { *slot });
        // The swirl, then the shortest way on to its slot.
        let travel = arc + wrapped(slot - from_angle - arc);
        let (from_z, to_z) = (location.z, level.unwrap_or(location.z));
        let finish_at = begin + duration;
        end = end.max(finish_at);
        clear_keys(&mut doc.scene, *id, Property::Location, *begin, finish_at);
        if spin.is_some() {
            clear_keys(&mut doc.scene, *id, Property::Rotation, *begin, finish_at);
        }
        for frame in sample_times(*begin, finish_at) {
            let e = GATHER.at((frame - begin) / duration);
            let (reach, angle) = (lerp(from_radius, radius, e), from_angle + travel * e);
            let place = Vec3::new(center.x + reach * angle.cos(), center.y + reach * angle.sin(), lerp(from_z, to_z, e));
            put_vector(&mut doc.scene, *id, Property::Location, frame, into.transform_point3(place))?;
            if let Some(spin) = spin {
                put_vector(&mut doc.scene, *id, Property::Rotation, frame, *rotation + spin * e)?;
            }
        }
    }
    finish(&mut doc.scene, end);
    Ok(json!({ "objects": ids, "starts": starts, "radius": radius, "end": end }))
}

// object.wave

fn wave(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let ids = chosen(doc, params)?;
    let start = start_frame(doc, params)?;
    let duration = frames(params, "duration", 10.0)?;
    let speed = bounded(params, "speed", 4.0, 0.001, 100_000.0)?;
    let fps = doc.scene.animation.timeline.fps.max(1.0);
    let evaluated = doc.scene.at_frame(start);
    let (origin, _) = target_point(&evaluated, params, "origin")?.ok_or_else(|| bad("`origin` is required"))?;
    let property = params::string(params, "property")?.unwrap_or("scale");
    let color = match property {
        "scale" => None,
        "color" => Some(params::color(params, "color")?.ok_or_else(|| bad("a color wave needs `color`"))?),
        other => return Err(bad(format!("`property` is scale or color, not {other}"))),
    };
    let amount = bounded(params, "amount", 0.25, -0.9, 100.0)?;
    // Each object's material, which must be its own: a shared one would switch for all at once.
    let mut materials = Vec::new();
    if color.is_some() {
        for id in &ids {
            let object = evaluated.object(*id).map_err(|error| error.to_string())?;
            let material = object.material.ok_or_else(|| bad(format!("{} has no material to colour", object.name)))?;
            if materials.contains(&material) {
                return Err(bad(format!("{} shares its material with another object in the wave; give each its own (material.new)", object.name)));
            }
            materials.push(material);
        }
    }
    let mut starts = Vec::with_capacity(ids.len());
    let mut end = start;
    for (index, id) in ids.iter().enumerate() {
        let reached = start + (object_center(&evaluated, *id)? - origin).length() / speed * fps;
        if reached.abs() > (kition_scene_model::animation::MAX_FRAME - 20_000) as f32 {
            return Err(bad("the wave reaches an object too late; raise `speed`"));
        }
        let done = reached + duration;
        starts.push(reached);
        end = end.max(done);
        match (color, materials.get(index)) {
            (Some(color), Some(material)) => {
                let from = evaluated.material(*material).map_err(|error| error.to_string())?.base_color;
                clear_keys(&mut doc.scene, *material, Property::BaseColor, reached, done);
                for frame in sample_times(reached, done) {
                    let e = sine((frame - reached) / duration);
                    for (channel, (a, b)) in (0u8..).zip(from.iter().zip(color.iter())) {
                        put_key(&mut doc.scene, *material, Property::BaseColor, channel, frame, lerp(*a, *b, e))?;
                    }
                }
            }
            _ => {
                let scale = evaluated.object(*id).map_err(|error| error.to_string())?.scale;
                clear_keys(&mut doc.scene, *id, Property::Scale, reached, done);
                for frame in sample_times(reached, done) {
                    let bump = (PI * ((frame - reached) / duration).clamp(0.0, 1.0)).sin();
                    put_vector(&mut doc.scene, *id, Property::Scale, frame, scale * (1.0 + amount * bump))?;
                }
            }
        }
    }
    finish(&mut doc.scene, end);
    Ok(json!({ "objects": ids, "starts": starts, "end": end }))
}

fn pop_in(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let ids = chosen(doc, params)?;
    let start = start_frame(doc, params)?;
    let duration = frames(params, "duration", 12.0)?;
    let starts = staggered(params, start, ids.len(), 3.0)?;
    let evaluated = doc.scene.at_frame(start);
    let mut end = start;
    for (id, begin) in ids.iter().zip(&starts) {
        let finish_at = begin + duration;
        end = end.max(finish_at);
        let scale = evaluated.object(*id).map_err(|error| error.to_string())?.scale;
        clear_keys(&mut doc.scene, *id, Property::Scale, *begin, finish_at);
        for frame in sample_times(*begin, finish_at) {
            // Never quite zero: a zero scale is flat and has no inverse.
            let size = POP.at((frame - begin) / duration).max(0.001);
            put_vector(&mut doc.scene, *id, Property::Scale, frame, scale * size)?;
        }
    }
    finish(&mut doc.scene, end);
    Ok(json!({ "objects": ids, "starts": starts, "end": end }))
}
