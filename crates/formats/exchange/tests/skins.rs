//! Skins and morph targets: our files back into our scenes, our files in the reference modeller
//! (Blender, as an oracle when `KITION_BLENDER` names it), and its files in ours. Compared by
//! where the posed, keyed vertices end up in the world at several frames.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::path::PathBuf;
use std::process::Command;

use kition_scene_exchange::{Format, export, import, place};
use kition_scene_model::armature::{Armature, Bone};
use kition_scene_model::glam::{Mat4, Vec3};
use kition_scene_model::{Fill, Modifier, ModifierKind, ObjectData, Primitive, Property, Scene, ShapeKey, VertexGroup};
use serde_json::Value;

/// A two-bone rig deforming a column with a shape key, keyed on frames 1 and 21.
fn rigged() -> Scene {
    let mut scene = Scene::empty();
    let lower = Bone::new("Lower", Vec3::ZERO, Vec3::Z);
    let mut upper = Bone::new("Upper", Vec3::Z, Vec3::new(0.0, 0.2, 2.0));
    upper.roll = 0.4;
    upper.parent = Some("Lower".into());
    upper.connected = true;
    let rig = scene.add_object("Rig", ObjectData::Armature(Armature { bones: vec![lower, upper] }));
    let mut column = Primitive::Cylinder { vertices: 8, radius: 0.2, depth: 2.0, fill: Fill::Ngon }.build().unwrap();
    column.uvs.clear();
    column.transform(Mat4::from_translation(Vec3::Z));
    column.groups = vec![
        VertexGroup { name: "Lower".into(), weights: column.positions.iter().map(|point| 1.0 - point.z / 2.0).collect() },
        VertexGroup { name: "Upper".into(), weights: column.positions.iter().map(|point| point.z / 2.0).collect() },
    ];
    column.shape_keys = vec![ShapeKey {
        name: "Wide".into(),
        offsets: column.positions.iter().map(|point| Vec3::new(point.x, point.y, 0.0) * 0.5).collect(),
        value: 0.0,
    }];
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
    scene.insert_key_at(mesh, Property::ShapeKey, "Wide", 1.0).unwrap();
    {
        let ObjectData::Armature(armature) = &mut scene.object_mut(rig).unwrap().data else { panic!() };
        let lower = armature.bone_mut("Lower").unwrap();
        lower.pose.location = Vec3::new(0.1, 0.0, 0.05);
        lower.pose.rotation = Vec3::new(0.0, 0.4, 0.0);
        armature.bone_mut("Upper").unwrap().pose.rotation = Vec3::new(0.5, 0.2, -0.3);
    }
    scene.object_mut(mesh).unwrap().mesh_mut().unwrap().shape_keys[0].value = 1.0;
    for bone in ["Lower", "Upper"] {
        for property in [Property::BoneLocation, Property::BoneRotation, Property::BoneScale] {
            scene.insert_key_at(rig, property, bone, 21.0).unwrap();
        }
    }
    scene.insert_key_at(mesh, Property::ShapeKey, "Wide", 21.0).unwrap();
    scene.go_to_frame(1.0);
    scene.validate().unwrap();
    scene
}

/// Every mesh vertex in world space at `frame`, sorted (vertex order differs between programs).
fn posed(scene: &Scene, frame: f32) -> Vec<Vec3> {
    let at = scene.at_frame(frame);
    let mut out = Vec::new();
    for object in at.objects.iter().filter(|object| object.mesh().is_some()) {
        let world = at.world_matrix(object.id).unwrap();
        out.extend(object.evaluated_mesh(&at).unwrap().positions.iter().map(|point| world.transform_point3(*point)));
    }
    sort(out)
}

fn sort(mut points: Vec<Vec3>) -> Vec<Vec3> {
    let round = |value: f32| (value * 1000.0).round() as i64;
    points.sort_by_key(|point| (round(point.x), round(point.y), round(point.z)));
    points.dedup_by(|a, b| a.distance(*b) < 1e-4);
    points
}

fn worst(a: &[Vec3], b: &[Vec3]) -> f32 {
    assert_eq!(a.len(), b.len(), "vertex counts");
    // Each point against its nearest (sorting by rounded coordinates can swap near neighbours).
    a.iter().map(|point| b.iter().map(|other| point.distance(*other)).fold(f32::INFINITY, f32::min)).fold(0.0, f32::max)
}

