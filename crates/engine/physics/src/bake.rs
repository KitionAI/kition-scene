//! Baking: runs a scene's rigid bodies, cloths, soft bodies, and dynamic hair over the physics
//! world's frame range and records every frame for the scene to show
//! (`kition_scene_model::physics::Baked`). A bake can run a few frames at a time (the editor
//! bakes in steps so the page stays responsive).

use std::collections::BTreeMap;

use glam::{Mat4, Quat, Vec3};
use kition_scene_model::physics::{Baked, BodyKind, BodyTrack, CollisionShape, Effector};
use kition_scene_model::{Id, Mesh, Scene};

use crate::cloth::{ClothSettings, ClothSim, ColliderSurface, FIELD_SCALE};
use crate::hair::{HairSettings, HairSim};
use crate::joint::Joint;
use crate::rigid::{Body, Motion, World};
use crate::shape::{Poly, Shape, TriangleMesh, hull};
use crate::soft::{SoftCollider, SoftSettings, SoftSim};

/// The most frames one bake records.
const MAX_FRAMES: i32 = 10_000;
/// Cloth steps per quality step, and solver iterations per step.
const CLOTH_SUBSTEPS: u32 = 4;
const CLOTH_ITERATIONS: u32 = 4;
/// Hair steps per quality step, and solver iterations per step (as cloth's).
const HAIR_SUBSTEPS: u32 = 4;
const HAIR_ITERATIONS: u32 = 4;

struct BodyRun {
    id: Id,
    index: usize,
    scale: Vec3,
    kinematic: bool,
}

struct ClothRun {
    effector_weights: kition_scene_model::physics::EffectorWeights,
    id: Id,
    sim: ClothSim,
    pinned: Vec<usize>,
    quality: u32,
    /// The mesh as the scene has it before modifiers (for pins and for the object's frame).
    moves: bool,
}

struct SoftRun {
    effector_weights: kition_scene_model::physics::EffectorWeights,
    id: Id,
    sim: SoftSim,
    /// The mesh as the scene has it before modifiers (object space), and where it puts the
    /// vertices in the world at the frame last run (the goals).
    local: Vec<Vec3>,
    goals: Vec<Vec3>,
    /// The soft body's own frames; before the first the mesh is at rest, after the last it holds.
    first: i32,
    last: i32,
    /// Moved by animation (the goals follow it).
    moves: bool,
}

struct HairRun {
    id: Id,
    index: usize,
    sim: HairSim,
    /// The parent strands at rest, in the emitter's own space, and in the world at the frame last
    /// run.
    local: Vec<Vec<Vec3>>,
    rest: Vec<Vec<Vec3>>,
    quality: u32,
    moves: bool,
}

struct ColliderRun {
    id: Id,
    thickness: f32,
    friction: f32,
    damping: f32,
    /// Its triangles in its own space (as at the start).
    local: Vec<[Vec3; 3]>,
    /// Moves during the bake (animated, or a simulated body).
    moves: bool,
    fixed: Option<ColliderSurface>,
}

/// A bake under way.
pub struct Bake {
    failure: Option<String>,
    base: Scene,
    start: i32,
    end: i32,
    substeps: u32,
    frame_time: f32,
    world: World,
    bodies: Vec<BodyRun>,
    cloths: Vec<ClothRun>,
    softs: Vec<SoftRun>,
    hairs: Vec<HairRun>,
    colliders: Vec<ColliderRun>,
    /// The force fields, as they are at the frame being run.
    effectors: Vec<Effector>,
    /// Frames still to run that need the scene evaluated at each frame.
    animated: bool,
    frame: i32,
    out: Baked,
}

/// The world matrix split into its pose and its scale.
fn pose_of(world: Mat4) -> (Vec3, Quat, Vec3) {
    let (scale, rotation, translation) = world.to_scale_rotation_translation();
    (translation, rotation.normalize(), scale)
}

/// The mesh a simulation starts from: the evaluated mesh for rigid bodies and colliders, the
/// mesh with shape keys (no modifiers) for cloth.
fn triangles(mesh: &Mesh) -> Vec<[Vec3; 3]> {
    mesh.triangles()
        .into_iter()
        .filter_map(|(triangle, _)| {
            let corner = |index: u32| mesh.positions.get(index as usize).copied();
            Some([corner(triangle[0])?, corner(triangle[1])?, corner(triangle[2])?])
        })
        .collect()
}

