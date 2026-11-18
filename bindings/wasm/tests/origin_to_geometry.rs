//! Origin to Geometry keeps the mesh and the object's children where they are in the world and
//! moves only the origin, to the centre of the mesh's bounds (case
//! `regressions/issue-origin-to-geometry`, found by the independent reading of C8-scene-read: the mesh
//! was moved by its centre instead of back by it, so the geometry and every child jumped by twice
//! the centre).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use glam::Vec3;
use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::json;

fn world_points(doc: &SceneDocument, name: &str) -> Vec<Vec3> {
    let scene = doc.scene();
    let object = scene.object_named(name).unwrap();
    let world = scene.world_matrix(object.id).unwrap();
    object.mesh().unwrap().positions.iter().map(|point| world.transform_point3(*point)).collect()
}

fn world_origin(doc: &SceneDocument, name: &str) -> Vec3 {
    let scene = doc.scene();
    scene.world_matrix(scene.object_named(name).unwrap().id).unwrap().transform_point3(Vec3::ZERO)
}

#[test]
fn origin_to_geometry_moves_the_origin_and_nothing_in_the_world() {
    let mut doc = SceneDocument::create("Scene.kscene", 320, 180).unwrap();
    // A 1 m cube whose geometry sits 2 m along x from its origin, the object at [0, 0, 1] turned
    // 90 degrees about z; a child 1 m above it.
    let shifted: Vec<[f32; 3]> = (0..8)
        .map(|n| [2.0 + if n & 1 == 0 { -0.5 } else { 0.5 }, if n & 2 == 0 { -0.5 } else { 0.5 }, if n & 4 == 0 { -0.5 } else { 0.5 }])
        .collect();
    let faces = json!([[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]]);
    doc.execute("add.cube", json!({ "name": "Box", "location": [0, 0, 1], "rotation": [0, 0, 90] })).unwrap();
    doc.execute("mesh.setGeometry", json!({ "object": "Box", "positions": shifted, "faces": faces })).unwrap();
    doc.execute("add.empty", json!({ "name": "Tag", "location": [0, 0, 2] })).unwrap();
    doc.execute("object.parent", json!({ "objects": ["Tag"], "parent": "Box" })).unwrap();
    let (points, tag) = (world_points(&doc, "Box"), world_origin(&doc, "Tag"));

    doc.execute("object.originToGeometry", json!({ "objects": ["Box"] })).unwrap();

    let close = |a: Vec3, b: Vec3| a.distance(b) < 1e-4;
    assert!(world_points(&doc, "Box").iter().zip(&points).all(|(now, was)| close(*now, *was)), "the mesh stays where it was in the world");
    assert!(close(world_origin(&doc, "Tag"), tag), "the child stays where it was: {} vs {tag}", world_origin(&doc, "Tag"));
    // The geometry's centre, local [2, 0, 0], turned 90 degrees about z and lifted 1 m.
    assert!(close(world_origin(&doc, "Box"), Vec3::new(0.0, 2.0, 1.0)), "the origin is at the geometry's centre: {}", world_origin(&doc, "Box"));
    let scene = doc.scene();
    let mesh = scene.object_named("Box").unwrap().mesh().unwrap();
    let centre = mesh.positions.iter().fold(Vec3::ZERO, |sum, point| sum + *point) / mesh.positions.len() as f32;
    assert!(close(centre, Vec3::ZERO), "the geometry is centred on the origin: {centre}");
}
