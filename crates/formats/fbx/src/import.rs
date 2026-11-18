//! FBX documents into Kition scene objects. Every object's world matrix is worked out in the file's own
//! space (its units and axes, FBX's full node transform with pivots, offsets, pre- and
//! post-rotation, and rotation order), turned into the scene's Z-up metres, and only then made
//! local to the object's parent; cameras and lights also turn from FBX's looking axis (+X for a
//! camera, -Y for a light) to the scene's (-Z). Skins become an armature with vertex groups,
//! blend shapes become shape keys, and animation curves are sampled at their keys.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use kition_scene_exchange::{Imported, ImportedChannel, ImportedObject};
use kition_scene_model::armature::{Armature, Bone, roll_towards};
use kition_scene_model::glam::{DMat3, DMat4, DVec3, Mat4, Vec2, Vec3};
use kition_scene_model::scene::matrix_euler;
use kition_scene_model::vertex_data::{ShapeKey, VertexGroup};
use kition_scene_model::{Camera, Image, Interpolation, Light, LightKind, Material, Mesh, ObjectData, Property, SceneError};

use crate::node::{Node, Prop};

/// KTime units per second.
pub const TICKS_PER_SECOND: f64 = 46_186_158_000.0;
/// The most keyframe samples taken from one file.
const MAX_SAMPLES: usize = 2_000_000;

pub(crate) fn invalid(message: impl Into<String>) -> SceneError {
    SceneError::InvalidFile(format!("FBX: {}", message.into()))
}

/// A connection between two objects (`OO`) or an object and another's property (`OP`).
struct Link {
    child: i64,
    parent: i64,
    property: Option<String>,
}

/// The document's objects and connections.
pub(crate) struct Document<'a> {
    /// Seconds added to every key: the file's time 0 is frame 1.
    pub(crate) offset: f64,
    objects: HashMap<i64, &'a Node>,
    links: Vec<Link>,
    /// Scene metres per file unit, and file axes to scene axes.
    pub(crate) to_scene: DMat4,
}

/// The part of an object's name before FBX's `\0\x01Class` suffix (or `Class::Name` in ASCII).
pub(crate) fn object_name(node: &Node) -> String {
    let raw = node.prop(1).and_then(Prop::as_str).unwrap_or_default();
    if let Some((name, _)) = raw.split_once("\u{0}\u{1}") {
        return name.to_string();
    }
    match raw.split_once("::") {
        Some((_, name)) => name.to_string(),
        None => raw.to_string(),
    }
}

/// An object's class (the third property: `Mesh`, `Null`, `LimbNode`, `Camera`, ...).
fn object_class(node: &Node) -> &str {
    node.prop(2).and_then(Prop::as_str).unwrap_or_default()
}

/// The `P` entries of a node's `Properties70` (or ASCII `Properties60`) by name.
pub(crate) fn properties(node: &Node) -> HashMap<String, Vec<Prop>> {
    let mut out = HashMap::new();
    for block in node.children.iter().filter(|child| child.name == "Properties70" || child.name == "Properties60") {
        for entry in block.children.iter().filter(|child| child.name == "P" || child.name == "Property") {
            let Some(name) = entry.prop(0).and_then(Prop::as_str) else { continue };
            // P: name, type, label, flags, values...; Property (6.x): name, type, flags, values.
            let skip = if entry.name == "P" { 4 } else { 3 };
            out.insert(name.to_string(), entry.props.iter().skip(skip).cloned().collect());
        }
    }
    out
}

fn vec3_of(props: &HashMap<String, Vec<Prop>>, name: &str, default: DVec3) -> DVec3 {
    match props.get(name).map(Vec::as_slice) {
        Some([x, y, z, ..]) => DVec3::new(x.as_f64().unwrap_or(default.x), y.as_f64().unwrap_or(default.y), z.as_f64().unwrap_or(default.z)),
        _ => default,
    }
}

fn number_of(props: &HashMap<String, Vec<Prop>>, name: &str, default: f64) -> f64 {
    props.get(name).and_then(|values| values.first()).and_then(Prop::as_f64).filter(|value| value.is_finite()).unwrap_or(default)
}

fn int_of(props: &HashMap<String, Vec<Prop>>, name: &str, default: i64) -> i64 {
    props.get(name).and_then(|values| values.first()).and_then(Prop::as_i64).unwrap_or(default)
}

/// XYZ Euler degrees in `order` (FBX's enum: XYZ, XZY, YZX, YXZ, ZXY, ZYX), first axis first.
pub(crate) fn euler(degrees: DVec3, order: i64) -> DMat3 {
    let (x, y, z) = (
        DMat3::from_rotation_x(degrees.x.to_radians()),
        DMat3::from_rotation_y(degrees.y.to_radians()),
        DMat3::from_rotation_z(degrees.z.to_radians()),
    );
    match order {
        1 => y * z * x,
        2 => x * z * y,
        3 => z * x * y,
        4 => y * x * z,
        5 => x * y * z,
        _ => z * y * x,
    }
}

/// A node's transform values, with any animated channels in place of the stored ones.
#[derive(Clone, Copy)]
pub(crate) struct Trs {
    pub(crate) translation: DVec3,
    pub(crate) rotation: DVec3,
    pub(crate) scale: DVec3,
}

/// FBX's node transform: T · Roff · Rp · Rpre · R · Rpost⁻¹ · Rp⁻¹ · Soff · Sp · S · Sp⁻¹.
pub(crate) fn local_matrix(props: &HashMap<String, Vec<Prop>>, trs: Trs) -> DMat4 {
    let order = int_of(props, "RotationOrder", 0);
    let translate = DMat4::from_translation;
    let rotate = |degrees: DVec3, order: i64| DMat4::from_mat3(euler(degrees, order));
    let rotation_offset = vec3_of(props, "RotationOffset", DVec3::ZERO);
    let rotation_pivot = vec3_of(props, "RotationPivot", DVec3::ZERO);
    let pre = vec3_of(props, "PreRotation", DVec3::ZERO);
    let post = vec3_of(props, "PostRotation", DVec3::ZERO);
    let scaling_offset = vec3_of(props, "ScalingOffset", DVec3::ZERO);
    let scaling_pivot = vec3_of(props, "ScalingPivot", DVec3::ZERO);
    translate(trs.translation)
        * translate(rotation_offset)
        * translate(rotation_pivot)
        * rotate(pre, 0)
        * rotate(trs.rotation, order)
        * rotate(post, 0).inverse()
        * translate(-rotation_pivot)
        * translate(scaling_offset)
        * translate(scaling_pivot)
        * DMat4::from_scale(trs.scale)
        * translate(-scaling_pivot)
}

