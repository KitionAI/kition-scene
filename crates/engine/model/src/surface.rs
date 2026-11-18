//! NURBS surface objects: patches of U × V weighted control points, each direction with its own
//! order, resolution, and knots (uniform, clamped to the end points, or Bézier-like), open or
//! cyclic, made into a grid of quads the way the reference application does: `resolution ×
//! points` samples along each direction of a patch, evenly over its parameter range. A patch one
//! point high is a NURBS curve in the surface and draws as a wire. The primitives' control
//! points, weights, and settings follow the reference's Add ▸ Surface menu (checked in
//! `tests/surfaces_blender.rs`).

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::curve::ControlPoint;
use crate::error::{Result, SceneError};
use crate::mesh::Mesh;

fn four() -> u32 {
    4
}

/// The knots of one direction: how a patch's points pull on the surface along it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Knots {
    /// The surface closes on itself along this direction.
    #[serde(default)]
    pub cyclic: bool,
    /// The surface reaches the first and last rows of points.
    #[serde(default)]
    pub endpoint: bool,
    /// Knots repeated as in Bézier segments: the surface passes through every `order - 1`th row.
    #[serde(default)]
    pub bezier: bool,
}

/// One patch of a surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Patch {
    /// `count_u × count_v` points, U changing fastest.
    pub points: Vec<ControlPoint>,
    pub count_u: u32,
    pub count_v: u32,
    #[serde(default = "four")]
    pub order_u: u32,
    #[serde(default = "four")]
    pub order_v: u32,
    #[serde(default = "four")]
    pub resolution_u: u32,
    #[serde(default = "four")]
    pub resolution_v: u32,
    #[serde(default)]
    pub knots_u: Knots,
    #[serde(default)]
    pub knots_v: Knots,
}

fn yes() -> bool {
    true
}

/// A surface object's data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Surface {
    pub patches: Vec<Patch>,
    /// Smooth shading for the surface's faces.
    #[serde(default = "yes")]
    pub smooth: bool,
}

/// The surfaces a person adds from Add ▸ Surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfacePrimitive {
    Curve,
    Circle,
    Patch,
    Cylinder,
    Sphere,
    Torus,
}

impl SurfacePrimitive {
    /// Every primitive, in menu order.
    pub const ALL: [Self; 6] = [Self::Curve, Self::Circle, Self::Patch, Self::Cylinder, Self::Sphere, Self::Torus];

