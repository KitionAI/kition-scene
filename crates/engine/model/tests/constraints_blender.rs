//! Constraints against the reference application. The IK cases are its results recorded from
//! Blender 5.2.3 (always run); the rest run when `KITION_BLENDER` names a Blender executable:
//! the same scene is built there and here, and the evaluated world matrices compared.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_scene_model::constraints::{Constraint, ConstraintKind, Space, Target, TrackAxis, UpAxis, bone_matrices};
use kition_scene_model::glam::{Mat4, Vec3};
use kition_scene_model::{Armature, Bone, Id, ObjectData, Scene};
use serde_json::Value;

fn empty(scene: &mut Scene, name: &str, location: Vec3, rotation: Vec3) -> Id {
    let id = scene.add_object(name, ObjectData::Empty { size: 1.0 });
    let object = scene.object_mut(id).unwrap();
    object.location = location;
    object.rotation = rotation;
    id
}

fn on(object: Id) -> Option<Target> {
    Some(Target { object, bone: None })
}

/// An IK chain (bones head → tail, connected when they meet) with an IK on the last bone; the
/// heads after solving.
fn ik_heads(bones: &[([f32; 3], [f32; 3])], goal: [f32; 3], pole: Option<([f32; 3], f32)>) -> Vec<Vec3> {
    let mut scene = Scene::empty();
    let target = empty(&mut scene, "Goal", Vec3::from(goal), Vec3::ZERO);
    let pole = pole.map(|(at, angle)| (empty(&mut scene, "Pole", Vec3::from(at), Vec3::ZERO), angle));
    let mut list: Vec<Bone> = Vec::new();
    for (index, (head, tail)) in bones.iter().enumerate() {
        let mut bone = Bone::new(format!("B{index}"), Vec3::from(*head), Vec3::from(*tail));
        if let Some(previous) = list.last() {
            bone.parent = Some(previous.name.clone());
            bone.connected = previous.tail.distance(bone.head) < 1e-6;
        }
        list.push(bone);
    }
    list.last_mut().unwrap().constraints.push(Constraint::new(ConstraintKind::Ik {
        target: on(target),
        pole: pole.map(|(id, _)| Target { object: id, bone: None }),
        pole_angle: pole.map_or(0.0, |(_, angle)| angle),
        chain: 0,
        iterations: 500,
    }));
    let names: Vec<String> = list.iter().map(|bone| bone.name.clone()).collect();
    let rig = scene.add_object("Rig", ObjectData::Armature(Armature { bones: list }));
    let posed = bone_matrices(&scene, rig, 0);
    names.iter().map(|name| posed[name].transform_point3(Vec3::ZERO)).collect()
}

fn close(found: &[Vec3], expected: &[[f32; 3]], tolerance: f32, label: &str) {
    for (got, want) in found.iter().zip(expected) {
        assert!(got.distance(Vec3::from(*want)) <= tolerance, "{label}: {got} against {want:?}");
    }
}

