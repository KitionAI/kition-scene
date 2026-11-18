//! Cloth: vertices with mass joined by links along the mesh's edges (stretch), across its quads
//! (shear), and over its edges (bending), solved as extended position-based dynamics, with
//! pinned vertices, air damping, and collisions against collision objects' surfaces.

use std::collections::BTreeMap;

use glam::Vec3;
use kition_scene_model::physics::{Effector, FieldKind};

use crate::collide::closest_on_triangle;

/// One link: two vertices kept `rest` apart, as softly as `compliance` allows.
#[derive(Debug, Clone, Copy)]
struct Link {
    a: usize,
    b: usize,
    rest: f32,
    compliance: f32,
}

/// Cloth settings the solver uses.
#[derive(Debug, Clone, Copy)]
pub struct ClothSettings {
    pub mass: f32,
    pub tension: f32,
    pub shear: f32,
    pub bending: f32,
    pub air_damping: f32,
    pub gravity: Vec3,
}

/// A simulated cloth, in world coordinates.
#[derive(Debug, Clone)]
pub struct ClothSim {
    pub positions: Vec<Vec3>,
    velocities: Vec<Vec3>,
    pinned: Vec<bool>,
    links: Vec<Link>,
    settings: ClothSettings,
    /// How close the cloth comes to collision surfaces.
    pub distance: f32,
    /// The faces as triangles (for fields and self collision).
    triangles: Vec<[usize; 3]>,
    /// How far apart the cloth keeps from itself (none: it passes through itself).
    pub self_distance: Option<f32>,
}

/// Air damping: the share of speed a vertex of 1 kg loses each second per unit of the setting
/// (measured against the reference's cloth falling with and without it).
const AIR: f32 = 0.24;
/// How hard fields push cloth: per square metre of cloth round a vertex, per unit of field
/// strength, per second of frame (measured against the reference's cloth in a Force field).
pub const FIELD_SCALE: f32 = 11.8;

/// How strongly links of each kind hold, per unit of the reference's stiffness settings.
const STRETCH_SCALE: f32 = 2000.0;
const BEND_SCALE: f32 = 40.0;

impl ClothSim {
    /// A cloth over a mesh's vertices (world positions) and faces; `pinned` vertices follow the
    /// mesh.
    pub fn new(positions: Vec<Vec3>, faces: &[Vec<u32>], pinned: Vec<bool>, settings: ClothSettings, distance: f32) -> Self {
        let at = |index: usize| positions.get(index).copied().unwrap_or(Vec3::ZERO);
        let mut links: BTreeMap<(usize, usize), (f32, f32)> = BTreeMap::new();
        let mut add = |a: usize, b: usize, stiffness: f32| {
            if a == b {
                return;
            }
            let key = (a.min(b), a.max(b));
            let compliance = 1.0 / stiffness.max(1e-6);
            let rest = at(a).distance(at(b));
            // A pair already linked keeps its stiffest link.
            let entry = links.entry(key).or_insert((rest, compliance));
            entry.1 = entry.1.min(compliance);
        };
        let stretch = settings.tension.max(1e-3) * STRETCH_SCALE;
        let shear = settings.shear.max(1e-3) * STRETCH_SCALE;
        let bend = settings.bending.max(1e-4) * BEND_SCALE;
        // Edges of each face: stretch; across quads: shear.
        let mut face_of_edge: BTreeMap<(usize, usize), Vec<(usize, usize)>> = BTreeMap::new();
        for face in faces {
            let count = face.len();
            let corner = |slot: usize| face.get(slot % count).map_or(0, |index| *index as usize);
            for slot in 0..count {
                let (a, b) = (corner(slot), corner(slot + 1));
                add(a, b, stretch);
                // The vertices beside the edge on this face: after b, and before a.
                face_of_edge.entry((a.min(b), a.max(b))).or_default().push((corner(slot + 2), corner(slot + count - 1)));
            }
            if count == 4 {
                add(corner(0), corner(2), shear);
                add(corner(1), corner(3), shear);
            }
        }
        // Over each edge between two faces: bending.
        let mut bends: Vec<(usize, usize)> = Vec::new();
        for (edge, sides) in &face_of_edge {
            if let [(after_one, before_one), (after_two, before_two)] = sides.as_slice() {
                // Each face runs the edge in turn; the other side's "after b" sits beside a.
                let _ = edge;
                bends.push((*after_one, *before_two));
                bends.push((*before_one, *after_two));
            }
        }
        for (a, b) in bends {
            let key = (a.min(b), a.max(b));
            if a != b && !links.contains_key(&key) {
                links.insert(key, (at(a).distance(at(b)), 1.0 / bend));
            }
        }
        let links = links.into_iter().map(|((a, b), (rest, compliance))| Link { a, b, rest, compliance }).collect();
        let count = positions.len();
        let mut triangles = Vec::new();
        for face in faces {
            let corner = |slot: usize| face.get(slot).map_or(0, |index| *index as usize);
            for slot in 1..face.len().saturating_sub(1) {
                let triangle = [corner(0), corner(slot), corner(slot + 1)];
                if triangle.iter().all(|index| *index < count) {
                    triangles.push(triangle);
                }
            }
        }
        Self { velocities: vec![Vec3::ZERO; count], positions, pinned, links, settings, distance, triangles, self_distance: None }
    }

