//! Curve nodes' work: the curve primitives, curves to meshes and points and back, filling
//! closed curves, and the values the curve read nodes give. The shapes, point orders, and
//! defaults follow the reference application's nodes of the same names (checked in
//! `tests/curves_blender.rs`).

use std::collections::BTreeSet;
use std::f32::consts::TAU;

use glam::{Vec2, Vec3};
use kition_scene_model::Mesh;
use kition_scene_model::outline;

use crate::curves::{ControlPoint, CurveType, CurvesComponent, Sample, count_for_length};
use crate::field::{Column, Scalar};
use crate::geometry::{Attribute, Domain, MAX_ELEMENTS, MeshComponent, PointsComponent};

fn index(at: usize) -> u32 {
    u32::try_from(at).unwrap_or(u32::MAX)
}

fn too_many(count: usize) -> Result<(), String> {
    if count > MAX_ELEMENTS { Err(format!("the curve would have more than {MAX_ELEMENTS} points")) } else { Ok(()) }
}

/// Curve Line: from `start` to `end`.
pub fn line(start: Vec3, end: Vec3) -> CurvesComponent {
    CurvesComponent::poly(&[start, end], false)
}

/// Curve Circle by radius: `resolution` points from +X, counter-clockwise.
pub fn circle(resolution: i32, radius: f32) -> Result<CurvesComponent, String> {
    let count = usize::try_from(resolution).unwrap_or(0);
    if count < 3 {
        return Ok(CurvesComponent::default());
    }
    too_many(count)?;
    let points: Vec<Vec3> = (0..count)
        .map(|at| {
            let angle = TAU * at as f32 / count as f32;
            Vec3::new(angle.cos(), angle.sin(), 0.0) * radius
        })
        .collect();
    Ok(CurvesComponent::poly(&points, true))
}

/// Curve Circle through three points: its centre too. `None` when the points are in a line.
pub fn circle_through(resolution: i32, a: Vec3, b: Vec3, c: Vec3) -> Result<Option<(CurvesComponent, Vec3)>, String> {
    let count = usize::try_from(resolution).unwrap_or(0);
    if count < 3 {
        return Ok(None);
    }
    too_many(count)?;
    let (ab, ac) = (b - a, c - a);
    let cross = ab.cross(ac);
    let area = cross.length_squared();
    if area < 1e-12 {
        return Ok(None);
    }
    // The circumcentre.
    let centre = a + (cross.cross(ab) * ac.length_squared() + ac.cross(cross) * ab.length_squared()) / (2.0 * area);
    let radius = centre.distance(a);
    let normal = cross.normalize();
    let along = ab.normalize();
    let across = normal.cross(along);
    let points: Vec<Vec3> = (0..count)
        .map(|at| {
            let angle = TAU * at as f32 / count as f32;
            centre + (across * angle.cos() + along * angle.sin()) * radius
        })
        .collect();
    Ok(Some((CurvesComponent::poly(&points, true), centre)))
}

/// Bézier Segment: two points with aligned handles; `offset` places the handles relative to
/// their points.
pub fn bezier_segment(resolution: i32, start: Vec3, start_handle: Vec3, end_handle: Vec3, end: Vec3, offset: bool) -> CurvesComponent {
    let (start_right, end_left) = if offset { (start + start_handle, end + end_handle) } else { (start_handle, end_handle) };
    let mut out = CurvesComponent::default();
    out.push(
        CurveType::Bezier,
        false,
        u32::try_from(resolution).unwrap_or(1).max(1),
        &[ControlPoint::bezier(start, start * 2.0 - start_right, start_right), ControlPoint::bezier(end, end_left, end * 2.0 - end_left)],
    );
    out
}

/// Quadrilateral shapes.
pub enum Quadrilateral {
    Rectangle { width: f32, height: f32 },
    Parallelogram { width: f32, height: f32, offset: f32 },
    Trapezoid { bottom: f32, top: f32, offset: f32, height: f32 },
    Kite { width: f32, bottom: f32, top: f32 },
    Points([Vec3; 4]),
}

