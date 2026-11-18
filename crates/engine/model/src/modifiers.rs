//! Modifiers: non-destructive steps applied to a mesh object's mesh, in order. Defaults follow
//! what a modeller expects from Add Modifier (a mirror across X, two array copies one width
//! apart, one subdivision level, ...).

use std::collections::{BTreeSet, HashMap};

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::csg::Operation;
use crate::error::{Result, SceneError};
use crate::mesh::{Mesh, face_edges};
use crate::scene::{Id, Object, Scene};

/// One modifier on an object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Modifier {
    pub name: String,
    /// Shown in the viewport and renders.
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(flatten)]
    pub kind: ModifierKind,
}

fn yes() -> bool {
    true
}

/// What a modifier does, and its settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ModifierKind {
    /// Mirrors the mesh across the object's local axes.
    Mirror {
        #[serde(default = "x_axis")]
        axes: [bool; 3],
        /// Joins vertices on the mirror plane.
        #[serde(default = "yes")]
        merge: bool,
        #[serde(default = "merge_threshold")]
        merge_threshold: f32,
    },
    /// Copies of the mesh, each one offset from the last.
    Array {
        #[serde(default = "two_count")]
        count: u32,
        /// The offset as a fraction of the mesh's size along each axis.
        #[serde(default = "x_offset")]
        relative_offset: Vec3,
        /// An offset in metres, added to the relative one.
        #[serde(default)]
        constant_offset: Vec3,
        #[serde(default)]
        merge: bool,
    },
    /// Catmull–Clark subdivision.
    Subdivision {
        #[serde(default = "one_level")]
        levels: u32,
    },
    /// Gives a surface thickness.
    Solidify {
        #[serde(default = "thickness")]
        thickness: f32,
        /// -1 grows the shell inwards, 1 outwards, 0 both ways.
        #[serde(default = "inwards")]
        offset: f32,
        /// Closes the open edges between the two surfaces.
        #[serde(default = "yes")]
        rim: bool,
    },
    /// Cuts every face into triangles.
    Triangulate,
    /// Joins vertices closer than `distance`.
    Weld {
        #[serde(default = "weld_distance")]
        distance: f32,
    },
    /// Moves each vertex towards the mean of its neighbours.
    Smooth {
        #[serde(default = "half")]
        factor: f32,
        #[serde(default = "one_level")]
        iterations: u32,
    },
    /// Rounds off edges sharper than `angle_limit`.
    Bevel {
        #[serde(default = "bevel_width")]
        width: f32,
        #[serde(default = "one_level")]
        segments: u32,
        /// Edges whose faces meet at more than this angle (radians) are bevelled.
        #[serde(default = "thirty_degrees")]
        angle_limit: f32,
    },
    /// Cuts, joins, or intersects another closed mesh object.
    Boolean {
        #[serde(default)]
        operation: Operation,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        object: Option<Id>,
    },
    /// Deforms the mesh by an armature object's pose, through vertex groups named after its bones.
    Armature {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        object: Option<Id>,
        /// Blends the bones as dual quaternions (the reference application's Preserve Volume).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        preserve_volume: bool,
        /// Vertex groups, envelopes, the modifier's own vertex group, and Multi Modifier.
        #[serde(flatten)]
        deform: crate::armature::ArmatureDeform,
    },
    /// Turns the edges round a local axis (a lathe), rising `screw_offset` per turn.
    Screw {
        #[serde(default = "sixteen")]
        steps: u32,
        /// Radians.
        #[serde(default = "full_turn")]
        angle: f32,
        #[serde(default)]
        screw_offset: f32,
        #[serde(default)]
        axis: ScrewAxis,
    },
    /// Pushes vertices along their normals or an axis by `(value - mid_level) × strength`,
    /// where `value` is an image's brightness when it has one, else procedural noise when
    /// `noise_scale` is above zero, else 1.
    Displace {
        #[serde(default = "one_f")]
        strength: f32,
        #[serde(default = "half")]
        mid_level: f32,
        #[serde(default)]
        direction: crate::modifiers2::DisplaceDirection,
        /// The noise's feature size in metres; 0 displaces every vertex alike.
        #[serde(default)]
        noise_scale: f32,
        /// An image (by hash in `Scene::images`) whose brightness is the value.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        image: Option<String>,
        /// Where the image is looked up.
        #[serde(default)]
        coordinates: crate::modifiers2::TextureCoordinates,
    },
    /// Fewer faces: collapses edges until about `ratio` of the triangles are left, undoes
    /// `iterations` half steps of grid subdivision, or joins faces flatter than `angle_limit`.
    Decimate {
        #[serde(default)]
        mode: DecimateMode,
        #[serde(default = "one_f")]
        ratio: f32,
        #[serde(default)]
        iterations: u32,
        /// Radians.
        #[serde(default = "five_degrees")]
        angle_limit: f32,
    },
    /// Moves every vertex onto another object's surface.
    Shrinkwrap {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        object: Option<Id>,
        #[serde(default)]
        wrap_method: crate::shrinkwrap::WrapMethod,
        /// How far from the surface the vertices stay, metres.
        #[serde(default)]
        offset: f32,
        /// Project: cast along the vertex normal.
        #[serde(default = "yes")]
        positive: bool,
        /// Project: cast against it.
        #[serde(default)]
        negative: bool,
    },
    /// Bends the mesh by a lattice object's moved points.
    Lattice {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        object: Option<Id>,
        #[serde(default = "one_f")]
        strength: f32,
    },
    /// Turns every edge into a strut `thickness` thick.
    Wireframe {
        #[serde(default = "wire_thickness")]
        thickness: f32,
        /// -1 to 1: where the struts sit across the surface.
        #[serde(default)]
        offset: f32,
    },
    /// Twists, bends, tapers, or stretches the mesh along a local axis.
    SimpleDeform {
        #[serde(default)]
        mode: crate::modifiers2::DeformMode,
        #[serde(default)]
        axis: ScrewAxis,
        /// Twist and bend: radians over the whole range.
        #[serde(default = "quarter_turn")]
        angle: f32,
        /// Taper and stretch: the amount over the whole range.
        #[serde(default = "quarter_turn")]
        factor: f32,
        /// The part of the range that deforms, 0 to 1 from the low end.
        #[serde(default = "full_range")]
        limits: [f32; 2],
    },
    /// Runs a node group on the mesh: its Group Input's geometry is the mesh so far, its Group
    /// Output's geometry the result. `inputs` sets the group's other inputs by name.
    GeometryNodes {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        group: Option<Id>,
        #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
        inputs: std::collections::BTreeMap<String, serde_json::Value>,
    },
    /// Multiresolution: the mesh's sculpted subdivision levels ([`crate::multires`]), shown at
    /// `levels` and sculpted at `sculpt_levels` (both at most the mesh's levels).
    Multires {
        #[serde(default)]
        levels: u32,
        #[serde(default)]
        sculpt_levels: u32,
    },
    /// Pulls vertices towards a sphere round the object's origin.
    Cast {
        #[serde(default = "half")]
        factor: f32,
        #[serde(default = "all_axes")]
        axes: [bool; 3],
    },
}

