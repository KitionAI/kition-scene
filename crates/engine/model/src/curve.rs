//! Curve objects: Bézier, poly, and NURBS splines, made into a mesh the way a modeller does it —
//! sampled `resolution` times per segment, then (when it has depth) swept with a round bevel
//! profile and/or extruded along the curve's frames, or, for a closed 2D curve, filled. Counts,
//! profiles, and handle rules follow the reference application (checked in
//! `tests/curves.rs`).

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::error::{Result, SceneError};
use crate::mesh::Mesh;
use crate::scene::Id;

/// How a Bézier point's handle is placed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HandleType {
    /// Placed by hand, independent of the other handle.
    Free,
    /// Placed by hand, kept in line with the other handle.
    Aligned,
    /// A third of the way to the neighbouring point: straight segments.
    Vector,
    /// Smooth, worked out from the neighbouring points.
    #[default]
    Auto,
}

fn one() -> f32 {
    1.0
}

fn is_one(value: &f32) -> bool {
    *value == 1.0
}

fn is_zero(value: &f32) -> bool {
    *value == 0.0
}

/// A point of a Bézier spline with its two handles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BezierPoint {
    pub co: Vec3,
    pub left: Vec3,
    pub right: Vec3,
    #[serde(default)]
    pub left_type: HandleType,
    #[serde(default)]
    pub right_type: HandleType,
    /// Scales the bevel at this point.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub radius: f32,
    /// Turns the bevel around the curve at this point, in radians.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tilt: f32,
}

impl BezierPoint {
    /// A point with automatic handles (worked out when the spline is evaluated).
    pub fn auto(co: Vec3) -> Self {
        Self { co, left: co, right: co, left_type: HandleType::Auto, right_type: HandleType::Auto, radius: 1.0, tilt: 0.0 }
    }
}

/// A point of a poly or NURBS spline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlPoint {
    pub co: Vec3,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub weight: f32,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub radius: f32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tilt: f32,
}

impl ControlPoint {
    pub fn at(co: Vec3) -> Self {
        Self { co, weight: 1.0, radius: 1.0, tilt: 0.0 }
    }
}

fn twelve() -> u32 {
    12
}

fn four() -> u32 {
    4
}

/// One spline of a curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Spline {
    Bezier {
        points: Vec<BezierPoint>,
        #[serde(default)]
        cyclic: bool,
        #[serde(default = "twelve")]
        resolution: u32,
    },
    Poly {
        points: Vec<ControlPoint>,
        #[serde(default)]
        cyclic: bool,
    },
    Nurbs {
        points: Vec<ControlPoint>,
        #[serde(default)]
        cyclic: bool,
        #[serde(default = "four")]
        order: u32,
        /// Clamped knots: the spline starts and ends on its end points.
        #[serde(default)]
        endpoint: bool,
        #[serde(default = "twelve")]
        resolution: u32,
    },
}

impl Spline {
    /// Whether the spline closes on itself.
    pub fn cyclic(&self) -> bool {
        match self {
            Self::Bezier { cyclic, .. } | Self::Poly { cyclic, .. } | Self::Nurbs { cyclic, .. } => *cyclic,
        }
    }

    /// Sets whether the spline closes on itself.
    pub fn set_cyclic(&mut self, on: bool) {
        match self {
            Self::Bezier { cyclic, .. } | Self::Poly { cyclic, .. } | Self::Nurbs { cyclic, .. } => *cyclic = on,
        }
    }

    /// The number of control points.
    pub fn len(&self) -> usize {
        match self {
            Self::Bezier { points, .. } => points.len(),
            Self::Poly { points, .. } | Self::Nurbs { points, .. } => points.len(),
        }
    }

    /// Whether the spline has no points.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The control points' positions.
    pub fn positions(&self) -> Vec<Vec3> {
        match self {
            Self::Bezier { points, .. } => points.iter().map(|point| point.co).collect(),
            Self::Poly { points, .. } | Self::Nurbs { points, .. } => points.iter().map(|point| point.co).collect(),
        }
    }
}

/// Whether the curve lives in its own XY plane (and can be filled) or in 3D.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dimensions {
    #[serde(rename = "2d")]
    Flat,
    #[default]
    #[serde(rename = "3d")]
    Spatial,
}

/// Which part of the bevel or fill is made.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FillMode {
    /// 3D: the whole bevel profile.
    #[default]
    Full,
    /// 3D: the front half of the profile.
    Half,
    /// 2D: the front and back faces.
    Both,
    /// 2D: the front face.
    Front,
    /// 2D: the back face.
    Back,
    /// 2D: no faces.
    None,
}