/// Quadrilateral: four points, closed.
pub fn quadrilateral(shape: &Quadrilateral) -> CurvesComponent {
    let flat = |x: f32, y: f32| Vec3::new(x, y, 0.0);
    let points = match *shape {
        Quadrilateral::Rectangle { width, height } => {
            let (x, y) = (width / 2.0, height / 2.0);
            [flat(x, y), flat(-x, y), flat(-x, -y), flat(x, -y)]
        }
        Quadrilateral::Parallelogram { width, height, offset } => {
            let (x, y, shift) = (width / 2.0, height / 2.0, offset / 2.0);
            [flat(x + shift, y), flat(-x + shift, y), flat(-x - shift, -y), flat(x - shift, -y)]
        }
        Quadrilateral::Trapezoid { bottom, top, offset, height } => {
            let y = height / 2.0;
            [flat(top / 2.0 + offset, y), flat(-top / 2.0 + offset, y), flat(-bottom / 2.0, -y), flat(bottom / 2.0, -y)]
        }
        Quadrilateral::Kite { width, bottom, top } => [flat(0.0, -bottom), flat(width / 2.0, 0.0), flat(0.0, top), flat(-width / 2.0, 0.0)],
        Quadrilateral::Points(points) => points,
    };
    CurvesComponent::poly(&points, true)
}

/// Spiral: `resolution` points per turn from `start_radius` to `end_radius`, rising `height`;
/// clockwise seen from above, or counter-clockwise when reversed.
pub fn spiral(resolution: i32, rotations: f32, start_radius: f32, end_radius: f32, height: f32, reverse: bool) -> Result<CurvesComponent, String> {
    let per_turn = resolution.max(1) as f32;
    let steps = (per_turn * rotations.abs()).floor();
    if !steps.is_finite() || steps < 1.0 {
        return Ok(CurvesComponent::default());
    }
    let steps = steps as usize;
    too_many(steps + 1)?;
    let direction = if reverse { 1.0 } else { -1.0 };
    let points: Vec<Vec3> = (0..=steps)
        .map(|at| {
            let factor = at as f32 / steps as f32;
            let angle = direction * TAU * rotations * factor;
            let radius = start_radius + (end_radius - start_radius) * factor;
            Vec3::new(angle.cos() * radius, angle.sin() * radius, height * factor)
        })
        .collect();
    Ok(CurvesComponent::poly(&points, false))
}

/// Star: outer and inner points in turn, the inner ones turned by `twist`; the outer points'
/// flags too.
pub fn star(points: i32, inner: f32, outer: f32, twist: f32) -> Result<(CurvesComponent, Vec<bool>), String> {
    let count = usize::try_from(points).unwrap_or(0);
    if count < 3 {
        return Ok((CurvesComponent::default(), Vec::new()));
    }
    too_many(count * 2)?;
    let step = TAU / count as f32;
    let mut out = Vec::with_capacity(count * 2);
    for at in 0..count {
        let angle = step * at as f32;
        out.push(Vec3::new(angle.cos(), angle.sin(), 0.0) * outer);
        let angle = angle + step / 2.0 + twist;
        out.push(Vec3::new(angle.cos(), angle.sin(), 0.0) * inner);
    }
    let outer_points = (0..count * 2).map(|at| at % 2 == 0).collect();
    Ok((CurvesComponent::poly(&out, true), outer_points))
}

