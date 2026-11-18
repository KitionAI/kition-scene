//! Texture coordinates and images through glTF and OBJ, here and in the reference modeller
//! (the cross-checks run when `KITION_BLENDER` names its executable).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use kition_scene_exchange::{Format, export, import, place};
use kition_scene_model::glam::Vec3;
use kition_scene_model::{Image, Mesh, ObjectData, Primitive, Scene};
use serde_json::Value;

/// Each corner's coordinate, keyed by its face's position set and its own position.
type Corners = BTreeMap<(Vec<[i64; 3]>, [i64; 3]), [i64; 2]>;

/// A 4 × 2 PNG.
fn png_bytes() -> Vec<u8> {
    let pixels: [[u8; 4]; 8] = [
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 255, 255, 255],
        [0, 0, 0, 255],
        [128, 128, 128, 255],
        [255, 255, 0, 255],
        [0, 255, 255, 255],
    ];
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, 4, 2);
    encoder.set_color(png::ColorType::Rgba);
    encoder.write_header().unwrap().write_image_data(&pixels.concat()).unwrap();
    out
}

fn textured_cube() -> Scene {
    let mut scene = Scene::empty();
    let cube = scene.add_object("Cube", ObjectData::Mesh(Primitive::Cube { size: 2.0 }.build().unwrap()));
    let material = scene.add_material("Painted");
    let hash = scene.add_image(Image::new("paint.png", png_bytes()).unwrap());
    scene.material_mut(material).unwrap().base_color_texture = Some(hash);
    scene.object_mut(cube).unwrap().material = Some(material);
    scene
}

/// Each corner's coordinate by (rounded position, face's rounded position set).
fn corners(mesh: &Mesh, offset: Vec3) -> Corners {
    let key = |point: Vec3| [point.x, point.y, point.z].map(|value| ((value - 0.0) * 1000.0).round() as i64);
    let mut out = BTreeMap::new();
    for (face, vertices) in mesh.faces.iter().enumerate() {
        let mut outline: Vec<[i64; 3]> = vertices.iter().map(|vertex| key(mesh.positions[*vertex as usize] + offset)).collect();
        outline.sort_unstable();
        for (corner, vertex) in vertices.iter().enumerate() {
            let uv = mesh.corner_uv(face, corner);
            out.insert((outline.clone(), key(mesh.positions[*vertex as usize] + offset)), [uv.x, uv.y].map(|value| (value * 1000.0).round() as i64));
        }
    }
    out
}

fn first_mesh(scene: &Scene) -> &Mesh {
    scene.objects.iter().find_map(|object| object.mesh()).unwrap()
}

/// Which coordinates meet at each position (glTF holds triangles, which each side reads back as
/// it likes, polygons or triangles).
type ByPosition = BTreeMap<[i64; 3], std::collections::BTreeSet<[i64; 2]>>;

fn by_position(map: Corners) -> ByPosition {
    let mut out = ByPosition::new();
    for ((_, position), uv) in map {
        out.entry(position).or_default().insert(uv);
    }
    out
}

#[test]
fn coordinates_and_images_round_trip() {
    let scene = textured_cube();
    let original = corners(first_mesh(&scene), Vec3::ZERO);
    for format in [Format::Glb, Format::Gltf, Format::Obj] {
        let bytes = export(&scene, format, None).unwrap();
        let mut back = Scene::empty();
        place(&mut back, import(&bytes, format).unwrap()).unwrap();
        let mesh = first_mesh(&back);
        mesh.validate().unwrap();
        assert_eq!(corners(mesh, Vec3::ZERO), original, "{format:?}: coordinates");
        if format != Format::Obj {
            let material = back.materials.first().unwrap();
            let image = back.image(material.base_color_texture.as_deref().unwrap()).unwrap();
            assert_eq!(*image.bytes, png_bytes(), "{format:?}: the image");
        }
    }
}

#[test]
fn hostile_textures_are_refused_or_skipped() {
    let scene = textured_cube();
    let gltf = String::from_utf8(export(&scene, Format::Gltf, None).unwrap()).unwrap();
    let mut json: Value = serde_json::from_str(&gltf).unwrap();
    // An image in a separate file is refused, naming it.
    json["images"][0] = serde_json::json!({ "uri": "paint.png" });
    let error = import(json.to_string().as_bytes(), Format::Gltf).unwrap_err();
    assert!(error.to_string().contains("paint.png"), "{error}");
    // An image that is not PNG or JPEG is left out.
    json["images"][0] = serde_json::json!({ "uri": "data:image/gif;base64,R0lGODlhAQABAAAAACw=" });
    let imported = import(json.to_string().as_bytes(), Format::Gltf).unwrap();
    assert!(imported.images.is_empty() && imported.materials[0].base_color_texture.is_none());
    // A texture naming an image that is not there is refused.
    json["textures"][0]["source"] = serde_json::json!(7);
    assert!(import(json.to_string().as_bytes(), Format::Gltf).is_err());
    // Coordinates that cannot be read are refused.
    let mut broken: Value = serde_json::from_str(&gltf).unwrap();
    let texcoord = broken["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"].as_u64().unwrap() as usize;
    broken["accessors"][texcoord]["count"] = serde_json::json!(100_000);
    assert!(import(broken.to_string().as_bytes(), Format::Gltf).is_err());
}

fn blender() -> Option<PathBuf> {
    std::env::var_os("KITION_BLENDER").map(PathBuf::from)
}

