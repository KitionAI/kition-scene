//! Filling closed 2D outlines with triangles: contours nest (an outline inside an outline is a
//! hole, one inside a hole is an island again, as with the counters of letters and 2D curves),
//! each hole is bridged to the outline around it, and the resulting polygons are cut into
//! triangles by ear clipping. Used by 2D curves and by text.

use glam::Vec2;

/// Twice the signed area: positive for counter-clockwise contours.
pub fn signed_area(points: &[Vec2]) -> f32 {
    let mut sum = 0.0;
    for (index, a) in points.iter().enumerate() {
        let b = points.get((index + 1) % points.len()).copied().unwrap_or(*a);
        sum += a.x * b.y - b.x * a.y;
    }
    sum
}

/// Whether `point` is inside the closed contour (even-odd).
pub fn contains(contour: &[Vec2], point: Vec2) -> bool {
    let mut inside = false;
    let count = contour.len();
    for index in 0..count {
        let (Some(a), Some(b)) = (contour.get(index), contour.get((index + count - 1) % count)) else { continue };
        if (a.y > point.y) != (b.y > point.y) {
            let x = a.x + (point.y - a.y) / (b.y - a.y) * (b.x - a.x);
            if point.x < x {
                inside = !inside;
            }
        }
    }
    inside
}

/// Triangles filling the contours, as indices into the contours' points taken in order (the
/// first contour's points first). Every triangle is counter-clockwise.
pub fn fill(contours: &[Vec<Vec2>]) -> Vec<[u32; 3]> {
    let mut offsets = Vec::with_capacity(contours.len());
    let mut next = 0u32;
    for contour in contours {
        offsets.push(next);
        next = next.saturating_add(u32::try_from(contour.len()).unwrap_or(u32::MAX));
    }
    let usable: Vec<usize> = (0..contours.len())
        .filter(|index| contours.get(*index).is_some_and(|contour| contour.len() >= 3 && signed_area(contour).abs() > 1e-12))
        .collect();
    // Nesting depth: how many other contours hold a point of this one.
    let depth = |index: usize| -> usize {
        let Some(probe) = contours.get(index).and_then(|contour| contour.first()).copied() else { return 0 };
        usable.iter().filter(|other| **other != index).filter(|other| contours.get(**other).is_some_and(|contour| contains(contour, probe))).count()
    };
    let depths: Vec<(usize, usize)> = usable.iter().map(|index| (*index, depth(*index))).collect();
    let mut triangles = Vec::new();
    for (outer, outer_depth) in depths.iter().filter(|(_, depth)| depth % 2 == 0) {
        let Some(outer_points) = contours.get(*outer) else { continue };
        // Holes directly inside this outline.
        let holes: Vec<usize> = depths
            .iter()
            .filter(|(hole, depth)| {
                *depth == outer_depth + 1
                    && contours.get(*hole).and_then(|contour| contour.first()).is_some_and(|probe| contains(outer_points, *probe))
            })
            .map(|(hole, _)| *hole)
            .collect();
        // A ring of (point, global index), counter-clockwise.
        let ring = |index: usize, counter_clockwise: bool| -> Vec<(Vec2, u32)> {
            let Some(contour) = contours.get(index) else { return Vec::new() };
            let base = offsets.get(index).copied().unwrap_or(0);
            let mut ring: Vec<(Vec2, u32)> =
                contour.iter().enumerate().map(|(slot, point)| (*point, base + u32::try_from(slot).unwrap_or(0))).collect();
            if (signed_area(contour) > 0.0) != counter_clockwise {
                ring.reverse();
            }
            ring
        };
        let mut polygon = ring(*outer, true);
        let mut hole_rings: Vec<Vec<(Vec2, u32)>> = holes.iter().map(|hole| ring(*hole, false)).collect();
        // Bridge the holes from the rightmost one in.
        hole_rings.sort_by(|a, b| max_x(b).total_cmp(&max_x(a)));
        for hole in hole_rings {
            polygon = bridge(polygon, hole);
        }
        triangles.extend(ear_clip(&polygon));
    }
    triangles
}

