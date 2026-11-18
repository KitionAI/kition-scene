//! Hair: parent strands rooted on the surface (straight along the normal until groomed), child
//! strands round or between them, and the grooming brushes of Particle Edit mode. Strands are
//! points root first, in the emitter object's own space.

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::rng;
use crate::settings::{Children, Kind, ParticleSystem};
use crate::surface::{self, Start, Surface};

/// One strand: its points, root first.
pub type Strand = Vec<Vec3>;

/// The parent strands' roots and normals, from the seed.
pub fn roots(system: &ParticleSystem, surface: &Surface) -> Vec<Start> {
    if system.kind != Kind::Hair {
        return Vec::new();
    }
    surface::starts(surface, system.emit_from, system.distribution, system.count, system.seed)
}

/// A straight strand from `start` along its normal.
fn straight(start: Start, length: f32, segments: u32) -> Strand {
    (0..=segments).map(|step| start.position + start.normal * (length * step as f32 / segments.max(1) as f32)).collect()
}

/// The parent strands: groomed ones when the system has them, else straight from the roots.
pub fn parents(system: &ParticleSystem, surface: &Surface) -> Vec<Strand> {
    if system.kind != Kind::Hair {
        return Vec::new();
    }
    if !system.groomed.is_empty() {
        return system.groomed.clone();
    }
    roots(system, surface).into_iter().map(|start| straight(start, system.hair_length, system.segments)).collect()
}

// Random channels.
const CHILD_ANGLE: u32 = 40;
const CHILD_RADIUS: u32 = 41;
const CHILD_SEED: u32 = 0x5EED;

/// Two directions across `normal`.
fn across(normal: Vec3) -> (Vec3, Vec3) {
    let normal = if normal == Vec3::ZERO { Vec3::Z } else { normal.normalize() };
    normal.any_orthonormal_pair()
}

/// The child strands of `parents` (which grew from `starts`).
pub fn children(system: &ParticleSystem, surface: &Surface, starts: &[Start], parents: &[Strand]) -> Vec<Strand> {
    let per = system.child_count as usize;
    match system.children {
        Children::None => Vec::new(),
        Children::Simple => {
            let mut out = Vec::with_capacity(parents.len() * per);
            for (parent_index, parent) in parents.iter().enumerate() {
                let normal = starts.get(parent_index).map_or(Vec3::Z, |start| start.normal);
                let (u, v) = across(normal);
                for child in 0..per {
                    let index = u32::try_from(parent_index * per + child).unwrap_or(u32::MAX);
                    let angle = rng::unit(system.seed ^ CHILD_SEED, index, CHILD_ANGLE) * std::f32::consts::TAU;
                    let radius = system.child_radius * rng::unit(system.seed ^ CHILD_SEED, index, CHILD_RADIUS).sqrt();
                    let offset = (u * angle.cos() + v * angle.sin()) * radius;
                    out.push(parent.iter().map(|point| *point + offset).collect());
                }
            }
            out
        }
        Children::Interpolated => {
            if parents.is_empty() {
                return Vec::new();
            }
            let count = u32::try_from(parents.len() * per).unwrap_or(u32::MAX);
            let child_starts = surface::starts(surface, system.emit_from, crate::settings::Distribution::Random, count, system.seed ^ CHILD_SEED);
            child_starts
                .into_iter()
                .map(|start| {
                    // The three nearest parent roots shape the child, by inverse distance.
                    let mut nearest: Vec<(f32, usize)> = parents
                        .iter()
                        .enumerate()
                        .filter_map(|(index, parent)| parent.first().map(|root| (root.distance_squared(start.position), index)))
                        .collect();
                    nearest.sort_by(|a, b| a.0.total_cmp(&b.0));
                    nearest.truncate(3);
                    let weights: Vec<(f32, usize)> = nearest.iter().map(|(distance, index)| (1.0 / (distance.sqrt() + 1e-4), *index)).collect();
                    let total: f32 = weights.iter().map(|(weight, _)| weight).sum();
                    let points = system.segments as usize + 1;
                    (0..points)
                        .map(|step| {
                            let offset = weights
                                .iter()
                                .filter_map(|(weight, index)| {
                                    let parent = parents.get(*index)?;
                                    Some((*parent.get(step)? - *parent.first()?) * (*weight / total.max(1e-9)))
                                })
                                .sum::<Vec3>();
                            start.position + offset
                        })
                        .collect()
                })
                .collect()
        }
    }
}

/// Every strand the system shows: parents, then children.
pub fn strands(system: &ParticleSystem, surface: &Surface) -> Vec<Strand> {
    let starts = roots(system, surface);
    let parents = parents(system, surface);
    let children = children(system, surface, &starts, &parents);
    let mut out = parents;
    out.extend(children);
    out
}

