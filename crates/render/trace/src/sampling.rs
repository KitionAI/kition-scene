//! Random numbers and the warps that turn them into directions and points.

use glam::{Vec2, Vec3};
use std::f32::consts::{PI, TAU};

/// A PCG32 generator: small, fast, and the same everywhere, so a render is reproducible.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
    increment: u64,
}

impl Rng {
    /// A generator for one pixel's sample `index` of a render seeded with `seed`.
    pub fn for_sample(seed: u64, pixel: u64, index: u64) -> Self {
        let stream = mix(pixel ^ mix(seed));
        let mut rng = Self { state: 0, increment: (stream << 1) | 1 };
        rng.next_u32();
        rng.state = rng.state.wrapping_add(mix(index.wrapping_add(seed.rotate_left(17))));
        rng.next_u32();
        rng
    }

    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(self.increment);
        let shifted = (((old >> 18) ^ old) >> 27) as u32;
        let rotation = (old >> 59) as u32;
        shifted.rotate_right(rotation)
    }

    /// A number in [0, 1).
    pub fn uniform(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0 / 16_777_216.0)
    }

    pub fn next2(&mut self) -> Vec2 {
        Vec2::new(self.uniform(), self.uniform())
    }
}

/// SplitMix64's finaliser: spreads nearby integers far apart.
pub fn mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

/// An orthonormal frame around a unit normal.
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    pub tangent: Vec3,
    pub bitangent: Vec3,
    pub normal: Vec3,
}

impl Frame {
    pub fn new(normal: Vec3) -> Self {
        let (tangent, bitangent) = normal.any_orthonormal_pair();
        Self { tangent, bitangent, normal }
    }

    pub fn to_local(&self, v: Vec3) -> Vec3 {
        Vec3::new(v.dot(self.tangent), v.dot(self.bitangent), v.dot(self.normal))
    }

    pub fn to_world(&self, v: Vec3) -> Vec3 {
        self.tangent * v.x + self.bitangent * v.y + self.normal * v.z
    }
}

/// A cosine-weighted direction on the +Z hemisphere.
pub fn cosine_hemisphere(u: Vec2) -> Vec3 {
    let radius = u.x.sqrt();
    let angle = TAU * u.y;
    Vec3::new(radius * angle.cos(), radius * angle.sin(), (1.0 - u.x).max(0.0).sqrt())
}

/// A uniform direction inside the cone around +Z whose half-angle has cosine `cos_max`.
pub fn uniform_cone(u: Vec2, cos_max: f32) -> Vec3 {
    let cos = 1.0 - u.x * (1.0 - cos_max);
    let sin = (1.0 - cos * cos).max(0.0).sqrt();
    let angle = TAU * u.y;
    Vec3::new(sin * angle.cos(), sin * angle.sin(), cos)
}

/// The solid-angle density of [`uniform_cone`].
pub fn cone_pdf(cos_max: f32) -> f32 {
    1.0 / (TAU * (1.0 - cos_max)).max(1e-12)
}

/// A point on the unit disk, evenly spread (Shirley and Chiu's concentric mapping).
pub fn concentric_disk(u: Vec2) -> Vec2 {
    let offset = u * 2.0 - Vec2::ONE;
    if offset == Vec2::ZERO {
        return Vec2::ZERO;
    }
    let (radius, theta) = if offset.x.abs() > offset.y.abs() {
        (offset.x, std::f32::consts::FRAC_PI_4 * (offset.y / offset.x))
    } else {
        (offset.y, std::f32::consts::FRAC_PI_2 - std::f32::consts::FRAC_PI_4 * (offset.x / offset.y))
    };
    Vec2::new(theta.cos(), theta.sin()) * radius
}

/// The power heuristic (β = 2) weight of a strategy with density `a` against one with `b`.
pub fn power_heuristic(a: f32, b: f32) -> f32 {
    let (a2, b2) = (a * a, b * b);
    if a2 + b2 <= 0.0 { 0.0 } else { a2 / (a2 + b2) }
}

/// Cosine-weighted density.
pub fn cosine_pdf(cos: f32) -> f32 {
    cos.max(0.0) / PI
}
