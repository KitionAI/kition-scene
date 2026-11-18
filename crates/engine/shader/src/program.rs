//! A shader group compiled for evaluation: the nodes the Material Output's Surface reaches, in an
//! order where each comes after what it reads, with settings parsed and images decoded once.
//! Values flow as numbers, vectors, colours, and surfaces; a link converts between the first
//! three as the reference does (a colour read as a number is its luminance, a vector its
//! average).

use std::collections::BTreeMap;
use std::sync::Arc;

use glam::{Mat3, Vec2, Vec3, Vec4};
use kition_scene_compositor::color::{self as compositor_color, Blend};
use kition_scene_model::Scene;
use kition_scene_model::node_tree::{self, Implicit, Node, NodeGroup, NodeKind, SocketDefault, SocketType, Tree};
use kition_scene_nodes::field::{map_range, math, vector_math};
use serde_json::Value;

use crate::closure::{Closure, Lobe, Phase, Principled, VolumeLobe};
use crate::noise::{Domain, Fractal, noise_texture};
use crate::textures::{self, Extension, Picture, Pictures, Wave};
use crate::voronoi::{self, Dims, Metric};
use crate::{Point, Space};

/// A value between nodes.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Val {
    None,
    F(f32),
    V(Vec3),
    C(Vec4),
    /// A surface: an index into the run's closures.
    S(usize),
}

const LUMINANCE: Vec3 = Vec3::new(0.2126, 0.7152, 0.0722);

impl Val {
    fn f(self) -> f32 {
        match self {
            Self::F(value) => value,
            Self::V(value) => (value.x + value.y + value.z) / 3.0,
            Self::C(value) => value.truncate().dot(LUMINANCE),
            _ => 0.0,
        }
    }

    fn v(self) -> Vec3 {
        match self {
            Self::F(value) => Vec3::splat(value),
            Self::V(value) => value,
            Self::C(value) => value.truncate(),
            _ => Vec3::ZERO,
        }
    }

    fn c(self) -> Vec4 {
        match self {
            Self::F(value) => Vec4::new(value, value, value, 1.0),
            Self::V(value) => value.extend(1.0),
            Self::C(value) => value,
            _ => Vec4::new(0.0, 0.0, 0.0, 1.0),
        }
    }
}

/// Where an input's value comes from.
#[derive(Debug, Clone, PartialEq)]
enum Src {
    Const(Val),
    /// Another node's output slot.
    Slot(usize),
    /// A property of the point, when nothing links into the input.
    Implicit(Implicit),
}

/// A node with its settings parsed.
#[derive(Debug)]
enum Op {
    Output,
    Value,
    Rgb,
    MixColor {
        blend: Blend,
        clamp_factor: bool,
        clamp_result: bool,
    },
    Invert,
    Math {
        op: String,
        clamp: bool,
    },
    VectorMath {
        op: String,
    },
    SeparateXyz,
    CombineXyz,
    MapRange {
        vector: bool,
        interpolation: String,
        clamp: bool,
    },
    TextureCoordinate,
    GeometryInfo,
    Fresnel,
    Principled,
    Diffuse,
    Glossy,
    Glass,
    Transparent,
    Emission,
    VolumeAbsorption,
    VolumeScatter {
        rayleigh: bool,
    },
    PrincipledVolume,
    MixShader,
    AddShader,
    Image {
        picture: Option<Arc<Picture>>,
        extension: Extension,
        closest: bool,
    },
    Noise {
        dims: String,
        kind: String,
        normalize: bool,
    },
    Voronoi {
        dims: Dims,
        distance: String,
        feature: String,
        normalize: bool,
    },
    Gradient {
        kind: String,
    },
    Checker,
    Wave {
        kind: String,
        bands: String,
        rings: String,
        profile: String,
    },
    ColorRamp {
        stops: Vec<(f32, Vec4)>,
        interpolation: String,
    },
    SeparateColor {
        mode: String,
    },
    CombineColor {
        mode: String,
    },
    Mapping {
        kind: String,
    },
    Bump {
        invert: bool,
    },
    NormalMap {
        space: String,
    },
    /// A kind the shader tree does not take: zero outputs.
    Unknown,
}

#[derive(Debug)]
struct Step {
    op: Op,
    /// One per input socket of the kind, in order.
    inputs: Vec<Src>,
    /// The first of the node's output slots (one per output socket of the kind).
    base: usize,
    /// For Bump: the steps its Height reads, in order, to run again at nearby points.
    height: Vec<usize>,
}

/// A compiled shader group.
#[derive(Debug)]
pub struct Program {
    steps: Vec<Step>,
    slots: usize,
    surface: Option<Src>,
    varying: bool,
    /// The Material Output's Volume.
    volume: Option<Src>,
    volume_varying: bool,
}

