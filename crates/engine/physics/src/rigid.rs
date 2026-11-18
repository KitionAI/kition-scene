//! Rigid bodies and their solver: semi-implicit Euler steps, a sort-and-sweep broad phase,
//! contacts from `collide`, and sequential impulses with warm starting, speculative contacts,
//! and Coulomb friction. Everything runs in a fixed order (body index, then contact feature), so
//! equal inputs give equal motion on every machine.

use std::collections::{BTreeMap, BTreeSet};

use glam::{Mat3, Quat, Vec3};

use crate::collide::{Contact, Placed, WorldPoly, contacts, poly_poly};
use crate::joint::Joint;
use crate::shape::{Poly, Shape};

/// How a body moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Dynamic,
    /// Never moves.
    Static,
    /// Follows a path it is given (an animated passive body).
    Kinematic,
}

/// One body.
#[derive(Debug, Clone)]
pub struct Body {
    pub shape: Shape,
    pub motion: Motion,
    pub position: Vec3,
    pub rotation: Quat,
    pub velocity: Vec3,
    pub spin: Vec3,
    pub inverse_mass: f32,
    /// The inverse of the inertia about the body's own axes (a diagonal).
    pub inverse_inertia: Vec3,
    pub friction: f32,
    pub restitution: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub margin: f32,
    /// Asleep: not simulated until something awake knocks into it.
    pub sleeping: bool,
    /// May fall asleep once slower than these speeds (m/s, rad/s) for [`SLEEP_AFTER`] seconds.
    pub can_sleep: bool,
    pub sleep_linear: f32,
    pub sleep_angular: f32,
    still_for: f32,
    reach: f32,
}

/// How long a body that may sleep must move slowly before it falls asleep, in seconds.
pub const SLEEP_AFTER: f32 = 2.0;

impl Body {
    /// A body of `shape` at a pose; `mass` is ignored unless it is dynamic. Inertia is a box's
    /// over the shape's bounds (a sphere's for spheres), as the reference's solver takes it.
    pub fn new(shape: Shape, motion: Motion, position: Vec3, rotation: Quat, mass: f32) -> Self {
        let (low, high) = shape.bounds();
        let size = (high - low).max(Vec3::splat(1e-3));
        let (inverse_mass, inverse_inertia) = if motion == Motion::Dynamic && mass > 0.0 {
            let inertia = match &shape {
                Shape::Sphere { radius } => Vec3::splat(0.4 * mass * radius * radius),
                _ => {
                    Vec3::new(size.y * size.y + size.z * size.z, size.x * size.x + size.z * size.z, size.x * size.x + size.y * size.y) * (mass / 12.0)
                }
            };
            (1.0 / mass, Vec3::ONE / inertia.max(Vec3::splat(1e-9)))
        } else {
            (0.0, Vec3::ZERO)
        };
        let reach = shape.reach();
        Self {
            shape,
            motion,
            position,
            rotation,
            velocity: Vec3::ZERO,
            spin: Vec3::ZERO,
            inverse_mass,
            inverse_inertia,
            friction: 0.5,
            restitution: 0.0,
            linear_damping: 0.04,
            angular_damping: 0.1,
            margin: 0.04,
            sleeping: false,
            can_sleep: false,
            sleep_linear: 0.4,
            sleep_angular: 0.5,
            still_for: 0.0,
            reach,
        }
    }

    /// Moved by the solver now: dynamic and awake.
    pub fn moves(&self) -> bool {
        self.motion == Motion::Dynamic && !self.sleeping
    }

    /// Wakes the body.
    pub fn wake(&mut self) {
        self.sleeping = false;
        self.still_for = 0.0;
    }

    pub(crate) fn world_inverse_inertia(&self) -> Mat3 {
        let rotation = Mat3::from_quat(self.rotation);
        rotation * Mat3::from_diagonal(self.inverse_inertia) * rotation.transpose()
    }

    pub(crate) fn velocity_at(&self, offset: Vec3) -> Vec3 {
        self.velocity + self.spin.cross(offset)
    }

