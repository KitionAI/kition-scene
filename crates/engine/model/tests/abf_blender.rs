//! Angle-based unwrapping against the reference application's angle-based unwrap of the same mesh
//! with the same seams: angle and area distortion of both, measured the same way. Runs when
//! `KITION_BLENDER` names a Blender executable; otherwise it is skipped.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::collections::BTreeSet;

use kition_scene_model::glam::{Vec2, Vec3};
use kition_scene_model::unwrap::{Method, distortion, unwrap_with};
use kition_scene_model::{Mesh, Primitive, seams};

const SCRIPT: &str = r#"
import bpy, bmesh, json, sys
args = json.loads(sys.argv[sys.argv.index('--') + 1])
bpy.ops.object.select_all(action='SELECT'); bpy.ops.object.delete()
if args['shape'] == 'cylinder':
    bpy.ops.mesh.primitive_cylinder_add(vertices=16, radius=1, depth=2)
else:
    bpy.ops.mesh.primitive_uv_sphere_add(segments=16, ring_count=8, radius=1)
obj = bpy.context.active_object
bpy.ops.object.mode_set(mode='EDIT')
bm = bmesh.from_edit_mesh(obj.data)
for edge in bm.edges:
    a, b = edge.verts[0].co, edge.verts[1].co
    edge.seam = eval(args['rule'])
    edge.select = False
for face in bm.faces:
    face.select = True
bmesh.update_edit_mesh(obj.data)
bpy.ops.mesh.select_all(action='SELECT')
bpy.ops.uv.unwrap(method=args['method'], margin=0.02)
bpy.ops.object.mode_set(mode='OBJECT')
mesh = obj.data
uv = mesh.uv_layers.active.data
out = {
    'positions': [list(v.co) for v in mesh.vertices],
    'faces': [list(p.vertices) for p in mesh.polygons],
    'uvs': [[list(uv[i].uv) for i in p.loop_indices] for p in mesh.polygons],
}
json.dump(out, open(args['out'], 'w'))
"#;

fn blender(shape: &str, rule: &str, method: &str) -> Option<Mesh> {
    let executable = std::env::var_os("KITION_BLENDER")?;
    let dir = std::env::temp_dir().join(format!("kition-scene-abf-{}-{shape}-{method}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("unwrap.py");
    std::fs::write(&script, SCRIPT).unwrap();
    let out = dir.join("out.json");
    let args = serde_json::json!({ "shape": shape, "rule": rule, "method": method, "out": out.to_str().unwrap() });
    let status = std::process::Command::new(executable)
        .args(["-b", "--factory-startup", "--python"])
        .arg(&script)
        .arg("--")
        .arg(args.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "Blender failed");
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let floats = |value: &serde_json::Value| value.as_array().unwrap().iter().map(|item| item.as_f64().unwrap() as f32).collect::<Vec<f32>>();
    let positions = value["positions"].as_array().unwrap().iter().map(|p| Vec3::from_slice(&floats(p))).collect();
    let faces =
        value["faces"].as_array().unwrap().iter().map(|face| face.as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect()).collect();
    let uvs = value["uvs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|face| face.as_array().unwrap().iter().map(|uv| Vec2::from_slice(&floats(uv))).collect())
        .collect();
    Some(Mesh { positions, faces, uvs, ..Default::default() })
}

fn ours(mut mesh: Mesh, rule: impl Fn(Vec3, Vec3) -> bool, method: Method) -> Mesh {
    let cuts: Vec<(u32, u32)> =
        mesh.edges().into_iter().filter(|[a, b]| rule(mesh.positions[*a as usize], mesh.positions[*b as usize])).map(|[a, b]| (a, b)).collect();
    seams::mark(&mut mesh, &cuts, true).unwrap();
    unwrap_with(&mut mesh, &BTreeSet::new(), 0.02, method).unwrap();
    mesh
}

#[test]
fn angle_based_unwraps_as_well_as_the_reference() {
    if std::env::var_os("KITION_BLENDER").is_none() {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    }
    let all = BTreeSet::new();
    // The sphere cut along one meridian; the cylinder along one side and around both caps.
    let sphere_rule = "abs(a.x) < 1e-4 and abs(b.x) < 1e-4 and a.y > -1e-4 and b.y > -1e-4";
    let cylinder_rule = "(abs(a.x) < 1e-4 and abs(b.x) < 1e-4 and a.y > 0 and b.y > 0) or (abs(a.z - b.z) < 1e-4 and abs(abs(a.z) - 1) < 1e-4)";
    let sphere_cut = |a: Vec3, b: Vec3| a.x.abs() < 1e-4 && b.x.abs() < 1e-4 && a.y > -1e-4 && b.y > -1e-4;
    let cylinder_cut = |a: Vec3, b: Vec3| {
        (a.x.abs() < 1e-4 && b.x.abs() < 1e-4 && a.y > 0.0 && b.y > 0.0) || ((a.z - b.z).abs() < 1e-4 && (a.z.abs() - 1.0).abs() < 1e-4)
    };
    let sphere = || Primitive::UvSphere { segments: 16, rings: 8, radius: 1.0 }.build().unwrap();
    let cylinder = || Primitive::Cylinder { vertices: 16, radius: 1.0, depth: 2.0, fill: kition_scene_model::Fill::Ngon }.build().unwrap();
    for (name, ours_mesh, reference) in [
        ("sphere", ours(sphere(), sphere_cut, Method::AngleBased), blender("sphere", sphere_rule, "ANGLE_BASED").unwrap()),
        ("cylinder", ours(cylinder(), cylinder_cut, Method::AngleBased), blender("cylinder", cylinder_rule, "ANGLE_BASED").unwrap()),
    ] {
        let (angle, area) = distortion(&ours_mesh, &all);
        let (reference_angle, reference_area) = distortion(&reference, &all);
        let conformal = match name {
            "sphere" => distortion(&ours(sphere(), sphere_cut, Method::Conformal), &all),
            _ => distortion(&ours(cylinder(), cylinder_cut, Method::Conformal), &all),
        };
        eprintln!(
            "{name}: ours angle-based angle {angle:.4} area {area:.4}; ours conformal angle {:.4} area {:.4}; reference angle-based angle {reference_angle:.4} area {reference_area:.4}",
            conformal.0, conformal.1
        );
        assert!(angle <= reference_angle * 1.05 + 1e-3, "{name}: angle {angle} against {reference_angle}");
        assert!(area <= reference_area * 1.05 + 1e-3, "{name}: area {area} against {reference_area}");
    }
}