#[test]
fn skins_and_shape_keys_round_trip() {
    let scene = rigged();
    assert!(worst(&posed(&scene, 1.0), &posed(&scene, 21.0)) > 0.3, "the pose and the shape key move the column");
    for format in [Format::Glb, Format::Gltf] {
        let bytes = export(&scene, format, None).unwrap();
        let imported = import(&bytes, format).unwrap();
        let mut back = Scene::empty();
        place(&mut back, imported).unwrap();
        back.validate().unwrap();
        let rig = back.objects.iter().find(|object| matches!(object.data, ObjectData::Armature(_))).unwrap();
        let ObjectData::Armature(armature) = &rig.data else { panic!() };
        assert_eq!(armature.bones.iter().map(|bone| bone.name.as_str()).collect::<Vec<_>>(), ["Lower", "Upper"]);
        assert_eq!(armature.bones[1].parent.as_deref(), Some("Lower"));
        let column = back.object_named("Column").unwrap();
        assert!(
            matches!(column.modifiers.first().map(|modifier| &modifier.kind), Some(ModifierKind::Armature { object: Some(id), .. }) if *id == rig.id)
        );
        let mesh = column.mesh().unwrap();
        assert_eq!(mesh.shape_keys.iter().map(|key| key.name.as_str()).collect::<Vec<_>>(), ["Wide"]);
        for vertex in 0..mesh.positions.len() {
            let sum: f32 = mesh.groups.iter().map(|group| group.weights[vertex]).sum();
            assert!((sum - 1.0).abs() < 1e-4, "weights sum to 1: {sum}");
        }
        for frame in [1.0, 6.0, 11.0, 21.0] {
            let difference = worst(&posed(&scene, frame), &posed(&back, frame));
            // Pose keys come back sampled per frame and linear between, so in-between frames
            // match the curve at the samples.
            assert!(difference < 2e-3, "{format:?} frame {frame}: {difference}");
        }
    }
}

const SCRIPT: &str = r#"
import bpy, json, sys
args = sys.argv[sys.argv.index("--") + 1:]
mode, path, out = args[0], args[1], args[2]
def posed():
    result = {}
    for frame in (1, 6, 11, 21):
        bpy.context.scene.frame_set(frame)
        deps = bpy.context.evaluated_depsgraph_get()
        points = []
        for o in bpy.context.scene.objects:
            # Skinned meshes only (the importer adds a bone display shape of its own).
            if o.type == "MESH" and len(o.vertex_groups):
                m = o.evaluated_get(deps).to_mesh()
                points += [list(o.matrix_world @ v.co) for v in m.vertices]
        result[frame] = points
    return result
if mode == "read":
    bpy.ops.object.select_all(action="SELECT"); bpy.ops.object.delete()
    bpy.ops.import_scene.gltf(filepath=path)
    bones = sum(len(o.data.bones) for o in bpy.context.scene.objects if o.type == "ARMATURE")
    sums = []
    for o in bpy.context.scene.objects:
        if o.type == "MESH" and len(o.vertex_groups):
            for v in o.data.vertices:
                sums.append(sum(g.weight for g in v.groups))
    keys = [k.name for o in bpy.context.scene.objects if o.type == "MESH" and o.data.shape_keys for k in o.data.shape_keys.key_blocks]
    json.dump({"bones": bones, "sums": sums, "keys": keys, "posed": posed()}, open(out, "w"))
else:
    bpy.ops.object.select_all(action="SELECT"); bpy.ops.object.delete()
    arm = bpy.data.armatures.new("Rig"); rig = bpy.data.objects.new("Rig", arm); bpy.context.scene.collection.objects.link(rig)
    rig.location = (1, 0, 0); rig.rotation_euler = (0, 0, 0.3)
    bpy.context.view_layer.objects.active = rig; bpy.ops.object.mode_set(mode="EDIT")
    lo = arm.edit_bones.new("Lower"); lo.head = (0, 0, 0); lo.tail = (0, 0, 1)
    up = arm.edit_bones.new("Upper"); up.head = (0, 0, 1); up.tail = (0, 0.2, 2); up.roll = 0.4; up.parent = lo; up.use_connect = True
    bpy.ops.object.mode_set(mode="OBJECT")
    bpy.ops.mesh.primitive_cylinder_add(vertices=8, radius=0.2, depth=2.0, location=(0, 0, 1))
    col = bpy.context.active_object
    import mathutils
    col.data.transform(mathutils.Matrix.Translation((0, 0, 1))); col.location = (0, 0, 0)
    col.parent = rig
    lo_g = col.vertex_groups.new(name="Lower"); up_g = col.vertex_groups.new(name="Upper")
    for v in col.data.vertices:
        t = v.co.z / 2; lo_g.add([v.index], 1 - t, "REPLACE"); up_g.add([v.index], t, "REPLACE")
    col.modifiers.new("Armature", "ARMATURE").object = rig
    col.shape_key_add(name="Basis"); wide = col.shape_key_add(name="Wide")
    for point in wide.data:
        point.co = mathutils.Vector((point.co.x * 1.5, point.co.y * 1.5, point.co.z))
    for b in rig.pose.bones:
        b.rotation_mode = "XYZ"
        for p in ("location", "rotation_euler", "scale"): b.keyframe_insert(p, frame=1)
    wide.value = 0.0; wide.keyframe_insert("value", frame=1)
    rig.pose.bones["Lower"].rotation_euler = (0, 0.4, 0); rig.pose.bones["Upper"].rotation_euler = (0.5, 0.2, -0.3)
    for b in rig.pose.bones:
        for p in ("location", "rotation_euler", "scale"): b.keyframe_insert(p, frame=21)
    wide.value = 1.0; wide.keyframe_insert("value", frame=21)
    bpy.ops.export_scene.gltf(filepath=path, export_format="GLB")
    json.dump({"posed": posed()}, open(out, "w"))