fn max_x(ring: &[(Vec2, u32)]) -> f32 {
    ring.iter().map(|(point, _)| point.x).fold(f32::NEG_INFINITY, f32::max)
}

/// Joins a clockwise hole to the counter-clockwise polygon around it with a bridge between two
/// points that see each other: the nearest pair that crosses no edge, runs inside the shape,
/// and lines up with no neighbouring edge (which would leave zero-area corners to fill).
fn bridge(polygon: Vec<(Vec2, u32)>, hole: Vec<(Vec2, u32)>) -> Vec<(Vec2, u32)> {
    if hole.is_empty() {
        return polygon;
    }
    let edges: Vec<(Vec2, Vec2)> = ring_edges(&polygon).chain(ring_edges(&hole)).collect();
    let outline: Vec<Vec2> = polygon.iter().map(|(point, _)| *point).collect();
    let counter: Vec<Vec2> = hole.iter().map(|(point, _)| *point).collect();
    let scale = outline.iter().chain(&counter).fold(0.0f32, |most, point| most.max(point.x.abs()).max(point.y.abs())).max(1e-6);
    let mut candidates: Vec<(f32, usize, usize)> = Vec::new();
    for (hole_slot, (m, _)) in hole.iter().enumerate() {
        for (slot, (p, _)) in polygon.iter().enumerate() {
            candidates.push((m.distance_squared(*p), hole_slot, slot));
        }
    }
    candidates.sort_by(|x, y| x.0.total_cmp(&y.0));
    let neighbours = |ring: &[(Vec2, u32)], slot: usize| -> (Vec2, Vec2) {
        let count = ring.len();
        (ring.get((slot + count - 1) % count).map_or(Vec2::ZERO, |entry| entry.0), ring.get((slot + 1) % count).map_or(Vec2::ZERO, |entry| entry.0))
    };
    let lined_up = |a: Vec2, b: Vec2, c: Vec2| {
        (cross(a, b, c)).abs() <= 1e-6 * (b - a).length().max(1e-12) * (c - b).length().max(1e-12) + 1e-12 * scale * scale
    };
    for (_, hole_slot, slot) in candidates {
        let (Some((m, _)), Some((p, _))) = (hole.get(hole_slot), polygon.get(slot)) else { continue };
        let (m, p) = (*m, *p);
        if m.distance(p) <= 1e-9 * scale {
            continue;
        }
        let (mp, mn) = neighbours(&hole, hole_slot);
        let (pp, pn) = neighbours(&polygon, slot);
        if lined_up(m, p, pp) || lined_up(m, p, pn) || lined_up(p, m, mp) || lined_up(p, m, mn) {
            continue;
        }
        let middle = (m + p) * 0.5;
        if !contains(&outline, middle) || contains(&counter, middle) {
            continue;
        }
        let blocked = edges.iter().any(|(a, b)| {
            if *a == m || *b == m || *a == p || *b == p {
                return false;
            }
            segments_touch(m, p, *a, *b)
        });
        if blocked {
            continue;
        }
        let mut out = Vec::with_capacity(polygon.len() + hole.len() + 2);
        out.extend(polygon.iter().take(slot + 1).copied());
        for offset in 0..=hole.len() {
            if let Some(entry) = hole.get((hole_slot + offset) % hole.len()) {
                out.push(*entry);
            }
        }
        if let Some(entry) = polygon.get(slot) {
            out.push(*entry);
        }
        out.extend(polygon.iter().skip(slot + 1).copied());
        return out;
    }
    // Nothing sees cleanly (degenerate input): bridge from the hole's first point to the nearest.
    let Some((m, _)) = hole.first().copied() else { return polygon };
    let Some(slot) = (0..polygon.len()).min_by(|x, y| {
        let distance = |slot: &usize| polygon.get(*slot).map_or(f32::INFINITY, |entry| entry.0.distance_squared(m));
        distance(x).total_cmp(&distance(y))
    }) else {
        return polygon;
    };
    let mut out = Vec::with_capacity(polygon.len() + hole.len() + 2);
    out.extend(polygon.iter().take(slot + 1).copied());
    out.extend(hole.iter().copied());
    if let Some(entry) = hole.first() {
        out.push(*entry);
    }
    if let Some(entry) = polygon.get(slot) {
        out.push(*entry);
    }
    out.extend(polygon.iter().skip(slot + 1).copied());
    out
}