fn one_f() -> f32 {
    1.0
}
fn five_degrees() -> f32 {
    5f32.to_radians()
}

/// How a Decimate modifier takes faces away.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DecimateMode {
    #[default]
    Collapse,
    Unsubdivide,
    Planar,
}
fn wire_thickness() -> f32 {
    0.02
}
fn quarter_turn() -> f32 {
    std::f32::consts::FRAC_PI_4
}
fn full_range() -> [f32; 2] {
    [0.0, 1.0]
}
fn all_axes() -> [bool; 3] {
    [true, true, true]
}

/// The local axis a screw turns round.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScrewAxis {
    X,
    Y,
    #[default]
    Z,
}

impl ScrewAxis {
    fn vector(self) -> Vec3 {
        match self {
            Self::X => Vec3::X,
            Self::Y => Vec3::Y,
            Self::Z => Vec3::Z,
        }
    }
}

fn x_axis() -> [bool; 3] {
    [true, false, false]
}
fn merge_threshold() -> f32 {
    0.001
}
fn two_count() -> u32 {
    2
}
fn x_offset() -> Vec3 {
    Vec3::X
}
fn one_level() -> u32 {
    1
}
fn thickness() -> f32 {
    0.01
}
fn inwards() -> f32 {
    -1.0
}
fn weld_distance() -> f32 {
    0.001
}
fn half() -> f32 {
    0.5
}
fn bevel_width() -> f32 {
    0.1
}
fn thirty_degrees() -> f32 {
    30f32.to_radians()
}
fn sixteen() -> u32 {
    16
}
fn full_turn() -> f32 {
    std::f32::consts::TAU
}

/// The kinds `modifier.add` accepts, with their display names.
pub const KINDS: &[(&str, &str)] = &[
    ("mirror", "Mirror"),
    ("array", "Array"),
    ("subdivision", "Subdivision"),
    ("solidify", "Solidify"),
    ("triangulate", "Triangulate"),
    ("weld", "Weld"),
    ("smooth", "Smooth"),
    ("bevel", "Bevel"),
    ("boolean", "Boolean"),
    ("screw", "Screw"),
    ("displace", "Displace"),
    ("decimate", "Decimate"),
    ("wireframe", "Wireframe"),
    ("simpleDeform", "SimpleDeform"),
    ("cast", "Cast"),
    ("multires", "Multires"),
    ("shrinkwrap", "Shrinkwrap"),
    ("lattice", "Lattice"),
    ("armature", "Armature"),
    ("geometryNodes", "GeometryNodes"),
];

/// What a Geometry Nodes modifier asks the node evaluator for.
pub struct NodesCall<'a> {
    /// The mesh so far (the group's input geometry).
    pub mesh: &'a Mesh,
    pub object: &'a Object,
    pub scene: &'a Scene,
    pub group: Id,
    pub inputs: &'a std::collections::BTreeMap<String, serde_json::Value>,
    /// How deep evaluation reached through other objects (an Object Info of an object whose own
    /// nodes read this one stops here).
    pub depth: u32,
}