/// Curve to Mesh: the profile's evaluated points swept along each curve (in the frame of its
/// normal, binormal, and tangent, scaled by `scale` per control point; as in the reference, the
/// curve's radius is not used), or,
/// without a profile, wires through the evaluated points. Open curves swept with closed profiles
/// get caps when `caps` is set.
pub fn curve_to_mesh(curves: &CurvesComponent, profile: Option<&CurvesComponent>, scale: &[f32], caps: bool) -> Mesh {
    let mut mesh = Mesh::default();
    let profiles: Vec<(Vec<Vec3>, bool)> = match profile {
        Some(profile) if profile.curves_len() > 0 => profile
            .evaluated()
            .into_iter()
            .filter(|evaluated| !evaluated.positions.is_empty())
            .map(|evaluated| (evaluated.positions, evaluated.cyclic))
            .collect(),
        _ => vec![(vec![Vec3::ZERO], false)],
    };
    let total = curves.evaluated_len().saturating_mul(profiles.iter().map(|(points, _)| points.len()).sum::<usize>().max(1));
    if total > MAX_ELEMENTS {
        return mesh;
    }
    let constant_scale = if scale.is_empty() { Some(1.0) } else { scale.first().filter(|first| scale.iter().all(|value| value == *first)).copied() };
    for evaluated in curves.evaluated() {
        let main = evaluated.positions.len();
        if main == 0 {
            continue;
        }
        let scales: Vec<f32> = if let Some(value) = constant_scale {
            vec![value; evaluated.from.len()]
        } else {
            evaluated.from.iter().map(|from| from.iter().map(|(index, weight)| scale.get(*index).copied().unwrap_or(1.0) * weight).sum()).collect()
        };
        let main_segments = if evaluated.cyclic && main > 2 { main } else { main - 1 };
        for (profile, profile_cyclic) in &profiles {
            let ring = profile.len();
            let first = mesh.positions.len();
            for at in 0..main {
                let pick = |values: &[Vec3]| values.get(at).copied().unwrap_or(Vec3::ZERO);
                let (position, tangent, normal) = (pick(&evaluated.positions), pick(&evaluated.tangents), pick(&evaluated.normals));
                let binormal = tangent.cross(normal);
                let size = scales.get(at).copied().unwrap_or(1.0);
                for point in profile {
                    mesh.positions.push(position + (normal * point.x + binormal * point.y + tangent * point.z) * size);
                }
            }
            let vertex = |main_at: usize, ring_at: usize| index(first + (main_at % main) * ring + ring_at % ring);
            let ring_segments = if *profile_cyclic && ring > 2 { ring } else { ring.saturating_sub(1) };
            if ring_segments > 0 && main_segments > 0 {
                for at in 0..main_segments {
                    for slot in 0..ring_segments {
                        mesh.faces.push(vec![vertex(at, slot), vertex(at, slot + 1), vertex(at + 1, slot + 1), vertex(at + 1, slot)]);
                    }
                }
            } else if ring_segments == 0 {
                for slot in 0..ring {
                    for at in 0..main_segments {
                        mesh.loose_edges.push([vertex(at, slot), vertex(at + 1, slot)]);
                    }
                }
            } else {
                for slot in 0..ring_segments {
                    mesh.loose_edges.push([vertex(0, slot), vertex(0, slot + 1)]);
                }
            }
            if caps && !evaluated.cyclic && *profile_cyclic && ring > 2 && main > 1 {
                mesh.faces.push((0..ring).rev().map(|slot| vertex(0, slot)).collect());
                mesh.faces.push((0..ring).map(|slot| vertex(main - 1, slot)).collect());
            }
        }
    }
    mesh
}

/// What Curve to Points places.
pub enum PointsMode<'a> {
    Evaluated,
    /// A count per curve.
    Count(&'a [usize]),
}

/// Curve to Points: samples of each curve as points with their radius and the curves'
/// attributes, and each point's tangent, normal, and rotation (stored under `names`).
pub fn curve_to_points(curves: &CurvesComponent, mode: &PointsMode<'_>, names: [&str; 3]) -> PointsComponent {
    let mut samples: Vec<(usize, Sample)> = Vec::new();
    for (curve, evaluated) in curves.evaluated().into_iter().enumerate() {
        let made = match mode {
            PointsMode::Evaluated => evaluated.all(),
            PointsMode::Count(counts) => evaluated.even(counts.get(curve).copied().unwrap_or(0)),
        };
        samples.extend(made.into_iter().map(|sample| (curve, sample)));
    }
    let mut points = PointsComponent {
        positions: samples.iter().map(|(_, sample)| sample.position).collect(),
        radius: samples.iter().map(|(_, sample)| sample.radius).collect(),
        attributes: Default::default(),
    };
    for (name, attribute) in &curves.attributes {
        let zero = attribute.values.first().map_or(Scalar::Float(0.0), |value| Scalar::zero(value.kind()));
        let values: Column = match attribute.domain {
            Domain::Point => {
                if let Some(first) = attribute.values.first().filter(|first| attribute.values.iter().all(|value| value == *first)) {
                    vec![*first; samples.len()]
                } else {
                    samples.iter().map(|(_, sample)| mix(&attribute.values, &sample.from, zero)).collect()
                }
            }
            _ => samples.iter().map(|(curve, _)| attribute.values.get(*curve).copied().unwrap_or(zero)).collect(),
        };
        points.attributes.insert(name.clone(), Attribute { domain: Domain::Point, values });
    }
    let [tangent, normal, rotation] = names;
    let column = |f: &dyn Fn(&Sample) -> Vec3| -> Column { samples.iter().map(|(_, sample)| Scalar::Vector(f(sample))).collect() };
    points.attributes.insert(tangent.to_owned(), Attribute { domain: Domain::Point, values: column(&|sample| sample.tangent) });
    points.attributes.insert(normal.to_owned(), Attribute { domain: Domain::Point, values: column(&|sample| sample.normal) });
    points.attributes.insert(
        rotation.to_owned(),
        Attribute { domain: Domain::Point, values: column(&|sample| CurvesComponent::rotation(sample.tangent, sample.normal)) },
    );
    points
}

