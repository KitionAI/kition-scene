//! Fields that read other geometry: Sample Index (a value of an element picked by index),
//! Sample Nearest (the index of the closest element), Raycast (what a ray from each element
//! hits), and Geometry Proximity (the closest place on points, edges, or faces).

use std::sync::Arc;

use glam::Vec3;

use crate::field::{self, Column, Context, Field, Kind, Scalar};
use crate::geometry::{Component, Domain, Geometry, MeshComponent};

/// The most element pairs one sampling compares (sources times targets).
pub const MAX_PAIRS: u64 = 400_000_000;

/// What Geometry Proximity looks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Element {
    Points,
    Edges,
    Faces,
}

/// A field that reads other geometry.
#[derive(Debug, Clone, PartialEq)]
pub enum Sample {
    Index {
        geometry: Arc<Geometry>,
        value: Field,
        index: Field,
        domain: Domain,
        clamp: bool,
        kind: Kind,
    },
    Nearest {
        geometry: Arc<Geometry>,
        position: Field,
        domain: Domain,
    },
    /// Outputs: Is Hit (0), Hit Position (1), Hit Normal (2), Hit Distance (3), Attribute (4).
    Raycast {
        target: Arc<Geometry>,
        attribute: Field,
        source: Field,
        direction: Field,
        length: Field,
        interpolated: bool,
        output: usize,
        kind: Kind,
    },
    /// Outputs: Position (0), Distance (1), Is Valid (2).
    Proximity {
        target: Arc<Geometry>,
        group: Field,
        source: Field,
        sample_group: Field,
        element: Element,
        output: usize,
    },
}

impl Sample {
    /// The value type the field gives.
    pub fn kind(&self) -> Kind {
        match self {
            Self::Index { kind, .. } => *kind,
            Self::Nearest { .. } => Kind::Int,
            Self::Raycast { output, kind, .. } => match output {
                0 => Kind::Bool,
                1 | 2 => Kind::Vector,
                3 => Kind::Float,
                _ => *kind,
            },
            Self::Proximity { output, .. } => match output {
                0 => Kind::Vector,
                1 => Kind::Float,
                _ => Kind::Bool,
            },
        }
    }
}

/// The component of `geometry` that has `domain`: the mesh first, then points, curves, and
/// instances, as the reference looks.
pub fn component_with(geometry: &Geometry, domain: Domain) -> Option<Component<'_>> {
    let mesh = geometry.mesh.as_ref().map(Component::Mesh);
    let points = geometry.points.as_ref().map(Component::Points);
    let curves = geometry.curves.as_ref().map(Component::Curves);
    let instances = geometry.instances.as_ref().map(Component::Instances);
    let candidates: Vec<Component<'_>> = match domain {
        Domain::Point => [mesh, points, curves].into_iter().flatten().collect(),
        Domain::Edge | Domain::Face | Domain::Corner => mesh.into_iter().collect(),
        Domain::Curve => curves.into_iter().collect(),
        Domain::Instance => instances.into_iter().collect(),
    };
    candidates.iter().copied().find(|component| component.domain_len(domain) > 0).or_else(|| candidates.first().copied())
}

fn too_many(sources: usize, targets: usize) -> bool {
    (sources as u64).saturating_mul(targets as u64) > MAX_PAIRS
}

