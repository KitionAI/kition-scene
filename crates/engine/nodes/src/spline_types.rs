//! Additional geometry-curve types, implemented from public application recordings and the
//! standard uniform Catmull–Rom and Cox–de Boor basis equations. No application source is used.

use glam::{DVec3, Vec3};
use kition_scene_model::HandleType;

use crate::curves::{CurveType, CurvesComponent, Evaluated, MadePoint, Sources};
use crate::geometry::MAX_ELEMENTS;

/// Generated knot vectors supported by the curve component.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum KnotsMode {
    #[default]
    Normal,
    Endpoint,
    Bezier,
}

/// Bounds local basis work independently of the number of curve points.
pub const MAX_NURBS_ORDER: u8 = 16;

fn point(curves: &CurvesComponent, index: usize) -> Vec3 {
    curves.positions.get(index).copied().unwrap_or(Vec3::ZERO)
}

pub(crate) fn weighted_position(curves: &CurvesComponent, from: &[(usize, f32)]) -> Vec3 {
    from.iter().fold(DVec3::ZERO, |sum, (index, weight)| sum + point(curves, *index).as_dvec3() * f64::from(*weight)).as_vec3()
}

pub(crate) fn catmull_from(curves: &CurvesComponent, curve: usize, segment: usize, t: f64) -> Vec<(usize, f32)> {
    let range = curves.range(curve);
    let count = range.len();
    if count < 2 {
        return vec![(range.start, 1.0)];
    }
    let cyclic = curves.cyclic.get(curve).copied().unwrap_or(false);
    let segment = segment.min(count - 1);
    let next = (segment + 1) % count;
    let before = if count == 2 {
        segment
    } else if segment == 0 {
        if cyclic { count - 1 } else { 0 }
    } else {
        segment - 1
    };
    let after = if count == 2 {
        next
    } else if cyclic {
        (segment + 2) % count
    } else {
        (segment + 2).min(count - 1)
    };
    let t2 = t * t;
    let t3 = t2 * t;
    let weights = [-0.5 * t + t2 - 0.5 * t3, 1.0 - 2.5 * t2 + 1.5 * t3, 0.5 * t + 2.0 * t2 - 1.5 * t3, -0.5 * t2 + 0.5 * t3];
    [before, segment, next, after].into_iter().zip(weights).map(|(at, weight)| (range.start + at, weight as f32)).collect()
}

/// Uniform cubic interpolation; open ends repeat the nearest control point.
pub(crate) fn evaluate_catmull(curves: &CurvesComponent, curve: usize, out: &mut Evaluated) {
    let range = curves.range(curve);
    let count = range.len();
    if count < 2 {
        out.positions.extend(range.clone().map(|at| point(curves, at)));
        out.from.extend(range.map(|at| Sources::Pair([(at, 1.0), (at, 0.0)])));
        out.control.extend(0..count);
        return;
    }
    let resolution = curves.resolution.get(curve).copied().unwrap_or(12).max(1) as usize;
    let segments = if out.cyclic { count } else { count - 1 };
    if segments.saturating_mul(resolution).saturating_add(usize::from(!out.cyclic)) > MAX_ELEMENTS {
        return;
    }
    for segment in 0..segments {
        out.control.push(out.positions.len());
        for step in 0..resolution {
            let t = step as f64 / resolution as f64;
            let from = catmull_from(curves, curve, segment, t);
            out.positions.push(weighted_position(curves, &from));
            out.from.push(from.into());
        }
    }
    if !out.cyclic {
        out.control.push(out.positions.len());
        out.positions.push(point(curves, range.end - 1));
        out.from.push(Sources::Pair([(range.end - 1, 1.0), (range.end - 1, 0.0)]));
    }
}

struct NurbsPlan {
    degree: usize,
    controls: usize,
    segments: usize,
    mode: KnotsMode,
    resolution: usize,
    cyclic: bool,
}

fn nurbs_plan(curves: &CurvesComponent, curve: usize) -> Option<NurbsPlan> {
    let count = curves.range(curve).len();
    let order = usize::from(curves.nurbs_order.get(curve).copied().unwrap_or(4));
    if order < 2 || order > count || order > usize::from(MAX_NURBS_ORDER) {
        return None;
    }
    let cyclic = curves.cyclic.get(curve).copied().unwrap_or(false) && count > 1;
    let mode = curves.knots_mode.get(curve).copied().unwrap_or_default();
    let degree = order - 1;
    let (controls, segments) = match mode {
        KnotsMode::Normal => {
            let controls = count + if cyclic { degree } else { 0 };
            (controls, controls - degree)
        }
        KnotsMode::Endpoint => {
            let controls = count + usize::from(cyclic);
            (controls, controls - degree)
        }
        KnotsMode::Bezier => {
            let offset = usize::from(degree > 1);
            let segments = if cyclic { count / degree } else { count.saturating_sub(1 + offset) / degree };
            if segments == 0 || cyclic && !count.is_multiple_of(degree) {
                return None;
            }
            (count, segments)
        }
    };
    Some(NurbsPlan { degree, controls, segments, mode, resolution: curves.resolution.get(curve).copied().unwrap_or(12).max(1) as usize, cyclic })
}

