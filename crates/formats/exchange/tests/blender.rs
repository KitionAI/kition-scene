//! Cross-checks against the reference modeller, run as a separate program: our files open
//! there with the same geometry, cameras, and lights, and its files open here. Skipped unless
//! `KITION_BLENDER` names the modeller's executable.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

use kition_scene_exchange::{Format, export, import, place};
use kition_scene_model::glam::{Mat4, Vec3};
use kition_scene_model::{Fill, ObjectData, Primitive, Scene};
use serde_json::Value;

/// Reads every file named on the command line into an empty scene, or writes the default scene
/// plus a UV sphere; either way prints what is in the scene as JSON to the last argument.
const SCRIPT: &str = r#"
import bpy, json, sys
args = sys.argv[sys.argv.index("--") + 1:]
mode, out = args[0], args[-1]
def describe():
    deps = bpy.context.evaluated_depsgraph_get()
    objects = []
    for o in bpy.context.scene.objects:
        item = {"name": o.name, "type": o.type, "location": list(o.matrix_world.translation)}
        if o.type == "MESH":
            m = o.evaluated_get(deps).to_mesh()
            m.calc_loop_triangles()
            world = [o.matrix_world @ v.co for v in m.vertices]
            item.update({"vertices": len(m.vertices), "faces": len(m.polygons), "triangles": len(m.loop_triangles),
                         "min": [min(p[i] for p in world) for i in range(3)], "max": [max(p[i] for p in world) for i in range(3)]})
        if o.type == "LIGHT":
            item.update({"light": o.data.type, "energy": o.data.energy, "forward": list(o.matrix_world.to_3x3() @ __import__("mathutils").Vector((0, 0, -1)))})
        if o.type == "CAMERA":
            import math
            c, r = o.data, bpy.context.scene.render
            aspect = r.resolution_x / r.resolution_y
            if c.sensor_fit == "VERTICAL" or (c.sensor_fit == "AUTO" and aspect < 1):
                size = c.sensor_height if c.sensor_fit == "VERTICAL" else c.sensor_width
                yfov = 2 * math.atan(size / 2 / c.lens)
            else:
                yfov = 2 * math.atan(math.tan(math.atan(c.sensor_width / 2 / c.lens)) / aspect)
            item.update({"lens": c.lens, "yfov": yfov, "forward": list(o.matrix_world.to_3x3() @ __import__("mathutils").Vector((0, 0, -1)))})
        objects.append(item)
    return objects
if mode == "read":
    bpy.ops.object.select_all(action="SELECT"); bpy.ops.object.delete()
    result = {}
    for path in args[1:-1]:
        for o in list(bpy.context.scene.objects): bpy.data.objects.remove(o)
        if path.endswith((".glb", ".gltf")): bpy.ops.import_scene.gltf(filepath=path)
        elif path.endswith(".obj"): bpy.ops.wm.obj_import(filepath=path)
        elif path.endswith(".stl"): bpy.ops.wm.stl_import(filepath=path)
        result[path.rsplit(".", 1)[1]] = describe()
    json.dump(result, open(out, "w"))
else:
    bpy.ops.mesh.primitive_uv_sphere_add(location=(3, 0, 0))
    bpy.ops.mesh.primitive_cylinder_add(vertices=12, location=(-3, 1, 0.5))
    folder = args[1]
    bpy.ops.export_scene.gltf(filepath=folder + "/b.glb", export_cameras=True, export_lights=True)
    bpy.ops.wm.obj_export(filepath=folder + "/b.obj", export_materials=False)
    bpy.ops.wm.stl_export(filepath=folder + "/b.stl")
    json.dump({"scene": describe()}, open(out, "w"))
"#;

fn blender() -> Option<PathBuf> {
    std::env::var_os("KITION_BLENDER").map(PathBuf::from)
}

fn folder(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("kition-scene-exchange-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn run(blender: &Path, dir: &Path, args: &[String]) -> Value {
    let script = dir.join("check.py");
    std::fs::write(&script, SCRIPT).unwrap();
    let out = dir.join("result.json");
    let status = Command::new(blender)
        .args(["-b", "--factory-startup", "--python"])
        .arg(&script)
        .arg("--")
        .args(args)
        .arg(&out)
        .output()
        .expect("the modeller runs");
    assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stderr));
    serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap()
}