    /// Each vertex's share of the cloth's area and its normal (area weighted), now.
    fn areas_and_normals(&self) -> (Vec<f32>, Vec<Vec3>) {
        let count = self.positions.len();
        let mut areas = vec![0.0f32; count];
        let mut normals = vec![Vec3::ZERO; count];
        for triangle in &self.triangles {
            let [a, b, c] = triangle.map(|index| self.positions.get(index).copied().unwrap_or(Vec3::ZERO));
            let cross = (b - a).cross(c - a);
            let area = cross.length() * 0.5;
            for index in triangle {
                if let (Some(slot), Some(normal)) = (areas.get_mut(*index), normals.get_mut(*index)) {
                    *slot += area / 3.0;
                    *normal += cross;
                }
            }
        }
        (areas, normals.into_iter().map(|normal| normal.normalize_or_zero()).collect())
    }

    /// The acceleration `effectors` give each vertex: the field's push on the cloth round it
    /// (Wind only across the cloth, along its normal), `scale` per unit of push and area.
    pub fn field_push(&self, effectors: &[Effector], scale: f32) -> Vec<Vec3> {
        if effectors.is_empty() {
            return Vec::new();
        }
        let (areas, normals) = self.areas_and_normals();
        let inverse_mass = 1.0 / self.settings.mass.max(1e-6);
        self.positions
            .iter()
            .zip(&self.velocities)
            .enumerate()
            .map(|(index, (point, velocity))| {
                let area = areas.get(index).copied().unwrap_or(0.0);
                let normal = normals.get(index).copied().unwrap_or(Vec3::ZERO);
                effectors
                    .iter()
                    .map(|effector| {
                        let force = effector.push(*point, *velocity).force;
                        let force = if effector.field.kind == FieldKind::Wind { normal * force.dot(normal) } else { force };
                        force * (scale * area * inverse_mass)
                    })
                    .sum()
            })
            .collect()
    }

