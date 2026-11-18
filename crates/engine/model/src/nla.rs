//! Nonlinear animation: tracks of action strips layered on an object, evaluated as the reference
//! application's NLA does (measured in `tests/nla_blender.rs`).
//!
//! - A **strip** plays an action's frames `actionStart`–`actionEnd` from `start`, `scale` times
//!   slower and `repeat` times over (backwards when `reversed`); it ends at
//!   `start + (actionEnd − actionStart) · scale · repeat`. At the very end of a whole number of
//!   repeats it shows the action's last frame, elsewhere a repeat starts at the first.
//! - A strip's **influence** ramps linearly from 0 over `blendIn` frames after its start and back
//!   to 0 over `blendOut` frames before its end, unless the strip has a fixed influence.
//!   **Auto blend** sets both from the strips of the tracks just below and above it that overlap
//!   its start or end.
//! - Outside its frames a strip **holds** (both ways, but only the first strip of a track holds
//!   backwards), **holds forward**, or does **nothing**; between strips of a track the nearest
//!   strip before holds forward if it may. A held strip shows the frame and influence of its end.
//! - Tracks are evaluated from the bottom up, each blending onto what is below: **replace**
//!   mixes towards the strip's value, **add** and **subtract** add or take away the value times
//!   the influence, **multiply** mixes towards the product, and **combine** adds the difference
//!   from the property's default (scale multiplies by the value to the power of the influence).
//!   Every property a strip on an unmuted track (muted or not itself) or the object's own keys
//!   animate starts from its default (location 0, scale 1, …); others keep their values. On/off
//!   properties round down.
//! - **Muted** strips and tracks do nothing; a **solo** track is the only one evaluated (the
//!   object's own keys then do nothing either).
//! - The object's own keys (the **active action**) go on top, with their own blend, influence,
//!   and extrapolation over their keyed range.
//! - **Transitions** mix the neighboring strips at their endpoints over the same lower stack.
//!   **Meta strips** contain strips with their own blending; moving or stretching a meta moves
//!   or stretches its children together. See `nla_groups` and `tests/nla6_blender.rs`.
//! - **Tweak mode** edits a strip's action in place: the object's own keys are put aside and the
//!   strip's action's keys become its own, played through the strip (its timing, influence, and
//!   blend); the tracks below are evaluated, the strip's track's other strips are not, and the
//!   tracks above, then the keys put aside (with the active action's blend), only with
//!   `upperStack`. Leaving it writes the keys back into the action.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::animation::{Action, ActionChannel, Channel, Property, evaluate_keys, in_nla, read_at};
use crate::error::{Result, SceneError};
use crate::scene::{Id, Scene, unique_name};

/// How a strip (or the active action) mixes onto what is below it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BlendType {
    #[default]
    Replace,
    Combine,
    Add,
    Subtract,
    Multiply,
}

impl BlendType {
    pub const ALL: [Self; 5] = [Self::Replace, Self::Combine, Self::Add, Self::Subtract, Self::Multiply];

    pub fn name(self) -> &'static str {
        match self {
            Self::Replace => "replace",
            Self::Combine => "combine",
            Self::Add => "add",
            Self::Subtract => "subtract",
            Self::Multiply => "multiply",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|blend| blend.name() == name)
    }
}

/// What a strip (or the active action) does outside its frames.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Extrapolation {
    #[default]
    Hold,
    HoldForward,
    Nothing,
}

impl Extrapolation {
    pub const ALL: [Self; 3] = [Self::Hold, Self::HoldForward, Self::Nothing];

    pub fn name(self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::HoldForward => "holdForward",
            Self::Nothing => "nothing",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|extrapolation| extrapolation.name() == name)
    }
}

fn one() -> f32 {
    1.0
}

fn is_one(value: &f32) -> bool {
    *value == 1.0
}

fn is_zero(value: &f32) -> bool {
    *value == 0.0
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

/// The strip's content. Existing action strips default to Action inside the NLA JSON register.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StripKind {
    #[default]
    Action,
    Transition,
    Meta,
}

/// One strip: an action, a transition between neighbors, or a group of children.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NlaStrip {
    pub name: String,
    #[serde(default, skip_serializing_if = "is_default")]
    pub kind: StripKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<NlaStrip>,
    /// The action played, by name.
    pub action: String,
    /// The scene frame the strip starts at.
    pub start: f32,
    /// The action's frames the strip plays.
    pub action_start: f32,
    pub action_end: f32,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub scale: f32,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub repeat: f32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reversed: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub blend_in: f32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub blend_out: f32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto_blend: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    pub extrapolation: Extrapolation,
    #[serde(default, skip_serializing_if = "is_default")]
    pub blend: BlendType,
    /// A fixed influence, in place of the blends in and out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub influence: Option<f32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub muted: bool,
}

/// The smallest scale and repeat a strip takes.
pub const MIN_SCALE: f32 = 1e-4;
pub const MIN_REPEAT: f32 = 0.01;
/// The most tracks on an object and strips on a track.
pub const MAX_TRACKS: usize = 256;
pub const MAX_STRIPS: usize = 1024;

impl NlaStrip {
    /// A strip playing all of an action's keyed frames from `start` (an action with one keyed
    /// frame plays one frame).
    pub fn new(name: impl Into<String>, action: &Action, start: f32) -> Self {
        let (action_start, action_end) = match action.range() {
            Some((first, last)) if last > first => (first, last),
            Some((first, _)) => (first, first + 1.0),
            None => (1.0, 2.0),
        };
        Self {
            name: name.into(),
            kind: StripKind::Action,
            children: Vec::new(),
            action: action.name.clone(),
            start,
            action_start,
            action_end,
            scale: 1.0,
            repeat: 1.0,
            reversed: false,
            blend_in: 0.0,
            blend_out: 0.0,
            auto_blend: false,
            extrapolation: Extrapolation::Hold,
            blend: BlendType::Replace,
            influence: None,
            muted: false,
        }
    }

