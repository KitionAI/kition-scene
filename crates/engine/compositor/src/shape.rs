//! Masks and distortions by position: Ellipse Mask and Lens Distortion. Positions are fractions
//! of the picture from its bottom left corner, as the reference has them.

use std::sync::Arc;

use crate::{Image, Plane};

/// How Ellipse Mask combines its ellipse with the incoming mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Add,
    Subtract,
    Multiply,
    Not,
}

impl Operation {
    pub fn parse(text: &str) -> Self {
        match text {
            "subtract" => Self::Subtract,
            "multiply" => Self::Multiply,
            "not" => Self::Not,
            _ => Self::Add,
        }
    }
}

/// An Ellipse Mask's ellipse.
#[derive(Debug, Clone, Copy)]
pub struct Ellipse {
    /// The centre, as fractions of the width and height from the bottom left.
    pub position: [f32; 3],
    /// The full width and height, as fractions of the picture's width.
    pub size: [f32; 3],
    /// Radians, anticlockwise.
    pub rotation: f32,
    pub operation: Operation,
}

/// The mask: `value` inside the ellipse combined with `mask` by the operation. Pixels are placed
/// with the first at 0 and the last at 1 across each side, heights in units of the width.
pub fn ellipse_mask(ellipse: &Ellipse, mask: &Plane, value: &Plane, width: usize, height: usize) -> Plane {
    let (sin, cos) = ellipse.rotation.sin_cos();
    let half = [(ellipse.size[0] * 0.5).abs().max(1e-6), (ellipse.size[1] * 0.5).abs().max(1e-6)];
    let aspect = height as f32 / width as f32;
    let across = |index: usize, count: usize| if count > 1 { index as f32 / (count - 1) as f32 } else { 0.5 };
    let mut out = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let dx = across(x, width) - ellipse.position[0];
            let dy = (across(height - 1 - y, height) - ellipse.position[1]) * aspect;
            let u = dx * cos + dy * sin;
            let v = -dx * sin + dy * cos;
            let inside = (u / half[0]).powi(2) + (v / half[1]).powi(2) <= 1.0;
            let index = y * width + x;
            let (mask, value) = (mask.get(index)[0], value.get(index)[0]);
            let result = match (ellipse.operation, inside) {
                (Operation::Add, true) => (mask + value).min(1.0).max(mask),
                (Operation::Subtract, true) => (mask - value).max(0.0),
                (Operation::Multiply, true) => mask * value,
                (Operation::Multiply, false) => 0.0,
                (Operation::Not, true) => {
                    if mask > 0.0 {
                        0.0
                    } else {
                        value
                    }
                }
                (_, false) => mask,
            };
            out.push([result, result, result, 1.0]);
        }
    }
    Plane::Pixels(Arc::new(out))
}

/// A Lens Distortion node's settings.
#[derive(Debug, Clone, Copy)]
pub struct Lens {
    pub horizontal: bool,
    pub distortion: f32,
    pub dispersion: f32,
    pub fit: bool,
    pub jitter: bool,
}

/// How far Horizontal dispersion moves red one way and blue the other, in pixels per unit.
const HORIZONTAL_SHIFT: f32 = 5.0;

/// Bends the picture about its centre (barrel for positive distortion; Fit scales a barrel to
/// keep the corners), splitting the colours apart by the dispersion. A pixel at `p` (each axis
/// from −1 to 1 about the centre) samples `p · 2s / (1 + √(1 − 4k s² |p|²))` with
/// `s = 1 / (1 + k)` (`1 / (1 + 2k)` fitted), which matches the reference to a few thousandths
/// of a pixel; dispersion gives red and blue their own `k`, which matches it only roughly.
/// Pixels from outside the picture are clear.
pub fn lens_distortion(image: &Image, lens: &Lens) -> Image {
    let (width, height) = (image.width as f32, image.height as f32);
    let centre = [(width - 1.0) * 0.5, (height - 1.0) * 0.5];
    let half = [width * 0.5, height * 0.5];
    let k = lens.distortion.clamp(-0.999, 1.0);
    let spread = lens.dispersion.clamp(0.0, 1.0) / 8.0;
    let ks = [k + spread, k, k - spread];
    let scale = 1.0 / (1.0 + if lens.fit && k > 0.0 { 2.0 * k } else { k } + 2.0 * spread);
    let mut pixels = Vec::with_capacity(image.pixels.len());
    for y in 0..image.height {
        for x in 0..image.width {
            let mut out = [0.0f32; 4];
            let mut alpha = 0.0;
            for (channel, (slot, channel_k)) in out.iter_mut().zip(ks).enumerate() {
                let (sx, sy) = if lens.horizontal {
                    let shift = HORIZONTAL_SHIFT * lens.dispersion * (1.0 - channel as f32);
                    (x as f32 + shift, y as f32)
                } else {
                    let u = (x as f32 - centre[0]) / half[0];
                    let v = (y as f32 - centre[1]) / half[1];
                    let t = 1.0 - 4.0 * channel_k * scale * scale * (u * u + v * v);
                    if t < 0.0 {
                        continue;
                    }
                    let factor = 2.0 * scale / (1.0 + t.sqrt());
                    (u * factor * half[0] + centre[0], v * factor * half[1] + centre[1])
                };
                let sample = image.sample(sx, sy, true);
                *slot = sample.get(channel).copied().unwrap_or(0.0);
                alpha += sample[3] / 3.0;
            }
            out[3] = alpha;
            pixels.push(out);
        }
    }
    Image { width: image.width, height: image.height, pixels }
}