pub(crate) fn rest_trs(props: &HashMap<String, Vec<Prop>>) -> Trs {
    Trs {
        translation: vec3_of(props, "Lcl Translation", DVec3::ZERO),
        rotation: vec3_of(props, "Lcl Rotation", DVec3::ZERO),
        scale: vec3_of(props, "Lcl Scaling", DVec3::ONE),
    }
}

/// The file's axes and unit to the scene's Z-up metres.
fn axis_conversion(settings: Option<&Node>) -> DMat4 {
    let props = settings.map(properties).unwrap_or_default();
    let unit = number_of(&props, "UnitScaleFactor", 1.0);
    let unit = if unit > 0.0 && unit < 1e6 { unit / 100.0 } else { 0.01 };
    let axis = |name: &str, default: i64| int_of(&props, name, default);
    let sign = |name: &str| if int_of(&props, name, 1) < 0 { -1.0 } else { 1.0 };
    let (up, front, coord) = (axis("UpAxis", 1), axis("FrontAxis", 2), axis("CoordAxis", 0));
    let mut set: BTreeSet<i64> = BTreeSet::new();
    set.extend([up, front, coord]);
    let unit_axis = |index: i64| match index {
        0 => DVec3::X,
        1 => DVec3::Y,
        _ => DVec3::Z,
    };
    // Scene axes for each file axis: the file's right goes to +X, its up to +Z, and its front
    // (towards the viewer) to -Y.
    let rotation = if set.len() == 3 && set.iter().all(|axis| (0..3).contains(axis)) {
        let mut columns = [DVec3::ZERO; 3];
        for (file, scene) in
            [(coord, DVec3::X * sign("CoordAxisSign")), (up, DVec3::Z * sign("UpAxisSign")), (front, -DVec3::Y * sign("FrontAxisSign"))]
        {
            if let Some(column) = columns.get_mut(usize::try_from(file).unwrap_or(0)) {
                *column = scene;
            }
        }
        let _ = unit_axis;
        let [x, y, z] = columns;
        DMat3::from_cols(x, y, z)
    } else {
        DMat3::from_rotation_x(std::f64::consts::FRAC_PI_2)
    };
    DMat4::from_scale(DVec3::splat(unit)) * DMat4::from_mat3(rotation)
}

impl<'a> Document<'a> {
    pub(crate) fn new(nodes: &'a [Node]) -> Result<Self, SceneError> {
        let mut objects = HashMap::new();
        if let Some(block) = nodes.iter().find(|node| node.name == "Objects") {
            for object in &block.children {
                if let Some(id) = object.prop(0).and_then(Prop::as_i64) {
                    objects.insert(id, object);
                }
            }
        }
        if objects.is_empty() {
            return Err(invalid("the file has no objects"));
        }
        let mut links = Vec::new();
        if let Some(block) = nodes.iter().find(|node| node.name == "Connections") {
            for link in block.children.iter().filter(|child| child.name == "C" || child.name == "Connect") {
                let kind = link.prop(0).and_then(Prop::as_str).unwrap_or_default();
                let (Some(child), Some(parent)) = (link.prop(1).and_then(Prop::as_i64), link.prop(2).and_then(Prop::as_i64)) else { continue };
                let property = if kind == "OP" { link.prop(3).and_then(Prop::as_str).map(str::to_string) } else { None };
                links.push(Link { child, parent, property });
            }
        }
        let settings = nodes.iter().find(|node| node.name == "GlobalSettings");
        let to_scene = axis_conversion(settings);
        let props = settings.map(properties).unwrap_or_default();
        let fps = match int_of(&props, "TimeMode", 11) {
            1 => 120.0,
            2 => 100.0,
            3 => 60.0,
            4 => 50.0,
            5 => 48.0,
            6 | 7 => 30.0,
            8 | 9 => 29.97,
            10 => 25.0,
            12 => 1000.0,
            13 => 23.976,
            14 => number_of(&props, "CustomFrameRate", 24.0),
            15 => 96.0,
            16 => 72.0,
            17 => 59.94,
            _ => 24.0,
        };
        let offset = if fps > 0.0 && fps.is_finite() { 1.0 / fps } else { 1.0 / 24.0 };
        Ok(Self { offset, objects, links, to_scene })
    }

    fn object(&self, id: i64) -> Option<&'a Node> {
        self.objects.get(&id).copied()
    }

    /// Objects connected under `parent` (with the property a connection names, if any).
    fn children(&self, parent: i64) -> impl Iterator<Item = (&'a Node, Option<&str>)> + '_ {
        self.links.iter().filter(move |link| link.parent == parent).filter_map(|link| Some((self.object(link.child)?, link.property.as_deref())))
    }

    /// Objects `child` is connected under.
    fn parents(&self, child: i64) -> impl Iterator<Item = (i64, Option<&str>)> + '_ {
        self.links.iter().filter(move |link| link.child == child).map(|link| (link.parent, link.property.as_deref()))
    }

    fn is_model(node: &Node) -> bool {
        node.name == "Model"
    }

    /// The model a model hangs under, or `None` at the root.
    fn model_parent(&self, id: i64) -> Option<i64> {
        self.parents(id).find_map(|(parent, property)| (property.is_none() && self.object(parent).is_some_and(Self::is_model)).then_some(parent))
    }
}

