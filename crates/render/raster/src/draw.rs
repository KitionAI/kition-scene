//! The picture being drawn: colour, eye depth, and which object and face covers each pixel;
//! triangles with perspective-correct attributes, and depth-tested lines and dots.

use glam::{Vec2, Vec3, Vec4};

use crate::view::Projector;

/// "Nothing here" in the object and face buffers.
pub const NONE: u32 = u32::MAX;

/// As a triangle's object: leave the object and face buffers as they are (marks that are not
/// picked, such as custom bone shapes' faces).
pub const KEEP: u32 = u32::MAX - 1;

/// A picture and its per-pixel buffers.
#[derive(Debug, Clone)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    /// Linear RGB, alpha.
    pub color: Vec<[f32; 4]>,
    /// Distance in front of the eye (infinity where nothing was drawn).
    pub depth: Vec<f32>,
    /// The object id at each pixel, or [`NONE`].
    pub object: Vec<u32>,
    /// The face index (within its object's drawn mesh) at each pixel, or [`NONE`].
    pub face: Vec<u32>,
}

/// One corner of a triangle on its way to the screen.
#[derive(Debug, Clone, Copy)]
pub struct Corner {
    pub clip: Vec4,
    /// Distance in front of the eye.
    pub depth: f32,
    /// World position.
    pub position: Vec3,
    /// World normal.
    pub normal: Vec3,
    /// Texture coordinate.
    pub uv: Vec2,
    /// World tangent (xyz) and bitangent sign (w), for normal maps; zero without them.
    pub tangent: Vec4,
}

/// What a fragment knows when it is shaded.
#[derive(Debug, Clone, Copy)]
pub struct Fragment {
    pub position: Vec3,
    pub normal: Vec3,
    pub x: u32,
    pub y: u32,
    /// Texture coordinate.
    pub uv: Vec2,
    /// How much of the texture (in UV units) one pixel covers on this triangle, for choosing a
    /// mip level.
    pub uv_per_pixel: f32,
    /// World tangent and bitangent sign, interpolated (zero without a normal map).
    pub tangent: Vec4,
}

impl Canvas {
    /// A canvas filled with `background` (linear RGBA).
    pub fn new(width: u32, height: u32, background: [f32; 4]) -> Self {
        let count = (width as usize).saturating_mul(height as usize);
        Self { width, height, color: vec![background; count], depth: vec![f32::INFINITY; count], object: vec![NONE; count], face: vec![NONE; count] }
    }