/// A strand's length.
pub fn length(strand: &[Vec3]) -> f32 {
    strand.windows(2).map(|pair| if let [a, b] = pair { a.distance(*b) } else { 0.0 }).sum()
}

/// The grooming brushes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GroomBrush {
    /// Drags strands along with the pointer, keeping their lengths.
    #[default]
    Comb,
    /// Shortens strands to where the brush touches them.
    Cut,
    /// Grows (or, subtracting, shrinks) strands.
    Length,
    /// Raises strands towards their root's normal.
    Puff,
    /// Evens out bends.
    Smooth,
}

impl GroomBrush {
    pub const ALL: [(&'static str, GroomBrush); 5] =
        [("comb", Self::Comb), ("cut", Self::Cut), ("length", Self::Length), ("puff", Self::Puff), ("smooth", Self::Smooth)];

    pub fn id(self) -> &'static str {
        Self::ALL.iter().find(|(_, brush)| *brush == self).map_or("comb", |(id, _)| id)
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().find(|(name, _)| *name == id).map(|(_, brush)| *brush)
    }
}

/// One dab of a grooming brush. Distances are measured where the caller says points show
/// (`to_screen`, pixels); `drag` is the pointer's move this dab in the strands' own space.
pub struct GroomDab<'a> {
    pub brush: GroomBrush,
    pub strength: f32,
    pub radius: f32,
    pub center: (f32, f32),
    pub drag: Vec3,
    /// Length and puff the other way.
    pub subtract: bool,
    pub to_screen: &'a dyn Fn(Vec3) -> Option<(f32, f32)>,
    /// Each strand's root normal (for puff).
    pub normals: &'a [Vec3],
}

impl GroomDab<'_> {
    /// How much the brush holds `point`, 0 to 1 (a smooth falloff to its edge).
    fn weight(&self, point: Vec3) -> f32 {
        let Some((x, y)) = (self.to_screen)(point) else { return 0.0 };
        let distance = (x - self.center.0).hypot(y - self.center.1) / self.radius.max(1e-3);
        if distance >= 1.0 {
            return 0.0;
        }
        let t = 1.0 - distance;
        t * t * (3.0 - 2.0 * t)
    }
}

/// Rebuilds `strand` from its root along its current directions with `lengths` per segment.
fn relength(strand: &mut [Vec3], lengths: &[f32]) {
    let original: Vec<Vec3> = strand.to_vec();
    for step in 1..strand.len() {
        let direction = match (original.get(step), original.get(step - 1)) {
            (Some(here), Some(before)) => (*here - *before).normalize_or_zero(),
            _ => Vec3::ZERO,
        };
        let previous = strand.get(step - 1).copied().unwrap_or(Vec3::ZERO);
        if let Some(point) = strand.get_mut(step) {
            *point = previous + direction * lengths.get(step - 1).copied().unwrap_or(0.0);
        }
    }
}

fn segment_lengths(strand: &[Vec3]) -> Vec<f32> {
    strand.windows(2).map(|pair| if let [a, b] = pair { a.distance(*b) } else { 0.0 }).collect()
}

/// Resamples `strand` to the same number of points along its first `share` (0 to 1) of length.
fn shorten(strand: &mut Strand, share: f32) {
    let total = length(strand);
    if total <= 0.0 || strand.len() < 2 {
        return;
    }
    let target = total * share.clamp(0.0, 1.0);
    let original = strand.clone();
    let steps = strand.len() - 1;
    for step in 1..=steps {
        let want = target * step as f32 / steps as f32;
        let mut walked = 0.0;
        let mut placed = original.last().copied().unwrap_or(Vec3::ZERO);
        for pair in original.windows(2) {
            let [a, b] = pair else { continue };
            let piece = a.distance(*b);
            if walked + piece >= want {
                let t = if piece > 0.0 { (want - walked) / piece } else { 0.0 };
                placed = a.lerp(*b, t);
                break;
            }
            walked += piece;
        }
        if let Some(point) = strand.get_mut(step) {
            *point = placed;
        }
    }
}

