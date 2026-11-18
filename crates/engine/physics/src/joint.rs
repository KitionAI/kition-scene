//! Rigid body constraints: two bodies joined at a point along a frame (the constraint object's
//! origin and axes when the simulation starts), solved as rows of sequential impulses alongside
//! the contacts. Each kind locks some of the six relative motions (three slides, three turns,
//! measured in the first body's frame) and limits or springs others:
//!
//! - Fixed locks all six; Point the three slides; Hinge the slides and the turns about X and
//!   Y (turning about Z, within its limit); Slider every turn and the slides along Y and Z
//!   (sliding along X, within its limit).
//! - Generic Spring locks or limits each axis as set, and pulls sprung axes back to where they
//!   started with a spring and damper.
//!
//! A breakable constraint whose rows push harder than its threshold in one step breaks for good.

use glam::{Quat, Vec3};
use kition_scene_model::physics::{JointKind, RigidConstraint};

use crate::rigid::World;

/// How much of the drift each step corrects.
const BAUMGARTE: f32 = 0.2;
/// Limits engage this far before they are reached (radians or metres), so a body stops at them.
const LIMIT_SLOP: f32 = 0.02;

/// A constraint between bodies `a` and `b` of a world.
#[derive(Debug, Clone)]
pub struct Joint {
    pub a: usize,
    pub b: usize,
    pub settings: RigidConstraint,
    /// The pivot in each body's own frame.
    local_a: Vec3,
    local_b: Vec3,
    /// The constraint's axes in each body's own frame.
    frame_a: Quat,
    frame_b: Quat,
    pub broken: bool,
}

impl Joint {
    /// A joint at `pivot` with axes `axes` (world), between the bodies as they are now.
    pub fn new(world: &World, a: usize, b: usize, pivot: Vec3, axes: Quat, settings: RigidConstraint) -> Option<Self> {
        let (body_a, body_b) = (world.bodies.get(a)?, world.bodies.get(b)?);
        let inverse_a = body_a.rotation.inverse();
        let inverse_b = body_b.rotation.inverse();
        Some(Self {
            a,
            b,
            settings,
            local_a: inverse_a * (pivot - body_a.position),
            local_b: inverse_b * (pivot - body_b.position),
            frame_a: (inverse_a * axes).normalize(),
            frame_b: (inverse_b * axes).normalize(),
            broken: false,
        })
    }

    /// Enabled and not broken.
    pub fn working(&self) -> bool {
        self.settings.enabled && !self.broken
    }
}

/// What a row constrains.
#[derive(Debug, Clone, Copy)]
enum Along {
    /// The pivots' relative velocity along a world direction.
    Slide(Vec3),
    /// The bodies' relative spin about a world axis.
    Turn(Vec3),
}

/// One row being solved this step.
#[derive(Debug, Clone, Copy)]
struct Row {
    joint: usize,
    along: Along,
    ra: Vec3,
    rb: Vec3,
    mass: f32,
    /// The relative velocity the row aims for.
    target: f32,
    low: f32,
    high: f32,
    impulse: f32,
}

/// The rows of a step.
pub struct Rows {
    rows: Vec<Row>,
}

/// The pivots, frames, and errors of a joint now.
struct State {
    ra: Vec3,
    rb: Vec3,
    axes: [Vec3; 3],
    /// The pivots' offset, in the joint's axes.
    offset: Vec3,
    /// The relative turn as a rotation vector, in the joint's axes.
    turn: Vec3,
    /// The turn about Z (the hinge's angle).
    twist: f32,
    /// Relative velocity and spin, in the joint's axes.
    velocity: Vec3,
    spin: Vec3,
}

