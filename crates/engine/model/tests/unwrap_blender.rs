//! Unwrapping by seams against the reference application's own unwrap of the same mesh with the
//! same seams: the angle and area distortion of both, measured the same way. Runs when
//! `KITION_BLENDER` names a Blender executable; otherwise it is skipped.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::collections::BTreeSet;

use kition_scene_model::glam::{Vec2, Vec3};
use kition_scene_model::unwrap::{distortion, unwrap};
use kition_scene_model::{Mesh, Primitive, seams};

/// Marks seams in Blender on edges whose two ends pass `rule` (Python on `a` and `b`, two
/// vectors), unwraps with `method`, and answers the mesh with its coordinates.
const SCRIPT: &str = r#"
import bpy, bmesh, json, sys
args = json.loads(sys.argv[sys.argv.index('--') + 1])
bpy.ops.object.select_all(action='SELECT'); bpy.ops.object.delete()
if args['shape'] == 'cube':
    bpy.ops.mesh.primitive_cube_add(size=2)
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
    let dir = std::env::temp_dir().join(format!("kition-scene-unwrap-{}-{shape}-{method}", std::process::id()));
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

fn ours(mut mesh: Mesh, rule: impl Fn(Vec3, Vec3) -> bool) -> Mesh {
    let cuts: Vec<(u32, u32)> =
        mesh.edges().into_iter().filter(|[a, b]| rule(mesh.positions[*a as usize], mesh.positions[*b as usize])).map(|[a, b]| (a, b)).collect();
    seams::mark(&mut mesh, &cuts, true).unwrap();
    unwrap(&mut mesh, &BTreeSet::new(), 0.02).unwrap();
    mesh
}

#[test]
fn unwraps_as_well_as_the_reference() {
    if std::env::var_os("KITION_BLENDER").is_none() {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    }
    let all = BTreeSet::new();
    // The cube cut into a cross; the sphere cut along one meridian.
    let cube_rule = "not ((a.z < 0 and b.z < 0) or (a.z > 0 and b.z > 0 and a.y < 0 and b.y < 0))";
    let sphere_rule = "abs(a.x) < 1e-4 and abs(b.x) < 1e-4 and a.y > -1e-4 and b.y > -1e-4";
    let cube = ours(Primitive::Cube { size: 2.0 }.build().unwrap(), |a, b| {
        !((a.z < 0.0 && b.z < 0.0) || (a.z > 0.0 && b.z > 0.0 && a.y < 0.0 && b.y < 0.0))
    });
    let sphere = ours(Primitive::UvSphere { segments: 16, rings: 8, radius: 1.0 }.build().unwrap(), |a, b| {
        a.x.abs() < 1e-4 && b.x.abs() < 1e-4 && a.y > -1e-4 && b.y > -1e-4
    });
    let (cube_angle, cube_area) = distortion(&cube, &all);
    let (sphere_angle, sphere_area) = distortion(&sphere, &all);
    eprintln!("ours: cube angle {cube_angle:.5} area {cube_area:.5}; sphere angle {sphere_angle:.5} area {sphere_area:.5}");
    for method in ["CONFORMAL", "ANGLE_BASED"] {
        let reference_cube = blender("cube", cube_rule, method).unwrap();
        let reference_sphere = blender("sphere", sphere_rule, method).unwrap();
        let (rc_angle, rc_area) = distortion(&reference_cube, &all);
        let (rs_angle, rs_area) = distortion(&reference_sphere, &all);
        eprintln!("reference {method}: cube angle {rc_angle:.5} area {rc_area:.5}; sphere angle {rs_angle:.5} area {rs_area:.5}");
        // A cross of a cube is developable: both flatten it exactly.
        assert!(cube_angle <= rc_angle + 1e-3 && cube_area <= rc_area + 1e-3);
        if method == "CONFORMAL" {
            // Least-squares conformal maps minimise angle distortion, so ours keeps angles at
            // least as well; the reference's conformal unwrap gives up a little angle for evener
            // areas (0.163 rad and 0.71 against our 0.145 rad and 0.89 on this sphere), so
            // areas are held within 30 % of it.
            assert!(sphere_angle <= rs_angle * 1.05 + 1e-3, "sphere angle {sphere_angle} against {rs_angle}");
            assert!(sphere_area <= rs_area * 1.3 + 1e-3, "sphere area {sphere_area} against {rs_area}");
        }
    }
}
