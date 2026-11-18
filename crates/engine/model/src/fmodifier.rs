//! F-curve modifiers: a channel's curve reshaped by a stack of modifiers, as the reference
//! application's graph editor offers them. Each was measured against the reference by sampling
//! curves frame by frame (`tests/nla_blender.rs`).
//!
//! - **Time modifiers** (Limits, Stepped, and Cycles when it is first in the stack) change the frame the
//!   curve is read at. They run from the last in the stack to the first, each on the frame the
//!   later ones gave; a Cycles modifier anywhere but first does nothing.
//! - **Value modifiers** (Generator, Built-in Function, Envelope, Noise, Limits) then run first to last on
//!   the curve's value, all at the frame the time modifiers gave.
//! - Each modifier can be limited to a frame range, with linear blends in and out, and scaled by
//!   an influence; the frame is checked against the range as the modifier receives it. A time
//!   modifier blends the frame, a value modifier the value.
//! - A Cycles modifier first in the stack, with repeat (or repeat with offset) on both sides, no
//!   count, no range and no influence, makes the curve cyclic: the first and last keys' automatic
//!   handles see the keys across the cycle (shifted by the cycle's change in value).
//!
//! Noise is Kition's own gradient noise: its scale, phase, offset, strength, depth, and blend
//! follow the reference, its exact shape does not.

use serde::{Deserialize, Serialize};

/// Where a modifier works and how strongly.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameRange {
    pub start: f32,
    pub end: f32,
    #[serde(default)]
    pub blend_in: f32,
    #[serde(default)]
    pub blend_out: f32,
}

/// How a Generator's coefficients make its polynomial.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PolynomialMode {
    /// `c0 + c1·x + c2·x² + …`
    #[default]
    Expanded,
    /// `(c0·x + c1)(c2·x + c3)…`
    Factorized,
}

/// The function of a Built-in Function modifier.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FunctionKind {
    #[default]
    Sin,
    Cos,
    Tan,
    Sqrt,
    Ln,
    /// The normalized sinc, `sin(πx) / (πx)`.
    Sinc,
}

/// What a Cycles modifier does before the first key or after the last.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CycleMode {
    /// The curve holds its end value.
    None,
    #[default]
    Repeat,
    /// Repeats, each cycle moved by the change from the first key to the last.
    RepeatOffset,
    /// Repeats, every other cycle backwards.
    Mirror,
}

/// How a Noise modifier mixes its noise into the curve.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NoiseBlend {
    /// Adds noise centred on zero.
    #[default]
    Replace,
    Add,
    Subtract,
    Multiply,
}

/// An envelope's control point: the range the reference range maps to at `frame`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvelopePoint {
    pub frame: f32,
    pub min: f32,
    pub max: f32,
}

/// What a modifier does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum FModifierKind {
    /// A polynomial of the frame, replacing the curve or added to it.
    Generator {
        #[serde(default)]
        mode: PolynomialMode,
        coefficients: Vec<f32>,
        #[serde(default)]
        additive: bool,
    },
    /// `amplitude · f(phaseMultiplier · frame + phaseOffset) + valueOffset`; outside the
    /// function's domain (square root at or below zero, logarithm at or below one, tangent at a
    /// pole) the result is zero (or nothing is added).
    Function {
        #[serde(default)]
        function: FunctionKind,
        amplitude: f32,
        phase_multiplier: f32,
        phase_offset: f32,
        value_offset: f32,
        #[serde(default)]
        additive: bool,
    },
    /// Maps the range `reference + min` to `reference + max` onto each control point's range,
    /// interpolated linearly between points and held beyond them.
    Envelope {
        reference: f32,
        min: f32,
        max: f32,
        #[serde(default)]
        points: Vec<EnvelopePoint>,
    },
    /// Repeats the keyed range before and after it (`count` 0 repeats forever).
    Cycles {
        before: CycleMode,
        after: CycleMode,
        #[serde(default)]
        before_count: u32,
        #[serde(default)]
        after_count: u32,
    },
    /// Noise over the frame: `(frame − offset) / scale` along, `phase` across.
    Noise {
        #[serde(default)]
        blend: NoiseBlend,
        scale: f32,
        strength: f32,
        phase: f32,
        offset: f32,
        depth: u8,
        lacunarity: f32,
        roughness: f32,
    },
    /// Clamps the evaluated frame (X) and value (Y), with each bound independently optional.
    Limits {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_x: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_x: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_y: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_y: Option<f32>,
    },
    /// Holds the curve for `step` frames at a time from `offset`, only from `start` and up to
    /// `end` when they are given.
    Stepped {
        step: f32,
        offset: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<f32>,
    },
}

