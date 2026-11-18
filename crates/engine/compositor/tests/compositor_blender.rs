//! Compositing nodes against the reference application. `fixtures/compositor-cases.json` lists
//! the cases (one node, its settings, its inputs); `fixtures/compositor-reference.json` holds
//! what Blender 5.2.3 made of each on a grid of pixels (`tests/blender/compositor.py` writes it;
//! with `KITION_BLENDER` naming a Blender executable the script runs again and must give the
//! same). The two input pictures are made here by the same formulas.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::collections::BTreeMap;

use kition_scene_compositor::{Image, Passes, composite};
use kition_scene_model::node_tree::{Link, Node, NodeGroup, Tree};
use serde_json::{Value, json};

const CASES: &str = include_str!("fixtures/compositor-cases.json");
const REFERENCE: &str = include_str!("fixtures/compositor-reference.json");

fn spec() -> Value {
    serde_json::from_str(CASES).unwrap()
}

fn picture(width: usize, height: usize, f: impl Fn(usize, usize) -> [f64; 4]) -> Image {
    let mut pixels = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            pixels.push(f(x, y).map(|value| value as f32));
        }
    }
    Image { width, height, pixels }
}

/// A: a colour ramp with a hot spot and a one-pixel spike.
fn picture_a(width: usize, height: usize) -> Image {
    picture(width, height, |x, y| {
        let (u, v) = ((x as f64 + 0.5) / width as f64, (y as f64 + 0.5) / height as f64);
        let (dx, dy) = (x as f64 - 30.0, y as f64 - 12.0);
        if (x, y) == (10, 20) {
            [12.0, 12.0, 12.0, 1.0]
        } else if dx * dx + dy * dy < 9.0 {
            [6.0, 5.0, 4.0, 1.0]
        } else {
            [1.2 * u, v, 0.25 + 0.5 * u * v, 1.0]
        }
    })
}

/// B: a ramp with alpha, premultiplied.
fn picture_b(width: usize, height: usize) -> Image {
    picture(width, height, |x, y| {
        let (u, v) = ((x as f64 + 0.5) / width as f64, (y as f64 + 0.5) / height as f64);
        let a = (1.5 * v - 0.25).clamp(0.0, 1.0);
        [0.9 * (1.0 - u) * a, (0.3 + 0.4 * v) * a, 0.8 * u * a, a]
    })
}

