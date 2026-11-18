//! The world as an image: an equirectangular picture all around the scene, looked up by
//! direction the way the reference maps one (`u` turning clockwise seen from above from −X, `v`
//! from the nadir to the zenith), turned about the up axis. It gives the path tracer an
//! importance-sampled light (rows and columns by brightness × the solid angle a texel covers),
//! and the fast renderer an irradiance for ambient light (nine spherical-harmonic coefficients).

use std::f32::consts::PI;

use glam::{Mat3, Vec2, Vec3};
use kition_scene_model::Scene;

use crate::texture::decode_linear;

/// The largest image side kept for lighting; bigger images are box-reduced.
const MAX_LIGHT_SIDE: usize = 1024;

/// A world image ready for lookups and sampling.
#[derive(Debug)]
pub struct Environment {
    width: usize,
    height: usize,
    /// Linear radiance per texel, top row first, times the world's strength.
    texels: Vec<Vec3>,
    /// The world's turn about the up axis: lookups turn directions by it.
    turn: Mat3,
    /// Row choice: running sums of each row's weight, normalised (length `height`).
    rows: Vec<f32>,
    /// Texel choice within a row: running sums, normalised per row (`width × height`).
    columns: Vec<f32>,
    /// The sum of all texel weights, for densities.
    total: f32,
    /// Irradiance coefficients (order 2), before the turn.
    sh: [Vec3; 9],
}

/// A direction drawn towards the world, with its radiance and density per solid angle.
#[derive(Debug, Clone, Copy)]
pub struct EnvironmentSample {
    pub direction: Vec3,
    pub radiance: Vec3,
    pub pdf: f32,
}

impl Environment {
    /// The scene's world image, when it has one that can be read.
    pub fn of(scene: &Scene) -> Option<Self> {
        let image = scene.image(scene.world.image.as_deref()?)?;
        let (width, height, texels) = decode_linear(image, true)?;
        let texels: Vec<Vec3> = texels.into_iter().map(|texel| texel.truncate().max(Vec3::ZERO)).collect();
        Some(Self::new(width, height, texels, scene.world.strength.max(0.0), scene.world.rotation))
    }

    /// An environment from linear texels (top row first).
    pub fn new(width: usize, height: usize, texels: Vec<Vec3>, strength: f32, rotation: f32) -> Self {
        let (width, height, texels) = reduce(width, height, texels);
        let texels: Vec<Vec3> = texels.into_iter().map(|texel| if texel.is_finite() { texel * strength } else { Vec3::ZERO }).collect();
        let mut rows = Vec::with_capacity(height);
        let mut columns = Vec::with_capacity(width * height);
        let mut total = 0.0f32;
        for y in 0..height {
            let sin = (PI * (y as f32 + 0.5) / height as f32).sin();
            let mut running = 0.0f32;
            for x in 0..width {
                let texel = texels.get(y * width + x).copied().unwrap_or(Vec3::ZERO);
                running += luminance(texel) * sin + 1e-6;
                columns.push(running);
            }
            if let Some(row) = columns.get_mut(y * width..(y + 1) * width) {
                for value in row.iter_mut() {
                    *value /= running.max(1e-12);
                }
            }
            total += running;
            rows.push(total);
        }
        for value in &mut rows {
            *value /= total.max(1e-12);
        }
        let sh = irradiance_coefficients(width, height, &texels);
        Self { width, height, texels, turn: Mat3::from_rotation_z(rotation), rows, columns, total, sh }
    }

    /// Where a world direction lands in the image (u, v in 0..1, v up).
    fn uv(&self, direction: Vec3) -> Vec2 {
        let d = (self.turn * direction).normalize_or_zero();
        let u = 0.5 - d.y.atan2(d.x) / (2.0 * PI);
        let v = 0.5 + d.z.clamp(-1.0, 1.0).asin() / PI;
        Vec2::new(u.rem_euclid(1.0), v.clamp(0.0, 1.0))
    }

    /// The world direction at image coordinates (the inverse of [`Environment::uv`]).
    fn direction(&self, uv: Vec2) -> Vec3 {
        let phi = (0.5 - uv.x) * 2.0 * PI;
        let theta = (uv.y - 0.5) * PI;
        let local = Vec3::new(theta.cos() * phi.cos(), theta.cos() * phi.sin(), theta.sin());
        self.turn.transpose() * local
    }

    /// The image's size, after any reduction for lighting.
    pub fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// Linear radiance per texel (strength included), top row first.
    pub fn texels(&self) -> &[Vec3] {
        &self.texels
    }

    /// The world's turn about the up axis (lookups turn directions by it).
    pub fn turn(&self) -> Mat3 {
        self.turn
    }

    /// The irradiance's nine spherical-harmonic coefficients, before the turn.
    pub fn coefficients(&self) -> [Vec3; 9] {
        self.sh
    }

