//! Curves as node trees pass them around: poly and Bézier curves stored point by point (as the
//! reference's curve geometry is), with a radius and a tilt per control point, handles for
//! Bézier points, and attributes on points and on curves. Evaluating a curve samples it
//! (`resolution` times per Bézier segment), and gives each evaluated point a tangent and a
//! minimum-twist normal; lengths along the evaluated points place samples for resampling,
//! trimming, and points along the curve.

use std::ops::{Deref, Range};

use glam::{Mat3, Mat4, Vec3};
use kition_scene_model::HandleType;
use kition_scene_model::curve::cubic;

use crate::field::{Column, Scalar};
use crate::geometry::{Attribute, Attributes, Domain, MAX_ELEMENTS, join_attributes};

/// How a curve runs through its control points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveType {
    /// Straight through every point.
    Poly,
    /// Cubic segments between points, shaped by each point's handles.
    Bezier,
    /// Uniform Catmull–Rom interpolation with repeated open endpoints.
    CatmullRom,
    /// Rational B-splines with a per-curve order and knot mode.
    Nurbs,
}

/// Curves: control points, each curve a run of them.
#[derive(Debug, Clone, PartialEq)]
pub struct CurvesComponent {
    pub positions: Vec<Vec3>,
    pub radius: Vec<f32>,
    /// Turns the normal around the tangent, in radians.
    pub tilt: Vec<f32>,
    /// Bézier handles (a poly point's are its position).
    pub handle_left: Vec<Vec3>,
    pub handle_right: Vec<Vec3>,
    pub handle_type_left: Vec<HandleType>,
    pub handle_type_right: Vec<HandleType>,
    /// Where each curve's points start, and one past the last point at the end.
    pub offsets: Vec<usize>,
    pub kinds: Vec<CurveType>,
    pub cyclic: Vec<bool>,
    /// Evaluated points per Bézier segment.
    pub resolution: Vec<u32>,
    /// NURBS order and generated knot mode, retained through curve operations.
    pub nurbs_order: Vec<u8>,
    pub knots_mode: Vec<crate::spline_types::KnotsMode>,
    /// Rational influence per control point.
    pub nurbs_weight: Vec<f32>,
    /// Attributes on points and on curves.
    pub attributes: Attributes,
}

impl Default for CurvesComponent {
    fn default() -> Self {
        Self {
            positions: Vec::new(),
            radius: Vec::new(),
            tilt: Vec::new(),
            handle_left: Vec::new(),
            handle_right: Vec::new(),
            handle_type_left: Vec::new(),
            handle_type_right: Vec::new(),
            offsets: vec![0],
            kinds: Vec::new(),
            cyclic: Vec::new(),
            resolution: Vec::new(),
            nurbs_order: Vec::new(),
            knots_mode: Vec::new(),
            nurbs_weight: Vec::new(),
            attributes: Attributes::new(),
        }
    }
}

/// One control point to add.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ControlPoint {
    pub position: Vec3,
    pub left: Vec3,
    pub right: Vec3,
    pub type_left: HandleType,
    pub type_right: HandleType,
    pub radius: f32,
    pub tilt: f32,
}

impl ControlPoint {
    /// A poly point.
    pub fn at(position: Vec3) -> Self {
        Self { position, left: position, right: position, type_left: HandleType::Vector, type_right: HandleType::Vector, radius: 1.0, tilt: 0.0 }
    }

    /// A Bézier point with aligned handles.
    pub fn bezier(position: Vec3, left: Vec3, right: Vec3) -> Self {
        Self { position, left, right, type_left: HandleType::Aligned, type_right: HandleType::Aligned, radius: 1.0, tilt: 0.0 }
    }
}

/// A point of a curve being made from another curve's points: its own position and handles,
/// and the source points (with weights) its radius, tilt, and attributes are mixed from.
#[derive(Debug, Clone, PartialEq)]
pub struct MadePoint {
    pub position: Vec3,
    pub left: Vec3,
    pub right: Vec3,
    pub type_left: HandleType,
    pub type_right: HandleType,
    pub from: Vec<(usize, f32)>,
}

impl MadePoint {
    /// A poly point mixed from `from`.
    pub fn poly(position: Vec3, from: Vec<(usize, f32)>) -> Self {
        Self { position, left: position, right: position, type_left: HandleType::Vector, type_right: HandleType::Vector, from }
    }
}

/// A curve being made from one of another component's curves.
#[derive(Debug, Clone, PartialEq)]
pub struct MadeCurve {
    /// The curve whose curve attributes it keeps.
    pub source: usize,
    pub kind: CurveType,
    pub cyclic: bool,
    pub resolution: u32,
    pub points: Vec<MadePoint>,
}

/// Source weights for one evaluated point. Existing Poly/Bézier paths stay inline;
/// only wider spline bases allocate a variable-size list.
#[derive(Debug, Clone, PartialEq)]
pub enum Sources {
    Pair([(usize, f32); 2]),
    Many(Vec<(usize, f32)>),
}
impl Deref for Sources {
    type Target = [(usize, f32)];
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Pair(values) => values,
            Self::Many(values) => values,
        }
    }
}
impl From<Vec<(usize, f32)>> for Sources {
    fn from(values: Vec<(usize, f32)>) -> Self {
        if values.len() <= 2 {
            Self::Pair([values.first().copied().unwrap_or((0, 0.0)), values.get(1).copied().unwrap_or((0, 0.0))])
        } else {
            Self::Many(values)
        }
    }
}

