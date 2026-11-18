//! FBX against the reference application, both ways: it writes its scenes (the default scene, a
//! skinned and animated column, a textured cube, shape keys and lights) and we read them; we write
//! what we read and it reads ours. Each side reports world matrices at frames 1, 11, and 21,
//! counts, deformed vertex positions, weights, and shape keys, which must agree. Runs when
//! `KITION_BLENDER` names a Blender executable; otherwise it is skipped.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::collections::BTreeMap;
use std::path::Path;

use kition_scene_model::glam::{Mat4, Vec3};
use kition_scene_model::{ObjectData, Scene};
use serde_json::{Value, json};

const FRAMES: [f32; 3] = [1.0, 11.0, 21.0];

/// Writes the reference scenes as FBX, and reports a scene (after `--report`) as JSON.
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
                entry['groups'] = {g.name: [round(g.weight(v.index), 5) if any(x.group == g.index for x in v.groups) else 0.0 for v in obj.data.vertices] for g in obj.vertex_groups}
                keys = obj.data.shape_keys
                entry['shapeKeys'] = {k.name: k.value for k in keys.key_blocks[1:]} if keys else {}
                entry['material'] = obj.active_material.name if obj.active_material else None
                images = []
                if obj.active_material and obj.active_material.node_tree:
                    images = [n.image.size[:] for n in obj.active_material.node_tree.nodes if n.type == 'TEX_IMAGE' and n.image]
                entry['images'] = [list(size) for size in images]
                obj.evaluated_get(depsgraph).to_mesh_clear()
            if obj.type == 'ARMATURE':
                entry['bones'] = sorted(b.name for b in obj.data.bones)
            if obj.type == 'LIGHT':
                entry['light'] = [obj.data.type, obj.data.energy]
            if obj.type == 'CAMERA':
                entry['camera'] = [obj.data.type, obj.data.lens, obj.data.sensor_width, obj.data.ortho_scale]
    json.dump(out, open(path, 'w'))

if args[0] == '--report':
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.fbx(filepath=args[1])
    report(args[2])
    sys.exit(0)

out = args[0]
def save(name):
    bpy.ops.export_scene.fbx(filepath=os.path.join(out, name + '.fbx'), path_mode='COPY', embed_textures=True)
    report(os.path.join(out, name + '.json'))

save('default')

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.mesh.primitive_cylinder_add(vertices=8, radius=0.3, depth=2, location=(0, 0, 1))
column = bpy.context.active_object
bpy.ops.object.mode_set(mode='EDIT')
bpy.ops.mesh.loopcut_slide(MESH_OT_loopcut={'number_cuts': 3, 'edge_index': 2})
bpy.ops.object.mode_set(mode='OBJECT')
bpy.ops.object.armature_add(location=(0, 0, 0))
rig = bpy.context.active_object
bpy.ops.object.mode_set(mode='EDIT')
root = rig.data.edit_bones[0]; root.head = (0, 0, 0); root.tail = (0, 0, 1)
top = rig.data.edit_bones.new('Bone.001'); top.head = (0, 0, 1); top.tail = (0, 0, 2); top.parent = root; top.use_connect = True
bpy.ops.object.mode_set(mode='OBJECT')
column.select_set(True); rig.select_set(True); bpy.context.view_layer.objects.active = rig
bpy.ops.object.parent_set(type='ARMATURE_AUTO')
bpy.ops.object.mode_set(mode='POSE')
bone = rig.pose.bones['Bone.001']; bone.rotation_mode = 'XYZ'
bone.rotation_euler = (0, 0, 0); bone.keyframe_insert('rotation_euler', frame=1)
bone.rotation_euler = (0.8, 0, 0.3); bone.keyframe_insert('rotation_euler', frame=21)
bpy.ops.object.mode_set(mode='OBJECT')
rig.location = (0, 0, 0); rig.keyframe_insert('location', frame=1)
rig.location = (1, 0.5, 0); rig.keyframe_insert('location', frame=21)
bpy.context.scene.frame_start = 1; bpy.context.scene.frame_end = 21
save('skinned')

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.mesh.primitive_cube_add(location=(1, 2, 3), rotation=(0.3, 0.2, 0.1))
cube = bpy.context.active_object
image = bpy.data.images.new('Tex', 4, 2); image.pixels = [1, 0, 0, 1] * 8
image.filepath_raw = os.path.join(out, 'tex.png'); image.file_format = 'PNG'; image.save()
material = bpy.data.materials.new('Painted'); material.use_nodes = True
node = material.node_tree.nodes.new('ShaderNodeTexImage'); node.image = image
material.node_tree.links.new(node.outputs['Color'], material.node_tree.nodes['Principled BSDF'].inputs['Base Color'])
cube.data.materials.append(material)
bpy.ops.mesh.primitive_plane_add(location=(-2, 0, 0))
plane = bpy.context.active_object
plane.shape_key_add(name='Basis'); key = plane.shape_key_add(name='Up'); key.data[0].co.z = 1.0; key.value = 0.25
bpy.ops.object.empty_add(location=(0, 0, 2)); parent = bpy.context.active_object
plane.parent = parent
for kind, location in [('SUN', (0, 0, 5)), ('SPOT', (3, 0, 3)), ('AREA', (-3, 0, 3))]:
    bpy.ops.object.light_add(type=kind, location=location)
    light = bpy.context.active_object; light.data.energy = {'SUN': 3.0, 'SPOT': 500.0, 'AREA': 200.0}[kind]
    if kind == 'SPOT':
        light.data.spot_size = math.radians(60)
