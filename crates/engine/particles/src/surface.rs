//! Where particles and strands start: points on an emitter's surface (by area), at its vertices,
//! or inside it, spread evenly (jittered) or at random, each from the seed alone.

use glam::Vec3;

use crate::rng;
use crate::settings::{Distribution, EmitFrom};

/// An emitter's surface as triangles, in whatever space the caller works in.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Surface {
    pub positions: Vec<Vec3>,
    /// One normal per position (smooth), unit length.
    pub normals: Vec<Vec3>,
    pub triangles: Vec<[u32; 3]>,
    /// How densely particles start near each position, 0 to 1 (a density vertex group); empty
    /// for everywhere alike.
    pub weights: Vec<f32>,
}

/// A start point: where, and the surface normal there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Start {
    pub position: Vec3,
    pub normal: Vec3,
}

// Random channels.
const PICK: u32 = 1;
const ALONG_A: u32 = 2;
const ALONG_B: u32 = 3;
const VOLUME: u32 = 10;

impl Surface {
    fn corner(&self, index: u32) -> (Vec3, Vec3) {
        let position = self.positions.get(index as usize).copied().unwrap_or(Vec3::ZERO);
        let normal = self.normals.get(index as usize).copied().unwrap_or(Vec3::Z);
        (position, normal)
    }

    /// The density weight at a position (1 without weights).
    fn weight(&self, index: u32) -> f32 {
        if self.weights.is_empty() {
            return 1.0;
        }
        self.weights.get(index as usize).copied().unwrap_or(0.0).clamp(0.0, 1.0)
    }

    /// Each triangle's area (times its corners' mean density weight), and their running total.
    fn areas(&self) -> (Vec<f32>, f32) {
        let mut running = Vec::with_capacity(self.triangles.len());
        let mut total = 0.0;
        for [a, b, c] in &self.triangles {
            let (pa, pb, pc) = (self.corner(*a).0, self.corner(*b).0, self.corner(*c).0);
            let density = if self.weights.is_empty() { 1.0 } else { (self.weight(*a) + self.weight(*b) + self.weight(*c)) / 3.0 };
            total += (pb - pa).cross(pc - pa).length() * 0.5 * density;
            running.push(total);
        }
        (running, total)
    }

    /// The point at barycentric `(u, v)` of triangle `triangle`.
    fn on_triangle(&self, triangle: usize, u: f32, v: f32) -> Option<Start> {
        let [a, b, c] = *self.triangles.get(triangle)?;
        // A uniform point: fold (u, v) into the triangle.
        let (u, v) = if u + v > 1.0 { (1.0 - u, 1.0 - v) } else { (u, v) };
        let w = 1.0 - u - v;
        let ((pa, na), (pb, nb), (pc, nc)) = (self.corner(a), self.corner(b), self.corner(c));
        let normal = (na * w + nb * u + nc * v).normalize_or_zero();
        let flat = (pb - pa).cross(pc - pa).normalize_or_zero();
        Some(Start { position: pa * w + pb * u + pc * v, normal: if normal == Vec3::ZERO { flat } else { normal } })
    }

    /// Whether `point` is inside the (closed) surface: a ray along +X crosses it an odd number of
    /// times.
    pub fn contains(&self, point: Vec3) -> bool {
        let mut crossings = 0u32;
        for [a, b, c] in &self.triangles {
            let (pa, pb, pc) = (self.corner(*a).0, self.corner(*b).0, self.corner(*c).0);
            if ray_hits(point, Vec3::X, pa, pb, pc).is_some() {
                crossings += 1;
            }
        }
        crossings % 2 == 1
    }

    /// The box round the positions.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        self.positions
            .iter()
            .fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(low, high), point| (low.min(*point), high.max(*point)))
    }
}

/// Where a ray from `origin` along `direction` meets triangle (`a`, `b`, `c`): the distance, if
/// it does, ahead of the origin.
pub fn ray_hits(origin: Vec3, direction: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let (edge1, edge2) = (b - a, c - a);
    let h = direction.cross(edge2);
    let det = edge1.dot(h);
    if det.abs() < 1e-12 {
        return None;
    }
    let inverse = 1.0 / det;
    let s = origin - a;
    let u = inverse * s.dot(h);
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(edge1);
    let v = inverse * direction.dot(q);
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = inverse * edge2.dot(q);
    (t > 1e-7).then_some(t)
}

/// The first triangle of the running area total at or above `target`.
fn pick(running: &[f32], target: f32) -> usize {
    running.partition_point(|total| *total < target).min(running.len().saturating_sub(1))
}