    fn slot(&self, x: i64, y: i64) -> Option<usize> {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return None;
        }
        usize::try_from(y * i64::from(self.width) + x).ok()
    }

    /// The colour at a pixel.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[f32; 4]> {
        self.slot(i64::from(x), i64::from(y)).and_then(|slot| self.color.get(slot).copied())
    }

    /// Blends `color` over the pixel with `alpha`.
    pub fn blend(&mut self, x: i64, y: i64, color: [f32; 3], alpha: f32) {
        if let Some(pixel) = self.slot(x, y).and_then(|slot| self.color.get_mut(slot)) {
            for (channel, value) in pixel.iter_mut().zip(color) {
                *channel += (value - *channel) * alpha;
            }
            pixel[3] = pixel[3] + (1.0 - pixel[3]) * alpha;
        }
    }

    /// Draws a triangle with depth testing, shading each covered pixel with `shade` (which
    /// returns linear RGB). Corners behind the near plane are clipped. A `flat` triangle is
    /// shaded once, at its first covered pixel, and filled with that colour.
    pub fn triangle(
        &mut self,
        projector: &Projector,
        corners: [Corner; 3],
        object: u32,
        face: u32,
        flat: bool,
        shade: &mut dyn FnMut(Fragment) -> [f32; 4],
    ) {
        let polygon = clip_near(&corners);
        let Some(first) = polygon.first().copied() else { return };
        let mut cached = None;
        for pair in polygon.windows(2).skip(1) {
            if let [b, c] = pair {
                self.raster(projector, [first, *b, *c], object, face, flat, &mut cached, shade);
            }
        }
    }

    fn raster(
        &mut self,
        projector: &Projector,
        corners: [Corner; 3],
        object: u32,
        face: u32,
        flat: bool,
        cached: &mut Option<[f32; 4]>,
        shade: &mut dyn FnMut(Fragment) -> [f32; 4],
    ) {
        let screen = corners.map(|corner| projector.to_screen(corner.clip));
        let inverse_w = corners.map(|corner| 1.0 / corner.clip.w);
        let [a, b, c] = screen;
        let area = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
        if !area.is_finite() || area.abs() < 1e-12 {
            return;
        }
        let min_x = a.x.min(b.x).min(c.x).floor().max(0.0) as i64;
        let max_x = a.x.max(b.x).max(c.x).ceil().min(self.width as f32) as i64;
        let min_y = a.y.min(b.y).min(c.y).floor().max(0.0) as i64;
        let max_y = a.y.max(b.y).max(c.y).ceil().min(self.height as f32) as i64;
        if min_x >= max_x || min_y >= max_y {
            return;
        }
        // Barycentric weights are linear across the picture: step them along each row.
        let inverse_area = 1.0 / area;
        let uv_area = (corners[1].uv - corners[0].uv).perp_dot(corners[2].uv - corners[0].uv).abs();
        let uv_per_pixel = (uv_area / area.abs().max(1e-12)).sqrt();
        let (step0, step1) = ((b.y - c.y) * inverse_area, (c.y - a.y) * inverse_area);
        // Depth over w is linear in the picture too; so is 1 / w.
        let depth_w = [corners[0].depth * inverse_w[0], corners[1].depth * inverse_w[1], corners[2].depth * inverse_w[2]];
        let width = self.width as usize;
        for y in min_y..max_y {
            let py = y as f32 + 0.5;
            let px = min_x as f32 + 0.5;
            let mut w0 = ((b.x - px) * (c.y - py) - (b.y - py) * (c.x - px)) * inverse_area;
            let mut w1 = ((c.x - px) * (a.y - py) - (c.y - py) * (a.x - px)) * inverse_area;
            let row = y as usize * width;
            for x in min_x..max_x {
                let (u0, u1) = (w0, w1);
                w0 += step0;
                w1 += step1;
                let u2 = 1.0 - u0 - u1;
                if u0 < 0.0 || u1 < 0.0 || u2 < 0.0 {
                    continue;
                }
                let sum = u0 * inverse_w[0] + u1 * inverse_w[1] + u2 * inverse_w[2];
                if sum <= 0.0 {
                    continue;
                }
                let depth = (u0 * depth_w[0] + u1 * depth_w[1] + u2 * depth_w[2]) / sum;
                let slot = row + x as usize;
                if self.depth.get(slot).is_none_or(|stored| depth >= *stored) {
                    continue;
                }
                let color = match (flat, *cached) {
                    (true, Some(color)) => color,
                    _ => {
                        // Perspective-correct weights.
                        let (p0, p1, p2) = (u0 * inverse_w[0] / sum, u1 * inverse_w[1] / sum, u2 * inverse_w[2] / sum);
                        let position = corners[0].position * p0 + corners[1].position * p1 + corners[2].position * p2;
                        let normal = (corners[0].normal * p0 + corners[1].normal * p1 + corners[2].normal * p2).normalize_or_zero();
                        let uv = corners[0].uv * p0 + corners[1].uv * p1 + corners[2].uv * p2;
                        let tangent = corners[0].tangent * p0 + corners[1].tangent * p1 + corners[2].tangent * p2;
                        let color = shade(Fragment { position, normal, x: x as u32, y: y as u32, uv, uv_per_pixel, tangent });
                        if flat {
                            *cached = Some(color);
                        }
                        color
                    }
                };
                // The fourth value is how much of the pixel the surface covers: none (a clipped
                // part) leaves it, some blends over it without hiding what is behind, all hides it.
                let coverage = color[3];
                if coverage.is_nan() || coverage <= 0.001 {
                    continue;
                }
                if coverage < 0.999 {
                    if let Some(pixel) = self.color.get_mut(slot) {
                        for (value, source) in pixel.iter_mut().zip(color).take(3) {
                            *value += (source - *value) * coverage;
                        }
                    }
                    if coverage >= 0.5 && object != KEEP {
                        if let Some(stored) = self.object.get_mut(slot) {
                            *stored = object;
                        }
                        if let Some(stored) = self.face.get_mut(slot) {
                            *stored = face;
                        }
                    }
                    continue;
                }
                if let Some(stored) = self.depth.get_mut(slot) {
                    *stored = depth;
                }
                if let Some(pixel) = self.color.get_mut(slot) {
                    *pixel = [color[0], color[1], color[2], 1.0];
                }
                if object == KEEP {
                    continue;
                }
                if let Some(stored) = self.object.get_mut(slot) {
                    *stored = object;
                }
                if let Some(stored) = self.face.get_mut(slot) {
                    *stored = face;
                }
            }
        }
    }

    /// Draws a world-space line, `width` pixels wide. With `depth_test`, parts behind drawn
    /// surfaces are hidden (a small tolerance lets edges show on their own faces).
    pub fn line(&mut self, projector: &Projector, from: Vec3, to: Vec3, color: [f32; 3], alpha: f32, width: u32, depth_test: bool) {
        let matrix = projector.view_projection();
        let view = projector.view;
        let ends = [from, to].map(|point| Corner {
            clip: matrix * point.extend(1.0),
            depth: -(view.transform_point3(point)).z,
            position: point,
            normal: Vec3::ZERO,
            uv: Vec2::ZERO,
            tangent: Vec4::ZERO,
        });
        let clipped = clip_near_segment(ends[0], ends[1]);
        let Some((start, end)) = clipped else { return };
        let a = projector.to_screen(start.clip);
        let b = projector.to_screen(end.clip);
        if !(a.is_finite() && b.is_finite()) {
            return;
        }
        // Only the part on the picture is stepped (Liang–Barsky against the picture, padded by
        // the line's width).
        let pad = width as f32 + 1.0;
        let Some((t0, t1)) = clip_to_rect(a, b, [-pad, -pad, self.width as f32 + pad, self.height as f32 + pad]) else { return };
        let length = ((b.x - a.x).abs().max((b.y - a.y).abs()) * (t1 - t0)).ceil();
        let steps = length.clamp(1.0, 40_000.0) as i64;
        let reach = i64::from(width.max(1)) - 1;
        for step in 0..=steps {
            let t = t0 + (t1 - t0) * step as f32 / steps as f32;
            let x = a.x + (b.x - a.x) * t;
            let y = a.y + (b.y - a.y) * t;
            // Depth along the segment, perspective-correct.
            let inverse = (1.0 - t) / start.clip.w + t / end.clip.w;
            let depth = ((1.0 - t) * start.depth / start.clip.w + t * end.depth / end.clip.w) / inverse;
            for dy in 0..=reach {
                for dx in 0..=reach {
                    let (px, py) = (x.floor() as i64 + dx, y.floor() as i64 + dy);
                    if depth_test && !self.visible(px, py, depth) {
                        continue;
                    }
                    self.blend(px, py, color, alpha);
                }
            }
        }
    }

    /// Draws a square dot of `size` pixels centred on a world point.
    pub fn dot(&mut self, projector: &Projector, point: Vec3, color: [f32; 3], size: u32, depth_test: bool) {
        let clip = projector.view_projection() * point.extend(1.0);
        if clip.z < 0.0 || clip.w <= 0.0 {
            return;
        }
        let depth = -(projector.view.transform_point3(point)).z;
        let screen = projector.to_screen(clip);
        let half = i64::from(size / 2);
        let (cx, cy) = (screen.x.floor() as i64, screen.y.floor() as i64);
        if depth_test && !self.visible(cx, cy, depth) {
            return;
        }
        for y in cy - half..=cy - half + i64::from(size.max(1)) - 1 {
            for x in cx - half..=cx - half + i64::from(size.max(1)) - 1 {
                self.blend(x, y, color, 1.0);
            }
        }
    }

    /// Whether something at `depth` in front of the eye shows at a pixel.
    pub fn visible(&self, x: i64, y: i64, depth: f32) -> bool {
        self.slot(x, y).and_then(|slot| self.depth.get(slot)).is_some_and(|stored| depth <= *stored * 1.002 + 1e-4)
    }

    /// The picture as straight-alpha sRGB RGBA8.
    pub fn to_rgba8(&self) -> Vec<u8> {
        let table = encode_table();
        let lookup = |value: f32| -> u8 {
            let value = if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.0 };
            table.get((value * (ENCODE_STEPS - 1) as f32).round() as usize).copied().unwrap_or(0)
        };
        let mut out = Vec::with_capacity(self.color.len() * 4);
        for pixel in &self.color {
            out.extend([lookup(pixel[0]), lookup(pixel[1]), lookup(pixel[2]), (pixel[3].clamp(0.0, 1.0) * 255.0).round() as u8]);
        }
        out
    }
}