    /// The box the body spans in the world (a sphere around its reach, plus its margin).
    fn sweep_box(&self) -> (Vec3, Vec3) {
        let reach = Vec3::splat(self.reach + self.margin);
        (self.position - reach, self.position + reach)
    }
}

/// A contact being solved this step.
#[derive(Debug, Clone, Copy)]
struct Solving {
    a: usize,
    b: usize,
    feature: u32,
    point: Vec3,
    ra: Vec3,
    rb: Vec3,
    normal: Vec3,
    tangents: [Vec3; 2],
    normal_mass: f32,
    tangent_mass: [f32; 2],
    bias: f32,
    friction: f32,
    normal_impulse: f32,
    tangent_impulse: [f32; 2],
}

/// The impulses a contact ended a step with, for warm starting the next.
#[derive(Debug, Clone, Copy)]
struct Kept {
    feature: u32,
    point: Vec3,
    normal_impulse: f32,
    tangent_impulse: [f32; 2],
}

/// A world of bodies.
#[derive(Debug, Clone)]
pub struct World {
    pub bodies: Vec<Body>,
    pub gravity: Vec3,
    pub iterations: u32,
    /// Constraints joining bodies.
    pub joints: Vec<Joint>,
    kept: BTreeMap<(usize, usize), Vec<Kept>>,
}

/// Passes over the joint rows per solver iteration (joints couple turns and slides, so they
/// take more passes to settle than contacts).
const JOINT_PASSES: u32 = 4;
const BAUMGARTE: f32 = 0.2;
const SLOP: f32 = 0.005;
/// Closing speeds below this never bounce (resting contacts stay put).
const BOUNCE_THRESHOLD: f32 = 1.0;
/// The fastest a contact pushes bodies apart to undo overlap, in m/s.
const MAX_CORRECTION: f32 = 3.0;

impl World {
    pub fn new(gravity: Vec3, iterations: u32) -> Self {
        Self { bodies: Vec::new(), gravity, iterations: iterations.max(1), joints: Vec::new(), kept: BTreeMap::new() }
    }

    /// Pairs that never collide: bodies a working joint joins with collisions off.
    fn ignored(&self) -> BTreeSet<(usize, usize)> {
        self.joints
            .iter()
            .filter(|joint| joint.working() && joint.settings.disable_collisions)
            .map(|joint| (joint.a.min(joint.b), joint.a.max(joint.b)))
            .collect()
    }