#[test]
fn ik_lands_where_the_reference_does() {
    let two = [([0.0, 0.0, 0.0], [0.0, 0.0, 1.0]), ([0.0, 0.0, 1.0], [0.0, 0.2, 2.0])];
    let straight = [([0.0, 0.0, 0.0], [0.0, 0.0, 1.0]), ([0.0, 0.0, 1.0], [0.0, 0.0, 2.0])];
    let three = [([0.0, 0.0, 0.0], [0.0, 0.0, 1.0]), ([0.0, 0.0, 1.0], [0.0, 0.3, 1.8]), ([0.0, 0.3, 1.8], [0.0, 0.2, 2.6])];
    // The bend keeps the rest pose's side.
    close(&ik_heads(&two, [0.0, 0.6, 1.4], None), &[[0.0, 0.0, 0.0], [0.0, -0.3146, 0.9492]], 1e-3, "bent");
    // A straight chain bends in the plane of the target, away from it.
    close(&ik_heads(&straight, [0.3, 0.4, 1.2], None), &[[0.0, 0.0, 0.0], [-0.2708, -0.3611, 0.8923]], 1e-3, "straight");
    close(&ik_heads(&two, [1.5, 0.5, 0.5], None), &[[0.0, 0.0, 0.0], [0.6553, -0.0111, 0.7553]], 0.04, "sideways");
    // The pole turns the chain about the root–goal line.
    close(&ik_heads(&two, [0.0, 0.6, 1.4], Some(([2.0, 0.0, 1.0], 0.0))), &[[0.0, 0.0, 0.0], [-0.1281, -0.3032, 0.9443]], 1e-3, "pole 0°");
    close(
        &ik_heads(&two, [0.0, 0.6, 1.4], Some(([2.0, 0.0, 1.0], -std::f32::consts::FRAC_PI_2))),
        &[[0.0, 0.0, 0.0], [-0.6506, 0.4126, 0.6375]],
        1e-3,
        "pole −90°",
    );
    // Out of reach: straight at the goal.
    close(&ik_heads(&two, [0.0, 3.0, 3.0], None), &[[0.0, 0.0, 0.0], [0.0, 0.705, 0.7092]], 0.01, "unreachable");
    // Three bones share the bend as the reference's solver does.
    close(&ik_heads(&three, [0.5, 0.5, 1.5], None), &[[0.0, 0.0, 0.0], [-0.1829, 0.2311, 0.9556], [0.0126, 1.0394, 1.1515]], 0.05, "three");
    close(&ik_heads(&three, [0.0, 0.8, 1.6], None), &[[0.0, 0.0, 0.0], [0.0, 0.16, 0.9871], [0.0, 0.9976, 0.8184]], 0.05, "three, planar");
    close(&ik_heads(&three, [-0.4, 0.2, 2.2], None), &[[0.0, 0.0, 0.0], [0.0276, 0.1167, 0.9928], [-0.1262, 0.682, 1.6147]], 0.05, "three, behind");
}

/// The Blender side: each case prints the owner's world matrix (row-major) as JSON.
const SCRIPT: &str = r#"
import bpy, json, sys, math
from mathutils import Vector, Euler
out = {}
def fresh():
    bpy.ops.wm.read_factory_settings(use_empty=True)
def empty(name, loc, rot=(0,0,0)):
    o = bpy.data.objects.new(name, None); o.location = loc; o.rotation_euler = rot; bpy.context.collection.objects.link(o); return o
def record(label, matrix):
    out[label] = [list(row) for row in matrix]
def case(label, kind, setup=None, influence=1.0, target_rot=(0,0,0)):
    fresh()
    owner = empty('Owner', (1, 2, 0.5), (0.1, 0.2, 0.3))
    target = empty('Target', (4, -1, 2), target_rot)
    c = owner.constraints.new(kind)
    if hasattr(c, 'target'): c.target = target
    c.influence = influence
    if setup: setup(c)
    bpy.context.view_layer.update()
    record(label, owner.matrix_world)
case('trackTo', 'TRACK_TO')
case('trackTo-half', 'TRACK_TO', influence=0.5)
case('trackTo-x-z', 'TRACK_TO', lambda c: (setattr(c, 'track_axis', 'TRACK_X'), setattr(c, 'up_axis', 'UP_Z')))
case('dampedTrack', 'DAMPED_TRACK')
case('copyLocation', 'COPY_LOCATION', lambda c: (setattr(c, 'use_y', False), setattr(c, 'invert_z', True), setattr(c, 'use_offset', True)))
case('copyRotation', 'COPY_ROTATION', target_rot=(0.4, -0.2, 1.0))
case('copyRotation-xz', 'COPY_ROTATION', lambda c: setattr(c, 'use_y', False), target_rot=(0.4, -0.2, 1.0))
case('limitRotation', 'LIMIT_ROTATION', lambda c: (setattr(c, 'use_limit_x', True), setattr(c, 'min_x', -0.05), setattr(c, 'max_x', 0.05), setattr(c, 'use_limit_z', True), setattr(c, 'min_z', 0.0), setattr(c, 'max_z', 0.1)))
# Child Of: the inverse is set when the constraint first evaluates; then the target moves.
fresh()
owner = empty('Owner', (1, 2, 0.5), (0.1, 0.2, 0.3)); target = empty('Target', (4, -1, 2), (0.3, 0, 0.2))
c = owner.constraints.new('CHILD_OF'); c.target = target
bpy.context.view_layer.update()
target.location = (5, 0, 3); target.rotation_euler = (0.3, 0.4, 0.9); target.scale = (1, 2, 1)
bpy.context.view_layer.update()
record('childOf', owner.matrix_world)
# Bone constraints on a moved, rotated armature.
for kind in ('DAMPED_TRACK', 'TRACK_TO'):
    fresh()
    arm = bpy.data.armatures.new('A'); rig = bpy.data.objects.new('Rig', arm); bpy.context.collection.objects.link(rig)
    rig.location = (0.5, 0, 0); rig.rotation_euler = (0, 0, 0.4)
    bpy.context.view_layer.objects.active = rig; bpy.ops.object.mode_set(mode='EDIT')
    b = arm.edit_bones.new('Bone'); b.head = (0, 0, 0); b.tail = (0, 0.3, 1); b.roll = 0.2
    bpy.ops.object.mode_set(mode='OBJECT')
    target = empty('Target', (2, 1, 3))
    c = rig.pose.bones['Bone'].constraints.new(kind); c.target = target
    bpy.context.view_layer.update()
    record('bone-' + kind, rig.matrix_world @ rig.pose.bones['Bone'].matrix)
