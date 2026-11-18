//! Node groups as data: the groups a scene holds (geometry node groups, which Geometry Nodes
//! modifiers run, compositing node groups, which a render passes through, and shader groups,
//! which give a material its surface), their nodes and links, and the catalogue of node kinds
//! (sockets, defaults, and settings, recorded from the reference application's nodes of the same
//! names). Evaluating a group is the `kition-scene-nodes`, `kition-scene-compositor`, and
//! `kition-scene-shader` crates' job; the model only stores groups and checks them.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Result, SceneError};
use crate::scene::Id;

/// A node group: nodes, the links between their sockets, the inputs a Geometry Nodes modifier
/// using it exposes as its settings, and the outputs a Group node using it inside another group
/// gives beside the geometry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeGroup {
    pub id: Id,
    pub name: String,
    /// What the group works on: geometry, or a render's picture.
    #[serde(default, skip_serializing_if = "Tree::is_geometry")]
    pub tree: Tree,
    /// The group's inputs beside the geometry, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<GroupSocket>,
    /// The group's outputs beside the geometry, in order (a Group node using the group gives
    /// them; a modifier takes only the geometry).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<GroupSocket>,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub links: Vec<Link>,
}

/// What a node group works on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Tree {
    /// Geometry, for Geometry Nodes modifiers.
    #[default]
    Geometry,
    /// A render's picture, for compositing.
    Compositor,
    /// A surface, for a material.
    Shader,
}

impl Tree {
    pub fn is_geometry(&self) -> bool {
        *self == Self::Geometry
    }

    /// The tree's name as files and the agent write it.
    pub fn id(self) -> &'static str {
        match self {
            Self::Geometry => "geometry",
            Self::Compositor => "compositor",
            Self::Shader => "shader",
        }
    }

    /// The tree a name gives, if any.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "geometry" => Some(Self::Geometry),
            "compositor" => Some(Self::Compositor),
            "shader" => Some(Self::Shader),
            _ => None,
        }
    }
}

/// One input or output of a group's interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupSocket {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: SocketType,
    /// The value a modifier (or a Group node) uses when it sets none; outputs have none.
    #[serde(default)]
    pub default: Value,
}

/// One node: its kind, where it sits in the editor, and the values of its unlinked inputs and
/// its settings, by socket or setting id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub id: Id,
    pub kind: String,
    #[serde(default)]
    pub location: [f32; 2],
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, Value>,
}

/// A link from one node's output socket to another's input socket.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub from: Id,
    pub from_socket: String,
    pub to: Id,
    pub to_socket: String,
}

/// What a socket carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SocketType {
    Geometry,
    Float,
    Int,
    Bool,
    Vector,
    /// Euler angles in radians (XYZ), as the reference's rotation sockets take them.
    Rotation,
    Object,
    Material,
    String,
    /// A colour with alpha (linear, premultiplied); in a compositing group, a picture.
    Color,
    /// A compositing node's curves (RGB Curves): data set on the node, never linked.
    Curves,
    /// A Color Ramp's stops: data set on the node, never linked.
    Ramp,
    /// A surface (a shader's closure), in a shader group.
    Shader,
    /// Follows a setting: `Socket::follows` names it (a Switch's or a Random Value's type).
    Any,
}

/// A socket's value when nothing links into it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SocketDefault {
    None,
    Float(f32),
    Int(i32),
    Bool(bool),
    Vector([f32; 3]),
    Color([f32; 4]),
    Str(&'static str),
}

impl SocketDefault {
    /// The default as JSON.
    pub fn json(self) -> Value {
        match self {
            Self::None => Value::Null,
            Self::Float(value) => serde_json::json!(value),
            Self::Int(value) => serde_json::json!(value),
            Self::Bool(value) => Value::Bool(value),
            Self::Vector(value) => serde_json::json!(value),
            Self::Color(value) => serde_json::json!(value),
            Self::Str(value) => Value::String(value.to_owned()),
        }
    }
}

/// A field an unlinked input reads when it has no value of its own (the reference's hidden
/// "implicit" inputs: Set Position's position, Random Value's ID).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Implicit {
    Position,
    Normal,
    Index,
    /// A shader's surface point in generated coordinates (0 to 1 across the object's bounds).
    Generated,
    /// A shader's surface point's texture coordinates.
    Uv,
}

