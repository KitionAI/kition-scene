//! Glare: picks out the highlights (light above a threshold, with a soft knee), spreads them as
//! bloom, fog glow, streaks, a star, ghosts, or sun beams, and adds the result to the picture.
//! The highlights match the reference exactly (`(m − t + k)² / 4k` inside the knee on the
//! brightest channel `m`, `m − t` above it); how each kind spreads them was measured from the
//! reference's output and is matched in its overall shape and energy, not pixel for pixel.

use crate::convolve::Kernel;
use crate::filter::{self, Filter};
use crate::{Image, Inputs, Rgba};

/// The kinds of glare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlareKind {
    /// The highlights convolved with a picture (`convolve`).
    Kernel,
    Bloom,
    FogGlow,
    Streaks,
    Ghosts,
    SimpleStar,
    SunBeams,
}

/// A Glare node's settings.
#[derive(Debug, Clone, Copy)]
pub struct Settings {
    pub kind: GlareKind,
    /// Pixels per working pixel: 1 at high quality, 2 at medium, 4 at low.
    pub step: usize,
    pub threshold: f32,
    pub smoothness: f32,
    pub clamp: Option<f32>,
    pub strength: f32,
    pub saturation: f32,
    pub tint: Rgba,
    pub size: f32,
    pub streaks: u32,
    pub angle: f32,
    pub iterations: u32,
    pub fade: f32,
    pub modulation: f32,
    pub diagonal: bool,
    pub sun: [f32; 2],
    pub jitter: f32,
}

impl Settings {
    /// The settings a node's inputs give.
    pub fn read(inputs: &mut Inputs<'_, '_>) -> Result<Self, String> {
        let kind = match inputs.setting("Type") {
            "kernel" => GlareKind::Kernel,
            "bloom" => GlareKind::Bloom,
            "fogGlow" => GlareKind::FogGlow,
            "ghosts" => GlareKind::Ghosts,
            "simpleStar" => GlareKind::SimpleStar,
            "sunBeams" => GlareKind::SunBeams,
            _ => GlareKind::Streaks,
        };
        let step = match inputs.setting("Quality") {
            "high" => 1,
            "low" => 4,
            _ => 2,
        };
        let clamp = if inputs.flag("Clamp Highlights")? { Some(inputs.number("Maximum Highlights")?) } else { None };
        let sun = inputs.vector("Sun Position")?;
        Ok(Self {
            kind,
            step,
            threshold: inputs.number("Highlights Threshold")?,
            smoothness: inputs.number("Highlights Smoothness")?.max(0.0),
            clamp,
            strength: inputs.number("Strength")?,
            saturation: inputs.number("Saturation")?,
            tint: inputs.color("Tint")?,
            size: inputs.number("Size")?.clamp(0.0, 1.0),
            streaks: inputs.number("Streaks")?.round().clamp(1.0, 16.0) as u32,
            angle: inputs.number("Streaks Angle")?,
            iterations: inputs.number("Iterations")?.round().clamp(2.0, 5.0) as u32,
            fade: inputs.number("Fade")?.clamp(0.75, 1.0),
            modulation: inputs.number("Color Modulation")?.clamp(0.0, 1.0),
            diagonal: inputs.flag("Diagonal Star")?,
            sun: [sun[0], sun[1]],
            jitter: inputs.number("Jitter")?.clamp(0.0, 1.0),
        })
    }
}

/// The highlights of one pixel.
pub fn highlight(pixel: Rgba, threshold: f32, smoothness: f32, clamp: Option<f32>) -> Rgba {
    let brightest = pixel[0].max(pixel[1]).max(pixel[2]);
    if brightest.is_nan() || brightest <= 0.0 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let knee = smoothness;
    let mut kept = if brightest <= threshold - knee {
        0.0
    } else if brightest < threshold + knee && knee > 0.0 {
        let x = brightest - threshold + knee;
        x * x / (4.0 * knee)
    } else {
        brightest - threshold
    };
    if let Some(most) = clamp {
        kept = kept.min(most.max(0.0));
    }
    let scale = kept / brightest;
    [pixel[0] * scale, pixel[1] * scale, pixel[2] * scale, 1.0]
}

