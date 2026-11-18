//! Meshes ready to draw, kept between pictures: an object's mesh after its modifiers, cut into
//! triangles, with face and vertex normals and whether it is closed. A picture only evaluates
//! and prepares an object again when its mesh or modifiers changed.

use std::collections::HashMap;
use std::sync::Arc;

use glam::{Vec2, Vec3, Vec4};
use kition_scene_model::{Id, Mesh, Object, Scene};

/// A mesh prepared for drawing.
#[derive(Debug)]
pub struct Prepared {
    pub mesh: Mesh,
    /// Triangles, each with the face it came from.
    pub triangles: Vec<([u32; 3], u32)>,
    /// Each triangle's corner texture coordinates; empty when the mesh has none.
    pub triangle_uvs: Vec<[Vec2; 3]>,
    /// One normal per face (object space).
    pub face_normals: Vec<Vec3>,
    /// One normal per vertex (object space); empty for flat-shaded meshes.
    pub vertex_normals: Vec<Vec3>,
    /// Each triangle's corner normals (object space), in the same order as `triangles`, when the
    /// mesh is smooth by angle (sharp edges split the normals); empty otherwise, when the
    /// vertex or face normals are the corners'.
    pub split_normals: Vec<[Vec3; 3]>,
    /// Every edge has a face on each side: back faces are hidden behind front ones.
    pub closed: bool,
    /// Each triangle's corner tangents (object space) and bitangent signs, made the first time
    /// a normal map needs them.
    tangents: std::sync::OnceLock<Vec<[Vec4; 3]>>,
}

impl Prepared {
    /// Prepares a mesh.
    pub fn new(mesh: Mesh) -> Self {
        let corner_triangles = mesh.corner_triangles();
        let uvs = mesh.has_uvs() && mesh.uvs_aligned();
        let mut triangles = Vec::with_capacity(corner_triangles.len());
        let mut triangle_uvs = Vec::with_capacity(if uvs { corner_triangles.len() } else { 0 });
        for &(corners, face) in &corner_triangles {
            let Some(vertices) = mesh.faces.get(face as usize) else { continue };
            let vertex = |corner: u32| vertices.get(corner as usize).copied().unwrap_or(0);
            triangles.push(([vertex(corners[0]), vertex(corners[1]), vertex(corners[2])], face));
            if uvs {
                triangle_uvs.push(corners.map(|corner| mesh.corner_uv(face as usize, corner as usize)));
            }
        }
        let face_normals = mesh.faces.iter().map(|face| mesh.face_normal(face)).collect();
        let vertex_normals = if mesh.smooth { mesh.vertex_normals() } else { Vec::new() };
        let split_normals = split_normals(&mesh, &corner_triangles);
        let closed = !mesh.faces.is_empty() && kition_scene_model::modifiers::boundary_edges(&mesh).is_empty();
        Self { mesh, triangles, triangle_uvs, face_normals, vertex_normals, split_normals, closed, tangents: std::sync::OnceLock::new() }
    }

    /// Prepares `mesh`, which has the same faces as `earlier`'s (only vertices moved, as a
    /// sculpt stroke moves them): its triangles, texture coordinates, and closedness are kept,
    /// and only the normals are worked out again.
    pub fn moved(earlier: &Prepared, mesh: Mesh) -> Self {
        let face_normals = mesh.faces.iter().map(|face| mesh.face_normal(face)).collect();
        let vertex_normals = if mesh.smooth { mesh.vertex_normals() } else { Vec::new() };
        let split_normals = if splits(&mesh) { split_normals(&mesh, &mesh.corner_triangles()) } else { Vec::new() };
        Self {
            triangles: earlier.triangles.clone(),
            triangle_uvs: earlier.triangle_uvs.clone(),
            closed: earlier.closed,
            mesh,
            face_normals,
            vertex_normals,
            split_normals,
            tangents: std::sync::OnceLock::new(),
        }
    }

    /// Each triangle's corner tangents and bitangent signs, in the same order as `triangles`
    /// (empty when the mesh has no texture coordinates).
    pub fn triangle_tangents(&self) -> &[[Vec4; 3]] {
        self.tangents.get_or_init(|| {
            let corners = kition_scene_model::tangents::corner_tangents(&self.mesh);
            if corners.is_empty() {
                return Vec::new();
            }
            self.mesh
                .corner_triangles()
                .into_iter()
                .filter(|(_, face)| self.mesh.faces.get(*face as usize).is_some())
                .map(|(triangle, face)| {
                    triangle.map(|corner| corners.get(face as usize).and_then(|list| list.get(corner as usize)).copied().unwrap_or(Vec4::ZERO))
                })
                .collect()
        })
    }
}

