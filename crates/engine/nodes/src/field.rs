//! Fields: values computed per element of a geometry's domain (a position per point, a normal
//! per face), built lazily from inputs (position, normal, index, stored attributes) and
//! functions (math, random, comparisons), and evaluated only when a geometry node needs them on a
//! domain.

use std::sync::Arc;

use glam::{Mat3, Vec3};
use kition_scene_model::scene::{euler_matrix, matrix_euler};

use crate::geometry::{Component, Domain};
use crate::random;

/// One value of a field or an attribute.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Scalar {
    Float(f32),
    Int(i32),
    Bool(bool),
    Vector(Vec3),
}

/// A field's or an attribute's value type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Float,
    Int,
    Bool,
    Vector,
}

impl Scalar {
    /// As a number (a vector's mean, a boolean's 0 or 1).
    pub fn float(self) -> f32 {
        match self {
            Self::Float(value) => value,
            Self::Int(value) => value as f32,
            Self::Bool(value) => f32::from(u8::from(value)),
            Self::Vector(value) => (value.x + value.y + value.z) / 3.0,
        }
    }

    /// As a whole number (rounded to nearest, as the reference converts).
    pub fn int(self) -> i32 {
        match self {
            Self::Int(value) => value,
            other => {
                let value = other.float();
                if value.is_finite() { value.round().clamp(i32::MIN as f32, i32::MAX as f32) as i32 } else { 0 }
            }
        }
    }

    /// As a boolean (positive is true).
    pub fn bool(self) -> bool {
        match self {
            Self::Bool(value) => value,
            Self::Int(value) => value > 0,
            other => other.float() > 0.0,
        }
    }

    /// As a vector (a number fills every axis).
    pub fn vector(self) -> Vec3 {
        match self {
            Self::Vector(value) => value,
            other => Vec3::splat(other.float()),
        }
    }

    /// Converted to `kind`.
    pub fn to(self, kind: Kind) -> Self {
        match kind {
            Kind::Float => Self::Float(self.float()),
            Kind::Int => Self::Int(self.int()),
            Kind::Bool => Self::Bool(self.bool()),
            Kind::Vector => Self::Vector(self.vector()),
        }
    }

    /// The value's type.
    pub fn kind(self) -> Kind {
        match self {
            Self::Float(_) => Kind::Float,
            Self::Int(_) => Kind::Int,
            Self::Bool(_) => Kind::Bool,
            Self::Vector(_) => Kind::Vector,
        }
    }

    /// The zero of `kind`.
    pub fn zero(kind: Kind) -> Self {
        match kind {
            Kind::Float => Self::Float(0.0),
            Kind::Int => Self::Int(0),
            Kind::Bool => Self::Bool(false),
            Kind::Vector => Self::Vector(Vec3::ZERO),
        }
    }
}

/// Values per element of a domain.
pub type Column = Vec<Scalar>;

/// A function a field applies to its arguments, element by element.
#[derive(Debug, Clone, PartialEq)]
pub enum Function {
    Math {
        op: String,
        clamp: bool,
    },
    VectorMath {
        op: String,
        value: bool,
    },
    Separate(usize),
    Combine,
    /// Random values of a type: arguments are the bounds (or the probability), the id, the seed.
    Random {
        kind: Kind,
    },
    Compare {
        op: String,
        kind: Kind,
        mode: String,
    },
    /// Switch, if, then, else.
    Switch,
    MapRange {
        vector: bool,
        interpolation: String,
        clamp: bool,
    },
    /// Converts its one argument.
    Convert(Kind),
    /// Rotate Euler: the rotation (Euler angles), then the rotation by Euler angles or by an
    /// axis and an angle, in the object's space or the rotation's own.
    RotateEuler {
        axis_angle: bool,
        local: bool,
    },
    /// Rotate Rotation: a rotation turned by another, globally or in its own space.
    RotateRotation {
        local: bool,
    },
    /// Rotate Vector: a vector turned by a rotation.
    RotateVector,
    /// Axis Angle to Rotation.
    AxisAngle,
    /// Invert Rotation.
    InvertRotation,
    /// Euler angles as the reference writes a rotation's (Rotation to Euler).
    Euler,
}