/// Opens `path` (a .glb) and prints its first mesh's corners and material image, or (with
/// `make`) writes a textured cube to `path` first.
const SCRIPT: &str = r#"
import bpy, json, sys
args = sys.argv[sys.argv.index("--") + 1:]
mode, path, out = args
bpy.ops.object.select_all(action="SELECT"); bpy.ops.object.delete()
if mode == "make":
    bpy.ops.mesh.primitive_cube_add()
    image = bpy.data.images.new("checker", 4, 2)
    image.pixels = [1,0,0,1, 0,1,0,1, 0,0,1,1, 1,1,1,1, 0,0,0,1, .5,.5,.5,1, 1,1,0,1, 0,1,1,1]
    image.filepath_raw = path + ".png"; image.file_format = "PNG"; image.save(); image.pack()
    material = bpy.data.materials.new("Checker"); material.use_nodes = True
    texture = material.node_tree.nodes.new("ShaderNodeTexImage"); texture.image = image
    material.node_tree.links.new(texture.outputs["Color"], material.node_tree.nodes["Principled BSDF"].inputs["Base Color"])
    bpy.context.active_object.data.materials.append(material)
    bpy.ops.export_scene.gltf(filepath=path)
    bpy.ops.object.select_all(action="SELECT"); bpy.ops.object.delete()
bpy.ops.import_scene.gltf(filepath=path)
o = [o for o in bpy.context.scene.objects if o.type == "MESH"][0]
m = o.data
uv = m.uv_layers.active.data
faces = [[[list(o.matrix_world @ m.vertices[m.loops[l].vertex_index].co), list(uv[l].uv)] for l in p.loop_indices] for p in m.polygons]
images = []
for material in m.materials:
    for node in material.node_tree.nodes:
        if node.type == "TEX_IMAGE" and node.image:
            # Through any nodes in between (a factor other than 1 adds a multiply).
            frontier, linked = [node], False
            for _ in range(4):
                sockets = [link.to_socket for n in frontier for output in n.outputs for link in output.links]
                linked = linked or any(socket.name == "Base Color" and socket.node.type == "BSDF_PRINCIPLED" for socket in sockets)
                frontier = [socket.node for socket in sockets]
            images.append({"size": list(node.image.size), "baseColor": linked})
json.dump({"faces": faces, "images": images}, open(out, "w"))
"#;

fn run(blender: &Path, mode: &str, path: &Path) -> Value {
    let dir = path.parent().unwrap();
    let script = dir.join("textures.py");
    std::fs::write(&script, SCRIPT).unwrap();
    let out = dir.join(format!("{mode}.json"));
    let status = Command::new(blender)
        .args(["-b", "--factory-startup", "--python"])
        .arg(&script)
        .arg("--")
        .args([PathBuf::from(mode), path.to_path_buf(), out.clone()])
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "Blender failed");
    serde_json::from_str(&std::fs::read_to_string(out).unwrap()).unwrap()
}

/// Corners from the modeller's report, keyed like [`corners`].
fn reported(faces: &Value) -> Corners {
    let key = |value: &Value| -> [i64; 3] { [0, 1, 2].map(|axis| (value[axis].as_f64().unwrap() * 1000.0).round() as i64) };
    let mut out = BTreeMap::new();
    for face in faces.as_array().unwrap() {
        let mut outline: Vec<[i64; 3]> = face.as_array().unwrap().iter().map(|corner| key(&corner[0])).collect();
        outline.sort_unstable();
        for corner in face.as_array().unwrap() {
            let uv = [corner[1][0].as_f64().unwrap(), corner[1][1].as_f64().unwrap()].map(|value| (value * 1000.0).round() as i64);
            out.insert((outline.clone(), key(&corner[0])), uv);
        }
    }
    out
}

#[test]
fn the_reference_reads_our_textures_and_we_read_its() {
    let Some(blender) = blender() else {
        eprintln!("skipped: set KITION_BLENDER to the reference modeller's executable");
        return;
    };
    let dir = std::env::temp_dir().join(format!("kition-scene-textures-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // Ours, opened there: the same coordinates at each corner, and the image wired to Base Color.
    let scene = textured_cube();
    let ours = dir.join("ours.glb");
    std::fs::write(&ours, export(&scene, Format::Glb, None).unwrap()).unwrap();
    let report = run(&blender, "read", &ours);
    assert_eq!(by_position(reported(&report["faces"])), by_position(corners(first_mesh(&scene), Vec3::ZERO)));
    assert_eq!(report["images"], serde_json::json!([{ "size": [4, 2], "baseColor": true }]));

    // Theirs, opened here: its cube's coordinates and its image.
    let theirs = dir.join("theirs.glb");
    let report = run(&blender, "make", &theirs);
    let mut back = Scene::empty();
    place(&mut back, import(&std::fs::read(&theirs).unwrap(), Format::Glb).unwrap()).unwrap();
    let mesh = first_mesh(&back);
    assert_eq!(by_position(corners(mesh, Vec3::ZERO)), by_position(reported(&report["faces"])));
    let hash = back.materials[0].base_color_texture.clone().expect("the image comes with the material");
    let image = back.image(&hash).unwrap();
    let decoder = png::Decoder::new(std::io::Cursor::new(image.bytes.as_slice()));
    let info = decoder.read_info().unwrap();
    assert_eq!((info.info().width, info.info().height), (4, 2));
    let _ = std::fs::remove_dir_all(&dir);
}
