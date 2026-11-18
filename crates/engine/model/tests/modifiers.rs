//! The bevel, boolean, and screw modifiers against the reference application's results with its
//! defaults (counts as vertices, edges, faces), and the evaluation key.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use kition_scene_model::csg::Operation;
use kition_scene_model::glam::{Mat4, Vec3};
use kition_scene_model::modifiers::{ScrewAxis, boundary_edges};
use kition_scene_model::{Mesh, Modifier, ModifierKind, ObjectData, Primitive, Scene};

fn counts(mesh: &Mesh) -> (usize, usize, usize) {
    (mesh.positions.len(), mesh.edges().len(), mesh.faces.len())
}

fn evaluated(primitive: Primitive, kind: &str) -> Mesh {
    let mut scene = Scene::empty();
    let id = scene.add_object("Object", ObjectData::Mesh(primitive.build().unwrap()));
    scene.object_mut(id).unwrap().modifiers.push(Modifier::new(ModifierKind::default_of(kind).unwrap()));
    scene.object(id).unwrap().evaluated_mesh(&scene).unwrap()
}

#[test]
fn bevel_defaults_match_the_reference() {
    let cube = evaluated(Primitive::Cube { size: 2.0 }, "bevel");
    assert_eq!(counts(&cube), (24, 48, 26));
    let cylinder = evaluated(Primitive::Cylinder { vertices: 8, radius: 1.0, depth: 2.0, fill: kition_scene_model::Fill::Ngon }, "bevel");
    assert_eq!(counts(&cylinder), (48, 96, 50));
    let cone = evaluated(Primitive::Cone { vertices: 8, radius1: 1.0, radius2: 0.0, depth: 2.0, fill: kition_scene_model::Fill::Ngon }, "bevel");
    assert_eq!(counts(&cone), (32, 64, 34));
    for mesh in [&cube, &cylinder, &cone] {
        assert!(boundary_edges(mesh).is_empty());
    }
    // A smooth sphere has no edge sharper than 30 degrees: nothing changes.
    let sphere = evaluated(Primitive::UvSphere { segments: 32, rings: 16, radius: 1.0 }, "bevel");
    assert_eq!(counts(&sphere), (482, 992, 512));
}

fn two_cubes(operation: Operation) -> (Scene, u32, u32) {
    let mut scene = Scene::empty();
    let cube = || ObjectData::Mesh(Primitive::Cube { size: 2.0 }.build().unwrap());
    let a = scene.add_object("A", cube());
    let b = scene.add_object("B", cube());
    scene.object_mut(b).unwrap().location = Vec3::ONE;
    scene.object_mut(a).unwrap().modifiers.push(Modifier::new(ModifierKind::Boolean { operation, object: Some(b) }));
    (scene, a, b)
}

#[test]
fn booleans_match_the_reference() {
    for (operation, (vertices, faces)) in [(Operation::Difference, (14, 9)), (Operation::Union, (20, 12)), (Operation::Intersect, (8, 6))] {
        let (scene, a, _) = two_cubes(operation);
        let mesh = scene.object(a).unwrap().evaluated_mesh(&scene).unwrap();
        assert_eq!((mesh.positions.len(), mesh.faces.len()), (vertices, faces), "{operation:?}");
        assert!(boundary_edges(&mesh).is_empty());
    }
}

#[test]
fn a_boolean_follows_its_operand_in_the_object_s_own_space() {
    let (mut scene, a, b) = two_cubes(Operation::Difference);
    // A is turned and moved; B is placed in the world: the cut follows B's world place.
    scene.object_mut(a).unwrap().location = Vec3::new(5.0, 0.0, 0.0);
    scene.object_mut(a).unwrap().rotation = Vec3::new(0.0, 0.0, std::f32::consts::FRAC_PI_2);
    scene.object_mut(b).unwrap().location = Vec3::new(6.0, 1.0, 1.0);
    let mesh = scene.object(a).unwrap().evaluated_mesh(&scene).unwrap();
    assert_eq!(mesh.faces.len(), 9);
}

#[test]
fn bad_operands_leave_the_mesh_alone() {
    let cube_counts = (8, 12, 6);
    // Missing, itself, a light, and a cycle (A cuts B, B cuts A).
    let (mut scene, a, b) = two_cubes(Operation::Difference);
    scene.object_mut(a).unwrap().modifiers[0].kind = ModifierKind::Boolean { operation: Operation::Difference, object: Some(999) };
    assert_eq!(counts(&scene.object(a).unwrap().evaluated_mesh(&scene).unwrap()), cube_counts);
    scene.object_mut(a).unwrap().modifiers[0].kind = ModifierKind::Boolean { operation: Operation::Difference, object: Some(a) };
    assert_eq!(counts(&scene.object(a).unwrap().evaluated_mesh(&scene).unwrap()), cube_counts);
    let light = scene.add_object("Light", ObjectData::Light(kition_scene_model::Light::new(kition_scene_model::LightKind::Point)));
    scene.object_mut(a).unwrap().modifiers[0].kind = ModifierKind::Boolean { operation: Operation::Difference, object: Some(light) };
    assert_eq!(counts(&scene.object(a).unwrap().evaluated_mesh(&scene).unwrap()), cube_counts);
    scene.object_mut(a).unwrap().modifiers[0].kind = ModifierKind::Boolean { operation: Operation::Difference, object: Some(b) };
    scene.object_mut(b).unwrap().modifiers.push(Modifier::new(ModifierKind::Boolean { operation: Operation::Union, object: Some(a) }));
    let mesh = scene.object(a).unwrap().evaluated_mesh(&scene).unwrap();
    mesh.validate().unwrap();
    // An open operand (a plane) cannot cut.
    let plane = scene.add_object("Plane", ObjectData::Mesh(Primitive::Plane { size: 2.0 }.build().unwrap()));
    scene.object_mut(a).unwrap().modifiers[0].kind = ModifierKind::Boolean { operation: Operation::Difference, object: Some(plane) };
    assert_eq!(counts(&scene.object(a).unwrap().evaluated_mesh(&scene).unwrap()), cube_counts);
}

