//! The node kinds added with groups and zones: Scene Time, attributes and statistics, the mesh
//! nodes and fields, sampling, instance transforms, and rotations.

use std::sync::Arc;

use glam::{Mat4, Vec3};
use kition_scene_model::csg::Operation;
use kition_scene_model::node_tree::{self, Node, NodeKind};
use kition_scene_model::scene::euler_matrix;

use crate::eval::{Data, Evaluator, Outputs, apply, bools, check_size};
use crate::field::{self, Accumulate, Column, Context, Field, Function, Kind, MeshInput, Scalar};
use crate::geometry::{Component, Domain, Geometry, MeshComponent};
use crate::mesh_ops::{self, QuadMethod};
use crate::sample::{Element, Sample};

/// The value type a `dataType` setting names.
fn data_kind(text: &str) -> Kind {
    match text {
        "int" => Kind::Int,
        "boolean" => Kind::Bool,
        "float_vector" => Kind::Vector,
        _ => Kind::Float,
    }
}

fn constant(value: Scalar) -> Data {
    Data::Field(Field::Constant(value))
}

fn count(value: usize) -> Data {
    constant(Scalar::Int(i32::try_from(value).unwrap_or(i32::MAX)))
}

/// The statistics of `values`: mean, median, sum, minimum, maximum, range, standard deviation,
/// and variance (of the whole population), per axis for vectors.
fn statistics(values: &[Scalar], vector: bool) -> [Scalar; 8] {
    let axes = if vector { 3 } else { 1 };
    let mut out = [[0.0f32; 3]; 8];
    for axis in 0..axes {
        let mut numbers: Vec<f32> =
            values.iter().map(|value| if vector { value.vector().to_array().get(axis).copied().unwrap_or(0.0) } else { value.float() }).collect();
        if numbers.is_empty() {
            continue;
        }
        numbers.sort_by(f32::total_cmp);
        let n = numbers.len() as f32;
        let sum: f32 = numbers.iter().sum();
        let mean = sum / n;
        let middle = numbers.len() / 2;
        let median = if numbers.len().is_multiple_of(2) {
            (numbers.get(middle.saturating_sub(1)).copied().unwrap_or(0.0) + numbers.get(middle).copied().unwrap_or(0.0)) / 2.0
        } else {
            numbers.get(middle).copied().unwrap_or(0.0)
        };
        let min = numbers.first().copied().unwrap_or(0.0);
        let max = numbers.last().copied().unwrap_or(0.0);
        let variance = numbers.iter().map(|value| (value - mean).powi(2)).sum::<f32>() / n;
        for (slot, value) in [mean, median, sum, min, max, max - min, variance.sqrt(), variance].into_iter().enumerate() {
            if let Some(cell) = out.get_mut(slot).and_then(|row| row.get_mut(axis)) {
                *cell = value;
            }
        }
    }
    out.map(|row| if vector { Scalar::Vector(Vec3::from_array(row)) } else { Scalar::Float(row[0]) })
}