/// Evaluates a sampling field on the context's domain.
pub fn evaluate(sample: &Sample, context: &Context<'_>) -> Column {
    let len = context.len();
    let zero = Scalar::zero(sample.kind());
    match sample {
        Sample::Index { geometry, value, index, domain, clamp, kind } => {
            let Some(source) = component_with(geometry, *domain) else { return vec![zero; len] };
            let values = field::evaluate(value, &Context { component: source, domain: *domain });
            let indices = field::evaluate(index, context);
            indices
                .iter()
                .map(|index| {
                    let index = index.int();
                    let at = if *clamp && !values.is_empty() {
                        usize::try_from(index.max(0)).unwrap_or(0).min(values.len() - 1)
                    } else {
                        match usize::try_from(index) {
                            Ok(at) => at,
                            Err(_) => return zero,
                        }
                    };
                    values.get(at).map_or(zero, |value| value.to(*kind))
                })
                .collect()
        }
        Sample::Nearest { geometry, position, domain } => {
            let positions: Vec<Vec3> = field::evaluate(position, context).iter().map(|value| value.vector()).collect();
            let Some(source) = component_with(geometry, *domain) else { return vec![Scalar::Int(0); len] };
            nearest_elements(source, *domain, &positions).into_iter().map(|index| Scalar::Int(index.map_or(0, to_int))).collect()
        }
        Sample::Raycast { target, attribute, source, direction, length, interpolated, output, kind } => {
            let sources: Vec<Vec3> = field::evaluate(source, context).iter().map(|value| value.vector()).collect();
            let directions: Vec<Vec3> = field::evaluate(direction, context).iter().map(|value| value.vector()).collect();
            let lengths: Vec<f32> = field::evaluate(length, context).iter().map(|value| value.float()).collect();
            let Some(mesh) = target.mesh.as_ref().filter(|mesh| !mesh.mesh.faces.is_empty()) else { return vec![zero; len] };
            let triangles = mesh.mesh.triangles();
            if too_many(len, triangles.len()) {
                return vec![zero; len];
            }
            let values = if *output == 4 {
                field::evaluate(attribute, &Context { component: Component::Mesh(mesh), domain: Domain::Point })
            } else {
                Vec::new()
            };
            (0..len)
                .map(|at| {
                    let origin = sources.get(at).copied().unwrap_or(Vec3::ZERO);
                    let direction = directions.get(at).copied().unwrap_or(Vec3::NEG_Z).normalize_or_zero();
                    let reach = lengths.get(at).copied().unwrap_or(0.0);
                    let hit = raycast(mesh, &triangles, origin, direction, reach);
                    match (output, hit) {
                        (0, hit) => Scalar::Bool(hit.is_some()),
                        // A miss is as far as the ray reaches.
                        (3, None) => Scalar::Float(reach),
                        (_, None) => zero,
                        (1, Some(hit)) => Scalar::Vector(origin + direction * hit.distance),
                        (2, Some(hit)) => Scalar::Vector(hit.normal),
                        (3, Some(hit)) => Scalar::Float(hit.distance),
                        (_, Some(hit)) => {
                            let corner = |slot: usize| {
                                hit.vertices.get(slot).and_then(|vertex| values.get(*vertex as usize)).copied().unwrap_or(zero).to(*kind)
                            };
                            let weight = |slot: usize| hit.weights.get(slot).copied().unwrap_or(0.0);
                            if *interpolated {
                                mix(*kind, &[(corner(0), weight(0)), (corner(1), weight(1)), (corner(2), weight(2))])
                            } else {
                                let nearest = (0..3).max_by(|a, b| weight(*a).total_cmp(&weight(*b))).unwrap_or(0);
                                corner(nearest)
                            }
                        }
                    }
                })
                .collect()
        }
        Sample::Proximity { target, group, source, sample_group, element, output } => {
            let sources: Vec<Vec3> = field::evaluate(source, context).iter().map(|value| value.vector()).collect();
            let wanted: Vec<i32> = field::evaluate(sample_group, context).iter().map(|value| value.int()).collect();
            let places = proximity(target, group, *element, &sources, &wanted);
            places
                .into_iter()
                .zip(&sources)
                .map(|(place, from)| match (output, place) {
                    (2, place) => Scalar::Bool(place.is_some()),
                    (_, None) => zero,
                    (0, Some(place)) => Scalar::Vector(place),
                    (_, Some(place)) => Scalar::Float(place.distance(*from)),
                })
                .collect()
        }
    }
}

fn to_int(index: usize) -> i32 {
    i32::try_from(index).unwrap_or(i32::MAX)
}

fn mix(kind: Kind, parts: &[(Scalar, f32)]) -> Scalar {
    match kind {
        Kind::Vector => Scalar::Vector(parts.iter().map(|(value, weight)| value.vector() * *weight).sum()),
        Kind::Float => Scalar::Float(parts.iter().map(|(value, weight)| value.float() * *weight).sum()),
        Kind::Int => Scalar::Int(parts.iter().map(|(value, weight)| value.float() * *weight).sum::<f32>().round() as i32),
        Kind::Bool => Scalar::Bool(parts.iter().map(|(value, weight)| value.float() * *weight).sum::<f32>() > 0.5),
    }
}

