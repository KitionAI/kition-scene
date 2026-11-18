//! Particle systems in the scene ([`kition_scene_particles`]): where an object's systems start
//! (its surface after modifiers), what they fly through (colliders), and what they show as
//! geometry. Renders and exports see particles through [`realize`], which adds them to a copy of
//! the scene as plain meshes in world space: halos and instances at each emitter particle, and
//! hair as thin ribbons (facing the eye when there is one). The particles themselves are never
//! saved; only the systems' settings and groomed hair are.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use glam::{Mat3, Mat4, Quat, Vec3};

pub use kition_scene_particles::{
    Boids, Cache, Children, Distribution, EmitFrom, Field, GroomBrush, GroomDab, HairDynamics, Kind, Particle, ParticlePhysics, ParticleSystem,
    RenderAs, Start, Strand, Surface, blob, boids, hair, sim,
};

use crate::error::{Result, SceneError};
use crate::mesh::Mesh;
use crate::scene::{Id, Object, ObjectData, Scene, euler_matrix};

/// Whether any object has a particle system.
pub fn any(scene: &Scene) -> bool {
    scene.objects.iter().any(|object| !object.particles.is_empty())
}

/// The object's surface after modifiers as triangles, in world space (`world`) or its own.
pub fn surface(scene: &Scene, id: Id, world: bool) -> Option<Surface> {
    let object = scene.object(id).ok()?;
    let mesh = object.evaluated_mesh(scene)?;
    let matrix = if world { scene.world_matrix(id).ok()? } else { Mat4::IDENTITY };
    let normal_matrix = Mat3::from_mat4(matrix).inverse().transpose();
    let normals = mesh.vertex_normals().into_iter().map(|normal| (normal_matrix * normal).normalize_or_zero()).collect();
    Some(Surface {
        positions: mesh.positions.iter().map(|point| matrix.transform_point3(*point)).collect(),
        normals,
        triangles: mesh.triangles().into_iter().map(|(triangle, _)| triangle).collect(),
        weights: Vec::new(),
    })
}

/// The surface system `system` of object `id` starts from: [`surface`] with the weights of the
/// system's density group (when the mesh has it after modifiers).
pub fn system_surface(scene: &Scene, id: Id, system: &ParticleSystem, world: bool) -> Option<Surface> {
    let mut found = surface(scene, id, world)?;
    if let Some(name) = &system.density_group {
        let mesh = scene.object(id).ok()?.evaluated_mesh(scene)?;
        // A group the mesh lacks weighs nothing: no particles, as the reference does.
        found.weights = match mesh.groups.iter().find(|group| &group.name == name) {
            Some(group) => (0..found.positions.len()).map(|index| group.weights.get(index).copied().unwrap_or(0.0)).collect(),
            None => vec![0.0; found.positions.len()],
        };
    }
    Some(found)
}

/// What system `system` of object `id` flies through: the emitter's world surface and axes, and
/// its colliders' triangles in world space.
pub fn field(scene: &Scene, id: Id, system: &ParticleSystem) -> Field {
    let axes = scene.world_matrix(id).map(|matrix| {
        let rotation = Mat3::from_mat4(matrix);
        Mat3::from_cols(rotation.x_axis.normalize_or_zero(), rotation.y_axis.normalize_or_zero(), rotation.z_axis.normalize_or_zero())
    });
    let mut colliders = Vec::new();
    // The system's own colliders, and every object with collision physics (which cloth collides
    // with too, as in the reference application), each once and never the emitter itself.
    let mut named: Vec<Id> = system.colliders.clone();
    for object in &scene.objects {
        if system.physics != ParticlePhysics::Boids
            && object.id != id
            && !object.hidden
            && object.physics.as_ref().is_some_and(|physics| physics.collision.is_some())
            && !named.contains(&object.id)
        {
            named.push(object.id);
        }
    }
    for collider in &named {
        let Some(found) = surface(scene, *collider, true) else { continue };
        for [a, b, c] in &found.triangles {
            let corner = |index: u32| found.positions.get(index as usize).copied().unwrap_or(Vec3::ZERO);
            colliders.push([corner(*a), corner(*b), corner(*c)]);
        }
    }
    let effectors = if system.physics == ParticlePhysics::Boids {
        Vec::new()
    } else {
        scene.effectors().into_iter().filter(|effector| effector.owner != id).collect()
    };
    let boid_targets = system
        .boids
        .rules
        .iter()
        .filter_map(|rule| {
            let target = rule.target?;
            Some((target, scene.world_matrix(target).ok()?.transform_point3(Vec3::ZERO)))
        })
        .collect();
    Field {
        boid_targets,
        surface: system_surface(scene, id, system, true).unwrap_or_default(),
        axes: axes.unwrap_or(Mat3::IDENTITY),
        colliders,
        effectors,
    }
}

