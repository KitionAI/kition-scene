//! Gradient noise and its fractal sums, as the Noise and Wave textures use them: Perlin's
//! improved noise (quintic fade, gradients picked by an integer hash of the lattice point) in one,
//! two, and three dimensions, and the fractal sums of "Texturing and Modeling: A Procedural
//! Approach" (fBm, multifractal, ridged and hybrid multifractal, hetero terrain). The hash is our
//! own, so the pattern differs from the reference's while its statistics match: the output scale
//! of each dimension was fitted so the spread of the reference's values is ours (see
//! `tests/shader_blender.rs`).

use glam::{Vec2, Vec3};

/// A 32-bit integer mix (Wellons' lowbias32).
pub fn mix32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

/// The hash of a lattice point.
pub fn hash3(x: i32, y: i32, z: i32, seed: u32) -> u32 {
    mix32((x as u32).wrapping_add(mix32((y as u32).wrapping_add(mix32((z as u32).wrapping_add(mix32(seed)))))))
}

/// A number from 0 to 1 from a hash.
pub fn unit(hash: u32) -> f32 {
    (hash >> 8) as f32 / (1u32 << 24) as f32
}

/// Three numbers from 0 to 1 for a lattice point.
pub fn unit3(x: i32, y: i32, z: i32, seed: u32) -> Vec3 {
    Vec3::new(unit(hash3(x, y, z, seed)), unit(hash3(x, y, z, seed.wrapping_add(1))), unit(hash3(x, y, z, seed.wrapping_add(2))))
}

fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn grad3(hash: u32, x: f32, y: f32, z: f32) -> f32 {
    let h = hash & 15;
    let u = if h < 8 { x } else { y };
    let v = if h < 4 {
        y
    } else if h == 12 || h == 14 {
        x
    } else {
        z
    };
    (if h & 1 == 0 { u } else { -u }) + (if h & 2 == 0 { v } else { -v })
}

fn grad2(hash: u32, x: f32, y: f32) -> f32 {
    let h = hash & 7;
    let (u, v) = if h < 4 { (x, y) } else { (y, x) };
    (if h & 1 == 0 { u } else { -u }) + (if h & 2 == 0 { 2.0 * v } else { -2.0 * v })
}

fn grad1(hash: u32, x: f32) -> f32 {
    let h = hash & 15;
    let g = 1.0 + (h & 7) as f32;
    if h & 8 == 0 { g * x } else { -g * x }
}

/// Output scales fitted to the reference's spread (signed noise from about −1 to 1).
const SCALE_1D: f32 = 0.188;
const SCALE_2D: f32 = 0.66;
const SCALE_3D: f32 = 0.982;

/// Signed noise of a number, about −1 to 1.
pub fn snoise1(x: f32) -> f32 {
    let (cell, t) = (x.floor(), x - x.floor());
    let i = cell as i32;
    SCALE_1D * lerp(grad1(hash3(i, 0, 0, 11), t), grad1(hash3(i + 1, 0, 0, 11), t - 1.0), fade(t))
}

/// Signed noise of a point in the plane, about −1 to 1.
pub fn snoise2(p: Vec2) -> f32 {
    let cell = p.floor();
    let t = p - cell;
    let (i, j) = (cell.x as i32, cell.y as i32);
    let h = |a: i32, b: i32| hash3(i + a, j + b, 0, 12);
    let (u, v) = (fade(t.x), fade(t.y));
    let bottom = lerp(grad2(h(0, 0), t.x, t.y), grad2(h(1, 0), t.x - 1.0, t.y), u);
    let top = lerp(grad2(h(0, 1), t.x, t.y - 1.0), grad2(h(1, 1), t.x - 1.0, t.y - 1.0), u);
    SCALE_2D * lerp(bottom, top, v)
}

/// Signed noise of a point in space, about −1 to 1.
pub fn snoise3(p: Vec3) -> f32 {
    let cell = p.floor();
    let t = p - cell;
    let (i, j, k) = (cell.x as i32, cell.y as i32, cell.z as i32);
    let h = |a: i32, b: i32, c: i32| hash3(i + a, j + b, k + c, 13);
    let (u, v, w) = (fade(t.x), fade(t.y), fade(t.z));
    let g = |a: i32, b: i32, c: i32| grad3(h(a, b, c), t.x - a as f32, t.y - b as f32, t.z - c as f32);
    let near = lerp(lerp(g(0, 0, 0), g(1, 0, 0), u), lerp(g(0, 1, 0), g(1, 1, 0), u), v);
    let far = lerp(lerp(g(0, 0, 1), g(1, 0, 1), u), lerp(g(0, 1, 1), g(1, 1, 1), u), v);
    SCALE_3D * lerp(near, far, w)
}