json.dump(out, open(sys.argv[sys.argv.index('--') + 1], 'w'))
"#;

fn row_major(value: &Value) -> Mat4 {
    let rows: Vec<Vec<f32>> =
        value.as_array().unwrap().iter().map(|row| row.as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect()).collect();
    Mat4::from_cols_array(&[
        rows[0][0], rows[1][0], rows[2][0], rows[3][0], rows[0][1], rows[1][1], rows[2][1], rows[3][1], rows[0][2], rows[1][2], rows[2][2],
        rows[3][2], rows[0][3], rows[1][3], rows[2][3], rows[3][3],
    ])
}

/// Our side of each object case: the owner's world matrix with `kind` on it.
fn ours(kind: ConstraintKind, influence: f32, target_rotation: Vec3) -> Mat4 {
    let mut scene = Scene::empty();
    let owner = empty(&mut scene, "Owner", Vec3::new(1.0, 2.0, 0.5), Vec3::new(0.1, 0.2, 0.3));
    let target = empty(&mut scene, "Target", Vec3::new(4.0, -1.0, 2.0), target_rotation);
    let kind = match kind {
        ConstraintKind::TrackTo { track_axis, up_axis, .. } => ConstraintKind::TrackTo { target: on(target), track_axis, up_axis },
        ConstraintKind::DampedTrack { track_axis, .. } => ConstraintKind::DampedTrack { target: on(target), track_axis },
        ConstraintKind::CopyLocation { axes, invert, offset, owner_space, target_space, .. } => {
            ConstraintKind::CopyLocation { target: on(target), axes, invert, offset, owner_space, target_space }
        }
        ConstraintKind::CopyRotation { axes, invert, owner_space, target_space, .. } => {
            ConstraintKind::CopyRotation { target: on(target), axes, invert, owner_space, target_space }
        }
        other => other,
    };
    scene.object_mut(owner).unwrap().constraints.push(Constraint { influence, ..Constraint::new(kind) });
    scene.world_matrix(owner).unwrap()
}

fn assert_matrix(label: &str, got: Mat4, want: Mat4, tolerance: f32) {
    assert!(got.abs_diff_eq(want, tolerance), "{label}:\nours      {got:?}\nreference {want:?}");
}

