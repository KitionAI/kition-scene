//! The scene: objects (meshes, lights, cameras, empties) in a parent hierarchy, their
//! materials, the world, and the render settings. Z is up, units are metres, and rotations
//! are XYZ Euler angles in radians (X applied first).

use glam::{Mat3, Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::error::{Result, SceneError};
use crate::mesh::{Bounds, Mesh};
use crate::modifiers::Modifier;

/// A stable object, material, or other id within one scene.
pub type Id = u32;

/// A point, sun, spot, or area light.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LightKind {
    #[default]
    Point,
    Sun,
    Spot,
    Area,
}

/// A light's settings. Point, spot, and area power is in watts; a sun's strength is in W/m².
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Light {
    pub kind: LightKind,
    pub color: Vec3,
    pub power: f32,
    /// The soft-shadow radius (point and spot) or the side (area), in metres.
    #[serde(default = "Light::default_radius")]
    pub radius: f32,
    /// A spot's cone angle, in radians.
    #[serde(default = "Light::default_spot_size")]
    pub spot_size: f32,
}

impl Light {
    fn default_radius() -> f32 {
        0.1
    }

    fn default_spot_size() -> f32 {
        45f32.to_radians()
    }

    /// A light of `kind` with the power a new one gets.
    pub fn new(kind: LightKind) -> Self {
        let power = match kind {
            LightKind::Point | LightKind::Spot => 1000.0,
            LightKind::Sun => 1.0,
            LightKind::Area => 1000.0,
        };
        Self { kind, color: Vec3::ONE, power, radius: Self::default_radius(), spot_size: Self::default_spot_size() }
    }
}

/// Which dimension of a camera's frame holds its sensor size or orthographic scale.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SensorFit {
    #[default]
    Auto,
    Horizontal,
    Vertical,
}

/// A camera's lens and sensor dimensions, in millimetres. Render pixels are square.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Camera {
    pub lens: f32,
    pub sensor: f32,
    #[serde(default = "Camera::default_sensor_height")]
    pub sensor_height: f32,
    #[serde(default)]
    pub sensor_fit: SensorFit,
    pub clip_start: f32,
    pub clip_end: f32,
    #[serde(default)]
    pub orthographic: bool,
    /// The span along the fitted dimension of an orthographic frame, in metres.
    #[serde(default = "Camera::default_ortho_scale")]
    pub ortho_scale: f32,
    /// Depth of field (thin lens), for path-traced renders.
    #[serde(default, skip_serializing_if = "DepthOfField::is_off")]
    pub dof: DepthOfField,
}

/// A camera's depth of field: what is in focus and how quickly the rest blurs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DepthOfField {
    pub enabled: bool,
    /// The aperture as an f-number: the lens's focal length over the opening's diameter.
    pub f_stop: f32,
    /// Metres in front of the camera that are sharp (when no object is focused on).
    pub focus_distance: f32,
    /// An object whose origin is kept in focus instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus_object: Option<Id>,
    /// The opening's blades: 0 for a round opening, 3 or more for a polygon (bokeh shapes).
    pub blades: u32,
    /// The blades' turn, in radians.
    pub rotation: f32,
    /// The opening's height over its width (anamorphic bokeh): above 1 narrows it, below 1
    /// widens it.
    pub ratio: f32,
}

impl Default for DepthOfField {
    /// The reference's camera defaults: off, f/2.8, focused 10 m away, round, ratio 1.
    fn default() -> Self {
        Self { enabled: false, f_stop: 2.8, focus_distance: 10.0, focus_object: None, blades: 0, rotation: 0.0, ratio: 1.0 }
    }
}

/// The most blades an opening takes.
pub const MAX_BLADES: u32 = 64;

impl DepthOfField {
    fn is_off(&self) -> bool {
        *self == Self::default()
    }
}

impl Camera {
    fn default_sensor_height() -> f32 {
        24.0
    }

    fn default_ortho_scale() -> f32 {
        6.0
    }

    /// Half the frame's width and height one metre in front of the camera. Auto uses
    /// sensor width on the longer image dimension; Vertical alone uses sensor height.
    /// Orthographic dimensions are independent of distance and sensor dimensions.
    pub fn frame_half_size(&self, aspect: f32) -> (f32, f32) {
        let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 1.0 };
        let vertical = self.sensor_fit == SensorFit::Vertical || (self.sensor_fit == SensorFit::Auto && aspect < 1.0);
        let sensor = if self.sensor_fit == SensorFit::Vertical { self.sensor_height } else { self.sensor };
        let half = if self.orthographic { self.ortho_scale / 2.0 } else { sensor / (2.0 * self.lens) };
        if vertical { (half * aspect, half) } else { (half, half / aspect) }
    }
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            lens: 50.0,
            sensor: 36.0,
            sensor_height: Self::default_sensor_height(),
            sensor_fit: SensorFit::Auto,
            clip_start: 0.1,
            clip_end: 100.0,
            orthographic: false,
            ortho_scale: Self::default_ortho_scale(),
            dof: DepthOfField::default(),
        }
    }
}

/// What an object is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ObjectData {
    Mesh(Mesh),
    Light(Light),
    Camera(Camera),
    /// An empty: a transform with nothing in it, drawn as axes of `size`.
    Empty {
        size: f32,
    },
    /// A skeleton of bones that deforms meshes through their Armature modifiers.
    Armature(crate::armature::Armature),
    /// Splines made into a wire, a swept tube, or a filled shape.
    Curve(crate::curve::Curve),
    /// Letters set in a font, made into a filled (and possibly extruded) shape.
    Text(crate::text::Text),
    /// A grid of points that bends meshes through their Lattice modifiers.
    Lattice(crate::lattice::Lattice),
    /// NURBS patches made into a grid of faces.
    Surface(crate::surface::Surface),
}

impl ObjectData {
    /// The kind's name as commands and inspect report it.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Mesh(_) => "mesh",
            Self::Light(_) => "light",
            Self::Camera(_) => "camera",
            Self::Empty { .. } => "empty",
            Self::Armature(_) => "armature",
            Self::Curve(_) => "curve",
            Self::Text(_) => "text",
            Self::Lattice(_) => "lattice",
            Self::Surface(_) => "surface",
        }
    }
}

/// One object in the scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Object {
    pub id: Id,
    pub name: String,
    pub data: ObjectData,
    pub location: Vec3,
    /// XYZ Euler angles in radians.
    pub rotation: Vec3,
    pub scale: Vec3,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Id>,
    /// The inverse of the parent's world matrix when the parent was set, so parenting does not
    /// move the child.
    #[serde(default = "identity", skip_serializing_if = "is_identity")]
    pub parent_inverse: Mat4,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<Modifier>,
    /// Constraints, evaluated after the object's own transform, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<crate::constraints::Constraint>,
    /// Rigid body, cloth, and collision settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physics: Option<crate::physics::ObjectPhysics>,
    /// Particle systems (emitters and hair) on a mesh, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub particles: Vec<crate::particles::ParticleSystem>,
    /// Nonlinear animation: tracks of action strips under the object's own keys.
    #[serde(default, skip_serializing_if = "crate::nla::Nla::is_empty")]
    pub nla: crate::nla::Nla,
}