fn mix(values: &Column, from: &[(usize, f32)], zero: Scalar) -> Scalar {
    let at = |index: usize| values.get(index).copied().unwrap_or(zero);
    match zero {
        Scalar::Float(_) => Scalar::Float(from.iter().map(|(index, weight)| at(*index).float() * weight).sum()),
        Scalar::Vector(_) => Scalar::Vector(from.iter().map(|(index, weight)| at(*index).vector() * *weight).sum()),
        _ => from.iter().max_by(|a, b| a.1.total_cmp(&b.1)).map_or(zero, |(index, _)| at(*index)),
    }
}

/// The per-curve counts a length gives.
pub fn counts_for_length(curves: &CurvesComponent, lengths: &[f32], keep_last: bool) -> Vec<usize> {
    curves
        .evaluated()
        .iter()
        .enumerate()
        .map(|(curve, evaluated)| {
            count_for_length(evaluated.length(), lengths.get(curve).copied().unwrap_or(0.1), evaluated.positions.len(), keep_last)
        })
        .collect()
}

/// Mesh to Curve from edges: chains of the selected edges, from each vertex where they do not
/// simply pass through (in vertex order, each edge's other end in vertex order), then the
/// closed loops left.
pub fn mesh_to_curve_edges(mesh: &MeshComponent, selected: &[bool]) -> CurvesComponent {
    let count = mesh.mesh.positions.len();
    let mut neighbours: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); count];
    for ([a, b], selected) in mesh.mesh.edges().into_iter().zip(selected) {
        let (a, b) = (a as usize, b as usize);
        if !*selected || a == b {
            continue;
        }
        if let Some(set) = neighbours.get_mut(a) {
            set.insert(b);
        }
        if let Some(set) = neighbours.get_mut(b) {
            set.insert(a);
        }
    }
    let degree = |vertex: usize| neighbours.get(vertex).map_or(0, BTreeSet::len);
    let mut used: BTreeSet<(usize, usize)> = BTreeSet::new();
    let key = |a: usize, b: usize| (a.min(b), a.max(b));
    let mut chains: Vec<(Vec<usize>, bool)> = Vec::new();
    let walk = |start: usize, next: usize, used: &mut BTreeSet<(usize, usize)>| -> (Vec<usize>, bool) {
        let mut chain = vec![start];
        let (mut previous, mut here) = (start, next);
        used.insert(key(previous, here));
        loop {
            if here == start {
                return (chain, true);
            }
            chain.push(here);
            if degree(here) != 2 {
                return (chain, false);
            }
            let Some(after) = neighbours.get(here).and_then(|set| set.iter().copied().find(|other| *other != previous)) else {
                return (chain, false);
            };
            if !used.insert(key(here, after)) {
                return (chain, false);
            }
            previous = here;
            here = after;
        }
    };
    for vertex in 0..count {
        if degree(vertex) == 2 || degree(vertex) == 0 {
            continue;
        }
        let around: Vec<usize> = neighbours.get(vertex).map(|set| set.iter().copied().collect()).unwrap_or_default();
        for next in around {
            if !used.contains(&key(vertex, next)) {
                chains.push(walk(vertex, next, &mut used));
            }
        }
    }
    for vertex in 0..count {
        let around: Vec<usize> = neighbours.get(vertex).map(|set| set.iter().copied().collect()).unwrap_or_default();
        for next in around {
            if !used.contains(&key(vertex, next)) {
                chains.push(walk(vertex, next, &mut used));
            }
        }
    }
    curves_from_vertices(mesh, &chains)
}