fn vec3(value: &Value) -> Vec3 {
    Vec3::new(value[0].as_f64().unwrap() as f32, value[1].as_f64().unwrap() as f32, value[2].as_f64().unwrap() as f32)
}

fn find<'a>(objects: &'a Value, name: &str) -> &'a Value {
    objects.as_array().unwrap().iter().find(|object| object["name"] == name).unwrap_or_else(|| panic!("no {name} in {objects}"))
}

fn sample() -> Scene {
    let mut scene = Scene::starter();
    let sphere = scene.add_object("Sphere", ObjectData::Mesh(Primitive::UvSphere { segments: 16, rings: 8, radius: 1.0 }.build().unwrap()));
    scene.object_mut(sphere).unwrap().location = Vec3::new(3.0, 1.0, 0.5);
    scene.object_mut(sphere).unwrap().rotation = Vec3::new(0.4, 0.0, 0.9);
    let cylinder = scene
        .add_object("Cylinder", ObjectData::Mesh(Primitive::Cylinder { vertices: 12, radius: 0.5, depth: 1.0, fill: Fill::Ngon }.build().unwrap()));
    scene.object_mut(cylinder).unwrap().location = Vec3::new(-2.0, 0.0, 1.0);
    scene.set_parent(cylinder, Some(sphere)).unwrap();
    scene
}

