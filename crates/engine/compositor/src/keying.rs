//! Keying: mattes from a key colour, a second picture, or brightness (Luminance Key, Difference
//! Key, Distance Key, Color Key, Chroma Key) and the Keying node's screen matte with its clean
//! up and despill. Each node gives its matte and the picture times the matte. The formulas were
//! found by fitting the reference's output on fixed pictures and match it to float rounding,
//! except the Keying node's feather, which matches it in shape.

use crate::{Image, LUMINANCE, Rgba};

/// The matte between `minimum` and `maximum` brightness.
pub fn luminance(pixel: Rgba, minimum: f32, maximum: f32) -> f32 {
    let luma = pixel[0] * LUMINANCE[0] + pixel[1] * LUMINANCE[1] + pixel[2] * LUMINANCE[2];
    let span = maximum - minimum;
    let value = if span.abs() > 1e-9 {
        (luma - minimum) / span
    } else if luma >= maximum {
        1.0
    } else {
        0.0
    };
    value.clamp(0.0, 1.0)
}

/// 0 within `tolerance` of the key, 1 past `tolerance + falloff`, and a straight ramp between;
/// never more than the pixel's alpha.
fn ramp(distance: f32, tolerance: f32, falloff: f32, alpha: f32) -> f32 {
    let value = if distance <= tolerance {
        0.0
    } else if distance <= tolerance + falloff && falloff > 0.0 {
        (distance - tolerance) / falloff
    } else {
        1.0
    };
    value.min(alpha)
}

/// The matte of how far `pixel` is from `other` (the mean difference of the colour channels).
pub fn difference(pixel: Rgba, other: Rgba, tolerance: f32, falloff: f32) -> f32 {
    let distance = ((pixel[0] - other[0]).abs() + (pixel[1] - other[1]).abs() + (pixel[2] - other[2]).abs()) / 3.0;
    ramp(distance, tolerance, falloff, pixel[3])
}

/// Studio-range BT.709 Y′CbCr (the rounded matrix), the Y offset left out.
fn ycc(pixel: Rgba) -> [f32; 3] {
    let [r, g, b] = [pixel[0], pixel[1], pixel[2]];
    [0.183 * r + 0.614 * g + 0.062 * b, -0.101 * r - 0.338 * g + 0.439 * b, 0.439 * r - 0.399 * g - 0.040 * b]
}

/// The Y′CbCr colour back to RGB.
fn rgb_of_ycc([y, cb, cr]: [f32; 3]) -> [f32; 3] {
    // The inverse of `ycc`'s matrix.
    const INVERSE: [[f32; 3]; 3] = invert([[0.183, 0.614, 0.062], [-0.101, -0.338, 0.439], [0.439, -0.399, -0.040]]);
    let row = |r: [f32; 3]| r[0] * y + r[1] * cb + r[2] * cr;
    [row(INVERSE[0]), row(INVERSE[1]), row(INVERSE[2])]
}

const fn invert(m: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let [[a, b, c], [d, e, f], [g, h, i]] = m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    [
        [(e * i - f * h) / det, (c * h - b * i) / det, (b * f - c * e) / det],
        [(f * g - d * i) / det, (a * i - c * g) / det, (c * d - a * f) / det],
        [(d * h - e * g) / det, (b * g - a * h) / det, (a * e - b * d) / det],
    ]
}

/// The matte of how far `pixel` is from `key`: straight-line distance in RGB, or in Y′CbCr.
pub fn distance(pixel: Rgba, key: Rgba, ycc_space: bool, tolerance: f32, falloff: f32) -> f32 {
    let (a, b) = if ycc_space { (ycc(pixel), ycc(key)) } else { ([pixel[0], pixel[1], pixel[2]], [key[0], key[1], key[2]]) };
    let distance = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
    ramp(distance, tolerance, falloff, pixel[3])
}

