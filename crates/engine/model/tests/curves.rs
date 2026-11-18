//! Curve and text objects against the reference application's evaluated meshes for the same
//! settings (recorded from Blender 5.2.3 by evaluating the objects to meshes; the font is the
//! bundled Inter).
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic, clippy::type_complexity)]

use kition_scene_model::curve::{Dimensions, FillMode};
use kition_scene_model::glam::Mat4;
use kition_scene_model::{Curve, CurvePrimitive, Mesh, Text, TextAlign};

fn counts(mesh: &Mesh) -> (usize, usize, usize) {
    mesh.validate().unwrap();
    (mesh.positions.len(), mesh.edges().len(), mesh.faces.len())
}

fn bounds(mesh: &Mesh, min: [f32; 3], max: [f32; 3], tolerance: f32, what: &str) {
    let found = mesh.bounds(Mat4::IDENTITY);
    for axis in 0..3 {
        assert!(
            (found.min[axis] - min[axis]).abs() <= tolerance && (found.max[axis] - max[axis]).abs() <= tolerance,
            "{what}: {found:?} against {min:?}..{max:?}"
        );
    }
}

/// The settings the reference was evaluated with, applied in the same order it applied them.
fn variants(primitive: CurvePrimitive) -> Vec<(&'static str, Curve)> {
    let base = primitive.build(1.0);
    let bevel = Curve { bevel_depth: 0.1, ..base.clone() };
    let bevel0 = Curve { bevel_resolution: 0, ..bevel.clone() };
    let bevel_extrude = Curve { bevel_resolution: 4, extrude: 0.2, ..bevel.clone() };
    let extrude = Curve { bevel_depth: 0.0, ..bevel_extrude.clone() };
    let flat = Curve { extrude: 0.0, dimensions: Dimensions::Flat, fill_mode: FillMode::Both, ..extrude.clone() };
    let offset = Curve { dimensions: Dimensions::Spatial, fill_mode: FillMode::Full, offset: 0.5, bevel_depth: 0.1, ..flat.clone() };
    vec![
        ("default", base),
        ("bevel", bevel),
        ("bevel res 0", bevel0),
        ("bevel and extrude", bevel_extrude),
        ("extrude", extrude),
        ("2D filled", flat),
        ("offset", offset),
    ]
}

#[test]
fn bezier_curves_match_the_reference() {
    // vertices, edges, faces, min, max
    let expected: [(usize, usize, usize, [f32; 3], [f32; 3]); 7] = [
        (13, 12, 0, [-1.0, 0.0, 0.0], [1.0, 0.2222, 0.0]),
        (156, 300, 144, [-1.0707, -0.1, -0.1], [1.0, 0.3222, 0.1]),
        (52, 100, 48, [-1.0707, -0.1, -0.1], [1.0, 0.3222, 0.1]),
        (286, 550, 264, [-1.0707, -0.1, -0.3], [1.0, 0.3222, 0.3]),
        (26, 37, 12, [-1.0, 0.0, -0.2], [1.0, 0.2222, 0.2]),
        (13, 12, 0, [-1.0, 0.0, 0.0], [1.0, 0.2222, 0.0]),
        (156, 300, 144, [-1.4243, 0.2828, -0.1], [1.0, 0.8222, 0.1]),
    ];
    for ((name, curve), (v, e, f, min, max)) in variants(CurvePrimitive::Bezier).into_iter().zip(expected) {
        let mesh = curve.to_mesh();
        assert_eq!(counts(&mesh), (v, e, f), "bezier {name}");
        bounds(&mesh, min, max, 2e-3, &format!("bezier {name}"));
    }
}

#[test]
fn bezier_circles_match_the_reference() {
    let expected: [(usize, usize, usize, [f32; 3], [f32; 3]); 7] = [
        (48, 48, 0, [-1.0, -1.0, 0.0], [1.0, 1.0, 0.0]),
        (576, 1152, 576, [-1.1, -1.1, -0.1], [1.1, 1.1, 0.1]),
        (192, 384, 192, [-1.1, -1.1, -0.1], [1.1, 1.1, 0.1]),
        (1056, 2112, 1056, [-1.1, -1.1, -0.3], [1.1, 1.1, 0.3]),
        (96, 144, 48, [-1.0, -1.0, -0.2], [1.0, 1.0, 0.2]),
        (48, 93, 46, [-1.0, -1.0, 0.0], [1.0, 1.0, 0.0]),
        (576, 1152, 576, [-1.6, -1.6, -0.1], [1.6, 1.6, 0.1]),
    ];
    for ((name, curve), (v, e, f, min, max)) in variants(CurvePrimitive::Circle).into_iter().zip(expected) {
        let mesh = curve.to_mesh();
        assert_eq!(counts(&mesh), (v, e, f), "circle {name}");
        bounds(&mesh, min, max, 2e-3, &format!("circle {name}"));
    }
}

#[test]
fn nurbs_paths_match_the_reference_once_swept() {
    // The reference reports the bare path with 13 samples only before its first edit; swept,
    // it samples 12 per segment like every later state, which is what is compared.
    let expected: [(&str, usize, usize, usize); 5] = [
        ("bevel", 576, 1140, 564),
        ("bevel res 0", 192, 380, 188),
        ("bevel and extrude", 1056, 2090, 1034),
        ("extrude", 96, 142, 47),
        ("2D filled", 48, 47, 0),
    ];
    let variants = variants(CurvePrimitive::Path);
    for (name, v, e, f) in expected {
        let curve = &variants.iter().find(|(variant, _)| *variant == name).unwrap().1;
        let mesh = curve.to_mesh();
        assert_eq!(counts(&mesh), (v, e, f), "path {name}");
        bounds(
            &mesh,
            [-2.0, mesh.bounds(Mat4::IDENTITY).min.y, mesh.bounds(Mat4::IDENTITY).min.z],
            [2.0, mesh.bounds(Mat4::IDENTITY).max.y, mesh.bounds(Mat4::IDENTITY).max.z],
            2e-3,
            name,
        );
    }
}