    /// The scene frames the strip lasts.
    pub fn length(&self) -> f32 {
        (self.action_end - self.action_start) * self.scale * self.repeat
    }

    /// The scene frame the strip ends at.
    pub fn end(&self) -> f32 {
        self.start + self.length()
    }

    /// The action frame the strip plays at scene `frame` (within its frames).
    pub fn action_time(&self, frame: f32) -> f32 {
        let span = self.action_end - self.action_start;
        if !(span > 0.0 && self.scale > 0.0) {
            return self.action_start;
        }
        let scaled = (frame - self.start) / self.scale;
        let mut within = scaled.rem_euclid(span);
        // The very end of a whole number of repeats is the action's last frame.
        if within == 0.0 && scaled > 0.0 && frame >= self.end() {
            within = span;
        }
        if self.reversed { self.action_end - within } else { self.action_start + within }
    }

    /// The action frame scene `frame` maps to without wrapping repeats (where a key inserted in
    /// tweak mode goes).
    pub fn unmapped(&self, frame: f32) -> f32 {
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        let local = (frame - self.start) / scale;
        if self.reversed { self.action_end - local } else { self.action_start + local }
    }

    /// The scene frame of action frame `time` in the strip's first repeat.
    pub fn scene_time(&self, time: f32) -> f32 {
        let local = if self.reversed { self.action_end - time } else { time - self.action_start };
        self.start + local * self.scale
    }

    /// The strip's influence at scene `frame` (within its frames).
    pub fn weight(&self, frame: f32) -> f32 {
        if let Some(influence) = self.influence {
            return influence;
        }
        let (start, end) = (self.start, self.end());
        let mut weight = 1.0;
        if self.blend_in > 0.0 && frame < start + self.blend_in {
            weight *= ((frame - start) / self.blend_in).clamp(0.0, 1.0);
        }
        if self.blend_out > 0.0 && frame > end - self.blend_out {
            weight *= ((end - frame) / self.blend_out).clamp(0.0, 1.0);
        }
        weight
    }

    /// Keeps the blends inside the strip: blend out first, then blend in.
    fn clamp_blends(&mut self) {
        let length = self.length().max(0.0);
        self.blend_out = self.blend_out.clamp(0.0, length);
        self.blend_in = self.blend_in.clamp(0.0, (length - self.blend_out).max(0.0));
    }
}

/// A track of strips, in frame order, not overlapping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NlaTrack {
    pub name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub muted: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub solo: bool,
    #[serde(default)]
    pub strips: Vec<NlaStrip>,
}

impl NlaTrack {
    /// The strip that plays at `frame`, and the frame it is evaluated at (its end or start when
    /// held).
    pub fn strip_at(&self, frame: f32) -> Option<(usize, f32)> {
        strip_at(&self.strips, frame)
    }

    /// Whether `[start, end]` overlaps none of the strips but `except`.
    pub fn has_room(&self, start: f32, end: f32, except: Option<usize>) -> bool {
        self.strips.iter().enumerate().all(|(slot, strip)| Some(slot) == except || end <= strip.start || start >= strip.end())
    }
}

fn strip_at(strips: &[NlaStrip], frame: f32) -> Option<(usize, f32)> {
    if let Some(slot) = strips.iter().position(|strip| strip.start <= frame && frame <= strip.end()) {
        return Some((slot, frame));
    }
    if let Some(slot) = strips.iter().rposition(|strip| strip.end() < frame) {
        let strip = strips.get(slot)?;
        return (strip.extrapolation != Extrapolation::Nothing).then(|| (slot, strip.end()));
    }
    let first = strips.first()?;
    (frame < first.start && first.extrapolation == Extrapolation::Hold).then_some((0, first.start))
}

/// How the object's own keys (its active action) go on top of the tracks.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionBlend {
    #[serde(default)]
    pub blend: BlendType,
    #[serde(default = "one")]
    pub influence: f32,
    #[serde(default)]
    pub extrapolation: Extrapolation,
}

impl Default for ActionBlend {
    fn default() -> Self {
        Self { blend: BlendType::Replace, influence: 1.0, extrapolation: Extrapolation::Hold }
    }
}

/// A strip being tweaked: its action's keys are the object's own until tweaking ends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tweak {
    pub track: usize,
    pub strip: usize,
    /// The action the strip plays: tweak mode holds only while that strip plays it.
    pub action: String,
    /// The tracks above are evaluated too.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub upper_stack: bool,
    /// The object's own keys, put aside.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stash: Vec<ActionChannel>,
}

/// An object's NLA.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Nla {
    /// Bottom first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracks: Vec<NlaTrack>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub action: ActionBlend,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tweak: Option<Tweak>,
}

/// The value a property starts from in the NLA.
pub fn default_value(property: Property) -> f32 {
    match property {
        Property::Scale | Property::BoneScale | Property::Color | Property::BaseColor => 1.0,
        Property::Power => 10.0,
        Property::Lens => 50.0,
        Property::Location | Property::Rotation | Property::Hidden | Property::BoneLocation | Property::BoneRotation | Property::ShapeKey => 0.0,
    }
}

