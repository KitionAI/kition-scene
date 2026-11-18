//! Constraints are baked into glTF, as the reference application's exporter does: a camera
//! tracking a keyed empty comes back from our own file following it, frame by frame.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_scene_exchange::{Format, export, import, place};
use kition_scene_model::constraints::{Constraint, ConstraintKind, Target, TrackAxis, UpAxis};
use kition_scene_model::glam::{Mat4, Vec3};
use kition_scene_model::{ObjectData, Property, Scene};

fn world(scene: &Scene, name: &str, frame: f32) -> Mat4 {
    let at = scene.at_frame(frame);
    at.world_matrix(at.object_named(name).unwrap().id).unwrap()
}

#[test]
fn a_tracking_camera_is_baked_into_its_animation() {
    let mut scene = Scene::starter();
    let camera = scene.object_named("Camera").unwrap().id;
    let goal = scene.add_object("Goal", ObjectData::Empty { size: 1.0 });
    scene.insert_key(goal, Property::Location, 1.0).unwrap();
    scene.object_mut(goal).unwrap().location = Vec3::new(3.0, 2.0, 1.0);
    scene.insert_key(goal, Property::Location, 21.0).unwrap();
    scene.go_to_frame(1.0);
    scene.object_mut(camera).unwrap().constraints.push(Constraint::new(ConstraintKind::TrackTo {
        target: Some(Target { object: goal, bone: None }),
        track_axis: TrackAxis::NegZ,
        up_axis: UpAxis::Y,
    }));
    scene.validate().unwrap();
    // The camera only turns because of its constraint: its own rotation is not keyed.
    assert!(!world(&scene, "Camera", 1.0).abs_diff_eq(world(&scene, "Camera", 21.0), 1e-3));

    let bytes = export(&scene, Format::Glb, None).unwrap();
    let mut back = Scene::empty();
    place(&mut back, import(&bytes, Format::Glb).unwrap()).unwrap();
    for frame in [1.0, 6.0, 11.0, 21.0] {
        let (ours, theirs) = (world(&scene, "Camera", frame), world(&back, "Camera", frame));
        assert!(ours.abs_diff_eq(theirs, 2e-3), "frame {frame}:\n{ours:?}\n{theirs:?}");
    }
    // The baked copy has no constraint of its own.
    assert!(back.object_named("Camera").unwrap().constraints.is_empty());
}

#[test]
fn a_constrained_bone_is_baked_into_the_skin_animation() {
    use kition_scene_model::{Armature, Bone, Modifier, ModifierKind, Primitive};
    let mut scene = Scene::empty();
    let goal = scene.add_object("Goal", ObjectData::Empty { size: 1.0 });
    scene.object_mut(goal).unwrap().location = Vec3::new(0.0, 1.0, 3.0);
    scene.insert_key(goal, Property::Location, 1.0).unwrap();
    scene.object_mut(goal).unwrap().location = Vec3::new(2.0, -1.0, 2.0);
    scene.insert_key(goal, Property::Location, 11.0).unwrap();
    scene.go_to_frame(1.0);
    let mut lower = Bone::new("Lower", Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0));
    lower.roll = 0.0;
    let mut upper = Bone::new("Upper", Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 2.0));
    upper.parent = Some("Lower".into());
    upper.connected = true;
    upper
        .constraints
        .push(Constraint::new(ConstraintKind::DampedTrack { target: Some(Target { object: goal, bone: None }), track_axis: TrackAxis::Y }));
    let rig = scene.add_object("Rig", ObjectData::Armature(Armature { bones: vec![lower, upper] }));
    let mesh = Primitive::Cylinder { vertices: 8, radius: 0.2, depth: 2.0, fill: kition_scene_model::Fill::Ngon }.build().unwrap();
    let column = scene.add_object("Column", ObjectData::Mesh(mesh));
    scene.object_mut(column).unwrap().location = Vec3::new(0.0, 0.0, 1.0);
    kition_scene_model::armature::bind(&mut scene, column, rig, true).unwrap();
    scene.object_mut(column).unwrap().modifiers.push(Modifier::new(ModifierKind::Armature {
        object: Some(rig),
        preserve_volume: false,
        deform: Default::default(),
    }));
    scene.validate().unwrap();

    let posed = |scene: &Scene, frame: f32| -> Vec<Vec3> {
        let at = scene.at_frame(frame);
        let object = at.object_named("Column").unwrap();
        let world = at.world_matrix(object.id).unwrap();
        object.evaluated_mesh(&at).unwrap().positions.iter().map(|point| world.transform_point3(*point)).collect()
    };
    let bytes = export(&scene, Format::Glb, None).unwrap();
    let mut back = Scene::empty();
    place(&mut back, import(&bytes, Format::Glb).unwrap()).unwrap();
    for frame in [1.0, 6.0, 11.0] {
        let (ours, theirs) = (posed(&scene, frame), posed(&back, frame));
        // Vertices may come back in another order: each must have a baked twin.
        let worst = ours.iter().map(|point| theirs.iter().map(|other| point.distance(*other)).fold(f32::MAX, f32::min)).fold(0.0f32, f32::max);
        assert!(worst < 2e-3, "frame {frame}: a vertex is {worst} m from the baked one");
    }
    // The bone really moved with its target.
    assert!(posed(&scene, 1.0).iter().zip(posed(&scene, 11.0)).any(|(a, b)| a.distance(b) > 0.1));
}