/// The picture with glare, the glare alone, and the highlights it came from; `kernel` is the
/// Kernel glare's kernel.
pub fn glare(image: &Image, settings: &Settings, kernel: &Kernel) -> (Image, Image, Image) {
    let highlights = Image {
        width: image.width,
        height: image.height,
        pixels: image.pixels.iter().map(|pixel| highlight(*pixel, settings.threshold, settings.smoothness, settings.clamp)).collect(),
    };
    // The kernel is the picture's own size: Kernel glare works at full size whatever the
    // quality (the reference makes no glare at all below high quality).
    let step = if settings.kind == GlareKind::Kernel { 1 } else { settings.step };
    let small = shrink(&highlights, step);
    let spread = match settings.kind {
        GlareKind::Kernel => crate::convolve::convolve(&small, kernel),
        GlareKind::Bloom => bloom(&small, settings.size),
        GlareKind::FogGlow => fog_glow(&small, settings.size),
        GlareKind::Streaks => streaks(&small, settings),
        GlareKind::Ghosts => ghosts(&small, settings),
        GlareKind::SimpleStar => star(&small, settings),
        GlareKind::SunBeams => sun_beams(&small, settings),
    };
    let spread = grow(&spread, image.width, image.height);
    let highlights = grow(&shrink(&highlights, step), image.width, image.height);
    let glare = Image { width: spread.width, height: spread.height, pixels: spread.pixels.iter().map(|pixel| finish(*pixel, settings)).collect() };
    let result = Image {
        width: image.width,
        height: image.height,
        pixels: image
            .pixels
            .iter()
            .zip(&glare.pixels)
            .map(|(pixel, glare)| [pixel[0] + glare[0], pixel[1] + glare[1], pixel[2] + glare[2], pixel[3]])
            .collect(),
    };
    (result, glare, highlights)
}

/// Strength, saturation (in HSV, keeping the brightest channel), and tint.
fn finish(pixel: Rgba, settings: &Settings) -> Rgba {
    let [h, s, v] = crate::color::rgb_to_hsv([pixel[0], pixel[1], pixel[2]]);
    let [r, g, b] = crate::color::hsv_to_rgb([h, s * settings.saturation, v]);
    let mut out = [r, g, b, 1.0];
    for (value, tint) in out.iter_mut().zip(settings.tint).take(3) {
        *value = (*value * tint * settings.strength).max(0.0);
    }
    out
}

/// The picture averaged down by `step` on each side (at least one pixel).
fn shrink(image: &Image, step: usize) -> Image {
    if step <= 1 {
        return image.clone();
    }
    let (width, height) = (image.width.div_ceil(step).max(1), image.height.div_ceil(step).max(1));
    let mut pixels = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0.0f32; 4];
            let mut count = 0.0;
            for sy in y * step..((y + 1) * step).min(image.height) {
                for sx in x * step..((x + 1) * step).min(image.width) {
                    let pixel = image.get(sx as isize, sy as isize).unwrap_or([0.0; 4]);
                    for (total, value) in sum.iter_mut().zip(pixel) {
                        *total += value;
                    }
                    count += 1.0;
                }
            }
            pixels.push(sum.map(|value| value / f32::max(count, 1.0)));
        }
    }
    Image { width, height, pixels }
}

/// The picture scaled to `width × height` with bilinear sampling.
fn grow(image: &Image, width: usize, height: usize) -> Image {
    if image.width == width && image.height == height {
        return image.clone();
    }
    let mut pixels = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let u = (x as f32 + 0.5) * image.width as f32 / width as f32 - 0.5;
            let v = (y as f32 + 0.5) * image.height as f32 / height as f32 - 0.5;
            pixels.push(image.sample(u, v, false));
        }
    }
    Image { width, height, pixels }
}

