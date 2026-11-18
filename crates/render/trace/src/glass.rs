//! Glass: a rough dielectric that reflects and refracts through GGX microfacets (Walter et
//! al.'s model with visible-normal sampling), and the layered material that mixes it into the
//! principled surface by the material's transmission, as the reference's principled surface does
//! (transmission applies to the dielectric part, tinted by the base colour). Directions are in the
//! shading frame, with the outgoing direction on the +Z side; incoming directions below the
//! surface are transmitted.

use std::f32::consts::PI;

use glam::{Vec2, Vec3};

use crate::bsdf::{Sample, Surface};

/// A rough dielectric boundary. `eta` is the index of the side light goes into over the side
/// the outgoing direction is on.
#[derive(Debug, Clone, Copy)]
pub struct Dielectric {
    pub eta: f32,
    pub roughness: f32,
    /// Transmitted light is tinted by this (the base colour).
    pub tint: Vec3,
}

/// Fresnel reflectance of a dielectric for the cosine on the incident side and the relative
/// index (`eta` = transmitted over incident); 1 under total internal reflection.
pub fn fresnel(cos_incident: f32, eta: f32) -> f32 {
    let cos_i = cos_incident.clamp(-1.0, 1.0);
    let (cos_i, eta) = if cos_i < 0.0 { (-cos_i, 1.0 / eta) } else { (cos_i, eta) };
    let sin2_t = (1.0 - cos_i * cos_i) / (eta * eta);
    if sin2_t >= 1.0 {
        return 1.0;
    }
    let cos_t = (1.0 - sin2_t).max(0.0).sqrt();
    let parallel = (eta * cos_i - cos_t) / (eta * cos_i + cos_t);
    let perpendicular = (cos_i - eta * cos_t) / (cos_i + eta * cos_t);
    0.5 * (parallel * parallel + perpendicular * perpendicular)
}

fn ggx_d(alpha: f32, cos_h: f32) -> f32 {
    let a2 = alpha * alpha;
    let d = cos_h * cos_h * (a2 - 1.0) + 1.0;
    a2 / (PI * d * d).max(1e-20)
}

fn smith_g1(alpha: f32, cos: f32) -> f32 {
    let cos = cos.abs().max(1e-6);
    let a2 = alpha * alpha;
    2.0 * cos / (cos + (a2 + (1.0 - a2) * cos * cos).sqrt())
}

/// Heitz's sampling of visible normals for an outgoing direction in the upper hemisphere.
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

impl Dielectric {
    fn alpha(&self) -> f32 {
        (self.roughness.clamp(0.0, 1.0).powi(2)).max(1e-3)
    }

    /// The half vector of a pair, in the outgoing side's hemisphere, and the relative index
    /// used (1 for reflection).
    fn half(&self, outgoing: Vec3, incoming: Vec3) -> Option<(Vec3, f32)> {
        let reflect = incoming.z > 0.0;
        let etap = if reflect { 1.0 } else { self.eta };
        let mut half = incoming * etap + outgoing;
        if half.length_squared() < 1e-12 {
            return None;
        }
        half = half.normalize();
        if half.z < 0.0 {
            half = -half;
        }
        // Microfacets seen from behind give nothing.
        if half.dot(incoming) * incoming.z < 0.0 || half.dot(outgoing) * outgoing.z < 0.0 {
            return None;
        }
        Some((half, etap))
    }

    /// BSDF × |cosine| for light arriving along `incoming` and leaving along `outgoing`.
    pub fn eval(&self, outgoing: Vec3, incoming: Vec3) -> Vec3 {
        if outgoing.z <= 0.0 || incoming.z == 0.0 {
            return Vec3::ZERO;
        }
        let Some((half, etap)) = self.half(outgoing, incoming) else { return Vec3::ZERO };
        let alpha = self.alpha();
        let d = ggx_d(alpha, half.z);
        let g = smith_g1(alpha, outgoing.z) * smith_g1(alpha, incoming.z);
        let f = fresnel(outgoing.dot(half), self.eta);
        if incoming.z > 0.0 {
            Vec3::splat(d * g * f / (4.0 * outgoing.z * incoming.z.abs())) * incoming.z.abs()
        } else {
            let denominator = (incoming.dot(half) + outgoing.dot(half) / etap).powi(2) * outgoing.z * incoming.z.abs();
            if denominator <= 1e-20 {
                return Vec3::ZERO;
            }
            // Radiance is compressed into the denser side: divide by the relative index squared.
            let value = d * g * (1.0 - f) * (incoming.dot(half) * outgoing.dot(half) / denominator).abs() / (etap * etap);
            self.tint * value * incoming.z.abs()
        }
    }