/// Evaluates a node group: the `kition-scene-nodes` crate's evaluator, installed once by
/// whoever runs documents ([`install_nodes_evaluator`]). Without one, Geometry Nodes modifiers
/// leave the mesh as it is.
pub type NodesEvaluator = fn(&NodesCall<'_>) -> std::result::Result<Mesh, String>;

static NODES_EVALUATOR: std::sync::OnceLock<NodesEvaluator> = std::sync::OnceLock::new();

/// Installs the node evaluator (the first one installed stays).
pub fn install_nodes_evaluator(evaluate: NodesEvaluator) {
    let _ = NODES_EVALUATOR.set(evaluate);
}

/// Why a Geometry Nodes modifier on `object` cannot evaluate, if it cannot (for diagnostics):
/// the first failing modifier's error.
pub fn nodes_error(object: &Object, scene: &Scene) -> Option<String> {
    let evaluate = NODES_EVALUATOR.get()?;
    let mut mesh = object.base_mesh(scene)?;
    crate::vertex_data::apply_shape_keys(&mut mesh);
    let mut stack = Stack::default();
    for modifier in object.modifiers.iter().filter(|modifier| modifier.enabled) {
        if let ModifierKind::GeometryNodes { group: Some(group), inputs } = &modifier.kind {
            match evaluate(&NodesCall { mesh: &mesh, object, scene, group: *group, inputs, depth: 0 }) {
                Ok(out) => mesh = out,
                Err(error) => return Some(format!("{}: {error}", modifier.name)),
            }
            stack = Stack::default();
        } else {
            mesh = stack.apply(modifier, mesh, object, scene, 0);
        }
    }
    None
}

/// How deep booleans may reach through other booleans (a cycle stops here).
const MAX_DEPTH: u32 = 4;

/// The largest number of faces a modifier may produce.
const MAX_FACES: usize = 2_000_000;

impl ModifierKind {
    /// The kind's default settings.
    pub fn default_of(kind: &str) -> Result<Self> {
        let value = serde_json::json!({ "type": kind });
        serde_json::from_value(value).map_err(|_| SceneError::params(format!("unknown modifier type `{kind}`")))
    }

    /// The display name, as a new modifier of this kind is called.
    pub fn display_name(&self) -> &'static str {
        let tag = match self {
            Self::Mirror { .. } => "mirror",
            Self::Array { .. } => "array",
            Self::Subdivision { .. } => "subdivision",
            Self::Solidify { .. } => "solidify",
            Self::Triangulate => "triangulate",
            Self::Weld { .. } => "weld",
            Self::Smooth { .. } => "smooth",
            Self::Bevel { .. } => "bevel",
            Self::Boolean { .. } => "boolean",
            Self::Armature { .. } => "armature",
            Self::Screw { .. } => "screw",
            Self::Displace { .. } => "displace",
            Self::Decimate { .. } => "decimate",
            Self::Wireframe { .. } => "wireframe",
            Self::SimpleDeform { .. } => "simpleDeform",
            Self::Cast { .. } => "cast",
            Self::Multires { .. } => "multires",
            Self::Shrinkwrap { .. } => "shrinkwrap",
            Self::Lattice { .. } => "lattice",
            Self::GeometryNodes { .. } => "geometryNodes",
        };
        KINDS.iter().find(|(id, _)| *id == tag).map_or("Modifier", |(_, name)| name)
    }

    /// Checks the settings' ranges.
    pub fn check(&self) -> Result<()> {
        let finite = |name: &str, value: f32| if value.is_finite() { Ok(()) } else { Err(SceneError::params(format!("{name} must be a number"))) };
        match self {
            Self::Mirror { merge_threshold, .. } => finite("mergeThreshold", *merge_threshold),
            Self::Array { count, relative_offset, constant_offset, .. } => {
                if !(1..=1000).contains(count) {
                    return Err(SceneError::params("count must be between 1 and 1000"));
                }
                if !(relative_offset.is_finite() && constant_offset.is_finite()) {
                    return Err(SceneError::params("the offsets must be numbers"));
                }
                Ok(())
            }
            Self::Subdivision { levels } => {
                if *levels > 6 {
                    return Err(SceneError::params("levels must be between 0 and 6"));
                }
                Ok(())
            }
            Self::Solidify { thickness, offset, .. } => {
                finite("thickness", *thickness)?;
                finite("offset", *offset)
            }
            Self::Triangulate => Ok(()),
            Self::Weld { distance } => finite("distance", *distance),
            Self::Smooth { factor, iterations } => {
                finite("factor", *factor)?;
                if *iterations > 100 {
                    return Err(SceneError::params("iterations must be between 0 and 100"));
                }
                Ok(())
            }
            Self::Bevel { width, segments, angle_limit } => {
                if !(width.is_finite() && *width > 0.0) {
                    return Err(SceneError::params("width must be a positive number"));
                }
                if !(1..=64).contains(segments) {
                    return Err(SceneError::params("segments must be between 1 and 64"));
                }
                if !(angle_limit.is_finite() && (0.0..=std::f32::consts::PI).contains(angle_limit)) {
                    return Err(SceneError::params("angleLimit must be between 0 and pi"));
                }
                Ok(())
            }
            Self::Boolean { .. } | Self::Armature { .. } => Ok(()),
            Self::Screw { steps, angle, screw_offset, .. } => {
                if !(1..=1024).contains(steps) {
                    return Err(SceneError::params("steps must be between 1 and 1024"));
                }
                finite("angle", *angle)?;
                if angle.abs() > 100.0 * std::f32::consts::TAU {
                    return Err(SceneError::params("angle is too large"));
                }
                finite("screwOffset", *screw_offset)
            }
            Self::Displace { strength, mid_level, noise_scale, image, .. } => {
                finite("strength", *strength)?;
                finite("midLevel", *mid_level)?;
                if !(noise_scale.is_finite() && *noise_scale >= 0.0) {
                    return Err(SceneError::params("noiseScale must be zero or more"));
                }
                if image.as_ref().is_some_and(|hash| hash.len() > 128) {
                    return Err(SceneError::params("image must be an image's hash"));
                }
                Ok(())
            }
            Self::Decimate { ratio, iterations, angle_limit, .. } => {
                if !(ratio.is_finite() && (0.0..=1.0).contains(ratio)) {
                    return Err(SceneError::params("ratio must be between 0 and 1"));
                }
                if *iterations > 16 {
                    return Err(SceneError::params("iterations must be between 0 and 16"));
                }
                if !(angle_limit.is_finite() && (0.0..=std::f32::consts::PI).contains(angle_limit)) {
                    return Err(SceneError::params("angleLimit must be between 0 and pi"));
                }
                Ok(())
            }
            Self::Shrinkwrap { offset, .. } => finite("offset", *offset),
            Self::Lattice { strength, .. } => {
                if !(strength.is_finite() && (-10.0..=10.0).contains(strength)) {
                    return Err(SceneError::params("strength must be between -10 and 10"));
                }
                Ok(())
            }
            Self::Wireframe { thickness, offset } => {
                finite("thickness", *thickness)?;
                if !(offset.is_finite() && (-1.0..=1.0).contains(offset)) {
                    return Err(SceneError::params("offset must be between -1 and 1"));
                }
                Ok(())
            }
            Self::SimpleDeform { angle, factor, limits, .. } => {
                finite("angle", *angle)?;
                finite("factor", *factor)?;
                if angle.abs() > 100.0 * std::f32::consts::TAU || factor.abs() > 1000.0 {
                    return Err(SceneError::params("the deformation is too large"));
                }
                let [low, high] = *limits;
                if !(low.is_finite() && high.is_finite() && (0.0..=1.0).contains(&low) && (0.0..=1.0).contains(&high) && low <= high) {
                    return Err(SceneError::params("limits must be two numbers from 0 to 1, low then high"));
                }
                Ok(())
            }
            Self::Cast { factor, .. } => finite("factor", *factor),
            Self::Multires { levels, sculpt_levels } => {
                if *levels > crate::multires::MAX_LEVELS || *sculpt_levels > crate::multires::MAX_LEVELS {
                    return Err(SceneError::params("levels must be between 0 and 6"));
                }
                Ok(())
            }
            Self::GeometryNodes { inputs, .. } => {
                if inputs.len() > 256 {
                    return Err(SceneError::params("a geometry nodes modifier takes at most 256 inputs"));
                }
                Ok(())
            }
        }
    }
}

