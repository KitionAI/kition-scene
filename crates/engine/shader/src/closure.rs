//! Surfaces: what a shader group gives a point. A closure is a weighted sum of scattering lobes
//! (the Principled BSDF's layers, diffuse, glossy, and glass) with light it sends out and light it
//! lets straight through; Mix Shader blends two, Add Shader sums them. Renderers draw the lobes
//! (the path tracer) or one principled surface that stands for them ([`Closure::flatten`]).

use glam::Vec3;

/// The Principled BSDF's inputs at a point. Normals and tangents are in world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Principled {
    pub base: Vec3,
    pub metallic: f32,
    pub roughness: f32,
    pub ior: f32,
    pub normal: Vec3,
    pub subsurface: f32,
    pub subsurface_radius: Vec3,
    pub subsurface_scale: f32,
    pub specular_level: f32,
    pub specular_tint: Vec3,
    pub anisotropic: f32,
    pub anisotropic_rotation: f32,
    pub tangent: Vec3,
    pub transmission: f32,
    pub coat: f32,
    pub coat_roughness: f32,
    pub coat_ior: f32,
    pub coat_tint: Vec3,
    pub coat_normal: Vec3,
    pub sheen: f32,
    pub sheen_roughness: f32,
    pub sheen_tint: Vec3,
}

impl Principled {
    /// The dielectric reflectance at normal incidence: the IOR's, scaled by twice the specular
    /// level and tinted, as the reference's specular inputs work.
    pub fn f0(&self) -> Vec3 {
        let ior = if self.ior.is_finite() && self.ior > 0.0 { self.ior } else { 1.5 };
        let f0 = ((ior - 1.0) / (ior + 1.0)).powi(2);
        (self.specular_tint * f0 * (2.0 * self.specular_level).max(0.0)).min(Vec3::ONE)
    }
}

/// One scattering lobe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Lobe {
    Principled(Principled),
    /// Lambertian (Oren–Nayar's roughness is not modelled).
    Diffuse {
        color: Vec3,
        roughness: f32,
        normal: Vec3,
    },
    /// A metal-like GGX reflection tinted by `color`.
    Glossy {
        color: Vec3,
        roughness: f32,
        anisotropy: f32,
        rotation: f32,
        normal: Vec3,
        tangent: Vec3,
    },
    /// A smooth or rough dielectric that reflects and refracts.
    Glass {
        color: Vec3,
        roughness: f32,
        ior: f32,
        normal: Vec3,
    },
}

/// How a volume scatters light around the direction it was going.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    /// Henyey–Greenstein with anisotropy `g` (−1 back, 0 even, 1 forward).
    HenyeyGreenstein(f32),
    /// Rayleigh: (1 + cos²θ) shaped, as small particles scatter.
    Rayleigh,
}

/// One part of a volume: per metre, how much light it absorbs and scatters (per channel), and
/// how it scatters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolumeLobe {
    pub absorption: Vec3,
    pub scattering: Vec3,
    pub phase: Phase,
}

/// A surface or a volume: lobes with their weights, light sent out, and light let through.
/// Volume nodes give `volume` lobes, and their light (per metre) is `emission`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Closure {
    pub lobes: Vec<(f32, Lobe)>,
    pub emission: Vec3,
    /// What passes straight through (Transparent BSDF, the Principled BSDF's alpha).
    pub transparent: Vec3,
    pub volume: Vec<VolumeLobe>,
}

/// Both closures together (the Add Shader node).
impl std::ops::Add for Closure {
    type Output = Self;

    fn add(mut self, other: Self) -> Self {
        self.lobes.extend(other.lobes);
        self.volume.extend(other.volume);
        self.emission += other.emission;
        self.transparent += other.transparent;
        self
    }
}

impl Closure {
    pub fn lobe(lobe: Lobe) -> Self {
        Self { lobes: vec![(1.0, lobe)], ..Self::default() }
    }