    /// The primitive's id as commands take it.
    pub fn id(self) -> &'static str {
        match self {
            Self::Curve => "curve",
            Self::Circle => "circle",
            Self::Patch => "patch",
            Self::Cylinder => "cylinder",
            Self::Sphere => "sphere",
            Self::Torus => "torus",
        }
    }

    /// The primitive an id names.
    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|primitive| primitive.id() == id)
    }

    /// The name a new object of this primitive gets.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Curve => "SurfCurve",
            Self::Circle => "SurfCircle",
            Self::Patch => "SurfPatch",
            Self::Cylinder => "SurfCylinder",
            Self::Sphere => "SurfSphere",
            Self::Torus => "SurfTorus",
        }
    }

    /// The surface with the reference's points and settings, scaled by `radius`.
    pub fn build(self, radius: f32) -> Surface {
        const H: f32 = std::f32::consts::FRAC_1_SQRT_2;
        let point = |x: f32, y: f32, z: f32, weight: f32| ControlPoint { co: Vec3::new(x, y, z) * radius, weight, radius: 1.0, tilt: 0.0 };
        // The reference's circle: eight points round a square, the corners weighted 1/√2.
        let ring: [(f32, f32, f32); 8] =
            [(0.0, -1.0, 1.0), (-1.0, -1.0, H), (-1.0, 0.0, 1.0), (-1.0, 1.0, H), (0.0, 1.0, 1.0), (1.0, 1.0, H), (1.0, 0.0, 1.0), (1.0, -1.0, H)];
        let circle_knots = Knots { cyclic: true, endpoint: true, bezier: true };
        let patch = |points: Vec<ControlPoint>, count_u: u32, count_v: u32, order_u: u32, order_v: u32, knots_u: Knots, knots_v: Knots| Patch {
            points,
            count_u,
            count_v,
            order_u,
            order_v,
            resolution_u: 4,
            resolution_v: 4,
            knots_u,
            knots_v,
        };
        let patch = match self {
            Self::Curve => patch(
                vec![point(-1.5, 0.0, 0.0, 1.0), point(-1.0, 1.0, 0.0, 1.0), point(1.0, 1.0, 0.0, 1.0), point(1.5, 0.0, 0.0, 1.0)],
                4,
                1,
                4,
                4,
                Knots::default(),
                Knots::default(),
            ),
            Self::Circle => patch(ring.iter().map(|(x, y, w)| point(*x, *y, 0.0, *w)).collect(), 8, 1, 3, 4, circle_knots, Knots::default()),
            Self::Patch => {
                let mut points = Vec::with_capacity(16);
                for x in [-1.5f32, -0.5, 0.5, 1.5] {
                    for y in [-1.5f32, -0.5, 0.5, 1.5] {
                        let inner = x.abs() < 1.0 && y.abs() < 1.0;
                        points.push(point(x, y, if inner { 1.0 } else { 0.0 }, 1.0));
                    }
                }
                patch(points, 4, 4, 4, 4, Knots::default(), Knots::default())
            }
            Self::Cylinder => {
                let points = [-1.0, 1.0]
                    .iter()
                    .flat_map(|z| ring.iter().map(move |(x, y, w)| (*x, *y, *z, *w)))
                    .map(|(x, y, z, w)| point(x, y, z, w))
                    .collect();
                patch(points, 8, 2, 3, 2, circle_knots, Knots::default())
            }
            Self::Sphere => {
                // A half circle from the bottom pole to the top (distance from the axis, height,
                // weight), turned round Z by the ring's eight directions.
                let arc: [(f32, f32, f32); 5] = [(0.0, -1.0, 1.0), (1.0, -1.0, H), (1.0, 0.0, 1.0), (1.0, 1.0, H), (0.0, 1.0, 1.0)];
                let turns: [(f32, f32, f32); 8] = [
                    (-1.0, 0.0, 1.0),
                    (-1.0, -1.0, H),
                    (0.0, -1.0, 1.0),
                    (1.0, -1.0, H),
                    (1.0, 0.0, 1.0),
                    (1.0, 1.0, H),
                    (0.0, 1.0, 1.0),
                    (-1.0, 1.0, H),
                ];
                let mut points = Vec::with_capacity(40);
                for (cx, cy, turn_weight) in turns {
                    for (radial, z, weight) in arc {
                        points.push(point(radial * cx, radial * cy, z, weight * turn_weight));
                    }
                }
                patch(points, 5, 8, 3, 3, Knots { cyclic: false, endpoint: true, bezier: true }, circle_knots)
            }
            Self::Torus => {
                // A small circle (radius ¼, centre ¾ from the axis), turned round Z by the ring.
                let small: [(f32, f32, f32); 8] = [
                    (0.75, -0.25, 1.0),
                    (1.0, -0.25, H),
                    (1.0, 0.0, 1.0),
                    (1.0, 0.25, H),
                    (0.75, 0.25, 1.0),
                    (0.5, 0.25, H),
                    (0.5, 0.0, 1.0),
                    (0.5, -0.25, H),
                ];
                let turns: [(f32, f32, f32); 8] = [
                    (-1.0, 0.0, 1.0),
                    (-1.0, -1.0, H),
                    (0.0, -1.0, 1.0),
                    (1.0, -1.0, H),
                    (1.0, 0.0, 1.0),
                    (1.0, 1.0, H),
                    (0.0, 1.0, 1.0),
                    (-1.0, 1.0, H),
                ];
                let mut points = Vec::with_capacity(64);
                for (cx, cy, turn_weight) in turns {
                    for (radial, z, weight) in small {
                        points.push(point(radial * cx, radial * cy, z, weight * turn_weight));
                    }
                }
                patch(points, 8, 8, 3, 3, circle_knots, circle_knots)
            }
        };
        Surface { patches: vec![patch], smooth: true }
    }
}

/// The largest number of samples one patch may make.
const MAX_SAMPLES: usize = 1_000_000;

