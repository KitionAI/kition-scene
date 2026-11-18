//! The Scene kit's geometry node evaluator. A Geometry Nodes modifier names a node group of the
//! scene; this crate runs it on the modifier's input mesh. Install it once with [`install`]
//! (the model crate holds only the data and calls whatever evaluator is installed).
//!
//! Geometry carries a mesh, a point cloud, curves, and instances, with attributes on points,
//! edges, faces, corners, curves, and instances. Numbers and vectors flow between nodes as
//! fields, evaluated on the domain of the geometry node that reads them. What comes out of the
//! group becomes the modifier's mesh: instances are realized, points become loose vertices,
//! curves become wires through their evaluated points, and named number attributes on points
//! become vertex groups.

pub mod curve_ops;
pub mod curves;
pub mod eval;
mod eval_more;
pub mod field;
pub mod geometry;
pub mod mesh_ops;
pub mod ops;
pub mod prims;
pub mod random;
pub mod sample;
mod spline_limits;
pub mod spline_types;
pub mod zones;

use kition_scene_model::Mesh;
use kition_scene_model::modifiers::{NodesCall, install_nodes_evaluator};

use crate::field::Scalar;
use crate::geometry::{Domain, Geometry, MeshComponent};

/// Installs this evaluator for Geometry Nodes modifiers (once per process; later calls do
/// nothing).
pub fn install() {
    install_nodes_evaluator(evaluate);
}

/// Runs the call's node group on its mesh.
pub fn evaluate(call: &NodesCall<'_>) -> Result<Mesh, String> {
    if call.depth > eval::MAX_DEPTH {
        return Err("objects read each other's geometry too deeply".into());
    }
    let group = call.scene.node_group(call.group).ok_or("the node group is missing")?;
    let mut input = MeshComponent::new(call.mesh.clone());
    // Shape keys are already mixed in; the mask is for sculpting only.
    input.mesh.shape_keys.clear();
    input.mesh.mask.clear();
    ops::groups_to_attributes(&mut input);
    let output = eval::Evaluator::new(call, group, Geometry { mesh: Some(input), ..Geometry::default() }).run()?;
    let mesh = finish(output);
    mesh.validate().map_err(|error| error.to_string())?;
    Ok(mesh)
}

/// The group's output as a mesh.
pub fn finish(output: Geometry) -> Mesh {
    let realized = if output.instances.is_some() { output.realized() } else { output };
    let mut component = realized.mesh.unwrap_or_default();
    if let Some(points) = realized.points {
        let mut cloud = MeshComponent::new(Mesh { positions: points.positions, ..Mesh::default() });
        cloud.attributes = points.attributes;
        component.append(&cloud);
    }
    if let Some(curves) = &realized.curves {
        component.append(&MeshComponent::new(curve_ops::curve_to_mesh(curves, None, &[], false)));
    }
    for attribute in component.attributes.values_mut() {
        if attribute.domain == Domain::Point {
            let zero = attribute.values.first().map_or(Scalar::Float(0.0), |value| Scalar::zero(value.kind()));
            attribute.values.resize(component.mesh.positions.len(), zero);
        }
    }
    let mut mesh = std::mem::take(&mut component.mesh);
    ops::attributes_to_groups(&component, &mut mesh);
    if !mesh.uvs_aligned() {
        mesh.uvs.clear();
    }
    mesh
}