fn ring_edges(ring: &[(Vec2, u32)]) -> impl Iterator<Item = (Vec2, Vec2)> + '_ {
    let count = ring.len();
    (0..count).filter_map(move |slot| Some((ring.get(slot)?.0, ring.get((slot + 1) % count)?.0)))
}

/// Whether segments a–b and c–d meet anywhere (touching counts).
fn segments_touch(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    let (d1, d2, d3, d4) = (cross(c, d, a), cross(c, d, b), cross(a, b, c), cross(a, b, d));
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0)) && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0)) {
        return true;
    }
    let on = |p: Vec2, q: Vec2, r: Vec2, turn: f32| {
        turn.abs() <= 1e-9 && r.x >= p.x.min(q.x) && r.x <= p.x.max(q.x) && r.y >= p.y.min(q.y) && r.y <= p.y.max(q.y)
    };
    on(c, d, a, d1) || on(c, d, b, d2) || on(a, b, c, d3) || on(a, b, d, d4)
}

fn cross(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

/// A corner that turns left by more than rounding (relative to its edges' lengths).
fn convex(a: Vec2, b: Vec2, c: Vec2) -> bool {
    cross(a, b, c) > 1e-6 * (b - a).length() * (c - b).length()
}

fn inside_triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> bool {
    let (d1, d2, d3) = (cross(a, b, p), cross(b, c, p), cross(c, a, p));
    let negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(negative && positive)
}

/// Ear clipping of a counter-clockwise polygon (which may touch itself along bridges).
fn ear_clip(polygon: &[(Vec2, u32)]) -> Vec<[u32; 3]> {
    let mut remaining: Vec<(Vec2, u32)> = polygon.to_vec();
    // Points that repeat consecutively add nothing.
    remaining.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
    let mut out = Vec::with_capacity(remaining.len().saturating_sub(2));
    let mut guard = 0usize;
    let limit = remaining.len().saturating_mul(remaining.len()).saturating_add(16);
    while remaining.len() > 3 && guard < limit {
        guard += 1;
        let count = remaining.len();
        let mut clipped = false;
        for slot in 0..count {
            let (Some(&(a, ia)), Some(&(b, ib)), Some(&(c, ic))) =
                (remaining.get((slot + count - 1) % count), remaining.get(slot), remaining.get((slot + 1) % count))
            else {
                continue;
            };
            if !convex(a, b, c) {
                continue;
            }
            let blocked = remaining.iter().any(|&(p, ip)| {
                if ip == ia || ip == ib || ip == ic || p == a || p == b || p == c {
                    return false;
                }
                inside_triangle(p, a, b, c)
            });
            if blocked {
                continue;
            }
            out.push([ia, ib, ic]);
            remaining.remove(slot);
            clipped = true;
            break;
        }
        if !clipped {
            // Only straight corners are left to remove: one goes, as a sliver, so the fill
            // keeps every outline point.
            let count = remaining.len();
            let straight = (0..count).find(|slot| {
                let (Some(a), Some(b), Some(c)) =
                    (remaining.get((slot + count - 1) % count), remaining.get(*slot), remaining.get((slot + 1) % count))
                else {
                    return false;
                };
                !convex(a.0, b.0, c.0) && cross(a.0, b.0, c.0) > -1e-6 * (b.0 - a.0).length() * (c.0 - b.0).length()
            });
            if let Some(slot) = straight {
                if let (Some(&(_, ia)), Some(&(_, ib)), Some(&(_, ic))) =
                    (remaining.get((slot + count - 1) % count), remaining.get(slot), remaining.get((slot + 1) % count))
                {
                    out.push([ia, ib, ic]);
                }
                remaining.remove(slot);
                clipped = true;
            }
        }
        if !clipped {
            // Nothing is an ear (rounding): take the most convex corner.
            let count = remaining.len();
            let best = (0..count).max_by(|x, y| {
                let turn = |slot: usize| {
                    let (Some(a), Some(b), Some(c)) =
                        (remaining.get((slot + count - 1) % count), remaining.get(slot), remaining.get((slot + 1) % count))
                    else {
                        return f32::NEG_INFINITY;
                    };
                    cross(a.0, b.0, c.0)
                };
                turn(*x).total_cmp(&turn(*y))
            });
            let Some(slot) = best else { break };
            if let (Some(&(_, ia)), Some(&(_, ib)), Some(&(_, ic))) =
                (remaining.get((slot + count - 1) % count), remaining.get(slot), remaining.get((slot + 1) % count))
            {
                out.push([ia, ib, ic]);
            }
            remaining.remove(slot);
        }
    }
    if let [(a, ia), (b, ib), (c, ic)] = remaining.as_slice()
        && convex(*a, *b, *c)
    {
        out.push([*ia, *ib, *ic]);
    }
    out.retain(|[a, b, c]| a != b && b != c && a != c);
    out
}

/// The contour without points that lie on a straight run between their neighbours (they would
/// only make slivers of triangles).
pub fn drop_straight(contour: &[Vec2]) -> Vec<Vec2> {
    let mut points: Vec<Vec2> = contour.to_vec();
    let mut changed = true;
    while changed && points.len() > 3 {
        changed = false;
        let count = points.len();
        for slot in 0..count {
            let (Some(a), Some(b), Some(c)) = (points.get((slot + count - 1) % count), points.get(slot), points.get((slot + 1) % count)) else {
                continue;
            };
            let (ab, bc) = (*b - *a, *c - *b);
            let scale = ab.length() * bc.length();
            if ab.length() < 1e-9 || (scale > 0.0 && (ab.x * bc.y - ab.y * bc.x).abs() <= 1e-6 * scale && ab.dot(bc) > 0.0) {
                points.remove(slot);
                changed = true;
                break;
            }
        }
    }
    points
}

/// Whether the contours touch or cross themselves or each other: edges crossing, a corner lying
/// on another edge, or edges running along each other.
pub fn crosses(contours: &[Vec<Vec2>]) -> bool {
    let edges = edges_of(contours);
    let size = extent(contours);
    for (index, (a, b, _)) in edges.iter().enumerate() {
        for (c, d, _) in edges.iter().skip(index + 1) {
            if !splits(*a, *b, *c, *d, size).is_empty() || !splits(*c, *d, *a, *b, size).is_empty() {
                return true;
            }
        }
    }
    false
}

fn extent(contours: &[Vec<Vec2>]) -> f64 {
    let span = contours
        .iter()
        .flatten()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), point| (low.min(point.x).min(point.y), high.max(point.x).max(point.y)));
    f64::from((span.1 - span.0).max(1e-6))
}