"#;

fn blender(mode: &str, file: &PathBuf, out: &PathBuf) -> Option<Value> {
    let executable = std::env::var_os("KITION_BLENDER")?;
    let dir = out.parent().unwrap();
    std::fs::write(dir.join("skins.py"), SCRIPT).unwrap();
    let status = Command::new(executable)
        .args(["-b", "--factory-startup", "--python"])
        .arg(dir.join("skins.py"))
        .arg("--")
        .arg(mode)
        .arg(file)
        .arg(out)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "Blender failed");
    Some(serde_json::from_str(&std::fs::read_to_string(out).unwrap()).unwrap())
}

fn points(value: &Value) -> Vec<Vec3> {
    sort(
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|point| Vec3::new(point[0].as_f64().unwrap() as f32, point[1].as_f64().unwrap() as f32, point[2].as_f64().unwrap() as f32))
            .collect(),
    )
}

#[test]
fn the_reference_reads_our_skins_and_we_read_its() {
    if std::env::var_os("KITION_BLENDER").is_none() {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    }
    let dir = std::env::temp_dir().join(format!("kition-scene-skins-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // Ours, read there.
    let scene = rigged();
    let ours = dir.join("ours.glb");
    std::fs::write(&ours, export(&scene, Format::Glb, None).unwrap()).unwrap();
    let read = blender("read", &ours, &dir.join("read.json")).unwrap();
    assert_eq!(read["bones"], 2);
    assert_eq!(read["keys"], serde_json::json!(["Basis", "Wide"]));
    assert!(read["sums"].as_array().unwrap().iter().all(|sum| (sum.as_f64().unwrap() - 1.0).abs() < 1e-3));
    let mut report = String::new();
    for frame in [1, 6, 11, 21] {
        let difference = worst(&posed(&scene, frame as f32), &points(&read["posed"][frame.to_string()]));
        report += &format!(" ours→ref f{frame} {difference:.5}");
        assert!(difference < 2e-3, "our file in the reference, frame {frame}: {difference}");
    }

    // Theirs, read here.
    let theirs = dir.join("theirs.glb");
    let written = blender("write", &theirs, &dir.join("written.json")).unwrap();
    let mut scene = Scene::empty();
    place(&mut scene, import(&std::fs::read(&theirs).unwrap(), Format::Glb).unwrap()).unwrap();
    scene.validate().unwrap();
    let bones = scene
        .objects
        .iter()
        .filter_map(|object| if let ObjectData::Armature(armature) = &object.data { Some(armature.bones.len()) } else { None })
        .sum::<usize>();
    assert_eq!(bones, 2);
    for frame in [1, 6, 11, 21] {
        let difference = worst(&posed(&scene, frame as f32), &points(&written["posed"][frame.to_string()]));
        report += &format!(" ref→ours f{frame} {difference:.5}");
        assert!(difference < 2e-3, "the reference's file here, frame {frame}: {difference}");
    }
    eprintln!("worst vertex differences:{report}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hostile_skins_are_refused() {
    let bytes = export(&rigged(), Format::Gltf, None).unwrap();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    let mut bad_joint = json.clone();
    bad_joint["skins"][0]["joints"][0] = serde_json::json!(999);
    assert!(import(&serde_json::to_vec(&bad_joint).unwrap(), Format::Gltf).is_err());
    let mut bad_binds = json.clone();
    let accessor = bad_binds["skins"][0]["inverseBindMatrices"].as_u64().unwrap() as usize;
    bad_binds["accessors"][accessor]["count"] = serde_json::json!(1);
    assert!(import(&serde_json::to_vec(&bad_binds).unwrap(), Format::Gltf).is_err());
    let mut cycle = json;
    cycle["nodes"][0]["children"] = serde_json::json!([0]);
    assert!(import(&serde_json::to_vec(&cycle).unwrap(), Format::Gltf).is_err());
}
