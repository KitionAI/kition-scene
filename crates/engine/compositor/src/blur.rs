//! Defocus and Vector Blur: blurs that vary over the picture.
//!
//! Defocus blurs each pixel by its circle of confusion: from its depth and the camera's lens and
//! focus (the thin lens: a point at depth z is spread over `A·f·|z − s| / (z·(s − f))` of the
//! sensor, A the opening `f / N`), or from the Z input times the Z scale, up to the most blur. A
//! pixel gathers its neighbours whose own circles reach it, through the bokeh's shape (a disc or
//! a polygon of three to eight blades, turned by the angle). It matches the reference's blur
//! sizes and its hard-edged shapes (a pixel reaches half a pixel past its circle); where blur
//! sizes vary, how neighbours of other sizes are weighted differs.
//!
//! Vector Blur smears each pixel along its motion (the speed pass's first two components, in
//! pixels a frame, y up) over half the shutter, nearer pixels covering farther ones, as the
//! reference's output on a moving disc shows. It matches the reference in shape.

use crate::{Image, Rgba};

/// The most blur a pixel may have (pixels).
pub const MOST: f32 = 256.0;

/// A bokeh's shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bokeh {
    /// 0 for a disc, else the polygon's corners.
    pub blades: u32,
    pub angle: f32,
}

impl Bokeh {
    pub fn parse(text: &str, angle: f32) -> Self {
        let blades = match text {
            "triangle" => 3,
            "square" => 4,
            "pentagon" => 5,
            "hexagon" => 6,
            "heptagon" => 7,
            "octagon" => 8,
            _ => 0,
        };
        Self { blades, angle }
    }

    /// How far (dx, dy) is from the centre in units of the bokeh's radius (1 on its edge).
    fn reach(&self, dx: f32, dy: f32) -> f32 {
        let distance = dx.hypot(dy);
        if self.blades < 3 || distance == 0.0 {
            return distance;
        }
        // A regular polygon with a corner at the angle: the distance to its edge along the
        // direction of (dx, dy).
        let sector = std::f32::consts::TAU / self.blades as f32;
        let theta = (dy.atan2(dx) - self.angle).rem_euclid(sector) - sector * 0.5;
        distance * theta.cos() / (sector * 0.5).cos()
    }
}

/// Defocus: each pixel's blur radius (pixels) and the bokeh.
pub fn defocus(image: &Image, radii: &[f32], bokeh: &Bokeh) -> Image {
    let (width, height) = (image.width as isize, image.height as isize);
    let largest = radii.iter().copied().filter(|radius| radius.is_finite()).fold(0.0f32, f32::max).min(MOST);
    let reach = (largest + 0.5).ceil() as isize;
    let radius_at =
        |x: isize, y: isize| radii.get((y * width + x) as usize).copied().filter(|value| value.is_finite()).unwrap_or(0.0).clamp(0.0, MOST);
    let mut pixels = Vec::with_capacity(image.pixels.len());
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0.0f32; 4];
            let mut total = 0.0f32;
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    let (sx, sy) = (x + dx, y + dy);
                    if sx < 0 || sy < 0 || sx >= width || sy >= height {
                        continue;
                    }
                    let radius = radius_at(sx, sy) + 0.5;
                    // The bokeh's y axis is up.
                    let weight = if bokeh.reach(dx as f32, -dy as f32) <= radius { 1.0 / (radius * radius) } else { 0.0 };
                    if weight <= 0.0 {
                        continue;
                    }
                    let pixel = image.pixels.get((sy * width + sx) as usize).copied().unwrap_or([0.0; 4]);
                    for (value, channel) in sum.iter_mut().zip(pixel) {
                        *value += weight * channel;
                    }
                    total += weight;
                }
            }
            pixels.push(if total > 0.0 {
                sum.map(|value| value / total)
            } else {
                image.pixels.get((y * width + x) as usize).copied().unwrap_or([0.0; 4])
            });
        }
    }
    Image { width: image.width, height: image.height, pixels }
}

/// The blur radius in pixels of a point at `depth` through `lens` at `f_stop`, for a picture
/// `width` pixels across.
pub fn circle_of_confusion(depth: f32, lens: &crate::Lens, f_stop: f32, width: usize) -> f32 {
    let focal = lens.focal_length.max(1e-3) / 1000.0;
    let focus = lens.focus_distance.max(focal * 1.001);
    let opening = focal / f_stop.max(1e-3);
    if !(depth.is_finite() && depth > 0.0) {
        // Nothing drawn: as far as can be.
        return opening * focal / (focus - focal) / (lens.sensor_width.max(1e-3) / 1000.0) * width as f32 * 0.5;
    }
    let diameter = opening * focal * (depth - focus).abs() / (depth * (focus - focal));
    diameter / (lens.sensor_width.max(1e-3) / 1000.0) * width as f32 * 0.5
}

/// Vector Blur: the picture, each pixel's speed (`[x, y, …]` in pixels a frame, y up), its
/// depth, the shutter (a share of a frame), and how many steps the smear takes.
pub fn vector_blur(image: &Image, speed: &[Rgba], depth: &[f32], shutter: f32, samples: u32) -> Image {
    let (width, height) = (image.width, image.height);
    let samples = samples.clamp(1, 256);
    let shutter = if shutter.is_finite() { shutter.clamp(0.0, 4.0) } else { 0.0 };
    let mut sum = vec![[0.0f32; 4]; image.pixels.len()];
    let mut frame: Vec<Option<(f32, Rgba)>> = vec![None; image.pixels.len()];
    for step in 0..samples {
        let t = shutter * 0.5 * (step as f32 + 0.5) / samples as f32;
        frame.iter_mut().for_each(|slot| *slot = None);
        for y in 0..height {
            for x in 0..width {
                let index = y * width + x;
                let motion = speed.get(index).copied().unwrap_or([0.0; 4]);
                let (mx, my) = (if motion[0].is_finite() { motion[0] } else { 0.0 }, if motion[1].is_finite() { motion[1] } else { 0.0 });
                let tx = (x as f32 + mx * t).round();
                let ty = (y as f32 - my * t).round();
                if tx < 0.0 || ty < 0.0 || tx >= width as f32 || ty >= height as f32 {
                    continue;
                }
                let target = ty as usize * width + tx as usize;
                let z = depth.get(index).copied().filter(|value| !value.is_nan()).unwrap_or(f32::INFINITY);
                let pixel = image.pixels.get(index).copied().unwrap_or([0.0; 4]);
                if let Some(slot) = frame.get_mut(target)
                    && slot.is_none_or(|(nearest, _)| z < nearest)
                {
                    *slot = Some((z, pixel));
                }
            }
        }
        for ((total, slot), pixel) in sum.iter_mut().zip(&frame).zip(&image.pixels) {
            // Where nothing lands, the still picture shows.
            let value = slot.map_or(*pixel, |(_, value)| value);
            for (total, channel) in total.iter_mut().zip(value) {
                *total += channel;
            }
        }
    }
    let pixels = sum.into_iter().map(|value| value.map(|channel| channel / samples as f32)).collect();
    Image { width, height, pixels }
}
