//! Armatures: bones with a rest shape (head, tail, roll in the armature's space) and a pose
//! (location, rotation, scale in each bone's rest space), and the Armature modifier that deforms
//! a mesh by them through its vertex groups (linear blend skinning, as the reference
//! application's default).
//!
//! A bone's rest orientation follows the reference application's convention: its Y axis runs
//! from head to tail (the rotation that takes +Y there by the shortest arc), turned by `roll`
//! about that axis. A bone's pose matrix in armature space is its parent's pose matrix, times
//! the bone's rest matrix relative to its parent, times the bone's own pose transform; a
//! connected bone's location is ignored, since its head stays on the parent's tail.
//!
//! A bone also has an envelope (a radius at its head and tail and a falloff distance beyond
//! them) that deforms vertices in no bone's group when the modifier uses envelopes, and may be
//! drawn as another object's mesh (a custom shape, [`BoneShape`]).

use std::collections::{BTreeMap, BTreeSet};

use glam::{Mat3, Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::animation::Property;
use crate::error::{Result, SceneError};
use crate::mesh::Mesh;
use crate::scene::{Id, Object, ObjectData, Scene, euler_matrix};

/// A bone's pose: its transform in its rest space.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BonePose {
    pub location: Vec3,
    /// XYZ Euler angles in radians.
    pub rotation: Vec3,
    pub scale: Vec3,
}

impl Default for BonePose {
    fn default() -> Self {
        Self { location: Vec3::ZERO, rotation: Vec3::ZERO, scale: Vec3::ONE }
    }
}

impl BonePose {
    /// Whether the pose is the rest pose.
    pub fn is_rest(&self) -> bool {
        *self == Self::default()
    }

    /// The pose as a matrix in the bone's rest space.
    pub fn matrix(&self, connected: bool) -> Mat4 {
        let location = if connected { Vec3::ZERO } else { self.location };
        Mat4::from_translation(location) * Mat4::from_mat3(euler_matrix(self.rotation)) * Mat4::from_scale(self.scale)
    }
}

/// One bone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bone {
    pub name: String,
    /// Where the bone starts and ends, in the armature's space (at rest).
    pub head: Vec3,
    pub tail: Vec3,
    /// The turn about the bone's own axis, radians.
    #[serde(default)]
    pub roll: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// The head sits on the parent's tail (and follows it).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub connected: bool,
    /// The bone deforms meshes (through the vertex group of its name).
    #[serde(default = "yes")]
    pub deform: bool,
    #[serde(default, skip_serializing_if = "BonePose::is_rest")]
    pub pose: BonePose,
    /// Constraints, evaluated after the bone's pose (IK after the others), in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<crate::constraints::Constraint>,
    /// The envelope's radius at the head, metres (the reference's 0.1).
    #[serde(default = "head_radius", skip_serializing_if = "is_head_radius")]
    pub head_radius: f32,
    /// The envelope's radius at the tail, metres (0.05).
    #[serde(default = "tail_radius", skip_serializing_if = "is_tail_radius")]
    pub tail_radius: f32,
    /// How far past the radii the envelope fades to nothing, metres (0.25).
    #[serde(default = "envelope_distance", skip_serializing_if = "is_envelope_distance")]
    pub envelope_distance: f32,
    /// The envelope's strength where it is full (1).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub envelope_weight: f32,
    /// Vertex group weights are multiplied by the envelope's falloff (not its weight).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub envelope_multiply: bool,
    /// Drawn as another object's mesh instead of an octahedron.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<BoneShape>,
}

fn yes() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

fn one() -> f32 {
    1.0
}

fn is_one(value: &f32) -> bool {
    value.to_bits() == 1.0f32.to_bits()
}

fn head_radius() -> f32 {
    0.1
}

fn is_head_radius(value: &f32) -> bool {
    value.to_bits() == head_radius().to_bits()
}

fn tail_radius() -> f32 {
    0.05
}

fn is_tail_radius(value: &f32) -> bool {
    value.to_bits() == tail_radius().to_bits()
}

fn envelope_distance() -> f32 {
    0.25
}

fn is_envelope_distance(value: &f32) -> bool {
    value.to_bits() == envelope_distance().to_bits()
}

/// The largest envelope radius, distance, and weight (the reference's limits).
pub const MAX_ENVELOPE: f32 = 1000.0;