/// 0 where `pixel`'s hue is within half `hue` of the key's (round the circle) and its saturation
/// and value within `saturation` and `value`; the pixel's alpha elsewhere.
pub fn color(pixel: Rgba, key: Rgba, hue: f32, saturation: f32, value: f32) -> f32 {
    let [h, s, v] = crate::color::rgb_to_hsv([pixel[0], pixel[1], pixel[2]]);
    let [kh, ks, kv] = crate::color::rgb_to_hsv([key[0], key[1], key[2]]);
    let turn = (h - kh).abs();
    let turn = turn.min(1.0 - turn);
    if turn < hue * 0.5 && (s - ks).abs() < saturation && (v - kv).abs() < value { 0.0 } else { pixel[3] }
}

/// Chroma Key: in the Cb–Cr plane turned so the key lies along x (offset by the half a step the
/// reference's 8-bit chroma centre adds), pixels inside the cone of half-angle `maximum / 2`
/// fade by twice their distance into it over `falloff`, and pixels within `minimum / 2` are keyed
/// out; never more than the pixel's alpha.
pub fn chroma(pixel: Rgba, key: Rgba, minimum: f32, maximum: f32, falloff: f32) -> f32 {
    const CENTRE: f32 = 128.0 / 255.0 - 0.5;
    let chroma = |pixel: Rgba| {
        let [_, cb, cr] = ycc(pixel);
        (cb + CENTRE, cr + CENTRE)
    };
    let (kb, kr) = chroma(key);
    let (sin, cos) = kr.atan2(kb).sin_cos();
    let (cb, cr) = chroma(pixel);
    let x = cb * cos + cr * sin;
    let z = cr * cos - cb * sin;
    let tangent = (maximum * 0.5).tan();
    let inside = if tangent.abs() > 1e-9 { x - z.abs() / tangent } else { -1.0 };
    let value = if inside > 0.0 {
        if z.abs() < x * (minimum * 0.5).tan() {
            0.0
        } else if falloff > 0.0 {
            (1.0 - 2.0 * inside / falloff).clamp(0.0, 1.0)
        } else {
            0.0
        }
    } else {
        1.0
    };
    value.min(pixel[3])
}

/// The picture times a matte (its alpha too).
pub fn keyed(pixel: Rgba, matte: f32) -> Rgba {
    pixel.map(|value| value * matte)
}

/// How a feather falls off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Falloff {
    Smooth,
    Sphere,
    Root,
    InverseSquare,
    Sharp,
    Linear,
}

impl Falloff {
    pub fn parse(text: &str) -> Self {
        match text {
            "sphere" => Self::Sphere,
            "root" => Self::Root,
            "inverseSquare" => Self::InverseSquare,
            "sharp" => Self::Sharp,
            "linear" => Self::Linear,
            _ => Self::Smooth,
        }
    }

    /// The weight at `t`, 1 at the source and 0 at the reach.
    fn at(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Smooth => t * t * (3.0 - 2.0 * t),
            Self::Sphere => (2.0 * t - t * t).max(0.0).sqrt(),
            Self::Root => t.sqrt(),
            Self::InverseSquare => t * (2.0 - t),
            Self::Sharp => t * t,
            Self::Linear => t,
        }
    }
}

/// The Keying node's settings.
#[derive(Debug, Clone, Copy)]
pub struct Keying {
    pub key: Rgba,
    pub pre_blur: i32,
    pub balance: f32,
    pub black: f32,
    pub white: f32,
    pub edge_size: i32,
    pub edge_tolerance: f32,
    pub post_blur: i32,
    pub dilate: i32,
    pub feather: i32,
    pub falloff: Falloff,
    pub despill: f32,
    pub despill_balance: f32,
}

/// The largest radius the Keying node's clean up reaches (pixels).
pub const MOST: i32 = 256;

/// The screen colour's channel and the other two, in order.
fn channels(key: Rgba) -> (usize, usize, usize) {
    if key[0] >= key[1] && key[0] >= key[2] {
        (0, 1, 2)
    } else if key[1] >= key[2] {
        (1, 0, 2)
    } else {
        (2, 0, 1)
    }
}

/// How much `pixel` is of the screen's primary channel over the other two (weighted by
/// `balance`), scaled by how little of the others there is.
fn screen_share(pixel: Rgba, (primary, first, second): (usize, usize, usize), balance: f32) -> f32 {
    let get = |index: usize| pixel.get(index).copied().unwrap_or(0.0);
    let mix = balance * get(first) + (1.0 - balance) * get(second);
    (get(primary) - mix) * (1.0 - mix)
}

