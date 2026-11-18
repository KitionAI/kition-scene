//! Soft bodies: a mesh's vertices with mass, held by springs along its edges (pull when
//! stretched, push when squeezed) and between vertices two edges apart (bending), drawn towards
//! where the mesh puts them by goal springs, slowed by the air, pushed by gravity and force
//! fields, and kept out of collision objects (bouncing as their damping allows) and, when asked,
//! out of each other.
//!
//! Time runs in frames, as in the reference application, whose soft bodies move the same per
//! frame whatever the frame rate (a frame stands for 1/√1000 s of its gravity: measured). The
//! settings' scales were measured against it (`tests/soft.rs`): a spring of pull `p` holds with
//! `p / (1 - p)` per unit of strain, a goal of stiffness `s` and weight `w` with `S / (1 - S)`
//! per metre where `S = s·w⁴`, bending with its setting per unit of strain; each spring also
//! loses `0.0489` of its stiffness as damping, and friction, goal damping, and edge damping slow
//! by `0.01` per unit. Steps are semi-implicit Euler, at least ten a frame and more for stiff
//! springs.

use std::collections::BTreeMap;

use glam::Vec3;
use kition_scene_model::physics::Effector;

use crate::cloth::ColliderSurface;

/// A frame stands for this many seconds of gravity (the reference's soft bodies, measured).
const GRAVITY_PER_FRAME: f32 = 0.001;
/// Damping per unit of friction, goal damping, and edge damping, per frame.
const DAMPING: f32 = 0.01;
/// Damping each spring has from its own stiffness (frames).
const STIFFNESS_DAMPING: f32 = 0.0489;
/// How hard fields push, per unit of strength and per frame squared, and how fast their flow
/// slows a vertex, per unit of flow and per frame (measured on a grid in a Force and a Wind).
const FIELD_PUSH: f32 = 0.0048;
const FIELD_FLOW: f32 = 0.115;
/// Speed kept on hitting a collision object: this, less `BOUNCE_DAMPING` per unit of its
/// damping (none from 0.26 up).
const BOUNCE: f32 = 0.58;
const BOUNCE_DAMPING: f32 = 2.2;
/// The fewest and most steps a frame.
const MIN_STEPS: u32 = 10;
const MAX_STEPS: u32 = 300;
/// Each step moves a vertex at most this share of its fastest spring's swing.
const STEP_SWING: f32 = 0.5;

/// Soft body settings the solver uses (the reference's names).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoftSettings {
    /// Each vertex's mass, kg.
    pub mass: f32,
    /// Air friction.
    pub friction: f32,
    /// Frames of simulated time per frame.
    pub speed: f32,
    /// Goal springs' stiffness and damping.
    pub goal_stiffness: f32,
    pub goal_damping: f32,
    /// Edge springs: stretched, squeezed, their damping, and bending between vertices two edges
    /// apart.
    pub pull: f32,
    pub push: f32,
    pub edge_damping: f32,
    pub bending: f32,
    /// Self collision: each vertex is a ball this share of its edges' mean length across.
    pub ball_size: Option<f32>,
    /// Gravity, m/s².
    pub gravity: Vec3,
}

#[derive(Debug, Clone, Copy)]
struct Spring {
    a: usize,
    b: usize,
    rest: f32,
    /// Stiffness per unit of strain when stretched and when squeezed.
    pull: f32,
    push: f32,
    /// Damping of the ends' speed apart.
    damping: f32,
}

/// A collision object's surface and how it takes a soft body's hits.
#[derive(Debug, Clone)]
pub struct SoftCollider {
    pub surface: ColliderSurface,
    /// The collision object's damping (0: bouncy, 0.26 and up: no bounce).
    pub damping: f32,
}

/// A simulated soft body, in world coordinates.
#[derive(Debug, Clone)]
pub struct SoftSim {
    failed: bool,
    pub positions: Vec<Vec3>,
    velocities: Vec<Vec3>,
    /// Goal stiffness per vertex (zero without a goal), and which vertices follow it exactly.
    goal: Vec<f32>,
    pinned: Vec<bool>,
    springs: Vec<Spring>,
    /// Ball radii for self collision, and the springs' pairs (which never collide).
    radii: Vec<f32>,
    joined: std::collections::BTreeSet<(usize, usize)>,
    settings: SoftSettings,
    steps: u32,
    /// The springs' stiffness is scaled by this to stay within the most steps.
    softened: f32,
}