/// An object whose mesh draws a bone (the reference's custom shape): placed at the posed bone by
/// `translation`, then turned by `rotation` and scaled by `scale` (and by the bone's length with
/// `bone_size`), all in the bone's own axes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoneShape {
    pub object: Id,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub translation: Vec3,
    /// XYZ Euler angles in radians.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rotation: Vec3,
    #[serde(default = "unit_scale", skip_serializing_if = "is_unit_scale")]
    pub scale: Vec3,
    /// Scaled by the bone's length too (the reference's Scale to Bone Length, on by default).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub bone_size: bool,
    /// Drawn as its edges only (the reference's Wireframe); otherwise its faces are filled too.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub wire: bool,
}

fn is_zero(value: &Vec3) -> bool {
    *value == Vec3::ZERO
}

fn unit_scale() -> Vec3 {
    Vec3::ONE
}

fn is_unit_scale(value: &Vec3) -> bool {
    *value == Vec3::ONE
}

impl BoneShape {
    /// A shape drawn by `object` with no offset.
    pub fn new(object: Id) -> Self {
        Self { object, translation: Vec3::ZERO, rotation: Vec3::ZERO, scale: Vec3::ONE, bone_size: true, wire: false }
    }

    /// Where the shape's mesh goes, from the bone's posed matrix and length.
    pub fn matrix(&self, posed: Mat4, length: f32) -> Mat4 {
        let size = if self.bone_size { length } else { 1.0 };
        posed * Mat4::from_translation(self.translation) * Mat4::from_mat3(euler_matrix(self.rotation)) * Mat4::from_scale(self.scale * size)
    }
}

impl Bone {
    /// A bone from `head` to `tail` with no parent.
    pub fn new(name: impl Into<String>, head: Vec3, tail: Vec3) -> Self {
        Self {
            name: name.into(),
            head,
            tail,
            roll: 0.0,
            parent: None,
            connected: false,
            deform: true,
            pose: BonePose::default(),
            constraints: Vec::new(),
            head_radius: head_radius(),
            tail_radius: tail_radius(),
            envelope_distance: envelope_distance(),
            envelope_weight: 1.0,
            envelope_multiply: false,
            shape: None,
        }
    }

    /// How much of the envelope reaches `point` (armature space, rest): 1 within the radii
    /// (interpolated from head to tail along the bone), falling off as `1 - (d / distance)²` over
    /// `distance` beyond them, 0 past that. Not scaled by the envelope's weight.
    pub fn envelope(&self, point: Vec3) -> f32 {
        let along = self.tail - self.head;
        let length = along.length_squared();
        let t = if length > 0.0 { ((point - self.head).dot(along) / length).clamp(0.0, 1.0) } else { 0.0 };
        let radius = self.head_radius + (self.tail_radius - self.head_radius) * t;
        let distance = point.distance(self.head + along * t);
        if distance < radius {
            1.0
        } else if self.envelope_distance > 0.0 && distance < radius + self.envelope_distance {
            let beyond = (distance - radius) / self.envelope_distance;
            1.0 - beyond * beyond
        } else {
            0.0
        }
    }

    /// The bone's length.
    pub fn length(&self) -> f32 {
        self.head.distance(self.tail)
    }

    /// The bone's rest orientation in the armature's space.
    pub fn rest_rotation(&self) -> Mat3 {
        bone_rotation(self.tail - self.head, self.roll)
    }

    /// The bone's rest matrix in the armature's space.
    pub fn rest_matrix(&self) -> Mat4 {
        Mat4::from_mat3(self.rest_rotation()).with_translation(self.head)
    }
}

trait WithTranslation {
    fn with_translation(self, translation: Vec3) -> Self;
}

impl WithTranslation for Mat4 {
    fn with_translation(mut self, translation: Vec3) -> Self {
        self.w_axis = translation.extend(1.0);
        self
    }
}

/// The rotation whose Y axis points along `direction`, turned by `roll` about it.
pub fn bone_rotation(direction: Vec3, roll: f32) -> Mat3 {
    let y = direction.normalize_or(Vec3::Y);
    let arc = if y.dot(Vec3::Y) < -0.999_999 { Quat::from_rotation_z(std::f32::consts::PI) } else { Quat::from_rotation_arc(Vec3::Y, y) };
    Mat3::from_quat(arc) * Mat3::from_rotation_y(roll)
}