/// Steps of the linear-to-sRGB table: fine enough that every byte the exact curve gives for a
/// linear value is reached within half a step.
const ENCODE_STEPS: usize = 1 << 14;

/// The linear-to-sRGB curve sampled at `ENCODE_STEPS` points, built once.
fn encode_table() -> &'static [u8] {
    static TABLE: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| (0..ENCODE_STEPS).map(|step| encode(step as f32 / (ENCODE_STEPS - 1) as f32)).collect())
}

/// A linear value as an sRGB byte.
pub fn encode(value: f32) -> u8 {
    let value = if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.0 };
    let srgb = if value <= 0.003_130_8 { value * 12.92 } else { 1.055 * value.powf(1.0 / 2.4) - 0.055 };
    (srgb * 255.0).round() as u8
}

/// An sRGB byte as a linear value.
pub fn decode(byte: u8) -> f32 {
    let value = f32::from(byte) / 255.0;
    if value <= 0.040_45 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
}

/// A `#rrggbb` colour as linear RGB.
pub fn hex(rgb: u32) -> [f32; 3] {
    [decode(((rgb >> 16) & 0xff) as u8), decode(((rgb >> 8) & 0xff) as u8), decode((rgb & 0xff) as u8)]
}

/// The parameter range of segment `a`–`b` inside `rect` (x0, y0, x1, y1), if any.
fn clip_to_rect(a: Vec3, b: Vec3, rect: [f32; 4]) -> Option<(f32, f32)> {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [(-dx, a.x - rect[0]), (dx, rect[2] - a.x), (-dy, a.y - rect[1]), (dy, rect[3] - a.y)] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let r = q / p;
        if p < 0.0 {
            t0 = t0.max(r);
        } else {
            t1 = t1.min(r);
        }
        if t0 > t1 {
            return None;
        }
    }
    Some((t0, t1))
}