/// A curve's evaluated points.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Evaluated {
    pub positions: Vec<Vec3>,
    pub tangents: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub radius: Vec<f32>,
    pub tilt: Vec<f32>,
    /// The control points (component indices) each evaluated point is mixed from.
    pub from: Vec<Sources>,
    /// The evaluated index of each control point.
    pub control: Vec<usize>,
    pub cyclic: bool,
    /// The length along the curve at the end of each segment (the closing one too, when
    /// cyclic).
    pub lengths: Vec<f32>,
}

/// A place along an evaluated curve: a segment and how far along it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Place {
    pub segment: usize,
    pub factor: f32,
}

/// A sample of an evaluated curve.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub position: Vec3,
    pub tangent: Vec3,
    pub normal: Vec3,
    pub radius: f32,
    pub from: Vec<(usize, f32)>,
}

fn rotate(vector: Vec3, axis: Vec3, angle: f32) -> Vec3 {
    let (sin, cos) = angle.sin_cos();
    vector * cos + axis.cross(vector) * sin + axis * axis.dot(vector) * (1.0 - cos)
}

/// The rotation-minimising turn of `normal` from `from` to `to` (unit tangents).
fn carry(normal: Vec3, from: Vec3, to: Vec3) -> Vec3 {
    let axis = from.cross(to);
    let length = axis.length();
    if length < 1e-7 {
        return normal;
    }
    rotate(normal, axis / length, from.dot(to).clamp(-1.0, 1.0).acos())
}

/// Tangents of a polyline: the bisector of the directions to the neighbours, the end segments'
/// directions at open ends.
fn poly_tangents(positions: &[Vec3], cyclic: bool) -> Vec<Vec3> {
    let count = positions.len();
    let at = |index: usize| positions.get(index).copied().unwrap_or(Vec3::ZERO);
    if count < 2 {
        return vec![Vec3::Z; count];
    }
    (0..count)
        .map(|index| {
            let here = at(index);
            if cyclic || (index > 0 && index + 1 < count) {
                let before = at((index + count - 1) % count);
                let after = at((index + 1) % count);
                ((here - before).normalize_or_zero() + (after - here).normalize_or_zero()).normalize_or_zero()
            } else if index == 0 {
                (at(1) - here).normalize_or_zero()
            } else {
                (here - at(index - 1)).normalize_or_zero()
            }
        })
        .collect()
}

/// Minimum-twist normals along `tangents`: the first is the tangent crossed with Z (X when the
/// tangent is vertical), each next one the last turned with the tangent; a cyclic curve spreads
/// the mismatch where it closes evenly along its length.
fn minimum_twist(tangents: &[Vec3], cyclic: bool) -> Vec<Vec3> {
    let Some(first) = tangents.first().copied() else { return Vec::new() };
    let mut start = first.cross(Vec3::Z);
    start = if start.length_squared() < 1e-12 { Vec3::X } else { start.normalize() };
    let mut normals = Vec::with_capacity(tangents.len());
    normals.push(start);
    for pair in tangents.windows(2) {
        let [from, to] = pair else { continue };
        let last = normals.last().copied().unwrap_or(start);
        normals.push(carry(last, *from, *to));
    }
    if cyclic && tangents.len() > 1 {
        let (last_tangent, last_normal) = (tangents.last().copied().unwrap_or(first), normals.last().copied().unwrap_or(start));
        let end = carry(last_normal, last_tangent, first);
        let angle = end.cross(start).dot(first).atan2(end.dot(start));
        let count = tangents.len() as f32;
        for (index, (normal, tangent)) in normals.iter_mut().zip(tangents).enumerate() {
            *normal = rotate(*normal, *tangent, angle * index as f32 / count);
        }
    }
    normals
}

/// The point between `a` and `b` at `t` of a cubic segment, split there: the left piece's
/// handle into the point, the point's handle out to the right piece, and the outer handles
/// shortened (`a`'s right one, `b`'s left one).
#[derive(Debug, Clone, Copy)]
struct Split {
    first_right: Vec3,
    left: Vec3,
    point: Vec3,
    right: Vec3,
    last_left: Vec3,
}

fn split(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, t: f32) -> Split {
    let (q0, q1, q2) = (p0.lerp(p1, t), p1.lerp(p2, t), p2.lerp(p3, t));
    let (r0, r1) = (q0.lerp(q1, t), q1.lerp(q2, t));
    Split { first_right: q0, left: r0, point: r0.lerp(r1, t), right: r1, last_left: q2 }
}

fn mix_from(a: &[(usize, f32)], b: &[(usize, f32)], t: f32) -> Vec<(usize, f32)> {
    let mut out: Vec<(usize, f32)> = Vec::with_capacity(a.len() + b.len());
    for (index, weight) in a.iter().map(|(index, weight)| (*index, weight * (1.0 - t))).chain(b.iter().map(|(index, weight)| (*index, weight * t))) {
        if weight == 0.0 {
            continue;
        }
        match out.iter_mut().find(|(other, _)| *other == index) {
            Some((_, total)) => *total += weight,
            None => out.push((index, weight)),
        }
    }
    out
}

/// Mixes values by weights: numbers and vectors by their weighted sum, whole numbers and
/// booleans from the heaviest source.
fn mix_values(values: &Column, from: &[(usize, f32)], zero: Scalar) -> Scalar {
    let at = |index: usize| values.get(index).copied().unwrap_or(zero);
    match zero {
        Scalar::Float(_) => Scalar::Float(from.iter().map(|(index, weight)| at(*index).float() * weight).sum()),
        Scalar::Vector(_) => Scalar::Vector(from.iter().map(|(index, weight)| at(*index).vector() * *weight).sum()),
        _ => from.iter().max_by(|a, b| a.1.total_cmp(&b.1)).map_or(zero, |(index, _)| at(*index)),
    }
}