/// How a taper object's value and each point's radius combine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaperRadius {
    /// The taper's value alone.
    #[default]
    Override,
    /// The taper's value times the radius.
    Multiply,
    /// The taper's value plus the radius.
    Add,
}

impl TaperRadius {
    fn is_override(&self) -> bool {
        *self == Self::Override
    }
}

/// The other curves a curve's shape reads: a bevel object's profile (with that object's scale)
/// and a taper object's thickness along the length.
#[derive(Debug, Clone, Copy, Default)]
pub struct Shapes<'a> {
    pub bevel: Option<(&'a Curve, Vec3)>,
    pub taper: Option<&'a Curve>,
}

/// A curve object's data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Curve {
    pub splines: Vec<Spline>,
    #[serde(default)]
    pub dimensions: Dimensions,
    #[serde(default)]
    pub fill_mode: FillMode,
    /// The radius of the round profile swept along the curve.
    #[serde(default)]
    pub bevel_depth: f32,
    #[serde(default = "four")]
    pub bevel_resolution: u32,
    /// Half the height of the ribbon the curve is extruded into.
    #[serde(default)]
    pub extrude: f32,
    /// Moves the curve along its normals.
    #[serde(default)]
    pub offset: f32,
    /// Closes the ends of an open bevelled curve.
    #[serde(default)]
    pub fill_caps: bool,
    /// The resolution new splines get.
    #[serde(default = "twelve")]
    pub resolution: u32,
    /// A curve object whose splines, in its own XY plane and scaled by its scale, are swept
    /// along this curve in place of the round bevel and the extrusion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bevel_object: Option<Id>,
    /// A curve object whose height along its X range sets the thickness from this curve's start
    /// to its end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taper_object: Option<Id>,
    /// How the taper and each point's radius combine.
    #[serde(default, skip_serializing_if = "TaperRadius::is_override")]
    pub taper_radius: TaperRadius,
}

impl Default for Curve {
    fn default() -> Self {
        Self {
            splines: Vec::new(),
            dimensions: Dimensions::Spatial,
            fill_mode: FillMode::Full,
            bevel_depth: 0.0,
            bevel_resolution: 4,
            extrude: 0.0,
            offset: 0.0,
            fill_caps: false,
            resolution: 12,
            bevel_object: None,
            taper_object: None,
            taper_radius: TaperRadius::Override,
        }
    }
}

/// The largest number of samples one curve may make.
const MAX_SAMPLES: usize = 200_000;

/// The constant the reference application divides a neighbour distance by for auto handles.
const AUTO_HANDLE: f32 = 2.5614;

/// The curves a person adds from Add ▸ Curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurvePrimitive {
    Bezier,
    Circle,
    Path,
}

impl CurvePrimitive {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Bezier => "BezierCurve",
            Self::Circle => "BezierCircle",
            Self::Path => "NurbsPath",
        }
    }

    /// The curve with the reference's default points, handles, and settings, scaled by `radius`.
    pub fn build(self, radius: f32) -> Curve {
        let r = radius;
        let point = |co: [f32; 3], left: [f32; 3], right: [f32; 3], kind: HandleType| BezierPoint {
            co: Vec3::from(co) * r,
            left: Vec3::from(left) * r,
            right: Vec3::from(right) * r,
            left_type: kind,
            right_type: kind,
            radius: 1.0,
            tilt: 0.0,
        };
        let spline = match self {
            Self::Bezier => Spline::Bezier {
                points: vec![
                    point([-1.0, 0.0, 0.0], [-1.5, -0.5, 0.0], [-0.5, 0.5, 0.0], HandleType::Aligned),
                    point([1.0, 0.0, 0.0], [0.0, 0.0, 0.0], [2.0, 0.0, 0.0], HandleType::Aligned),
                ],
                cyclic: false,
                resolution: 12,
            },
            Self::Circle => Spline::Bezier {
                points: [[-1.0, 0.0], [0.0, 1.0], [1.0, 0.0], [0.0, -1.0]]
                    .iter()
                    .map(|[x, y]| BezierPoint::auto(Vec3::new(*x, *y, 0.0) * r))
                    .collect(),
                cyclic: true,
                resolution: 12,
            },
            Self::Path => Spline::Nurbs {
                points: (0..5).map(|index| ControlPoint::at(Vec3::new(index as f32 - 2.0, 0.0, 0.0) * r)).collect(),
                cyclic: false,
                order: 5,
                endpoint: true,
                resolution: 12,
            },
        };
        let mut curve = Curve { splines: vec![spline], ..Curve::default() };
        curve.recalculate_handles();
        curve
    }
}