/// The roll that makes a bone's Z axis point as close as can be to `up` (in the armature's space).
pub fn roll_towards(direction: Vec3, up: Vec3) -> f32 {
    let base = bone_rotation(direction, 0.0);
    let local = base.transpose() * up;
    // Turning by roll about Y takes the Z axis to (sin roll, 0, cos roll).
    local.x.atan2(local.z)
}

/// An armature's bones.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Armature {
    pub bones: Vec<Bone>,
}

/// The most bones an armature may have.
pub const MAX_BONES: usize = 1024;

impl Armature {
    /// The armature a new one starts with: one bone a metre long, standing up.
    pub fn starter() -> Self {
        Self { bones: vec![Bone::new("Bone", Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0))] }
    }

    /// The bone named `name`.
    pub fn bone(&self, name: &str) -> Option<&Bone> {
        self.bones.iter().find(|bone| bone.name == name)
    }

    /// The bone named `name`, to change.
    pub fn bone_mut(&mut self, name: &str) -> Option<&mut Bone> {
        self.bones.iter_mut().find(|bone| bone.name == name)
    }

    /// `base`, or `base.001`, ... when a bone already has it.
    pub fn unique_name(&self, base: &str) -> String {
        crate::scene::unique_name(base, |candidate| self.bones.iter().any(|bone| bone.name == candidate))
    }

    /// Checks names, parents (no cycles), and numbers.
    pub fn validate(&self) -> Result<()> {
        if self.bones.len() > MAX_BONES {
            return Err(SceneError::InvalidFile("the armature has too many bones".into()));
        }
        let mut names = BTreeSet::new();
        for bone in &self.bones {
            if bone.name.is_empty() || !names.insert(bone.name.as_str()) {
                return Err(SceneError::InvalidFile(format!("bone name `{}` is empty or repeated", bone.name)));
            }
            let numbers = [bone.head, bone.tail, bone.pose.location, bone.pose.rotation, bone.pose.scale];
            if !(numbers.iter().all(|value| value.is_finite()) && bone.roll.is_finite()) {
                return Err(SceneError::InvalidFile(format!("bone `{}` is not finite", bone.name)));
            }
            if bone.length() < 1e-6 {
                return Err(SceneError::InvalidFile(format!("bone `{}` has no length", bone.name)));
            }
            let envelope = [bone.head_radius, bone.tail_radius, bone.envelope_distance, bone.envelope_weight];
            if !envelope.iter().all(|value| value.is_finite() && (0.0..=MAX_ENVELOPE).contains(value)) {
                return Err(SceneError::InvalidFile(format!("bone `{}` has an envelope out of range", bone.name)));
            }
            if let Some(shape) = &bone.shape
                && !(shape.translation.is_finite() && shape.rotation.is_finite() && shape.scale.is_finite())
            {
                return Err(SceneError::InvalidFile(format!("bone `{}` has a custom shape that is not finite", bone.name)));
            }
            crate::constraints::check_stack(&bone.constraints, true)?;
        }
        for bone in &self.bones {
            let mut current = bone.parent.as_deref();
            let mut steps = 0;
            while let Some(parent) = current {
                if !names.contains(parent) {
                    return Err(SceneError::InvalidFile(format!("bone `{}` has a missing parent", bone.name)));
                }
                if parent == bone.name || steps > self.bones.len() {
                    return Err(SceneError::InvalidFile("the bone hierarchy has a cycle".into()));
                }
                current = self.bone(parent).and_then(|parent| parent.parent.as_deref());
                steps += 1;
            }
        }
        Ok(())
    }

    /// Every bone's pose matrix in the armature's space, by name.
    pub fn pose_matrices(&self) -> BTreeMap<String, Mat4> {
        let mut out = BTreeMap::new();
        for bone in &self.bones {
            self.pose_of(bone, &mut out, 0);
        }
        out
    }

    fn pose_of(&self, bone: &Bone, done: &mut BTreeMap<String, Mat4>, depth: usize) -> Mat4 {
        if let Some(matrix) = done.get(&bone.name) {
            return *matrix;
        }
        let parent = bone.parent.as_deref().and_then(|name| self.bone(name)).filter(|_| depth <= self.bones.len());
        let matrix = match parent {
            Some(parent) => {
                let parent_pose = self.pose_of(parent, done, depth + 1);
                parent_pose * parent.rest_matrix().inverse() * bone.rest_matrix() * bone.pose.matrix(bone.connected)
            }
            None => bone.rest_matrix() * bone.pose.matrix(false),
        };
        done.insert(bone.name.clone(), matrix);
        matrix
    }

    /// Each deforming bone's skinning matrix (pose times inverse rest, armature space), by name.
    pub fn skin_matrices(&self) -> BTreeMap<String, Mat4> {
        self.skin_matrices_from(&self.pose_matrices())
    }

    /// [`Armature::skin_matrices`] from given pose matrices (constraints applied).
    pub fn skin_matrices_from(&self, poses: &BTreeMap<String, Mat4>) -> BTreeMap<String, Mat4> {
        self.bones
            .iter()
            .filter(|bone| bone.deform)
            .filter_map(|bone| Some((bone.name.clone(), *poses.get(&bone.name)? * bone.rest_matrix().inverse())))
            .collect()
    }

    /// Where a bone's head and tail are now (posed), in the armature's space.
    pub fn posed_segment(&self, name: &str) -> Option<(Vec3, Vec3)> {
        let bone = self.bone(name)?;
        let matrix = *self.pose_matrices().get(name)?;
        Some((matrix.transform_point3(Vec3::ZERO), matrix.transform_point3(Vec3::new(0.0, bone.length(), 0.0))))
    }

    /// Renames a bone, following through parents.
    pub fn rename(&mut self, from: &str, to: &str) {
        for bone in &mut self.bones {
            if bone.name == from {
                bone.name = to.to_string();
            }
            if bone.parent.as_deref() == Some(from) {
                bone.parent = Some(to.to_string());
            }
        }
    }

    /// Removes bones; their children take the removed bone's parent.
    pub fn remove(&mut self, names: &BTreeSet<String>) {
        let parents: BTreeMap<String, Option<String>> = self.bones.iter().map(|bone| (bone.name.clone(), bone.parent.clone())).collect();
        let lift = |mut parent: Option<String>| {
            let mut steps = 0;
            while let Some(name) = parent.clone() {
                if !names.contains(&name) || steps > parents.len() {
                    break;
                }
                parent = parents.get(&name).cloned().flatten();
                steps += 1;
            }
            parent
        };
        self.bones.retain(|bone| !names.contains(&bone.name));
        for bone in &mut self.bones {
            if bone.parent.as_ref().is_some_and(|parent| names.contains(parent)) {
                bone.parent = lift(bone.parent.clone());
                bone.connected = false;
            }
        }
    }
}

