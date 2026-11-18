//! The video sequencer: strips on numbered channels along the scene's timeline, each showing
//! something for a span of frames (the scene through its camera, an image, a colour, a text, the
//! channels below it again, a sound's place) or a transition between two others. A frame of the
//! sequence stacks the strips under it from the lowest channel up, each blended over what is
//! below with its blend mode and opacity (the pixels are display values, as the reference
//! application's sequencer works in sRGB); `kition-scene-sequencer` draws it.
//!
//! A strip's content starts at `start` and lasts `length` frames; `trim_start` and `trim_end`
//! hide frames at either end, so it shows from [`Strip::left`] up to (not including)
//! [`Strip::right`]. A transition shows where its two inputs overlap, on a channel above both.

use serde::{Deserialize, Serialize};

use crate::Id;
use crate::error::{Result, SceneError};

/// The highest channel.
pub const MAX_CHANNEL: u32 = 128;
/// The longest a strip may be, in frames.
pub const MAX_LENGTH: i32 = 1_000_000;
/// The furthest a strip may start from frame 0, either way.
pub const MAX_FRAME: i32 = 1_000_000;

/// How a strip is laid over the channels below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Blend {
    Replace,
    Cross,
    Darken,
    Multiply,
    ColorBurn,
    LinearBurn,
    Lighten,
    Screen,
    ColorDodge,
    Add,
    Overlay,
    SoftLight,
    HardLight,
    VividLight,
    LinearLight,
    PinLight,
    Difference,
    Exclusion,
    Subtract,
    Hue,
    Saturation,
    Color,
    Value,
    #[default]
    AlphaOver,
    AlphaUnder,
    GammaCross,
}

impl Blend {
    pub const ALL: [Blend; 26] = [
        Self::Replace,
        Self::Cross,
        Self::Darken,
        Self::Multiply,
        Self::ColorBurn,
        Self::LinearBurn,
        Self::Lighten,
        Self::Screen,
        Self::ColorDodge,
        Self::Add,
        Self::Overlay,
        Self::SoftLight,
        Self::HardLight,
        Self::VividLight,
        Self::LinearLight,
        Self::PinLight,
        Self::Difference,
        Self::Exclusion,
        Self::Subtract,
        Self::Hue,
        Self::Saturation,
        Self::Color,
        Self::Value,
        Self::AlphaOver,
        Self::AlphaUnder,
        Self::GammaCross,
    ];

    /// The name commands and files use.
    pub fn id(self) -> &'static str {
        match self {
            Self::Replace => "replace",
            Self::Cross => "cross",
            Self::Darken => "darken",
            Self::Multiply => "multiply",
            Self::ColorBurn => "colorBurn",
            Self::LinearBurn => "linearBurn",
            Self::Lighten => "lighten",
            Self::Screen => "screen",
            Self::ColorDodge => "colorDodge",
            Self::Add => "add",
            Self::Overlay => "overlay",
            Self::SoftLight => "softLight",
            Self::HardLight => "hardLight",
            Self::VividLight => "vividLight",
            Self::LinearLight => "linearLight",
            Self::PinLight => "pinLight",
            Self::Difference => "difference",
            Self::Exclusion => "exclusion",
            Self::Subtract => "subtract",
            Self::Hue => "hue",
            Self::Saturation => "saturation",
            Self::Color => "color",
            Self::Value => "value",
            Self::AlphaOver => "alphaOver",
            Self::AlphaUnder => "alphaUnder",
            Self::GammaCross => "gammaCross",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|blend| blend.id() == text)
    }
}

/// How a strip's picture is sampled when it is moved by part of a pixel, scaled, or turned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Filter {
    /// Nearest for whole-pixel moves, bilinear otherwise.
    #[default]
    Auto,
    Nearest,
    Bilinear,
}

/// Where a strip's picture sits in the frame: moved by `offset` pixels (x right, y up), scaled
/// and turned (degrees, counter-clockwise) about `origin` (a point of the picture, 0 to 1 from
/// its bottom left), flipped before all that.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StripTransform {
    pub offset: [f32; 2],
    pub scale: [f32; 2],
    pub rotation: f32,
    pub origin: [f32; 2],
    pub filter: Filter,
    pub flip_x: bool,
    pub flip_y: bool,
}

