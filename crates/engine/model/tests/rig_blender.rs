//! Bones, poses, and skinning against the reference application (Blender), run as an oracle:
//! the same armature and weights, keyed the same way, must put the same vertices in the same
//! places. Runs when `KITION_BLENDER` names a Blender executable; otherwise it is skipped.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_scene_model::armature::{Armature, Bone, bone_rotation};
use kition_scene_model::glam::{Mat4, Vec3};
use kition_scene_model::{Fill, Modifier, ModifierKind, ObjectData, Primitive, Property, Scene, VertexGroup};

const SCRIPT: &str = r#"
import bpy, json, sys
bpy.ops.object.select_all(action='SELECT'); bpy.ops.object.delete()
out = {}
arm = bpy.data.armatures.new("Rig"); rig = bpy.data.objects.new("Rig", arm); bpy.context.scene.collection.objects.link(rig)
rig.location = (1.0, 0.0, 0.0); rig.rotation_euler = (0.0, 0.0, 0.3)
bpy.context.view_layer.objects.active = rig; bpy.ops.object.mode_set(mode='EDIT')
specs = [("Up",(0,0,0),(0,0,1),0.0),("Down",(0,0,0),(0,0,-1),0.0),("X",(0,0,0),(1,0,0),0.0),("Diag",(1,2,3),(2,1,5),0.7),("Roll",(0,0,0),(0,0,1),1.2),("NegY",(0,0,0),(0,-1,0),0.3)]
for n,h,t,r in specs:
    b = arm.edit_bones.new(n); b.head=h; b.tail=t; b.roll=r; b.use_deform=False
lower = arm.edit_bones.new("Lower"); lower.head=(0,0,0); lower.tail=(0,0,1)
upper = arm.edit_bones.new("Upper"); upper.head=(0,0,1); upper.tail=(0,0.2,2); upper.roll=0.4; upper.parent=lower; upper.use_connect=True
bpy.ops.object.mode_set(mode='OBJECT')
out["rest"] = {b.name: [list(c) for c in b.matrix_local.transposed()] for b in arm.bones}
bpy.ops.mesh.primitive_cylinder_add(vertices=8, radius=0.2, depth=2.0, location=(0,0,0))
column = bpy.context.active_object
column.data.transform(__import__('mathutils').Matrix.Translation((0,0,1)))
column.location = (1.0, 0.0, 0.0); column.rotation_euler = (0.0, 0.0, 0.3)
lo = column.vertex_groups.new(name="Lower"); up = column.vertex_groups.new(name="Upper")
for v in column.data.vertices:
    t = v.co.z / 2.0
    lo.add([v.index], 1.0 - t, 'REPLACE'); up.add([v.index], t, 'REPLACE')
mod = column.modifiers.new("Armature", 'ARMATURE'); mod.object = rig
for b in rig.pose.bones:
    b.rotation_mode = 'XYZ'
scene = bpy.context.scene
scene.frame_set(1)
for b in rig.pose.bones:
    b.keyframe_insert("location", frame=1); b.keyframe_insert("rotation_euler", frame=1); b.keyframe_insert("scale", frame=1)
pl = rig.pose.bones["Lower"]; pu = rig.pose.bones["Upper"]
pl.location = (0.1, 0.0, 0.05); pl.rotation_euler = (0.0, 0.4, 0.0); pl.scale = (1.0, 1.2, 1.0)
pu.rotation_euler = (0.5, 0.2, -0.3)
for b in (pl, pu):
    b.keyframe_insert("location", frame=21); b.keyframe_insert("rotation_euler", frame=21); b.keyframe_insert("scale", frame=21)
frames = {}
for frame in (1, 11, 21):
    scene.frame_set(frame)
    deps = bpy.context.evaluated_depsgraph_get()
    ev = column.evaluated_get(deps)
    frames[frame] = [list(ev.matrix_world @ v.co) for v in ev.data.vertices]
out["frames"] = frames
out["vertices"] = [list(v.co) for v in column.data.vertices]
json.dump(out, open(sys.argv[sys.argv.index('--') + 1], 'w'))
"#;

