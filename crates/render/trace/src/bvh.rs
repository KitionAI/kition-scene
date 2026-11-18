//! A bounding volume hierarchy over triangles, built with binned surface-area heuristics and
//! walked without recursion.

use glam::Vec3;

/// One triangle as the intersection test wants it.
#[derive(Debug, Clone, Copy)]
pub struct Triangle {
    pub a: Vec3,
    pub edge1: Vec3,
    pub edge2: Vec3,
}

impl Triangle {
    pub fn new(a: Vec3, b: Vec3, c: Vec3) -> Self {
        Self { a, edge1: b - a, edge2: c - a }
    }

    fn bounds(&self) -> (Vec3, Vec3) {
        let (b, c) = (self.a + self.edge1, self.a + self.edge2);
        (self.a.min(b).min(c), self.a.max(b).max(c))
    }

    fn centroid(&self) -> Vec3 {
        self.a + (self.edge1 + self.edge2) / 3.0
    }

    /// Möller–Trumbore: the distance and barycentrics (u for b, v for c) of a hit in
    /// (`near`, `far`).
    pub fn intersect(&self, origin: Vec3, direction: Vec3, near: f32, far: f32) -> Option<(f32, f32, f32)> {
        let p = direction.cross(self.edge2);
        let determinant = self.edge1.dot(p);
        if determinant.abs() < 1e-12 {
            return None;
        }
        let inverse = 1.0 / determinant;
        let s = origin - self.a;
        let u = s.dot(p) * inverse;
        if !(0.0..=1.0).contains(&u) {
            return None;
        }
        let q = s.cross(self.edge1);
        let v = direction.dot(q) * inverse;
        if v < 0.0 || u + v > 1.0 {
            return None;
        }
        let t = self.edge2.dot(q) * inverse;
        (t > near && t < far).then_some((t, u, v))
    }
}

#[derive(Debug, Clone, Copy)]
struct Node {
    min: Vec3,
    max: Vec3,
    /// The first child (inner nodes, the second is `first + 1`) or the first triangle (leaves).
    first: u32,
    /// Triangles in a leaf; zero for inner nodes.
    count: u32,
}

/// A hit: distance, the triangle (its index in the original order), and barycentrics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub distance: f32,
    pub triangle: u32,
    pub u: f32,
    pub v: f32,
}

/// The hierarchy, owning its triangles in leaf order.
#[derive(Debug, Clone, Default)]
pub struct Bvh {
    nodes: Vec<Node>,
    triangles: Vec<Triangle>,
    /// The original index of each triangle in leaf order.
    order: Vec<u32>,
}

const BINS: usize = 12;
const LEAF_SIZE: usize = 4;
const STACK: usize = 64;

fn area(min: Vec3, max: Vec3) -> f32 {
    let size = (max - min).max(Vec3::ZERO);
    2.0 * (size.x * size.y + size.y * size.z + size.z * size.x)
}