/// The frame the scene is at.
pub fn frame(scene: &Scene) -> f32 {
    scene.animation.timeline.frame as f32
}

/// The live particles of emitter system `index` of object `id` at the scene's frame.
pub fn particles(scene: &Scene, id: Id, index: usize, cache: &mut Cache) -> Arc<Vec<Particle>> {
    let Some(system) = scene.object(id).ok().and_then(|object| object.particles.get(index)) else { return Arc::new(Vec::new()) };
    cache.particles_at(system, &field(scene, id, system), frame(scene))
}

/// Every strand hair system `index` of object `id` (on its own-space surface `local`) shows now,
/// in the object's own space: its parents as the shown physics bake moves them when it does (hair
/// dynamics), else as groomed or grown, then the children, which follow the parents either way.
pub fn shown_strands(scene: &Scene, id: Id, index: usize, system: &ParticleSystem, local: &Surface) -> Vec<Strand> {
    let starts = hair::roots(system, local);
    let rest = hair::parents(system, local);
    let parents = match scene.simulated_hair(id, index) {
        Some(baked) if baked.len() == rest.len() && baked.iter().zip(&rest).all(|(moved, still)| moved.len() == still.len()) => baked,
        _ => rest,
    };
    let children = hair::children(system, local, &starts, &parents);
    let mut out = parents;
    out.extend(children);
    out
}

/// Every strand of hair system `index` of object `id` (parents, then children) as shown now, in
/// world space (`world`) or the object's own.
pub fn strands(scene: &Scene, id: Id, index: usize, world: bool) -> Vec<Strand> {
    let Some(system) = scene.object(id).ok().and_then(|object| object.particles.get(index)) else { return Vec::new() };
    let Some(local) = system_surface(scene, id, system, false) else { return Vec::new() };
    let all = shown_strands(scene, id, index, system, &local);
    if !world {
        return all;
    }
    let Ok(matrix) = scene.world_matrix(id) else { return Vec::new() };
    all.into_iter().map(|strand| strand.into_iter().map(|point| matrix.transform_point3(point)).collect()).collect()
}

/// Where geometry is shown from, for hair that faces the eye.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct View {
    pub eye: Option<Vec3>,
    /// What one pixel covers: radians in a perspective view, metres in an orthographic one.
    /// Hair and halos are never drawn thinner than a pixel. Zero for none (final renders and
    /// exports keep the true sizes).
    pub pixel: f32,
    /// Orthographic: a pixel covers the same everywhere, and hair faces along `toward`.
    pub orthographic: Option<Vec3>,
}

impl View {
    /// The smallest half-width that still shows at `point`.
    fn least_half(&self, point: Vec3) -> f32 {
        match (self.orthographic, self.eye) {
            _ if self.pixel <= 0.0 => 0.0,
            (Some(_), _) => self.pixel * 0.5,
            (None, Some(eye)) => self.pixel * eye.distance(point) * 0.5,
            (None, None) => 0.0,
        }
    }

