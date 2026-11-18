//! Shrinkwrap, Lattice, Decimate's un-subdivide and planar ways, and Displace with an image
//! against the reference application. `fixtures/modifiers3-reference.json` holds what Blender
//! 5.2.3 made of each case (`tests/blender/modifiers3.py` writes it; with `KITION_BLENDER`
//! naming a Blender executable the script runs again and must give the same). Each case is
//! built here from the same input mesh, so vertices compare one to one.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::collections::BTreeMap;

use kition_scene_model::glam::{Vec2, Vec3};
use kition_scene_model::modifiers::DecimateMode;
use kition_scene_model::modifiers2::TextureCoordinates;
use kition_scene_model::shrinkwrap::WrapMethod;
use kition_scene_model::{Image, Lattice, LatticeInterpolation, Mesh, Modifier, ModifierKind, ObjectData, Scene};
use serde_json::Value;

fn reference() -> Value {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/modifiers3-reference.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn vec3(value: &Value) -> Vec3 {
    Vec3::new(value[0].as_f64().unwrap() as f32, value[1].as_f64().unwrap() as f32, value[2].as_f64().unwrap() as f32)
}

fn mesh(value: &Value) -> Mesh {
    let positions = value["positions"].as_array().unwrap().iter().map(vec3).collect();
    let faces = value["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|face| face.as_array().unwrap().iter().map(|index| index.as_u64().unwrap() as u32).collect())
        .collect();
    let mut out = Mesh { positions, faces, ..Default::default() };
    if let Some(uvs) = value.get("uvs").and_then(Value::as_array) {
        out.uvs = uvs
            .iter()
            .map(|face| face.as_array().unwrap().iter().map(|uv| Vec2::new(uv[0].as_f64().unwrap() as f32, uv[1].as_f64().unwrap() as f32)).collect())
            .collect();
    }
    out
}

/// The largest distance between our vertices and the reference's.
fn worst(ours: &Mesh, theirs: &Value) -> f32 {
    let theirs = mesh(theirs);
    assert_eq!(ours.positions.len(), theirs.positions.len());
    ours.positions.iter().zip(&theirs.positions).map(|(a, b)| a.distance(*b)).fold(0.0, f32::max)
}

fn evaluate(scene: &Scene, id: u32) -> Mesh {
    scene.object(id).unwrap().evaluated_mesh(scene).unwrap()
}

fn cases(kind: &str) -> Vec<Value> {
    reference()["cases"].as_array().unwrap().iter().filter(|case| case["kind"] == kind).cloned().collect()
}

#[test]
fn shrinkwrap_lands_where_the_reference_does() {
    for case in cases("shrinkwrap") {
        let mut scene = Scene::empty();
        let target = scene.add_object("Target", ObjectData::Mesh(mesh(&case["target"])));
        let sphere = scene.add_object("Sphere", ObjectData::Mesh(mesh(&case["mesh"])));
        let method = match case["method"].as_str().unwrap() {
            "PROJECT" => WrapMethod::Project,
            "NEAREST_VERTEX" => WrapMethod::NearestVertex,
            _ => WrapMethod::NearestSurface,
        };
        scene.object_mut(sphere).unwrap().modifiers.push(Modifier::new(ModifierKind::Shrinkwrap {
            object: Some(target),
            wrap_method: method,
            offset: case["offset"].as_f64().unwrap() as f32,
            positive: true,
            negative: case["negative"].as_bool().unwrap(),
        }));
        let ours = evaluate(&scene, sphere);
        let theirs = mesh(&case["result"]);
        let before = mesh(&case["mesh"]);
        // A vertex as near two faces (or vertices) may go to either: then both must have moved
        // as far.
        let error = ours
            .positions
            .iter()
            .zip(&theirs.positions)
            .zip(&before.positions)
            .map(|((a, b), start)| {
                let apart = a.distance(*b);
                if method == WrapMethod::Project { apart } else { apart.min((a.distance(*start) - b.distance(*start)).abs()) }
            })
            .fold(0.0, f32::max);
        let label = format!("{method:?} offset {} negative {}", case["offset"], case["negative"]);
        eprintln!("shrinkwrap {label}: {error:.5}");
        assert!(error < 2e-3, "{label}: {error}");
    }
}

#[test]
fn lattices_bend_meshes_as_the_reference_does() {
    for case in cases("lattice") {
        let resolution = case["resolution"].as_u64().unwrap() as u32;
        let mut lattice = Lattice::new([resolution; 3]);
        lattice.interpolation = if case["interpolation"] == "linear" { LatticeInterpolation::Linear } else { LatticeInterpolation::Bspline };
        for step in case["moves"].as_array().unwrap() {
            lattice.points[step[0].as_u64().unwrap() as usize] += vec3(&step[1]);
        }
        let mut scene = Scene::empty();
        let grid = scene.add_object("Lattice", ObjectData::Lattice(lattice));
        let object = scene.object_mut(grid).unwrap();
        object.scale = Vec3::splat(case["scale"].as_f64().unwrap() as f32);
        object.location = vec3(&case["location"]);
        let cube = scene.add_object("Cube", ObjectData::Mesh(mesh(&case["mesh"])));
        scene
            .object_mut(cube)
            .unwrap()
            .modifiers
            .push(Modifier::new(ModifierKind::Lattice { object: Some(grid), strength: case["strength"].as_f64().unwrap() as f32 }));
        let error = worst(&evaluate(&scene, cube), &case["result"]);
        let label = format!("{resolution} {} strength {}", case["interpolation"], case["strength"]);
        eprintln!("lattice {label}: {error:.5}");
        assert!(error < 1e-3, "{label}: {error}");
    }
}

#[test]
fn decimation_matches_the_reference() {
    for case in cases("decimate") {
        let settings = &case["settings"];
        let unsubdivide = settings["decimate_type"] == "UNSUBDIV";
        let kind = ModifierKind::Decimate {
            mode: if unsubdivide { DecimateMode::Unsubdivide } else { DecimateMode::Planar },
            ratio: 1.0,
            iterations: settings["iterations"].as_u64().unwrap_or(0) as u32,
            angle_limit: settings["angle_limit"].as_f64().unwrap_or(0.0) as f32,
        };
        let mut scene = Scene::empty();
        let id = scene.add_object("Mesh", ObjectData::Mesh(mesh(&case["mesh"])));
        scene.object_mut(id).unwrap().modifiers.push(Modifier::new(kind));
        let out = evaluate(&scene, id);
        out.validate().unwrap();
        let mut sizes: BTreeMap<String, u64> = BTreeMap::new();
        for face in &out.faces {
            *sizes.entry(face.len().to_string()).or_default() += 1;
        }
        let label = format!("{} {settings}", case["name"]);
        let (vertices, faces) = (case["vertices"].as_u64().unwrap() as usize, case["faces"].as_u64().unwrap() as usize);
        eprintln!("decimate {label}: {} vertices, {} faces {sizes:?} (reference {vertices}, {faces})", out.positions.len(), out.faces.len());
        // Planar joins on a curved surface depend on the order of near ties, so a sphere's
        // counts are compared loosely; the rest match exactly, vertex for vertex.
        if case["name"] == "sphere" && !unsubdivide {
            let near = |ours: usize, theirs: usize| (ours as f32 - theirs as f32).abs() <= theirs as f32 * 0.25;
            assert!(near(out.positions.len(), vertices) && near(out.faces.len(), faces), "{label}");
            continue;
        }
        assert_eq!((out.positions.len(), out.faces.len()), (vertices, faces), "{label}");
        let expected: BTreeMap<String, u64> = serde_json::from_value(case["sizes"].clone()).unwrap();
        assert_eq!(sizes, expected, "{label}");
        let mut ours: Vec<[i64; 3]> = out.positions.iter().map(|point| point.to_array().map(|value| (value * 1e4).round() as i64)).collect();
        let mut theirs: Vec<[i64; 3]> = mesh(&json_positions(&case["positions"]))
            .positions
            .iter()
            .map(|point| point.to_array().map(|value| (value * 1e4).round() as i64))
            .collect();
        ours.sort_unstable();
        theirs.sort_unstable();
        assert_eq!(ours, theirs, "{label}");
    }
}

fn json_positions(positions: &Value) -> Value {
    serde_json::json!({ "positions": positions, "faces": [] })
}

/// A PNG of the rows the reference stored (top row first).
fn png(rows: &Value) -> Vec<u8> {
    let rows = rows.as_array().unwrap();
    let (height, width) = (rows.len() as u32, rows[0].as_array().unwrap().len() as u32);
    let mut data = Vec::new();
    for row in rows {
        for pixel in row.as_array().unwrap() {
            data.extend(pixel.as_array().unwrap().iter().map(|channel| channel.as_u64().unwrap() as u8));
        }
    }
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(&data).unwrap();
    bytes
}

#[test]
fn displace_reads_images_as_the_reference_does() {
    for case in cases("displace") {
        let mut scene = Scene::empty();
        let hash = scene.add_image(Image::new("Image.png", png(&case["image"])).unwrap());
        let coordinates = match case["coordinates"].as_str().unwrap() {
            "uv" => TextureCoordinates::Uv,
            "global" => TextureCoordinates::Global,
            _ => TextureCoordinates::Local,
        };
        let id = scene.add_object("Plane", ObjectData::Mesh(mesh(&case["mesh"])));
        let object = scene.object_mut(id).unwrap();
        object.location = vec3(&case["location"]);
        let mut kind = ModifierKind::default_of("displace").unwrap();
        if let ModifierKind::Displace { strength, image, coordinates: slot, .. } = &mut kind {
            *strength = case["strength"].as_f64().unwrap() as f32;
            *image = Some(hash.clone());
            *slot = coordinates;
        }
        object.modifiers.push(Modifier::new(kind));
        let error = worst(&evaluate(&scene, id), &case["result"]);
        let label = format!("{}×{} image, {coordinates:?}", case["image"][0].as_array().unwrap().len(), case["image"].as_array().unwrap().len());
        eprintln!("displace {label}: {error:.5}");
        assert!(error < 2e-3, "{label}: {error}");
    }
}

#[test]
fn the_reference_gives_the_recorded_results() {
    let Some(executable) = std::env::var_os("KITION_BLENDER") else {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    };
    let out = std::env::temp_dir().join(format!("kition-scene-modifiers3-{}.json", std::process::id()));
    let status = std::process::Command::new(executable)
        .args(["-b", "--factory-startup", "--python"])
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/blender/modifiers3.py"))
        .arg("--")
        .arg(&out)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    // The reference lists a sphere's faces in a different order from run to run, so faces (and
    // the texture coordinates that follow them) are left out of the comparison, and its planar
    // decimation of a sphere differs by a few faces between runs, so only that it ran is.
    fn without_faces(value: &mut Value) {
        if value["kind"] == "decimate" && value["name"] == "sphere" && value["settings"]["decimate_type"] == "DISSOLVE" {
            for key in ["vertices", "faces", "sizes", "positions"] {
                value.as_object_mut().map(|map| map.remove(key));
            }
        }
        match value {
            Value::Object(map) => {
                map.remove("faces");
                map.remove("uvs");
                map.values_mut().for_each(without_faces);
            }
            Value::Array(list) => list.iter_mut().for_each(without_faces),
            _ => {}
        }
    }
    let mut fresh: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    let mut recorded = reference();
    without_faces(&mut fresh);
    without_faces(&mut recorded);
    // Numbers within 10⁻⁴ (the last recorded digit moves with the order the reference sums in).
    fn same(a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Number(x), Value::Number(y)) => (x.as_f64().unwrap() - y.as_f64().unwrap()).abs() <= 1e-4,
            (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(x, y)| same(x, y)),
            (Value::Object(x), Value::Object(y)) => {
                x.len() == y.len() && x.iter().all(|(key, value)| y.get(key).is_some_and(|other| same(value, other)))
            }
            _ => a == b,
        }
    }
    let changed: Vec<usize> =
        (0..recorded["cases"].as_array().unwrap().len()).filter(|index| !same(&fresh["cases"][index], &recorded["cases"][index])).collect();
    assert!(changed.is_empty(), "the reference's results changed for cases {changed:?}; rerun tests/blender/modifiers3.py into the fixture");
}
