//! Images on surfaces: PNG and JPEG decoded once into linear RGBA with a box-filtered mip chain,
//! sampled bilinearly from the level that matches how many texels one pixel covers. Colour
//! images are decoded from sRGB; data images (metallic and roughness) are kept as they are.

use std::collections::HashMap;
use std::sync::Arc;

use glam::{Vec2, Vec4};
use kition_scene_model::{Image, ImageFormat, Scene};

use crate::draw::decode;

/// One level of a mip chain.
#[derive(Debug)]
struct Level {
    width: usize,
    height: usize,
    texels: Vec<Vec4>,
}

/// A decoded image and its smaller versions.
#[derive(Debug)]
pub struct Texture {
    levels: Vec<Level>,
}

/// Decodes an image to straight RGBA8 (sRGB), with the size. A high dynamic range image is
/// clipped at 1, for thumbnails.
pub fn decode_rgba(image: &Image) -> Option<(usize, usize, Vec<u8>)> {
    match image.format {
        ImageFormat::Png | ImageFormat::Jpeg => kition_scene_model::pixels::decode_rgba8(image),
        ImageFormat::Hdr => {
            let (width, height, pixels) = kition_scene_model::hdr::decode(&image.bytes)?;
            let rgba = pixels.iter().flat_map(|[r, g, b]| [crate::draw::encode(*r), crate::draw::encode(*g), crate::draw::encode(*b), 255]).collect();
            Some((width, height, rgba))
        }
    }
}

/// Decodes an image to linear RGBA (colour images from sRGB when `srgb`), with the size.
pub fn decode_linear(image: &Image, srgb: bool) -> Option<(usize, usize, Vec<Vec4>)> {
    if image.format == ImageFormat::Hdr {
        let (width, height, pixels) = kition_scene_model::hdr::decode(&image.bytes)?;
        return Some((width, height, pixels.into_iter().map(|[r, g, b]| Vec4::new(r, g, b, 1.0)).collect()));
    }
    let (width, height, rgba) = decode_rgba(image)?;
    let channel = |byte: u8| if srgb { decode(byte) } else { f32::from(byte) / 255.0 };
    let texels = rgba.as_chunks::<4>().0.iter().map(|[r, g, b, a]| Vec4::new(channel(*r), channel(*g), channel(*b), f32::from(*a) / 255.0)).collect();
    Some((width, height, texels))
}

impl Texture {
    /// Decodes an image; `srgb` for colour images.
    pub fn new(image: &Image, srgb: bool) -> Option<Self> {
        let (width, height, texels) = decode_linear(image, srgb)?;
        let mut levels = vec![Level { width, height, texels }];
        while let Some(last) = levels.last() {
            if last.width == 1 && last.height == 1 {
                break;
            }
            let (width, height) = ((last.width / 2).max(1), (last.height / 2).max(1));
            let mut texels = Vec::with_capacity(width * height);
            for y in 0..height {
                for x in 0..width {
                    let mut sum = Vec4::ZERO;
                    let mut count = 0.0;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let (sx, sy) = ((x * 2 + dx).min(last.width - 1), (y * 2 + dy).min(last.height - 1));
                        if let Some(texel) = last.texels.get(sy * last.width + sx) {
                            sum += *texel;
                            count += 1.0;
                        }
                    }
                    texels.push(sum / f32::max(count, 1.0));
                }
            }
            levels.push(Level { width, height, texels });
        }
        Some(Self { levels })
    }

    /// The colour at `uv` (repeating; V up) where one pixel covers `uv_per_pixel` of the texture.
    pub fn sample(&self, uv: Vec2, uv_per_pixel: f32) -> Vec4 {
        let Some(base) = self.levels.first() else { return Vec4::ONE };
        let texels_per_pixel = uv_per_pixel.max(0.0) * base.width.max(base.height) as f32;
        let level = if texels_per_pixel > 1.0 { texels_per_pixel.log2().round() as usize } else { 0 };
        let Some(level) = self.levels.get(level.min(self.levels.len() - 1)) else { return Vec4::ONE };
        let (width, height) = (level.width as f32, level.height as f32);
        let x = uv.x.rem_euclid(1.0) * width - 0.5;
        let y = (1.0 - uv.y.rem_euclid(1.0)) * height - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let at = |tx: f32, ty: f32| {
            let tx = (tx as i64).rem_euclid(level.width as i64) as usize;
            let ty = (ty as i64).rem_euclid(level.height as i64) as usize;
            level.texels.get(ty * level.width + tx).copied().unwrap_or(Vec4::ONE)
        };
        let top = at(x0, y0).lerp(at(x0 + 1.0, y0), fx);
        let bottom = at(x0, y0 + 1.0).lerp(at(x0 + 1.0, y0 + 1.0), fx);
        top.lerp(bottom, fy)
    }
}

/// Decoded textures by image hash and colour space, kept between pictures.
#[derive(Debug, Default)]
pub struct TextureCache {
    entries: HashMap<(String, bool), Option<Arc<Texture>>>,
}

impl TextureCache {
    /// The decoded image `hash` (`None` when the scene has no such image or it cannot be read).
    pub fn get(&mut self, scene: &Scene, hash: &str, srgb: bool) -> Option<Arc<Texture>> {
        self.entries
            .entry((hash.to_string(), srgb))
            .or_insert_with(|| scene.image(hash).and_then(|image| Texture::new(image, srgb)).map(Arc::new))
            .clone()
    }

    /// Forgets images the scene no longer has.
    pub fn retain(&mut self, scene: &Scene) {
        self.entries.retain(|(hash, _), _| scene.image(hash).is_some());
    }
}