/// `value` blended onto `lower` with `weight`.
pub fn blend(blend: BlendType, property: Property, lower: f32, value: f32, weight: f32) -> f32 {
    match blend {
        BlendType::Replace => lower + (value - lower) * weight,
        BlendType::Add => lower + value * weight,
        BlendType::Subtract => lower - value * weight,
        BlendType::Multiply => lower * (1.0 - weight) + lower * value * weight,
        BlendType::Combine => match property {
            Property::Scale | Property::BoneScale => {
                let factor = value.powf(weight);
                if factor.is_finite() { lower * factor } else { lower }
            }
            _ => lower + (value - default_value(property)) * weight,
        },
    }
}

type Path = (Property, u8, String);

/// A set of channels played as one action (an action's, or the object's own).
enum Keys<'a> {
    Action(&'a [ActionChannel]),
    Own(&'a [&'a Channel]),
}

impl Keys<'_> {
    fn each(&self, mut visit: impl FnMut(Property, u8, &str, Option<f32>), frame: Option<f32>) {
        let at = |keys: &[crate::Keyframe], modifiers: &[crate::fmodifier::FModifier], property: Property| {
            frame.and_then(|frame| evaluate_keys(keys, modifiers, property, frame))
        };
        match self {
            Keys::Action(channels) => {
                for channel in channels.iter() {
                    visit(channel.property, channel.index, &channel.path, at(&channel.keys, &channel.modifiers, channel.property));
                }
            }
            Keys::Own(channels) => {
                for channel in channels.iter() {
                    visit(channel.property, channel.index, &channel.path, at(&channel.keys, &channel.modifiers, channel.property));
                }
            }
        }
    }
}

impl Keys<'_> {
    /// Calls `visit` with each channel's first and last keyed frame.
    fn ranges(&self, mut visit: impl FnMut(f32, f32)) {
        let mut each = |keys: &[crate::Keyframe]| {
            if let (Some(first), Some(last)) = (keys.first(), keys.last()) {
                visit(first.frame, last.frame);
            }
        };
        match self {
            Keys::Action(channels) => channels.iter().for_each(|channel| each(&channel.keys)),
            Keys::Own(channels) => channels.iter().for_each(|channel| each(&channel.keys)),
        }
    }
}

struct Stack {
    values: Vec<(Path, f32)>,
    slots: BTreeMap<Path, usize>,
}

impl Stack {
    fn touch(&mut self, property: Property, index: u8, path: &str) {
        let key = (property, index, path.to_string());
        if !self.slots.contains_key(&key) {
            self.slots.insert(key.clone(), self.values.len());
            self.values.push((key, default_value(property)));
        }
    }

    fn layer(&mut self, keys: &Keys<'_>, time: f32, kind: BlendType, weight: f32) {
        let mut updates = Vec::new();
        keys.each(
            |property, index, path, value| {
                if let Some(value) = value {
                    updates.push((property, index, path.to_string(), value));
                }
            },
            Some(time),
        );
        for (property, index, path, value) in updates {
            self.touch(property, index, &path);
            if let Some(entry) = self.slots.get(&(property, index, path)).and_then(|slot| self.values.get_mut(*slot)) {
                entry.1 = blend(kind, property, entry.1, value, weight);
            }
        }
    }
}

impl Stack {
    fn strip(&mut self, strips: &[NlaStrip], at: usize, actions: &[Action], time: f32, endpoint: bool) {
        let Some(strip) = strips.get(at) else { return };
        if strip.muted && !endpoint {
            return;
        }
        match strip.kind {
            StripKind::Action => {
                if let Some(action) = actions.iter().find(|action| action.name == strip.action) {
                    self.layer(&Keys::Action(&action.channels), strip.action_time(time), strip.blend, strip.weight(time));
                }
            }
            StripKind::Meta => {
                if let Some((slot, held)) = strip_at(&strip.children, time) {
                    self.strip(&strip.children, slot, actions, held, false);
                }
            }
            StripKind::Transition => {
                let Some(left) = at.checked_sub(1) else { return };
                let Some((a, b)) = strips.get(left).zip(strips.get(at + 1)) else { return };
                if a.kind == StripKind::Transition || b.kind == StripKind::Transition || strip.length() <= 0.0 {
                    return;
                }
                // evaluate() registers every endpoint channel before playing any strips,
                // including meta children. Only scalar values change here; their slots stay
                // fixed. Swap the left result with the saved lower stack so both endpoints
                // start from that same lower stack, without cloning paths or lookup trees.
                let mut before: Vec<f32> = self.values.iter().map(|(_, value)| *value).collect();
                self.strip(strips, left, actions, a.end(), true);
                for ((_, value), lower) in self.values.iter_mut().zip(&mut before) {
                    std::mem::swap(value, lower);
                }
                self.strip(strips, at + 1, actions, b.start, true);
                let t = ((time - strip.start) / strip.length()).clamp(0.0, 1.0);
                for ((_, value), a) in self.values.iter_mut().zip(before) {
                    *value = a + (*value - a) * t;
                }
            }
        }
    }
}