/// Builds nodes over boxes with their centroids; answers the nodes and the boxes' order in the
/// leaves.
fn build_nodes(bounds: &[(Vec3, Vec3)], centroids: &[Vec3]) -> (Vec<Node>, Vec<u32>) {
    let count = bounds.len();
    let mut order: Vec<u32> = (0..u32::try_from(count).unwrap_or(u32::MAX)).collect();
    let mut nodes = vec![Node { min: Vec3::ZERO, max: Vec3::ZERO, first: 0, count: 0 }];
    // (node, start, end) still to split.
    let mut work = vec![(0usize, 0usize, count)];
    while let Some((node, start, end)) = work.pop() {
        let range = order.get(start..end).unwrap_or(&[]);
        let (mut min, mut max) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        let (mut cmin, mut cmax) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for index in range {
            if let (Some((lo, hi)), Some(centre)) = (bounds.get(*index as usize), centroids.get(*index as usize)) {
                min = min.min(*lo);
                max = max.max(*hi);
                cmin = cmin.min(*centre);
                cmax = cmax.max(*centre);
            }
        }
        let leaf = |nodes: &mut Vec<Node>| {
            if let Some(slot) = nodes.get_mut(node) {
                *slot = Node { min, max, first: u32::try_from(start).unwrap_or(0), count: u32::try_from(end - start).unwrap_or(0) };
            }
        };
        if end - start <= LEAF_SIZE {
            leaf(&mut nodes);
            continue;
        }
        // The best split over binned centroids on each axis.
        let extent = cmax - cmin;
        let mut best: Option<(f32, usize, f32)> = None;
        for axis in 0..3 {
            let (lo, span) = (cmin[axis], extent[axis]);
            if span <= 1e-9 {
                continue;
            }
            let mut bins = [(Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY), 0usize); BINS];
            for index in range {
                let (Some((blo, bhi)), Some(centre)) = (bounds.get(*index as usize), centroids.get(*index as usize)) else { continue };
                let slot = (((centre[axis] - lo) / span * BINS as f32) as usize).min(BINS - 1);
                if let Some(bin) = bins.get_mut(slot) {
                    bin.0 = bin.0.min(*blo);
                    bin.1 = bin.1.max(*bhi);
                    bin.2 += 1;
                }
            }
            for split in 1..BINS {
                let (mut lmin, mut lmax, mut lcount) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY), 0usize);
                let (mut rmin, mut rmax, mut rcount) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY), 0usize);
                for (slot, bin) in bins.iter().enumerate() {
                    if bin.2 == 0 {
                        continue;
                    }
                    if slot < split {
                        lmin = lmin.min(bin.0);
                        lmax = lmax.max(bin.1);
                        lcount += bin.2;
                    } else {
                        rmin = rmin.min(bin.0);
                        rmax = rmax.max(bin.1);
                        rcount += bin.2;
                    }
                }
                if lcount == 0 || rcount == 0 {
                    continue;
                }
                let cost = area(lmin, lmax) * lcount as f32 + area(rmin, rmax) * rcount as f32;
                if best.is_none_or(|(known, _, _)| cost < known) {
                    best = Some((cost, axis, lo + span * split as f32 / BINS as f32));
                }
            }
        }
        let Some((cost, axis, position)) = best else {
            leaf(&mut nodes);
            continue;
        };
        if cost >= area(min, max) * (end - start) as f32 && end - start <= 16 {
            leaf(&mut nodes);
            continue;
        }
        // Partition the range around the split.
        let slice = order.get_mut(start..end).unwrap_or(&mut []);
        let mut middle = 0;
        for scan in 0..slice.len() {
            let index = slice.get(scan).copied().unwrap_or(0);
            if centroids.get(index as usize).is_some_and(|centre| centre[axis] < position) {
                slice.swap(scan, middle);
                middle += 1;
            }
        }
        if middle == 0 || middle == slice.len() {
            middle = slice.len() / 2;
        }
        let left = nodes.len();
        nodes.push(Node { min: Vec3::ZERO, max: Vec3::ZERO, first: 0, count: 0 });
        nodes.push(Node { min: Vec3::ZERO, max: Vec3::ZERO, first: 0, count: 0 });
        if let Some(slot) = nodes.get_mut(node) {
            *slot = Node { min, max, first: u32::try_from(left).unwrap_or(0), count: 0 };
        }
        work.push((left, start, start + middle));
        work.push((left + 1, start + middle, end));
    }
    (nodes, order)
}

impl Bvh {
    /// Builds the hierarchy over `triangles` (hit indices refer to this order).
    pub fn build(triangles: Vec<Triangle>) -> Self {
        let bounds: Vec<(Vec3, Vec3)> = triangles.iter().map(Triangle::bounds).collect();
        let centroids: Vec<Vec3> = triangles.iter().map(Triangle::centroid).collect();
        let (nodes, order) = build_nodes(&bounds, &centroids);
        let ordered = order.iter().filter_map(|index| triangles.get(*index as usize).copied()).collect();
        Self { nodes, triangles: ordered, order }
    }

    /// Whether the hierarchy holds nothing.
    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }

    /// The nearest hit along the ray in (`near`, `far`), or with `any`, the first one found.
    pub fn intersect(&self, origin: Vec3, direction: Vec3, near: f32, far: f32, any: bool) -> Option<Hit> {
        if self.triangles.is_empty() {
            return None;
        }
        walk(&self.nodes, origin, direction, far, any, |slot, far| {
            let (t, u, v) = self.triangles.get(slot)?.intersect(origin, direction, near, far)?;
            Some(Hit { distance: t, triangle: self.order.get(slot).copied().unwrap_or(0), u, v })
        })
    }
}

fn slab(node: &Node, origin: Vec3, inverse: Vec3, far: f32) -> Option<f32> {
    let t0 = (node.min - origin) * inverse;
    let t1 = (node.max - origin) * inverse;
    let near = t0.min(t1).max_element().max(0.0);
    let exit = t0.max(t1).min_element().min(far);
    (near <= exit).then_some(near)
}