/// One socket of a node kind.
#[derive(Debug, Clone, Copy)]
pub struct Socket {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: SocketType,
    pub default: SocketDefault,
    pub implicit: Option<Implicit>,
    /// Takes several links (Join Geometry's geometry).
    pub multi: bool,
    /// Shown only when this setting has one of these values.
    pub when: Option<(&'static str, &'static [&'static str])>,
    /// For `SocketType::Any`: the setting whose value gives the type.
    pub follows: Option<&'static str>,
}

/// One setting of a node kind: a choice from `choices` (a boolean setting's choices are
/// `false` and `true`).
#[derive(Debug, Clone, Copy)]
pub struct Setting {
    pub id: &'static str,
    pub choices: &'static [&'static str],
    pub default: &'static str,
}

/// A node kind.
#[derive(Debug, Clone, Copy)]
pub struct NodeKind {
    pub id: &'static str,
    pub title: &'static str,
    pub category: &'static str,
    pub inputs: &'static [Socket],
    pub outputs: &'static [Socket],
    pub settings: &'static [Setting],
}

pub(crate) const fn s(id: &'static str, kind: SocketType, default: SocketDefault) -> Socket {
    Socket { id, name: id, kind, default, implicit: None, multi: false, when: None, follows: None }
}

pub(crate) const fn geo(id: &'static str) -> Socket {
    s(id, SocketType::Geometry, SocketDefault::None)
}

pub(crate) const fn float(id: &'static str, value: f32) -> Socket {
    s(id, SocketType::Float, SocketDefault::Float(value))
}

pub(crate) const fn int(id: &'static str, value: i32) -> Socket {
    s(id, SocketType::Int, SocketDefault::Int(value))
}

pub(crate) const fn boolean(id: &'static str, value: bool) -> Socket {
    s(id, SocketType::Bool, SocketDefault::Bool(value))
}

pub(crate) const fn vector(id: &'static str, value: [f32; 3]) -> Socket {
    s(id, SocketType::Vector, SocketDefault::Vector(value))
}

pub(crate) const fn out(id: &'static str, kind: SocketType) -> Socket {
    s(id, kind, SocketDefault::None)
}

pub(crate) const fn named(mut socket: Socket, name: &'static str) -> Socket {
    socket.name = name;
    socket
}

pub(crate) const fn implicit(mut socket: Socket, field: Implicit) -> Socket {
    socket.implicit = Some(field);
    socket
}

pub(crate) const fn when(mut socket: Socket, setting: &'static str, values: &'static [&'static str]) -> Socket {
    socket.when = Some((setting, values));
    socket
}

const fn any(id: &'static str, follows: &'static str) -> Socket {
    let mut socket = s(id, SocketType::Any, SocketDefault::None);
    socket.follows = Some(follows);
    socket
}

const fn rotation(id: &'static str) -> Socket {
    s(id, SocketType::Rotation, SocketDefault::Vector([0.0, 0.0, 0.0]))
}

const fn with_default(mut socket: Socket, default: SocketDefault) -> Socket {
    socket.default = default;
    socket
}

const fn multi(mut socket: Socket) -> Socket {
    socket.multi = true;
    socket
}

const SELECTION: Socket = boolean("Selection", true);
const FILL: &[&str] = &["none", "ngon", "triangle_fan"];

/// The math operations of the Math node.
pub const MATH: &[&str] = &[
    "add",
    "subtract",
    "multiply",
    "divide",
    "multiply_add",
    "power",
    "logarithm",
    "sqrt",
    "inverse_sqrt",
    "absolute",
    "exponent",
    "minimum",
    "maximum",
    "less_than",
    "greater_than",
    "sign",
    "compare",
    "smooth_min",
    "smooth_max",
    "round",
    "floor",
    "ceil",
    "trunc",
    "fract",
    "modulo",
    "floored_modulo",
    "wrap",
    "snap",
    "pingpong",
    "sine",
    "cosine",
    "tangent",
    "arcsine",
    "arccosine",
    "arctangent",
    "arctan2",
    "sinh",
    "cosh",
    "tanh",
    "radians",
    "degrees",
];
/// Math operations that take one value.
pub const MATH_UNARY: &[&str] = &[
    "sqrt",
    "inverse_sqrt",
    "absolute",
    "exponent",
    "sign",
    "round",
    "floor",
    "ceil",
    "trunc",
    "fract",
    "sine",
    "cosine",
    "tangent",
    "arcsine",
    "arccosine",
    "arctangent",
    "sinh",
    "cosh",
    "tanh",
    "radians",
    "degrees",
];
/// Math operations that take three values.
pub const MATH_TERNARY: &[&str] = &["multiply_add", "compare", "smooth_min", "smooth_max", "wrap"];

/// The vector math operations of the Vector Math node.
pub const VECTOR_MATH: &[&str] = &[
    "add",
    "subtract",
    "multiply",
    "divide",
    "multiply_add",
    "cross_product",
    "project",
    "reflect",
    "refract",
    "faceforward",
    "dot_product",
    "distance",
    "length",
    "scale",
    "normalize",
    "absolute",
    "power",
    "sign",
    "minimum",
    "maximum",
    "round",
    "floor",
    "ceil",
    "fraction",
    "modulo",
    "wrap",
    "snap",
    "sine",
    "cosine",
    "tangent",
];
/// Vector math operations whose result is a number (the Value output).
pub const VECTOR_MATH_VALUE: &[&str] = &["dot_product", "distance", "length"];
const VECTOR_THREE: &[&str] = &["multiply_add", "faceforward", "wrap"];
const VECTOR_TWO_PLUS: &[&str] = &[
    "add",
    "subtract",
    "multiply",
    "divide",
    "multiply_add",
    "cross_product",
    "project",
    "reflect",
    "refract",
    "faceforward",
    "dot_product",
    "distance",
    "power",
    "minimum",
    "maximum",
    "modulo",
    "wrap",
    "snap",
];

/// The comparisons of the Compare node.
pub const COMPARE: &[&str] = &["less_than", "less_equal", "greater_than", "greater_equal", "equal", "not_equal"];
const SWITCH_TYPES: &[&str] = &["float", "int", "boolean", "vector", "rotation", "geometry", "object", "material"];
const FIELD_TYPES: &[&str] = &["float", "int", "boolean", "float_vector"];
const DOMAINS: &[&str] = &["point", "edge", "face", "corner", "curve", "instance"];
const MESH_DOMAINS: &[&str] = &["point", "edge", "face", "corner"];
const NUMBER_TYPES: &[&str] = &["float", "float_vector"];
const ACCUMULATE_TYPES: &[&str] = &["float", "int", "float_vector"];
const CURVE_MODES: &[&str] = &["evaluated", "count", "length"];

/// Every node kind, in the add menu's order.
pub static NODE_KINDS: &[NodeKind] = &[
    NodeKind {
        id: "groupInput",
        title: "Group Input",
        category: "Group",
        inputs: &[],
        outputs: &[out("Geometry", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind { id: "groupOutput", title: "Group Output", category: "Group", inputs: &[geo("Geometry")], outputs: &[], settings: &[] },
    // A group used as a node: its sockets are the named group's interface (`group` holds the
    // group's id).
    NodeKind { id: "group", title: "Group", category: "Group", inputs: &[], outputs: &[], settings: &[] },
    // Zones: an input and an output node (the input's `output` holds the output's id) with the
    // items the output's `items` lists as sockets on both.
    NodeKind {
        id: "repeatInput",
        title: "Repeat Input",
        category: "Utilities",
        inputs: &[int("Iterations", 1)],
        outputs: &[out("Iteration", SocketType::Int)],
        settings: &[],
    },
    NodeKind { id: "repeatOutput", title: "Repeat Output", category: "Utilities", inputs: &[], outputs: &[], settings: &[] },
    NodeKind {
        id: "simulationInput",
        title: "Simulation Input",
        category: "Simulation",
        inputs: &[],
        outputs: &[out("Delta Time", SocketType::Float)],
        settings: &[],
    },
    NodeKind {
        id: "simulationOutput",
        title: "Simulation Output",
        category: "Simulation",
        inputs: &[boolean("Skip", false)],
        outputs: &[],
        settings: &[],
    },
    NodeKind {
        id: "meshCube",
        title: "Cube",
        category: "Mesh Primitives",
        inputs: &[vector("Size", [1.0, 1.0, 1.0]), int("Vertices X", 2), int("Vertices Y", 2), int("Vertices Z", 2)],
        outputs: &[out("Mesh", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "meshGrid",
        title: "Grid",
        category: "Mesh Primitives",
        inputs: &[float("Size X", 1.0), float("Size Y", 1.0), int("Vertices X", 3), int("Vertices Y", 3)],
        outputs: &[out("Mesh", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "meshUvSphere",
        title: "UV Sphere",
        category: "Mesh Primitives",
        inputs: &[int("Segments", 32), int("Rings", 16), float("Radius", 1.0)],
        outputs: &[out("Mesh", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "meshIcoSphere",
        title: "Ico Sphere",
        category: "Mesh Primitives",
        inputs: &[float("Radius", 1.0), int("Subdivisions", 1)],
        outputs: &[out("Mesh", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "meshCylinder",
        title: "Cylinder",
        category: "Mesh Primitives",
        inputs: &[int("Vertices", 32), int("Side Segments", 1), int("Fill Segments", 1), float("Radius", 1.0), float("Depth", 2.0)],
        outputs: &[out("Mesh", SocketType::Geometry), out("Top", SocketType::Bool), out("Side", SocketType::Bool), out("Bottom", SocketType::Bool)],
        settings: &[Setting { id: "fillType", choices: FILL, default: "ngon" }],
    },
    NodeKind {
        id: "meshCone",
        title: "Cone",
        category: "Mesh Primitives",
        inputs: &[
            int("Vertices", 32),
            int("Side Segments", 1),
            int("Fill Segments", 1),
            float("Radius Top", 0.0),
            float("Radius Bottom", 1.0),
            float("Depth", 2.0),
        ],
        outputs: &[out("Mesh", SocketType::Geometry), out("Top", SocketType::Bool), out("Bottom", SocketType::Bool), out("Side", SocketType::Bool)],
        settings: &[Setting { id: "fillType", choices: FILL, default: "ngon" }],
    },
    NodeKind {
        id: "meshLine",
        title: "Mesh Line",
        category: "Mesh Primitives",
        inputs: &[int("Count", 10), vector("Start Location", [0.0, 0.0, 0.0]), vector("Offset", [0.0, 0.0, 1.0])],
        outputs: &[out("Mesh", SocketType::Geometry)],
        settings: &[Setting { id: "mode", choices: &["offset", "end_points"], default: "offset" }],
    },
    NodeKind {
        id: "meshCircle",
        title: "Mesh Circle",
        category: "Mesh Primitives",
        inputs: &[int("Vertices", 32), float("Radius", 1.0)],
        outputs: &[out("Mesh", SocketType::Geometry)],
        settings: &[Setting { id: "fillType", choices: FILL, default: "none" }],
    },
    NodeKind {
        id: "curveLine",
        title: "Curve Line",
        category: "Curve Primitives",
        inputs: &[
            vector("Start", [0.0, 0.0, 0.0]),
            when(vector("End", [0.0, 0.0, 1.0]), "mode", &["points"]),
            when(vector("Direction", [0.0, 0.0, 1.0]), "mode", &["direction"]),
            when(float("Length", 1.0), "mode", &["direction"]),
        ],
        outputs: &[out("Curve", SocketType::Geometry)],
        settings: &[Setting { id: "mode", choices: &["points", "direction"], default: "points" }],
    },
    NodeKind {
        id: "curveCircle",
        title: "Curve Circle",
        category: "Curve Primitives",
        inputs: &[
            int("Resolution", 32),
            when(vector("Point 1", [-1.0, 0.0, 0.0]), "mode", &["points"]),
            when(vector("Point 2", [0.0, 1.0, 0.0]), "mode", &["points"]),
            when(vector("Point 3", [1.0, 0.0, 0.0]), "mode", &["points"]),
            when(float("Radius", 1.0), "mode", &["radius"]),
        ],
        outputs: &[out("Curve", SocketType::Geometry), when(out("Center", SocketType::Vector), "mode", &["points"])],
        settings: &[Setting { id: "mode", choices: &["points", "radius"], default: "radius" }],
    },
    NodeKind {
        id: "curveBezierSegment",
        title: "Bézier Segment",
        category: "Curve Primitives",
        inputs: &[
            int("Resolution", 16),
            vector("Start", [-1.0, 0.0, 0.0]),
            vector("Start Handle", [-0.5, 0.5, 0.0]),
            vector("End Handle", [0.0, 0.0, 0.0]),
            vector("End", [1.0, 0.0, 0.0]),
        ],
        outputs: &[out("Curve", SocketType::Geometry)],
        settings: &[Setting { id: "mode", choices: &["position", "offset"], default: "position" }],
    },
    NodeKind {
        id: "curveQuadrilateral",
        title: "Quadrilateral",
        category: "Curve Primitives",
        inputs: &[
            when(float("Width", 2.0), "mode", &["rectangle", "parallelogram", "kite"]),
            when(float("Height", 2.0), "mode", &["rectangle", "parallelogram", "trapezoid"]),
            when(float("Bottom Width", 4.0), "mode", &["trapezoid"]),
            when(float("Top Width", 2.0), "mode", &["trapezoid"]),
            when(float("Offset", 1.0), "mode", &["parallelogram", "trapezoid"]),
            when(float("Bottom Height", 3.0), "mode", &["kite"]),
            when(float("Top Height", 1.0), "mode", &["kite"]),
            when(vector("Point 1", [-1.0, -1.0, 0.0]), "mode", &["points"]),
            when(vector("Point 2", [1.0, -1.0, 0.0]), "mode", &["points"]),
            when(vector("Point 3", [1.0, 1.0, 0.0]), "mode", &["points"]),
            when(vector("Point 4", [-1.0, 1.0, 0.0]), "mode", &["points"]),
        ],
        outputs: &[out("Curve", SocketType::Geometry)],
        settings: &[Setting { id: "mode", choices: &["rectangle", "parallelogram", "trapezoid", "kite", "points"], default: "rectangle" }],
    },
    NodeKind {
        id: "curveSpiral",
        title: "Spiral",
        category: "Curve Primitives",
        inputs: &[
            int("Resolution", 32),
            float("Rotations", 2.0),
            float("Start Radius", 1.0),
            float("End Radius", 2.0),
            float("Height", 2.0),
            boolean("Reverse", false),
        ],
        outputs: &[out("Curve", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "curveStar",
        title: "Star",
        category: "Curve Primitives",
        inputs: &[int("Points", 8), float("Inner Radius", 1.0), float("Outer Radius", 2.0), float("Twist", 0.0)],
        outputs: &[out("Curve", SocketType::Geometry), out("Outer Points", SocketType::Bool)],
        settings: &[],
    },
    NodeKind {
        id: "curveToMesh",
        title: "Curve to Mesh",
        category: "Curve",
        inputs: &[geo("Curve"), geo("Profile Curve"), float("Scale", 1.0), boolean("Fill Caps", false)],
        outputs: &[out("Mesh", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "curveToPoints",
        title: "Curve to Points",
        category: "Curve",
        inputs: &[geo("Curve"), when(int("Count", 10), "mode", &["count"]), when(float("Length", 0.1), "mode", &["length"])],
        outputs: &[
            out("Points", SocketType::Geometry),
            out("Tangent", SocketType::Vector),
            out("Normal", SocketType::Vector),
            out("Rotation", SocketType::Rotation),
        ],
        settings: &[Setting { id: "mode", choices: CURVE_MODES, default: "count" }],
    },
    NodeKind {
        id: "fillCurve",
        title: "Fill Curve",
        category: "Curve",
        inputs: &[geo("Curve"), int("Group ID", 0)],
        outputs: &[out("Mesh", SocketType::Geometry)],
        settings: &[
            Setting { id: "mode", choices: &["triangles", "ngons"], default: "triangles" },
            Setting { id: "fillRule", choices: &["even_odd", "non_zero"], default: "even_odd" },
        ],
    },
    NodeKind {
        id: "resampleCurve",
        title: "Resample Curve",
        category: "Curve",
        inputs: &[geo("Curve"), SELECTION, when(int("Count", 10), "mode", &["count"]), when(float("Length", 0.1), "mode", &["length"])],
        outputs: &[out("Curve", SocketType::Geometry)],
        settings: &[
            Setting { id: "mode", choices: CURVE_MODES, default: "count" },
            Setting { id: "keepLastSegment", choices: &["false", "true"], default: "true" },
        ],
    },
    NodeKind {
        id: "trimCurve",
        title: "Trim Curve",
        category: "Curve",
        inputs: &[
            geo("Curve"),
            SELECTION,
            when(float("Start", 0.0), "mode", &["factor"]),
            when(float("End", 1.0), "mode", &["factor"]),
            when(named(float("Start_001", 0.0), "Start"), "mode", &["length"]),
            when(named(float("End_001", 1.0), "End"), "mode", &["length"]),
        ],
        outputs: &[out("Curve", SocketType::Geometry)],
        settings: &[Setting { id: "mode", choices: &["factor", "length"], default: "factor" }],
    },
    NodeKind {
        id: "reverseCurve",
        title: "Reverse Curve",
        category: "Curve",
        inputs: &[geo("Curve"), SELECTION],
        outputs: &[out("Curve", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "setCurveRadius",
        title: "Set Curve Radius",
        category: "Curve",
        inputs: &[geo("Curve"), SELECTION, float("Radius", 0.005)],
        outputs: &[out("Curve", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "setCurveTilt",
        title: "Set Curve Tilt",
        category: "Curve",
        inputs: &[geo("Curve"), SELECTION, float("Tilt", 0.0)],
        outputs: &[out("Curve", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "setSplineType",
        title: "Set Spline Type",
        category: "Curve",
        inputs: &[geo("Curve"), SELECTION],
        outputs: &[out("Curve", SocketType::Geometry)],
        settings: &[Setting { id: "splineType", choices: &["poly", "catmullRom", "bezier", "nurbs"], default: "poly" }],
    },
    NodeKind {
        id: "setNurbsOrder",
        title: "Set NURBS Order",
        category: "Curve",
        inputs: &[geo("Curves"), SELECTION, int("Order", 4)],
        outputs: &[out("Curves", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "setNurbsWeight",
        title: "Set NURBS Weight",
        category: "Curve",
        inputs: &[geo("Curves"), SELECTION, float("Weight", 1.0)],
        outputs: &[out("Curves", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "setSplineCyclic",
        title: "Set Spline Cyclic",
        category: "Curve",
        inputs: &[named(geo("Geometry"), "Curve"), SELECTION, boolean("Cyclic", false)],
        outputs: &[named(out("Geometry", SocketType::Geometry), "Curve")],
        settings: &[],
    },
    NodeKind {
        id: "setSplineResolution",
        title: "Set Spline Resolution",
        category: "Curve",
        inputs: &[named(geo("Geometry"), "Curve"), SELECTION, int("Resolution", 12)],
        outputs: &[named(out("Geometry", SocketType::Geometry), "Curve")],
        settings: &[],
    },
    NodeKind {
        id: "splineParameter",
        title: "Spline Parameter",
        category: "Curve",
        inputs: &[],
        outputs: &[out("Factor", SocketType::Float), out("Length", SocketType::Float), out("Index", SocketType::Int)],
        settings: &[],
    },
    NodeKind {
        id: "curveLength",
        title: "Curve Length",
        category: "Curve",
        inputs: &[geo("Curve")],
        outputs: &[out("Length", SocketType::Float)],
        settings: &[],
    },
    NodeKind {
        id: "endpointSelection",
        title: "Endpoint Selection",
        category: "Curve",
        inputs: &[int("Start Size", 1), int("End Size", 1)],
        outputs: &[out("Selection", SocketType::Bool)],
        settings: &[],
    },
    NodeKind {
        id: "transform",
        title: "Transform Geometry",
        category: "Geometry",
        inputs: &[
            geo("Geometry"),
            vector("Translation", [0.0, 0.0, 0.0]),
            s("Rotation", SocketType::Rotation, SocketDefault::Vector([0.0, 0.0, 0.0])),
            vector("Scale", [1.0, 1.0, 1.0]),
        ],
        outputs: &[out("Geometry", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "joinGeometry",
        title: "Join Geometry",
        category: "Geometry",
        inputs: &[multi(geo("Geometry"))],
        outputs: &[out("Geometry", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "setPosition",
        title: "Set Position",
        category: "Geometry",
        inputs: &[geo("Geometry"), SELECTION, implicit(vector("Position", [0.0, 0.0, 0.0]), Implicit::Position), vector("Offset", [0.0, 0.0, 0.0])],
        outputs: &[out("Geometry", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "deleteGeometry",
        title: "Delete Geometry",
        category: "Geometry",
        inputs: &[geo("Geometry"), SELECTION],
        outputs: &[out("Geometry", SocketType::Geometry)],
        settings: &[
            Setting { id: "domain", choices: &["point", "edge", "face", "instance"], default: "point" },
            Setting { id: "mode", choices: &["all", "edge_face", "only_face"], default: "all" },
        ],
    },
    NodeKind {
        id: "mergeByDistance",
        title: "Merge by Distance",
        category: "Geometry",
        inputs: &[geo("Geometry"), SELECTION, float("Distance", 0.001)],
        outputs: &[out("Geometry", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "subdivideMesh",
        title: "Subdivide Mesh",
        category: "Mesh",
        inputs: &[named(geo("Mesh"), "Mesh"), int("Level", 1)],
        outputs: &[out("Mesh", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "subdivisionSurface",
        title: "Subdivision Surface",
        category: "Mesh",
        inputs: &[geo("Mesh"), int("Level", 1)],
        outputs: &[out("Mesh", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "extrudeMesh",
        title: "Extrude Mesh",
        category: "Mesh",
        inputs: &[
            geo("Mesh"),
            SELECTION,
            implicit(vector("Offset", [0.0, 0.0, 0.0]), Implicit::Normal),
            float("Offset Scale", 1.0),
            when(boolean("Individual", true), "mode", &["faces"]),
        ],
        outputs: &[out("Mesh", SocketType::Geometry), out("Top", SocketType::Bool), out("Side", SocketType::Bool)],
        settings: &[Setting { id: "mode", choices: &["vertices", "edges", "faces"], default: "faces" }],
    },
    NodeKind {
        id: "meshToPoints",
        title: "Mesh to Points",
        category: "Mesh",
        inputs: &[geo("Mesh"), SELECTION, implicit(vector("Position", [0.0, 0.0, 0.0]), Implicit::Position), float("Radius", 0.05)],
        outputs: &[out("Points", SocketType::Geometry)],
        settings: &[Setting { id: "mode", choices: &["vertices", "edges", "faces", "corners"], default: "vertices" }],
    },
    NodeKind {
        id: "meshToCurve",
        title: "Mesh to Curve",
        category: "Mesh",
        inputs: &[geo("Mesh"), SELECTION],
        outputs: &[out("Curve", SocketType::Geometry)],
        settings: &[Setting { id: "mode", choices: &["edges", "faces"], default: "edges" }],
    },
    NodeKind {
        id: "distributePointsOnFaces",
        title: "Distribute Points on Faces",
        category: "Point",
        inputs: &[geo("Mesh"), SELECTION, float("Density", 10.0), int("Seed", 0)],
        outputs: &[out("Points", SocketType::Geometry), out("Normal", SocketType::Vector), out("Rotation", SocketType::Rotation)],
        settings: &[],
    },
    NodeKind {
        id: "pointsToVertices",
        title: "Points to Vertices",
        category: "Point",
        inputs: &[geo("Points"), SELECTION],
        outputs: &[out("Mesh", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "instanceOnPoints",
        title: "Instance on Points",
        category: "Instances",
        inputs: &[
            geo("Points"),
            SELECTION,
            geo("Instance"),
            s("Rotation", SocketType::Rotation, SocketDefault::Vector([0.0, 0.0, 0.0])),
            vector("Scale", [1.0, 1.0, 1.0]),
        ],
        outputs: &[out("Instances", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "realizeInstances",
        title: "Realize Instances",
        category: "Instances",
        inputs: &[geo("Geometry")],
        outputs: &[out("Geometry", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "setMaterial",
        title: "Set Material",
        category: "Material",
        inputs: &[geo("Geometry"), SELECTION, s("Material", SocketType::Material, SocketDefault::None)],
        outputs: &[out("Geometry", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "storeNamedAttribute",
        title: "Store Named Attribute",
        category: "Attribute",
        inputs: &[geo("Geometry"), SELECTION, s("Name", SocketType::String, SocketDefault::Str("")), any("Value", "dataType")],
        outputs: &[out("Geometry", SocketType::Geometry)],
        settings: &[Setting { id: "dataType", choices: FIELD_TYPES, default: "float" }, Setting { id: "domain", choices: DOMAINS, default: "point" }],
    },
    NodeKind {
        id: "captureAttribute",
        title: "Capture Attribute",
        category: "Attribute",
        inputs: &[geo("Geometry"), any("Value", "dataType")],
        outputs: &[out("Geometry", SocketType::Geometry), any("Value", "dataType")],
        settings: &[Setting { id: "dataType", choices: FIELD_TYPES, default: "float" }, Setting { id: "domain", choices: DOMAINS, default: "point" }],
    },
    NodeKind { id: "position", title: "Position", category: "Input", inputs: &[], outputs: &[out("Position", SocketType::Vector)], settings: &[] },
    NodeKind { id: "normal", title: "Normal", category: "Input", inputs: &[], outputs: &[out("Normal", SocketType::Vector)], settings: &[] },
    NodeKind { id: "index", title: "Index", category: "Input", inputs: &[], outputs: &[out("Index", SocketType::Int)], settings: &[] },
    NodeKind {
        id: "value",
        title: "Value",
        category: "Input",
        inputs: &[float("Value", 0.5)],
        outputs: &[out("Value", SocketType::Float)],
        settings: &[],
    },
    NodeKind {
        id: "vector",
        title: "Vector",
        category: "Input",
        inputs: &[vector("Vector", [0.0, 0.0, 0.0])],
        outputs: &[out("Vector", SocketType::Vector)],
        settings: &[],
    },
    NodeKind {
        id: "objectInfo",
        title: "Object Info",
        category: "Input",
        inputs: &[s("Object", SocketType::Object, SocketDefault::None), boolean("As Instance", false)],
        outputs: &[
            out("Location", SocketType::Vector),
            out("Rotation", SocketType::Rotation),
            out("Scale", SocketType::Vector),
            out("Geometry", SocketType::Geometry),
        ],
        settings: &[Setting { id: "transformSpace", choices: &["original", "relative"], default: "original" }],
    },
    NodeKind {
        id: "separateXyz",
        title: "Separate XYZ",
        category: "Utilities",
        inputs: &[vector("Vector", [0.0, 0.0, 0.0])],
        outputs: &[out("X", SocketType::Float), out("Y", SocketType::Float), out("Z", SocketType::Float)],
        settings: &[],
    },
    NodeKind {
        id: "combineXyz",
        title: "Combine XYZ",
        category: "Utilities",
        inputs: &[float("X", 0.0), float("Y", 0.0), float("Z", 0.0)],
        outputs: &[out("Vector", SocketType::Vector)],
        settings: &[],
    },
    NodeKind {
        id: "math",
        title: "Math",
        category: "Utilities",
        inputs: &[
            float("Value", 0.5),
            when(float("Value_001", 0.5), "operation", NOT_UNARY),
            when(float("Value_002", 0.5), "operation", MATH_TERNARY),
        ],
        outputs: &[out("Value", SocketType::Float)],
        settings: &[
            Setting { id: "operation", choices: MATH, default: "add" },
            Setting { id: "useClamp", choices: &["false", "true"], default: "false" },
        ],
    },
    NodeKind {
        id: "vectorMath",
        title: "Vector Math",
        category: "Utilities",
        inputs: &[
            vector("Vector", [0.0, 0.0, 0.0]),
            when(vector("Vector_001", [0.0, 0.0, 0.0]), "operation", VECTOR_TWO_PLUS),
            when(vector("Vector_002", [0.0, 0.0, 0.0]), "operation", VECTOR_THREE),
            when(float("Scale", 1.0), "operation", &["scale", "refract"]),
        ],
        outputs: &[
            when(out("Vector", SocketType::Vector), "operation", VECTOR_RESULT),
            when(out("Value", SocketType::Float), "operation", VECTOR_MATH_VALUE),
        ],
        settings: &[Setting { id: "operation", choices: VECTOR_MATH, default: "add" }],
    },
    NodeKind {
        id: "randomValue",
        title: "Random Value",
        category: "Utilities",
        inputs: &[
            when(float("Min", 0.0), "dataType", &["float"]),
            when(float("Max", 1.0), "dataType", &["float"]),
            when(vector("Min_001", [0.0, 0.0, 0.0]), "dataType", &["float_vector"]),
            when(vector("Max_001", [1.0, 1.0, 1.0]), "dataType", &["float_vector"]),
            when(int("Min_002", 0), "dataType", &["int"]),
            when(int("Max_002", 100), "dataType", &["int"]),
            when(float("Probability", 0.5), "dataType", &["boolean"]),
            implicit(int("ID", 0), Implicit::Index),
            int("Seed", 0),
        ],
        outputs: &[any("Value", "dataType")],
        settings: &[Setting { id: "dataType", choices: FIELD_TYPES, default: "float" }],
    },
    NodeKind {
        id: "compare",
        title: "Compare",
        category: "Utilities",
        inputs: &[
            when(float("A", 0.0), "dataType", &["float"]),
            when(float("B", 0.0), "dataType", &["float"]),
            when(int("A_INT", 0), "dataType", &["int"]),
            when(int("B_INT", 0), "dataType", &["int"]),
            when(vector("A_VEC3", [0.0, 0.0, 0.0]), "dataType", &["float_vector"]),
            when(vector("B_VEC3", [0.0, 0.0, 0.0]), "dataType", &["float_vector"]),
            when(float("Epsilon", 0.001), "operation", &["equal", "not_equal"]),
        ],
        outputs: &[out("Result", SocketType::Bool)],
        settings: &[
            Setting { id: "operation", choices: COMPARE, default: "greater_than" },
            Setting { id: "dataType", choices: &["float", "int", "float_vector"], default: "float" },
            Setting { id: "mode", choices: &["element", "length", "average", "dot_product", "direction"], default: "element" },
        ],
    },
    NodeKind {
        id: "switch",
        title: "Switch",
        category: "Utilities",
        inputs: &[boolean("Switch", false), any("False", "inputType"), any("True", "inputType")],
        outputs: &[any("Output", "inputType")],
        settings: &[Setting { id: "inputType", choices: SWITCH_TYPES, default: "float" }],
    },
    NodeKind {
        id: "mapRange",
        title: "Map Range",
        category: "Utilities",
        inputs: &[
            when(float("Value", 1.0), "dataType", &["float"]),
            when(float("From Min", 0.0), "dataType", &["float"]),
            when(float("From Max", 1.0), "dataType", &["float"]),
            when(float("To Min", 0.0), "dataType", &["float"]),
            when(float("To Max", 1.0), "dataType", &["float"]),
            when(float("Steps", 4.0), "interpolationType", &["stepped"]),
            when(vector("Vector", [0.0, 0.0, 0.0]), "dataType", &["float_vector"]),
            when(vector("From_Min_FLOAT3", [0.0, 0.0, 0.0]), "dataType", &["float_vector"]),
            when(vector("From_Max_FLOAT3", [1.0, 1.0, 1.0]), "dataType", &["float_vector"]),
            when(vector("To_Min_FLOAT3", [0.0, 0.0, 0.0]), "dataType", &["float_vector"]),
            when(vector("To_Max_FLOAT3", [1.0, 1.0, 1.0]), "dataType", &["float_vector"]),
        ],
        outputs: &[
            when(out("Result", SocketType::Float), "dataType", &["float"]),
            when(out("Vector", SocketType::Vector), "dataType", &["float_vector"]),
        ],
        settings: &[
            Setting { id: "dataType", choices: &["float", "float_vector"], default: "float" },
            Setting { id: "interpolationType", choices: &["linear", "stepped", "smoothstep", "smootherstep"], default: "linear" },
            Setting { id: "clamp", choices: &["false", "true"], default: "true" },
        ],
    },
    NodeKind {
        id: "sceneTime",
        title: "Scene Time",
        category: "Input",
        inputs: &[],
        outputs: &[out("Seconds", SocketType::Float), out("Frame", SocketType::Float)],
        settings: &[],
    },
    NodeKind {
        id: "namedAttribute",
        title: "Named Attribute",
        category: "Attribute",
        inputs: &[s("Name", SocketType::String, SocketDefault::Str(""))],
        outputs: &[any("Attribute", "dataType"), out("Exists", SocketType::Bool)],
        settings: &[Setting { id: "dataType", choices: FIELD_TYPES, default: "float" }],
    },
    NodeKind {
        id: "domainSize",
        title: "Domain Size",
        category: "Attribute",
        inputs: &[geo("Geometry")],
        outputs: &[
            when(out("Point Count", SocketType::Int), "component", &["mesh", "pointcloud", "curve"]),
            when(out("Edge Count", SocketType::Int), "component", &["mesh"]),
            when(out("Face Count", SocketType::Int), "component", &["mesh"]),
            when(out("Face Corner Count", SocketType::Int), "component", &["mesh"]),
            when(out("Spline Count", SocketType::Int), "component", &["curve"]),
            when(out("Instance Count", SocketType::Int), "component", &["instances"]),
        ],
        settings: &[Setting { id: "component", choices: &["mesh", "pointcloud", "curve", "instances"], default: "mesh" }],
    },
    NodeKind {
        id: "attributeStatistic",
        title: "Attribute Statistic",
        category: "Attribute",
        inputs: &[geo("Geometry"), SELECTION, any("Attribute", "dataType")],
        outputs: &[
            any("Mean", "dataType"),
            any("Median", "dataType"),
            any("Sum", "dataType"),
            any("Min", "dataType"),
            any("Max", "dataType"),
            any("Range", "dataType"),
            any("Standard Deviation", "dataType"),
            any("Variance", "dataType"),
        ],
        settings: &[
            Setting { id: "dataType", choices: NUMBER_TYPES, default: "float" },
            Setting { id: "domain", choices: DOMAINS, default: "point" },
        ],
    },
    NodeKind {
        id: "accumulateField",
        title: "Accumulate Field",
        category: "Utilities",
        inputs: &[with_default(any("Value", "dataType"), SocketDefault::Float(1.0)), named(int("Group Index", 0), "Group ID")],
        outputs: &[any("Leading", "dataType"), any("Trailing", "dataType"), any("Total", "dataType")],
        settings: &[
            Setting { id: "dataType", choices: ACCUMULATE_TYPES, default: "float" },
            Setting { id: "domain", choices: DOMAINS, default: "point" },
        ],
    },
    NodeKind {
        id: "meshBoolean",
        title: "Mesh Boolean",
        category: "Mesh",
        inputs: &[when(geo("Mesh 1"), "operation", &["difference"]), multi(geo("Mesh 2"))],
        outputs: &[out("Mesh", SocketType::Geometry)],
        settings: &[Setting { id: "operation", choices: &["intersect", "union", "difference"], default: "difference" }],
    },
    NodeKind {
        id: "dualMesh",
        title: "Dual Mesh",
        category: "Mesh",
        inputs: &[geo("Mesh"), boolean("Keep Boundaries", false)],
        outputs: &[out("Dual Mesh", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "flipFaces",
        title: "Flip Faces",
        category: "Mesh",
        inputs: &[geo("Mesh"), SELECTION],
        outputs: &[out("Mesh", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "splitEdges",
        title: "Split Edges",
        category: "Mesh",
        inputs: &[geo("Mesh"), SELECTION],
        outputs: &[out("Mesh", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "triangulate",
        title: "Triangulate",
        category: "Mesh",
        inputs: &[geo("Mesh"), SELECTION],
        outputs: &[out("Mesh", SocketType::Geometry)],
        settings: &[
            Setting {
                id: "quadMethod",
                choices: &["beauty", "fixed", "fixed_alternate", "shortest_diagonal", "longest_diagonal"],
                default: "shortest_diagonal",
            },
            Setting { id: "ngonMethod", choices: &["beauty", "clip"], default: "beauty" },
        ],
    },
    NodeKind {
        id: "scaleElements",
        title: "Scale Elements",
        category: "Mesh",
        inputs: &[
            geo("Geometry"),
            SELECTION,
            float("Scale", 1.0),
            implicit(vector("Center", [0.0, 0.0, 0.0]), Implicit::Position),
            when(vector("Axis", [1.0, 0.0, 0.0]), "scaleMode", &["single_axis"]),
        ],
        outputs: &[out("Geometry", SocketType::Geometry)],
        settings: &[
            Setting { id: "domain", choices: &["face", "edge"], default: "face" },
            Setting { id: "scaleMode", choices: &["uniform", "single_axis"], default: "uniform" },
        ],
    },
    NodeKind {
        id: "meshIsland",
        title: "Mesh Island",
        category: "Mesh",
        inputs: &[],
        outputs: &[out("Island Index", SocketType::Int), out("Island Count", SocketType::Int)],
        settings: &[],
    },
    NodeKind {
        id: "edgeAngle",
        title: "Edge Angle",
        category: "Mesh",
        inputs: &[],
        outputs: &[out("Unsigned Angle", SocketType::Float), out("Signed Angle", SocketType::Float)],
        settings: &[],
    },
    NodeKind { id: "faceArea", title: "Face Area", category: "Mesh", inputs: &[], outputs: &[out("Area", SocketType::Float)], settings: &[] },
    NodeKind {
        id: "edgeNeighbors",
        title: "Edge Neighbors",
        category: "Mesh",
        inputs: &[],
        outputs: &[out("Face Count", SocketType::Int)],
        settings: &[],
    },
    NodeKind {
        id: "sampleIndex",
        title: "Sample Index",
        category: "Geometry",
        inputs: &[geo("Geometry"), any("Value", "dataType"), int("Index", 0)],
        outputs: &[any("Value", "dataType")],
        settings: &[
            Setting { id: "dataType", choices: FIELD_TYPES, default: "float" },
            Setting { id: "domain", choices: DOMAINS, default: "point" },
            Setting { id: "clamp", choices: &["false", "true"], default: "false" },
        ],
    },
    NodeKind {
        id: "sampleNearest",
        title: "Sample Nearest",
        category: "Geometry",
        inputs: &[geo("Geometry"), implicit(vector("Sample Position", [0.0, 0.0, 0.0]), Implicit::Position)],
        outputs: &[out("Index", SocketType::Int)],
        settings: &[Setting { id: "domain", choices: MESH_DOMAINS, default: "point" }],
    },
    NodeKind {
        id: "raycast",
        title: "Raycast",
        category: "Geometry",
        inputs: &[
            geo("Target Geometry"),
            any("Attribute", "dataType"),
            implicit(vector("Source Position", [0.0, 0.0, 0.0]), Implicit::Position),
            vector("Ray Direction", [0.0, 0.0, -1.0]),
            float("Ray Length", 100.0),
        ],
        outputs: &[
            out("Is Hit", SocketType::Bool),
            out("Hit Position", SocketType::Vector),
            out("Hit Normal", SocketType::Vector),
            out("Hit Distance", SocketType::Float),
            any("Attribute", "dataType"),
        ],
        settings: &[
            Setting { id: "dataType", choices: FIELD_TYPES, default: "float" },
            Setting { id: "interpolation", choices: &["interpolated", "nearest"], default: "interpolated" },
        ],
    },
    NodeKind {
        id: "geometryProximity",
        title: "Geometry Proximity",
        category: "Geometry",
        inputs: &[
            named(geo("Target"), "Geometry"),
            int("Group ID", 0),
            named(implicit(vector("Source Position", [0.0, 0.0, 0.0]), Implicit::Position), "Sample Position"),
            int("Sample Group ID", 0),
        ],
        outputs: &[out("Position", SocketType::Vector), out("Distance", SocketType::Float), out("Is Valid", SocketType::Bool)],
        settings: &[Setting { id: "targetElement", choices: &["points", "edges", "faces"], default: "faces" }],
    },
    NodeKind {
        id: "translateInstances",
        title: "Translate Instances",
        category: "Instances",
        inputs: &[geo("Instances"), SELECTION, vector("Translation", [0.0, 0.0, 0.0]), boolean("Local Space", true)],
        outputs: &[out("Instances", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "rotateInstances",
        title: "Rotate Instances",
        category: "Instances",
        inputs: &[
            geo("Instances"),
            SELECTION,
            s("Rotation", SocketType::Rotation, SocketDefault::Vector([0.0, 0.0, 0.0])),
            vector("Pivot Point", [0.0, 0.0, 0.0]),
            boolean("Local Space", true),
        ],
        outputs: &[out("Instances", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "scaleInstances",
        title: "Scale Instances",
        category: "Instances",
        inputs: &[geo("Instances"), SELECTION, vector("Scale", [1.0, 1.0, 1.0]), vector("Center", [0.0, 0.0, 0.0]), boolean("Local Space", true)],
        outputs: &[out("Instances", SocketType::Geometry)],
        settings: &[],
    },
    NodeKind {
        id: "instanceRotation",
        title: "Instance Rotation",
        category: "Instances",
        inputs: &[],
        outputs: &[out("Rotation", SocketType::Rotation)],
        settings: &[],
    },
    NodeKind {
        id: "instanceScale",
        title: "Instance Scale",
        category: "Instances",
        inputs: &[],
        outputs: &[out("Scale", SocketType::Vector)],
        settings: &[],
    },
    NodeKind {
        id: "rotateEuler",
        title: "Rotate Euler",
        category: "Rotation",
        inputs: &[
            vector("Rotation", [0.0, 0.0, 0.0]),
            when(vector("Rotate By", [0.0, 0.0, 0.0]), "rotationType", &["euler"]),
            when(vector("Axis", [0.0, 0.0, 1.0]), "rotationType", &["axis_angle"]),
            when(float("Angle", 0.0), "rotationType", &["axis_angle"]),
        ],
        outputs: &[out("Rotation", SocketType::Vector)],
        settings: &[
            Setting { id: "rotationType", choices: &["axis_angle", "euler"], default: "euler" },
            Setting { id: "space", choices: &["object", "local"], default: "object" },
        ],
    },
    NodeKind {
        id: "rotateRotation",
        title: "Rotate Rotation",
        category: "Rotation",
        inputs: &[rotation("Rotation"), rotation("Rotate By")],
        outputs: &[out("Rotation", SocketType::Rotation)],
        settings: &[Setting { id: "rotationSpace", choices: &["global", "local"], default: "global" }],
    },
    NodeKind {
        id: "rotateVector",
        title: "Rotate Vector",
        category: "Rotation",
        inputs: &[vector("Vector", [0.0, 0.0, 0.0]), rotation("Rotation")],
        outputs: &[out("Vector", SocketType::Vector)],
        settings: &[],
    },
    NodeKind {
        id: "eulerToRotation",
        title: "Euler to Rotation",
        category: "Rotation",
        inputs: &[vector("Euler", [0.0, 0.0, 0.0])],
        outputs: &[out("Rotation", SocketType::Rotation)],
        settings: &[],
    },
    NodeKind {
        id: "rotationToEuler",
        title: "Rotation to Euler",
        category: "Rotation",
        inputs: &[rotation("Rotation")],
        outputs: &[out("Euler", SocketType::Vector)],
        settings: &[],
    },
    NodeKind {
        id: "axisAngleToRotation",
        title: "Axis Angle to Rotation",
        category: "Rotation",
        inputs: &[vector("Axis", [0.0, 0.0, 1.0]), float("Angle", 0.0)],
        outputs: &[out("Rotation", SocketType::Rotation)],
        settings: &[],
    },
    NodeKind {
        id: "invertRotation",
        title: "Invert Rotation",
        category: "Rotation",
        inputs: &[rotation("Rotation")],
        outputs: &[out("Rotation", SocketType::Rotation)],
        settings: &[],
    },
];

const NOT_UNARY: &[&str] = &[
    "add",
    "subtract",
    "multiply",
    "divide",
    "multiply_add",
    "power",
    "logarithm",
    "minimum",
    "maximum",
    "less_than",
    "greater_than",
    "compare",
    "smooth_min",
    "smooth_max",
    "modulo",
    "floored_modulo",
    "wrap",
    "snap",
    "pingpong",
    "arctan2",
];
const VECTOR_RESULT: &[&str] = &[
    "add",
    "subtract",
    "multiply",
    "divide",
    "multiply_add",
    "cross_product",
    "project",
    "reflect",
    "refract",
    "faceforward",
    "scale",
    "normalize",
    "absolute",
    "power",
    "sign",
    "minimum",
    "maximum",
    "round",
    "floor",
    "ceil",
    "fraction",
    "modulo",
    "wrap",
    "snap",
    "sine",
    "cosine",
    "tangent",
];

/// The node kind `id`.
pub fn node_kind(id: &str) -> Option<&'static NodeKind> {
    NODE_KINDS.iter().chain(crate::compositor_kinds::COMPOSITOR_KINDS).chain(crate::shader_kinds::SHADER_KINDS).find(|kind| kind.id == id)
}

/// Node kinds both trees take.
const SHARED_KINDS: &[&str] = &["value"];

/// Whether a group of `tree` takes nodes of kind `id`.
pub fn in_tree(id: &str, tree: Tree) -> bool {
    match tree {
        Tree::Geometry => NODE_KINDS.iter().any(|kind| kind.id == id),
        Tree::Compositor => SHARED_KINDS.contains(&id) || crate::compositor_kinds::COMPOSITOR_KINDS.iter().any(|kind| kind.id == id),
        Tree::Shader => crate::shader_kinds::SHARED_WITH_SHADERS.contains(&id) || crate::shader_kinds::SHADER_KINDS.iter().any(|kind| kind.id == id),
    }
}

/// The node kinds a group of `tree` takes, in the add menu's order.
pub fn kinds_of(tree: Tree) -> Vec<&'static NodeKind> {
    match tree {
        Tree::Geometry => NODE_KINDS.iter().collect(),
        Tree::Compositor => {
            crate::compositor_kinds::COMPOSITOR_KINDS.iter().chain(NODE_KINDS.iter().filter(|kind| SHARED_KINDS.contains(&kind.id))).collect()
        }
        Tree::Shader => {
            crate::shader_kinds::SHADER_KINDS.iter().chain(crate::shader_kinds::SHARED_WITH_SHADERS.iter().filter_map(|id| node_kind(id))).collect()
        }
    }
}

/// The settings' values for a node: its own, else the kind's defaults.
pub fn setting<'a>(node: &'a Node, kind: &NodeKind, id: &str) -> &'a str {
    match node.values.get(id) {
        Some(Value::String(value)) => value.as_str(),
        Some(Value::Bool(true)) => "true",
        Some(Value::Bool(false)) => "false",
        _ => kind.settings.iter().find(|setting| setting.id == id).map_or("", |setting| setting.default),
    }
}

/// Whether a socket shows for a node's settings.
pub fn socket_shown(node: &Node, kind: &NodeKind, socket: &Socket) -> bool {
    match socket.when {
        None => true,
        Some((id, values)) => values.contains(&setting(node, kind, id)),
    }
}

/// The type a socket has for a node's settings (an `Any` socket follows its setting).
pub fn socket_type(node: &Node, kind: &NodeKind, socket: &Socket) -> SocketType {
    match (socket.kind, socket.follows) {
        (SocketType::Any, Some(id)) => match setting(node, kind, id) {
            "int" => SocketType::Int,
            "boolean" => SocketType::Bool,
            "float_vector" | "vector" => SocketType::Vector,
            "rotation" => SocketType::Rotation,
            "geometry" => SocketType::Geometry,
            "object" => SocketType::Object,
            "material" => SocketType::Material,
            "rgba" | "color" => SocketType::Color,
            _ => SocketType::Float,
        },
        (kind, _) => kind,
    }
}

/// A group's Group Input node's outputs: the geometry, then the interface's inputs.
pub fn group_input_sockets(group: &NodeGroup) -> Vec<(String, SocketType)> {
    let mut out = vec![("Geometry".to_owned(), SocketType::Geometry)];
    out.extend(group.inputs.iter().map(|input| (input.name.clone(), input.kind)));
    out
}

/// A group's Group Output node's inputs: the geometry, then the interface's outputs.
pub fn group_output_sockets(group: &NodeGroup) -> Vec<(String, SocketType)> {
    let mut out = vec![("Geometry".to_owned(), SocketType::Geometry)];
    out.extend(group.outputs.iter().map(|output| (output.name.clone(), output.kind)));
    out
}

/// The types a group's interface sockets and a zone's items take.
pub const INTERFACE_TYPES: &[SocketType] =
    &[SocketType::Geometry, SocketType::Float, SocketType::Int, SocketType::Bool, SocketType::Vector, SocketType::Rotation];

/// One item a zone carries: a socket on both of its nodes, of `kind`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZoneItem {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: SocketType,
}

/// The most items one zone carries, and inputs or outputs one group has.
pub const MAX_ITEMS: usize = 64;

/// Socket names zone items may not take (the zone nodes' own sockets, and the values naming
/// the pair and the items).
pub const RESERVED_ITEM_NAMES: &[&str] = &["Iterations", "Iteration", "Delta Time", "Skip", "items", "output", "group"];

/// Whether `kind` is the input node of a zone, and the kind of its output node.
pub fn zone_output_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "repeatInput" => Some("repeatOutput"),
        "simulationInput" => Some("simulationOutput"),
        _ => None,
    }
}

/// Whether `kind` is a zone's output node.
pub fn is_zone_output(kind: &str) -> bool {
    matches!(kind, "repeatOutput" | "simulationOutput")
}

/// The id of the zone output a zone input node is paired with.
pub fn paired_output(node: &Node) -> Option<Id> {
    node.values.get("output").and_then(Value::as_u64).and_then(|id| Id::try_from(id).ok())
}

/// The group a Group node uses.
pub fn used_group(node: &Node) -> Option<Id> {
    if node.kind != "group" {
        return None;
    }
    node.values.get("group").and_then(Value::as_u64).and_then(|id| Id::try_from(id).ok())
}

/// The items a zone output node lists (one geometry when it lists none, as a new zone has).
pub fn items_of(output: &Node) -> Vec<ZoneItem> {
    match output.values.get("items") {
        Some(value) => serde_json::from_value(value.clone()).unwrap_or_default(),
        None => vec![ZoneItem { name: "Geometry".into(), kind: SocketType::Geometry }],
    }
}

impl NodeGroup {
    /// The zone output node a zone input node is paired with, if it is one of the right kind.
    pub fn zone_output_of(&self, input: &Node) -> Option<&Node> {
        let wanted = zone_output_kind(&input.kind)?;
        self.node(paired_output(input)?).filter(|node| node.kind == wanted)
    }

    /// The zone input node paired with a zone output node.
    pub fn zone_input_of(&self, output: &Node) -> Option<&Node> {
        self.nodes.iter().find(|node| zone_output_kind(&node.kind) == Some(output.kind.as_str()) && paired_output(node) == Some(output.id))
    }

    /// The items a zone node carries (an unpaired input carries none).
    pub fn zone_items(&self, node: &Node) -> Vec<ZoneItem> {
        if is_zone_output(&node.kind) {
            return items_of(node);
        }
        self.zone_output_of(node).map(items_of).unwrap_or_default()
    }
}

/// The groups a group's Group nodes use, directly.
pub fn groups_used(group: &NodeGroup) -> BTreeSet<Id> {
    group.nodes.iter().filter_map(used_group).collect()
}

/// Whether group `from` uses group `goal` (itself, or through Group nodes, at any depth).
pub fn uses_group(groups: &[NodeGroup], from: Id, goal: Id) -> bool {
    let mut seen = BTreeSet::new();
    let mut pending = vec![from];
    while let Some(at) = pending.pop() {
        if !seen.insert(at) {
            continue;
        }
        let Some(group) = groups.iter().find(|group| group.id == at) else { continue };
        for used in groups_used(group) {
            if used == goal {
                return true;
            }
            pending.push(used);
        }
    }
    false
}

/// Whether a node's sockets follow data (a group's interface, a zone's items) beside its kind's.
pub fn has_dynamic_sockets(kind: &str) -> bool {
    matches!(kind, "groupInput" | "groupOutput" | "group") || zone_output_kind(kind).is_some() || is_zone_output(kind)
}

/// The largest number of nodes one group may hold.
pub const MAX_NODES: usize = 2_000;

impl NodeGroup {
    /// A new group with a Group Input linked straight to a Group Output (which changes nothing).
    pub fn new(id: Id, name: impl Into<String>, input: Id, output: Id) -> Self {
        Self {
            id,
            name: name.into(),
            tree: Tree::Geometry,
            inputs: Vec::new(),
            outputs: Vec::new(),
            nodes: vec![
                Node { id: input, kind: "groupInput".into(), location: [-300.0, 0.0], values: BTreeMap::new() },
                Node { id: output, kind: "groupOutput".into(), location: [300.0, 0.0], values: BTreeMap::new() },
            ],
            links: vec![Link { from: input, from_socket: "Geometry".into(), to: output, to_socket: "Geometry".into() }],
        }
    }

    /// A new compositing group: Render Layers linked straight to the Group Output (which
    /// changes nothing).
    pub fn compositor(id: Id, name: impl Into<String>, input: Id, output: Id) -> Self {
        Self {
            id,
            name: name.into(),
            tree: Tree::Compositor,
            inputs: Vec::new(),
            outputs: Vec::new(),
            nodes: vec![
                Node { id: input, kind: "renderLayers".into(), location: [-300.0, 0.0], values: BTreeMap::new() },
                Node { id: output, kind: "compositorOutput".into(), location: [300.0, 0.0], values: BTreeMap::new() },
            ],
            links: vec![Link { from: input, from_socket: "Image".into(), to: output, to_socket: "Image".into() }],
        }
    }

    /// A new shader group: a Principled BSDF into the Material Output.
    pub fn shader(id: Id, name: impl Into<String>, principled: Id, output: Id) -> Self {
        Self {
            id,
            name: name.into(),
            tree: Tree::Shader,
            inputs: Vec::new(),
            outputs: Vec::new(),
            nodes: vec![
                Node { id: principled, kind: "principledBsdf".into(), location: [-300.0, 0.0], values: BTreeMap::new() },
                Node { id: output, kind: "materialOutput".into(), location: [300.0, 0.0], values: BTreeMap::new() },
            ],
            links: vec![Link { from: principled, from_socket: "BSDF".into(), to: output, to_socket: "Surface".into() }],
        }
    }

    /// The node with `id`.
    pub fn node(&self, id: Id) -> Option<&Node> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// The node with `id`, to change.
    pub fn node_mut(&mut self, id: Id) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|node| node.id == id)
    }

    /// The output sockets a node has: its kind's, or for a Group Input the group's inputs, for a
    /// Group node the used group's outputs, and for a zone node its items (`groups` are the
    /// scene's, which Group nodes name).
    pub fn outputs_of(&self, node: &Node, groups: &[NodeGroup]) -> Vec<(String, SocketType)> {
        match node.kind.as_str() {
            "groupInput" => return group_input_sockets(self),
            "group" => {
                return used_group(node).and_then(|id| groups.iter().find(|group| group.id == id)).map(group_output_sockets).unwrap_or_default();
            }
            _ => {}
        }
        let Some(kind) = node_kind(&node.kind) else { return Vec::new() };
        let mut out: Vec<(String, SocketType)> = kind.outputs.iter().map(|socket| (socket.id.to_owned(), socket_type(node, kind, socket))).collect();
        if is_zone_output(&node.kind) || zone_output_kind(&node.kind).is_some() {
            out.extend(self.zone_items(node).into_iter().map(|item| (item.name, item.kind)));
        }
        out
    }

    /// The input sockets a node has, and whether each takes several links (see
    /// [`NodeGroup::outputs_of`]).
    pub fn inputs_of(&self, node: &Node, groups: &[NodeGroup]) -> Vec<(String, SocketType, bool)> {
        match node.kind.as_str() {
            "groupOutput" => return group_output_sockets(self).into_iter().map(|(id, kind)| (id, kind, false)).collect(),
            "group" => {
                return used_group(node)
                    .and_then(|id| groups.iter().find(|group| group.id == id))
                    .map(group_input_sockets)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(id, kind)| (id, kind, false))
                    .collect();
            }
            _ => {}
        }
        let Some(kind) = node_kind(&node.kind) else { return Vec::new() };
        let mut out: Vec<(String, SocketType, bool)> =
            kind.inputs.iter().map(|socket| (socket.id.to_owned(), socket_type(node, kind, socket), socket.multi)).collect();
        if is_zone_output(&node.kind) || zone_output_kind(&node.kind).is_some() {
            out.extend(self.zone_items(node).into_iter().map(|item| (item.name, item.kind, false)));
        }
        out
    }

    /// Checks kinds, ids, links, and values (`groups` are the scene's, which Group nodes name).
    pub fn validate(&self, groups: &[NodeGroup]) -> Result<()> {
        if self.nodes.len() > MAX_NODES {
            return Err(SceneError::InvalidFile(format!("node group {} has too many nodes", self.name)));
        }
        let bad = |what: String| Err(SceneError::InvalidFile(format!("node group {} {what}", self.name)));
        let mut ids = BTreeSet::new();
        for node in &self.nodes {
            if !ids.insert(node.id) {
                return bad(format!("repeats node {}", node.id));
            }
            if node_kind(&node.kind).is_none() {
                return bad(format!("has a node of unknown kind {}", node.kind));
            }
            if !in_tree(&node.kind, self.tree) {
                return bad(format!("has a node of kind {} its tree does not take", node.kind));
            }
            if !(node.location.iter().all(|value| value.is_finite())) {
                return Err(SceneError::InvalidFile(format!("node {} is not at a finite place", node.id)));
            }
            if let Some(used) = used_group(node)
                && !groups.iter().any(|group| group.id == used && group.tree == Tree::Geometry)
            {
                return bad(format!("has a Group node using a missing or non-geometry group {used}"));
            }
            if is_zone_output(&node.kind) {
                if node.values.get("items").is_some_and(|items| serde_json::from_value::<Vec<ZoneItem>>(items.clone()).is_err()) {
                    return bad(format!("has a zone with damaged items at node {}", node.id));
                }
                let items = items_of(node);
                let mut names = BTreeSet::new();
                if items.len() > MAX_ITEMS
                    || items.iter().any(|item| {
                        item.name.trim().is_empty()
                            || RESERVED_ITEM_NAMES.contains(&item.name.as_str())
                            || !INTERFACE_TYPES.contains(&item.kind)
                            || !names.insert(item.name.as_str())
                    })
                {
                    return bad(format!("has a zone with unnamed, repeated, or wrongly typed items at node {}", node.id));
                }
            }
            if zone_output_kind(&node.kind).is_some()
                && let Some(output) = paired_output(node)
            {
                if self.zone_output_of(node).is_none() {
                    return bad(format!("pairs zone input {} with a missing output {output}", node.id));
                }
                if self
                    .nodes
                    .iter()
                    .any(|other| other.id != node.id && zone_output_kind(&other.kind).is_some() && paired_output(other) == Some(output))
                {
                    return bad(format!("pairs two zone inputs with output {output}"));
                }
            }
        }
        for (sockets, outputs) in [(&self.inputs, false), (&self.outputs, true)] {
            let mut names = BTreeSet::new();
            for socket in sockets {
                if socket.name.trim().is_empty() || socket.name == "Geometry" || !names.insert(socket.name.as_str()) {
                    return bad("has an unnamed or repeated input or output".into());
                }
                let allowed = if outputs {
                    INTERFACE_TYPES.contains(&socket.kind)
                } else {
                    !matches!(socket.kind, SocketType::Any | SocketType::Color | SocketType::Curves | SocketType::Ramp | SocketType::Shader)
                };
                if self.tree != Tree::Geometry || !allowed {
                    return bad(format!("has an input or output {} of a type groups cannot take", socket.name));
                }
            }
        }
        for link in &self.links {
            let (Some(from), Some(to)) = (self.node(link.from), self.node(link.to)) else {
                return bad("links a missing node".into());
            };
            if !self.outputs_of(from, groups).iter().any(|(id, _)| *id == link.from_socket) {
                return bad(format!("links from a missing socket {}", link.from_socket));
            }
            if !self.inputs_of(to, groups).iter().any(|(id, _, _)| *id == link.to_socket) {
                return bad(format!("links to a missing socket {}", link.to_socket));
            }
        }
        Ok(())
    }
}

/// Whether an input socket of a node kind takes several links.
pub fn is_multi_input(kind: &str, socket: &str) -> bool {
    node_kind(kind).is_some_and(|kind| kind.inputs.iter().any(|input| input.id == socket && input.multi))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_unique_sockets_and_known_settings() {
        let mut kinds = BTreeSet::new();
        for kind in NODE_KINDS {
            assert!(kinds.insert(kind.id), "{} twice", kind.id);
            let mut inputs = BTreeSet::new();
            for socket in kind.inputs {
                assert!(inputs.insert(socket.id), "{}: input {} twice", kind.id, socket.id);
                if let Some((setting, values)) = socket.when {
                    let known = kind.settings.iter().find(|item| item.id == setting).unwrap_or_else(|| panic!("{}: no setting {setting}", kind.id));
                    for value in values {
                        assert!(known.choices.contains(value), "{}: {setting} has no {value}", kind.id);
                    }
                }
            }
            for setting in kind.settings {
                assert!(setting.choices.contains(&setting.default), "{}: {} default", kind.id, setting.id);
            }
        }
    }

    #[test]
    fn a_new_group_passes_its_geometry_through_and_validates() {
        let group = NodeGroup::new(10, "Geometry Nodes", 11, 12);
        group.validate(&[]).unwrap();
        assert_eq!(group.links.len(), 1);
        let math =
            Node { id: 1, kind: "math".into(), location: [0.0, 0.0], values: BTreeMap::from([("operation".into(), serde_json::json!("sqrt"))]) };
        let kind = node_kind("math").unwrap();
        let shown: Vec<&str> = kind.inputs.iter().filter(|socket| socket_shown(&math, kind, socket)).map(|socket| socket.id).collect();
        assert_eq!(shown, ["Value"]);
    }
}