pub(crate) fn nurbs_evaluated_len(curves: &CurvesComponent, curve: usize) -> usize {
    nurbs_plan(curves, curve)
        .map_or(curves.range(curve).len(), |plan| plan.segments.saturating_mul(plan.resolution).saturating_add(usize::from(!plan.cyclic)))
}

/// Nonzero B-spline basis functions at `u`, in a span found by binary search. Local support
/// keeps work quadratic only in the bounded degree, never in the full control-point count.
fn basis(knots: &[f64], degree: usize, controls: usize, u: f64) -> Vec<(usize, f64)> {
    let span = knots.partition_point(|knot| *knot <= u).saturating_sub(1).min(controls.saturating_sub(1)).max(degree);
    let knot = |index: usize| knots.get(index).copied().unwrap_or(u);
    let mut values = vec![0.0; degree + 1];
    if let Some(first) = values.first_mut() {
        *first = 1.0;
    }
    for level in 1..=degree {
        let mut saved = 0.0;
        for r in 0..level {
            let left = u - knot(span + 1 - level + r);
            let right = knot(span + r + 1) - u;
            let denominator = left + right;
            let term = if denominator != 0.0 { values.get(r).copied().unwrap_or(0.0) / denominator } else { 0.0 };
            if let Some(value) = values.get_mut(r) {
                *value = saved + right * term;
            }
            saved = left * term;
        }
        if let Some(value) = values.get_mut(level) {
            *value = saved;
        }
    }
    values.into_iter().enumerate().map(|(local, value)| (span - degree + local, value)).collect()
}

fn bernstein(degree: usize, t: f64) -> Vec<f64> {
    let mut values = vec![0.0; degree + 1];
    if let Some(first) = values.first_mut() {
        *first = 1.0;
    }
    for level in 1..=degree {
        let mut previous = 0.0;
        for at in 0..=level {
            if let Some(value) = values.get_mut(at) {
                let old = *value;
                *value = old * (1.0 - t) + previous * t;
                previous = old;
            }
        }
    }
    values
}

pub(crate) fn evaluate_nurbs(curves: &CurvesComponent, curve: usize, out: &mut Evaluated) {
    let range = curves.range(curve);
    let count = range.len();
    let Some(plan) = nurbs_plan(curves, curve) else {
        out.positions.extend(range.clone().map(|at| point(curves, at)));
        out.from.extend(range.map(|at| Sources::Pair([(at, 1.0), (at, 0.0)])));
        out.control.extend(0..count);
        return;
    };
    let total = nurbs_evaluated_len(curves, curve);
    if total > MAX_ELEMENTS {
        return;
    }
    let knots: Vec<f64> = (0..plan.controls + plan.degree + 1)
        .map(|at| if plan.mode == KnotsMode::Endpoint { at.saturating_sub(plan.degree).min(plan.segments) as f64 } else { at as f64 })
        .collect();
    let begin = if plan.mode == KnotsMode::Normal { plan.degree as f64 } else { 0.0 };
    for step in 0..total {
        let parameter = step as f64 / plan.resolution as f64;
        let basis = if plan.mode == KnotsMode::Bezier {
            let segment = (step / plan.resolution).min(plan.segments - 1);
            let t = parameter - segment as f64;
            let offset = usize::from(plan.degree > 1);
            bernstein(plan.degree, t).into_iter().enumerate().map(|(at, value)| (offset + segment * plan.degree + at, value)).collect()
        } else {
            basis(&knots, plan.degree, plan.controls, begin + parameter)
        };
        let mut weighted: Vec<(usize, f64)> = basis
            .into_iter()
            .map(|(at, value)| {
                let index = range.start + at % count;
                (index, value * f64::from(curves.nurbs_weight.get(index).copied().unwrap_or(1.0)))
            })
            .collect();
        let sum: f64 = weighted.iter().map(|(_, value)| *value).sum();
        for (_, value) in &mut weighted {
            *value = if sum != 0.0 { *value / sum } else { 0.0 };
        }
        let from: Vec<(usize, f32)> = weighted.into_iter().map(|(at, value)| (at, value as f32)).collect();
        out.positions.push(weighted_position(curves, &from));
        out.from.push(from.into());
    }
    out.control.extend((0..count).map(|at| at.saturating_mul(total.saturating_sub(1)) / count.saturating_sub(1).max(1)));
}

fn catmull_handles(curves: &CurvesComponent, curve: usize, local: usize) -> (Vec3, Vec3) {
    let range = curves.range(curve);
    let count = range.len();
    let cyclic = curves.cyclic.get(curve).copied().unwrap_or(false);
    let before = if local == 0 { if cyclic { count.saturating_sub(1) } else { 0 } } else { local - 1 };
    let after = if cyclic { (local + 1) % count.max(1) } else { (local + 1).min(count.saturating_sub(1)) };
    let here = point(curves, range.start + local).as_dvec3();
    let tangent = (point(curves, range.start + after).as_dvec3() - point(curves, range.start + before).as_dvec3()) / 6.0;
    ((here - tangent).as_vec3(), (here + tangent).as_vec3())
}

