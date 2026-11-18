//! The Scene kit's document model: a Z-up scene of mesh, light, camera, and empty objects with
//! materials and modifiers, and the mesh operations Edit Mode runs.
#![forbid(unsafe_code)]

pub mod abf;
pub mod animation;
pub mod armature;
pub mod bevel;
pub mod color;
pub mod compositor_kinds;
pub mod constraint_math;
pub mod constraints;
pub mod csg;
pub mod curve;
pub mod dissolve;
pub mod drivers;
pub mod dual_quat;
pub mod error;
pub mod expression;
pub mod fmodifier;
pub mod hdr;
pub mod heat;
pub mod ik;
pub mod knife;
pub mod lattice;
pub mod mesh;
mod minimum_stretch;
pub mod modifiers;
pub mod modifiers2;
pub mod multires;
pub mod nla;
mod nla_groups;
pub mod node_tree;
pub mod ops;
pub mod ops2;
pub mod outline;
pub mod particles;
pub mod physics;
mod physics_weights;
pub mod pixels;
pub mod primitives;
pub mod proportional;
pub mod scene;
pub mod sculpt;
pub mod seams;
pub mod sequencer;
pub mod shader_kinds;
pub mod shrinkwrap;
pub mod stitch;
pub mod surface;
pub mod tangents;
pub mod text;
pub mod unwrap;
pub mod uv;
pub mod vertex_data;
pub mod weight_paint;

pub use animation::{Animation, Channel, Interpolation, Keyframe, Property, Timeline};
pub use armature::{Armature, ArmatureDeform, Bone, BonePose, BoneShape};
pub use constraints::{Constraint, ConstraintKind, Target};
pub use curve::{BezierPoint, ControlPoint, Curve, CurvePrimitive, HandleType, Spline};
pub use error::{Result, SceneError};
pub use glam;
pub use lattice::{Lattice, LatticeInterpolation};
pub use mesh::{Bounds, Mesh};
pub use modifiers::{Modifier, ModifierKind};
pub use node_tree::{GroupSocket, Link, Node, NodeGroup, SocketType};
pub use primitives::{Fill, Primitive};
pub use scene::{
    AlphaMode, AmbientOcclusion, Camera, DepthOfField, Id, IdAllocator, Image, ImageFormat, Light, LightKind, Material, MotionBlur, Object,
    ObjectData, RenderSettings, Scene, ShutterPosition, TextureSlot, World,
};
pub use surface::{Surface, SurfacePrimitive};
pub use text::{Text, TextAlign};
pub use vertex_data::{ShapeKey, VertexGroup};