fn json_val(value: &Value, kind: SocketType) -> Val {
    let number = |value: &Value| value.as_f64().map(|number| number as f32);
    let array = |value: &Value| value.as_array().map(|items| items.iter().map(|item| number(item).unwrap_or(0.0)).collect::<Vec<f32>>());
    match kind {
        SocketType::Float | SocketType::Int => Val::F(number(value).or_else(|| value.as_bool().map(f32::from)).unwrap_or(0.0)),
        SocketType::Bool => Val::F(value.as_bool().map_or_else(|| number(value).unwrap_or(0.0), f32::from)),
        SocketType::Vector | SocketType::Rotation => match array(value).as_deref() {
            Some([x, y, z, ..]) => Val::V(Vec3::new(*x, *y, *z)),
            _ => Val::V(Vec3::ZERO),
        },
        SocketType::Color => match array(value).as_deref() {
            Some([r, g, b, a]) => Val::C(Vec4::new(*r, *g, *b, *a)),
            Some([r, g, b]) => Val::C(Vec4::new(*r, *g, *b, 1.0)),
            _ => Val::C(Vec4::new(0.0, 0.0, 0.0, 1.0)),
        },
        _ => Val::None,
    }
}

fn default_val(default: SocketDefault) -> Val {
    match default {
        SocketDefault::Float(value) => Val::F(value),
        SocketDefault::Int(value) => Val::F(value as f32),
        SocketDefault::Bool(value) => Val::F(f32::from(value)),
        SocketDefault::Vector(value) => Val::V(Vec3::from(value)),
        SocketDefault::Color(value) => Val::C(Vec4::from(value)),
        SocketDefault::None | SocketDefault::Str(_) => Val::None,
    }
}