impl Nla {
    /// Whether the NLA does nothing (no tracks, no settings on the own keys, not tweaking).
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty() && self.action == ActionBlend::default() && self.tweak.is_none()
    }

    /// The strip being tweaked.
    pub fn tweaked(&self) -> Option<&NlaStrip> {
        let tweak = self.tweak.as_ref()?;
        self.tracks.get(tweak.track)?.strips.get(tweak.strip).filter(|strip| strip.action == tweak.action)
    }

    /// The NLA without tweak mode (what the replica's `nla` register holds; tweak mode, with the
    /// keys put aside, is a register of its own).
    pub fn without_tweak(&self) -> Nla {
        Nla { tweak: None, ..self.clone() }
    }

    /// The solo track, if any.
    pub fn solo(&self) -> Option<usize> {
        self.tracks.iter().position(|track| track.solo)
    }

    /// The values the NLA gives at `frame`: every property its strips' actions or the object's
    /// own keys (`own`) animate. `actions` are the scene's.
    pub fn evaluate(&self, own: &[&Channel], actions: &[Action], frame: f32) -> Vec<(Property, u8, String, f32)> {
        if self.check_structure().is_err() {
            return Vec::new();
        }
        let action = |name: &str| actions.iter().find(|action| action.name == name);
        let mut stack = Stack { values: Vec::new(), slots: BTreeMap::new() };
        let solo = self.solo();
        for (slot, track) in self.tracks.iter().enumerate() {
            // Muted tracks (and with a solo track, every other) leave their properties alone;
            // muted strips do not.
            if track.muted || solo.is_some_and(|solo| solo != slot) {
                continue;
            }
            for strip in &track.strips {
                strip.visit_actions(&mut |name| {
                    if let Some(action) = action(name) {
                        Keys::Action(&action.channels).each(|property, index, path, _| stack.touch(property, index, path), None);
                    }
                });
            }
        }
        let own_keys = Keys::Own(own);
        own_keys.each(|property, index, path, _| stack.touch(property, index, path), None);
        let own_keys = || Keys::Own(own);

        let tweak = self.tweak.as_ref().filter(|tweak| self.tracks.get(tweak.track).is_some_and(|track| track.strips.get(tweak.strip).is_some()));
        for (slot, track) in self.tracks.iter().enumerate() {
            if let Some(tweak) = tweak {
                if slot == tweak.track {
                    if let Some(strip) = track.strips.get(tweak.strip) {
                        let held = if strip.start <= frame && frame <= strip.end() {
                            Some(frame)
                        } else if frame > strip.end() && strip.extrapolation != Extrapolation::Nothing {
                            Some(strip.end())
                        } else if frame < strip.start && strip.extrapolation == Extrapolation::Hold && tweak.strip == 0 {
                            Some(strip.start)
                        } else {
                            None
                        };
                        if let Some(at) = held {
                            stack.layer(&own_keys(), strip.action_time(at), strip.blend, strip.weight(at));
                        }
                    }
                    continue;
                }
                if slot > tweak.track && !tweak.upper_stack {
                    continue;
                }
            }
            if track.muted || solo.is_some_and(|solo| solo != slot) {
                continue;
            }
            let Some((at, time)) = track.strip_at(frame) else { continue };
            stack.strip(&track.strips, at, actions, time, false);
        }
        // The object's own keys go on top (in tweak mode with the upper stack, the keys put
        // aside do).
        let top = match tweak {
            None if solo.is_none() => Some(own_keys()),
            Some(tweak) if tweak.upper_stack => Some(Keys::Action(&tweak.stash)),
            _ => None,
        };
        if let Some(top) = top {
            let mut range: Option<(f32, f32)> = None;
            top.ranges(|first, last| range = Some(range.map_or((first, last), |(a, b)| (a.min(first), b.max(last)))));
            if tweak.is_some() {
                top.each(|property, index, path, _| stack.touch(property, index, path), None);
            }
            let plays = match (self.action.extrapolation, range) {
                (_, None) => false,
                (Extrapolation::Hold, Some(_)) => true,
                (Extrapolation::HoldForward, Some((first, _))) => frame >= first,
                (Extrapolation::Nothing, Some((first, last))) => first <= frame && frame <= last,
            };
            if plays {
                stack.layer(&top, frame, self.action.blend, self.action.influence);
            }
        }
        stack
            .values
            .into_iter()
            .map(|((property, index, path), value)| {
                let value = if property == Property::Hidden { value.floor().clamp(0.0, 1.0) } else { value };
                (property, index, path, value)
            })
            .collect()
    }

    /// Clamps every strip's blends and works out the auto blends again, as after any change.
    pub fn settle(&mut self) {
        if self.check_structure().is_err() {
            return;
        }
        for track in &mut self.tracks {
            crate::nla_groups::settle_strips(&mut track.strips);
        }
        let spans: Vec<Vec<(f32, f32)>> =
            self.tracks.iter().map(|track| track.strips.iter().map(|strip| (strip.start, strip.end())).collect()).collect();
        for (slot, track) in self.tracks.iter_mut().enumerate() {
            for strip in &mut track.strips {
                if strip.auto_blend {
                    let (start, end) = (strip.start, strip.end());
                    let (mut blend_in, mut blend_out) = (0.0f32, 0.0f32);
                    let neighbours = [slot.checked_sub(1), Some(slot + 1)];
                    for other in neighbours.into_iter().flatten().filter_map(|near| spans.get(near)).flatten() {
                        if start < other.1 && other.1 <= end {
                            blend_in = blend_in.max(other.1 - start);
                        }
                        if start <= other.0 && other.0 < end {
                            blend_out = blend_out.max(end - other.0);
                        }
                    }
                    strip.blend_in = blend_in;
                    strip.blend_out = blend_out;
                }
                strip.clamp_blends();
            }
        }
    }

    /// Checks the NLA of an object read from a file, against the scene's actions.
    pub fn check(&self, actions: &[Action]) -> std::result::Result<(), String> {
        self.check_structure()?;
        if self.tracks.len() > MAX_TRACKS {
            return Err(format!("an object has more than {MAX_TRACKS} NLA tracks"));
        }
        let finite = |values: &[f32]| values.iter().all(|value| value.is_finite() && value.abs() <= crate::animation::MAX_FRAME as f32 * 16.0);
        if self.tracks.iter().filter(|track| track.solo).count() > 1 {
            return Err("only one NLA track is solo".into());
        }
        for track in &self.tracks {
            crate::nla_groups::check_transitions(&track.strips)?;
            if track.strips.len() > MAX_STRIPS || track.name.len() > 128 {
                return Err(format!("an NLA track has more than {MAX_STRIPS} strips or a name too long"));
            }
            let mut pending: Vec<(&NlaStrip, usize)> = track.strips.iter().map(|strip| (strip, 0)).collect();
            let mut count = 0;
            while let Some((strip, depth)) = pending.pop() {
                count += 1;
                if depth > crate::nla_groups::MAX_META_DEPTH || count > MAX_STRIPS {
                    return Err("too many nested NLA strips".into());
                }
                if strip.kind == StripKind::Meta {
                    if strip.children.is_empty()
                        || !strip.action.is_empty()
                        || strip.scale != 1.0
                        || strip.repeat != 1.0
                        || strip.reversed
                        || strip.children.windows(2).any(|pair| matches!(pair, [a,b] if b.start < a.end() - 1e-4))
                    {
                        return Err("a meta needs ordered children and a plain scene frame range".into());
                    }
                    if strip.children.iter().any(|child| child.start < strip.start - 1e-4 || child.end() > strip.end() + 1e-4) {
                        return Err("a meta's children must stay inside its frame range".into());
                    }
                    pending.extend(strip.children.iter().map(|child| (child, depth + 1)));
                } else if !strip.children.is_empty() {
                    return Err("only meta strips have children".into());
                }
                if strip.kind == StripKind::Action && !actions.iter().any(|action| action.name == strip.action) {
                    return Err(format!("strip `{}` plays action `{}`, which does not exist", strip.name, strip.action));
                }
                if strip.name.len() > 128
                    || !finite(&[
                        strip.start,
                        strip.action_start,
                        strip.action_end,
                        strip.scale,
                        strip.repeat,
                        strip.blend_in,
                        strip.blend_out,
                        strip.end(),
                    ])
                    || strip.action_end <= strip.action_start
                    || strip.scale < MIN_SCALE
                    || strip.repeat < MIN_REPEAT
                    || strip.blend_in < 0.0
                    || strip.blend_out < 0.0
                    || strip.blend_in + strip.blend_out > strip.length() + 1e-3
                    || strip.influence.is_some_and(|influence| !(0.0..=1.0).contains(&influence))
                {
                    return Err(format!("strip `{}` has a frame range, scale, repeat, blend, or influence out of range", strip.name));
                }
            }
            if track.strips.windows(2).any(|pair| matches!(pair, [a, b] if b.start < a.end() - 1e-4)) {
                return Err(format!("the strips of NLA track `{}` overlap or are out of order", track.name));
            }
        }
        if !(self.action.influence.is_finite() && (0.0..=1.0).contains(&self.action.influence)) {
            return Err("the active action's influence is from 0 to 1".into());
        }
        if let Some(tweak) = &self.tweak
            && self.tweaked().is_none()
        {
            return Err(format!("tweak mode names strip {} of track {}, which does not exist", tweak.strip, tweak.track));
        }
        Ok(())
    }
}

