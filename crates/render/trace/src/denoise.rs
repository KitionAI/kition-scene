//! An edge-avoiding À-trous wavelet filter: noise is averaged away across pixels that share a
//! surface (similar normal and colour), never across edges. Light is filtered without the
//! surface colour (demodulated), so textures stay sharp.

use glam::Vec3;

const KERNEL: [f32; 3] = [3.0 / 8.0, 1.0 / 4.0, 1.0 / 16.0];

fn luminance(c: Vec3) -> f32 {
    c.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

/// Denoises a `width` × `height` picture.
pub fn atrous(colour: &[Vec3], albedo: &[Vec3], normal: &[Vec3], width: u32, height: u32) -> Vec<Vec3> {
    let (w, h) = (width as i64, height as i64);
    let base = |slot: usize| albedo.get(slot).copied().unwrap_or(Vec3::ONE).max(Vec3::splat(0.02));
    let mut light: Vec<Vec3> = colour.iter().enumerate().map(|(slot, c)| *c / base(slot)).collect();
    let mut sigma_colour = 0.6f32;
    for level in 0..4 {
        let step = 1i64 << level;
        let mut next = light.clone();
        for y in 0..h {
            for x in 0..w {
                let centre = (y * w + x) as usize;
                let (Some(c0), Some(n0), Some(a0)) = (light.get(centre), normal.get(centre), albedo.get(centre)) else { continue };
                let mut sum = Vec3::ZERO;
                let mut weights = 0.0f32;
                for dy in -2i64..=2 {
                    for dx in -2i64..=2 {
                        let (sx, sy) = (x + dx * step, y + dy * step);
                        if sx < 0 || sy < 0 || sx >= w || sy >= h {
                            continue;
                        }
                        let slot = (sy * w + sx) as usize;
                        let (Some(c), Some(n), Some(a)) = (light.get(slot), normal.get(slot), albedo.get(slot)) else { continue };
                        let kernel = KERNEL.get(dx.unsigned_abs() as usize).copied().unwrap_or(0.0)
                            * KERNEL.get(dy.unsigned_abs() as usize).copied().unwrap_or(0.0);
                        let wn = n0.dot(*n).max(0.0).powi(32);
                        let wa = (-(*a0 - *a).length_squared() / 0.02).exp();
                        let wc = (-(luminance(*c0) - luminance(*c)).abs() / (sigma_colour * luminance(*c0).max(0.05))).exp();
                        let weight = kernel * wn * wa * wc;
                        sum += *c * weight;
                        weights += weight;
                    }
                }
                if let Some(slot) = next.get_mut(centre) {
                    *slot = if weights > 0.0 { sum / weights } else { *c0 };
                }
            }
        }
        light = next;
        sigma_colour *= 0.6;
    }
    light.iter().enumerate().map(|(slot, l)| *l * base(slot)).collect()
}
