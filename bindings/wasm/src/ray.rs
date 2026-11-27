//! Rays against the scene as the viewport draws it: the surface under a screen point, for placing
//! the 3D cursor (Shift+right click), snapping to faces and telling whether a point to snap to is
//! hidden behind a surface. Each drawn mesh gets a bounding volume hierarchy in its own space,
//! built the first time a ray needs it and kept while the mesh the viewport draws stays the same
//! (moving or turning an object keeps it: the ray is taken into the object's space instead). No
//! software picture is drawn for it.

use std::collections::HashMap;
use std::sync::Arc;

use glam::{Mat4, Vec3};
use kition_scene_model::Id;
use kition_scene_raster::{EditOverlay, Overlays, Prepared};
use kition_scene_trace::bvh::{Bvh, Triangle};

use crate::{Mode, SceneDocument};

/// The most triangles one mesh may have for a ray to look at it (a bigger one is passed over).
const MAX_TRIANGLES: usize = 4_000_000;

/// What a ray met first.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RayHit {
    /// The object whose surface it is.
    pub object: Id,
    /// The face of that object's drawn mesh.
    pub face: u32,
    /// Where, in world space, and the surface's normal there (towards the side the ray came from).
    pub point: Vec3,
    pub normal: Vec3,
    /// How far along the ray (in its own direction's units).
    pub distance: f32,
}

/// One mesh's hierarchy, with the prepared mesh it was built from (kept so the comparison by
/// pointer stays sound).
struct Entry {
    from: Arc<Prepared>,
    bvh: Arc<Bvh>,
}

/// The hierarchies kept between rays, by object.
#[derive(Default)]
pub(crate) struct Rays {
    entries: HashMap<Id, Entry>,
}

impl std::fmt::Debug for Rays {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Rays").field("meshes", &self.entries.len()).finish()
    }
}

impl Rays {
    /// The hierarchy over `prepared` (object `id`'s drawn mesh), built when it is new.
    fn of(&mut self, id: Id, prepared: &Arc<Prepared>) -> Option<Arc<Bvh>> {
        if let Some(entry) = self.entries.get(&id)
            && Arc::ptr_eq(&entry.from, prepared)
        {
            return Some(Arc::clone(&entry.bvh));
        }
        if prepared.triangles.len() > MAX_TRIANGLES {
            return None;
        }
        let positions = &prepared.mesh.positions;
        let triangles: Vec<Triangle> = prepared
            .triangles
            .iter()
            .map(|([a, b, c], _)| {
                let at = |index: &u32| positions.get(*index as usize).copied().unwrap_or(Vec3::ZERO);
                Triangle::new(at(a), at(b), at(c))
            })
            .collect();
        let bvh = Arc::new(Bvh::build(triangles));
        self.entries.insert(id, Entry { from: Arc::clone(prepared), bvh: Arc::clone(&bvh) });
        Some(bvh)
    }

    /// Forgets the meshes of objects that are gone.
    fn retain(&mut self, keep: &[Id]) {
        self.entries.retain(|id, _| keep.contains(id));
    }
}

/// One mesh a ray looks at: its object, the drawn mesh, its hierarchy, its world matrix and that
/// matrix's inverse.
type RayMesh = (Id, Arc<Prepared>, Arc<Bvh>, Mat4, Mat4);

/// The drawn meshes a ray looks at, taken once (finding them hashes each mesh): a query of many
/// rays (a snap's steps, a hover) takes one and asks it again.
#[derive(Default)]
pub(crate) struct RayScene {
    meshes: Vec<RayMesh>,
}

impl RayScene {
    /// The first surface the ray from `origin` along `direction` meets in (`near`, `far`).
    pub(crate) fn hit(&self, origin: Vec3, direction: Vec3, near: f32, far: f32) -> Option<RayHit> {
        self.hit_except(origin, direction, near, far, &|_| false)
    }

    /// As [`Self::hit`], passing over the meshes of the objects `skip` names.
    pub(crate) fn hit_except(&self, origin: Vec3, direction: Vec3, near: f32, far: f32, skip: &dyn Fn(Id) -> bool) -> Option<RayHit> {
        if !(origin.is_finite() && direction.is_finite()) || direction.length_squared() < 1e-20 {
            return None;
        }
        let mut best: Option<RayHit> = None;
        for (id, prepared, bvh, _, inverse) in &self.meshes {
            if skip(*id) {
                continue;
            }
            // The same line in the object's space: distances along it stay the same.
            let (local_origin, local_direction) = (inverse.transform_point3(origin), inverse.transform_vector3(direction));
            let limit = best.map_or(far, |hit| hit.distance);
            let Some(hit) = bvh.intersect(local_origin, local_direction, near, limit, false) else { continue };
            let face = prepared.triangles.get(hit.triangle as usize).map_or(0, |(_, face)| *face);
            let local_normal = prepared.face_normals.get(face as usize).copied().unwrap_or(Vec3::Z);
            let mut normal = inverse.transpose().transform_vector3(local_normal).normalize_or(Vec3::Z);
            if normal.dot(direction) > 0.0 {
                normal = -normal;
            }
            best = Some(RayHit { object: *id, face, point: origin + direction * hit.distance, normal, distance: hit.distance });
        }
        best
    }