impl ModifierKind {
    /// The object the modifier reads (a boolean's or shrinkwrap's mesh, the lattice, the
    /// armature), when it names one.
    pub fn object(&self) -> Option<Id> {
        match self {
            Self::Boolean { object, .. } | Self::Armature { object, .. } | Self::Shrinkwrap { object, .. } | Self::Lattice { object, .. } => *object,
            _ => None,
        }
    }

    /// Lets go of an object among `ids` (deleted): the field is empty, as after clearing it.
    pub fn forget(&mut self, ids: &[Id]) {
        if let Self::Boolean { object, .. } | Self::Armature { object, .. } | Self::Shrinkwrap { object, .. } | Self::Lattice { object, .. } = self
            && object.is_some_and(|id| ids.contains(&id))
        {
            *object = None;
        }
    }
}

impl Modifier {
    /// A modifier of `kind` with its defaults.
    pub fn new(kind: ModifierKind) -> Self {
        Self { name: kind.display_name().to_string(), enabled: true, kind }
    }

    /// The mesh after this modifier. A result that would be too large, or that the modifier
    /// cannot make (an open mesh in a boolean), is the input unchanged.
    pub fn apply(&self, mesh: &Mesh, object: &Object, scene: &Scene) -> Mesh {
        self.apply_at(mesh, object, scene, 0)
    }

    /// [`Modifier::apply`] `depth` booleans deep.
    pub fn apply_at(&self, mesh: &Mesh, object: &Object, scene: &Scene, depth: u32) -> Mesh {
        let out = match &self.kind {
            ModifierKind::Mirror { axes, merge, merge_threshold } => mirror(mesh, *axes, *merge, *merge_threshold),
            ModifierKind::Array { count, relative_offset, constant_offset, merge } => array(mesh, *count, *relative_offset, *constant_offset, *merge),
            ModifierKind::Subdivision { levels } => {
                let mut out = mesh.clone();
                for _ in 0..*levels {
                    if out.faces.len().saturating_mul(4) > MAX_FACES {
                        break;
                    }
                    out = catmull_clark(&out);
                }
                out
            }
            ModifierKind::Solidify { thickness, offset, rim } => solidify(mesh, *thickness, *offset, *rim),
            ModifierKind::Triangulate => triangulate(mesh),
            ModifierKind::Weld { distance } => {
                let mut out = mesh.clone();
                out.weld(distance.abs());
                out
            }
            ModifierKind::Smooth { factor, iterations } => smooth(mesh, *factor, *iterations),
            ModifierKind::Bevel { width, segments, angle_limit } => bevel(mesh, *width, *segments, *angle_limit),
            ModifierKind::Armature { object: rig, preserve_volume, deform } => {
                rig.and_then(|rig| crate::armature::deform_with(mesh, object, scene, rig, *preserve_volume, deform)).unwrap_or_else(|| mesh.clone())
            }
            ModifierKind::Boolean { operation, object: operand } => {
                boolean(mesh, *operation, *operand, object, scene, depth).unwrap_or_else(|| mesh.clone())
            }
            ModifierKind::Screw { steps, angle, screw_offset, axis } => screw(mesh, *steps, *angle, *screw_offset, *axis),
            ModifierKind::Displace { strength, mid_level, direction, noise_scale, image, coordinates } => {
                let values = image.as_ref().and_then(|hash| {
                    let to_world = scene.world_matrix(object.id).unwrap_or(glam::Mat4::IDENTITY);
                    crate::modifiers2::image_values(mesh, scene.image(hash)?, *coordinates, to_world)
                });
                // An image that cannot be read displaces nothing.
                let values = match (image, values) {
                    (Some(_), None) => Some(vec![*mid_level; mesh.positions.len()]),
                    (_, values) => values,
                };
                crate::modifiers2::displace(mesh, *strength, *mid_level, *direction, *noise_scale, values.as_deref())
            }
            ModifierKind::Decimate { mode, ratio, iterations, angle_limit } => match mode {
                DecimateMode::Collapse => crate::modifiers2::decimate(mesh, *ratio),
                DecimateMode::Unsubdivide => crate::dissolve::unsubdivide(mesh, *iterations),
                DecimateMode::Planar => crate::dissolve::planar(mesh, *angle_limit),
            },
            ModifierKind::Shrinkwrap { object: target, wrap_method, offset, positive, negative } => target
                .and_then(|target| {
                    let wrap = crate::shrinkwrap::Wrap { method: *wrap_method, offset: *offset, positive: *positive, negative: *negative };
                    crate::shrinkwrap::apply(mesh, object, scene, target, wrap, depth)
                })
                .unwrap_or_else(|| mesh.clone()),
            ModifierKind::Lattice { object: lattice, strength } => {
                lattice.and_then(|lattice| crate::lattice::deform(mesh, object, scene, lattice, *strength)).unwrap_or_else(|| mesh.clone())
            }
            ModifierKind::Wireframe { thickness, offset } => crate::modifiers2::wireframe(mesh, *thickness, *offset),
            ModifierKind::SimpleDeform { mode, axis, angle, factor, limits } => {
                crate::modifiers2::simple_deform(mesh, *mode, *axis, *angle, *factor, *limits)
            }
            ModifierKind::Cast { factor, axes } => crate::modifiers2::cast(mesh, *factor, *axes),
            ModifierKind::Multires { levels, .. } => crate::multires::evaluate(mesh, *levels),
            ModifierKind::GeometryNodes { group, inputs } => match (group, NODES_EVALUATOR.get()) {
                (Some(group), Some(evaluate)) => {
                    evaluate(&NodesCall { mesh, object, scene, group: *group, inputs, depth }).unwrap_or_else(|_| mesh.clone())
                }
                _ => mesh.clone(),
            },
        };
        let mut out = if out.faces.len() > MAX_FACES { mesh.clone() } else { out };
        // Modifiers that make faces of their own (boolean, screw) get coordinates from the faces
        // they came from; a result without them never keeps a broken layer.
        if mesh.has_uvs() && mesh.uvs_aligned() && !(out.has_uvs() && out.uvs_aligned()) {
            crate::uv::carry(Some(mesh), &mut out);
        } else if !out.uvs_aligned() {
            out.uvs.clear();
        }
        // Vertex groups go along by position, so an Armature modifier further down still finds them.
        if !mesh.groups.is_empty() && crate::vertex_data::aligned(mesh) && !crate::vertex_data::aligned(&out) {
            crate::vertex_data::carry(crate::vertex_data::snapshot(mesh).as_ref(), &mut out);
        } else if !crate::vertex_data::aligned(&out) {
            out.groups.clear();
            out.shape_keys.clear();
        }
        out
    }
}