    /// The density [`Dielectric::sample`] draws `incoming` with.
    pub fn pdf(&self, outgoing: Vec3, incoming: Vec3) -> f32 {
        if outgoing.z <= 0.0 || incoming.z == 0.0 {
            return 0.0;
        }
        let Some((half, etap)) = self.half(outgoing, incoming) else { return 0.0 };
        let alpha = self.alpha();
        let visible = smith_g1(alpha, outgoing.z) / outgoing.z * ggx_d(alpha, half.z) * outgoing.dot(half).abs();
        let f = fresnel(outgoing.dot(half), self.eta);
        if incoming.z > 0.0 {
            visible / (4.0 * outgoing.dot(half).abs()) * f
        } else {
            let jacobian = incoming.dot(half).abs() / (incoming.dot(half) + outgoing.dot(half) / etap).powi(2);
            visible * jacobian * (1.0 - f)
        }
    }

    /// Draws an incoming direction: reflected or refracted by Fresnel.
    pub fn sample(&self, outgoing: Vec3, choice: f32, u: Vec2) -> Option<Vec3> {
        if outgoing.z <= 0.0 {
            return None;
        }
        let half = sample_visible_normal(outgoing, self.alpha(), u);
        let cos_o = outgoing.dot(half);
        let f = fresnel(cos_o, self.eta);
        if choice < f {
            let reflected = half * (2.0 * cos_o) - outgoing;
            (reflected.z > 0.0).then_some(reflected)
        } else {
            // Snell: refract the outgoing direction through the microfacet into the other side.
            let eta = self.eta;
            let sin2_t = (1.0 - cos_o * cos_o).max(0.0) / (eta * eta);
            if sin2_t >= 1.0 {
                return None;
            }
            let cos_t = (1.0 - sin2_t).sqrt();
            let refracted = -outgoing / eta + half * (cos_o / eta - cos_t);
            (refracted.z < 0.0).then(|| refracted.normalize_or_zero())
        }
    }
}

/// A principled surface with a share of glass.
#[derive(Debug, Clone, Copy)]
pub struct Layered {
    pub surface: Surface,
    pub glass: Dielectric,
    /// How much of the surface is glass (transmission × the dielectric share), 0 to 1.
    pub share: f32,
}

impl Layered {
    pub fn eval(&self, outgoing: Vec3, incoming: Vec3) -> Vec3 {
        let mut value = Vec3::ZERO;
        if self.share < 1.0 {
            value += self.surface.eval(outgoing, incoming) * (1.0 - self.share);
        }
        if self.share > 0.0 {
            value += self.glass.eval(outgoing, incoming) * self.share;
        }
        value
    }

    /// The diffuse part of [`Layered::eval`] (the glass and the specular layer are glossy).
    pub fn diffuse(&self, outgoing: Vec3, incoming: Vec3) -> Vec3 {
        if self.share >= 1.0 { Vec3::ZERO } else { self.surface.diffuse(outgoing, incoming) * (1.0 - self.share) }
    }

    /// The diffuse colour (for the diffuse passes).
    pub fn diffuse_color(&self, cos_out: f32) -> Vec3 {
        self.surface.diffuse_color(cos_out) * (1.0 - self.share).max(0.0)
    }

    /// The glossy colour seen at `cos_out` (for the glossy passes): the specular layer's, and
    /// the glass's tint where there is glass.
    pub fn glossy_color(&self, cos_out: f32) -> Vec3 {
        self.surface.glossy_color(cos_out) * (1.0 - self.share).max(0.0) + self.glass.tint * self.share.clamp(0.0, 1.0)
    }

