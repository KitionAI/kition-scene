//! The transform, keying, filter, and blur nodes against the reference application.
//! `fixtures/nodes-cases.json` lists the cases (one node, its settings, its inputs);
//! `fixtures/nodes-reference.json` holds what Blender 5.2.3 made of each on a grid of pixels
//! (`tests/blender/nodes.py` writes it; with `KITION_BLENDER` naming a Blender executable the
//! script runs again and must give the same). The input pictures are made here by the same
//! formulas as in the script.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::collections::BTreeMap;

use kition_scene_compositor::{Image, Lens, Passes, composite};
use kition_scene_model::node_tree::{Link, Node, NodeGroup, Tree};
use serde_json::{Value, json};

const CASES: &str = include_str!("fixtures/nodes-cases.json");
const REFERENCE: &str = include_str!("fixtures/nodes-reference.json");

/// The camera the script gives the reference's scene (Defocus reads it with its Z buffer).
const LENS: Lens = Lens { focal_length: 50.0, sensor_width: 36.0, focus_distance: 4.0 };

fn picture(width: usize, height: usize, f: impl Fn(usize, usize) -> [f64; 4]) -> Image {
    let mut pixels = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            pixels.push(f(x, y).map(|value| value as f32));
        }
    }
    Image { width, height, pixels }
}

fn in_disc(x: usize, y: usize) -> bool {
    let (dx, dy) = (x as f64 - 24.0, y as f64 - 16.0);
    dx * dx + dy * dy < 49.0
}

/// The pictures by name, `width × height` (the kernel K is 9 × 9).
fn pictures(width: usize, height: usize) -> BTreeMap<&'static str, Image> {
    let (w, h) = (width as f64, height as f64);
    let uv = move |x: usize, y: usize| ((x as f64 + 0.5) / w, (y as f64 + 0.5) / h);
    let mut out = BTreeMap::new();
    // A: a colour ramp with a hot spot and a one-pixel spike.
    out.insert(
        "A",
        picture(width, height, |x, y| {
            let (u, v) = uv(x, y);
            let (dx, dy) = (x as f64 - 30.0, y as f64 - 12.0);
            if (x, y) == (10, 20) {
                [12.0, 12.0, 12.0, 1.0]
            } else if dx * dx + dy * dy < 9.0 {
                [6.0, 5.0, 4.0, 1.0]
            } else {
                [1.2 * u, v, 0.25 + 0.5 * u * v, 1.0]
            }
        }),
    );
    // B: a ramp with alpha, premultiplied.
    out.insert(
        "B",
        picture(width, height, |x, y| {
            let (u, v) = uv(x, y);
            let a = (1.5 * v - 0.25).clamp(0.0, 1.0);
            [0.9 * (1.0 - u) * a, (0.3 + 0.4 * v) * a, 0.8 * u * a, a]
        }),
    );
    // G: a green screen with a soft-edged subject and a spill-lit patch.
    out.insert(
        "G",
        picture(width, height, |x, y| {
            let (u, v) = uv(x, y);
            let back = [0.1 + 0.05 * u, 0.75 - 0.1 * v, 0.12];
            let mut color = back;
            let d = (x as f64 - 18.0).hypot(y as f64 - 16.0);
            if d < 9.0 {
                let t = ((9.0 - d) / 2.0).min(1.0);
                let subject = [0.85, 0.45, 0.35];
                for ((value, back), subject) in color.iter_mut().zip(back).zip(subject) {
                    *value = back + (subject - back) * t;
                }
            }
            if (32..=42).contains(&x) && (6..=26).contains(&y) {
                color = [0.4 * (0.6 + 0.4 * v), 0.6, 0.35 * (0.6 + 0.4 * v)];
            }
            [color[0], color[1], color[2], 1.0]
        }),
    );
    // Z: a depth ramp with a near square.
    out.insert(
        "Z",
        picture(width, height, |x, y| {
            let (u, _) = uv(x, y);
            let z = if (10..=18).contains(&x) && (8..=16).contains(&y) { 0.6 } else { 1.0 + 9.0 * u };
            [z, z, z, 1.0]
        }),
    );
    // M: a bright disc on a dark ramp; V: the disc's speed (to the previous and the next
    // frame); D: its depth.
    out.insert(
        "M",
        picture(width, height, |x, y| {
            let (u, v) = uv(x, y);
            if in_disc(x, y) { [1.0, 0.6, 0.2, 1.0] } else { [0.1 * u, 0.05 + 0.1 * v, 0.1, 1.0] }
        }),
    );
    out.insert("V", picture(width, height, |x, y| if in_disc(x, y) { [-4.0, 1.0, 4.0, -1.0] } else { [0.0; 4] }));
    out.insert(
        "D",
        picture(width, height, |x, y| {
            let z = if in_disc(x, y) { 2.0 } else { 10.0 };
            [z, z, z, 1.0]
        }),
    );
    // N: integer hash noise.
    out.insert(
        "N",
        picture(width, height, |x, y| {
            let channel = |c: u64| (((x as u64 * 73_856_093) ^ (y as u64 * 19_349_663) ^ (c * 83_492_791)) % 1000) as f64 / 1000.0;
            [channel(0), channel(1), channel(2), 1.0]
        }),
    );
    // Q and P: colour grids.
    out.insert("Q", picture(width, height, |x, y| [0.5 * x as f64 / 47.0, y as f64 / 31.0, 0.0, 1.0]));
    out.insert("P", picture(width, height, |x, y| [x as f64 / 47.0, 0.6, y as f64 / 31.0, 1.0]));
    // K: a 9 × 9 kernel, a cross fading from its centre.
    out.insert(
        "K",
        picture(9, 9, |x, y| {
            if (x, y) == (4, 4) {
                [1.0, 1.0, 1.0, 1.0]
            } else if x == 4 || y == 4 {
                let f = 0.5f64.powi((x as i32 - 4 + y as i32 - 4).abs());
                [f, 0.5 * f, 0.25 * f, 1.0]
            } else {
                [0.0, 0.0, 0.0, 1.0]
            }
        }),
    );
    out
}