/// A rigid body's collision shape from its mesh's vertices in the body's frame.
fn shape_for(kind: CollisionShape, passive: bool, local: &[Vec3], local_triangles: Vec<[Vec3; 3]>, margin: f32) -> Option<Shape> {
    let (low, high) =
        local.iter().fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(low, high), point| (low.min(*point), high.max(*point)));
    if low.x > high.x {
        return None;
    }
    let size = high - low;
    // Centred on the origin, as the reference makes primitive shapes.
    let half = size / 2.0;
    match kind {
        CollisionShape::Box => Poly::cuboid((low + high) / 2.0, half.max(Vec3::splat(margin.max(0.005)))).map(Shape::Poly),
        CollisionShape::Sphere => Some(Shape::Sphere { radius: half.max_element().max(0.005) }),
        CollisionShape::Capsule | CollisionShape::Cylinder => {
            let radius = half.x.max(half.y).max(0.005);
            let mut points = Vec::new();
            let around = 24;
            if kind == CollisionShape::Cylinder {
                for z in [-half.z, half.z] {
                    for step in 0..around {
                        let angle = std::f32::consts::TAU * step as f32 / around as f32;
                        points.push(Vec3::new(radius * angle.cos(), radius * angle.sin(), z));
                    }
                }
            } else {
                let length = (half.z - radius).max(0.0);
                for (centre, sign) in [(length, 1.0f32), (-length, -1.0f32)] {
                    for ring in 0..=4 {
                        let lift = std::f32::consts::FRAC_PI_2 * ring as f32 / 4.0;
                        for step in 0..around {
                            let angle = std::f32::consts::TAU * step as f32 / around as f32;
                            points.push(Vec3::new(
                                radius * lift.cos() * angle.cos(),
                                radius * lift.cos() * angle.sin(),
                                centre + sign * radius * lift.sin(),
                            ));
                        }
                    }
                }
            }
            hull(&points).map(Shape::Poly)
        }
        CollisionShape::Mesh if passive => Some(Shape::Mesh(TriangleMesh::new(local_triangles))),
        CollisionShape::Mesh | CollisionShape::ConvexHull => match hull(local) {
            Some(poly) => Some(Shape::Poly(poly)),
            None => {
                // A flat mesh (a plane): give it a skin below its face, so it has a volume.
                let normal = local_triangles.iter().map(|[a, b, c]| (*b - *a).cross(*c - *a)).sum::<Vec3>().normalize_or(Vec3::Z);
                let depth = margin.max(0.01);
                let mut points = local.to_vec();
                points.extend(local.iter().map(|point| *point - normal * depth));
                hull(&points).map(Shape::Poly)
            }
        },
    }
}

