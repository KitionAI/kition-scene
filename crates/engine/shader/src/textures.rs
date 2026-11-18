//! The Gradient, Checker, and Wave textures, and images as an Image Texture reads them. The
//! formulas were measured from the reference's output (`tests/shader_blender.rs`).

use std::collections::HashMap;
use std::f32::consts::PI;
use std::sync::Arc;

use glam::{Vec2, Vec3, Vec4};
use kition_scene_model::{ImageFormat, Scene};

use crate::noise::{Domain, Fractal, fbm};

/// The Gradient Texture's value at `p`.
pub fn gradient(kind: &str, p: Vec3) -> f32 {
    match kind {
        "quadratic" => p.x.max(0.0).powi(2),
        "easing" => {
            let r = p.x.clamp(0.0, 1.0);
            r * r * (3.0 - 2.0 * r)
        }
        "diagonal" => (p.x + p.y) * 0.5,
        "radial" => p.y.atan2(p.x) / (2.0 * PI) + 0.5,
        "quadratic_sphere" => (0.999_999 - p.length()).max(0.0).powi(2),
        "spherical" => (0.999_999 - p.length()).max(0.0),
        _ => p.x,
    }
}

/// The Checker Texture: whether `p` (already scaled) is in a square of the first colour.
pub fn checker(p: Vec3) -> bool {
    let q = (p + Vec3::splat(0.000_001)) * 0.999_999;
    let odd = |value: f32| (value.floor() as i64).rem_euclid(2) == 1;
    (odd(q.x) == odd(q.y)) == odd(q.z)
}

/// The Wave Texture's settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wave<'a> {
    pub kind: &'a str,
    pub bands: &'a str,
    pub rings: &'a str,
    pub profile: &'a str,
    pub distortion: f32,
    pub detail: f32,
    pub detail_scale: f32,
    pub detail_roughness: f32,
    pub phase: f32,
}

/// The Wave Texture's value at `p` (already scaled).
pub fn wave(w: &Wave<'_>, p: Vec3) -> f32 {
    // Scaled so one unit of the texture's scale is about one band, as the reference's are.
    let p = (p + Vec3::splat(0.000_001)) * 0.999_999;
    let mut n = if w.kind == "rings" {
        let q = match w.rings {
            "y" => Vec3::new(p.x, 0.0, p.z),
            "z" => Vec3::new(p.x, p.y, 0.0),
            "spherical" => p,
            _ => Vec3::new(0.0, p.y, p.z),
        };
        q.length() * 20.0
    } else {
        match w.bands {
            "y" => p.y * 20.0,
            "z" => p.z * 20.0,
            "diagonal" => (p.x + p.y + p.z) * 10.0,
            _ => p.x * 20.0,
        }
    };
    n += w.phase;
    if w.distortion != 0.0 {
        let fractal = Fractal { detail: w.detail, roughness: w.detail_roughness, lacunarity: 2.0, offset: 0.0, gain: 1.0 };
        n += w.distortion * (fbm(Domain::D3(p * w.detail_scale), &fractal, true) * 2.0 - 1.0);
    }
    match w.profile {
        "saw" => {
            let n = n / (2.0 * PI);
            n - n.floor()
        }
        "tri" => {
            let n = n / (2.0 * PI);
            ((n + 0.5) - (n + 0.5).floor() - 0.5).abs() * 2.0
        }
        _ => 0.5 + 0.5 * (n - PI / 2.0).sin(),
    }
}

/// HSV or HSL to RGB, and back.
pub fn from_model(mode: &str, [a, b, c]: [f32; 3]) -> [f32; 3] {
    match mode {
        "hsv" => kition_scene_compositor::color::hsv_to_rgb([a, b, c]),
        "hsl" => hsl_to_rgb([a, b, c]),
        _ => [a, b, c],
    }
}

pub fn to_model(mode: &str, rgb: [f32; 3]) -> [f32; 3] {
    match mode {
        "hsv" => kition_scene_compositor::color::rgb_to_hsv(rgb),
        "hsl" => rgb_to_hsl(rgb),
        _ => rgb,
    }
}

fn rgb_to_hsl([r, g, b]: [f32; 3]) -> [f32; 3] {
    let (high, low) = (r.max(g).max(b), r.min(g).min(b));
    let l = (high + low) * 0.5;
    if high == low {
        return [0.0, 0.0, l];
    }
    let d = high - low;
    let s = if l > 0.5 { d / (2.0 - high - low) } else { d / (high + low) };
    let h = if high == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if high == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    [h / 6.0, s, l]
}

fn hsl_to_rgb([h, s, l]: [f32; 3]) -> [f32; 3] {
    let channel = |n: f32| {
        let k = (n + h * 12.0).rem_euclid(12.0);
        let a = s * l.min(1.0 - l);
        l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    [channel(0.0), channel(8.0), channel(4.0)]
}

/// An image decoded to linear RGBA (colour images from sRGB), rows from the top.
#[derive(Debug)]
pub struct Picture {
    pub width: usize,
    pub height: usize,
    pub texels: Vec<Vec4>,
}

/// How an image is read past its edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extension {
    Repeat,
    Extend,
    Clip,
    Mirror,
}

