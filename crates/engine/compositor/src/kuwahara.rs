//! Kuwahara: paints the picture with flat strokes by taking, for each pixel, the mean of the
//! calmest region around it.
//!
//! Classic: the four square quadrants of `size + 1` pixels that meet at the pixel (Kuwahara et
//! al., 1976); the one whose colour channels vary least gives its mean. It matches the
//! reference to float rounding, except where quadrants vary exactly as much (flat ramps), where
//! either may be taken.
//!
//! Anisotropic: Kyprianidis, Kang, and Döllner's filter ("Image and Video Abstraction by
//! Anisotropic Kuwahara Filtering", 2009) with the polynomial sector weights of Kyprianidis's
//! 2011 follow-up: the local orientation from the smoothed structure tensor stretches the
//! filter into an ellipse along edges, eight overlapping sectors give their means, and each is
//! weighted by how calm it is. It matches the reference in shape only.

use crate::filter::{self, Filter};
use crate::{Image, Rgba};

/// The largest radius (pixels).
pub const MOST: f32 = 128.0;

/// The pixel at (`x`, `y`) with the edge pixels repeating.
fn at(image: &Image, x: isize, y: isize) -> Rgba {
    image.at(x, y)
}

/// The classic filter with quadrants `size` pixels past the centre.
pub fn classic(image: &Image, size: f32) -> Image {
    let radius = size.clamp(0.0, MOST).round() as isize;
    if radius == 0 {
        return image.clone();
    }
    let mut pixels = Vec::with_capacity(image.pixels.len());
    for y in 0..image.height as isize {
        for x in 0..image.width as isize {
            let mut best: Option<(f32, Rgba)> = None;
            for (sx, sy) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] {
                // Only the quadrant's pixels inside the picture count.
                let mut sum = [0.0f32; 4];
                let mut count = 0.0f32;
                for j in 0..=radius {
                    for i in 0..=radius {
                        if let Some(pixel) = image.get(x + sx * i, y + sy * j) {
                            for (total, value) in sum.iter_mut().zip(pixel) {
                                *total += value;
                            }
                            count += 1.0;
                        }
                    }
                }
                let mean = sum.map(|value| value / count.max(1.0));
                let mut variance = 0.0f32;
                for j in 0..=radius {
                    for i in 0..=radius {
                        if let Some(pixel) = image.get(x + sx * i, y + sy * j) {
                            variance += pixel.iter().zip(mean).take(3).map(|(value, mean)| (value - mean) * (value - mean)).sum::<f32>();
                        }
                    }
                }
                let variance = variance / count.max(1.0);
                if best.is_none_or(|(least, _)| variance < least) {
                    best = Some((variance, mean));
                }
            }
            pixels.push(best.map_or([0.0; 4], |(_, mean)| mean));
        }
    }
    Image { width: image.width, height: image.height, pixels }
}

/// The anisotropic filter's settings.
#[derive(Debug, Clone, Copy)]
pub struct Anisotropic {
    /// The filter's radius in pixels.
    pub size: f32,
    /// How far the structure tensor is smoothed (pixels).
    pub uniformity: f32,
    /// How strongly calm sectors win (0 to 1).
    pub sharpness: f32,
    /// How far the filter stretches along edges (0 to 1).
    pub eccentricity: f32,
}

/// The number of sectors.
const SECTORS: usize = 8;

/// The structure tensor (E, F, G) per pixel: the colour gradients' outer products summed over
/// the channels (Sobel derivatives), smoothed by a Gaussian of `smoothing` pixels.
fn structure(image: &Image, smoothing: f32) -> Vec<[f32; 3]> {
    let (width, height) = (image.width, image.height);
    let mut tensor = Image::filled(width, height, [0.0; 4]);
    for y in 0..height as isize {
        for x in 0..width as isize {
            let p = |dx: isize, dy: isize| at(image, x + dx, y + dy);
            let mut e = 0.0;
            let mut f = 0.0;
            let mut g = 0.0;
            for channel in 0..3 {
                let c = |dx: isize, dy: isize| p(dx, dy).get(channel).copied().unwrap_or(0.0);
                let gx = (c(1, -1) + 2.0 * c(1, 0) + c(1, 1) - c(-1, -1) - 2.0 * c(-1, 0) - c(-1, 1)) / 4.0;
                let gy = (c(-1, 1) + 2.0 * c(0, 1) + c(1, 1) - c(-1, -1) - 2.0 * c(0, -1) - c(1, -1)) / 4.0;
                e += gx * gx;
                f += gx * gy;
                g += gy * gy;
            }
            if let Some(slot) = tensor.pixels.get_mut(y as usize * width + x as usize) {
                *slot = [e, f, g, 0.0];
            }
        }
    }
    let size = (smoothing * 3.0).clamp(0.0, MOST);
    let smoothed = filter::blur(&tensor, Filter::Gaussian, size, size);
    smoothed.pixels.iter().map(|pixel| [pixel[0], pixel[1], pixel[2]]).collect()
}