/// The armature of object `id`.
pub fn armature_of(scene: &Scene, id: Id) -> Option<&Armature> {
    match &scene.object(id).ok()?.data {
        ObjectData::Armature(armature) => Some(armature),
        _ => None,
    }
}

/// The armature of object `id`, to change.
pub fn armature_mut(scene: &mut Scene, id: Id) -> Option<&mut Armature> {
    match &mut scene.object_mut(id).ok()?.data {
        ObjectData::Armature(armature) => Some(armature),
        _ => None,
    }
}

/// A component of a bone's pose or a shape key's value (the animation system's named channels).
pub fn read_named(scene: &Scene, target: Id, property: Property, index: u8, path: &str) -> Option<f32> {
    let slot = usize::from(index);
    let object = scene.object(target).ok()?;
    match (property, &object.data) {
        (Property::BoneLocation, ObjectData::Armature(armature)) => armature.bone(path)?.pose.location.to_array().get(slot).copied(),
        (Property::BoneRotation, ObjectData::Armature(armature)) => armature.bone(path)?.pose.rotation.to_array().get(slot).copied(),
        (Property::BoneScale, ObjectData::Armature(armature)) => armature.bone(path)?.pose.scale.to_array().get(slot).copied(),
        (Property::ShapeKey, ObjectData::Mesh(mesh)) if index == 0 => mesh.shape_keys.iter().find(|key| key.name == path).map(|key| key.value),
        _ => None,
    }
}

/// Writes a component of a bone's pose or a shape key's value. Returns whether it exists.
pub fn write_named(scene: &mut Scene, target: Id, property: Property, index: u8, path: &str, value: f32) -> bool {
    let slot = usize::from(index);
    let Ok(object) = scene.object_mut(target) else { return false };
    let set = |vector: &mut Vec3| {
        let mut array = vector.to_array();
        let found = array.get_mut(slot).map(|component| *component = value).is_some();
        *vector = Vec3::from_array(array);
        found
    };
    match (property, &mut object.data) {
        (Property::BoneLocation, ObjectData::Armature(armature)) => armature.bone_mut(path).is_some_and(|bone| set(&mut bone.pose.location)),
        (Property::BoneRotation, ObjectData::Armature(armature)) => armature.bone_mut(path).is_some_and(|bone| set(&mut bone.pose.rotation)),
        (Property::BoneScale, ObjectData::Armature(armature)) => armature.bone_mut(path).is_some_and(|bone| set(&mut bone.pose.scale)),
        (Property::ShapeKey, ObjectData::Mesh(mesh)) if index == 0 => match mesh.shape_keys.iter_mut().find(|key| key.name == path) {
            Some(key) => {
                key.value = value.clamp(crate::vertex_data::SHAPE_KEY_RANGE.0, crate::vertex_data::SHAPE_KEY_RANGE.1);
                true
            }
            None => false,
        },
        _ => false,
    }
}

