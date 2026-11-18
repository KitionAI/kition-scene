//! Every primitive, with its default parameters, against the reference application's counts and
//! dimensions (tools/blender-reference/reference.py records them).
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use kition_scene_model::glam::Mat4;
use kition_scene_model::modifiers::boundary_edges;
use kition_scene_model::{Mesh, Primitive};

fn defaults(kind: &str) -> Mesh {
    let primitive: Primitive = serde_json::from_value(serde_json::json!({ "type": kind })).unwrap();
    primitive.build().unwrap()
}

#[test]
fn defaults_match_the_reference_counts_and_sizes() {
    // kind, vertices, edges, faces, dimensions.
    let expected: &[(&str, usize, usize, usize, [f32; 3])] = &[
        ("plane", 4, 4, 1, [2.0, 2.0, 0.0]),
        ("cube", 8, 12, 6, [2.0, 2.0, 2.0]),
        ("circle", 32, 32, 0, [2.0, 2.0, 0.0]),
        ("uvSphere", 482, 992, 512, [2.0, 2.0, 2.0]),
        ("icoSphere", 42, 120, 80, [1.9021, 2.0, 2.0]),
        ("cylinder", 64, 96, 34, [2.0, 2.0, 2.0]),
        ("cone", 33, 64, 33, [2.0, 2.0, 2.0]),
        ("torus", 576, 1152, 576, [2.5, 2.5, 0.5]),
        ("grid", 121, 220, 100, [2.0, 2.0, 0.0]),
    ];
    for (kind, vertices, edges, faces, size) in expected {
        let mesh = defaults(kind);
        mesh.validate().unwrap();
        assert_eq!((mesh.positions.len(), mesh.edges().len(), mesh.faces.len()), (*vertices, *edges, *faces), "{kind}: counts");
        let got = mesh.bounds(Mat4::IDENTITY).size();
        for axis in 0..3 {
            assert!((got[axis] - size[axis]).abs() < 2e-3, "{kind}: size {got} against {size:?}");
        }
    }
}

#[test]
fn closed_primitives_are_closed_and_face_outwards() {
    for kind in ["cube", "uvSphere", "icoSphere", "cylinder", "cone", "torus"] {
        let mesh = defaults(kind);
        assert!(boundary_edges(&mesh).is_empty(), "{kind} has open edges");
        let mut check = mesh.clone();
        kition_scene_model::ops::recalculate_outside(&mut check);
        assert_eq!(check.faces, mesh.faces, "{kind} has a face pointing inwards");
    }
}

#[test]
fn hostile_parameters_are_refused() {
    for params in [
        serde_json::json!({ "type": "cube", "size": -1.0 }),
        serde_json::json!({ "type": "uvSphere", "segments": 2 }),
        serde_json::json!({ "type": "icoSphere", "subdivisions": 40 }),
        serde_json::json!({ "type": "cone", "radius1": 0.0, "radius2": 0.0 }),
        serde_json::json!({ "type": "torus", "majorSegments": 1000, "minorSegments": 1000, "minorRadius": 0.1 }),
    ] {
        let primitive: Primitive = serde_json::from_value(params.clone()).unwrap();
        assert!(primitive.build().is_err(), "{params} was accepted");
    }
}