/// Runs a modifier stack in order. What the last Armature modifier got is kept, so a Multi
/// Modifier right below it deforms the same mesh and mixes its result over the one above.
#[derive(Debug, Default)]
pub struct Stack {
    armature_input: Option<Mesh>,
}

impl Stack {
    /// The mesh after `modifier`, the next enabled one of `object`'s stack.
    pub fn apply(&mut self, modifier: &Modifier, mesh: Mesh, object: &Object, scene: &Scene, depth: u32) -> Mesh {
        let ModifierKind::Armature { object: rig, preserve_volume, deform } = &modifier.kind else {
            self.armature_input = None;
            return modifier.apply_at(&mesh, object, scene, depth);
        };
        let shared = self.armature_input.take().filter(|input| deform.multi_modifier && input.positions.len() == mesh.positions.len());
        let Some(input) = shared else {
            let out = modifier.apply_at(&mesh, object, scene, depth);
            self.armature_input = Some(mesh);
            return out;
        };
        let moved = rig.and_then(|rig| crate::armature::deformed_positions(&input, object, scene, rig, *preserve_volume, deform));
        let mut out = mesh;
        if let Some(moved) = moved {
            let mask = deform.mask(&input);
            for (index, (point, target)) in out.positions.iter_mut().zip(moved).enumerate() {
                let share = mask.as_ref().map_or(1.0, |mask| mask.get(index).copied().unwrap_or(0.0));
                *point = point.lerp(target, share);
            }
        }
        self.armature_input = Some(input);
        out
    }
}

fn bevel(mesh: &Mesh, width: f32, segments: u32, angle_limit: f32) -> Mesh {
    let mut faces_of: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (index, face) in mesh.faces.iter().enumerate() {
        for (a, b) in face_edges(face) {
            faces_of.entry((a.min(b), a.max(b))).or_default().push(index);
        }
    }
    let normals: Vec<Vec3> = mesh.faces.iter().map(|face| mesh.face_normal(face)).collect();
    let mut edges: Vec<(u32, u32)> = faces_of
        .iter()
        .filter_map(|(edge, faces)| {
            let [a, b] = faces.as_slice() else { return None };
            let (na, nb) = (normals.get(*a)?, normals.get(*b)?);
            (na.dot(*nb).clamp(-1.0, 1.0).acos() > angle_limit + 1e-4).then_some(*edge)
        })
        .collect();
    edges.sort_unstable();
    if edges.is_empty() {
        return mesh.clone();
    }
    let mut out = mesh.clone();
    match crate::bevel::bevel_edges(&mut out, &edges, width, segments) {
        Ok(_) => out,
        Err(_) => mesh.clone(),
    }
}

fn boolean(mesh: &Mesh, operation: Operation, operand: Option<Id>, object: &Object, scene: &Scene, depth: u32) -> Option<Mesh> {
    let operand_id = operand?;
    if operand_id == object.id || depth >= MAX_DEPTH {
        return None;
    }
    let other = scene.object(operand_id).ok()?;
    let mut other_mesh = other.evaluated_mesh_at(scene, depth + 1)?;
    let relative = scene.world_matrix(object.id).ok()?.inverse() * scene.world_matrix(operand_id).ok()?;
    if !relative.is_finite() {
        return None;
    }
    other_mesh.transform(relative);
    let closed = |candidate: &Mesh| !candidate.faces.is_empty() && boundary_edges(candidate).is_empty();
    if !closed(mesh) || !closed(&other_mesh) {
        return None;
    }
    crate::csg::boolean(mesh, &other_mesh, operation)
}