/// Feeds what an object's evaluated mesh depends on through vertex groups, shape keys, and
/// Armature modifiers into `hasher` (the armature's bones, pose, and where it is relative to
/// the mesh).
pub fn hash_deformation(object: &Object, scene: &Scene, hasher: &mut impl std::hash::Hasher) {
    use std::hash::Hash;
    if let Some(mesh) = object.mesh() {
        for group in &mesh.groups {
            group.name.hash(hasher);
            group.weights.iter().map(|weight| weight.to_bits()).collect::<Vec<_>>().hash(hasher);
        }
        for key in &mesh.shape_keys {
            key.name.hash(hasher);
            key.value.to_bits().hash(hasher);
            key.offsets.iter().map(|offset| offset.to_array().map(f32::to_bits)).collect::<Vec<_>>().hash(hasher);
        }
    }
    for modifier in object.modifiers.iter().filter(|modifier| modifier.enabled) {
        let crate::modifiers::ModifierKind::Armature { object: Some(rig), .. } = modifier.kind else { continue };
        if let Some(armature) = armature_of(scene, rig) {
            serde_json::to_string(armature).unwrap_or_default().hash(hasher);
            // Constraints move bones with their targets: the evaluated pose decides.
            if armature.bones.iter().any(|bone| !bone.constraints.is_empty()) {
                for matrix in crate::constraints::bone_matrices(scene, rig, 0).values() {
                    matrix.to_cols_array().map(f32::to_bits).hash(hasher);
                }
            }
        }
        if let (Ok(mine), Ok(theirs)) = (scene.world_matrix(object.id), scene.world_matrix(rig)) {
            (mine.inverse() * theirs).to_cols_array().map(f32::to_bits).hash(hasher);
        }
    }
}

/// How an Armature modifier finds each vertex's bones, beside the armature and Preserve Volume
/// (the reference's Bind To and vertex group options).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArmatureDeform {
    /// Vertices in deforming bones' groups follow those bones by the groups' weights.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub vertex_groups: bool,
    /// Vertices in no deforming bone's group follow the bones whose envelopes reach them.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub envelopes: bool,
    /// A group whose weights say how much of the deformation each vertex takes (all of it when
    /// the mesh has no group of that name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertex_group: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub invert_vertex_group: bool,
    /// Deforms what the Armature modifier just above got, not what it made, and mixes its
    /// result over that one by the vertex group (the reference's Multi Modifier).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub multi_modifier: bool,
}

impl Default for ArmatureDeform {
    fn default() -> Self {
        Self { vertex_groups: true, envelopes: false, vertex_group: None, invert_vertex_group: false, multi_modifier: false }
    }
}

impl ArmatureDeform {
    /// How much of the deformation each vertex of `mesh` takes; `None` when all of it.
    pub fn mask(&self, mesh: &Mesh) -> Option<Vec<f32>> {
        let name = self.vertex_group.as_deref()?;
        let group = mesh.groups.iter().find(|group| group.name == name && group.weights.len() == mesh.positions.len())?;
        Some(
            group.weights.iter().map(|weight| if self.invert_vertex_group { 1.0 - weight.clamp(0.0, 1.0) } else { weight.clamp(0.0, 1.0) }).collect(),
        )
    }
}

/// The mesh deformed by armature object `rig`'s pose through its vertex groups (groups named
/// after deforming bones), as an Armature modifier with its default settings deforms it.
pub fn deform(mesh: &Mesh, object: &Object, scene: &Scene, rig: Id, preserve_volume: bool) -> Option<Mesh> {
    deform_with(mesh, object, scene, rig, preserve_volume, &ArmatureDeform::default())
}