/// Runs one case: the inputs as Image nodes, the node, the group's output.
fn run(case: &Value, pictures: &BTreeMap<&'static str, Image>, width: usize, height: usize) -> Image {
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
    let images = |name: &str| pictures.get(name).cloned();
    let base = Image::filled(width, height, [0.0, 0.0, 0.0, 1.0]);
    let passes = Passes { image: &base, depth: None, images: &images, layers: None, lens: Some(LENS) };
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

/// The largest difference of any channel on the grid, the mean difference, and the energy of
/// ours over theirs (per colour channel, summed over the grid).
fn compare(spec: &Value, ours: &Image, theirs: &[f32]) -> (f32, f32, [f32; 3]) {
    let grid = |key: &str| -> Vec<usize> { spec[key].as_array().unwrap().iter().map(|value| value.as_u64().unwrap() as usize).collect() };
    let (columns, rows) = (grid("columns"), grid("rows"));
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
    (worst, total / (index * 4) as f32, energy.map(|[ours, theirs]| if theirs.abs() > 1e-6 { ours / theirs } else { 1.0 }))
}

/// The largest difference each case may have and how far its energy may stray (as a share):
/// exact (float rounding) unless listed; see each entry's reason.
fn tolerance(name: &str) -> (f32, f32) {
    match name {
        // The pre-blur goes through Y′CbCr and back; the reference's way back differs a little.
        "keying-pre-blur" => (1e-3, 2e-3),
        // Matched in shape only: the feather's falloff (a separable spread) and the anisotropic
        // filter (from its paper, not fitted) differ in detail.
        "keying-feather" | "keying-feather-linear" => (0.3, 0.1),
        "kuwahara-anisotropic" | "kuwahara-anisotropic-soft" => (0.11, 0.01),
        "kuwahara-anisotropic-noise" => (0.6, 0.02),
        // Defocus: the blur sizes and the hard-edged bokeh match; how neighbours of other blur
        // sizes are weighted (the ramp, the hexagon over a varying Z) differs near the hot spot.
        "defocus-z-scale" | "defocus-z-buffer-near" | "defocus-z-buffer-far" => (0.04, 2e-3),
        "defocus-z-scale-wide" | "defocus-square" => (0.08, 3e-3),
        "defocus-z-buffer-ramp" | "defocus-hexagon" => (2.2, 0.015),
        // Vector Blur: the smear's length and direction match, its falloff only in shape.
        "vector-blur" | "vector-blur-shutter" => (0.75, 0.1),
        _ => (2e-4, 1e-4),
    }
}

#[test]
fn the_nodes_match_the_reference() {
    let spec: Value = serde_json::from_str(CASES).unwrap();
    let reference: Value = serde_json::from_str(REFERENCE).unwrap();
    let (width, height) = (spec["width"].as_u64().unwrap() as usize, spec["height"].as_u64().unwrap() as usize);
    let pictures = pictures(width, height);
    let mut failures = Vec::new();
    for case in spec["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let ours = run(case, &pictures, width, height);
        let theirs = decode(reference["results"][name].as_str().unwrap());
        let (worst, mean, energy) = compare(&spec, &ours, &theirs);
        if std::env::var("COMPOSITOR_DEBUG").is_ok_and(|wanted| wanted == name) {
            let channel: usize = std::env::var("COMPOSITOR_CHANNEL").ok().and_then(|value| value.parse().ok()).unwrap_or(0);
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
                        format!("{:.3}/{:.3}", ours.pixels[y * width + x][channel], theirs[(row * columns + column) * 4 + channel])
                    })
                    .collect();
                eprintln!("y {y:2}: {}", line.join(" "));
            }
        }
        let report = format!("{name}: worst {worst:.5}, mean {mean:.5}, energy {energy:.3?}");
        eprintln!("{report}");
        let (most, stray) = tolerance(name);
        if worst > most || energy.iter().any(|ratio| (ratio - 1.0).abs() > stray) {
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
    let out = std::env::temp_dir().join(format!("kition-scene-compositor-nodes-{}.json", std::process::id()));
    let status = std::process::Command::new(executable)
        .args(["-b", "--factory-startup", "--python"])
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/blender/nodes.py"))
        .arg("--")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/nodes-cases.json"))
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
