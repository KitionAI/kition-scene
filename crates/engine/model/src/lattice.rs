//! Lattices: a grid of points in a unit cube round the object's origin whose moved points bend
//! the meshes of Lattice modifiers that name it. A mesh point moves by the grid's offsets,
//! blended by a uniform cubic B-spline (the end points repeating past the grid, so far outside it
//! a point moves with the nearest layer) or trilinearly (clamped to the grid), as the
//! reference application does (checked in `tests/lattice_blender.rs`).

use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

use crate::error::{Result, SceneError};
use crate::mesh::Mesh;
use crate::scene::{Id, Object, ObjectData, Scene};

/// The most points along one side of a lattice.
pub const MAX_SIDE: u32 = 64;

/// How a lattice blends its points' offsets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LatticeInterpolation {
    #[default]
    Bspline,
    Linear,
}

/// A grid of `resolution[0] × resolution[1] × resolution[2]` points, U fastest then V then W.
/// Each point rests evenly spaced in the cube from -0.5 to 0.5 on every axis; `points` holds
/// where each one is now, in the lattice object's own axes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lattice {
    pub resolution: [u32; 3],
    pub points: Vec<Vec3>,
    #[serde(default)]
    pub interpolation: LatticeInterpolation,
}

impl Default for Lattice {
    fn default() -> Self {
        Self::new([2, 2, 2])
    }
}

/// Where point `index` of a side with `count` points rests.
fn rest_coordinate(index: u32, count: u32) -> f32 {
    if count <= 1 { 0.0 } else { index as f32 / (count - 1) as f32 - 0.5 }
}

/// The uniform cubic B-spline weight at `distance` points from a point.
fn bspline(distance: f32) -> f32 {
    let x = distance.abs();
    if x < 1.0 {
        (4.0 - 6.0 * x * x + 3.0 * x * x * x) / 6.0
    } else if x < 2.0 {
        (2.0 - x).powi(3) / 6.0
    } else {
        0.0
    }
}

/// The weights of the points of one side at `u` (in point spacings from the first point).
fn side_weights(u: f32, count: u32, interpolation: LatticeInterpolation) -> Vec<(usize, f32)> {
    let last = count.saturating_sub(1) as i64;
    if last == 0 {
        return vec![(0, 1.0)];
    }
    match interpolation {
        LatticeInterpolation::Linear => {
            let u = u.clamp(0.0, last as f32);
            let low = (u.floor() as i64).min(last - 1);
            let t = u - low as f32;
            vec![(low as usize, 1.0 - t), (low as usize + 1, t)]
        }
        LatticeInterpolation::Bspline => {
            let mut out: Vec<(usize, f32)> = Vec::new();
            let centre = u.floor() as i64;
            // Places past either end repeat the end point.
            for virtual_index in (centre - 1)..=(centre + 2) {
                let weight = bspline(u - virtual_index as f32);
                if weight <= 0.0 {
                    continue;
                }
                let index = virtual_index.clamp(0, last) as usize;
                match out.iter_mut().find(|(slot, _)| *slot == index) {
                    Some((_, total)) => *total += weight,
                    None => out.push((index, weight)),
                }
            }
            out
        }
    }
}

impl Lattice {
    /// A lattice at rest with `resolution` points along U, V, and W.
    pub fn new(resolution: [u32; 3]) -> Self {
        let mut lattice = Self { resolution, points: Vec::new(), interpolation: LatticeInterpolation::default() };
        lattice.points = lattice.rest_points();
        lattice
    }

    /// How many points the lattice has.
    pub fn point_count(&self) -> usize {
        self.resolution.iter().map(|side| *side as usize).product()
    }

    /// The index of point (u, v, w).
    pub fn index(&self, u: u32, v: u32, w: u32) -> usize {
        let [nu, nv, _] = self.resolution.map(|side| side as usize);
        u as usize + v as usize * nu + w as usize * nu * nv
    }

