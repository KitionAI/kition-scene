//! Blur: a separable filter as wide as the size in pixels on each axis. The filters' shapes were
//! measured from the reference's Blur node (a spike on a ramp gives each filter's weights):
//! Gaussian with a standard deviation of a third of the size; a box; a tent; and the quadratic
//! and cubic B-splines, Catmull-Rom, and Mitchell–Netravali reaching 1.6 of their own units
//! across the size.

use crate::{Image, Rgba};

/// The Blur node's filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    Flat,
    Tent,
    Quadratic,
    Cubic,
    Gaussian,
    FastGaussian,
    CatmullRom,
    Mitchell,
}

impl Filter {
    pub fn parse(text: &str) -> Self {
        match text {
            "flat" => Self::Flat,
            "tent" => Self::Tent,
            "quadratic" => Self::Quadratic,
            "cubic" => Self::Cubic,
            "fastGaussian" => Self::FastGaussian,
            "catrom" => Self::CatmullRom,
            "mitch" => Self::Mitchell,
            _ => Self::Gaussian,
        }
    }

    /// The filter's weight at `x`, the distance in sizes (0 to 1).
    pub fn weight(self, x: f32) -> f32 {
        let x = x.abs();
        if x > 1.0 {
            return 0.0;
        }
        let t = 1.6 * x;
        match self {
            Self::Flat => 1.0,
            Self::Tent => 1.0 - x,
            Self::Gaussian | Self::FastGaussian => (-4.5 * x * x).exp(),
            Self::Quadratic => {
                if t < 0.5 {
                    0.75 - t * t
                } else if t < 1.5 {
                    0.5 * (t - 1.5) * (t - 1.5)
                } else {
                    0.0
                }
            }
            Self::Cubic => {
                if t < 1.0 {
                    (4.0 - 6.0 * t * t + 3.0 * t * t * t) / 6.0
                } else if t < 2.0 {
                    (2.0 - t).powi(3) / 6.0
                } else {
                    0.0
                }
            }
            Self::CatmullRom => cubic(t, 0.0, 0.5),
            Self::Mitchell => cubic(t, 1.0 / 3.0, 1.0 / 3.0),
        }
    }
}

/// The Mitchell–Netravali family of cubic filters with parameters `b` and `c`.
fn cubic(t: f32, b: f32, c: f32) -> f32 {
    if t < 1.0 {
        ((12.0 - 9.0 * b - 6.0 * c) * t * t * t + (-18.0 + 12.0 * b + 6.0 * c) * t * t + (6.0 - 2.0 * b)) / 6.0
    } else if t < 2.0 {
        ((-b - 6.0 * c) * t * t * t + (6.0 * b + 30.0 * c) * t * t + (-12.0 * b - 48.0 * c) * t + (8.0 * b + 24.0 * c)) / 6.0
    } else {
        0.0
    }
}

/// The weights of a filter `size` pixels wide, from −radius to radius, summing to one.
pub fn weights(filter: Filter, size: f32) -> Vec<f32> {
    if !(size.is_finite() && size >= 1.0) {
        return vec![1.0];
    }
    let radius = size.floor() as isize;
    let mut out: Vec<f32> = (-radius..=radius).map(|offset| filter.weight(offset as f32 / size)).collect();
    let total: f32 = out.iter().sum();
    if total.abs() > 1e-9 {
        for weight in &mut out {
            *weight /= total;
        }
    }
    out
}

/// Blurs along one axis: `horizontal` or down the columns. Pixels past the edges repeat the
/// edge, or are clear when `clear`.
fn pass(image: &Image, kernel: &[f32], horizontal: bool, clear: bool) -> Image {
    if kernel.len() <= 1 {
        return image.clone();
    }
    let radius = (kernel.len() / 2) as isize;
    let mut out = Vec::with_capacity(image.pixels.len());
    for y in 0..image.height as isize {
        for x in 0..image.width as isize {
            let mut sum: Rgba = [0.0; 4];
            for (index, weight) in kernel.iter().enumerate() {
                let offset = index as isize - radius;
                let (sx, sy) = if horizontal { (x + offset, y) } else { (x, y + offset) };
                let pixel = if clear { image.get(sx, sy).unwrap_or([0.0; 4]) } else { image.at(sx, sy) };
                for (total, value) in sum.iter_mut().zip(pixel) {
                    *total += weight * value;
                }
            }
            out.push(sum);
        }
    }
    Image { width: image.width, height: image.height, pixels: out }
}

/// Blurs `image` with `filter`, `size_x` and `size_y` pixels wide.
pub fn blur(image: &Image, filter: Filter, size_x: f32, size_y: f32) -> Image {
    blur_bounded(image, filter, size_x, size_y, false)
}

/// Blurs `image`; with `extend`, the picture is clear past its edges (the reference's Extend
/// Bounds grows the picture by the blur's reach, and its output keeps the middle), so light
/// spreads off the edges rather than piling up at them.
pub fn blur_bounded(image: &Image, filter: Filter, size_x: f32, size_y: f32, extend: bool) -> Image {
    let across = pass(image, &weights(filter, size_x), true, extend);
    pass(&across, &weights(filter, size_y), false, extend)
}
