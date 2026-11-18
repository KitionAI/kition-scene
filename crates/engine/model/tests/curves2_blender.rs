//! Bevel and taper objects on curves, text boxes, text on a curve, and NURBS surfaces against the
//! reference application. `fixtures/curves2-reference.json` holds what Blender 5.2.3 made of each
//! case (`tests/blender/curves2.py` writes it; with `KITION_BLENDER` naming a Blender executable
//! the script runs again and must give the same). The vertices are compared as sets: every one of
//! ours must lie near one of the reference's and the other way round.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_scene_model::curve::{BezierPoint, ControlPoint, HandleType, Spline, TaperRadius};
use kition_scene_model::glam::Vec3;
use kition_scene_model::surface::{Knots, SurfacePrimitive};
use kition_scene_model::text::{TextOverflow, TextVerticalAlign};
use kition_scene_model::{Curve, Mesh, ObjectData, Scene, Text, TextAlign};
use serde_json::Value;

fn reference() -> Value {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/curves2-reference.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn cases(kind: &str) -> Vec<Value> {
    reference()["cases"].as_array().unwrap().iter().filter(|case| case["kind"] == kind).cloned().collect()
}

fn vec3(value: &Value) -> Vec3 {
    Vec3::new(value[0].as_f64().unwrap() as f32, value[1].as_f64().unwrap() as f32, value[2].as_f64().unwrap() as f32)
}

fn number(value: &Value) -> f32 {
    value.as_f64().unwrap() as f32
}

/// The splines the script wrote, with the handles the reference worked out kept as they are.
fn splines(value: &Value) -> Vec<Spline> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|spline| {
            let cyclic = spline["cyclic"].as_bool().unwrap();
            let points = spline["points"].as_array().unwrap();
            if spline["type"] == "bezier" {
                Spline::Bezier {
                    points: points
                        .iter()
                        .map(|point| BezierPoint {
                            co: vec3(&point["co"]),
                            left: vec3(&point["left"]),
                            right: vec3(&point["right"]),
                            left_type: HandleType::Free,
                            right_type: HandleType::Free,
                            radius: number(&point["radius"]),
                            tilt: 0.0,
                        })
                        .collect(),
                    cyclic,
                    resolution: spline["resolution"].as_u64().unwrap() as u32,
                }
            } else {
                Spline::Poly {
                    points: points
                        .iter()
                        .map(|point| ControlPoint { radius: number(&point["radius"]), ..ControlPoint::at(vec3(&point["co"])) })
                        .collect(),
                    cyclic,
                }
            }
        })
        .collect()
}

fn curve(value: &Value) -> Curve {
    Curve { splines: splines(value), ..Curve::default() }
}

/// The farthest any vertex of either mesh is from the nearest vertex of the other.
fn apart(ours: &Mesh, theirs: &Value) -> f32 {
    let theirs: Vec<Vec3> = theirs["positions"].as_array().unwrap().iter().map(vec3).collect();
    let nearest = |point: &Vec3, among: &[Vec3]| among.iter().map(|other| other.distance(*point)).fold(f32::INFINITY, f32::min);
    let one = ours.positions.iter().map(|point| nearest(point, &theirs)).fold(0.0, f32::max);
    let other = theirs.iter().map(|point| nearest(point, &ours.positions)).fold(0.0, f32::max);
    one.max(other)
}

fn evaluate(scene: &Scene, id: u32) -> Mesh {
    scene.object(id).unwrap().evaluated_mesh(scene).unwrap()
}

#[test]
fn bevel_objects_sweep_like_the_reference() {
    for case in cases("bevel") {
        let what = format!("{} profile, caps {}", case["name"], case["caps"]);
        let mut scene = Scene::empty();
        let profile = scene.add_object("Profile", ObjectData::Curve(curve(&case["profile"])));
        scene.object_mut(profile).unwrap().scale = vec3(&case["scale"]);
        let path = Curve { bevel_object: Some(profile), fill_caps: case["caps"].as_bool().unwrap(), ..curve(&case["path"]) };
        let path = scene.add_object("Path", ObjectData::Curve(path));
        let mesh = evaluate(&scene, path);
        let result = &case["result"];
        assert_eq!(
            (mesh.positions.len(), mesh.faces.len()),
            (result["vertices"].as_u64().unwrap() as usize, result["faces"].as_u64().unwrap() as usize),
            "{what}"
        );
        let worst = apart(&mesh, result);
        assert!(worst < 1e-3, "{what}: {worst}");
    }
}

#[test]
fn taper_objects_thin_the_bevel_like_the_reference() {
    for case in cases("taper") {
        let what = format!("{} taper, {}", case["name"], case["mode"]);
        let mut scene = Scene::empty();
        let taper = scene.add_object("Taper", ObjectData::Curve(curve(&case["taper"])));
        let mode = match case["mode"].as_str().unwrap() {
            "MULTIPLY" => TaperRadius::Multiply,
            "ADD" => TaperRadius::Add,
            _ => TaperRadius::Override,
        };
        let path = Curve {
            taper_object: Some(taper),
            taper_radius: mode,
            bevel_depth: number(&case["bevel_depth"]),
            bevel_resolution: case["bevel_resolution"].as_u64().unwrap() as u32,
            ..curve(&case["path"])
        };
        let path = scene.add_object("Path", ObjectData::Curve(path));
        let mesh = evaluate(&scene, path);
        assert_eq!(mesh.positions.len(), case["result"]["vertices"].as_u64().unwrap() as usize, "{what}");
        let worst = apart(&mesh, &case["result"]);
        assert!(worst < 1e-3, "{what}: {worst}");
    }
}

