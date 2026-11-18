//! Voronoi (cellular) noise, as the Voronoi Texture makes it: one feature point per unit cell,
//! moved from the cell's corner by a random offset scaled by the randomness, and what a point
//! sees of the nearest ones: the distance to the nearest (F1) or second nearest (F2), a smoothed
//! F1 (Quilez's smooth Voronoi), the distance to the nearest cell edge, or half the distance from
//! the nearest feature point to its own nearest neighbour (the n-sphere radius). Fractal detail
//! sums octaves as fBm does. Our hash places the points, so cells differ from the reference's
//! while their statistics match (`tests/shader_blender.rs`).

use glam::{Vec2, Vec3};

use crate::noise::unit3;

/// How distances are measured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Metric {
    Euclidean,
    Manhattan,
    Chebychev,
    Minkowski(f32),
}

impl Metric {
    pub fn parse(text: &str, exponent: f32) -> Self {
        match text {
            "manhattan" => Self::Manhattan,
            "chebychev" => Self::Chebychev,
            "minkowski" => Self::Minkowski(exponent),
            _ => Self::Euclidean,
        }
    }

    fn distance(self, d: Vec3) -> f32 {
        match self {
            Self::Euclidean => d.length(),
            Self::Manhattan => d.abs().element_sum(),
            Self::Chebychev => d.abs().max_element(),
            Self::Minkowski(exponent) => {
                if exponent <= 0.0 {
                    return 0.0;
                }
                let a = d.abs();
                (a.x.powf(exponent) + a.y.powf(exponent) + a.z.powf(exponent)).powf(1.0 / exponent)
            }
        }
    }
}

/// What a Voronoi lookup gives: the feature's distance, its cell's colour, and where it is.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Feature {
    pub distance: f32,
    pub color: Vec3,
    pub position: Vec3,
}

/// The dimensions a lookup works in: points with the unused components held at zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dims {
    D1,
    D2,
    D3,
}

impl Dims {
    pub fn parse(text: &str) -> Self {
        match text {
            "1d" => Self::D1,
            "2d" => Self::D2,
            _ => Self::D3,
        }
    }

    fn flatten(self, p: Vec3) -> Vec3 {
        match self {
            Self::D1 => Vec3::new(p.x, 0.0, 0.0),
            Self::D2 => Vec3::new(p.x, p.y, 0.0),
            Self::D3 => p,
        }
    }

    /// The neighbour cells within `reach` cells.
    fn neighbours(self, reach: i32) -> impl Iterator<Item = [i32; 3]> {
        let (ry, rz) = match self {
            Self::D1 => (0, 0),
            Self::D2 => (reach, 0),
            Self::D3 => (reach, reach),
        };
        (-rz..=rz).flat_map(move |k| (-ry..=ry).flat_map(move |j| (-reach..=reach).map(move |i| [i, j, k])))
    }

    fn offset(self, cell: [i32; 3], randomness: f32) -> Vec3 {
        self.flatten(unit3(cell[0], cell[1], cell[2], 101) * randomness)
    }
}

fn cell_color(cell: [i32; 3]) -> Vec3 {
    unit3(cell[0], cell[1], cell[2], 202)
}

/// One cell and its feature point relative to the cell the point is in.
fn feature_point(dims: Dims, base: [i32; 3], step: [i32; 3], randomness: f32) -> ([i32; 3], Vec3) {
    let cell = [base[0] + step[0], base[1] + step[1], base[2] + step[2]];
    (cell, Vec3::new(step[0] as f32, step[1] as f32, step[2] as f32) + dims.offset(cell, randomness))
}

fn split(dims: Dims, p: Vec3) -> ([i32; 3], Vec3) {
    let p = dims.flatten(p);
    let floor = p.floor();
    ([floor.x as i32, floor.y as i32, floor.z as i32], p - floor)
}

