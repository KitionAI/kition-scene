//! Transform and crop nodes: Translate, Rotate, Scale, Transform, Flip, and Crop. Each picture
//! keeps the run's size: a node moves, turns, or scales the picture about its centre and samples
//! it back onto the same pixels, as the reference does when its group's output takes the
//! transformed picture (measured from its output: the samples land where the transformation
//! puts each pixel's centre, with no extra filtering when the picture shrinks).
//!
//! Positions are in pixels with the y axis up and the origin at the picture's centre; angles are
//! radians, anticlockwise.

use crate::{Image, Rgba};

/// How a picture is read between pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interpolation {
    Nearest,
    Bilinear,
    /// The cubic B-spline (it smooths a little even where nothing moves).
    Bicubic,
}

impl Interpolation {
    /// Anisotropic filtering reads like bilinear for the moves and scales these nodes make at
    /// the run's size (measured: the reference gives the same pixels).
    pub fn parse(text: &str) -> Self {
        match text {
            "nearest" => Self::Nearest,
            "bicubic" => Self::Bicubic,
            _ => Self::Bilinear,
        }
    }
}

/// What a picture is beyond its edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extension {
    /// Clear.
    Clip,
    /// The edge pixels repeat.
    Extend,
    /// The picture repeats.
    Repeat,
}

impl Extension {
    pub fn parse(text: &str) -> Self {
        match text {
            "extend" => Self::Extend,
            "repeat" => Self::Repeat,
            _ => Self::Clip,
        }
    }

    /// The pixel index `index` stands for on an axis of `count` pixels, if any.
    fn index(self, index: isize, count: usize) -> Option<usize> {
        let count = count as isize;
        if count <= 0 {
            return None;
        }
        match self {
            Self::Clip => (0..count).contains(&index).then_some(index as usize),
            Self::Extend => Some(index.clamp(0, count - 1) as usize),
            Self::Repeat => Some(index.rem_euclid(count) as usize),
        }
    }
}

/// How a node reads its picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sampler {
    pub interpolation: Interpolation,
    pub x: Extension,
    pub y: Extension,
}

impl Sampler {
    fn pixel(&self, image: &Image, x: isize, y: isize) -> Rgba {
        match (self.x.index(x, image.width), self.y.index(y, image.height)) {
            (Some(x), Some(y)) => image.pixels.get(y * image.width + x).copied().unwrap_or([0.0; 4]),
            _ => [0.0; 4],
        }
    }

    /// The picture at (`u`, `v`): pixels from the top left, pixel centres at halves.
    pub fn sample(&self, image: &Image, u: f32, v: f32) -> Rgba {
        if !(u.is_finite() && v.is_finite()) {
            return [0.0; 4];
        }
        match self.interpolation {
            // A position on a pixel's edge reads the pixel before it (as the reference does).
            Interpolation::Nearest => self.pixel(image, (u - 1.0).ceil() as isize, (v - 1.0).ceil() as isize),
            Interpolation::Bilinear => {
                let (x, y) = (u - 0.5, v - 0.5);
                let (x0, y0) = (x.floor(), y.floor());
                // The weights in 256ths, as texture hardware (and the reference) has them.
                let (fx, fy) = (quantize(x - x0), quantize(y - y0));
                let (x0, y0) = (x0 as isize, y0 as isize);
                let mut out = [0.0; 4];
                for (dy, wy) in [(0, 1.0 - fy), (1, fy)] {
                    for (dx, wx) in [(0, 1.0 - fx), (1, fx)] {
                        let pixel = self.pixel(image, x0 + dx, y0 + dy);
                        for (value, channel) in out.iter_mut().zip(pixel) {
                            *value += wx * wy * channel;
                        }
                    }
                }
                out
            }
            Interpolation::Bicubic => {
                // The cubic B-spline as four bilinear reads (each axis's four weights folded
                // into two reads between pixels), as texture hardware and the reference do it.
                let (x, y) = (u - 0.5, v - 0.5);
                let (x0, y0) = (x.floor(), y.floor());
                let fold = |start: f32, [w0, w1, w2, w3]: [f32; 4]| {
                    let (near, far) = (w0 + w1, w2 + w3);
                    let at = |weight: f32, sum: f32| if sum > 0.0 { weight / sum } else { 0.0 };
                    [(near, start - 1.0 + at(w1, near)), (far, start + 1.0 + at(w3, far))]
                };
                let (across, down) = (fold(x0, b_spline(x - x0)), fold(y0, b_spline(y - y0)));
                let bilinear = Self { interpolation: Interpolation::Bilinear, ..*self };
                let mut out = [0.0; 4];
                for (wy, ty) in down {
                    for (wx, tx) in across {
                        let pixel = bilinear.sample(image, tx + 0.5, ty + 0.5);
                        for (value, channel) in out.iter_mut().zip(pixel) {
                            *value += wx * wy * channel;
                        }
                    }
                }
                out
            }
        }
    }
}

