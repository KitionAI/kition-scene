//! Running a node group: each node is evaluated once, when something downstream needs one of
//! its outputs. Geometry flows as values; numbers and vectors flow as fields, which geometry
//! nodes evaluate on the domain they work on.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;
use std::sync::Arc;

use glam::{Mat3, Mat4, Vec3};
use kition_scene_model::modifiers::{NodesCall, catmull_clark};
use kition_scene_model::node_tree::{self, Implicit, Node, NodeKind, Socket, SocketDefault};
use kition_scene_model::scene::euler_matrix;
use kition_scene_model::{Id, NodeGroup, SocketType};
use serde_json::Value;

use crate::curve_ops::{self, FillRule, PointsMode, Quadrilateral};
use crate::curves::CurvesComponent;
use crate::field::{self, Column, Context, Field, Function, Kind, Scalar};
use crate::geometry::{Attribute, Component, Domain, Geometry, Instance, InstancesComponent, MAX_ELEMENTS, MeshComponent, PointsComponent};
use crate::ops::{self, Extrude, Removal};
use crate::prims::{self, Cone, Fill};

/// How deep Object Info may reach through objects whose own nodes read other objects.
pub const MAX_DEPTH: u32 = 4;

/// A value on a socket.
#[derive(Debug, Clone)]
pub enum Data {
    Geometry(Arc<Geometry>),
    Field(Field),
    Object(Option<Id>),
    Material(Option<Id>),
    Text(String),
}

fn field_kind(kind: SocketType) -> Option<Kind> {
    match kind {
        SocketType::Float => Some(Kind::Float),
        SocketType::Int => Some(Kind::Int),
        SocketType::Bool => Some(Kind::Bool),
        SocketType::Vector | SocketType::Rotation => Some(Kind::Vector),
        _ => None,
    }
}

/// The value of JSON `value` as data of type `kind`.
pub fn json_data(value: &Value, kind: SocketType) -> Data {
    let number = |value: &Value| value.as_f64().map(|number| number as f32);
    match kind {
        // Colours and curves are compositing sockets: geometry groups never hold them.
        SocketType::Geometry | SocketType::Any | SocketType::Color | SocketType::Curves | SocketType::Ramp | SocketType::Shader => {
            Data::Geometry(Arc::new(Geometry::default()))
        }
        SocketType::Object => Data::Object(value.as_u64().and_then(|id| Id::try_from(id).ok())),
        SocketType::Material => Data::Material(value.as_u64().and_then(|id| Id::try_from(id).ok())),
        SocketType::String => Data::Text(value.as_str().unwrap_or_default().to_owned()),
        SocketType::Bool => {
            Data::Field(Field::Constant(Scalar::Bool(value.as_bool().unwrap_or_else(|| number(value).is_some_and(|number| number > 0.0)))))
        }
        SocketType::Int => Data::Field(Field::Constant(Scalar::Int(number(value).map_or(0, |number| number.round() as i32)))),
        SocketType::Float => Data::Field(Field::Constant(Scalar::Float(number(value).unwrap_or(0.0)))),
        SocketType::Vector | SocketType::Rotation => {
            let vector = match value {
                Value::Array(items) => Vec3::new(
                    items.first().and_then(number).unwrap_or(0.0),
                    items.get(1).and_then(number).unwrap_or(0.0),
                    items.get(2).and_then(number).unwrap_or(0.0),
                ),
                other => Vec3::splat(number(other).unwrap_or(0.0)),
            };
            Data::Field(Field::Constant(Scalar::Vector(vector)))
        }
    }
}

pub(crate) fn default_data(default: SocketDefault, kind: SocketType) -> Data {
    let value = default.json();
    if value.is_null() {
        return match kind {
            SocketType::Float | SocketType::Int | SocketType::Bool | SocketType::Vector | SocketType::Rotation => json_data(&Value::from(0), kind),
            _ => json_data(&Value::Null, kind),
        };
    }
    json_data(&value, kind)
}

/// `data` as type `kind` (a number becomes a vector, a field asked for as geometry is empty).
pub(crate) fn convert(data: Data, kind: SocketType) -> Data {
    match (data, kind) {
        (Data::Field(field), kind) if field_kind(kind).is_some() => Data::Field(field.convert(field_kind(kind).unwrap_or(Kind::Float))),
        (Data::Geometry(geometry), SocketType::Geometry) => Data::Geometry(geometry),
        (Data::Object(object), SocketType::Object) => Data::Object(object),
        (Data::Material(material), SocketType::Material) => Data::Material(material),
        (Data::Text(text), SocketType::String) => Data::Text(text),
        (_, kind) => default_data(SocketDefault::None, kind),
    }
}

/// A node's outputs, by socket id.
pub(crate) type Outputs = BTreeMap<String, Data>;

/// Simulation states worked out in one run: by zone (the group's prefix and the output node)
/// and frame.
pub(crate) type States = Rc<RefCell<HashMap<(String, Id, i32), Arc<Outputs>>>>;

/// Runs one group.
pub struct Evaluator<'a> {
    pub(crate) call: &'a NodesCall<'a>,
    pub(crate) group: &'a NodeGroup,
    pub(crate) input: Arc<Geometry>,
    pub(crate) done: HashMap<Id, Arc<Outputs>>,
    pub(crate) running: BTreeSet<Id>,
    /// Nodes whose geometry descends from the bounded additional spline types.
    bounded_splines: BTreeSet<Id>,
    /// What the Group Input gives, for a group a Group node runs (the modifier's inputs
    /// otherwise).
    pub(crate) interface: Option<Arc<Outputs>>,
    /// Where this group sits: the Group nodes it is run through, so anonymous attributes of two
    /// uses of one group stay apart.
    pub(crate) prefix: String,
    /// How many Group nodes deep this group runs.
    pub(crate) depth: u32,
    /// The frame Scene Time and simulation zones see.
    pub(crate) frame: f32,
    /// Simulation states already worked out in this run, by zone (prefix and output node) and
    /// frame.
    pub(crate) states: States,
}

pub(crate) fn anonymous(prefix: &str, node: &Node, socket: &str) -> String {
    format!(".{prefix}{}.{socket}", node.id)
}

impl<'a> Evaluator<'a> {
    pub fn new(call: &'a NodesCall<'a>, group: &'a NodeGroup, input: Geometry) -> Self {
        Self {
            call,
            group,
            input: Arc::new(input),
            done: HashMap::new(),
            running: BTreeSet::new(),
            bounded_splines: BTreeSet::new(),
            interface: None,
            prefix: String::new(),
            depth: 0,
            frame: call.scene.nodes_frame(),
            states: Rc::new(RefCell::new(HashMap::new())),
        }
    }

    /// An evaluator for `group` run by a Group node of this one, or for this group at another
    /// frame (sharing the simulation states worked out so far).
    pub(crate) fn nested(
        &self,
        group: &'a NodeGroup,
        input: Arc<Geometry>,
        interface: Option<Arc<Outputs>>,
        prefix: String,
        depth: u32,
        frame: f32,
    ) -> Self {
        Self {
            call: self.call,
            group,
            input,
            done: HashMap::new(),
            running: BTreeSet::new(),
            bounded_splines: BTreeSet::new(),
            interface,
            prefix,
            depth,
            frame,
            states: self.states.clone(),
        }
    }

    /// The geometry the group's output gets.
    pub fn run(mut self) -> Result<Geometry, String> {
        let outputs = self.run_outputs()?;
        match outputs.get("Geometry") {
            Some(Data::Geometry(geometry)) => Ok(Arc::unwrap_or_clone(geometry.clone())),
            _ => Ok(Geometry::default()),
        }
    }

