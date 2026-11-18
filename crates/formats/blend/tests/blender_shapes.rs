//! Curves, texts, and object keyframes in files the reference application writes, read back and
//! compared with what it reports: spline layouts and handles, the meshes the curves evaluate to,
//! text settings and outlines, and keyframed values between and on the keys. Runs when
//! `KITION_BLENDER` names a Blender executable.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_scene_model::glam::Mat4;
use kition_scene_model::{HandleType, ObjectData, Property, Scene, Spline, TextAlign};
use serde_json::Value;

const SCRIPT: &str = r#"
import bpy, sys, json
out = sys.argv[sys.argv.index('--') + 1]
inter = bpy.data.fonts.load(sys.argv[sys.argv.index('--') + 2])
bpy.ops.curve.primitive_bezier_curve_add(location=(0, 2, 0)); c = bpy.context.active_object; c.name = 'Curve'
c.data.bevel_depth = 0.1; c.data.bevel_resolution = 3; c.data.extrude = 0.2; c.data.offset = 0.75; c.data.resolution_u = 6; c.data.use_fill_caps = True
c.data.splines[0].bezier_points[0].radius = 2.0; c.data.splines[0].bezier_points[1].tilt = 0.5
c.data.splines[0].bezier_points[0].handle_left_type = 'VECTOR'
bpy.ops.curve.primitive_bezier_circle_add(radius=1.5, location=(3, 0, 0)); ci = bpy.context.active_object; ci.name = 'Circle'
ci.data.dimensions = '2D'; ci.data.fill_mode = 'FRONT'
bpy.ops.curve.primitive_bezier_circle_add(location=(-3, 0, 0)); ring = bpy.context.active_object; ring.name = 'Ring'
ring.data.bevel_depth = 0.1; ring.data.splines[0].bezier_points[1].co = (0, 1.5, 0.5)
bpy.ops.curve.primitive_nurbs_path_add(location=(0, -3, 0)); p = bpy.context.active_object; p.name = 'Path'; p.data.bevel_depth = 0.05
cu = bpy.data.curves.new('PolyData', 'CURVE'); cu.dimensions = '3D'; s = cu.splines.new('POLY'); s.points.add(2)
for i, co in enumerate([(0, 0, 0, 1), (1, 0, 1, 1), (2, 1, 0, 1)]): s.points[i].co = co
s.use_cyclic_u = True
po = bpy.data.objects.new('Poly', cu); bpy.context.collection.objects.link(po)
bpy.ops.object.text_add(location=(0, 0, 2)); t = bpy.context.active_object; t.name = 'Words'
t.data.font = inter; t.data.body = 'Hi\nYou'; t.data.size = 2.0; t.data.align_x = 'CENTER'; t.data.space_line = 1.2
t.data.offset_x = 0.25; t.data.offset_y = -0.5; t.data.resolution_u = 8
bpy.ops.object.text_add(location=(0, 0, 4)); t2 = bpy.context.active_object; t2.name = 'Solid'
t2.data.font = inter; t2.data.body = 'Kit'; t2.data.extrude = 0.1; t2.data.bevel_depth = 0.02; t2.data.align_x = 'RIGHT'
cube = bpy.data.objects['Cube']
cube.location = (0, 0, 0); cube.keyframe_insert('location', frame=1)
cube.location = (2, 0, 1); cube.keyframe_insert('location', frame=30)
cube.rotation_euler = (0, 0, 0); cube.keyframe_insert('rotation_euler', index=2, frame=1)
cube.rotation_euler = (0, 0, 1.5); cube.keyframe_insert('rotation_euler', index=2, frame=40)
cube.scale = (1, 1, 1); cube.keyframe_insert('scale', index=0, frame=5)
cube.scale = (3, 1, 1); cube.keyframe_insert('scale', index=0, frame=25)
bpy.context.scene.render.fps = 24
bpy.ops.wm.save_as_mainfile(filepath=out + '/shapes.blend', compress=False)
fcurves = []
for layer in cube.animation_data.action.layers:
    for strip in layer.strips:
        for bag in strip.channelbags:
            fcurves += list(bag.fcurves)
for fc in fcurves:
    for k in fc.keyframe_points: k.interpolation = 'LINEAR' if fc.data_path == 'scale' else k.interpolation