    /// The radiance arriving from `direction` (bilinear).
    pub fn radiance(&self, direction: Vec3) -> Vec3 {
        let uv = self.uv(direction);
        let x = uv.x * self.width as f32 - 0.5;
        let y = (1.0 - uv.y) * self.height as f32 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let at = |tx: f32, ty: f32| {
            let tx = (tx as i64).rem_euclid(self.width as i64) as usize;
            let ty = (ty as i64).clamp(0, self.height as i64 - 1) as usize;
            self.texels.get(ty * self.width + tx).copied().unwrap_or(Vec3::ZERO)
        };
        let top = at(x0, y0).lerp(at(x0 + 1.0, y0), fx);
        let bottom = at(x0, y0 + 1.0).lerp(at(x0 + 1.0, y0 + 1.0), fx);
        top.lerp(bottom, fy)
    }

    /// The radiance of the texel `direction` falls in (what sampling assumes).
    fn texel(&self, direction: Vec3) -> (usize, usize) {
        let uv = self.uv(direction);
        let x = ((uv.x * self.width as f32) as usize).min(self.width - 1);
        let y = (((1.0 - uv.y) * self.height as f32) as usize).min(self.height - 1);
        (x, y)
    }

    /// The density (per solid angle) with which [`Environment::sample`] draws `direction`.
    pub fn pdf(&self, direction: Vec3) -> f32 {
        let (x, y) = self.texel(direction);
        self.texel_pdf(x, y)
    }

    fn texel_pdf(&self, x: usize, y: usize) -> f32 {
        let sin = (PI * (y as f32 + 0.5) / self.height as f32).sin();
        if sin <= 1e-6 || self.total <= 0.0 {
            return 0.0;
        }
        let weight = luminance(self.texels.get(y * self.width + x).copied().unwrap_or(Vec3::ZERO)) * sin + 1e-6;
        // Texel probability over its solid angle: (2π/w)(π/h) sin θ.
        let probability = weight / self.total;
        probability * (self.width * self.height) as f32 / (2.0 * PI * PI * sin)
    }

    /// Draws a direction towards the world by brightness.
    pub fn sample(&self, u: Vec2) -> Option<EnvironmentSample> {
        let y = self.rows.partition_point(|value| *value < u.x).min(self.height - 1);
        let row = self.columns.get(y * self.width..(y + 1) * self.width)?;
        let x = row.partition_point(|value| *value < u.y).min(self.width - 1);
        // A point inside the texel (jitter from the leftover of the choices keeps it continuous).
        let row_low = if y == 0 { 0.0 } else { self.rows.get(y - 1).copied().unwrap_or(0.0) };
        let row_high = self.rows.get(y).copied().unwrap_or(1.0);
        let column_low = if x == 0 { 0.0 } else { row.get(x - 1).copied().unwrap_or(0.0) };
        let column_high = row.get(x).copied().unwrap_or(1.0);
        let fy = ((u.x - row_low) / (row_high - row_low).max(1e-12)).clamp(0.0, 0.999_999);
        let fx = ((u.y - column_low) / (column_high - column_low).max(1e-12)).clamp(0.0, 0.999_999);
        let uv = Vec2::new((x as f32 + fx) / self.width as f32, 1.0 - (y as f32 + fy) / self.height as f32);
        let direction = self.direction(uv).normalize_or_zero();
        let pdf = self.texel_pdf(x, y);
        if pdf <= 0.0 {
            return None;
        }
        Some(EnvironmentSample { direction, radiance: self.texels.get(y * self.width + x).copied().unwrap_or(Vec3::ZERO), pdf })
    }

    /// The irradiance (incoming light on a surface facing `normal`) divided by π: the colour a
    /// white diffuse surface reflects under this world.
    pub fn diffuse(&self, normal: Vec3) -> Vec3 {
        let n = (self.turn * normal).normalize_or_zero();
        let (x, y, z) = (n.x, n.y, n.z);
        let c = &self.sh;
        // Ramamoorthi and Hanrahan's irradiance from the nine coefficients.
        let (c1, c2, c3, c4, c5) = (0.429_043, 0.511_664, 0.743_125, 0.886_227, 0.247_708);
        let e = c[8] * (c1 * (x * x - y * y)) + c[6] * (c3 * z * z) + c[0] * c4 - c[6] * c5
            + (c[4] * (x * y) + c[7] * (x * z) + c[5] * (y * z)) * (2.0 * c1)
            + (c[3] * x + c[1] * y + c[2] * z) * (2.0 * c2);
        (e / PI).max(Vec3::ZERO)
    }
}

fn luminance(c: Vec3) -> f32 {
    c.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

/// Halves the image until it fits the lighting size.
fn reduce(mut width: usize, mut height: usize, mut texels: Vec<Vec3>) -> (usize, usize, Vec<Vec3>) {
    while width > MAX_LIGHT_SIDE || height > MAX_LIGHT_SIDE {
        let (w, h) = ((width / 2).max(1), (height / 2).max(1));
        let mut next = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let mut sum = Vec3::ZERO;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    sum += texels.get((y * 2 + dy).min(height - 1) * width + (x * 2 + dx).min(width - 1)).copied().unwrap_or(Vec3::ZERO);
                }
                next.push(sum / 4.0);
            }
        }
        (width, height, texels) = (w, h, next);
    }
    if texels.len() != width * height || width == 0 || height == 0 {
        return (1, 1, vec![Vec3::ZERO]);
    }
    (width, height, texels)
}

