//! Contacts between two shapes placed in the world: separating axes and face clipping for
//! polyhedra (a whole contact patch, so stacks stand), closest points for spheres. Every
//! contact's normal points from the first shape to the second; its depth is how far they
//! overlap (negative within the margin, before they touch).

use glam::{Quat, Vec3};

use crate::shape::{Poly, Shape};

/// One contact point.
#[derive(Debug, Clone, Copy)]
pub struct Contact {
    /// On the second shape's surface, in the world.
    pub point: Vec3,
    /// From the first shape to the second.
    pub normal: Vec3,
    pub depth: f32,
    /// Identifies the same contact from one step to the next (for warm starting).
    pub feature: u32,
}

/// A shape where it is: its body's position and orientation.
#[derive(Debug, Clone, Copy)]
pub struct Placed<'a> {
    pub shape: &'a Shape,
    pub position: Vec3,
    pub rotation: Quat,
}

/// A polyhedron in world coordinates.
#[derive(Debug, Clone)]
pub struct WorldPoly {
    pub vertices: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub offsets: Vec<f32>,
    pub poly: Poly,
    pub centre: Vec3,
}

impl WorldPoly {
    pub fn new(poly: &Poly, position: Vec3, rotation: Quat) -> Self {
        let vertices: Vec<Vec3> = poly.vertices.iter().map(|vertex| position + rotation * *vertex).collect();
        let normals: Vec<Vec3> = poly.faces.iter().map(|face| rotation * face.normal).collect();
        let offsets: Vec<f32> = poly
            .faces
            .iter()
            .zip(&normals)
            .map(|(face, normal)| face.corners.first().and_then(|corner| vertices.get(*corner)).map_or(0.0, |point| normal.dot(*point)))
            .collect();
        let centre = if vertices.is_empty() { position } else { vertices.iter().copied().sum::<Vec3>() / vertices.len() as f32 };
        Self { vertices, normals, offsets, poly: poly.clone(), centre }
    }

    fn vertex(&self, index: usize) -> Vec3 {
        self.vertices.get(index).copied().unwrap_or(Vec3::ZERO)
    }

    fn support(&self, direction: Vec3) -> Vec3 {
        self.vertices.iter().copied().max_by(|a, b| a.dot(direction).total_cmp(&b.dot(direction))).unwrap_or(Vec3::ZERO)
    }

    fn face_corners(&self, face: usize) -> Vec<Vec3> {
        self.poly.faces.get(face).map(|face| face.corners.iter().map(|corner| self.vertex(*corner)).collect()).unwrap_or_default()
    }
}

/// The deepest-separating face of `a` against `b`: (separation, face).
fn face_query(a: &WorldPoly, b: &WorldPoly) -> (f32, usize) {
    let mut best = (f32::NEG_INFINITY, 0);
    for (index, (normal, offset)) in a.normals.iter().zip(&a.offsets).enumerate() {
        let separation = b.support(-*normal).dot(*normal) - offset;
        if separation > best.0 {
            best = (separation, index);
        }
    }
    best
}

/// Whether two edges' arcs on the Gauss map cross (a Minkowski face): `a`, `b` the first edge's
/// face normals, `c`, `d` the second's, negated.
fn minkowski_face(a: Vec3, b: Vec3, c: Vec3, d: Vec3) -> bool {
    let bxa = b.cross(a);
    let dxc = d.cross(c);
    let cba = c.dot(bxa);
    let dba = d.dot(bxa);
    let adc = a.dot(dxc);
    let bdc = b.dot(dxc);
    cba * dba < 0.0 && adc * bdc < 0.0 && cba * bdc > 0.0
}

/// The deepest-separating edge pair: (separation, edge of a, edge of b, axis).
fn edge_query(a: &WorldPoly, b: &WorldPoly) -> (f32, usize, usize, Vec3) {
    let mut best = (f32::NEG_INFINITY, 0, 0, Vec3::Z);
    for (ia, edge_a) in a.poly.edges.iter().enumerate() {
        let (na, nb) = (a.normals.get(edge_a.left).copied().unwrap_or(Vec3::Z), a.normals.get(edge_a.right).copied().unwrap_or(Vec3::Z));
        let (pa, qa) = (a.vertex(edge_a.a), a.vertex(edge_a.b));
        for (ib, edge_b) in b.poly.edges.iter().enumerate() {
            let (nc, nd) = (b.normals.get(edge_b.left).copied().unwrap_or(Vec3::Z), b.normals.get(edge_b.right).copied().unwrap_or(Vec3::Z));
            if !minkowski_face(na, nb, -nc, -nd) {
                continue;
            }
            let (pb, qb) = (b.vertex(edge_b.a), b.vertex(edge_b.b));
            let axis = (qa - pa).cross(qb - pb);
            let length = axis.length();
            if length < 1e-6 * (qa - pa).length().max(1e-6) * (qb - pb).length().max(1e-6) {
                continue;
            }
            let mut axis = axis / length;
            if axis.dot(pa - a.centre) < 0.0 {
                axis = -axis;
            }
            let separation = axis.dot(pb - pa);
            if separation > best.0 {
                best = (separation, ia, ib, axis);
            }
        }
    }
    best
}