impl Scene {
    fn nla_of(&self, object: Id) -> Result<&Nla> {
        Ok(&self.object(object)?.nla)
    }

    fn nla_mut(&mut self, object: Id) -> Result<&mut Nla> {
        Ok(&mut self.object_mut(object)?.nla)
    }

    /// The strip being tweaked on `object`.
    pub fn tweaked_strip(&self, object: Id) -> Option<&NlaStrip> {
        self.object(object).ok()?.nla.tweaked()
    }

    /// The frame a key inserted on `object` at scene `frame` goes at: the tweaked strip's
    /// action frame in tweak mode, else `frame`.
    pub fn key_time(&self, object: Id, frame: f32) -> f32 {
        self.tweaked_strip(object).map_or(frame, |strip| strip.unmapped(frame))
    }

    /// A new track on top of `object`'s NLA (or at `at`, counted from the bottom). Returns its
    /// position.
    pub fn nla_add_track(&mut self, object: Id, name: Option<&str>, at: Option<usize>) -> Result<usize> {
        let nla = self.nla_mut(object)?;
        if nla.tweak.is_some() {
            return Err(SceneError::NotAllowed("leave tweak mode first".into()));
        }
        if nla.tracks.len() >= MAX_TRACKS {
            return Err(SceneError::params(format!("an object has at most {MAX_TRACKS} tracks")));
        }
        let name = unique_name(name.unwrap_or("NlaTrack"), |taken| nla.tracks.iter().any(|track| track.name == taken));
        let at = at.unwrap_or(nla.tracks.len()).min(nla.tracks.len());
        nla.tracks.insert(at, NlaTrack { name, muted: false, solo: false, strips: Vec::new() });
        Ok(at)
    }