impl Evaluated {
    /// The number of segments.
    pub fn segments(&self) -> usize {
        let count = self.positions.len();
        if self.cyclic { count } else { count.saturating_sub(1) }
    }

    /// The curve's length.
    pub fn length(&self) -> f32 {
        self.lengths.last().copied().unwrap_or(0.0)
    }

    /// The length along the curve at evaluated point `index`.
    pub fn length_at(&self, index: usize) -> f32 {
        if index == 0 { 0.0 } else { self.lengths.get(index - 1).copied().unwrap_or(0.0) }
    }

    /// Where `length` along the curve falls: the last segment starting at or before it.
    pub fn place(&self, length: f32) -> Place {
        let segments = self.segments();
        if segments == 0 {
            return Place { segment: 0, factor: 0.0 };
        }
        let length = length.clamp(0.0, self.length());
        // Segment `s` starts at `length_at(s)`: the first segment ending after `length`.
        let segment = self.lengths.partition_point(|end| *end <= length).min(segments - 1);
        let start = self.length_at(segment);
        let span = self.lengths.get(segment).copied().unwrap_or(start) - start;
        let factor = if span > 0.0 { ((length - start) / span).clamp(0.0, 1.0) } else { 0.0 };
        Place { segment, factor }
    }

    /// The curve sampled at `place`.
    pub fn sample(&self, place: Place) -> Sample {
        let count = self.positions.len();
        let a = place.segment.min(count.saturating_sub(1));
        let b = if count == 0 { 0 } else { (a + 1) % count };
        let t = if count < 2 { 0.0 } else { place.factor };
        let pick = |values: &[Vec3], index: usize| values.get(index).copied().unwrap_or(Vec3::ZERO);
        let lerp = |values: &[f32], index: usize, other: usize| {
            let (x, y) = (values.get(index).copied().unwrap_or(0.0), values.get(other).copied().unwrap_or(0.0));
            x + (y - x) * t
        };
        let from = mix_from(self.from.get(a).map_or(&[][..], |from| &from[..]), self.from.get(b).map_or(&[][..], |from| &from[..]), t);
        Sample {
            position: pick(&self.positions, a).lerp(pick(&self.positions, b), t),
            tangent: pick(&self.tangents, a).lerp(pick(&self.tangents, b), t).normalize_or_zero(),
            normal: pick(&self.normals, a).lerp(pick(&self.normals, b), t).normalize_or_zero(),
            radius: lerp(&self.radius, a, b),
            from,
        }
    }

    /// `count` samples evenly spaced by length (a cyclic curve's last one before its start).
    pub fn even(&self, count: usize) -> Vec<Sample> {
        let total = self.length();
        let steps = if self.cyclic { count } else { count.saturating_sub(1) };
        (0..count)
            .map(|index| {
                let length = if steps == 0 { 0.0 } else { total * index as f32 / steps as f32 };
                self.sample(self.place(length))
            })
            .collect()
    }

    /// Every evaluated point as a sample.
    pub fn all(&self) -> Vec<Sample> {
        (0..self.positions.len()).map(|index| self.sample(Place { segment: index, factor: 0.0 })).collect()
    }
}

/// How many samples a length gives a curve: one per whole `length` and one more; a curve shorter
/// than `length` keeps both ends when `keep_last` is set.
pub fn count_for_length(total: f32, length: f32, points: usize, keep_last: bool) -> usize {
    let length = length.max(1e-4);
    let whole = (total / length).floor();
    let count = if whole.is_finite() { (whole as usize).saturating_add(1).min(MAX_ELEMENTS) } else { 1 };
    if count == 1 && keep_last && points > 1 && total > 0.0 { 2 } else { count }
}

impl CurvesComponent {
    /// The number of curves.
    pub fn curves_len(&self) -> usize {
        self.kinds.len()
    }

    /// The number of control points.
    pub fn points_len(&self) -> usize {
        self.positions.len()
    }

    /// The control points of curve `curve`.
    pub fn range(&self, curve: usize) -> Range<usize> {
        let start = self.offsets.get(curve).copied().unwrap_or(0);
        let end = self.offsets.get(curve + 1).copied().unwrap_or(start);
        start..end.max(start)
    }

    /// The curve each control point belongs to.
    pub fn point_curves(&self) -> Vec<usize> {
        let mut out = Vec::with_capacity(self.points_len());
        for curve in 0..self.curves_len() {
            out.extend(std::iter::repeat_n(curve, self.range(curve).len()));
        }
        out
    }

    /// Adds a curve.
    pub fn push(&mut self, kind: CurveType, cyclic: bool, resolution: u32, points: &[ControlPoint]) {
        for point in points {
            self.positions.push(point.position);
            self.handle_left.push(point.left);
            self.handle_right.push(point.right);
            self.handle_type_left.push(point.type_left);
            self.handle_type_right.push(point.type_right);
            self.radius.push(point.radius);
            self.tilt.push(point.tilt);
            self.nurbs_weight.push(1.0);
        }
        self.kinds.push(kind);
        self.cyclic.push(cyclic);
        self.resolution.push(resolution.max(1));
        self.nurbs_order.push(4);
        self.knots_mode.push(crate::spline_types::KnotsMode::Normal);
        self.offsets.push(self.positions.len());
    }

    /// A component with one poly curve through `points`.
    pub fn poly(points: &[Vec3], cyclic: bool) -> Self {
        let mut out = Self::default();
        out.push(CurveType::Poly, cyclic, 12, &points.iter().copied().map(ControlPoint::at).collect::<Vec<_>>());
        out
    }

