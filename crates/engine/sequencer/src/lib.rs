//! The Scene kit's video sequencer: a frame of the scene's strips (`kition_scene_model::sequencer`)
//! stacked from the lowest channel up. Each strip's picture is made (by the caller for scene and
//! image strips, here for colours, texts, adjustments, and transitions), cropped, placed in the
//! frame, its colours adjusted, and laid over what is below with its blend mode and opacity.
//! Strips a transition takes as inputs show only through it while it runs. Pixels are straight
//! display values from 0 to 1 (the reference's sequencer works in sRGB).
#![forbid(unsafe_code)]

pub mod blend;
pub mod place;
pub mod text;
pub mod transition;

use std::collections::BTreeSet;

use kition_scene_model::Id;
use kition_scene_model::sequencer::{Sequence, Strip, StripKind};

/// A picture: rows from the top, straight RGBA display values.
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<[f32; 4]>,
}

impl Picture {
    /// A transparent picture.
    pub fn empty(width: usize, height: usize) -> Self {
        Self { width, height, pixels: vec![[0.0; 4]; width * height] }
    }

    /// A picture of one colour.
    pub fn filled(width: usize, height: usize, colour: [f32; 4]) -> Self {
        Self { width, height, pixels: vec![colour; width * height] }
    }

    pub fn get(&self, x: usize, row: usize) -> Option<[f32; 4]> {
        if x >= self.width {
            return None;
        }
        self.pixels.get(row * self.width + x).copied()
    }

    pub fn set(&mut self, x: usize, row: usize, value: [f32; 4]) {
        if x < self.width
            && let Some(slot) = self.pixels.get_mut(row * self.width + x)
        {
            *slot = value;
        }
    }

    /// Rounded to 8-bit straight RGBA.
    pub fn to_rgba8(&self) -> Vec<u8> {
        self.pixels.iter().flat_map(|pixel| pixel.map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8)).collect()
    }
}

/// Makes the pictures of scene and image strips: the strip and the frame of the sequence
/// (for a scene strip, the scene's frame is `timeline start + frame − strip start`). `None`
/// draws nothing.
pub trait Sources {
    fn picture(&mut self, strip: &Strip, frame: i32) -> Option<Picture>;
}

impl<F: FnMut(&Strip, i32) -> Option<Picture>> Sources for F {
    fn picture(&mut self, strip: &Strip, frame: i32) -> Option<Picture> {
        self(strip, frame)
    }
}

/// How far into its frames a transition is at `frame`: 0 on its first, approaching 1 on its last.
pub fn factor(sequence: &Sequence, strip: &Strip, frame: i32) -> f32 {
    let (left, right) = sequence.range(strip);
    if right <= left { 0.0 } else { (frame - left) as f32 / (right - left) as f32 }
}

/// The strips that draw on `frame`, lowest channel first, without the inputs of a transition
/// that runs then.
pub fn stack(sequence: &Sequence, frame: i32) -> Vec<&Strip> {
    let shown: Vec<&Strip> = sequence.strips.iter().filter(|strip| strip.draws() && !strip.mute && sequence.shows(strip, frame)).collect();
    let inputs: BTreeSet<Id> = shown.iter().filter_map(|strip| strip.kind.inputs()).flatten().collect();
    let mut stack: Vec<&Strip> = shown.into_iter().filter(|strip| !inputs.contains(&strip.id)).collect();
    stack.sort_by_key(|strip| strip.channel);
    stack
}