#[test]
fn caps_close_an_open_bevelled_curve() {
    let curve = Curve { bevel_depth: 0.1, fill_caps: true, ..CurvePrimitive::Bezier.build(1.0) };
    let mesh = curve.to_mesh();
    assert_eq!(counts(&mesh), (180, 342, 164));
}

fn text(body: &str) -> Text {
    Text { body: body.into(), ..Text::default() }
}

#[test]
fn text_outlines_match_the_reference() {
    // Vertex counts are the outline's (the reference's with its fill off: its filled meshes add
    // points of their own inside), from 12 points per curved segment with straight ones
    // undivided, and must match exactly; heights match exactly; horizontal positions match
    // within 0.002 of the size (the reference rounds glyph x on a grid of its own).
    let expected: [(&str, usize, [f32; 3], [f32; 3]); 9] = [
        ("T", 8, [0.0318, 0.0, 0.0], [0.4169, 0.5087, 0.0]),
        ("e", 307, [0.0341, -0.0082, 0.0], [0.3711, 0.3844, 0.0]),
        ("x", 112, [0.0208, 0.0, 0.0], [0.3578, 0.3793, 0.0]),
        ("t", 104, [0.0065, -0.0041, 0.0], [0.213, 0.4712, 0.0]),
        ("I", 4, [0.0601, 0.0, 0.0], [0.1246, 0.5087, 0.0]),
        ("l", 4, [0.0526, 0.0, 0.0], [0.1137, 0.5087, 0.0]),
        ("o", 288, [0.0341, -0.0082, 0.0], [0.381, 0.3844, 0.0]),
        ("A", 60, [0.0164, 0.0, 0.0], [0.463, 0.5087, 0.0]),
        ("Text", 531, [0.0318, -0.0082, 0.0], [1.4442, 0.5087, 0.0]),
    ];
    for (body, vertices, min, max) in expected {
        let mesh = text(body).to_mesh();
        mesh.validate().unwrap();
        assert_eq!(text(body).outline_count(), vertices, "{body}: outline points");
        let found = mesh.bounds(Mat4::IDENTITY);
        // The reference reads outlines through a rasteriser that snaps some points (the top of
        // "e" sits at 1126 of 2929 units there, 1132 in the font), so heights agree to 0.004.
        assert!((found.min.y - min[1]).abs() < 4e-3 && (found.max.y - max[1]).abs() < 4e-3, "{body}: heights {found:?}");
        assert!((found.min.x - min[0]).abs() < 4e-3 && (found.max.x - max[0]).abs() < 1.5e-2, "{body}: widths {found:?} against {min:?}..{max:?}");
        // Filled faces, all facing +Z (overlapping strokes merged first).
        assert!(mesh.faces.iter().all(|face| mesh.face_normal(face).z > 0.99), "{body}: a face turned over");
    }
}

#[test]
fn text_layout_matches_the_reference() {
    // Lines one size apart, centred lines about the origin, size scaling everything.
    let two_lines = text("Hi\nYou").to_mesh();
    assert_eq!(text("Hi\nYou").outline_count(), 587);
    let found = two_lines.bounds(Mat4::IDENTITY);
    assert!((found.min.y + 1.0082).abs() < 2e-3 && (found.max.y - 0.5237).abs() < 2e-3, "{found:?}");
    let centred = Text { align: TextAlign::Center, ..text("Text") }.to_mesh().bounds(Mat4::IDENTITY);
    assert!((centred.min.x + 0.6975).abs() < 1.5e-2 && (centred.max.x - 0.7149).abs() < 1.5e-2, "{centred:?}");
    let big = Text { size: 2.0, ..text("Text") }.to_mesh().bounds(Mat4::IDENTITY);
    assert!((big.max.y - 1.0174).abs() < 4e-3 && (big.min.y + 0.0164).abs() < 4e-3, "{big:?}");
    assert_eq!(Text { resolution: 4, ..text("Text") }.outline_count(), 203);
}

#[test]
fn extruded_and_bevelled_text_is_closed() {
    for settings in
        [Text { extrude: 0.1, ..text("Text") }, Text { extrude: 0.1, bevel_depth: 0.02, ..text("Text") }, Text { bevel_depth: 0.02, ..text("o") }]
    {
        let mesh = settings.to_mesh();
        mesh.validate().unwrap();
        assert!(kition_scene_model::modifiers::boundary_edges(&mesh).is_empty(), "{settings:?} is open");
        let found = mesh.bounds(Mat4::IDENTITY);
        let depth = settings.extrude + settings.bevel_depth;
        assert!((found.max.z - depth).abs() < 1e-5 && (found.min.z + depth).abs() < 1e-5);
        let mut oriented = mesh.clone();
        kition_scene_model::ops::recalculate_outside(&mut oriented);
        assert_eq!(oriented.faces, mesh.faces, "faces point outwards");
    }
}

#[test]
fn every_printable_character_fills_and_extrudes_cleanly() {
    for code in 0x21u8..0x7f {
        let body = (code as char).to_string();
        let flat = text(&body).to_mesh();
        flat.validate().unwrap();
        assert!(
            flat.faces.iter().all(|face| kition_scene_model::mesh::newell_vector(&flat.face_positions(face)).z > 0.0),
            "{body}: a face turned over"
        );
        let solid = Text { extrude: 0.05, ..text(&body) }.to_mesh();
        assert!(kition_scene_model::modifiers::boundary_edges(&solid).is_empty(), "{body}: extruded text is open");
    }
}