    /// The direction a ribbon at `point` faces, if the view sets one.
    fn toward(&self, point: Vec3) -> Option<Vec3> {
        match (self.orthographic, self.eye) {
            (Some(toward), _) => Some(toward),
            (None, Some(eye)) => Some((eye - point).normalize_or_zero()),
            (None, None) => None,
        }
    }
}

/// A scene with its particles made into meshes, and which emitter each added object belongs to.
#[derive(Debug, Clone)]
pub struct Realized {
    pub scene: Scene,
    pub owners: BTreeMap<Id, Id>,
}

/// A small octahedron round `center`.
fn halo(mesh: &mut Mesh, center: Vec3, radius: f32) {
    let base = mesh.vertex_count();
    for axis in [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z] {
        mesh.positions.push(center + axis * radius);
    }
    // +X 0, -X 1, +Y 2, -Y 3, +Z 4, -Z 5, counter-clockwise from outside.
    for [a, b, c] in [[0, 2, 4], [2, 1, 4], [1, 3, 4], [3, 0, 4], [2, 0, 5], [1, 2, 5], [3, 1, 5], [0, 3, 5]] {
        mesh.faces.push(vec![base + a, base + b, base + c]);
    }
}

/// A strand as a ribbon tapering from `width` at the root to nothing at the tip, turned to face
/// `view`'s eye (or across `normal` without one).
fn ribbon(mesh: &mut Mesh, strand: &[Vec3], width: f32, normal: Vec3, view: View) {
    let count = strand.len();
    if count < 2 {
        return;
    }
    let base = mesh.vertex_count();
    for (step, point) in strand.iter().enumerate() {
        let before = strand.get(step.saturating_sub(1)).copied().unwrap_or(*point);
        let after = strand.get(step + 1).copied().unwrap_or(*point);
        let tangent = (after - before).normalize_or_zero();
        let toward = view.toward(*point).unwrap_or(normal);
        let mut side = tangent.cross(toward).normalize_or_zero();
        if side == Vec3::ZERO {
            side = tangent.any_orthonormal_vector();
        }
        let share = 1.0 - step as f32 / (count - 1) as f32;
        let half = (width * 0.5 * share).max(view.least_half(*point));
        mesh.positions.push(*point - side * half);
        mesh.positions.push(*point + side * half);
    }
    for step in 0..count as u32 - 1 {
        let (a, b) = (base + step * 2, base + step * 2 + 1);
        mesh.faces.push(vec![a, a + 2, b + 2, b]);
    }
}

/// The geometry of one system of `object`, in world space; `None` when it shows nothing.
fn system_mesh(scene: &Scene, object: &Object, index: usize, view: View, cache: &mut Cache) -> Option<(Mesh, Option<Id>)> {
    let system = object.particles.get(index)?;
    let mut mesh = Mesh::default();
    match system.kind {
        Kind::Emitter => {
            let live = particles(scene, object.id, index, cache);
            match system.render_as {
                RenderAs::None => None,
                RenderAs::Halo => {
                    for particle in live.iter() {
                        halo(&mut mesh, particle.position, (particle.size * 0.5).max(view.least_half(particle.position) * 3.0));
                    }
                    Some((mesh, object.material))
                }
                RenderAs::Object => {
                    let source_id = system.instance_object.filter(|source| *source != object.id)?;
                    let source = scene.object(source_id).ok()?;
                    let shape = source.evaluated_mesh(scene)?;
                    for particle in live.iter() {
                        let mut copy = shape.clone();
                        let turn = Quat::from_mat3(&euler_matrix(particle.rotation));
                        copy.transform(Mat4::from_scale_rotation_translation(Vec3::splat(particle.size), turn, particle.position));
                        mesh.append(&copy);
                    }
                    Some((mesh, source.material))
                }
            }
        }
        Kind::Hair => {
            let local = system_surface(scene, object.id, system, false)?;
            let starts = hair::roots(system, &local);
            let matrix = scene.world_matrix(object.id).ok()?;
            let normal_matrix = Mat3::from_mat4(matrix).inverse().transpose();
            let scale = (matrix.x_axis.truncate().length() + matrix.y_axis.truncate().length() + matrix.z_axis.truncate().length()) / 3.0;
            for (strand_index, strand) in shown_strands(scene, object.id, index, system, &local).into_iter().enumerate() {
                let world: Vec<Vec3> = strand.iter().map(|point| matrix.transform_point3(*point)).collect();
                let root_normal =
                    starts.get(strand_index % starts.len().max(1)).map_or(Vec3::Z, |start| (normal_matrix * start.normal).normalize_or_zero());
                ribbon(&mut mesh, &world, system.strand_width * scale, root_normal, view);
            }
            Some((mesh, object.material))
        }
    }
}

