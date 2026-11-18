//! Primitive texture coordinates against the reference application's layouts
//! (tests/fixtures/reference-uvs.json), and coordinates surviving edits and modifiers.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use std::collections::BTreeSet;

use kition_scene_model::glam::Vec3;
use kition_scene_model::{Mesh, Modifier, ModifierKind, ObjectData, Primitive, Scene, ops};
use serde_json::Value;

fn key(point: &[f64]) -> [i64; 3] {
    [0, 1, 2].map(|axis| (point[axis] * 1000.0).round() as i64)
}

fn our_key(point: Vec3) -> [i64; 3] {
    [point.x, point.y, point.z].map(|value| (f64::from(value) * 1000.0).round() as i64)
}

#[test]
fn primitives_lay_out_coordinates_like_the_reference() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/reference-uvs.json")).unwrap();
    for (name, case) in fixture["cases"].as_object().unwrap() {
        let primitive: Primitive = serde_json::from_value(case["params"].clone()).unwrap();
        let mesh = primitive.build().unwrap();
        mesh.validate().unwrap();
        let faces = case["faces"].as_array().unwrap();
        assert_eq!(mesh.faces.len(), faces.len(), "{name}: face count");
        for reference in faces {
            let corners = reference.as_array().unwrap();
            let mut wanted: BTreeSet<[i64; 3]> =
                corners.iter().map(|corner| key(&serde_json::from_value::<Vec<f64>>(corner[0].clone()).unwrap())).collect();
            let ours = mesh
                .faces
                .iter()
                .position(|face| {
                    let have: BTreeSet<[i64; 3]> = face.iter().map(|vertex| our_key(mesh.positions[*vertex as usize])).collect();
                    have == wanted
                })
                .unwrap_or_else(|| panic!("{name}: no face at {wanted:?}"));
            wanted.clear();
            for corner in corners {
                let position = key(&serde_json::from_value::<Vec<f64>>(corner[0].clone()).unwrap());
                let uv: Vec<f64> = serde_json::from_value(corner[1].clone()).unwrap();
                let slot = mesh.faces[ours].iter().position(|vertex| our_key(mesh.positions[*vertex as usize]) == position).unwrap();
                let got = mesh.corner_uv(ours, slot);
                assert!(
                    (f64::from(got.x) - uv[0]).abs() < 2e-3 && (f64::from(got.y) - uv[1]).abs() < 2e-3,
                    "{name}: face {ours} corner {position:?}: {got} against {uv:?}"
                );
            }
        }
    }
}

fn top(mesh: &Mesh) -> u32 {
    mesh.faces.iter().position(|face| mesh.face_normal(face).z > 0.99).unwrap() as u32
}

#[test]
fn edits_keep_coordinates_valid_and_untouched_faces_exact() {
    let cube = Primitive::Cube { size: 2.0 }.build().unwrap();
    type Edit = Box<dyn Fn(&mut Mesh)>;
    let edits: Vec<(&str, Edit)> = vec![
        (
            "extrude",
            Box::new(|mesh: &mut Mesh| {
                let face = top(mesh);
                ops::extrude_region(mesh, &[face], 1.0, Vec3::ZERO).unwrap();
            }),
        ),
        (
            "inset",
            Box::new(|mesh: &mut Mesh| {
                let face = top(mesh);
                ops::inset(mesh, &[face], 0.2).unwrap();
            }),
        ),
        (
            "subdivide",
            Box::new(|mesh: &mut Mesh| {
                let face = top(mesh);
                ops::subdivide(mesh, &[face]).unwrap();
            }),
        ),
        (
            "loop cut",
            Box::new(|mesh: &mut Mesh| {
                ops::loop_cut(mesh, (0, 1), 2, 0.5).unwrap();
            }),
        ),
        (
            "bevel",
            Box::new(|mesh: &mut Mesh| {
                ops::bevel_edges(mesh, &[(0, 1)], 0.1, 2).unwrap();
            }),
        ),
        (
            "merge",
            Box::new(|mesh: &mut Mesh| {
                ops::merge_at_center(mesh, &[1, 3]).unwrap();
            }),
        ),
        (
            "delete",
            Box::new(|mesh: &mut Mesh| {
                ops::delete_vertices(mesh, &[0]).unwrap();
            }),
        ),
        (
            "flip",
            Box::new(|mesh: &mut Mesh| {
                ops::flip(mesh, &[0, 1]).unwrap();
            }),
        ),
    ];
    for (name, edit) in edits {
        let mut mesh = cube.clone();
        edit(&mut mesh);
        mesh.validate().unwrap_or_else(|error| panic!("{name}: {error}"));
        assert!(mesh.has_uvs(), "{name} dropped the coordinates");
        // Every face that is still a face of the cube keeps its coordinates.
        for (index, face) in mesh.faces.iter().enumerate() {
            let points: BTreeSet<[i64; 3]> = face.iter().map(|vertex| our_key(mesh.positions[*vertex as usize])).collect();
            if let Some(original) = cube
                .faces
                .iter()
                .position(|old| old.iter().map(|vertex| our_key(cube.positions[*vertex as usize])).collect::<BTreeSet<_>>() == points)
            {
                for (corner, vertex) in face.iter().enumerate() {
                    let old_corner =
                        cube.faces[original].iter().position(|old| cube.positions[*old as usize] == mesh.positions[*vertex as usize]).unwrap();
                    assert_eq!(mesh.corner_uv(index, corner), cube.corner_uv(original, old_corner), "{name}: face {index}");
                }
            }
        }
        // New corners on the cube's old surface are interpolated inside the old faces' layout.
        assert!(mesh.uvs.iter().flatten().all(|uv| (-1e-3..=1.001).contains(&uv.x) && (-1e-3..=1.001).contains(&uv.y)), "{name}");
    }
}

#[test]
fn modifiers_carry_coordinates() {
    let mut scene = Scene::empty();
    let id = scene.add_object("Cube", ObjectData::Mesh(Primitive::Cube { size: 2.0 }.build().unwrap()));
    let cutter = scene.add_object("Cutter", ObjectData::Mesh(Primitive::Cube { size: 2.0 }.build().unwrap()));
    scene.object_mut(cutter).unwrap().location = Vec3::splat(1.0);
    for kind in [
        ModifierKind::Mirror { axes: [true, false, false], merge: true, merge_threshold: 0.001 },
        ModifierKind::default_of("array").unwrap(),
        ModifierKind::Subdivision { levels: 2 },
        ModifierKind::default_of("solidify").unwrap(),
        ModifierKind::Triangulate,
        ModifierKind::default_of("bevel").unwrap(),
        ModifierKind::default_of("weld").unwrap(),
        ModifierKind::default_of("smooth").unwrap(),
        serde_json::from_value(serde_json::json!({ "type": "boolean", "object": cutter })).unwrap(),
    ] {
        let name = kind.display_name();
        let object = scene.object_mut(id).unwrap();
        object.modifiers = vec![Modifier::new(kind)];
        let evaluated = scene.object(id).unwrap().evaluated_mesh(&scene).unwrap();
        evaluated.validate().unwrap_or_else(|error| panic!("{name}: {error}"));
        assert!(evaluated.has_uvs(), "{name} dropped the coordinates");
    }
    // Subdivision keeps a seam's two sides apart and corners exact.
    let cube = Primitive::Cube { size: 2.0 }.build().unwrap();
    let once = kition_scene_model::modifiers::catmull_clark(&cube);
    for (face, uvs) in once.uvs.iter().enumerate() {
        let source = face / 4;
        let corner = face % 4;
        assert_eq!(uvs[0], cube.corner_uv(source, corner));
    }
}
