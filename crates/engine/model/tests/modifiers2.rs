//! Displace, decimate, wireframe, simple deform, and cast against the reference application's
//! results for the same meshes and settings (counts, bounds, and sample vertices it reported).
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use kition_scene_model::glam::{Mat4, Vec3};
use kition_scene_model::modifiers::{DecimateMode, ModifierKind, ScrewAxis, boundary_edges};
use kition_scene_model::modifiers2::{DeformMode, DisplaceDirection, TextureCoordinates};
use kition_scene_model::{Mesh, Modifier, Object, ObjectData, Primitive, Scene};

fn apply(mesh: Mesh, kind: ModifierKind) -> Mesh {
    let object = Object::new(1, "Test", ObjectData::Mesh(mesh.clone()));
    Modifier::new(kind).apply(&mesh, &object, &Scene::empty())
}

fn cube() -> Mesh {
    Primitive::Cube { size: 2.0 }.build().unwrap()
}

fn assert_bounds(mesh: &Mesh, min: [f32; 3], max: [f32; 3], what: &str) {
    let bounds = mesh.bounds(Mat4::IDENTITY);
    assert!(bounds.min.abs_diff_eq(Vec3::from(min), 1e-3) && bounds.max.abs_diff_eq(Vec3::from(max), 1e-3), "{what}: {bounds:?}");
}

fn defaults(kind: &str) -> ModifierKind {
    ModifierKind::default_of(kind).unwrap()
}

#[test]
fn displace_moves_along_normals_or_an_axis() {
    let out = apply(cube(), defaults("displace"));
    assert_bounds(&out, [-1.2887; 3], [1.2887; 3], "default displace");
    let out = apply(
        cube(),
        ModifierKind::Displace {
            strength: 2.0,
            mid_level: 0.25,
            direction: DisplaceDirection::X,
            noise_scale: 0.0,
            image: None,
            coordinates: TextureCoordinates::Local,
        },
    );
    assert_bounds(&out, [0.5, -1.0, -1.0], [2.5, 1.0, 1.0], "x displace");
    // Noise is deterministic and keeps the mesh valid.
    let noisy = ModifierKind::Displace {
        strength: 0.3,
        mid_level: 0.5,
        direction: DisplaceDirection::Normal,
        noise_scale: 0.5,
        image: None,
        coordinates: TextureCoordinates::Local,
    };
    let sphere = Primitive::IcoSphere { subdivisions: 3, radius: 1.0 }.build().unwrap();
    assert_eq!(apply(sphere.clone(), noisy.clone()), apply(sphere, noisy));
}

#[test]
fn wireframe_matches_the_reference_frame() {
    let out = apply(cube(), defaults("wireframe"));
    assert_eq!((out.positions.len(), out.edges().len(), out.faces.len()), (40, 96, 48));
    assert_bounds(&out, [-1.0058; 3], [1.0058; 3], "wireframe 0.02");
    assert!(boundary_edges(&out).is_empty(), "the frame is closed");
    let out = apply(cube(), ModifierKind::Wireframe { thickness: 0.1, offset: 1.0 });
    assert_bounds(&out, [-1.0577; 3], [1.0577; 3], "wireframe outwards");
    // A plane's struts are inset half the thickness from its edges.
    let plane = apply(Primitive::Plane { size: 2.0 }.build().unwrap(), defaults("wireframe"));
    assert_eq!(plane.positions.len(), 12);
    assert!(plane.positions.iter().any(|point| point.abs_diff_eq(Vec3::new(-0.99, -0.99, 0.0), 1e-5)));
}

#[test]
fn decimate_halves_a_sphere_like_the_reference() {
    let sphere = Primitive::UvSphere { segments: 32, rings: 16, radius: 1.0 }.build().unwrap();
    let out = apply(sphere, ModifierKind::Decimate { mode: DecimateMode::Collapse, ratio: 0.5, iterations: 0, angle_limit: 0.1 });
    out.validate().unwrap();
    // The reference keeps 242 of 482 vertices.
    assert!((232..=252).contains(&out.positions.len()), "{} vertices", out.positions.len());
    assert!(boundary_edges(&out).is_empty(), "still closed");
    let bounds = out.bounds(Mat4::IDENTITY);
    assert!(bounds.size().min_element() > 1.9, "the shape keeps its size: {bounds:?}");
}

