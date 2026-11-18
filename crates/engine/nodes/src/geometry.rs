//! Geometry as node trees pass it around: a mesh, a point cloud, curves, and instances (other
//! geometry placed by transforms), each optional, with attributes stored per domain.

use std::collections::BTreeMap;
use std::sync::Arc;

use glam::{Mat3, Mat4, Vec3};
use kition_scene_model::{Id, Mesh};

use crate::curves::CurvesComponent;
use crate::field::{Column, Scalar};

/// Where an attribute lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    Point,
    Edge,
    Face,
    Corner,
    Instance,
    /// Whole curves (a curve component's splines).
    Curve,
}

impl Domain {
    /// The domain a setting names.
    pub fn parse(name: &str) -> Self {
        match name {
            "edge" => Self::Edge,
            "face" => Self::Face,
            "corner" => Self::Corner,
            "instance" => Self::Instance,
            "curve" => Self::Curve,
            _ => Self::Point,
        }
    }
}

/// An attribute: its domain and its values.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub domain: Domain,
    pub values: Column,
}

/// Attributes by name.
pub type Attributes = BTreeMap<String, Attribute>;

/// A mesh with its attributes and the material each face is given.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshComponent {
    pub mesh: Mesh,
    pub attributes: Attributes,
    /// A material per face (empty: none set).
    pub materials: Vec<Option<Id>>,
}

/// Points with a radius each.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PointsComponent {
    pub positions: Vec<Vec3>,
    pub radius: Vec<f32>,
    pub attributes: Attributes,
}

/// One instance: geometry placed by a transform.
#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    pub geometry: Arc<Geometry>,
    pub transform: Mat4,
}

/// Instances.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InstancesComponent {
    pub items: Vec<Instance>,
    pub attributes: Attributes,
}

/// Geometry: a mesh, points, curves, and instances, each optional.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Geometry {
    pub mesh: Option<MeshComponent>,
    pub points: Option<PointsComponent>,
    pub curves: Option<CurvesComponent>,
    pub instances: Option<InstancesComponent>,
}

