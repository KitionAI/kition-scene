//! Participating media: what fills space between surfaces (the world's volume) or inside a closed
//! mesh (a material's volume). Media are homogeneous: per metre they absorb, scatter, and send
//! out light the same everywhere; where several overlap their coefficients add. Distances are
//! sampled in proportion to transmittance on one colour channel chosen at random, weighted by
//! the mean over the three (spectral MIS), and scattered light leaves by the media's phase
//! functions.

use std::f32::consts::PI;

use glam::{Vec2, Vec3};
use kition_scene_shader::{Closure, Phase};

use crate::sampling::Frame;

/// A homogeneous medium.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Medium {
    pub absorption: Vec3,
    pub scattering: Vec3,
    /// Light sent out per metre.
    pub emission: Vec3,
    /// The phase functions with their shares of the scattering (by mean).
    pub phases: Vec<(f32, Phase)>,
}

fn finite(value: Vec3) -> Vec3 {
    if value.is_finite() { value.max(Vec3::ZERO) } else { Vec3::ZERO }
}

impl Medium {
    /// The medium a shader group's Volume gives, when it does anything.
    pub fn of(closure: &Closure) -> Option<Self> {
        let mut medium = Self { emission: finite(closure.emission), ..Self::default() };
        for lobe in &closure.volume {
            medium.absorption += finite(lobe.absorption);
            let scattering = finite(lobe.scattering);
            medium.scattering += scattering;
            let weight = scattering.element_sum() / 3.0;
            if weight > 0.0 {
                medium.phases.push((weight, lobe.phase));
            }
        }
        (!medium.is_empty()).then_some(medium)
    }

    pub fn is_empty(&self) -> bool {
        self.extinction().max_element() <= 0.0 && self.emission.max_element() <= 0.0
    }

    pub fn extinction(&self) -> Vec3 {
        self.absorption + self.scattering
    }

    /// Adds another medium overlapping this one.
    pub fn add(&mut self, other: &Self) {
        self.absorption += other.absorption;
        self.scattering += other.scattering;
        self.emission += other.emission;
        self.phases.extend(other.phases.iter().copied());
    }

    /// The share of light that gets `distance` metres through (zero at infinity unless clear).
    pub fn transmittance(&self, distance: f32) -> Vec3 {
        let sigma = self.extinction();
        let channel = |sigma: f32| {
            if sigma <= 0.0 {
                1.0
            } else if distance.is_finite() {
                (-sigma * distance).exp()
            } else {
                0.0
            }
        };
        Vec3::new(channel(sigma.x), channel(sigma.y), channel(sigma.z))
    }

    /// The light the medium sends towards the start of a `distance` metre segment, seen through
    /// itself: emission × (1 − e^(−σt·d)) / σt per channel.
    pub fn emitted(&self, distance: f32) -> Vec3 {
        if self.emission.max_element() <= 0.0 {
            return Vec3::ZERO;
        }
        let sigma = self.extinction();
        let channel = |emission: f32, sigma: f32| {
            if emission <= 0.0 {
                0.0
            } else if sigma > 0.0 {
                emission * if distance.is_finite() { -(-sigma * distance).exp_m1() } else { 1.0 } / sigma
            } else if distance.is_finite() {
                emission * distance
            } else {
                0.0
            }
        };
        Vec3::new(channel(self.emission.x, sigma.x), channel(self.emission.y, sigma.y), channel(self.emission.z, sigma.z))
    }

    /// The phase function's value for light going `incoming` → scattered towards `outgoing`
    /// (both unit; `incoming` is the direction light travels before scattering).
    pub fn phase(&self, incoming: Vec3, outgoing: Vec3) -> f32 {
        let total: f32 = self.phases.iter().map(|(weight, _)| *weight).sum();
        if total <= 0.0 {
            return 1.0 / (4.0 * PI);
        }
        let cos = incoming.dot(outgoing).clamp(-1.0, 1.0);
        self.phases.iter().map(|(weight, phase)| weight / total * phase_value(*phase, cos)).sum()
    }

