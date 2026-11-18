//! Render passes against the reference application's path tracer (Cycles 5.2.3, CPU, no
//! denoising) on the same scenes at 96 × 54: `still` (a floor, a red cube parented to an empty,
//! a metal ball, a glowing box, a sun, a grey world) with every light pass, normal, ambient
//! occlusion, environment, and Cryptomatte; `moving` and `moving-blur` (a sliding cube and a
//! turning one) with the vector pass, motion blur off and on; and the Cryptomatte node's mattes
//! and Pick (`still` and `pick`). The references
//! (`fixtures/passes/<case>.PASSES.CYCLES.json`, made by `blender/passes.py`) hold each pass's
//! means over 8 × 9 pixel blocks and its values at the blocks' centres, and the Cryptomatte
//! manifests; with `KITION_BLENDER` naming a Blender executable the script runs again and must
//! give the same.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic, clippy::needless_range_loop)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use kition_scene_model::glam::Vec3;
use kition_scene_model::{Id, Light, LightKind, ObjectData, Primitive, Property, Scene};
use kition_scene_trace::passes::{Geometry, geometry};
use kition_scene_trace::{TraceOptions, Tracer};
use serde_json::Value;

const WIDTH: u32 = 96;
const HEIGHT: u32 = 54;
const BLOCK: [u32; 2] = [8, 9];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/passes")
}

fn material(scene: &mut Scene, name: &str, colour: [f32; 3], roughness: f32, metallic: f32) -> Id {
    let id = scene.add_material(name);
    let m = scene.material_mut(id).unwrap();
    m.base_color = [colour[0], colour[1], colour[2], 1.0];
    m.roughness = roughness;
    m.metallic = metallic;
    id
}

fn mesh(scene: &mut Scene, name: &str, primitive: Primitive, location: Vec3, material: Id, smooth: bool) -> Id {
    let mut built = primitive.build().unwrap();
    built.smooth = smooth;
    let id = scene.add_object(name, ObjectData::Mesh(built));
    let object = scene.object_mut(id).unwrap();
    object.location = location;
    object.material = Some(material);
    id
}

fn base() -> Scene {
    let mut scene = Scene::empty();
    scene.world.color = Vec3::splat(0.05);
    scene.render.width = WIDTH;
    scene.render.height = HEIGHT;
    let floor = material(&mut scene, "Floor", [0.8; 3], 0.5, 0.0);
    mesh(&mut scene, "Floor", Primitive::Plane { size: 20.0 }, Vec3::ZERO, floor, false);
    let mut sun = Light::new(LightKind::Sun);
    sun.power = 3.0;
    sun.radius = 0.1;
    let id = scene.add_object("Sun", ObjectData::Light(sun));
    scene.object_mut(id).unwrap().rotation = Vec3::new(40.0, 0.0, 30.0) * (std::f32::consts::PI / 180.0);
    let camera = scene.add_object("Camera", ObjectData::Camera(Default::default()));
    let object = scene.object_mut(camera).unwrap();
    object.location = Vec3::new(7.0, -7.0, 4.5);
    object.rotation = Vec3::new(66.0, 0.0, 45.0) * (std::f32::consts::PI / 180.0);
    scene.camera = Some(camera);
    scene
}

fn still() -> Scene {
    let mut scene = base();
    let rig = scene.add_object("Rig", ObjectData::Empty { size: 1.0 });
    let red = material(&mut scene, "Red", [0.8, 0.1, 0.1], 0.5, 0.0);
    let cube = mesh(&mut scene, "Cube", Primitive::Cube { size: 1.0 }, Vec3::new(-1.0, 0.0, 0.5), red, false);
    scene.set_parent(cube, Some(rig)).unwrap();
    let metal = material(&mut scene, "Metal", [0.9, 0.7, 0.4], 0.25, 1.0);
    mesh(&mut scene, "Ball", Primitive::UvSphere { segments: 32, rings: 16, radius: 0.6 }, Vec3::new(1.0, 0.5, 0.6), metal, true);
    let glow = scene.add_material("Glow");
    let m = scene.material_mut(glow).unwrap();
    m.base_color = [0.0, 0.0, 0.0, 1.0];
    m.emission = Vec3::new(1.0, 0.8, 0.4);
    m.emission_strength = 5.0;
    m.roughness = 1.0;
    mesh(&mut scene, "Lamp", Primitive::Cube { size: 0.4 }, Vec3::new(0.5, -1.5, 0.2), glow, false);
    scene
}