impl<'a> Evaluator<'a> {
    pub(crate) fn evaluate_more(&mut self, node: &Node, kind: &NodeKind) -> Result<Outputs, String> {
        let setting = |id: &str| node_tree::setting(node, kind, id).to_owned();
        let mut out = Outputs::new();
        let geometry = |geometry: Geometry| Data::Geometry(Arc::new(geometry));
        match node.kind.as_str() {
            "group" => return self.group_node(node),
            "repeatInput" | "simulationInput" => return self.zone_input(node),
            "repeatOutput" => return self.repeat_output(node),
            "simulationOutput" => return self.simulation_output(node),
            "sceneTime" => {
                let fps = self.call.scene.animation.timeline.fps;
                out.insert("Seconds".into(), constant(Scalar::Float(if fps > 0.0 { self.frame / fps } else { 0.0 })));
                out.insert("Frame".into(), constant(Scalar::Float(self.frame)));
            }
            "namedAttribute" => {
                let name = match self.input(node, kind, "Name")? {
                    Data::Text(name) => name,
                    _ => String::new(),
                };
                let value_kind = data_kind(&setting("dataType"));
                let attribute = if name == "position" { Field::Position.convert(value_kind) } else { Field::Attribute(name.clone(), value_kind) };
                out.insert("Attribute".into(), Data::Field(attribute));
                out.insert("Exists".into(), Data::Field(Field::Exists(name)));
            }
            "domainSize" => {
                let source = self.geometry(node, kind, "Geometry")?;
                match setting("component").as_str() {
                    "pointcloud" => {
                        out.insert("Point Count".into(), count(source.points.as_ref().map_or(0, |points| points.positions.len())));
                    }
                    "curve" => {
                        let curves = source.curves.as_ref();
                        out.insert("Point Count".into(), count(curves.map_or(0, |curves| curves.points_len())));
                        out.insert("Spline Count".into(), count(curves.map_or(0, |curves| curves.curves_len())));
                    }
                    "instances" => {
                        out.insert("Instance Count".into(), count(source.instances.as_ref().map_or(0, |instances| instances.items.len())));
                    }
                    _ => {
                        let mesh = source.mesh.as_ref();
                        for (socket, domain) in [
                            ("Point Count", Domain::Point),
                            ("Edge Count", Domain::Edge),
                            ("Face Count", Domain::Face),
                            ("Face Corner Count", Domain::Corner),
                        ] {
                            out.insert(socket.into(), count(mesh.map_or(0, |mesh| mesh.domain_len(domain))));
                        }
                    }
                }
            }
            "attributeStatistic" => {
                let source = self.geometry(node, kind, "Geometry")?;
                let selection = self.field(node, kind, "Selection")?;
                let attribute = self.field(node, kind, "Attribute")?;
                let domain = Domain::parse(&setting("domain"));
                let vector = setting("dataType") == "float_vector";
                let mut values = Vec::new();
                let components: Vec<Component<'_>> = source
                    .mesh
                    .iter()
                    .map(Component::Mesh)
                    .chain(source.points.iter().map(Component::Points))
                    .chain(source.curves.iter().map(Component::Curves))
                    .chain(source.instances.iter().map(Component::Instances))
                    .filter(|component| match component {
                        Component::Mesh(_) => matches!(domain, Domain::Point | Domain::Edge | Domain::Face | Domain::Corner),
                        Component::Points(_) => domain == Domain::Point,
                        Component::Curves(_) => matches!(domain, Domain::Point | Domain::Curve),
                        Component::Instances(_) => domain == Domain::Instance,
                    })
                    .collect();
                for component in components {
                    let context = Context { component, domain };
                    let selected = field::evaluate(&selection, &context);
                    let found = field::evaluate(&attribute, &context);
                    values.extend(found.into_iter().zip(selected).filter(|(_, selected)| selected.bool()).map(|(value, _)| value));
                }
                let names = ["Mean", "Median", "Sum", "Min", "Max", "Range", "Standard Deviation", "Variance"];
                for (name, value) in names.into_iter().zip(statistics(&values, vector)) {
                    out.insert(name.into(), constant(value));
                }
            }
            "accumulateField" => {
                let value_kind = data_kind(&setting("dataType"));
                let value = self.field(node, kind, "Value")?.convert(value_kind);
                let group = self.field(node, kind, "Group Index")?;
                let domain = Domain::parse(&setting("domain"));
                for (which, socket) in ["Leading", "Trailing", "Total"].into_iter().enumerate() {
                    let accumulate = Accumulate { value: value.clone(), group: group.clone(), domain, kind: value_kind, which };
                    out.insert(socket.into(), Data::Field(Field::Accumulate(Arc::new(accumulate))));
                }
            }
            "meshBoolean" => {
                let operation = match setting("operation").as_str() {
                    "union" => Operation::Union,
                    "intersect" => Operation::Intersect,
                    _ => Operation::Difference,
                };
                let first = if operation == Operation::Difference {
                    self.geometry(node, kind, "Mesh 1")?.realized().mesh.map(|mesh| mesh.mesh)
                } else {
                    None
                };
                let links: Vec<_> = self.group.links.iter().filter(|link| link.to == node.id && link.to_socket == "Mesh 2").cloned().collect();
                let mut others = Vec::new();
                for link in links {
                    if let Some(Data::Geometry(part)) = self.outputs(link.from)?.get(&link.from_socket)
                        && let Some(mesh) = part.realized().mesh
                    {
                        others.push(mesh.mesh);
                    }
                }
                let mesh = mesh_ops::boolean(first.as_ref(), &others, operation)?;
                out.insert("Mesh".into(), geometry(Geometry::from_mesh(mesh)));
            }
            "dualMesh" => {
                let source = self.geometry(node, kind, "Mesh")?;
                let keep = self.single(node, kind, "Keep Boundaries")?.bool();
                let mut result = Geometry::default();
                if let Some(mesh) = &source.mesh {
                    result.mesh = Some(MeshComponent::new(mesh_ops::dual_mesh(mesh, keep)?));
                }
                out.insert("Dual Mesh".into(), geometry(result));
            }
            "flipFaces" | "splitEdges" | "triangulate" => {
                let mut target = self.owned(node, kind, "Mesh")?;
                let selection = self.field(node, kind, "Selection")?;
                if let Some(mesh) = &mut target.mesh {
                    match node.kind.as_str() {
                        "flipFaces" => {
                            let selected = bools(&selection, Component::Mesh(mesh), Domain::Face);
                            mesh_ops::flip_faces(mesh, &selected);
                        }
                        "splitEdges" => {
                            let selected = bools(&selection, Component::Mesh(mesh), Domain::Edge);
                            mesh_ops::split_edges(mesh, &selected)?;
                        }
                        _ => {
                            let selected = bools(&selection, Component::Mesh(mesh), Domain::Face);
                            mesh_ops::triangulate(mesh, &selected, QuadMethod::parse(&setting("quadMethod")), setting("ngonMethod") == "beauty")?;
                        }
                    }
                }
                out.insert("Mesh".into(), geometry(target));
            }
            "scaleElements" => {
                let mut target = self.owned(node, kind, "Geometry")?;
                let selection = self.field(node, kind, "Selection")?;
                let scale = self.field(node, kind, "Scale")?;
                let centre = self.field(node, kind, "Center")?;
                let axis = self.field(node, kind, "Axis")?;
                let domain = if setting("domain") == "edge" { Domain::Edge } else { Domain::Face };
                let single_axis = setting("scaleMode") == "single_axis";
                if let Some(mesh) = &mut target.mesh {
                    let context = Context { component: Component::Mesh(mesh), domain };
                    let selected: Vec<bool> = field::evaluate(&selection, &context).iter().map(|value| value.bool()).collect();
                    let pick =
                        |column: Column| -> Column { column.into_iter().zip(&selected).filter(|(_, keep)| **keep).map(|(value, _)| value).collect() };
                    let scales: Vec<f32> = pick(field::evaluate(&scale, &context)).iter().map(|value| value.float()).collect();
                    let centres: Vec<Vec3> = pick(field::evaluate(&centre, &context)).iter().map(|value| value.vector()).collect();
                    let axes: Vec<Vec3> = pick(field::evaluate(&axis, &context)).iter().map(|value| value.vector()).collect();
                    let elements: Vec<Vec<u32>> = if domain == Domain::Edge {
                        mesh.mesh.edges().into_iter().zip(&selected).filter(|(_, keep)| **keep).map(|(edge, _)| edge.to_vec()).collect()
                    } else {
                        mesh.mesh.faces.iter().zip(&selected).filter(|(_, keep)| **keep).map(|(face, _)| face.clone()).collect()
                    };
                    mesh_ops::scale_elements(&mut mesh.mesh, &elements, &scales, &centres, single_axis.then_some(axes.as_slice()));
                }
                out.insert("Geometry".into(), geometry(target));
            }
            "meshIsland" => {
                out.insert("Island Index".into(), Data::Field(Field::Mesh(MeshInput::IslandIndex)));
                out.insert("Island Count".into(), Data::Field(Field::Mesh(MeshInput::IslandCount)));
            }
            "edgeAngle" => {
                out.insert("Unsigned Angle".into(), Data::Field(Field::Mesh(MeshInput::UnsignedAngle)));
                out.insert("Signed Angle".into(), Data::Field(Field::Mesh(MeshInput::SignedAngle)));
            }
            "faceArea" => {
                out.insert("Area".into(), Data::Field(Field::Mesh(MeshInput::FaceArea)));
            }
            "edgeNeighbors" => {
                out.insert("Face Count".into(), Data::Field(Field::Mesh(MeshInput::EdgeFaces)));
            }
            "sampleIndex" => {
                let source = self.geometry(node, kind, "Geometry")?;
                let value_kind = data_kind(&setting("dataType"));
                let value = self.field(node, kind, "Value")?.convert(value_kind);
                let index = self.field(node, kind, "Index")?;
                let sample = Sample::Index {
                    geometry: source,
                    value,
                    index,
                    domain: Domain::parse(&setting("domain")),
                    clamp: setting("clamp") == "true",
                    kind: value_kind,
                };
                out.insert("Value".into(), Data::Field(Field::Sample(Arc::new(sample))));
            }
            "sampleNearest" => {
                let source = self.geometry(node, kind, "Geometry")?;
                let position = self.field(node, kind, "Sample Position")?;
                let sample = Sample::Nearest { geometry: source, position, domain: Domain::parse(&setting("domain")) };
                out.insert("Index".into(), Data::Field(Field::Sample(Arc::new(sample))));
            }
            "raycast" => {
                let target = self.geometry(node, kind, "Target Geometry")?.realized();
                let target = Arc::new(target);
                let value_kind = data_kind(&setting("dataType"));
                let attribute = self.field(node, kind, "Attribute")?.convert(value_kind);
                let (source, direction, length) =
                    (self.field(node, kind, "Source Position")?, self.field(node, kind, "Ray Direction")?, self.field(node, kind, "Ray Length")?);
                let interpolated = setting("interpolation") != "nearest";
                for (output, socket) in ["Is Hit", "Hit Position", "Hit Normal", "Hit Distance", "Attribute"].into_iter().enumerate() {
                    let sample = Sample::Raycast {
                        target: target.clone(),
                        attribute: attribute.clone(),
                        source: source.clone(),
                        direction: direction.clone(),
                        length: length.clone(),
                        interpolated,
                        output,
                        kind: value_kind,
                    };
                    out.insert(socket.into(), Data::Field(Field::Sample(Arc::new(sample))));
                }
            }
            "geometryProximity" => {
                let target = Arc::new(self.geometry(node, kind, "Target")?.realized());
                let (group, source, sample_group) =
                    (self.field(node, kind, "Group ID")?, self.field(node, kind, "Source Position")?, self.field(node, kind, "Sample Group ID")?);
                let element = match setting("targetElement").as_str() {
                    "points" => Element::Points,
                    "edges" => Element::Edges,
                    _ => Element::Faces,
                };
                for (output, socket) in ["Position", "Distance", "Is Valid"].into_iter().enumerate() {
                    let sample = Sample::Proximity {
                        target: target.clone(),
                        group: group.clone(),
                        source: source.clone(),
                        sample_group: sample_group.clone(),
                        element,
                        output,
                    };
                    out.insert(socket.into(), Data::Field(Field::Sample(Arc::new(sample))));
                }
            }
            "translateInstances" | "rotateInstances" | "scaleInstances" => {
                let mut target = self.owned(node, kind, "Instances")?;
                let selection = self.field(node, kind, "Selection")?;
                let local = self.field(node, kind, "Local Space")?;
                let (first, second) = match node.kind.as_str() {
                    "translateInstances" => (self.field(node, kind, "Translation")?, Field::Constant(Scalar::Vector(Vec3::ZERO))),
                    "rotateInstances" => (self.field(node, kind, "Rotation")?, self.field(node, kind, "Pivot Point")?),
                    _ => (self.field(node, kind, "Scale")?, self.field(node, kind, "Center")?),
                };
                if let Some(instances) = &mut target.instances {
                    let context = Context { component: Component::Instances(instances), domain: Domain::Instance };
                    let (selected, locals, firsts, seconds) = (
                        field::evaluate(&selection, &context),
                        field::evaluate(&local, &context),
                        field::evaluate(&first, &context),
                        field::evaluate(&second, &context),
                    );
                    let made: Vec<Option<Mat4>> = instances
                        .items
                        .iter()
                        .enumerate()
                        .map(|(at, item)| {
                            if !selected.get(at).is_some_and(|value| value.bool()) {
                                return None;
                            }
                            let local = locals.get(at).is_some_and(|value| value.bool());
                            let value = firsts.get(at).map_or(Vec3::ZERO, |value| value.vector());
                            let point = seconds.get(at).map_or(Vec3::ZERO, |value| value.vector());
                            Some(moved_instance(&node.kind, item.transform, local, value, point))
                        })
                        .collect();
                    for (item, made) in instances.items.iter_mut().zip(made) {
                        if let Some(made) = made.filter(|made| made.is_finite()) {
                            item.transform = made;
                        }
                    }
                }
                check_size(&target)?;
                out.insert("Instances".into(), geometry(target));
            }
            "instanceRotation" => {
                out.insert("Rotation".into(), Data::Field(Field::InstanceRotation));
            }
            "instanceScale" => {
                out.insert("Scale".into(), Data::Field(Field::InstanceScale));
            }
            "rotateEuler" => {
                let axis_angle = setting("rotationType") == "axis_angle";
                let arguments = vec![
                    self.field(node, kind, "Rotation")?,
                    self.field(node, kind, "Rotate By")?,
                    self.field(node, kind, "Axis")?,
                    self.field(node, kind, "Angle")?,
                ];
                let function = Function::RotateEuler { axis_angle, local: setting("space") == "local" };
                out.insert("Rotation".into(), Data::Field(apply(function, arguments)));
            }
            "rotateRotation" => {
                let arguments = vec![self.field(node, kind, "Rotation")?, self.field(node, kind, "Rotate By")?];
                let function = Function::RotateRotation { local: setting("rotationSpace") == "local" };
                out.insert("Rotation".into(), Data::Field(apply(function, arguments)));
            }
            "rotateVector" => {
                let arguments = vec![self.field(node, kind, "Vector")?, self.field(node, kind, "Rotation")?];
                out.insert("Vector".into(), Data::Field(apply(Function::RotateVector, arguments)));
            }
            "eulerToRotation" => {
                let euler = self.field(node, kind, "Euler")?;
                out.insert("Rotation".into(), Data::Field(apply(Function::Euler, vec![euler])));
            }
            "rotationToEuler" => {
                let rotation = self.field(node, kind, "Rotation")?;
                out.insert("Euler".into(), Data::Field(apply(Function::Euler, vec![rotation])));
            }
            "axisAngleToRotation" => {
                let arguments = vec![self.field(node, kind, "Axis")?, self.field(node, kind, "Angle")?];
                out.insert("Rotation".into(), Data::Field(apply(Function::AxisAngle, arguments)));
            }
            "invertRotation" => {
                let rotation = self.field(node, kind, "Rotation")?;
                out.insert("Rotation".into(), Data::Field(apply(Function::InvertRotation, vec![rotation])));
            }
            other => return Err(format!("unknown node kind {other}")),
        }
        Ok(out)
    }
}