/// Mesh to Curve from faces: one closed curve round each selected face.
pub fn mesh_to_curve_faces(mesh: &MeshComponent, selected: &[bool]) -> CurvesComponent {
    let chains: Vec<(Vec<usize>, bool)> = mesh
        .mesh
        .faces
        .iter()
        .zip(selected)
        .filter(|(_, selected)| **selected)
        .map(|(face, _)| (face.iter().map(|vertex| *vertex as usize).collect(), true))
        .collect();
    curves_from_vertices(mesh, &chains)
}

fn curves_from_vertices(mesh: &MeshComponent, chains: &[(Vec<usize>, bool)]) -> CurvesComponent {
    let mut out = CurvesComponent::default();
    let mut order = Vec::new();
    for (chain, cyclic) in chains {
        let points: Vec<ControlPoint> =
            chain.iter().map(|vertex| ControlPoint::at(mesh.mesh.positions.get(*vertex).copied().unwrap_or(Vec3::ZERO))).collect();
        out.push(CurveType::Poly, *cyclic, 12, &points);
        order.extend(chain.iter().copied());
    }
    for (name, attribute) in &mesh.attributes {
        if name.starts_with('.') {
            continue;
        }
        let on_points = mesh.adapt(&attribute.values, attribute.domain, Domain::Point);
        let zero = on_points.first().map_or(Scalar::Float(0.0), |value| Scalar::zero(value.kind()));
        let values = order.iter().map(|vertex| on_points.get(*vertex).copied().unwrap_or(zero)).collect();
        out.attributes.insert(name.clone(), Attribute { domain: Domain::Point, values });
    }
    out
}

/// The most evaluated points Fill Curve triangulates (its outlines are cut in quadratic time).
pub const MAX_FILL: usize = 50_000;

/// How Fill Curve decides what is inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillRule {
    EvenOdd,
    NonZero,
}