    /// Keeps vertices `distance` from the cloth's own triangles, on the side they were on.
    fn separate(&mut self, previous: &[Vec3], distance: f32) {
        let triangles: Vec<[Vec3; 3]> =
            self.triangles.iter().map(|triangle| triangle.map(|index| self.positions.get(index).copied().unwrap_or(Vec3::ZERO))).collect();
        let grid = crate::shape::TriangleMesh::new(triangles.clone());
        let free = |pinned: &[bool], index: usize| !pinned.get(index).copied().unwrap_or(false);
        for vertex in 0..self.positions.len() {
            let Some(point) = self.positions.get(vertex).copied() else { continue };
            let reach = Vec3::splat(distance);
            for found in grid.near(point - reach, point + reach) {
                let Some(corners) = self.triangles.get(found as usize).copied() else { continue };
                if corners.contains(&vertex) {
                    continue;
                }
                let Some([a, b, c]) = triangles.get(found as usize).copied() else { continue };
                let normal = (b - a).cross(c - a).normalize_or_zero();
                if normal == Vec3::ZERO {
                    continue;
                }
                let closest = closest_on_triangle(point, a, b, c);
                if point.distance_squared(closest) >= distance * distance {
                    continue;
                }
                // The side the vertex was on at the start of the step.
                let before = |index: usize| previous.get(index).copied().unwrap_or(Vec3::ZERO);
                let [pa, pb, pc] = corners.map(before);
                let was = (before(vertex) - (pa + pb + pc) / 3.0).dot((pb - pa).cross(pc - pa));
                let side = if was >= 0.0 { normal } else { -normal };
                let depth = distance - (point - closest).dot(side);
                if depth <= 0.0 {
                    continue;
                }
                let movable = corners.iter().filter(|index| free(&self.pinned, **index)).count();
                let (own, theirs) = match (free(&self.pinned, vertex), movable > 0) {
                    (true, true) => (0.5, 0.5),
                    (true, false) => (1.0, 0.0),
                    (false, true) => (0.0, 1.0),
                    (false, false) => continue,
                };
                if let Some(slot) = self.positions.get_mut(vertex) {
                    *slot += side * (depth * own);
                }
                for index in corners {
                    if free(&self.pinned, index)
                        && let Some(slot) = self.positions.get_mut(index)
                    {
                        *slot -= side * (depth * theirs);
                    }
                }
            }
        }
    }

    pub(crate) fn is_finite(&self) -> bool {
        self.positions.iter().chain(&self.velocities).all(|value| value.is_finite())
    }

    /// One step of `dt`: pinned vertices go to `pins` (world positions of the mesh as animated).
    /// `push` is each vertex's extra acceleration (from fields; empty for none).
    pub fn step(&mut self, dt: f32, iterations: u32, pins: &[Vec3], colliders: &[ColliderSurface], push: &[Vec3]) {
        if dt.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
            return;
        }
        let previous = self.positions.clone();
        let damping = 1.0 / (1.0 + self.settings.air_damping * AIR / self.settings.mass.max(1e-6) * dt);
        for (index, (position, velocity)) in self.positions.iter_mut().zip(&mut self.velocities).enumerate() {
            if self.pinned.get(index).copied().unwrap_or(false) {
                *position = pins.get(index).copied().unwrap_or(*position);
                *velocity = Vec3::ZERO;
                continue;
            }
            let extra = push.get(index).copied().unwrap_or(Vec3::ZERO);
            *velocity = (*velocity + (self.settings.gravity + extra) * dt) * damping;
            *position += *velocity * dt;
        }
        let inverse_mass = 1.0 / self.settings.mass.max(1e-6);
        let weight = |index: usize, pinned: &[bool]| if pinned.get(index).copied().unwrap_or(false) { 0.0 } else { inverse_mass };
        let mut lambdas = vec![0.0f32; self.links.len()];
        for _ in 0..iterations.max(1) {
            for (link, lambda) in self.links.iter().zip(&mut lambdas) {
                let (wa, wb) = (weight(link.a, &self.pinned), weight(link.b, &self.pinned));
                if wa + wb <= 0.0 {
                    continue;
                }
                let (Some(&pa), Some(&pb)) = (self.positions.get(link.a), self.positions.get(link.b)) else { continue };
                let delta = pa - pb;
                let length = delta.length();
                if length < 1e-9 {
                    continue;
                }
                let gradient = delta / length;
                let constraint = length - link.rest;
                let alpha = link.compliance / (dt * dt);
                let change = (-constraint - alpha * *lambda) / (wa + wb + alpha);
                *lambda += change;
                if let Some(point) = self.positions.get_mut(link.a) {
                    *point += gradient * (change * wa);
                }
                if let Some(point) = self.positions.get_mut(link.b) {
                    *point -= gradient * (change * wb);
                }
            }
        }
        // Stop before spatial collision indexing if a strong field diverged.
        if !self.is_finite() {
            return;
        }
        if let Some(distance) = self.self_distance {
            self.separate(&previous, distance);
        }
        // Collisions: out of every collision surface, with friction along it.
        for (index, position) in self.positions.iter_mut().enumerate() {
            if self.pinned.get(index).copied().unwrap_or(false) {
                continue;
            }
            let before = previous.get(index).copied().unwrap_or(*position);
            for collider in colliders {
                if let Some(corrected) = collider.push_out(*position, before, self.distance) {
                    *position = corrected;
                }
            }
        }
        for ((velocity, position), before) in self.velocities.iter_mut().zip(&self.positions).zip(&previous) {
            *velocity = (*position - *before) / dt;
        }
        for (index, pinned) in self.pinned.iter().enumerate() {
            if *pinned && let Some(velocity) = self.velocities.get_mut(index) {
                *velocity = Vec3::ZERO;
            }
        }
    }
}