/// [`deform`] with the modifier's settings: the deformed positions mixed over the mesh's own by
/// the settings' vertex group.
pub fn deform_with(mesh: &Mesh, object: &Object, scene: &Scene, rig: Id, preserve_volume: bool, settings: &ArmatureDeform) -> Option<Mesh> {
    let moved = deformed_positions(mesh, object, scene, rig, preserve_volume, settings)?;
    let mask = settings.mask(mesh);
    let mut out = mesh.clone();
    for (index, (point, target)) in out.positions.iter_mut().zip(moved).enumerate() {
        let share = mask.as_ref().map_or(1.0, |mask| mask.get(index).copied().unwrap_or(0.0));
        *point = point.lerp(target, share);
    }
    Some(out)
}

/// Each vertex of `mesh` moved wholly by armature object `rig`'s pose (no mixing by the
/// modifier's vertex group). A vertex with a positive weight in a deforming bone's group follows
/// those bones (each weight times the bone's envelope falloff with Multiply); otherwise, with
/// envelopes on, the bones whose envelopes reach it (falloff times envelope weight). Weights are
/// normalised per vertex; vertices with none stay. With `preserve_volume` the bones' transforms
/// are blended as dual quaternions ([`crate::dual_quat`]) instead of linearly, so twisted and
/// bent joints keep their thickness.
pub fn deformed_positions(
    mesh: &Mesh,
    object: &Object,
    scene: &Scene,
    rig: Id,
    preserve_volume: bool,
    settings: &ArmatureDeform,
) -> Option<Vec<Vec3>> {
    let armature = armature_of(scene, rig)?;
    if rig == object.id {
        return None;
    }
    let (mesh_world, rig_world) = (scene.world_matrix(object.id).ok()?, scene.world_matrix(rig).ok()?);
    // Mesh space to armature space and back.
    let to_rig = rig_world.inverse() * mesh_world;
    let from_rig = to_rig.inverse();
    let skins = armature.skin_matrices_from(&crate::constraints::bone_matrices(scene, rig, 0));
    // The deforming bones, each with its skinning matrix, dual-quaternion part, and group.
    struct Deformer<'a> {
        bone: &'a Bone,
        skin: Mat4,
        part: Option<crate::dual_quat::Part>,
        weights: Option<&'a [f32]>,
    }
    let deformers: Vec<Deformer> = armature
        .bones
        .iter()
        .filter_map(|bone| {
            let skin = *skins.get(&bone.name)?;
            let weights = settings
                .vertex_groups
                .then(|| mesh.groups.iter().find(|group| group.name == bone.name && group.weights.len() == mesh.positions.len()))
                .flatten()
                .map(|group| group.weights.as_slice());
            let part = preserve_volume.then(|| crate::dual_quat::part(skin, bone.rest_matrix()));
            Some(Deformer { bone, skin, part, weights })
        })
        .collect();
    let mut out = mesh.positions.clone();
    let mut chosen: Vec<(f32, usize)> = Vec::with_capacity(deformers.len());
    for (index, point) in out.iter_mut().enumerate() {
        let local = to_rig.transform_point3(*point);
        chosen.clear();
        let grouped = deformers.iter().any(|deformer| deformer.weights.and_then(|weights| weights.get(index)).is_some_and(|weight| *weight > 0.0));
        if grouped {
            for (slot, deformer) in deformers.iter().enumerate() {
                let Some(weight) = deformer.weights.and_then(|weights| weights.get(index)).copied().filter(|weight| *weight > 0.0) else { continue };
                let weight = if deformer.bone.envelope_multiply { weight * deformer.bone.envelope(local) } else { weight };
                if weight > 0.0 {
                    chosen.push((weight, slot));
                }
            }
        } else if settings.envelopes {
            for (slot, deformer) in deformers.iter().enumerate() {
                let weight = deformer.bone.envelope(local) * deformer.bone.envelope_weight;
                if weight > 0.0 {
                    chosen.push((weight, slot));
                }
            }
        }
        if preserve_volume {
            let blended: Vec<(f32, crate::dual_quat::Part)> =
                chosen.iter().filter_map(|(weight, slot)| Some((*weight, deformers.get(*slot)?.part?))).collect();
            if let Some(blend) = crate::dual_quat::blend(&blended) {
                *point = from_rig.transform_point3(blend.rigid.transform_point3(blend.scale.transform_point3(local)));
            }
            continue;
        }
        let mut sum = Vec3::ZERO;
        let mut total = 0.0;
        for (weight, slot) in &chosen {
            let Some(deformer) = deformers.get(*slot) else { continue };
            sum += deformer.skin.transform_point3(local) * *weight;
            total += *weight;
        }
        if total > 1e-6 {
            *point = from_rig.transform_point3(sum / total);
        }
    }
    Some(out)
}

