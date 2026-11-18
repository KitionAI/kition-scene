//! Node groups run end to end: geometry flows through nodes, fields evaluate per domain, and
//! the modifier's output is a valid mesh.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

mod common;

use std::collections::BTreeMap;

use common::{INPUT, Tree};
use kition_scene_model::glam::{Mat4, Vec3};
use kition_scene_model::{GroupSocket, Mesh, Primitive, SocketType};
use serde_json::json;

fn cube() -> Mesh {
    Primitive::Cube { size: 2.0 }.build().unwrap()
}

fn bounds(mesh: &Mesh) -> (Vec3, Vec3) {
    let bounds = mesh.bounds(Mat4::IDENTITY);
    (bounds.min, bounds.max)
}

#[test]
fn a_new_group_passes_the_mesh_through() {
    let mut tree = Tree::new();
    tree.output(INPUT, "Geometry");
    let mesh = cube();
    assert_eq!(tree.run(mesh.clone()).unwrap(), mesh);
}

#[test]
fn an_unlinked_output_is_empty_and_a_cycle_is_an_error() {
    let tree = Tree::new();
    assert!(tree.run(cube()).unwrap().positions.is_empty());
    let mut tree = Tree::new();
    let a = tree.add("transform", json!({}));
    let b = tree.add("transform", json!({}));
    tree.link(a, "Geometry", b, "Geometry");
    tree.link(b, "Geometry", a, "Geometry");
    tree.output(b, "Geometry");
    assert!(tree.run(cube()).unwrap_err().contains("cycle"));
}

#[test]
fn scatter_instance_and_realize() {
    let mut tree = Tree::new();
    let grid = tree.add("meshGrid", json!({ "Size X": 4.0, "Size Y": 4.0 }));
    let scatter = tree.add("distributePointsOnFaces", json!({ "Density": 10.0, "Seed": 0 }));
    let cube = tree.add("meshCube", json!({ "Size": [0.1, 0.1, 0.1] }));
    let instance = tree.add("instanceOnPoints", json!({}));
    let realize = tree.add("realizeInstances", json!({}));
    tree.link(grid, "Mesh", scatter, "Mesh");
    tree.link(scatter, "Points", instance, "Points");
    tree.link(cube, "Mesh", instance, "Instance");
    tree.link(instance, "Instances", realize, "Geometry");
    tree.output(realize, "Geometry");
    let mesh = tree.run(Mesh::default()).unwrap();
    let points = mesh.positions.len() / 8;
    assert_eq!(mesh.positions.len() % 8, 0);
    assert_eq!(mesh.faces.len(), points * 6);
    assert!((points as f32 - 160.0).abs() < 40.0, "{points}");
    let (low, high) = bounds(&mesh);
    assert!(low.x >= -2.05 - 1e-4 && high.x <= 2.05 + 1e-4 && (high.z - 0.05).abs() < 1e-4);
    // The same seed scatters the same way; another seed does not.
    assert_eq!(tree.run(Mesh::default()).unwrap(), mesh);
    tree.group.node_mut(scatter).unwrap().values.insert("Seed".into(), json!(1));
    assert_ne!(tree.run(Mesh::default()).unwrap(), mesh);
}

#[test]
fn random_offsets_spread_evenly() {
    let mut tree = Tree::new();
    let cube = tree.add("meshCube", json!({}));
    let subdivide = tree.add("subdivideMesh", json!({ "Level": 2 }));
    let random = tree.add("randomValue", json!({ "dataType": "float_vector", "Min_001": [-0.1, -0.1, -0.1], "Max_001": [0.1, 0.1, 0.1] }));
    let set = tree.add("setPosition", json!({}));
    tree.link(cube, "Mesh", subdivide, "Mesh");
    tree.link(subdivide, "Mesh", set, "Geometry");
    tree.link(random, "Value", set, "Offset");
    tree.output(set, "Geometry");
    let mesh = tree.run(Mesh::default()).unwrap();
    assert_eq!((mesh.positions.len(), mesh.faces.len()), (98, 96));
    let offsets: Vec<f32> = mesh.positions.iter().flat_map(|point| (*point - (*point * 4.0).round() / 4.0).to_array()).collect();
    assert!(offsets.iter().all(|offset| offset.abs() <= 0.1 + 1e-6));
    let mean = offsets.iter().sum::<f32>() / offsets.len() as f32;
    let spread = (offsets.iter().map(|offset| (offset - mean).powi(2)).sum::<f32>() / offsets.len() as f32).sqrt();
    assert!(mean.abs() < 0.015 && (spread - 0.0577).abs() < 0.01, "{mean} {spread}");
}