    /// The number of elements in a domain.
    pub fn domain_len(&self, domain: Domain) -> usize {
        match domain {
            Domain::Point => self.points_len(),
            Domain::Curve => self.curves_len(),
            _ => 0,
        }
    }

    /// Values from domain `from` moved to domain `to` (points to curves by their mean, curves to
    /// their points).
    pub fn adapt(&self, values: &Column, from: Domain, to: Domain) -> Column {
        let zero = values.first().map_or(Scalar::Float(0.0), |value| Scalar::zero(value.kind()));
        if from == to {
            return values.clone();
        }
        match (from, to) {
            (Domain::Curve, Domain::Point) => self.point_curves().into_iter().map(|curve| values.get(curve).copied().unwrap_or(zero)).collect(),
            (Domain::Point, Domain::Curve) => (0..self.curves_len())
                .map(|curve| {
                    let range = self.range(curve);
                    let len = range.len();
                    let weights: Vec<(usize, f32)> = range.map(|index| (index, 1.0 / len as f32)).collect();
                    match zero {
                        Scalar::Bool(_) => Scalar::Bool(weights.iter().all(|(index, _)| values.get(*index).is_some_and(|value| value.bool()))),
                        Scalar::Int(_) => Scalar::Int(mix_values(values, &weights, Scalar::Float(0.0)).float().round() as i32),
                        _ => mix_values(values, &weights, zero),
                    }
                })
                .collect(),
            _ => vec![zero; self.domain_len(to)],
        }
    }

    /// Curve `curve` evaluated.
    pub fn evaluate(&self, curve: usize) -> Evaluated {
        self.evaluate_with_constants(curve, self.constant_attributes())
    }

    pub(crate) fn constant_attributes(&self) -> [Option<f32>; 2] {
        [&self.radius, &self.tilt].map(|values| values.first().filter(|first| values.iter().all(|value| value == *first)).copied())
    }

    pub(crate) fn evaluate_with_constants(&self, curve: usize, constants: [Option<f32>; 2]) -> Evaluated {
        let range = self.range(curve);
        let count = range.len();
        let cyclic = self.cyclic.get(curve).copied().unwrap_or(false) && count > 1;
        let kind = self.kinds.get(curve).copied().unwrap_or(CurveType::Poly);
        let first = range.start;
        let position = |index: usize| self.positions.get(first + index).copied().unwrap_or(Vec3::ZERO);
        let mut out = Evaluated { cyclic, ..Evaluated::default() };
        if count == 0 {
            return out;
        }
        match kind {
            CurveType::Poly => {
                out.positions = (0..count).map(position).collect();
                out.from = (0..count).map(|index| Sources::Pair([(first + index, 1.0), (first + index, 0.0)])).collect();
                out.control = (0..count).collect();
            }
            CurveType::Bezier => {
                let resolution = self.resolution.get(curve).copied().unwrap_or(12).max(1) as usize;
                let segments = if cyclic { count } else { count - 1 };
                if segments.saturating_mul(resolution) > MAX_ELEMENTS {
                    return out;
                }
                let right = |index: usize| self.handle_right.get(first + index).copied().unwrap_or(position(index));
                let left = |index: usize| self.handle_left.get(first + index).copied().unwrap_or(position(index));
                for segment in 0..segments {
                    let next = (segment + 1) % count;
                    let resolution = if self.handle_type_right.get(first + segment) == Some(&HandleType::Vector)
                        && self.handle_type_left.get(first + next) == Some(&HandleType::Vector)
                    {
                        1
                    } else {
                        resolution
                    };
                    out.control.push(out.positions.len());
                    for step in 0..resolution {
                        let t = step as f32 / resolution as f32;
                        out.positions.push(cubic(position(segment), right(segment), left(next), position(next), t));
                        out.from.push(Sources::Pair([(first + segment, 1.0 - t), (first + next, t)]));
                    }
                }
                if !cyclic {
                    out.control.push(out.positions.len());
                    out.positions.push(position(count - 1));
                    out.from.push(Sources::Pair([(first + count - 1, 1.0), (first + count - 1, 0.0)]));
                }
            }
            CurveType::CatmullRom => crate::spline_types::evaluate_catmull(self, curve, &mut out),
            CurveType::Nurbs => crate::spline_types::evaluate_nurbs(self, curve, &mut out),
        }
        out.tangents = poly_tangents(&out.positions, cyclic);
        if kind == CurveType::Bezier && !cyclic && count > 1 {
            // The ends follow their handles.
            let start = (self.handle_right.get(first).copied().unwrap_or(position(0)) - position(0)).normalize_or_zero();
            let end = (position(count - 1) - self.handle_left.get(first + count - 1).copied().unwrap_or(position(count - 1))).normalize_or_zero();
            if start != Vec3::ZERO
                && let Some(tangent) = out.tangents.first_mut()
            {
                *tangent = start;
            }
            if end != Vec3::ZERO
                && let Some(tangent) = out.tangents.last_mut()
            {
                *tangent = end;
            }
        }
        let mix = |values: &[f32], from: &Sources| from.iter().map(|(index, weight)| values.get(*index).copied().unwrap_or(0.0) * weight).sum();
        let interpolate = |values: &[f32], constant: Option<f32>| {
            if let Some(value) = constant { vec![value; out.from.len()] } else { out.from.iter().map(|from| mix(values, from)).collect() }
        };
        let [radius, tilt] = constants;
        out.radius = interpolate(&self.radius, radius);
        out.tilt = interpolate(&self.tilt, tilt);
        out.normals = minimum_twist(&out.tangents, cyclic)
            .into_iter()
            .zip(&out.tangents)
            .zip(&out.tilt)
            .map(|((normal, tangent), tilt)| if *tilt == 0.0 { normal } else { rotate(normal, *tangent, *tilt) })
            .collect();
        let mut total = 0.0;
        let points = out.positions.len();
        for segment in 0..out.segments() {
            let (a, b) =
                (out.positions.get(segment).copied().unwrap_or(Vec3::ZERO), out.positions.get((segment + 1) % points).copied().unwrap_or(Vec3::ZERO));
            total += a.distance(b);
            out.lengths.push(total);
        }
        out
    }

