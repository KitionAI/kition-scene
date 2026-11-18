//! Files the reference application writes, read back and compared with what it reports about
//! them: world matrices, mesh counts and sizes, materials, cameras, lights, modifiers, and
//! visibility, plain and zstd-compressed. Runs when `KITION_BLENDER` names a Blender executable.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_scene_model::modifiers::ModifierKind;
use kition_scene_model::{ObjectData, Scene};
use serde_json::Value;

const SCRIPT: &str = r#"
import bpy, sys, json, math
out = sys.argv[sys.argv.index('--') + 1]
cube = bpy.data.objects['Cube']
m = cube.modifiers.new('Mir', 'MIRROR'); m.use_axis = (True, False, True)
a = cube.modifiers.new('Arr', 'ARRAY'); a.count = 3; a.relative_offset_displace = (1.5, 0, 0)
s = cube.modifiers.new('Sub', 'SUBSURF'); s.levels = 2
bpy.ops.mesh.primitive_uv_sphere_add(location=(3, 0, 0)); sph = bpy.context.active_object; sph.name = 'Ball'
b = cube.modifiers.new('Boo', 'BOOLEAN'); b.object = sph; b.operation = 'UNION'
sd = sph.modifiers.new('Def', 'SIMPLE_DEFORM'); sd.deform_method = 'BEND'; sd.deform_axis = 'Z'
sph.parent = cube; sph.matrix_parent_inverse = cube.matrix_world.inverted()
mat = bpy.data.materials.new('Red'); mat.use_nodes = True
p = mat.node_tree.nodes['Principled BSDF']; p.inputs['Base Color'].default_value = (0.8, 0.1, 0.05, 1); p.inputs['Metallic'].default_value = 0.7; p.inputs['Roughness'].default_value = 0.2
sph.data.materials.append(mat)
cube.scale = (2, 1, 0.5); cube.rotation_euler = (0.3, 0.1, 0.7)
modes = ['XYZ', 'XZY', 'YXZ', 'YZX', 'ZXY', 'ZYX', 'QUATERNION', 'AXIS_ANGLE']
parent = None
for index, mode in enumerate(modes):
    bpy.ops.mesh.primitive_cone_add(vertices=5 + index, location=(index * 0.5, 1, 0.2 * index))
    o = bpy.context.active_object; o.name = 'R' + mode; o.rotation_mode = mode
    if mode == 'QUATERNION': o.rotation_quaternion = (0.8, 0.2, -0.4, 0.3)
    elif mode == 'AXIS_ANGLE': o.rotation_axis_angle = (0.7, 0.3, 0.5, 0.2)
    else: o.rotation_euler = (0.2 + index * 0.1, -0.3, 0.5)
    o.scale = (1, 1.5, 0.75)
    if parent: o.parent = parent
    parent = o
bpy.ops.mesh.primitive_circle_add(vertices=7, location=(0, -3, 0)); bpy.context.active_object.name = 'Ring'
bpy.ops.mesh.primitive_torus_add(location=(0, -6, 0)); t = bpy.context.active_object; t.name = 'Donut'; bpy.ops.object.shade_smooth(); t.hide_viewport = True
bpy.data.objects['Camera'].data.type = 'ORTHO'; bpy.data.objects['Camera'].data.ortho_scale = 9
bpy.data.lights['Light'].type = 'SPOT'; bpy.data.lights['Light'].energy = 750; bpy.data.lights['Light'].spot_size = math.radians(60)
bpy.ops.object.empty_add(location=(0, 0, 5)); bpy.context.active_object.name = 'Pivot'
bpy.ops.wm.save_as_mainfile(filepath=out + '/plain.blend', compress=False)
bpy.ops.wm.save_as_mainfile(filepath=out + '/packed.blend', compress=True)
report = {}
for o in bpy.data.objects:
    entry = {'type': o.type, 'matrix': [v for col in o.matrix_world.transposed() for v in col], 'hidden': o.hide_viewport,
             'modifiers': [m.type for m in o.modifiers]}
    if o.type == 'MESH':
        me = o.data; xs = [v.co for v in me.vertices]
        entry.update(vertices=len(me.vertices), faces=len(me.polygons), edges=len(me.edges),
                     min=[min(c[i] for c in xs) for i in range(3)], max=[max(c[i] for c in xs) for i in range(3)],
                     smooth=sum(p.use_smooth for p in me.polygons) * 2 > len(me.polygons) if me.polygons else False,
                     material=me.materials[0].name if me.materials else None)
    if o.type == 'CAMERA':
        c = o.data; entry.update(lens=c.lens, sensor=c.sensor_width, clip=[c.clip_start, c.clip_end], ortho=c.type == 'ORTHO', ortho_scale=c.ortho_scale)
    if o.type == 'LIGHT':
        l = o.data; entry.update(kind=l.type, energy=l.energy, color=list(l.color), spot=l.spot_size if l.type == 'SPOT' else None)
    report[o.name] = entry
print('REPORT' + json.dumps(report))
"#;

fn close(a: f32, b: f64, tolerance: f64) -> bool {
    (f64::from(a) - b).abs() <= tolerance
}