    /// The surface under the pixel (`x`, `y`) of `projector`'s view.
    pub(crate) fn at(&self, projector: &kition_scene_raster::Projector, x: f32, y: f32) -> Option<RayHit> {
        self.at_except(projector, x, y, &|_| false)
    }

    /// As [`Self::at`], passing over the meshes of the objects `skip` names.
    pub(crate) fn at_except(&self, projector: &kition_scene_raster::Projector, x: f32, y: f32, skip: &dyn Fn(Id) -> bool) -> Option<RayHit> {
        if !(x.is_finite() && y.is_finite()) {
            return None;
        }
        let (origin, direction) = projector.ray(x, y);
        self.hit_except(origin, direction, 0.0, f32::INFINITY, skip)
    }

    /// The meshes it looks at: each object with its drawn mesh and world matrix.
    pub(crate) fn meshes(&self) -> impl Iterator<Item = (Id, &Prepared, Mat4)> + '_ {
        self.meshes.iter().map(|(id, prepared, _, world, _)| (*id, prepared.as_ref(), *world))
    }

    /// Whether `point` shows in `projector`'s view: no surface lies between it and the eye.
    pub(crate) fn shows(&self, projector: &kition_scene_raster::Projector, point: Vec3) -> bool {
        let eye = projector.eye();
        let origin = if projector.perspective {
            eye
        } else {
            // Orthographic: from the eye's plane, along the view.
            let back = (glam::Mat3::from_mat4(projector.view.inverse()) * Vec3::Z).normalize_or(Vec3::Z);
            point + back * ((eye - point).dot(back).max(0.0) + 1.0)
        };
        let between = point - origin;
        let length = between.length();
        if !(length.is_finite() && length > 1e-6) {
            return true;
        }
        let direction = between / length;
        // A little short of the point: its own faces do not hide it.
        let margin = (length * 1e-5).max(1e-4);
        self.hit(origin, direction, 0.0, length - margin).is_none()
    }

    /// Whether it looks at nothing.
    pub(crate) fn is_empty(&self) -> bool {
        self.meshes.is_empty()
    }
}

impl SceneDocument {
    /// The meshes the viewport draws, with their world matrices: edit mode's mesh as it is being
    /// edited, the others after their modifiers, and the particles drawn as meshes.
    pub(crate) fn ray_meshes(&self) -> Vec<(Id, Arc<Prepared>, Mat4)> {
        let overlays = match self.session.mode {
            Mode::Edit(object) => {
                Overlays { edit: Some(EditOverlay { object, vertices: Default::default(), faces: Default::default() }), ..Overlays::default() }
            }
            Mode::Object => Overlays::default(),
        };
        // Particles drawn as meshes (halos, instances, hair) are surfaces too: picking one picks
        // its emitter (`commands_particles::owners`).
        let shown = crate::commands_particles::viewport_scene(self);
        kition_scene_raster::viewport_meshes(shown.as_ref().unwrap_or(&self.scene), &overlays, &mut self.meshes.borrow_mut())
    }

    /// The visible meshes as drawn now that `skip` does not pass over, ready for rays (each
    /// mesh's hierarchy built the first time).
    pub(crate) fn ray_scene(&self, skip: &dyn Fn(Id) -> bool) -> RayScene {
        let meshes = self.ray_meshes();
        let mut rays = self.rays.borrow_mut();
        let ids: Vec<Id> = meshes.iter().map(|(id, ..)| *id).collect();
        rays.retain(&ids);
        let meshes = meshes
            .into_iter()
            .filter(|(id, ..)| !skip(*id))
            .filter_map(|(id, prepared, world)| {
                let inverse = world.inverse();
                if !inverse.is_finite() {
                    return None;
                }
                let bvh = rays.of(id, &prepared)?;
                Some((id, prepared, bvh, world, inverse))
            })
            .collect();
        RayScene { meshes }
    }

    /// The surface under the viewport's pixel (`x`, `y`) as it is drawn now, of the meshes `skip`
    /// does not pass over.
    pub(crate) fn surface_at(&self, x: f32, y: f32, skip: &dyn Fn(Id) -> bool) -> Option<RayHit> {
        self.ray_scene(skip).at(&self.projector(), x, y)
    }
}