#[test]
fn bones_and_skinning_match_the_reference() {
    let Some(blender) = std::env::var_os("KITION_BLENDER") else {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    };
    let dir = std::env::temp_dir().join(format!("kition-scene-rig-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("rig.py"), SCRIPT).unwrap();
    let output = dir.join("rig.json");
    let status = std::process::Command::new(blender)
        .args(["-b", "--factory-startup", "--python"])
        .arg(dir.join("rig.py"))
        .arg("--")
        .arg(&output)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let reference: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&output).unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    // Rest matrices: the bone's axes from head, tail, and roll.
    let specs = [
        ("Up", Vec3::ZERO, Vec3::Z, 0.0),
        ("Down", Vec3::ZERO, Vec3::NEG_Z, 0.0),
        ("X", Vec3::ZERO, Vec3::X, 0.0),
        ("Diag", Vec3::new(1.0, 2.0, 3.0), Vec3::new(2.0, 1.0, 5.0), 0.7),
        ("Roll", Vec3::ZERO, Vec3::Z, 1.2),
        ("NegY", Vec3::ZERO, Vec3::NEG_Y, 0.3),
    ];
    for (name, head, tail, roll) in specs {
        let ours = bone_rotation(tail - head, roll);
        let columns = &reference["rest"][name];
        for (axis, column) in [ours.x_axis, ours.y_axis, ours.z_axis].iter().enumerate() {
            let theirs = Vec3::new(
                columns[axis][0].as_f64().unwrap() as f32,
                columns[axis][1].as_f64().unwrap() as f32,
                columns[axis][2].as_f64().unwrap() as f32,
            );
            assert!(column.abs_diff_eq(theirs, 1e-4), "{name} axis {axis}: {column} against {theirs}");
        }
    }

    // The same rig, column, weights, and keys.
    let mut scene = Scene::empty();
    let mut lower = Bone::new("Lower", Vec3::ZERO, Vec3::Z);
    lower.deform = true;
    let mut upper = Bone::new("Upper", Vec3::Z, Vec3::new(0.0, 0.2, 2.0));
    upper.roll = 0.4;
    upper.parent = Some("Lower".into());
    upper.connected = true;
    let rig = scene.add_object("Rig", ObjectData::Armature(Armature { bones: vec![lower, upper] }));
    let mut column = Primitive::Cylinder { vertices: 8, radius: 0.2, depth: 2.0, fill: Fill::Ngon }.build().unwrap();
    column.transform(Mat4::from_translation(Vec3::Z));
    let lower_weights: Vec<f32> = column.positions.iter().map(|point| 1.0 - point.z / 2.0).collect();
    let upper_weights: Vec<f32> = column.positions.iter().map(|point| point.z / 2.0).collect();
    column.groups = vec![VertexGroup { name: "Lower".into(), weights: lower_weights }, VertexGroup { name: "Upper".into(), weights: upper_weights }];
    let positions = column.positions.clone();
    let mesh = scene.add_object("Column", ObjectData::Mesh(column));
    for id in [rig, mesh] {
        let object = scene.object_mut(id).unwrap();
        object.location = Vec3::new(1.0, 0.0, 0.0);
        object.rotation = Vec3::new(0.0, 0.0, 0.3);
    }
    scene.object_mut(mesh).unwrap().modifiers.push(Modifier::new(ModifierKind::Armature {
        object: Some(rig),
        preserve_volume: false,
        deform: Default::default(),
    }));
    for bone in ["Lower", "Upper"] {
        for property in [Property::BoneLocation, Property::BoneRotation, Property::BoneScale] {
            scene.insert_key_at(rig, property, bone, 1.0).unwrap();
        }
    }
    {
        let ObjectData::Armature(armature) = &mut scene.object_mut(rig).unwrap().data else { panic!() };
        let lower = armature.bone_mut("Lower").unwrap();
        lower.pose.location = Vec3::new(0.1, 0.0, 0.05);
        lower.pose.rotation = Vec3::new(0.0, 0.4, 0.0);
        lower.pose.scale = Vec3::new(1.0, 1.2, 1.0);
        armature.bone_mut("Upper").unwrap().pose.rotation = Vec3::new(0.5, 0.2, -0.3);
    }
    for bone in ["Lower", "Upper"] {
        for property in [Property::BoneLocation, Property::BoneRotation, Property::BoneScale] {
            scene.insert_key_at(rig, property, bone, 21.0).unwrap();
        }
    }
    scene.validate().unwrap();

    // Vertices are matched by rest position (the two cylinders list them in different orders).
    let rest: Vec<Vec3> = reference["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|point| Vec3::new(point[0].as_f64().unwrap() as f32, point[1].as_f64().unwrap() as f32, point[2].as_f64().unwrap() as f32))
        .collect();
    let order: Vec<usize> =
        positions.iter().map(|point| rest.iter().position(|other| other.distance(*point) < 1e-4).expect("a vertex of the reference")).collect();
    let mut worst = 0.0f32;
    for frame in [1, 11, 21] {
        let posed = scene.at_frame(frame as f32);
        let world = posed.world_matrix(mesh).unwrap();
        let evaluated = posed.object(mesh).unwrap().evaluated_mesh(&posed).unwrap();
        let theirs = reference["frames"][frame.to_string()].as_array().unwrap();
        for (ours, slot) in evaluated.positions.iter().zip(&order) {
            let point = &theirs[*slot];
            let theirs = Vec3::new(point[0].as_f64().unwrap() as f32, point[1].as_f64().unwrap() as f32, point[2].as_f64().unwrap() as f32);
            let ours = world.transform_point3(*ours);
            worst = worst.max(ours.distance(theirs));
            assert!(ours.distance(theirs) < 2e-3, "frame {frame}: {ours} against {theirs}");
        }
    }
    let posed = scene.at_frame(21.0);
    let moved = posed
        .object(mesh)
        .unwrap()
        .evaluated_mesh(&posed)
        .unwrap()
        .positions
        .iter()
        .zip(&positions)
        .map(|(a, b)| a.distance(*b))
        .fold(0.0f32, f32::max);
    assert!(moved > 0.3, "the pose moves the column ({moved})");
    eprintln!("skinned column: worst vertex difference {worst:.6} m over frames 1, 11, 21 (the pose moves vertices up to {moved:.3} m)");
}
