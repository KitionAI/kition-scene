//! Weight painting: brushes that change one vertex group's weights where the brush touches
//! (each vertex's touch is the brush's falloff at its distance, times the strength), and
//! normalising every group so each vertex's weights sum to one. Brush defaults follow the
//! reference application's weight paint brush: Draw mode, weight 1, strength 1, a 50-pixel
//! radius, smooth falloff.
//!
//! X mirror paints the vertex across the mesh's X = 0 plane too ([`mirror_map`]: by position,
//! or by topology for meshes whose sides match only in their edges), in the group of the
//! mirrored name ([`flip_name`]: `Arm.L` and `Arm.R`); the gradient tool fills weights along a
//! line or around a point ([`gradient`]), as the reference's do.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::error::{Result, SceneError};
use crate::mesh::Mesh;
use crate::proportional::Falloff;
use crate::vertex_data::VertexGroup;

/// What a brush stroke does to the weights it touches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BrushMode {
    /// Moves weights towards the brush's weight.
    #[default]
    Draw,
    /// Adds the brush's weight.
    Add,
    /// Takes the brush's weight away.
    Subtract,
    /// Moves weights towards the mean of their neighbours'.
    Blur,
}

/// A weight paint brush.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Brush {
    #[serde(default)]
    pub mode: BrushMode,
    /// The weight Draw paints and Add and Subtract add or take away, 0 to 1.
    #[serde(default = "one")]
    pub weight: f32,
    /// How much of a change one dab makes at the brush's centre, 0 to 1.
    #[serde(default = "one")]
    pub strength: f32,
    /// The radius in screen pixels.
    #[serde(default = "fifty")]
    pub radius: f32,
    #[serde(default)]
    pub falloff: Falloff,
    /// Only vertices whose faces look towards the eye.
    #[serde(default)]
    pub front_faces_only: bool,
}

fn one() -> f32 {
    1.0
}
fn fifty() -> f32 {
    50.0
}

impl Default for Brush {
    fn default() -> Self {
        Self { mode: BrushMode::Draw, weight: 1.0, strength: 1.0, radius: 50.0, falloff: Falloff::Smooth, front_faces_only: false }
    }
}

impl Brush {
    /// Checks the numbers.
    pub fn check(&self) -> Result<()> {
        let unit = |value: f32| value.is_finite() && (0.0..=1.0).contains(&value);
        if !(unit(self.weight) && unit(self.strength) && self.radius.is_finite() && (1.0..=2000.0).contains(&self.radius)) {
            return Err(SceneError::params("brush weight and strength go from 0 to 1, and its radius from 1 to 2000 pixels"));
        }
        Ok(())
    }

    /// How strongly the brush touches a vertex `distance` pixels from its centre.
    pub fn touch(&self, distance: f32) -> f32 {
        crate::proportional::weight(self.falloff, distance, self.radius) * self.strength
    }
}

/// The group named `name`, made (all zero) if the mesh has none of that name.
pub fn group_mut<'a>(mesh: &'a mut Mesh, name: &str) -> Result<&'a mut VertexGroup> {
    if name.trim().is_empty() {
        return Err(SceneError::params("a vertex group needs a name"));
    }
    let count = mesh.positions.len();
    if !mesh.groups.iter().any(|group| group.name == name) {
        mesh.groups.push(VertexGroup { name: name.to_string(), weights: vec![0.0; count] });
    }
    let group = mesh.groups.iter_mut().find(|group| group.name == name).ok_or_else(|| SceneError::not_found(format!("vertex group {name}")))?;
    group.weights.resize(count, 0.0);
    Ok(group)
}

/// One dab of `brush` on group `group`: `touches` are (vertex, touch) pairs, each touch the
/// brush's falloff and strength there.
pub fn dab(mesh: &mut Mesh, group: &str, touches: &[(u32, f32)], brush: &Brush) -> Result<()> {
    brush.check()?;
    let neighbours = if brush.mode == BrushMode::Blur { neighbours(mesh) } else { Vec::new() };
    let count = mesh.positions.len();
    let weights = &mut group_mut(mesh, group)?.weights;
    let before = weights.clone();
    for (vertex, touch) in touches {
        let touch = touch.clamp(0.0, 1.0);
        if touch <= 0.0 || *vertex as usize >= count {
            continue;
        }
        let Some(weight) = weights.get_mut(*vertex as usize) else { continue };
        *weight = match brush.mode {
            BrushMode::Draw => *weight + (brush.weight - *weight) * touch,
            BrushMode::Add => *weight + brush.weight * touch,
            BrushMode::Subtract => *weight - brush.weight * touch,
            BrushMode::Blur => {
                let around = neighbours.get(*vertex as usize).map_or(&[][..], Vec::as_slice);
                if around.is_empty() {
                    *weight
                } else {
                    let mean = around.iter().filter_map(|other| before.get(*other as usize)).sum::<f32>() / around.len() as f32;
                    *weight + (mean - *weight) * touch
                }
            }
        }
        .clamp(0.0, 1.0);
    }
    Ok(())
}