/// One component of a geometry, to evaluate fields on.
#[derive(Clone, Copy)]
pub enum Component<'a> {
    Mesh(&'a MeshComponent),
    Points(&'a PointsComponent),
    Curves(&'a CurvesComponent),
    Instances(&'a InstancesComponent),
}

/// The largest number of elements a node may make (points, vertices, instances).
pub const MAX_ELEMENTS: usize = 4_000_000;

fn average(values: &[Scalar]) -> Option<Scalar> {
    let first = *values.first()?;
    Some(match first {
        Scalar::Bool(_) => Scalar::Bool(values.iter().all(|value| value.bool())),
        Scalar::Int(_) => Scalar::Int((values.iter().map(|value| f64::from(value.int())).sum::<f64>() / values.len() as f64).round() as i32),
        Scalar::Float(_) => Scalar::Float(values.iter().map(|value| value.float()).sum::<f32>() / values.len() as f32),
        Scalar::Vector(_) => Scalar::Vector(values.iter().map(|value| value.vector()).sum::<Vec3>() / values.len() as f32),
    })
}

/// Collects values gathered per target element into one each (a mean; booleans: all true).
fn gather(groups: Vec<Vec<Scalar>>, zero: Scalar, any_bool: bool) -> Column {
    groups
        .into_iter()
        .map(|group| match (zero, any_bool) {
            (Scalar::Bool(_), true) => Scalar::Bool(group.iter().any(|value| value.bool())),
            _ => average(&group).unwrap_or(zero),
        })
        .collect()
}

impl MeshComponent {
    /// A mesh without attributes.
    pub fn new(mesh: Mesh) -> Self {
        Self { mesh, attributes: Attributes::new(), materials: Vec::new() }
    }

    /// The number of elements in a domain.
    pub fn domain_len(&self, domain: Domain) -> usize {
        match domain {
            Domain::Point => self.mesh.positions.len(),
            Domain::Edge => self.mesh.edges().len(),
            Domain::Face => self.mesh.faces.len(),
            Domain::Corner => self.mesh.faces.iter().map(Vec::len).sum(),
            Domain::Instance | Domain::Curve => 0,
        }
    }

    fn corner_vertices(&self) -> Vec<u32> {
        self.mesh.faces.iter().flatten().copied().collect()
    }

    /// Values from domain `from` moved to domain `to`.
    pub fn adapt(&self, values: &Column, from: Domain, to: Domain) -> Column {
        if from == to {
            return values.clone();
        }
        let zero = values.first().map_or(Scalar::Float(0.0), |value| Scalar::zero(value.kind()));
        let points = self.mesh.positions.len();
        let at = |index: usize| values.get(index).copied().unwrap_or(zero);
        // Everything goes through points, faces, or corners.
        match (from, to) {
            (Domain::Point, Domain::Face) => self
                .mesh
                .faces
                .iter()
                .map(|face| average(&face.iter().map(|vertex| at(*vertex as usize)).collect::<Vec<_>>()).unwrap_or(zero))
                .collect(),
            (Domain::Point, Domain::Corner) => self.corner_vertices().into_iter().map(|vertex| at(vertex as usize)).collect(),
            (Domain::Point, Domain::Edge) => {
                self.mesh.edges().into_iter().map(|[a, b]| average(&[at(a as usize), at(b as usize)]).unwrap_or(zero)).collect()
            }
            (Domain::Face, Domain::Point) => {
                let mut groups = vec![Vec::new(); points];
                for (index, face) in self.mesh.faces.iter().enumerate() {
                    for vertex in face {
                        if let Some(group) = groups.get_mut(*vertex as usize) {
                            group.push(at(index));
                        }
                    }
                }
                gather(groups, zero, true)
            }
            (Domain::Face, Domain::Corner) => {
                self.mesh.faces.iter().enumerate().flat_map(|(index, face)| std::iter::repeat_n(at(index), face.len())).collect()
            }
            (Domain::Corner, Domain::Point) => {
                let mut groups = vec![Vec::new(); points];
                for (index, vertex) in self.corner_vertices().into_iter().enumerate() {
                    if let Some(group) = groups.get_mut(vertex as usize) {
                        group.push(at(index));
                    }
                }
                gather(groups, zero, false)
            }
            (Domain::Corner, Domain::Face) => {
                let mut start = 0;
                self.mesh
                    .faces
                    .iter()
                    .map(|face| {
                        let group: Vec<Scalar> = (start..start + face.len()).map(at).collect();
                        start += face.len();
                        average(&group).unwrap_or(zero)
                    })
                    .collect()
            }
            (Domain::Edge, Domain::Point) => {
                let mut groups = vec![Vec::new(); points];
                for (index, [a, b]) in self.mesh.edges().into_iter().enumerate() {
                    for vertex in [a, b] {
                        if let Some(group) = groups.get_mut(vertex as usize) {
                            group.push(at(index));
                        }
                    }
                }
                gather(groups, zero, true)
            }
            (Domain::Edge, _) => {
                let on_points = self.adapt(values, Domain::Edge, Domain::Point);
                self.adapt(&on_points, Domain::Point, to)
            }
            (_, Domain::Edge) => {
                let on_points = self.adapt(values, from, Domain::Point);
                self.adapt(&on_points, Domain::Point, Domain::Edge)
            }
            _ => vec![zero; self.domain_len(to)],
        }
    }

    /// Positions on a domain: vertices, edge midpoints, face centres, corners' vertices.
    pub fn positions(&self, domain: Domain) -> Vec<Vec3> {
        let column: Column = self.mesh.positions.iter().copied().map(Scalar::Vector).collect();
        self.adapt(&column, Domain::Point, domain).into_iter().map(Scalar::vector).collect()
    }

    /// Normals on a domain: vertex normals, face normals (and their corners'), edges' mean.
    pub fn normals(&self, domain: Domain) -> Vec<Vec3> {
        match domain {
            Domain::Point => self.mesh.vertex_normals(),
            Domain::Face => self.mesh.faces.iter().map(|face| self.mesh.face_normal(face)).collect(),
            Domain::Corner => self.mesh.faces.iter().flat_map(|face| std::iter::repeat_n(self.mesh.face_normal(face), face.len())).collect(),
            Domain::Edge => {
                let normals = self.mesh.vertex_normals();
                self.mesh
                    .edges()
                    .into_iter()
                    .map(|[a, b]| {
                        (normals.get(a as usize).copied().unwrap_or(Vec3::Z) + normals.get(b as usize).copied().unwrap_or(Vec3::Z))
                            .normalize_or_zero()
                    })
                    .collect()
            }
            Domain::Instance | Domain::Curve => Vec::new(),
        }
    }
}

impl Component<'_> {
    /// The number of elements in a domain.
    pub fn domain_len(&self, domain: Domain) -> usize {
        match self {
            Self::Mesh(mesh) => mesh.domain_len(domain),
            Self::Points(points) => points.positions.len(),
            Self::Curves(curves) => curves.domain_len(domain),
            Self::Instances(instances) => instances.items.len(),
        }
    }

    /// Positions on a domain.
    pub fn positions(&self, domain: Domain) -> Vec<Vec3> {
        match self {
            Self::Mesh(mesh) => mesh.positions(domain),
            Self::Points(points) => points.positions.clone(),
            Self::Curves(curves) => curves.positions(domain),
            Self::Instances(instances) => instances.items.iter().map(|item| item.transform.w_axis.truncate()).collect(),
        }
    }

    /// Normals on a domain (points and instances point up).
    pub fn normals(&self, domain: Domain) -> Vec<Vec3> {
        match self {
            Self::Mesh(mesh) => mesh.normals(domain),
            Self::Points(points) => vec![Vec3::Z; points.positions.len()],
            Self::Curves(curves) => curves.normals(domain),
            Self::Instances(instances) => instances.items.iter().map(|item| item.transform.transform_vector3(Vec3::Z).normalize_or_zero()).collect(),
        }
    }

    /// Values from domain `from` moved to domain `to` (points and instances have one domain each).
    pub fn adapt(&self, values: &Column, from: Domain, to: Domain) -> Column {
        match self {
            Self::Mesh(mesh) => mesh.adapt(values, from, to),
            Self::Curves(curves) => curves.adapt(values, from, to),
            Self::Points(_) | Self::Instances(_) => {
                let len = self.domain_len(to);
                let zero = values.first().map_or(Scalar::Float(0.0), |value| Scalar::zero(value.kind()));
                let mut out = if values.len() == len { values.clone() } else { vec![zero; len] };
                out.truncate(len);
                out
            }
        }
    }

    /// A stored attribute on a domain, moved there from its own.
    pub fn attribute(&self, name: &str, domain: Domain) -> Option<Column> {
        match self {
            Self::Mesh(mesh) => {
                let attribute = mesh.attributes.get(name)?;
                Some(mesh.adapt(&attribute.values, attribute.domain, domain))
            }
            Self::Points(points) => points.attributes.get(name).map(|attribute| attribute.values.clone()),
            Self::Curves(curves) => curves.attribute(name, domain),
            Self::Instances(instances) => instances.attributes.get(name).map(|attribute| attribute.values.clone()),
        }
    }
}

/// Keeps the entries of `values` at `keep` (a per-element flag).
pub fn filter<T: Clone>(values: &[T], keep: &[bool]) -> Vec<T> {
    values.iter().zip(keep).filter(|(_, keep)| **keep).map(|(value, _)| value.clone()).collect()
}

pub(crate) fn join_attributes(target: &mut Attributes, target_len: usize, other: &Attributes, other_len: usize) {
    let names: Vec<String> = target.keys().chain(other.keys()).cloned().collect();
    for name in names {
        let (mine, theirs) = (target.get(&name).cloned(), other.get(&name));
        let kind =
            mine.as_ref().or(theirs).and_then(|attribute| attribute.values.first()).map_or(Scalar::Float(0.0), |value| Scalar::zero(value.kind()));
        let domain = mine.as_ref().or(theirs).map_or(Domain::Point, |attribute| attribute.domain);
        let mut values = mine.map_or_else(|| vec![kind; target_len], |attribute| attribute.values);
        values.resize(target_len, kind);
        match theirs {
            Some(attribute) if attribute.domain == domain => {
                values.extend(attribute.values.iter().map(|value| value.to(kind.kind())));
                values.resize(target_len + other_len, kind);
            }
            _ => values.resize(target_len + other_len, kind),
        }
        target.insert(name, Attribute { domain, values });
    }
}

impl MeshComponent {
    /// Appends `other` (its attributes follow; ones only one side has are zero on the other).
    pub fn append(&mut self, other: &MeshComponent) {
        let lens = |mesh: &MeshComponent| [Domain::Point, Domain::Edge, Domain::Face, Domain::Corner].map(|domain| mesh.domain_len(domain));
        let (mine, theirs) = (lens(self), lens(other));
        let faces = self.mesh.faces.len();
        let had_materials = !self.materials.is_empty() || !other.materials.is_empty();
        let mut attributes = std::mem::take(&mut self.attributes);
        let mut by_domain: BTreeMap<String, Attribute> = BTreeMap::new();
        for (index, domain) in [Domain::Point, Domain::Edge, Domain::Face, Domain::Corner].into_iter().enumerate() {
            let pick = |attributes: &Attributes| -> Attributes {
                attributes
                    .iter()
                    .filter(|(_, attribute)| attribute.domain == domain)
                    .map(|(name, attribute)| (name.clone(), attribute.clone()))
                    .collect()
            };
            let mut mine_domain = pick(&attributes);
            let theirs_domain = pick(&other.attributes);
            join_attributes(&mut mine_domain, mine.get(index).copied().unwrap_or(0), &theirs_domain, theirs.get(index).copied().unwrap_or(0));
            by_domain.extend(mine_domain);
        }
        attributes.clear();
        self.attributes = by_domain;
        if had_materials {
            self.materials.resize(faces, None);
            if other.materials.is_empty() {
                self.materials.extend(std::iter::repeat_n(None, other.mesh.faces.len()));
            } else {
                self.materials.extend(other.materials.iter().copied());
            }
        }
        self.mesh.append(&other.mesh);
    }

    /// Moves the mesh by `transform`.
    pub fn transform(&mut self, transform: Mat4) {
        for point in &mut self.mesh.positions {
            *point = transform.transform_point3(*point);
        }
        if transform.determinant() < 0.0 {
            self.mesh.flip_all();
        }
    }
}

impl PointsComponent {
    /// Appends `other`.
    pub fn append(&mut self, other: &PointsComponent) {
        let len = self.positions.len();
        join_attributes(&mut self.attributes, len, &other.attributes, other.positions.len());
        self.positions.extend_from_slice(&other.positions);
        self.radius.extend_from_slice(&other.radius);
    }
}

impl InstancesComponent {
    /// Appends `other`.
    pub fn append(&mut self, other: &InstancesComponent) {
        let len = self.items.len();
        join_attributes(&mut self.attributes, len, &other.attributes, other.items.len());
        self.items.extend(other.items.iter().cloned());
    }
}

impl Geometry {
    /// Geometry with just a mesh.
    pub fn from_mesh(mesh: Mesh) -> Self {
        Self { mesh: Some(MeshComponent::new(mesh)), ..Self::default() }
    }

    /// Joins `others` into one geometry, component by component.
    pub fn join(parts: impl IntoIterator<Item = Geometry>) -> Self {
        let mut out = Geometry::default();
        for part in parts {
            if let Some(mesh) = part.mesh {
                match &mut out.mesh {
                    Some(target) => target.append(&mesh),
                    None => out.mesh = Some(mesh),
                }
            }
            if let Some(points) = part.points {
                match &mut out.points {
                    Some(target) => target.append(&points),
                    None => out.points = Some(points),
                }
            }
            if let Some(curves) = part.curves {
                match &mut out.curves {
                    Some(target) => target.append(&curves),
                    None => out.curves = Some(curves),
                }
            }
            if let Some(instances) = part.instances {
                match &mut out.instances {
                    Some(target) => target.append(&instances),
                    None => out.instances = Some(instances),
                }
            }
        }
        out
    }

    /// Moves every component by `transform` (instances' transforms are composed).
    pub fn transform(&mut self, transform: Mat4) {
        if let Some(mesh) = &mut self.mesh {
            mesh.transform(transform);
        }
        if let Some(points) = &mut self.points {
            for point in &mut points.positions {
                *point = transform.transform_point3(*point);
            }
            let scale = Mat3::from_mat4(transform).determinant().abs().cbrt();
            for radius in &mut points.radius {
                *radius *= scale;
            }
        }
        if let Some(curves) = &mut self.curves {
            curves.transform(transform);
        }
        if let Some(instances) = &mut self.instances {
            for item in &mut instances.items {
                item.transform = transform * item.transform;
            }
        }
    }

    /// The number of elements across every component and instance level, to cap growth.
    pub fn size(&self) -> usize {
        let mesh = self.mesh.as_ref().map_or(0, |mesh| mesh.mesh.positions.len() + mesh.mesh.faces.len());
        let points = self.points.as_ref().map_or(0, |points| points.positions.len());
        let curves = self.curves.as_ref().map_or(0, |curves| curves.points_len().saturating_add(curves.evaluated_len()));
        let instances = self
            .instances
            .as_ref()
            .map_or(0, |instances| instances.items.iter().map(|item| 1 + item.geometry.size()).fold(0usize, usize::saturating_add));
        mesh.saturating_add(points).saturating_add(curves).saturating_add(instances)
    }

    /// Instances turned into real geometry (recursively), joined with the rest.
    pub fn realized(&self) -> Geometry {
        let mut parts = vec![Geometry { mesh: self.mesh.clone(), points: self.points.clone(), curves: self.curves.clone(), instances: None }];
        if let Some(instances) = &self.instances {
            for (at, item) in instances.items.iter().enumerate() {
                let mut part = item.geometry.realized();
                part.transform(item.transform);
                // The instance's own attributes spread over its points.
                for (name, attribute) in &instances.attributes {
                    let Some(value) = attribute.values.get(at).copied() else { continue };
                    if let Some(mesh) = &mut part.mesh {
                        let values = vec![value; mesh.mesh.positions.len()];
                        mesh.attributes.insert(name.clone(), Attribute { domain: Domain::Point, values });
                    }
                    if let Some(points) = &mut part.points {
                        let values = vec![value; points.positions.len()];
                        points.attributes.insert(name.clone(), Attribute { domain: Domain::Point, values });
                    }
                    if let Some(curves) = &mut part.curves {
                        let values = vec![value; curves.points_len()];
                        curves.attributes.insert(name.clone(), Attribute { domain: Domain::Point, values });
                    }
                }
                parts.push(part);
            }
        }
        Geometry::join(parts)
    }

    /// The geometry as a mesh: instances realized, points as loose vertices, curves as wires
    /// through their evaluated points.
    pub fn into_mesh(self) -> Mesh {
        let realized = if self.instances.is_some() { self.realized() } else { self };
        let mut mesh = realized.mesh.map(|component| component.mesh).unwrap_or_default();
        if let Some(curves) = &realized.curves {
            mesh.append(&crate::curve_ops::curve_to_mesh(curves, None, &[], false));
        }
        if let Some(points) = realized.points {
            mesh.positions.extend(points.positions);
            if mesh.has_uvs() {
                mesh.fill_uvs();
            }
        }
        mesh
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kition_scene_model::Primitive;

    #[test]
    fn attributes_move_between_domains() {
        let cube = MeshComponent::new(Primitive::Cube { size: 2.0 }.build().unwrap());
        let heights: Column = cube.mesh.positions.iter().map(|point| Scalar::Float(point.z)).collect();
        let on_faces = cube.adapt(&heights, Domain::Point, Domain::Face);
        assert_eq!(on_faces.len(), 6);
        assert!(on_faces.iter().any(|value| (value.float() - 1.0).abs() < 1e-6));
        let back = cube.adapt(&on_faces, Domain::Face, Domain::Point);
        assert_eq!(back.len(), 8);
        assert_eq!(cube.positions(Domain::Edge).len(), 12);
    }

    #[test]
    fn realizing_places_each_instance() {
        let cube = Geometry::from_mesh(Primitive::Cube { size: 2.0 }.build().unwrap());
        let shared = Arc::new(cube);
        let instances = InstancesComponent {
            items: (0..3)
                .map(|index| Instance { geometry: shared.clone(), transform: Mat4::from_translation(Vec3::X * 3.0 * index as f32) })
                .collect(),
            attributes: Attributes::new(),
        };
        let geometry = Geometry { instances: Some(instances), ..Geometry::default() };
        let mesh = geometry.into_mesh();
        assert_eq!(mesh.positions.len(), 24);
        assert_eq!(mesh.faces.len(), 18);
        assert!((mesh.bounds(Mat4::IDENTITY).max.x - 7.0).abs() < 1e-5);
    }
}
