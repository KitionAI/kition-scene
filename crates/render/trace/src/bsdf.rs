//! The surface model: a dielectric-or-metal surface with a GGX microfacet specular layer over a
//! Lambertian base, as glTF's metallic-roughness model and the reference's principled surface
//! describe it. Directions are in the shading frame (normal along +Z).

use std::f32::consts::PI;

use glam::{Vec2, Vec3};

use crate::sampling::{cosine_hemisphere, cosine_pdf};

/// A surface point's material.
#[derive(Debug, Clone, Copy)]
pub struct Surface {
    pub base: Vec3,
    pub metallic: f32,
    pub roughness: f32,
}

/// A sampled direction with the surface's response and the density it was drawn with.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub direction: Vec3,
    /// BSDF × cosine ÷ density.
    pub weight: Vec3,
    pub pdf: f32,
}

fn schlick(f0: Vec3, cos: f32) -> Vec3 {
    let m = (1.0 - cos.clamp(0.0, 1.0)).powi(5);
    f0 + (Vec3::ONE - f0) * m
}

fn luminance(c: Vec3) -> f32 {
    c.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

impl Surface {
    fn alpha(&self) -> f32 {
        (self.roughness.clamp(0.0, 1.0) * self.roughness.clamp(0.0, 1.0)).max(1e-3)
    }

    fn f0(&self) -> Vec3 {
        Vec3::splat(0.04).lerp(self.base, self.metallic.clamp(0.0, 1.0))
    }

    /// The share of samples spent on the specular lobe.
    fn specular_chance(&self, cos_out: f32) -> f32 {
        let spec = luminance(schlick(self.f0(), cos_out));
        let diffuse = luminance(self.base) * (1.0 - self.metallic.clamp(0.0, 1.0));
        (spec / (spec + diffuse).max(1e-6)).clamp(0.1, 0.9)
    }

    fn ggx_d(alpha: f32, cos_h: f32) -> f32 {
        let a2 = alpha * alpha;
        let d = cos_h * cos_h * (a2 - 1.0) + 1.0;
        a2 / (PI * d * d).max(1e-12)
    }

    fn smith_g1(alpha: f32, cos: f32) -> f32 {
        let cos = cos.max(1e-6);
        let a2 = alpha * alpha;
        2.0 * cos / (cos + (a2 + (1.0 - a2) * cos * cos).sqrt())
    }

    /// BSDF × cosine for light arriving along `incoming` and leaving along `outgoing`.
    ///
    /// The specular layer is single-scattering GGX plus Kulla and Conty's lobe for the light that
    /// scatters between microfacets before leaving; the base gets what the layer does not
    /// reflect towards the viewer (one minus its directional albedo with Fresnel).
    pub fn eval(&self, outgoing: Vec3, incoming: Vec3) -> Vec3 {
        let (cos_o, cos_i) = (outgoing.z, incoming.z);
        if cos_o <= 0.0 || cos_i <= 0.0 {
            return Vec3::ZERO;
        }
        let half = (outgoing + incoming).normalize_or_zero();
        let alpha = self.alpha();
        let f0 = self.f0();
        let fresnel = schlick(f0, outgoing.dot(half));
        let single = fresnel * (Self::ggx_d(alpha, half.z) * Self::smith_g1(alpha, cos_o) * Self::smith_g1(alpha, cos_i) / (4.0 * cos_o * cos_i));
        let (e_o, e_i, e_avg) =
            (directional_albedo(cos_o, self.roughness), directional_albedo(cos_i, self.roughness), average_albedo(self.roughness));
        let f_avg = f0 + (Vec3::ONE - f0) / 21.0;
        let multiple = if e_avg < 0.999 {
            let lobe = (1.0 - e_o) * (1.0 - e_i) / (PI * (1.0 - e_avg));
            f_avg * f_avg * e_avg / (Vec3::ONE - f_avg * (1.0 - e_avg)) * lobe
        } else {
            Vec3::ZERO
        };
        let coat = dielectric_albedo(cos_o, self.roughness);
        let diffuse = self.base * (1.0 - self.metallic.clamp(0.0, 1.0)) * (1.0 - coat) / PI;
        (single + multiple + diffuse) * cos_i
    }

    /// The diffuse base's part of [`Surface::eval`].
    pub fn diffuse(&self, outgoing: Vec3, incoming: Vec3) -> Vec3 {
        let (cos_o, cos_i) = (outgoing.z, incoming.z);
        if cos_o <= 0.0 || cos_i <= 0.0 {
            return Vec3::ZERO;
        }
        self.base * (1.0 - self.metallic.clamp(0.0, 1.0)) * (1.0 - dielectric_albedo(cos_o, self.roughness)) / PI * cos_i
    }

    /// The diffuse base's colour seen at `cos_out`: what the specular layer over it lets
    /// through.
    pub fn diffuse_color(&self, cos_out: f32) -> Vec3 {
        self.base * (1.0 - self.metallic.clamp(0.0, 1.0)) * (1.0 - dielectric_albedo(cos_out.max(0.0), self.roughness))
    }

    /// How much the specular layer reflects seen at `cos_out` (its colour, for the glossy
    /// passes).
    pub fn glossy_color(&self, cos_out: f32) -> Vec3 {
        schlick(self.f0(), cos_out) * directional_albedo(cos_out.max(0.02), self.roughness)
    }

    /// The density [`Surface::sample`] draws `incoming` with.
    pub fn pdf(&self, outgoing: Vec3, incoming: Vec3) -> f32 {
        if outgoing.z <= 0.0 || incoming.z <= 0.0 {
            return 0.0;
        }
        let half = (outgoing + incoming).normalize_or_zero();
        let alpha = self.alpha();
        // Visible-normal sampling: D_v(h) / (4 o·h) = G1(o) D(h) / (4 cos_o).
        let specular = Self::smith_g1(alpha, outgoing.z) * Self::ggx_d(alpha, half.z) / (4.0 * outgoing.z);
        let chance = self.specular_chance(outgoing.z);
        chance * specular + (1.0 - chance) * cosine_pdf(incoming.z)
    }

    /// Draws a direction for light to arrive from.
    pub fn sample(&self, outgoing: Vec3, choice: f32, u: Vec2) -> Option<Sample> {
        if outgoing.z <= 0.0 {
            return None;
        }
        let incoming = if choice < self.specular_chance(outgoing.z) {
            let half = sample_visible_normal(outgoing, self.alpha(), u);
            let reflected = half * (2.0 * outgoing.dot(half)) - outgoing;
            if reflected.z <= 0.0 {
                return None;
            }
            reflected
        } else {
            cosine_hemisphere(u)
        };
        let pdf = self.pdf(outgoing, incoming);
        if pdf <= 1e-8 {
            return None;
        }
        Some(Sample { direction: incoming, weight: self.eval(outgoing, incoming) / pdf, pdf })
    }
}

const TABLE: usize = 32;

/// The specular layer's directional albedo with a white Fresnel term (single scattering),
/// tabulated over the cosine of the outgoing angle and roughness by integrating the lobe once:
/// with visible-normal samples, each sample's weight is G1 of the reflected direction.
fn directional_albedo(cos_out: f32, roughness: f32) -> f32 {
    static TABLE_DATA: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    let table = TABLE_DATA.get_or_init(|| tabulate(|_, _, incoming, alpha| Surface::smith_g1(alpha, incoming.z)));
    lookup(table, cos_out, roughness)
}

/// The cosine-weighted average of [`directional_albedo`] over the hemisphere.
fn average_albedo(roughness: f32) -> f32 {
    let steps = 32;
    (0..steps)
        .map(|step| {
            let cos = (step as f32 + 0.5) / steps as f32;
            2.0 * cos * directional_albedo(cos, roughness) / steps as f32
        })
        .sum()
}

/// The directional albedo of a dielectric specular layer (F0 = 0.04, single scattering with
/// Fresnel): the share of light the layer reflects, which the base under it does not get.
fn dielectric_albedo(cos_out: f32, roughness: f32) -> f32 {
    static TABLE_DATA: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    let table = TABLE_DATA.get_or_init(|| {
        tabulate(|outgoing, half, incoming, alpha| schlick(Vec3::splat(0.04), outgoing.dot(half)).x * Surface::smith_g1(alpha, incoming.z))
    });
    lookup(table, cos_out, roughness)
}

fn tabulate(weight: impl Fn(Vec3, Vec3, Vec3, f32) -> f32) -> Vec<f32> {
    let mut out = Vec::with_capacity(TABLE * TABLE);
    let side = 16u32;
    for r in 0..TABLE {
        let roughness = r as f32 / (TABLE - 1) as f32;
        let alpha = (roughness * roughness).max(1e-3);
        for c in 0..TABLE {
            let cos = (c as f32 / (TABLE - 1) as f32).max(0.02);
            let outgoing = Vec3::new((1.0 - cos * cos).sqrt(), 0.0, cos);
            let mut sum = 0.0;
            for i in 0..side {
                for j in 0..side {
                    let u = Vec2::new((i as f32 + 0.5) / side as f32, (j as f32 + 0.5) / side as f32);
                    let half = sample_visible_normal(outgoing, alpha, u);
                    let incoming = half * (2.0 * outgoing.dot(half)) - outgoing;
                    if incoming.z > 0.0 {
                        sum += weight(outgoing, half, incoming, alpha);
                    }
                }
            }
            out.push(sum / (side * side) as f32);
        }
    }
    out
}

fn lookup(table: &[f32], cos_out: f32, roughness: f32) -> f32 {
    let at = |r: usize, c: usize| table.get(r.min(TABLE - 1) * TABLE + c.min(TABLE - 1)).copied().unwrap_or(1.0);
    let (fr, fc) = (roughness.clamp(0.0, 1.0) * (TABLE - 1) as f32, cos_out.clamp(0.0, 1.0) * (TABLE - 1) as f32);
    let (r0, c0) = (fr.floor() as usize, fc.floor() as usize);
    let (tr, tc) = (fr - r0 as f32, fc - c0 as f32);
    let top = at(r0, c0) * (1.0 - tc) + at(r0, c0 + 1) * tc;
    let bottom = at(r0 + 1, c0) * (1.0 - tc) + at(r0 + 1, c0 + 1) * tc;
    top * (1.0 - tr) + bottom * tr
}

/// Heitz's sampling of the GGX distribution of visible normals.
fn sample_visible_normal(outgoing: Vec3, alpha: f32, u: Vec2) -> Vec3 {
    let stretched = Vec3::new(alpha * outgoing.x, alpha * outgoing.y, outgoing.z).normalize_or_zero();
    let length2 = stretched.x * stretched.x + stretched.y * stretched.y;
    let t1 = if length2 > 0.0 { Vec3::new(-stretched.y, stretched.x, 0.0) / length2.sqrt() } else { Vec3::X };
    let t2 = stretched.cross(t1);
    let r = u.x.sqrt();
    let phi = 2.0 * PI * u.y;
    let p1 = r * phi.cos();
    let s = 0.5 * (1.0 + stretched.z);
    let p2 = (1.0 - s) * (1.0 - p1 * p1).max(0.0).sqrt() + s * r * phi.sin();
    let n = t1 * p1 + t2 * p2 + stretched * (1.0 - p1 * p1 - p2 * p2).max(0.0).sqrt();
    Vec3::new(alpha * n.x, alpha * n.y, n.z.max(1e-6)).normalize_or_zero()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampling::Rng;

    /// Estimates the albedo (reflected share of uniform light) for a direction.
    fn albedo(surface: &Surface, outgoing: Vec3, count: u32) -> Vec3 {
        let mut rng = Rng::for_sample(7, 1, 0);
        let mut sum = Vec3::ZERO;
        for _ in 0..count {
            if let Some(sample) = surface.sample(outgoing, rng.uniform(), rng.next2()) {
                sum += sample.weight;
            }
        }
        sum / count as f32
    }

    #[test]
    fn a_white_surface_never_reflects_more_than_it_gets() {
        for roughness in [0.05, 0.3, 0.5, 1.0] {
            for metallic in [0.0, 1.0] {
                let surface = Surface { base: Vec3::ONE, metallic, roughness };
                for cos in [0.2f32, 0.6, 1.0] {
                    let outgoing = Vec3::new((1.0 - cos * cos).sqrt(), 0.0, cos);
                    let a = albedo(&surface, outgoing, 40_000);
                    assert!(a.max_element() <= 1.02, "roughness {roughness} metallic {metallic} cos {cos}: {a}");
                    assert!(a.min_element() >= 0.6, "roughness {roughness} metallic {metallic} cos {cos}: {a}");
                }
            }
        }
    }

    #[test]
    fn sampling_density_matches_eval() {
        // E[eval / pdf] over the sampler equals the sampler's own weight average.
        let surface = Surface { base: Vec3::splat(0.5), metallic: 0.3, roughness: 0.4 };
        let outgoing = Vec3::new(0.3, 0.1, 0.95).normalize();
        let mut rng = Rng::for_sample(3, 2, 1);
        for _ in 0..1000 {
            if let Some(sample) = surface.sample(outgoing, rng.uniform(), rng.next2()) {
                let direct = surface.eval(outgoing, sample.direction) / surface.pdf(outgoing, sample.direction);
                assert!((direct - sample.weight).abs().max_element() < 1e-3);
            }
        }
    }
}