/// Animated values of one model's transform, by property and component.
#[derive(Default)]
struct ModelCurves {
    /// (property, component) → curve keys (seconds, value, interpolation).
    curves: BTreeMap<(&'static str, usize), Curve>,
}

#[derive(Clone)]
pub(crate) struct Curve {
    pub(crate) keys: Vec<(f64, f64)>,
    /// Per key: 0 constant, 1 linear, 2 cubic, with its right and next-left slopes (per second).
    pub(crate) modes: Vec<(u8, f64, f64)>,
}

impl Curve {
    pub(crate) fn evaluate(&self, time: f64) -> Option<f64> {
        let first = self.keys.first()?;
        if time <= first.0 {
            return Some(first.1);
        }
        for (index, pair) in self.keys.windows(2).enumerate() {
            let [(t0, v0), (t1, v1)] = pair else { continue };
            if time > *t1 {
                continue;
            }
            let span = (t1 - t0).max(1e-12);
            let u = ((time - t0) / span).clamp(0.0, 1.0);
            let (mode, right, left) = self.modes.get(index).copied().unwrap_or((1, 0.0, 0.0));
            return Some(match mode {
                0 => *v0,
                2 => {
                    // Cubic Hermite with the stored slopes.
                    let (h00, h10, h01, h11) =
                        (2.0 * u.powi(3) - 3.0 * u * u + 1.0, u.powi(3) - 2.0 * u * u + u, -2.0 * u.powi(3) + 3.0 * u * u, u.powi(3) - u * u);
                    h00 * v0 + h10 * span * right + h01 * v1 + h11 * span * left
                }
                _ => v0 + (v1 - v0) * u,
            });
        }
        self.keys.last().map(|key| key.1)
    }
}

/// An AnimationCurve's keys.
fn read_curve(node: &Node) -> Option<Curve> {
    let times = node.child("KeyTime")?.prop(0)?.as_i64s()?;
    let values = node.child("KeyValueFloat")?.prop(0)?.as_f64s()?;
    if times.len() != values.len() || times.is_empty() || times.len() > MAX_SAMPLES {
        return None;
    }
    let flags = node.child("KeyAttrFlags").and_then(|flags| flags.prop(0)).and_then(Prop::as_i64s).unwrap_or_default();
    let data = node.child("KeyAttrDataFloat").and_then(|data| data.prop(0)).and_then(Prop::as_f64s).unwrap_or_default();
    let counts = node.child("KeyAttrRefCount").and_then(|counts| counts.prop(0)).and_then(Prop::as_i64s).unwrap_or_default();
    // Attributes are shared by runs of keys: KeyAttrRefCount says how many use each.
    let mut modes = Vec::with_capacity(times.len());
    for (attribute, count) in counts.iter().enumerate() {
        let flag = flags.get(attribute).copied().unwrap_or(4);
        let mode = if flag & 0x2 != 0 {
            0
        } else if flag & 0x8 != 0 {
            2
        } else {
            1
        };
        let right = data.get(attribute * 4).copied().unwrap_or(0.0);
        let left = data.get(attribute * 4 + 1).copied().unwrap_or(0.0);
        for _ in 0..usize::try_from(*count).unwrap_or(0).min(times.len()) {
            modes.push((mode, right, left));
        }
    }
    modes.resize(times.len(), (1, 0.0, 0.0));
    let keys = times
        .iter()
        .zip(&values)
        .map(|(time, value)| (*time as f64 / TICKS_PER_SECOND, *value))
        .filter(|(time, value)| time.is_finite() && value.is_finite())
        .collect::<Vec<_>>();
    if keys.windows(2).any(|pair| matches!(pair, [a, b] if b.0 < a.0)) {
        return None;
    }
    Some(Curve { keys, modes })
}

fn curves_of(document: &Document<'_>, model: i64) -> ModelCurves {
    let mut out = ModelCurves::default();
    for (node, property) in
        document.links.iter().filter(|link| link.parent == model).filter_map(|link| Some((document.object(link.child)?, link.property.as_deref())))
    {
        if node.name != "AnimationCurveNode" {
            continue;
        }
        let key: &'static str = match property {
            Some("Lcl Translation") => "T",
            Some("Lcl Rotation") => "R",
            Some("Lcl Scaling") => "S",
            Some("DeformPercent") => "D",
            _ => continue,
        };
        let id = node.prop(0).and_then(Prop::as_i64).unwrap_or_default();
        for (curve, component) in document.children(id) {
            let index = match component {
                Some("d|X") => 0,
                Some("d|Y") => 1,
                Some("d|Z") => 2,
                Some("d|DeformPercent") => 0,
                _ => continue,
            };
            if curve.name != "AnimationCurve" {
                continue;
            }
            if let Some(curve) = read_curve(curve) {
                out.curves.insert((key, index), curve);
            }
        }
    }
    out
}

/// What a model became.
struct Made {
    model: i64,
    /// Index into the imported objects.
    index: usize,
    /// The scene-space world matrix at rest.
    world: DMat4,
    /// Right factor from the file's node frame to the object's frame (cameras and lights).
    turn: DMat4,
}

fn to_f32(matrix: DMat4) -> Mat4 {
    matrix.as_mat4()
}

/// Location, XYZ Euler rotation, and scale of a matrix; `None` when it shears.
fn decompose(matrix: Mat4) -> Option<(Vec3, Vec3, Vec3)> {
    let (scale, rotation, translation) = matrix.to_scale_rotation_translation();
    let rebuilt = Mat4::from_scale_rotation_translation(scale, rotation, translation);
    let tolerance = 1e-3 * matrix.to_cols_array().iter().fold(1.0f32, |most, value| most.max(value.abs()));
    if !rebuilt.abs_diff_eq(matrix, tolerance) || !scale.is_finite() {
        return None;
    }
    Some((translation, matrix_euler(kition_scene_model::glam::Mat3::from_quat(rotation)), scale))
}

/// Reads an FBX document's scene.
pub fn import(nodes: &[Node]) -> Result<Imported, SceneError> {
    let document = Document::new(nodes)?;
    let mut imported = Imported::default();
    // Materials and their images.
    let mut material_index: HashMap<i64, usize> = HashMap::new();
    for (id, node) in &document.objects {
        if node.name == "Material" {
            material_index.insert(*id, imported.materials.len());
            let material = read_material(&document, *id, node, &mut imported.images);
            imported.materials.push(material);
        }
    }
    // Models, parents before children.
    let mut models: Vec<i64> = document.objects.iter().filter(|(_, node)| Document::is_model(node)).map(|(id, _)| *id).collect();
    models.sort_unstable();
    let limit = models.len();
    let depth = |id: i64| {
        let mut depth = 0;
        let mut current = document.model_parent(id);
        while let Some(parent) = current {
            depth += 1;
            if depth > limit {
                break;
            }
            current = document.model_parent(parent);
        }
        depth
    };
    models.sort_by_key(|id| (depth(*id), *id));
    // Skeleton bones go into armatures rather than becoming objects.
    let bones: BTreeSet<i64> =
        models.iter().copied().filter(|id| document.object(*id).is_some_and(|node| object_class(node) == "LimbNode")).collect();
    let mut file_world: HashMap<i64, DMat4> = HashMap::new();
    let mut made: HashMap<i64, Made> = HashMap::new();
    for id in &models {
        let Some(node) = document.object(*id) else { continue };
        let props = properties(node);
        let parent = document.model_parent(*id);
        let parent_world = parent.and_then(|parent| file_world.get(&parent).copied()).unwrap_or(DMat4::IDENTITY);
        let world = parent_world * local_matrix(&props, rest_trs(&props));
        file_world.insert(*id, world);
        if bones.contains(id) {
            continue;
        }
        let class = object_class(node);
        let attribute = document.children(*id).find(|(child, _)| child.name == "NodeAttribute").map(|(child, _)| child);
        let geometry = document.children(*id).find(|(child, property)| child.name == "Geometry" && property.is_none()).map(|(child, _)| child);
        let (data, turn) = match (class, geometry, attribute) {
            (_, Some(geometry), _) => (ObjectData::Mesh(read_mesh(&document, geometry, &props)?), DMat4::IDENTITY),
            ("Camera", _, attribute) => (ObjectData::Camera(read_camera(attribute, &document)), DMat4::from_rotation_y(std::f64::consts::FRAC_PI_2)),
            ("Light", _, attribute) => (ObjectData::Light(read_light(attribute)), DMat4::from_rotation_x(std::f64::consts::FRAC_PI_2)),
            _ => {
                let has_bones = document.children(*id).any(|(child, _)| bones.contains(&child.prop(0).and_then(Prop::as_i64).unwrap_or_default()));
                if has_bones {
                    (ObjectData::Armature(Armature::default()), DMat4::IDENTITY)
                } else {
                    (ObjectData::Empty { size: 1.0 }, DMat4::IDENTITY)
                }
            }
        };
        let scene_world = document.to_scene * world * turn.inverse();
        let parent_made = parent.and_then(|parent| made.get(&parent));
        let parent_scene_world = parent_made.map_or(DMat4::IDENTITY, |parent| parent.world);
        let local = to_f32(parent_scene_world.inverse() * scene_world);
        let (location, rotation, scale, parent_inverse) = match decompose(local) {
            Some((location, rotation, scale)) => (location, rotation, scale, Mat4::IDENTITY),
            // A shearing local matrix: the object keeps its world matrix through its parent inverse.
            None => {
                let (location, rotation, scale) = decompose(to_f32(scene_world)).unwrap_or((Vec3::ZERO, Vec3::ZERO, Vec3::ONE));
                (location, rotation, scale, to_f32(parent_scene_world.inverse()))
            }
        };
        let material = document
            .children(*id)
            .find_map(|(child, _)| (child.name == "Material").then(|| child.prop(0).and_then(Prop::as_i64)).flatten())
            .and_then(|material| material_index.get(&material).copied());
        let index = imported.objects.len();
        imported.objects.push(ImportedObject {
            name: object_name(node),
            data,
            location,
            rotation,
            scale,
            parent: parent_made.map(|parent| parent.index),
            parent_inverse,
            material,
        });
        made.insert(*id, Made { model: *id, index, world: scene_world, turn });
    }
    build_armatures(&document, &bones, &file_world, &mut made, &mut imported)?;
    read_animation(&document, &bones, &made, &file_world, &mut imported);
    Ok(imported)
}