/// A goal weight's stiffness (`None`: the vertex follows its goal exactly).
pub fn goal_stiffness(stiffness: f32, weight: f32) -> Option<f32> {
    if weight >= 0.9999 {
        return None;
    }
    let share = (stiffness * weight.max(0.0).powi(4)).clamp(0.0, 0.999);
    Some(share / (1.0 - share))
}

fn spring_stiffness(setting: f32) -> f32 {
    let share = setting.clamp(0.0, 0.999);
    share / (1.0 - share)
}

impl SoftSim {
    /// A soft body over `positions` (world) joined along `edges`; `weights` are the vertices'
    /// goal weights (none: no goal).
    pub fn new(positions: Vec<Vec3>, edges: &[[usize; 2]], weights: Option<&[f32]>, settings: SoftSettings) -> Self {
        let count = positions.len();
        let at = |index: usize| positions.get(index).copied().unwrap_or(Vec3::ZERO);
        let (pull, push) = (spring_stiffness(settings.pull), spring_stiffness(settings.push));
        let damping = DAMPING * settings.edge_damping;
        let mut joined = std::collections::BTreeSet::new();
        let mut springs = Vec::new();
        let mut neighbours: Vec<Vec<usize>> = vec![Vec::new(); count];
        for [a, b] in edges {
            let (a, b) = (*a.min(b), *a.max(b));
            if a == b || b >= count || !joined.insert((a, b)) {
                continue;
            }
            let rest = at(a).distance(at(b));
            if rest > 1e-6 {
                springs.push(Spring { a, b, rest, pull, push, damping });
            }
            if let Some(first) = neighbours.get_mut(a) {
                first.push(b);
            }
            if let Some(second) = neighbours.get_mut(b) {
                second.push(a);
            }
        }
        // Bending: every vertex to the vertices two edges away it is not joined to.
        if settings.bending > 0.0 {
            let mut bends: BTreeMap<(usize, usize), f32> = BTreeMap::new();
            for (vertex, around) in neighbours.iter().enumerate() {
                for middle in around {
                    for far in neighbours.get(*middle).map(Vec::as_slice).unwrap_or_default() {
                        let key = (vertex.min(*far), vertex.max(*far));
                        if *far != vertex && !joined.contains(&key) {
                            bends.entry(key).or_insert_with(|| at(key.0).distance(at(key.1)));
                        }
                    }
                }
            }
            for ((a, b), rest) in bends {
                if rest > 1e-6 {
                    springs.push(Spring { a, b, rest, pull: settings.bending, push: settings.bending, damping });
                }
            }
        }
        let (goal, pinned): (Vec<f32>, Vec<bool>) = match weights {
            Some(weights) => (0..count)
                .map(|index| match goal_stiffness(settings.goal_stiffness, weights.get(index).copied().unwrap_or(0.0)) {
                    Some(stiffness) => (stiffness, false),
                    None => (0.0, true),
                })
                .unzip(),
            None => (vec![0.0; count], vec![false; count]),
        };
        let radii = match settings.ball_size {
            Some(size) => (0..count)
                .map(|index| {
                    let around = neighbours.get(index).map(Vec::as_slice).unwrap_or_default();
                    let mean = around.iter().map(|other| at(index).distance(at(*other))).sum::<f32>() / around.len().max(1) as f32;
                    size * mean * 0.5
                })
                .collect(),
            None => Vec::new(),
        };
        let mut sim = Self {
            failed: false,
            velocities: vec![Vec3::ZERO; count],
            positions,
            goal,
            pinned,
            springs,
            radii,
            joined,
            settings,
            steps: MIN_STEPS,
            softened: 1.0,
        };
        sim.plan_steps();
        sim
    }