impl Default for StripTransform {
    fn default() -> Self {
        Self { offset: [0.0, 0.0], scale: [1.0, 1.0], rotation: 0.0, origin: [0.5, 0.5], filter: Filter::Auto, flip_x: false, flip_y: false }
    }
}

impl StripTransform {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Pixels cut away from each edge of a strip's picture (they turn transparent; the rest stays
/// where it was).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Crop {
    pub left: u32,
    pub right: u32,
    pub top: u32,
    pub bottom: u32,
}

impl Crop {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// How a wipe uncovers its second input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum WipeShape {
    /// An edge sweeps across the frame.
    #[default]
    Single,
    /// A band through the middle widens.
    Double,
    /// A circle grows from the middle.
    Iris,
    /// A hand sweeps round from twelve o'clock.
    Clock,
}

/// Which way a transition runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum WipeDirection {
    #[default]
    Out,
    In,
}

/// What a transition does with its two inputs.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum Transition {
    /// The first fades into the second.
    Cross,
    /// The same, mixed on square roots (brighter in the middle).
    GammaCross,
    /// The second uncovers the first in a shape; `angle` (degrees) turns single and double
    /// wipes, and `blur` (0 to 1) softens the edge.
    Wipe {
        #[serde(default)]
        shape: WipeShape,
        #[serde(default)]
        direction: WipeDirection,
        #[serde(default)]
        angle: f32,
        #[serde(default)]
        blur: f32,
    },
}

impl Transition {
    pub fn id(&self) -> &'static str {
        match self {
            Self::Cross => "cross",
            Self::GammaCross => "gammaCross",
            Self::Wipe { .. } => "wipe",
        }
    }
}

/// Where a text strip's box sits against its location.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Anchor {
    Start,
    #[default]
    Center,
    End,
}

/// A text strip's words and look.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StripText {
    pub text: String,
    /// Letter height in pixels (the font's whole box).
    pub size: f32,
    /// Straight colour and alpha (display values).
    pub color: [f32; 4],
    /// Where the text is, 0 to 1 across and up the frame.
    pub location: [f32; 2],
    /// Which part of the text's box sits on the location, across and up.
    pub anchor: [Anchor; 2],
    /// How lines line up with one another.
    pub align: Anchor,
    /// A box behind the text, with its colour and the margin around the letters (pixels).
    pub boxed: bool,
    pub box_color: [f32; 4],
    pub box_margin: f32,
    /// A shadow under the letters, with its colour and how far it falls (pixels, right and down).
    pub shadow: bool,
    pub shadow_color: [f32; 4],
    pub shadow_offset: [f32; 2],
}

impl Default for StripText {
    fn default() -> Self {
        Self {
            text: "Text".into(),
            size: 60.0,
            color: [1.0, 1.0, 1.0, 1.0],
            location: [0.5, 0.5],
            anchor: [Anchor::Center, Anchor::Center],
            align: Anchor::Center,
            boxed: false,
            box_color: [0.2, 0.2, 0.2, 1.0],
            box_margin: 10.0,
            shadow: false,
            shadow_color: [0.0, 0.0, 0.0, 1.0],
            shadow_offset: [4.0, 4.0],
        }
    }
}

/// What a strip shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum StripKind {
    /// The scene through a camera (the scene's own when `camera` is none), its frames from the
    /// timeline's start on.
    Scene {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        camera: Option<Id>,
    },
    /// A still image of the scene's (by hash); nothing while it has none.
    Image {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        image: Option<String>,
    },
    /// A solid colour (display values).
    Color {
        color: [f32; 3],
    },
    Text(StripText),
    /// The channels below again, with this strip's crop, transform, and colour.
    Adjustment,
    /// A sound's place on the timeline (its volume and pan are kept; it draws nothing).
    Sound {
        #[serde(default = "one")]
        volume: f32,
        #[serde(default)]
        pan: f32,
    },
    /// A transition from `inputs[0]` to `inputs[1]` where they overlap.
    Transition {
        #[serde(flatten)]
        transition: Transition,
        inputs: [Id; 2],
    },
}

