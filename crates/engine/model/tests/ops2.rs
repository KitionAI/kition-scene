//! The second set of Edit Mode operations against the reference application's results for the
//! same mesh and selection: vertex, edge, and face counts, face sizes, and bounds.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::collections::BTreeSet;

use kition_scene_model::glam::{Mat4, Vec3};
use kition_scene_model::modifiers::boundary_edges;
use kition_scene_model::{Fill, Mesh, Primitive, ops2};

fn cube() -> Mesh {
    Primitive::Cube { size: 2.0 }.build().unwrap()
}

fn set(values: &[u32]) -> BTreeSet<u32> {
    values.iter().copied().collect()
}

/// Vertices, edges, faces, and sorted face sizes.
fn counts(mesh: &Mesh) -> (usize, usize, usize, Vec<usize>) {
    mesh.validate().unwrap();
    let mut sizes: Vec<usize> = mesh.faces.iter().map(Vec::len).collect();
    sizes.sort_unstable();
    (mesh.positions.len(), mesh.edges().len(), mesh.faces.len(), sizes)
}

fn bounds(mesh: &Mesh, min: [f32; 3], max: [f32; 3]) {
    let found = mesh.bounds(Mat4::IDENTITY);
    assert!(found.min.abs_diff_eq(Vec3::from(min), 1e-3) && found.max.abs_diff_eq(Vec3::from(max), 1e-3), "{found:?}");
}

#[test]
fn extruding_vertices_edges_and_single_faces() {
    let mut mesh = cube();
    ops2::extrude_loose(&mut mesh, &set(&[7]), Vec3::Z).unwrap();
    assert_eq!(counts(&mesh), (9, 13, 6, vec![4; 6]));
    bounds(&mesh, [-1.0; 3], [1.0, 1.0, 2.0]);

    let mut mesh = cube();
    ops2::extrude_loose(&mut mesh, &set(&[3, 7]), Vec3::Z).unwrap();
    assert_eq!(counts(&mesh), (10, 15, 7, vec![4; 7]));

    let mut mesh = cube();
    ops2::extrude_individual(&mut mesh, &[5, 1], 0.5).unwrap();
    assert_eq!(counts(&mesh), (16, 28, 14, vec![4; 14]));
    bounds(&mesh, [-1.0; 3], [1.0, 1.5, 1.5]);
    assert!(boundary_edges(&mesh).is_empty());
}

#[test]
fn dissolving_vertices_edges_and_faces() {
    let mut grid = Primitive::Grid { x_subdivisions: 2, y_subdivisions: 2, size: 2.0 }.build().unwrap();
    ops2::dissolve_vertices(&mut grid, &set(&[4])).unwrap();
    assert_eq!(counts(&grid), (8, 8, 1, vec![8]));

    let mut mesh = cube();
    ops2::dissolve_edges(&mut mesh, &[(3, 7)]).unwrap();
    assert_eq!(counts(&mesh), (6, 9, 5, vec![3, 3, 4, 4, 4]));
    assert!(boundary_edges(&mesh).is_empty());

    let mut mesh = cube();
    ops2::dissolve_faces(&mut mesh, &[5, 1]).unwrap();
    assert_eq!(counts(&mesh), (8, 11, 5, vec![4, 4, 4, 4, 6]));
    assert!(boundary_edges(&mesh).is_empty());
}

#[test]
fn bridging_two_rings() {
    let mut mesh = Primitive::Circle { vertices: 6, radius: 1.0, fill: Fill::Nothing }.build().unwrap();
    let mut top = mesh.clone();
    for point in &mut top.positions {
        point.z += 2.0;
    }
    mesh.append(&top);
    let all: BTreeSet<u32> = (0..mesh.vertex_count()).collect();
    let made = ops2::bridge(&mut mesh, &all).unwrap();
    assert_eq!(made.len(), 6);
    assert_eq!(counts(&mesh), (12, 18, 6, vec![4; 6]));
    bounds(&mesh, [-0.866, -1.0, 0.0], [0.866, 1.0, 2.0]);
}