fn state(world: &World, joint: &Joint) -> Option<State> {
    let (a, b) = (world.bodies.get(joint.a)?, world.bodies.get(joint.b)?);
    let ra = a.rotation * joint.local_a;
    let rb = b.rotation * joint.local_b;
    let frame_a = a.rotation * joint.frame_a;
    let frame_b = b.rotation * joint.frame_b;
    let axes = [frame_a * Vec3::X, frame_a * Vec3::Y, frame_a * Vec3::Z];
    let gap = (b.position + rb) - (a.position + ra);
    let mut relative = frame_a.inverse() * frame_b;
    if relative.w < 0.0 {
        relative = -relative;
    }
    let (axis, angle) = relative.to_axis_angle();
    let turn = if angle.is_finite() { axis * angle } else { Vec3::ZERO };
    let twist = 2.0 * relative.z.atan2(relative.w);
    let along = |vector: Vec3| Vec3::new(vector.dot(axes[0]), vector.dot(axes[1]), vector.dot(axes[2]));
    let velocity = b.velocity_at(rb) - a.velocity_at(ra);
    Some(State { ra, rb, axes, offset: along(gap), turn, twist, velocity: along(velocity), spin: along(b.spin - a.spin) })
}

fn row_mass(world: &World, joint: &Joint, along: Along, ra: Vec3, rb: Vec3) -> f32 {
    let (Some(a), Some(b)) = (world.bodies.get(joint.a), world.bodies.get(joint.b)) else { return 0.0 };
    let (inverse_a, inverse_b) = (a.world_inverse_inertia(), b.world_inverse_inertia());
    let (mass_a, mass_b) = (if a.moves() { a.inverse_mass } else { 0.0 }, if b.moves() { b.inverse_mass } else { 0.0 });
    let (inverse_a, inverse_b) = (if a.moves() { inverse_a } else { glam::Mat3::ZERO }, if b.moves() { inverse_b } else { glam::Mat3::ZERO });
    let k = match along {
        Along::Slide(n) => mass_a + mass_b + (inverse_a * ra.cross(n)).cross(ra).dot(n) + (inverse_b * rb.cross(n)).cross(rb).dot(n),
        Along::Turn(n) => (inverse_a * n).dot(n) + (inverse_b * n).dot(n),
    };
    if k > 1e-12 { 1.0 / k } else { 0.0 }
}