/// Walks the hierarchy along a ray, testing leaf slots with `test` (given the nearest distance
/// so far): the nearest hit, or with `any`, the first one found.
fn walk(nodes: &[Node], origin: Vec3, direction: Vec3, mut far: f32, any: bool, mut test: impl FnMut(usize, f32) -> Option<Hit>) -> Option<Hit> {
    if nodes.is_empty() {
        return None;
    }
    let inverse = Vec3::ONE / direction;
    let mut stack = [0u32; STACK];
    let mut top = 1usize;
    let mut best: Option<Hit> = None;
    while top > 0 {
        top -= 1;
        let Some(node) = stack.get(top).and_then(|index| nodes.get(*index as usize)) else { break };
        if slab(node, origin, inverse, far).is_none() {
            continue;
        }
        if node.count > 0 {
            let start = node.first as usize;
            for slot in start..start + node.count as usize {
                if let Some(hit) = test(slot, far) {
                    far = hit.distance;
                    best = Some(hit);
                    if any {
                        return best;
                    }
                }
            }
            continue;
        }
        // Nearer child last, so it is walked first.
        let (left, right) = (node.first, node.first + 1);
        let distance = |index: u32| nodes.get(index as usize).and_then(|child| slab(child, origin, inverse, far));
        let (dl, dr) = (distance(left), distance(right));
        let pushes: [(Option<f32>, u32); 2] = match (dl, dr) {
            (Some(a), Some(b)) if a < b => [(dr, right), (dl, left)],
            _ => [(dl, left), (dr, right)],
        };
        for (hit, index) in pushes {
            if hit.is_some()
                && top < STACK
                && let Some(slot) = stack.get_mut(top)
            {
                *slot = index;
                top += 1;
            }
        }
    }
    best
}

/// A hierarchy over triangles that move while the shutter is open: each has its corners at
/// evenly spaced times from 0 (the shutter opens) to 1 (it closes), straight between them.
/// Its boxes hold every time's corners, so one hierarchy serves the whole shutter.
#[derive(Debug, Clone, Default)]
pub struct MotionBvh {
    nodes: Vec<Node>,
    /// `steps` triangles per moving triangle, in leaf order.
    triangles: Vec<Triangle>,
    order: Vec<u32>,
    steps: usize,
}

impl MotionBvh {
    /// Builds the hierarchy over triangles given at `steps` times each (`triangles[i * steps + s]`
    /// is triangle `i` at step `s`; hit indices count triangles, not steps).
    pub fn build(triangles: &[Triangle], steps: usize) -> Self {
        let steps = steps.max(1);
        let count = triangles.len() / steps;
        let mut bounds = Vec::with_capacity(count);
        let mut centroids = Vec::with_capacity(count);
        for index in 0..count {
            let at = triangles.get(index * steps..(index + 1) * steps).unwrap_or(&[]);
            let (mut min, mut max) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
            for triangle in at {
                let (lo, hi) = triangle.bounds();
                min = min.min(lo);
                max = max.max(hi);
            }
            bounds.push((min, max));
            centroids.push((min + max) * 0.5);
        }
        let (nodes, order) = build_nodes(&bounds, &centroids);
        let ordered = order
            .iter()
            .flat_map(|index| triangles.get(*index as usize * steps..(*index as usize + 1) * steps).unwrap_or(&[]).iter().copied())
            .collect();
        Self { nodes, triangles: ordered, order, steps }
    }

    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }

    /// Where `time` (0 to 1) falls between steps: the earlier step and the share of the way to
    /// the next.
    pub fn bracket(steps: usize, time: f32) -> (usize, f32) {
        let span = steps.saturating_sub(1).max(1) as f32;
        let at = (time.clamp(0.0, 1.0) * span).min(span);
        let first = (at.floor() as usize).min(steps.saturating_sub(2));
        (first, at - first as f32)
    }

    /// The triangle in slot `slot` at `time`.
    fn at(&self, slot: usize, time: f32) -> Option<Triangle> {
        let base = slot * self.steps;
        if self.steps == 1 {
            return self.triangles.get(base).copied();
        }
        let (first, share) = Self::bracket(self.steps, time);
        let (a, b) = (self.triangles.get(base + first)?, self.triangles.get(base + first + 1)?);
        Some(Triangle { a: a.a.lerp(b.a, share), edge1: a.edge1.lerp(b.edge1, share), edge2: a.edge2.lerp(b.edge2, share) })
    }

    /// The nearest hit at `time` along the ray in (`near`, `far`), or with `any`, the first one.
    pub fn intersect(&self, origin: Vec3, direction: Vec3, near: f32, far: f32, any: bool, time: f32) -> Option<Hit> {
        if self.triangles.is_empty() {
            return None;
        }
        walk(&self.nodes, origin, direction, far, any, |slot, far| {
            let (t, u, v) = self.at(slot, time)?.intersect(origin, direction, near, far)?;
            Some(Hit { distance: t, triangle: self.order.get(slot).copied().unwrap_or(0), u, v })
        })
    }
}