fn origin(base: [i32; 3]) -> Vec3 {
    Vec3::new(base[0] as f32, base[1] as f32, base[2] as f32)
}

/// The nearest feature (`second` for the second nearest).
pub fn nearest(dims: Dims, p: Vec3, metric: Metric, randomness: f32, second: bool) -> Feature {
    let (base, local) = split(dims, p);
    let (mut first, mut next) = (Feature { distance: f32::MAX, ..Feature::default() }, Feature { distance: f32::MAX, ..Feature::default() });
    for step in dims.neighbours(1) {
        let (cell, point) = feature_point(dims, base, step, randomness);
        let distance = metric.distance(point - local);
        let found = Feature { distance, color: cell_color(cell), position: point + origin(base) };
        if distance < first.distance {
            next = first;
            first = found;
        } else if distance < next.distance {
            next = found;
        }
    }
    if second { next } else { first }
}

/// Quilez's smooth F1: the distance, colour, and position blended over nearby cells.
pub fn smooth(dims: Dims, p: Vec3, metric: Metric, randomness: f32, smoothness: f32) -> Feature {
    let (base, local) = split(dims, p);
    let smoothness = (smoothness * 0.5).clamp(0.0, 0.5);
    let mut out = Feature { distance: 8.0, color: Vec3::ZERO, position: Vec3::ZERO };
    for step in dims.neighbours(2) {
        let (cell, point) = feature_point(dims, base, step, randomness);
        let distance = metric.distance(point - local);
        let h = if smoothness > 0.0 {
            let x = (0.5 + 0.5 * (out.distance - distance) / smoothness).clamp(0.0, 1.0);
            x * x * (3.0 - 2.0 * x)
        } else if distance < out.distance {
            1.0
        } else {
            0.0
        };
        let mut correction = smoothness * h * (1.0 - h);
        out.distance = out.distance + (distance - out.distance) * h - correction;
        correction /= 1.0 + 3.0 * smoothness;
        out.color = out.color.lerp(cell_color(cell), h) - Vec3::splat(correction);
        out.position = out.position.lerp(point, h) - Vec3::splat(correction);
    }
    out.position += origin(base);
    out
}

/// The distance to the nearest cell edge (euclidean).
pub fn edge_distance(dims: Dims, p: Vec3, randomness: f32) -> f32 {
    let (base, local) = split(dims, p);
    let mut closest = Vec3::ZERO;
    let mut best = f32::MAX;
    for step in dims.neighbours(1) {
        let (_, point) = feature_point(dims, base, step, randomness);
        let to = point - local;
        if to.length_squared() < best {
            best = to.length_squared();
            closest = to;
        }
    }
    let mut edge = f32::MAX;
    for step in dims.neighbours(2) {
        let (_, point) = feature_point(dims, base, step, randomness);
        let to = point - local;
        let between = to - closest;
        if between.length_squared() > 1e-8 {
            edge = edge.min((0.5 * (closest + to)).dot(between.normalize()));
        }
    }
    edge.max(0.0)
}

/// Half the distance from the nearest feature point to its own nearest neighbour.
pub fn sphere_radius(dims: Dims, p: Vec3, randomness: f32) -> f32 {
    let (base, local) = split(dims, p);
    let (mut closest, mut cell, mut best) = (Vec3::ZERO, [0; 3], f32::MAX);
    for step in dims.neighbours(1) {
        let (_, point) = feature_point(dims, base, step, randomness);
        let distance = point.distance(local);
        if distance < best {
            best = distance;
            closest = point;
            cell = step;
        }
    }
    let mut nearest = f32::MAX;
    for step in dims.neighbours(1) {
        if step == [0, 0, 0] {
            continue;
        }
        let around = [cell[0] + step[0], cell[1] + step[1], cell[2] + step[2]];
        let (_, point) = feature_point(dims, base, around, randomness);
        nearest = nearest.min(point.distance(closest));
    }
    nearest * 0.5
}