    /// Everything the group's output gets: the geometry and the group's outputs.
    pub(crate) fn run_outputs(&mut self) -> Result<Outputs, String> {
        let Some(output) = self.group.nodes.iter().find(|node| node.kind == "groupOutput") else {
            return Err("the node group has no Group Output".into());
        };
        let mut out = Outputs::new();
        for (id, kind) in node_tree::group_output_sockets(self.group) {
            let data = self.socket(output, &id, kind, &Value::Null)?;
            out.insert(id, data);
        }
        Ok(out)
    }

    /// The data on a node's input `socket` of type `kind`, for sockets that follow data (a
    /// group's interface, a zone's items): the first link's, else the node's own value, else
    /// `default`.
    pub(crate) fn socket(&mut self, node: &Node, socket: &str, kind: SocketType, default: &Value) -> Result<Data, String> {
        let link = self.group.links.iter().find(|link| link.to == node.id && link.to_socket == socket).cloned();
        if let Some(link) = link {
            let outputs = self.outputs(link.from)?;
            return Ok(convert(outputs.get(&link.from_socket).cloned().unwrap_or_else(|| default_data(SocketDefault::None, kind)), kind));
        }
        if let Some(value) = node.values.get(socket) {
            return Ok(json_data(value, kind));
        }
        if default.is_null() { Ok(default_data(SocketDefault::None, kind)) } else { Ok(json_data(default, kind)) }
    }

    pub(crate) fn outputs(&mut self, id: Id) -> Result<Arc<Outputs>, String> {
        if let Some(done) = self.done.get(&id) {
            return Ok(done.clone());
        }
        if !self.running.insert(id) {
            return Err("the node group has a cycle".into());
        }
        let node = self.group.node(id).ok_or_else(|| format!("node {id} is missing"))?;
        let outputs = Arc::new(self.evaluate(node)?);
        if self.inherits_spline_bounds(node)
            || outputs.values().any(|value| matches!(value, Data::Geometry(geometry) if crate::spline_limits::contains_new_splines(geometry)))
        {
            for value in outputs.values() {
                if let Data::Geometry(geometry) = value {
                    check_size(geometry)?;
                    crate::spline_limits::validate(geometry)?;
                }
            }
            self.bounded_splines.insert(id);
        }
        self.running.remove(&id);
        self.done.insert(id, outputs.clone());
        Ok(outputs)
    }

    fn inherits_spline_bounds(&self, node: &Node) -> bool {
        self.group.links.iter().any(|link| link.to == node.id && self.bounded_splines.contains(&link.from))
    }

    /// The data on input `socket` (the first link's, or the node's own value, or the default).
    pub(crate) fn input(&mut self, node: &Node, kind: &NodeKind, socket: &str) -> Result<Data, String> {
        let Some(spec) = kind.inputs.iter().find(|item| item.id == socket) else {
            return Err(format!("{} has no input {socket}", kind.title));
        };
        let wanted = node_tree::socket_type(node, kind, spec);
        let link = self.group.links.iter().find(|link| link.to == node.id && link.to_socket == socket).cloned();
        if let Some(link) = link {
            let outputs = self.outputs(link.from)?;
            return Ok(convert(outputs.get(&link.from_socket).cloned().unwrap_or_else(|| default_data(SocketDefault::None, wanted)), wanted));
        }
        Ok(unlinked(node, spec, wanted))
    }

    pub(crate) fn field(&mut self, node: &Node, kind: &NodeKind, socket: &str) -> Result<Field, String> {
        match self.input(node, kind, socket)? {
            Data::Field(field) => Ok(field),
            _ => Ok(Field::Constant(Scalar::Float(0.0))),
        }
    }

    pub(crate) fn single(&mut self, node: &Node, kind: &NodeKind, socket: &str) -> Result<Scalar, String> {
        Ok(field::single(&self.field(node, kind, socket)?))
    }

    pub(crate) fn geometry(&mut self, node: &Node, kind: &NodeKind, socket: &str) -> Result<Arc<Geometry>, String> {
        match self.input(node, kind, socket)? {
            Data::Geometry(geometry) => Ok(geometry),
            _ => Ok(Arc::new(Geometry::default())),
        }
    }

    pub(crate) fn owned(&mut self, node: &Node, kind: &NodeKind, socket: &str) -> Result<Geometry, String> {
        Ok(Arc::unwrap_or_clone(self.geometry(node, kind, socket)?))
    }