fn identity() -> Mat4 {
    Mat4::IDENTITY
}

fn is_identity(matrix: &Mat4) -> bool {
    *matrix == Mat4::IDENTITY
}

/// The rotation matrix of XYZ Euler angles: X first, then Y, then Z (all about fixed axes).
pub fn euler_matrix(rotation: Vec3) -> Mat3 {
    Mat3::from_rotation_z(rotation.z) * Mat3::from_rotation_y(rotation.y) * Mat3::from_rotation_x(rotation.x)
}

/// The XYZ Euler angles of a rotation (the inverse of [`euler_matrix`]).
pub fn matrix_euler(rotation: Mat3) -> Vec3 {
    let (z, y, x) = Quat::from_mat3(&rotation).to_euler(glam::EulerRot::ZYX);
    Vec3::new(x, y, z)
}

impl Object {
    /// A new object at the origin.
    pub fn new(id: Id, name: impl Into<String>, data: ObjectData) -> Self {
        Self {
            id,
            name: name.into(),
            data,
            location: Vec3::ZERO,
            rotation: Vec3::ZERO,
            scale: Vec3::ONE,
            parent: None,
            parent_inverse: Mat4::IDENTITY,
            hidden: false,
            material: None,
            modifiers: Vec::new(),
            constraints: Vec::new(),
            physics: None,
            particles: Vec::new(),
            nla: crate::nla::Nla::default(),
        }
    }

    /// The object's own transform, relative to its parent.
    pub fn local_matrix(&self) -> Mat4 {
        Mat4::from_translation(self.location) * Mat4::from_mat3(euler_matrix(self.rotation)) * Mat4::from_scale(self.scale)
    }

    /// Sets location, rotation, and scale from a matrix (shear is lost).
    pub fn set_local_matrix(&mut self, matrix: Mat4) {
        let (scale, rotation, location) = matrix.to_scale_rotation_translation();
        self.location = location;
        self.rotation = matrix_euler(Mat3::from_quat(rotation));
        self.scale = scale;
    }

    /// The mesh, for mesh objects.
    /// The surface the object draws before its modifiers: a mesh object's mesh, or the mesh a
    /// curve or a text makes.
    pub fn base_mesh(&self, scene: &Scene) -> Option<Mesh> {
        match &self.data {
            ObjectData::Mesh(mesh) => Some(mesh.clone()),
            ObjectData::Curve(curve) => Some(curve.to_mesh_with(self.curve_shapes(scene))),
            ObjectData::Text(text) => {
                let path = self.follow_path(scene);
                Some(text.to_mesh_on(path.as_ref()))
            }
            ObjectData::Surface(surface) => Some(surface.to_mesh()),
            _ => None,
        }
    }