fn read_material(document: &Document<'_>, id: i64, node: &Node, images: &mut Vec<Image>) -> Material {
    let props = properties(node);
    let mut material = Material::new(0, object_name(node));
    let diffuse = vec3_of(&props, "DiffuseColor", vec3_of(&props, "Diffuse", DVec3::splat(0.8)));
    let factor = number_of(&props, "DiffuseFactor", 1.0);
    let opacity = 1.0 - number_of(&props, "TransparencyFactor", 0.0).clamp(0.0, 1.0);
    let base = (diffuse * factor).as_vec3();
    material.base_color = [base.x.clamp(0.0, 1.0), base.y.clamp(0.0, 1.0), base.z.clamp(0.0, 1.0), (opacity as f32).clamp(0.0, 1.0)];
    if opacity < 0.999 {
        material.alpha_mode = kition_scene_model::AlphaMode::Blend;
    }
    // Shininess as the reference application's importer reads it: roughness = 1 - sqrt(s) / 10.
    let shininess = number_of(&props, "ShininessExponent", number_of(&props, "Shininess", 20.0)).clamp(0.0, 100.0);
    material.roughness = (1.0 - shininess.sqrt() / 10.0).clamp(0.0, 1.0) as f32;
    material.metallic = number_of(&props, "ReflectionFactor", 0.0).clamp(0.0, 1.0) as f32;
    let emissive = vec3_of(&props, "EmissiveColor", DVec3::ZERO) * number_of(&props, "EmissiveFactor", 0.0);
    if emissive.max_element() > 0.0 {
        material.emission = emissive.as_vec3();
        material.emission_strength = 1.0;
    }
    for (texture, property) in document.children(id) {
        if texture.name != "Texture" {
            continue;
        }
        let Some(image) = texture_image(document, texture) else { continue };
        let hash = image.hash.clone();
        if !images.iter().any(|other| other.hash == hash) {
            images.push(image);
        }
        match property {
            Some("DiffuseColor") | Some("Maya|baseColor") | Some("Maya|TEX_color_map") => material.base_color_texture = Some(hash),
            Some("NormalMap") | Some("Bump") => material.normal_texture = Some(hash),
            _ => {}
        }
    }
    material
}

/// A texture's embedded image (its Video's `Content`); textures that only name a file on disk
/// are skipped.
fn texture_image(document: &Document<'_>, texture: &Node) -> Option<Image> {
    let id = texture.prop(0).and_then(Prop::as_i64)?;
    let (video, _) = document.children(id).find(|(child, _)| child.name == "Video")?;
    let bytes = match video.child("Content")?.prop(0)? {
        Prop::Raw(bytes) => bytes.clone(),
        _ => return None,
    };
    let name = video
        .child("RelativeFilename")
        .and_then(|node| node.prop(0))
        .and_then(Prop::as_str)
        .map(|path| path.rsplit(['/', '\\']).next().unwrap_or(path).to_string())
        .unwrap_or_else(|| object_name(video));
    Image::new(name, bytes).ok()
}

/// A layer element's per-corner values: `mapping` and `reference` decide where each comes from.
fn per_corner<T: Copy>(element: &Node, values: &[T], corners: &[(usize, usize, u32)], vertex_count: usize) -> Option<Vec<T>> {
    let mapping = element.child("MappingInformationType").and_then(|node| node.prop(0)).and_then(Prop::as_str).unwrap_or("ByPolygonVertex");
    let reference = element.child("ReferenceInformationType").and_then(|node| node.prop(0)).and_then(Prop::as_str).unwrap_or("Direct");
    let index_name = ["UVIndex", "NormalsIndex", "MaterialsIndex"];
    let indices: Option<Vec<i64>> =
        element.children.iter().find(|child| index_name.contains(&child.name.as_str())).and_then(|node| node.prop(0)).and_then(Prop::as_i64s);
    let lookup = |slot: usize| -> Option<T> {
        let index = if reference == "IndexToDirect" || reference == "Index" { usize::try_from(*indices.as_ref()?.get(slot)?).ok()? } else { slot };
        values.get(index).copied()
    };
    let mut out = Vec::with_capacity(corners.len());
    for (corner, polygon, vertex) in corners {
        let slot = match mapping {
            "ByPolygonVertex" => *corner,
            "ByVertex" | "ByVertice" | "ByControlPoint" => *vertex as usize,
            "ByPolygon" => *polygon,
            "AllSame" => 0,
            _ => return None,
        };
        if mapping.starts_with("ByVert") && *vertex as usize >= vertex_count {
            return None;
        }
        out.push(lookup(slot)?);
    }
    Some(out)
}