/// The knot vector of one direction for `count` points of `order` (cyclic directions wrap their
/// first `order - 1` points round), and the range of parameters the surface spans.
fn knot_vector(count: usize, order: usize, knots: Knots) -> (Vec<f32>, f32, f32) {
    let n = if knots.cyclic { count + order - 1 } else { count };
    let total = n + order;
    let vector: Vec<f32> = if knots.bezier {
        let step = (order - 1).max(1);
        if knots.cyclic {
            (0..total).map(|index| ((index + order - 2) / step) as f32).collect()
        } else {
            // `order` at each end, `order - 1` of each value between.
            (0..total)
                .map(|index| {
                    if index < order {
                        0.0
                    } else if index >= n {
                        ((n - order) / step + 1) as f32
                    } else {
                        ((index - order) / step + 1) as f32
                    }
                })
                .collect()
        }
    } else if knots.endpoint && !knots.cyclic {
        (0..total)
            .map(|index| {
                if index < order {
                    0.0
                } else if index >= n {
                    (n - order + 1) as f32
                } else {
                    (index - order + 1) as f32
                }
            })
            .collect()
    } else {
        (0..total).map(|index| index as f32).collect()
    };
    let start = vector.get(order - 1).copied().unwrap_or(0.0);
    let end = vector.get(n).copied().unwrap_or(1.0);
    (vector, start, end)
}

/// The `order` basis functions' values at `u` (Cox–de Boor), one per control point.
fn basis(knots: &[f32], n: usize, order: usize, u: f32) -> Vec<f32> {
    let k = |slot: usize| knots.get(slot).copied().unwrap_or(0.0);
    let mut values: Vec<f32> = (0..n + order - 1).map(|index| if u >= k(index) && u < k(index + 1) { 1.0 } else { 0.0 }).collect();
    for degree in 1..order {
        for index in 0..(n + order - 1 - degree) {
            let left = {
                let span = k(index + degree) - k(index);
                if span > 0.0 { (u - k(index)) / span * values.get(index).copied().unwrap_or(0.0) } else { 0.0 }
            };
            let right = {
                let span = k(index + degree + 1) - k(index + 1);
                if span > 0.0 { (k(index + degree + 1) - u) / span * values.get(index + 1).copied().unwrap_or(0.0) } else { 0.0 }
            };
            if let Some(value) = values.get_mut(index) {
                *value = left + right;
            }
        }
    }
    values.truncate(n);
    values
}

/// One direction's samples: for each, the basis value of every (wrapped) control point.
fn direction(count: usize, order: usize, resolution: usize, knots: Knots, line: bool) -> Option<(Vec<Vec<f32>>, usize)> {
    let order = order.clamp(2, count.max(2));
    let (vector, start, end) = knot_vector(count, order, knots);
    let n = if knots.cyclic { count + order - 1 } else { count };
    // A surface takes `resolution × points` samples each way; a patch one row high is a curve,
    // which takes `resolution` per segment.
    let samples = if line && !knots.cyclic { count.saturating_sub(1) * resolution } else { count * resolution };
    if !(2..=MAX_SAMPLES).contains(&samples) {
        return None;
    }
    let steps = if knots.cyclic { samples } else { samples - 1 };
    let rows = (0..samples)
        .map(|step| {
            let mut u = start + (end - start) * step as f32 / steps as f32;
            if u >= end {
                u = end - 1e-6 * (end - start).max(1.0);
            }
            basis(&vector, n, order, u)
        })
        .collect();
    Some((rows, n))
}

impl Patch {
    fn point(&self, u: usize, v: usize) -> Option<&ControlPoint> {
        let u = u % self.count_u.max(1) as usize;
        let v = v % self.count_v.max(1) as usize;
        self.points.get(v * self.count_u as usize + u)
    }