/// The anisotropic filter.
pub fn anisotropic(image: &Image, settings: &Anisotropic) -> Image {
    let radius = settings.size.clamp(0.0, MOST);
    if radius < 1.0 {
        return image.clone();
    }
    let tensors = structure(image, settings.uniformity.max(0.0));
    // The ellipse's stretch: α = 1 / eccentricity (more eccentric, more stretched).
    let alpha = 1.0 / settings.eccentricity.clamp(0.01, 1.0);
    // How strongly the calmer sectors win.
    let q = 1.0 + 15.0 * settings.sharpness.clamp(0.0, 1.0);
    let reach = radius.ceil() as isize;
    let zeta = 2.0 / radius;
    let eta = (zeta + (std::f32::consts::PI / SECTORS as f32).cos()) / (std::f32::consts::PI / SECTORS as f32).sin().powi(2);
    let mut pixels = Vec::with_capacity(image.pixels.len());
    for y in 0..image.height as isize {
        for x in 0..image.width as isize {
            let [e, f, g] = tensors.get(y as usize * image.width + x as usize).copied().unwrap_or([0.0; 3]);
            let root = ((e - g) * (e - g) + 4.0 * f * f).sqrt();
            let (l1, l2) = ((e + g + root) * 0.5, (e + g - root) * 0.5);
            let anisotropy = if l1 + l2 > 1e-9 { (l1 - l2) / (l1 + l2) } else { 0.0 };
            // The direction along the edge.
            let (tx, ty) = (l1 - e, -f);
            let length = tx.hypot(ty);
            let (cos, sin) = if length > 1e-9 { (tx / length, ty / length) } else { (1.0, 0.0) };
            let a = radius * ((alpha + anisotropy) / alpha).clamp(0.1, 2.0);
            let b = radius * (alpha / (alpha + anisotropy)).clamp(0.1, 2.0);
            let mut means = [[0.0f32; 4]; SECTORS];
            let mut squares = [[0.0f32; 3]; SECTORS];
            let mut weights = [0.0f32; SECTORS];
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    // Into the ellipse's frame, scaled to the unit disc.
                    let (fx, fy) = (dx as f32, dy as f32);
                    let u = (cos * fx + sin * fy) / a;
                    let v = (-sin * fx + cos * fy) / b;
                    let distance = u * u + v * v;
                    if distance > 1.0 {
                        continue;
                    }
                    let pixel = at(image, x + dx, y + dy);
                    let fall = (-3.125 * distance).exp();
                    for (k, ((mean, square), weight)) in means.iter_mut().zip(squares.iter_mut()).zip(weights.iter_mut()).enumerate() {
                        // The sector's polynomial weight (rotated to sector k).
                        let angle = std::f32::consts::TAU * k as f32 / SECTORS as f32;
                        let (s, c) = angle.sin_cos();
                        let (pu, pv) = (c * u + s * v, -s * u + c * v);
                        let w = (pu + zeta - eta * pv * pv).max(0.0);
                        let w = w * w * fall;
                        if w <= 0.0 {
                            continue;
                        }
                        for (total, value) in mean.iter_mut().zip(pixel) {
                            *total += w * value;
                        }
                        for (total, value) in square.iter_mut().zip(pixel) {
                            *total += w * value * value;
                        }
                        *weight += w;
                    }
                }
            }
            let mut out = [0.0f32; 4];
            let mut total = 0.0f32;
            for ((mean, square), weight) in means.iter().zip(&squares).zip(&weights) {
                if *weight <= 0.0 {
                    continue;
                }
                let m = mean.map(|value| value / weight);
                let spread: f32 = square.iter().zip(m).map(|(square, m)| (square / weight - m * m).max(0.0)).sum::<f32>().sqrt();
                let w = 1.0 / (1.0 + (255.0 * spread).powf(q));
                for (value, m) in out.iter_mut().zip(m) {
                    *value += w * m;
                }
                total += w;
            }
            pixels.push(if total > 0.0 { out.map(|value| value / total) } else { at(image, x, y) });
        }
    }
    Image { width: image.width, height: image.height, pixels }
}