fn screw(mesh: &Mesh, steps: u32, angle: f32, offset: f32, axis: ScrewAxis) -> Mesh {
    let edges = mesh.edges();
    if edges.is_empty() {
        return mesh.clone();
    }
    let direction = axis.vector();
    let full = (angle.abs() - std::f32::consts::TAU).abs() < 1e-4 && offset == 0.0;
    let copies = if full { steps } else { steps + 1 };
    let count = mesh.vertex_count();
    let mut out = Mesh { smooth: mesh.smooth, smooth_angle: mesh.smooth_angle, ..Default::default() };
    for copy in 0..copies {
        let t = copy as f32 / steps as f32;
        let turn = glam::Mat3::from_axis_angle(direction, angle * t);
        out.positions.extend(mesh.positions.iter().map(|point| turn * *point + direction * (offset * t)));
    }
    for step in 0..steps {
        let (now, next) = (step * count, ((step + 1) % copies) * count);
        for [a, b] in &edges {
            out.faces.push(vec![now + a, now + b, next + b, next + a]);
        }
    }
    // Vertices on the axis stay one vertex, so caps close in a point instead of a sliver.
    let mut remap: Vec<u32> = (0..out.vertex_count()).collect();
    for copy in 1..copies {
        for vertex in 0..count {
            let on_axis = mesh.positions.get(vertex as usize).is_some_and(|point| (*point - direction * point.dot(direction)).length() < 1e-6);
            if on_axis
                && offset == 0.0
                && let Some(slot) = remap.get_mut((copy * count + vertex) as usize)
            {
                *slot = vertex;
            }
        }
    }
    out.reindex(&remap);
    out.remove_unused_vertices();
    if !out.faces.is_empty() && boundary_edges(&out).is_empty() {
        crate::ops::recalculate_outside(&mut out);
    }
    if out.validate().is_err() {
        return mesh.clone();
    }
    out
}

fn mirror(mesh: &Mesh, axes: [bool; 3], merge: bool, threshold: f32) -> Mesh {
    let mut out = mesh.clone();
    for (axis, on) in axes.iter().enumerate() {
        if !on {
            continue;
        }
        let mut flip = Vec3::ONE;
        if let Some(component) = flip.as_mut().get_mut(axis) {
            *component = -1.0;
        }
        let mut copy = out.clone();
        for point in &mut copy.positions {
            *point *= flip;
        }
        copy.flip_all();
        let original = out.vertex_count();
        let offset = out.append(&copy);
        if merge {
            // A vertex on the plane joins its own mirror image.
            let mut remap: Vec<u32> = (0..out.vertex_count()).collect();
            for index in 0..original {
                let on_plane = out
                    .positions
                    .get(index as usize)
                    .and_then(|point| point.as_ref().get(axis).copied())
                    .is_some_and(|value| value.abs() <= threshold);
                if on_plane && let Some(slot) = remap.get_mut((index + offset) as usize) {
                    *slot = index;
                }
            }
            out.reindex(&remap);
            out.remove_unused_vertices();
        }
    }
    out
}

fn array(mesh: &Mesh, count: u32, relative: Vec3, constant: Vec3, merge: bool) -> Mesh {
    let size = mesh.bounds(glam::Mat4::IDENTITY).size();
    let step = relative * size + constant;
    let mut out = Mesh { smooth: mesh.smooth, smooth_angle: mesh.smooth_angle, ..Default::default() };
    for copy in 0..count {
        let mut part = mesh.clone();
        for point in &mut part.positions {
            *point += step * copy as f32;
        }
        out.append(&part);
        if out.faces.len() > MAX_FACES {
            break;
        }
    }
    if merge {
        out.weld(0.01);
    }
    out
}

fn triangulate(mesh: &Mesh) -> Mesh {
    let carry = mesh.has_uvs() && mesh.uvs_aligned();
    let mut faces = Vec::new();
    let mut uvs = Vec::new();
    for (corners, face) in mesh.corner_triangles() {
        let Some(vertices) = mesh.faces.get(face as usize) else { continue };
        let vertex = |corner: u32| vertices.get(corner as usize).copied().unwrap_or(0);
        faces.push(corners.iter().map(|corner| vertex(*corner)).collect());
        if carry {
            uvs.push(corners.iter().map(|corner| mesh.corner_uv(face as usize, *corner as usize)).collect());
        }
    }
    Mesh {
        positions: mesh.positions.clone(),
        faces,
        loose_edges: mesh.loose_edges.clone(),
        smooth: mesh.smooth,
        smooth_angle: mesh.smooth_angle,
        uvs,
        groups: mesh.groups.clone(),
        shape_keys: mesh.shape_keys.clone(),
        ..Default::default()
    }
}

/// The neighbours of each vertex along edges.
fn neighbours(mesh: &Mesh) -> Vec<Vec<u32>> {
    let mut out = vec![Vec::new(); mesh.positions.len()];
    for [a, b] in mesh.edges() {
        if let Some(list) = out.get_mut(a as usize) {
            list.push(b);
        }
        if let Some(list) = out.get_mut(b as usize) {
            list.push(a);
        }
    }
    out
}

fn smooth(mesh: &Mesh, factor: f32, iterations: u32) -> Mesh {
    let around = neighbours(mesh);
    let mut out = mesh.clone();
    for _ in 0..iterations {
        let next: Vec<Vec3> = out
            .positions
            .iter()
            .enumerate()
            .map(|(index, point)| {
                let list = around.get(index).map_or(&[][..], Vec::as_slice);
                if list.is_empty() {
                    return *point;
                }
                let mean = list.iter().filter_map(|other| out.positions.get(*other as usize)).copied().sum::<Vec3>() / list.len() as f32;
                point.lerp(mean, factor)
            })
            .collect();
        out.positions = next;
    }
    out
}