/// The Voronoi Texture's settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub dims: Dims,
    pub metric: Metric,
    pub randomness: f32,
    pub detail: f32,
    pub roughness: f32,
    pub lacunarity: f32,
    pub smoothness: f32,
    pub normalize: bool,
}

/// The Voronoi Texture's F1, F2, or smooth F1 at `p` (already scaled), fractal when it has
/// detail; the position comes back in `p`'s units.
pub fn texture(feature: &str, p: Vec3, s: &Settings) -> Feature {
    let one = |q: Vec3| match feature {
        "f2" => nearest(s.dims, q, s.metric, s.randomness, true),
        "smooth_f1" => smooth(s.dims, q, s.metric, s.randomness, s.smoothness),
        _ => nearest(s.dims, q, s.metric, s.randomness, false),
    };
    let detail = s.detail.clamp(0.0, 15.0);
    if detail == 0.0 {
        let mut found = one(p);
        if s.normalize {
            found.distance /= max_distance(s.dims, s.metric, s.randomness, feature);
        }
        return found;
    }
    let (mut sum, mut most, mut amplitude, mut scale) = (Feature::default(), 0.0f32, 1.0f32, 1.0f32);
    let octaves = detail.floor() as u32;
    let rest = detail - detail.floor();
    for octave in 0..=octaves + u32::from(rest > 0.0) {
        let weight = if octave > octaves { rest } else { 1.0 };
        let found = one(p * scale);
        sum.distance += found.distance * amplitude * weight;
        sum.color += found.color * amplitude * weight;
        sum.position = sum.position.lerp(found.position / scale, weight / (octave as f32 + 1.0));
        most += amplitude * weight;
        amplitude *= s.roughness.clamp(0.0, 1.0);
        scale *= s.lacunarity;
    }
    if s.normalize {
        sum.distance /= most * max_distance(s.dims, s.metric, s.randomness, feature);
        sum.color /= most;
    }
    sum
}

/// The largest distance a feature can be away (for normalizing), measured once per setting.
fn max_distance(dims: Dims, metric: Metric, randomness: f32, feature: &str) -> f32 {
    let along = match dims {
        Dims::D1 => Vec3::X,
        Dims::D2 => Vec3::new(1.0, 1.0, 0.0),
        Dims::D3 => Vec3::ONE,
    };
    let reach = metric.distance(along * (0.5 + 0.5 * randomness));
    if feature == "f2" { reach * 2.0 } else { reach }
}

/// A point in the plane as a 3D point.
pub fn plane(p: Vec2) -> Vec3 {
    p.extend(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_randomness_features_sit_on_the_lattice() {
        let found = nearest(Dims::D3, Vec3::new(0.25, 0.25, 0.25), Metric::Euclidean, 0.0, false);
        assert!((found.distance - (3.0f32 * 0.0625).sqrt()).abs() < 1e-6);
        assert_eq!(found.position, Vec3::ZERO);
        let second = nearest(Dims::D2, Vec3::new(0.25, 0.5, 0.0), Metric::Manhattan, 0.0, true);
        assert!((second.distance - 0.75).abs() < 1e-6, "{second:?}");
        let edge = edge_distance(Dims::D2, Vec3::new(0.4, 0.1, 0.0), 0.0);
        assert!((edge - 0.1).abs() < 1e-5, "{edge}");
        assert!((sphere_radius(Dims::D3, Vec3::new(0.1, 0.2, 0.1), 0.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn smooth_f1_without_smoothness_is_f1() {
        let p = Vec3::new(2.3, -1.7, 0.4);
        let hard = nearest(Dims::D3, p, Metric::Euclidean, 1.0, false);
        let soft = smooth(Dims::D3, p, Metric::Euclidean, 1.0, 0.0);
        assert!((hard.distance - soft.distance).abs() < 1e-5);
        assert!((hard.color - soft.color).abs().max_element() < 1e-5);
    }
}