fn read_mesh(document: &Document<'_>, geometry: &Node, model: &HashMap<String, Vec<Prop>>) -> Result<Mesh, SceneError> {
    let vertices =
        geometry.child("Vertices").and_then(|node| node.prop(0)).and_then(Prop::as_f64s).ok_or_else(|| invalid("a mesh has no vertices"))?;
    let polygon_index = geometry.child("PolygonVertexIndex").and_then(|node| node.prop(0)).and_then(Prop::as_i64s).unwrap_or_default();
    if vertices.len() % 3 != 0 || vertices.len() / 3 > 20_000_000 || polygon_index.len() > 50_000_000 {
        return Err(invalid("a mesh's vertices are damaged or too many"));
    }
    // The geometric transform applies to the mesh only.
    let geometric = local_matrix(
        &HashMap::new(),
        Trs {
            translation: vec3_of(model, "GeometricTranslation", DVec3::ZERO),
            rotation: vec3_of(model, "GeometricRotation", DVec3::ZERO),
            scale: vec3_of(model, "GeometricScaling", DVec3::ONE),
        },
    );
    let mut mesh = Mesh {
        positions: vertices.as_chunks::<3>().0.iter().map(|point| geometric.transform_point3(DVec3::from_array(*point)).as_vec3()).collect(),
        ..Default::default()
    };
    let count = mesh.positions.len();
    let mut face: Vec<u32> = Vec::new();
    // (corner, polygon, vertex) for every corner, in file order.
    let mut corners: Vec<(usize, usize, u32)> = Vec::with_capacity(polygon_index.len());
    for (corner, raw) in polygon_index.iter().enumerate() {
        let (index, last) = if *raw < 0 { (-raw - 1, true) } else { (*raw, false) };
        let vertex =
            u32::try_from(index).ok().filter(|vertex| (*vertex as usize) < count).ok_or_else(|| invalid("a face names a vertex outside the mesh"))?;
        face.push(vertex);
        corners.push((corner, mesh.faces.len(), vertex));
        if last {
            mesh.faces.push(std::mem::take(&mut face));
        }
    }
    // Edges without faces (FBX lists every edge; those of no face are loose).
    // Two-corner "faces" are lines.
    let mut faces = Vec::with_capacity(mesh.faces.len());
    let mut kept_faces = Vec::with_capacity(mesh.faces.len());
    for (index, face) in std::mem::take(&mut mesh.faces).into_iter().enumerate() {
        match face.as_slice() {
            [a, b] if a != b => mesh.loose_edges.push([*a, *b]),
            _ if face.len() >= 3 && face.iter().collect::<BTreeSet<_>>().len() == face.len() => {
                faces.push(face);
                kept_faces.push(index);
            }
            _ => {}
        }
    }
    mesh.faces = faces;
    // UVs.
    if let Some(element) = geometry.child("LayerElementUV")
        && let Some(values) = element.child("UV").and_then(|node| node.prop(0)).and_then(Prop::as_f64s)
    {
        let uvs: Vec<Vec2> = values.as_chunks::<2>().0.iter().map(|[u, v]| Vec2::new(*u as f32, *v as f32)).collect();
        if let Some(per) = per_corner(element, &uvs, &corners, count) {
            // Corners run polygon by polygon: regroup them, keeping the polygons kept.
            let mut all: Vec<Vec<Vec2>> = Vec::new();
            for (slot, (_, polygon, _)) in corners.iter().enumerate() {
                if all.len() <= *polygon {
                    all.resize_with(polygon + 1, Vec::new);
                }
                if let (Some(list), Some(uv)) = (all.get_mut(*polygon), per.get(slot)) {
                    list.push(*uv);
                }
            }
            let by_face: Vec<Vec<Vec2>> = kept_faces.iter().map(|index| all.get(*index).cloned().unwrap_or_default()).collect();
            mesh.uvs = by_face;
            if !mesh.uvs_aligned() {
                mesh.uvs.clear();
            }
        }
    }
    // Smooth when the stored normals differ from the faces' own.
    if let Some(element) = geometry.child("LayerElementNormal")
        && let Some(values) = element.child("Normals").and_then(|node| node.prop(0)).and_then(Prop::as_f64s)
    {
        let normals: Vec<DVec3> = values.as_chunks::<3>().0.iter().map(|normal| DVec3::from_array(*normal)).collect();
        if let Some(per) = per_corner(element, &normals, &corners, count) {
            let mut differ = 0usize;
            let mut total = 0usize;
            for (slot, (_, face_index, _)) in corners.iter().enumerate() {
                let Some(position) = kept_faces.iter().position(|kept| kept == face_index) else { continue };
                let Some(face) = mesh.faces.get(position) else { continue };
                let flat = mesh.face_normal(face).as_dvec3();
                if let Some(normal) = per.get(slot) {
                    total += 1;
                    if normal.normalize_or_zero().dot(flat) < 0.999 {
                        differ += 1;
                    }
                }
            }
            mesh.smooth = total > 0 && differ * 2 > total;
        }
    }
    // Shape keys: Shape geometries under BlendShapeChannel deformers.
    let id = geometry.prop(0).and_then(Prop::as_i64).unwrap_or_default();
    for (deformer, _) in document.children(id) {
        if object_class(deformer) != "BlendShape" {
            continue;
        }
        let deformer_id = deformer.prop(0).and_then(Prop::as_i64).unwrap_or_default();
        for (channel, _) in document.children(deformer_id) {
            if object_class(channel) != "BlendShapeChannel" {
                continue;
            }
            let channel_id = channel.prop(0).and_then(Prop::as_i64).unwrap_or_default();
            let percent = channel.child("DeformPercent").and_then(|node| node.prop(0)).and_then(Prop::as_f64).unwrap_or(0.0);
            let Some((shape, _)) = document.children(channel_id).find(|(child, _)| child.name == "Geometry") else { continue };
            let indexes = shape.child("Indexes").and_then(|node| node.prop(0)).and_then(Prop::as_i64s).unwrap_or_default();
            let offsets = shape.child("Vertices").and_then(|node| node.prop(0)).and_then(Prop::as_f64s).unwrap_or_default();
            let mut key = vec![Vec3::ZERO; count];
            for (index, offset) in indexes.iter().zip(offsets.as_chunks::<3>().0) {
                if let Some(slot) = usize::try_from(*index).ok().and_then(|index| key.get_mut(index)) {
                    *slot = geometric.transform_vector3(DVec3::from_array(*offset)).as_vec3();
                }
            }
            let name = object_name(channel);
            mesh.shape_keys.push(ShapeKey {
                name: if name.is_empty() { object_name(shape) } else { name },
                offsets: key,
                value: (percent / 100.0).clamp(-10.0, 10.0) as f32,
            });
        }
    }
    mesh.validate()?;
    Ok(mesh)
}