    /// The closure scaled by `factor`.
    pub fn scaled(mut self, factor: f32) -> Self {
        for (weight, _) in &mut self.lobes {
            *weight *= factor;
        }
        self.emission *= factor;
        self.transparent *= factor;
        self.lobes.retain(|(weight, _)| *weight > 0.0);
        let factor = factor.max(0.0);
        for lobe in &mut self.volume {
            lobe.absorption *= factor;
            lobe.scattering *= factor;
        }
        self.volume.retain(|lobe| lobe.absorption.max_element() > 0.0 || lobe.scattering.max_element() > 0.0);
        self
    }

    /// How much of the surface is there: one less what passes straight through.
    pub fn coverage(&self) -> f32 {
        (1.0 - (self.transparent.x + self.transparent.y + self.transparent.z) / 3.0).clamp(0.0, 1.0)
    }

    /// One principled surface that stands for the lobes (their weighted average), for renderers
    /// that draw one.
    pub fn flatten(&self) -> Flat {
        let total: f32 = self.lobes.iter().map(|(weight, _)| *weight).sum();
        let mut flat = Flat { emission: self.emission, alpha: self.coverage(), ..Flat::default() };
        if total <= 0.0 {
            flat.base = Vec3::ZERO;
            return flat;
        }
        let (mut base, mut metallic, mut roughness, mut transmission, mut ior, mut normal) = (Vec3::ZERO, 0.0, 0.0, 0.0, 0.0, Vec3::ZERO);
        for (weight, lobe) in &self.lobes {
            let share = weight / total;
            let (b, m, r, t, i, n) = match lobe {
                Lobe::Principled(p) => (p.base, p.metallic, p.roughness, p.transmission, p.ior, p.normal),
                Lobe::Diffuse { color, normal, .. } => (*color, 0.0, 1.0, 0.0, 1.5, *normal),
                Lobe::Glossy { color, roughness, normal, .. } => (*color, 1.0, *roughness, 0.0, 1.5, *normal),
                Lobe::Glass { color, roughness, ior, normal } => (*color, 0.0, *roughness, 1.0, *ior, *normal),
            };
            base += b * share;
            metallic += m * share;
            roughness += r * share;
            transmission += t * share;
            ior += i * share;
            normal += n * share;
        }
        // Lobes with less weight than the whole darken the surface.
        flat.base = base * total.min(1.0);
        flat.metallic = metallic;
        flat.roughness = roughness;
        flat.transmission = transmission;
        flat.ior = ior;
        flat.normal = normal.normalize_or_zero();
        flat
    }
}

/// A principled surface standing for a closure.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flat {
    pub base: Vec3,
    pub metallic: f32,
    pub roughness: f32,
    pub emission: Vec3,
    pub alpha: f32,
    pub transmission: f32,
    pub ior: f32,
    /// World normal (zero when the lobes have none).
    pub normal: Vec3,
}

impl Default for Flat {
    fn default() -> Self {
        Self {
            base: Vec3::splat(0.8),
            metallic: 0.0,
            roughness: 0.5,
            emission: Vec3::ZERO,
            alpha: 1.0,
            transmission: 0.0,
            ior: 1.5,
            normal: Vec3::ZERO,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixing_halves_weights_and_flattening_averages() {
        let red = Closure::lobe(Lobe::Diffuse { color: Vec3::X, roughness: 0.0, normal: Vec3::Z });
        let gold = Closure::lobe(Lobe::Glossy {
            color: Vec3::new(1.0, 0.8, 0.0),
            roughness: 0.2,
            anisotropy: 0.0,
            rotation: 0.0,
            normal: Vec3::Z,
            tangent: Vec3::X,
        });
        let mixed = red.scaled(0.5) + gold.scaled(0.5);
        let flat = mixed.flatten();
        assert!((flat.base - Vec3::new(1.0, 0.4, 0.0)).length() < 1e-6);
        assert!((flat.metallic - 0.5).abs() < 1e-6);
        let clear = Closure { transparent: Vec3::ONE, ..Closure::default() };
        assert_eq!(clear.coverage(), 0.0);
        assert_eq!(clear.flatten().base, Vec3::ZERO);
    }
}