/// One evaluated sample along a spline.
#[derive(Debug, Clone, Copy)]
struct Sample {
    position: Vec3,
    radius: f32,
    tilt: f32,
    /// The curve's own direction here, when the spline knows it (Bézier derivatives).
    tangent: Option<Vec3>,
}

impl Curve {
    /// Checks the curve's numbers and sizes.
    pub fn validate(&self) -> Result<()> {
        let invalid = |message: &str| Err(SceneError::InvalidFile(message.into()));
        for value in [self.bevel_depth, self.extrude, self.offset] {
            if !value.is_finite() || value.abs() > 1e6 {
                return invalid("a curve setting is not a usable number");
            }
        }
        if self.bevel_depth < 0.0 || self.extrude < 0.0 {
            return invalid("a curve's bevel and extrusion cannot be negative");
        }
        if self.bevel_resolution > 64 || !(1..=1024).contains(&self.resolution) {
            return invalid("a curve's resolution is out of range");
        }
        for spline in &self.splines {
            match spline {
                Spline::Bezier { points, resolution, .. } => {
                    if !(1..=1024).contains(resolution) {
                        return invalid("a spline's resolution is out of range");
                    }
                    if points.iter().any(|point| {
                        !(point.co.is_finite()
                            && point.left.is_finite()
                            && point.right.is_finite()
                            && point.radius.is_finite()
                            && point.tilt.is_finite())
                    }) {
                        return invalid("a curve point is not finite");
                    }
                }
                Spline::Poly { points, .. } => {
                    if points.iter().any(|point| !(point.co.is_finite() && point.radius.is_finite() && point.tilt.is_finite())) {
                        return invalid("a curve point is not finite");
                    }
                }
                Spline::Nurbs { points, order, resolution, .. } => {
                    if !(2..=6).contains(order) || !(1..=1024).contains(resolution) {
                        return invalid("a NURBS spline's order or resolution is out of range");
                    }
                    if points
                        .iter()
                        .any(|point| !(point.co.is_finite() && point.weight.is_finite() && point.weight > 0.0 && point.radius.is_finite()))
                    {
                        return invalid("a curve point is not finite");
                    }
                }
            }
        }
        Ok(())
    }

    /// Places every Auto and Vector handle from its neighbours, and keeps Aligned handles in
    /// line (the moved side wins: the right one), as the reference does after an edit.
    pub fn recalculate_handles(&mut self) {
        for spline in &mut self.splines {
            if let Spline::Bezier { points, cyclic, .. } = spline {
                recalculate_bezier(points, *cyclic);
            }
        }
    }

    /// Samples every spline: positions with the radius and tilt at each.
    fn sampled(&self) -> Vec<(Vec<Sample>, bool)> {
        self.splines
            .iter()
            .map(|spline| (sample_spline(spline), spline.cyclic() && spline.len() > 2))
            .filter(|(samples, _)| !samples.is_empty())
            .collect()
    }

    /// The path along the first spline, in the object's own space: its evaluated points (a closed
    /// spline ends where it starts) and the radius at each. `None` without two points.
    pub fn path(&self) -> Option<(Vec<Vec3>, Vec<f32>)> {
        let spline = self.splines.first()?;
        let mut samples = sample_spline(spline);
        if spline.cyclic()
            && spline.len() > 2
            && let Some(first) = samples.first().copied()
        {
            samples.push(first);
        }
        if samples.len() < 2 {
            return None;
        }
        Some((samples.iter().map(|sample| sample.position).collect(), samples.iter().map(|sample| sample.radius).collect()))
    }

    /// The curve as a mesh, in the object's own space, without bevel or taper objects.
    pub fn to_mesh(&self) -> Mesh {
        self.to_mesh_with(Shapes::default())
    }

    /// The curve's evaluated points, spline by spline, in its own space (what text on the curve
    /// and bevel profiles follow).
    pub fn evaluated_splines(&self) -> Vec<(Vec<Vec3>, bool)> {
        self.sampled().into_iter().map(|(samples, cyclic)| (samples.iter().map(|sample| sample.position).collect(), cyclic)).collect()
    }