/// Converts selected splines. NURBS to Bézier is deliberately limited to the exact reversible
/// cubic Bézier-knot representation; other NURBS conversions need a separate fitting policy.
pub fn convert(curves: &CurvesComponent, selected: &[bool], target: CurveType) -> Result<CurvesComponent, String> {
    let mut made = Vec::with_capacity(curves.curves_len());
    let mut settings = Vec::with_capacity(curves.curves_len());
    let mut reset_weights = Vec::new();
    for curve in 0..curves.curves_len() {
        let mut output = curves.kept(curve);
        let source = output.kind;
        let mut order = curves.nurbs_order.get(curve).copied().unwrap_or(4);
        let mut knots = curves.knots_mode.get(curve).copied().unwrap_or_default();
        if !selected.get(curve).copied().unwrap_or(false) || source == target {
            made.push(output);
            settings.push((order, knots));
            continue;
        }
        if target == CurveType::Nurbs {
            reset_weights.push(curve);
            order = 4;
            knots = if output.cyclic { KnotsMode::Normal } else { KnotsMode::Endpoint };
            if matches!(source, CurveType::Bezier | CurveType::CatmullRom) {
                if curves.points_len().saturating_mul(3) > MAX_ELEMENTS {
                    return Err("converting splines would exceed the geometry point limit".into());
                }
                knots = KnotsMode::Bezier;
                let mut expanded = Vec::with_capacity(output.points.len() * 3);
                for (local, control) in output.points.iter().enumerate() {
                    let (left, right) =
                        if source == CurveType::CatmullRom { catmull_handles(curves, curve, local) } else { (control.left, control.right) };
                    for position in [left, control.position, right] {
                        expanded.push(MadePoint::poly(position, control.from.clone()));
                    }
                }
                output.points = expanded;
            }
        } else if target == CurveType::Bezier {
            if source == CurveType::Nurbs {
                if order != 4 || knots != KnotsMode::Bezier || !output.points.len().is_multiple_of(3) {
                    return Err(
                        "NURBS to Bezier requires cubic Bezier knots from a Bezier or Catmull Rom conversion; resample to Poly first for other NURBS"
                            .into(),
                    );
                }
                let range = curves.range(curve);
                let first_weight = curves.nurbs_weight.get(range.start).copied().unwrap_or(1.0);
                if first_weight <= 0.0 || range.clone().any(|at| curves.nurbs_weight.get(at).copied().unwrap_or(1.0) != first_weight) {
                    return Err("NURBS to Bezier requires equal positive weights; resample to Poly first for rational curves".into());
                }
                output.points = output
                    .points
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .filter_map(|triple| {
                        let mut control = triple.get(1)?.clone();
                        control.left = triple.first()?.position;
                        control.right = triple.get(2)?.position;
                        control.type_left = HandleType::Aligned;
                        control.type_right = HandleType::Aligned;
                        Some(control)
                    })
                    .collect();
            } else {
                let count = output.points.len();
                let range = curves.range(curve);
                for (local, control) in output.points.iter_mut().enumerate() {
                    if source == CurveType::CatmullRom {
                        (control.left, control.right) = catmull_handles(curves, curve, local);
                        control.type_left = HandleType::Aligned;
                        control.type_right = HandleType::Aligned;
                    } else {
                        let cyclic = output.cyclic;
                        let before = if local == 0 { if cyclic { count.saturating_sub(1) } else { 0 } } else { local - 1 };
                        let after = if cyclic { (local + 1) % count.max(1) } else { (local + 1).min(count.saturating_sub(1)) };
                        control.left = control.position.lerp(point(curves, range.start + before), 1.0 / 3.0);
                        control.right = control.position.lerp(point(curves, range.start + after), 1.0 / 3.0);
                        if !cyclic && local == 0 && count > 1 {
                            control.left = control.position * 2.0 - control.right;
                        }
                        if !cyclic && local + 1 == count && count > 1 {
                            control.right = control.position * 2.0 - control.left;
                        }
                        control.type_left = HandleType::Vector;
                        control.type_right = HandleType::Vector;
                    }
                }
            }
        }
        if target == CurveType::Poly {
            output.resolution = 12;
        }
        output.kind = target;
        made.push(output);
        settings.push((order, knots));
    }
    let mut out = curves.make(made);
    for (curve, (order, knots)) in settings.into_iter().enumerate() {
        if let Some(value) = out.nurbs_order.get_mut(curve) {
            *value = order;
        }
        if let Some(value) = out.knots_mode.get_mut(curve) {
            *value = knots;
        }
    }
    for curve in reset_weights {
        for at in out.range(curve) {
            if let Some(weight) = out.nurbs_weight.get_mut(at) {
                *weight = 1.0;
            }
        }
    }
    Ok(out)
}