/// X mirror for brush strokes: each vertex's partner across X = 0 ([`mirror_map`]), and whether
/// the partner is painted in the group of the mirrored name.
#[derive(Debug, Clone, PartialEq)]
pub struct Mirror {
    pub partners: Vec<Option<u32>>,
    pub names: bool,
}

/// [`dab`] with X mirror: each touched vertex's new weight goes to its partner too, in the group
/// of the mirrored name (made if the mesh has none). Where a vertex and its partner are both
/// touched in the same group, both take the weight of the one the brush touches more; in two
/// groups each side keeps its own and gives its weight to the other's mirrored group.
pub fn dab_mirrored(mesh: &mut Mesh, group: &str, touches: &[(u32, f32)], brush: &Brush, mirror: &Mirror) -> Result<()> {
    let before = group_mut(mesh, group)?.weights.clone();
    dab(mesh, group, touches, brush)?;
    let after = group_mut(mesh, group)?.weights.clone();
    let other = if mirror.names { flip_name(group) } else { group.to_string() };
    let same = other == group;
    // The strongest touch per vertex, and the weight it left.
    let mut touched: BTreeMap<u32, f32> = BTreeMap::new();
    for (vertex, touch) in touches {
        let touch = touch.clamp(0.0, 1.0);
        if touch > 0.0 && (*vertex as usize) < before.len() {
            let entry = touched.entry(*vertex).or_insert(0.0);
            *entry = entry.max(touch);
        }
    }
    let mut writes: Vec<(u32, f32)> = Vec::new();
    for (vertex, touch) in &touched {
        let Some(partner) = mirror.partners.get(*vertex as usize).copied().flatten() else { continue };
        let Some(weight) = after.get(*vertex as usize).copied() else { continue };
        if same {
            if partner == *vertex {
                continue;
            }
            // The partner's own touch wins when it is stronger (or as strong and later).
            if let Some(theirs) = touched.get(&partner)
                && (*theirs > *touch || (theirs.to_bits() == touch.to_bits() && partner > *vertex))
            {
                continue;
            }
        }
        writes.push((partner, weight));
    }
    // The mirrored group is made with the stroke, painted or not, as in the reference.
    let weights = &mut group_mut(mesh, &other)?.weights;
    for (vertex, weight) in writes {
        if let Some(slot) = weights.get_mut(vertex as usize) {
            *slot = weight;
        }
    }
    Ok(())
}

/// The largest distance between a vertex and another's mirror image that still pairs them (the
/// reference pairs vertices 1 × 10⁻⁵ m off and not 2 × 10⁻⁵ m).
pub const MIRROR_TOLERANCE: f32 = 1.5e-5;

/// Each vertex's partner across the X = 0 plane, or `None`. By position: the vertex within
/// [`MIRROR_TOLERANCE`] of its mirror image (itself on the plane). By topology: vertices the
/// edges cannot tell apart (refined by their neighbours' classes until the classes stop
/// splitting), each paired with the one of its class nearest its mirror image, so the sides
/// may differ in shape as long as they match in their edges.
pub fn mirror_map(mesh: &Mesh, topology: bool) -> Vec<Option<u32>> {
    let flip = |point: glam::Vec3| glam::Vec3::new(-point.x, point.y, point.z);
    if topology {
        let classes = topology_classes(mesh);
        let mut members: BTreeMap<u64, Vec<u32>> = BTreeMap::new();
        for (vertex, class) in classes.iter().enumerate() {
            members.entry(*class).or_default().push(vertex as u32);
        }
        return mesh
            .positions
            .iter()
            .zip(&classes)
            .map(|(point, class)| {
                let image = flip(*point);
                members.get(class)?.iter().copied().min_by(|a, b| {
                    let distance = |vertex: &u32| mesh.positions.get(*vertex as usize).map_or(f32::INFINITY, |other| other.distance(image));
                    distance(a).total_cmp(&distance(b)).then(a.cmp(b))
                })
            })
            .collect();
    }
    let cell = MIRROR_TOLERANCE * 4.0;
    let key = |point: glam::Vec3| ((point.x / cell).floor() as i64, (point.y / cell).floor() as i64, (point.z / cell).floor() as i64);
    let mut grid: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::new();
    for (vertex, point) in mesh.positions.iter().enumerate() {
        if point.is_finite() {
            grid.entry(key(*point)).or_default().push(vertex as u32);
        }
    }
    mesh.positions
        .iter()
        .map(|point| {
            if !point.is_finite() {
                return None;
            }
            let image = flip(*point);
            let (x, y, z) = key(image);
            let mut best: Option<(f32, u32)> = None;
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        for vertex in grid.get(&(x + dx, y + dy, z + dz)).into_iter().flatten() {
                            let Some(other) = mesh.positions.get(*vertex as usize) else { continue };
                            let distance = other.distance(image);
                            if distance <= MIRROR_TOLERANCE && best.is_none_or(|(nearest, _)| distance < nearest) {
                                best = Some((distance, *vertex));
                            }
                        }
                    }
                }
            }
            best.map(|(_, vertex)| vertex)
        })
        .collect()
}