    pub fn pdf(&self, outgoing: Vec3, incoming: Vec3) -> f32 {
        let mut pdf = 0.0;
        if self.share < 1.0 {
            pdf += self.surface.pdf(outgoing, incoming) * (1.0 - self.share);
        }
        if self.share > 0.0 {
            pdf += self.glass.pdf(outgoing, incoming) * self.share;
        }
        pdf
    }

    /// Draws a direction from one layer, chosen by its share; the weight is the whole surface's.
    pub fn sample(&self, outgoing: Vec3, choice: f32, u: Vec2, second: f32) -> Option<Sample> {
        let incoming =
            if choice < self.share { self.glass.sample(outgoing, second, u)? } else { self.surface.sample(outgoing, second, u)?.direction };
        let pdf = self.pdf(outgoing, incoming);
        if pdf <= 1e-8 {
            return None;
        }
        Some(Sample { direction: incoming, weight: self.eval(outgoing, incoming) / pdf, pdf })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampling::Rng;

    #[test]
    fn fresnel_matches_known_values() {
        // Normal incidence on glass: ((1.5 − 1) / (1.5 + 1))² = 0.04.
        assert!((fresnel(1.0, 1.5) - 0.04).abs() < 1e-5);
        // Grazing reflects everything; leaving glass past the critical angle reflects totally.
        assert!(fresnel(0.0, 1.5) > 0.99);
        assert_eq!(fresnel(0.3, 1.0 / 1.5), 1.0);
    }

    #[test]
    fn white_glass_neither_gains_nor_loses_much_energy() {
        // Reflected plus transmitted, with the 1/η² radiance scaling undone, is close to 1; single
        // scattering loses the light that bounces between microfacets, more the rougher it is.
        for (roughness, floor) in [(0.05f32, 0.95f32), (0.3, 0.85), (0.6, 0.65)] {
            for eta in [1.5f32, 1.0 / 1.5] {
                let glass = Dielectric { eta, roughness, tint: Vec3::ONE };
                for cos in [0.3f32, 0.7, 1.0] {
                    let outgoing = Vec3::new((1.0 - cos * cos).sqrt(), 0.0, cos);
                    let mut rng = Rng::for_sample(5, 1, 2);
                    let mut sum = 0.0;
                    let count = 40_000;
                    for _ in 0..count {
                        let Some(incoming) = glass.sample(outgoing, rng.uniform(), rng.next2()) else { continue };
                        let pdf = glass.pdf(outgoing, incoming);
                        if pdf > 0.0 {
                            let scale = if incoming.z < 0.0 { eta * eta } else { 1.0 };
                            sum += glass.eval(outgoing, incoming).x / pdf * scale;
                        }
                    }
                    let total = sum / count as f32;
                    assert!((floor..=1.03).contains(&total), "roughness {roughness} eta {eta} cos {cos}: {total}");
                }
            }
        }
    }

    #[test]
    fn sampling_density_matches_eval_on_both_sides() {
        let layered = Layered {
            surface: Surface { base: Vec3::splat(0.5), metallic: 0.0, roughness: 0.4 },
            glass: Dielectric { eta: 1.45, roughness: 0.3, tint: Vec3::splat(0.9) },
            share: 0.7,
        };
        let outgoing = Vec3::new(0.3, -0.2, 0.9).normalize();
        let mut rng = Rng::for_sample(9, 4, 1);
        let mut below = 0;
        for _ in 0..2000 {
            let Some(sample) = layered.sample(outgoing, rng.uniform(), rng.next2(), rng.uniform()) else { continue };
            if sample.direction.z < 0.0 {
                below += 1;
            }
            let direct = layered.eval(outgoing, sample.direction) / layered.pdf(outgoing, sample.direction);
            assert!((direct - sample.weight).abs().max_element() < 1e-3 * direct.max_element().max(1.0));
        }
        assert!(below > 500, "{below}");
    }
}