/// A collision object's surface in the world, for cloth.
#[derive(Debug, Clone)]
pub struct ColliderSurface {
    pub triangles: Vec<[Vec3; 3]>,
    pub thickness: f32,
    pub friction: f32,
    grid: crate::shape::TriangleMesh,
}

impl ColliderSurface {
    pub fn new(triangles: Vec<[Vec3; 3]>, thickness: f32, friction: f32) -> Self {
        let grid = crate::shape::TriangleMesh::new(triangles.clone());
        Self { triangles, thickness, friction, grid }
    }

    /// Where `point` (coming from `before`) must go to stay `distance` plus the thickness
    /// outside the surface, if it is too close or went through, with friction.
    fn push_out(&self, point: Vec3, before: Vec3, distance: f32) -> Option<Vec3> {
        let (target, normal) = self.contact(point, before, distance)?;
        Some(self.with_friction(target, before, normal))
    }

    /// Where `point` (coming from `before`) must go to stay `distance` plus the thickness
    /// outside the surface, and the surface's normal there, if it is too close or went through.
    pub(crate) fn contact(&self, point: Vec3, before: Vec3, distance: f32) -> Option<(Vec3, Vec3)> {
        let reach = self.thickness + distance;
        let low = point.min(before) - Vec3::splat(reach);
        let high = point.max(before) + Vec3::splat(reach);
        let mut best: Option<(f32, Vec3, Vec3)> = None;
        for index in self.grid.near(low, high) {
            let Some([a, b, c]) = self.triangles.get(index as usize).copied() else { continue };
            let normal = (b - a).cross(c - a).normalize_or_zero();
            if normal == Vec3::ZERO {
                continue;
            }
            let closest = closest_on_triangle(point, a, b, c);
            let offset = point - closest;
            let signed = offset.dot(normal);
            // Went through this step: it was in front and is now behind.
            let crossed = (before - a).dot(normal) >= 0.0 && signed < 0.0 && offset.length() < reach + (point - before).length();
            let close = offset.length() < reach && signed >= -reach;
            if !(close || crossed) {
                continue;
            }
            let gap = offset.length();
            if best.is_none_or(|(nearest, _, _)| gap < nearest) {
                best = Some((gap, closest, normal));
            }
        }
        let (_, closest, normal) = best?;
        Some((closest + normal * (self.thickness + distance), normal))
    }