fn key(scene: &mut Scene, id: Id, property: Property, frame: f32, value: Vec3) {
    let object = scene.object_mut(id).unwrap();
    match property {
        Property::Location => object.location = value,
        Property::Rotation => object.rotation = value,
        _ => panic!("not keyed here"),
    }
    scene.insert_key(id, property, frame).unwrap();
}

fn moving(blur: bool) -> Scene {
    let mut scene = base();
    let red = material(&mut scene, "Red", [0.8, 0.1, 0.1], 0.5, 0.0);
    let slider = mesh(&mut scene, "Slider", Primitive::Cube { size: 1.0 }, Vec3::new(-3.0, -1.0, 0.5), red, false);
    key(&mut scene, slider, Property::Location, 1.0, Vec3::new(-3.0, -1.0, 0.5));
    key(&mut scene, slider, Property::Location, 11.0, Vec3::new(3.0, -1.0, 0.5));
    let blue = material(&mut scene, "Blue", [0.1, 0.2, 0.8], 0.5, 0.0);
    let spinner = mesh(&mut scene, "Spinner", Primitive::Cube { size: 1.4 }, Vec3::new(0.0, 2.5, 0.7), blue, false);
    key(&mut scene, spinner, Property::Rotation, 1.0, Vec3::ZERO);
    key(&mut scene, spinner, Property::Rotation, 11.0, Vec3::new(0.0, 0.0, std::f32::consts::PI));
    scene.render.motion_blur.enabled = blur;
    scene.render.motion_blur.shutter = 1.0;
    scene.go_to_frame(6.0);
    scene
}

fn decode(text: &str) -> Vec<f32> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bits = 0u32;
    let mut count = 0;
    let mut bytes = Vec::new();
    for byte in text.bytes().filter(|byte| *byte != b'=') {
        bits = (bits << 6) | ALPHABET.iter().position(|item| *item == byte).unwrap() as u32;
        count += 6;
        if count >= 8 {
            count -= 8;
            bytes.push((bits >> count) as u8);
        }
    }
    bytes.as_chunks::<4>().0.iter().map(|chunk| f32::from_le_bytes(*chunk)).collect()
}

fn reference(case: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(fixtures().join(format!("{case}.PASSES.CYCLES.json"))).unwrap()).unwrap()
}

/// A pass of ours as the reference's block means and centre values (`channels` of each pixel).
fn blocks(pass: &[[f32; 4]], channels: usize) -> (Vec<f32>, Vec<f32>) {
    let (columns, rows) = (WIDTH / BLOCK[0], HEIGHT / BLOCK[1]);
    let (mut means, mut centres) = (Vec::new(), Vec::new());
    for by in 0..rows {
        for bx in 0..columns {
            for channel in 0..channels {
                let mut total = 0.0;
                for y in by * BLOCK[1]..(by + 1) * BLOCK[1] {
                    for x in bx * BLOCK[0]..(bx + 1) * BLOCK[0] {
                        total += pass[(y * WIDTH + x) as usize][channel];
                    }
                }
                means.push(total / (BLOCK[0] * BLOCK[1]) as f32);
            }
            let (cx, cy) = (bx * BLOCK[0] + BLOCK[0] / 2, by * BLOCK[1] + BLOCK[1] / 2);
            for channel in 0..channels {
                centres.push(pass[(cy * WIDTH + cx) as usize][channel]);
            }
        }
    }
    (means, centres)
}