/// Where edge a–b must be cut for edge c–d: where they cross, where c or d lies on a–b, as
/// parameters along a–b strictly inside it, with the point there.
fn splits(a: glam::DVec2, b: glam::DVec2, c: glam::DVec2, d: glam::DVec2, size: f64) -> Vec<(f64, glam::DVec2)> {
    let tolerance = size * 1e-7;
    let r = b - a;
    let length = r.length();
    if length <= tolerance {
        return Vec::new();
    }
    let along = |point: glam::DVec2| -> Option<f64> {
        let t = (point - a).dot(r) / (length * length);
        let off = (point - (a + r * t)).length();
        (off <= tolerance && t > tolerance / length && t < 1.0 - tolerance / length).then_some(t)
    };
    let mut out = Vec::new();
    for point in [c, d] {
        if let Some(t) = along(point) {
            out.push((t, point));
        }
    }
    if out.is_empty()
        && let Some((t, u)) = segment_hit(a, b, c, d)
        && t > 1e-9
        && t < 1.0 - 1e-9
        && u > 1e-9
        && u < 1.0 - 1e-9
    {
        out.push((t, a + r * t));
    }
    out
}

fn edges_of(contours: &[Vec<Vec2>]) -> Vec<(glam::DVec2, glam::DVec2, usize)> {
    let mut out = Vec::new();
    for (index, contour) in contours.iter().enumerate() {
        let count = contour.len();
        for slot in 0..count {
            let (Some(a), Some(b)) = (contour.get(slot), contour.get((slot + 1) % count)) else { continue };
            if a != b {
                out.push((a.as_dvec2(), b.as_dvec2(), index));
            }
        }
    }
    out
}