/// Applies a dab to `strands`; answers how many strands it changed.
pub fn groom(strands: &mut [Strand], dab: &GroomDab<'_>) -> usize {
    let mut touched = 0;
    for (index, strand) in strands.iter_mut().enumerate() {
        let weights: Vec<f32> = strand.iter().map(|point| dab.weight(*point)).collect();
        if weights.iter().skip(1).all(|weight| *weight <= 0.0) {
            continue;
        }
        touched += 1;
        let lengths = segment_lengths(strand);
        let points = strand.len();
        match dab.brush {
            GroomBrush::Comb => {
                for step in 1..points {
                    let weight = weights.get(step).copied().unwrap_or(0.0) * dab.strength;
                    if let Some(point) = strand.get_mut(step) {
                        *point += dab.drag * weight;
                    }
                }
                relength(strand, &lengths);
            }
            GroomBrush::Cut => {
                // Cut at the first point the brush holds (never the root).
                if let Some(first) = (1..points).find(|step| weights.get(*step).copied().unwrap_or(0.0) > 0.0) {
                    let up_to: f32 = lengths.iter().take(first.saturating_sub(1)).sum();
                    let total: f32 = lengths.iter().sum();
                    let keep = if total > 0.0 { (up_to / total).max(0.05) } else { 1.0 };
                    shorten(strand, keep.max(1.0 - dab.strength).min(1.0));
                }
            }
            GroomBrush::Length => {
                let most = weights.iter().copied().fold(0.0f32, f32::max);
                let factor = 1.0 + 0.1 * dab.strength * most * if dab.subtract { -1.0 } else { 1.0 };
                let scaled: Vec<f32> = lengths.iter().map(|piece| piece * factor.max(0.05)).collect();
                relength(strand, &scaled);
            }
            GroomBrush::Puff => {
                let normal = dab.normals.get(index).copied().unwrap_or(Vec3::Z);
                let normal = if dab.subtract { -normal } else { normal };
                let original = strand.clone();
                for step in 1..points {
                    let weight = (weights.get(step).copied().unwrap_or(0.0) * dab.strength * 0.3).min(1.0);
                    let (Some(here), Some(before)) = (original.get(step), original.get(step - 1)) else { continue };
                    let direction = (*here - *before).normalize_or_zero().lerp(normal, weight).normalize_or_zero();
                    if let Some(point) = strand.get_mut(step) {
                        *point = *before + direction;
                    }
                }
                relength(strand, &lengths);
            }
            GroomBrush::Smooth => {
                let original = strand.clone();
                for step in 1..points.saturating_sub(1) {
                    let weight = weights.get(step).copied().unwrap_or(0.0) * dab.strength * 0.5;
                    let (Some(before), Some(here), Some(after)) = (original.get(step - 1), original.get(step), original.get(step + 1)) else {
                        continue;
                    };
                    if let Some(point) = strand.get_mut(step) {
                        *point = here.lerp((*before + *after) * 0.5, weight);
                    }
                }
                relength(strand, &lengths);
            }
        }
    }
    touched
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sphere_like() -> Surface {
        // An octahedron: closed, normals outwards.
        let positions = vec![Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z];
        let triangles = vec![[0, 2, 4], [2, 1, 4], [1, 3, 4], [3, 0, 4], [2, 0, 5], [1, 2, 5], [3, 1, 5], [0, 3, 5]];
        Surface { normals: positions.clone(), positions, triangles, weights: Vec::new() }
    }

    fn hair() -> ParticleSystem {
        let mut system = ParticleSystem::new("Hair", Kind::Hair);
        system.count = 200;
        system
    }

    #[test]
    fn strands_grow_along_the_normals_at_their_length() {
        let system = hair();
        let all = strands(&system, &sphere_like());
        assert_eq!(all.len(), 200);
        for strand in &all {
            assert_eq!(strand.len(), 6);
            assert!((length(strand) - 4.0).abs() < 1e-4);
        }
    }

    #[test]
    fn children_follow_their_parents() {
        let mut system = hair();
        system.children = Children::Simple;
        assert_eq!(strands(&system, &sphere_like()).len(), 200 * 11);
        system.children = Children::Interpolated;
        let all = strands(&system, &sphere_like());
        assert_eq!(all.len(), 200 * 11);
        assert!(all.iter().skip(200).all(|strand| strand.len() == 6));
    }

    #[test]
    fn combing_bends_strands_and_keeps_their_length() {
        let system = hair();
        let surface = sphere_like();
        let mut groomed = parents(&system, &surface);
        let normals: Vec<Vec3> = roots(&system, &surface).iter().map(|start| start.normal).collect();
        // Seen from above: x, y are pixels.
        let to_screen = |point: Vec3| Some((point.x * 100.0, point.y * 100.0));
        let dab = GroomDab {
            brush: GroomBrush::Comb,
            strength: 1.0,
            radius: 1000.0,
            center: (0.0, 0.0),
            drag: Vec3::X * 0.5,
            subtract: false,
            to_screen: &to_screen,
            normals: &normals,
        };
        let before = groomed.clone();
        assert_eq!(groom(&mut groomed, &dab), 200);
        assert_ne!(groomed, before);
        for (old, new) in before.iter().zip(&groomed) {
            assert!((length(old) - length(new)).abs() < 1e-3);
            assert_eq!(old.first(), new.first(), "roots stay");
        }
        let cut = GroomDab { brush: GroomBrush::Cut, ..dab };
        groom(&mut groomed, &cut);
        assert!(groomed.iter().all(|strand| length(strand) < 4.0));
    }
}