/// The largest difference of the block means, and the means' total over the reference's.
fn compare(ours: &[f32], theirs: &[f32]) -> (f32, f32) {
    let worst = ours.iter().zip(theirs).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    let (a, b): (f32, f32) = (ours.iter().map(|value| value.abs()).sum(), theirs.iter().map(|value| value.abs()).sum());
    (worst, if b > 1e-6 { a / b } else { 1.0 })
}

fn traced(scene: &Scene, samples: u32) -> BTreeMap<String, Vec<[f32; 4]>> {
    let mut tracer = Tracer::new(scene, TraceOptions { samples, passes: true, ..Default::default() }).unwrap();
    tracer.finish();
    tracer.passes().unwrap()
}

/// How far each light and geometric pass of `still` may be from the reference's block means
/// (largest block difference, and the total's share) at 128 samples against the reference's
/// 512: all matched in shape, the light split following the tracer's own surface model.
fn tolerance(pass: &str) -> (f32, f32) {
    match pass {
        "Normal" => (0.02, 0.005),
        "Ambient Occlusion" => (0.03, 0.005),
        "Environment" => (0.001, 0.01),
        "Emission" => (0.015, 0.005),
        "Diffuse Color" => (0.025, 0.025),
        "Diffuse Direct" => (0.22, 0.02),
        "Diffuse Indirect" => (0.045, 0.035),
        // The specular layer's colour is an estimate of the reference's (21% more over the
        // picture), and its light follows from it.
        "Glossy Color" => (0.12, 0.21),
        "Glossy Direct" => (0.37, 0.045),
        _ => (0.14, 0.045),
    }
}