/// The most coefficients a Generator takes (an order-10 polynomial).
pub const MAX_COEFFICIENTS: usize = 22;
/// The most control points an Envelope takes.
pub const MAX_ENVELOPE_POINTS: usize = 1024;
/// The deepest noise.
pub const MAX_NOISE_DEPTH: u8 = 32;

/// One modifier on a channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FModifier {
    #[serde(flatten)]
    pub kind: FModifierKind,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub muted: bool,
    /// Scales the modifier's effect; full when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub influence: Option<f32>,
    /// Limits the modifier to a frame range; everywhere when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<FrameRange>,
}

impl FModifierKind {
    /// The modifier called `name` with the reference's defaults.
    pub fn new(name: &str) -> Option<Self> {
        Some(match name {
            "generator" => Self::Generator { mode: PolynomialMode::Expanded, coefficients: vec![0.0, 1.0], additive: false },
            "function" => Self::Function {
                function: FunctionKind::Sin,
                amplitude: 1.0,
                phase_multiplier: 1.0,
                phase_offset: 0.0,
                value_offset: 0.0,
                additive: false,
            },
            "envelope" => Self::Envelope { reference: 0.0, min: -1.0, max: 1.0, points: Vec::new() },
            "cycles" => Self::Cycles { before: CycleMode::Repeat, after: CycleMode::Repeat, before_count: 0, after_count: 0 },
            "noise" => Self::Noise {
                blend: NoiseBlend::Replace,
                scale: 1.0,
                strength: 1.0,
                phase: 1.0,
                offset: 0.0,
                depth: 0,
                lacunarity: 2.0,
                roughness: 0.5,
            },
            "limits" => Self::Limits { min_x: None, max_x: None, min_y: None, max_y: None },
            "stepped" => Self::Stepped { step: 2.0, offset: 0.0, start: None, end: None },
            _ => return None,
        })
    }

    /// The name commands use.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Generator { .. } => "generator",
            Self::Function { .. } => "function",
            Self::Envelope { .. } => "envelope",
            Self::Cycles { .. } => "cycles",
            Self::Noise { .. } => "noise",
            Self::Stepped { .. } => "stepped",
            Self::Limits { .. } => "limits",
        }
    }
}

fn finite(values: &[f32]) -> bool {
    values.iter().all(|value| value.is_finite())
}

impl FModifier {
    pub fn new(kind: FModifierKind) -> Self {
        Self { kind, muted: false, influence: None, range: None }
    }