/// Whether a mesh's corners have normals of their own (smooth by angle).
fn splits(mesh: &Mesh) -> bool {
    mesh.smooth && mesh.smooth_angle.is_some()
}

/// The corner normals of `triangles` (corners as positions within their face), when the mesh
/// splits them; empty otherwise.
fn split_normals(mesh: &Mesh, triangles: &[([u32; 3], u32)]) -> Vec<[Vec3; 3]> {
    if !splits(mesh) {
        return Vec::new();
    }
    let corners = mesh.corner_normals();
    triangles
        .iter()
        .filter(|(_, face)| mesh.faces.get(*face as usize).is_some())
        .map(|(triangle, face)| {
            let normals = corners.get(*face as usize);
            triangle.map(|corner| normals.and_then(|list| list.get(corner as usize)).copied().unwrap_or(Vec3::Z))
        })
        .collect()
}

/// What a prepared world image was made from: the image's hash, and the strength and turn bits.
type EnvironmentKey = (String, u32, u32);

/// Prepared meshes by object, each with the key of what it was made from.
#[derive(Debug, Default)]
pub struct MeshCache {
    entries: HashMap<Id, (u64, Arc<Prepared>)>,
    /// Decoded images.
    pub textures: crate::texture::TextureCache,
    /// The last shadow maps made, with the key of the lights and casters they were made from.
    shadows: Option<(u64, Vec<Option<Arc<crate::shadow::ShadowMap>>>)>,
    /// The world image prepared for lighting, with the image, strength, and turn it was made with.
    environment: Option<(EnvironmentKey, Option<Arc<crate::environment::Environment>>)>,
}

impl MeshCache {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// The object's mesh after its modifiers (or, with `raw`, the mesh being edited without
    /// them), prepared; `None` for objects that are not meshes.
    pub fn get(&mut self, scene: &Scene, object: &Object, raw: bool) -> Option<Arc<Prepared>> {
        // Curves and texts draw the meshes they make; edit mode's raw mesh is a mesh object's.
        if raw {
            object.mesh()?;
        } else if !object.has_geometry() {
            return None;
        }
        let key = if raw { raw_key(object) } else { object.evaluation_key(scene) };
        if let Some((stored, prepared)) = self.entries.get(&object.id)
            && *stored == key
        {
            return Some(Arc::clone(prepared));
        }
        let mesh = if raw { object.mesh().cloned()? } else { object.evaluated_mesh(scene)? };
        // Only the vertices moved (a sculpt stroke, a grab): keep the topology.
        let same_faces = self.entries.get(&object.id).filter(|(_, earlier)| {
            earlier.mesh.positions.len() == mesh.positions.len()
                && earlier.mesh.faces == mesh.faces
                && earlier.mesh.uvs == mesh.uvs
                && earlier.mesh.loose_edges == mesh.loose_edges
        });
        let prepared = Arc::new(match same_faces {
            Some((_, earlier)) => Prepared::moved(earlier, mesh),
            None => Prepared::new(mesh),
        });
        self.entries.insert(object.id, (key, Arc::clone(&prepared)));
        Some(prepared)
    }

    /// `mesh` prepared for object `id` in place of its own (sculpt mode's multiresolution
    /// level), kept while the mesh stays the same.
    pub fn given(&mut self, id: Id, mesh: &kition_scene_model::Mesh) -> Arc<Prepared> {
        let mut hash = Fnv::new();
        hash.word(0x6c76_6c00);
        hash.word(u32::from(mesh.smooth));
        hash.word(mesh.smooth_angle.map_or(u32::MAX, f32::to_bits));
        for point in &mesh.positions {
            for value in point.to_array() {
                hash.word(value.to_bits());
            }
        }
        for face in &mesh.faces {
            hash.word(u32::try_from(face.len()).unwrap_or(u32::MAX));
            for index in face {
                hash.word(*index);
            }
        }
        let key = hash.0;
        if let Some((stored, prepared)) = self.entries.get(&id)
            && *stored == key
        {
            return Arc::clone(prepared);
        }
        let same_faces =
            self.entries.get(&id).filter(|(_, earlier)| earlier.mesh.positions.len() == mesh.positions.len() && earlier.mesh.faces == mesh.faces);
        let prepared = Arc::new(match same_faces {
            Some((_, earlier)) => Prepared::moved(earlier, mesh.clone()),
            None => Prepared::new(mesh.clone()),
        });
        self.entries.insert(id, (key, Arc::clone(&prepared)));
        prepared
    }