    /// Puts a strip playing `action` on `object` from `start`: on `track`, or on the top track
    /// when it has room, or on a new track on top. Returns the track and the strip's position.
    pub fn nla_add_strip(&mut self, object: Id, track: Option<usize>, action: &str, start: f32) -> Result<(usize, usize)> {
        if !start.is_finite() || start.abs() > crate::animation::MAX_FRAME as f32 {
            return Err(SceneError::params("the start frame is out of range"));
        }
        let played = self.animation.action(action).cloned().ok_or_else(|| SceneError::NotFound(format!("no action {action}")))?;
        let tweaking = self.nla_of(object)?.tweak.is_some();
        if tweaking {
            return Err(SceneError::NotAllowed("leave tweak mode first".into()));
        }
        let mut strip = NlaStrip::new(played.name.clone(), &played, start);
        let nla = self.nla_mut(object)?;
        let fits = |track: &NlaTrack| track.has_room(strip.start, strip.end(), None) && track.strips.len() < MAX_STRIPS;
        let slot = match track {
            Some(slot) => {
                let track = nla.tracks.get(slot).ok_or_else(|| SceneError::NotFound(format!("no track {slot}")))?;
                if !fits(track) {
                    return Err(SceneError::params("the strip overlaps another on that track"));
                }
                slot
            }
            None => match nla.tracks.len().checked_sub(1).filter(|top| nla.tracks.get(*top).is_some_and(fits)) {
                Some(top) => top,
                None => self.nla_add_track(object, None, None)?,
            },
        };
        let nla = self.nla_mut(object)?;
        let track = nla.tracks.get_mut(slot).ok_or_else(|| SceneError::NotFound(format!("no track {slot}")))?;
        strip.name = unique_name(&strip.name, |taken| track.strips.iter().any(|other| other.name == taken));
        let at = track.strips.iter().position(|other| other.start > strip.start).unwrap_or(track.strips.len());
        track.strips.insert(at, strip);
        nla.settle();
        Ok((slot, at))
    }

    /// Push Down: `object`'s own keys become a new action played by a strip on a new track on
    /// top. Returns the action's name.
    pub fn nla_push_down(&mut self, object: Id, name: Option<&str>) -> Result<String> {
        if self.nla_of(object)?.tweak.is_some() {
            return Err(SceneError::NotAllowed("leave tweak mode first".into()));
        }
        let channels: Vec<ActionChannel> = self
            .animation
            .channels
            .iter()
            .filter(|channel| channel.target == object && in_nla(channel) && !channel.keys.is_empty())
            .map(|channel| ActionChannel {
                property: channel.property,
                index: channel.index,
                path: channel.path.clone(),
                keys: channel.keys.clone(),
                modifiers: channel.modifiers.clone(),
            })
            .collect();
        if channels.is_empty() {
            return Err(SceneError::NotAllowed("the object has no keys to push down".into()));
        }
        let base = match name {
            Some(name) => name.to_string(),
            None => format!("{}Action", self.object(object)?.name),
        };
        let name = unique_name(&base, |taken| self.animation.actions.iter().any(|action| action.name == taken));
        let action = Action { name: name.clone(), channels };
        let start = action.range().map_or(1.0, |range| range.0);
        self.animation.actions.push(action);
        let track = self.nla_add_track(object, None, None)?;
        self.nla_add_strip(object, Some(track), &name, start)?;
        self.animation.channels.retain(|channel| !(channel.target == object && in_nla(channel)));
        Ok(name)
    }

    /// Enters tweak mode on strip `strip` of track `track`: the object's own keys are put aside
    /// and the strip's action's keys become its own.
    pub fn nla_tweak(&mut self, object: Id, track: usize, strip: usize, upper_stack: bool) -> Result<()> {
        let nla = self.nla_of(object)?;
        if nla.tweak.is_some() {
            return Err(SceneError::NotAllowed("already in tweak mode".into()));
        }
        if nla.tracks.get(track).and_then(|found| found.strips.get(strip)).is_some_and(|strip| strip.kind != StripKind::Action) {
            return Err(SceneError::NotAllowed("only action strips can be tweaked; ungroup a meta to edit its child actions".into()));
        }
        let played = nla
            .tracks
            .get(track)
            .and_then(|found| found.strips.get(strip))
            .map(|found| found.action.clone())
            .ok_or_else(|| SceneError::NotFound(format!("no strip {strip} on track {track}")))?;
        let action = self.animation.action(&played).cloned().ok_or_else(|| SceneError::NotFound(format!("no action {played}")))?;
        let stash: Vec<ActionChannel> = self
            .animation
            .channels
            .iter()
            .filter(|channel| channel.target == object && in_nla(channel))
            .map(|channel| ActionChannel {
                property: channel.property,
                index: channel.index,
                path: channel.path.clone(),
                keys: channel.keys.clone(),
                modifiers: channel.modifiers.clone(),
            })
            .collect();
        self.animation.channels.retain(|channel| !(channel.target == object && in_nla(channel)));
        for channel in &action.channels {
            if !self.fits_own(object, channel) {
                continue;
            }
            self.animation.channels.push(Channel {
                target: object,
                property: channel.property,
                index: channel.index,
                path: channel.path.clone(),
                keys: channel.keys.clone(),
                driver: None,
                modifiers: channel.modifiers.clone(),
            });
        }
        self.nla_mut(object)?.tweak = Some(Tweak { track, strip, action: played, upper_stack, stash });
        Ok(())
    }

    /// Whether an action's channel can be one of `object`'s own keys (the object has the
    /// property and no driver on it).
    fn fits_own(&self, object: Id, channel: &ActionChannel) -> bool {
        channel.property != Property::ShapeKey
            && !channel.property.on_material()
            && read_at(self, object, channel.property, channel.index, &channel.path).is_some()
            && self.driver_of(object, channel.property, channel.index, &channel.path).is_none()
    }