fn one() -> f32 {
    1.0
}

impl StripKind {
    /// The name commands and files use.
    pub fn id(&self) -> &'static str {
        match self {
            Self::Scene { .. } => "scene",
            Self::Image { .. } => "image",
            Self::Color { .. } => "color",
            Self::Text(_) => "text",
            Self::Adjustment => "adjustment",
            Self::Sound { .. } => "sound",
            Self::Transition { transition, .. } => transition.id(),
        }
    }

    /// The blend mode a new strip of this kind gets (the reference's defaults).
    pub fn default_blend(&self) -> Blend {
        match self {
            Self::Adjustment => Blend::Cross,
            _ => Blend::AlphaOver,
        }
    }

    pub fn inputs(&self) -> Option<[Id; 2]> {
        match self {
            Self::Transition { inputs, .. } => Some(*inputs),
            _ => None,
        }
    }
}

/// One strip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Strip {
    pub id: Id,
    pub name: String,
    pub channel: u32,
    /// The frame the content starts on.
    pub start: i32,
    /// The content's length in frames.
    pub length: i32,
    #[serde(default)]
    pub trim_start: i32,
    #[serde(default)]
    pub trim_end: i32,
    #[serde(default)]
    pub mute: bool,
    #[serde(default)]
    pub lock: bool,
    #[serde(default)]
    pub blend: Blend,
    #[serde(default = "one")]
    pub opacity: f32,
    #[serde(default, skip_serializing_if = "StripTransform::is_default")]
    pub transform: StripTransform,
    #[serde(default, skip_serializing_if = "Crop::is_default")]
    pub crop: Crop,
    /// Scales the colours' saturation (1 leaves them).
    #[serde(default = "one")]
    pub saturation: f32,
    /// Scales the colours (1 leaves them).
    #[serde(default = "one")]
    pub multiply: f32,
    pub kind: StripKind,
}

impl Strip {
    pub fn new(id: Id, name: impl Into<String>, channel: u32, start: i32, length: i32, kind: StripKind) -> Self {
        Self {
            id,
            name: name.into(),
            channel,
            start,
            length,
            trim_start: 0,
            trim_end: 0,
            mute: false,
            lock: false,
            blend: kind.default_blend(),
            opacity: 1.0,
            transform: StripTransform::default(),
            crop: Crop::default(),
            saturation: 1.0,
            multiply: 1.0,
            kind,
        }
    }

    /// The first frame it shows on (for a transition, see [`Sequence::range`]).
    pub fn left(&self) -> i32 {
        self.start.saturating_add(self.trim_start)
    }

    /// The frame after the last it shows on.
    pub fn right(&self) -> i32 {
        self.start.saturating_add(self.length).saturating_sub(self.trim_end)
    }

    pub fn is_transition(&self) -> bool {
        matches!(self.kind, StripKind::Transition { .. })
    }

    /// Whether it draws a picture (sounds do not).
    pub fn draws(&self) -> bool {
        !matches!(self.kind, StripKind::Sound { .. })
    }