    /// Every curve evaluated.
    pub fn evaluated(&self) -> Vec<Evaluated> {
        let constants = self.constant_attributes();
        (0..self.curves_len()).map(|curve| self.evaluate_with_constants(curve, constants)).collect()
    }

    /// The number of evaluated points of every curve together.
    pub fn evaluated_len(&self) -> usize {
        (0..self.curves_len())
            .map(|curve| {
                let count = self.range(curve).len();
                match self.kinds.get(curve) {
                    Some(CurveType::Nurbs) => crate::spline_types::nurbs_evaluated_len(self, curve),
                    Some(CurveType::Bezier | CurveType::CatmullRom) if count > 1 => {
                        let cyclic = self.cyclic.get(curve).copied().unwrap_or(false);
                        let segments = if cyclic { count } else { count - 1 };
                        segments.saturating_mul(self.resolution.get(curve).copied().unwrap_or(1) as usize).saturating_add(usize::from(!cyclic))
                    }
                    _ => count,
                }
            })
            .fold(0usize, usize::saturating_add)
    }

    /// Positions on a domain: control points, or each curve's mean.
    pub fn positions(&self, domain: Domain) -> Vec<Vec3> {
        let column: Column = self.positions.iter().copied().map(Scalar::Vector).collect();
        self.adapt(&column, Domain::Point, domain).into_iter().map(Scalar::vector).collect()
    }

    /// Normals on a domain: each control point's evaluated normal (a curve's: their mean).
    pub fn normals(&self, domain: Domain) -> Vec<Vec3> {
        let mut out = Vec::with_capacity(self.points_len());
        let constants = self.constant_attributes();
        for curve in 0..self.curves_len() {
            let evaluated = if self.kinds.get(curve) == Some(&CurveType::Nurbs) {
                let range = self.range(curve);
                let positions: Vec<Vec3> = range.clone().filter_map(|at| self.positions.get(at).copied()).collect();
                let mut polygon = Self::poly(&positions, self.cyclic.get(curve).copied().unwrap_or(false));
                polygon.tilt = range.map(|at| self.tilt.get(at).copied().unwrap_or(0.0)).collect();
                polygon.evaluate(0)
            } else {
                self.evaluate_with_constants(curve, constants)
            };
            out.extend(evaluated.control.iter().map(|index| evaluated.normals.get(*index).copied().unwrap_or(Vec3::Z)));
        }
        out.resize(self.points_len(), Vec3::Z);
        match domain {
            Domain::Point => out,
            _ => {
                let column: Column = out.into_iter().map(Scalar::Vector).collect();
                self.adapt(&column, Domain::Point, domain).into_iter().map(|value| value.vector().normalize_or_zero()).collect()
            }
        }
    }

    /// A stored attribute on a domain; `radius` and `tilt` are the points' own.
    pub fn attribute(&self, name: &str, domain: Domain) -> Option<Column> {
        let (values, from) = match name {
            "radius" => (self.radius.iter().copied().map(Scalar::Float).collect(), Domain::Point),
            "tilt" => (self.tilt.iter().copied().map(Scalar::Float).collect(), Domain::Point),
            "nurbs_weight" => (self.nurbs_weight.iter().copied().map(Scalar::Float).collect(), Domain::Point),
            "nurbs_order" => (self.nurbs_order.iter().map(|order| Scalar::Int(i32::from(*order))).collect(), Domain::Curve),
            _ => {
                let attribute = self.attributes.get(name)?;
                (attribute.values.clone(), attribute.domain)
            }
        };
        Some(self.adapt(&values, from, domain))
    }

    /// Moves the points and handles by `transform`.
    pub fn transform(&mut self, transform: Mat4) {
        for point in self.positions.iter_mut().chain(&mut self.handle_left).chain(&mut self.handle_right) {
            *point = transform.transform_point3(*point);
        }
    }

    /// Appends `other` (attributes only one side has are zero on the other).
    pub fn append(&mut self, other: &CurvesComponent) {
        let mut attributes = std::mem::take(&mut self.attributes);
        let mut joined = Attributes::new();
        for domain in [Domain::Point, Domain::Curve] {
            let pick = |attributes: &Attributes| -> Attributes {
                attributes
                    .iter()
                    .filter(|(_, attribute)| attribute.domain == domain)
                    .map(|(name, attribute)| (name.clone(), attribute.clone()))
                    .collect()
            };
            let mut mine = pick(&attributes);
            join_attributes(&mut mine, self.domain_len(domain), &pick(&other.attributes), other.domain_len(domain));
            joined.extend(mine);
        }
        attributes.clear();
        self.attributes = joined;
        let base = self.positions.len();
        self.positions.extend_from_slice(&other.positions);
        self.radius.extend_from_slice(&other.radius);
        self.tilt.extend_from_slice(&other.tilt);
        self.handle_left.extend_from_slice(&other.handle_left);
        self.handle_right.extend_from_slice(&other.handle_right);
        self.handle_type_left.extend_from_slice(&other.handle_type_left);
        self.handle_type_right.extend_from_slice(&other.handle_type_right);
        self.offsets.extend(other.offsets.iter().skip(1).map(|offset| offset + base));
        self.kinds.extend_from_slice(&other.kinds);
        self.cyclic.extend_from_slice(&other.cyclic);
        self.resolution.extend_from_slice(&other.resolution);
        self.nurbs_order.extend_from_slice(&other.nurbs_order);
        self.knots_mode.extend_from_slice(&other.knots_mode);
        self.nurbs_weight.extend_from_slice(&other.nurbs_weight);
    }