    /// The bevel and taper objects' curves a curve object reads (never its own).
    pub fn curve_shapes<'a>(&self, scene: &'a Scene) -> crate::curve::Shapes<'a> {
        let ObjectData::Curve(curve) = &self.data else { return crate::curve::Shapes::default() };
        let other = |id: Option<Id>| {
            id.filter(|id| *id != self.id).and_then(|id| scene.object(id).ok()).and_then(|object| match &object.data {
                ObjectData::Curve(curve) => Some((curve, object.scale)),
                _ => None,
            })
        };
        crate::curve::Shapes { bevel: other(curve.bevel_object), taper: other(curve.taper_object).map(|(curve, _)| curve) }
    }

    /// The path a text object's letters follow, if it follows a curve.
    pub fn follow_path(&self, scene: &Scene) -> Option<crate::text::Path> {
        let ObjectData::Text(text) = &self.data else { return None };
        let id = text.follow_curve.filter(|id| *id != self.id)?;
        let object = scene.object(id).ok()?;
        let ObjectData::Curve(curve) = &object.data else { return None };
        crate::text::Path::of(curve, object.scale)
    }

    /// The objects whose own data this object's shape reads: a curve's bevel and taper objects,
    /// the curve a text follows.
    pub fn shape_sources(&self) -> Vec<Id> {
        match &self.data {
            ObjectData::Curve(curve) => [curve.bevel_object, curve.taper_object].into_iter().flatten().filter(|id| *id != self.id).collect(),
            ObjectData::Text(text) => text.follow_curve.into_iter().filter(|id| *id != self.id).collect(),
            _ => Vec::new(),
        }
    }

    /// Whether the object draws a surface or a wire: a mesh, a curve, a text, or a surface.
    pub fn has_geometry(&self) -> bool {
        matches!(self.data, ObjectData::Mesh(_) | ObjectData::Curve(_) | ObjectData::Text(_) | ObjectData::Surface(_))
    }

    pub fn mesh(&self) -> Option<&Mesh> {
        match &self.data {
            ObjectData::Mesh(mesh) => Some(mesh),
            _ => None,
        }
    }

    /// The mesh, for mesh objects, to change.
    pub fn mesh_mut(&mut self) -> Option<&mut Mesh> {
        match &mut self.data {
            ObjectData::Mesh(mesh) => Some(mesh),
            _ => None,
        }
    }

    /// The mesh after the object's enabled modifiers, for mesh objects.
    pub fn evaluated_mesh(&self, scene: &Scene) -> Option<Mesh> {
        self.evaluated_mesh_at(scene, 0)
    }

    /// [`Object::evaluated_mesh`] reached `depth` booleans deep (a boolean operand's own
    /// booleans are one deeper; a cycle stops at a fixed depth).
    pub fn evaluated_mesh_at(&self, scene: &Scene, depth: u32) -> Option<Mesh> {
        let mut mesh = self.base_mesh(scene)?;
        // Shape keys come first, as in the reference application.
        crate::vertex_data::apply_shape_keys(&mut mesh);
        // A baked cloth moves the vertices next, before the modifiers.
        if let Some(positions) = scene.simulated_cloth(self.id).filter(|positions| positions.len() == mesh.positions.len()) {
            mesh.positions = positions;
        }
        let mut stack = crate::modifiers::Stack::default();
        for modifier in self.modifiers.iter().filter(|modifier| modifier.enabled) {
            mesh = stack.apply(modifier, mesh, self, scene, depth);
        }
        Some(mesh)
    }

    /// A hash of everything the evaluated mesh depends on: the mesh, the modifiers, and for
    /// booleans the operands' own keys and where they sit relative to this object. Equal keys
    /// mean an evaluated mesh can be reused.
    pub fn evaluation_key(&self, scene: &Scene) -> u64 {
        let key = self.evaluation_key_at(scene, 0);
        match scene.cloth_key(self.id) {
            Some(cloth) => {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                (key, cloth).hash(&mut hasher);
                hasher.finish()
            }
            None => key,
        }
    }

    /// [`Object::evaluation_key`] without a baked cloth's motion (what a bake starts from).
    pub fn rest_key(&self, scene: &Scene) -> u64 {
        self.evaluation_key_at(scene, 0)
    }

    fn evaluation_key_at(&self, scene: &Scene, depth: u32) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.data.kind().hash(&mut hasher);
        if let Some(mesh) = self.mesh() {
            mesh.smooth.hash(&mut hasher);
            mesh.smooth_angle.map(f32::to_bits).hash(&mut hasher);
            for point in &mesh.positions {
                point.to_array().map(f32::to_bits).hash(&mut hasher);
            }
            mesh.faces.hash(&mut hasher);
            mesh.loose_edges.hash(&mut hasher);
            for point in mesh.multires.iter().flatten() {
                point.to_array().map(f32::to_bits).hash(&mut hasher);
            }
        }
        if matches!(self.data, ObjectData::Curve(_) | ObjectData::Text(_) | ObjectData::Surface(_)) {
            serde_json::to_string(&self.data).unwrap_or_default().hash(&mut hasher);
        }
        // Bevel, taper, and followed curves: their data and scale.
        for source in self.shape_sources() {
            if let Ok(other) = scene.object(source) {
                serde_json::to_string(&other.data).unwrap_or_default().hash(&mut hasher);
                other.scale.to_array().map(f32::to_bits).hash(&mut hasher);
            }
        }
        crate::armature::hash_deformation(self, scene, &mut hasher);
        serde_json::to_string(&self.modifiers).unwrap_or_default().hash(&mut hasher);
        for modifier in self.modifiers.iter().filter(|modifier| modifier.enabled) {
            let crate::modifiers::ModifierKind::Boolean { object: Some(operand), .. } = modifier.kind else { continue };
            if depth >= 4 || operand == self.id {
                continue;
            }
            let Ok(other) = scene.object(operand) else { continue };
            other.evaluation_key_at(scene, depth + 1).hash(&mut hasher);
            if let (Ok(mine), Ok(theirs)) = (scene.world_matrix(self.id), scene.world_matrix(operand)) {
                (mine.inverse() * theirs).to_cols_array().map(f32::to_bits).hash(&mut hasher);
            }
        }
        // A shrinkwrap's target and a lattice, and where they sit relative to this object.
        for modifier in self.modifiers.iter().filter(|modifier| modifier.enabled) {
            let (crate::modifiers::ModifierKind::Shrinkwrap { object: Some(other), .. }
            | crate::modifiers::ModifierKind::Lattice { object: Some(other), .. }) = modifier.kind
            else {
                continue;
            };
            if depth >= 4 || other == self.id {
                continue;
            }
            let Ok(target) = scene.object(other) else { continue };
            match &target.data {
                ObjectData::Lattice(lattice) => serde_json::to_string(lattice).unwrap_or_default().hash(&mut hasher),
                _ => target.evaluation_key_at(scene, depth + 1).hash(&mut hasher),
            }
            if let (Ok(mine), Ok(theirs)) = (scene.world_matrix(self.id), scene.world_matrix(other)) {
                (mine.inverse() * theirs).to_cols_array().map(f32::to_bits).hash(&mut hasher);
            }
        }
        // A node group's content, and the objects its Object Info nodes read, with where they are.
        for modifier in self.modifiers.iter().filter(|modifier| modifier.enabled) {
            let crate::modifiers::ModifierKind::GeometryNodes { group: Some(group), .. } = modifier.kind else { continue };
            let Some(group) = scene.node_group(group) else { continue };
            // The groups its Group nodes use, at any depth, count too; and the frame, when a
            // simulation zone or Scene Time reads it.
            let used: Vec<&crate::node_tree::NodeGroup> = scene
                .node_groups
                .iter()
                .filter(|other| other.id == group.id || crate::node_tree::uses_group(&scene.node_groups, group.id, other.id))
                .collect();
            for part in &used {
                serde_json::to_string(part).unwrap_or_default().hash(&mut hasher);
            }
            if used
                .iter()
                .any(|part| part.nodes.iter().any(|node| matches!(node.kind.as_str(), "simulationInput" | "simulationOutput" | "sceneTime")))
            {
                scene.nodes_frame().to_bits().hash(&mut hasher);
                scene.animation.timeline.start.hash(&mut hasher);
                scene.animation.timeline.fps.to_bits().hash(&mut hasher);
            }
            if depth >= 4 {
                continue;
            }
            for node in used.iter().flat_map(|part| part.nodes.iter()).filter(|node| node.kind == "objectInfo") {
                let Some(target) = node.values.get("Object").and_then(serde_json::Value::as_u64).and_then(|id| Id::try_from(id).ok()) else {
                    continue;
                };
                let Ok(other) = scene.object(target) else { continue };
                if target != self.id {
                    other.evaluation_key_at(scene, depth + 1).hash(&mut hasher);
                }
                if let (Ok(mine), Ok(theirs)) = (scene.world_matrix(self.id), scene.world_matrix(target)) {
                    (mine.inverse() * theirs).to_cols_array().map(f32::to_bits).hash(&mut hasher);
                    theirs.to_cols_array().map(f32::to_bits).hash(&mut hasher);
                }
            }
        }
        hasher.finish()
    }
}

/// A surface material (the metallic-roughness model glTF uses).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Material {
    pub id: Id,
    pub name: String,
    /// Linear RGBA.
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    #[serde(default)]
    pub emission: Vec3,
    #[serde(default)]
    pub emission_strength: f32,
    /// The base colour image (sRGB), multiplied by `base_color`: an image's hash in `Scene::images`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_color_texture: Option<String>,
    /// Roughness in the green channel and metallic in the blue one (linear), multiplying
    /// `roughness` and `metallic`, as glTF keeps them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metallic_roughness_texture: Option<String>,
    /// A tangent-space normal map (linear; X right along U, Y up along V, Z out of the surface),
    /// as glTF and MikkTSpace bakers write them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal_texture: Option<String>,
    /// How strongly the normal map bends the surface (glTF's `scale`).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub normal_strength: f32,
    /// How much light passes through the surface (glass at 1), 0 to 1.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub transmission: f32,
    /// The index of refraction (1.5 for glass, the reference's default).
    #[serde(default = "glass_ior", skip_serializing_if = "is_glass_ior")]
    pub ior: f32,
    /// How the base colour's alpha (and the base colour image's) is used.
    #[serde(default, skip_serializing_if = "AlphaMode::is_opaque")]
    pub alpha_mode: AlphaMode,
    /// With [`AlphaMode::Clip`], the alpha below which the surface is not there.
    #[serde(default = "half", skip_serializing_if = "is_half")]
    pub alpha_cutoff: f32,
    /// The shader group (`Tree::Shader`) that gives the surface, in place of the values above
    /// (which stay for when the group is let go).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shader: Option<Id>,
}

/// How a material's alpha works, as glTF names the modes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AlphaMode {
    /// Alpha is ignored.
    #[default]
    Opaque,
    /// The surface is there where alpha reaches the cutoff, and not elsewhere.
    Clip,
    /// The surface covers what is behind it by its alpha.
    Blend,
}

impl AlphaMode {
    fn is_opaque(&self) -> bool {
        *self == Self::Opaque
    }
}

fn is_zero(value: &f32) -> bool {
    *value == 0.0
}

fn glass_ior() -> f32 {
    1.5
}

fn is_glass_ior(value: &f32) -> bool {
    *value == 1.5
}

fn half() -> f32 {
    0.5
}

fn is_half(value: &f32) -> bool {
    *value == 0.5
}