fn ramp(node: &Node) -> Vec<(f32, Vec4)> {
    let stops: Vec<(f32, Vec4)> = node
        .values
        .get("ramp")
        .and_then(|ramp| ramp.get("elements"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|element| {
            let [position, color] = element.as_array()?.as_slice() else { return None };
            let channels: Vec<f32> = color.as_array()?.iter().filter_map(Value::as_f64).map(|value| value as f32).collect();
            let color = match channels.as_slice() {
                [r, g, b] => Vec4::new(*r, *g, *b, 1.0),
                [r, g, b, a, ..] => Vec4::new(*r, *g, *b, *a),
                _ => return None,
            };
            Some((position.as_f64()? as f32, color))
        })
        .collect();
    if stops.is_empty() { vec![(0.0, Vec4::new(0.0, 0.0, 0.0, 1.0)), (1.0, Vec4::ONE)] } else { stops }
}

fn op_of(scene: &Scene, node: &Node, kind: &NodeKind, pictures: &mut Pictures) -> Op {
    let setting = |id: &str| node_tree::setting(node, kind, id).to_owned();
    let flag = |id: &str| node_tree::setting(node, kind, id) == "true";
    match node.kind.as_str() {
        "materialOutput" => Op::Output,
        "value" => Op::Value,
        "rgb" => Op::Rgb,
        "mixColor" => {
            Op::MixColor { blend: Blend::parse(&setting("blendType")), clamp_factor: flag("clampFactor"), clamp_result: flag("clampResult") }
        }
        "invert" => Op::Invert,
        "math" => Op::Math { op: setting("operation"), clamp: flag("useClamp") },
        "vectorMath" => Op::VectorMath { op: setting("operation") },
        "separateXyz" => Op::SeparateXyz,
        "combineXyz" => Op::CombineXyz,
        "mapRange" => {
            Op::MapRange { vector: setting("dataType") == "float_vector", interpolation: setting("interpolationType"), clamp: flag("clamp") }
        }
        "textureCoordinate" => Op::TextureCoordinate,
        "geometryInfo" => Op::GeometryInfo,
        "fresnel" => Op::Fresnel,
        "principledBsdf" => Op::Principled,
        "diffuseBsdf" => Op::Diffuse,
        "glossyBsdf" => Op::Glossy,
        "glassBsdf" => Op::Glass,
        "transparentBsdf" => Op::Transparent,
        "emission" => Op::Emission,
        "volumeAbsorption" => Op::VolumeAbsorption,
        "volumeScatter" => Op::VolumeScatter { rayleigh: setting("phase") == "rayleigh" },
        "principledVolume" => Op::PrincipledVolume,
        "mixShader" => Op::MixShader,
        "addShader" => Op::AddShader,
        "imageTexture" => {
            let srgb = setting("colorSpace") != "non_color";
            let picture =
                node.values.get("Image").and_then(Value::as_str).filter(|hash| !hash.is_empty()).and_then(|hash| pictures.get(scene, hash, srgb));
            Op::Image { picture, extension: Extension::parse(&setting("extension")), closest: setting("interpolation") == "closest" }
        }
        "noiseTexture" => Op::Noise { dims: setting("noiseDimensions"), kind: setting("noiseType"), normalize: flag("normalize") },
        "voronoiTexture" => Op::Voronoi {
            dims: Dims::parse(&setting("voronoiDimensions")),
            distance: setting("distance"),
            feature: setting("feature"),
            normalize: flag("normalize"),
        },
        "gradientTexture" => Op::Gradient { kind: setting("gradientType") },
        "checkerTexture" => Op::Checker,
        "waveTexture" => Op::Wave {
            kind: setting("waveType"),
            bands: setting("bandsDirection"),
            rings: setting("ringsDirection"),
            profile: setting("waveProfile"),
        },
        "colorRamp" => {
            let mut stops = ramp(node);
            stops.sort_by(|a, b| a.0.total_cmp(&b.0));
            Op::ColorRamp { stops, interpolation: setting("interpolation") }
        }
        "separateColor" => Op::SeparateColor { mode: setting("mode") },
        "combineColor" => Op::CombineColor { mode: setting("mode") },
        "mapping" => Op::Mapping { kind: setting("vectorType") },
        "bump" => Op::Bump { invert: flag("invert") },
        "normalMap" => Op::NormalMap { space: setting("space") },
        _ => Op::Unknown,
    }
}

/// Nodes that read the point themselves (beyond their inputs).
fn reads_point(op: &Op) -> bool {
    matches!(op, Op::TextureCoordinate | Op::GeometryInfo | Op::Fresnel | Op::Bump { .. } | Op::NormalMap { .. })
}

/// Nodes that make surfaces: their Normal inputs follow the point without making the surface's
/// values vary.
fn makes_surface(op: &Op) -> bool {
    matches!(op, Op::Principled | Op::Diffuse | Op::Glossy | Op::Glass)
}

impl Program {
    /// Compiles a shader group of `scene` (its images decoded through `pictures`). A group whose
    /// Material Output has nothing in its Surface gives a black surface.
    pub fn new(scene: &Scene, group: &NodeGroup, pictures: &mut Pictures) -> Self {
        let empty = Self { steps: Vec::new(), slots: 0, surface: None, varying: false, volume: None, volume_varying: false };
        if group.tree != Tree::Shader {
            return empty;
        }
        // The last Material Output is the one that counts.
        let Some(output) = group.nodes.iter().rev().find(|node| node.kind == "materialOutput") else { return empty };
        let mut incoming: BTreeMap<(u32, &str), (u32, &str)> = BTreeMap::new();
        for link in &group.links {
            incoming.insert((link.to, link.to_socket.as_str()), (link.from, link.from_socket.as_str()));
        }
        // Depth-first from the output: each node after the nodes it reads.
        let mut order: Vec<u32> = Vec::new();
        let mut state: BTreeMap<u32, bool> = BTreeMap::new();
        fn visit(id: u32, group: &NodeGroup, incoming: &BTreeMap<(u32, &str), (u32, &str)>, state: &mut BTreeMap<u32, bool>, order: &mut Vec<u32>) {
            match state.get(&id) {
                Some(_) => return,
                None => {
                    state.insert(id, false);
                }
            }
            for ((to, _), (from, _)) in incoming.range((id, "")..) {
                if *to != id {
                    break;
                }
                if group.node(*from).is_some() && state.get(from) != Some(&false) {
                    visit(*from, group, incoming, state, order);
                }
            }
            state.insert(id, true);
            order.push(id);
        }
        visit(output.id, group, &incoming, &mut state, &mut order);
        // Slots: each node's outputs in turn.
        let mut base_of: BTreeMap<u32, usize> = BTreeMap::new();
        let mut slots = 0;
        for id in &order {
            let Some(kind) = group.node(*id).and_then(|node| node_tree::node_kind(&node.kind)) else { continue };
            base_of.insert(*id, slots);
            slots += kind.outputs.len();
        }
        let slot_of = |from: u32, socket: &str| -> Option<usize> {
            let node = group.node(from)?;
            let kind = node_tree::node_kind(&node.kind)?;
            let index = kind.outputs.iter().position(|item| item.id == socket)?;
            Some(base_of.get(&from)? + index)
        };
        let mut steps = Vec::new();
        let mut step_of: BTreeMap<u32, usize> = BTreeMap::new();
        let mut varying_slots = vec![false; slots];
        for id in &order {
            let Some(node) = group.node(*id) else { continue };
            let Some(kind) = node_tree::node_kind(&node.kind) else { continue };
            let op = if node_tree::in_tree(&node.kind, Tree::Shader) { op_of(scene, node, kind, pictures) } else { Op::Unknown };
            let mut varying = reads_point(&op);
            let inputs: Vec<Src> = kind
                .inputs
                .iter()
                .map(|socket| {
                    if let Some((from, from_socket)) = incoming.get(&(node.id, socket.id))
                        && let Some(slot) = slot_of(*from, from_socket)
                    {
                        varying |= varying_slots.get(slot).copied().unwrap_or(false);
                        return Src::Slot(slot);
                    }
                    if let Some(value) = node.values.get(socket.id) {
                        return Src::Const(json_val(value, socket.kind));
                    }
                    match socket.implicit {
                        Some(implicit) => {
                            varying |= !(implicit == Implicit::Normal && makes_surface(&op));
                            Src::Implicit(implicit)
                        }
                        None => Src::Const(default_val(socket.default)),
                    }
                })
                .collect();
            let base = base_of.get(id).copied().unwrap_or(0);
            for index in 0..kind.outputs.len() {
                if let Some(flag) = varying_slots.get_mut(base + index) {
                    *flag = varying;
                }
            }
            step_of.insert(*id, steps.len());
            steps.push(Step { op, inputs, base, height: Vec::new() });
        }
        // Bump nodes run what their Height reads again at nearby points.
        for (id, &index) in &step_of {
            if !matches!(steps.get(index).map(|step| &step.op), Some(Op::Bump { .. })) {
                continue;
            }
            let mut needed: Vec<usize> = Vec::new();
            let mut stack: Vec<u32> = incoming.get(&(*id, "Height")).map(|(from, _)| *from).into_iter().collect();
            while let Some(at) = stack.pop() {
                let Some(&step) = step_of.get(&at) else { continue };
                if needed.contains(&step) {
                    continue;
                }
                needed.push(step);
                for ((to, _), (from, _)) in incoming.range((at, "")..) {
                    if *to != at {
                        break;
                    }
                    stack.push(*from);
                }
            }
            needed.sort_unstable();
            if let Some(step) = steps.get_mut(index) {
                step.height = needed;
            }
        }
        let input = |socket: &str| incoming.get(&(output.id, socket)).and_then(|(from, from_socket)| slot_of(*from, from_socket)).map(Src::Slot);
        let varies = |src: &Option<Src>| match src {
            Some(Src::Slot(slot)) => varying_slots.get(*slot).copied().unwrap_or(false),
            _ => false,
        };
        let (surface, volume) = (input("Surface"), input("Volume"));
        let (varying, volume_varying) = (varies(&surface), varies(&volume));
        Self { steps, slots, surface, varying, volume, volume_varying }
    }

    /// Whether something links into the Material Output's Surface (a material with only a
    /// volume has no surface: light passes its boundary unchanged).
    pub fn has_surface(&self) -> bool {
        self.surface.is_some()
    }

    /// Whether something links into the Material Output's Volume.
    pub fn has_volume(&self) -> bool {
        self.volume.is_some()
    }

    /// Whether the volume's values change from point to point.
    pub fn volume_varying(&self) -> bool {
        self.volume_varying
    }

    /// The volume at a point (`point.normal` is unused there; the incoming direction is the
    /// ray's): its lobes and the light it sends out per metre.
    pub fn eval_volume(&self, point: &Point, space: &Space) -> Closure {
        self.eval_output(self.volume.as_ref(), point, space)
    }

    /// Whether the surface's values change from point to point (textures, coordinates, bumps).
    pub fn varying(&self) -> bool {
        self.varying
    }

    /// The surface at a point.
    pub fn eval(&self, point: &Point, space: &Space) -> Closure {
        self.eval_output(self.surface.as_ref(), point, space)
    }

    fn eval_output(&self, output: Option<&Src>, point: &Point, space: &Space) -> Closure {
        let Some(Src::Slot(surface)) = output else { return Closure::default() };
        let mut slots = vec![Val::None; self.slots];
        let mut closures: Vec<Closure> = Vec::new();
        for index in 0..self.steps.len() {
            self.run(index, point, space, &mut slots, &mut closures);
        }
        match slots.get(*surface) {
            Some(Val::S(index)) => closures.get(*index).cloned().unwrap_or_default(),
            _ => Closure::default(),
        }
    }

    fn read(&self, src: &Src, point: &Point, space: &Space, slots: &[Val]) -> Val {
        match src {
            Src::Const(value) => *value,
            Src::Slot(slot) => slots.get(*slot).copied().unwrap_or(Val::None),
            Src::Implicit(implicit) => match implicit {
                Implicit::Normal => Val::V(point.normal),
                Implicit::Generated => Val::V(space.generated(point.position)),
                Implicit::Uv => Val::V(point.uv.extend(0.0)),
                Implicit::Position => Val::V(point.position),
                Implicit::Index => Val::F(0.0),
            },
        }
    }

    fn run(&self, index: usize, point: &Point, space: &Space, slots: &mut [Val], closures: &mut Vec<Closure>) {
        let Some(step) = self.steps.get(index) else { return };
        let input = |at: usize| step.inputs.get(at).map_or(Val::None, |src| self.read(src, point, space, slots));
        let f = |at: usize| input(at).f();
        let v = |at: usize| input(at).v();
        let c = |at: usize| input(at).c();
        let unit = |at: usize| {
            let n = v(at).normalize_or_zero();
            if n == Vec3::ZERO { point.normal } else { n }
        };
        let shader = |at: usize, closures: &[Closure]| match input(at) {
            Val::S(index) => closures.get(index).cloned().unwrap_or_default(),
            _ => Closure::default(),
        };
        let mut outputs: Vec<Val> = Vec::new();
        match &step.op {
            Op::Output | Op::Unknown => {}
            Op::Value => outputs.push(Val::F(f(0))),
            Op::Rgb => outputs.push(Val::C(c(0))),
            Op::MixColor { blend, clamp_factor, clamp_result } => {
                let factor = if *clamp_factor { f(0).clamp(0.0, 1.0) } else { f(0) };
                let mixed = compositor_color::mix(*blend, factor, c(1).to_array(), c(2).to_array());
                let mut out = Vec4::from(mixed);
                if *clamp_result {
                    out = out.clamp(Vec4::ZERO, Vec4::ONE);
                }
                outputs.push(Val::C(out));
            }
            Op::Invert => {
                let color = c(0).to_array();
                outputs.push(Val::C(Vec4::from(compositor_color::invert(color, f(1), f(2) > 0.5, f(3) > 0.5))));
            }
            Op::Math { op, clamp } => {
                let value = math(op, f(0), f(1), f(2));
                outputs.push(Val::F(if *clamp { value.clamp(0.0, 1.0) } else { value }));
            }
            Op::VectorMath { op } => {
                let out = vector_math(op, v(0), v(1), v(2), f(3));
                outputs.push(Val::V(out));
                outputs.push(Val::F(out.x));
            }
            Op::SeparateXyz => {
                let value = v(0);
                outputs.extend([Val::F(value.x), Val::F(value.y), Val::F(value.z)]);
            }
            Op::CombineXyz => outputs.push(Val::V(Vec3::new(f(0), f(1), f(2)))),
            Op::MapRange { vector, interpolation, clamp } => {
                if *vector {
                    let (value, from_min, from_max, to_min, to_max) = (v(6), v(7), v(8), v(9), v(10));
                    let axis = |a: usize| map_range(value[a], from_min[a], from_max[a], to_min[a], to_max[a], f(5), interpolation, *clamp);
                    outputs.push(Val::F(0.0));
                    outputs.push(Val::V(Vec3::new(axis(0), axis(1), axis(2))));
                } else {
                    outputs.push(Val::F(map_range(f(0), f(1), f(2), f(3), f(4), f(5), interpolation, *clamp)));
                    outputs.push(Val::V(Vec3::ZERO));
                }
            }
            Op::TextureCoordinate => {
                let reflection = (2.0 * point.normal.dot(point.incoming) * point.normal - point.incoming).normalize_or_zero();
                outputs.extend([
                    Val::V(space.generated(point.position)),
                    Val::V(space.object_normal(point.normal)),
                    Val::V(point.uv.extend(0.0)),
                    Val::V(space.object(point.position)),
                    Val::V(reflection),
                ]);
            }
            Op::GeometryInfo => outputs.extend([
                Val::V(point.position),
                Val::V(point.normal),
                Val::V(point.tangent_axis()),
                Val::V(point.true_normal),
                Val::V(point.incoming),
                Val::F(f32::from(point.backfacing)),
            ]),
            Op::Fresnel => {
                let ior = f(0).max(1e-5);
                let eta = if point.backfacing { 1.0 / ior } else { ior };
                outputs.push(Val::F(fresnel(unit(1).dot(point.incoming), eta)));
            }
            Op::Principled => {
                let p = Principled {
                    base: c(0).truncate(),
                    metallic: f(1).clamp(0.0, 1.0),
                    roughness: f(2).clamp(0.0, 1.0),
                    ior: f(3),
                    normal: unit(5),
                    subsurface: f(6).clamp(0.0, 1.0),
                    subsurface_radius: v(7),
                    subsurface_scale: f(8),
                    specular_level: f(9).max(0.0),
                    specular_tint: c(10).truncate(),
                    anisotropic: f(11).clamp(0.0, 1.0),
                    anisotropic_rotation: f(12),
                    tangent: {
                        let t = v(13);
                        if t == Vec3::ZERO { point.tangent_axis() } else { t.normalize_or_zero() }
                    },
                    transmission: f(14).clamp(0.0, 1.0),
                    coat: f(15).clamp(0.0, 1.0),
                    coat_roughness: f(16).clamp(0.0, 1.0),
                    coat_ior: f(17),
                    coat_tint: c(18).truncate(),
                    coat_normal: unit(19),
                    sheen: f(20).max(0.0),
                    sheen_roughness: f(21).clamp(0.0, 1.0),
                    sheen_tint: c(22).truncate(),
                };
                let alpha = f(4).clamp(0.0, 1.0);
                let emission = c(23).truncate() * f(24);
                let closure = Closure { lobes: vec![(1.0, Lobe::Principled(p))], emission, ..Closure::default() };
                let mut closure = closure.scaled(alpha);
                closure.transparent += Vec3::splat(1.0 - alpha);
                outputs.push(push(closures, closure));
            }
            Op::Diffuse => {
                let lobe = Lobe::Diffuse { color: c(0).truncate(), roughness: f(1), normal: unit(2) };
                outputs.push(push(closures, Closure::lobe(lobe)));
            }
            Op::Glossy => {
                let tangent = {
                    let t = v(5);
                    if t == Vec3::ZERO { point.tangent_axis() } else { t.normalize_or_zero() }
                };
                let lobe = Lobe::Glossy {
                    color: c(0).truncate(),
                    roughness: f(1).clamp(0.0, 1.0),
                    anisotropy: f(2).clamp(-1.0, 1.0),
                    rotation: f(3),
                    normal: unit(4),
                    tangent,
                };
                outputs.push(push(closures, Closure::lobe(lobe)));
            }
            Op::Glass => {
                let lobe = Lobe::Glass { color: c(0).truncate(), roughness: f(1).clamp(0.0, 1.0), ior: f(2).max(1e-3), normal: unit(3) };
                outputs.push(push(closures, Closure::lobe(lobe)));
            }
            Op::Transparent => outputs.push(push(closures, Closure { transparent: c(0).truncate(), ..Closure::default() })),
            Op::Emission => outputs.push(push(closures, Closure { emission: c(0).truncate() * f(1), ..Closure::default() })),
            // Volumes, per metre and unit density as the reference's nodes measure: absorption
            // takes away one less the colour, scattering the colour itself.
            Op::VolumeAbsorption => {
                let density = f(1).max(0.0);
                let absorption = (Vec3::ONE - c(0).truncate().clamp(Vec3::ZERO, Vec3::ONE)) * density;
                let lobe = VolumeLobe { absorption, scattering: Vec3::ZERO, phase: Phase::HenyeyGreenstein(0.0) };
                outputs.push(push(closures, Closure { volume: vec![lobe], ..Closure::default() }));
            }
            Op::VolumeScatter { rayleigh } => {
                let density = f(1).max(0.0);
                let scattering = c(0).truncate().clamp(Vec3::ZERO, Vec3::ONE) * density;
                let phase = if *rayleigh { Phase::Rayleigh } else { Phase::HenyeyGreenstein(f(2).clamp(-0.999, 0.999)) };
                outputs
                    .push(push(closures, Closure { volume: vec![VolumeLobe { absorption: Vec3::ZERO, scattering, phase }], ..Closure::default() }));
            }
            Op::PrincipledVolume => {
                // Scattering is the colour; what is not scattered is absorbed, less as the
                // absorption colour's square root rises (measured: one less √colour).
                let density = f(1).max(0.0);
                let color = c(0).truncate().clamp(Vec3::ZERO, Vec3::ONE);
                let absorption_color = c(3).truncate().clamp(Vec3::ZERO, Vec3::ONE);
                let absorbed =
                    (Vec3::ONE - color) * (Vec3::ONE - Vec3::new(absorption_color.x.sqrt(), absorption_color.y.sqrt(), absorption_color.z.sqrt()));
                let lobe = VolumeLobe {
                    absorption: absorbed * density,
                    scattering: color * density,
                    phase: Phase::HenyeyGreenstein(f(2).clamp(-0.999, 0.999)),
                };
                let emission = c(5).truncate().max(Vec3::ZERO) * f(4).max(0.0);
                outputs.push(push(closures, Closure { volume: vec![lobe], emission, ..Closure::default() }));
            }
            Op::MixShader => {
                let factor = f(0).clamp(0.0, 1.0);
                let mixed = shader(1, closures).scaled(1.0 - factor) + shader(2, closures).scaled(factor);
                outputs.push(push(closures, mixed));
            }
            Op::AddShader => {
                let sum = shader(0, closures) + shader(1, closures);
                outputs.push(push(closures, sum));
            }
            Op::Image { picture, extension, closest } => {
                let uv = v(0).truncate();
                let color = picture.as_ref().map_or(Vec4::new(1.0, 0.0, 1.0, 1.0), |picture| picture.sample(uv, *extension, *closest));
                outputs.push(Val::C(color.truncate().extend(1.0)));
                outputs.push(Val::F(color.w));
            }
            Op::Noise { dims, kind, normalize } => {
                let fractal = Fractal { detail: f(2), roughness: f(3), lacunarity: f(4), offset: f(5), gain: f(6) };
                let p = Domain::of(dims, v(0) * f(1));
                let (value, color) = noise_texture(p, kind, &fractal, f(7), *normalize);
                outputs.push(Val::F(value));
                outputs.push(Val::C(color.extend(1.0)));
            }
            Op::Voronoi { dims, distance, feature, normalize } => {
                let scale = f(1);
                let p = v(0) * scale;
                let settings = voronoi::Settings {
                    dims: *dims,
                    metric: Metric::parse(distance, f(6)),
                    randomness: f(7).clamp(0.0, 1.0),
                    detail: f(2),
                    roughness: f(3),
                    lacunarity: f(4),
                    smoothness: f(5),
                    normalize: *normalize,
                };
                match feature.as_str() {
                    "distance_to_edge" => {
                        outputs.extend([Val::F(voronoi::edge_distance(*dims, p, settings.randomness)), Val::None, Val::None, Val::F(0.0)])
                    }
                    "n_sphere_radius" => {
                        outputs.extend([Val::F(0.0), Val::None, Val::None, Val::F(voronoi::sphere_radius(*dims, p, settings.randomness))])
                    }
                    _ => {
                        let found = voronoi::texture(feature, p, &settings);
                        let position = if scale != 0.0 { found.position / scale } else { Vec3::ZERO };
                        outputs.extend([Val::F(found.distance), Val::C(found.color.extend(1.0)), Val::V(position), Val::F(0.0)]);
                    }
                }
            }
            Op::Gradient { kind } => {
                let value = textures::gradient(kind, v(0)).clamp(0.0, 1.0);
                outputs.push(Val::C(Vec4::new(value, value, value, 1.0)));
                outputs.push(Val::F(value));
            }
            Op::Checker => {
                let first = textures::checker(v(0) * f(3));
                outputs.push(Val::C(if first { c(1) } else { c(2) }));
                outputs.push(Val::F(f32::from(first)));
            }
            Op::Wave { kind, bands, rings, profile } => {
                let wave =
                    Wave { kind, bands, rings, profile, distortion: f(2), detail: f(3), detail_scale: f(4), detail_roughness: f(5), phase: f(6) };
                let value = textures::wave(&wave, v(0) * f(1));
                outputs.push(Val::C(Vec4::new(value, value, value, 1.0)));
                outputs.push(Val::F(value));
            }
            Op::ColorRamp { stops, interpolation } => {
                let color = ramp_at(stops, interpolation, f(0));
                outputs.push(Val::C(color));
                outputs.push(Val::F(color.w));
            }
            Op::SeparateColor { mode } => {
                let [a, b, d] = textures::to_model(mode, c(0).truncate().to_array());
                outputs.extend([Val::F(a), Val::F(b), Val::F(d)]);
            }
            Op::CombineColor { mode } => {
                let rgb = textures::from_model(mode, [f(0), f(1), f(2)]);
                outputs.push(Val::C(Vec3::from(rgb).extend(1.0)));
            }
            Op::Mapping { kind } => outputs.push(Val::V(mapping(kind, v(0), v(1), v(2), v(3)))),
            Op::Bump { invert } => {
                let normal = unit(3);
                let height = f(2);
                let distance = if *invert { -f(1) } else { f(1) };
                let strength = f(0).max(0.0);
                let bent = if step.height.is_empty() || distance == 0.0 {
                    normal
                } else {
                    let t1 = {
                        let along = point.dpdu - normal * normal.dot(point.dpdu);
                        let along = along.normalize_or_zero();
                        if along != Vec3::ZERO { along } else { point.tangent_axis() }
                    };
                    let t2 = normal.cross(t1);
                    let epsilon = BUMP_STEP * point.position.abs().max_element().max(1.0);
                    let at = |offset: Vec3| {
                        let moved = point.moved(offset);
                        let mut scratch = slots.to_vec();
                        let mut scratch_closures = Vec::new();
                        for dependency in &step.height {
                            self.run(*dependency, &moved, space, &mut scratch, &mut scratch_closures);
                        }
                        step.inputs.get(2).map_or(0.0, |src| self.read(src, &moved, space, &scratch).f())
                    };
                    let gradient = t1 * ((at(t1 * epsilon) - height) / epsilon) + t2 * ((at(t2 * epsilon) - height) / epsilon);
                    (normal - gradient * distance).normalize_or_zero()
                };
                let out = (normal * (1.0 - strength.min(1.0)) + bent * strength.min(1.0)).normalize_or_zero();
                outputs.push(Val::V(if out == Vec3::ZERO { normal } else { out }));
            }
            Op::NormalMap { space: kind } => {
                let strength = f(0).max(0.0);
                let color = c(1).truncate() * 2.0 - Vec3::ONE;
                let normal = point.normal;
                let mapped = match kind.as_str() {
                    "object" => space.world_normal(color),
                    "world" => color.normalize_or_zero(),
                    _ => {
                        let tangent = point.tangent.truncate();
                        let tangent = (tangent - normal * normal.dot(tangent)).normalize_or_zero();
                        if tangent == Vec3::ZERO {
                            normal
                        } else {
                            let bitangent = normal.cross(tangent) * if point.tangent.w < 0.0 { -1.0 } else { 1.0 };
                            (tangent * color.x + bitangent * color.y + normal * color.z).normalize_or_zero()
                        }
                    }
                };
                let out = (normal + (mapped - normal) * strength).normalize_or_zero();
                outputs.push(Val::V(if out == Vec3::ZERO { normal } else { out }));
            }
        }
        for (offset, value) in outputs.into_iter().enumerate() {
            if let Some(slot) = slots.get_mut(step.base + offset) {
                *slot = value;
            }
        }
    }
}

/// How far Bump looks around a point for the height's slope, in metres.
const BUMP_STEP: f32 = 1e-3;

fn push(closures: &mut Vec<Closure>, closure: Closure) -> Val {
    closures.push(closure);
    Val::S(closures.len() - 1)
}

/// Dielectric Fresnel reflectance for the cosine on the incident side and the relative index.
pub fn fresnel(cosine: f32, eta: f32) -> f32 {
    let c = cosine.abs();
    let g2 = eta * eta - 1.0 + c * c;
    if g2 <= 0.0 {
        return 1.0;
    }
    let g = g2.sqrt();
    let a = (g - c) / (g + c);
    let b = (c * (g + c) - 1.0) / (c * (g - c) + 1.0);
    0.5 * a * a * (1.0 + b * b)
}

fn rotation(euler: Vec3) -> Mat3 {
    Mat3::from_rotation_z(euler.z) * Mat3::from_rotation_y(euler.y) * Mat3::from_rotation_x(euler.x)
}

/// The Mapping node.
fn mapping(kind: &str, vector: Vec3, location: Vec3, euler: Vec3, scale: Vec3) -> Vec3 {
    let turn = rotation(euler);
    let safe = |value: f32| if value != 0.0 { 1.0 / value } else { 0.0 };
    match kind {
        "texture" => {
            let inverse = Vec3::new(safe(scale.x), safe(scale.y), safe(scale.z));
            (turn.transpose() * (vector - location)) * inverse
        }
        "vector" => turn * (vector * scale),
        "normal" => (turn * (vector * Vec3::new(safe(scale.x), safe(scale.y), safe(scale.z)))).normalize_or_zero(),
        _ => turn * (vector * scale) + location,
    }
}

/// The Color Ramp at `at`.
fn ramp_at(stops: &[(f32, Vec4)], interpolation: &str, at: f32) -> Vec4 {
    let Some(&(first_at, first)) = stops.first() else { return Vec4::ZERO };
    if at <= first_at {
        return first;
    }
    for pair in stops.windows(2) {
        let [(a_at, a), (b_at, b)] = pair else { continue };
        if at < *b_at {
            if interpolation == "constant" {
                return *a;
            }
            let span = b_at - a_at;
            let mut t = if span > 0.0 { (at - a_at) / span } else { 1.0 };
            if interpolation == "ease" {
                t = t * t * (3.0 - 2.0 * t);
            }
            return a.lerp(*b, t);
        }
    }
    stops.last().map_or(first, |(_, last)| *last)
}

/// A point on a flat plane in world space facing +Z, for evaluating texture nodes alone.
pub fn plane_point(uv: Vec2) -> Point {
    let mut point = Point::at(Vec3::new(2.0 * uv.x - 1.0, 2.0 * uv.y - 1.0, 0.0), Vec3::Z);
    point.uv = uv;
    point.dpdu = Vec3::X * 2.0;
    point.dpdv = Vec3::Y * 2.0;
    point.tangent = Vec4::new(1.0, 0.0, 0.0, 1.0);
    point
}

#[cfg(test)]
mod tests {
    use super::*;
    use kition_scene_model::node_tree::Link;
    use serde_json::json;
    use std::f32::consts::PI;

    fn node(id: u32, kind: &str, values: Value) -> Node {
        Node {
            id,
            kind: kind.into(),
            location: [0.0, 0.0],
            values: values.as_object().map(|map| map.clone().into_iter().collect()).unwrap_or_default(),
        }
    }

    fn link(from: u32, from_socket: &str, to: u32, to_socket: &str) -> Link {
        Link { from, from_socket: from_socket.into(), to, to_socket: to_socket.into() }
    }

    fn group(nodes: Vec<Node>, links: Vec<Link>) -> NodeGroup {
        let group = NodeGroup { id: 1, name: "Shader".into(), tree: Tree::Shader, inputs: Vec::new(), outputs: Vec::new(), nodes, links };
        group.validate(&[]).unwrap();
        group
    }

    #[test]
    fn a_new_shader_group_is_the_default_principled_surface() {
        let scene = Scene::default();
        let program = Program::new(&scene, &NodeGroup::shader(1, "Shader", 2, 3), &mut Pictures::default());
        assert!(!program.varying());
        let closure = program.eval(&plane_point(Vec2::splat(0.5)), &Space::identity());
        let flat = closure.flatten();
        assert!((flat.base - Vec3::splat(0.8)).length() < 1e-6);
        assert_eq!((flat.metallic, flat.roughness, flat.alpha), (0.0, 0.5, 1.0));
        let Some((_, Lobe::Principled(p))) = closure.lobes.first() else { panic!("{closure:?}") };
        assert!((p.f0() - Vec3::splat(0.04)).length() < 1e-6);
    }

    #[test]
    fn a_checker_into_a_mix_of_diffuse_and_emission_varies() {
        let scene = Scene::default();
        let g = group(
            vec![
                node(1, "checkerTexture", json!({ "Scale": 2.0 })),
                node(2, "diffuseBsdf", json!({})),
                node(3, "emission", json!({ "Strength": 3.0 })),
                node(4, "mixShader", json!({})),
                node(5, "materialOutput", json!({})),
            ],
            vec![
                link(1, "Color", 2, "Color"),
                link(1, "Fac", 4, "Fac"),
                link(2, "BSDF", 4, "Shader"),
                link(3, "Emission", 4, "Shader_001"),
                link(4, "Shader", 5, "Surface"),
            ],
        );
        let program = Program::new(&scene, &g, &mut Pictures::default());
        assert!(program.varying());
        let first = program.eval(&plane_point(Vec2::new(0.1, 0.1)), &Space::identity());
        let second = program.eval(&plane_point(Vec2::new(0.6, 0.1)), &Space::identity());
        // Generated coordinates of the unit space over the plane: one square lit, one not.
        assert_ne!(first.emission, second.emission);
        assert!(first.emission.x == 0.0 || second.emission.x == 0.0);
        assert!(first.emission.x == 3.0 || second.emission.x == 3.0);
    }

    #[test]
    fn bump_bends_the_normal_up_a_slope() {
        let scene = Scene::default();
        // Height = the point's x: the normal tilts towards −x.
        let g = group(
            vec![
                node(1, "textureCoordinate", json!({})),
                node(2, "separateXyz", json!({})),
                node(3, "bump", json!({ "Distance": 1.0 })),
                node(4, "diffuseBsdf", json!({})),
                node(5, "materialOutput", json!({})),
            ],
            vec![link(1, "Object", 2, "Vector"), link(2, "X", 3, "Height"), link(3, "Normal", 4, "Normal"), link(4, "BSDF", 5, "Surface")],
        );
        let program = Program::new(&scene, &g, &mut Pictures::default());
        let closure = program.eval(&plane_point(Vec2::splat(0.5)), &Space::identity());
        let Some((_, Lobe::Diffuse { normal, .. })) = closure.lobes.first() else { panic!() };
        let expected = Vec3::new(-1.0, 0.0, 1.0).normalize();
        assert!((*normal - expected).length() < 1e-3, "{normal}");
    }

    #[test]
    fn color_ramps_and_mapping() {
        let stops = vec![(0.25, Vec4::new(1.0, 0.0, 0.0, 1.0)), (0.75, Vec4::new(0.0, 0.0, 1.0, 1.0))];
        assert_eq!(ramp_at(&stops, "linear", 0.0), stops[0].1);
        assert_eq!(ramp_at(&stops, "linear", 0.5), Vec4::new(0.5, 0.0, 0.5, 1.0));
        assert_eq!(ramp_at(&stops, "constant", 0.7), stops[0].1);
        assert_eq!(ramp_at(&stops, "linear", 0.9), stops[1].1);
        let p = mapping("point", Vec3::X, Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, PI / 2.0), Vec3::splat(2.0));
        assert!((p - Vec3::new(0.0, 2.0, 1.0)).length() < 1e-6);
        let back = mapping("texture", p, Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, PI / 2.0), Vec3::splat(2.0));
        assert!((back - Vec3::X).length() < 1e-6);
        assert!((fresnel(1.0, 1.5) - 0.04).abs() < 1e-6);
    }
}