    /// Leaves tweak mode: the object's keys go back into the strip's action (with the action's
    /// channels the object cannot take), and its own keys come back.
    pub fn nla_tweak_exit(&mut self, object: Id) -> Result<()> {
        let Some(tweak) = self.nla_mut(object)?.tweak.take() else {
            return Err(SceneError::NotAllowed("not in tweak mode".into()));
        };
        let played = self.nla_of(object)?.tracks.get(tweak.track).and_then(|track| track.strips.get(tweak.strip)).map(|strip| strip.action.clone());
        let mine: Vec<ActionChannel> = self
            .animation
            .channels
            .iter()
            .filter(|channel| channel.target == object && in_nla(channel) && !channel.keys.is_empty())
            .map(|channel| ActionChannel {
                property: channel.property,
                index: channel.index,
                path: channel.path.clone(),
                keys: channel.keys.clone(),
                modifiers: channel.modifiers.clone(),
            })
            .collect();
        if let Some(name) = played {
            let kept: Vec<ActionChannel> = self
                .animation
                .action(&name)
                .map(|action| action.channels.iter().filter(|channel| !self.fits_own(object, channel)).cloned().collect())
                .unwrap_or_default();
            if let Some(action) = self.animation.actions.iter_mut().find(|action| action.name == name) {
                action.channels = mine.into_iter().chain(kept).collect();
            }
        }
        self.animation.channels.retain(|channel| !(channel.target == object && in_nla(channel)));
        for channel in tweak.stash {
            self.animation.channels.push(Channel {
                target: object,
                property: channel.property,
                index: channel.index,
                path: channel.path,
                keys: channel.keys,
                driver: None,
                modifiers: channel.modifiers,
            });
        }
        Ok(())
    }

    /// Strips playing an action that is gone go (tweak mode on one ends first); strips follow
    /// a renamed action (`to`).
    pub fn nla_follow_action(&mut self, name: &str, to: Option<&str>) {
        let ids: Vec<Id> = self.objects.iter().map(|object| object.id).collect();
        for id in ids {
            let tweaked_gone = to.is_none() && self.tweaked_strip(id).is_some_and(|strip| strip.action == name);
            if tweaked_gone {
                let _ = self.nla_tweak_exit(id);
            }
            let Ok(object) = self.object_mut(id) else { continue };
            if let (Some(to), Some(tweak)) = (to, object.nla.tweak.as_mut())
                && tweak.action == name
            {
                tweak.action = to.to_string();
            }
            object.nla.follow_action(name, to);
        }
    }

    /// Checks every object's NLA against the scene's actions.
    pub fn validate_nla(&self) -> Result<()> {
        for object in &self.objects {
            object.nla.check(&self.animation.actions).map_err(|reason| SceneError::InvalidFile(format!("{}: {reason}", object.name)))?;
            if let Some(tweak) = &object.nla.tweak {
                let mut seen = std::collections::BTreeSet::new();
                if tweak.stash.iter().any(|channel| {
                    channel.index >= channel.property.components()
                        || channel.property.named() == channel.path.is_empty()
                        || !seen.insert((channel.property, channel.index, channel.path.clone()))
                        || channel.keys.iter().any(|key| !(key.frame.is_finite() && key.value.is_finite()))
                        || channel.modifiers.iter().any(|modifier| modifier.check().is_err())
                }) {
                    return Err(SceneError::InvalidFile(format!("{}: the keys put aside for tweak mode are out of range", object.name)));
                }
            }
        }
        Ok(())
    }
}

/// The most frames a bake samples.
pub const MAX_BAKE_FRAMES: i64 = 100_000;

impl Scene {
    /// The scene with its layered animation baked to plain keys on every frame, as exporters
    /// write it: each object's NLA and F-curve modifiers, over the timeline and every keyed or
    /// played frame. `None` when there is nothing to bake.
    pub fn baked_animation(&self) -> Result<Option<Scene>> {
        let layered: Vec<Id> = self.objects.iter().filter(|object| !object.nla.is_empty()).map(|object| object.id).collect();
        let modified = self.animation.channels.iter().any(|channel| channel.driver.is_none() && !channel.modifiers.is_empty());
        if layered.is_empty() && !modified {
            return Ok(None);
        }
        let frames = |first: f32, last: f32| -> Result<std::ops::RangeInclusive<i64>> {
            let (first, last) = (first.floor() as i64, last.ceil() as i64);
            if last - first > MAX_BAKE_FRAMES {
                return Err(SceneError::params("the animation is too long to bake"));
            }
            Ok(first..=last)
        };
        let constant = |property: Property| if property == Property::Hidden { crate::Interpolation::Constant } else { crate::Interpolation::Linear };
        let mut out = self.clone();
        let timeline = self.animation.timeline;
        for channel in out.animation.channels.iter_mut() {
            if channel.driver.is_some() || channel.modifiers.is_empty() || (layered.contains(&channel.target) && in_nla(channel)) {
                continue;
            }
            let (Some(first), Some(last)) = (channel.keys.first(), channel.keys.last()) else { continue };
            let range = frames(first.frame.min(timeline.start as f32), last.frame.max(timeline.end as f32))?;
            let keys = range
                .filter_map(|frame| {
                    let value = channel.evaluate(frame as f32)?;
                    value.is_finite().then_some(crate::Keyframe { frame: frame as f32, value, interpolation: constant(channel.property) })
                })
                .collect();
            channel.keys = keys;
            channel.modifiers.clear();
        }
        for id in layered {
            let own: Vec<&Channel> = self.animation.channels.iter().filter(|channel| channel.target == id && in_nla(channel)).collect();
            let nla = &self.object(id)?.nla;
            let spans = nla
                .tracks
                .iter()
                .flat_map(|track| track.strips.iter().map(|strip| (strip.start, strip.end())))
                .chain(own.iter().filter_map(|channel| Some((channel.keys.first()?.frame, channel.keys.last()?.frame))))
                .chain(
                    nla.tweak
                        .iter()
                        .flat_map(|tweak| tweak.stash.iter().filter_map(|channel| Some((channel.keys.first()?.frame, channel.keys.last()?.frame)))),
                )
                .chain(std::iter::once((timeline.start as f32, timeline.end as f32)));
            let Some((first, last)) = spans.reduce(|a, b| (a.0.min(b.0), a.1.max(b.1))) else { continue };
            let mut baked: Vec<Channel> = Vec::new();
            for frame in frames(first, last)? {
                for (property, index, path, value) in nla.evaluate(&own, &self.animation.actions, frame as f32) {
                    if !value.is_finite() || self.driver_of(id, property, index, &path).is_some() {
                        continue;
                    }
                    let key = crate::Keyframe { frame: frame as f32, value, interpolation: constant(property) };
                    match baked.iter_mut().find(|channel| channel.property == property && channel.index == index && channel.path == path) {
                        Some(channel) => channel.keys.push(key),
                        None => baked.push(Channel { target: id, property, index, path, keys: vec![key], driver: None, modifiers: Vec::new() }),
                    }
                }
            }
            out.animation.channels.retain(|channel| !(channel.target == id && in_nla(channel)));
            out.animation.channels.extend(baked);
            out.object_mut(id)?.nla = Nla::default();
        }
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Interpolation, Keyframe};