    fn evaluate(&mut self, node: &Node) -> Result<Outputs, String> {
        let kind = node_tree::node_kind(&node.kind).ok_or_else(|| format!("unknown node kind {}", node.kind))?;
        let setting = |id: &str| node_tree::setting(node, kind, id).to_owned();
        let mut out = Outputs::new();
        let geometry = |geometry: Geometry| Data::Geometry(Arc::new(geometry));
        let mesh_out = |mesh: kition_scene_model::Mesh| Data::Geometry(Arc::new(Geometry::from_mesh(mesh)));
        let curves_out = |curves: CurvesComponent| Data::Geometry(Arc::new(Geometry { curves: Some(curves), ..Geometry::default() }));
        match node.kind.as_str() {
            "groupInput" => {
                if let Some(interface) = &self.interface {
                    return Ok(interface.as_ref().clone());
                }
                out.insert("Geometry".into(), Data::Geometry(self.input.clone()));
                for input in &self.group.inputs {
                    let value = self.call.inputs.get(&input.name).unwrap_or(&input.default);
                    out.insert(input.name.clone(), json_data(value, input.kind));
                }
            }
            "groupOutput" => {}
            "meshCube" => {
                let size = self.single(node, kind, "Size")?.vector();
                let counts = [
                    self.single(node, kind, "Vertices X")?.int(),
                    self.single(node, kind, "Vertices Y")?.int(),
                    self.single(node, kind, "Vertices Z")?.int(),
                ];
                out.insert("Mesh".into(), mesh_out(prims::cube(size, counts)?));
            }
            "meshGrid" => {
                let mesh = prims::grid(
                    self.single(node, kind, "Size X")?.float(),
                    self.single(node, kind, "Size Y")?.float(),
                    self.single(node, kind, "Vertices X")?.int(),
                    self.single(node, kind, "Vertices Y")?.int(),
                )?;
                out.insert("Mesh".into(), mesh_out(mesh));
            }
            "meshUvSphere" | "meshIcoSphere" => {
                let radius = self.single(node, kind, "Radius")?.float();
                let primitive = if node.kind == "meshUvSphere" {
                    let segments = u32::try_from(self.single(node, kind, "Segments")?.int()).unwrap_or(0);
                    let rings = u32::try_from(self.single(node, kind, "Rings")?.int()).unwrap_or(0);
                    if segments < 3 || rings < 2 {
                        out.insert("Mesh".into(), mesh_out(kition_scene_model::Mesh::default()));
                        return Ok(out);
                    }
                    kition_scene_model::Primitive::UvSphere { segments, rings, radius: radius.abs().max(1e-6) }
                } else {
                    let subdivisions = u32::try_from(self.single(node, kind, "Subdivisions")?.int()).unwrap_or(1).max(1);
                    kition_scene_model::Primitive::IcoSphere { subdivisions, radius: radius.abs().max(1e-6) }
                };
                let mut mesh = primitive.build().map_err(|error| error.to_string())?;
                if radius < 0.0 {
                    mesh.transform(Mat4::from_scale(Vec3::splat(-1.0)));
                }
                out.insert("Mesh".into(), mesh_out(mesh));
            }
            "meshCylinder" | "meshCone" => {
                let cylinder = node.kind == "meshCylinder";
                let depth = self.single(node, kind, "Depth")?.float();
                let (radius_top, radius_bottom) = if cylinder {
                    let radius = self.single(node, kind, "Radius")?.float();
                    (radius, radius)
                } else {
                    (self.single(node, kind, "Radius Top")?.float(), self.single(node, kind, "Radius Bottom")?.float())
                };
                let cone = Cone {
                    vertices: self.single(node, kind, "Vertices")?.int(),
                    side_segments: self.single(node, kind, "Side Segments")?.int(),
                    fill_segments: self.single(node, kind, "Fill Segments")?.int(),
                    radius_top,
                    radius_bottom,
                    depth,
                    fill: Fill::parse(&setting("fillType")),
                    bottom: if cylinder { -depth / 2.0 } else { 0.0 },
                };
                let names = [anonymous(&self.prefix, node, "Top"), anonymous(&self.prefix, node, "Side"), anonymous(&self.prefix, node, "Bottom")];
                let component = prims::cone(&cone, [names[0].as_str(), names[1].as_str(), names[2].as_str()])?;
                out.insert("Mesh".into(), geometry(Geometry { mesh: Some(component), ..Geometry::default() }));
                for (socket, name) in ["Top", "Side", "Bottom"].into_iter().zip(names) {
                    out.insert(socket.into(), Data::Field(Field::Attribute(name, Kind::Bool)));
                }
            }
            "meshLine" => {
                let mesh = prims::line(
                    self.single(node, kind, "Count")?.int(),
                    self.single(node, kind, "Start Location")?.vector(),
                    self.single(node, kind, "Offset")?.vector(),
                    setting("mode") == "end_points",
                )?;
                out.insert("Mesh".into(), mesh_out(mesh));
            }
            "meshCircle" => {
                let mesh = prims::circle(
                    self.single(node, kind, "Vertices")?.int(),
                    self.single(node, kind, "Radius")?.float(),
                    Fill::parse(&setting("fillType")),
                )?;
                out.insert("Mesh".into(), mesh_out(mesh));
            }
            "curveLine" => {
                let start = self.single(node, kind, "Start")?.vector();
                let end = if setting("mode") == "direction" {
                    start + self.single(node, kind, "Direction")?.vector().normalize_or_zero() * self.single(node, kind, "Length")?.float()
                } else {
                    self.single(node, kind, "End")?.vector()
                };
                out.insert("Curve".into(), curves_out(curve_ops::line(start, end)));
            }
            "curveCircle" => {
                let resolution = self.single(node, kind, "Resolution")?.int();
                let (curves, centre) = if setting("mode") == "points" {
                    let points = [self.single(node, kind, "Point 1")?, self.single(node, kind, "Point 2")?, self.single(node, kind, "Point 3")?];
                    curve_ops::circle_through(resolution, points[0].vector(), points[1].vector(), points[2].vector())?
                        .unwrap_or((CurvesComponent::default(), Vec3::ZERO))
                } else {
                    (curve_ops::circle(resolution, self.single(node, kind, "Radius")?.float())?, Vec3::ZERO)
                };
                out.insert("Curve".into(), curves_out(curves));
                out.insert("Center".into(), Data::Field(Field::Constant(Scalar::Vector(centre))));
            }
            "curveBezierSegment" => {
                let curves = curve_ops::bezier_segment(
                    self.single(node, kind, "Resolution")?.int(),
                    self.single(node, kind, "Start")?.vector(),
                    self.single(node, kind, "Start Handle")?.vector(),
                    self.single(node, kind, "End Handle")?.vector(),
                    self.single(node, kind, "End")?.vector(),
                    setting("mode") == "offset",
                );
                out.insert("Curve".into(), curves_out(curves));
            }
            "curveQuadrilateral" => {
                let mut value = |socket: &str| self.single(node, kind, socket).map(Scalar::float);
                let shape = match setting("mode").as_str() {
                    "parallelogram" => Quadrilateral::Parallelogram { width: value("Width")?, height: value("Height")?, offset: value("Offset")? },
                    "trapezoid" => Quadrilateral::Trapezoid {
                        bottom: value("Bottom Width")?,
                        top: value("Top Width")?,
                        offset: value("Offset")?,
                        height: value("Height")?,
                    },
                    "kite" => Quadrilateral::Kite { width: value("Width")?, bottom: value("Bottom Height")?, top: value("Top Height")? },
                    "points" => Quadrilateral::Points([
                        self.single(node, kind, "Point 1")?.vector(),
                        self.single(node, kind, "Point 2")?.vector(),
                        self.single(node, kind, "Point 3")?.vector(),
                        self.single(node, kind, "Point 4")?.vector(),
                    ]),
                    _ => Quadrilateral::Rectangle { width: value("Width")?, height: value("Height")? },
                };
                out.insert("Curve".into(), curves_out(curve_ops::quadrilateral(&shape)));
            }
            "curveSpiral" => {
                let curves = curve_ops::spiral(
                    self.single(node, kind, "Resolution")?.int(),
                    self.single(node, kind, "Rotations")?.float(),
                    self.single(node, kind, "Start Radius")?.float(),
                    self.single(node, kind, "End Radius")?.float(),
                    self.single(node, kind, "Height")?.float(),
                    self.single(node, kind, "Reverse")?.bool(),
                )?;
                out.insert("Curve".into(), curves_out(curves));
            }
            "curveStar" => {
                let (mut curves, outer) = curve_ops::star(
                    self.single(node, kind, "Points")?.int(),
                    self.single(node, kind, "Inner Radius")?.float(),
                    self.single(node, kind, "Outer Radius")?.float(),
                    self.single(node, kind, "Twist")?.float(),
                )?;
                let name = anonymous(&self.prefix, node, "Outer Points");
                curves.attributes.insert(name.clone(), Attribute { domain: Domain::Point, values: outer.into_iter().map(Scalar::Bool).collect() });
                out.insert("Curve".into(), curves_out(curves));
                out.insert("Outer Points".into(), Data::Field(Field::Attribute(name, Kind::Bool)));
            }
            "curveToMesh" => {
                let source = self.geometry(node, kind, "Curve")?;
                let profile = self.geometry(node, kind, "Profile Curve")?;
                let scale = self.field(node, kind, "Scale")?;
                let caps = self.single(node, kind, "Fill Caps")?.bool();
                let mut result = Geometry::default();
                if let Some(curves) = &source.curves {
                    if self.inherits_spline_bounds(node) {
                        let profile_points = profile.curves.as_ref().map_or(1, |profile| profile.evaluated_len().max(1));
                        if curves.evaluated_len().saturating_mul(profile_points) > MAX_ELEMENTS {
                            return Err(format!("the spline sweep would have more than {MAX_ELEMENTS} elements"));
                        }
                    }
                    let scales: Vec<f32> = floats(&scale, Component::Curves(curves), Domain::Point);
                    result.mesh = Some(MeshComponent::new(curve_ops::curve_to_mesh(curves, profile.curves.as_ref(), &scales, caps)));
                }
                check_size(&result)?;
                out.insert("Mesh".into(), geometry(result));
            }
            "meshToCurve" => {
                let source = self.geometry(node, kind, "Mesh")?;
                let selection = self.field(node, kind, "Selection")?;
                let mut result = Geometry::default();
                if let Some(mesh) = &source.mesh {
                    result.curves = Some(if setting("mode") == "faces" {
                        curve_ops::mesh_to_curve_faces(mesh, &bools(&selection, Component::Mesh(mesh), Domain::Face))
                    } else {
                        curve_ops::mesh_to_curve_edges(mesh, &bools(&selection, Component::Mesh(mesh), Domain::Edge))
                    });
                }
                out.insert("Curve".into(), geometry(result));
            }
            "curveToPoints" => {
                let source = self.geometry(node, kind, "Curve")?;
                let names =
                    [anonymous(&self.prefix, node, "Tangent"), anonymous(&self.prefix, node, "Normal"), anonymous(&self.prefix, node, "Rotation")];
                let mut points = PointsComponent::default();
                if let Some(curves) = &source.curves {
                    let mode = setting("mode");
                    let counts: Vec<usize> = match mode.as_str() {
                        "length" => {
                            let lengths = floats(&self.field(node, kind, "Length")?, Component::Curves(curves), Domain::Curve);
                            curve_ops::counts_for_length(curves, &lengths, true)
                        }
                        _ => counts(&self.field(node, kind, "Count")?, curves),
                    };
                    let mode = if mode == "evaluated" { PointsMode::Evaluated } else { PointsMode::Count(&counts) };
                    if counts.iter().fold(0usize, |sum, count| sum.saturating_add(*count)) > MAX_ELEMENTS {
                        return Err(format!("the points would be more than {MAX_ELEMENTS}"));
                    }
                    points = curve_ops::curve_to_points(curves, &mode, [&names[0], &names[1], &names[2]]);
                }
                out.insert("Points".into(), geometry(Geometry { points: Some(points), ..Geometry::default() }));
                for (socket, name) in ["Tangent", "Normal", "Rotation"].into_iter().zip(names) {
                    out.insert(socket.into(), Data::Field(Field::Attribute(name, Kind::Vector)));
                }
            }
            "fillCurve" => {
                let source = self.geometry(node, kind, "Curve")?;
                let group = self.field(node, kind, "Group ID")?;
                let mut result = Geometry::default();
                if let Some(curves) = &source.curves {
                    let groups: Vec<i32> = field::evaluate(&group, &Context { component: Component::Curves(curves), domain: Domain::Curve })
                        .iter()
                        .map(|value| value.int())
                        .collect();
                    let rule = if setting("fillRule") == "non_zero" { FillRule::NonZero } else { FillRule::EvenOdd };
                    result.mesh = Some(MeshComponent::new(curve_ops::fill_curve(curves, &groups, rule, setting("mode") == "ngons")?));
                }
                out.insert("Mesh".into(), geometry(result));
            }
            "resampleCurve" | "trimCurve" | "reverseCurve" => {
                let mut target = self.owned(node, kind, "Curve")?;
                let selection = self.field(node, kind, "Selection")?;
                if let Some(curves) = &target.curves {
                    let on_curves = Component::Curves(curves);
                    let selected = bools(&selection, on_curves, Domain::Curve);
                    let made = match node.kind.as_str() {
                        "resampleCurve" => match setting("mode").as_str() {
                            "evaluated" => curves.resample(&selected, None),
                            mode => {
                                let counts = if mode == "length" {
                                    let lengths = floats(&self.field(node, kind, "Length")?, on_curves, Domain::Curve);
                                    curve_ops::counts_for_length(curves, &lengths, setting("keepLastSegment") != "false")
                                } else {
                                    counts(&self.field(node, kind, "Count")?, curves)
                                };
                                if counts.iter().fold(0usize, |sum, count| sum.saturating_add(*count)) > MAX_ELEMENTS {
                                    return Err(format!("the curves would have more than {MAX_ELEMENTS} points"));
                                }
                                curves.resample(&selected, Some(&counts))
                            }
                        },
                        "trimCurve" => {
                            let factor = setting("mode") != "length";
                            let (start, end) = if factor { ("Start", "End") } else { ("Start_001", "End_001") };
                            let mut starts = floats(&self.field(node, kind, start)?, on_curves, Domain::Curve);
                            let mut ends = floats(&self.field(node, kind, end)?, on_curves, Domain::Curve);
                            for value in starts.iter_mut().chain(ends.iter_mut()).filter(|value| !value.is_finite()) {
                                *value = 0.0;
                            }
                            if factor {
                                for (curve, evaluated) in curves.evaluated().iter().enumerate() {
                                    for value in [starts.get_mut(curve), ends.get_mut(curve)].into_iter().flatten() {
                                        *value = value.clamp(0.0, 1.0) * evaluated.length();
                                    }
                                }
                            }
                            curves.trim(&selected, &starts, &ends)
                        }
                        _ => curves.reverse(&selected),
                    };
                    target.curves = Some(made);
                }
                check_size(&target)?;
                out.insert("Curve".into(), geometry(target));
            }
            "setCurveRadius" | "setCurveTilt" => {
                let mut target = self.owned(node, kind, "Curve")?;
                let selection = self.field(node, kind, "Selection")?;
                let radius = node.kind == "setCurveRadius";
                let value = self.field(node, kind, if radius { "Radius" } else { "Tilt" })?;
                if let Some(curves) = &mut target.curves {
                    let selected = bools(&selection, Component::Curves(curves), Domain::Point);
                    let values = floats(&value, Component::Curves(curves), Domain::Point);
                    let slots = if radius { &mut curves.radius } else { &mut curves.tilt };
                    for ((slot, selected), value) in slots.iter_mut().zip(selected).zip(values) {
                        if selected && value.is_finite() {
                            *slot = value;
                        }
                    }
                }
                out.insert("Curve".into(), geometry(target));
            }
            "setSplineType" => {
                let mut target = self.owned(node, kind, "Curve")?;
                let selection = self.field(node, kind, "Selection")?;
                if let Some(curves) = &target.curves {
                    let selected = bools(&selection, Component::Curves(curves), Domain::Curve);
                    let curve_type = match setting("splineType").as_str() {
                        "nurbs" => crate::curves::CurveType::Nurbs,
                        "catmullRom" => crate::curves::CurveType::CatmullRom,
                        "bezier" => crate::curves::CurveType::Bezier,
                        _ => crate::curves::CurveType::Poly,
                    };
                    target.curves = Some(crate::spline_types::convert(curves, &selected, curve_type)?);
                }
                check_size(&target)?;
                out.insert("Curve".into(), geometry(target));
            }
            "setNurbsOrder" | "setNurbsWeight" => {
                let mut target = self.owned(node, kind, "Curves")?;
                let selection = self.field(node, kind, "Selection")?;
                let order = node.kind == "setNurbsOrder";
                let value = self.field(node, kind, if order { "Order" } else { "Weight" })?;
                if let Some(curves) = &mut target.curves {
                    let domain = if order { Domain::Curve } else { Domain::Point };
                    let context = Context { component: Component::Curves(curves), domain };
                    let (selected, values) = (field::evaluate(&selection, &context), field::evaluate(&value, &context));
                    for (at, (selected, value)) in selected.into_iter().zip(values).enumerate() {
                        if !selected.bool() {
                            continue;
                        }
                        if order {
                            let value = value.int().max(1);
                            if value > i32::from(crate::spline_types::MAX_NURBS_ORDER) {
                                return Err("NURBS order is limited to 16".into());
                            }
                            if let Some(slot) = curves.nurbs_order.get_mut(at) {
                                *slot = value as u8;
                            }
                        } else {
                            let value = value.float();
                            if !value.is_finite() || value > 1.0e8 {
                                return Err("NURBS weight must be finite and at most 100000000".into());
                            }
                            if let Some(slot) = curves.nurbs_weight.get_mut(at) {
                                *slot = value.max(0.0);
                            }
                        }
                    }
                }
                check_size(&target)?;
                out.insert("Curves".into(), geometry(target));
            }
            "setSplineCyclic" | "setSplineResolution" => {
                let mut target = self.owned(node, kind, "Geometry")?;
                let selection = self.field(node, kind, "Selection")?;
                let cyclic = node.kind == "setSplineCyclic";
                let value = self.field(node, kind, if cyclic { "Cyclic" } else { "Resolution" })?;
                if let Some(curves) = &mut target.curves {
                    let context = Context { component: Component::Curves(curves), domain: Domain::Curve };
                    let (selected, values) = (field::evaluate(&selection, &context), field::evaluate(&value, &context));
                    for (curve, (selected, value)) in selected.into_iter().zip(values).enumerate() {
                        if !selected.bool() {
                            continue;
                        }
                        if cyclic {
                            if let Some(slot) = curves.cyclic.get_mut(curve) {
                                *slot = value.bool();
                            }
                        } else if let Some(slot) = curves.resolution.get_mut(curve) {
                            *slot = u32::try_from(value.int()).unwrap_or(1).clamp(1, 1024);
                        }
                    }
                }
                check_size(&target)?;
                out.insert("Geometry".into(), geometry(target));
            }
            "splineParameter" => {
                for (which, socket) in ["Factor", "Length", "Index"].into_iter().enumerate() {
                    out.insert(socket.into(), Data::Field(Field::SplineParameter(which)));
                }
            }
            "curveLength" => {
                let source = self.geometry(node, kind, "Curve")?;
                let length = source.curves.as_ref().map_or(0.0, curve_ops::total_length);
                out.insert("Length".into(), Data::Field(Field::Constant(Scalar::Float(length))));
            }
            "endpointSelection" => {
                let sizes = (self.field(node, kind, "Start Size")?, self.field(node, kind, "End Size")?);
                out.insert("Selection".into(), Data::Field(Field::Endpoints(Arc::new(sizes))));
            }
            "transform" => {
                let mut target = self.owned(node, kind, "Geometry")?;
                let matrix = Mat4::from_translation(self.single(node, kind, "Translation")?.vector())
                    * Mat4::from_mat3(euler_matrix(self.single(node, kind, "Rotation")?.vector()))
                    * Mat4::from_scale(self.single(node, kind, "Scale")?.vector());
                target.transform(matrix);
                out.insert("Geometry".into(), geometry(target));
            }
            "joinGeometry" => {
                let links: Vec<_> = self.group.links.iter().filter(|link| link.to == node.id && link.to_socket == "Geometry").cloned().collect();
                let mut parts = Vec::new();
                for link in links {
                    if let Some(Data::Geometry(part)) = self.outputs(link.from)?.get(&link.from_socket) {
                        parts.push(Arc::unwrap_or_clone(part.clone()));
                    }
                }
                let joined = Geometry::join(parts);
                check_size(&joined)?;
                out.insert("Geometry".into(), geometry(joined));
            }
            "setPosition" => {
                let mut target = self.owned(node, kind, "Geometry")?;
                let selection = self.field(node, kind, "Selection")?;
                let position = self.field(node, kind, "Position")?;
                let offset = self.field(node, kind, "Offset")?;
                let moved = |component: Component<'_>| -> Vec<Option<Vec3>> {
                    let context = Context { component, domain: Domain::Point };
                    let (selected, positions, offsets) =
                        (field::evaluate(&selection, &context), field::evaluate(&position, &context), field::evaluate(&offset, &context));
                    selected
                        .iter()
                        .zip(positions.iter().zip(&offsets))
                        .map(|(selected, (position, offset))| selected.bool().then(|| position.vector() + offset.vector()))
                        .collect()
                };
                if let Some(mesh) = &mut target.mesh {
                    let new = moved(Component::Mesh(mesh));
                    for (point, new) in mesh.mesh.positions.iter_mut().zip(new) {
                        if let Some(new) = new.filter(|new| new.is_finite()) {
                            *point = new;
                        }
                    }
                }
                if let Some(points) = &mut target.points {
                    let new = moved(Component::Points(points));
                    for (point, new) in points.positions.iter_mut().zip(new) {
                        if let Some(new) = new.filter(|new| new.is_finite()) {
                            *point = new;
                        }
                    }
                }
                if let Some(curves) = &mut target.curves {
                    let new = moved(Component::Curves(curves));
                    curves.move_points(&new);
                }
                if let Some(instances) = &mut target.instances {
                    let new = moved(Component::Instances(instances));
                    for (item, new) in instances.items.iter_mut().zip(new) {
                        if let Some(new) = new.filter(|new| new.is_finite()) {
                            item.transform.w_axis = new.extend(1.0);
                        }
                    }
                }
                out.insert("Geometry".into(), geometry(target));
            }
            "deleteGeometry" => {
                let mut target = self.owned(node, kind, "Geometry")?;
                let selection = self.field(node, kind, "Selection")?;
                let domain = Domain::parse(&setting("domain"));
                let mode = setting("mode");
                if let Some(mesh) = &mut target.mesh
                    && domain != Domain::Instance
                {
                    let selected: Vec<bool> =
                        field::evaluate(&selection, &Context { component: Component::Mesh(mesh), domain }).iter().map(|value| value.bool()).collect();
                    let mut removal = Removal { remove_orphans: mode == "all", keep_face_edges: mode == "only_face", ..Removal::default() };
                    match (domain, mode.as_str()) {
                        (Domain::Point, "all") => removal.points = selected,
                        (Domain::Point, _) => {
                            removal.faces = mesh
                                .mesh
                                .faces
                                .iter()
                                .map(|face| face.iter().all(|vertex| selected.get(*vertex as usize).copied().unwrap_or(false)))
                                .collect();
                            if mode == "edge_face" {
                                removal.edges = mesh
                                    .mesh
                                    .edges()
                                    .into_iter()
                                    .filter(|[a, b]| {
                                        selected.get(*a as usize).copied().unwrap_or(false) && selected.get(*b as usize).copied().unwrap_or(false)
                                    })
                                    .map(|[a, b]| (a.min(b), a.max(b)))
                                    .collect();
                            }
                        }
                        (Domain::Edge, _) => {
                            let edges = mesh.mesh.edges();
                            let chosen: Vec<(u32, u32)> = edges
                                .iter()
                                .zip(&selected)
                                .filter(|(_, selected)| **selected)
                                .map(|([a, b], _)| ((*a).min(*b), (*a).max(*b)))
                                .collect();
                            if mode == "only_face" {
                                let chosen: BTreeSet<(u32, u32)> = chosen.into_iter().collect();
                                removal.faces = mesh
                                    .mesh
                                    .faces
                                    .iter()
                                    .map(|face| {
                                        face.iter().zip(face.iter().cycle().skip(1)).any(|(a, b)| chosen.contains(&((*a).min(*b), (*a).max(*b))))
                                    })
                                    .collect();
                            } else {
                                removal.edges = chosen.into_iter().collect();
                            }
                        }
                        _ => removal.faces = selected,
                    }
                    mesh.remove(&removal);
                }
                if let Some(points) = &mut target.points
                    && domain == Domain::Point
                {
                    let keep: Vec<bool> = field::evaluate(&selection, &Context { component: Component::Points(points), domain })
                        .iter()
                        .map(|value| !value.bool())
                        .collect();
                    points.retain(&keep);
                }
                if let Some(instances) = &mut target.instances
                    && domain == Domain::Instance
                {
                    let keep: Vec<bool> = field::evaluate(&selection, &Context { component: Component::Instances(instances), domain })
                        .iter()
                        .map(|value| !value.bool())
                        .collect();
                    retain_instances(instances, &keep);
                }
                out.insert("Geometry".into(), geometry(target));
            }
            "mergeByDistance" => {
                let mut target = self.owned(node, kind, "Geometry")?;
                let selection = self.field(node, kind, "Selection")?;
                let distance = self.single(node, kind, "Distance")?.float().max(0.0);
                if let Some(mesh) = &mut target.mesh {
                    let selected = bools(&selection, Component::Mesh(mesh), Domain::Point);
                    mesh.merge(&selected, distance);
                }
                if let Some(points) = &mut target.points {
                    let selected = bools(&selection, Component::Points(points), Domain::Point);
                    points.merge(&selected, distance);
                }
                out.insert("Geometry".into(), geometry(target));
            }
            "subdivideMesh" | "subdivisionSurface" => {
                let mut target = self.owned(node, kind, "Mesh")?;
                let level = self.single(node, kind, "Level")?.int().clamp(0, 6);
                if let Some(component) = &mut target.mesh {
                    let mut mesh = std::mem::take(&mut component.mesh);
                    for _ in 0..level {
                        let corners: usize = mesh.faces.iter().map(Vec::len).sum();
                        if mesh.positions.len() + corners * 2 > MAX_ELEMENTS {
                            return Err(format!("subdividing would make more than {MAX_ELEMENTS} vertices"));
                        }
                        if node.kind == "subdivideMesh" {
                            let all: Vec<u32> = (0..u32::try_from(mesh.faces.len()).unwrap_or(0)).collect();
                            kition_scene_model::ops::subdivide(&mut mesh, &all).map_err(|error| error.to_string())?;
                        } else {
                            mesh = catmull_clark(&mesh);
                        }
                    }
                    if level > 0 {
                        if node.kind == "subdivisionSurface" {
                            kition_scene_model::modifiers::limit_positions(&mut mesh);
                        }
                        ops::drop_vertex_data(&mut mesh);
                        mesh.seams.clear();
                        mesh.pinned.clear();
                        *component = MeshComponent::new(mesh);
                    } else {
                        component.mesh = mesh;
                    }
                }
                out.insert("Mesh".into(), geometry(target));
            }
            "extrudeMesh" => {
                let mut target = self.owned(node, kind, "Mesh")?;
                let selection = self.field(node, kind, "Selection")?;
                let offset = self.field(node, kind, "Offset")?;
                let scale = self.field(node, kind, "Offset Scale")?;
                let mode = match setting("mode").as_str() {
                    "vertices" => Extrude::Vertices,
                    "edges" => Extrude::Edges,
                    _ => Extrude::Faces { individual: self.single(node, kind, "Individual")?.bool() },
                };
                let (top, side) = (anonymous(&self.prefix, node, "Top"), anonymous(&self.prefix, node, "Side"));
                if let Some(mesh) = &mut target.mesh {
                    let domain = match mode {
                        Extrude::Vertices => Domain::Point,
                        Extrude::Edges => Domain::Edge,
                        Extrude::Faces { .. } => Domain::Face,
                    };
                    let selected = bools(&selection, Component::Mesh(mesh), domain);
                    // Offsets are per point for vertices and edges, per face for faces.
                    let offset_domain = if domain == Domain::Edge { Domain::Point } else { domain };
                    let context = Context { component: Component::Mesh(mesh), domain: offset_domain };
                    let offsets: Vec<Vec3> = field::evaluate(&offset, &context)
                        .iter()
                        .zip(field::evaluate(&scale, &context))
                        .map(|(offset, scale)| offset.vector() * scale.float())
                        .collect();
                    ops::extrude(mesh, mode, &selected, &offsets, &top, &side)?;
                }
                out.insert("Mesh".into(), geometry(target));
                out.insert("Top".into(), Data::Field(Field::Attribute(top, Kind::Bool)));
                out.insert("Side".into(), Data::Field(Field::Attribute(side, Kind::Bool)));
            }
            "meshToPoints" => {
                let source = self.geometry(node, kind, "Mesh")?;
                let selection = self.field(node, kind, "Selection")?;
                let position = self.field(node, kind, "Position")?;
                let radius = self.field(node, kind, "Radius")?;
                let domain = match setting("mode").as_str() {
                    "edges" => Domain::Edge,
                    "faces" => Domain::Face,
                    "corners" => Domain::Corner,
                    _ => Domain::Point,
                };
                let mut points = PointsComponent::default();
                if let Some(mesh) = &source.mesh {
                    let context = Context { component: Component::Mesh(mesh), domain };
                    let keep: Vec<bool> = field::evaluate(&selection, &context).iter().map(|value| value.bool()).collect();
                    points.positions =
                        crate::geometry::filter(&field::evaluate(&position, &context).iter().map(|value| value.vector()).collect::<Vec<_>>(), &keep);
                    points.radius =
                        crate::geometry::filter(&field::evaluate(&radius, &context).iter().map(|value| value.float()).collect::<Vec<_>>(), &keep);
                    let carried: Vec<(String, Column)> = mesh
                        .attributes
                        .iter()
                        .map(|(name, attribute)| (name.clone(), mesh.adapt(&attribute.values, attribute.domain, domain)))
                        .collect();
                    points.attributes = ops::pick_attributes(&carried, &keep, Domain::Point);
                }
                out.insert("Points".into(), geometry(Geometry { points: Some(points), ..Geometry::default() }));
            }
            "pointsToVertices" => {
                let source = self.geometry(node, kind, "Points")?;
                let selection = self.field(node, kind, "Selection")?;
                let mut result = Geometry { instances: source.instances.clone(), ..Geometry::default() };
                if let Some(points) = &source.points {
                    let keep = bools(&selection, Component::Points(points), Domain::Point);
                    let mut component = MeshComponent::new(kition_scene_model::Mesh {
                        positions: crate::geometry::filter(&points.positions, &keep),
                        ..Default::default()
                    });
                    let carried: Vec<(String, Column)> =
                        points.attributes.iter().map(|(name, attribute)| (name.clone(), attribute.values.clone())).collect();
                    component.attributes = ops::pick_attributes(&carried, &keep, Domain::Point);
                    result.mesh = Some(component);
                }
                out.insert("Mesh".into(), geometry(result));
            }
            "distributePointsOnFaces" => {
                let source = self.geometry(node, kind, "Mesh")?;
                let selection = self.field(node, kind, "Selection")?;
                let density = self.field(node, kind, "Density")?;
                let seed = self.single(node, kind, "Seed")?.int();
                let (normal, rotation) = (anonymous(&self.prefix, node, "Normal"), anonymous(&self.prefix, node, "Rotation"));
                let mut points = PointsComponent::default();
                if let Some(mesh) = &source.mesh {
                    let context = Context { component: Component::Mesh(mesh), domain: Domain::Face };
                    let selected: Vec<bool> = field::evaluate(&selection, &context).iter().map(|value| value.bool()).collect();
                    let densities: Vec<f32> = field::evaluate(&density, &context).iter().map(|value| value.float()).collect();
                    points = ops::distribute(mesh, &selected, &densities, seed, [&normal, &rotation])?;
                }
                out.insert("Points".into(), geometry(Geometry { points: Some(points), ..Geometry::default() }));
                out.insert("Normal".into(), Data::Field(Field::Attribute(normal, Kind::Vector)));
                out.insert("Rotation".into(), Data::Field(Field::Attribute(rotation, Kind::Vector)));
            }
            "instanceOnPoints" => {
                let source = self.geometry(node, kind, "Points")?;
                let instance = self.geometry(node, kind, "Instance")?;
                let selection = self.field(node, kind, "Selection")?;
                let rotation = self.field(node, kind, "Rotation")?;
                let scale = self.field(node, kind, "Scale")?;
                let mut instances = InstancesComponent::default();
                let components: Vec<Component<'_>> =
                    source.mesh.iter().map(Component::Mesh).chain(source.points.iter().map(Component::Points)).collect();
                for component in components {
                    let context = Context { component, domain: Domain::Point };
                    let keep: Vec<bool> = field::evaluate(&selection, &context).iter().map(|value| value.bool()).collect();
                    let positions = component.positions(Domain::Point);
                    let rotations = field::evaluate(&rotation, &context);
                    let scales = field::evaluate(&scale, &context);
                    let mut part = InstancesComponent::default();
                    for (at, position) in positions.iter().enumerate() {
                        if !keep.get(at).copied().unwrap_or(false) {
                            continue;
                        }
                        let rotation = rotations.get(at).map_or(Vec3::ZERO, |value| value.vector());
                        let scale = scales.get(at).map_or(Vec3::ONE, |value| value.vector());
                        let transform = Mat4::from_translation(*position) * Mat4::from_mat3(euler_matrix(rotation)) * Mat4::from_scale(scale);
                        part.items.push(Instance { geometry: instance.clone(), transform });
                    }
                    let carried: Vec<(String, Column)> = match component {
                        Component::Mesh(mesh) => mesh
                            .attributes
                            .iter()
                            .map(|(name, attribute)| (name.clone(), mesh.adapt(&attribute.values, attribute.domain, Domain::Point)))
                            .collect(),
                        Component::Points(points) => {
                            points.attributes.iter().map(|(name, attribute)| (name.clone(), attribute.values.clone())).collect()
                        }
                        Component::Curves(_) | Component::Instances(_) => Vec::new(),
                    };
                    part.attributes = ops::pick_attributes(&carried, &keep, Domain::Instance);
                    instances.append(&part);
                }
                let result = Geometry { instances: Some(instances), ..Geometry::default() };
                check_size(&result)?;
                out.insert("Instances".into(), geometry(result));
            }
            "realizeInstances" => {
                let source = self.geometry(node, kind, "Geometry")?;
                let realized = source.realized();
                check_size(&realized)?;
                out.insert("Geometry".into(), geometry(realized));
            }
            "setMaterial" => {
                let mut target = self.owned(node, kind, "Geometry")?;
                let selection = self.field(node, kind, "Selection")?;
                let material = match self.input(node, kind, "Material")? {
                    Data::Material(material) => material,
                    _ => None,
                };
                if let Some(mesh) = &mut target.mesh {
                    let selected = bools(&selection, Component::Mesh(mesh), Domain::Face);
                    mesh.materials.resize(mesh.mesh.faces.len(), None);
                    for (slot, selected) in mesh.materials.iter_mut().zip(selected) {
                        if selected {
                            *slot = material;
                        }
                    }
                }
                out.insert("Geometry".into(), geometry(target));
            }
            "storeNamedAttribute" | "captureAttribute" => {
                let capture = node.kind == "captureAttribute";
                let mut target = self.owned(node, kind, "Geometry")?;
                let value = self.field(node, kind, "Value")?;
                let selection = if capture { Field::Constant(Scalar::Bool(true)) } else { self.field(node, kind, "Selection")? };
                let name = if capture {
                    anonymous(&self.prefix, node, "Value")
                } else {
                    match self.input(node, kind, "Name")? {
                        Data::Text(name) => name,
                        _ => String::new(),
                    }
                };
                let domain = Domain::parse(&setting("domain"));
                let value_kind = value.kind();
                if !name.is_empty() {
                    if let Some(mesh) = &mut target.mesh
                        && !matches!(domain, Domain::Instance | Domain::Curve)
                    {
                        let column = stored(&value, &selection, Component::Mesh(mesh), domain, &name);
                        mesh.attributes.insert(name.clone(), Attribute { domain, values: column });
                    }
                    if let Some(points) = &mut target.points
                        && domain == Domain::Point
                    {
                        let column = stored(&value, &selection, Component::Points(points), domain, &name);
                        points.attributes.insert(name.clone(), Attribute { domain, values: column });
                    }
                    if let Some(curves) = &mut target.curves
                        && matches!(domain, Domain::Point | Domain::Curve)
                    {
                        let column = stored(&value, &selection, Component::Curves(curves), domain, &name);
                        curves.attributes.insert(name.clone(), Attribute { domain, values: column });
                    }
                    if let Some(instances) = &mut target.instances
                        && domain == Domain::Instance
                    {
                        let column = stored(&value, &selection, Component::Instances(instances), domain, &name);
                        instances.attributes.insert(name.clone(), Attribute { domain, values: column });
                    }
                }
                out.insert("Geometry".into(), geometry(target));
                if capture {
                    out.insert("Value".into(), Data::Field(Field::Attribute(name, value_kind)));
                }
            }
            "position" => {
                out.insert("Position".into(), Data::Field(Field::Position));
            }
            "normal" => {
                out.insert("Normal".into(), Data::Field(Field::Normal));
            }
            "index" => {
                out.insert("Index".into(), Data::Field(Field::Index));
            }
            "value" => {
                out.insert("Value".into(), self.input(node, kind, "Value")?);
            }
            "vector" => {
                out.insert("Vector".into(), self.input(node, kind, "Vector")?);
            }
            "objectInfo" => self.object_info(node, kind, &mut out)?,
            "separateXyz" => {
                let vector = self.field(node, kind, "Vector")?;
                for (axis, socket) in ["X", "Y", "Z"].into_iter().enumerate() {
                    out.insert(socket.into(), Data::Field(apply(Function::Separate(axis), vec![vector.clone()])));
                }
            }
            "combineXyz" => {
                let arguments = vec![self.field(node, kind, "X")?, self.field(node, kind, "Y")?, self.field(node, kind, "Z")?];
                out.insert("Vector".into(), Data::Field(apply(Function::Combine, arguments)));
            }
            "math" => {
                let arguments = vec![self.field(node, kind, "Value")?, self.field(node, kind, "Value_001")?, self.field(node, kind, "Value_002")?];
                let function = Function::Math { op: setting("operation"), clamp: setting("useClamp") == "true" };
                out.insert("Value".into(), Data::Field(apply(function, arguments)));
            }
            "vectorMath" => {
                let arguments = vec![
                    self.field(node, kind, "Vector")?,
                    self.field(node, kind, "Vector_001")?,
                    self.field(node, kind, "Vector_002")?,
                    self.field(node, kind, "Scale")?,
                ];
                let op = setting("operation");
                out.insert("Vector".into(), Data::Field(apply(Function::VectorMath { op: op.clone(), value: false }, arguments.clone())));
                out.insert("Value".into(), Data::Field(apply(Function::VectorMath { op, value: true }, arguments)));
            }
            "randomValue" => {
                let (kind_out, low, high) = match setting("dataType").as_str() {
                    "float_vector" => (Kind::Vector, "Min_001", "Max_001"),
                    "int" => (Kind::Int, "Min_002", "Max_002"),
                    "boolean" => (Kind::Bool, "Probability", "Probability"),
                    _ => (Kind::Float, "Min", "Max"),
                };
                let arguments =
                    vec![self.field(node, kind, low)?, self.field(node, kind, high)?, self.field(node, kind, "ID")?, self.field(node, kind, "Seed")?];
                out.insert("Value".into(), Data::Field(apply(Function::Random { kind: kind_out }, arguments)));
            }
            "compare" => {
                let (kind_in, a, b) = match setting("dataType").as_str() {
                    "int" => (Kind::Int, "A_INT", "B_INT"),
                    "float_vector" => (Kind::Vector, "A_VEC3", "B_VEC3"),
                    _ => (Kind::Float, "A", "B"),
                };
                let arguments = vec![self.field(node, kind, a)?, self.field(node, kind, b)?, self.field(node, kind, "Epsilon")?];
                let function = Function::Compare { op: setting("operation"), kind: kind_in, mode: setting("mode") };
                out.insert("Result".into(), Data::Field(apply(function, arguments)));
            }
            "switch" => {
                let switch = self.field(node, kind, "Switch")?;
                let (when_false, when_true) = (self.input(node, kind, "False")?, self.input(node, kind, "True")?);
                let chosen = match (when_false, when_true) {
                    (Data::Field(when_false), Data::Field(when_true)) => Data::Field(apply(Function::Switch, vec![switch, when_false, when_true])),
                    (when_false, when_true) => {
                        if field::single(&switch).bool() {
                            when_true
                        } else {
                            when_false
                        }
                    }
                };
                out.insert("Output".into(), chosen);
            }
            "mapRange" => {
                let vector = setting("dataType") == "float_vector";
                let sockets: [&str; 5] = if vector {
                    ["Vector", "From_Min_FLOAT3", "From_Max_FLOAT3", "To_Min_FLOAT3", "To_Max_FLOAT3"]
                } else {
                    ["Value", "From Min", "From Max", "To Min", "To Max"]
                };
                let mut arguments = Vec::new();
                for socket in sockets {
                    arguments.push(self.field(node, kind, socket)?);
                }
                let steps = self.field(node, kind, "Steps")?;
                arguments.push(if vector { steps.convert(Kind::Vector) } else { steps });
                let function = Function::MapRange { vector, interpolation: setting("interpolationType"), clamp: setting("clamp") == "true" };
                out.insert((if vector { "Vector" } else { "Result" }).into(), Data::Field(apply(function, arguments)));
            }
            _ => return self.evaluate_more(node, kind),
        }
        Ok(out)
    }

    fn object_info(&mut self, node: &Node, kind: &NodeKind, out: &mut Outputs) -> Result<(), String> {
        let target = match self.input(node, kind, "Object")? {
            Data::Object(Some(id)) => self.call.scene.objects.iter().find(|object| object.id == id),
            _ => None,
        };
        let empty = || Data::Geometry(Arc::new(Geometry::default()));
        let Some(target) = target else {
            out.insert("Location".into(), Data::Field(Field::Constant(Scalar::Vector(Vec3::ZERO))));
            out.insert("Rotation".into(), Data::Field(Field::Constant(Scalar::Vector(Vec3::ZERO))));
            out.insert("Scale".into(), Data::Field(Field::Constant(Scalar::Vector(Vec3::ZERO))));
            out.insert("Geometry".into(), empty());
            return Ok(());
        };
        let world = self.call.scene.world_matrix(target.id).unwrap_or_else(|_| target.local_matrix());
        let relative = node_tree::setting(node, kind, "transformSpace") == "relative";
        let matrix = if relative {
            let own = self.call.scene.world_matrix(self.call.object.id).unwrap_or_else(|_| self.call.object.local_matrix());
            own.inverse() * world
        } else {
            world
        };
        let (scale, rotation, location) = matrix.to_scale_rotation_translation();
        out.insert("Location".into(), Data::Field(Field::Constant(Scalar::Vector(location))));
        out.insert(
            "Rotation".into(),
            Data::Field(Field::Constant(Scalar::Vector(kition_scene_model::scene::matrix_euler(Mat3::from_quat(rotation))))),
        );
        out.insert("Scale".into(), Data::Field(Field::Constant(Scalar::Vector(scale))));
        // An object reading itself, or a chain of objects too deep, gives no geometry.
        let mesh = if target.id == self.call.object.id || self.call.depth + 1 > MAX_DEPTH {
            None
        } else {
            target.evaluated_mesh_at(self.call.scene, self.call.depth + 1)
        };
        let Some(mesh) = mesh else {
            out.insert("Geometry".into(), empty());
            return Ok(());
        };
        let placed = if relative { matrix } else { Mat4::IDENTITY };
        let as_instance = self.single(node, kind, "As Instance")?.bool();
        let geometry = if as_instance {
            let item = Instance { geometry: Arc::new(Geometry::from_mesh(mesh)), transform: placed };
            Geometry { instances: Some(InstancesComponent { items: vec![item], attributes: Default::default() }), ..Geometry::default() }
        } else {
            let mut geometry = Geometry::from_mesh(mesh);
            geometry.transform(placed);
            geometry
        };
        out.insert("Geometry".into(), Data::Geometry(Arc::new(geometry)));
        Ok(())
    }
}