fn text_of(settings: &Value) -> Text {
    let string = |key: &str, default: &str| settings.get(key).and_then(Value::as_str).unwrap_or(default).to_owned();
    let float = |key: &str, default: f32| settings.get(key).and_then(Value::as_f64).map_or(default, |value| value as f32);
    Text {
        body: string("body", "Cube sets type in a box"),
        size: float("size", 1.0),
        line_spacing: float("line_spacing", 1.0),
        resolution: 4,
        align: match string("align", "LEFT").as_str() {
            "CENTER" => TextAlign::Center,
            "RIGHT" => TextAlign::Right,
            "JUSTIFY" => TextAlign::Justify,
            "FLUSH" => TextAlign::Flush,
            _ => TextAlign::Left,
        },
        align_y: match string("align_y", "TOP_BASELINE").as_str() {
            "TOP" => TextVerticalAlign::Top,
            "CENTER" => TextVerticalAlign::Center,
            "BOTTOM" => TextVerticalAlign::Bottom,
            "BOTTOM_BASELINE" => TextVerticalAlign::BottomBaseline,
            _ => TextVerticalAlign::TopBaseline,
        },
        overflow: match string("overflow", "NONE").as_str() {
            "SCALE" => TextOverflow::Scale,
            "TRUNCATE" => TextOverflow::Truncate,
            _ => TextOverflow::None,
        },
        box_width: float("width", 0.0),
        box_height: float("height", 0.0),
        box_x: float("x", 0.0),
        box_y: float("y", 0.0),
        ..Text::default()
    }
}

/// The text's outline points as a mesh of loose vertices.
fn outline(text: &Text, path: Option<&kition_scene_model::text::Path>) -> Mesh {
    Mesh { positions: text.outline_points(path), ..Mesh::default() }
}

#[test]
fn text_boxes_wrap_align_and_overflow_like_the_reference() {
    // The outline, point for point, with glyphs read at the reference's optical size.
    for case in cases("text").into_iter().filter(|case| case.get("curve").is_none()) {
        let what = case["name"].as_str().unwrap().to_owned();
        let mesh = outline(&text_of(&case["settings"]), None);
        let result = &case["result"];
        assert_eq!(mesh.positions.len(), result["vertices"].as_u64().unwrap() as usize, "{what}");
        let worst = apart(&mesh, result);
        assert!(worst < 1e-3, "{what}: {worst}");
    }
}

#[test]
fn text_on_a_curve_follows_it_like_the_reference() {
    for case in cases("text").into_iter().filter(|case| case.get("curve").is_some()) {
        let what = case["name"].as_str().unwrap().to_owned();
        let path = kition_scene_model::text::Path::of(&curve(&case["curve"]), Vec3::ONE).unwrap();
        let mesh = outline(&text_of(&case["settings"]), Some(&path));
        let result = &case["result"];
        assert_eq!(mesh.positions.len(), result["vertices"].as_u64().unwrap() as usize, "{what}");
        let worst = apart(&mesh, result);
        assert!(worst < 1e-3, "{what}: {worst}");
    }
}

#[test]
fn surfaces_evaluate_like_the_reference() {
    for case in cases("surface") {
        let what = case["name"].as_str().unwrap().to_owned();
        let primitive = SurfacePrimitive::ALL.into_iter().find(|primitive| {
            let id = match case["primitive"].as_str().unwrap() {
                "surface" => "patch",
                other => other,
            };
            primitive.id() == id
        });
        let mut surface = primitive.unwrap().build(1.0);
        let spline = &case["spline"];
        let knots =
            |value: &Value| Knots { cyclic: value[0].as_bool().unwrap(), endpoint: value[1].as_bool().unwrap(), bezier: value[2].as_bool().unwrap() };
        let patch = &mut surface.patches[0];
        patch.order_u = spline["order_u"].as_u64().unwrap() as u32;
        if patch.count_v > 1 {
            patch.order_v = spline["order_v"].as_u64().unwrap() as u32;
        }
        patch.resolution_u = spline["resolution_u"].as_u64().unwrap() as u32;
        patch.resolution_v = spline["resolution_v"].as_u64().unwrap() as u32;
        patch.knots_u = knots(&spline["knots_u"]);
        patch.knots_v = knots(&spline["knots_v"]);
        let mesh = surface.to_mesh();
        let result = &case["result"];
        assert_eq!(
            (mesh.positions.len(), mesh.faces.len()),
            (result["vertices"].as_u64().unwrap() as usize, result["faces"].as_u64().unwrap() as usize),
            "{what}"
        );
        let worst = apart(&mesh, result);
        assert!(worst < 1e-3, "{what}: {worst}");
    }
}

#[test]
fn the_reference_gives_the_recorded_results() {
    let Some(executable) = std::env::var_os("KITION_BLENDER") else {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    };
    let out = std::env::temp_dir().join(format!("kition-scene-curves2-{}.json", std::process::id()));
    let status = std::process::Command::new(executable)
        .args(["-b", "--factory-startup", "--python"])
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/blender/curves2.py"))
        .arg("--")
        .arg(&out)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let fresh: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    std::fs::remove_file(&out).ok();
    assert_eq!(fresh["cases"], reference()["cases"]);
}