    /// The patch's grid of evaluated points (U changing fastest), its size, and whether each
    /// direction closes.
    fn evaluate(&self) -> Option<(Vec<Vec3>, usize, usize)> {
        let (count_u, count_v) = (self.count_u as usize, self.count_v as usize);
        let line = count_v == 1;
        let (rows_u, n_u) = direction(count_u, self.order_u as usize, self.resolution_u.max(1) as usize, self.knots_u, line)?;
        let (rows_v, n_v) = if line {
            (vec![vec![1.0]], 1)
        } else {
            direction(count_v, self.order_v as usize, self.resolution_v.max(1) as usize, self.knots_v, false)?
        };
        if rows_u.len().saturating_mul(rows_v.len()) > MAX_SAMPLES {
            return None;
        }
        let mut out = Vec::with_capacity(rows_u.len() * rows_v.len());
        for row_v in &rows_v {
            for row_u in &rows_u {
                let (mut position, mut weight) = (Vec3::ZERO, 0.0);
                for (v, basis_v) in row_v.iter().enumerate().take(n_v) {
                    if *basis_v == 0.0 {
                        continue;
                    }
                    for (u, basis_u) in row_u.iter().enumerate().take(n_u) {
                        if *basis_u == 0.0 {
                            continue;
                        }
                        let Some(point) = self.point(u, v) else { continue };
                        let w = basis_u * basis_v * point.weight;
                        position += point.co * w;
                        weight += w;
                    }
                }
                out.push(if weight > 1e-12 { position / weight } else { Vec3::ZERO });
            }
        }
        Some((out, rows_u.len(), rows_v.len()))
    }
}

impl Surface {
    /// Checks the surface's sizes and numbers.
    pub fn validate(&self) -> Result<()> {
        let invalid = |message: &str| Err(SceneError::InvalidFile(message.into()));
        for patch in &self.patches {
            if patch.count_u == 0 || patch.count_v == 0 || patch.points.len() != (patch.count_u as usize).saturating_mul(patch.count_v as usize) {
                return invalid("a surface patch's points do not fill its grid");
            }
            if !(2..=6).contains(&patch.order_u) || !(2..=6).contains(&patch.order_v) {
                return invalid("a surface patch's order is out of range");
            }
            if !(1..=64).contains(&patch.resolution_u) || !(1..=64).contains(&patch.resolution_v) {
                return invalid("a surface patch's resolution is out of range");
            }
            if patch.points.iter().any(|point| !(point.co.is_finite() && point.weight.is_finite() && point.weight > 0.0)) {
                return invalid("a surface point is not finite");
            }
        }
        Ok(())
    }

    /// The control points of every patch, in order.
    pub fn positions(&self) -> Vec<Vec3> {
        self.patches.iter().flat_map(|patch| patch.points.iter().map(|point| point.co)).collect()
    }

    /// The surface as a mesh, in the object's own space.
    pub fn to_mesh(&self) -> Mesh {
        let mut mesh = Mesh { smooth: self.smooth, ..Mesh::default() };
        for patch in &self.patches {
            let Some((points, size_u, size_v)) = patch.evaluate() else { continue };
            let first = mesh.vertex_count();
            mesh.positions.extend(&points);
            let index = |u: usize, v: usize| first + u32::try_from(v * size_u + u).unwrap_or(0);
            let (cyclic_u, cyclic_v) = (patch.knots_u.cyclic, patch.knots_v.cyclic);
            if size_v == 1 {
                for u in 0..size_u.saturating_sub(1) {
                    mesh.loose_edges.push([index(u, 0), index(u + 1, 0)]);
                }
                if cyclic_u && size_u > 2 {
                    mesh.loose_edges.push([index(size_u - 1, 0), index(0, 0)]);
                }
                continue;
            }
            let spans_u = if cyclic_u { size_u } else { size_u - 1 };
            let spans_v = if cyclic_v { size_v } else { size_v - 1 };
            for v in 0..spans_v {
                for u in 0..spans_u {
                    let (u1, v1) = ((u + 1) % size_u, (v + 1) % size_v);
                    mesh.faces.push(vec![index(u, v), index(u1, v), index(u1, v1), index(u, v1)]);
                }
            }
        }
        mesh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_sample_resolution_times_points_each_way() {
        let counts: Vec<(usize, usize)> = SurfacePrimitive::ALL
            .iter()
            .map(|primitive| {
                let mesh = primitive.build(1.0).to_mesh();
                (mesh.positions.len(), mesh.faces.len())
            })
            .collect();
        // Curve, circle, patch, cylinder, sphere, torus: the reference's counts.
        assert_eq!(counts, vec![(12, 0), (32, 0), (256, 225), (256, 224), (640, 608), (1024, 1024)]);
    }

    #[test]
    fn the_cylinder_is_round() {
        let mesh = SurfacePrimitive::Cylinder.build(1.0).to_mesh();
        for point in &mesh.positions {
            assert!((point.truncate().length() - 1.0).abs() < 1e-4, "{point}");
        }
    }
}