/// Locks, limits, and springs as rows for one joint; springs act at once.
fn rows_of(world: &mut World, index: usize, dt: f32, out: &mut Vec<Row>) {
    let Some(joint) = world.joints.get(index).cloned() else { return };
    if !joint.working() {
        return;
    }
    let Some(now) = state(world, &joint) else { return };
    let moving =
        |world: &World| world.bodies.get(joint.a).is_some_and(|body| body.moves()) || world.bodies.get(joint.b).is_some_and(|body| body.moves());
    if !moving(world) {
        return;
    }
    let settings = &joint.settings;
    // A row aiming for relative velocity `target` with its impulse between `low` and `high`.
    let mut add = |world: &World, along: Along, target: f32, low: f32, high: f32| {
        let mass = row_mass(world, &joint, along, now.ra, now.rb);
        if mass <= 0.0 {
            return;
        }
        out.push(Row { joint: index, along, ra: now.ra, rb: now.rb, mass, target, low, high, impulse: 0.0 });
    };
    let correct = |error: f32| -BAUMGARTE / dt * error;
    // A limit on a coordinate `value` between `low` and `high` along `along`: near an end it
    // may close the gap this step, no more; past it, it is pushed back.
    let limit = |world: &World, add: &mut dyn FnMut(&World, Along, f32, f32, f32), along: Along, value: f32, range: [f32; 2]| {
        if range[0] >= range[1] {
            add(world, along, correct(value - range[0]), f32::NEG_INFINITY, f32::INFINITY);
            return;
        }
        let toward = |gap: f32| if gap > 0.0 { gap / dt } else { correct(-gap) };
        if value < range[0] + LIMIT_SLOP {
            // The coordinate may grow, never shrink past the lower end.
            add(world, along, -toward(value - range[0]), 0.0, f32::INFINITY);
        }
        if value > range[1] - LIMIT_SLOP {
            add(world, along, toward(range[1] - value), f32::NEG_INFINITY, 0.0);
        }
    };
    let lock = f32::INFINITY;
    let slide = |axis: usize| Along::Slide(now.axes.get(axis).copied().unwrap_or(Vec3::X));
    let turn = |axis: usize| Along::Turn(now.axes.get(axis).copied().unwrap_or(Vec3::X));
    let offset = now.offset.to_array();
    let turned = now.turn.to_array();
    let pinned = |world: &World, add: &mut dyn FnMut(&World, Along, f32, f32, f32), axes: &[usize]| {
        for &axis in axes {
            add(world, slide(axis), correct(offset.get(axis).copied().unwrap_or(0.0)), -lock, lock);
        }
    };
    let held = |world: &World, add: &mut dyn FnMut(&World, Along, f32, f32, f32), axes: &[usize]| {
        for &axis in axes {
            add(world, turn(axis), correct(turned.get(axis).copied().unwrap_or(0.0)), -lock, lock);
        }
    };
    match settings.kind {
        JointKind::Fixed => {
            pinned(world, &mut add, &[0, 1, 2]);
            held(world, &mut add, &[0, 1, 2]);
        }
        JointKind::Point => pinned(world, &mut add, &[0, 1, 2]),
        JointKind::Hinge => {
            pinned(world, &mut add, &[0, 1, 2]);
            held(world, &mut add, &[0, 1]);
            if let Some(range) = settings.angle_limit {
                limit(world, &mut add, turn(2), now.twist, range);
            }
        }
        JointKind::Slider => {
            pinned(world, &mut add, &[1, 2]);
            held(world, &mut add, &[0, 1, 2]);
            if let Some(range) = settings.distance_limit {
                limit(world, &mut add, slide(0), now.offset.x, range);
            }
        }
        JointKind::Motor => {
            // Observed in Blender: positive targets drive A relative to B along frame A's X.
            // Its Max Impulse control gives a step impulse of value * dt. Accumulated row
            // bounds apply once per step, irrespective of solver iterations.
            let cap = |value: f32| {
                let impulse = value * dt;
                if settings.breakable { impulse.min(settings.breaking_threshold) } else { impulse }
            };
            if settings.linear_motor {
                let impulse = cap(settings.linear_max_impulse);
                add(world, slide(0), -settings.linear_target_velocity, -impulse, impulse);
            }
            if settings.angular_motor {
                let impulse = cap(settings.angular_max_impulse);
                add(world, turn(0), -settings.angular_target_velocity, -impulse, impulse);
            }
        }
        JointKind::GenericSpring => {
            for axis in 0..3 {
                if let Some(Some(range)) = settings.linear_limits.get(axis) {
                    limit(world, &mut add, slide(axis), offset.get(axis).copied().unwrap_or(0.0), *range);
                }
                if let Some(Some(range)) = settings.angular_limits.get(axis) {
                    limit(world, &mut add, turn(axis), turned.get(axis).copied().unwrap_or(0.0), *range);
                }
            }
            // Springs: a force back towards the start and a damper, applied now.
            let velocity = now.velocity.to_array();
            let spin = now.spin.to_array();
            for axis in 0..3 {
                let direction = now.axes.get(axis).copied().unwrap_or(Vec3::X);
                if let Some(Some(stiffness)) = settings.linear_springs.get(axis) {
                    let force =
                        -stiffness * offset.get(axis).copied().unwrap_or(0.0) - settings.spring_damping * velocity.get(axis).copied().unwrap_or(0.0);
                    world.apply(joint.a, joint.b, now.ra, now.rb, direction * (force * dt));
                }
                if let Some(Some(stiffness)) = settings.angular_springs.get(axis) {
                    let torque =
                        -stiffness * turned.get(axis).copied().unwrap_or(0.0) - settings.spring_damping * spin.get(axis).copied().unwrap_or(0.0);
                    world.apply_angular(joint.a, joint.b, direction * (torque * dt));
                }
            }
        }
    }
}