    /// Steps a frame: enough for the stiffest vertex to swing at most `STEP_SWING` a step, from
    /// ten to the most (stiffer settings are softened to what the most steps can follow).
    fn plan_steps(&mut self) {
        let mut stiff = self.goal.clone();
        for spring in &self.springs {
            let stiffness = spring.pull.max(spring.push) / spring.rest;
            for end in [spring.a, spring.b] {
                if let Some(slot) = stiff.get_mut(end) {
                    *slot += stiffness;
                }
            }
        }
        let mass = self.settings.mass.max(1e-4);
        let largest = stiff.iter().copied().fold(0.0f32, f32::max);
        let speed = self.settings.speed.max(0.0);
        let swing = (2.0 * largest / mass).sqrt() * speed;
        let least = (MIN_STEPS as f32 * speed).ceil().max(1.0);
        let wanted = (swing / STEP_SWING).ceil().max(least);
        if wanted.is_finite() && wanted <= MAX_STEPS as f32 {
            self.steps = wanted as u32;
            self.softened = 1.0;
        } else {
            self.steps = MAX_STEPS;
            let fits = MAX_STEPS as f32 * STEP_SWING;
            self.softened = if swing > 0.0 && swing.is_finite() { (fits / swing).powi(2) } else { 1.0 };
        }
    }

    /// The same body started again at rest at `positions`.
    pub fn restarted(&self, positions: &[Vec3]) -> Self {
        let mut again = self.clone();
        again.failed = false;
        if positions.len() == again.positions.len() {
            again.positions = positions.to_vec();
        }
        again.velocities = vec![Vec3::ZERO; again.positions.len()];
        again
    }

    /// Steps a frame takes.
    pub fn steps(&self) -> u32 {
        self.steps
    }

    /// The vertices' velocities (metres per frame).
    pub fn velocities(&self) -> &[Vec3] {
        &self.velocities
    }

    pub(crate) fn is_finite(&self) -> bool {
        !self.failed && self.positions.iter().chain(&self.velocities).all(|value| value.is_finite())
    }