    /// The curve as a mesh, in the object's own space, with the bevel and taper objects' curves.
    pub fn to_mesh_with(&self, shapes: Shapes<'_>) -> Mesh {
        let mut mesh = Mesh::default();
        let flat = self.dimensions == Dimensions::Flat;
        let mut sampled = self.sampled();
        if flat {
            for (samples, _) in &mut sampled {
                for sample in samples.iter_mut() {
                    sample.position.z = 0.0;
                }
            }
        }
        let total: usize = sampled.iter().map(|(samples, _)| samples.len()).sum();
        if total > MAX_SAMPLES {
            return mesh;
        }
        if let Some(taper) = shapes.taper {
            let taper = taper_table(taper);
            if taper.len() >= 2 {
                for (samples, cyclic) in &mut sampled {
                    let steps = if *cyclic { samples.len() } else { samples.len().saturating_sub(1) }.max(1);
                    for (index, sample) in samples.iter_mut().enumerate() {
                        let value = taper_at(&taper, index as f32 / steps as f32);
                        sample.radius = match self.taper_radius {
                            TaperRadius::Override => value,
                            TaperRadius::Multiply => value * sample.radius,
                            TaperRadius::Add => value + sample.radius,
                        };
                    }
                }
            }
        }
        let profiles: Vec<Profile> = match shapes.bevel {
            Some((bevel, scale)) => object_profiles(bevel, scale),
            None => profile(self.bevel_depth, self.bevel_resolution, self.extrude, self.fill_mode).into_iter().collect(),
        };
        let profile = profiles.first().cloned();
        let fill = flat && matches!(self.fill_mode, FillMode::Both | FillMode::Front | FillMode::Back);
        let mut outlines: Vec<Vec<Vec2>> = Vec::new();
        for (samples, cyclic) in &sampled {
            let frames = frames(samples, *cyclic, flat);
            let positions: Vec<Vec3> =
                samples.iter().zip(&frames).map(|(sample, (normal, _))| sample.position + *normal * (self.offset * sample.radius)).collect();
            if fill && *cyclic {
                outlines.push(positions.iter().map(|point| point.truncate()).collect());
            }
            match &profile {
                None => {
                    // A wire: the samples and the edges between them.
                    let first = mesh.vertex_count();
                    mesh.positions.extend(&positions);
                    let count = u32::try_from(positions.len()).unwrap_or(0);
                    for index in 0..count.saturating_sub(1) {
                        mesh.loose_edges.push([first + index, first + index + 1]);
                    }
                    if *cyclic && count > 2 {
                        mesh.loose_edges.push([first + count - 1, first]);
                    }
                }
                Some(_) => {
                    for profile in &profiles {
                        sweep(&mut mesh, &positions, samples, &frames, profile, *cyclic, self.fill_caps);
                    }
                }
            }
        }
        if fill && !outlines.is_empty() {
            fill_flat(&mut mesh, &outlines, self.extrude, self.fill_mode, profile.is_some());
        }
        // A filled 2D curve without depth draws its fill only, not its outline as a wire too.
        if fill && profile.is_none() && !mesh.faces.is_empty() {
            mesh.loose_edges.clear();
            mesh.remove_unused_vertices();
        }
        mesh
    }
}

fn recalculate_bezier(points: &mut [BezierPoint], cyclic: bool) {
    let count = points.len();
    if count == 0 {
        return;
    }
    let positions: Vec<Vec3> = points.iter().map(|point| point.co).collect();
    for index in 0..count {
        let co = positions.get(index).copied().unwrap_or(Vec3::ZERO);
        // Neighbours; an open end mirrors its one neighbour.
        let previous = if index > 0 {
            positions.get(index - 1).copied()
        } else if cyclic && count > 2 {
            positions.last().copied()
        } else {
            None
        };
        let next = if index + 1 < count {
            positions.get(index + 1).copied()
        } else if cyclic && count > 2 {
            positions.first().copied()
        } else {
            None
        };
        let previous = previous.or_else(|| next.map(|next| co * 2.0 - next)).unwrap_or(co);
        let next = next.or(Some(co * 2.0 - previous)).unwrap_or(co);
        let Some(point) = points.get_mut(index) else { continue };
        let (back, ahead) = (co - previous, next - co);
        let (len_back, len_ahead) = (back.length(), ahead.length());
        if point.left_type == HandleType::Auto || point.right_type == HandleType::Auto {
            let mut direction = Vec3::ZERO;
            if len_back > 1e-9 {
                direction += back / len_back;
            }
            if len_ahead > 1e-9 {
                direction += ahead / len_ahead;
            }
            let direction = direction.normalize_or_zero();
            if point.left_type == HandleType::Auto {
                point.left = co - direction * (len_back / AUTO_HANDLE);
            }
            if point.right_type == HandleType::Auto {
                point.right = co + direction * (len_ahead / AUTO_HANDLE);
            }
        }
        if point.left_type == HandleType::Vector {
            point.left = co + (previous - co) / 3.0;
        }
        if point.right_type == HandleType::Vector {
            point.right = co + (next - co) / 3.0;
        }
        // Aligned pairs: the left handle turns into line with the right one, keeping its length.
        if point.left_type == HandleType::Aligned && point.right_type != HandleType::Free {
            let along = (co - point.right).normalize_or_zero();
            if along != Vec3::ZERO {
                point.left = co + along * (point.left - co).length();
            }
        } else if point.right_type == HandleType::Aligned && point.left_type != HandleType::Free {
            let along = (co - point.left).normalize_or_zero();
            if along != Vec3::ZERO {
                point.right = co + along * (point.right - co).length();
            }
        }
    }
}