/// A weight between pixels to the nearest 256th.
fn quantize(fraction: f32) -> f32 {
    (fraction * 256.0).round() / 256.0
}

/// The cubic B-spline's weights for the four pixels around a position `t` past the second.
fn b_spline(t: f32) -> [f32; 4] {
    let s = 1.0 - t;
    [s * s * s / 6.0, (3.0 * t * t * t - 6.0 * t * t + 4.0) / 6.0, (-3.0 * t * t * t + 3.0 * t * t + 3.0 * t + 1.0) / 6.0, t * t * t / 6.0]
}

/// A move, turn, and scale about the picture's centre: a point `p` (y up, from the centre) goes
/// to `offset + rotate(angle) · (scale ⊙ p)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub offset: [f32; 2],
    pub angle: f32,
    pub scale: [f32; 2],
}

impl Placement {
    pub const IDENTITY: Self = Self { offset: [0.0, 0.0], angle: 0.0, scale: [1.0, 1.0] };
}

/// The picture placed by `placement` and read back onto its own pixels.
pub fn place(image: &Image, placement: &Placement, sampler: &Sampler) -> Image {
    let (width, height) = (image.width as f32, image.height as f32);
    let (cx, cy) = (width * 0.5, height * 0.5);
    let (sin, cos) = (-placement.angle).sin_cos();
    let scale = placement.scale.map(|value| if value.is_finite() && value.abs() > 1e-6 { value } else { 0.0 });
    let mut pixels = Vec::with_capacity(image.pixels.len());
    for y in 0..image.height {
        for x in 0..image.width {
            if scale.contains(&0.0) {
                pixels.push([0.0; 4]);
                continue;
            }
            let qx = x as f32 + 0.5 - cx - placement.offset[0];
            let qy = cy - (y as f32 + 0.5) - placement.offset[1];
            let (rx, ry) = (qx * cos - qy * sin, qx * sin + qy * cos);
            let (sx, sy) = (rx / scale[0], ry / scale[1]);
            pixels.push(sampler.sample(image, cx + sx, cy - sy));
        }
    }
    Image { width: image.width, height: image.height, pixels }
}

/// The picture mirrored left to right and/or top to bottom.
pub fn flip(image: &Image, horizontal: bool, vertical: bool) -> Image {
    let mut pixels = Vec::with_capacity(image.pixels.len());
    for y in 0..image.height {
        let sy = if vertical { image.height - 1 - y } else { y };
        for x in 0..image.width {
            let sx = if horizontal { image.width - 1 - x } else { x };
            pixels.push(image.pixels.get(sy * image.width + sx).copied().unwrap_or([0.0; 4]));
        }
    }
    Image { width: image.width, height: image.height, pixels }
}

/// A Crop node's rectangle: its lower left corner and its size in pixels, from the picture's
/// lower left corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crop {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
    /// Clear what is outside the rectangle and keep the picture where it is; otherwise the
    /// rectangle alone is the picture, in the middle.
    pub alpha: bool,
}

/// The picture cropped to the rectangle (kept inside the picture).
pub fn crop(image: &Image, crop: &Crop) -> Image {
    let (width, height) = (image.width as i64, image.height as i64);
    let x0 = crop.x.clamp(0, width);
    let y0 = crop.y.clamp(0, height);
    let x1 = crop.x.saturating_add(crop.width.max(0)).clamp(x0, width);
    let y1 = crop.y.saturating_add(crop.height.max(0)).clamp(y0, height);
    // Rows from the top: the rectangle's rows are height - y1 to height - y0.
    let (top, bottom) = (height - y1, height - y0);
    let mut out = Image::filled(image.width, image.height, [0.0; 4]);
    let (kept_width, kept_height) = (x1 - x0, bottom - top);
    // In the middle, rounding down from the lower left (the reference's y axis is up).
    let (left, up) = if crop.alpha { (x0, top) } else { ((width - kept_width) / 2, height - (height - kept_height) / 2 - kept_height) };
    for row in 0..kept_height {
        for column in 0..kept_width {
            let from = ((top + row) * width + x0 + column) as usize;
            let to = ((up + row) * width + left + column) as usize;
            if let (Some(pixel), Some(slot)) = (image.pixels.get(from).copied(), out.pixels.get_mut(to)) {
                *slot = pixel;
            }
        }
    }
    out
}