/// The closest point to `point` on the segment from `a` to `b`.
pub fn closest_on_segment(point: Vec3, a: Vec3, b: Vec3) -> Vec3 {
    let span = b - a;
    let length = span.length_squared();
    if length <= 0.0 {
        return a;
    }
    a + span * ((point - a).dot(span) / length).clamp(0.0, 1.0)
}

/// The closest point to `p` on triangle `a b c`.
pub fn closest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denominator = va + vb + vc;
    if denominator == 0.0 {
        return a;
    }
    a + ab * (vb / denominator) + ac * (vc / denominator)
}

/// The index of the element of `domain` closest to each position: points by their position,
/// edges and faces by the closest place on them, corners by the closest corner of the closest
/// face.
fn nearest_elements(source: Component<'_>, domain: Domain, positions: &[Vec3]) -> Vec<Option<usize>> {
    match (source, domain) {
        (Component::Mesh(mesh), Domain::Edge) => {
            let points = &mesh.mesh.positions;
            let edges = mesh.mesh.edges();
            if too_many(positions.len(), edges.len()) {
                return vec![None; positions.len()];
            }
            let at = |index: u32| points.get(index as usize).copied().unwrap_or(Vec3::ZERO);
            positions
                .iter()
                .map(|point| closest_index(edges.iter().map(|[a, b]| closest_on_segment(*point, at(*a), at(*b)).distance_squared(*point))))
                .collect()
        }
        (Component::Mesh(mesh), Domain::Face | Domain::Corner) => {
            let triangles = mesh.mesh.triangles();
            if too_many(positions.len(), triangles.len()) {
                return vec![None; positions.len()];
            }
            let points = &mesh.mesh.positions;
            let at = |index: u32| points.get(index as usize).copied().unwrap_or(Vec3::ZERO);
            let starts: Vec<usize> = mesh
                .mesh
                .faces
                .iter()
                .scan(0usize, |start, face| {
                    let here = *start;
                    *start += face.len();
                    Some(here)
                })
                .collect();
            positions
                .iter()
                .map(|point| {
                    let best = closest_index(
                        triangles.iter().map(|([a, b, c], _)| closest_on_triangle(*point, at(*a), at(*b), at(*c)).distance_squared(*point)),
                    )?;
                    let (_, face) = triangles.get(best)?;
                    if domain == Domain::Face {
                        return Some(*face as usize);
                    }
                    let corners = mesh.mesh.faces.get(*face as usize)?;
                    let corner = closest_index(corners.iter().map(|vertex| at(*vertex).distance_squared(*point)))?;
                    Some(starts.get(*face as usize)? + corner)
                })
                .collect()
        }
        (component, _) => {
            let targets = component.positions(Domain::Point);
            if too_many(positions.len(), targets.len()) {
                return vec![None; positions.len()];
            }
            positions.iter().map(|point| closest_index(targets.iter().map(|target| target.distance_squared(*point)))).collect()
        }
    }
}

/// The index of the smallest distance (the first of equal ones).
fn closest_index(distances: impl Iterator<Item = f32>) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (index, distance) in distances.enumerate() {
        if distance.is_finite() && best.is_none_or(|(_, smallest)| distance < smallest) {
            best = Some((index, distance));
        }
    }
    best.map(|(index, _)| index)
}

/// Where a ray hits a mesh.
struct Hit {
    distance: f32,
    normal: Vec3,
    vertices: [u32; 3],
    weights: [f32; 3],
}