/// `count` start points on `surface` from `seed`. A surface with nothing to start from gives
/// none.
pub fn starts(surface: &Surface, from: EmitFrom, distribution: Distribution, count: u32, seed: u32) -> Vec<Start> {
    let count_f = count.max(1) as f32;
    match from {
        EmitFrom::Faces => {
            let (running, total) = surface.areas();
            if surface.triangles.is_empty() || total <= 0.0 {
                return Vec::new();
            }
            (0..count)
                .filter_map(|index| {
                    // Jittered: each particle has its own share of the area, jittered within it.
                    let share = match distribution {
                        Distribution::Jittered => (index as f32 + rng::unit(seed, index, PICK)) / count_f,
                        Distribution::Random => rng::unit(seed, index, PICK),
                    };
                    surface.on_triangle(pick(&running, share * total), rng::unit(seed, index, ALONG_A), rng::unit(seed, index, ALONG_B))
                })
                .collect()
        }
        EmitFrom::Verts => {
            let vertices = surface.positions.len();
            if vertices == 0 {
                return Vec::new();
            }
            if !surface.weights.is_empty() {
                // Each vertex takes a share of the particles in proportion to its weight.
                let mut running = Vec::with_capacity(vertices);
                let mut total = 0.0;
                for index in 0..vertices {
                    total += surface.weight(u32::try_from(index).unwrap_or(u32::MAX));
                    running.push(total);
                }
                if total <= 0.0 {
                    return Vec::new();
                }
                return (0..count)
                    .map(|index| {
                        let share = match distribution {
                            Distribution::Jittered => (index as f32 + 0.5) / count_f,
                            Distribution::Random => rng::unit(seed, index, PICK),
                        };
                        let slot = pick(&running, share * total);
                        let (position, normal) = surface.corner(u32::try_from(slot).unwrap_or(0));
                        Start { position, normal }
                    })
                    .collect();
            }
            (0..count)
                .map(|index| {
                    let slot = match distribution {
                        Distribution::Jittered => (index as usize * vertices) / count.max(1) as usize,
                        Distribution::Random => (rng::unit(seed, index, PICK) * vertices as f32) as usize,
                    }
                    .min(vertices - 1);
                    let (position, normal) = surface.corner(u32::try_from(slot).unwrap_or(0));
                    Start { position, normal }
                })
                .collect()
        }
        EmitFrom::Volume => {
            let (low, high) = surface.bounds();
            if surface.triangles.is_empty() || !(low.is_finite() && high.is_finite()) {
                return Vec::new();
            }
            let size = high - low;
            (0..count)
                .filter_map(|index| {
                    // Up to 64 tries per particle inside the box; a surface too thin to hit gives none.
                    (0..64u32).find_map(|attempt| {
                        let channel = VOLUME + attempt * 3;
                        let point = low
                            + size
                                * Vec3::new(
                                    rng::unit(seed, index, channel),
                                    rng::unit(seed, index, channel + 1),
                                    rng::unit(seed, index, channel + 2),
                                );
                        surface.contains(point).then_some(Start { position: point, normal: Vec3::Z })
                    })
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2 m square in the XY plane, facing up.
    pub fn plane() -> Surface {
        Surface {
            positions: vec![Vec3::new(-1.0, -1.0, 0.0), Vec3::new(1.0, -1.0, 0.0), Vec3::new(1.0, 1.0, 0.0), Vec3::new(-1.0, 1.0, 0.0)],
            normals: vec![Vec3::Z; 4],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            weights: Vec::new(),
        }
    }

    #[test]
    fn jittered_points_cover_the_plane_evenly() {
        let points = starts(&plane(), EmitFrom::Faces, Distribution::Jittered, 1000, 0);
        assert_eq!(points.len(), 1000);
        assert!(points.iter().all(|start| start.position.x.abs() <= 1.0 && start.position.y.abs() <= 1.0 && start.position.z == 0.0));
        assert!(points.iter().all(|start| start.normal == Vec3::Z));
        // Each quadrant holds about a quarter.
        let quadrant = points.iter().filter(|start| start.position.x > 0.0 && start.position.y > 0.0).count();
        assert!((200..300).contains(&quadrant), "{quadrant}");
        assert_eq!(points, starts(&plane(), EmitFrom::Faces, Distribution::Jittered, 1000, 0));
        assert_ne!(points, starts(&plane(), EmitFrom::Faces, Distribution::Jittered, 1000, 1));
    }

    #[test]
    fn vertices_and_volume() {
        let at_vertices = starts(&plane(), EmitFrom::Verts, Distribution::Jittered, 8, 0);
        assert!(at_vertices.iter().all(|start| start.position.x.abs() == 1.0 && start.position.y.abs() == 1.0));
        // A flat plane holds no volume.
        assert!(starts(&plane(), EmitFrom::Volume, Distribution::Random, 10, 0).is_empty());
    }

    #[test]
    fn density_weights_steer_where_particles_start() {
        // Weight 1 on the right edge, 0 on the left: each triangle is weighted by its corners'
        // mean (2/3 and 1/3), uniform within, so the mean x is 2/3 × 1/3 − 1/3 × 1/3 = 1/9.
        let mut ramp = plane();
        ramp.weights = vec![0.0, 1.0, 1.0, 0.0];
        let points = starts(&ramp, EmitFrom::Faces, Distribution::Jittered, 1000, 0);
        assert_eq!(points.len(), 1000, "the count stays");
        let mean = points.iter().map(|start| start.position.x).sum::<f32>() / 1000.0;
        assert!((mean - 1.0 / 9.0).abs() < 0.03, "{mean}");
        let at_vertices = starts(&ramp, EmitFrom::Verts, Distribution::Jittered, 10, 0);
        assert!(at_vertices.iter().all(|start| start.position.x == 1.0), "only weighted vertices");
        ramp.weights = vec![0.0; 4];
        assert!(starts(&ramp, EmitFrom::Faces, Distribution::Jittered, 10, 0).is_empty(), "no weight, no particles");
    }
}