/// The edges used by exactly one face (the boundary), in their face's direction.
pub fn boundary_edges(mesh: &Mesh) -> Vec<(u32, u32)> {
    let mut count: HashMap<(u32, u32), usize> = HashMap::new();
    for face in &mesh.faces {
        for (a, b) in face_edges(face) {
            *count.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let mut out = Vec::new();
    for face in &mesh.faces {
        for (a, b) in face_edges(face) {
            if count.get(&(a.min(b), a.max(b))) == Some(&1) {
                out.push((a, b));
            }
        }
    }
    out
}

fn solidify(mesh: &Mesh, thickness: f32, offset: f32, rim: bool) -> Mesh {
    let normals = mesh.vertex_normals();
    // The outer surface moves by (offset + 1) / 2 of the thickness along the normal, the inner by
    // (offset - 1) / 2.
    let outer = thickness * (offset + 1.0) / 2.0;
    let inner = thickness * (offset - 1.0) / 2.0;
    let count = mesh.vertex_count();
    let mut out = Mesh { smooth: mesh.smooth, smooth_angle: mesh.smooth_angle, ..Default::default() };
    let shift = |distance: f32| -> Vec<Vec3> { mesh.positions.iter().zip(&normals).map(|(point, normal)| *point + *normal * distance).collect() };
    out.positions = shift(outer);
    out.positions.extend(shift(inner));
    out.faces = mesh.faces.clone();
    out.faces.extend(mesh.faces.iter().map(|face| face.iter().rev().map(|index| index + count).collect()));
    let carry = mesh.has_uvs() && mesh.uvs_aligned();
    if carry {
        // The inner surface wears the outer one's coordinates; the rim takes its edge's.
        out.uvs = mesh.uvs.clone();
        out.uvs.extend(mesh.uvs.iter().map(|uvs| uvs.iter().rev().copied().collect()));
    }
    if rim {
        let mut corner_of: HashMap<(u32, u32), (glam::Vec2, glam::Vec2)> = HashMap::new();
        if carry {
            for (face_index, face) in mesh.faces.iter().enumerate() {
                for (corner, (a, b)) in face_edges(face).enumerate() {
                    corner_of.insert((a, b), (mesh.corner_uv(face_index, corner), mesh.corner_uv(face_index, (corner + 1) % face.len())));
                }
            }
        }
        for (a, b) in boundary_edges(mesh) {
            out.faces.push(vec![b, a, a + count, b + count]);
            if carry {
                let (ua, ub) = corner_of.get(&(a, b)).copied().unwrap_or_default();
                out.uvs.push(vec![ub, ua, ua, ub]);
            }
        }
    }
    out.loose_edges = mesh.loose_edges.clone();
    out
}

/// Moves each vertex of a quad mesh onto the Catmull–Clark limit surface (where endless
/// subdivision would take it), as the reference's Subdivision Surface node shows it.
pub fn limit_positions(mesh: &mut Mesh) {
    let count = mesh.positions.len();
    let mut edges: HashMap<(u32, u32), usize> = HashMap::new();
    for face in &mesh.faces {
        for (a, b) in face_edges(face) {
            *edges.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let mut neighbours: Vec<BTreeSet<u32>> = vec![BTreeSet::new(); count];
    let mut boundary: Vec<Vec<u32>> = vec![Vec::new(); count];
    for ((a, b), faces) in &edges {
        for (from, to) in [(a, b), (b, a)] {
            if let Some(set) = neighbours.get_mut(*from as usize) {
                set.insert(*to);
            }
            if *faces == 1
                && let Some(list) = boundary.get_mut(*from as usize)
            {
                list.push(*to);
            }
        }
    }
    let mut diagonals: Vec<Vec3> = vec![Vec3::ZERO; count];
    let mut quads = vec![true; count];
    for face in &mesh.faces {
        for (corner, vertex) in face.iter().enumerate() {
            let opposite = face.get((corner + 2) % face.len()).and_then(|other| mesh.positions.get(*other as usize)).copied().unwrap_or(Vec3::ZERO);
            if let Some(sum) = diagonals.get_mut(*vertex as usize) {
                *sum += opposite;
            }
            if face.len() != 4
                && let Some(flag) = quads.get_mut(*vertex as usize)
            {
                *flag = false;
            }
        }
    }
    let at = |vertex: u32| mesh.positions.get(vertex as usize).copied().unwrap_or(Vec3::ZERO);
    let limits: Vec<Vec3> = (0..count)
        .map(|vertex| {
            let point = at(u32::try_from(vertex).unwrap_or(u32::MAX));
            let ring = neighbours.get(vertex).map_or(0, BTreeSet::len);
            match boundary.get(vertex).map_or(&[][..], Vec::as_slice) {
                [] if ring >= 3 && quads.get(vertex).copied().unwrap_or(false) => {
                    let n = ring as f32;
                    let edge_sum: Vec3 = neighbours.get(vertex).into_iter().flatten().map(|other| at(*other)).sum();
                    let diagonal = diagonals.get(vertex).copied().unwrap_or(Vec3::ZERO);
                    (n * n * point + 4.0 * edge_sum + diagonal) / (n * (n + 5.0))
                }
                [a, b] => (at(*a) + 4.0 * point + at(*b)) / 6.0,
                _ => point,
            }
        })
        .collect();
    mesh.positions = limits;
}

/// One level of Catmull–Clark subdivision: every n-gon becomes n quads. Boundary edges and
/// vertices follow the cubic B-spline boundary rule, so open meshes keep their outline.
pub fn catmull_clark(mesh: &Mesh) -> Mesh {
    let vertex = |index: u32| mesh.positions.get(index as usize).copied().unwrap_or(Vec3::ZERO);
    let face_points: Vec<Vec3> = mesh.faces.iter().map(|face| mesh.face_center(face)).collect();

    // Edges with the faces on each side.
    let mut edge_faces: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (face_index, face) in mesh.faces.iter().enumerate() {
        for (a, b) in face_edges(face) {
            edge_faces.entry((a.min(b), a.max(b))).or_default().push(face_index);
        }
    }
    let mut edge_order: Vec<(u32, u32)> = edge_faces.keys().copied().collect();
    edge_order.sort_unstable();

    let mut positions = mesh.positions.clone();
    let mut edge_index: HashMap<(u32, u32), u32> = HashMap::new();
    let mut edge_points: HashMap<(u32, u32), Vec3> = HashMap::new();
    for key in &edge_order {
        let faces = edge_faces.get(key).map_or(&[][..], Vec::as_slice);
        let midpoint = (vertex(key.0) + vertex(key.1)) * 0.5;
        let point = if faces.len() == 2 {
            let mean = faces.iter().filter_map(|face| face_points.get(*face)).copied().sum::<Vec3>() / 2.0;
            (vertex(key.0) + vertex(key.1) + mean * 2.0) / 4.0
        } else {
            midpoint
        };
        edge_points.insert(*key, midpoint);
        edge_index.insert(*key, u32::try_from(positions.len()).unwrap_or(u32::MAX));
        positions.push(point);
    }

    // Original vertices move: interior ones by (F + 2R + (n - 3)P) / n, boundary ones by the
    // boundary rule, and loose ones stay.
    let mut vertex_faces: Vec<Vec<usize>> = vec![Vec::new(); mesh.positions.len()];
    for (face_index, face) in mesh.faces.iter().enumerate() {
        for index in face {
            if let Some(list) = vertex_faces.get_mut(*index as usize) {
                list.push(face_index);
            }
        }
    }
    let mut vertex_edges: Vec<Vec<(u32, u32)>> = vec![Vec::new(); mesh.positions.len()];
    for key in &edge_order {
        for end in [key.0, key.1] {
            if let Some(list) = vertex_edges.get_mut(end as usize) {
                list.push(*key);
            }
        }
    }
    for (index, slot) in positions.iter_mut().take(mesh.positions.len()).enumerate() {
        let point = *slot;
        let faces = vertex_faces.get(index).map_or(&[][..], Vec::as_slice);
        let edges = vertex_edges.get(index).map_or(&[][..], Vec::as_slice);
        if faces.is_empty() {
            continue;
        }
        let boundary: Vec<&(u32, u32)> = edges.iter().filter(|key| edge_faces.get(key).is_some_and(|list| list.len() == 1)).collect();
        if boundary.is_empty() {
            let n = faces.len() as f32;
            let f = faces.iter().filter_map(|face| face_points.get(*face)).copied().sum::<Vec3>() / n;
            let r = edges.iter().filter_map(|key| edge_points.get(key)).copied().sum::<Vec3>() / edges.len().max(1) as f32;
            *slot = (f + r * 2.0 + point * (n - 3.0)) / n;
        } else if boundary.len() == 2 {
            let sum = boundary.iter().filter_map(|key| edge_points.get(*key)).copied().sum::<Vec3>();
            *slot = point * 0.5 + sum * 0.25;
        }
        // Corners where more than two boundary edges meet stay put.
    }

    let mut out = Mesh { positions, smooth: mesh.smooth, smooth_angle: mesh.smooth_angle, ..Default::default() };
    // Texture coordinates subdivide linearly within each face (corners keep theirs, edge points
    // take their edge's middle, the centre the face's mean), so seams stay exactly where they were.
    let carry = mesh.has_uvs() && mesh.uvs_aligned();
    for (face_index, face) in mesh.faces.iter().enumerate() {
        let centre = out.vertex_count();
        out.positions.push(face_points.get(face_index).copied().unwrap_or(Vec3::ZERO));
        let count = face.len();
        let uv = |corner: usize| mesh.corner_uv(face_index, corner % count.max(1));
        let centre_uv = (0..count).map(uv).sum::<glam::Vec2>() / count.max(1) as f32;
        for corner in 0..count {
            let (Some(&previous), Some(&current), Some(&next)) =
                (face.get((corner + count - 1) % count), face.get(corner), face.get((corner + 1) % count))
            else {
                continue;
            };
            let edge_before = edge_index.get(&(previous.min(current), previous.max(current))).copied().unwrap_or(0);
            let edge_after = edge_index.get(&(current.min(next), current.max(next))).copied().unwrap_or(0);
            out.faces.push(vec![current, edge_after, centre, edge_before]);
            if carry {
                let here = uv(corner);
                out.uvs.push(vec![here, (here + uv(corner + 1)) * 0.5, centre_uv, (here + uv(corner + count - 1)) * 0.5]);
            }
        }
    }
    for [a, b] in &mesh.loose_edges {
        out.loose_edges.push([*a, *b]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::Primitive;

    fn cube() -> Mesh {
        Primitive::Cube { size: 2.0 }.build().unwrap()
    }

    #[test]
    fn subdividing_a_cube_makes_twenty_four_quads() {
        let once = catmull_clark(&cube());
        assert_eq!(once.positions.len(), 26);
        assert_eq!(once.faces.len(), 24);
        // A closed mesh shrinks inwards; the corners move to ±5/9.
        let corner = once.positions[7];
        assert!((corner - Vec3::splat(5.0 / 9.0)).length() < 1e-5, "{corner}");
    }

    #[test]
    fn mirror_merges_the_seam() {
        let mut half = Primitive::Plane { size: 2.0 }.build().unwrap();
        for point in &mut half.positions {
            point.x = (point.x + 1.0) / 2.0; // x from 0 to 1
        }
        let out = mirror(&half, [true, false, false], true, 0.001);
        assert_eq!(out.positions.len(), 6);
        assert_eq!(out.faces.len(), 2);
        let mesh_normal = out.face_normal(&out.faces[1]);
        assert!(mesh_normal.z > 0.99, "the mirrored face still faces up: {mesh_normal}");
    }

    #[test]
    fn solidify_closes_a_plane() {
        let plane = Primitive::Plane { size: 2.0 }.build().unwrap();
        let out = solidify(&plane, 0.1, -1.0, true);
        assert_eq!(out.positions.len(), 8);
        assert_eq!(out.faces.len(), 6);
        assert!(boundary_edges(&out).is_empty());
    }

    #[test]
    fn array_places_copies_one_width_apart() {
        let out = array(&cube(), 3, Vec3::X, Vec3::ZERO, false);
        assert_eq!(out.faces.len(), 18);
        let bounds = out.bounds(glam::Mat4::IDENTITY);
        assert!((bounds.max.x - 5.0).abs() < 1e-5);
    }

    #[test]
    fn defaults_parse_by_kind() {
        assert_eq!(ModifierKind::default_of("subdivision").unwrap(), ModifierKind::Subdivision { levels: 1 });
        assert!(ModifierKind::default_of("bogus").is_err());
    }
}