/// Zero outside the picture.
fn sample_zero(image: &Image, x: isize, y: isize) -> Rgba {
    image.get(x, y).unwrap_or([0.0; 4])
}

fn add(into: &mut Image, from: &Image, weight: f32) {
    for (pixel, other) in into.pixels.iter_mut().zip(&from.pixels) {
        for (value, extra) in pixel.iter_mut().zip(other) {
            *value += weight * extra;
        }
    }
}

/// The weights bloom gives the highlights and each halved level after it (fitted to the
/// reference's output; levels past these fade by half each).
const BLOOM_WEIGHTS: [f32; 5] = [0.25, 0.111, 0.177, 0.203, BLOOM_LAST];
const BLOOM_LAST: f32 = 0.072;

/// Bloom: the highlights halved in size again and again; each level grown back a step at a time
/// with a tent blur at every step, and the levels summed with fitted weights. Larger sizes keep
/// more levels (as many as halving the picture's shorter side allows).
fn bloom(image: &Image, size: f32) -> Image {
    let smallest = image.width.min(image.height).max(2) as f32;
    let most = smallest.log2().floor().max(1.0);
    let levels = ((most * (0.5 + size)).round() as usize).clamp(1, most as usize);
    let mut chain = vec![image.clone()];
    while chain.len() < levels {
        let Some(last) = chain.last() else { break };
        if last.width < 2 || last.height < 2 {
            break;
        }
        chain.push(shrink(last, 2));
    }
    let mut out = Image::filled(image.width, image.height, [0.0, 0.0, 0.0, 1.0]);
    for (index, level) in chain.iter().enumerate() {
        let weight = BLOOM_WEIGHTS.get(index).copied().unwrap_or_else(|| BLOOM_LAST * 0.5f32.powi(index as i32 - 4));
        let mut grown = level.clone();
        for larger in chain.iter().take(index).rev() {
            grown = filter::blur(&grow(&grown, larger.width, larger.height), Filter::Tent, 2.0, 2.0);
        }
        add(&mut out, &grown, weight);
    }
    out
}

/// Fog glow: a wide, very peaked glow (almost all the light stays where it was).
fn fog_glow(image: &Image, size: f32) -> Image {
    let radius = ((image.width.max(image.height) as f32) * size * 0.25).round().max(1.0) as isize;
    let mut kernel = Vec::new();
    let mut total = 0.0;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let distance = ((dx * dx + dy * dy) as f32).sqrt();
            if distance > radius as f32 {
                continue;
            }
            let weight = (-10.0 * distance).exp();
            total += weight;
            kernel.push((dx, dy, weight));
        }
    }
    let mut pixels = Vec::with_capacity(image.pixels.len());
    for y in 0..image.height as isize {
        for x in 0..image.width as isize {
            let mut sum = [0.0f32; 3];
            for (dx, dy, weight) in &kernel {
                let pixel = sample_zero(image, x - dx, y - dy);
                for (total, value) in sum.iter_mut().zip(pixel) {
                    *total += weight * value;
                }
            }
            pixels.push([sum[0] / total, sum[1] / total, sum[2] / total, 1.0]);
        }
    }
    Image { width: image.width, height: image.height, pixels }
}

/// Spreads light along a line of pixels: each pass carries `fade` of each pixel's light to the
/// next one way and then back the other (both ways when `both`), keeping the total.
fn sweep(line: &mut [Rgba], fade: f32, passes: u32, both: bool) {
    let keep = if both { ((1.0 - fade) / (1.0 + fade)).max(0.0) } else { 1.0 - fade };
    let carry = |pixel: &mut Rgba, carried: &mut Rgba| {
        for (value, before) in pixel.iter_mut().zip(*carried).take(3) {
            *value += fade * before;
        }
        *carried = *pixel;
    };
    for _ in 0..passes {
        let mut carried = [0.0; 4];
        for pixel in line.iter_mut() {
            carry(pixel, &mut carried);
        }
        if both {
            let mut carried = [0.0; 4];
            for pixel in line.iter_mut().rev() {
                carry(pixel, &mut carried);
            }
        }
        for value in line.iter_mut() {
            for channel in value.iter_mut().take(3) {
                *channel *= keep;
            }
        }
    }
}