/// Runs one case: the inputs as Image nodes, the node, the group's output.
fn run(case: &Value, a: &Image, b: &Image) -> Image {
    let mut nodes = Vec::new();
    let mut links = Vec::new();
    let values: BTreeMap<String, Value> = case["values"].as_object().unwrap().iter().map(|(key, value)| (key.clone(), value.clone())).collect();
    nodes.push(Node { id: 1, kind: case["kind"].as_str().unwrap().to_owned(), location: [0.0, 0.0], values });
    nodes.push(Node { id: 2, kind: "compositorOutput".into(), location: [0.0, 0.0], values: BTreeMap::new() });
    links.push(Link { from: 1, from_socket: case["output"].as_str().unwrap().to_owned(), to: 2, to_socket: "Image".into() });
    for (index, (socket, source)) in case["inputs"].as_object().unwrap().iter().enumerate() {
        let (name, output) = source.as_str().unwrap().split_once('.').unwrap_or((source.as_str().unwrap(), "Image"));
        let id = 10 + index as u32;
        nodes.push(Node { id, kind: "image".into(), location: [0.0, 0.0], values: BTreeMap::from([("Image".to_owned(), json!(name))]) });
        links.push(Link { from: id, from_socket: output.to_owned(), to: 1, to_socket: socket.clone() });
    }
    let group = NodeGroup { id: 100, name: "Case".into(), tree: Tree::Compositor, inputs: Vec::new(), outputs: Vec::new(), nodes, links };
    group.validate(&[]).unwrap();
    let images = |name: &str| match name {
        "A" => Some(a.clone()),
        "B" => Some(b.clone()),
        _ => None,
    };
    let passes = Passes { image: a, depth: None, images: &images, layers: None, lens: None };
    composite(&group, &passes).unwrap().image.unwrap()
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

/// How a case compares: the largest difference of any channel on the grid, the mean
/// difference, and the energy of ours over theirs (per colour channel, summed over the grid).
struct Compared {
    worst: f32,
    mean: f32,
    energy: [f32; 3],
}

fn compare(spec: &Value, case: &Value, ours: &Image, theirs: &[f32]) -> Compared {
    let columns: Vec<usize> = spec["columns"].as_array().unwrap().iter().map(|value| value.as_u64().unwrap() as usize).collect();
    let rows: Vec<usize> = spec["rows"].as_array().unwrap().iter().map(|value| value.as_u64().unwrap() as usize).collect();
    let mut worst = 0.0f32;
    let mut total = 0.0f32;
    let mut energy = [[0.0f32; 2]; 3];
    let mut index = 0;
    for y in &rows {
        for x in &columns {
            let mine = ours.pixels[y * ours.width + x];
            for channel in 0..4 {
                let difference = (mine[channel] - theirs[index * 4 + channel]).abs();
                worst = worst.max(difference);
                total += difference;
                if channel < 3 {
                    energy[channel][0] += mine[channel];
                    energy[channel][1] += theirs[index * 4 + channel];
                }
            }
            index += 1;
        }
    }
    let _ = case;
    Compared { worst, mean: total / (index * 4) as f32, energy: energy.map(|[ours, theirs]| if theirs.abs() > 1e-6 { ours / theirs } else { 1.0 }) }
}

/// The largest difference each case may have, and how far its energy may stray (as a share):
/// exact (float rounding) unless listed. RGB Curves differ by the reference's half-float curve
/// table; Lens Distortion by its sampling (and roughly, with dispersion and distortion
/// together); glare spreads light by measured shapes, so it is held to its energy and to bounds
/// well under its peaks (a spike of 11 above the threshold).
fn tolerance(name: &str) -> (f32, f32) {
    match name {
        "curves-combined" | "curves-channels" | "curves-levels" => (1e-3, 1e-3),
        "lens-barrel" | "lens-pincushion-fit" | "lens-barrel-fit" => (0.01, 1e-3),
        "lens-dispersion" | "lens-both-fit" => (0.03, 1e-3),
        "lens-both" => (0.25, 0.01),
        "glare-bloom" | "glare-bloom-image" | "glare-bloom-size" => (0.08, 0.05),
        "glare-bloom-tint" => (0.04, 0.05),
        "glare-fog-glow" | "glare-fog-glow-size" => (0.002, 1e-3),
        "glare-highlights-medium" => (0.26, 0.05),
        "glare-simple-star" | "glare-simple-star-straight" => (0.07, 0.03),
        "glare-streaks" => (0.2, 0.06),
        "glare-streaks-angle" => (0.55, 0.08),
        "glare-ghosts" => (0.35, 0.15),
        "glare-sun-beams" => (1.35, 0.06),
        _ => (2e-4, 1e-4),
    }
}

#[test]
fn compositing_nodes_match_the_reference() {
    let spec = spec();
    let reference: Value = serde_json::from_str(REFERENCE).unwrap();
    let (width, height) = (spec["width"].as_u64().unwrap() as usize, spec["height"].as_u64().unwrap() as usize);
    let (a, b) = (picture_a(width, height), picture_b(width, height));
    let mut failures = Vec::new();
    for case in spec["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let ours = run(case, &a, &b);
        let theirs = decode(reference["results"][name].as_str().unwrap());
        let compared = compare(&spec, case, &ours, &theirs);
        if std::env::var("COMPOSITOR_DEBUG").is_ok_and(|wanted| wanted == name) {
            let columns = spec["columns"].as_array().unwrap().len();
            for (row, y) in spec["rows"].as_array().unwrap().iter().enumerate() {
                let y = y.as_u64().unwrap() as usize;
                let line: Vec<String> = spec["columns"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                    .map(|(column, x)| {
                        let x = x.as_u64().unwrap() as usize;
                        format!("{:.3}/{:.3}", ours.pixels[y * width + x][1], theirs[(row * columns + column) * 4 + 1])
                    })
                    .collect();
                eprintln!("y {y:2}: {}", line.join(" "));
            }
        }
        let report = format!("{name}: worst {:.5}, mean {:.5}, energy {:.3?}", compared.worst, compared.mean, compared.energy);
        eprintln!("{report}");
        let (worst, energy) = tolerance(name);
        if compared.worst > worst || compared.energy.iter().any(|ratio| (ratio - 1.0).abs() > energy) {
            failures.push(report);
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn the_reference_gives_the_same_again() {
    let Some(executable) = std::env::var_os("KITION_BLENDER") else {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    };
    let out = std::env::temp_dir().join(format!("kition-scene-compositor-{}.json", std::process::id()));
    let status = std::process::Command::new(executable)
        .args(["-b", "--factory-startup", "--python"])
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/blender/compositor.py"))
        .arg("--")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/compositor-cases.json"))
        .arg(&out)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let again: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    let _ = std::fs::remove_file(&out);
    let reference: Value = serde_json::from_str(REFERENCE).unwrap();
    for (name, value) in reference["results"].as_object().unwrap() {
        let (old, new) = (decode(value.as_str().unwrap()), decode(again["results"][name].as_str().unwrap()));
        let worst = old.iter().zip(&new).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(worst < 1e-4, "{name} changed by {worst}");
    }
}