fn read_camera(attribute: Option<&Node>, document: &Document<'_>) -> Camera {
    let props = attribute.map(properties).unwrap_or_default();
    let mut camera = Camera::default();
    let unit = document.to_scene.x_axis.truncate().length();
    camera.lens = number_of(&props, "FocalLength", 50.0).clamp(1.0, 5000.0) as f32;
    let film = number_of(&props, "FilmWidth", 36.0 / 25.4);
    camera.sensor = (film * 25.4).clamp(1.0, f64::from(f32::MAX)) as f32;
    camera.sensor_height = (number_of(&props, "FilmHeight", 24.0 / 25.4) * 25.4).clamp(1.0, f64::from(f32::MAX)) as f32;
    camera.clip_start = (number_of(&props, "NearPlane", 10.0) * unit).max(1e-4) as f32;
    camera.clip_end = (number_of(&props, "FarPlane", 10_000.0) * unit).max(f64::from(camera.clip_start) * 2.0) as f32;
    if int_of(&props, "CameraProjectionType", 0) == 1 {
        camera.orthographic = true;
        camera.ortho_scale = number_of(&props, "OrthoZoom", 6.0).max(1e-3) as f32;
    }
    camera
}

fn read_light(attribute: Option<&Node>) -> Light {
    let props = attribute.map(properties).unwrap_or_default();
    let kind = match int_of(&props, "LightType", 0) {
        1 => LightKind::Sun,
        2 => LightKind::Spot,
        3 => LightKind::Area,
        _ => LightKind::Point,
    };
    let mut light = Light::new(kind);
    light.color = vec3_of(&props, "Color", DVec3::ONE).as_vec3();
    light.power = (number_of(&props, "Intensity", 100_000.0) / 100.0).max(0.0) as f32;
    if kind == LightKind::Spot {
        light.spot_size = number_of(&props, "OuterAngle", 45.0).clamp(1.0, 180.0).to_radians() as f32;
    }
    light
}

/// The deepest ancestor model of `bone` that is not a bone (the skeleton's armature), if any.
fn skeleton_owner(document: &Document<'_>, bones: &BTreeSet<i64>, bone: i64) -> Option<i64> {
    let mut current = document.model_parent(bone);
    let mut steps = 0;
    while let Some(at) = current {
        if !bones.contains(&at) {
            return Some(at);
        }
        current = document.model_parent(at);
        steps += 1;
        if steps > bones.len() {
            return None;
        }
    }
    None
}

/// Bones ordered by their depth in the hierarchy, then by id.
fn document_order(document: &Document<'_>, bones: &BTreeSet<i64>) -> Vec<i64> {
    let depth = |id: i64| {
        let mut depth = 0usize;
        let mut current = document.model_parent(id);
        while let Some(parent) = current {
            depth += 1;
            if depth > bones.len() + 64 {
                break;
            }
            current = document.model_parent(parent);
        }
        depth
    };
    let mut ordered: Vec<i64> = bones.iter().copied().collect();
    ordered.sort_by_key(|id| (depth(*id), *id));
    ordered
}