    /// Pairs whose boxes overlap (sort and sweep on X), mesh or static first, in a fixed order.
    fn pairs(&self) -> Vec<(usize, usize)> {
        let ignored = self.ignored();
        let mut spans: Vec<(f32, f32, usize, (Vec3, Vec3))> = self
            .bodies
            .iter()
            .enumerate()
            .map(|(index, body)| {
                let (low, high) = body.sweep_box();
                (low.x, high.x, index, (low, high))
            })
            .collect();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.2.cmp(&b.2)));
        let mut out = Vec::new();
        for (slot, (_, high_x, i, (low_i, high_i))) in spans.iter().enumerate() {
            for (low_x, _, j, (low_j, high_j)) in spans.iter().skip(slot + 1) {
                if low_x > high_x {
                    break;
                }
                if low_i.y > high_j.y || low_j.y > high_i.y || low_i.z > high_j.z || low_j.z > high_i.z {
                    continue;
                }
                let (Some(a), Some(b)) = (self.bodies.get(*i), self.bodies.get(*j)) else { continue };
                if !a.moves() && !b.moves() || ignored.contains(&((*i).min(*j), (*i).max(*j))) {
                    continue;
                }
                // The non-dynamic one first (a mesh is never dynamic).
                let ordered = if a.motion != Motion::Dynamic || (b.motion == Motion::Dynamic && i < j) { (*i, *j) } else { (*j, *i) };
                out.push(ordered);
            }
        }
        out.sort_unstable();
        out
    }

    /// Contacts of one pair, `a` first.
    fn pair_contacts(&self, a: usize, b: usize) -> Vec<Contact> {
        let (Some(body_a), Some(body_b)) = (self.bodies.get(a), self.bodies.get(b)) else { return Vec::new() };
        let margin = body_a.margin.max(body_b.margin);
        match &body_a.shape {
            Shape::Mesh(mesh) => {
                // Triangle by triangle, in the mesh's frame.
                let inverse = body_a.rotation.inverse();
                let local_centre = inverse * (body_b.position - body_a.position);
                let reach = Vec3::splat(body_b.reach + margin);
                let near = mesh.near(local_centre - reach, local_centre + reach);
                let mut found: Vec<Contact> = Vec::new();
                for index in near {
                    let Some([p, q, r]) = mesh.triangles.get(index as usize).copied() else { continue };
                    let Some(triangle) = Poly::triangle(p, q, r) else { continue };
                    let world = WorldPoly::new(&triangle, body_a.position, body_a.rotation);
                    let more = match &body_b.shape {
                        Shape::Poly(poly) => poly_poly(&world, &WorldPoly::new(poly, body_b.position, body_b.rotation), margin),
                        Shape::Sphere { radius } => {
                            let corner = |slot: usize| world.vertices.get(slot).copied().unwrap_or(Vec3::ZERO);
                            let surface = crate::collide::closest_on_triangle(body_b.position, corner(0), corner(1), corner(2));
                            let offset = body_b.position - surface;
                            let distance = offset.length();
                            let face = world.normals.first().copied().unwrap_or(Vec3::Z);
                            let normal = if distance > 1e-6 { offset / distance } else { face };
                            let depth = radius - distance;
                            if depth < -margin {
                                Vec::new()
                            } else {
                                vec![Contact { point: body_b.position - normal * *radius, normal, depth, feature: 0 }]
                            }
                        }
                        Shape::Mesh(_) => Vec::new(),
                    };
                    // A triangle's back face never pushes (the body came through the front).
                    let face = world.normals.first().copied().unwrap_or(Vec3::Z);
                    for mut contact in more {
                        if contact.normal.dot(face) < 0.0 {
                            continue;
                        }
                        contact.feature = index.wrapping_mul(16).wrapping_add(contact.feature & 15);
                        found.push(contact);
                    }
                }
                if found.len() > 4 {
                    found.sort_by(|x, y| y.depth.total_cmp(&x.depth).then(x.feature.cmp(&y.feature)));
                    let mut kept = vec![found.remove(0)];
                    while kept.len() < 4 && !found.is_empty() {
                        let slot = found
                            .iter()
                            .enumerate()
                            .max_by(|(_, x), (_, y)| {
                                let spread = |c: &Contact| kept.iter().map(|k| k.point.distance_squared(c.point)).fold(f32::INFINITY, f32::min);
                                spread(x).total_cmp(&spread(y))
                            })
                            .map_or(0, |(slot, _)| slot);
                        kept.push(found.remove(slot));
                    }
                    found = kept;
                }
                found
            }
            _ => contacts(
                Placed { shape: &body_a.shape, position: body_a.position, rotation: body_a.rotation },
                Placed { shape: &body_b.shape, position: body_b.position, rotation: body_b.rotation },
                margin,
            ),
        }
    }

    /// One step of `dt` seconds. Kinematic bodies must have their velocities set to reach their
    /// next pose (`drive`) before.
    pub fn step(&mut self, dt: f32) {
        if dt.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
            return;
        }
        // Forces and damping.
        let gravity = self.gravity;
        for body in self.bodies.iter_mut().filter(|body| body.moves()) {
            body.velocity += gravity * dt;
            body.velocity *= (1.0 - body.linear_damping).clamp(0.0, 1.0).powf(dt);
            body.spin *= (1.0 - body.angular_damping).clamp(0.0, 1.0).powf(dt);
        }
        // Contacts.
        let mut solving: Vec<Solving> = Vec::new();
        let mut next_kept: BTreeMap<(usize, usize), Vec<Kept>> = BTreeMap::new();
        self.wake_joined();
        for (a, b) in self.pairs() {
            let found = self.pair_contacts(a, b);
            if found.is_empty() {
                continue;
            }
            // Something awake (or moved by its animation) touching a sleeping body wakes it.
            for (sleeper, other) in [(a, b), (b, a)] {
                let knocks = self.bodies.get(other).is_some_and(|body| body.moves() || body.motion == Motion::Kinematic);
                if knocks && let Some(body) = self.bodies.get_mut(sleeper).filter(|body| body.sleeping) {
                    body.wake();
                }
            }
            let (Some(body_a), Some(body_b)) = (self.bodies.get(a), self.bodies.get(b)) else { continue };
            let previous = self.kept.get(&(a, b));
            let friction = (body_a.friction * body_b.friction).max(0.0);
            let restitution = (body_a.restitution * body_b.restitution).clamp(0.0, 1.0);
            let (inv_a, inv_b) = (body_a.world_inverse_inertia(), body_b.world_inverse_inertia());
            for contact in found {
                let normal = contact.normal;
                let ra = contact.point - body_a.position;
                let rb = contact.point - body_b.position;
                let tangent = if normal.x.abs() < 0.57 { normal.cross(Vec3::X) } else { normal.cross(Vec3::Y) }.normalize_or_zero();
                let tangents = [tangent, normal.cross(tangent)];
                let mass = |axis: Vec3| {
                    let k = body_a.inverse_mass
                        + body_b.inverse_mass
                        + (inv_a * ra.cross(axis)).cross(ra).dot(axis)
                        + (inv_b * rb.cross(axis)).cross(rb).dot(axis);
                    if k > 1e-12 { 1.0 / k } else { 0.0 }
                };
                let relative = body_b.velocity_at(rb) - body_a.velocity_at(ra);
                let closing = relative.dot(normal);
                let mut bias = if contact.depth > SLOP {
                    (BAUMGARTE / dt * (contact.depth - SLOP)).min(MAX_CORRECTION)
                } else if contact.depth < 0.0 {
                    // Speculative: the gap may close this step, no faster.
                    contact.depth / dt
                } else {
                    0.0
                };
                if closing < -BOUNCE_THRESHOLD && restitution > 0.0 {
                    bias = bias.max(-restitution * closing);
                }
                let warm = previous
                    .and_then(|list| list.iter().find(|kept| kept.feature == contact.feature && kept.point.distance_squared(contact.point) < 0.0025));
                solving.push(Solving {
                    a,
                    b,
                    feature: contact.feature,
                    point: contact.point,
                    ra,
                    rb,
                    normal,
                    tangents,
                    normal_mass: mass(normal),
                    tangent_mass: [mass(tangents[0]), mass(tangents[1])],
                    bias,
                    friction,
                    normal_impulse: warm.map_or(0.0, |kept| kept.normal_impulse),
                    tangent_impulse: warm.map_or([0.0; 2], |kept| kept.tangent_impulse),
                });
            }
        }
        let mut rows = crate::joint::prepare(self, dt);
        // Warm start.
        for contact in &solving {
            let impulse = contact.normal * contact.normal_impulse
                + contact.tangents[0] * contact.tangent_impulse[0]
                + contact.tangents[1] * contact.tangent_impulse[1];
            self.apply(contact.a, contact.b, contact.ra, contact.rb, impulse);
        }
        // Velocity iterations: joints, then contacts, in turn.
        for _ in 0..self.iterations {
            for _ in 0..JOINT_PASSES {
                crate::joint::solve(self, &mut rows);
            }
            for contact in &mut solving {
                let (Some(a), Some(b)) = (self.bodies.get(contact.a), self.bodies.get(contact.b)) else { continue };
                // Friction first, bounded by the current normal impulse.
                let relative = b.velocity_at(contact.rb) - a.velocity_at(contact.ra);
                let limit = contact.friction * contact.normal_impulse;
                let mut change = Vec3::ZERO;
                for ((tangent, mass), impulse) in contact.tangents.iter().zip(&contact.tangent_mass).zip(&mut contact.tangent_impulse) {
                    let lambda = -mass * relative.dot(*tangent);
                    let old = *impulse;
                    let new = (old + lambda).clamp(-limit, limit);
                    *impulse = new;
                    change += *tangent * (new - old);
                }
                self.apply(contact.a, contact.b, contact.ra, contact.rb, change);
                let (Some(a), Some(b)) = (self.bodies.get(contact.a), self.bodies.get(contact.b)) else { continue };
                let closing = (b.velocity_at(contact.rb) - a.velocity_at(contact.ra)).dot(contact.normal);
                let lambda = contact.normal_mass * (contact.bias - closing);
                let old = contact.normal_impulse;
                let new = (old + lambda).max(0.0);
                contact.normal_impulse = new;
                self.apply(contact.a, contact.b, contact.ra, contact.rb, contact.normal * (new - old));
            }
        }
        for contact in &solving {
            next_kept.entry((contact.a, contact.b)).or_default().push(Kept {
                feature: contact.feature,
                point: contact.point,
                normal_impulse: contact.normal_impulse,
                tangent_impulse: contact.tangent_impulse,
            });
        }
        self.kept = next_kept;
        crate::joint::finish(self, &rows);
        // Positions.
        for body in self.bodies.iter_mut().filter(|body| body.motion == Motion::Kinematic || body.moves()) {
            body.position += body.velocity * dt;
            let spin = Quat::from_xyzw(body.spin.x, body.spin.y, body.spin.z, 0.0);
            let turned = body.rotation + spin * body.rotation * (0.5 * dt);
            body.rotation = turned.normalize();
        }
        // Sleep: a body that may sleep and has moved slowly long enough stops.
        for body in self.bodies.iter_mut().filter(|body| body.moves() && body.can_sleep) {
            if body.velocity.length() < body.sleep_linear && body.spin.length() < body.sleep_angular {
                body.still_for += dt;
                if body.still_for >= SLEEP_AFTER {
                    body.sleeping = true;
                    body.velocity = Vec3::ZERO;
                    body.spin = Vec3::ZERO;
                }
            } else {
                body.still_for = 0.0;
            }
        }
    }

    /// Wakes sleeping bodies a working joint ties to a body that is awake.
    fn wake_joined(&mut self) {
        let mut woken = Vec::new();
        for joint in self.joints.iter().filter(|joint| joint.working()) {
            for (sleeper, other) in [(joint.a, joint.b), (joint.b, joint.a)] {
                let (Some(sleeping), Some(other)) = (self.bodies.get(sleeper), self.bodies.get(other)) else { continue };
                if sleeping.sleeping && (other.moves() || other.motion == Motion::Kinematic) {
                    woken.push(sleeper);
                }
            }
        }
        for index in woken {
            if let Some(body) = self.bodies.get_mut(index) {
                body.wake();
            }
        }
    }

    pub(crate) fn apply(&mut self, a: usize, b: usize, ra: Vec3, rb: Vec3, impulse: Vec3) {
        if let Some(body) = self.bodies.get_mut(a)
            && body.moves()
        {
            body.velocity -= impulse * body.inverse_mass;
            let inverse = body.world_inverse_inertia();
            body.spin -= inverse * ra.cross(impulse);
        }
        if let Some(body) = self.bodies.get_mut(b)
            && body.moves()
        {
            body.velocity += impulse * body.inverse_mass;
            let inverse = body.world_inverse_inertia();
            body.spin += inverse * rb.cross(impulse);
        }
    }

    /// Applies an angular impulse: `a` turns against it, `b` with it.
    pub(crate) fn apply_angular(&mut self, a: usize, b: usize, impulse: Vec3) {
        if let Some(body) = self.bodies.get_mut(a)
            && body.moves()
        {
            let inverse = body.world_inverse_inertia();
            body.spin -= inverse * impulse;
        }
        if let Some(body) = self.bodies.get_mut(b)
            && body.moves()
        {
            let inverse = body.world_inverse_inertia();
            body.spin += inverse * impulse;
        }
    }

    /// Gives a kinematic body the velocities that take it to `position`, `rotation` in `dt`.
    pub fn drive(&mut self, index: usize, position: Vec3, rotation: Quat, dt: f32) {
        let Some(body) = self.bodies.get_mut(index) else { return };
        if body.motion != Motion::Kinematic || dt.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
            return;
        }
        body.velocity = (position - body.position) / dt;
        let mut delta = rotation * body.rotation.inverse();
        if delta.w < 0.0 {
            delta = -delta;
        }
        let (axis, angle) = delta.to_axis_angle();
        body.spin = if angle.is_finite() { axis * (angle / dt) } else { Vec3::ZERO };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ground() -> Body {
        Body::new(
            Shape::Poly(Poly::cuboid(Vec3::ZERO, Vec3::new(10.0, 10.0, 0.5)).unwrap()),
            Motion::Static,
            Vec3::new(0.0, 0.0, -0.5),
            Quat::IDENTITY,
            0.0,
        )
    }

    #[test]
    fn a_box_falls_and_rests_on_the_ground() {
        let mut world = World::new(Vec3::new(0.0, 0.0, -9.81), 10);
        world.bodies.push(ground());
        world.bodies.push(Body::new(
            Shape::Poly(Poly::cuboid(Vec3::ZERO, Vec3::splat(0.5)).unwrap()),
            Motion::Dynamic,
            Vec3::new(0.0, 0.0, 3.0),
            Quat::IDENTITY,
            1.0,
        ));
        for _ in 0..(240 * 4) {
            world.step(1.0 / 240.0);
        }
        let block = &world.bodies[1];
        assert!((block.position.z - 0.5).abs() < 0.02, "rests on the ground: {}", block.position);
        assert!(block.velocity.length() < 0.05, "and stays: {}", block.velocity);
        assert!(block.rotation.angle_between(Quat::IDENTITY) < 0.02);
    }

    #[test]
    fn a_ball_bounces_with_restitution() {
        let mut world = World::new(Vec3::new(0.0, 0.0, -9.81), 10);
        let mut floor = ground();
        floor.restitution = 1.0;
        world.bodies.push(floor);
        let mut ball = Body::new(Shape::Sphere { radius: 0.5 }, Motion::Dynamic, Vec3::new(0.0, 0.0, 2.5), Quat::IDENTITY, 1.0);
        ball.restitution = 0.8;
        ball.linear_damping = 0.0;
        world.bodies.push(ball);
        let mut highest_after_bounce = 0.0f32;
        let mut bounced = false;
        for _ in 0..(240 * 3) {
            world.step(1.0 / 240.0);
            let ball = &world.bodies[1];
            if ball.velocity.z > 0.5 {
                bounced = true;
            }
            if bounced {
                highest_after_bounce = highest_after_bounce.max(ball.position.z);
            }
        }
        assert!(bounced, "it bounced");
        // Falling 2 m and keeping 0.8 of the speed rises about 0.64 × 2 m.
        assert!((highest_after_bounce - 0.5 - 1.28).abs() < 0.2, "{highest_after_bounce}");
    }

    #[test]
    fn stepping_is_deterministic() {
        let run = || {
            let mut world = World::new(Vec3::new(0.0, 0.0, -9.81), 10);
            world.bodies.push(ground());
            for level in 0..4 {
                let offset = Vec3::new(0.03 * level as f32, 0.0, 0.5 + level as f32 * 1.01);
                world.bodies.push(Body::new(
                    Shape::Poly(Poly::cuboid(Vec3::ZERO, Vec3::splat(0.5)).unwrap()),
                    Motion::Dynamic,
                    offset,
                    Quat::from_rotation_z(0.1 * level as f32),
                    1.0,
                ));
            }
            for _ in 0..600 {
                world.step(1.0 / 240.0);
            }
            world.bodies.iter().map(|body| (body.position, body.rotation)).collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }
}