/// `scene` with every visible particle system's geometry added as world-space mesh objects
/// (named after their emitter and system, with ids after the scene's); `None` when no object
/// has a particle system.
pub fn realize(scene: &Scene, view: View, cache: &mut Cache) -> Option<Realized> {
    if !any(scene) {
        return None;
    }
    let mut out = scene.clone();
    let mut owners = BTreeMap::new();
    for object in scene.objects.iter().filter(|object| !object.hidden && !object.particles.is_empty()) {
        for index in 0..object.particles.len() {
            let Some((mesh, material)) = system_mesh(scene, object, index, view, cache) else { continue };
            if mesh.faces.is_empty() {
                continue;
            }
            let id = out.next_id;
            out.next_id = out.next_id.saturating_add(1);
            let name = object.particles.get(index).map_or_else(String::new, |system| format!("{} {}", object.name, system.name));
            let mut made = Object::new(id, name, ObjectData::Mesh(mesh));
            made.material = material;
            out.objects.push(made);
            owners.insert(id, object.id);
        }
    }
    Some(Realized { scene: out, owners })
}

/// Checks every object's systems: settings in range, names unique per object, only on meshes,
/// colliders, instances and boid targets that exist (and are not the emitter).
pub fn validate(scene: &Scene) -> Result<()> {
    for object in &scene.objects {
        if object.particles.is_empty() {
            continue;
        }
        if !matches!(object.data, ObjectData::Mesh(_)) {
            return Err(SceneError::InvalidFile(format!("{} has particle systems but is not a mesh", object.name)));
        }
        let mut names = BTreeSet::new();
        for system in &object.particles {
            system.check().map_err(|error| SceneError::InvalidFile(format!("{}'s particle system {}: {error}", object.name, system.name)))?;
            let missing = |id: &Id| scene.object(*id).is_err() || *id == object.id;
            if system.colliders.iter().any(missing)
                || system.instance_object.as_ref().is_some_and(missing)
                || system.boids.rules.iter().filter_map(|rule| rule.target.as_ref()).any(missing)
            {
                return Err(SceneError::InvalidFile(format!("{}'s particle system {} names a missing object", object.name, system.name)));
            }
            if !names.insert(system.name.as_str()) {
                return Err(SceneError::InvalidFile(format!("{} has two particle systems called {}", object.name, system.name)));
            }
        }
    }
    Ok(())
}

/// Lets go of deleted objects: colliders, instances and boid targets naming them.
pub fn forget(scene: &mut Scene, ids: &[Id]) {
    for object in &mut scene.objects {
        for system in &mut object.particles {
            system.colliders.retain(|collider| !ids.contains(collider));
            for rule in &mut system.boids.rules {
                if rule.target.is_some_and(|target| ids.contains(&target)) {
                    rule.target = None;
                }
            }
            if system.instance_object.is_some_and(|source| ids.contains(&source)) {
                system.instance_object = None;
            }
        }
    }
}