    /// Whether every number is usable; the reason when not.
    pub fn check(&self) -> std::result::Result<(), String> {
        if let Some(influence) = self.influence
            && !(influence.is_finite() && (0.0..=1.0).contains(&influence))
        {
            return Err("an influence is from 0 to 1".into());
        }
        if let Some(range) = self.range
            && !(finite(&[range.start, range.end, range.blend_in, range.blend_out])
                && range.start <= range.end
                && range.blend_in >= 0.0
                && range.blend_out >= 0.0
                && range.blend_in + range.blend_out <= range.end - range.start + 1e-4)
        {
            return Err("a frame range runs from its start to its end, with blends that fit inside it".into());
        }
        match &self.kind {
            FModifierKind::Generator { mode, coefficients, .. } => {
                let fits = match mode {
                    PolynomialMode::Expanded => !coefficients.is_empty(),
                    PolynomialMode::Factorized => !coefficients.is_empty() && coefficients.len() % 2 == 0,
                };
                if !fits || coefficients.len() > MAX_COEFFICIENTS || !finite(coefficients) {
                    return Err(format!("a generator takes 1 to {MAX_COEFFICIENTS} finite coefficients (an even number when factorized)"));
                }
            }
            FModifierKind::Function { amplitude, phase_multiplier, phase_offset, value_offset, .. } => {
                if !finite(&[*amplitude, *phase_multiplier, *phase_offset, *value_offset]) {
                    return Err("a function's numbers must be finite".into());
                }
            }
            FModifierKind::Envelope { reference, min, max, points } => {
                if !finite(&[*reference, *min, *max])
                    || points.len() > MAX_ENVELOPE_POINTS
                    || points.iter().any(|point| !finite(&[point.frame, point.min, point.max]))
                    || points.windows(2).any(|pair| matches!(pair, [a, b] if b.frame <= a.frame))
                {
                    return Err(format!("an envelope's numbers must be finite, with at most {MAX_ENVELOPE_POINTS} points in frame order"));
                }
            }
            FModifierKind::Cycles { before_count, after_count, .. } => {
                if *before_count > 1_000_000 || *after_count > 1_000_000 {
                    return Err("a cycle count is at most 1000000".into());
                }
            }
            FModifierKind::Noise { scale, strength, phase, offset, depth, lacunarity, roughness, .. } => {
                if !finite(&[*scale, *strength, *phase, *offset, *lacunarity, *roughness])
                    || *depth > MAX_NOISE_DEPTH
                    || *lacunarity < 0.0
                    || *roughness < 0.0
                {
                    return Err(format!("noise takes finite numbers, a depth up to {MAX_NOISE_DEPTH}, and a lacunarity and roughness from 0"));
                }
            }
            FModifierKind::Limits { min_x, max_x, min_y, max_y } => {
                if [min_x, max_x, min_y, max_y].into_iter().flatten().any(|value| !value.is_finite()) {
                    return Err("limits must be finite".into());
                }
            }
            FModifierKind::Stepped { step, offset, start, end } => {
                if !(step.is_finite() && *step > 0.0 && offset.is_finite())
                    || start.is_some_and(|f| !f.is_finite())
                    || end.is_some_and(|f| !f.is_finite())
                {
                    return Err("a step is a positive number of frames, with finite offset and limits".into());
                }
            }
        }
        Ok(())
    }

    /// How strongly the modifier works at `frame`: its range's blends times its influence.
    fn weight(&self, frame: f32) -> f32 {
        let mut weight = self.influence.unwrap_or(1.0);
        if let Some(range) = self.range {
            if frame < range.start || frame > range.end {
                return 0.0;
            }
            if range.blend_in > 0.0 && frame < range.start + range.blend_in {
                weight *= (frame - range.start) / range.blend_in;
            }
            if range.blend_out > 0.0 && frame > range.end - range.blend_out {
                weight *= (range.end - frame) / range.blend_out;
            }
        }
        weight
    }
}

/// The span of a curve's keys: first and last frame and value.
#[derive(Debug, Clone, Copy)]
pub struct Span {
    pub first_frame: f32,
    pub last_frame: f32,
    pub first_value: f32,
    pub last_value: f32,
}

impl Span {
    pub fn period(&self) -> f32 {
        self.last_frame - self.first_frame
    }

    pub fn change(&self) -> f32 {
        self.last_value - self.first_value
    }
}

/// Whether the stack makes the curve cyclic (its end keys' handles see across the cycle).
pub fn cyclic(modifiers: &[FModifier]) -> bool {
    matches!(
        modifiers.first(),
        Some(FModifier {
            kind: FModifierKind::Cycles {
                before: CycleMode::Repeat | CycleMode::RepeatOffset,
                after: CycleMode::Repeat | CycleMode::RepeatOffset,
                before_count: 0,
                after_count: 0,
            },
            muted: false,
            influence: None,
            range: None,
        })
    )
}

/// A Cycles modifier's frame and value offset at `frame`.
fn cycle(span: Span, before: CycleMode, after: CycleMode, before_count: u32, after_count: u32, frame: f32) -> (f32, f32) {
    let period = span.period();
    if period.is_nan() || period <= 0.0 || !frame.is_finite() {
        return (frame, 0.0);
    }
    let (mode, count, cycles, mapped) = if frame < span.first_frame {
        let cycles = ((span.first_frame - frame) / period).ceil();
        (before, before_count, -cycles, frame + cycles * period)
    } else if frame > span.last_frame {
        let cycles = ((frame - span.last_frame) / period).ceil();
        (after, after_count, cycles, frame - cycles * period)
    } else {
        return (frame, 0.0);
    };
    if mode == CycleMode::None || (count > 0 && cycles.abs() > count as f32) {
        return (frame, 0.0);
    }
    match mode {
        CycleMode::RepeatOffset => (mapped, cycles * span.change()),
        CycleMode::Mirror if (cycles.abs() as i64) % 2 == 1 => (span.first_frame + span.last_frame - mapped, 0.0),
        _ => (mapped, 0.0),
    }
}

