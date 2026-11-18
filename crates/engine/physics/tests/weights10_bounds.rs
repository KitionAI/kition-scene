//! Full public ranges are settings, while overflow is an explicit derived-bake failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use glam::Vec3;
use kition_scene_model::{
    ObjectData, Primitive, Scene,
    physics::{Cloth, Effector, EffectorWeights, FieldKind, ForceField, ObjectPhysics, SoftBody},
};
use kition_scene_physics::Bake;

#[test]
fn weights_scale_field_coefficients_before_nonlinear_evaluation_and_default_bits_are_unchanged() {
    for kind in [FieldKind::Force, FieldKind::Wind, FieldKind::Vortex, FieldKind::Turbulence, FieldKind::Drag] {
        let source = Effector {
            field: ForceField { strength: 2.75, flow: 0.37, linear_drag: 0.7, quadratic_drag: 0.3, ..ForceField::new(kind) },
            origin: Vec3::X,
            axis: Vec3::Z,
            owner: 5,
        };
        let defaults = EffectorWeights::default().placed(std::slice::from_ref(&source), 1).unwrap();
        assert_eq!(defaults[0].field, source.field);
        for (position, velocity) in [(Vec3::new(2.0, 3.0, -1.0), Vec3::new(0.3, -0.2, 1.0)), (Vec3::ZERO, Vec3::ZERO)] {
            assert_eq!(defaults[0].push(position, velocity), source.push(position, velocity));
        }
        let weighted = EffectorWeights { all: 0.5, ..Default::default() }.placed(std::slice::from_ref(&source), 1).unwrap();
        let mut expected = source.field.clone();
        if kind == FieldKind::Drag {
            expected.linear_drag *= 0.5;
            expected.quadratic_drag *= 0.5;
        } else {
            expected.strength *= 0.5;
            expected.flow *= 0.5;
        }
        assert_eq!(weighted[0].field, expected);
        assert!(EffectorWeights::default().placed(std::slice::from_ref(&source), 5).unwrap().is_empty());
        for all in [0.0, -1.0] {
            assert!(EffectorWeights { all, ..Default::default() }.placed(std::slice::from_ref(&source), 1).unwrap().is_empty());
        }
        let mut overflowing = source.clone();
        if kind == FieldKind::Drag {
            overflowing.field.linear_drag = f32::MAX;
        } else {
            overflowing.field.strength = f32::MAX;
        }
        let error = EffectorWeights { all: 200.0, ..Default::default() }.placed(&[overflowing], 1).unwrap_err();
        assert!(error.to_string().contains("coefficients must be finite"));
    }
    for all in [-200.0, 200.0] {
        EffectorWeights { all, ..Default::default() }.check().unwrap();
    }
    for gravity in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -200.01, 200.01] {
        assert!(EffectorWeights { gravity, ..Default::default() }.check().is_err());
    }
}

#[test]
fn extrema_can_bake_finitely_and_divergence_discards_every_partial_frame() {
    for soft in [false, true] {
        let mut scene = Scene::empty();
        scene.physics.frame_end = 12;
        let id = scene.add_object("Mesh", ObjectData::Mesh(Primitive::Plane { size: 1.0 }.build().unwrap()));
        let weights = EffectorWeights { all: -200.0, gravity: -200.0, force: -200.0, wind: -200.0, vortex: -200.0, turbulence: -200.0, drag: -200.0 };
        let physics = if soft {
            ObjectPhysics {
                soft_body: Some(SoftBody { goal: false, edges: false, effector_weights: weights, ..Default::default() }),
                ..Default::default()
            }
        } else {
            ObjectPhysics { cloth: Some(Cloth { effector_weights: weights, ..Default::default() }), ..Default::default() }
        };
        scene.object_mut(id).unwrap().physics = Some(physics);
        scene.validate().unwrap();
        let stable = kition_scene_physics::bake(&scene).unwrap();
        assert!(stable.cloths[&id].iter().flatten().all(|point| point.is_finite()));
        assert!(stable.cloths[&id].last().unwrap().iter().all(|point| point.z > 1.0));
        scene.physics.gravity = Vec3::ZERO;
        let field = scene.add_object("Vortex", ObjectData::Empty { size: 1.0 });
        scene.object_mut(field).unwrap().location = Vec3::new(3.0, 0.0, 0.0);
        scene.object_mut(field).unwrap().physics =
            Some(ObjectPhysics { field: Some(ForceField { strength: 10000.0, ..ForceField::new(FieldKind::Vortex) }), ..Default::default() });
        scene.validate().unwrap();
        let before = scene.clone();
        let mut bake = Bake::new(&scene).unwrap();
        assert_eq!(bake.result().frames, 1);
        assert!(bake.step(12));
        let error = bake.error().unwrap().to_string();
        assert!(error.contains("nonfinite"), "{error}");
        assert_eq!(bake.progress().0, 0);
        let failed = bake.result();
        assert!(failed.cloths.is_empty() && failed.bodies.is_empty() && failed.hair.is_empty());
        assert!(bake.step(12));
        assert_eq!(bake.error(), Some(error.as_str()));
        assert_eq!(scene, before);
    }
}