/// Fill Curve: each group's curves (their evaluated points flattened onto XY) filled with
/// triangles, or with one face per outline when `ngons` is set and the outlines neither nest
/// nor cross. Every face faces +Z.
pub fn fill_curve(curves: &CurvesComponent, groups: &[i32], rule: FillRule, ngons: bool) -> Result<Mesh, String> {
    let mut mesh = Mesh::default();
    if curves.evaluated_len() > MAX_FILL {
        return Err(format!("Fill Curve takes at most {MAX_FILL} evaluated points"));
    }
    let evaluated = curves.evaluated();
    let mut starts = Vec::with_capacity(evaluated.len());
    for curve in &evaluated {
        starts.push(mesh.positions.len());
        mesh.positions.extend(curve.positions.iter().map(|point| Vec3::new(point.x, point.y, 0.0)));
    }
    let mut seen: Vec<i32> = Vec::new();
    for group in groups.iter().take(evaluated.len()) {
        if !seen.contains(group) {
            seen.push(*group);
        }
    }
    for group in seen {
        let members: Vec<usize> = (0..evaluated.len())
            .filter(|curve| groups.get(*curve) == Some(&group))
            .filter(|curve| evaluated.get(*curve).is_some_and(|curve| curve.positions.len() >= 3))
            .collect();
        let contours: Vec<Vec<Vec2>> = members
            .iter()
            .filter_map(|curve| evaluated.get(*curve))
            .map(|curve| curve.positions.iter().map(|point| point.truncate()).collect())
            .collect();
        let bases: Vec<usize> = members.iter().map(|curve| starts.get(*curve).copied().unwrap_or(0)).collect();
        let global = |contour: usize, slot: usize| index(bases.get(contour).copied().unwrap_or(0) + slot);
        if contours.is_empty() {
            continue;
        }
        let nested = (0..contours.len()).any(|a| (0..contours.len()).any(|b| a != b && inside(&contours, a, b)));
        if ngons && !nested && !outline::crosses(&contours) {
            for (contour, points) in contours.iter().enumerate() {
                let mut face: Vec<u32> = (0..points.len()).map(|slot| global(contour, slot)).collect();
                if outline::signed_area(points) < 0.0 {
                    face.reverse();
                }
                mesh.faces.push(face);
            }
            continue;
        }
        // Under the non-zero rule a contour inside one that winds the same way bounds nothing:
        // its points lie inside the fill.
        let boundary: Vec<bool> = match rule {
            FillRule::EvenOdd => vec![true; contours.len()],
            FillRule::NonZero => winding_boundaries(&contours),
        };
        let kept: Vec<usize> = (0..contours.len()).filter(|contour| boundary.get(*contour).copied().unwrap_or(true)).collect();
        let kept_contours: Vec<Vec<Vec2>> = kept.iter().filter_map(|contour| contours.get(*contour).cloned()).collect();
        let mut local: Vec<(Vec2, u32)> = Vec::new();
        for (slot, contour) in kept.iter().enumerate() {
            for (at, point) in kept_contours.get(slot).into_iter().flatten().enumerate() {
                local.push((*point, global(*contour, at)));
            }
        }
        let mut triangles: Vec<[u32; 3]> = outline::fill(&kept_contours)
            .into_iter()
            .filter_map(|[a, b, c]| Some([local.get(a as usize)?.1, local.get(b as usize)?.1, local.get(c as usize)?.1]))
            .collect();
        for (contour, points) in contours.iter().enumerate() {
            if boundary.get(contour).copied().unwrap_or(true) {
                continue;
            }
            for (slot, point) in points.iter().enumerate() {
                insert_point(&mut triangles, &mesh.positions, *point, global(contour, slot));
            }
        }
        for triangle in &mut triangles {
            let corner = |at: u32| mesh.positions.get(at as usize).map_or(Vec2::ZERO, |point| point.truncate());
            let [a, b, c] = *triangle;
            if (corner(b) - corner(a)).perp_dot(corner(c) - corner(a)) < 0.0 {
                *triangle = [a, c, b];
            }
        }
        mesh.faces.extend(triangles.into_iter().map(|triangle| triangle.to_vec()));
    }
    Ok(mesh)
}

/// Whether contour `a` lies inside contour `b`.
fn inside(contours: &[Vec<Vec2>], a: usize, b: usize) -> bool {
    match (contours.get(a).and_then(|contour| contour.first()), contours.get(b)) {
        (Some(point), Some(other)) => outline::contains(other, *point),
        _ => false,
    }
}

/// Which contours bound the region of non-zero winding: the winding just inside each one
/// against just outside it.
fn winding_boundaries(contours: &[Vec<Vec2>]) -> Vec<bool> {
    let turn = |contour: usize| if contours.get(contour).is_some_and(|points| outline::signed_area(points) > 0.0) { 1 } else { -1 };
    (0..contours.len())
        .map(|contour| {
            let outside: i32 = (0..contours.len()).filter(|other| *other != contour && inside(contours, contour, *other)).map(turn).sum();
            let within = outside + turn(contour);
            (outside == 0) != (within == 0)
        })
        .collect()
}

/// Splits the triangle holding `point` into three round it (a point inside the fill that is not
/// on its outline).
fn insert_point(triangles: &mut Vec<[u32; 3]>, positions: &[Vec3], point: Vec2, vertex: u32) {
    let corner = |at: u32| positions.get(at as usize).map_or(Vec2::ZERO, |point| point.truncate());
    let holds = |[a, b, c]: [u32; 3]| {
        let (pa, pb, pc) = (corner(a), corner(b), corner(c));
        let sides = [(pb - pa).perp_dot(point - pa), (pc - pb).perp_dot(point - pb), (pa - pc).perp_dot(point - pc)];
        sides.iter().all(|side| *side >= -1e-7) || sides.iter().all(|side| *side <= 1e-7)
    };
    let Some(found) = triangles.iter().position(|triangle| holds(*triangle)) else { return };
    let [a, b, c] = triangles.swap_remove(found);
    triangles.extend([[a, b, vertex], [b, c, vertex], [c, a, vertex]]);
}