/// Classes of vertices the edges cannot tell apart: degree first, then each vertex's class and
/// its neighbours' classes, until the number of classes stops growing.
fn topology_classes(mesh: &Mesh) -> Vec<u64> {
    let around = neighbours(mesh);
    let mut classes: Vec<u64> = around.iter().map(|list| list.len() as u64).collect();
    let mut count = classes.iter().collect::<BTreeSet<_>>().len();
    for _ in 0..mesh.positions.len().min(256) {
        let signatures: Vec<(u64, Vec<u64>)> = around
            .iter()
            .zip(&classes)
            .map(|(list, class)| {
                let mut theirs: Vec<u64> = list.iter().filter_map(|other| classes.get(*other as usize).copied()).collect();
                theirs.sort_unstable();
                (*class, theirs)
            })
            .collect();
        let mut numbers: BTreeMap<&(u64, Vec<u64>), u64> = BTreeMap::new();
        for signature in &signatures {
            let next = numbers.len() as u64;
            numbers.entry(signature).or_insert(next);
        }
        let refined: Vec<u64> = signatures.iter().map(|signature| numbers.get(signature).copied().unwrap_or(0)).collect();
        let refined_count = numbers.len();
        classes = refined;
        if refined_count <= count {
            break;
        }
        count = refined_count;
    }
    classes
}

/// The name for the other side, as the reference flips bone and vertex group names: a trailing
/// `.` and digits stays; then `L`/`R` (either case) after a `.`, `_`, `-`, or space at the end,
/// or before one at the start; else, in names longer than five letters, `Right` or `Left` (in
/// any case, written back lower, upper, or capitalised as its first two letters are) at the
/// start, then at the end, `Right` before `Left`. Other names stay as they are.
pub fn flip_name(name: &str) -> String {
    let (base, number) = match name.rfind('.') {
        Some(at) if at + 1 < name.len() && name.get(at + 1..).is_some_and(|digits| digits.bytes().all(|byte| byte.is_ascii_digit())) => {
            (name.get(..at).unwrap_or(name), name.get(at..).unwrap_or(""))
        }
        _ => (name, ""),
    };
    let chars: Vec<char> = base.chars().collect();
    let separator = |c: char| matches!(c, '.' | '_' | '-' | ' ');
    let side = |c: char| match c {
        'L' => Some('R'),
        'R' => Some('L'),
        'l' => Some('r'),
        'r' => Some('l'),
        _ => None,
    };
    let count = chars.len();
    let mut flipped = chars.clone();
    let mut done = false;
    if count >= 3
        && let (Some(last), Some(before)) = (chars.last().copied(), chars.get(count - 2).copied())
        && separator(before)
        && let Some(other) = side(last)
    {
        if let Some(slot) = flipped.last_mut() {
            *slot = other;
        }
        done = true;
    }
    if !done
        && count >= 3
        && let (Some(first), Some(after)) = (chars.first().copied(), chars.get(1).copied())
        && separator(after)
        && let Some(other) = side(first)
    {
        if let Some(slot) = flipped.first_mut() {
            *slot = other;
        }
        done = true;
    }
    if done {
        return flipped.into_iter().collect::<String>() + number;
    }
    if count > 5 {
        let cased = |word: &str, like: &[char]| -> String {
            match (like.first(), like.get(1)) {
                (Some(first), _) if first.is_lowercase() => word.to_lowercase(),
                (_, Some(second)) if second.is_uppercase() => word.to_uppercase(),
                _ => {
                    let mut out = word.to_lowercase();
                    if let Some(head) = out.get_mut(..1) {
                        head.make_ascii_uppercase();
                    }
                    out
                }
            }
        };
        for (find, put) in [("right", "left"), ("left", "right")] {
            let size = find.len();
            let starts: String = chars.iter().take(size).collect();
            if starts.eq_ignore_ascii_case(find) {
                let rest: String = chars.iter().skip(size).collect();
                return cased(put, chars.get(..size).unwrap_or(&[])) + &rest + number;
            }
            let ends: String = chars.iter().skip(count - size).collect();
            if ends.eq_ignore_ascii_case(find) {
                let rest: String = chars.iter().take(count - size).collect();
                return rest + &cased(put, chars.get(count - size..).unwrap_or(&[])) + number;
            }
        }
    }
    name.to_string()
}