/// Bones into armatures (one per skeleton owner), skin clusters into vertex groups and
/// Armature modifiers.
fn build_armatures(
    document: &Document<'_>,
    bones: &BTreeSet<i64>,
    file_world: &HashMap<i64, DMat4>,
    made: &mut HashMap<i64, Made>,
    imported: &mut Imported,
) -> Result<(), SceneError> {
    if bones.is_empty() {
        return Ok(());
    }
    // Bind matrices (file space) from the clusters, where they are given.
    let mut bind: HashMap<i64, DMat4> = HashMap::new();
    for node in document.objects.values() {
        if node.name != "Deformer" || object_class(node) != "Cluster" {
            continue;
        }
        let id = node.prop(0).and_then(Prop::as_i64).unwrap_or_default();
        let Some(link) = document.children(id).find_map(|(child, _)| {
            (bones.contains(&child.prop(0).and_then(Prop::as_i64).unwrap_or_default())).then(|| child.prop(0).and_then(Prop::as_i64)).flatten()
        }) else {
            continue;
        };
        if let Some(matrix) = node.child("TransformLink").and_then(|node| node.prop(0)).and_then(Prop::as_f64s).and_then(|values| dmat4(&values)) {
            bind.insert(link, matrix);
        }
    }
    // Group bones by owner; a skeleton with no non-bone ancestor gets a new armature object.
    let mut owners: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    // Parents before children (the models are already in that order).
    let ordered: Vec<i64> = document_order(document, bones);
    for bone in &ordered {
        owners.entry(skeleton_owner(document, bones, *bone).unwrap_or(0)).or_default().push(*bone);
    }
    let mut armature_of_bone: HashMap<i64, (usize, String)> = HashMap::new();
    let mut armature_worlds: HashMap<usize, DMat4> = HashMap::new();
    for (owner, members) in &owners {
        let (index, armature_world) = match made.get(owner) {
            Some(owner_made)
                if matches!(
                    imported.objects.get(owner_made.index).map(|object| &object.data),
                    Some(ObjectData::Armature(_)) | Some(ObjectData::Empty { .. })
                ) =>
            {
                (owner_made.index, owner_made.world)
            }
            _ => {
                let index = imported.objects.len();
                imported.objects.push(ImportedObject {
                    name: "Armature".into(),
                    data: ObjectData::Armature(Armature::default()),
                    location: Vec3::ZERO,
                    rotation: Vec3::ZERO,
                    scale: Vec3::ONE,
                    parent: made.get(owner).map(|owner| owner.index),
                    parent_inverse: Mat4::IDENTITY,
                    material: None,
                });
                (index, made.get(owner).map_or(DMat4::IDENTITY, |owner| owner.world))
            }
        };
        armature_worlds.insert(index, armature_world);
        let to_armature = armature_world.inverse();
        let mut armature = Armature::default();
        // Bones by name; the rest matrix in the armature's space (scale taken out).
        let mut rests: HashMap<i64, (DVec3, DMat3)> = HashMap::new();
        for bone in members {
            let world = bind.get(bone).copied().or_else(|| file_world.get(bone).copied()).unwrap_or(DMat4::IDENTITY);
            let in_armature = to_armature * document.to_scene * world;
            let (_, rotation, translation) = in_armature.to_scale_rotation_translation();
            rests.insert(*bone, (translation, DMat3::from_quat(rotation)));
        }
        // Leaf "_end" bones (and only those) give their parent's tail and are not bones.
        let ends: BTreeSet<i64> = members
            .iter()
            .copied()
            .filter(|bone| {
                document.object(*bone).is_some_and(|node| object_name(node).ends_with("_end"))
                    && !members.iter().any(|other| document.model_parent(*other) == Some(*bone))
            })
            .collect();
        for bone in members.iter().filter(|bone| !ends.contains(bone)) {
            let Some(node) = document.object(*bone) else { continue };
            let Some((head, rotation)) = rests.get(bone).copied() else { continue };
            let axis = rotation * DVec3::Y;
            // The tail: the nearest child head along the bone's axis, else a tenth of a metre.
            let length = members
                .iter()
                .filter(|other| document.model_parent(**other) == Some(*bone))
                .filter_map(|child| rests.get(child).map(|(child_head, _)| (*child_head - head).dot(axis)))
                .filter(|along| *along > 1e-5)
                .fold(None, |best: Option<f64>, along| Some(best.map_or(along, |best| best.min(along))))
                .unwrap_or(0.1);
            let tail = head + axis * length;
            let mut made_bone = Bone::new(object_name(node), head.as_vec3(), tail.as_vec3());
            made_bone.roll = roll_towards(axis.as_vec3(), (rotation * DVec3::Z).as_vec3());
            if let Some(parent) = document.model_parent(*bone).filter(|parent| members.contains(parent) && !ends.contains(parent)) {
                made_bone.parent = document.object(parent).map(object_name);
                made_bone.connected = rests.get(&parent).is_some_and(|(parent_head, parent_rotation)| {
                    let parent_axis = *parent_rotation * DVec3::Y;
                    let parent_length = (head - *parent_head).dot(parent_axis);
                    (*parent_head + parent_axis * parent_length - head).length() < 1e-4
                });
            }
            made_bone.name = armature.unique_name(&made_bone.name);
            armature_of_bone.insert(*bone, (index, made_bone.name.clone()));
            armature.bones.push(made_bone);
        }
        armature.validate()?;
        if let Some(object) = imported.objects.get_mut(index) {
            object.data = ObjectData::Armature(armature);
        }
        made.insert(-1 - i64::try_from(index).unwrap_or(0), Made { model: *owner, index, world: armature_world, turn: DMat4::IDENTITY });
    }
    // Skins: each mesh's clusters become vertex groups named after their bones.
    for (model, model_made) in made.iter() {
        let Some(ImportedObject { data: ObjectData::Mesh(_), .. }) = imported.objects.get(model_made.index) else { continue };
        let Some((geometry, _)) = document.children(*model).find(|(child, property)| child.name == "Geometry" && property.is_none()) else {
            continue;
        };
        let geometry_id = geometry.prop(0).and_then(Prop::as_i64).unwrap_or_default();
        let mut groups: Vec<VertexGroup> = Vec::new();
        let mut rig = None;
        for (skin, _) in document.children(geometry_id) {
            if object_class(skin) != "Skin" {
                continue;
            }
            let skin_id = skin.prop(0).and_then(Prop::as_i64).unwrap_or_default();
            for (cluster, _) in document.children(skin_id) {
                let cluster_id = cluster.prop(0).and_then(Prop::as_i64).unwrap_or_default();
                let Some((armature, name)) = document
                    .children(cluster_id)
                    .find_map(|(child, _)| armature_of_bone.get(&child.prop(0).and_then(Prop::as_i64).unwrap_or_default()).cloned())
                else {
                    continue;
                };
                rig = Some(armature);
                let indexes = cluster.child("Indexes").and_then(|node| node.prop(0)).and_then(Prop::as_i64s).unwrap_or_default();
                let weights = cluster.child("Weights").and_then(|node| node.prop(0)).and_then(Prop::as_f64s).unwrap_or_default();
                let count = match imported.objects.get(model_made.index).map(|object| &object.data) {
                    Some(ObjectData::Mesh(mesh)) => mesh.positions.len(),
                    _ => 0,
                };
                let mut group = VertexGroup { name, weights: vec![0.0; count] };
                for (index, weight) in indexes.iter().zip(&weights) {
                    if let Some(slot) = usize::try_from(*index).ok().and_then(|index| group.weights.get_mut(index)) {
                        *slot = (*weight as f32).clamp(0.0, 1.0);
                    }
                }
                groups.push(group);
            }
        }
        let rig_world = rig.and_then(|rig| armature_worlds.get(&rig).copied());
        if let (Some(rig), Some(object)) = (rig, imported.objects.get_mut(model_made.index)) {
            if let ObjectData::Mesh(mesh) = &mut object.data {
                mesh.groups = groups;
            }
            // A skinned mesh at the root goes under its armature (keeping where it is), as the
            // files' writers took it out from there.
            if object.parent.is_none()
                && let Some(rig_world) = rig_world
            {
                object.parent = Some(rig);
                object.parent_inverse = rig_world.inverse().as_mat4();
            }
            imported.deforms.push((model_made.index, rig));
        }
    }
    Ok(())
}

fn dmat4(values: &[f64]) -> Option<DMat4> {
    let array: [f64; 16] = values.get(..16)?.try_into().ok()?;
    let matrix = DMat4::from_cols_array(&array);
    matrix.is_finite().then_some(matrix)
}