/// The frame the curve is read at, and the value a Cycles offset adds.
pub fn time(modifiers: &[FModifier], span: Option<Span>, frame: f32) -> (f32, f32) {
    let (mut time, mut offset) = (frame, 0.0);
    for (slot, modifier) in modifiers.iter().enumerate().rev() {
        if modifier.muted {
            continue;
        }
        let (mapped, added) = match (&modifier.kind, span) {
            (FModifierKind::Limits { min_x, max_x, .. }, _) => (limit(time, *min_x, *max_x), 0.0),
            (FModifierKind::Stepped { step, offset: from, start, end }, _) => {
                if start.is_some_and(|start| time < start) || end.is_some_and(|end| time > end) || step.is_nan() || *step <= 0.0 {
                    continue;
                }
                (from + ((time - from) / step).trunc() * step, 0.0)
            }
            (FModifierKind::Cycles { before, after, before_count, after_count }, Some(span)) if slot == 0 => {
                cycle(span, *before, *after, *before_count, *after_count, time)
            }
            _ => continue,
        };
        let weight = modifier.weight(time);
        if weight <= 0.0 {
            continue;
        }
        time += (mapped - time) * weight;
        offset += added * weight;
    }
    (time, offset)
}

/// The value modifiers applied to `value` at frame `time`.
pub fn value(modifiers: &[FModifier], time: f32, mut value: f32) -> f32 {
    for modifier in modifiers {
        if modifier.muted {
            continue;
        }
        let Some(changed) = apply(&modifier.kind, time, value) else { continue };
        let weight = modifier.weight(time);
        if weight <= 0.0 {
            continue;
        }
        value += (changed - value) * weight;
    }
    value
}

// Sequential bounds also define inverted bounds without a panicking `clamp`.
fn limit(value: f32, min: Option<f32>, max: Option<f32>) -> f32 {
    let value = min.map_or(value, |min| value.max(min));
    max.map_or(value, |max| value.min(max))
}

fn apply(kind: &FModifierKind, time: f32, value: f32) -> Option<f32> {
    Some(match kind {
        FModifierKind::Limits { min_y, max_y, .. } => limit(value, *min_y, *max_y),
        FModifierKind::Generator { mode, coefficients, additive } => {
            let result = match mode {
                PolynomialMode::Expanded => coefficients.iter().rev().fold(0.0f32, |sum, coefficient| sum * time + coefficient),
                PolynomialMode::Factorized => coefficients
                    .chunks(2)
                    .map(|pair| match pair {
                        [a, b] => a * time + b,
                        _ => 1.0,
                    })
                    .product(),
            };
            if *additive { value + result } else { result }
        }
        FModifierKind::Function { function, amplitude, phase_multiplier, phase_offset, value_offset, additive } => {
            let x = phase_multiplier * time + phase_offset;
            let y = match function {
                FunctionKind::Sin => Some(x.sin()),
                FunctionKind::Cos => Some(x.cos()),
                FunctionKind::Tan => (x.cos().abs() > 1e-6).then(|| x.tan()),
                FunctionKind::Sqrt => (x > 0.0).then(|| x.sqrt()),
                FunctionKind::Ln => (x > 1.0).then(|| x.ln()),
                FunctionKind::Sinc => {
                    let px = std::f32::consts::PI * x;
                    Some(if px.abs() < 1e-6 { 1.0 } else { px.sin() / px })
                }
            };
            match (y, additive) {
                (Some(y), true) => value + amplitude * y + value_offset,
                (Some(y), false) => amplitude * y + value_offset,
                (None, true) => value,
                (None, false) => 0.0,
            }
        }
        FModifierKind::Envelope { reference, min, max, points } => {
            let (Some(first), Some(last)) = (points.first(), points.last()) else { return None };
            let (low, high) = if time <= first.frame {
                (first.min, first.max)
            } else if time >= last.frame {
                (last.min, last.max)
            } else {
                let next = points.iter().position(|point| point.frame > time)?;
                let (Some(a), Some(b)) = (points.get(next.checked_sub(1)?), points.get(next)) else { return None };
                let t = (time - a.frame) / (b.frame - a.frame);
                (a.min + (b.min - a.min) * t, a.max + (b.max - a.max) * t)
            };
            let span = max - min;
            if span.abs() < 1e-12 {
                return None;
            }
            low + (value - (reference + min)) / span * (high - low)
        }
        FModifierKind::Noise { blend, scale, strength, phase, offset, depth, lacunarity, roughness } => {
            let x = if scale.abs() < 1e-9 { 0.0 } else { (time - offset) / scale };
            let noise = turbulence(x, *phase, *depth, *lacunarity, *roughness);
            match blend {
                NoiseBlend::Replace => value + (noise - 0.5) * strength,
                NoiseBlend::Add => value + noise * strength,
                NoiseBlend::Subtract => value - noise * strength,
                NoiseBlend::Multiply => value * noise * strength,
            }
        }
        FModifierKind::Cycles { .. } | FModifierKind::Stepped { .. } => return None,
    })
}