    /// The scene's world image ready for lighting (kept while the image, strength, and turn stay).
    pub fn environment(&mut self, scene: &Scene) -> Option<Arc<crate::environment::Environment>> {
        let Some(hash) = scene.world.image.clone() else {
            self.environment = None;
            return None;
        };
        let key = (hash, scene.world.strength.to_bits(), scene.world.rotation.to_bits());
        if let Some((stored, environment)) = &self.environment
            && *stored == key
        {
            return environment.clone();
        }
        let environment = crate::environment::Environment::of(scene).map(Arc::new);
        self.environment = Some((key, environment.clone()));
        environment
    }

    /// The key the object's prepared mesh was made from.
    pub fn key(&self, id: Id) -> Option<u64> {
        self.entries.get(&id).map(|(key, _)| *key)
    }

    /// The shadow maps made from `key`, if they are the last ones made.
    pub(crate) fn shadows(&self, key: u64) -> Option<Vec<Option<Arc<crate::shadow::ShadowMap>>>> {
        self.shadows.as_ref().filter(|(stored, _)| *stored == key).map(|(_, maps)| maps.clone())
    }

    /// Keeps shadow maps for the next picture.
    pub(crate) fn store_shadows(&mut self, key: u64, maps: Vec<Option<Arc<crate::shadow::ShadowMap>>>) {
        self.shadows = Some((key, maps));
    }

    /// Forgets objects that are no longer in the scene.
    pub fn retain(&mut self, scene: &Scene) {
        self.entries.retain(|id, _| scene.object(*id).is_ok());
        self.textures.retain(scene);
    }

    /// How many objects are cached.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is cached.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// FNV-1a, 64-bit.
pub(crate) struct Fnv(u64);

impl Fnv {
    pub(crate) fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    pub(crate) fn finish(&self) -> u64 {
        self.0
    }

    pub(crate) fn long(&mut self, word: u64) {
        self.bytes(&word.to_le_bytes());
    }

    pub(crate) fn bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    pub(crate) fn word(&mut self, word: u32) {
        self.bytes(&word.to_le_bytes());
    }
}

/// A key of the object's own mesh, as edit mode shows it (without modifiers); evaluated meshes
/// use the model's `Object::evaluation_key`, which also follows the objects modifiers refer to.
fn raw_key(object: &Object) -> u64 {
    let mut hash = Fnv::new();
    hash.word(0x7261_7700);
    if let Some(mesh) = object.mesh() {
        hash.word(u32::from(mesh.smooth));
        hash.word(u32::try_from(mesh.positions.len()).unwrap_or(u32::MAX));
        for point in &mesh.positions {
            for value in point.to_array() {
                hash.word(value.to_bits());
            }
        }
        hash.word(u32::try_from(mesh.faces.len()).unwrap_or(u32::MAX));
        for face in &mesh.faces {
            hash.word(u32::try_from(face.len()).unwrap_or(u32::MAX));
            for index in face {
                hash.word(*index);
            }
        }
        for [a, b] in &mesh.loose_edges {
            hash.word(*a);
            hash.word(*b);
        }
    }
    hash.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use kition_scene_model::{Modifier, ModifierKind};

    #[test]
    fn the_cache_follows_changes() {
        let mut scene = Scene::starter();
        let cube = scene.object_named("Cube").map(|object| object.id).unwrap();
        let mut cache = MeshCache::new();
        let first = cache.get(&scene, scene.object(cube).unwrap(), false).unwrap();
        let again = cache.get(&scene, scene.object(cube).unwrap(), false).unwrap();
        assert!(Arc::ptr_eq(&first, &again), "unchanged objects are not prepared again");
        scene.object_mut(cube).unwrap().modifiers.push(Modifier::new(ModifierKind::Subdivision { levels: 1 }));
        let changed = cache.get(&scene, scene.object(cube).unwrap(), false).unwrap();
        assert_eq!(changed.mesh.faces.len(), 24);
        let raw = cache.get(&scene, scene.object(cube).unwrap(), true).unwrap();
        assert_eq!(raw.mesh.faces.len(), 6);
        scene.remove_objects(&[cube]).unwrap();
        cache.retain(&scene);
        assert!(cache.is_empty());
    }
}