pub(crate) fn unlinked(node: &Node, spec: &Socket, wanted: SocketType) -> Data {
    if let Some(value) = node.values.get(spec.id) {
        return json_data(value, wanted);
    }
    if let Some(implicit) = spec.implicit {
        return Data::Field(match implicit {
            Implicit::Position => Field::Position,
            Implicit::Normal => Field::Normal,
            Implicit::Index => Field::Index,
            // Shader-only: no geometry node has these.
            Implicit::Generated | Implicit::Uv => Field::Position,
        });
    }
    default_data(spec.default, wanted)
}

pub(crate) fn apply(function: Function, arguments: Vec<Field>) -> Field {
    let field = Field::Apply(Arc::new(function), arguments);
    // Constant arguments make a constant: work it out once.
    if field.is_constant() { Field::Constant(field::single(&field)) } else { field }
}

pub(crate) fn bools(field: &Field, component: Component<'_>, domain: Domain) -> Vec<bool> {
    field::evaluate(field, &Context { component, domain }).iter().map(|value| value.bool()).collect()
}

pub(crate) fn floats(field: &Field, component: Component<'_>, domain: Domain) -> Vec<f32> {
    field::evaluate(field, &Context { component, domain }).iter().map(|value| value.float()).collect()
}

/// A count per curve (at least one each).
pub(crate) fn counts(field: &Field, curves: &CurvesComponent) -> Vec<usize> {
    field::evaluate(field, &Context { component: Component::Curves(curves), domain: Domain::Curve })
        .iter()
        .map(|value| usize::try_from(value.int()).unwrap_or(0).clamp(1, MAX_ELEMENTS))
        .collect()
}