#[test]
fn extrusion_moves_the_top_faces() {
    let mut tree = Tree::new();
    let cube = tree.add("meshCube", json!({}));
    let extrude = tree.add("extrudeMesh", json!({ "Offset Scale": 0.5, "Individual": false }));
    tree.link(cube, "Mesh", extrude, "Mesh");
    tree.output(extrude, "Mesh");
    let mesh = tree.run(Mesh::default()).unwrap();
    assert_eq!((mesh.positions.len(), mesh.faces.len()), (8, 6));
    assert!((bounds(&mesh).1.x - 2.0 / 3.0).abs() < 1e-5);
    // Only the top face, by comparing its normal: a column rises from it.
    let normal = tree.add("normal", json!({}));
    let separate = tree.add("separateXyz", json!({}));
    let compare = tree.add("compare", json!({ "operation": "greater_than", "B": 0.5 }));
    tree.link(normal, "Normal", separate, "Vector");
    tree.link(separate, "Z", compare, "A");
    tree.link(compare, "Result", extrude, "Selection");
    let vector = tree.add("vector", json!({ "Vector": [0.0, 0.0, 1.0] }));
    tree.link(vector, "Vector", extrude, "Offset");
    tree.group.node_mut(extrude).unwrap().values.insert("Offset Scale".into(), json!(1.0));
    let mesh = tree.run(Mesh::default()).unwrap();
    assert_eq!((mesh.positions.len(), mesh.faces.len()), (12, 10));
    assert!((bounds(&mesh).1.z - 1.5).abs() < 1e-5);
}

#[test]
fn group_inputs_come_from_the_modifier() {
    let mut tree = Tree::new();
    tree.group.inputs.push(GroupSocket { name: "Height".into(), kind: SocketType::Float, default: json!(1.0) });
    let combine = tree.add("combineXyz", json!({}));
    let transform = tree.add("transform", json!({}));
    tree.link(INPUT, "Height", combine, "Z");
    tree.link(INPUT, "Geometry", transform, "Geometry");
    tree.link(combine, "Vector", transform, "Translation");
    tree.output(transform, "Geometry");
    assert!((bounds(&tree.run(cube()).unwrap()).1.z - 2.0).abs() < 1e-6);
    let inputs = BTreeMap::from([("Height".to_owned(), json!(3.0))]);
    assert!((bounds(&tree.run_with(cube(), &inputs).unwrap()).1.z - 4.0).abs() < 1e-6);
}

#[test]
fn fields_follow_their_domain() {
    // Lift only the vertices with a positive X, by an amount from Map Range of their index.
    let mut tree = Tree::new();
    let grid = tree.add("meshGrid", json!({ "Vertices X": 5, "Vertices Y": 2 }));
    let position = tree.add("position", json!({}));
    let separate = tree.add("separateXyz", json!({}));
    let compare = tree.add("compare", json!({ "operation": "greater_than" }));
    let index = tree.add("index", json!({}));
    let map = tree.add("mapRange", json!({ "From Max": 9.0, "To Max": 1.0 }));
    let combine = tree.add("combineXyz", json!({}));
    let set = tree.add("setPosition", json!({}));
    tree.link(grid, "Mesh", set, "Geometry");
    tree.link(position, "Position", separate, "Vector");
    tree.link(separate, "X", compare, "A");
    tree.link(compare, "Result", set, "Selection");
    tree.link(index, "Index", map, "Value");
    tree.link(map, "Result", combine, "Z");
    tree.link(combine, "Vector", set, "Offset");
    tree.output(set, "Geometry");
    let mesh = tree.run(Mesh::default()).unwrap();
    for (at, point) in mesh.positions.iter().enumerate() {
        let expected = if point.x > 0.0 { at as f32 / 9.0 } else { 0.0 };
        assert!((point.z - expected).abs() < 1e-5, "{at}: {point}");
    }
}