/// A lattice hash (a 32-bit integer mix).
fn hash(x: i32, y: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ 0x2545_f491;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^ (h >> 15)
}

/// Two-dimensional gradient noise, about −0.7 to 0.7, zero on the lattice.
fn gradient(x: f32, y: f32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (ix, iy) = (x0 as i32, y0 as i32);
    let corner = |dx: i32, dy: i32| {
        let angle = (hash(ix.wrapping_add(dx), iy.wrapping_add(dy)) as f32 / u32::MAX as f32) * std::f32::consts::TAU;
        angle.cos() * (fx - dx as f32) + angle.sin() * (fy - dy as f32)
    };
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (u, v) = (fade(fx), fade(fy));
    let bottom = corner(0, 0) + (corner(1, 0) - corner(0, 0)) * u;
    let top = corner(0, 1) + (corner(1, 1) - corner(0, 1)) * u;
    bottom + (top - bottom) * v
}

/// How far noise strays from 0.5: chosen so its standard deviation matches the reference's
/// (0.124 at depth 0, measured over 120 frames).
const SPREAD: f32 = 0.49;

/// Noise from 0 to 1 (about), octaves `depth + 1` deep.
fn turbulence(x: f32, y: f32, depth: u8, lacunarity: f32, roughness: f32) -> f32 {
    if !(x.is_finite() && y.is_finite()) || x.abs() > 1e7 || y.abs() > 1e7 {
        return 0.5;
    }
    let (mut sum, mut total, mut amplitude, mut frequency) = (0.0, 0.0, 1.0f32, 1.0f32);
    for _ in 0..=depth.min(MAX_NOISE_DEPTH) {
        // Off the lattice, where gradient noise is zero.
        sum += gradient(x * frequency + 0.371, y * frequency + 0.629) * amplitude;
        total += amplitude;
        amplitude *= roughness;
        frequency *= lacunarity;
        if amplitude < 1e-6 || !frequency.is_finite() || frequency > 1e6 {
            break;
        }
    }
    0.5 + SPREAD * if total > 0.0 { sum / total } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_stays_near_its_range_and_is_repeatable() {
        let kind = FModifierKind::new("noise").unwrap();
        let values: Vec<f32> = (0..400).map(|frame| value(&[FModifier::new(kind.clone())], frame as f32 * 0.37, 0.0)).collect();
        assert!(values.iter().all(|value| value.abs() < 0.5));
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        assert!(mean.abs() < 0.05, "{mean}");
        assert_eq!(values, (0..400).map(|frame| value(&[FModifier::new(kind.clone())], frame as f32 * 0.37, 0.0)).collect::<Vec<_>>());
    }

    #[test]
    fn hostile_numbers_never_panic() {
        for name in ["generator", "function", "envelope", "cycles", "noise", "stepped"] {
            let modifier = FModifier::new(FModifierKind::new(name).unwrap());
            for frame in [f32::NAN, f32::INFINITY, -1e30, 0.0, 1e30] {
                let span = Span { first_frame: 1.0, last_frame: 1.0, first_value: 0.0, last_value: 0.0 };
                let (time, _) = time(std::slice::from_ref(&modifier), Some(span), frame);
                let _ = value(std::slice::from_ref(&modifier), time, 1.0);
            }
        }
    }
}