/// Gives a mesh (object `object`, armature `rig`) a vertex group per deforming bone. With
/// `automatic`, weights are bone heat ([`crate::heat`], as the reference application's
/// automatic weights); for a mesh too large for that, they fall off with each vertex's distance
/// to the bones (the nearest bone dominates; weights below a twentieth of the strongest are
/// dropped; each vertex sums to 1). Returns the groups made.
pub fn bind(scene: &mut Scene, object: Id, rig: Id, automatic: bool) -> Result<Vec<String>> {
    let armature = armature_of(scene, rig).ok_or_else(|| SceneError::params("that object is not an armature"))?.clone();
    let (mesh_world, rig_world) = (scene.world_matrix(object)?, scene.world_matrix(rig)?);
    let to_rig = rig_world.inverse() * mesh_world;
    let poses = armature.pose_matrices();
    let segments: Vec<(String, Vec3, Vec3)> = armature
        .bones
        .iter()
        .filter(|bone| bone.deform)
        .filter_map(|bone| {
            let pose = poses.get(&bone.name)?;
            Some((bone.name.clone(), pose.transform_point3(Vec3::ZERO), pose.transform_point3(Vec3::new(0.0, bone.length(), 0.0))))
        })
        .collect();
    let mesh = scene.object_mut(object)?.mesh_mut().ok_or_else(|| SceneError::params("that object is not a mesh"))?;
    let points: Vec<Vec3> = mesh.positions.iter().map(|point| to_rig.transform_point3(*point)).collect();
    let mut made = Vec::new();
    let mut weights: Vec<Vec<f32>> = vec![vec![0.0; points.len()]; segments.len()];
    let pairs: Vec<(Vec3, Vec3)> = segments.iter().map(|(_, head, tail)| (*head, *tail)).collect();
    let heated = if automatic { crate::heat::weights(mesh, to_rig, &pairs) } else { None };
    if let Some(heated) = heated {
        weights = heated;
    } else if automatic && !segments.is_empty() {
        let scale = segments.iter().map(|(_, head, tail)| head.distance(*tail)).sum::<f32>() / segments.len() as f32;
        let floor = (scale * 0.05).max(1e-4);
        for (vertex, point) in points.iter().enumerate() {
            let raw: Vec<f32> = segments
                .iter()
                .map(|(_, head, tail)| {
                    let distance = distance_to_segment(*point, *head, *tail).max(floor);
                    1.0 / distance.powi(4)
                })
                .collect();
            let strongest = raw.iter().copied().fold(0.0f32, f32::max);
            let kept: Vec<f32> = raw.iter().map(|value| if *value >= strongest * 0.05 { *value } else { 0.0 }).collect();
            let total: f32 = kept.iter().sum();
            if total <= 0.0 {
                continue;
            }
            for (bone, value) in kept.iter().enumerate() {
                if let Some(slot) = weights.get_mut(bone).and_then(|list| list.get_mut(vertex)) {
                    *slot = value / total;
                }
            }
        }
    }
    for ((name, _, _), values) in segments.iter().zip(weights) {
        match mesh.groups.iter_mut().find(|group| &group.name == name) {
            Some(group) => {
                if automatic {
                    group.weights = values;
                }
            }
            None => mesh.groups.push(crate::vertex_data::VertexGroup { name: name.clone(), weights: values }),
        }
        made.push(name.clone());
    }
    Ok(made)
}

