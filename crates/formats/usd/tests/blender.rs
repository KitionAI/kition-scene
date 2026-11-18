//! USD against the reference application, both ways (USDA, and its USDC and USDZ read too): it writes its scenes (the default scene,
//! a scene of parented, animated, and material-bound meshes with every light kind, and a column
//! skinned to a keyed two-bone rig with a keyed shape key beside a shape-keyed grid) and we
//! read them; we write what we read and it reads ours. Each side reports world matrices at frames
//! 1, 11, and 21, counts, vertex positions, vertex group weights, materials, lights, and cameras, which must agree.
//! Runs when `KITION_BLENDER` names a Blender executable; otherwise it is skipped.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::collections::BTreeMap;
use std::path::Path;

use kition_scene_model::glam::{Mat4, Vec3};
use kition_scene_model::{ObjectData, Scene};
use serde_json::{Value, json};

const FRAMES: [f32; 3] = [1.0, 11.0, 21.0];

/// Writes the reference scenes as USDA, and reports a scene (after `--report`) as JSON.
const SCRIPT: &str = r#"
import bpy, json, math, os, sys
args = sys.argv[sys.argv.index('--') + 1:]
FRAMES = [1, 11, 21]

def report(path):
    scene = bpy.context.scene
    out = {}
    for frame in FRAMES:
        scene.frame_set(frame)
        depsgraph = bpy.context.evaluated_depsgraph_get()
        for obj in scene.objects:
            entry = out.setdefault(obj.name, {'type': obj.type, 'worlds': [], 'deformed': []})
            entry['worlds'].append([v for col in zip(*obj.matrix_world) for v in col])
            if obj.type == 'MESH':
                mesh = obj.evaluated_get(depsgraph).to_mesh()
                entry['deformed'].append([list(obj.matrix_world @ v.co) for v in mesh.vertices])
                entry['vertices'] = len(obj.data.vertices)
                entry['faces'] = len(obj.data.polygons)
                # Vertex group weights, normalised per vertex.
                names = {group.index: group.name for group in obj.vertex_groups}
                groups = {name: [0.0] * len(obj.data.vertices) for name in names.values()}
                for vertex in obj.data.vertices:
                    total = sum(item.weight for item in vertex.groups if item.group in names)
                    for item in vertex.groups:
                        if item.group in names and total > 0:
                            groups[names[item.group]][vertex.index] = item.weight / total
                entry['groups'] = groups
                material = obj.active_material
                entry['material'] = None
                if material and material.node_tree:
                    surface = next((n for n in material.node_tree.nodes if n.type == 'BSDF_PRINCIPLED'), None)
                    links = surface.inputs['Base Color'].links if surface else []
                    if links and links[0].from_node.type == 'TEX_IMAGE' and links[0].from_node.image:
                        # A base colour image: its size (the factor stays on the shader).
                        entry['texture'] = list(links[0].from_node.image.size)
                    elif surface:
                        entry['material'] = list(surface.inputs['Base Color'].default_value)[:3] + [surface.inputs['Metallic'].default_value, surface.inputs['Roughness'].default_value]
                obj.evaluated_get(depsgraph).to_mesh_clear()
            if obj.type == 'LIGHT':
                light = obj.data
                size = light.size if light.type == 'AREA' else getattr(light, 'shadow_soft_size', 0.0)
                entry['light'] = [light.type, light.energy, size, getattr(light, 'spot_size', 0.0), list(light.color)]
            if obj.type == 'CAMERA':
                entry['camera'] = [obj.data.type, obj.data.lens, obj.data.sensor_width, obj.data.ortho_scale]
    json.dump(out, open(path, 'w'))

if args[0] == '--report':
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.wm.usd_import(filepath=args[1])
    report(args[2])
    sys.exit(0)

out = args[0]
def save(name):
    for extension in ('usda', 'usdc', 'usdz'):
        bpy.ops.wm.usd_export(filepath=os.path.join(out, name + '.' + extension), export_animation=True)
    report(os.path.join(out, name + '.json'))

save('default')

bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
scene.frame_start = 1; scene.frame_end = 21
bpy.ops.mesh.primitive_cube_add(location=(1, 2, 3), rotation=(0.3, 0.2, 0.1))
cube = bpy.context.active_object
material = bpy.data.materials.new('Painted'); material.use_nodes = True
surface = material.node_tree.nodes['Principled BSDF']
surface.inputs['Base Color'].default_value = (0.2, 0.4, 0.6, 1); surface.inputs['Metallic'].default_value = 0.3; surface.inputs['Roughness'].default_value = 0.7
cube.data.materials.append(material)
cube.keyframe_insert('location', frame=1); cube.keyframe_insert('rotation_euler', frame=1)
cube.location = (2, 2, 4); cube.rotation_euler = (0.3, 1.2, 0.1)
cube.keyframe_insert('location', frame=21); cube.keyframe_insert('rotation_euler', frame=21)
bpy.ops.mesh.primitive_uv_sphere_add(segments=12, ring_count=6, radius=0.5, location=(1, 2, 5))
sphere = bpy.context.active_object
sphere.parent = cube; sphere.matrix_parent_inverse = cube.matrix_world.inverted()
bpy.ops.object.shade_smooth()
bpy.ops.mesh.primitive_plane_add(location=(-2, 0, 0))
plane = bpy.context.active_object
bpy.ops.object.empty_add(location=(0, 0, 2), rotation=(0, 0, 0.5)); parent = bpy.context.active_object
parent.scale = (2, 2, 2)
plane.parent = parent
for kind, location in [('POINT', (0, 3, 3)), ('SUN', (0, 0, 5)), ('SPOT', (3, 0, 3)), ('AREA', (-3, 0, 3))]:
    bpy.ops.object.light_add(type=kind, location=location, rotation=(0.4, 0.1, 0))
    light = bpy.context.active_object; light.data.energy = {'POINT': 800.0, 'SUN': 3.0, 'SPOT': 500.0, 'AREA': 200.0}[kind]
    light.data.color = (1, 0.8, 0.6)
    if kind == 'SPOT':
        light.data.spot_size = math.radians(60); light.data.shadow_soft_size = 0.2
    if kind == 'AREA':
        light.data.size = 1.5