#[test]
fn delete_merge_and_points() {
    // Delete the faces facing up, then turn what is left into points and back into vertices.
    let mut tree = Tree::new();
    let normal = tree.add("normal", json!({}));
    let separate = tree.add("separateXyz", json!({}));
    let compare = tree.add("compare", json!({ "operation": "greater_than", "B": 0.5 }));
    let delete = tree.add("deleteGeometry", json!({ "domain": "face" }));
    tree.link(INPUT, "Geometry", delete, "Geometry");
    tree.link(normal, "Normal", separate, "Vector");
    tree.link(separate, "Z", compare, "A");
    tree.link(compare, "Result", delete, "Selection");
    tree.output(delete, "Geometry");
    let open = tree.run(cube()).unwrap();
    assert_eq!((open.positions.len(), open.faces.len()), (8, 5));
    let points = tree.add("meshToPoints", json!({ "mode": "faces" }));
    let vertices = tree.add("pointsToVertices", json!({}));
    tree.group.links.retain(|link| link.to != common::OUTPUT);
    tree.link(delete, "Geometry", points, "Mesh");
    tree.link(points, "Points", vertices, "Points");
    tree.output(vertices, "Mesh");
    let centres = tree.run(cube()).unwrap();
    assert_eq!((centres.positions.len(), centres.faces.len()), (5, 0));
    // Merging two copies of a cube gives one cube's vertices back.
    let mut tree = Tree::new();
    let join = tree.add("joinGeometry", json!({}));
    let merge = tree.add("mergeByDistance", json!({}));
    tree.link(INPUT, "Geometry", join, "Geometry");
    tree.link(INPUT, "Geometry", join, "Geometry");
    tree.link(join, "Geometry", merge, "Geometry");
    tree.output(merge, "Geometry");
    let merged = tree.run(cube()).unwrap();
    assert_eq!(merged.positions.len(), 8);
}

#[test]
fn named_attributes_become_vertex_groups() {
    let mut tree = Tree::new();
    let position = tree.add("position", json!({}));
    let separate = tree.add("separateXyz", json!({}));
    let store = tree.add("storeNamedAttribute", json!({ "Name": "height" }));
    tree.link(INPUT, "Geometry", store, "Geometry");
    tree.link(position, "Position", separate, "Vector");
    tree.link(separate, "Z", store, "Value");
    tree.output(store, "Geometry");
    let mesh = tree.run(cube()).unwrap();
    let group = kition_scene_model::vertex_data::group(&mesh, "height").unwrap();
    for (weight, point) in group.weights.iter().zip(&mesh.positions) {
        assert_eq!(*weight, point.z.clamp(0.0, 1.0));
    }
}

#[test]
fn switch_capture_math_and_primitives() {
    let mut tree = Tree::new();
    let cylinder = tree.add("meshCylinder", json!({ "Vertices": 8 }));
    let sphere = tree.add("meshUvSphere", json!({ "Segments": 8, "Rings": 4 }));
    let switch = tree.add("switch", json!({ "inputType": "geometry", "Switch": true }));
    tree.link(cylinder, "Mesh", switch, "False");
    tree.link(sphere, "Mesh", switch, "True");
    let math = tree.add("math", json!({ "operation": "multiply", "Value": 2.0, "Value_001": 3.0 }));
    let combine = tree.add("combineXyz", json!({}));
    let transform = tree.add("transform", json!({}));
    tree.link(math, "Value", combine, "X");
    tree.link(switch, "Output", transform, "Geometry");
    tree.link(combine, "Vector", transform, "Translation");
    tree.output(transform, "Geometry");
    let mesh = tree.run(Mesh::default()).unwrap();
    assert_eq!(mesh.positions.len(), 8 * 3 + 2);
    assert!((bounds(&mesh).1.x - 7.0).abs() < 1e-5);
    // Capture the cylinder's Top selection before turning it into points; only top faces remain.
    let mut tree = Tree::new();
    let cylinder = tree.add("meshCylinder", json!({ "Vertices": 8 }));
    let delete = tree.add("deleteGeometry", json!({ "domain": "face" }));
    let not = tree.add("math", json!({ "operation": "subtract", "Value": 1.0 }));
    tree.link(cylinder, "Mesh", delete, "Geometry");
    tree.link(cylinder, "Top", not, "Value_001");
    tree.link(not, "Value", delete, "Selection");
    tree.output(delete, "Geometry");
    let top = tree.run(Mesh::default()).unwrap();
    assert_eq!((top.positions.len(), top.faces.len()), (8, 1));
}