/// The derivative of a cubic Bézier segment.
pub fn cubic_derivative(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, t: f32) -> Vec3 {
    let u = 1.0 - t;
    (p1 - p0) * (3.0 * u * u) + (p2 - p1) * (6.0 * u * t) + (p3 - p2) * (3.0 * t * t)
}

/// A point on a cubic Bézier segment.
pub fn cubic(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, t: f32) -> Vec3 {
    let u = 1.0 - t;
    p0 * (u * u * u) + p1 * (3.0 * u * u * t) + p2 * (3.0 * u * t * t) + p3 * (t * t * t)
}

fn sample_spline(spline: &Spline) -> Vec<Sample> {
    match spline {
        Spline::Bezier { points, cyclic, resolution } => {
            let count = points.len();
            if count == 0 {
                return Vec::new();
            }
            let cyclic = *cyclic && count > 2;
            let segments = if cyclic { count } else { count - 1 };
            let resolution = (*resolution).max(1) as usize;
            if segments.saturating_mul(resolution) > MAX_SAMPLES {
                return Vec::new();
            }
            let mut out = Vec::with_capacity(segments * resolution + 1);
            for segment in 0..segments {
                let (Some(a), Some(b)) = (points.get(segment), points.get((segment + 1) % count)) else { continue };
                for step in 0..resolution {
                    let t = step as f32 / resolution as f32;
                    out.push(Sample {
                        position: cubic(a.co, a.right, b.left, b.co, t),
                        radius: a.radius + (b.radius - a.radius) * t,
                        tilt: a.tilt + (b.tilt - a.tilt) * t,
                        tangent: Some(cubic_derivative(a.co, a.right, b.left, b.co, t)),
                    });
                }
            }
            if !cyclic && let (Some(before), Some(last)) = (points.get(count.saturating_sub(2)), points.last()) {
                let tangent = if count >= 2 { Some(cubic_derivative(before.co, before.right, last.left, last.co, 1.0)) } else { None };
                out.push(Sample { position: last.co, radius: last.radius, tilt: last.tilt, tangent });
            }
            out
        }
        Spline::Poly { points, .. } => {
            points.iter().map(|point| Sample { position: point.co, radius: point.radius, tilt: point.tilt, tangent: None }).collect()
        }
        Spline::Nurbs { points, cyclic, order, endpoint, resolution } => {
            nurbs(points, *cyclic && points.len() > 2, *order as usize, *endpoint, (*resolution).max(1) as usize)
        }
    }
}

/// Uniform B-spline evaluation with weights, sampled `resolution` times per segment over the
/// valid parameter range (both ends included for an open spline).
fn nurbs(points: &[ControlPoint], cyclic: bool, order: usize, endpoint: bool, resolution: usize) -> Vec<Sample> {
    let count = points.len();
    if count < 2 {
        return points.iter().map(|point| Sample { position: point.co, radius: point.radius, tilt: point.tilt, tangent: None }).collect();
    }
    let order = order.clamp(2, count.max(2));
    // Cyclic splines wrap their first `order - 1` points around.
    let control: Vec<&ControlPoint> = if cyclic { points.iter().chain(points.iter().take(order - 1)).collect() } else { points.iter().collect() };
    let n = control.len();
    let knots: Vec<f32> = if endpoint && !cyclic {
        (0..n + order)
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
        (0..n + order).map(|index| index as f32).collect()
    };
    let (start, end) = (knots.get(order - 1).copied().unwrap_or(0.0), knots.get(n).copied().unwrap_or(1.0));
    let segments = if cyclic { count } else { count - 1 };
    let total = segments.saturating_mul(resolution);
    if !(2..=MAX_SAMPLES).contains(&total) {
        return Vec::new();
    }
    let steps = if cyclic { total } else { total - 1 };
    let mut out = Vec::with_capacity(total);
    for step in 0..total {
        let mut u = start + (end - start) * step as f32 / steps as f32;
        if u >= end {
            u = end - 1e-6 * (end - start).max(1.0);
        }
        // Cox–de Boor basis functions of degree order - 1.
        let mut basis: Vec<f32> = (0..n + order - 1)
            .map(|index| {
                let (Some(a), Some(b)) = (knots.get(index), knots.get(index + 1)) else { return 0.0 };
                if u >= *a && u < *b { 1.0 } else { 0.0 }
            })
            .collect();
        for degree in 1..order {
            for index in 0..(n + order - 1 - degree) {
                let k = |slot: usize| knots.get(slot).copied().unwrap_or(0.0);
                let left = {
                    let span = k(index + degree) - k(index);
                    if span > 0.0 { (u - k(index)) / span * basis.get(index).copied().unwrap_or(0.0) } else { 0.0 }
                };
                let right = {
                    let span = k(index + degree + 1) - k(index + 1);
                    if span > 0.0 { (k(index + degree + 1) - u) / span * basis.get(index + 1).copied().unwrap_or(0.0) } else { 0.0 }
                };
                if let Some(value) = basis.get_mut(index) {
                    *value = left + right;
                }
            }
        }
        let (mut position, mut weight, mut radius, mut tilt) = (Vec3::ZERO, 0.0, 0.0, 0.0);
        for (index, point) in control.iter().enumerate() {
            let w = basis.get(index).copied().unwrap_or(0.0) * point.weight;
            position += point.co * w;
            weight += w;
            radius += point.radius * w;
            tilt += point.tilt * w;
        }
        if weight > 1e-12 {
            out.push(Sample { position: position / weight, radius: radius / weight, tilt: tilt / weight, tangent: None });
        }
    }
    out
}