/// A mesh's own value per element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshInput {
    /// Which connected part each element is in.
    IslandIndex,
    /// How many connected parts the mesh has.
    IslandCount,
    /// The angle between an edge's two faces' normals.
    UnsignedAngle,
    /// That angle, negative where the edge is concave.
    SignedAngle,
    FaceArea,
    /// How many faces use an edge.
    EdgeFaces,
}

/// Accumulate Field: running totals of `value` over `domain`, per group.
#[derive(Debug, Clone, PartialEq)]
pub struct Accumulate {
    pub value: Field,
    pub group: Field,
    pub domain: Domain,
    pub kind: Kind,
    /// Leading (0), Trailing (1), or Total (2).
    pub which: usize,
}

/// A field.
#[derive(Debug, Clone, PartialEq)]
pub enum Field {
    Constant(Scalar),
    Position,
    Normal,
    Index,
    /// A stored attribute (named, or anonymous: a node's own output, such as a distributed
    /// point's normal).
    Attribute(String, Kind),
    Apply(Arc<Function>, Vec<Field>),
    /// Spline Parameter's factor (0), length (1), or index (2) on curves.
    SplineParameter(usize),
    /// Endpoint Selection: the first and last points of each curve, as many as the two fields
    /// (evaluated per curve) say.
    Endpoints(Arc<(Field, Field)>),
    Mesh(MeshInput),
    /// Instances' rotation (Euler angles) or scale, from their transforms.
    InstanceRotation,
    InstanceScale,
    /// Whether an attribute of this name is there.
    Exists(String),
    Accumulate(Arc<Accumulate>),
    /// A value read from other geometry (Sample Index, Sample Nearest, Raycast, Geometry
    /// Proximity).
    Sample(Arc<crate::sample::Sample>),
}

impl Field {
    /// The value type the field gives.
    pub fn kind(&self) -> Kind {
        match self {
            Self::Constant(value) => value.kind(),
            Self::Position | Self::Normal => Kind::Vector,
            Self::Index => Kind::Int,
            Self::SplineParameter(which) => {
                if *which == 2 {
                    Kind::Int
                } else {
                    Kind::Float
                }
            }
            Self::Endpoints(_) | Self::Exists(_) => Kind::Bool,
            Self::Mesh(input) => match input {
                MeshInput::IslandIndex | MeshInput::IslandCount | MeshInput::EdgeFaces => Kind::Int,
                MeshInput::UnsignedAngle | MeshInput::SignedAngle | MeshInput::FaceArea => Kind::Float,
            },
            Self::InstanceRotation | Self::InstanceScale => Kind::Vector,
            Self::Accumulate(accumulate) => accumulate.kind,
            Self::Sample(sample) => sample.kind(),
            Self::Attribute(_, kind) => *kind,
            Self::Apply(function, arguments) => match function.as_ref() {
                Function::Math { .. } | Function::Separate(_) => Kind::Float,
                Function::VectorMath { value, .. } => {
                    if *value {
                        Kind::Float
                    } else {
                        Kind::Vector
                    }
                }
                Function::Combine
                | Function::RotateEuler { .. }
                | Function::RotateRotation { .. }
                | Function::RotateVector
                | Function::AxisAngle
                | Function::InvertRotation
                | Function::Euler => Kind::Vector,
                Function::Random { kind } | Function::Convert(kind) => *kind,
                Function::Compare { .. } => Kind::Bool,
                Function::Switch => arguments.get(1).map_or(Kind::Float, Field::kind),
                Function::MapRange { vector, .. } => {
                    if *vector {
                        Kind::Vector
                    } else {
                        Kind::Float
                    }
                }
            },
        }
    }