#[test]
fn files_the_reference_writes_read_back() {
    let Some(blender) = std::env::var_os("KITION_BLENDER") else {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    };
    let dir = std::env::temp_dir().join(format!("kition-scene-blend-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("make.py");
    std::fs::write(&script, SCRIPT).unwrap();
    let output = std::process::Command::new(blender).args(["-b", "--factory-startup", "--python"]).arg(&script).arg("--").arg(&dir).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let report: Value = serde_json::from_str(stdout.lines().find_map(|line| line.strip_prefix("REPORT")).expect("Blender's report")).unwrap();
    let report = report.as_object().unwrap();

    for file in ["plain.blend", "packed.blend"] {
        let bytes = std::fs::read(dir.join(file)).unwrap();
        let read = kition_scene_blend::read(&bytes).unwrap();
        let mut scene = Scene::empty();
        kition_scene_blend::place(&mut scene, read).unwrap();
        assert_eq!(scene.objects.len(), report.len(), "{file}: object count");
        for (name, expected) in report {
            let object = scene.object_named(name).unwrap_or_else(|| panic!("{file}: {name} is missing"));
            let world = scene.world_matrix(object.id).unwrap().to_cols_array();
            let matrix = expected["matrix"].as_array().unwrap();
            for (index, value) in matrix.iter().enumerate() {
                assert!(close(world[index], value.as_f64().unwrap(), 1e-4), "{file}: {name} world matrix {index}: {} vs {value}", world[index]);
            }
            assert_eq!(object.hidden, expected["hidden"].as_bool().unwrap(), "{file}: {name} hidden");
            match &object.data {
                ObjectData::Mesh(mesh) => {
                    assert_eq!(mesh.positions.len() as u64, expected["vertices"].as_u64().unwrap(), "{file}: {name} vertices");
                    assert_eq!(mesh.faces.len() as u64, expected["faces"].as_u64().unwrap(), "{file}: {name} faces");
                    assert_eq!(mesh.edges().len() as u64, expected["edges"].as_u64().unwrap(), "{file}: {name} edges");
                    assert_eq!(mesh.smooth, expected["smooth"].as_bool().unwrap(), "{file}: {name} shading");
                    let bounds = mesh.bounds(kition_scene_model::glam::Mat4::IDENTITY);
                    for axis in 0..3 {
                        assert!(close(bounds.min[axis], expected["min"][axis].as_f64().unwrap(), 1e-5), "{file}: {name} bounds");
                        assert!(close(bounds.max[axis], expected["max"][axis].as_f64().unwrap(), 1e-5), "{file}: {name} bounds");
                    }
                    let material = object.material.map(|id| scene.material(id).unwrap().name.clone());
                    assert_eq!(material.as_deref(), expected["material"].as_str(), "{file}: {name} material");
                    let kinds: Vec<&str> = object
                        .modifiers
                        .iter()
                        .map(|modifier| match modifier.kind {
                            ModifierKind::Mirror { .. } => "MIRROR",
                            ModifierKind::Array { .. } => "ARRAY",
                            ModifierKind::Subdivision { .. } => "SUBSURF",
                            ModifierKind::Boolean { .. } => "BOOLEAN",
                            ModifierKind::SimpleDeform { .. } => "SIMPLE_DEFORM",
                            _ => "OTHER",
                        })
                        .collect();
                    let wanted: Vec<&str> = expected["modifiers"].as_array().unwrap().iter().map(|kind| kind.as_str().unwrap()).collect();
                    assert_eq!(kinds, wanted, "{file}: {name} modifiers");
                }
                ObjectData::Camera(camera) => {
                    assert!(close(camera.lens, expected["lens"].as_f64().unwrap(), 1e-4));
                    assert!(close(camera.sensor, expected["sensor"].as_f64().unwrap(), 1e-4));
                    assert_eq!(camera.orthographic, expected["ortho"].as_bool().unwrap());
                    assert!(close(camera.ortho_scale, expected["ortho_scale"].as_f64().unwrap(), 1e-4));
                }
                ObjectData::Light(light) => {
                    assert!(close(light.power, expected["energy"].as_f64().unwrap(), 1e-3));
                    assert_eq!(format!("{:?}", light.kind).to_uppercase(), expected["kind"].as_str().unwrap());
                    assert!(close(light.spot_size, expected["spot"].as_f64().unwrap(), 1e-5));
                }
                ObjectData::Empty { .. } => assert_eq!(expected["type"], "EMPTY"),
                ObjectData::Armature(_) => assert_eq!(expected["type"], "ARMATURE"),
                ObjectData::Curve(_) => assert_eq!(expected["type"], "CURVE"),
                ObjectData::Text(_) => assert_eq!(expected["type"], "FONT"),
                ObjectData::Surface(_) => assert_eq!(expected["type"], "SURFACE"),
                ObjectData::Lattice(_) => assert_eq!(expected["type"], "LATTICE"),
            }
        }
        // The boolean's operand is the placed sphere, and the material is the Principled one.
        let cube = scene.object_named("Cube").unwrap();
        let ball = scene.object_named("Ball").unwrap().id;
        assert!(cube.modifiers.iter().any(|modifier| matches!(modifier.kind, ModifierKind::Boolean { object: Some(id), .. } if id == ball)));
        let red = scene.materials.iter().find(|material| material.name == "Red").unwrap();
        assert_eq!(red.base_color, [0.8, 0.1, 0.05, 1.0]);
        assert!((red.metallic - 0.7).abs() < 1e-6 && (red.roughness - 0.2).abs() < 1e-6);
        assert_eq!(scene.camera, scene.object_named("Camera").map(|object| object.id));
    }

    // A real file, cut and flipped, never panics.
    let bytes = std::fs::read(dir.join("plain.blend")).unwrap();
    let step = (bytes.len() / 3000).max(1);
    for cut in (0..bytes.len()).step_by(step) {
        let _ = kition_scene_blend::read(&bytes[..cut]);
    }
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    for _ in 0..300 {
        let mut copy = bytes.clone();
        for _ in 0..8 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let at = (seed % copy.len() as u64) as usize;
            copy[at] = (seed >> 24) as u8;
        }
        let _ = kition_scene_blend::read(&copy);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