/// An instance's transform after Translate, Rotate, or Scale Instances. In the instance's own
/// space a translation and a scale come after its transform, and a rotation turns its rotation
/// (keeping its scale) about the pivot in its scaled space; in the group's space they come before
/// it, about `point`.
fn moved_instance(kind: &str, transform: Mat4, local: bool, value: Vec3, point: Vec3) -> Mat4 {
    let about = |point: Vec3, change: Mat4| Mat4::from_translation(point) * change * Mat4::from_translation(-point);
    match (kind, local) {
        ("translateInstances", true) => transform * Mat4::from_translation(value),
        ("translateInstances", false) => Mat4::from_translation(value) * transform,
        ("rotateInstances", true) => {
            let linear = glam::Mat3::from_mat4(transform);
            let rotation = crate::field::rotation_part(linear);
            let scale = Vec3::new(linear.x_axis.length(), linear.y_axis.length(), linear.z_axis.length());
            let turn = about(scale * point, Mat4::from_mat3(euler_matrix(value)));
            Mat4::from_translation(transform.w_axis.truncate()) * Mat4::from_mat3(rotation) * turn * Mat4::from_scale(scale)
        }
        ("rotateInstances", false) => about(point, Mat4::from_mat3(euler_matrix(value))) * transform,
        (_, true) => transform * about(point, Mat4::from_scale(value)),
        (_, false) => about(point, Mat4::from_scale(value)) * transform,
    }
}