/// The closest points of two segments.
fn closest_segments(p1: Vec3, q1: Vec3, p2: Vec3, q2: Vec3) -> (Vec3, Vec3) {
    let (d1, d2, r) = (q1 - p1, q2 - p2, p1 - p2);
    let (a, e, f) = (d1.dot(d1), d2.dot(d2), d2.dot(r));
    let c = d1.dot(r);
    let b = d1.dot(d2);
    let denom = a * e - b * b;
    let mut s = if denom > 1e-12 { ((b * f - c * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
    let mut t = if e > 1e-12 { (b * s + f) / e } else { 0.0 };
    if t < 0.0 {
        t = 0.0;
        s = if a > 1e-12 { (-c / a).clamp(0.0, 1.0) } else { 0.0 };
    } else if t > 1.0 {
        t = 1.0;
        s = if a > 1e-12 { ((b - c) / a).clamp(0.0, 1.0) } else { 0.0 };
    }
    (p1 + d1 * s, p2 + d2 * t)
}

/// Clips a polygon to the half space `normal · p <= offset`.
fn clip(polygon: &[Vec3], normal: Vec3, offset: f32) -> Vec<Vec3> {
    let mut out = Vec::with_capacity(polygon.len() + 2);
    let count = polygon.len();
    for slot in 0..count {
        let (Some(&a), Some(&b)) = (polygon.get(slot), polygon.get((slot + 1) % count)) else { continue };
        let (da, db) = (normal.dot(a) - offset, normal.dot(b) - offset);
        if da <= 0.0 {
            out.push(a);
        }
        if (da <= 0.0) != (db <= 0.0) {
            let t = da / (da - db);
            out.push(a + (b - a) * t);
        }
    }
    out
}

/// Keeps at most four points: the deepest, then the ones spreading the patch most.
fn reduce(mut points: Vec<(Vec3, f32)>) -> Vec<(Vec3, f32)> {
    if points.len() <= 4 {
        return points;
    }
    points.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut kept = vec![points.remove(0)];
    while kept.len() < 4 && !points.is_empty() {
        let farthest = points
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| {
                let spread = |point: &Vec3| kept.iter().map(|(other, _)| point.distance_squared(*other)).fold(f32::INFINITY, f32::min);
                spread(&a.0).total_cmp(&spread(&b.0))
            })
            .map(|(slot, _)| slot);
        match farthest {
            Some(slot) => kept.push(points.remove(slot)),
            None => break,
        }
    }
    kept
}

/// Contacts between two polyhedra within `margin`.
pub fn poly_poly(a: &WorldPoly, b: &WorldPoly, margin: f32) -> Vec<Contact> {
    let (sep_a, face_a) = face_query(a, b);
    if sep_a > margin {
        return Vec::new();
    }
    let (sep_b, face_b) = face_query(b, a);
    if sep_b > margin {
        return Vec::new();
    }
    let (sep_e, edge_a, edge_b, axis) = edge_query(a, b);
    if sep_e > margin {
        return Vec::new();
    }
    // Faces win unless the edges separate clearly more (keeps resting contacts on faces).
    let face_best = sep_a.max(sep_b);
    if sep_e > face_best + 0.005 {
        let (Some(ea), Some(eb)) = (a.poly.edges.get(edge_a), b.poly.edges.get(edge_b)) else { return Vec::new() };
        let (_pa, pb) = closest_segments(a.vertex(ea.a), a.vertex(ea.b), b.vertex(eb.a), b.vertex(eb.b));
        let feature = 0x8000_0000 | ((edge_a as u32 & 0x7fff) << 15) | (edge_b as u32 & 0x7fff);
        return vec![Contact { point: pb, normal: axis, depth: -sep_e, feature }];
    }
    // Reference face on whichever polyhedron separates more (bias to the first).
    let (reference, incident, face, flip) = if sep_b > sep_a + 1e-4 { (b, a, face_b, true) } else { (a, b, face_a, false) };
    let normal = reference.normals.get(face).copied().unwrap_or(Vec3::Z);
    let offset = reference.offsets.get(face).copied().unwrap_or(0.0);
    // The incident face: the most anti-parallel one.
    let incident_face =
        incident.normals.iter().enumerate().min_by(|(_, x), (_, y)| x.dot(normal).total_cmp(&y.dot(normal))).map_or(0, |(slot, _)| slot);
    let mut polygon = incident.face_corners(incident_face);
    let corners = reference.face_corners(face);
    let count = corners.len();
    for slot in 0..count {
        let (Some(&p), Some(&q)) = (corners.get(slot), corners.get((slot + 1) % count)) else { continue };
        let side = (q - p).cross(normal).normalize_or_zero();
        polygon = clip(&polygon, side, side.dot(p));
        if polygon.is_empty() {
            break;
        }
    }
    let mut points: Vec<(Vec3, f32)> = Vec::new();
    for point in polygon {
        let distance = normal.dot(point) - offset;
        if distance <= margin {
            points.push((point, -distance));
        }
    }
    let base =
        (face as u32).wrapping_mul(1_000_003).wrapping_add((incident_face as u32).wrapping_mul(1009)).wrapping_add(u32::from(flip)).wrapping_mul(8)
            & 0x7fff_ffff;
    reduce(points)
        .into_iter()
        .enumerate()
        .map(|(slot, (point, depth))| {
            // Points on the incident shape; the normal points from a to b either way.
            let (point, contact_normal) = if flip { (point + normal * depth, -normal) } else { (point, normal) };
            Contact { point, normal: contact_normal, depth, feature: base.wrapping_add(slot as u32) & 0x7fff_ffff }
        })
        .collect()
}

/// The closest point on a convex polyhedron's surface to `point`, and whether `point` is inside.
fn closest_on_poly(poly: &WorldPoly, point: Vec3) -> (Vec3, Vec3, bool) {
    // Inside: the least-penetrated face.
    let mut most = (f32::NEG_INFINITY, 0usize);
    for (index, (normal, offset)) in poly.normals.iter().zip(&poly.offsets).enumerate() {
        let distance = normal.dot(point) - offset;
        if distance > most.0 {
            most = (distance, index);
        }
    }
    if most.0 <= 0.0 {
        let normal = poly.normals.get(most.1).copied().unwrap_or(Vec3::Z);
        return (point - normal * most.0, normal, true);
    }
    // Outside: the nearest point over every face polygon.
    let mut best = (f32::INFINITY, point, Vec3::Z);
    for (index, normal) in poly.normals.iter().enumerate() {
        let corners = poly.face_corners(index);
        let offset = poly.offsets.get(index).copied().unwrap_or(0.0);
        let projected = point - *normal * (normal.dot(point) - offset);
        let count = corners.len();
        let inside = (0..count).all(|slot| {
            let (Some(&p), Some(&q)) = (corners.get(slot), corners.get((slot + 1) % count)) else { return true };
            (q - p).cross(projected - p).dot(*normal) >= 0.0
        });
        let candidate = if inside {
            projected
        } else {
            (0..count)
                .filter_map(|slot| {
                    let (p, q) = (*corners.get(slot)?, *corners.get((slot + 1) % count)?);
                    let edge = q - p;
                    let t = if edge.length_squared() > 0.0 { ((point - p).dot(edge) / edge.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
                    Some(p + edge * t)
                })
                .min_by(|x, y| x.distance_squared(point).total_cmp(&y.distance_squared(point)))
                .unwrap_or(projected)
        };
        let distance = candidate.distance_squared(point);
        if distance < best.0 {
            best = (distance, candidate, (point - candidate).normalize_or(*normal));
        }
    }
    (best.1, best.2, false)
}

/// The closest point on a triangle to `point`.
pub fn closest_on_triangle(point: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let (ab, ac, ap) = (b - a, c - a, point - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = point - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = point - c;
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
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

/// Contacts between a sphere (centre, radius) as the second shape and a polyhedron as the first.
fn poly_sphere(poly: &WorldPoly, centre: Vec3, radius: f32, margin: f32) -> Vec<Contact> {
    let (surface, normal, inside) = closest_on_poly(poly, centre);
    let distance = if inside { -surface.distance(centre) } else { surface.distance(centre) };
    let depth = radius - distance;
    if depth < -margin {
        return Vec::new();
    }
    vec![Contact { point: centre - normal * radius, normal, depth, feature: 1 }]
}

/// Contacts between two placed shapes within `margin` (the first must not be a mesh unless the
/// second is not either; meshes are handled triangle by triangle by the caller).
pub fn contacts(a: Placed<'_>, b: Placed<'_>, margin: f32) -> Vec<Contact> {
    match (a.shape, b.shape) {
        (Shape::Sphere { radius: ra }, Shape::Sphere { radius: rb }) => {
            let offset = b.position - a.position;
            let distance = offset.length();
            let depth = ra + rb - distance;
            if depth < -margin {
                return Vec::new();
            }
            let normal = if distance > 1e-9 { offset / distance } else { Vec3::Z };
            vec![Contact { point: b.position - normal * *rb, normal, depth, feature: 0 }]
        }
        (Shape::Poly(poly), Shape::Sphere { radius }) => poly_sphere(&WorldPoly::new(poly, a.position, a.rotation), b.position, *radius, margin),
        (Shape::Sphere { radius }, Shape::Poly(poly)) => {
            let mut found = poly_sphere(&WorldPoly::new(poly, b.position, b.rotation), a.position, *radius, margin);
            for contact in &mut found {
                // The roles swap: the point moves to the polyhedron's surface, the normal turns.
                contact.point += contact.normal * contact.depth;
                contact.normal = -contact.normal;
            }
            found
        }
        (Shape::Poly(pa), Shape::Poly(pb)) => {
            poly_poly(&WorldPoly::new(pa, a.position, a.rotation), &WorldPoly::new(pb, b.position, b.rotation), margin)
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(half: f32) -> Shape {
        Shape::Poly(Poly::cuboid(Vec3::ZERO, Vec3::splat(half)).unwrap())
    }

    #[test]
    fn a_box_resting_on_a_box_touches_at_four_corners() {
        let ground = Shape::Poly(Poly::cuboid(Vec3::ZERO, Vec3::new(5.0, 5.0, 0.5)).unwrap());
        let block = cube(0.5);
        let found = contacts(
            Placed { shape: &ground, position: Vec3::new(0.0, 0.0, -0.5), rotation: Quat::IDENTITY },
            Placed { shape: &block, position: Vec3::new(0.0, 0.0, 0.49), rotation: Quat::IDENTITY },
            0.04,
        );
        assert_eq!(found.len(), 4, "{found:?}");
        for contact in found {
            assert!(contact.normal.abs_diff_eq(Vec3::Z, 1e-5), "{contact:?}");
            assert!((contact.depth - 0.01).abs() < 1e-4, "{contact:?}");
            assert!(contact.point.z.abs() < 0.02);
        }
    }

    #[test]
    fn apart_boxes_have_no_contacts_and_edges_meet_at_one_point() {
        let block = cube(0.5);
        assert!(
            contacts(
                Placed { shape: &block, position: Vec3::ZERO, rotation: Quat::IDENTITY },
                Placed { shape: &block, position: Vec3::new(2.0, 0.0, 0.0), rotation: Quat::IDENTITY },
                0.04
            )
            .is_empty()
        );
        // A cube turned 45° about two axes so an edge pokes into the other's edge.
        let turned = Quat::from_rotation_z(std::f32::consts::FRAC_PI_4) * Quat::from_rotation_x(std::f32::consts::FRAC_PI_4);
        let found = contacts(
            Placed { shape: &block, position: Vec3::ZERO, rotation: Quat::from_rotation_x(std::f32::consts::FRAC_PI_4) },
            Placed { shape: &block, position: Vec3::new(0.0, 0.0, 1.38), rotation: turned },
            0.04,
        );
        assert!(!found.is_empty());
        assert!(found.iter().all(|contact| contact.normal.z > 0.5));
    }

    #[test]
    fn a_sphere_on_a_box_touches_once_from_the_box_up() {
        let ground = cube(1.0);
        let ball = Shape::Sphere { radius: 0.5 };
        let found = contacts(
            Placed { shape: &ground, position: Vec3::ZERO, rotation: Quat::IDENTITY },
            Placed { shape: &ball, position: Vec3::new(0.2, 0.1, 1.45), rotation: Quat::IDENTITY },
            0.04,
        );
        assert_eq!(found.len(), 1);
        assert!(found[0].normal.abs_diff_eq(Vec3::Z, 1e-5));
        assert!((found[0].depth - 0.05).abs() < 1e-4);
        let swapped = contacts(
            Placed { shape: &ball, position: Vec3::new(0.2, 0.1, 1.45), rotation: Quat::IDENTITY },
            Placed { shape: &ground, position: Vec3::ZERO, rotation: Quat::IDENTITY },
            0.04,
        );
        assert!(swapped[0].normal.abs_diff_eq(-Vec3::Z, 1e-5));
        assert!((swapped[0].depth - 0.05).abs() < 1e-4);
    }
}