impl Material {
    /// How much of the surface is there at a point, from the base colour's alpha times the
    /// image's: 1 for opaque materials, 0 or 1 for clipped ones.
    pub fn coverage(&self, alpha: f32) -> f32 {
        match self.alpha_mode {
            AlphaMode::Opaque => 1.0,
            AlphaMode::Clip => {
                if alpha >= self.alpha_cutoff {
                    1.0
                } else {
                    0.0
                }
            }
            AlphaMode::Blend => alpha.clamp(0.0, 1.0),
        }
    }
}

fn is_one(value: &f32) -> bool {
    *value == 1.0
}

/// A material's image slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureSlot {
    BaseColor,
    MetallicRoughness,
    Normal,
}

impl TextureSlot {
    /// The slot named as commands name it (`baseColor`, `metallicRoughness`).
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "baseColor" => Some(Self::BaseColor),
            "metallicRoughness" => Some(Self::MetallicRoughness),
            "normal" => Some(Self::Normal),
            _ => None,
        }
    }
}

/// The encoding of an image kept in a scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageFormat {
    Png,
    Jpeg,
    /// Radiance RGBE (`.hdr`): high dynamic range, for world images.
    Hdr,
}

impl ImageFormat {
    /// Recognises PNG and JPEG by their first bytes.
    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(Self::Png)
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(Self::Jpeg)
        } else if bytes.starts_with(b"#?RADIANCE") || bytes.starts_with(b"#?RGBE") {
            Some(Self::Hdr)
        } else {
            None
        }
    }

    /// The file extension.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Hdr => "hdr",
        }
    }

    /// The media type.
    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Hdr => "image/vnd.radiance",
        }
    }
}

/// An image a material uses, kept by the hash of its bytes (so a scene holds each image once).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Image {
    /// The SHA-256 of the bytes, lowercase hex.
    pub hash: String,
    pub name: String,
    pub format: ImageFormat,
    /// The encoded file; stored beside the scene's JSON, never inside it.
    #[serde(skip)]
    pub bytes: std::sync::Arc<Vec<u8>>,
}

/// The largest image a scene takes (64 MiB).
pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;

impl Image {
    /// An image from its encoded bytes (PNG, JPEG, or Radiance HDR).
    pub fn new(name: impl Into<String>, bytes: Vec<u8>) -> Result<Self> {
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(SceneError::params("the image is larger than 64 MiB"));
        }
        let format = ImageFormat::sniff(&bytes).ok_or_else(|| SceneError::params("the image is not a PNG, JPEG, or Radiance HDR file"))?;
        Ok(Self { hash: hash_bytes(&bytes), name: name.into(), format, bytes: std::sync::Arc::new(bytes) })
    }
}

/// SHA-256 of `bytes` as lowercase hex.
pub fn hash_bytes(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

impl Material {
    /// A new material with the defaults a modeller gives one.
    pub fn new(id: Id, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            base_color: [0.8, 0.8, 0.8, 1.0],
            metallic: 0.0,
            roughness: 0.5,
            emission: Vec3::ZERO,
            emission_strength: 0.0,
            base_color_texture: None,
            metallic_roughness_texture: None,
            normal_texture: None,
            normal_strength: 1.0,
            transmission: 0.0,
            ior: 1.5,
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            shader: None,
        }
    }

    /// The images the material uses, by hash.
    pub fn images(&self) -> impl Iterator<Item = &String> {
        [&self.base_color_texture, &self.metallic_roughness_texture, &self.normal_texture].into_iter().flatten()
    }

    /// The image hash in a slot.
    pub fn texture(&self, slot: TextureSlot) -> Option<&str> {
        match slot {
            TextureSlot::BaseColor => self.base_color_texture.as_deref(),
            TextureSlot::MetallicRoughness => self.metallic_roughness_texture.as_deref(),
            TextureSlot::Normal => self.normal_texture.as_deref(),
        }
    }

    /// Sets (or clears) a slot.
    pub fn set_texture(&mut self, slot: TextureSlot, image: Option<String>) {
        match slot {
            TextureSlot::BaseColor => self.base_color_texture = image,
            TextureSlot::MetallicRoughness => self.metallic_roughness_texture = image,
            TextureSlot::Normal => self.normal_texture = image,
        }
    }
}

/// The world: what is behind everything, and the light it gives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct World {
    /// Linear RGB.
    pub color: Vec3,
    /// How strongly the world colour (or image) lights the scene.
    #[serde(default = "one")]
    pub strength: f32,
    /// An equirectangular image (by hash in `Scene::images`) the world shows and lights with
    /// instead of its colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// The image's turn about the up axis, in radians.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rotation: f32,
    /// A shader group (`Tree::Shader`) whose Material Output's Volume fills the space between
    /// surfaces (mist, haze, light shafts), in path-traced renders.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<Id>,
}

fn one() -> f32 {
    1.0
}

impl Default for World {
    fn default() -> Self {
        Self { color: Vec3::splat(0.050_88), strength: 1.0, image: None, rotation: 0.0, volume: None }
    }
}

/// Where the shutter's opening sits around the frame it renders.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShutterPosition {
    /// Opens before the frame and closes after it.
    #[default]
    Center,
    /// Opens at the frame.
    Start,
    /// Closes at the frame.
    End,
}

/// Motion blur: what moves while the shutter is open is blurred along its path.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MotionBlur {
    pub enabled: bool,
    /// How long the shutter is open, in frames (0.5, the reference's default).
    pub shutter: f32,
    pub position: ShutterPosition,
}

impl Default for MotionBlur {
    fn default() -> Self {
        Self { enabled: false, shutter: 0.5, position: ShutterPosition::Center }
    }
}

impl MotionBlur {
    fn is_off(&self) -> bool {
        *self == Self::default()
    }

    /// The longest shutter, in frames.
    pub const MAX_SHUTTER: f32 = 100.0;

    /// The frames the shutter is open between, around `frame`; `None` when motion blur is off.
    pub fn interval(&self, frame: f32) -> Option<(f32, f32)> {
        if !(self.enabled && self.shutter.is_finite() && self.shutter > 0.0 && frame.is_finite()) {
            return None;
        }
        let shutter = self.shutter.min(Self::MAX_SHUTTER);
        Some(match self.position {
            ShutterPosition::Center => (frame - shutter / 2.0, frame + shutter / 2.0),
            ShutterPosition::Start => (frame, frame + shutter),
            ShutterPosition::End => (frame - shutter, frame),
        })
    }
}

/// Ambient occlusion in fast renders: creases and covered places get less of the world's light
/// (the reference's ambient occlusion distance and factor; a path-traced render finds occlusion
/// by tracing and leaves these alone).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AmbientOcclusion {
    pub enabled: bool,
    /// How far around a point other surfaces occlude it, in metres; `None` works it out from
    /// the scene: 6 % of its bounds' diagonal, between 5 cm and 2 m.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance: Option<f32>,
    /// How much of the occlusion shows: 0 none, 1 all of it, more to darken further.
    pub strength: f32,
}

impl Default for AmbientOcclusion {
    fn default() -> Self {
        Self { enabled: true, distance: None, strength: 1.0 }
    }
}