#[test]
fn our_files_open_in_the_reference_modeller() {
    let Some(blender) = blender() else { return };
    let dir = folder("ours");
    let scene = sample();
    let mut paths = Vec::new();
    for format in [Format::Glb, Format::Gltf, Format::Obj, Format::Stl] {
        let path = dir.join(format!("ours.{}", format.extension()));
        std::fs::write(&path, export(&scene, format, None).unwrap()).unwrap();
        paths.push(path.to_string_lossy().into_owned());
    }
    let mut args = vec!["read".to_string()];
    args.extend(paths);
    let result = run(&blender, &dir, &args);
    let world_bounds = |name: &str| {
        let object = scene.object_named(name).unwrap();
        let mesh = object.evaluated_mesh(&scene).unwrap();
        mesh.bounds(scene.world_matrix(object.id).unwrap())
    };
    for format in ["glb", "gltf", "obj"] {
        let objects = &result[format];
        for name in ["Cube", "Sphere", "Cylinder"] {
            let object = find(objects, name);
            let mesh = scene.object_named(name).unwrap().mesh().unwrap();
            let triangles = mesh.triangles().len() as u64;
            assert_eq!(object["triangles"].as_u64(), Some(triangles), "{format} {name}");
            if format == "obj" {
                assert_eq!(object["vertices"].as_u64(), Some(mesh.positions.len() as u64), "{format} {name}");
                assert_eq!(object["faces"].as_u64(), Some(mesh.faces.len() as u64), "{format} {name}");
            }
            let bounds = world_bounds(name);
            assert!(
                vec3(&object["min"]).abs_diff_eq(bounds.min, 1e-4) && vec3(&object["max"]).abs_diff_eq(bounds.max, 1e-4),
                "{format} {name}: {object}"
            );
        }
        if format != "obj" {
            for name in ["Camera", "Light"] {
                let object = find(objects, name);
                let ours = scene.object_named(name).unwrap();
                let world = scene.world_matrix(ours.id).unwrap();
                assert!(vec3(&object["location"]).abs_diff_eq(world.transform_point3(Vec3::ZERO), 1e-4), "{format} {name}");
                let forward = world.transform_vector3(Vec3::NEG_Z).normalize();
                assert!(vec3(&object["forward"]).normalize().abs_diff_eq(forward, 1e-4), "{format} {name} looks the wrong way: {object}");
            }
            assert!((find(objects, "Light")["energy"].as_f64().unwrap() - 1000.0).abs() < 0.1);
            // The modeller may store the same view as another lens on another sensor side, so
            // the vertical field of view is compared (a 50 mm lens on 36 mm across 16:9).
            let yfov = find(objects, "Camera")["yfov"].as_f64().unwrap();
            let want = 2.0 * ((18.0f64 / 50.0).atan().tan() / (1920.0 / 1080.0)).atan();
            assert!((yfov - want).abs() < 1e-5, "yfov {yfov} against {want}");
        }
    }
    let stl = result["stl"].as_array().unwrap();
    assert_eq!(stl.len(), 1);
    let triangles: usize =
        ["Cube", "Sphere", "Cylinder"].iter().map(|name| scene.object_named(name).unwrap().mesh().unwrap().triangles().len()).sum();
    assert_eq!(stl[0]["triangles"].as_u64(), Some(triangles as u64));
    let all = ["Cube", "Sphere", "Cylinder"]
        .iter()
        .map(|name| world_bounds(name))
        .fold(kition_scene_model::Bounds::EMPTY, kition_scene_model::Bounds::union);
    assert!(vec3(&stl[0]["min"]).abs_diff_eq(all.min, 1e-4) && vec3(&stl[0]["max"]).abs_diff_eq(all.max, 1e-4));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_reference_modellers_files_open_here() {
    let Some(blender) = blender() else { return };
    let dir = folder("theirs");
    let result = run(&blender, &dir, &["write".to_string(), dir.to_string_lossy().into_owned()]);
    let theirs = &result["scene"];
    for (file, format) in [("b.glb", Format::Glb), ("b.obj", Format::Obj), ("b.stl", Format::Stl)] {
        let mut scene = Scene::empty();
        place(&mut scene, import(&std::fs::read(dir.join(file)).unwrap(), format).unwrap()).unwrap();
        scene.validate().unwrap();
        if format == Format::Stl {
            let object = &scene.objects[0];
            let mesh = object.mesh().unwrap();
            let triangles: u64 = theirs.as_array().unwrap().iter().filter_map(|object| object["triangles"].as_u64()).sum();
            assert_eq!(mesh.faces.len() as u64, triangles);
            continue;
        }
        for name in ["Cube", "Sphere", "Cylinder"] {
            let reference = find(theirs, name);
            let object = scene.object_named(name).unwrap_or_else(|| panic!("{file}: no {name}"));
            let mesh = object.mesh().unwrap();
            assert_eq!(mesh.positions.len() as u64, reference["vertices"].as_u64().unwrap(), "{file} {name}: vertices");
            let triangles: usize = mesh.triangles().len();
            assert_eq!(triangles as u64, reference["triangles"].as_u64().unwrap(), "{file} {name}: triangles");
            if format == Format::Obj {
                assert_eq!(mesh.faces.len() as u64, reference["faces"].as_u64().unwrap(), "{file} {name}: faces");
            }
            let bounds = mesh.bounds(scene.world_matrix(object.id).unwrap());
            assert!(
                bounds.min.abs_diff_eq(vec3(&reference["min"]), 1e-4) && bounds.max.abs_diff_eq(vec3(&reference["max"]), 1e-4),
                "{file} {name}: {bounds:?} against {reference}"
            );
        }
        if format == Format::Glb {
            for name in ["Camera", "Light"] {
                let reference = find(theirs, name);
                let object = scene.object_named(name).unwrap();
                let world: Mat4 = scene.world_matrix(object.id).unwrap();
                assert!(world.transform_point3(Vec3::ZERO).abs_diff_eq(vec3(&reference["location"]), 1e-4), "{name}");
                assert!(
                    world.transform_vector3(Vec3::NEG_Z).normalize().abs_diff_eq(vec3(&reference["forward"]).normalize(), 1e-4),
                    "{name} looks the wrong way"
                );
            }
            let ObjectData::Light(light) = &scene.object_named("Light").unwrap().data else { panic!() };
            assert!((light.power - 1000.0).abs() < 0.1, "{}", light.power);
            let ObjectData::Camera(camera) = &scene.object_named("Camera").unwrap().data else { panic!() };
            assert!((camera.lens - 50.0).abs() < 1e-2, "{}", camera.lens);
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}