    /// Where every point rests, in order.
    pub fn rest_points(&self) -> Vec<Vec3> {
        let [nu, nv, nw] = self.resolution;
        let mut out = Vec::with_capacity(self.point_count());
        for w in 0..nw {
            for v in 0..nv {
                for u in 0..nu {
                    out.push(Vec3::new(rest_coordinate(u, nu), rest_coordinate(v, nv), rest_coordinate(w, nw)));
                }
            }
        }
        out
    }

    /// Checks the resolution and that there is one finite point per grid place.
    pub fn check(&self) -> Result<()> {
        if self.resolution.iter().any(|side| !(1..=MAX_SIDE).contains(side)) {
            return Err(SceneError::params(format!("a lattice has 1 to {MAX_SIDE} points along each side")));
        }
        if self.points.len() != self.point_count() {
            return Err(SceneError::params("a lattice needs one point per place in its grid"));
        }
        if self.points.iter().any(|point| !point.is_finite()) {
            return Err(SceneError::params("a lattice point is not a finite number"));
        }
        Ok(())
    }

    /// How far a point at `local` (lattice axes) moves.
    pub fn offset_at(&self, local: Vec3) -> Vec3 {
        if self.points.len() != self.point_count() {
            return Vec3::ZERO;
        }
        let rest = self.rest_points();
        let place = |coordinate: f32, count: u32| if count <= 1 { 0.0 } else { (coordinate + 0.5) * (count - 1) as f32 };
        let [nu, nv, nw] = self.resolution;
        let weights_u = side_weights(place(local.x, nu), nu, self.interpolation);
        let weights_v = side_weights(place(local.y, nv), nv, self.interpolation);
        let weights_w = side_weights(place(local.z, nw), nw, self.interpolation);
        let mut offset = Vec3::ZERO;
        for (w, weight_w) in &weights_w {
            for (v, weight_v) in &weights_v {
                for (u, weight_u) in &weights_u {
                    let index = self.index(*u as u32, *v as u32, *w as u32);
                    let (Some(point), Some(at_rest)) = (self.points.get(index), rest.get(index)) else { continue };
                    offset += (*point - *at_rest) * (weight_u * weight_v * weight_w);
                }
            }
        }
        offset
    }

    /// The lattice with another resolution, keeping its shape: each new point moves as the old
    /// lattice moves the place it rests at.
    pub fn resized(&self, resolution: [u32; 3]) -> Self {
        let mut out = Self { resolution, points: Vec::new(), interpolation: self.interpolation };
        out.points = out.rest_points().into_iter().map(|rest| rest + self.offset_at(rest)).collect();
        out
    }

    /// The grid's edges between neighbouring points, as point index pairs (for drawing).
    pub fn edges(&self) -> Vec<(usize, usize)> {
        let [nu, nv, nw] = self.resolution;
        let mut out = Vec::new();
        for w in 0..nw {
            for v in 0..nv {
                for u in 0..nu {
                    let here = self.index(u, v, w);
                    if u + 1 < nu {
                        out.push((here, self.index(u + 1, v, w)));
                    }
                    if v + 1 < nv {
                        out.push((here, self.index(u, v + 1, w)));
                    }
                    if w + 1 < nw {
                        out.push((here, self.index(u, v, w + 1)));
                    }
                }
            }
        }
        out
    }
}

/// The mesh of `object` bent by lattice object `lattice`, `strength` of the way (1 the whole
/// offset); `None` when that is not a lattice.
pub fn deform(mesh: &Mesh, object: &Object, scene: &Scene, lattice: Id, strength: f32) -> Option<Mesh> {
    let other = scene.object(lattice).ok()?;
    let ObjectData::Lattice(grid) = &other.data else { return None };
    let to_world = scene.world_matrix(object.id).ok()?;
    let lattice_world = scene.world_matrix(lattice).ok()?;
    // The mesh's axes to the lattice's, and back.
    let into: Mat4 = lattice_world.inverse() * to_world;
    let back = into.inverse();
    if !(into.is_finite() && back.is_finite()) {
        return None;
    }
    let mut out = mesh.clone();
    for point in &mut out.positions {
        let local = into.transform_point3(*point);
        let moved = local + grid.offset_at(local) * strength;
        *point = back.transform_point3(moved);
    }
    Some(out)
}