impl AmbientOcclusion {
    /// The longest distance, in metres.
    pub const MAX_DISTANCE: f32 = 1000.0;
    /// The largest strength.
    pub const MAX_STRENGTH: f32 = 4.0;

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The distance a scene whose bounds' diagonal is `diagonal` metres uses.
    pub fn distance_for(&self, diagonal: f32) -> f32 {
        match self.distance {
            Some(distance) if distance.is_finite() && distance > 0.0 => distance.min(Self::MAX_DISTANCE),
            _ => (diagonal * 0.06).clamp(0.05, 2.0),
        }
    }

    /// The light share left at a point whose estimated sky share is `factor`, by the strength.
    pub fn weaken(&self, factor: f32) -> f32 {
        if self.strength == 1.0 {
            return factor;
        }
        let strength = if self.strength.is_finite() { self.strength.clamp(0.0, Self::MAX_STRENGTH) } else { 1.0 };
        (1.0 - strength * (1.0 - factor)).clamp(0.0, 1.0)
    }
}

/// The size of a rendered picture, the compositing group a render passes through, and motion
/// blur.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderSettings {
    pub width: u32,
    pub height: u32,
    /// The compositing node group renders pass through, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compositor: Option<Id>,
    /// Whether renders pass through the compositing group (the reference's Post Processing ▸
    /// Compositing; on by default).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub compositing: bool,
    /// Whether renders show the sequencer's strips, when there are any (the reference's Post
    /// Processing ▸ Sequencer; on by default).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub sequencer: bool,
    #[serde(default, skip_serializing_if = "MotionBlur::is_off")]
    pub motion_blur: MotionBlur,
    /// Ambient occlusion in fast renders.
    #[serde(default, skip_serializing_if = "AmbientOcclusion::is_default")]
    pub ambient_occlusion: AmbientOcclusion,
    /// How a final render's light becomes the picture (view transform, look, exposure).
    #[serde(default, skip_serializing_if = "crate::color::ColorManagement::is_default")]
    pub color_management: crate::color::ColorManagement,
}

fn yes() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            compositor: None,
            compositing: true,
            sequencer: true,
            motion_blur: MotionBlur::default(),
            ambient_occlusion: AmbientOcclusion::default(),
            color_management: crate::color::ColorManagement::default(),
        }
    }
}

impl RenderSettings {
    /// The compositing group a render passes through now: none when compositing is off.
    pub fn active_compositor(&self) -> Option<Id> {
        self.compositor.filter(|_| self.compositing)
    }
}

/// The largest render side.
pub const MAX_RENDER_SIDE: u32 = 8192;

/// A whole scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    pub objects: Vec<Object>,
    #[serde(default)]
    pub materials: Vec<Material>,
    #[serde(default)]
    pub world: World,
    #[serde(default)]
    pub render: RenderSettings,
    /// The camera renders are taken from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<Id>,
    /// The next unused id.
    pub next_id: Id,
    /// The timeline and keyframes.
    #[serde(default, skip_serializing_if = "crate::animation::Animation::is_default")]
    pub animation: crate::animation::Animation,
    /// Images materials use, each once.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<Image>,
    /// The rigid body world's settings (gravity, steps, frames).
    #[serde(default, skip_serializing_if = "crate::physics::PhysicsWorld::is_default")]
    pub physics: crate::physics::PhysicsWorld,
    /// The physics bake the scene shows (session state, never saved).
    #[serde(skip)]
    pub physics_cache: crate::physics::PhysicsCache,
    /// Where this replica mints new ids, when it shares the scene with others (each replica its
    /// own range, so two people adding at once never mint the same id). Not part of the scene.
    #[serde(skip)]
    pub allocator: IdAllocator,
    /// Geometry node groups, which Geometry Nodes modifiers run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub node_groups: Vec<crate::node_tree::NodeGroup>,
    /// The video sequencer's strips.
    #[serde(default, skip_serializing_if = "crate::sequencer::Sequence::is_empty")]
    pub sequence: crate::sequencer::Sequence,
}

/// A range of ids a replica mints from: `[next, end)`. Never part of a scene's content: two
/// scenes that differ only in it are equal.
#[derive(Debug, Clone, Copy, Default)]
pub struct IdAllocator(pub Option<(Id, Id)>);

impl PartialEq for IdAllocator {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Default for Scene {
    fn default() -> Self {
        Self::empty()
    }
}

impl Scene {
    /// The frame node trees see (Scene Time, simulation zones): the frame `go_to_frame` last
    /// went to, else the timeline's.
    pub fn nodes_frame(&self) -> f32 {
        self.physics_cache.frame.unwrap_or(self.animation.timeline.frame as f32)
    }

    /// A scene with nothing in it.
    pub fn empty() -> Self {
        Self {
            objects: Vec::new(),
            materials: Vec::new(),
            world: World::default(),
            render: RenderSettings::default(),
            camera: None,
            next_id: 1,
            animation: crate::animation::Animation::default(),
            images: Vec::new(),
            physics: crate::physics::PhysicsWorld::default(),
            physics_cache: crate::physics::PhysicsCache::default(),
            allocator: IdAllocator::default(),
            node_groups: Vec::new(),
            sequence: crate::sequencer::Sequence::default(),
        }
    }

    /// The scene a new file starts with: a 2 m cube at the origin, a point light, and a camera
    /// looking at the cube.
    pub fn starter() -> Self {
        let mut scene = Self::empty();
        let cube = crate::primitives::Primitive::Cube { size: 2.0 }.build().unwrap_or_default();
        let material = scene.add_material("Material");
        let cube = scene.add_object("Cube", ObjectData::Mesh(cube));
        if let Ok(object) = scene.object_mut(cube) {
            object.material = Some(material);
        }
        let light = scene.add_object("Light", ObjectData::Light(Light::new(LightKind::Point)));
        if let Ok(object) = scene.object_mut(light) {
            object.location = Vec3::new(4.076_25, 1.005_45, 5.903_86);
            object.rotation = Vec3::new(0.650_33, 0.055_22, 1.866_39);
        }
        let camera = scene.add_object("Camera", ObjectData::Camera(Camera::default()));
        if let Ok(object) = scene.object_mut(camera) {
            object.location = Vec3::new(7.358_89, -6.925_79, 4.958_31);
            object.rotation = Vec3::new(1.109_32, 0.0, 0.814_93);
        }
        scene.camera = Some(camera);
        scene
    }

    /// The node group with `id`.
    pub fn node_group(&self, id: Id) -> Option<&crate::node_tree::NodeGroup> {
        self.node_groups.iter().find(|group| group.id == id)
    }

    /// The node group with `id`, to change.
    pub fn node_group_mut(&mut self, id: Id) -> Option<&mut crate::node_tree::NodeGroup> {
        self.node_groups.iter_mut().find(|group| group.id == id)
    }