bpy.ops.wm.save_as_mainfile(filepath=out + '/shapes.blend', compress=False)
report = {'objects': {}, 'keys': []}
dg = bpy.context.evaluated_depsgraph_get()
for o in bpy.data.objects:
    e = {'type': o.type}
    if o.type == 'CURVE':
        e['splines'] = [{'type': s.type, 'points': len(s.bezier_points) if s.type == 'BEZIER' else len(s.points), 'cyclic': s.use_cyclic_u,
                         'handles': [[b.handle_left_type, b.handle_right_type] for b in s.bezier_points]} for s in o.data.splines]
        e['dims'] = o.data.dimensions; e['fill'] = o.data.fill_mode
    if o.type == 'FONT':
        d = o.data; e.update(body=d.body, size=d.size, align=d.align_x, line=d.space_line, offset=[d.offset_x, d.offset_y], extrude=d.extrude, bevel=d.bevel_depth)
        saved = (d.fill_mode, d.extrude, d.bevel_depth)
        d.fill_mode = 'NONE'; d.extrude = 0; d.bevel_depth = 0; dg.update()
        m = o.evaluated_get(dg).to_mesh(); e['outline'] = len(m.vertices); o.evaluated_get(dg).to_mesh_clear()
        d.fill_mode, d.extrude, d.bevel_depth = saved; dg.update()
    if o.type in ('CURVE', 'FONT'):
        m = o.evaluated_get(dg).to_mesh(); xs = [v.co for v in m.vertices]
        e.update(vertices=len(m.vertices), faces=len(m.polygons), min=[min(c[i] for c in xs) for i in range(3)], max=[max(c[i] for c in xs) for i in range(3)])
        o.evaluated_get(dg).to_mesh_clear()
    report['objects'][o.name] = e
for fc in fcurves:
    report['keys'].append({'path': fc.data_path, 'index': fc.array_index, 'values': [[f, fc.evaluate(f)] for f in (1, 5, 10, 15, 25, 30, 35, 40, 50)],
                           'interpolation': fc.keyframe_points[0].interpolation})
print('REPORT' + json.dumps(report))
"#;

fn close(a: f32, b: f64, tolerance: f64) -> bool {
    (f64::from(a) - b).abs() <= tolerance
}

fn handle_name(kind: HandleType) -> &'static str {
    match kind {
        HandleType::Free => "FREE",
        HandleType::Aligned => "ALIGNED",
        HandleType::Vector => "VECTOR",
        HandleType::Auto => "AUTO",
    }
}