    /// `target` with friction: only part of the slide from `before` along the surface (of
    /// `normal`) is kept this step.
    pub(crate) fn with_friction(&self, target: Vec3, before: Vec3, normal: Vec3) -> Vec3 {
        let along = (target - before) - normal * (target - before).dot(normal);
        let keep = 1.0 / (1.0 + self.friction * 0.2);
        target - along * (1.0 - keep)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(count: usize, size: f32, height: f32) -> (Vec<Vec3>, Vec<Vec<u32>>) {
        let mut positions = Vec::new();
        for y in 0..=count {
            for x in 0..=count {
                positions.push(Vec3::new(-size / 2.0 + size * x as f32 / count as f32, -size / 2.0 + size * y as f32 / count as f32, height));
            }
        }
        let at = |x: usize, y: usize| (y * (count + 1) + x) as u32;
        let mut faces = Vec::new();
        for y in 0..count {
            for x in 0..count {
                faces.push(vec![at(x, y), at(x + 1, y), at(x + 1, y + 1), at(x, y + 1)]);
            }
        }
        (positions, faces)
    }

    fn settings() -> ClothSettings {
        ClothSettings { mass: 0.3, tension: 15.0, shear: 5.0, bending: 0.5, air_damping: 1.0, gravity: Vec3::new(0.0, 0.0, -9.81) }
    }

    #[test]
    fn a_pinned_corner_holds_the_cloth_up_and_edges_barely_stretch() {
        let (positions, faces) = sheet(8, 2.0, 0.0);
        let mut pinned = vec![false; positions.len()];
        pinned[0] = true;
        let pins = positions.clone();
        let mut cloth = ClothSim::new(positions.clone(), &faces, pinned, settings(), 0.015);
        for _ in 0..(24 * 2 * 20) {
            cloth.step(1.0 / 480.0, 4, &pins, &[], &[]);
        }
        assert!(cloth.positions[0].abs_diff_eq(positions[0], 1e-6), "the pin holds");
        let far = cloth.positions.last().unwrap();
        assert!(far.z < -1.5, "the free corner hangs: {far}");
        // The diagonal hangs about its own length (2.83 m) below the pin, stretched a little.
        assert!(far.distance(cloth.positions[0]) < 2.83 * 1.08, "{}", far.distance(cloth.positions[0]));
    }

    #[test]
    fn cloth_drapes_over_a_floor_without_going_through() {
        let (positions, faces) = sheet(6, 1.0, 0.5);
        let pinned = vec![false; positions.len()];
        let floor = ColliderSurface::new(
            vec![
                [Vec3::new(-5.0, -5.0, 0.0), Vec3::new(5.0, -5.0, 0.0), Vec3::new(5.0, 5.0, 0.0)],
                [Vec3::new(-5.0, -5.0, 0.0), Vec3::new(5.0, 5.0, 0.0), Vec3::new(-5.0, 5.0, 0.0)],
            ],
            0.02,
            5.0,
        );
        let mut cloth = ClothSim::new(positions.clone(), &faces, pinned, settings(), 0.015);
        for _ in 0..(24 * 2 * 10) {
            cloth.step(1.0 / 240.0, 4, &positions, std::slice::from_ref(&floor), &[]);
        }
        for position in &cloth.positions {
            assert!(position.z > 0.02 && position.z < 0.05, "rests on the floor: {position}");
        }
    }

    #[test]
    fn self_collision_keeps_two_layers_apart() {
        // Two sheets, one above the other and offset, fall onto a floor.
        let drop = |self_distance: Option<f32>| {
            let (mut positions, mut faces) = sheet(8, 1.0, 0.3);
            let (upper, upper_faces) = sheet(8, 1.0, 0.6);
            let base = positions.len() as u32;
            positions.extend(upper.iter().map(|point| *point + Vec3::new(0.031, 0.027, 0.0)));
            faces.extend(upper_faces.iter().map(|face| face.iter().map(|index| index + base).collect()));
            let floor = ColliderSurface::new(
                vec![
                    [Vec3::new(-5.0, -5.0, 0.0), Vec3::new(5.0, -5.0, 0.0), Vec3::new(5.0, 5.0, 0.0)],
                    [Vec3::new(-5.0, -5.0, 0.0), Vec3::new(5.0, 5.0, 0.0), Vec3::new(-5.0, 5.0, 0.0)],
                ],
                0.02,
                5.0,
            );
            let mut cloth = ClothSim::new(positions.clone(), &faces, vec![false; positions.len()], settings(), 0.015);
            cloth.self_distance = self_distance;
            for _ in 0..(240 * 2) {
                cloth.step(1.0 / 240.0, 4, &positions, std::slice::from_ref(&floor), &[]);
            }
            let half = positions.len() / 2;
            let mean = |range: &[Vec3]| range.iter().map(|point| point.z).sum::<f32>() / range.len() as f32;
            mean(&cloth.positions[half..]) - mean(&cloth.positions[..half])
        };
        let apart = drop(Some(0.015));
        let through = drop(None);
        assert!(apart > 0.01, "the upper sheet lies on the lower: {apart}");
        assert!(through.abs() < 0.005, "without self collision they meet: {through}");
    }
}