/// Spline Parameter on the points: each control point's length along its curve, that over the
/// curve's length, and its index in the curve.
pub fn spline_parameter_points(curves: &CurvesComponent) -> [Column; 3] {
    let (mut factors, mut lengths, mut indices) = (Vec::new(), Vec::new(), Vec::new());
    let constants = curves.constant_attributes();
    for curve in 0..curves.curves_len() {
        // NURBS control points need not lie on the evaluated spline. The reference reports
        // their control-polygon distances for this point-domain field.
        let evaluated = if curves.kinds.get(curve) == Some(&CurveType::Nurbs) {
            let positions: Vec<Vec3> = curves.range(curve).filter_map(|at| curves.positions.get(at).copied()).collect();
            CurvesComponent::poly(&positions, curves.cyclic.get(curve).copied().unwrap_or(false)).evaluate(0)
        } else {
            curves.evaluate_with_constants(curve, constants)
        };
        let total = evaluated.length();
        let count = evaluated.control.len();
        for (slot, at) in evaluated.control.iter().enumerate() {
            let length = evaluated.length_at(*at);
            let factor = if total > 0.0 {
                length / total
            } else if count > 1 {
                slot as f32 / (count - 1) as f32
            } else {
                0.0
            };
            factors.push(Scalar::Float(factor));
            lengths.push(Scalar::Float(length));
            indices.push(Scalar::Int(i32::try_from(slot).unwrap_or(i32::MAX)));
        }
    }
    [factors, lengths, indices]
}

/// Spline Parameter on the curves: the length of the curves before each one, that over the
/// length of all of them, and zero for the index.
pub fn spline_parameter_curves(curves: &CurvesComponent) -> [Column; 3] {
    let lengths: Vec<f32> = curves.evaluated().iter().map(|evaluated| evaluated.length()).collect();
    let total: f32 = lengths.iter().sum();
    let mut before = 0.0;
    let (mut factors, mut accumulated) = (Vec::new(), Vec::new());
    for length in &lengths {
        factors.push(Scalar::Float(if total > 0.0 { before / total } else { 0.0 }));
        accumulated.push(Scalar::Float(before));
        before += length;
    }
    let indices = vec![Scalar::Int(0); lengths.len()];
    [factors, accumulated, indices]
}

/// The length of all the curves together.
pub fn total_length(curves: &CurvesComponent) -> f32 {
    curves.evaluated().iter().map(|evaluated| evaluated.length()).sum()
}

/// Endpoint Selection: the first `starts` and last `ends` points of each curve.
pub fn endpoints(curves: &CurvesComponent, starts: &[i32], ends: &[i32]) -> Column {
    let mut out = Vec::with_capacity(curves.points_len());
    for curve in 0..curves.curves_len() {
        let count = curves.range(curve).len();
        let start = usize::try_from(starts.get(curve).copied().unwrap_or(0)).unwrap_or(0);
        let end = usize::try_from(ends.get(curve).copied().unwrap_or(0)).unwrap_or(0);
        out.extend((0..count).map(|at| Scalar::Bool(at < start || at + end >= count)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_swept_with_a_circle_is_a_capped_tube() {
        let mesh = curve_to_mesh(&line(Vec3::ZERO, Vec3::Z), Some(&circle(4, 0.5).unwrap()), &[], true);
        assert_eq!((mesh.positions.len(), mesh.faces.len(), mesh.edges().len()), (8, 6, 12));
        assert_eq!(mesh.faces[0], [0, 1, 5, 4]);
        assert_eq!(mesh.faces[4], [3, 2, 1, 0]);
        assert!((mesh.positions[5] - Vec3::new(0.0, 0.5, 1.0)).length() < 1e-6);
    }

    #[test]
    fn a_ring_fills_around_its_hole() {
        let mut ring = circle(6, 1.0).unwrap();
        ring.append(&circle(4, 0.4).unwrap());
        assert_eq!(fill_curve(&ring, &[0, 0], FillRule::EvenOdd, false).unwrap().faces.len(), 10);
        assert_eq!(fill_curve(&ring, &[0, 0], FillRule::NonZero, false).unwrap().faces.len(), 12);
        assert_eq!(fill_curve(&circle(6, 1.0).unwrap(), &[0], FillRule::EvenOdd, true).unwrap().faces.len(), 1);
    }
}