fn lerp(a: Corner, b: Corner, t: f32) -> Corner {
    Corner {
        clip: a.clip.lerp(b.clip, t),
        depth: a.depth + (b.depth - a.depth) * t,
        position: a.position.lerp(b.position, t),
        normal: a.normal.lerp(b.normal, t),
        uv: a.uv.lerp(b.uv, t),
        tangent: a.tangent.lerp(b.tangent, t),
    }
}

/// The polygon's part in front of the near plane (clip z ≥ 0).
fn clip_near(corners: &[Corner; 3]) -> Vec<Corner> {
    let mut out = Vec::with_capacity(4);
    for (index, current) in corners.iter().enumerate() {
        let Some(next) = corners.get((index + 1) % 3) else { continue };
        let (inside_current, inside_next) = (current.clip.z >= 0.0, next.clip.z >= 0.0);
        if inside_current {
            out.push(*current);
        }
        if inside_current != inside_next {
            let t = current.clip.z / (current.clip.z - next.clip.z);
            out.push(lerp(*current, *next, t));
        }
    }
    out
}

fn clip_near_segment(a: Corner, b: Corner) -> Option<(Corner, Corner)> {
    match (a.clip.z >= 0.0, b.clip.z >= 0.0) {
        (true, true) => Some((a, b)),
        (false, false) => None,
        (true, false) => Some((a, lerp(a, b, a.clip.z / (a.clip.z - b.clip.z)))),
        (false, true) => Some((lerp(a, b, a.clip.z / (a.clip.z - b.clip.z)), b)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_round_trips() {
        for byte in 0..=255u8 {
            assert_eq!(encode(decode(byte)), byte);
        }
    }
}