impl Bake {
    /// Sets up a bake of `scene`'s physics over its world's frame range.
    pub fn new(scene: &Scene) -> Result<Self, String> {
        let mut base = scene.clone();
        base.physics_cache = Default::default();
        base.physics.check().map_err(|error| error.to_string())?;
        let world_settings = base.physics.clone();
        let start = world_settings.frame_start;
        let end = world_settings.frame_end.min(start + MAX_FRAMES);
        let fps = base.animation.timeline.fps.max(1.0);
        let frame_time = world_settings.time_scale / fps;
        let key = base.physics_key();
        let first = base.at_frame(start as f32);
        let mut world = World::new(world_settings.gravity, world_settings.iterations);
        let mut bodies = Vec::new();
        let mut cloths = Vec::new();
        let mut softs = Vec::new();
        let mut hairs = Vec::new();
        let mut colliders = Vec::new();
        let mut animated = false;
        let moves_by_animation = |id: Id| -> bool {
            let mut current = Some(id);
            let mut depth = 0;
            while let Some(at) = current {
                if base.animation.channels.iter().any(|channel| channel.target == at) {
                    return true;
                }
                current = base.object(at).ok().and_then(|object| object.parent);
                depth += 1;
                if depth > base.objects.len() {
                    break;
                }
            }
            false
        };
        for object in first.simulated_objects() {
            if object.hidden {
                continue;
            }
            let Ok(world_matrix) = first.world_matrix(object.id) else { continue };
            for (index, system) in object.particles.iter().enumerate() {
                let Some(dynamics) = system.dynamics.as_ref().filter(|_| system.kind == kition_scene_model::particles::Kind::Hair) else {
                    continue;
                };
                let Some(surface) = kition_scene_model::particles::system_surface(&first, object.id, system, false) else { continue };
                let local = kition_scene_model::particles::hair::parents(system, &surface);
                let rest: Vec<Vec<Vec3>> =
                    local.iter().map(|strand| strand.iter().map(|point| world_matrix.transform_point3(*point)).collect()).collect();
                let settings = HairSettings {
                    mass: dynamics.mass,
                    stiffness: dynamics.stiffness,
                    damping: dynamics.damping,
                    air_damping: dynamics.air_damping,
                    pin_stiffness: dynamics.pin_stiffness,
                    collision_distance: dynamics.collision.then_some(dynamics.collision_distance),
                    gravity: world_settings.gravity,
                };
                let moves = moves_by_animation(object.id);
                animated |= moves;
                hairs.push(HairRun {
                    id: object.id,
                    index,
                    sim: HairSim::new(rest.clone(), settings),
                    local,
                    rest,
                    quality: dynamics.quality,
                    moves,
                });
            }
            let Some(physics) = object.physics.clone() else { continue };
            let (position, rotation, scale) = pose_of(world_matrix);
            let to_local = Mat4::from_rotation_translation(rotation, position).inverse();
            if let Some(body) = &physics.rigid_body {
                let Some(mesh) = object.evaluated_mesh(&first) else { continue };
                let local: Vec<Vec3> = mesh.positions.iter().map(|point| to_local.transform_point3(world_matrix.transform_point3(*point))).collect();
                let local_triangles: Vec<[Vec3; 3]> =
                    triangles(&mesh).into_iter().map(|t| t.map(|p| to_local.transform_point3(world_matrix.transform_point3(p)))).collect();
                let passive = body.kind == BodyKind::Passive;
                let Some(shape) = shape_for(body.shape, passive, &local, local_triangles, body.margin) else { continue };
                let kinematic = passive && body.animated;
                let motion = if kinematic {
                    Motion::Kinematic
                } else if passive || !body.enabled {
                    Motion::Static
                } else {
                    Motion::Dynamic
                };
                animated |= kinematic;
                let mut made = Body::new(shape, motion, position, rotation, body.mass);
                made.friction = body.friction;
                made.restitution = body.restitution;
                made.linear_damping = body.linear_damping;
                made.angular_damping = body.angular_damping;
                made.margin = body.margin;
                made.can_sleep = body.deactivation;
                made.sleeping = body.deactivation && body.start_deactivated && motion == Motion::Dynamic;
                made.sleep_linear = body.deactivate_linear;
                made.sleep_angular = body.deactivate_angular;
                bodies.push(BodyRun { id: object.id, index: world.bodies.len(), scale, kinematic });
                world.bodies.push(made);
            }
            if let Some(collision) = &physics.collision {
                let Some(mesh) = object.evaluated_mesh(&first) else { continue };
                let simulated = physics.rigid_body.as_ref().is_some_and(|body| body.kind == BodyKind::Active || body.animated);
                let moves = simulated || moves_by_animation(object.id);
                animated |= moves && !simulated;
                let local: Vec<[Vec3; 3]> =
                    triangles(&mesh).into_iter().map(|t| t.map(|p| to_local.transform_point3(world_matrix.transform_point3(p)))).collect();
                let fixed = (!moves).then(|| {
                    let world_triangles = local.iter().map(|t| t.map(|p| position + rotation * p)).collect();
                    ColliderSurface::new(world_triangles, collision.thickness, collision.friction)
                });
                colliders.push(ColliderRun {
                    id: object.id,
                    thickness: collision.thickness,
                    friction: collision.friction,
                    damping: collision.damping,
                    local,
                    moves,
                    fixed,
                });
            }
            if let Some(cloth) = &physics.cloth {
                cloth.effector_weights.check().map_err(|error| error.to_string())?;
                let Some(mut mesh) = object.base_mesh(scene) else { continue };
                kition_scene_model::vertex_data::apply_shape_keys(&mut mesh);
                let positions: Vec<Vec3> = mesh.positions.iter().map(|point| world_matrix.transform_point3(*point)).collect();
                let pinned: Vec<bool> = match cloth.pin_group.as_ref().and_then(|name| mesh.groups.iter().find(|group| &group.name == name)) {
                    Some(group) => (0..positions.len()).map(|index| group.weights.get(index).copied().unwrap_or(0.0) > 0.5).collect(),
                    None => vec![false; positions.len()],
                };
                let pinned_indices: Vec<usize> = pinned.iter().enumerate().filter(|(_, pin)| **pin).map(|(index, _)| index).collect();
                let moves = moves_by_animation(object.id) && !pinned_indices.is_empty();
                animated |= moves;
                let settings = ClothSettings {
                    mass: cloth.mass,
                    tension: cloth.tension,
                    shear: cloth.shear,
                    bending: cloth.bending,
                    air_damping: cloth.air_damping,
                    gravity: world_settings.gravity * cloth.effector_weights.gravity,
                };
                let mut sim = ClothSim::new(positions, &mesh.faces, pinned, settings, cloth.collision_distance);
                sim.self_distance = cloth.self_collision.then_some(cloth.self_distance);
                cloths.push(ClothRun {
                    effector_weights: cloth.effector_weights,
                    id: object.id,
                    sim,
                    pinned: pinned_indices,
                    quality: cloth.quality,
                    moves,
                });
            }
            // A soft body (an object that is cloth stays cloth).
            if let Some(soft) = physics.soft_body.as_ref().filter(|_| physics.cloth.is_none()) {
                soft.effector_weights.check().map_err(|error| error.to_string())?;
                let Some(mut mesh) = object.base_mesh(scene) else { continue };
                kition_scene_model::vertex_data::apply_shape_keys(&mut mesh);
                let goals: Vec<Vec3> = mesh.positions.iter().map(|point| world_matrix.transform_point3(*point)).collect();
                let edges: Vec<[usize; 2]> =
                    if soft.edges { mesh.edges().into_iter().map(|[a, b]| [a as usize, b as usize]).collect() } else { Vec::new() };
                let weights = soft.goal.then(|| soft.goal_weights(goals.len(), &mesh.groups));
                let settings = SoftSettings {
                    mass: soft.mass,
                    friction: soft.friction,
                    speed: soft.speed,
                    goal_stiffness: soft.goal_stiffness,
                    goal_damping: soft.goal_damping,
                    pull: soft.pull,
                    push: soft.push,
                    edge_damping: soft.damping,
                    bending: soft.bending,
                    ball_size: soft.self_collision.then_some(soft.ball_size),
                    gravity: world_settings.gravity * soft.effector_weights.gravity,
                };
                let moves = moves_by_animation(object.id);
                animated |= moves;
                let sim = SoftSim::new(goals.clone(), &edges, weights.as_deref(), settings);
                softs.push(SoftRun {
                    effector_weights: soft.effector_weights,
                    id: object.id,
                    sim,
                    local: mesh.positions.clone(),
                    goals,
                    first: soft.frame_start.max(start),
                    last: soft.frame_end,
                    moves,
                });
            }
        }
        // Constraints join two rigid bodies at their object's origin, along its axes.
        for object in first.physics_objects() {
            let Some(constraint) = object.physics.as_ref().and_then(|physics| physics.rigid_constraint.clone()) else { continue };
            if object.hidden || !constraint.enabled {
                continue;
            }
            let index_of = |id: Option<Id>| bodies.iter().find(|run: &&BodyRun| Some(run.id) == id).map(|run| run.index);
            let (Some(a), Some(b)) = (index_of(constraint.object1), index_of(constraint.object2)) else { continue };
            if a == b {
                continue;
            }
            let Ok(world_matrix) = first.world_matrix(object.id) else { continue };
            let (pivot, axes, _) = pose_of(world_matrix);
            if let Some(joint) = Joint::new(&world, a, b, pivot, axes, constraint) {
                world.joints.push(joint);
            }
        }
        // Fields: an animated field object makes the bake follow the animation.
        let effectors = first.effectors();
        animated |= effectors.iter().any(|effector| moves_by_animation(effector.owner));
        let mut bake = Self {
            failure: None,
            base,
            start,
            end,
            substeps: world_settings.substeps,
            frame_time,
            world,
            bodies,
            cloths,
            softs,
            hairs,
            colliders,
            effectors,
            animated,
            frame: start,
            out: Baked { key, start, frames: 0, bodies: BTreeMap::new(), cloths: BTreeMap::new(), hair: BTreeMap::new() },
        };
        bake.record(&first, start).map_err(|(id, detail)| format!("failed: physics bake for object {id}: {detail}"))?;
        Ok(bake)
    }