#[test]
fn simple_deform_twists_bends_tapers_and_stretches() {
    let case = |mode, axis| {
        apply(
            cube(),
            ModifierKind::SimpleDeform { mode, axis, angle: std::f32::consts::FRAC_PI_4, factor: std::f32::consts::FRAC_PI_4, limits: [0.0, 1.0] },
        )
    };
    let cases = [
        (DeformMode::Twist, ScrewAxis::X, [-1.0, -1.3066, -1.3066], [1.0, 1.3066, 1.3066], [-1.0, -1.3066, -0.5412]),
        (DeformMode::Twist, ScrewAxis::Z, [-1.3066, -1.3066, -1.0], [1.3066, 1.3066, 1.0], [-1.3066, -0.5412, -1.0]),
        (DeformMode::Bend, ScrewAxis::X, [-1.0, -0.73, -1.3572], [1.0, 1.1177, 1.3572], [-1.0, -0.73, -1.3572]),
        (DeformMode::Bend, ScrewAxis::Z, [-1.3572, -0.73, -1.0], [1.3572, 1.1177, 1.0], [-1.3572, -0.73, -1.0]),
        (DeformMode::Taper, ScrewAxis::X, [-1.0, -1.3927, -1.3927], [1.0, 1.3927, 1.3927], [-1.0, -0.6073, -0.6073]),
        (DeformMode::Taper, ScrewAxis::Z, [-1.3927, -1.3927, -1.0], [1.3927, 1.3927, 1.0], [-0.6073, -0.6073, -1.0]),
        (DeformMode::Stretch, ScrewAxis::X, [-1.3927, -1.0, -1.0], [1.3927, 1.0, 1.0], [-1.3927, -1.0, -1.0]),
        (DeformMode::Stretch, ScrewAxis::Z, [-1.0, -1.0, -1.3927], [1.0, 1.0, 1.3927], [-1.0, -1.0, -1.3927]),
    ];
    for (mode, axis, min, max, first) in cases {
        let out = case(mode, axis);
        assert_bounds(&out, min, max, &format!("{mode:?} {axis:?}"));
        assert!(out.positions[0].abs_diff_eq(Vec3::from(first), 1e-3), "{mode:?} {axis:?}: {}", out.positions[0]);
    }
    // Bend round Y on scattered points, as the reference placed them.
    let points = [[0.5, 0.25, 0.1], [-0.3, 0.6, -0.4], [0.2, -0.7, 0.9], [-0.9, -0.1, 0.3]];
    let expected = [[0.5021, 0.25, 0.0697], [-0.2432, 0.6, -0.4679], [0.4099, -0.7, 0.7528], [-0.8581, -0.1, 0.4606]];
    let mesh = Mesh { positions: points.iter().map(|point| Vec3::from(*point)).collect(), ..Default::default() };
    let out = apply(
        mesh,
        ModifierKind::SimpleDeform {
            mode: DeformMode::Bend,
            axis: ScrewAxis::Y,
            angle: std::f32::consts::FRAC_PI_4,
            factor: 0.0,
            limits: [0.0, 1.0],
        },
    );
    for (got, want) in out.positions.iter().zip(expected) {
        assert!(got.abs_diff_eq(Vec3::from(want), 1e-3), "{got} vs {want:?}");
    }
}

#[test]
fn cast_pulls_towards_a_sphere_round_the_origin() {
    assert_eq!(apply(cube(), defaults("cast")).positions, cube().positions, "a cube's corners are already on one sphere");
    let mut shifted = cube();
    for point in &mut shifted.positions {
        point.x += 1.0;
    }
    let out = apply(shifted, ModifierKind::Cast { factor: 1.0, axes: [true; 3] });
    assert_bounds(&out, [0.0, -1.366, -1.366], [1.5774, 1.366, 1.366], "cast of an offset cube");
}

#[test]
fn hostile_settings_are_refused() {
    for kind in [
        ModifierKind::Decimate { mode: DecimateMode::Collapse, ratio: 2.0, iterations: 0, angle_limit: 0.1 },
        ModifierKind::Wireframe { thickness: f32::NAN, offset: 0.0 },
        ModifierKind::SimpleDeform { mode: DeformMode::Twist, axis: ScrewAxis::Z, angle: 1.0, factor: 1.0, limits: [0.8, 0.2] },
        ModifierKind::Displace {
            strength: 1.0,
            mid_level: 0.5,
            direction: DisplaceDirection::Normal,
            noise_scale: -1.0,
            image: None,
            coordinates: TextureCoordinates::Local,
        },
    ] {
        assert!(kind.check().is_err(), "{kind:?}");
    }
}