#[test]
fn bisecting_with_and_without_clearing() {
    let mut mesh = cube();
    ops2::bisect(&mut mesh, Vec3::new(0.0, 0.0, 0.3), Vec3::Z, false, false, false).unwrap();
    assert_eq!(counts(&mesh), (12, 20, 10, vec![4; 10]));

    let mut mesh = cube();
    let cut = ops2::bisect(&mut mesh, Vec3::new(0.0, 0.0, 0.3), Vec3::Z, false, true, true).unwrap();
    assert_eq!(counts(&mesh), (8, 12, 6, vec![4; 6]));
    bounds(&mesh, [-1.0; 3], [1.0, 1.0, 0.3]);
    assert!(boundary_edges(&mesh).is_empty(), "the fill closes the cut");
    assert_eq!(cut.len(), 4);
    let mut check = mesh.clone();
    kition_scene_model::ops::recalculate_outside(&mut check);
    assert_eq!(check.faces, mesh.faces, "the fill faces outwards");

    let mut mesh = cube();
    ops2::bisect(&mut mesh, Vec3::new(0.0, 0.0, 0.3), Vec3::Z, true, false, false).unwrap();
    assert_eq!(counts(&mesh), (8, 12, 5, vec![4; 5]));
    bounds(&mesh, [-1.0, -1.0, 0.3], [1.0; 3]);
}

#[test]
fn spinning_a_ring() {
    let circle = Primitive::Circle { vertices: 8, radius: 1.0, fill: Fill::Nothing }.build().unwrap();
    let all: BTreeSet<u32> = (0..8).collect();
    let mut quarter = circle.clone();
    ops2::spin(&mut quarter, &all, 4, 90f32.to_radians(), Vec3::Y, Vec3::new(2.0, 0.0, 0.0)).unwrap();
    assert_eq!(counts(&quarter), (40, 72, 32, vec![4; 32]));
    bounds(&quarter, [-1.0, -1.0, -3.0], [2.0, 1.0, 0.0]);

    let mut whole = circle;
    ops2::spin(&mut whole, &all, 8, 360f32.to_radians(), Vec3::Y, Vec3::new(2.0, 0.0, 0.0)).unwrap();
    assert_eq!(counts(&whole), (64, 128, 64, vec![4; 64]));
    bounds(&whole, [-1.0, -1.0, -3.0], [5.0, 1.0, 3.0]);
    assert!(boundary_edges(&whole).is_empty(), "a whole turn closes into a torus");
}

#[test]
fn bevelling_vertices() {
    let mut mesh = cube();
    ops2::bevel_vertices(&mut mesh, &(0..8).collect(), 0.2).unwrap();
    let mut sizes = vec![3; 8];
    sizes.extend([8; 6]);
    assert_eq!(counts(&mesh), (24, 36, 14, sizes));
    assert!(boundary_edges(&mesh).is_empty());

    let mut mesh = cube();
    ops2::bevel_vertices(&mut mesh, &set(&[7]), 0.2).unwrap();
    assert_eq!(counts(&mesh), (10, 15, 7, vec![3, 4, 4, 4, 5, 5, 5]));
}

#[test]
fn symmetrizing() {
    let mut mesh = cube();
    ops2::symmetrize(&mut mesh, 0, true).unwrap();
    assert_eq!(counts(&mesh), (12, 20, 10, vec![4; 10]));
    assert!(boundary_edges(&mesh).is_empty());

    let mut sphere = Primitive::UvSphere { segments: 8, rings: 4, radius: 1.0 }.build().unwrap();
    for point in &mut sphere.positions {
        point.x += 0.3;
    }
    ops2::symmetrize(&mut sphere, 0, false).unwrap();
    let (vertices, edges, faces, _) = counts(&sphere);
    assert_eq!((vertices, edges, faces), (30, 60, 32));
    bounds(&sphere, [-0.7, -0.8757, -0.8757], [0.7, 0.8757, 0.8757]);
}

#[test]
fn splitting_a_selection_off() {
    let mut mesh = cube();
    let top: BTreeSet<u32> = set(&[1, 3, 5, 7]);
    let part = ops2::split_off(&mesh, &top);
    assert_eq!(counts(&part), (4, 4, 1, vec![4]));
    ops2::remove_split(&mut mesh, &top);
    assert_eq!(counts(&mesh), (8, 12, 5, vec![4; 5]), "the shared vertices stay with the sides");
}