bpy.ops.object.camera_add(location=(0, -5, 2), rotation=(1.2, 0, 0.1))
bpy.context.active_object.data.lens = 35
# A plane with an image on its base colour.
bpy.ops.mesh.primitive_plane_add(location=(0, -2, 0))
tile = bpy.context.active_object; tile.name = 'Tile'
image = bpy.data.images.new('Checker', 8, 4)
image.pixels = [value for index in range(32) for value in ((1, 1, 1, 1) if (index + index // 8) % 2 else (0.1, 0.2, 0.8, 1))]
image.filepath_raw = os.path.join(out, 'checker.png'); image.file_format = 'PNG'; image.save()
tiled = bpy.data.materials.new('Tiled'); tiled.use_nodes = True
node = tiled.node_tree.nodes.new('ShaderNodeTexImage'); node.image = image
tiled.node_tree.links.new(node.outputs['Color'], tiled.node_tree.nodes['Principled BSDF'].inputs['Base Color'])
tile.data.materials.append(tiled)
save('things')

# A column skinned to a two-bone rig with automatic weights, a shape key, and keys on both.
import bmesh
bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
scene.frame_start = 1; scene.frame_end = 21
mesh = bpy.data.meshes.new('Column')
column = bpy.data.objects.new('Column', mesh)
scene.collection.objects.link(column)
built = bmesh.new()
bmesh.ops.create_cone(built, cap_ends=True, segments=8, radius1=0.3, radius2=0.3, depth=2)
bmesh.ops.subdivide_edges(built, edges=[e for e in built.edges if abs(e.verts[0].co.z - e.verts[1].co.z) > 1.5], cuts=7)
built.to_mesh(mesh); built.free()
column.location = (0, 0, 1)
armature = bpy.data.armatures.new('Armature')
rig = bpy.data.objects.new('Rig', armature)
scene.collection.objects.link(rig)
bpy.context.view_layer.objects.active = rig
bpy.ops.object.mode_set(mode='EDIT')
lower = armature.edit_bones.new('Lower'); lower.head = (0, 0, 0); lower.tail = (0, 0, 1)
upper = armature.edit_bones.new('Upper'); upper.head = (0, 0, 1); upper.tail = (0, 0, 2); upper.parent = lower; upper.use_connect = True
bpy.ops.object.mode_set(mode='OBJECT')
bpy.ops.object.select_all(action='DESELECT')
column.select_set(True); rig.select_set(True); bpy.context.view_layer.objects.active = rig
bpy.ops.object.parent_set(type='ARMATURE_AUTO')
column.shape_key_add(name='Basis')
wide = column.shape_key_add(name='Wide')
for point in wide.data:
    point.co.x *= 1.5; point.co.y *= 1.5
wide.value = 0.0; wide.keyframe_insert('value', frame=1)
wide.value = 1.0; wide.keyframe_insert('value', frame=21)
bpy.context.view_layer.objects.active = rig
bpy.ops.object.mode_set(mode='POSE')
pose = rig.pose.bones['Upper']
pose.rotation_mode = 'XYZ'
pose.rotation_euler = (0, 0, 0); pose.keyframe_insert('rotation_euler', frame=1)
pose.rotation_euler = (0.8, 0.2, 0); pose.keyframe_insert('rotation_euler', frame=21)
bpy.ops.object.mode_set(mode='OBJECT')
# A shape-keyed grid no armature deforms.
bpy.ops.mesh.primitive_grid_add(x_subdivisions=3, y_subdivisions=3, size=1, location=(2, 0, 0))
flag = bpy.context.active_object; flag.name = 'Flag'
flag.shape_key_add(name='Basis')
lift = flag.shape_key_add(name='Lift')
for point in lift.data:
    point.co.z += point.co.x
lift.value = 0.25; lift.keyframe_insert('value', frame=1)
lift.value = 0.75; lift.keyframe_insert('value', frame=21)
for extension in ('usda', 'usdc', 'usdz'):
    bpy.ops.wm.usd_export(filepath=os.path.join(out, 'skinned.' + extension), export_animation=True, export_armatures=True, export_shapekeys=True)
report(os.path.join(out, 'skinned.json'))
"#;

fn blender() -> Option<String> {
    std::env::var("KITION_BLENDER").ok().filter(|path| !path.is_empty())
}

fn run(blender: &str, script: &Path, args: &[&str]) {
    let status = std::process::Command::new(blender)
        .args(["-b", "--factory-startup", "--python"])
        .arg(script)
        .arg("--")
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "Blender failed on {args:?}");
}

/// Our report of a scene, in the same shape as the script's.
fn report(scene: &Scene) -> Value {
    let mut out: BTreeMap<String, Value> = BTreeMap::new();
    for frame in FRAMES {
        let at = scene.at_frame(frame);
        for object in &at.objects {
            let entry = out.entry(object.name.clone()).or_insert_with(|| json!({ "worlds": [], "deformed": [] }));
            let world = at.world_matrix(object.id).unwrap();
            entry["worlds"].as_array_mut().unwrap().push(json!(world.to_cols_array()));
            match &object.data {
                // Imports never make curves, texts or surfaces (they come in as meshes).
                ObjectData::Curve(_) | ObjectData::Text(_) => entry["type"] = json!("CURVE"),
                ObjectData::Surface(_) => entry["type"] = json!("SURFACE"),
                ObjectData::Mesh(mesh) => {
                    let evaluated = object.evaluated_mesh(&at).unwrap();
                    entry["deformed"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!(evaluated.positions.iter().map(|point| world.transform_point3(*point).to_array()).collect::<Vec<_>>()));
                    entry["type"] = json!("MESH");
                    // A base colour image: a PNG's size from its header.
                    if let Some(image) = object
                        .material
                        .and_then(|id| at.material(id).ok())
                        .and_then(|material| material.base_color_texture.as_ref())
                        .and_then(|hash| at.image(hash))
                    {
                        let field = |at: usize| u32::from_be_bytes(image.bytes[at..at + 4].try_into().unwrap());
                        entry["texture"] = json!([field(16), field(20)]);
                    }
                    entry["vertices"] = json!(mesh.positions.len());
                    entry["faces"] = json!(mesh.faces.len());
                    let mut groups: BTreeMap<String, Vec<f32>> =
                        mesh.groups.iter().map(|group| (group.name.clone(), vec![0.0; mesh.positions.len()])).collect();
                    for vertex in 0..mesh.positions.len() {
                        let total: f32 = mesh.groups.iter().filter_map(|group| group.weights.get(vertex)).sum();
                        for group in &mesh.groups {
                            if let (Some(weight), Some(list)) = (group.weights.get(vertex), groups.get_mut(&group.name))
                                && total > 0.0
                            {
                                list[vertex] = weight / total;
                            }
                        }
                    }
                    entry["groups"] = json!(groups);
                    let material = object.material.and_then(|id| at.material(id).ok()).filter(|material| material.base_color_texture.is_none());
                    entry["material"] = json!(material.map(|material| [
                        material.base_color[0],
                        material.base_color[1],
                        material.base_color[2],
                        material.metallic,
                        material.roughness
                    ]));
                }
                ObjectData::Light(light) => {
                    entry["type"] = json!("LIGHT");
                    entry["light"] =
                        json!([format!("{:?}", light.kind).to_uppercase(), light.power, light.radius, light.spot_size, light.color.to_array()]);
                }
                ObjectData::Camera(camera) => {
                    entry["type"] = json!("CAMERA");
                    entry["camera"] = json!([if camera.orthographic { "ORTHO" } else { "PERSP" }, camera.lens, camera.sensor, camera.ortho_scale]);
                }
                ObjectData::Armature(_) => entry["type"] = json!("ARMATURE"),
                ObjectData::Empty { .. } | ObjectData::Lattice(_) => entry["type"] = json!("EMPTY"),
            }
        }
    }
    json!(out)
}

fn matrix(value: &Value) -> Mat4 {
    let values: Vec<f32> = value.as_array().unwrap().iter().map(|value| value.as_f64().unwrap() as f32).collect();
    Mat4::from_cols_slice(&values)
}

fn points(value: &Value) -> Vec<Vec3> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|point| Vec3::new(point[0].as_f64().unwrap() as f32, point[1].as_f64().unwrap() as f32, point[2].as_f64().unwrap() as f32))
        .collect()
}