/// The gradient tool's shape.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GradientKind {
    /// Full weight up to the start, fading across the line to the end.
    #[default]
    Linear,
    /// Full weight at the start, fading out to the end's distance.
    Radial,
}

/// The gradient tool from `start` to `end` (viewport pixels) on group `group`: each vertex
/// drawn at `screen` moves towards the brush's weight by the brush's falloff over the share of
/// the way it is from start to end (along the line, or by distance for radial) times the
/// strength; vertices at or past the end stay. X mirror does not apply, as in the reference.
pub fn gradient(
    mesh: &mut Mesh,
    group: &str,
    screen: &[Option<(f32, f32)>],
    start: (f32, f32),
    end: (f32, f32),
    kind: GradientKind,
    brush: &Brush,
) -> Result<()> {
    brush.check()?;
    let (dx, dy) = (end.0 - start.0, end.1 - start.1);
    let length = dx.hypot(dy);
    if !(length.is_finite() && length > 1e-6 && start.0.is_finite() && start.1.is_finite()) {
        return Err(SceneError::params("a gradient needs two different points"));
    }
    let weights = &mut group_mut(mesh, group)?.weights;
    for (weight, place) in weights.iter_mut().zip(screen) {
        let Some((x, y)) = *place else { continue };
        let share = match kind {
            GradientKind::Linear => ((x - start.0) * dx + (y - start.1) * dy) / (length * length),
            GradientKind::Radial => (x - start.0).hypot(y - start.1) / length,
        };
        if !share.is_finite() {
            continue;
        }
        let touch = crate::proportional::weight(brush.falloff, share.max(0.0), 1.0) * brush.strength;
        if touch > 0.0 {
            *weight = (*weight + (brush.weight - *weight) * touch).clamp(0.0, 1.0);
        }
    }
    Ok(())
}

/// Scales every vertex's weights over all groups so they sum to one (vertices in no group stay
/// out). With `locked`, that group's weights stay and the others share what is left.
pub fn normalize_all(mesh: &mut Mesh, locked: Option<&str>) {
    let count = mesh.positions.len();
    for group in &mut mesh.groups {
        group.weights.resize(count, 0.0);
    }
    for vertex in 0..count {
        let kept = locked
            .and_then(|name| mesh.groups.iter().find(|group| group.name == name))
            .and_then(|group| group.weights.get(vertex).copied())
            .unwrap_or(0.0);
        let others: f32 = mesh.groups.iter().filter(|group| Some(group.name.as_str()) != locked).filter_map(|group| group.weights.get(vertex)).sum();
        if others <= 0.0 {
            continue;
        }
        let scale = (1.0 - kept).max(0.0) / others;
        for group in mesh.groups.iter_mut().filter(|group| Some(group.name.as_str()) != locked) {
            if let Some(weight) = group.weights.get_mut(vertex) {
                *weight *= scale;
            }
        }
    }
}

/// The vertices joined to each vertex by an edge.
fn neighbours(mesh: &Mesh) -> Vec<Vec<u32>> {
    let mut out: Vec<BTreeSet<u32>> = vec![BTreeSet::new(); mesh.positions.len()];
    for [a, b] in mesh.edges() {
        if let Some(set) = out.get_mut(a as usize) {
            set.insert(b);
        }
        if let Some(set) = out.get_mut(b as usize) {
            set.insert(a);
        }
    }
    out.into_iter().map(|set| set.into_iter().collect()).collect()
}