/// The normal and binormal at each sample: rotation-minimising frames (the reference's
/// "minimum" twist), starting from Z × tangent; a flat curve keeps Z as its binormal.
fn frames(samples: &[Sample], cyclic: bool, flat: bool) -> Vec<(Vec3, Vec3)> {
    let count = samples.len();
    let position = |index: usize| samples.get(index).map_or(Vec3::ZERO, |sample| sample.position);
    let tangents: Vec<Vec3> = (0..count)
        .map(|index| {
            let (before, after) = if cyclic {
                (position((index + count - 1) % count), position((index + 1) % count))
            } else {
                (position(index.saturating_sub(1)), position((index + 1).min(count - 1)))
            };
            // Between two samples, the bisector of the directions to them; at an open end, the
            // curve's own direction there.
            let here = position(index);
            let interior = cyclic || (index > 0 && index + 1 < count);
            let bisector = ((here - before).normalize_or_zero() + (after - here).normalize_or_zero()).normalize_or_zero();
            let mut tangent = if interior && bisector != Vec3::ZERO {
                bisector
            } else {
                samples
                    .get(index)
                    .and_then(|sample| sample.tangent)
                    .map(Vec3::normalize_or_zero)
                    .filter(|tangent| *tangent != Vec3::ZERO)
                    .unwrap_or((after - before).normalize_or_zero())
            };
            if tangent == Vec3::ZERO {
                tangent = Vec3::X;
            }
            tangent
        })
        .collect();
    let mut out = Vec::with_capacity(count);
    let first_tangent = tangents.first().copied().unwrap_or(Vec3::X);
    let mut normal = Vec3::Z.cross(first_tangent).normalize_or_zero();
    if normal == Vec3::ZERO {
        normal = Vec3::Y.cross(first_tangent).normalize_or_zero();
    }
    for (index, tangent) in tangents.iter().enumerate() {
        if flat {
            let mut flat_normal = Vec3::Z.cross(*tangent).normalize_or_zero();
            if flat_normal == Vec3::ZERO {
                flat_normal = normal;
            }
            normal = flat_normal;
        } else if index > 0 {
            // Double reflection (rotation-minimising frames).
            let (Some(previous_tangent), p0, p1) = (tangents.get(index - 1), position(index - 1), position(index)) else { continue };
            let v1 = p1 - p0;
            let c1 = v1.dot(v1);
            if c1 > 1e-12 {
                let r = normal - v1 * (2.0 / c1 * v1.dot(normal));
                let t_l = *previous_tangent - v1 * (2.0 / c1 * v1.dot(*previous_tangent));
                let v2 = *tangent - t_l;
                let c2 = v2.dot(v2);
                normal = if c2 > 1e-12 { r - v2 * (2.0 / c2 * v2.dot(r)) } else { r };
            }
            normal = (normal - *tangent * normal.dot(*tangent)).normalize_or_zero();
            if normal == Vec3::ZERO {
                normal = Vec3::Z.cross(*tangent).normalize_or_zero();
            }
        }
        let tilt = samples.get(index).map_or(0.0, |sample| sample.tilt);
        let binormal = tangent.cross(normal).normalize_or_zero();
        let (n, b) =
            if tilt != 0.0 { (normal * tilt.cos() + binormal * tilt.sin(), binormal * tilt.cos() - normal * tilt.sin()) } else { (normal, binormal) };
        out.push((n, b));
    }
    out
}

