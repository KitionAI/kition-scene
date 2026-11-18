//! Object animation through glTF: our own round trip, and (with `KITION_BLENDER` naming the
//! reference modeller's executable) its importer reading our keys and its exporter's keys read
//! here, compared frame by frame in world space.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::process::Command;

use kition_scene_exchange::{Format, export, import, place};
use kition_scene_model::fmodifier::{FModifier, FModifierKind};
use kition_scene_model::glam::{Mat4, Vec3};
use kition_scene_model::nla::BlendType;
use kition_scene_model::{ObjectData, Property, Scene};
use serde_json::Value;

/// The starter scene with the cube moving and turning over frames 1 to 21, and the camera
/// dollying in.
fn animated() -> Scene {
    let mut scene = Scene::starter();
    let cube = scene.object_named("Cube").unwrap().id;
    let camera = scene.object_named("Camera").unwrap().id;
    for property in [Property::Location, Property::Rotation] {
        scene.insert_key(cube, property, 1.0).unwrap();
    }
    scene.insert_key(camera, Property::Location, 1.0).unwrap();
    {
        let object = scene.object_mut(cube).unwrap();
        object.location = Vec3::new(4.0, 2.0, 1.0);
        object.rotation = Vec3::new(0.0, 0.0, 1.5);
    }
    scene.object_mut(camera).unwrap().location *= 0.6;
    for property in [Property::Location, Property::Rotation] {
        scene.insert_key(cube, property, 21.0).unwrap();
    }
    scene.insert_key(camera, Property::Location, 21.0).unwrap();
    scene.go_to_frame(1.0);
    scene
}

fn world(scene: &Scene, name: &str, frame: f32) -> Mat4 {
    let at = scene.at_frame(frame);
    at.world_matrix(at.object_named(name).unwrap().id).unwrap()
}

#[test]
fn animation_round_trips_through_gltf() {
    let scene = animated();
    let bytes = export(&scene, Format::Glb, None).unwrap();
    let imported = import(&bytes, Format::Glb).unwrap();
    assert!(!imported.animation.is_empty());
    let mut back = Scene::empty();
    place(&mut back, imported).unwrap();
    back.validate().unwrap();
    for name in ["Cube", "Camera"] {
        for frame in [1.0, 5.0, 11.0, 17.0, 21.0, 30.0] {
            let (a, b) = (world(&scene, name, frame), world(&back, name, frame));
            assert!(a.abs_diff_eq(b, 1e-3), "{name} at frame {frame}:\n{a}\n{b}");
        }
    }
    // The light does not move, so it has no keys.
    let light = back.object_named("Light").unwrap().id;
    assert!(back.animation.channels.iter().all(|channel| channel.target != light));
    // Keys land on every frame of the moving range.
    let cube = back.object_named("Cube").unwrap().id;
    let channel = back.channel(cube, Property::Location, 0).unwrap();
    assert_eq!(channel.keys.len(), 21);
    assert_eq!(channel.keys.first().unwrap().frame, 1.0);
}

#[test]
fn hostile_animation_data_is_refused() {
    let scene = animated();
    let bytes = export(&scene, Format::Gltf, None).unwrap();
    let mut json: Value = serde_json::from_slice(&bytes).unwrap();
    let pristine = json.clone();
    json["animations"][0]["channels"][0]["sampler"] = 99.into();
    assert!(import(&serde_json::to_vec(&json).unwrap(), Format::Gltf).is_err());
    let mut json = pristine.clone();
    let output = json["animations"][0]["samplers"][0]["output"].as_u64().unwrap() as usize;
    json["accessors"][output]["count"] = 1_000_000_000u64.into();
    assert!(import(&serde_json::to_vec(&json).unwrap(), Format::Gltf).is_err());
    let mut json = pristine;
    let input = json["animations"][0]["samplers"][0]["input"].as_u64().unwrap() as usize;
    json["accessors"][input]["type"] = "VEC2".into();
    assert!(import(&serde_json::to_vec(&json).unwrap(), Format::Gltf).is_err());
}

const SCRIPT: &str = r#"
import bpy, json, sys
args = sys.argv[sys.argv.index("--") + 1:]
mode, path, out = args
frames = [1, 5, 11, 17, 21]
def sample(names):
    result = {}
    for name in names:
        o = bpy.data.objects.get(name)
        if o is None: continue
        rows = []
        for f in frames:
            bpy.context.scene.frame_set(f)
            m = o.matrix_world
            rows.append([list(r) for r in m])
        result[name] = rows
    return result
if mode == "read":
    for o in list(bpy.data.objects): bpy.data.objects.remove(o)
    bpy.ops.import_scene.gltf(filepath=path)
    result = sample(["Cube", "Camera"])
else:
    o = bpy.data.objects["Cube"]
    o.location = (0, 0, 0); o.rotation_euler = (0, 0, 0)
    o.keyframe_insert("location", frame=1); o.keyframe_insert("rotation_euler", frame=1)
    o.location = (4, 2, 1); o.rotation_euler = (0.3, 0, 1.5)
    o.keyframe_insert("location", frame=21); o.keyframe_insert("rotation_euler", frame=21)
    result = sample(["Cube"])
    bpy.ops.export_scene.gltf(filepath=path)
json.dump(result, open(out, "w"))
"#;

fn blender(mode: &str, path: &std::path::Path) -> Option<Value> {
    let executable = std::env::var_os("KITION_BLENDER")?;
    let dir = path.parent().unwrap();
    let script = dir.join("animation.py");
    std::fs::write(&script, SCRIPT).unwrap();
    let out = dir.join(format!("{mode}.json"));
    let status = Command::new(executable)
        .args(["-b", "--factory-startup", "--python"])
        .arg(&script)
        .arg("--")
        .arg(mode)
        .arg(path)
        .arg(&out)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "the reference modeller failed");
    Some(serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap())
}