/// Where noise is taken: a number, a point in the plane, or one in space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Domain {
    D1(f32),
    D2(Vec2),
    D3(Vec3),
}

impl Domain {
    /// The domain `dimensions` ("1d", "2d", "3d") takes from a vector.
    pub fn of(dimensions: &str, p: Vec3) -> Self {
        match dimensions {
            "1d" => Self::D1(p.x),
            "2d" => Self::D2(p.truncate()),
            _ => Self::D3(p),
        }
    }

    pub fn snoise(self) -> f32 {
        match self {
            Self::D1(x) => snoise1(x),
            Self::D2(p) => snoise2(p),
            Self::D3(p) => snoise3(p),
        }
    }

    pub fn scaled(self, factor: f32) -> Self {
        match self {
            Self::D1(x) => Self::D1(x * factor),
            Self::D2(p) => Self::D2(p * factor),
            Self::D3(p) => Self::D3(p * factor),
        }
    }

    /// The point moved by `offset` (only the components it has).
    pub fn moved(self, offset: Vec3) -> Self {
        match self {
            Self::D1(x) => Self::D1(x + offset.x),
            Self::D2(p) => Self::D2(p + offset.truncate()),
            Self::D3(p) => Self::D3(p + offset),
        }
    }
}

/// The fractal sums' settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fractal {
    /// Octaves past the first; a fraction blends the last one in.
    pub detail: f32,
    /// How much each octave keeps of the one before.
    pub roughness: f32,
    /// How much finer each octave is.
    pub lacunarity: f32,
    pub offset: f32,
    pub gain: f32,
}

/// fBm: octaves summed, from 0 to 1 around 0.5 when `normalize` (else signed).
pub fn fbm(p: Domain, f: &Fractal, normalize: bool) -> f32 {
    let detail = f.detail.clamp(0.0, 15.0);
    let (mut scale, mut amplitude, mut most, mut sum) = (1.0f32, 1.0f32, 0.0f32, 0.0f32);
    for _ in 0..=detail.floor() as u32 {
        sum += p.scaled(scale).snoise() * amplitude;
        most += amplitude;
        amplitude *= f.roughness.clamp(0.0, 1.0);
        scale *= f.lacunarity;
    }
    let rest = detail - detail.floor();
    let finish = |sum: f32, most: f32| if normalize { 0.5 * sum / most.max(1e-12) + 0.5 } else { sum };
    if rest > 0.0 {
        let last = sum + p.scaled(scale).snoise() * amplitude;
        lerp(finish(sum, most), finish(last, most + amplitude), rest)
    } else {
        finish(sum, most)
    }
}

/// Multifractal: octaves multiplied.
pub fn multifractal(p: Domain, f: &Fractal) -> f32 {
    let detail = f.detail.clamp(0.0, 15.0);
    let (mut value, mut power, mut scale) = (1.0f32, 1.0f32, 1.0f32);
    for _ in 0..=detail.floor() as u32 {
        value *= power * p.scaled(scale).snoise() + 1.0;
        power *= f.roughness;
        scale *= f.lacunarity;
    }
    let rest = detail - detail.floor();
    if rest > 0.0 {
        value *= rest * power * p.scaled(scale).snoise() + 1.0;
    }
    value
}

/// Hetero terrain: octaves scaled by the height so far.
pub fn hetero_terrain(p: Domain, f: &Fractal) -> f32 {
    let detail = f.detail.clamp(0.0, 15.0);
    let mut value = f.offset + p.snoise();
    let (mut power, mut scale) = (f.roughness, f.lacunarity);
    for _ in 1..=detail.floor() as u32 {
        value += (p.scaled(scale).snoise() + f.offset) * power * value;
        power *= f.roughness;
        scale *= f.lacunarity;
    }
    let rest = detail - detail.floor();
    if rest > 0.0 {
        value += rest * (p.scaled(scale).snoise() + f.offset) * power * value;
    }
    value
}