/// A box blur `size` pixels across (the edge pixels repeating) of one value per pixel.
fn box_blur(values: &[f32], width: usize, height: usize, size: i32) -> Vec<f32> {
    let radius = (size.clamp(0, MOST) / 2) as isize;
    if radius == 0 {
        return values.to_vec();
    }
    let count = (2 * radius + 1) as f32;
    let at = |x: isize, y: isize, data: &[f32]| {
        let x = x.clamp(0, width as isize - 1) as usize;
        let y = y.clamp(0, height as isize - 1) as usize;
        data.get(y * width + x).copied().unwrap_or(0.0)
    };
    let mut across = vec![0.0; values.len()];
    for y in 0..height as isize {
        for x in 0..width as isize {
            let sum: f32 = (-radius..=radius).map(|dx| at(x + dx, y, values)).sum();
            if let Some(slot) = across.get_mut(y as usize * width + x as usize) {
                *slot = sum / count;
            }
        }
    }
    let mut out = vec![0.0; values.len()];
    for y in 0..height as isize {
        for x in 0..width as isize {
            let sum: f32 = (-radius..=radius).map(|dy| at(x, y + dy, &across)).sum();
            if let Some(slot) = out.get_mut(y as usize * width + x as usize) {
                *slot = sum / count;
            }
        }
    }
    out
}

/// The largest (or smallest) value within a disc of `radius` pixels (inside the picture).
fn morph(values: &[f32], width: usize, height: usize, radius: i32, grow: bool) -> Vec<f32> {
    let radius = radius.clamp(0, MOST) as isize;
    let mut out = values.to_vec();
    for y in 0..height as isize {
        for x in 0..width as isize {
            let mut best = if grow { f32::NEG_INFINITY } else { f32::INFINITY };
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    let (sx, sy) = (x + dx, y + dy);
                    if dx * dx + dy * dy > radius * radius || sx < 0 || sy < 0 || sx >= width as isize || sy >= height as isize {
                        continue;
                    }
                    let value = values.get(sy as usize * width + sx as usize).copied().unwrap_or(0.0);
                    best = if grow { best.max(value) } else { best.min(value) };
                }
            }
            if let Some(slot) = out.get_mut(y as usize * width + x as usize) {
                *slot = best;
            }
        }
    }
    out
}

/// Spreads the matte out (or in, for a negative size) by `size` pixels with a falloff: each
/// pixel takes the most of its neighbours' values faded by their distance.
fn feather(values: &[f32], width: usize, height: usize, size: i32, falloff: Falloff) -> Vec<f32> {
    let grow = size > 0;
    let radius = size.unsigned_abs().min(MOST as u32) as isize;
    let reach = radius as f32 + 1.0;
    let mut out = values.to_vec();
    for y in 0..height as isize {
        for x in 0..width as isize {
            let mut best = if grow { f32::NEG_INFINITY } else { f32::INFINITY };
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    let (sx, sy) = (x + dx, y + dy);
                    if sx < 0 || sy < 0 || sx >= width as isize || sy >= height as isize {
                        continue;
                    }
                    let distance = ((dx * dx + dy * dy) as f32).sqrt();
                    if distance > radius as f32 {
                        continue;
                    }
                    let weight = falloff.at(1.0 - distance / reach);
                    let value = values.get(sy as usize * width + sx as usize).copied().unwrap_or(0.0);
                    best = if grow { best.max(value * weight) } else { best.min(1.0 - (1.0 - value) * weight) };
                }
            }
            if let Some(slot) = out.get_mut(y as usize * width + x as usize) {
                *slot = best;
            }
        }
    }
    out
}