/// A row-major 4×4 from the reference's matrix rows.
fn matrix(rows: &Value) -> Mat4 {
    let row = |r: usize| -> [f32; 4] { std::array::from_fn(|c| rows[r][c].as_f64().unwrap() as f32) };
    Mat4::from_cols_array_2d(&[row(0), row(1), row(2), row(3)]).transpose()
}

#[test]
fn the_reference_reads_our_animation_and_we_read_its() {
    if std::env::var_os("KITION_BLENDER").is_none() {
        eprintln!("skipped: set KITION_BLENDER to the reference modeller's executable");
        return;
    }
    let dir = std::env::temp_dir().join(format!("kition-scene-animation-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let frames = [1.0, 5.0, 11.0, 17.0, 21.0];

    // Ours, read there.
    let scene = animated();
    let ours = dir.join("ours.glb");
    std::fs::write(&ours, export(&scene, Format::Glb, None).unwrap()).unwrap();
    let read = blender("read", &ours).unwrap();
    for name in ["Cube", "Camera"] {
        for (slot, frame) in frames.iter().enumerate() {
            let theirs = matrix(&read[name][slot]);
            let mine = world(&scene, name, *frame);
            // Cameras and lights look down -Z in both; compare positions and axes.
            assert!(theirs.w_axis.abs_diff_eq(mine.w_axis, 2e-3), "{name} at {frame}: {theirs} against {mine}");
            if !matches!(scene.object_named(name).unwrap().data, ObjectData::Camera(_)) {
                assert!(theirs.abs_diff_eq(mine, 2e-3), "{name} at {frame}: {theirs} against {mine}");
            }
        }
    }

    // Theirs, read here.
    let theirs_path = dir.join("theirs.glb");
    let written = blender("write", &theirs_path).unwrap();
    let mut here = Scene::empty();
    place(&mut here, import(&std::fs::read(&theirs_path).unwrap(), Format::Glb).unwrap()).unwrap();
    here.validate().unwrap();
    for (slot, frame) in frames.iter().enumerate() {
        let theirs = matrix(&written["Cube"][slot]);
        let mine = world(&here, "Cube", *frame);
        assert!(theirs.abs_diff_eq(mine, 2e-3), "Cube at {frame}: {theirs} against {mine}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The starter scene with layered animation: the cube's keys pushed down into a strip played
/// twice as slow from frame 5, a lift added on a track above at half influence, a turn of its
/// own repeating under a Cycles modifier; the light stepping along under a Stepped modifier.
fn layered() -> Scene {
    let mut scene = Scene::starter();
    let cube = scene.object_named("Cube").unwrap().id;
    let light = scene.object_named("Light").unwrap().id;
    scene.insert_key(cube, Property::Location, 1.0).unwrap();
    scene.object_mut(cube).unwrap().location.x = 4.0;
    scene.insert_key(cube, Property::Location, 11.0).unwrap();
    scene.nla_push_down(cube, Some("Slide")).unwrap();
    scene.object_mut(cube).unwrap().nla.tracks[0].strips[0].scale = 2.0;
    scene.object_mut(cube).unwrap().nla.tracks[0].strips[0].start = 5.0;
    scene.object_mut(cube).unwrap().location.z = 1.0;
    scene.insert_key(cube, Property::Location, 1.0).unwrap();
    scene.nla_push_down(cube, Some("Lift")).unwrap();
    scene.object_mut(cube).unwrap().nla.tracks[1].strips[0].blend = BlendType::Add;
    scene.object_mut(cube).unwrap().nla.tracks[1].strips[0].influence = Some(0.5);
    scene.insert_key(cube, Property::Rotation, 1.0).unwrap();
    scene.object_mut(cube).unwrap().rotation.z = 1.0;
    scene.insert_key(cube, Property::Rotation, 9.0).unwrap();
    for channel in scene.animation.channels.iter_mut().filter(|channel| channel.target == cube) {
        channel.modifiers.push(FModifier::new(FModifierKind::new("cycles").unwrap()));
    }
    scene.insert_key(light, Property::Location, 1.0).unwrap();
    scene.object_mut(light).unwrap().location.y += 3.0;
    scene.insert_key(light, Property::Location, 25.0).unwrap();
    for channel in scene.animation.channels.iter_mut().filter(|channel| channel.target == light) {
        channel.modifiers.push(FModifier::new(FModifierKind::Stepped { step: 4.0, offset: 0.0, start: None, end: None }));
    }
    scene.animation.timeline.end = 40;
    scene.validate().unwrap();
    scene.go_to_frame(1.0);
    scene
}

/// Every frame of the timeline matches between the layered scene and what a file brought back.
fn same_motion(scene: &Scene, back: &Scene) {
    for name in ["Cube", "Light"] {
        for frame in 1..=40 {
            let (a, b) = (scene.at_frame(frame as f32), back.at_frame(frame as f32));
            let (a, b) = (a.world_matrix(a.object_named(name).unwrap().id).unwrap(), b.world_matrix(b.object_named(name).unwrap().id).unwrap());
            assert!(a.abs_diff_eq(b, 1e-4), "{name} at frame {frame}:\n{a}\n{b}");
        }
    }
}

#[test]
fn nla_and_modifiers_go_out_baked_in_gltf() {
    let scene = layered();
    let imported = import(&export(&scene, Format::Glb, None).unwrap(), Format::Glb).unwrap();
    let mut back = Scene::empty();
    place(&mut back, imported).unwrap();
    back.validate().unwrap();
    same_motion(&scene, &back);
}