#[test]
fn the_light_and_geometric_passes_match_the_reference() {
    let reference = reference("still");
    let scene = still();
    let mut ours = traced(&scene, 128);
    let shapes: Geometry = geometry(&scene).unwrap();
    ours.extend(shapes.passes.clone());
    let mut failures = Vec::new();
    for (name, channels) in [
        ("Diffuse Direct", 3),
        ("Diffuse Indirect", 3),
        ("Diffuse Color", 3),
        ("Glossy Direct", 3),
        ("Glossy Indirect", 3),
        ("Glossy Color", 3),
        ("Emission", 3),
        ("Environment", 3),
        ("Ambient Occlusion", 3),
        ("Normal", 3),
    ] {
        let theirs = decode(reference["passes"][format!("ViewLayer.{name}")]["means"].as_str().unwrap());
        let (means, _) = blocks(&ours[name], channels);
        let (worst, energy) = compare(&means, &theirs);
        let report = format!("{name}: worst block {worst:.4}, energy {energy:.3}");
        eprintln!("{report}");
        let (most, stray) = tolerance(name);
        if worst > most || (energy - 1.0).abs() > stray {
            failures.push(report);
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The Cryptomatte names hash to the reference's ids, and where one name covers a block's
/// centre pixel in both, it is the same name.
#[test]
fn cryptomatte_ids_match_the_reference() {
    let reference = reference("still");
    let shapes = geometry(&still()).unwrap();
    let manifests = reference["manifests"].as_object().unwrap();
    for manifest in manifests.values() {
        for (name, hex) in manifest.as_object().unwrap() {
            assert_eq!(kition_scene_compositor::cryptomatte::hex(name), hex.as_str().unwrap(), "{name}");
        }
    }
    for (layer, coverage) in [("CryptoObject00", &shapes.objects), ("CryptoMaterial00", &shapes.materials), ("CryptoAsset00", &shapes.assets)] {
        let theirs = decode(reference["passes"][format!("ViewLayer.{layer}")]["centres"].as_str().unwrap());
        let mut agreed = 0;
        for (block, values) in theirs.chunks(4).enumerate() {
            let (bx, by) = (block as u32 % (WIDTH / BLOCK[0]), block as u32 / (WIDTH / BLOCK[0]));
            let (cx, cy) = (bx * BLOCK[0] + BLOCK[0] / 2, by * BLOCK[1] + BLOCK[1] / 2);
            let pixel = &coverage[(cy * WIDTH + cx) as usize];
            let ours = pixel.iter().max_by(|a, b| a.1.total_cmp(b.1));
            match ours {
                Some((name, share)) if *share > 0.99 && values[1] > 0.99 => {
                    assert_eq!(kition_scene_compositor::cryptomatte::id(name).to_bits(), values[0].to_bits(), "{layer} block {block}: {name}");
                    agreed += 1;
                }
                None => assert!(values[1] < 0.01, "{layer} block {block}: the reference has something here"),
                _ => {}
            }
        }
        assert!(agreed > 40, "{layer}: only {agreed} blocks agree");
    }
}

/// The vector pass: where each block centre's surface was a frame before and will be a frame
/// after (pixels), as the reference's; with motion blur on, nothing, as the reference leaves it.
#[test]
fn the_vector_pass_matches_the_reference() {
    for (case, blur) in [("moving", false), ("moving-blur", true)] {
        let reference = reference(case);
        let theirs = decode(reference["passes"]["ViewLayer.Vector"]["centres"].as_str().unwrap());
        let shapes = geometry(&moving(blur)).unwrap();
        let (_, centres) = blocks(&shapes.passes["Vector"], 4);
        // Inside a moving object both agree to a few hundredths of a pixel (the turning cube's
        // Bézier-keyed speed changes between frames); at its edges the reference's pixel
        // filter mixes in a little of the object's motion where ours sees only the floor.
        let length = |values: &[f32]| values.iter().map(|value| value.abs()).fold(0.0f32, f32::max);
        let (mut inside, mut worst) = (0, 0.0f32);
        for (block, (ours, theirs)) in centres.chunks(4).zip(theirs.chunks(4)).enumerate() {
            let difference = ours.iter().zip(theirs).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
            eprintln!("  {block}: {ours:.2?} {theirs:.2?}");
            if length(ours) > 0.5 && length(theirs) > 0.5 {
                inside += 1;
                worst = worst.max(difference);
            } else {
                assert!(length(ours) < 1.0 && length(theirs) < 1.0, "{case} block {block}: {ours:?} {theirs:?}");
            }
        }
        eprintln!("{case}: {inside} blocks inside, worst {worst:.4}");
        assert!(worst < 0.08, "{case}: {worst}");
        assert_eq!(inside > 0, !blur, "{case}");
    }
}

#[test]
fn the_reference_gives_the_same_again() {
    let Some(executable) = std::env::var_os("KITION_BLENDER") else {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    };
    for case in ["still", "pick", "moving", "moving-blur"] {
        let out = std::env::temp_dir().join(format!("kition-scene-passes-{case}-{}.json", std::process::id()));
        let status = std::process::Command::new(&executable)
            .args(["-b", "--factory-startup", "--python"])
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/blender/passes.py"))
            .arg("--")
            .arg(case)
            .arg(&out)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        let again: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        let _ = std::fs::remove_file(&out);
        assert_eq!(again["passes"], reference(case)["passes"], "{case}");
    }
}

/// Shards' sums carry the light passes: merged, they give the single tracer's passes bit for bit.
#[test]
fn sharded_passes_merge_to_the_whole() {
    let mut scene = still();
    scene.render.width = 24;
    scene.render.height = 14;
    let options = TraceOptions { samples: 3, passes: true, ..Default::default() };
    let mut whole = Tracer::new(&scene, options).unwrap();
    whole.finish();
    let mut collector = Tracer::collector(&scene, options).unwrap();
    for index in 0..3 {
        let mut shard = Tracer::shard(&scene, options, index, 3).unwrap();
        shard.finish();
        collector.merge(&shard.sums()).unwrap();
    }
    assert_eq!(collector.passes(), whole.passes());
    // A collector without passes takes no sums with them.
    let plain = Tracer::collector(&scene, TraceOptions { passes: false, ..options }).unwrap();
    let mut shard = Tracer::shard(&scene, options, 0, 1).unwrap();
    shard.finish();
    assert!({ plain }.merge(&shard.sums()).is_err());
}

/// The compositor's Cryptomatte node on our passes: its mattes (the object Cube, the material
/// Metal, the asset Rig) against the reference's node on its own, and its Pick colours exactly
/// where one object covers a block's centre in both.
#[test]
fn the_cryptomatte_node_matches_the_reference() {
    use kition_scene_compositor::cryptomatte::{Layer, Layers as Crypto};
    use kition_scene_compositor::{Image, Layers, Passes, composite};
    use kition_scene_model::node_tree::{Link, Node, NodeGroup, Tree};

    let shapes = geometry(&still()).unwrap();
    let layers = Layers {
        passes: BTreeMap::new(),
        cryptomatte: Some(Crypto {
            object: Layer::from_coverage(&shapes.objects),
            material: Layer::from_coverage(&shapes.materials),
            asset: Layer::from_coverage(&shapes.assets),
        }),
    };
    let run = |layer: &str, matte: &str, output: &str| -> Vec<[f32; 4]> {
        let values = BTreeMap::from([("Layer".to_owned(), Value::from(layer)), ("Matte ID".to_owned(), Value::from(matte))]);
        let nodes = vec![
            Node { id: 1, kind: "cryptomatte".into(), location: [0.0, 0.0], values },
            Node { id: 2, kind: "compositorOutput".into(), location: [0.0, 0.0], values: BTreeMap::new() },
        ];
        let links = vec![Link { from: 1, from_socket: output.into(), to: 2, to_socket: "Image".into() }];
        let group = NodeGroup { id: 3, name: "Mattes".into(), tree: Tree::Compositor, inputs: Vec::new(), outputs: Vec::new(), nodes, links };
        let image = Image::filled(WIDTH as usize, HEIGHT as usize, [0.0, 0.0, 0.0, 1.0]);
        let images = |_: &str| None;
        let passes = Passes { image: &image, depth: None, images: &images, layers: Some(&layers), lens: None };
        composite(&group, &passes).unwrap().image.unwrap().pixels
    };
    let still = reference("still");
    let theirs = decode(still["passes"]["Composite.Combined"]["means"].as_str().unwrap());
    for (channel, (layer, matte)) in [("object", "Cube"), ("material", "Metal"), ("asset", "Rig")].into_iter().enumerate() {
        let (means, _) = blocks(&run(layer, matte, "Matte"), 1);
        let theirs: Vec<f32> = theirs.chunks(4).map(|pixel| pixel[channel]).collect();
        let (worst, energy) = compare(&means, &theirs);
        eprintln!("{layer} {matte}: worst block {worst:.4}, energy {energy:.3}");
        // The mattes' coverage at edges is our sixteen points against the reference's filter.
        assert!(worst < 0.02 && (energy - 1.0).abs() < 0.003, "{layer} {matte}: {worst} {energy}");
    }
    let pick = run("object", "", "Pick");
    let (_, ours) = blocks(&pick, 4);
    let theirs = decode(reference("pick")["passes"]["Composite.Combined"]["centres"].as_str().unwrap());
    let ids = decode(still["passes"]["ViewLayer.CryptoObject00"]["centres"].as_str().unwrap());
    let mut agreed = 0;
    for (block, (ours, theirs)) in ours.chunks(4).zip(theirs.chunks(4)).enumerate() {
        let (bx, by) = (block as u32 % (WIDTH / BLOCK[0]), block as u32 / (WIDTH / BLOCK[0]));
        let covered =
            shapes.objects[((by * BLOCK[1] + BLOCK[1] / 2) * WIDTH + bx * BLOCK[0] + BLOCK[0] / 2) as usize].values().any(|share| *share > 0.99);
        if covered && ids[block * 4 + 1] > 0.99 {
            assert_eq!(ours, theirs, "block {block}");
            agreed += 1;
        }
    }
    assert!(agreed > 40, "only {agreed} blocks agree");
}