    /// Fields push awake rigid bodies (their push is a force scaled by the frame's time, as the
    /// reference applies it, less their flow's slowing).
    fn push_bodies(&mut self, dt: f32) {
        if self.effectors.is_empty() {
            return;
        }
        for run in &self.bodies {
            let Some(body) = self.world.bodies.get_mut(run.index).filter(|body| body.moves()) else { continue };
            let mut acceleration = Vec3::ZERO;
            for effector in self.effectors.iter().filter(|effector| effector.owner != run.id) {
                let push = effector.push(body.position, body.velocity);
                acceleration += push.force * (self.frame_time * body.inverse_mass) - body.velocity * push.flow;
            }
            body.velocity += acceleration * dt;
        }
    }

    /// Records the current state as the next frame.
    fn record(&mut self, scene: &Scene, frame: i32) -> Result<(), (Id, &'static str)> {
        for run in &self.bodies {
            let Some(body) = self.world.bodies.get(run.index) else { continue };
            self.out
                .bodies
                .entry(run.id)
                .or_insert_with(|| BodyTrack { scale: run.scale, frames: Vec::new() })
                .frames
                .push((body.position, body.rotation));
        }
        for run in &self.cloths {
            let inverse = scene.world_matrix(run.id).map(|world| world.inverse()).unwrap_or(Mat4::IDENTITY);
            let local: Vec<Vec3> = run.sim.positions.iter().map(|point| inverse.transform_point3(*point)).collect();
            if local.iter().any(|point| !point.is_finite()) {
                return Err((run.id, "cloth local positions became nonfinite"));
            }
            self.out.cloths.entry(run.id).or_default().push(local);
        }
        for run in &self.softs {
            let local: Vec<Vec3> = if frame < run.first {
                run.local.clone()
            } else {
                let inverse = scene.world_matrix(run.id).map(|world| world.inverse()).unwrap_or(Mat4::IDENTITY);
                run.sim.positions.iter().map(|point| inverse.transform_point3(*point)).collect()
            };
            if local.iter().any(|point| !point.is_finite()) {
                return Err((run.id, "soft-body local positions became nonfinite"));
            }
            self.out.cloths.entry(run.id).or_default().push(local);
        }
        for run in &self.hairs {
            let inverse = scene.world_matrix(run.id).map(|world| world.inverse()).unwrap_or(Mat4::IDENTITY);
            let local: Vec<Vec<Vec3>> =
                run.sim.strands.iter().map(|strand| strand.iter().map(|point| inverse.transform_point3(*point)).collect()).collect();
            self.out.hair.entry((run.id, run.index)).or_default().push(local);
        }
        self.out.frames += 1;
        Ok(())
    }