/// A circle of 8 in the XZ plane, centred 2 m out along X: a torus's profile.
fn profile() -> Mesh {
    let positions = (0..8)
        .map(|step| {
            let angle = std::f32::consts::TAU * step as f32 / 8.0;
            Vec3::new(2.0 + 0.5 * angle.cos(), 0.0, 0.5 * angle.sin())
        })
        .collect();
    Mesh { positions, loose_edges: (0..8).map(|step| [step, (step + 1) % 8]).collect(), ..Default::default() }
}

fn screwed(angle: f32, screw_offset: f32) -> Mesh {
    let mut scene = Scene::empty();
    let id = scene.add_object("Profile", ObjectData::Mesh(profile()));
    scene.object_mut(id).unwrap().modifiers.push(Modifier::new(ModifierKind::Screw { steps: 16, angle, screw_offset, axis: ScrewAxis::Z }));
    scene.object(id).unwrap().evaluated_mesh(&scene).unwrap()
}

#[test]
fn screw_matches_the_reference() {
    assert_eq!(
        ModifierKind::default_of("screw").unwrap(),
        ModifierKind::Screw { steps: 16, angle: std::f32::consts::TAU, screw_offset: 0.0, axis: ScrewAxis::Z }
    );
    let torus = screwed(std::f32::consts::TAU, 0.0);
    assert_eq!(counts(&torus), (128, 256, 128));
    assert!(boundary_edges(&torus).is_empty());
    let bounds = torus.bounds(Mat4::IDENTITY);
    assert!(bounds.min.abs_diff_eq(Vec3::new(-2.5, -2.5, -0.5), 1e-4) && bounds.max.abs_diff_eq(Vec3::new(2.5, 2.5, 0.5), 1e-4), "{bounds:?}");
    let (v, _, f) = counts(&screwed(std::f32::consts::PI, 0.0));
    assert_eq!((v, f), (136, 128));
    let (v, _, f) = counts(&screwed(std::f32::consts::TAU, 1.0));
    assert_eq!((v, f), (136, 128));
}

#[test]
fn a_lathe_closes_on_the_axis() {
    // A half circle from pole to pole turned round Z: a closed sphere-like solid.
    let positions: Vec<Vec3> = (0..=6)
        .map(|step| {
            let angle = std::f32::consts::PI * step as f32 / 6.0;
            Vec3::new(angle.sin(), 0.0, angle.cos())
        })
        .collect();
    let mut scene = Scene::empty();
    let id = scene.add_object(
        "Profile",
        ObjectData::Mesh(Mesh { positions, loose_edges: (0..6).map(|step| [step, step + 1]).collect(), ..Default::default() }),
    );
    scene.object_mut(id).unwrap().modifiers.push(Modifier::new(ModifierKind::default_of("screw").unwrap()));
    let mesh = scene.object(id).unwrap().evaluated_mesh(&scene).unwrap();
    mesh.validate().unwrap();
    assert_eq!(mesh.positions.len(), 5 * 16 + 2);
    assert!(boundary_edges(&mesh).is_empty());
}

#[test]
fn the_evaluation_key_follows_what_the_mesh_depends_on() {
    let (mut scene, a, b) = two_cubes(Operation::Difference);
    let other = scene.add_object("Other", ObjectData::Empty { size: 1.0 });
    let key = |scene: &Scene| scene.object(a).unwrap().evaluation_key(scene);
    let first = key(&scene);
    assert_eq!(key(&scene), first);
    scene.object_mut(other).unwrap().location = Vec3::X;
    assert_eq!(key(&scene), first, "an unrelated object moved");
    scene.object_mut(a).unwrap().location = Vec3::new(3.0, 0.0, 0.0);
    assert_ne!(key(&scene), first, "the cut moved relative to A");
    let moved = key(&scene);
    scene.object_mut(b).unwrap().location += Vec3::new(3.0, 0.0, 0.0);
    assert_ne!(key(&scene), moved);
    scene.object_mut(a).unwrap().location += Vec3::new(3.0, 0.0, 0.0);
    let together = key(&scene);
    scene.object_mut(b).unwrap().mesh_mut().unwrap().positions[0].x += 0.1;
    assert_ne!(key(&scene), together, "the operand's mesh changed");
    scene.object_mut(a).unwrap().modifiers[0].enabled = false;
    assert_ne!(key(&scene), together);
}

#[test]
fn new_kinds_check_their_ranges() {
    for (kind, bad) in [
        ("bevel", serde_json::json!({ "type": "bevel", "width": -1.0 })),
        ("bevel", serde_json::json!({ "type": "bevel", "segments": 0 })),
        ("screw", serde_json::json!({ "type": "screw", "steps": 0 })),
        ("screw", serde_json::json!({ "type": "screw", "angle": 1e9 })),
    ] {
        let parsed: ModifierKind = serde_json::from_value(bad.clone()).unwrap();
        assert!(parsed.check().is_err(), "{kind}: {bad}");
    }
    assert!(ModifierKind::default_of("boolean").unwrap().check().is_ok());
    assert_eq!(ModifierKind::default_of("bevel").unwrap(), ModifierKind::Bevel { width: 0.1, segments: 1, angle_limit: 30f32.to_radians() });
}
