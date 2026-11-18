//! Normal maps through glTF: our round trip, and (when `KITION_BLENDER` names the reference
//! modeller's executable) its importer wiring our map into its surface with our strength, and
//! our importer reading the map and strength it exports.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::path::PathBuf;
use std::process::Command;

use kition_scene_exchange::{Format, export, import, place};
use kition_scene_model::{Image, ObjectData, Primitive, Scene};
use serde_json::Value;

/// A 2 × 2 PNG tilted towards +U.
fn normal_png() -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, 2, 2);
    encoder.set_color(png::ColorType::Rgb);
    encoder.write_header().unwrap().write_image_data(&[200, 128, 230].repeat(4)).unwrap();
    out
}

fn bumpy_cube(strength: f32) -> Scene {
    let mut scene = Scene::empty();
    let cube = scene.add_object("Cube", ObjectData::Mesh(Primitive::Cube { size: 2.0 }.build().unwrap()));
    let material = scene.add_material("Bumpy");
    let hash = scene.add_image(Image::new("bumps.png", normal_png()).unwrap());
    let found = scene.material_mut(material).unwrap();
    found.normal_texture = Some(hash);
    found.normal_strength = strength;
    scene.object_mut(cube).unwrap().material = Some(material);
    scene
}

#[test]
fn normal_maps_round_trip_through_gltf() {
    for (format, strength) in [(Format::Glb, 0.7), (Format::Gltf, 1.0)] {
        let scene = bumpy_cube(strength);
        let bytes = export(&scene, format, None).unwrap();
        let mut back = Scene::empty();
        place(&mut back, import(&bytes, format).unwrap()).unwrap();
        let material = &back.materials[0];
        let hash = material.normal_texture.as_deref().expect("the map comes back");
        assert_eq!(back.image(hash).unwrap().bytes.as_slice(), normal_png().as_slice());
        assert!((material.normal_strength - strength).abs() < 1e-6);
        assert!(material.base_color_texture.is_none());
    }
}

/// `read`: imports `path` and reports the normal map wiring. `make`: first builds a cube whose
/// material has a Normal Map node (strength 0.5) and exports it to `path`.
const SCRIPT: &str = r#"
import bpy, json, sys
mode, path, out = sys.argv[sys.argv.index("--") + 1:]
bpy.ops.object.select_all(action="SELECT"); bpy.ops.object.delete()
if mode == "make":
    bpy.ops.mesh.primitive_cube_add()
    image = bpy.data.images.new("bumps", 2, 2)
    image.pixels = [0.78, 0.5, 0.9, 1] * 4
    image.colorspace_settings.name = "Non-Color"
    image.filepath_raw = path + ".png"; image.file_format = "PNG"; image.save(); image.pack()
    material = bpy.data.materials.new("Bumpy"); material.use_nodes = True
    nodes, links = material.node_tree.nodes, material.node_tree.links
    texture = nodes.new("ShaderNodeTexImage"); texture.image = image
    normal = nodes.new("ShaderNodeNormalMap"); normal.inputs["Strength"].default_value = 0.5
    links.new(texture.outputs["Color"], normal.inputs["Color"])
    links.new(normal.outputs["Normal"], nodes["Principled BSDF"].inputs["Normal"])
    bpy.context.active_object.data.materials.append(material)
    bpy.ops.export_scene.gltf(filepath=path)
    bpy.ops.object.select_all(action="SELECT"); bpy.ops.object.delete()
bpy.ops.import_scene.gltf(filepath=path)
o = [o for o in bpy.context.scene.objects if o.type == "MESH"][0]
report = []
for material in o.data.materials:
    for node in material.node_tree.nodes:
        if node.type != "NORMAL_MAP":
            continue
        image = [link.from_node for link in node.inputs["Color"].links if link.from_node.type == "TEX_IMAGE"]
        wired = any(link.to_node.type == "BSDF_PRINCIPLED" and link.to_socket.name == "Normal" for link in node.outputs["Normal"].links)
        report.append({"strength": round(node.inputs["Strength"].default_value, 4), "wired": wired, "size": list(image[0].image.size) if image else None, "space": node.space})
json.dump(report, open(out, "w"))
"#;

fn run(blender: &std::ffi::OsStr, mode: &str, path: &std::path::Path) -> Value {
    let dir = path.parent().unwrap();
    let script = dir.join("normals.py");
    std::fs::write(&script, SCRIPT).unwrap();
    let out = dir.join(format!("{mode}.json"));
    let status = Command::new(blender)
        .args(["-b", "--factory-startup", "--python"])
        .arg(&script)
        .arg("--")
        .args([PathBuf::from(mode), path.to_path_buf(), out.clone()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "Blender failed");
    serde_json::from_str(&std::fs::read_to_string(out).unwrap()).unwrap()
}

#[test]
fn the_reference_reads_our_normal_maps_and_we_read_its() {
    let Some(blender) = std::env::var_os("KITION_BLENDER") else {
        eprintln!("skipped: set KITION_BLENDER to the reference modeller's executable");
        return;
    };
    let dir = std::env::temp_dir().join(format!("kition-scene-normals-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ours = dir.join("ours.glb");
    std::fs::write(&ours, export(&bumpy_cube(0.7), Format::Glb, None).unwrap()).unwrap();
    let report = run(&blender, "read", &ours);
    assert_eq!(report, serde_json::json!([{ "strength": 0.7, "wired": true, "size": [2, 2], "space": "TANGENT" }]));

    let theirs = dir.join("theirs.glb");
    run(&blender, "make", &theirs);
    let mut back = Scene::empty();
    place(&mut back, import(&std::fs::read(&theirs).unwrap(), Format::Glb).unwrap()).unwrap();
    let material = &back.materials[0];
    assert!(material.normal_texture.is_some(), "the map comes with the material");
    assert!((material.normal_strength - 0.5).abs() < 1e-4, "{}", material.normal_strength);
    let _ = std::fs::remove_dir_all(&dir);
}