    /// The physics key of the scene this bake runs.
    pub fn key(&self) -> u64 {
        self.out.key
    }

    /// Whether every frame is baked or the run failed; check [`Self::error`] before using it.
    pub fn done(&self) -> bool {
        self.failure.is_some() || self.frame >= self.end
    }

    /// A failed bake is derived state only: the scene remains editable and saveable.
    pub fn error(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    fn fail(&mut self, object: Id, detail: &str) -> bool {
        self.failure = Some(format!(
            "failed: physics bake for object {object} at frame {}: {detail}; reduce field strength or effector weights",
            self.frame + 1
        ));
        self.out.frames = 0;
        self.out.cloths.clear();
        self.out.bodies.clear();
        self.out.hair.clear();
        true
    }

    /// Frames baked and frames in all.
    pub fn progress(&self) -> (usize, usize) {
        (self.out.frames, (self.end - self.start + 1).max(1) as usize)
    }

    /// The frames baked so far.
    pub fn result(&self) -> Baked {
        self.out.clone()
    }

    /// Runs up to `frames` more frames; answers whether the bake finished or failed.
    pub fn step(&mut self, frames: usize) -> bool {
        for _ in 0..frames {
            if self.done() {
                break;
            }
            let next_frame = self.frame + 1;
            let next = if self.animated { Some(self.base.at_frame(next_frame as f32)) } else { None };
            // Rigid bodies: kinematic ones follow their animation through the substeps.
            let targets: Vec<(usize, Vec3, Quat, Vec3, Quat)> = match &next {
                Some(scene) => self
                    .bodies
                    .iter()
                    .filter(|run| run.kinematic)
                    .filter_map(|run| {
                        let body = self.world.bodies.get(run.index)?;
                        let (position, rotation, _) = pose_of(scene.world_matrix(run.id).ok()?);
                        Some((run.index, body.position, body.rotation, position, rotation))
                    })
                    .collect(),
                None => Vec::new(),
            };
            if let Some(scene) = &next
                && !self.effectors.is_empty()
            {
                self.effectors = scene.effectors();
            }
            let substeps = self.substeps.max(1);
            let dt = self.frame_time / substeps as f32;
            for step in 1..=substeps {
                let t = step as f32 / substeps as f32;
                for (index, from_position, from_rotation, to_position, to_rotation) in &targets {
                    self.world.drive(*index, from_position.lerp(*to_position, t), from_rotation.slerp(*to_rotation, t), dt);
                }
                self.push_bodies(dt);
                self.world.step(dt);
            }
            // Cloth, soft bodies, and hair, against the collision surfaces where they are at the
            // end of the frame.
            if !(self.cloths.is_empty() && self.softs.is_empty() && self.hairs.is_empty()) {
                let simulated: BTreeMap<Id, (Vec3, Quat)> = self
                    .bodies
                    .iter()
                    .filter_map(|run| self.world.bodies.get(run.index).map(|body| (run.id, (body.position, body.rotation))))
                    .collect();
                let placed: Vec<(Id, f32, ColliderSurface)> = self
                    .colliders
                    .iter()
                    .filter_map(|collider| {
                        if let Some(fixed) = &collider.fixed {
                            return Some((collider.id, collider.damping, fixed.clone()));
                        }
                        let (position, rotation) = match simulated.get(&collider.id) {
                            Some(pose) => *pose,
                            None => {
                                let (position, rotation, _) = pose_of(next.as_ref()?.world_matrix(collider.id).ok()?);
                                (position, rotation)
                            }
                        };
                        let world_triangles = collider.local.iter().map(|t| t.map(|p| position + rotation * p)).collect();
                        let _ = collider.moves;
                        Some((collider.id, collider.damping, ColliderSurface::new(world_triangles, collider.thickness, collider.friction)))
                    })
                    .collect();
                let surfaces: Vec<ColliderSurface> = placed.iter().map(|(_, _, surface)| surface.clone()).collect();
                for run in &mut self.cloths {
                    let pins: Vec<Vec3> = match (&next, run.moves) {
                        (Some(scene), true) => {
                            let world = scene.world_matrix(run.id).unwrap_or(Mat4::IDENTITY);
                            let mesh = scene.object(run.id).ok().and_then(|object| object.base_mesh(scene)).unwrap_or_default();
                            let mut pins = run.sim.positions.clone();
                            for index in &run.pinned {
                                if let (Some(slot), Some(point)) = (pins.get_mut(*index), mesh.positions.get(*index)) {
                                    *slot = world.transform_point3(*point);
                                }
                            }
                            pins
                        }
                        _ => run.sim.positions.clone(),
                    };
                    let steps = run.quality.max(1) * CLOTH_SUBSTEPS;
                    let dt = self.frame_time / steps as f32;
                    let effectors = match run.effector_weights.placed(&self.effectors, run.id) {
                        Ok(value) => value,
                        Err(error) => {
                            let id = run.id;
                            return self.fail(id, &error.to_string());
                        }
                    };
                    for _ in 0..steps {
                        let push = run.sim.field_push(&effectors, FIELD_SCALE * self.frame_time);
                        run.sim.step(dt, CLOTH_ITERATIONS, &pins, &surfaces, &push);
                        if !run.sim.is_finite() {
                            let id = run.id;
                            return self.fail(id, "cloth positions or velocities became nonfinite");
                        }
                    }
                }
                for run in &mut self.softs {
                    let goals = match (&next, run.moves) {
                        (Some(scene), true) => {
                            let world = scene.world_matrix(run.id).unwrap_or(Mat4::IDENTITY);
                            let mut mesh = scene.object(run.id).ok().and_then(|object| object.base_mesh(scene)).unwrap_or_default();
                            kition_scene_model::vertex_data::apply_shape_keys(&mut mesh);
                            if mesh.positions.len() == run.goals.len() {
                                mesh.positions.iter().map(|point| world.transform_point3(*point)).collect()
                            } else {
                                run.goals.clone()
                            }
                        }
                        _ => run.goals.clone(),
                    };
                    if next_frame == run.first {
                        run.sim = run.sim.restarted(&goals);
                    } else if next_frame > run.first && next_frame <= run.last {
                        let colliders: Vec<SoftCollider> = placed
                            .iter()
                            .filter(|(id, _, _)| *id != run.id)
                            .map(|(_, damping, surface)| SoftCollider { surface: surface.clone(), damping: *damping })
                            .collect();
                        let effectors = match run.effector_weights.placed(&self.effectors, run.id) {
                            Ok(value) => value,
                            Err(error) => {
                                let id = run.id;
                                return self.fail(id, &error.to_string());
                            }
                        };
                        run.sim.step_frame(&run.goals, &goals, &colliders, &effectors);
                        if !run.sim.is_finite() {
                            let id = run.id;
                            return self.fail(id, "soft-body positions or velocities became nonfinite");
                        }
                    }
                    run.goals = goals;
                }
                for run in &mut self.hairs {
                    let rest = match (&next, run.moves) {
                        (Some(scene), true) => {
                            let world = scene.world_matrix(run.id).unwrap_or(Mat4::IDENTITY);
                            run.local.iter().map(|strand| strand.iter().map(|point| world.transform_point3(*point)).collect()).collect()
                        }
                        _ => run.rest.clone(),
                    };
                    let steps = run.quality.max(1) * HAIR_SUBSTEPS;
                    let dt = crate::hair::FRAME_TIME / steps as f32;
                    let colliders: Vec<ColliderSurface> =
                        placed.iter().filter(|(id, _, _)| *id != run.id).map(|(_, _, surface)| surface.clone()).collect();
                    let effectors: Vec<Effector> = self.effectors.iter().filter(|effector| effector.owner != run.id).cloned().collect();
                    for step in 1..=steps {
                        let t = step as f32 / steps as f32;
                        let between: Vec<Vec<Vec3>> = if run.moves {
                            run.rest.iter().zip(&rest).map(|(a, b)| a.iter().zip(b).map(|(a, b)| a.lerp(*b, t)).collect()).collect()
                        } else {
                            rest.clone()
                        };
                        run.sim.step(dt, HAIR_ITERATIONS, &between, &colliders, &effectors);
                    }
                    run.rest = rest;
                }
            }
            let shown = match next {
                Some(scene) => scene,
                None => self.base.clone(),
            };
            if let Err((id, detail)) = self.record(&shown, next_frame) {
                return self.fail(id, detail);
            }
            self.frame = next_frame;
        }
        self.done()
    }
}

/// Bakes a scene's physics in one go.
pub fn bake(scene: &Scene) -> Result<Baked, String> {
    let mut run = Bake::new(scene)?;
    while !run.step(16) {}
    match run.error() {
        Some(error) => Err(error.to_string()),
        None => Ok(run.result()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kition_scene_model::physics::{Cloth, Collision, ObjectPhysics, RigidBody};
    use kition_scene_model::{ObjectData, Primitive};

    fn add(scene: &mut Scene, name: &str, primitive: Primitive, location: Vec3, physics: ObjectPhysics) -> Id {
        let mesh = primitive.build().unwrap();
        let id = scene.add_object(name, ObjectData::Mesh(mesh));
        let object = scene.objects.iter_mut().find(|object| object.id == id).unwrap();
        object.location = location;
        object.physics = Some(physics);
        id
    }

    fn body(kind: BodyKind) -> ObjectPhysics {
        ObjectPhysics { rigid_body: Some(RigidBody { kind, ..RigidBody::default() }), ..ObjectPhysics::default() }
    }

    fn ground(scene: &mut Scene) -> Id {
        add(scene, "Ground", Primitive::Plane { size: 20.0 }, Vec3::ZERO, body(BodyKind::Passive))
    }

    #[test]
    fn a_dropped_cube_comes_to_rest_on_the_ground_and_the_scene_shows_it() {
        let mut scene = Scene::empty();
        scene.physics.frame_end = 100;
        ground(&mut scene);
        let cube = add(&mut scene, "Cube", Primitive::Cube { size: 2.0 }, Vec3::new(0.0, 0.0, 4.0), body(BodyKind::Active));
        let baked = bake(&scene).unwrap();
        assert_eq!(baked.frames, 100);
        let track = &baked.bodies[&cube];
        let rest = track.frames.last().unwrap().0;
        assert!((rest.z - 1.0).abs() < 0.06, "rests on the ground: {rest}");
        scene.physics_cache.baked = Some(std::sync::Arc::new(baked));
        assert!(scene.refresh_physics(), "the bake matches the scene");
        let shown = scene.at_frame(100.0);
        let world = shown.world_matrix(cube).unwrap();
        assert!((world.w_axis.z - rest.z).abs() < 1e-4);
        // Moving the cube makes the bake out of date.
        scene.objects.iter_mut().find(|object| object.id == cube).unwrap().location.z = 5.0;
        assert!(!scene.refresh_physics());
    }

    #[test]
    fn bakes_are_deterministic_and_stepping_matches_one_go() {
        let mut scene = Scene::empty();
        scene.physics.frame_end = 40;
        ground(&mut scene);
        for level in 0..3 {
            add(&mut scene, "Box", Primitive::Cube { size: 1.0 }, Vec3::new(0.0, 0.0, 0.5 + level as f32 * 1.01), body(BodyKind::Active));
        }
        let one = bake(&scene).unwrap();
        let mut run = Bake::new(&scene).unwrap();
        while !run.step(3) {}
        let two = run.result();
        assert_eq!(one.bodies, two.bodies);
        assert_eq!(run.progress(), (40, 40));
    }

    #[test]
    fn dynamic_hair_bakes_shows_and_goes_out_of_date_with_its_settings() {
        let mut scene = Scene::empty();
        scene.physics.frame_end = 24;
        let id = scene.add_object("Head", ObjectData::Mesh(Primitive::Plane { size: 0.1 }.build().unwrap()));
        let mut system = kition_scene_model::particles::ParticleSystem::new("Hair", kition_scene_model::particles::Kind::Hair);
        system.count = 4;
        system.segments = 4;
        system.hair_length = 1.0;
        system.dynamics = Some(kition_scene_model::particles::HairDynamics::default());
        let object = scene.object_mut(id).unwrap();
        object.rotation = Vec3::new(std::f32::consts::FRAC_PI_2, 0.0, 0.0);
        object.particles.push(system);
        assert!(scene.physics_objects().next().is_none() && scene.simulated_objects().next().is_some());
        let at_rest = kition_scene_model::particles::strands(&scene, id, 0, true);
        let baked = bake(&scene).unwrap();
        assert_eq!(baked.hair[&(id, 0)].len(), 24);
        scene.physics_cache.baked = Some(std::sync::Arc::new(baked));
        assert!(scene.refresh_physics());
        let drooped = kition_scene_model::particles::strands(&scene.at_frame(24.0), id, 0, true);
        for (rest, now) in at_rest.iter().zip(&drooped) {
            assert!(rest[0].distance(now[0]) < 1e-5, "the root stays");
            assert!(now[4].z < rest[4].z - 0.3, "the tip droops: {} from {}", now[4], rest[4]);
        }
        // Stiffer hair is another bake.
        scene.object_mut(id).unwrap().particles[0].dynamics.as_mut().unwrap().stiffness = 2.0;
        assert!(!scene.refresh_physics());
        let shown = kition_scene_model::particles::strands(&scene.at_frame(24.0), id, 0, true);
        assert_eq!(shown, at_rest, "an out-of-date bake shows nothing");
    }

    #[test]
    fn a_soft_body_rests_before_its_frames_and_holds_after_them() {
        let mut scene = Scene::empty();
        scene.physics.frame_end = 40;
        let soft = kition_scene_model::physics::SoftBody { frame_start: 10, frame_end: 20, ..Default::default() };
        let physics = ObjectPhysics { soft_body: Some(soft), ..ObjectPhysics::default() };
        let cube = add(&mut scene, "Jelly", Primitive::Cube { size: 2.0 }, Vec3::new(0.0, 0.0, 3.0), physics);
        let rest = scene.object(cube).unwrap().mesh().unwrap().positions.clone();
        let baked = bake(&scene).unwrap();
        let frames = &baked.cloths[&cube];
        assert_eq!(frames.len(), 40);
        assert!(frames[..10].iter().all(|frame| *frame == rest), "at rest to frame 10");
        assert!(frames[14] != rest, "simulated from frame 11");
        assert!(frames[20..].iter().all(|frame| *frame == frames[19]), "held from frame 20");
    }

    #[test]
    fn cloth_falls_onto_a_collision_sphere() {
        let mut scene = Scene::empty();
        scene.physics.frame_end = 30;
        let collider = ObjectPhysics { collision: Some(Collision::default()), ..ObjectPhysics::default() };
        add(&mut scene, "Sphere", Primitive::UvSphere { segments: 32, rings: 16, radius: 1.0 }, Vec3::ZERO, collider);
        let cloth_physics = ObjectPhysics { cloth: Some(Cloth::default()), ..ObjectPhysics::default() };
        let cloth =
            add(&mut scene, "Cloth", Primitive::Grid { x_subdivisions: 20, y_subdivisions: 20, size: 3.0 }, Vec3::new(0.0, 0.0, 1.5), cloth_physics);
        let baked = bake(&scene).unwrap();
        let last = baked.cloths[&cloth].last().unwrap();
        // Object space: the cloth sits at z = 1.5, the sphere's top at -0.5 below it.
        let centre = last.iter().min_by(|a, b| (a.x * a.x + a.y * a.y).total_cmp(&(b.x * b.x + b.y * b.y))).unwrap();
        assert!(centre.z + 1.5 > 1.0 && centre.z + 1.5 < 1.1, "the middle lies on the sphere: {centre}");
        let lowest = last.iter().map(|point| point.z).fold(f32::INFINITY, f32::min);
        assert!(lowest + 1.5 < 0.9, "the edges hang down: {lowest}");
    }
}