fn close(label: &str, a: &Value, b: &Value) {
    let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
    assert!((a - b).abs() < 1e-3 * a.abs().max(1.0), "{label}: {a} and {b}");
}

/// Every object of `reference` agrees with `ours`.
fn compare(label: &str, reference: &Value, ours: &Value) {
    for (name, theirs) in reference.as_object().unwrap() {
        let Some(mut mine) = ours.get(name) else {
            panic!("{label}: {name} is missing; ours has {:?}", ours.as_object().unwrap().keys().collect::<Vec<_>>())
        };
        let kind = theirs["type"].as_str().unwrap();
        // The reference's reader splits an object with both data and children into an empty and
        // a ".001" object holding the data (its own files included).
        if mine["type"] == "EMPTY" && kind != "EMPTY" {
            mine = ours.get(format!("{name}.001")).unwrap_or_else(|| panic!("{label}: {name} is an empty with no data object"));
        }
        assert_eq!(mine["type"].as_str(), Some(kind), "{label}: {name}'s type");
        for (frame, (a, b)) in theirs["worlds"].as_array().unwrap().iter().zip(mine["worlds"].as_array().unwrap()).enumerate() {
            let (a, b) = (matrix(a), matrix(b));
            assert!(a.abs_diff_eq(b, 2e-3), "{label}: {name}'s world at frame {} differs:\n{a}\n{b}", FRAMES[frame]);
        }
        match kind {
            "MESH" => {
                assert_eq!(theirs["vertices"], mine["vertices"], "{label}: {name}'s vertex count");
                assert_eq!(theirs["faces"], mine["faces"], "{label}: {name}'s face count");
                for (frame, (a, b)) in theirs["deformed"].as_array().unwrap().iter().zip(mine["deformed"].as_array().unwrap()).enumerate() {
                    let (a, b) = (points(a), points(b));
                    assert_eq!(a.len(), b.len(), "{label}: {name}'s vertex count");
                    let worst = a.iter().zip(&b).map(|(a, b)| a.distance(*b)).fold(0.0f32, f32::max);
                    assert!(worst < 2e-3, "{label}: {name}'s vertices at frame {} are off by {worst}", FRAMES[frame]);
                }
                // Every group with weight in theirs has the same normalised weights in ours.
                for (group, weights) in theirs["groups"].as_object().into_iter().flatten() {
                    let weights = weights.as_array().unwrap();
                    if weights.iter().all(|weight| weight.as_f64() == Some(0.0)) {
                        continue;
                    }
                    let ours = mine["groups"][group].as_array().unwrap_or_else(|| panic!("{label}: {name} has no group {group}"));
                    for (a, b) in weights.iter().zip(ours) {
                        assert!((a.as_f64().unwrap() - b.as_f64().unwrap()).abs() < 2e-3, "{label}: {name}'s weights in {group} differ: {a} and {b}");
                    }
                }
                if let Some(material) = theirs["material"].as_array() {
                    let ours = mine["material"].as_array().unwrap_or_else(|| panic!("{label}: {name} has no material"));
                    for (slot, (a, b)) in material.iter().zip(ours).enumerate() {
                        close(&format!("{label}: {name}'s material {slot}"), a, b);
                    }
                }
            }
            "LIGHT" => {
                assert_eq!(theirs["light"][0], mine["light"][0], "{label}: {name}'s kind");
                close(&format!("{label}: {name}'s power"), &theirs["light"][1], &mine["light"][1]);
                if theirs["light"][0] != "SUN" {
                    close(&format!("{label}: {name}'s size"), &theirs["light"][2], &mine["light"][2]);
                }
                if theirs["light"][0] == "SPOT" {
                    close(&format!("{label}: {name}'s spot size"), &theirs["light"][3], &mine["light"][3]);
                }
                for channel in 0..3 {
                    close(&format!("{label}: {name}'s colour"), &theirs["light"][4][channel], &mine["light"][4][channel]);
                }
            }
            "CAMERA" => {
                assert_eq!(theirs["camera"][0], mine["camera"][0], "{label}: {name}'s projection");
                close(&format!("{label}: {name}'s lens"), &theirs["camera"][1], &mine["camera"][1]);
                close(&format!("{label}: {name}'s sensor"), &theirs["camera"][2], &mine["camera"][2]);
            }
            _ => {}
        }
    }
}