/// Which pixels lie on the matte's edges: more than a tenth of the square `size` pixels around
/// each differ from it by more than the tolerance (the edge pixels repeating past the picture).
fn edges(matte: &[f32], width: usize, height: usize, size: i32, tolerance: f32) -> Vec<bool> {
    let radius = size.clamp(0, MOST) as isize;
    let window = ((2 * radius + 1) * (2 * radius + 1)) as f32;
    let at = |x: isize, y: isize| {
        let x = x.clamp(0, width as isize - 1) as usize;
        let y = y.clamp(0, height as isize - 1) as usize;
        matte.get(y * width + x).copied().unwrap_or(0.0)
    };
    let mut out = Vec::with_capacity(matte.len());
    for y in 0..height as isize {
        for x in 0..width as isize {
            let here = at(x, y);
            let mut count = 0usize;
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    if (at(x + dx, y + dy) - here).abs() > tolerance {
                        count += 1;
                    }
                }
            }
            out.push(count as f32 > 0.1 * window);
        }
    }
    out
}

/// The Keying node: its picture (despilled, times the matte), its matte, and its edges.
/// `garbage` and `core` are per-pixel mattes (what is surely background, surely foreground).
pub fn keying(image: &Image, garbage: &[f32], core: &[f32], settings: &Keying) -> (Image, Vec<f32>, Vec<f32>) {
    let (width, height) = (image.width, image.height);
    let order = channels(settings.key);
    // Blurring the chroma first, in Y′CbCr.
    let source: Vec<Rgba> = if settings.pre_blur > 0 {
        let ycc: Vec<[f32; 3]> = image.pixels.iter().map(|pixel| ycc(*pixel)).collect();
        let cb = box_blur(&ycc.iter().map(|value| value[1]).collect::<Vec<_>>(), width, height, settings.pre_blur);
        let cr = box_blur(&ycc.iter().map(|value| value[2]).collect::<Vec<_>>(), width, height, settings.pre_blur);
        ycc.iter()
            .zip(cb.iter().zip(&cr))
            .zip(&image.pixels)
            .map(|((value, (cb, cr)), pixel)| {
                let [r, g, b] = rgb_of_ycc([value[0], *cb, *cr]);
                [r, g, b, pixel[3]]
            })
            .collect()
    } else {
        image.pixels.clone()
    };
    let screen = screen_share(settings.key, order, settings.balance);
    let mut matte: Vec<f32> = source
        .iter()
        .map(|pixel| {
            let share = screen_share(*pixel, order, settings.balance);
            if screen.abs() > 1e-9 { (1.0 - share / screen).clamp(0.0, 1.0) } else { 1.0 }
        })
        .collect();
    let on_edge = edges(&matte, width, height, settings.edge_size, settings.edge_tolerance);
    let span = settings.white - settings.black;
    for ((value, edge), (garbage, core)) in matte.iter_mut().zip(&on_edge).zip(garbage.iter().zip(core)) {
        if !edge {
            *value = if span.abs() > 1e-9 {
                ((*value - settings.black) / span).clamp(0.0, 1.0)
            } else if *value >= settings.white {
                1.0
            } else {
                0.0
            };
        }
        *value = value.min(1.0 - garbage.clamp(0.0, 1.0));
        *value = value.max(*core);
    }
    if settings.post_blur > 0 {
        matte = box_blur(&matte, width, height, settings.post_blur);
    }
    if settings.dilate != 0 {
        matte = morph(&matte, width, height, settings.dilate.abs(), settings.dilate > 0);
    }
    if settings.feather != 0 {
        matte = feather(&matte, width, height, settings.feather, settings.falloff);
    }
    let (primary, first, second) = order;
    let pixels = image
        .pixels
        .iter()
        .zip(&matte)
        .map(|(pixel, matte)| {
            let mut out = *pixel;
            let get = |index: usize| pixel.get(index).copied().unwrap_or(0.0);
            let spill = get(primary) - (settings.despill_balance * get(first) + (1.0 - settings.despill_balance) * get(second));
            if spill > 0.0
                && let Some(slot) = out.get_mut(primary)
            {
                *slot -= settings.despill * spill;
            }
            keyed(out, *matte)
        })
        .collect();
    let edges = on_edge.iter().map(|edge| if *edge { 1.0 } else { 0.0 }).collect();
    (Image { width, height, pixels }, matte, edges)
}