    /// Whether the field is the same for every element (no input reads the geometry).
    pub fn is_constant(&self) -> bool {
        match self {
            Self::Constant(_) => true,
            Self::Position
            | Self::Normal
            | Self::Index
            | Self::Attribute(..)
            | Self::SplineParameter(_)
            | Self::Endpoints(_)
            | Self::Mesh(_)
            | Self::InstanceRotation
            | Self::InstanceScale
            | Self::Exists(_)
            | Self::Accumulate(_)
            | Self::Sample(_) => false,
            Self::Apply(_, arguments) => arguments.iter().all(Field::is_constant),
        }
    }

    /// The field converted to `kind`.
    pub fn convert(self, kind: Kind) -> Self {
        if self.kind() == kind {
            return self;
        }
        match self {
            Self::Constant(value) => Self::Constant(value.to(kind)),
            other => Self::Apply(Arc::new(Function::Convert(kind)), vec![other]),
        }
    }
}

/// Where a field is evaluated: a component of a geometry and one of its domains.
pub struct Context<'a> {
    pub component: Component<'a>,
    pub domain: Domain,
}

impl Context<'_> {
    /// The number of elements.
    pub fn len(&self) -> usize {
        self.component.domain_len(self.domain)
    }

    /// Whether the domain has no elements.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Evaluates `field` on every element of the context's domain.
pub fn evaluate(field: &Field, context: &Context<'_>) -> Column {
    let len = context.len();
    match field {
        Field::Constant(value) => vec![*value; len],
        Field::Position => context.component.positions(context.domain).into_iter().map(Scalar::Vector).collect(),
        Field::Normal => context.component.normals(context.domain).into_iter().map(Scalar::Vector).collect(),
        Field::Index => (0..len).map(|index| Scalar::Int(i32::try_from(index).unwrap_or(i32::MAX))).collect(),
        Field::Attribute(name, kind) => match context.component.attribute(name, context.domain) {
            Some(column) => column.into_iter().map(|value| value.to(*kind)).collect(),
            None => vec![Scalar::zero(*kind); len],
        },
        Field::SplineParameter(which) => match context.component {
            Component::Curves(curves) => {
                let columns = match context.domain {
                    Domain::Curve => crate::curve_ops::spline_parameter_curves(curves),
                    _ => crate::curve_ops::spline_parameter_points(curves),
                };
                let mut column = columns.into_iter().nth(*which).unwrap_or_default();
                let zero = Scalar::zero(field.kind());
                column.resize(len, zero);
                column
            }
            _ => vec![Scalar::zero(field.kind()); len],
        },
        Field::Endpoints(sizes) => match context.component {
            Component::Curves(curves) => {
                let on_curves = Context { component: context.component, domain: Domain::Curve };
                let starts: Vec<i32> = evaluate(&sizes.0, &on_curves).iter().map(|value| value.int()).collect();
                let ends: Vec<i32> = evaluate(&sizes.1, &on_curves).iter().map(|value| value.int()).collect();
                let on_points = crate::curve_ops::endpoints(curves, &starts, &ends);
                curves.adapt(&on_points, Domain::Point, context.domain)
            }
            _ => vec![Scalar::Bool(false); len],
        },
        Field::Mesh(input) => match context.component {
            Component::Mesh(mesh) => crate::mesh_ops::mesh_input(mesh, *input, context.domain),
            _ => vec![Scalar::zero(field.kind()); len],
        },
        Field::InstanceRotation | Field::InstanceScale => match context.component {
            Component::Instances(instances) => instances
                .items
                .iter()
                .map(|item| {
                    let matrix = Mat3::from_mat4(item.transform);
                    if matches!(field, Field::InstanceScale) {
                        Scalar::Vector(Vec3::new(matrix.x_axis.length(), matrix.y_axis.length(), matrix.z_axis.length()))
                    } else {
                        Scalar::Vector(matrix_euler(rotation_part(matrix)))
                    }
                })
                .collect(),
            _ => vec![Scalar::zero(Kind::Vector); len],
        },
        Field::Exists(name) => {
            let there = name == "position" || context.component.attribute(name, context.domain).is_some();
            vec![Scalar::Bool(there); len]
        }
        Field::Accumulate(accumulate) => accumulated(accumulate, context),
        Field::Sample(sample) => crate::sample::evaluate(sample, context),
        Field::Apply(function, arguments) => {
            let columns: Vec<Column> = arguments.iter().map(|argument| evaluate(argument, context)).collect();
            let mut out = Vec::with_capacity(len);
            let mut row = Vec::with_capacity(columns.len());
            for index in 0..len {
                row.clear();
                row.extend(columns.iter().map(|column| column.get(index).copied().unwrap_or(Scalar::Float(0.0))));
                out.push(apply(function, &row));
            }
            out
        }
    }
}