/// The pixels of `image` along lines in `direction` (a step of one pixel across or diagonally),
/// swept with [`sweep`] and put back.
fn along(image: &Image, direction: [isize; 2], fade: f32, passes: u32, both: bool) -> Image {
    let mut out = image.clone();
    let (width, height) = (image.width as isize, image.height as isize);
    // Every line starts at a pixel whose step back leaves the picture.
    let mut starts = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let (px, py) = (x - direction[0], y - direction[1]);
            if px < 0 || py < 0 || px >= width || py >= height {
                starts.push((x, y));
            }
        }
    }
    for (x0, y0) in starts {
        let mut slots = Vec::new();
        let (mut x, mut y) = (x0, y0);
        while x >= 0 && y >= 0 && x < width && y < height {
            slots.push(y as usize * image.width + x as usize);
            x += direction[0];
            y += direction[1];
        }
        let mut line: Vec<Rgba> = slots.iter().map(|slot| image.pixels.get(*slot).copied().unwrap_or([0.0; 4])).collect();
        sweep(&mut line, fade, passes, both);
        for (slot, value) in slots.iter().zip(line) {
            if let Some(target) = out.pixels.get_mut(*slot) {
                *target = value;
            }
        }
    }
    out
}

/// How far a star's or streak's light reaches per pixel at a fade setting (fitted: 0.54 at the
/// default 0.9).
fn reach(fade: f32) -> f32 {
    fade.clamp(0.0, 1.0).powf(5.85)
}

/// Streaks: for each of `streaks` directions, passes that add the light 1, 2, and 3 steps
/// back along the direction, the step four times longer each pass and each tap faded by `fade`
/// per pixel, colours shifting along the streak by the colour modulation.
fn streaks(image: &Image, settings: &Settings) -> Image {
    let mut out = Image::filled(image.width, image.height, [0.0, 0.0, 0.0, 1.0]);
    let count = settings.streaks.max(1);
    for index in 0..count {
        let angle = settings.angle + std::f32::consts::TAU * index as f32 / count as f32;
        let direction = [angle.cos(), -angle.sin()];
        let mut current = image.clone();
        let mut step = 1.0f32;
        for pass in 0..settings.iterations {
            let mut total = 1.0f32;
            let taps: Vec<(f32, [f32; 3])> = (1..4)
                .map(|tap| {
                    let weight = settings.fade.powf(tap as f32 * step);
                    let shift = settings.modulation * (tap as f32 / 3.0) * (pass as f32 + 1.0) / settings.iterations as f32;
                    (weight, [1.0, 1.0 - 0.5 * shift, 1.0 - shift])
                })
                .collect();
            for (weight, _) in &taps {
                total += weight;
            }
            let mut pixels = Vec::with_capacity(current.pixels.len());
            for y in 0..image.height {
                for x in 0..image.width {
                    let mut sum = sample_zero(&current, x as isize, y as isize);
                    for (tap, (weight, tint)) in taps.iter().enumerate() {
                        let t = (tap + 1) as f32 * step;
                        let sample = current.sample(x as f32 - direction[0] * t, y as f32 - direction[1] * t, true);
                        for ((value, tint), sample) in sum.iter_mut().zip(tint).zip(sample) {
                            *value += weight * tint * sample;
                        }
                    }
                    pixels.push([sum[0] / total, sum[1] / total, sum[2] / total, 1.0]);
                }
            }
            current = Image { width: image.width, height: image.height, pixels };
            step *= 4.0;
        }
        add(&mut out, &current, STREAK_GAIN * STREAK_GROWTH.powi(settings.iterations as i32));
    }
    out
}