    /// One frame: the goals move from `from` to `to` (world positions of the mesh as animated)
    /// over it.
    pub fn step_frame(&mut self, from: &[Vec3], to: &[Vec3], colliders: &[SoftCollider], effectors: &[Effector]) {
        let speed = self.settings.speed;
        if !(speed.is_finite() && speed > 0.0) || self.positions.is_empty() {
            return;
        }
        let h = speed / self.steps as f32;
        let mass = self.settings.mass.max(1e-4);
        let inverse_mass = 1.0 / mass;
        let gravity = self.settings.gravity * GRAVITY_PER_FRAME;
        let friction = DAMPING * self.settings.friction;
        let goal_damping = DAMPING * self.settings.goal_damping;
        let soft = self.softened;
        let goal_speed: Vec<Vec3> = from.iter().zip(to).map(|(a, b)| (*b - *a) / speed).collect();
        let count = self.positions.len();
        let mut forces = vec![Vec3::ZERO; count];
        for step in 1..=self.steps {
            let t = step as f32 / self.steps as f32;
            let goal_at = |index: usize| match (from.get(index), to.get(index)) {
                (Some(a), Some(b)) => Some(a.lerp(*b, t)),
                _ => None,
            };
            forces.iter_mut().for_each(|force| *force = Vec3::ZERO);
            for spring in &self.springs {
                let (Some(&pa), Some(&pb)) = (self.positions.get(spring.a), self.positions.get(spring.b)) else { continue };
                let delta = pb - pa;
                let length = delta.length();
                if length < 1e-9 {
                    continue;
                }
                let along = delta / length;
                let strain = (length - spring.rest) / spring.rest;
                let stiffness = if strain > 0.0 { spring.pull } else { spring.push } * soft;
                let apart = match (self.velocities.get(spring.a), self.velocities.get(spring.b)) {
                    (Some(va), Some(vb)) => (*vb - *va).dot(along),
                    _ => 0.0,
                };
                let damping = STIFFNESS_DAMPING * stiffness / spring.rest + spring.damping;
                let force = along * (stiffness * strain + damping * apart);
                if let Some(slot) = forces.get_mut(spring.a) {
                    *slot += force;
                }
                if let Some(slot) = forces.get_mut(spring.b) {
                    *slot -= force;
                }
            }
            for index in 0..count {
                let (Some(position), Some(velocity)) = (self.positions.get(index).copied(), self.velocities.get(index).copied()) else { continue };
                let goal = goal_at(index);
                if self.pinned.get(index).copied().unwrap_or(false) {
                    if let (Some(goal), Some(slot), Some(speed)) = (goal, self.positions.get_mut(index), self.velocities.get_mut(index)) {
                        *slot = goal;
                        *speed = goal_speed.get(index).copied().unwrap_or(Vec3::ZERO);
                    }
                    continue;
                }
                let mut force = forces.get(index).copied().unwrap_or(Vec3::ZERO) - velocity * friction;
                let stiffness = self.goal.get(index).copied().unwrap_or(0.0) * soft;
                if stiffness > 0.0
                    && let Some(goal) = goal
                {
                    let relative = velocity - goal_speed.get(index).copied().unwrap_or(Vec3::ZERO);
                    force += (goal - position) * stiffness - relative * (goal_damping + STIFFNESS_DAMPING * stiffness);
                }
                let mut acceleration = gravity + force * inverse_mass;
                for effector in effectors {
                    // Fields push in the reference's units per second; a frame is 1/√1000 s.
                    let push = effector.push(position, velocity * 1000f32.sqrt());
                    acceleration += push.force * (FIELD_PUSH * inverse_mass) - velocity * (push.flow * FIELD_FLOW);
                }
                let velocity = velocity + acceleration * h;
                if let (Some(slot), Some(speed)) = (self.positions.get_mut(index), self.velocities.get_mut(index)) {
                    *speed = velocity;
                    *slot = position + velocity * h;
                }
            }
            if !self.is_finite() {
                self.failed = true;
                self.positions = to.to_vec();
                self.velocities = vec![Vec3::ZERO; self.positions.len()];
                return;
            }
            if !self.radii.is_empty() {
                self.separate();
            }
            for collider in colliders {
                let keep = (BOUNCE - BOUNCE_DAMPING * collider.damping).max(0.0);
                for index in 0..count {
                    if self.pinned.get(index).copied().unwrap_or(false) {
                        continue;
                    }
                    let (Some(position), Some(velocity)) = (self.positions.get(index).copied(), self.velocities.get(index).copied()) else {
                        continue;
                    };
                    let Some((target, normal)) = collider.surface.contact(position, position - velocity * h, 0.0) else { continue };
                    let into = velocity.dot(normal);
                    let mut velocity = velocity;
                    if into < 0.0 {
                        velocity -= normal * (into * (1.0 + keep));
                        // Friction slows the slide along the surface.
                        let along = velocity - normal * velocity.dot(normal);
                        velocity -= along * (1.0 - 1.0 / (1.0 + collider.surface.friction * 0.2));
                    }
                    if let (Some(slot), Some(speed)) = (self.positions.get_mut(index), self.velocities.get_mut(index)) {
                        *slot = target;
                        *speed = velocity;
                    }
                }
            }
        }
        // A setting past what the steps can follow never leaves a broken body: it stops.
        if self.positions.iter().chain(&self.velocities).any(|value| !value.is_finite()) {
            self.failed = true;
            self.positions = to.to_vec();
            self.velocities = vec![Vec3::ZERO; self.positions.len()];
        }
    }