/// The cross-section swept along the curve: each point is a unit round part (along the normal
/// and the binormal, scaled by the bevel depth) and a lift along the binormal (the extrusion);
/// the point's radius scales both, and the offset.
#[derive(Debug, Clone)]
struct Profile {
    points: Vec<(Vec2, f32)>,
    depth: f32,
    closed: bool,
    /// Capped even when open (a bevel object's open profile closes its caps across).
    caps_open: bool,
}

/// A bevel object's splines as profiles: each evaluated point's X and Y in the object's own
/// space, scaled by its scale; X runs against the normal, Y along the binormal.
fn object_profiles(bevel: &Curve, scale: Vec3) -> Vec<Profile> {
    bevel
        .sampled()
        .into_iter()
        .filter(|(samples, _)| samples.len() >= 2)
        .map(|(samples, cyclic)| Profile {
            points: samples.iter().map(|sample| (Vec2::new(-sample.position.x * scale.x, sample.position.y * scale.y), 0.0)).collect(),
            depth: 1.0,
            closed: cyclic,
            caps_open: true,
        })
        .collect()
}

/// A taper curve's first spline as (x, height) pairs in the order it runs.
fn taper_table(taper: &Curve) -> Vec<Vec2> {
    taper.sampled().into_iter().next().map(|(samples, _)| samples.iter().map(|sample| sample.position.truncate()).collect()).unwrap_or_default()
}

/// The taper's height at `fraction` of its X range, between the points either side.
fn taper_at(table: &[Vec2], fraction: f32) -> f32 {
    let (min, max) = table.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), point| (min.min(point.x), max.max(point.x)));
    let x = min + (max - min) * fraction.clamp(0.0, 1.0);
    for pair in table.windows(2) {
        let [a, b] = pair else { continue };
        let (low, high) = if a.x <= b.x { (a, b) } else { (b, a) };
        if x >= low.x && x <= high.x {
            let span = high.x - low.x;
            return if span > 1e-9 { low.y + (high.y - low.y) * (x - low.x) / span } else { high.y };
        }
    }
    table.last().map_or(1.0, |point| point.y)
}

fn profile(depth: f32, resolution: u32, extrude: f32, fill: FillMode) -> Option<Profile> {
    let r = resolution as usize;
    let half = fill == FillMode::Half;
    let keep_half = |points: Vec<(Vec2, f32)>| -> Vec<(Vec2, f32)> { points.into_iter().filter(|(round, lift)| round.y + lift >= -1e-6).collect() };
    if depth > 0.0 && extrude > 0.0 {
        // Quarter arcs of r + 1 segments around the extruded sides, from the bottom.
        let step = FRAC_PI_2 / (r + 1) as f32;
        let mut points = Vec::with_capacity(4 * r + 6);
        let mut arc = |from: f32, slots: std::ops::RangeInclusive<usize>, lift: f32| {
            for slot in slots {
                let angle = from + step * slot as f32;
                points.push((Vec2::new(angle.cos(), angle.sin()), lift));
            }
        };
        arc(-FRAC_PI_2, 0..=r + 1, -extrude);
        arc(0.0, 0..=r + 1, extrude);
        arc(FRAC_PI_2, 1..=r + 1, extrude);
        arc(PI, 0..=r, -extrude);
        return Some(if half {
            Profile { points: keep_half(points), depth, closed: false, caps_open: false }
        } else {
            Profile { points, depth, closed: true, caps_open: false }
        });
    }
    if depth > 0.0 {
        // A full circle of 2r + 4 points, the first one step past the normal.
        let count = 2 * r + 4;
        let points: Vec<(Vec2, f32)> = (1..=count)
            .map(|k| {
                let angle = TAU * k as f32 / count as f32;
                (Vec2::new(angle.cos(), angle.sin()), 0.0)
            })
            .collect();
        return Some(if half {
            Profile { points: keep_half(points), depth, closed: false, caps_open: false }
        } else {
            Profile { points, depth, closed: true, caps_open: false }
        });
    }
    if extrude > 0.0 {
        return Some(Profile { points: vec![(Vec2::ZERO, -extrude), (Vec2::ZERO, extrude)], depth: 0.0, closed: false, caps_open: false });
    }
    None
}