/// Simple star: the highlights spread both ways along four directions (diagonal or along the
/// axes), with a little of them left in place.
fn star(image: &Image, settings: &Settings) -> Image {
    let directions: [[isize; 2]; 2] = if settings.diagonal { [[1, 1], [1, -1]] } else { [[1, 0], [0, 1]] };
    let mut out = Image::filled(image.width, image.height, [0.0, 0.0, 0.0, 1.0]);
    add(&mut out, image, STAR_CENTRE);
    for direction in directions {
        add(&mut out, &along(image, direction, reach(settings.fade), settings.iterations, true), STAR_ARMS);
    }
    out
}

/// Gains fitted to the reference's output (its energy and its arms' brightness).
const STAR_CENTRE: f32 = 0.037;
const GHOST_GAIN: f32 = 0.63;
const SUN_BEAM_GAIN: f32 = 1.06;
const STAR_ARMS: f32 = 0.35;
/// A streak's gain grows with its passes (fitted to two cases: three passes of four streaks and
/// two passes of three).
const STREAK_GAIN: f32 = 0.14;
const STREAK_GROWTH: f32 = 1.39;

/// Ghosts: blurred copies of the highlights mirrored through the centre at several scales.
fn ghosts(image: &Image, settings: &Settings) -> Image {
    let wide = image.width.max(image.height) as f32 / 3.0;
    let blurred = filter::blur(image, Filter::Gaussian, wide, wide);
    let scales = [-1.5f32, -0.6, 0.45, -0.3, 0.7, -2.1];
    let count = (settings.iterations as usize * 2).min(scales.len());
    let (cx, cy) = (image.width as f32 / 2.0, image.height as f32 / 2.0);
    let mut out = Image::filled(image.width, image.height, [0.0, 0.0, 0.0, 1.0]);
    for (index, scale) in scales.iter().take(count).enumerate() {
        let shift = 1.0 - settings.modulation * index as f32 / count as f32;
        let tint = [shift, 1.0, 2.0 - shift];
        for y in 0..image.height {
            for x in 0..image.width {
                let u = cx + (x as f32 + 0.5 - cx) / scale - 0.5;
                let v = cy + (y as f32 + 0.5 - cy) / scale - 0.5;
                let sample = blurred.sample(u, v, true);
                if let Some(pixel) = out.pixels.get_mut(y * image.width + x) {
                    for ((value, sample), tint) in pixel.iter_mut().zip(sample).zip(tint) {
                        *value += sample * tint * GHOST_GAIN / count as f32;
                    }
                }
            }
        }
    }
    out
}

/// Sun beams: the highlights smeared away from the sun position, fading with distance.
fn sun_beams(image: &Image, settings: &Settings) -> Image {
    let sun = [settings.sun[0] * image.width as f32, (1.0 - settings.sun[1]) * image.height as f32];
    let reach = settings.size.max(0.01) * image.width.max(image.height) as f32;
    let mut pixels = Vec::with_capacity(image.pixels.len());
    let steps = 32;
    for y in 0..image.height {
        for x in 0..image.width {
            let position = [x as f32 + 0.5, y as f32 + 0.5];
            let to_sun = [sun[0] - position[0], sun[1] - position[1]];
            let distance = to_sun[0].hypot(to_sun[1]);
            let length = distance.min(reach);
            let mut sum = [0.0f32; 3];
            let mut weight = 0.0f32;
            for step in 0..steps {
                let t = step as f32 / steps as f32;
                let jitter = 1.0 - settings.jitter * 0.5;
                let along = if distance > 0.0 { length * t * jitter / distance } else { 0.0 };
                let sample = image.sample(position[0] + to_sun[0] * along - 0.5, position[1] + to_sun[1] * along - 0.5, true);
                let fall = (1.0 - t) * (1.0 - t);
                for (value, sample) in sum.iter_mut().zip(sample) {
                    *value += sample * fall;
                }
                weight += fall;
            }
            let scale = SUN_BEAM_GAIN / weight.max(1e-6);
            pixels.push([sum[0] * scale, sum[1] * scale, sum[2] * scale, 1.0]);
        }
    }
    Image { width: image.width, height: image.height, pixels }
}