    /// Keeps vertices that no spring joins their balls' width apart.
    fn separate(&mut self) {
        let largest = self.radii.iter().copied().fold(0.0f32, f32::max);
        if largest <= 0.0 {
            return;
        }
        let cell = largest * 2.0;
        let key = |point: Vec3| ((point.x / cell).floor() as i64, (point.y / cell).floor() as i64, (point.z / cell).floor() as i64);
        let mut grid: BTreeMap<(i64, i64, i64), Vec<usize>> = BTreeMap::new();
        for (index, point) in self.positions.iter().enumerate() {
            grid.entry(key(*point)).or_default().push(index);
        }
        for index in 0..self.positions.len() {
            let Some(point) = self.positions.get(index).copied() else { continue };
            let (x, y, z) = key(point);
            let mut near = Vec::new();
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        // Finite simulated positions can exceed the integer cell range before
                        // a later integration step diverges. Outside cells have no entries.
                        let (Some(nx), Some(ny), Some(nz)) = (x.checked_add(dx), y.checked_add(dy), z.checked_add(dz)) else { continue };
                        if let Some(found) = grid.get(&(nx, ny, nz)) {
                            near.extend(found.iter().copied().filter(|other| *other > index));
                        }
                    }
                }
            }
            for other in near {
                if self.joined.contains(&(index, other)) {
                    continue;
                }
                let (Some(a), Some(b)) = (self.positions.get(index).copied(), self.positions.get(other).copied()) else { continue };
                let reach = self.radii.get(index).copied().unwrap_or(0.0) + self.radii.get(other).copied().unwrap_or(0.0);
                let offset = b - a;
                let distance = offset.length();
                if distance >= reach || distance < 1e-9 {
                    continue;
                }
                let normal = offset / distance;
                let free = |vertex: usize| !self.pinned.get(vertex).copied().unwrap_or(false);
                let (move_a, move_b) = match (free(index), free(other)) {
                    (true, true) => (0.5, 0.5),
                    (true, false) => (1.0, 0.0),
                    (false, true) => (0.0, 1.0),
                    (false, false) => continue,
                };
                let depth = reach - distance;
                let (va, vb) = (self.velocities.get(index).copied().unwrap_or(Vec3::ZERO), self.velocities.get(other).copied().unwrap_or(Vec3::ZERO));
                // The two stop closing in on each other.
                let closing = (vb - va).dot(normal).min(0.0);
                if let Some(slot) = self.positions.get_mut(index) {
                    *slot = a - normal * (depth * move_a);
                }
                if let Some(slot) = self.positions.get_mut(other) {
                    *slot = b + normal * (depth * move_b);
                }
                if let Some(slot) = self.velocities.get_mut(index) {
                    *slot = va + normal * (closing * move_a);
                }
                if let Some(slot) = self.velocities.get_mut(other) {
                    *slot = vb - normal * (closing * move_b);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> SoftSettings {
        SoftSettings {
            mass: 1.0,
            friction: 0.5,
            speed: 1.0,
            goal_stiffness: 0.5,
            goal_damping: 0.0,
            pull: 0.5,
            push: 0.5,
            edge_damping: 0.5,
            bending: 0.0,
            ball_size: None,
            gravity: Vec3::new(0.0, 0.0, -9.81),
        }
    }

    fn run(sim: &mut SoftSim, goals: &[Vec3], frames: usize) -> Vec<Vec<Vec3>> {
        let mut out = vec![sim.positions.clone()];
        for _ in 1..frames {
            sim.step_frame(goals, goals, &[], &[]);
            out.push(sim.positions.clone());
        }
        out
    }

    #[test]
    fn a_free_vertex_falls_a_frame_at_a_time_whatever_the_frame_rate() {
        let start = vec![Vec3::ZERO];
        let mut sim = SoftSim::new(start.clone(), &[], None, SoftSettings { friction: 0.0, ..settings() });
        let frames = run(&mut sim, &start, 48);
        // The reference: -0.0054 after a frame, -2.83706 after 24, -10.85822 after 47.
        assert!((frames[1][0].z + 0.0054).abs() < 5e-5, "{}", frames[1][0].z);
        assert!((frames[24][0].z + 2.83706).abs() < 1e-3, "{}", frames[24][0].z);
        assert!((frames[47][0].z + 10.85822).abs() < 1e-3, "{}", frames[47][0].z);
    }

    #[test]
    fn a_goal_holds_a_vertex_on_a_spring_and_a_full_weight_pins_it() {
        let start = vec![Vec3::ZERO, Vec3::X];
        let mut sim = SoftSim::new(start.clone(), &[], Some(&[0.7, 1.0]), settings());
        let frames = run(&mut sim, &start, 240);
        // Weight 0.7, stiffness 0.5: S = 0.12005, 136.4 per metre: it settles 0.0719 m down.
        let tail: f32 = frames[120..].iter().map(|frame| frame[0].z).sum::<f32>() / 120.0;
        assert!((tail + 0.0719).abs() < 2e-3, "{tail}");
        assert!(frames.iter().all(|frame| frame[1] == Vec3::X), "the pinned vertex stays");
    }

    #[test]
    fn edges_hold_a_hanging_vertex_by_their_strain() {
        // One vertex pinned, one hanging 1 m below on an edge of pull 0.5: 9.81/1000 a frame²
        // held by 1 per unit of strain stretches it 9.81 mm.
        let start = vec![Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0)];
        let mut sim = SoftSim::new(start.clone(), &[[0, 1]], Some(&[1.0, 0.0]), SoftSettings { friction: 5.0, ..settings() });
        let frames = run(&mut sim, &start, 240);
        let last = frames.last().unwrap();
        assert!((last[1].z + 1.00981).abs() < 1e-3, "{}", last[1].z);
    }

    #[test]
    fn a_floor_stops_a_falling_body_and_damping_takes_its_bounce() {
        let floor = |damping: f32| SoftCollider {
            surface: ColliderSurface::new(
                vec![
                    [Vec3::new(-5.0, -5.0, 0.0), Vec3::new(5.0, -5.0, 0.0), Vec3::new(5.0, 5.0, 0.0)],
                    [Vec3::new(-5.0, -5.0, 0.0), Vec3::new(5.0, 5.0, 0.0), Vec3::new(-5.0, 5.0, 0.0)],
                ],
                0.02,
                5.0,
            ),
            damping,
        };
        let bounce = |damping: f32| {
            let start = vec![Vec3::new(0.0, 0.0, 1.0)];
            let mut sim = SoftSim::new(start.clone(), &[], None, settings());
            let mut heights = Vec::new();
            for _ in 0..60 {
                sim.step_frame(&start, &start, &[floor(damping)], &[]);
                heights.push(sim.positions[0].z);
            }
            // The first touch: the lowest of the first 20 frames (the fall takes 14).
            let first = (0..20).min_by(|a, b| heights[*a].total_cmp(&heights[*b])).unwrap();
            (heights[first..].iter().copied().fold(0.0f32, f32::max), *heights.last().unwrap())
        };
        let (lively, rest) = bounce(0.0);
        let (dead, dead_rest) = bounce(0.5);
        // The reference bounces back to 0.321 m.
        assert!((lively - 0.321).abs() < 0.02, "bounces back up: {lively}");
        assert!(dead < 0.03, "no bounce: {dead}");
        assert!((rest - 0.02).abs() < 1e-3 && (dead_rest - 0.02).abs() < 1e-3, "rests on the floor's thickness: {rest} {dead_rest}");
    }

    #[test]
    fn self_collision_keeps_vertices_apart() {
        // Two separate edges, one falling onto the other (held by full goals).
        let start = vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.3), Vec3::new(1.0, 0.0, 0.3)];
        let edges = [[0, 1], [2, 3]];
        let weights = [1.0, 1.0, 0.0, 0.0];
        let drop = |ball: Option<f32>| {
            let mut sim = SoftSim::new(start.clone(), &edges, Some(&weights), SoftSettings { ball_size: ball, friction: 5.0, ..settings() });
            run(&mut sim, &start, 120).last().unwrap()[2].z
        };
        let apart = drop(Some(0.49));
        let through = drop(None);
        assert!(apart > 0.4, "rests on the lower edge, a ball's width up: {apart}");
        assert!(through < -0.5, "falls through without: {through}");
    }

    #[test]
    fn settings_past_the_steps_soften_and_never_break() {
        let start = vec![Vec3::ZERO, Vec3::new(0.0, 0.0, -0.001)];
        let mut sim = SoftSim::new(start.clone(), &[[0, 1]], Some(&[1.0, 0.0]), SoftSettings { pull: 0.999, mass: 1e-4, ..settings() });
        assert_eq!(sim.steps(), MAX_STEPS);
        let frames = run(&mut sim, &start, 30);
        assert!(frames.iter().flatten().all(|point| point.is_finite()));
    }
}
