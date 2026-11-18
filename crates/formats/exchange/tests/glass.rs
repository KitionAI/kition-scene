//! Glass and alpha through glTF (`alphaMode`, `alphaCutoff`, `KHR_materials_transmission`,
//! `KHR_materials_ior`): our round trip, and (when `KITION_BLENDER` names the reference modeller's
//! executable) its importer reading our materials and our importer reading its exports.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::path::PathBuf;
use std::process::Command;

use kition_scene_exchange::{Format, export, import, place};
use kition_scene_model::{AlphaMode, ObjectData, Primitive, Scene};
use serde_json::Value;

/// A glass cube, a half-transparent blended sphere, and a clipped plane.
fn sample() -> Scene {
    let mut scene = Scene::empty();
    for (name, primitive) in [
        ("Glass", Primitive::Cube { size: 2.0 }),
        ("Veil", Primitive::UvSphere { segments: 16, rings: 8, radius: 1.0 }),
        ("Leaf", Primitive::Plane { size: 2.0 }),
    ] {
        let object = scene.add_object(name, ObjectData::Mesh(primitive.build().unwrap()));
        let material = scene.add_material(name);
        let found = scene.material_mut(material).unwrap();
        match name {
            "Glass" => {
                found.transmission = 1.0;
                found.ior = 1.45;
                found.roughness = 0.05;
            }
            "Veil" => {
                found.base_color[3] = 0.4;
                found.alpha_mode = AlphaMode::Blend;
            }
            _ => {
                found.base_color[3] = 0.3;
                found.alpha_mode = AlphaMode::Clip;
                found.alpha_cutoff = 0.25;
            }
        }
        scene.object_mut(object).unwrap().material = Some(material);
    }
    scene
}

fn material<'a>(scene: &'a Scene, name: &str) -> &'a kition_scene_model::Material {
    scene.materials.iter().find(|material| material.name == name).unwrap()
}

#[test]
fn glass_and_alpha_round_trip_through_gltf() {
    for format in [Format::Glb, Format::Gltf] {
        let bytes = export(&sample(), format, None).unwrap();
        let mut back = Scene::empty();
        place(&mut back, import(&bytes, format).unwrap()).unwrap();
        let glass = material(&back, "Glass");
        assert_eq!((glass.transmission, glass.ior, glass.alpha_mode), (1.0, 1.45, AlphaMode::Opaque));
        let veil = material(&back, "Veil");
        assert_eq!(veil.alpha_mode, AlphaMode::Blend);
        assert!((veil.base_color[3] - 0.4).abs() < 1e-6);
        let leaf = material(&back, "Leaf");
        assert_eq!((leaf.alpha_mode, leaf.alpha_cutoff), (AlphaMode::Clip, 0.25));
    }
}

#[test]
fn hostile_values_are_kept_in_range() {
    let gltf = br#"{"asset":{"version":"2.0"},"materials":[{"name":"M","alphaMode":"MASK","alphaCutoff":7,
        "extensions":{"KHR_materials_transmission":{"transmissionFactor":-3},"KHR_materials_ior":{"ior":0}}}],
        "meshes":[],"nodes":[],"scenes":[{"nodes":[]}]}"#;
    let imported = import(gltf, Format::Gltf).unwrap();
    let material = &imported.materials[0];
    assert_eq!((material.alpha_cutoff, material.transmission, material.ior), (1.0, 0.0, 1.5));
}

/// `read`: imports `path` and reports each material's Principled values and render method.
/// `make`: first builds a glass cube (IOR 1.4, transmission 1) and a blended plane (alpha 0.35)
/// and exports them to `path`.
const SCRIPT: &str = r#"
import bpy, json, sys
mode, path, out = sys.argv[sys.argv.index("--") + 1:]
bpy.ops.object.select_all(action="SELECT"); bpy.ops.object.delete()
if mode == "make":
    for name, setup in (("Glass", {"Transmission Weight": 1.0, "IOR": 1.4, "Roughness": 0.1}), ("Veil", {"Alpha": 0.35})):
        bpy.ops.mesh.primitive_cube_add() if name == "Glass" else bpy.ops.mesh.primitive_plane_add(location=(3, 0, 0))
        material = bpy.data.materials.new(name); material.use_nodes = True
        principled = material.node_tree.nodes["Principled BSDF"]
        for key, value in setup.items():
            principled.inputs[key].default_value = value
        if name == "Veil":
            material.surface_render_method = "BLENDED"
        bpy.context.active_object.data.materials.append(material)
    bpy.ops.export_scene.gltf(filepath=path, export_format="GLB")
    bpy.ops.object.select_all(action="SELECT"); bpy.ops.object.delete()
    for block in list(bpy.data.materials): bpy.data.materials.remove(block)
bpy.ops.import_scene.gltf(filepath=path)
result = {}
for material in bpy.data.materials:
    if not material.use_nodes: continue
    principled = next((node for node in material.node_tree.nodes if node.type == "BSDF_PRINCIPLED"), None)
    if principled is None: continue
    result[material.name] = {
        "transmission": principled.inputs["Transmission Weight"].default_value,
        "ior": principled.inputs["IOR"].default_value,
        "alpha": principled.inputs["Alpha"].default_value,
        "method": material.surface_render_method,
    }
json.dump(result, open(out, "w"))
"#;

fn run(blender: &PathBuf, args: &[&str]) -> Value {
    let dir = std::env::temp_dir().join(format!("kition-scene-glass-{}-{}", args[0], std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("check.py");
    std::fs::write(&script, SCRIPT).unwrap();
    let out = dir.join("result.json");
    let status = Command::new(blender).args(["-b", "--factory-startup", "--python"]).arg(&script).arg("--").args(args).arg(&out).output().unwrap();
    assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stderr));
    serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap()
}

#[test]
fn the_reference_reads_our_glass_and_we_read_its() {
    let Some(blender) = std::env::var_os("KITION_BLENDER").map(PathBuf::from) else {
        eprintln!("skipped: set KITION_BLENDER to the reference modeller's executable");
        return;
    };
    let dir = std::env::temp_dir().join(format!("kition-scene-glass-files-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // Ours, read there.
    let ours = dir.join("ours.glb");
    std::fs::write(&ours, export(&sample(), Format::Glb, None).unwrap()).unwrap();
    let read = run(&blender, &["read", ours.to_str().unwrap()]);
    let glass = &read["Glass"];
    assert!((glass["transmission"].as_f64().unwrap() - 1.0).abs() < 1e-4, "{read}");
    assert!((glass["ior"].as_f64().unwrap() - 1.45).abs() < 1e-4, "{read}");
    let veil = &read["Veil"];
    assert!((veil["alpha"].as_f64().unwrap() - 0.4).abs() < 1e-4, "{read}");
    assert_eq!(veil["method"], "BLENDED", "{read}");
    // Theirs, read here.
    let theirs = dir.join("theirs.glb");
    run(&blender, &["make", theirs.to_str().unwrap()]);
    let imported = import(&std::fs::read(&theirs).unwrap(), Format::Glb).unwrap();
    let find = |name: &str| imported.materials.iter().find(|material| material.name == name).unwrap();
    let glass = find("Glass");
    assert!((glass.transmission - 1.0).abs() < 1e-4 && (glass.ior - 1.4).abs() < 1e-4, "{glass:?}");
    let veil = find("Veil");
    assert_eq!(veil.alpha_mode, AlphaMode::Blend, "{veil:?}");
    assert!((veil.base_color[3] - 0.35).abs() < 1e-3, "{veil:?}");
}