/// The order-2 spherical-harmonic projection of the radiance (unturned frame).
fn irradiance_coefficients(width: usize, height: usize, texels: &[Vec3]) -> [Vec3; 9] {
    let mut c = [Vec3::ZERO; 9];
    for y in 0..height {
        let theta = ((y as f32 + 0.5) / height as f32 - 0.5) * PI;
        let solid = (2.0 * PI / width as f32) * (PI / height as f32) * theta.cos();
        for x in 0..width {
            let phi = (0.5 - (x as f32 + 0.5) / width as f32) * 2.0 * PI;
            let (dx, dy, dz) = (theta.cos() * phi.cos(), theta.cos() * phi.sin(), theta.sin());
            let radiance = texels.get(y * width + x).copied().unwrap_or(Vec3::ZERO) * solid;
            let basis = [
                0.282_095,
                0.488_603 * dy,
                0.488_603 * dz,
                0.488_603 * dx,
                1.092_548 * dx * dy,
                1.092_548 * dy * dz,
                0.315_392 * (3.0 * dz * dz - 1.0),
                1.092_548 * dx * dz,
                0.546_274 * (dx * dx - dy * dy),
            ];
            for (slot, value) in c.iter_mut().zip(basis) {
                *slot += radiance * value;
            }
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniform(value: f32) -> Environment {
        Environment::new(64, 32, vec![Vec3::splat(value); 64 * 32], 1.0, 0.0)
    }

    #[test]
    fn directions_and_coordinates_invert() {
        let environment = Environment::new(64, 32, vec![Vec3::ONE; 64 * 32], 1.0, 0.7);
        for direction in [Vec3::X, Vec3::Y, Vec3::new(0.3, -0.5, 0.8).normalize(), Vec3::new(-1.0, 0.2, -0.4).normalize()] {
            let back = environment.direction(environment.uv(direction));
            assert!(back.abs_diff_eq(direction, 1e-4), "{direction} {back}");
        }
        // The image's middle column faces −X; the top row is the zenith.
        let plain = uniform(1.0);
        assert!((plain.uv(Vec3::NEG_X).x - 0.0).abs() < 1e-5 || (plain.uv(Vec3::NEG_X).x - 1.0).abs() < 1e-5);
        assert!((plain.uv(Vec3::X).x - 0.5).abs() < 1e-5);
        assert!((plain.uv(Vec3::Y).x - 0.25).abs() < 1e-5);
        assert!((plain.uv(Vec3::Z).y - 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_uniform_world_lights_like_its_colour() {
        let environment = uniform(0.5);
        for normal in [Vec3::Z, Vec3::X, Vec3::new(0.2, 0.4, -0.9).normalize()] {
            assert!((environment.diffuse(normal) - Vec3::splat(0.5)).abs().max_element() < 0.01, "{}", environment.diffuse(normal));
        }
    }

    #[test]
    fn sampling_densities_integrate_to_one_and_favour_the_bright_part() {
        // A bright patch in one quarter of the sky.
        let mut texels = vec![Vec3::splat(0.1); 64 * 32];
        for y in 4..10 {
            for x in 40..50 {
                texels[y * 64 + x] = Vec3::splat(20.0);
            }
        }
        let environment = Environment::new(64, 32, texels, 1.0, 0.0);
        // ∫ pdf dω over a fine grid of directions.
        let mut integral = 0.0;
        let (nu, nv) = (256, 128);
        for j in 0..nv {
            let theta = ((j as f32 + 0.5) / nv as f32 - 0.5) * PI;
            for i in 0..nu {
                let phi = (i as f32 + 0.5) / nu as f32 * 2.0 * PI;
                let d = Vec3::new(theta.cos() * phi.cos(), theta.cos() * phi.sin(), theta.sin());
                integral += environment.pdf(d) * (2.0 * PI / nu as f32) * (PI / nv as f32) * theta.cos();
            }
        }
        assert!((integral - 1.0).abs() < 0.03, "{integral}");
        let mut bright = 0;
        let mut rng = 12345u32;
        for _ in 0..2000 {
            rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let a = (rng >> 8) as f32 / (1u32 << 24) as f32;
            rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let b = (rng >> 8) as f32 / (1u32 << 24) as f32;
            let sample = environment.sample(Vec2::new(a, b)).unwrap();
            if sample.radiance.x > 10.0 {
                bright += 1;
            }
            assert!((environment.pdf(sample.direction) - sample.pdf).abs() <= sample.pdf * 0.01 + 1e-6);
        }
        assert!(bright > 1500, "{bright}");
    }
}