fn raycast(mesh: &MeshComponent, triangles: &[([u32; 3], u32)], origin: Vec3, direction: Vec3, reach: f32) -> Option<Hit> {
    if direction == Vec3::ZERO || reach.is_nan() || reach <= 0.0 || !origin.is_finite() {
        return None;
    }
    let at = |index: u32| mesh.mesh.positions.get(index as usize).copied().unwrap_or(Vec3::ZERO);
    let mut best: Option<Hit> = None;
    for (vertices, face) in triangles {
        let [a, b, c] = vertices.map(at);
        let (ab, ac) = (b - a, c - a);
        let p = direction.cross(ac);
        let determinant = ab.dot(p);
        if determinant.abs() < 1e-12 {
            continue;
        }
        let inverse = 1.0 / determinant;
        let t_vec = origin - a;
        let u = t_vec.dot(p) * inverse;
        if !(-1e-6..=1.0 + 1e-6).contains(&u) {
            continue;
        }
        let q = t_vec.cross(ab);
        let v = direction.dot(q) * inverse;
        if v < -1e-6 || u + v > 1.0 + 1e-6 {
            continue;
        }
        let distance = ac.dot(q) * inverse;
        if distance < 0.0 || distance > reach || best.as_ref().is_some_and(|hit| hit.distance <= distance) {
            continue;
        }
        let normal = mesh.mesh.faces.get(*face as usize).map_or(Vec3::Z, |face| mesh.mesh.face_normal(face));
        best = Some(Hit { distance, normal, vertices: *vertices, weights: [1.0 - u - v, u, v] });
    }
    best
}

/// The closest place on the target's points, edges, or faces whose group is the one wanted,
/// for each source position.
fn proximity(target: &Geometry, group: &Field, element: Element, sources: &[Vec3], wanted: &[i32]) -> Vec<Option<Vec3>> {
    // Candidates: (group, kind of shape, its corners).
    let mut shapes: Vec<(i32, [Vec3; 3], u8)> = Vec::new();
    if let Some(mesh) = &target.mesh {
        let points = &mesh.mesh.positions;
        let at = |index: u32| points.get(index as usize).copied().unwrap_or(Vec3::ZERO);
        let groups_on = |domain: Domain| -> Vec<i32> {
            field::evaluate(group, &Context { component: Component::Mesh(mesh), domain }).iter().map(|value| value.int()).collect()
        };
        match element {
            Element::Points => {
                let groups = groups_on(Domain::Point);
                shapes.extend(points.iter().enumerate().map(|(index, point)| (groups.get(index).copied().unwrap_or(0), [*point; 3], 1)));
            }
            Element::Edges => {
                let groups = groups_on(Domain::Edge);
                shapes.extend(
                    mesh.mesh
                        .edges()
                        .iter()
                        .enumerate()
                        .map(|(index, [a, b])| (groups.get(index).copied().unwrap_or(0), [at(*a), at(*b), at(*b)], 2)),
                );
            }
            Element::Faces => {
                let groups = groups_on(Domain::Face);
                shapes.extend(
                    mesh.mesh
                        .triangles()
                        .iter()
                        .map(|([a, b, c], face)| (groups.get(*face as usize).copied().unwrap_or(0), [at(*a), at(*b), at(*c)], 3)),
                );
            }
        }
    }
    if element == Element::Points
        && let Some(points) = &target.points
    {
        let groups: Vec<i32> = field::evaluate(group, &Context { component: Component::Points(points), domain: Domain::Point })
            .iter()
            .map(|value| value.int())
            .collect();
        shapes.extend(points.positions.iter().enumerate().map(|(index, point)| (groups.get(index).copied().unwrap_or(0), [*point; 3], 1)));
    }
    if too_many(sources.len(), shapes.len()) {
        return vec![None; sources.len()];
    }
    sources
        .iter()
        .enumerate()
        .map(|(at, source)| {
            let wanted = wanted.get(at).copied().unwrap_or(0);
            let mut best: Option<(f32, Vec3)> = None;
            for (group, [a, b, c], kind) in &shapes {
                if *group != wanted {
                    continue;
                }
                let place = match kind {
                    1 => *a,
                    2 => closest_on_segment(*source, *a, *b),
                    _ => closest_on_triangle(*source, *a, *b, *c),
                };
                let distance = place.distance_squared(*source);
                if best.is_none_or(|(smallest, _)| distance < smallest) {
                    best = Some((distance, place));
                }
            }
            best.map(|(_, place)| place)
        })
        .collect()
}