    fn action(name: &str, keys: &[(f32, f32)]) -> Action {
        Action {
            name: name.into(),
            channels: vec![ActionChannel {
                property: Property::Location,
                index: 0,
                path: String::new(),
                keys: keys.iter().map(|(frame, value)| Keyframe { frame: *frame, value: *value, interpolation: Interpolation::Linear }).collect(),
                modifiers: Vec::new(),
            }],
        }
    }

    #[test]
    fn a_strip_maps_scene_frames_to_its_action() {
        let mut strip = NlaStrip::new("A", &action("A", &[(1.0, 0.0), (11.0, 10.0)]), 5.0);
        strip.repeat = 2.0;
        strip.scale = 1.5;
        assert_eq!(strip.end(), 35.0);
        assert_eq!(strip.action_time(5.0), 1.0);
        assert_eq!(strip.action_time(20.0), 1.0);
        assert_eq!(strip.action_time(35.0), 11.0);
        strip.reversed = true;
        assert_eq!(strip.action_time(5.0), 11.0);
        assert_eq!(strip.scene_time(strip.unmapped(12.0)), 12.0);
    }

    #[test]
    fn auto_blends_follow_the_neighbouring_tracks() {
        let a = action("A", &[(0.0, 0.0), (10.0, 0.0)]);
        let track = |start: f32| NlaTrack {
            name: "T".into(),
            muted: false,
            solo: false,
            strips: vec![NlaStrip { auto_blend: true, ..NlaStrip::new("A", &a, start) }],
        };
        let mut nla = Nla { tracks: vec![track(10.0), track(15.0)], ..Nla::default() };
        nla.settle();
        assert_eq!((nla.tracks[0].strips[0].blend_in, nla.tracks[0].strips[0].blend_out), (0.0, 5.0));
        assert_eq!((nla.tracks[1].strips[0].blend_in, nla.tracks[1].strips[0].blend_out), (5.0, 0.0));
    }

    #[test]
    fn a_bake_gives_the_same_frames_without_layers() {
        let mut scene = Scene::starter();
        let cube = scene.object_named("Cube").unwrap().id;
        scene.animation.actions.push(action("A", &[(1.0, 0.0), (11.0, 10.0)]));
        let (track, _) = scene.nla_add_strip(cube, None, "A", 5.0).unwrap();
        let nla = &mut scene.object_mut(cube).unwrap().nla;
        nla.tracks[track].strips[0].scale = 1.5;
        nla.tracks[track].strips[0].repeat = 2.0;
        scene.insert_key(cube, Property::Scale, 3.0).unwrap();
        scene.animation.channels[0].modifiers.push(crate::fmodifier::FModifier::new(crate::fmodifier::FModifierKind::new("noise").unwrap()));
        let baked = scene.baked_animation().unwrap().unwrap();
        assert!(baked.object(cube).unwrap().nla.is_empty());
        assert!(baked.animation.channels.iter().all(|channel| channel.modifiers.is_empty()));
        baked.validate_animation().unwrap();
        for frame in [1, 5, 12, 20, 35, 40, 250] {
            let (a, b) = (scene.at_frame(frame as f32), baked.at_frame(frame as f32));
            assert_eq!(a.object(cube).unwrap().location, b.object(cube).unwrap().location, "frame {frame}");
            assert_eq!(a.object(cube).unwrap().scale, b.object(cube).unwrap().scale, "frame {frame}");
        }
    }

    #[test]
    fn hostile_numbers_never_panic() {
        let a = action("A", &[(0.0, 0.0), (10.0, 1.0)]);
        let mut strip = NlaStrip::new("A", &a, 0.0);
        for (scale, repeat) in [(0.0, 1.0), (f32::NAN, 1.0), (1.0, f32::INFINITY), (1e-30, 1e30)] {
            strip.scale = scale;
            strip.repeat = repeat;
            let nla = Nla { tracks: vec![NlaTrack { name: "T".into(), muted: false, solo: false, strips: vec![strip.clone()] }], ..Nla::default() };
            for frame in [f32::NAN, -1e30, 0.0, 5.0, 1e30] {
                let _ = nla.evaluate(&[], std::slice::from_ref(&a), frame);
            }
            assert!(nla.check(std::slice::from_ref(&a)).is_err() || scale == 1.0);
        }
    }
}