/// A system name not yet used on `object` (`base`, then `base.001`, ...).
pub fn unique_name(object: &Object, base: &str) -> String {
    crate::scene::unique_name(base, |candidate| object.particles.iter().any(|system| system.name == candidate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::Primitive;

    fn scene_with(kind: Kind) -> (Scene, Id) {
        let mut scene = Scene::empty();
        let plane = scene.add_object("Plane", ObjectData::Mesh(Primitive::Plane { size: 2.0 }.build().unwrap()));
        scene.object_mut(plane).unwrap().particles.push(ParticleSystem::new("Particles", kind));
        (scene, plane)
    }

    #[test]
    fn realizing_adds_one_mesh_per_system_owned_by_its_emitter() {
        let (mut scene, plane) = scene_with(Kind::Emitter);
        scene.animation.timeline.frame = 20;
        let mut cache = Cache::new();
        let realized = realize(&scene, View::default(), &mut cache).unwrap();
        assert_eq!(realized.scene.objects.len(), 2);
        let (made, owner) = realized.owners.iter().next().unwrap();
        assert_eq!(*owner, plane);
        let mesh = realized.scene.object(*made).unwrap().mesh().unwrap();
        // 96 particles alive at frame 20, an octahedron each.
        let alive = particles(&scene, plane, 0, &mut cache).len();
        assert_eq!(mesh.faces.len(), alive * 8);
        assert!(realized.scene.validate().is_ok());
        assert!(realize(&Scene::starter(), View::default(), &mut cache).is_none());
    }

    #[test]
    fn hair_becomes_ribbons_that_face_the_eye() {
        let (scene, plane) = scene_with(Kind::Hair);
        let all = strands(&scene, plane, 0, true);
        assert_eq!(all.len(), 1000);
        let mut cache = Cache::new();
        let view = View { eye: Some(Vec3::new(0.0, -10.0, 2.0)), ..View::default() };
        let realized = realize(&scene, view, &mut cache).unwrap();
        let ribbons = realized.scene.objects.last().unwrap().mesh().unwrap();
        assert_eq!(ribbons.faces.len(), 1000 * 5);
        // Each ribbon is 1 cm wide at its root.
        let root_width = ribbons.positions[0].distance(ribbons.positions[1]);
        assert!((root_width - 0.01).abs() < 1e-5, "{root_width}");
    }

    #[test]
    fn objects_with_collision_physics_stop_particles_like_listed_colliders() {
        let (mut scene, plane) = scene_with(Kind::Emitter);
        scene.animation.timeline.frame = 60;
        let system = scene.object(plane).unwrap().particles[0].clone();
        let falling = field(&scene, plane, &system).colliders.len();
        assert_eq!(falling, 0);
        // A floor under the emitter, with collision physics but not named by the system.
        let floor = scene.add_object("Floor", ObjectData::Mesh(Primitive::Plane { size: 20.0 }.build().unwrap()));
        scene.object_mut(floor).unwrap().location = Vec3::new(0.0, 0.0, -1.0);
        scene.object_mut(floor).unwrap().physics =
            Some(crate::physics::ObjectPhysics { collision: Some(crate::physics::Collision::default()), ..Default::default() });
        assert_eq!(field(&scene, plane, &system).colliders.len(), 2, "the floor's two triangles");
        let mut cache = Cache::new();
        let lowest = particles(&scene, plane, 0, &mut cache).iter().map(|particle| particle.position.z).fold(f32::INFINITY, f32::min);
        assert!(lowest >= -1.01, "no particle falls through the floor: {lowest}");
    }

    #[test]
    fn deleting_a_collider_lets_go_and_bad_systems_are_refused() {
        let (mut scene, plane) = scene_with(Kind::Emitter);
        let floor = scene.add_object("Floor", ObjectData::Mesh(Primitive::Plane { size: 10.0 }.build().unwrap()));
        scene.object_mut(plane).unwrap().particles[0].colliders.push(floor);
        scene.object_mut(plane).unwrap().particles[0].instance_object = Some(floor);
        scene.remove_objects(&[floor]).unwrap();
        let system = &scene.object(plane).unwrap().particles[0];
        assert!(system.colliders.is_empty() && system.instance_object.is_none());
        scene.object_mut(plane).unwrap().particles.push(ParticleSystem::new("Particles", Kind::Hair));
        assert!(scene.validate().is_err(), "two systems with one name");
    }
}