impl Extension {
    pub fn parse(text: &str) -> Self {
        match text {
            "extend" => Self::Extend,
            "clip" => Self::Clip,
            "mirror" => Self::Mirror,
            _ => Self::Repeat,
        }
    }
}

fn srgb(byte: u8) -> f32 {
    let value = f32::from(byte) / 255.0;
    if value <= 0.040_45 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
}

impl Picture {
    /// Decodes a scene image; `srgb` for colour images (high dynamic range images are linear).
    pub fn decode(scene: &Scene, hash: &str, srgb_colors: bool) -> Option<Self> {
        let image = scene.image(hash)?;
        if image.format == ImageFormat::Hdr {
            let (width, height, pixels) = kition_scene_model::hdr::decode(&image.bytes)?;
            return Some(Self { width, height, texels: pixels.into_iter().map(|[r, g, b]| Vec4::new(r, g, b, 1.0)).collect() });
        }
        let (width, height, rgba) = kition_scene_model::pixels::decode_rgba8(image)?;
        let channel = |byte: u8| if srgb_colors { srgb(byte) } else { f32::from(byte) / 255.0 };
        let texels =
            rgba.as_chunks::<4>().0.iter().map(|[r, g, b, a]| Vec4::new(channel(*r), channel(*g), channel(*b), f32::from(*a) / 255.0)).collect();
        Some(Self { width, height, texels })
    }

    fn texel(&self, x: i64, y: i64, extension: Extension) -> Vec4 {
        let (w, h) = (self.width as i64, self.height as i64);
        let place = |value: i64, size: i64| -> Option<i64> {
            match extension {
                Extension::Repeat => Some(value.rem_euclid(size)),
                Extension::Extend => Some(value.clamp(0, size - 1)),
                Extension::Clip => (0..size).contains(&value).then_some(value),
                Extension::Mirror => {
                    let period = value.rem_euclid(2 * size);
                    Some(if period < size { period } else { 2 * size - 1 - period })
                }
            }
        };
        match (place(x, w), place(y, h)) {
            (Some(x), Some(y)) => self.texels.get((y * w + x) as usize).copied().unwrap_or(Vec4::ZERO),
            _ => Vec4::ZERO,
        }
    }

    /// The colour at `uv` (V up), bilinear or nearest.
    pub fn sample(&self, uv: Vec2, extension: Extension, closest: bool) -> Vec4 {
        if self.width == 0 || self.height == 0 {
            return Vec4::ZERO;
        }
        let x = uv.x * self.width as f32;
        let y = (1.0 - uv.y) * self.height as f32;
        if closest {
            return self.texel(x.floor() as i64, y.floor() as i64, extension);
        }
        let (x, y) = (x - 0.5, y - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let top = self.texel(x0, y0, extension).lerp(self.texel(x0 + 1, y0, extension), fx);
        let bottom = self.texel(x0, y0 + 1, extension).lerp(self.texel(x0 + 1, y0 + 1, extension), fx);
        top.lerp(bottom, fy)
    }
}

/// Decoded images by hash and colour space, kept between programs.
#[derive(Debug, Default)]
pub struct Pictures {
    entries: HashMap<(String, bool), Option<Arc<Picture>>>,
}

impl Pictures {
    pub fn get(&mut self, scene: &Scene, hash: &str, srgb_colors: bool) -> Option<Arc<Picture>> {
        self.entries.entry((hash.to_owned(), srgb_colors)).or_insert_with(|| Picture::decode(scene, hash, srgb_colors).map(Arc::new)).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsl_round_trips() {
        for rgb in [[0.2, 0.5, 0.9], [0.9, 0.1, 0.3], [0.4, 0.4, 0.4], [1.0, 0.8, 0.0]] {
            let back = hsl_to_rgb(rgb_to_hsl(rgb));
            for (a, b) in rgb.iter().zip(back) {
                assert!((a - b).abs() < 1e-5, "{rgb:?} {back:?}");
            }
        }
    }

    #[test]
    fn pictures_extend_clip_and_mirror() {
        let picture = Picture { width: 2, height: 1, texels: vec![Vec4::splat(0.0), Vec4::splat(1.0)] };
        let at = |u: f32, extension| picture.sample(Vec2::new(u, 0.5), extension, true).x;
        assert_eq!(at(1.2, Extension::Repeat), 0.0);
        assert_eq!(at(1.2, Extension::Extend), 1.0);
        assert_eq!(at(1.2, Extension::Clip), 0.0);
        assert_eq!(at(1.2, Extension::Mirror), 1.0);
        assert_eq!(at(1.7, Extension::Mirror), 0.0);
    }
}