/// Hybrid multifractal: octaves weighted by the ones before.
pub fn hybrid_multifractal(p: Domain, f: &Fractal) -> f32 {
    let detail = f.detail.clamp(0.0, 15.0);
    let (mut value, mut weight, mut power, mut scale) = (0.0f32, 1.0f32, 1.0f32, 1.0f32);
    let octaves = detail.floor() as u32;
    for _ in 0..=octaves {
        if weight <= 0.001 {
            break;
        }
        weight = weight.min(1.0);
        let signal = (p.scaled(scale).snoise() + f.offset) * power;
        power *= f.roughness;
        value += weight * signal;
        weight *= f.gain * signal;
        scale *= f.lacunarity;
    }
    let rest = detail - detail.floor();
    if rest > 0.0 && weight > 0.001 {
        weight = weight.min(1.0);
        value += rest * weight * (p.scaled(scale).snoise() + f.offset) * power;
    }
    value
}

/// Ridged multifractal: sharp ridges where the noise crosses zero.
pub fn ridged_multifractal(p: Domain, f: &Fractal) -> f32 {
    let detail = f.detail.clamp(0.0, 15.0);
    let mut signal = (f.offset - p.snoise().abs()).powi(2);
    let mut value = signal;
    let (mut power, mut scale) = (f.roughness, f.lacunarity);
    for _ in 1..=detail.floor() as u32 {
        let weight = (signal * f.gain).clamp(0.0, 1.0);
        signal = (f.offset - p.scaled(scale).snoise().abs()).powi(2) * weight;
        value += signal * power;
        power *= f.roughness;
        scale *= f.lacunarity;
    }
    value
}

/// The fractal sum `kind` names.
pub fn fractal(kind: &str, p: Domain, f: &Fractal, normalize: bool) -> f32 {
    match kind {
        "multifractal" => multifractal(p, f),
        "ridged_multifractal" => ridged_multifractal(p, f),
        "hybrid_multifractal" => hybrid_multifractal(p, f),
        "hetero_terrain" => hetero_terrain(p, f),
        _ => fbm(p, f, normalize),
    }
}

/// Offsets that move the Color output's green and blue (and distortion's three axes) to
/// unrelated noise.
const OFFSETS: [Vec3; 5] = [
    Vec3::new(13.5, 51.25, 7.75),
    Vec3::new(91.5, 3.25, 40.75),
    Vec3::new(27.25, 70.5, 88.5),
    Vec3::new(64.75, 19.5, 5.25),
    Vec3::new(8.5, 36.75, 59.25),
];

/// The Noise Texture: its factor and colour at `p` (already scaled).
pub fn noise_texture(p: Domain, kind: &str, f: &Fractal, distortion: f32, normalize: bool) -> (f32, Vec3) {
    let p = if distortion != 0.0 {
        let offset = Vec3::new(p.moved(OFFSETS[0]).snoise(), p.moved(OFFSETS[1]).snoise(), p.moved(OFFSETS[2]).snoise()) * distortion;
        p.moved(offset)
    } else {
        p
    };
    let value = fractal(kind, p, f, normalize);
    let green = fractal(kind, p.moved(OFFSETS[3]), f, normalize);
    let blue = fractal(kind, p.moved(OFFSETS[4]), f, normalize);
    (value, Vec3::new(value, green, blue))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_zero_on_the_lattice_and_continuous() {
        assert_eq!(snoise3(Vec3::new(3.0, -2.0, 5.0)), 0.0);
        assert_eq!(snoise2(Vec2::new(-4.0, 1.0)), 0.0);
        let p = Vec3::new(0.3, 1.7, -2.2);
        let step = Vec3::splat(1e-4);
        assert!((snoise3(p) - snoise3(p + step)).abs() < 1e-3);
        let values: Vec<f32> = (0..2000).map(|index| snoise3(Vec3::new(index as f32 * 0.37, index as f32 * 0.13, 0.5))).collect();
        assert!(values.iter().all(|value| value.abs() <= 1.2));
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        assert!(mean.abs() < 0.05, "{mean}");
    }

    #[test]
    fn normalized_fbm_sits_around_a_half() {
        let f = Fractal { detail: 2.5, roughness: 0.5, lacunarity: 2.0, offset: 0.0, gain: 1.0 };
        let values: Vec<f32> = (0..2000).map(|index| fbm(Domain::D3(Vec3::new(index as f32 * 0.31, 0.7, index as f32 * 0.11)), &f, true)).collect();
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        assert!((mean - 0.5).abs() < 0.03, "{mean}");
        assert!(values.iter().all(|value| (-0.1..=1.1).contains(value)));
    }
}