#[test]
fn constraints_match_the_reference() {
    let Some(blender) = std::env::var_os("KITION_BLENDER") else {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    };
    let dir = std::env::temp_dir().join(format!("kition-scene-constraints-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("constraints.py");
    std::fs::write(&script, SCRIPT).unwrap();
    let output = dir.join("out.json");
    let status = std::process::Command::new(blender)
        .args(["-b", "--factory-startup", "--python"])
        .arg(&script)
        .arg("--")
        .arg(&output)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "Blender failed");
    let reference: Value = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    let want = |label: &str| row_major(&reference[label]);
    let none = Vec3::ZERO;
    let track = |track_axis, up_axis| ConstraintKind::TrackTo { target: None, track_axis, up_axis };
    assert_matrix("trackTo", ours(track(TrackAxis::NegZ, UpAxis::Y), 1.0, none), want("trackTo"), 1e-4);
    assert_matrix("trackTo-half", ours(track(TrackAxis::NegZ, UpAxis::Y), 0.5, none), want("trackTo-half"), 1e-4);
    assert_matrix("trackTo-x-z", ours(track(TrackAxis::X, UpAxis::Z), 1.0, none), want("trackTo-x-z"), 1e-4);
    assert_matrix("dampedTrack", ours(ConstraintKind::DampedTrack { target: None, track_axis: TrackAxis::Y }, 1.0, none), want("dampedTrack"), 1e-4);
    let copy_location = ConstraintKind::CopyLocation {
        target: None,
        axes: [true, false, true],
        invert: [false, false, true],
        offset: true,
        owner_space: Space::World,
        target_space: Space::World,
    };
    assert_matrix("copyLocation", ours(copy_location, 1.0, none), want("copyLocation"), 1e-4);
    let rotated = Vec3::new(0.4, -0.2, 1.0);
    assert_matrix(
        "copyRotation",
        ours(
            ConstraintKind::CopyRotation { target: None, axes: [true; 3], invert: [false; 3], owner_space: Space::World, target_space: Space::World },
            1.0,
            rotated,
        ),
        want("copyRotation"),
        1e-4,
    );
    assert_matrix(
        "copyRotation-xz",
        ours(
            ConstraintKind::CopyRotation {
                target: None,
                axes: [true, false, true],
                invert: [false; 3],
                owner_space: Space::World,
                target_space: Space::World,
            },
            1.0,
            rotated,
        ),
        want("copyRotation-xz"),
        1e-4,
    );
    let limit = ConstraintKind::LimitRotation {
        limit: [true, false, true],
        min: Vec3::new(-0.05, 0.0, 0.0),
        max: Vec3::new(0.05, 0.0, 0.1),
        space: Space::World,
    };
    assert_matrix("limitRotation", ours(limit, 1.0, none), want("limitRotation"), 1e-4);

    // Child Of: set with the target where it starts, then the target moves.
    let mut scene = Scene::empty();
    let owner = empty(&mut scene, "Owner", Vec3::new(1.0, 2.0, 0.5), Vec3::new(0.1, 0.2, 0.3));
    let target = empty(&mut scene, "Target", Vec3::new(4.0, -1.0, 2.0), Vec3::new(0.3, 0.0, 0.2));
    let inverse = scene.world_matrix(target).unwrap().inverse();
    scene.object_mut(owner).unwrap().constraints.push(Constraint::new(ConstraintKind::ChildOf {
        target: on(target),
        inverse,
        location: [true; 3],
        rotation: [true; 3],
        scale: [true; 3],
    }));
    let moved = scene.object_mut(target).unwrap();
    moved.location = Vec3::new(5.0, 0.0, 3.0);
    moved.rotation = Vec3::new(0.3, 0.4, 0.9);
    moved.scale = Vec3::new(1.0, 2.0, 1.0);
    assert_matrix("childOf", scene.world_matrix(owner).unwrap(), want("childOf"), 1e-4);

    // Bone constraints on a moved, turned armature.
    for (label, kind) in [
        ("bone-DAMPED_TRACK", ConstraintKind::DampedTrack { target: None, track_axis: TrackAxis::Y }),
        ("bone-TRACK_TO", ConstraintKind::TrackTo { target: None, track_axis: TrackAxis::NegZ, up_axis: UpAxis::Y }),
    ] {
        let mut scene = Scene::empty();
        let target = empty(&mut scene, "Target", Vec3::new(2.0, 1.0, 3.0), Vec3::ZERO);
        let mut bone = Bone::new("Bone", Vec3::ZERO, Vec3::new(0.0, 0.3, 1.0));
        bone.roll = 0.2;
        bone.constraints.push(Constraint::new(match kind {
            ConstraintKind::DampedTrack { track_axis, .. } => ConstraintKind::DampedTrack { target: on(target), track_axis },
            ConstraintKind::TrackTo { track_axis, up_axis, .. } => ConstraintKind::TrackTo { target: on(target), track_axis, up_axis },
            other => other,
        }));
        let rig = scene.add_object("Rig", ObjectData::Armature(Armature { bones: vec![bone] }));
        let object = scene.object_mut(rig).unwrap();
        object.location = Vec3::new(0.5, 0.0, 0.0);
        object.rotation = Vec3::new(0.0, 0.0, 0.4);
        let world = scene.world_matrix(rig).unwrap() * bone_matrices(&scene, rig, 0)["Bone"];
        assert_matrix(label, world, want(label), 1e-4);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