/// The reference application's weight colours: blue (0) through cyan, green, and yellow to
/// red (1), as linear RGB.
pub fn ramp(weight: f32) -> [f32; 3] {
    let stops: [(f32, [f32; 3]); 5] =
        [(0.0, [0.0, 0.0, 1.0]), (0.25, [0.0, 1.0, 1.0]), (0.5, [0.0, 1.0, 0.0]), (0.75, [1.0, 1.0, 0.0]), (1.0, [1.0, 0.0, 0.0])];
    let weight = if weight.is_finite() { weight.clamp(0.0, 1.0) } else { 0.0 };
    for pair in stops.windows(2) {
        if let [(low, from), (high, to)] = pair
            && weight <= *high
        {
            let t = (weight - low) / (high - low);
            return [from[0] + (to[0] - from[0]) * t, from[1] + (to[1] - from[1]) * t, from[2] + (to[2] - from[2]) * t];
        }
    }
    [1.0, 0.0, 0.0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::Primitive;

    fn plane() -> Mesh {
        Primitive::Grid { x_subdivisions: 4, y_subdivisions: 4, size: 2.0 }.build().unwrap()
    }

    #[test]
    fn modes_change_weights_as_named() {
        let mut mesh = plane();
        let draw = Brush::default();
        dab(&mut mesh, "Group", &[(0, 1.0), (1, 0.5)], &draw).unwrap();
        let weights = &mesh.groups[0].weights;
        assert_eq!((weights[0], weights[1], weights[2]), (1.0, 0.5, 0.0));
        dab(&mut mesh, "Group", &[(0, 0.25)], &Brush { mode: BrushMode::Subtract, weight: 1.0, ..draw }).unwrap();
        assert!((mesh.groups[0].weights[0] - 0.75).abs() < 1e-6);
        dab(&mut mesh, "Group", &[(2, 0.5)], &Brush { mode: BrushMode::Add, weight: 0.4, ..draw }).unwrap();
        assert!((mesh.groups[0].weights[2] - 0.2).abs() < 1e-6);
        // Blur pulls a vertex towards its neighbours' mean.
        let mut mesh = plane();
        dab(&mut mesh, "Group", &[(12, 1.0)], &draw).unwrap();
        dab(&mut mesh, "Group", &[(12, 1.0)], &Brush { mode: BrushMode::Blur, ..draw }).unwrap();
        assert!(mesh.groups[0].weights[12] < 0.1);
        assert!(dab(&mut mesh, "Group", &[(0, 1.0)], &Brush { strength: 2.0, ..draw }).is_err());
    }

    #[test]
    fn normalising_makes_each_vertex_sum_to_one() {
        let mut mesh = plane();
        dab(&mut mesh, "A", &[(0, 1.0), (1, 1.0)], &Brush { weight: 0.6, ..Brush::default() }).unwrap();
        dab(&mut mesh, "B", &[(0, 1.0)], &Brush { weight: 0.9, ..Brush::default() }).unwrap();
        normalize_all(&mut mesh, None);
        let (a, b) = (&mesh.groups[0].weights, &mesh.groups[1].weights);
        assert!((a[0] + b[0] - 1.0).abs() < 1e-6 && (a[1] - 1.0).abs() < 1e-6 && a[2] == 0.0);
        // A locked group keeps its weight.
        let mut mesh = plane();
        dab(&mut mesh, "A", &[(0, 1.0)], &Brush { weight: 0.3, ..Brush::default() }).unwrap();
        dab(&mut mesh, "B", &[(0, 1.0)], &Brush { weight: 0.3, ..Brush::default() }).unwrap();
        dab(&mut mesh, "C", &[(0, 1.0)], &Brush { weight: 0.1, ..Brush::default() }).unwrap();
        normalize_all(&mut mesh, Some("A"));
        let weight = |name: &str| mesh.groups.iter().find(|group| group.name == name).unwrap().weights[0];
        assert!((weight("A") - 0.3).abs() < 1e-6 && (weight("B") - 0.525).abs() < 1e-5 && (weight("C") - 0.175).abs() < 1e-5);
    }

    #[test]
    fn the_ramp_runs_blue_to_red() {
        assert_eq!(ramp(0.0), [0.0, 0.0, 1.0]);
        assert_eq!(ramp(0.5), [0.0, 1.0, 0.0]);
        assert_eq!(ramp(1.0), [1.0, 0.0, 0.0]);
        assert_eq!(ramp(0.125), [0.0, 0.5, 1.0]);
    }
}