    /// New curves made from this component's points: radius, tilt, and point attributes mixed
    /// from their sources, curve attributes kept from their source curves.
    pub fn make(&self, curves: Vec<MadeCurve>) -> CurvesComponent {
        let mut out = CurvesComponent::default();
        let mut sources = Vec::new();
        let mut froms: Vec<Vec<(usize, f32)>> = Vec::new();
        for curve in curves {
            for point in &curve.points {
                out.positions.push(point.position);
                out.handle_left.push(point.left);
                out.handle_right.push(point.right);
                out.handle_type_left.push(point.type_left);
                out.handle_type_right.push(point.type_right);
            }
            froms.extend(curve.points.into_iter().map(|point| point.from));
            out.kinds.push(curve.kind);
            out.cyclic.push(curve.cyclic);
            out.resolution.push(curve.resolution.max(1));
            out.nurbs_order.push(self.nurbs_order.get(curve.source).copied().unwrap_or(4));
            out.knots_mode.push(self.knots_mode.get(curve.source).copied().unwrap_or_default());
            out.offsets.push(out.positions.len());
            sources.push(curve.source);
        }
        let floats = |values: &[f32]| -> Vec<f32> {
            if let Some(first) = values.first().filter(|first| values.iter().all(|value| value == *first)) {
                return vec![*first; froms.len()];
            }
            let column: Column = values.iter().copied().map(Scalar::Float).collect();
            froms.iter().map(|from| mix_values(&column, from, Scalar::Float(0.0)).float()).collect()
        };
        out.radius = floats(&self.radius);
        out.tilt = floats(&self.tilt);
        out.nurbs_weight = floats(&self.nurbs_weight);
        for (name, attribute) in &self.attributes {
            let zero = attribute.values.first().map_or(Scalar::Float(0.0), |value| Scalar::zero(value.kind()));
            let values = match attribute.domain {
                Domain::Point => {
                    if let Some(first) = attribute.values.first().filter(|first| attribute.values.iter().all(|value| value == *first)) {
                        vec![*first; froms.len()]
                    } else {
                        froms.iter().map(|from| mix_values(&attribute.values, from, zero)).collect()
                    }
                }
                _ => sources.iter().map(|source| attribute.values.get(*source).copied().unwrap_or(zero)).collect(),
            };
            out.attributes.insert(name.clone(), Attribute { domain: attribute.domain, values });
        }
        out
    }

    /// Curve `curve` as it is, to pass through [`CurvesComponent::make`].
    pub fn kept(&self, curve: usize) -> MadeCurve {
        let points = self
            .range(curve)
            .map(|index| MadePoint {
                position: self.positions.get(index).copied().unwrap_or(Vec3::ZERO),
                left: self.handle_left.get(index).copied().unwrap_or(Vec3::ZERO),
                right: self.handle_right.get(index).copied().unwrap_or(Vec3::ZERO),
                type_left: self.handle_type_left.get(index).copied().unwrap_or_default(),
                type_right: self.handle_type_right.get(index).copied().unwrap_or_default(),
                from: vec![(index, 1.0)],
            })
            .collect();
        MadeCurve {
            source: curve,
            kind: self.kinds.get(curve).copied().unwrap_or(CurveType::Poly),
            cyclic: self.cyclic.get(curve).copied().unwrap_or(false),
            resolution: self.resolution.get(curve).copied().unwrap_or(12),
            points,
        }
    }

    /// The points' handles moved by the same offsets as their points (when the reference moves a
    /// Bézier point, its handles go with it).
    pub fn move_points(&mut self, new: &[Option<Vec3>]) {
        for (index, new) in new.iter().enumerate() {
            let Some(new) = new.filter(|new| new.is_finite()) else { continue };
            let Some(point) = self.positions.get_mut(index) else { continue };
            let offset = new - *point;
            *point = new;
            for handle in [self.handle_left.get_mut(index), self.handle_right.get_mut(index)].into_iter().flatten() {
                *handle += offset;
            }
        }
    }

    /// Reverses the selected curves: their points run the other way, handles swapped.
    pub fn reverse(&self, selected: &[bool]) -> CurvesComponent {
        let curves = (0..self.curves_len())
            .map(|curve| {
                let mut made = self.kept(curve);
                if selected.get(curve).copied().unwrap_or(false) {
                    made.points.reverse();
                    for point in &mut made.points {
                        std::mem::swap(&mut point.left, &mut point.right);
                        std::mem::swap(&mut point.type_left, &mut point.type_right);
                    }
                }
                made
            })
            .collect();
        self.make(curves)
    }