/// A field's single value, for an input that takes one (a grid's vertex count): fields that read
/// the geometry see one element at the origin.
pub fn single(field: &Field) -> Scalar {
    let empty = crate::geometry::PointsComponent { positions: vec![Vec3::ZERO], radius: vec![0.0], attributes: Default::default() };
    let context = Context { component: Component::Points(&empty), domain: Domain::Point };
    evaluate(field, &context).first().copied().unwrap_or(Scalar::Float(0.0))
}

fn arg(row: &[Scalar], index: usize) -> Scalar {
    row.get(index).copied().unwrap_or(Scalar::Float(0.0))
}

fn safe_divide(a: f32, b: f32) -> f32 {
    if b == 0.0 { 0.0 } else { a / b }
}

fn wrap(value: f32, max: f32, min: f32) -> f32 {
    let range = max - min;
    if range == 0.0 { min } else { value - range * ((value - min) / range).floor() }
}

fn smooth_min(a: f32, b: f32, c: f32) -> f32 {
    if c != 0.0 {
        let h = (c - (a - b).abs()).max(0.0) / c;
        a.min(b) - h * h * h * c / 6.0
    } else {
        a.min(b)
    }
}

/// The Math node's operations, on numbers.
pub fn math(op: &str, a: f32, b: f32, c: f32) -> f32 {
    match op {
        "add" => a + b,
        "subtract" => a - b,
        "multiply" => a * b,
        "divide" => safe_divide(a, b),
        "multiply_add" => a * b + c,
        "power" if a >= 0.0 || b.fract() == 0.0 => a.powf(b),
        "logarithm" if a > 0.0 && b > 0.0 && b != 1.0 => a.ln() / b.ln(),
        "sqrt" if a > 0.0 => a.sqrt(),
        "inverse_sqrt" if a > 0.0 => 1.0 / a.sqrt(),
        "absolute" => a.abs(),
        "exponent" => a.exp(),
        "minimum" => a.min(b),
        "maximum" => a.max(b),
        "less_than" => f32::from(u8::from(a < b)),
        "greater_than" => f32::from(u8::from(a > b)),
        "sign" => {
            if a > 0.0 {
                1.0
            } else if a < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
        "compare" => f32::from(u8::from((a - b).abs() <= c.max(1e-5))),
        "smooth_min" => smooth_min(a, b, c),
        "smooth_max" => -smooth_min(-a, -b, c),
        "round" => (a + 0.5).floor(),
        "floor" => a.floor(),
        "ceil" => a.ceil(),
        "trunc" => a.trunc(),
        "fract" => a - a.floor(),
        "modulo" if b != 0.0 => a % b,
        "floored_modulo" if b != 0.0 => a - (a / b).floor() * b,
        "wrap" => wrap(a, b, c),
        "snap" if b != 0.0 => (a / b).floor() * b,
        "pingpong" if b != 0.0 => (wrap(a, b * 2.0, 0.0) - b).abs(),
        "sine" => a.sin(),
        "cosine" => a.cos(),
        "tangent" => a.tan(),
        "arcsine" if (-1.0..=1.0).contains(&a) => a.asin(),
        "arccosine" if (-1.0..=1.0).contains(&a) => a.acos(),
        "arctangent" => a.atan(),
        "arctan2" => a.atan2(b),
        "sinh" => a.sinh(),
        "cosh" => a.cosh(),
        "tanh" => a.tanh(),
        "radians" => a.to_radians(),
        "degrees" => a.to_degrees(),
        _ => 0.0,
    }
}

fn per_axis(a: Vec3, b: Vec3, f: impl Fn(f32, f32) -> f32) -> Vec3 {
    Vec3::new(f(a.x, b.x), f(a.y, b.y), f(a.z, b.z))
}

/// The Vector Math node's operations: a vector, or a number in `x` for the value operations.
pub fn vector_math(op: &str, a: Vec3, b: Vec3, c: Vec3, scale: f32) -> Vec3 {
    match op {
        "add" => a + b,
        "subtract" => a - b,
        "multiply" => a * b,
        "divide" => per_axis(a, b, safe_divide),
        "multiply_add" => a * b + c,
        "cross_product" => a.cross(b),
        "project" => {
            let length = b.length_squared();
            if length > 0.0 { b * (a.dot(b) / length) } else { Vec3::ZERO }
        }
        "reflect" => {
            let normal = b.normalize_or_zero();
            a - 2.0 * a.dot(normal) * normal
        }
        "refract" => {
            let normal = b.normalize_or_zero();
            let cosine = a.dot(normal);
            let k = 1.0 - scale * scale * (1.0 - cosine * cosine);
            if k < 0.0 { Vec3::ZERO } else { scale * a - (scale * cosine + k.sqrt()) * normal }
        }
        "faceforward" => {
            if c.dot(b) < 0.0 {
                a
            } else {
                -a
            }
        }
        "dot_product" => Vec3::splat(a.dot(b)),
        "distance" => Vec3::splat(a.distance(b)),
        "length" => Vec3::splat(a.length()),
        "scale" => a * scale,
        "normalize" => a.normalize_or_zero(),
        "absolute" => a.abs(),
        "power" => per_axis(a, b, |x, y| math("power", x, y, 0.0)),
        "sign" => per_axis(a, a, |x, _| math("sign", x, 0.0, 0.0)),
        "minimum" => a.min(b),
        "maximum" => a.max(b),
        "round" => per_axis(a, a, |x, _| (x + 0.5).floor()),
        "floor" => a.floor(),
        "ceil" => a.ceil(),
        "fraction" => a - a.floor(),
        "modulo" => per_axis(a, b, |x, y| math("modulo", x, y, 0.0)),
        "wrap" => Vec3::new(wrap(a.x, b.x, c.x), wrap(a.y, b.y, c.y), wrap(a.z, b.z, c.z)),
        "snap" => per_axis(a, b, |x, y| math("snap", x, y, 0.0)),
        "sine" => Vec3::new(a.x.sin(), a.y.sin(), a.z.sin()),
        "cosine" => Vec3::new(a.x.cos(), a.y.cos(), a.z.cos()),
        "tangent" => Vec3::new(a.x.tan(), a.y.tan(), a.z.tan()),
        _ => Vec3::ZERO,
    }
}

fn compare(op: &str, kind: Kind, mode: &str, a: Scalar, b: Scalar, epsilon: f32) -> bool {
    let (x, y) = match kind {
        Kind::Vector => {
            let (u, v) = (a.vector(), b.vector());
            match mode {
                "length" => (u.length(), v.length()),
                "average" => ((u.x + u.y + u.z) / 3.0, (v.x + v.y + v.z) / 3.0),
                "dot_product" => (u.dot(v), epsilon),
                "direction" => (u.normalize_or_zero().dot(v.normalize_or_zero()).clamp(-1.0, 1.0).acos(), epsilon),
                _ => {
                    let check = |x: f32, y: f32| compare(op, Kind::Float, "element", Scalar::Float(x), Scalar::Float(y), epsilon);
                    return check(u.x, v.x) && check(u.y, v.y) && check(u.z, v.z);
                }
            }
        }
        Kind::Int => (a.int() as f32, b.int() as f32),
        _ => (a.float(), b.float()),
    };
    match op {
        "less_than" => x < y,
        "less_equal" => x <= y,
        "greater_than" => x > y,
        "greater_equal" => x >= y,
        "equal" => (x - y).abs() <= epsilon,
        "not_equal" => (x - y).abs() > epsilon,
        _ => false,
    }
}

/// The Map Range node on one number.
pub fn map_range(value: f32, from_min: f32, from_max: f32, to_min: f32, to_max: f32, steps: f32, interpolation: &str, clamp: bool) -> f32 {
    let span = from_max - from_min;
    if span == 0.0 {
        return 0.0;
    }
    let mut factor = (value - from_min) / span;
    match interpolation {
        "stepped" if steps > 0.0 => factor = (factor * (steps + 1.0)).floor() / steps,
        "smoothstep" => {
            let t = factor.clamp(0.0, 1.0);
            factor = t * t * (3.0 - 2.0 * t);
        }
        "smootherstep" => {
            let t = factor.clamp(0.0, 1.0);
            factor = t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
        }
        _ => {}
    }
    let result = to_min + factor * (to_max - to_min);
    if clamp && interpolation != "smoothstep" && interpolation != "smootherstep" {
        let (low, high) = if to_min < to_max { (to_min, to_max) } else { (to_max, to_min) };
        result.clamp(low, high)
    } else {
        result
    }
}

fn apply(function: &Function, row: &[Scalar]) -> Scalar {
    match function {
        Function::Math { op, clamp } => {
            let value = math(op, arg(row, 0).float(), arg(row, 1).float(), arg(row, 2).float());
            Scalar::Float(if *clamp { value.clamp(0.0, 1.0) } else { value })
        }
        Function::VectorMath { op, value } => {
            let out = vector_math(op, arg(row, 0).vector(), arg(row, 1).vector(), arg(row, 2).vector(), arg(row, 3).float());
            if *value { Scalar::Float(out.x) } else { Scalar::Vector(out) }
        }
        Function::Separate(axis) => Scalar::Float(arg(row, 0).vector().to_array().get(*axis).copied().unwrap_or(0.0)),
        Function::Combine => Scalar::Vector(Vec3::new(arg(row, 0).float(), arg(row, 1).float(), arg(row, 2).float())),
        Function::Random { kind } => {
            // Arguments: min, max (or probability), id, seed.
            let id = arg(row, 2).int();
            let seed = arg(row, 3).int();
            match kind {
                Kind::Float => {
                    let (low, high) = (arg(row, 0).float(), arg(row, 1).float());
                    Scalar::Float(low + (high - low) * random::unit(id, seed, 0))
                }
                Kind::Int => {
                    let (low, high) = (arg(row, 0).int(), arg(row, 1).int());
                    let (low, high) = (low.min(high), low.max(high));
                    let span = i64::from(high) - i64::from(low) + 1;
                    let pick = (f64::from(random::unit(id, seed, 0)) * span as f64).floor() as i64;
                    Scalar::Int(i32::try_from(i64::from(low) + pick.min(span - 1)).unwrap_or(low))
                }
                Kind::Bool => Scalar::Bool(random::unit(id, seed, 0) < arg(row, 0).float()),
                Kind::Vector => {
                    let (low, high) = (arg(row, 0).vector(), arg(row, 1).vector());
                    let unit = Vec3::new(random::unit(id, seed, 0), random::unit(id, seed, 1), random::unit(id, seed, 2));
                    Scalar::Vector(low + (high - low) * unit)
                }
            }
        }
        Function::Compare { op, kind, mode } => Scalar::Bool(compare(op, *kind, mode, arg(row, 0), arg(row, 1), arg(row, 2).float())),
        Function::Switch => {
            if arg(row, 0).bool() {
                arg(row, 2)
            } else {
                arg(row, 1)
            }
        }
        Function::MapRange { vector, interpolation, clamp } => {
            if *vector {
                let (value, from_min, from_max, to_min, to_max, steps) = (
                    arg(row, 0).vector(),
                    arg(row, 1).vector(),
                    arg(row, 2).vector(),
                    arg(row, 3).vector(),
                    arg(row, 4).vector(),
                    arg(row, 5).vector(),
                );
                let axis = |index: usize| {
                    let pick = |v: Vec3| v.to_array().get(index).copied().unwrap_or(0.0);
                    map_range(pick(value), pick(from_min), pick(from_max), pick(to_min), pick(to_max), pick(steps), interpolation, *clamp)
                };
                Scalar::Vector(Vec3::new(axis(0), axis(1), axis(2)))
            } else {
                Scalar::Float(map_range(
                    arg(row, 0).float(),
                    arg(row, 1).float(),
                    arg(row, 2).float(),
                    arg(row, 3).float(),
                    arg(row, 4).float(),
                    arg(row, 5).float(),
                    interpolation,
                    *clamp,
                ))
            }
        }
        Function::Convert(kind) => arg(row, 0).to(*kind),
        Function::RotateEuler { axis_angle, local } => {
            let rotation = euler_matrix(arg(row, 0).vector());
            let by = if *axis_angle { axis_angle_matrix(arg(row, 2).vector(), arg(row, 3).float()) } else { euler_matrix(arg(row, 1).vector()) };
            Scalar::Vector(matrix_euler(if *local { rotation * by } else { by * rotation }))
        }
        Function::RotateRotation { local } => {
            let (rotation, by) = (euler_matrix(arg(row, 0).vector()), euler_matrix(arg(row, 1).vector()));
            Scalar::Vector(matrix_euler(if *local { rotation * by } else { by * rotation }))
        }
        Function::RotateVector => Scalar::Vector(euler_matrix(arg(row, 1).vector()) * arg(row, 0).vector()),
        Function::AxisAngle => Scalar::Vector(matrix_euler(axis_angle_matrix(arg(row, 0).vector(), arg(row, 1).float()))),
        Function::InvertRotation => Scalar::Vector(matrix_euler(euler_matrix(arg(row, 0).vector()).transpose())),
        Function::Euler => Scalar::Vector(matrix_euler(euler_matrix(arg(row, 0).vector()))),
    }
}

/// The rotation in a transform's 3 × 3 part: its axes made unit length, then turned into a
/// quaternion (from the largest of the trace and the diagonal) and back, so a sheared transform
/// gives the rotation the reference reads from it.
pub fn rotation_part(matrix: Mat3) -> Mat3 {
    let [x, y, z] = [matrix.x_axis.normalize_or_zero(), matrix.y_axis.normalize_or_zero(), matrix.z_axis.normalize_or_zero()];
    // m[row][column]
    let (m00, m10, m20) = (x.x, x.y, x.z);
    let (m01, m11, m21) = (y.x, y.y, y.z);
    let (m02, m12, m22) = (z.x, z.y, z.z);
    let trace = m00 + m11 + m22;
    let quaternion = if trace > 0.0 {
        let w = 0.5 * (1.0 + trace).sqrt();
        glam::Quat::from_xyzw((m21 - m12) / (4.0 * w), (m02 - m20) / (4.0 * w), (m10 - m01) / (4.0 * w), w)
    } else if m00 >= m11 && m00 >= m22 {
        let s = 2.0 * (1.0 + m00 - m11 - m22).max(0.0).sqrt();
        glam::Quat::from_xyzw(s / 4.0, (m01 + m10) / s, (m02 + m20) / s, (m21 - m12) / s)
    } else if m11 >= m22 {
        let s = 2.0 * (1.0 + m11 - m00 - m22).max(0.0).sqrt();
        glam::Quat::from_xyzw((m01 + m10) / s, s / 4.0, (m12 + m21) / s, (m02 - m20) / s)
    } else {
        let s = 2.0 * (1.0 + m22 - m00 - m11).max(0.0).sqrt();
        glam::Quat::from_xyzw((m02 + m20) / s, (m12 + m21) / s, s / 4.0, (m10 - m01) / s)
    };
    if quaternion.is_finite() && quaternion.length_squared() > 0.0 { Mat3::from_quat(quaternion.normalize()) } else { Mat3::IDENTITY }
}

/// The rotation by `angle` about `axis` (none about a zero axis).
fn axis_angle_matrix(axis: Vec3, angle: f32) -> Mat3 {
    let axis = axis.normalize_or_zero();
    if axis == Vec3::ZERO || !angle.is_finite() { Mat3::IDENTITY } else { Mat3::from_axis_angle(axis, angle) }
}

/// Accumulate Field on its own domain, moved to the context's.
fn accumulated(accumulate: &Accumulate, context: &Context<'_>) -> Column {
    let own = Context { component: context.component, domain: accumulate.domain };
    let values = evaluate(&accumulate.value, &own);
    let groups = evaluate(&accumulate.group, &own);
    let zero = Scalar::zero(accumulate.kind);
    let add = |a: Scalar, b: Scalar| match accumulate.kind {
        Kind::Int => Scalar::Int(a.int().wrapping_add(b.int())),
        Kind::Vector => Scalar::Vector(a.vector() + b.vector()),
        _ => Scalar::Float(a.float() + b.float()),
    };
    let mut running: std::collections::HashMap<i32, Scalar> = std::collections::HashMap::new();
    let mut leading = Vec::with_capacity(values.len());
    let mut trailing = Vec::with_capacity(values.len());
    for (value, group) in values.iter().zip(&groups) {
        let value = value.to(accumulate.kind);
        let total = running.entry(group.int()).or_insert(zero);
        trailing.push(*total);
        *total = add(*total, value);
        leading.push(*total);
    }
    let column = match accumulate.which {
        0 => leading,
        1 => trailing,
        _ => groups.iter().map(|group| running.get(&group.int()).copied().unwrap_or(zero)).collect(),
    };
    context.component.adapt(&column, accumulate.domain, context.domain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn math_covers_the_edges() {
        assert_eq!(math("divide", 1.0, 0.0, 0.0), 0.0);
        assert_eq!(math("sqrt", -4.0, 0.0, 0.0), 0.0);
        assert_eq!(math("floored_modulo", -1.0, 3.0, 0.0), 2.0);
        assert_eq!(math("wrap", 5.0, 3.0, 1.0), 1.0);
        assert!((math("pingpong", 3.0, 2.0, 0.0) - 1.0).abs() < 1e-6);
        assert_eq!(math("multiply_add", 2.0, 3.0, 4.0), 10.0);
    }

    #[test]
    fn conversions_follow_the_reference_rules() {
        assert_eq!(Scalar::Float(2.6).int(), 3);
        assert!(!Scalar::Float(0.0).bool());
        assert_eq!(Scalar::Vector(Vec3::new(1.0, 2.0, 3.0)).float(), 2.0);
        assert_eq!(Scalar::Float(2.0).vector(), Vec3::splat(2.0));
    }

    #[test]
    fn map_range_clamps_and_steps() {
        assert_eq!(map_range(2.0, 0.0, 1.0, 0.0, 10.0, 4.0, "linear", true), 10.0);
        assert_eq!(map_range(2.0, 0.0, 1.0, 0.0, 10.0, 4.0, "linear", false), 20.0);
        assert_eq!(map_range(0.3, 0.0, 1.0, 0.0, 1.0, 4.0, "stepped", true), 0.25);
    }
}