#[test]
fn curves_texts_and_keys_the_reference_writes_read_back() {
    let Some(blender) = std::env::var_os("KITION_BLENDER") else {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    };
    let dir = std::env::temp_dir().join(format!("kition-scene-blend-shapes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // The texts set in the bundled font, as Kition Scene sets every text.
    let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../engine/model/fonts/inter/Inter-Variable.ttf");
    let script = dir.join("make.py");
    std::fs::write(&script, SCRIPT).unwrap();
    let output = std::process::Command::new(blender)
        .args(["-b", "--factory-startup", "--python"])
        .arg(&script)
        .arg("--")
        .arg(&dir)
        .arg(font)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let report: Value = serde_json::from_str(stdout.lines().find_map(|line| line.strip_prefix("REPORT")).expect("Blender's report")).unwrap();

    let bytes = std::fs::read(dir.join("shapes.blend")).unwrap();
    let mut scene = Scene::empty();
    kition_scene_blend::place(&mut scene, kition_scene_blend::read(&bytes).unwrap()).unwrap();
    for (name, expected) in report["objects"].as_object().unwrap() {
        let object = scene.object_named(name).unwrap_or_else(|| panic!("{name} is missing"));
        match &object.data {
            ObjectData::Curve(curve) => {
                assert_eq!(expected["type"], "CURVE");
                assert_eq!(expected["dims"], if curve.dimensions == kition_scene_model::curve::Dimensions::Flat { "2D" } else { "3D" }, "{name}");
                let splines = expected["splines"].as_array().unwrap();
                assert_eq!(curve.splines.len(), splines.len(), "{name}");
                for (spline, wanted) in curve.splines.iter().zip(splines) {
                    let kind = match spline {
                        Spline::Bezier { .. } => "BEZIER",
                        Spline::Poly { .. } => "POLY",
                        Spline::Nurbs { .. } => "NURBS",
                    };
                    assert_eq!(kind, wanted["type"], "{name}");
                    assert_eq!(spline.len() as u64, wanted["points"].as_u64().unwrap(), "{name}");
                    assert_eq!(spline.cyclic(), wanted["cyclic"].as_bool().unwrap(), "{name}");
                    if let Spline::Bezier { points, .. } = spline {
                        let handles: Vec<Value> =
                            points.iter().map(|point| serde_json::json!([handle_name(point.left_type), handle_name(point.right_type)])).collect();
                        assert_eq!(Value::Array(handles), wanted["handles"], "{name}");
                    }
                }
                // The evaluated mesh: counts exactly, bounds within a hair.
                let mesh = curve.to_mesh();
                assert_eq!(mesh.positions.len() as u64, expected["vertices"].as_u64().unwrap(), "{name}: vertices");
                assert_eq!(mesh.faces.len() as u64, expected["faces"].as_u64().unwrap(), "{name}: faces");
                let bounds = mesh.bounds(Mat4::IDENTITY);
                for axis in 0..3 {
                    assert!(close(bounds.min[axis], expected["min"][axis].as_f64().unwrap(), 2e-3), "{name}: bounds {bounds:?} against {expected}");
                    assert!(close(bounds.max[axis], expected["max"][axis].as_f64().unwrap(), 2e-3), "{name}: bounds {bounds:?} against {expected}");
                }
            }
            ObjectData::Text(text) => {
                assert_eq!(expected["type"], "FONT");
                assert_eq!(text.body, expected["body"].as_str().unwrap());
                assert!(close(text.size, expected["size"].as_f64().unwrap(), 1e-6));
                assert!(close(text.line_spacing, expected["line"].as_f64().unwrap(), 1e-6));
                assert!(
                    close(text.offset_x, expected["offset"][0].as_f64().unwrap(), 1e-6)
                        && close(text.offset_y, expected["offset"][1].as_f64().unwrap(), 1e-6)
                );
                assert!(
                    close(text.extrude, expected["extrude"].as_f64().unwrap(), 1e-6)
                        && close(text.bevel_depth, expected["bevel"].as_f64().unwrap(), 1e-6)
                );
                let align = match text.align {
                    TextAlign::Left => "LEFT",
                    TextAlign::Center => "CENTER",
                    TextAlign::Right => "RIGHT",
                    TextAlign::Justify => "JUSTIFY",
                    TextAlign::Flush => "FLUSH",
                };
                assert_eq!(align, expected["align"].as_str().unwrap(), "{name}");
                // The outline matches point for point; the layout's heights exactly and its
                // widths within the reference's glyph rounding.
                assert_eq!(text.outline_count() as u64, expected["outline"].as_u64().unwrap(), "{name}: outline points");
                let bounds = text.to_mesh().bounds(Mat4::IDENTITY);
                for axis in [1, 2] {
                    assert!(close(bounds.min[axis], expected["min"][axis].as_f64().unwrap(), 1e-2), "{name}: bounds {bounds:?} against {expected}");
                    assert!(close(bounds.max[axis], expected["max"][axis].as_f64().unwrap(), 1e-2), "{name}: bounds {bounds:?} against {expected}");
                }
                assert!(
                    close(bounds.min.x, expected["min"][0].as_f64().unwrap(), 3e-2)
                        && close(bounds.max.x, expected["max"][0].as_f64().unwrap(), 3e-2),
                    "{name}: widths {bounds:?} against {expected}"
                );
            }
            _ => {}
        }
    }

    // Keyframes: on the keys and between them (Bézier and linear).
    let cube = scene.object_named("Cube").unwrap().id;
    let keys = report["keys"].as_array().unwrap();
    assert_eq!(scene.animation.channels.iter().filter(|channel| channel.target == cube).count(), keys.len());
    for wanted in keys {
        let property = match wanted["path"].as_str().unwrap() {
            "location" => Property::Location,
            "rotation_euler" => Property::Rotation,
            _ => Property::Scale,
        };
        let index = wanted["index"].as_u64().unwrap() as u8;
        let channel =
            scene.animation.channels.iter().find(|channel| channel.target == cube && channel.property == property && channel.index == index).unwrap();
        for pair in wanted["values"].as_array().unwrap() {
            let (frame, value) = (pair[0].as_f64().unwrap() as f32, pair[1].as_f64().unwrap());
            let found = channel.evaluate(frame).unwrap();
            assert!(close(found, value, 1e-4), "{:?} {index} at {frame}: {found} against {value}", property);
        }
    }
}