    /// Resamples the selected curves to poly curves of `counts` points each (or, without counts,
    /// to their evaluated points).
    pub fn resample(&self, selected: &[bool], counts: Option<&[usize]>) -> CurvesComponent {
        let curves = self
            .evaluated()
            .into_iter()
            .enumerate()
            .map(|(curve, evaluated)| {
                if !selected.get(curve).copied().unwrap_or(false) || evaluated.positions.is_empty() {
                    return self.kept(curve);
                }
                let samples = match counts {
                    Some(counts) => evaluated.even(counts.get(curve).copied().unwrap_or(1).max(1)),
                    None => evaluated.all(),
                };
                MadeCurve {
                    source: curve,
                    kind: CurveType::Poly,
                    cyclic: self.cyclic.get(curve).copied().unwrap_or(false),
                    resolution: self.resolution.get(curve).copied().unwrap_or(12),
                    points: samples.into_iter().map(|sample| MadePoint::poly(sample.position, sample.from)).collect(),
                }
            })
            .collect();
        self.make(curves)
    }

    /// Trims the selected curves to the part between `starts` and `ends` (lengths along each);
    /// a cyclic curve trimmed from after its end round to before it runs through its start.
    pub fn trim(&self, selected: &[bool], starts: &[f32], ends: &[f32]) -> CurvesComponent {
        let curves = self
            .evaluated()
            .into_iter()
            .enumerate()
            .map(|(curve, evaluated)| {
                if !selected.get(curve).copied().unwrap_or(false) || evaluated.segments() == 0 {
                    return self.kept(curve);
                }
                let total = evaluated.length();
                let start = starts.get(curve).copied().unwrap_or(0.0).clamp(0.0, total);
                let mut end = ends.get(curve).copied().unwrap_or(total).clamp(0.0, total);
                if !evaluated.cyclic && end < start {
                    end = start;
                }
                let points = match self.kinds.get(curve) {
                    Some(CurveType::Bezier) => self.trim_bezier(curve, &evaluated, start, end),
                    Some(CurveType::CatmullRom) => self.trim_catmull(curve, &evaluated, start, end),
                    _ => self.trim_poly(curve, &evaluated, start, end),
                };
                MadeCurve {
                    source: curve,
                    kind: match self.kinds.get(curve) {
                        Some(CurveType::Bezier) => CurveType::Bezier,
                        Some(CurveType::CatmullRom) => CurveType::CatmullRom,
                        _ => CurveType::Poly,
                    },
                    cyclic: false,
                    resolution: if self.kinds.get(curve) == Some(&CurveType::Nurbs) { 12 } else { self.resolution.get(curve).copied().unwrap_or(12) },
                    points,
                }
            })
            .collect();
        self.make(curves)
    }

    fn trim_catmull(&self, curve: usize, evaluated: &Evaluated, start: f32, end: f32) -> Vec<MadePoint> {
        let from = evaluated.place(start);
        let to = evaluated.place(end);
        let resolution = self.resolution.get(curve).copied().unwrap_or(12).max(1) as usize;
        let sample = |place: Place| {
            let segment = place.segment / resolution;
            let factor = ((place.segment % resolution) as f64 + f64::from(place.factor)) / resolution as f64;
            let from = crate::spline_types::catmull_from(self, curve, segment, factor);
            MadePoint::poly(crate::spline_types::weighted_position(self, &from), from)
        };
        let mut points = vec![sample(from)];
        if start == end {
            return points;
        }
        let count = evaluated.positions.len();
        let last = if end < start { to.segment + count } else { to.segment };
        for at in from.segment + 1..=last {
            let index = at % count.max(1);
            if evaluated.control.binary_search(&index).is_ok() {
                points.push(MadePoint::poly(
                    evaluated.positions.get(index).copied().unwrap_or(Vec3::ZERO),
                    evaluated.from.get(index).map(|from| from.to_vec()).unwrap_or_default(),
                ));
            }
        }
        points.push(sample(to));
        points
    }

    fn trim_poly(&self, _curve: usize, evaluated: &Evaluated, start: f32, end: f32) -> Vec<MadePoint> {
        let count = evaluated.positions.len();
        let from = evaluated.place(start);
        let to = evaluated.place(end);
        let point = |place: Place| {
            let sample = evaluated.sample(place);
            MadePoint::poly(sample.position, sample.from)
        };
        let mut points = vec![point(from)];
        if start == end {
            return points;
        }
        // Through the control points after the start's segment up to the end's, round the
        // start of a cyclic curve when the end comes first.
        let last = if end < start || (to.segment < from.segment) { to.segment + count } else { to.segment };
        for index in from.segment + 1..=last {
            let index = index % count.max(1);
            points.push(MadePoint::poly(
                evaluated.positions.get(index).copied().unwrap_or(Vec3::ZERO),
                evaluated.from.get(index).map(|from| from.to_vec()).unwrap_or_default(),
            ));
        }
        points.push(point(to));
        points
    }