#[test]
fn usda_agrees_with_the_reference_application_both_ways() {
    let Some(blender) = blender() else {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    };
    let dir = std::env::temp_dir().join(format!("kition-scene-usd-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("usd.py");
    std::fs::write(&script, SCRIPT).unwrap();
    run(&blender, &script, &[dir.to_str().unwrap()]);
    for name in ["default", "things", "skinned"] {
        // Theirs, read by us.
        let reference: Value = serde_json::from_slice(&std::fs::read(dir.join(format!("{name}.json"))).unwrap()).unwrap();
        // The binary layer and the package read the same as the text.
        for extension in ["usdc", "usdz"] {
            let mut scene = Scene::empty();
            kition_scene_exchange::place(
                &mut scene,
                kition_scene_usd::read(&std::fs::read(dir.join(format!("{name}.{extension}"))).unwrap()).unwrap(),
            )
            .unwrap();
            let ours = report(&scene);
            compare(&format!("{name}: their {extension} read by us"), &reference, &ours);
            // A package carries its images.
            if extension == "usdz" && name == "things" {
                assert_eq!(ours["Tile"]["texture"], json!([8, 4]), "their usdz's image");
                assert_eq!(reference["Tile"]["texture"], json!([8, 4]));
                // Ours, as a package, read by them.
                let written = dir.join("things-ours.usdz");
                std::fs::write(&written, kition_scene_usd::write_package(&scene, None).unwrap()).unwrap();
                let back = dir.join("things-ours-usdz.json");
                run(&blender, &script, &["--report", written.to_str().unwrap(), back.to_str().unwrap()]);
                let theirs: Value = serde_json::from_slice(&std::fs::read(&back).unwrap()).unwrap();
                compare(&format!("{name}: our usdz read by them"), &ours, &theirs);
                assert_eq!(theirs["Tile"]["texture"], json!([8, 4]), "our usdz's image read by them");
            }
        }
        let mut scene = Scene::empty();
        kition_scene_exchange::place(&mut scene, kition_scene_usd::read(&std::fs::read(dir.join(format!("{name}.usda"))).unwrap()).unwrap()).unwrap();
        let ours = report(&scene);
        compare(&format!("{name}: theirs read by us"), &reference, &ours);
        // Ours, read by them.
        let written = dir.join(format!("{name}-ours.usda"));
        std::fs::write(&written, kition_scene_usd::write(&scene, None).unwrap()).unwrap();
        let back = dir.join(format!("{name}-ours.json"));
        run(&blender, &script, &["--report", written.to_str().unwrap(), back.to_str().unwrap()]);
        let theirs: Value = serde_json::from_slice(&std::fs::read(&back).unwrap()).unwrap();
        compare(&format!("{name}: ours read by them"), &ours, &theirs);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Binary layers the reference application writes, cut short at every length and with bytes
/// flipped at random, are refused or read; they never panic. Runs with `KITION_BLENDER`.
#[test]
fn damaged_binary_layers_never_panic() {
    let Some(blender) = blender() else {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    };
    let dir = std::env::temp_dir().join(format!("kition-scene-usdc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("usd.py");
    std::fs::write(&script, SCRIPT).unwrap();
    run(&blender, &script, &[dir.to_str().unwrap()]);
    for name in ["default.usdc", "things.usdc", "things.usdz", "skinned.usdc"] {
        let file = std::fs::read(dir.join(name)).unwrap();
        assert!(kition_scene_usd::read(&file).is_ok(), "{name} reads");
        for end in 0..file.len() {
            let _ = kition_scene_usd::read(&file[..end]);
        }
        let mut state = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..4000 {
            let mut damaged = file.clone();
            for _ in 0..1 + (state % 4) {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let at = (state % damaged.len() as u64) as usize;
                damaged[at] = (state >> 32) as u8;
            }
            let _ = kition_scene_usd::read(&damaged);
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