bpy.ops.object.camera_add(location=(0, -5, 2), rotation=(1.2, 0, 0.1))
camera = bpy.context.active_object; camera.data.type = 'ORTHO'; camera.data.ortho_scale = 4.0
save('things')
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
                    entry["vertices"] = json!(mesh.positions.len());
                    entry["faces"] = json!(mesh.faces.len());
                    entry["groups"] = json!(mesh.groups.iter().map(|group| (group.name.clone(), group.weights.clone())).collect::<BTreeMap<_, _>>());
                    entry["shapeKeys"] = json!(mesh.shape_keys.iter().map(|key| (key.name.clone(), key.value)).collect::<BTreeMap<_, _>>());
                    let material = object.material.and_then(|id| at.material(id).ok());
                    entry["material"] = json!(material.map(|material| material.name.clone()));
                    let image = material.and_then(|material| material.base_color_texture.as_ref()).and_then(|hash| at.image(hash));
                    entry["hasImage"] = json!(image.is_some());
                }
                ObjectData::Armature(armature) => {
                    entry["type"] = json!("ARMATURE");
                    let mut bones: Vec<String> = armature.bones.iter().map(|bone| bone.name.clone()).collect();
                    bones.sort();
                    entry["bones"] = json!(bones);
                }
                ObjectData::Light(light) => {
                    entry["type"] = json!("LIGHT");
                    entry["light"] = json!([format!("{:?}", light.kind).to_uppercase(), light.power]);
                }
                ObjectData::Camera(camera) => {
                    entry["type"] = json!("CAMERA");
                    entry["camera"] = json!([if camera.orthographic { "ORTHO" } else { "PERSP" }, camera.lens, camera.sensor, camera.ortho_scale]);
                }
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