    fn trim_bezier(&self, curve: usize, evaluated: &Evaluated, start: f32, end: f32) -> Vec<MadePoint> {
        let range = self.range(curve);
        let (first, count) = (range.start, range.len());
        let parameter = |place: Place| -> (usize, f32) {
            let segment = evaluated.control.partition_point(|start| *start <= place.segment).saturating_sub(1).min(count.saturating_sub(1));
            let start = evaluated.control.get(segment).copied().unwrap_or(0);
            let end = evaluated.control.get(segment + 1).copied().unwrap_or(evaluated.positions.len());
            let steps = end.saturating_sub(start).max(1);
            (segment, (place.segment.saturating_sub(start) as f32 + place.factor) / steps as f32)
        };
        let (start_segment, start_t) = parameter(evaluated.place(start));
        let (end_segment, end_t) = parameter(evaluated.place(end));
        let wraps = end < start || (start != end && end_segment < start_segment) || (end_segment == start_segment && end_t < start_t);
        let original = |index: usize| -> MadePoint {
            let at = first + index % count;
            MadePoint {
                position: self.positions.get(at).copied().unwrap_or(Vec3::ZERO),
                left: self.handle_left.get(at).copied().unwrap_or(Vec3::ZERO),
                right: self.handle_right.get(at).copied().unwrap_or(Vec3::ZERO),
                type_left: self.handle_type_left.get(at).copied().unwrap_or_default(),
                type_right: self.handle_type_right.get(at).copied().unwrap_or_default(),
                from: vec![(at, 1.0)],
            }
        };
        let segment_of = |segment: usize| -> (MadePoint, MadePoint) { (original(segment), original(segment + 1)) };
        let at = |segment: usize, t: f32| -> MadePoint {
            let (a, b) = segment_of(segment);
            let cut = split(a.position, a.right, b.left, b.position, t);
            MadePoint {
                position: cut.point,
                left: cut.left,
                right: cut.right,
                type_left: a.type_right,
                type_right: b.type_left,
                from: vec![(first + segment % count, 1.0 - t), (first + (segment + 1) % count, t)],
            }
        };
        if start == end {
            let mut point = at(start_segment, start_t);
            point.from.retain(|(_, weight)| *weight > 0.0);
            return vec![point];
        }
        let end_segment = if wraps { end_segment + count } else { end_segment };
        if start_segment == end_segment {
            // One piece of one segment: cut at the end, then cut that piece at the start.
            let (a, b) = segment_of(start_segment);
            let tail = split(a.position, a.right, b.left, b.position, end_t);
            let mut last = at(start_segment, end_t);
            last.right = if end_t >= 1.0 { b.right } else { tail.right };
            let local = if end_t > 0.0 { start_t / end_t } else { 0.0 };
            let head = split(a.position, tail.first_right, tail.left, tail.point, local);
            let mut point = at(start_segment, start_t);
            point.position = head.point;
            point.left = if start_t <= 0.0 { a.left } else { head.left };
            point.right = head.right;
            last.left = head.last_left;
            return vec![point, last];
        }
        let mut points = Vec::new();
        let (a, b) = segment_of(start_segment);
        let cut = split(a.position, a.right, b.left, b.position, start_t);
        let mut head = at(start_segment, start_t);
        if start_t <= 0.0 {
            head = a.clone();
        } else {
            head.position = cut.point;
            head.left = cut.left;
            head.right = cut.right;
        }
        points.push(head);
        for index in start_segment + 1..=end_segment {
            let mut point = original(index);
            if index == start_segment + 1 && start_t > 0.0 {
                point.left = cut.last_left;
            }
            points.push(point);
        }
        // The end: cut the last segment.
        let (a, b) = segment_of(end_segment);
        let cut = split(a.position, a.right, b.left, b.position, end_t);
        if let Some(last) = points.last_mut() {
            last.right = cut.first_right;
        }
        let mut tail = at(end_segment, end_t);
        tail.left = cut.left;
        tail.right = if end_t >= 1.0 { b.right } else { cut.right };
        points.push(tail);
        points
    }

    /// Keeps the curves at `keep`.
    pub fn retain_curves(&self, keep: &[bool]) -> CurvesComponent {
        let curves = (0..self.curves_len()).filter(|curve| keep.get(*curve).copied().unwrap_or(false)).map(|curve| self.kept(curve)).collect();
        self.make(curves)
    }

    /// The rotation (XYZ Euler, radians) that turns Z to `tangent` and X to `normal`.
    pub fn rotation(tangent: Vec3, normal: Vec3) -> Vec3 {
        let binormal = tangent.cross(normal);
        kition_scene_model::scene::matrix_euler(Mat3::from_cols(normal, binormal, tangent))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bezier_segment_evaluates_its_resolution_and_end() {
        let mut curves = CurvesComponent::default();
        curves.push(
            CurveType::Bezier,
            false,
            4,
            &[
                ControlPoint::bezier(Vec3::new(-1.0, 0.0, 0.0), Vec3::new(-1.5, -0.5, 0.0), Vec3::new(-0.5, 0.5, 0.0)),
                ControlPoint::bezier(Vec3::X, Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0)),
            ],
        );
        let evaluated = curves.evaluate(0);
        assert_eq!(evaluated.positions.len(), 5);
        assert!((evaluated.positions[2] - Vec3::new(-0.1875, 0.1875, 0.0)).length() < 1e-5);
        assert!((evaluated.tangents[0] - Vec3::new(1.0, 1.0, 0.0).normalize()).length() < 1e-5);
        assert!((evaluated.normals[4] - Vec3::new(0.0, -1.0, 0.0)).length() < 1e-5);
        assert_eq!(evaluated.control, [0, 4]);
    }

    #[test]
    fn lengths_place_samples() {
        let curves = CurvesComponent::poly(&[Vec3::ZERO, Vec3::Z, Vec3::new(1.0, 0.0, 1.0)], false);
        let evaluated = curves.evaluate(0);
        assert_eq!(evaluated.length(), 2.0);
        assert_eq!(evaluated.place(1.5), Place { segment: 1, factor: 0.5 });
        let samples = evaluated.even(5);
        assert!((samples[3].position - Vec3::new(0.5, 0.0, 1.0)).length() < 1e-6);
        assert_eq!(count_for_length(1.0, 2.0, 2, true), 2);
        assert_eq!(count_for_length(1.0, 2.0, 2, false), 1);
        assert_eq!(count_for_length(1.0, 0.25, 2, true), 5);
    }
}