/// Builds every working joint's rows for a step of `dt`.
pub fn prepare(world: &mut World, dt: f32) -> Rows {
    let mut rows = Vec::new();
    for index in 0..world.joints.len() {
        rows_of(world, index, dt, &mut rows);
    }
    Rows { rows }
}

/// One pass over the rows.
pub fn solve(world: &mut World, rows: &mut Rows) {
    for row in &mut rows.rows {
        let Some(joint) = world.joints.get(row.joint) else { continue };
        let (a, b) = (joint.a, joint.b);
        let (Some(body_a), Some(body_b)) = (world.bodies.get(a), world.bodies.get(b)) else { continue };
        let current = match row.along {
            Along::Slide(n) => (body_b.velocity_at(row.rb) - body_a.velocity_at(row.ra)).dot(n),
            Along::Turn(n) => (body_b.spin - body_a.spin).dot(n),
        };
        let lambda = row.mass * (row.target - current);
        let old = row.impulse;
        row.impulse = (old + lambda).clamp(row.low, row.high);
        let change = row.impulse - old;
        match row.along {
            Along::Slide(n) => world.apply(a, b, row.ra, row.rb, n * change),
            Along::Turn(n) => world.apply_angular(a, b, n * change),
        }
    }
}

/// Breaks the breakable joints whose rows pushed harder than their threshold this step.
pub fn finish(world: &mut World, rows: &Rows) {
    let mut totals = vec![0.0f32; world.joints.len()];
    for row in &rows.rows {
        if let Some(total) = totals.get_mut(row.joint) {
            if world.joints.get(row.joint).is_some_and(|joint| joint.settings.kind == JointKind::Motor) {
                // The reference breaks a motor when either drive reaches its threshold,
                // not when two individually weaker drives add up to it.
                *total = total.max(row.impulse.abs());
            } else {
                *total += row.impulse.abs();
            }
        }
    }
    for (joint, total) in world.joints.iter_mut().zip(totals) {
        if joint.settings.breakable
            && (total > joint.settings.breaking_threshold
                || (joint.settings.kind == JointKind::Motor && total > 0.0 && total >= joint.settings.breaking_threshold))
        {
            joint.broken = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rigid::{Body, Motion};
    use crate::shape::{Poly, Shape};

    fn world_with(kind: JointKind, bob: Vec3, axes: Quat, settings: impl FnOnce(&mut RigidConstraint)) -> World {
        let mut world = World::new(Vec3::new(0.0, 0.0, -9.81), 10);
        let anchor = Shape::Poly(Poly::cuboid(Vec3::ZERO, Vec3::splat(0.1)).unwrap());
        world.bodies.push(Body::new(anchor, Motion::Static, Vec3::ZERO, Quat::IDENTITY, 1.0));
        let block = Shape::Poly(Poly::cuboid(Vec3::ZERO, Vec3::splat(0.25)).unwrap());
        let mut body = Body::new(block, Motion::Dynamic, bob, Quat::IDENTITY, 1.0);
        body.linear_damping = 0.0;
        body.angular_damping = 0.0;
        world.bodies.push(body);
        let mut constraint = RigidConstraint { kind, ..RigidConstraint::default() };
        settings(&mut constraint);
        let joint = Joint::new(&world, 0, 1, Vec3::ZERO, axes, constraint).unwrap();
        world.joints.push(joint);
        world
    }

    fn run(world: &mut World, seconds: f32) {
        for _ in 0..(seconds * 240.0) as usize {
            world.step(1.0 / 240.0);
        }
    }

    #[test]
    fn a_hinge_swings_a_pendulum_at_a_fixed_length_in_its_plane() {
        let mut world = world_with(JointKind::Hinge, Vec3::new(2.0, 0.0, 0.0), Quat::from_rotation_x(std::f32::consts::FRAC_PI_2), |_| {});
        let mut lowest = 0.0f32;
        for _ in 0..240 {
            world.step(1.0 / 240.0);
            let bob = world.bodies[1].position;
            assert!((bob.length() - 2.0).abs() < 0.03, "the arm keeps its length: {bob}");
            assert!(bob.y.abs() < 1e-3, "it swings in the XZ plane: {bob}");
            lowest = lowest.min(bob.z);
        }
        assert!(lowest < -1.95, "it swings through the bottom: {lowest}");
    }

    #[test]
    fn a_hinge_limit_stops_the_swing() {
        let limit = 30f32.to_radians();
        let mut world = world_with(JointKind::Hinge, Vec3::new(2.0, 0.0, 0.0), Quat::from_rotation_x(std::f32::consts::FRAC_PI_2), |c| {
            c.angle_limit = Some([-limit, limit]);
        });
        run(&mut world, 2.0);
        let bob = world.bodies[1].position;
        // The reference stops at (1.732, 0, -1.0).
        assert!((bob.x - 1.732).abs() < 0.05 && (bob.z + 1.0).abs() < 0.05, "{bob}");
    }

    #[test]
    fn a_slider_slides_along_its_axis_only_and_stops_at_its_limit() {
        let axes = Quat::from_rotation_y(45f32.to_radians());
        let mut world = world_with(JointKind::Slider, Vec3::new(0.0, 0.0, -1.0), axes, |c| c.distance_limit = Some([-1.0, 1.0]));
        run(&mut world, 2.0);
        let bob = world.bodies[1].position;
        // The reference stops 1 m along the axis: (0.707, 0, -1.707), not turned.
        assert!((bob - Vec3::new(0.707, 0.0, -1.707)).length() < 0.03, "{bob}");
        assert!(world.bodies[1].rotation.angle_between(Quat::IDENTITY) < 0.01);
    }

    #[test]
    fn a_fixed_joint_holds_and_a_weak_one_breaks() {
        let mut world = world_with(JointKind::Fixed, Vec3::new(2.0, 0.0, 0.0), Quat::IDENTITY, |c| {
            c.breakable = true;
            c.breaking_threshold = 0.2;
        });
        run(&mut world, 1.0);
        assert!((world.bodies[1].position - Vec3::new(2.0, 0.0, 0.0)).length() < 0.05, "holds: {}", world.bodies[1].position);
        let mut weak = world_with(JointKind::Fixed, Vec3::new(2.0, 0.0, 0.0), Quat::IDENTITY, |c| {
            c.breakable = true;
            c.breaking_threshold = 0.05;
        });
        run(&mut weak, 1.0);
        assert!(weak.joints[0].broken && weak.bodies[1].position.z < -3.0, "breaks and falls: {}", weak.bodies[1].position);
    }

    #[test]
    fn a_generic_spring_bobs_about_where_it_balances() {
        let mut world = world_with(JointKind::GenericSpring, Vec3::new(0.0, 0.0, -1.0), Quat::IDENTITY, |c| {
            c.linear_limits = [Some([0.0, 0.0]), Some([0.0, 0.0]), None];
            c.angular_limits = [Some([0.0, 0.0]); 3];
            c.linear_springs = [None, None, Some(10.0)];
        });
        let mut heights = Vec::new();
        for _ in 0..(240 * 4) {
            world.step(1.0 / 240.0);
            heights.push(world.bodies[1].position.z);
        }
        // k = 10, m = 1: a period of 2 s about -1 - 0.981; the reference's first low is -2.725.
        let first_low = heights.iter().take(360).copied().fold(f32::INFINITY, f32::min);
        assert!((first_low + 2.725).abs() < 0.12, "{first_low}");
        let last = heights.iter().rev().take(480).sum::<f32>() / 480.0;
        assert!((last + 1.981).abs() < 0.1, "{last}");
    }
}