/// A strip's picture before its blend: made, cropped, placed, its colours adjusted.
fn strip_picture(sequence: &Sequence, strip: &Strip, frame: i32, below: &Picture, sources: &mut dyn Sources, depth: u32) -> Picture {
    let (width, height) = (below.width, below.height);
    let made = match &strip.kind {
        StripKind::Color { color } => Some(Picture::filled(width, height, [color[0], color[1], color[2], 1.0])),
        StripKind::Text(text) => Some(text::draw(text, width, height)),
        StripKind::Adjustment => Some(below.clone()),
        StripKind::Sound { .. } => None,
        StripKind::Scene { .. } | StripKind::Image { .. } => sources.picture(strip, frame),
        StripKind::Transition { transition, inputs } => {
            let input = |id: Id, sources: &mut dyn Sources| match sequence.strip(id) {
                Some(input) if depth < 8 => strip_picture(sequence, input, frame, below, sources, depth + 1),
                _ => Picture::empty(width, height),
            };
            let a = input(inputs[0], sources);
            let b = input(inputs[1], sources);
            Some(transition::apply(transition, &a, &b, factor(sequence, strip, frame)))
        }
    };
    let Some(made) = made else { return Picture::empty(width, height) };
    let mut placed = place::place(&made, &strip.crop, &strip.transform, width, height);
    place::adjust(&mut placed, strip.saturation, strip.multiply);
    placed
}

/// The sequence's frame `frame`, `width` × `height`.
pub fn render(sequence: &Sequence, frame: i32, width: usize, height: usize, sources: &mut dyn Sources) -> Picture {
    let mut out = Picture::empty(width, height);
    for strip in stack(sequence, frame) {
        let picture = strip_picture(sequence, strip, frame, &out, sources, 0);
        for (below, above) in out.pixels.iter_mut().zip(&picture.pixels) {
            *below = blend::pixel(strip.blend, *below, *above, strip.opacity);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use kition_scene_model::sequencer::Transition;

    fn color(id: Id, channel: u32, start: i32, length: i32, rgb: [f32; 3]) -> Strip {
        Strip::new(id, format!("Color {id}"), channel, start, length, StripKind::Color { color: rgb })
    }

    fn none(_: &Strip, _: i32) -> Option<Picture> {
        None
    }

    #[test]
    fn a_cross_runs_over_its_inputs_overlap() {
        let mut sequence = Sequence { strips: vec![color(1, 1, 1, 20, [1.0, 0.2, 0.0]), color(2, 2, 5, 20, [0.0, 0.5, 1.0])] };
        sequence.strips.push(Strip::new(3, "Cross", 3, 0, 1, StripKind::Transition { transition: Transition::Cross, inputs: [1, 2] }));
        // Before it: the first alone; on its first frame, still the first; then a sixteenth in.
        let at = |frame: i32| render(&sequence, frame, 4, 4, &mut none).pixels[0];
        assert_eq!(at(4), [1.0, 0.2, 0.0, 1.0]);
        assert_eq!(at(5), [1.0, 0.2, 0.0, 1.0]);
        let sixth = at(6);
        assert!((sixth[0] - 15.0 / 16.0).abs() < 1e-5, "{sixth:?}");
        assert_eq!(at(21), [0.0, 0.5, 1.0, 1.0]);
    }

    #[test]
    fn muted_strips_and_sounds_draw_nothing() {
        let mut sequence = Sequence { strips: vec![color(1, 1, 1, 10, [1.0, 0.0, 0.0])] };
        sequence.strips.push(Strip::new(2, "Sound", 2, 1, 10, StripKind::Sound { volume: 1.0, pan: 0.0 }));
        assert_eq!(render(&sequence, 3, 4, 4, &mut none).pixels[0], [1.0, 0.0, 0.0, 1.0]);
        sequence.strips[0].mute = true;
        assert_eq!(render(&sequence, 3, 4, 4, &mut none).pixels[0], [0.0; 4]);
    }

    #[test]
    fn an_adjustment_strip_reworks_the_channels_below() {
        let mut sequence = Sequence { strips: vec![color(1, 1, 1, 10, [0.8, 0.3, 0.1])] };
        let mut adjustment = Strip::new(2, "Adjustment", 2, 1, 10, StripKind::Adjustment);
        adjustment.multiply = 0.5;
        sequence.strips.push(adjustment);
        let pixel = render(&sequence, 3, 4, 4, &mut none).pixels[0];
        assert!((pixel[0] - 0.4).abs() < 1e-5 && (pixel[3] - 1.0).abs() < 1e-5, "{pixel:?}");
    }
}