    /// Takes the next id.
    pub fn take_id(&mut self) -> Id {
        if let IdAllocator(Some((next, end))) = self.allocator
            && next < end
        {
            self.allocator = IdAllocator(Some((next.saturating_add(1), end)));
            self.next_id = self.next_id.max(next.saturating_add(1));
            return next;
        }
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    /// Adds an object with a name not yet used ("Cube", then "Cube.001", ...).
    pub fn add_object(&mut self, name: &str, data: ObjectData) -> Id {
        let id = self.take_id();
        let name = self.unique_object_name(name);
        self.objects.push(Object::new(id, name, data));
        id
    }

    /// Adds a material with a name not yet used.
    /// Deletes a material: objects that use it are left without one, and animation of it, drivers
    /// reading it and images only it used go with it.
    pub fn remove_material(&mut self, id: Id) -> Result<()> {
        self.material(id)?;
        self.materials.retain(|material| material.id != id);
        for object in &mut self.objects {
            if object.material == Some(id) {
                object.material = None;
            }
        }
        self.prune_animation();
        self.prune_images();
        Ok(())
    }

    pub fn add_material(&mut self, name: &str) -> Id {
        let id = self.take_id();
        let name = unique_name(name, |candidate| self.materials.iter().any(|material| material.name == candidate));
        self.materials.push(Material::new(id, name));
        id
    }

    /// `name`, or `name.001`, `name.002`, ... when an object already has it.
    pub fn unique_object_name(&self, name: &str) -> String {
        unique_name(name, |candidate| self.objects.iter().any(|object| object.name == candidate))
    }

    /// The object with `id`.
    pub fn object(&self, id: Id) -> Result<&Object> {
        self.objects.iter().find(|object| object.id == id).ok_or_else(|| SceneError::not_found(format!("object {id}")))
    }

    /// The object with `id`, to change.
    pub fn object_mut(&mut self, id: Id) -> Result<&mut Object> {
        self.objects.iter_mut().find(|object| object.id == id).ok_or_else(|| SceneError::not_found(format!("object {id}")))
    }

    /// The object named `name`.
    pub fn object_named(&self, name: &str) -> Option<&Object> {
        self.objects.iter().find(|object| object.name == name)
    }

    /// The material with `id`.
    pub fn material(&self, id: Id) -> Result<&Material> {
        self.materials.iter().find(|material| material.id == id).ok_or_else(|| SceneError::not_found(format!("material {id}")))
    }

    /// The material with `id`, to change.
    pub fn material_mut(&mut self, id: Id) -> Result<&mut Material> {
        self.materials.iter_mut().find(|material| material.id == id).ok_or_else(|| SceneError::not_found(format!("material {id}")))
    }

    /// The object's matrix in world space (its parents' transforms applied).
    pub fn world_matrix(&self, id: Id) -> Result<Mat4> {
        crate::constraints::world_matrix(self, id, 0)
    }

    /// Whether `ancestor` is `id` or one of its parents.
    pub fn is_ancestor(&self, ancestor: Id, id: Id) -> bool {
        let mut current = Some(id);
        let mut depth = 0;
        while let Some(at) = current {
            if at == ancestor {
                return true;
            }
            current = self.object(at).ok().and_then(|object| object.parent);
            depth += 1;
            if depth > self.objects.len() {
                return false;
            }
        }
        false
    }

    /// The frame an object's own fields are in: its parent's world matrix times its parent
    /// inverse, or the identity without a parent. Never its own constraints or a baked pose.
    pub fn parent_frame(&self, id: Id) -> Result<Mat4> {
        let object = self.object(id)?;
        match object.parent {
            Some(parent) => Ok(self.world_matrix(parent)? * object.parent_inverse),
            None => Ok(Mat4::IDENTITY),
        }
    }

    /// Sets (or clears) an object's parent, keeping where its fields and its parents put it in the
    /// world (Keep Transform): an old parent's part moves into the fields, the new parent's is
    /// undone by the parent inverse. Its own constraints are not baked (they still act on top).
    pub fn set_parent(&mut self, child: Id, parent: Option<Id>) -> Result<()> {
        let had_parent = self.object(child)?.parent.is_some();
        let placed = self.parent_frame(child)? * self.object(child)?.local_matrix();
        let inverse = match parent {
            Some(parent) => {
                self.object(parent)?;
                if self.is_ancestor(child, parent) {
                    return Err(SceneError::params("an object cannot be parented to itself or its own child"));
                }
                self.world_matrix(parent)?.inverse()
            }
            None => Mat4::IDENTITY,
        };
        let object = self.object_mut(child)?;
        object.parent = parent;
        object.parent_inverse = inverse;
        // Without an old parent the fields already are the place: they stay as typed.
        if had_parent {
            object.set_local_matrix(placed);
        }
        Ok(())
    }

    /// Removes objects; their children move to the removed object's parent, staying in place.
    pub fn remove_objects(&mut self, ids: &[Id]) -> Result<()> {
        for id in ids {
            self.object(*id)?;
        }
        for id in ids {
            let children: Vec<Id> = self.objects.iter().filter(|object| object.parent == Some(*id)).map(|object| object.id).collect();
            let grandparent = self.object(*id)?.parent.filter(|parent| !ids.contains(parent));
            for child in children {
                if !ids.contains(&child) {
                    self.set_parent(child, grandparent)?;
                }
            }
        }
        self.objects.retain(|object| !ids.contains(&object.id));
        for object in &mut self.objects {
            let bones = match &mut object.data {
                ObjectData::Armature(armature) => armature.bones.iter_mut().flat_map(|bone| bone.constraints.iter_mut()).collect(),
                _ => Vec::new(),
            };
            for constraint in object.constraints.iter_mut().chain(bones) {
                constraint.kind.forget(ids);
            }
            // Modifiers, a curve's bevel and taper objects and the curve a text follows let go
            // of a deleted object, as when their field is cleared.
            for modifier in &mut object.modifiers {
                modifier.kind.forget(ids);
            }
            let gone = |id: &mut Option<Id>| {
                if id.is_some_and(|id| ids.contains(&id)) {
                    *id = None;
                }
            };
            match &mut object.data {
                ObjectData::Curve(curve) => {
                    gone(&mut curve.bevel_object);
                    gone(&mut curve.taper_object);
                }
                ObjectData::Text(text) => gone(&mut text.follow_curve),
                _ => {}
            }
        }
        // A node's Object setting (Object Info) lets go of a deleted object, as the repair after
        // a merged draft lets go of one the draft named.
        for group in &mut self.node_groups {
            for node in &mut group.nodes {
                if node.values.get("Object").and_then(serde_json::Value::as_u64).is_some_and(|target| ids.iter().any(|id| u64::from(*id) == target)) {
                    node.values.remove("Object");
                }
            }
        }
        self.prune_animation();
        crate::particles::forget(self, ids);
        self.forget_physics(ids);
        for object in &mut self.objects {
            if let ObjectData::Camera(camera) = &mut object.data
                && camera.dof.focus_object.is_some_and(|focus| ids.contains(&focus))
            {
                camera.dof.focus_object = None;
            }
            // Bones drawn by a removed object get their octahedra back.
            if let ObjectData::Armature(armature) = &mut object.data {
                for bone in &mut armature.bones {
                    if bone.shape.as_ref().is_some_and(|shape| ids.contains(&shape.object)) {
                        bone.shape = None;
                    }
                }
            }
        }
        if self.camera.is_some_and(|camera| ids.contains(&camera)) {
            self.camera = self.objects.iter().find(|object| matches!(object.data, ObjectData::Camera(_))).map(|object| object.id);
        }
        let objects: Vec<Id> = self.objects.iter().map(|object| object.id).collect();
        self.sequence.forget(&|id| objects.contains(&id), &|_| true);
        Ok(())
    }

    /// The world-space box of visible mesh objects (after modifiers), lattices' points, and the
    /// origins of other visible objects.
    pub fn bounds(&self, only: Option<&[Id]>) -> Bounds {
        let mut bounds = Bounds::EMPTY;
        for object in &self.objects {
            if object.hidden || only.is_some_and(|ids| !ids.contains(&object.id)) {
                continue;
            }
            let Ok(world) = self.world_matrix(object.id) else { continue };
            match (object.evaluated_mesh(self), &object.data) {
                (Some(mesh), _) if !mesh.positions.is_empty() => bounds = bounds.union(mesh.bounds(world)),
                (_, ObjectData::Lattice(lattice)) => {
                    bounds = lattice.points.iter().fold(bounds, |bounds, point| bounds.with(world.transform_point3(*point)));
                }
                _ => bounds = bounds.with(world.transform_point3(Vec3::ZERO)),
            }
        }
        bounds
    }

    /// Checks ids, references, meshes, and the hierarchy of a scene read from a file.
    /// Adds an image (or finds the same bytes already here) and answers its hash.
    pub fn add_image(&mut self, image: Image) -> String {
        let hash = image.hash.clone();
        if !self.images.iter().any(|other| other.hash == hash) {
            self.images.push(image);
        }
        hash
    }

    /// The image with `hash`.
    pub fn image(&self, hash: &str) -> Option<&Image> {
        self.images.iter().find(|image| image.hash == hash)
    }

    /// Drops images nothing uses (no material, the world, a Displace modifier, a compositing
    /// Image node, a shader's Image Texture, or an image strip).
    pub fn prune_images(&mut self) {
        let displaced = self.objects.iter().flat_map(|object| &object.modifiers).filter_map(|modifier| match &modifier.kind {
            crate::modifiers::ModifierKind::Displace { image, .. } => image.clone(),
            _ => None,
        });
        let composited = self
            .node_groups
            .iter()
            .flat_map(|group| &group.nodes)
            .filter(|node| matches!(node.kind.as_str(), "image" | "imageTexture"))
            .filter_map(|node| node.values.get("Image").and_then(serde_json::Value::as_str).map(str::to_owned));
        let used: std::collections::BTreeSet<String> = self
            .materials
            .iter()
            .flat_map(|material| material.images().cloned())
            .chain(self.world.image.clone())
            .chain(displaced)
            .chain(composited)
            .chain(self.sequence.images().cloned())
            .collect();
        self.images.retain(|image| used.contains(&image.hash));
    }

    pub fn validate(&self) -> Result<()> {
        self.physics.check().map_err(|error| SceneError::InvalidFile(format!("the physics world: {error}")))?;
        for object in &self.objects {
            if let Some(physics) = &object.physics {
                physics.check().map_err(|error| SceneError::InvalidFile(format!("{}'s physics: {error}", object.name)))?;
            }
        }
        self.check_physics_references()?;
        crate::particles::validate(self)?;
        for material in &self.materials {
            if !material.normal_strength.is_finite() {
                return Err(SceneError::InvalidFile(format!("material {} has a normal strength that is not a number", material.name)));
            }
            for hash in material.images() {
                if self.image(hash).is_none() {
                    return Err(SceneError::InvalidFile(format!("material {} names a missing image", material.name)));
                }
            }
            if let Some(group) = material.shader
                && self.node_group(group).is_none_or(|group| group.tree != crate::node_tree::Tree::Shader)
            {
                return Err(SceneError::InvalidFile(format!("material {}'s shader group is missing or not a shader group", material.name)));
            }
            if !(material.transmission.is_finite() && material.ior.is_finite() && material.alpha_cutoff.is_finite() && material.ior > 0.0) {
                return Err(SceneError::InvalidFile(format!(
                    "material {} has a transmission, IOR, or alpha cutoff that is not a number",
                    material.name
                )));
            }
        }
        if let Some(hash) = &self.world.image
            && self.image(hash).is_none()
        {
            return Err(SceneError::InvalidFile("the world names a missing image".into()));
        }
        if let Some(group) = self.world.volume
            && self.node_group(group).is_none_or(|group| group.tree != crate::node_tree::Tree::Shader)
        {
            return Err(SceneError::InvalidFile("the world's volume group is missing or not a shader group".into()));
        }
        let blur = &self.render.motion_blur;
        if !(blur.shutter.is_finite() && blur.shutter >= 0.0) {
            return Err(SceneError::InvalidFile("the motion blur shutter is out of range".into()));
        }
        let occlusion = &self.render.ambient_occlusion;
        if occlusion.distance.is_some_and(|distance| !(distance.is_finite() && distance > 0.0 && distance <= AmbientOcclusion::MAX_DISTANCE))
            || !(occlusion.strength.is_finite() && (0.0..=AmbientOcclusion::MAX_STRENGTH).contains(&occlusion.strength))
        {
            return Err(SceneError::InvalidFile("the ambient occlusion settings are out of range".into()));
        }
        if !self.render.color_management.exposure.is_finite()
            || self.render.color_management.exposure.abs() > crate::color::ColorManagement::MAX_EXPOSURE
        {
            return Err(SceneError::InvalidFile("the render exposure is out of range".into()));
        }
        if !(self.world.rotation.is_finite() && self.world.strength.is_finite()) {
            return Err(SceneError::InvalidFile("the world's rotation or strength is not a number".into()));
        }
        let mut hashes = std::collections::BTreeSet::new();
        for image in &self.images {
            if !hashes.insert(&image.hash) {
                return Err(SceneError::InvalidFile("an image is listed twice".into()));
            }
        }
        let mut ids = std::collections::BTreeSet::new();
        let group_ids = self.node_groups.iter().flat_map(|group| std::iter::once(group.id).chain(group.nodes.iter().map(|node| node.id)));
        let strip_ids = self.sequence.ids();
        for id in
            self.objects.iter().map(|object| object.id).chain(self.materials.iter().map(|material| material.id)).chain(group_ids).chain(strip_ids)
        {
            if id == 0 || id >= self.next_id || !ids.insert(id) {
                return Err(SceneError::InvalidFile(format!("id {id} is repeated or out of range")));
            }
        }
        for object in &self.objects {
            if let Some(parent) = object.parent {
                self.object(parent).map_err(|_| SceneError::InvalidFile(format!("{} has a missing parent", object.name)))?;
            }
            if let Some(material) = object.material {
                self.material(material).map_err(|_| SceneError::InvalidFile(format!("{} has a missing material", object.name)))?;
            }
            if !(object.location.is_finite() && object.rotation.is_finite() && object.scale.is_finite()) {
                return Err(SceneError::InvalidFile(format!("{} has a transform that is not finite", object.name)));
            }
            match &object.data {
                ObjectData::Mesh(mesh) => mesh.validate()?,
                ObjectData::Armature(armature) => {
                    armature.validate()?;
                    for shape in armature.bones.iter().filter_map(|bone| bone.shape.as_ref()) {
                        if shape.object == object.id || self.object(shape.object).is_err() {
                            return Err(SceneError::InvalidFile(format!("{} draws a bone by a missing object or itself", object.name)));
                        }
                    }
                }
                ObjectData::Curve(curve) => curve.validate()?,
                ObjectData::Text(text) => text.validate()?,
                ObjectData::Surface(surface) => surface.validate()?,
                ObjectData::Lattice(lattice) => {
                    lattice.check().map_err(|error| SceneError::InvalidFile(format!("{}: {error}", object.name)))?;
                }
                ObjectData::Camera(camera) => {
                    if !camera.sensor_height.is_finite() || camera.sensor_height <= 0.0 {
                        return Err(SceneError::InvalidFile(format!("{}'s sensor height must be finite and positive", object.name)));
                    }
                    let dof = &camera.dof;
                    if !(dof.f_stop.is_finite()
                        && dof.f_stop > 0.0
                        && dof.focus_distance.is_finite()
                        && dof.rotation.is_finite()
                        && dof.ratio.is_finite()
                        && dof.ratio > 0.0
                        && dof.blades <= MAX_BLADES)
                    {
                        return Err(SceneError::InvalidFile(format!("{}'s depth of field is out of range", object.name)));
                    }
                    if let Some(focus) = camera.dof.focus_object {
                        self.object(focus).map_err(|_| SceneError::InvalidFile(format!("{} focuses on a missing object", object.name)))?;
                    }
                }
                _ => {}
            }
            crate::constraints::check_stack(&object.constraints, false)?;
            self.world_matrix(object.id)?;
        }
        for group in &self.node_groups {
            group.validate(&self.node_groups)?;
            if crate::node_tree::uses_group(&self.node_groups, group.id, group.id) {
                return Err(SceneError::InvalidFile(format!("node group {} uses itself", group.name)));
            }
        }
        for object in &self.objects {
            let references = object.modifiers.iter().filter_map(|modifier| modifier.kind.object()).chain(object.shape_sources());
            for id in references {
                self.object(id).map_err(|_| SceneError::InvalidFile(format!("{} reads a missing object", object.name)))?;
            }
            for modifier in &object.modifiers {
                if let crate::modifiers::ModifierKind::GeometryNodes { group: Some(group), .. } = modifier.kind
                    && self.node_group(group).is_none()
                {
                    return Err(SceneError::InvalidFile(format!("{}'s {} runs a missing node group", object.name, modifier.name)));
                }
                if let crate::modifiers::ModifierKind::GeometryNodes { group: Some(group), .. } = modifier.kind
                    && self.node_group(group).is_some_and(|group| group.tree != crate::node_tree::Tree::Geometry)
                {
                    return Err(SceneError::InvalidFile(format!("{}'s {} runs a compositing node group", object.name, modifier.name)));
                }
            }
        }
        crate::constraints::check_cycles(self).map_err(|error| SceneError::InvalidFile(error.to_string().replace("invalid-params: ", "")))?;
        if let Some(camera) = self.camera
            && !matches!(self.object(camera).map(|object| &object.data), Ok(ObjectData::Camera(_)))
        {
            return Err(SceneError::InvalidFile("the scene camera is not a camera".into()));
        }
        if let Some(group) = self.render.compositor
            && self.node_group(group).is_none_or(|group| group.tree != crate::node_tree::Tree::Compositor)
        {
            return Err(SceneError::InvalidFile("the render's compositing group is missing or not a compositing group".into()));
        }
        self.sequence.validate()?;
        for strip in &self.sequence.strips {
            match &strip.kind {
                crate::sequencer::StripKind::Image { image: Some(hash) } if self.image(hash).is_none() => {
                    return Err(SceneError::InvalidFile(format!("strip {} shows a missing image", strip.name)));
                }
                crate::sequencer::StripKind::Scene { camera: Some(camera) }
                    if !matches!(self.object(*camera).map(|object| &object.data), Ok(ObjectData::Camera(_))) =>
                {
                    return Err(SceneError::InvalidFile(format!("strip {}'s camera is not a camera", strip.name)));
                }
                _ => {}
            }
        }
        if self.render.width == 0 || self.render.height == 0 || self.render.width > MAX_RENDER_SIDE || self.render.height > MAX_RENDER_SIDE {
            return Err(SceneError::InvalidFile("the render size is out of range".into()));
        }
        self.validate_animation()
    }
}

/// `base`, or `base.001`, `base.002`, ... (a trailing `.NNN` of `base` is replaced) when `taken`.
pub fn unique_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    let base = base.trim();
    let base = if base.is_empty() { "Object" } else { base };
    if !taken(base) {
        return base.to_string();
    }
    let stem = match base.rsplit_once('.') {
        Some((stem, suffix)) if suffix.len() == 3 && suffix.bytes().all(|byte| byte.is_ascii_digit()) => stem,
        _ => base,
    };
    (1..u32::MAX).map(|number| format!("{stem}.{number:03}")).find(|candidate| !taken(candidate)).unwrap_or_else(|| stem.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn euler_angles_round_trip() {
        let rotation = Vec3::new(0.3, -0.7, 2.1);
        let back = matrix_euler(euler_matrix(rotation));
        assert!((euler_matrix(back) - euler_matrix(rotation)).abs_diff_eq(Mat3::ZERO, 1e-5));
    }

    #[test]
    fn x_rotation_applies_first() {
        // X by 90° takes +Y to +Z; then Z by 90° leaves +Z alone.
        let matrix = euler_matrix(Vec3::new(std::f32::consts::FRAC_PI_2, 0.0, std::f32::consts::FRAC_PI_2));
        assert!((matrix * Vec3::Y).abs_diff_eq(Vec3::Z, 1e-6));
    }

    #[test]
    fn names_get_numbered() {
        let taken = ["Cube", "Cube.001"];
        assert_eq!(unique_name("Cube", |name| taken.contains(&name)), "Cube.002");
        assert_eq!(unique_name("Cube.001", |name| taken.contains(&name)), "Cube.002");
        assert_eq!(unique_name("Sphere", |name| taken.contains(&name)), "Sphere");
    }

    #[test]
    fn parenting_keeps_world_position() {
        let mut scene = Scene::empty();
        let parent = scene.add_object("Parent", ObjectData::Empty { size: 1.0 });
        let child = scene.add_object("Child", ObjectData::Empty { size: 1.0 });
        scene.object_mut(parent).unwrap().location = Vec3::new(1.0, 2.0, 3.0);
        scene.object_mut(parent).unwrap().rotation = Vec3::new(0.0, 0.0, 1.0);
        scene.object_mut(child).unwrap().location = Vec3::new(5.0, 0.0, 0.0);
        let before = scene.world_matrix(child).unwrap();
        scene.set_parent(child, Some(parent)).unwrap();
        assert!(scene.world_matrix(child).unwrap().abs_diff_eq(before, 1e-5));
        scene.object_mut(parent).unwrap().location += Vec3::X;
        assert!(scene.world_matrix(child).unwrap().w_axis.abs_diff_eq(before.w_axis + glam::Vec4::X, 1e-5));
        scene.set_parent(child, None).unwrap();
        assert!(scene.object(child).unwrap().location.abs_diff_eq(Vec3::new(6.0, 0.0, 0.0), 1e-5));
        assert!(scene.set_parent(parent, Some(parent)).is_err());
    }
}