#[test]
fn object_info_reads_another_object() {
    let mut tree = Tree::new();
    let info = tree.add("objectInfo", json!({ "transformSpace": "relative" }));
    tree.output(info, "Geometry");
    let (mut scene, own) = tree.scene(Mesh::default());
    let other = scene.add_object("Other", kition_scene_model::ObjectData::Mesh(cube()));
    scene.object_mut(other).unwrap().location = Vec3::new(5.0, 0.0, 0.0);
    scene.node_groups[0].node_mut(info).unwrap().values.insert("Object".into(), json!(other));
    scene.object_mut(own).unwrap().modifiers.push(common::modifier());
    kition_scene_nodes::install();
    let object = scene.object(own).unwrap();
    let mesh = object.evaluated_mesh(&scene).unwrap();
    assert_eq!(mesh.positions.len(), 8);
    assert!((bounds(&mesh).0.x - 4.0).abs() < 1e-5);
    // An object reading itself gets nothing (and does not loop).
    scene.node_groups[0].node_mut(info).unwrap().values.insert("Object".into(), json!(own));
    assert!(scene.object(own).unwrap().evaluated_mesh(&scene).unwrap().positions.is_empty());
}

#[test]
fn curves_reach_the_modifier_as_wires() {
    // A modifier's output is a mesh: curves become their evaluated points joined by edges.
    let mut tree = Tree::new();
    let circle = tree.add("curveCircle", json!({ "Resolution": 6 }));
    let segment = tree.add("curveBezierSegment", json!({ "Resolution": 4 }));
    let join = tree.add("joinGeometry", json!({}));
    tree.link(circle, "Curve", join, "Geometry");
    tree.link(segment, "Curve", join, "Geometry");
    tree.output(join, "Geometry");
    let mesh = tree.run(cube()).unwrap();
    assert_eq!((mesh.positions.len(), mesh.loose_edges.len(), mesh.faces.len()), (11, 10, 0));
    mesh.validate().unwrap();
}

#[test]
fn hostile_curve_settings_fail_cleanly() {
    let run = |kind: &str, values: serde_json::Value| {
        let mut tree = Tree::new();
        let line = tree.add("curveLine", json!({}));
        let node = tree.add(kind, values);
        tree.link(line, "Curve", node, "Curve");
        tree.output(node, "Curve");
        tree.run(cube())
    };
    assert!(run("resampleCurve", json!({ "Count": 2_000_000_000 })).is_err(), "a resample past the element limit is an error");
    assert!(run("resampleCurve", json!({ "mode": "length", "Length": 0.0 })).is_ok());
    let trimmed = run("trimCurve", json!({ "mode": "length", "Start_001": f64::MAX, "End_001": -1.0 })).unwrap();
    assert!(trimmed.positions.iter().all(|point| point.is_finite()));
    let mut tree = Tree::new();
    let spiral = tree.add("curveSpiral", json!({ "Resolution": 1_000_000, "Rotations": 1_000_000.0 }));
    tree.output(spiral, "Curve");
    assert!(tree.run(cube()).is_err(), "a spiral past the element limit is an error");
    let mut tree = Tree::new();
    let circle =
        tree.add("curveCircle", json!({ "mode": "points", "Point 1": [0.0, 0.0, 0.0], "Point 2": [1.0, 1.0, 1.0], "Point 3": [2.0, 2.0, 2.0] }));
    tree.output(circle, "Curve");
    assert!(tree.run(cube()).unwrap().positions.is_empty(), "three points in a line make no circle");
    let mut tree = Tree::new();
    let circle = tree.add("curveCircle", json!({ "Resolution": 100_000 }));
    let fill = tree.add("fillCurve", json!({}));
    tree.link(circle, "Curve", fill, "Curve");
    tree.output(fill, "Mesh");
    assert!(tree.run(cube()).is_err(), "a fill past its limit is an error");
}