    /// A direction scattered from `incoming` by the phase functions (one chosen by its share),
    /// and the mixture's density there.
    pub fn sample_phase(&self, incoming: Vec3, pick: f32, u: Vec2) -> (Vec3, f32) {
        let total: f32 = self.phases.iter().map(|(weight, _)| *weight).sum();
        let mut chosen = Phase::HenyeyGreenstein(0.0);
        let mut left = pick * total;
        for (weight, phase) in &self.phases {
            chosen = *phase;
            if left < *weight {
                break;
            }
            left -= weight;
        }
        let cos = match chosen {
            Phase::HenyeyGreenstein(g) => sample_hg_cos(g, u.x),
            Phase::Rayleigh => sample_rayleigh_cos(u.x),
        };
        let sin = (1.0 - cos * cos).max(0.0).sqrt();
        let angle = 2.0 * PI * u.y;
        let frame = Frame::new(incoming);
        let direction = frame.to_world(Vec3::new(sin * angle.cos(), sin * angle.sin(), cos)).normalize_or_zero();
        (direction, self.phase(incoming, direction))
    }
}

/// A phase function's value at the cosine between the directions light travels before and after.
fn phase_value(phase: Phase, cos: f32) -> f32 {
    match phase {
        Phase::HenyeyGreenstein(g) => {
            let denominator = 1.0 + g * g - 2.0 * g * cos;
            (1.0 - g * g) / (4.0 * PI * denominator * denominator.max(1e-12).sqrt())
        }
        Phase::Rayleigh => 3.0 / (16.0 * PI) * (1.0 + cos * cos),
    }
}

/// The cosine of a Henyey–Greenstein direction.
fn sample_hg_cos(g: f32, u: f32) -> f32 {
    if g.abs() < 1e-3 {
        return 1.0 - 2.0 * u;
    }
    let square = (1.0 - g * g) / (1.0 - g + 2.0 * g * u);
    ((1.0 + g * g - square * square) / (2.0 * g)).clamp(-1.0, 1.0)
}

/// The cosine of a Rayleigh direction: inverts the CDF (3c + c³ + 4) / 8 in closed form.
fn sample_rayleigh_cos(u: f32) -> f32 {
    let z = 4.0 * u - 2.0;
    let root = (z * z + 1.0).sqrt();
    ((z + root).cbrt() + (z - root).cbrt()).clamp(-1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn integrate(phase: Phase) -> f32 {
        let n = 20000;
        (0..n).map(|i| phase_value(phase, -1.0 + 2.0 * (i as f32 + 0.5) / n as f32) * 2.0 * PI * 2.0 / n as f32).sum()
    }

    #[test]
    fn phase_functions_integrate_to_one_and_sample_their_shapes() {
        for phase in [Phase::HenyeyGreenstein(0.0), Phase::HenyeyGreenstein(0.6), Phase::HenyeyGreenstein(-0.4), Phase::Rayleigh] {
            assert!((integrate(phase) - 1.0).abs() < 1e-3, "{phase:?}");
        }
        // Mean cosine of Henyey–Greenstein samples is g; Rayleigh's is 0.
        let mean = |f: &dyn Fn(f32) -> f32| (0..10000).map(|i| f((i as f32 + 0.5) / 10000.0)).sum::<f32>() / 10000.0;
        assert!((mean(&|u| sample_hg_cos(0.6, u)) - 0.6).abs() < 1e-3);
        assert!(mean(&sample_rayleigh_cos).abs() < 1e-3);
        // Rayleigh's CDF inverted: the sampled cosine gives back u.
        for u in [0.1f32, 0.5, 0.9] {
            let c = sample_rayleigh_cos(u);
            assert!(((3.0 * c + c * c * c + 4.0) / 8.0 - u).abs() < 1e-4);
        }
    }

    #[test]
    fn transmittance_and_emission_follow_the_coefficients() {
        let medium = Medium { absorption: Vec3::new(0.5, 0.0, 1.0), scattering: Vec3::new(0.5, 0.0, 0.0), emission: Vec3::ONE, phases: Vec::new() };
        let t = medium.transmittance(2.0);
        assert!((t.x - (-2.0f32).exp()).abs() < 1e-6 && t.y == 1.0);
        let e = medium.emitted(2.0);
        assert!((e.y - 2.0).abs() < 1e-6 && (e.x - (1.0 - (-2.0f32).exp())).abs() < 1e-6);
        assert_eq!(medium.transmittance(f32::INFINITY), Vec3::new(0.0, 1.0, 0.0));
    }
}