    fn check(&self) -> Result<()> {
        let bad = |what: &str| Err(SceneError::InvalidFile(format!("strip {}: {what}", self.name)));
        if !(1..=MAX_CHANNEL).contains(&self.channel) {
            return bad("its channel is out of range");
        }
        if !(1..=MAX_LENGTH).contains(&self.length) || self.start.abs() > MAX_FRAME {
            return bad("its frames are out of range");
        }
        if self.trim_start < 0 || self.trim_end < 0 || self.trim_start.saturating_add(self.trim_end) >= self.length {
            return bad("its trims leave nothing");
        }
        let finite = |values: &[f32]| values.iter().all(|value| value.is_finite());
        let transform = &self.transform;
        if !finite(&[self.opacity, self.saturation, self.multiply, transform.rotation])
            || !finite(&transform.offset)
            || !finite(&transform.scale)
            || !finite(&transform.origin)
        {
            return bad("a setting is not a number");
        }
        match &self.kind {
            StripKind::Color { color } if !finite(color) => bad("its colour is not a number"),
            StripKind::Text(text) if !(finite(&text.color) && finite(&text.location) && text.size.is_finite() && text.size > 0.0) => {
                bad("its text settings are not numbers")
            }
            StripKind::Transition { inputs, transition } => {
                if inputs[0] == inputs[1] || inputs.contains(&self.id) {
                    return bad("its inputs must be two other strips");
                }
                if let Transition::Wipe { angle, blur, .. } = transition
                    && !(angle.is_finite() && blur.is_finite())
                {
                    return bad("its wipe settings are not numbers");
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// The scene's strips.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Sequence {
    pub strips: Vec<Strip>,
}

impl Sequence {
    pub fn is_empty(&self) -> bool {
        self.strips.is_empty()
    }

    pub fn strip(&self, id: Id) -> Option<&Strip> {
        self.strips.iter().find(|strip| strip.id == id)
    }

    pub fn strip_mut(&mut self, id: Id) -> Option<&mut Strip> {
        self.strips.iter_mut().find(|strip| strip.id == id)
    }

    /// The frames a strip shows on, `[left, right)`: a transition's where its inputs overlap
    /// (empty when they do not).
    pub fn range(&self, strip: &Strip) -> (i32, i32) {
        match strip.kind.inputs() {
            Some([a, b]) => match (self.strip(a), self.strip(b)) {
                (Some(a), Some(b)) => {
                    let (left, right) = (a.left().max(b.left()), a.right().min(b.right()));
                    (left, right.max(left))
                }
                _ => (strip.left(), strip.left()),
            },
            None => (strip.left(), strip.right()),
        }
    }

    /// Whether `strip` shows on `frame`.
    pub fn shows(&self, strip: &Strip, frame: i32) -> bool {
        let (left, right) = self.range(strip);
        (left..right).contains(&frame)
    }

    /// The first and last-but-one frames any strip shows on.
    pub fn bounds(&self) -> Option<(i32, i32)> {
        let ranges = self.strips.iter().map(|strip| self.range(strip)).filter(|(left, right)| right > left);
        ranges.fold(None, |acc, (left, right)| Some(acc.map_or((left, right), |(l, r): (i32, i32)| (l.min(left), r.max(right)))))
    }

    /// Whether a strip on `channel` showing `[left, right)` would overlap another (`except`).
    pub fn overlaps(&self, channel: u32, left: i32, right: i32, except: &[Id]) -> bool {
        self.strips.iter().filter(|other| other.channel == channel && !except.contains(&other.id)).any(|other| {
            let (l, r) = self.range(other);
            l < right && left < r
        })
    }

    /// The lowest channel from `channel` up where `[left, right)` is free.
    pub fn free_channel(&self, channel: u32, left: i32, right: i32, except: &[Id]) -> Option<u32> {
        (channel.max(1)..=MAX_CHANNEL).find(|channel| !self.overlaps(*channel, left, right, except))
    }

    /// The transitions that take `id` as an input.
    pub fn transitions_of(&self, id: Id) -> Vec<Id> {
        self.strips.iter().filter(|strip| strip.kind.inputs().is_some_and(|inputs| inputs.contains(&id))).map(|strip| strip.id).collect()
    }

    /// Strip names in use.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.strips.iter().map(|strip| strip.name.as_str())
    }

    /// Checks each strip, unique ids, inputs that exist and sit below their transition, and no
    /// two strips overlapping on a channel.
    pub fn validate(&self) -> Result<()> {
        let mut ids = std::collections::BTreeSet::new();
        for strip in &self.strips {
            strip.check()?;
            if !ids.insert(strip.id) {
                return Err(SceneError::InvalidFile(format!("strip id {} is repeated", strip.id)));
            }
        }
        for strip in &self.strips {
            if let Some(inputs) = strip.kind.inputs() {
                for input in inputs {
                    let other = self.strip(input).ok_or_else(|| SceneError::InvalidFile(format!("{} names a missing input", strip.name)))?;
                    if other.is_transition() || !other.draws() || other.channel >= strip.channel {
                        return Err(SceneError::InvalidFile(format!("{}'s inputs must be pictures on lower channels", strip.name)));
                    }
                }
            }
        }
        for strip in &self.strips {
            let (left, right) = self.range(strip);
            if right > left && self.overlaps(strip.channel, left, right, &[strip.id]) {
                return Err(SceneError::InvalidFile(format!("{} overlaps another strip on channel {}", strip.name, strip.channel)));
            }
        }
        Ok(())
    }

    /// Ids its strips use.
    pub fn ids(&self) -> impl Iterator<Item = Id> + '_ {
        self.strips.iter().map(|strip| strip.id)
    }

    /// The images its strips show.
    pub fn images(&self) -> impl Iterator<Item = &String> {
        self.strips.iter().filter_map(|strip| match &strip.kind {
            StripKind::Image { image } => image.as_ref(),
            _ => None,
        })
    }

    /// Lets go of what is gone: transitions whose inputs are, cameras and images no longer in
    /// the scene.
    pub fn forget(&mut self, objects: &dyn Fn(Id) -> bool, images: &dyn Fn(&str) -> bool) {
        loop {
            let ids: Vec<Id> = self.strips.iter().map(|strip| strip.id).collect();
            let before = self.strips.len();
            self.strips.retain(|strip| strip.kind.inputs().is_none_or(|inputs| inputs.iter().all(|input| ids.contains(input))));
            if self.strips.len() == before {
                break;
            }
        }
        for strip in &mut self.strips {
            match &mut strip.kind {
                StripKind::Scene { camera } => *camera = camera.filter(|id| objects(*id)),
                StripKind::Image { image } => *image = image.take().filter(|hash| images(hash)),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn color(id: Id, channel: u32, start: i32, length: i32) -> Strip {
        Strip::new(id, format!("Color {id}"), channel, start, length, StripKind::Color { color: [1.0, 0.0, 0.0] })
    }

    #[test]
    fn a_transition_spans_where_its_inputs_overlap() {
        let mut sequence = Sequence { strips: vec![color(1, 1, 1, 20), color(2, 2, 5, 20)] };
        sequence.strips.push(Strip::new(3, "Cross", 3, 0, 1, StripKind::Transition { transition: Transition::Cross, inputs: [1, 2] }));
        assert_eq!(sequence.range(&sequence.strips[2]), (5, 21));
        assert!(sequence.validate().is_ok());
        sequence.strips[0].trim_end = 6;
        assert_eq!(sequence.range(&sequence.strips[2]), (5, 15));
    }

    #[test]
    fn overlapping_strips_on_a_channel_are_refused() {
        let sequence = Sequence { strips: vec![color(1, 1, 1, 20), color(2, 1, 10, 20)] };
        assert!(sequence.validate().is_err());
        assert_eq!(sequence.free_channel(1, 10, 30, &[2]), Some(2));
    }

    #[test]
    fn forgetting_an_input_drops_its_transition() {
        let mut sequence = Sequence { strips: vec![color(1, 1, 1, 20), color(2, 2, 5, 20)] };
        sequence.strips.push(Strip::new(3, "Cross", 3, 0, 1, StripKind::Transition { transition: Transition::Cross, inputs: [1, 2] }));
        sequence.strips.remove(0);
        sequence.forget(&|_| true, &|_| true);
        assert_eq!(sequence.strips.len(), 1);
    }

    #[test]
    fn strips_read_back_as_written() {
        let mut strip = color(4, 2, -3, 9);
        strip.blend = Blend::Screen;
        strip.transform.rotation = 15.0;
        strip.crop.left = 3;
        let text = serde_json::to_string(&strip).unwrap_or_default();
        let back: Strip = serde_json::from_str(&text).unwrap_or_else(|_| color(0, 1, 0, 1));
        assert_eq!(back, strip);
        for blend in Blend::ALL {
            assert_eq!(Blend::parse(blend.id()), Some(blend));
            assert_eq!(serde_json::to_value(blend).ok(), Some(serde_json::Value::String(blend.id().into())));
        }
    }
}