/// Sweeps the profile along the samples: one ring per sample, quads between rings.
fn sweep(mesh: &mut Mesh, positions: &[Vec3], samples: &[Sample], frames: &[(Vec3, Vec3)], profile: &Profile, cyclic: bool, caps: bool) {
    let ring_size = u32::try_from(profile.points.len()).unwrap_or(0);
    if ring_size == 0 {
        return;
    }
    let first = mesh.vertex_count();
    for ((position, sample), (normal, binormal)) in positions.iter().zip(samples).zip(frames) {
        let scale = profile.depth * sample.radius;
        for (round, lift) in &profile.points {
            mesh.positions.push(*position + *normal * (round.x * scale) + *binormal * (round.y * scale + lift * sample.radius));
        }
    }
    let rings = u32::try_from(positions.len()).unwrap_or(0);
    let spans = if cyclic { rings } else { rings.saturating_sub(1) };
    let around = if profile.closed { ring_size } else { ring_size.saturating_sub(1) };
    let first_face = mesh.faces.len();
    for ring in 0..spans {
        let next = (ring + 1) % rings;
        for slot in 0..around {
            let slot_next = (slot + 1) % ring_size;
            let a = first + ring * ring_size + slot;
            let b = first + ring * ring_size + slot_next;
            let c = first + next * ring_size + slot_next;
            let d = first + next * ring_size + slot;
            mesh.faces.push(vec![a, d, c, b]);
        }
    }
    orient_outwards(mesh, first_face, positions);
    if caps && !cyclic && (profile.closed || (profile.caps_open && ring_size >= 3)) && rings >= 2 {
        // Each end gets its own copy of the ring, fanned into triangles facing out.
        for ring in [0, rings - 1] {
            let start = mesh.vertex_count();
            for slot in 0..ring_size {
                if let Some(point) = mesh.positions.get((first + ring * ring_size + slot) as usize).copied() {
                    mesh.positions.push(point);
                }
            }
            let cap_first = mesh.faces.len();
            for slot in 1..ring_size.saturating_sub(1) {
                mesh.faces.push(vec![start, start + slot, start + slot + 1]);
            }
            let along = match (positions.get(ring as usize), positions.get(if ring == 0 { 1 } else { ring as usize - 1 })) {
                (Some(here), Some(other)) => *here - *other,
                _ => Vec3::ZERO,
            };
            if let Some(face) = mesh.faces.get(cap_first).cloned()
                && mesh.face_normal(&face).dot(along) < 0.0
            {
                for face in mesh.faces.iter_mut().skip(cap_first) {
                    face.reverse();
                }
            }
        }
    }
}

/// Turns the swept faces outwards: the first face's normal must point away from the curve.
fn orient_outwards(mesh: &mut Mesh, index: usize, positions: &[Vec3]) {
    let Some(centre) = positions.first().copied() else { return };
    let Some(face) = mesh.faces.get(index).cloned() else { return };
    let normal = mesh.face_normal(&face);
    let middle = mesh.face_center(&face);
    if normal.dot(middle - centre) < 0.0 {
        for face in mesh.faces.iter_mut().skip(index) {
            face.reverse();
        }
    }
}

/// The front and back faces of a filled 2D curve, at ±extrude.
fn fill_flat(mesh: &mut Mesh, outlines: &[Vec<Vec2>], extrude: f32, mode: FillMode, has_depth: bool) {
    let triangles = crate::outline::fill(outlines);
    let points: Vec<Vec2> = outlines.iter().flatten().copied().collect();
    let sides: &[(f32, bool)] = match mode {
        FillMode::Front => &[(1.0, false)],
        FillMode::Back => &[(-1.0, true)],
        _ if extrude > 0.0 || has_depth => &[(1.0, false), (-1.0, true)],
        _ => &[(1.0, false)],
    };
    for (side, reverse) in sides {
        let first = mesh.vertex_count();
        mesh.positions.extend(points.iter().map(|point| Vec3::new(point.x, point.y, side * extrude)));
        for [a, b, c] in &triangles {
            let face = if *reverse { vec![first + a, first + c, first + b] } else { vec![first + a, first + b, first + c] };
            mesh.faces.push(face);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_handles_on_a_circle_follow_the_reference() {
        let curve = CurvePrimitive::Circle.build(1.0);
        let Spline::Bezier { points, .. } = &curve.splines[0] else { panic!() };
        // The reference's circle: handles 0.55213 from each point, along the tangent.
        assert!((points[0].right - Vec3::new(-1.0, 0.55213, 0.0)).length() < 1e-4, "{:?}", points[0].right);
        assert!((points[1].left - Vec3::new(-0.55213, 1.0, 0.0)).length() < 1e-4);
    }

    #[test]
    fn a_nurbs_path_samples_twelve_per_segment() {
        let mesh = Curve { bevel_depth: 0.1, ..CurvePrimitive::Path.build(1.0) }.to_mesh();
        assert_eq!(mesh.positions.len(), 48 * 12);
    }
}