/// Every object of `reference` (except end bones' and the reference's own extras) agrees with `ours`.
fn compare(label: &str, reference: &Value, ours: &Value) {
    for (name, theirs) in reference.as_object().unwrap() {
        let Some(mine) = ours.get(name) else {
            panic!("{label}: {name} is missing; ours has {:?}", ours.as_object().unwrap().keys().collect::<Vec<_>>())
        };
        let kind = theirs["type"].as_str().unwrap();
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
                    assert_eq!(a.len(), b.len(), "{label}: {name}'s deformed vertex count");
                    let worst = a.iter().zip(&b).map(|(a, b)| a.distance(*b)).fold(0.0f32, f32::max);
                    assert!(worst < 2e-3, "{label}: {name}'s vertices at frame {} are off by {worst}", FRAMES[frame]);
                }
                for (group, weights) in theirs["groups"].as_object().unwrap() {
                    let ours = mine["groups"].get(group).unwrap_or_else(|| panic!("{label}: {name} has no group {group}"));
                    for (a, b) in weights.as_array().unwrap().iter().zip(ours.as_array().unwrap()) {
                        assert!((a.as_f64().unwrap() - b.as_f64().unwrap()).abs() < 1e-3, "{label}: {name}'s {group} weights");
                    }
                }
                for (key, value) in theirs["shapeKeys"].as_object().unwrap() {
                    let ours = mine["shapeKeys"].get(key).unwrap_or_else(|| panic!("{label}: {name} has no shape key {key}"));
                    assert!((value.as_f64().unwrap() - ours.as_f64().unwrap()).abs() < 1e-4, "{label}: {name}'s {key} value");
                }
                if let Some(images) = theirs.get("images") {
                    assert_eq!(!images.as_array().unwrap().is_empty(), mine["hasImage"].as_bool().unwrap_or(false), "{label}: {name}'s image");
                }
            }
            "ARMATURE" => {
                let ours: Vec<String> = mine["bones"].as_array().unwrap().iter().map(|bone| bone.as_str().unwrap().to_string()).collect();
                for bone in theirs["bones"].as_array().unwrap() {
                    let bone = bone.as_str().unwrap();
                    if !bone.ends_with("_end") {
                        assert!(ours.iter().any(|name| name == bone), "{label}: {name} lacks bone {bone}");
                    }
                }
            }
            "LIGHT" => {
                // The reference's reader takes area lights as point lights (its own included).
                let kinds = (theirs["light"][0].as_str().unwrap(), mine["light"][0].as_str().unwrap());
                assert!(kinds.0 == kinds.1 || kinds == ("AREA", "POINT"), "{label}: {name}'s kind: {kinds:?}");
                assert!((theirs["light"][1].as_f64().unwrap() - mine["light"][1].as_f64().unwrap()).abs() < 1e-3, "{label}: {name}'s power");
            }
            "CAMERA" => {
                assert_eq!(theirs["camera"][0], mine["camera"][0], "{label}: {name}'s projection");
                for slot in 1..4 {
                    // An orthographic camera's lens, or a perspective one's ortho scale, does not travel.
                    let relevant = (slot == 3) == (theirs["camera"][0] == "ORTHO");
                    if relevant || slot == 2 {
                        assert!(
                            (theirs["camera"][slot].as_f64().unwrap() - mine["camera"][slot].as_f64().unwrap()).abs() < 1e-3,
                            "{label}: {name}'s camera {slot}"
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

#[test]
fn fbx_agrees_with_the_reference_application_both_ways() {
    let Some(blender) = blender() else {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    };
    let dir = std::env::temp_dir().join(format!("kition-scene-fbx-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("fbx.py");
    std::fs::write(&script, SCRIPT).unwrap();
    run(&blender, &script, &[dir.to_str().unwrap()]);
    for name in ["default", "skinned", "things"] {
        // Theirs, read by us.
        let reference: Value = serde_json::from_slice(&std::fs::read(dir.join(format!("{name}.json"))).unwrap()).unwrap();
        let mut scene = Scene::empty();
        kition_scene_exchange::place(&mut scene, kition_scene_fbx::read(&std::fs::read(dir.join(format!("{name}.fbx"))).unwrap()).unwrap()).unwrap();
        let ours = report(&scene);
        compare(&format!("{name}: theirs read by us"), &reference, &ours);
        // Ours, read by them.
        let written = dir.join(format!("{name}-ours.fbx"));
        std::fs::write(&written, kition_scene_fbx::write(&scene, None).unwrap()).unwrap();
        let back = dir.join(format!("{name}-ours.json"));
        run(&blender, &script, &["--report", written.to_str().unwrap(), back.to_str().unwrap()]);
        let theirs: Value = serde_json::from_slice(&std::fs::read(&back).unwrap()).unwrap();
        compare(&format!("{name}: ours read by them"), &ours_without_ends(&ours), &theirs);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Our report with what the reference does not keep (images it finds by path) left out.
fn ours_without_ends(ours: &Value) -> Value {
    let mut out = ours.clone();
    for (_, entry) in out.as_object_mut().unwrap() {
        if let Some(object) = entry.as_object_mut() {
            object.remove("hasImage");
        }
    }
    out
}