/// Where segments a–b and c–d cross, as parameters along each.
fn segment_hit(a: glam::DVec2, b: glam::DVec2, c: glam::DVec2, d: glam::DVec2) -> Option<(f64, f64)> {
    let r = b - a;
    let q = d - c;
    let denominator = r.x * q.y - r.y * q.x;
    if denominator.abs() < 1e-18 {
        return None;
    }
    let w = c - a;
    let t = (w.x * q.y - w.y * q.x) / denominator;
    let u = (w.x * r.y - w.y * r.x) / denominator;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some((t, u))
}

/// The boundary of the region the contours cover under the non-zero rule: crossing edges are
/// split where they cross, and only the pieces with the covered side on one hand and the
/// uncovered side on the other are kept, chained into loops with the covered side on their
/// left (outlines counter-clockwise, holes clockwise). Overlapping strokes of variable fonts
/// become one clean outline.
pub fn resolve(contours: &[Vec<Vec2>]) -> Vec<Vec<Vec2>> {
    use glam::DVec2;
    let edges = edges_of(contours);
    let size = extent(contours);
    // Points by identity: every crossing is one point for both edges.
    let mut points: Vec<DVec2> = Vec::new();
    let point_of = |point: DVec2, points: &mut Vec<DVec2>| -> usize {
        if let Some(found) = points.iter().position(|other| other.distance(point) <= size * 1e-7) {
            return found;
        }
        points.push(point);
        points.len() - 1
    };
    let mut cuts: Vec<Vec<(f64, usize)>> =
        edges.iter().map(|(a, b, _)| vec![(0.0, point_of(*a, &mut points)), (1.0, point_of(*b, &mut points))]).collect();
    for first in 0..edges.len() {
        for second in 0..edges.len() {
            if first == second {
                continue;
            }
            let (Some((a, b, _)), Some((c, d, _))) = (edges.get(first), edges.get(second)) else { continue };
            for (t, point) in splits(*a, *b, *c, *d, size) {
                let id = point_of(point, &mut points);
                if let Some(list) = cuts.get_mut(first) {
                    list.push((t, id));
                }
            }
        }
    }
    // Winding number of a point against every original edge.
    let winding = |point: DVec2| -> i32 {
        let mut count = 0;
        for (a, b, _) in &edges {
            if a.y <= point.y {
                if b.y > point.y && (b.x - a.x) * (point.y - a.y) - (point.x - a.x) * (b.y - a.y) > 0.0 {
                    count += 1;
                }
            } else if b.y <= point.y && (b.x - a.x) * (point.y - a.y) - (point.x - a.x) * (b.y - a.y) < 0.0 {
                count -= 1;
            }
        }
        count
    };
    let mut kept: Vec<(usize, usize)> = Vec::new();
    for list in &mut cuts {
        list.sort_by(|x, y| x.0.total_cmp(&y.0));
        list.dedup_by(|x, y| x.1 == y.1);
        for pair in list.windows(2) {
            let [(_, from), (_, to)] = pair else { continue };
            if from == to {
                continue;
            }
            let (Some(a), Some(b)) = (points.get(*from), points.get(*to)) else { continue };
            let middle = (*a + *b) * 0.5;
            let direction = (*b - *a).normalize_or_zero();
            let left = DVec2::new(-direction.y, direction.x) * size * 1e-6;
            let (covered_left, covered_right) = (winding(middle + left) != 0, winding(middle - left) != 0);
            if covered_left && !covered_right {
                kept.push((*from, *to));
            } else if covered_right && !covered_left {
                kept.push((*to, *from));
            }
        }
    }
    kept.sort_unstable();
    kept.dedup();
    // Chain into loops, turning as far right as possible where loops touch.
    let mut used = vec![false; kept.len()];
    let mut loops = Vec::new();
    for start in 0..kept.len() {
        if used.get(start).copied().unwrap_or(true) {
            continue;
        }
        let mut chain: Vec<usize> = Vec::new();
        let mut current = start;
        let guard = kept.len() + 1;
        for _ in 0..guard {
            if let Some(flag) = used.get_mut(current) {
                *flag = true;
            }
            let Some(&(from, to)) = kept.get(current) else { break };
            chain.push(from);
            let Some(start_from) = kept.get(start).map(|edge| edge.0) else { break };
            if to == start_from {
                break;
            }
            let (Some(a), Some(b)) = (points.get(from), points.get(to)) else { break };
            let incoming = (*b - *a).normalize_or_zero();
            let next = kept
                .iter()
                .enumerate()
                .filter(|(index, edge)| edge.0 == to && !used.get(*index).copied().unwrap_or(true))
                .map(|(index, edge)| {
                    let out = points.get(edge.1).map_or(DVec2::ZERO, |end| (*end - *b).normalize_or_zero());
                    // Signed turn: negative is right.
                    let turn = (incoming.x * out.y - incoming.y * out.x).atan2(incoming.dot(out));
                    (index, turn)
                })
                .min_by(|x, y| x.1.total_cmp(&y.1));
            let Some((index, _)) = next else { break };
            current = index;
        }
        if chain.len() >= 3 {
            loops.push(chain.iter().filter_map(|index| points.get(*index)).map(|point| point.as_vec2()).collect());
        }
    }
    loops
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(size: f32, clockwise: bool) -> Vec<Vec2> {
        let mut points = vec![Vec2::new(-size, -size), Vec2::new(size, -size), Vec2::new(size, size), Vec2::new(-size, size)];
        if clockwise {
            points.reverse();
        }
        points
    }

    fn area(contours: &[Vec<Vec2>], triangles: &[[u32; 3]]) -> f32 {
        let points: Vec<Vec2> = contours.iter().flatten().copied().collect();
        triangles.iter().map(|[a, b, c]| cross(points[*a as usize], points[*b as usize], points[*c as usize]) / 2.0).sum()
    }

    #[test]
    fn a_square_with_a_hole_fills_the_ring_between() {
        // Either winding works for either contour: nesting decides.
        for (outer, inner) in [(false, false), (true, true), (false, true), (true, false)] {
            let contours = vec![square(2.0, outer), square(1.0, inner)];
            let triangles = fill(&contours);
            assert_eq!(triangles.len(), 8, "n + 2h - 2 triangles");
            assert!((area(&contours, &triangles) - 12.0).abs() < 1e-4);
        }
    }

    #[test]
    fn an_island_inside_a_hole_is_filled_again() {
        let contours = vec![square(3.0, false), square(2.0, true), square(1.0, false)];
        let triangles = fill(&contours);
        assert!((area(&contours, &triangles) - (36.0 - 16.0 + 4.0)).abs() < 1e-4);
    }

    #[test]
    fn overlapping_strokes_resolve_to_one_outline() {
        // Two overlapping squares (a plus-like overlap): one outline, no crossings left.
        let a = vec![Vec2::new(0.0, 0.0), Vec2::new(2.0, 0.0), Vec2::new(2.0, 2.0), Vec2::new(0.0, 2.0)];
        let b = vec![Vec2::new(1.0, 1.0), Vec2::new(3.0, 1.0), Vec2::new(3.0, 3.0), Vec2::new(1.0, 3.0)];
        let contours = vec![a, b];
        assert!(crosses(&contours));
        let resolved = resolve(&contours);
        assert_eq!(resolved.len(), 1);
        assert!(!crosses(&resolved));
        assert!((signed_area(&resolved[0]) / 2.0 - 7.0).abs() < 1e-4);
        let triangles = fill(&resolved);
        assert!((area(&resolved, &triangles) - 7.0).abs() < 1e-4);
    }

    #[test]
    fn a_circle_fills_with_n_minus_two_triangles() {
        let circle: Vec<Vec2> = (0..48)
            .map(|k| {
                let a = std::f32::consts::TAU * k as f32 / 48.0;
                Vec2::new(a.cos(), a.sin())
            })
            .collect();
        let triangles = fill(&[circle]);
        assert_eq!(triangles.len(), 46);
    }
}