/// Animation: each animated model's curves sampled at their keys, as scene transforms (for
/// objects) or poses (for bones), and shape key values.
fn read_animation(
    document: &Document<'_>,
    bones: &BTreeSet<i64>,
    made: &HashMap<i64, Made>,
    file_world: &HashMap<i64, DMat4>,
    imported: &mut Imported,
) {
    // Every model's curves.
    let mut animated: HashMap<i64, ModelCurves> = HashMap::new();
    for id in document.objects.keys() {
        if document.object(*id).is_some_and(Document::is_model) {
            let curves = curves_of(document, *id);
            if !curves.curves.is_empty() {
                animated.insert(*id, curves);
            }
        }
    }
    let mut budget = MAX_SAMPLES;
    // The local matrix of a model at `time`.
    let local_at = |id: i64, time: Option<f64>| -> DMat4 {
        let Some(node) = document.object(id) else { return DMat4::IDENTITY };
        let props = properties(node);
        let mut trs = rest_trs(&props);
        if let (Some(time), Some(curves)) = (time, animated.get(&id)) {
            for ((property, component), curve) in &curves.curves {
                let Some(value) = curve.evaluate(time) else { continue };
                let target = match *property {
                    "T" => &mut trs.translation,
                    "R" => &mut trs.rotation,
                    "S" => &mut trs.scale,
                    _ => continue,
                };
                match component {
                    0 => target.x = value,
                    1 => target.y = value,
                    _ => target.z = value,
                }
            }
        }
        local_matrix(&props, trs)
    };
    let world_at = |id: i64, time: f64| -> DMat4 {
        let mut chain = vec![id];
        let mut current = document.model_parent(id);
        while let Some(parent) = current {
            if chain.len() > document.objects.len() {
                break;
            }
            chain.push(parent);
            current = document.model_parent(parent);
        }
        chain.iter().rev().fold(DMat4::IDENTITY, |world, model| world * local_at(*model, Some(time)))
    };
    let made_by_model: HashMap<i64, &Made> = made.values().map(|entry| (entry.model, entry)).collect();
    // Objects whose transform (or an ancestor's, for the world) is animated.
    for (id, curves) in &animated {
        let mut times: BTreeSet<i64> = BTreeSet::new();
        for curve in curves.curves.values() {
            times.extend(curve.keys.iter().map(|(time, _)| (time * 1e6).round() as i64));
        }
        let samples: Vec<f64> = times.iter().map(|time| *time as f64 / 1e6).collect();
        if samples.len() > budget {
            break;
        }
        budget -= samples.len();
        let interpolation = if curves.curves.values().all(|curve| curve.modes.iter().all(|mode| mode.0 == 0)) {
            Interpolation::Constant
        } else {
            Interpolation::Linear
        };
        if bones.contains(id) {
            // A pose bone: its local matrix against its rest local matrix.
            let Some(node) = document.object(*id) else { continue };
            let Some(armature) = made.values().find(|entry| {
                imported
                    .objects
                    .get(entry.index)
                    .is_some_and(|object| matches!(&object.data, ObjectData::Armature(armature) if armature.bone(&object_name(node)).is_some()))
            }) else {
                continue;
            };
            let name = object_name(node);
            let rest = local_at(*id, None);
            let mut values: Vec<[f32; 9]> = Vec::new();
            for time in &samples {
                let pose = (rest.inverse() * local_at(*id, Some(*time))).as_mat4();
                let (location, rotation, scale) = decompose(pose).unwrap_or((Vec3::ZERO, Vec3::ZERO, Vec3::ONE));
                values.push([location.x, location.y, location.z, rotation.x, rotation.y, rotation.z, scale.x, scale.y, scale.z]);
            }
            unwrap_rotations(&mut values, 3);
            push_channels(
                imported,
                document.offset,
                armature.index,
                &name,
                &[Property::BoneLocation, Property::BoneRotation, Property::BoneScale],
                &samples,
                &values,
                interpolation,
            );
            continue;
        }
        if curves.curves.keys().any(|(property, _)| *property == "D") {
            continue;
        }
        let Some(entry) = made_by_model.get(id) else { continue };
        let parent_world = |time: f64| document.model_parent(*id).map_or(DMat4::IDENTITY, |parent| world_at(parent, time));
        let parent_turn = document.model_parent(*id).and_then(|parent| made_by_model.get(&parent)).map_or(DMat4::IDENTITY, |parent| parent.turn);
        let mut values: Vec<[f32; 9]> = Vec::new();
        for time in &samples {
            let parent_scene = document.to_scene * parent_world(*time) * parent_turn.inverse();
            let world = document.to_scene * world_at(*id, *time) * entry.turn.inverse();
            let object = imported.objects.get(entry.index);
            let parent_inverse = object.map_or(DMat4::IDENTITY, |object| object.parent_inverse.as_dmat4());
            let local = if object.is_some_and(|object| object.parent.is_some()) { (parent_scene * parent_inverse).inverse() * world } else { world };
            let (location, rotation, scale) = decompose(local.as_mat4()).unwrap_or((Vec3::ZERO, Vec3::ZERO, Vec3::ONE));
            values.push([location.x, location.y, location.z, rotation.x, rotation.y, rotation.z, scale.x, scale.y, scale.z]);
        }
        unwrap_rotations(&mut values, 3);
        push_channels(
            imported,
            document.offset,
            entry.index,
            "",
            &[Property::Location, Property::Rotation, Property::Scale],
            &samples,
            &values,
            interpolation,
        );
    }
    // Shape key values (DeformPercent on a BlendShapeChannel).
    for node in document.objects.values() {
        if object_class(node) != "BlendShapeChannel" {
            continue;
        }
        let id = node.prop(0).and_then(Prop::as_i64).unwrap_or_default();
        let curves = curves_of(document, id);
        let Some(curve) = curves.curves.get(&("D", 0)) else { continue };
        // The mesh the channel's blend shape deforms.
        let Some(geometry) = document.parents(id).find_map(|(deformer, _)| document.parents(deformer).map(|(geometry, _)| geometry).next()) else {
            continue;
        };
        let Some(model) = document.parents(geometry).find_map(|(model, _)| made_by_model.get(&model).copied()) else { continue };
        imported.animation.push(ImportedChannel {
            object: model.index,
            property: Property::ShapeKey,
            index: 0,
            path: object_name(node),
            keys: curve.keys.iter().map(|(time, value)| ((*time + document.offset) as f32, (*value / 100.0) as f32)).collect(),
            interpolation: Interpolation::Linear,
        });
    }
    let _ = file_world;
}

/// Keeps Euler angles continuous between samples (no 2π jumps).
fn unwrap_rotations(values: &mut [[f32; 9]], start: usize) {
    for index in 1..values.len() {
        for component in start..start + 3 {
            let (Some(previous), Some(current)) =
                (values.get(index - 1).and_then(|row| row.get(component)).copied(), values.get(index).and_then(|row| row.get(component)).copied())
            else {
                continue;
            };
            let mut value = current;
            while value - previous > std::f32::consts::PI {
                value -= std::f32::consts::TAU;
            }
            while previous - value > std::f32::consts::PI {
                value += std::f32::consts::TAU;
            }
            if let Some(slot) = values.get_mut(index).and_then(|row| row.get_mut(component)) {
                *slot = value;
            }
        }
    }
}

fn push_channels(
    imported: &mut Imported,
    offset: f64,
    object: usize,
    path: &str,
    properties: &[Property; 3],
    samples: &[f64],
    values: &[[f32; 9]],
    interpolation: Interpolation,
) {
    for (slot, property) in properties.iter().enumerate() {
        for component in 0..3 {
            let column = slot * 3 + component;
            let keys: Vec<(f32, f32)> =
                samples.iter().zip(values).map(|(time, row)| ((*time + offset) as f32, row.get(column).copied().unwrap_or(0.0))).collect();
            // Channels that never move are left out.
            let first = keys.first().map_or(0.0, |key| key.1);
            if keys.iter().all(|key| (key.1 - first).abs() < 1e-6) && keys.len() > 1 {
                continue;
            }
            imported.animation.push(ImportedChannel {
                object,
                property: *property,
                index: u8::try_from(component).unwrap_or(0),
                path: path.to_string(),
                keys,
                interpolation,
            });
        }
    }
}