/// A Store Named Attribute's values: the field where selected, the attribute's old values (or
/// zero) elsewhere.
pub(crate) fn stored(value: &Field, selection: &Field, component: Component<'_>, domain: Domain, name: &str) -> Column {
    let context = Context { component, domain };
    let new = field::evaluate(value, &context);
    let selected = field::evaluate(selection, &context);
    if selected.iter().all(|selected| selected.bool()) {
        return new;
    }
    let kind = value.kind();
    let old = component.attribute(name, domain).unwrap_or_default();
    new.into_iter()
        .zip(selected)
        .enumerate()
        .map(|(at, (new, selected))| if selected.bool() { new } else { old.get(at).map_or(Scalar::zero(kind), |old| old.to(kind)) })
        .collect()
}

pub(crate) fn retain_instances(instances: &mut InstancesComponent, keep: &[bool]) {
    instances.items = crate::geometry::filter(&instances.items, keep);
    for attribute in instances.attributes.values_mut() {
        attribute.values = crate::geometry::filter(&attribute.values, keep);
    }
}

pub(crate) fn check_size(geometry: &Geometry) -> Result<(), String> {
    if geometry.size() > MAX_ELEMENTS { Err(format!("the geometry would have more than {MAX_ELEMENTS} elements")) } else { Ok(()) }
}