/// The distance from `point` to the segment `a`–`b`.
pub fn distance_to_segment(point: Vec3, a: Vec3, b: Vec3) -> f32 {
    let along = b - a;
    let length = along.length_squared();
    let t = if length > 0.0 { ((point - a).dot(along) / length).clamp(0.0, 1.0) } else { 0.0 };
    point.distance(a + along * t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modifiers::{Modifier, ModifierKind};
    use crate::primitives::Primitive;

    fn two_bones() -> Armature {
        let mut lower = Bone::new("Lower", Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0));
        lower.roll = 0.0;
        let mut upper = Bone::new("Upper", Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 2.0));
        upper.parent = Some("Lower".into());
        upper.connected = true;
        Armature { bones: vec![lower, upper] }
    }

    #[test]
    fn rest_axes_follow_the_bone() {
        let bone = Bone::new("B", Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0));
        // A bone standing up along +Z: its Y axis is +Z and its Z axis -Y (the reference's frame).
        let rotation = bone.rest_rotation();
        assert!((rotation * Vec3::Y).abs_diff_eq(Vec3::Z, 1e-6));
        assert!((rotation * Vec3::Z).abs_diff_eq(Vec3::NEG_Y, 1e-6));
        let rolled = bone_rotation(Vec3::Z, std::f32::consts::FRAC_PI_2);
        assert!((rolled * Vec3::Y).abs_diff_eq(Vec3::Z, 1e-6));
        let roll = roll_towards(Vec3::Z, Vec3::X);
        assert!((bone_rotation(Vec3::Z, roll) * Vec3::Z).abs_diff_eq(Vec3::X, 1e-5));
    }

    #[test]
    fn a_child_follows_its_parent() {
        let mut armature = two_bones();
        armature.validate().unwrap();
        armature.bone_mut("Lower").unwrap().pose.rotation = Vec3::new(std::f32::consts::FRAC_PI_2, 0.0, 0.0);
        // Turning the lower bone 90° about its X axis lays the chain down.
        let (head, tail) = armature.posed_segment("Upper").unwrap();
        assert!(head.abs_diff_eq(armature.posed_segment("Lower").unwrap().1, 1e-5));
        assert!((tail - head).length() > 0.99);
        assert!(tail.z.abs() < 1e-4 || (tail - head).z.abs() < 1e-4, "{head} {tail}");
    }

    #[test]
    fn skinning_bends_a_column() {
        let mut scene = Scene::empty();
        let rig = scene.add_object("Armature", ObjectData::Armature(two_bones()));
        let mut column = Primitive::Cylinder { vertices: 8, radius: 0.2, depth: 2.0, fill: crate::primitives::Fill::Ngon }.build().unwrap();
        // Rings along the length, as a mesh made for skinning has (bone heat spreads weight
        // across long faces otherwise).
        for _ in 0..3 {
            let sides: Vec<u32> = (0..column.faces.len() as u32).filter(|face| column.faces[*face as usize].len() == 4).collect();
            crate::ops::subdivide(&mut column, &sides).unwrap();
        }
        column.transform(Mat4::from_translation(Vec3::new(0.0, 0.0, 1.0)));
        let count = column.positions.len();
        let mesh = scene.add_object("Column", ObjectData::Mesh(column));
        let groups = bind(&mut scene, mesh, rig, true).unwrap();
        assert_eq!(groups, vec!["Lower".to_string(), "Upper".to_string()]);
        let object = scene.object_mut(mesh).unwrap();
        object.modifiers.push(Modifier::new(ModifierKind::Armature { object: Some(rig), preserve_volume: false, deform: Default::default() }));
        for group in &object.mesh().unwrap().groups {
            assert_eq!(group.weights.len(), count);
        }
        let rest = scene.object(mesh).unwrap().evaluated_mesh(&scene).unwrap();
        assert!(rest.positions.iter().zip(&scene.object(mesh).unwrap().mesh().unwrap().positions).all(|(a, b)| a.abs_diff_eq(*b, 1e-5)));
        armature_mut(&mut scene, rig).unwrap().bone_mut("Upper").unwrap().pose.rotation = Vec3::new(std::f32::consts::FRAC_PI_2, 0.0, 0.0);
        let bent = scene.object(mesh).unwrap().evaluated_mesh(&scene).unwrap();
        let top = bent.positions.iter().copied().max_by(|a, b| a.y.abs().total_cmp(&b.y.abs())).unwrap();
        assert!(top.y.abs() > 0.8, "the top swings over: {top}");
        let bottom = bent.positions.iter().copied().min_by(|a, b| a.z.total_cmp(&b.z)).unwrap();
        assert!(bottom.z.abs() < 1e-4, "the bottom stays: {bottom}");
    }

    #[test]
    fn removing_a_bone_lifts_its_children() {
        let mut armature = two_bones();
        armature.bones.push(Bone { parent: Some("Upper".into()), ..Bone::new("Tip", Vec3::new(0.0, 0.0, 2.0), Vec3::new(0.0, 0.0, 3.0)) });
        armature.remove(&BTreeSet::from(["Upper".to_string()]));
        assert_eq!(armature.bone("Tip").unwrap().parent.as_deref(), Some("Lower"));
        armature.validate().unwrap();
    }
}
