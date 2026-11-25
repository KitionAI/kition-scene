//! Compositing through commands: a compositing group renders pass through, its nodes edited with
//! `node.*`, kept apart from geometry groups, saved, undone, and previewed.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn starter() -> SceneDocument {
    let mut made = SceneDocument::create("Untitled.kscene", 1280, 720).unwrap();
    let mut doc = SceneDocument::open("t.kscene", &made.save_file(None).unwrap()).unwrap();
    run(&mut doc, "render.set", json!({ "width": 96, "height": 54 }));
    doc
}

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn unbase64(text: &str) -> Vec<u8> {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let (mut bits, mut count, mut out) = (0u32, 0, Vec::new());
    for byte in text.bytes().filter(|byte| *byte != b'=') {
        bits = (bits << 6) | TABLE.iter().position(|item| *item == byte).unwrap() as u32;
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
        }
    }
    out
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let triple = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        for index in 0..4 {
            out.push(if index <= chunk.len() { TABLE[((triple >> (18 - 6 * index)) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// Width, height, and RGBA8 of a PNG.
fn pixels(png: &[u8]) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut buffer).unwrap();
    buffer.truncate(frame.buffer_size());
    (frame.width, frame.height, buffer)
}

fn render(doc: &mut SceneDocument) -> (u32, u32, Vec<u8>) {
    let answer = run(doc, "render.image", json!({ "samples": 1 }));
    pixels(&unbase64(answer["base64"].as_str().unwrap()))
}

fn mean(rgba: &[u8]) -> f64 {
    let sum: u64 = rgba.chunks(4).map(|pixel| u64::from(pixel[0]) + u64::from(pixel[1]) + u64::from(pixel[2])).sum();
    sum as f64 / (rgba.len() / 4 * 3) as f64
}

/// A compositing group with an Exposure node between Render Layers and the output.
fn with_exposure(doc: &mut SceneDocument, stops: f64) -> (u64, u64) {
    let made = run(doc, "compositor.new", json!({}));
    let group = made["group"].as_u64().unwrap();
    let exposure = run(doc, "node.addNode", json!({ "group": group, "kind": "exposure", "values": { "Exposure": stops } }))["node"].as_u64().unwrap();
    run(doc, "node.link", json!({ "group": group, "from": made["renderLayers"], "fromSocket": "Image", "to": exposure, "toSocket": "Image" }));
    run(doc, "node.link", json!({ "group": group, "from": exposure, "fromSocket": "Image", "to": made["output"], "toSocket": "Image" }));
    (group, exposure)
}

#[test]
fn renders_pass_through_the_compositing_group_while_compositing_is_on() {
    let mut doc = starter();
    let plain = render(&mut doc);
    let (group, _) = with_exposure(&mut doc, 1.0);
    let inspected = doc.inspect().unwrap();
    assert_eq!(inspected["render"]["compositor"], group);
    let brighter = render(&mut doc);
    assert_eq!((brighter.0, brighter.1), (96, 54));
    assert!(mean(&brighter.2) > mean(&plain.2) + 10.0, "{} against {}", mean(&brighter.2), mean(&plain.2));
    run(&mut doc, "compositor.set", json!({ "compositing": false }));
    assert_eq!(render(&mut doc).2, plain.2);
    run(&mut doc, "compositor.set", json!({ "compositing": true }));
    run(&mut doc, "compositor.set", json!({ "group": null }));
    assert_eq!(render(&mut doc).2, plain.2);
}

#[test]
fn a_pass_through_group_changes_nothing_but_light_past_white() {
    let mut doc = starter();
    let plain = render(&mut doc);
    run(&mut doc, "compositor.new", json!({}));
    let passed = render(&mut doc);
    let worst = plain.2.iter().zip(&passed.2).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
    assert!(worst <= 1, "{worst}");
}

#[test]
fn compositing_and_geometry_groups_keep_to_their_own_nodes() {
    let mut doc = starter();
    let group = run(&mut doc, "compositor.new", json!({}))["group"].clone();
    assert!(doc.execute("node.addNode", json!({ "group": group, "kind": "meshCube" })).is_err());
    assert!(doc.execute("node.addInput", json!({ "group": group, "name": "Size", "type": "float" })).is_err());
    let geometry = run(&mut doc, "node.newGroup", json!({ "object": "Cube" }))["group"].clone();
    assert!(doc.execute("node.addNode", json!({ "group": geometry, "kind": "glare" })).is_err());
    // A Geometry Nodes modifier cannot run a compositing group, nor renders a geometry group.
    assert!(doc.execute("modifier.set", json!({ "object": "Cube", "index": 0, "group": group })).is_err());
    assert!(doc.execute("compositor.set", json!({ "group": geometry })).is_err());
    let described = run(&mut doc, "node.describe", json!({ "tree": "compositor" }));
    let kinds: Vec<&str> = described["kinds"].as_array().unwrap().iter().map(|kind| kind["id"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"glare") && kinds.contains(&"value") && !kinds.contains(&"meshCube"), "{kinds:?}");
    // A colour takes three or four channels, none below zero; curves take sorted points.
    let rgb = run(&mut doc, "node.addNode", json!({ "group": group, "kind": "rgb" }))["node"].clone();
    run(&mut doc, "node.setValue", json!({ "group": group, "node": rgb, "id": "Color", "value": [1, 0.5, 0] }));
    assert!(doc.execute("node.setValue", json!({ "group": group, "node": rgb, "id": "Color", "value": [-1, 0, 0] })).is_err());
    let curves = run(&mut doc, "node.addNode", json!({ "group": group, "kind": "rgbCurves" }))["node"].clone();
    run(&mut doc, "node.setValue", json!({ "group": group, "node": curves, "id": "curves", "value": { "c": [[1, 1], [0, 0], [0.4, 0.6]] } }));
    assert!(doc.execute("node.setValue", json!({ "group": group, "node": curves, "id": "curves", "value": { "c": [[0, 0]] } })).is_err());
    assert!(doc.execute("node.link", json!({ "group": group, "from": rgb, "fromSocket": "Color", "to": curves, "toSocket": "curves" })).is_err());
    let inspected = doc.inspect().unwrap();
    let group_json = inspected["nodeGroups"].as_array().unwrap().iter().find(|item| item["id"] == group).unwrap();
    assert_eq!(group_json["tree"], "compositor");
    let node = group_json["nodes"].as_array().unwrap().iter().find(|item| item["id"] == curves).unwrap();
    assert_eq!(node["values"]["curves"]["c"], json!([[0.0, 0.0], [0.4, 0.6], [1.0, 1.0]]));
    let node = group_json["nodes"].as_array().unwrap().iter().find(|item| item["id"] == rgb).unwrap();
    assert_eq!(node["values"]["Color"], json!([1, 0.5, 0, 1.0]));
}

#[test]
fn compositing_is_saved_undone_and_removed_with_its_group() {
    let mut doc = starter();
    let (group, exposure) = with_exposure(&mut doc, -1.0);
    let reopened = SceneDocument::open("t.kscene", &doc.save_file(None).unwrap()).unwrap();
    let inspected = reopened.inspect().unwrap();
    assert_eq!(inspected["render"]["compositor"], group);
    let saved = inspected["nodeGroups"].as_array().unwrap().iter().find(|item| item["id"] == group).unwrap();
    assert_eq!(saved["tree"], "compositor");
    assert!(saved["nodes"].as_array().unwrap().iter().any(|node| node["id"] == exposure && node["values"]["Exposure"] == -1.0));
    run(&mut doc, "node.removeGroup", json!({ "group": group }));
    assert_eq!(doc.inspect().unwrap()["render"]["compositor"], Value::Null);
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(doc.inspect().unwrap()["render"]["compositor"], group);
    for _ in 0..4 {
        run(&mut doc, "edit.undo", json!({}));
    }
    assert!(doc.inspect().unwrap()["nodeGroups"].as_array().unwrap().is_empty());
}

/// A 2 × 1 PNG: one red pixel, one clear one.
fn red_and_clear() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[255, 0, 0, 255, 0, 0, 0, 0]).unwrap();
    }
    out
}

#[test]
fn an_image_node_lays_a_picture_over_the_render_and_keeps_it_in_the_file() {
    let mut doc = starter();
    let made = run(&mut doc, "compositor.new", json!({}));
    let group = made["group"].clone();
    let image = run(&mut doc, "node.addNode", json!({ "group": group, "kind": "image" }))["node"].clone();
    let set = run(&mut doc, "compositor.setImage", json!({ "node": image, "base64": base64(&red_and_clear()), "name": "Logo.png" }));
    let hash = set["image"].as_str().unwrap().to_owned();
    assert!(doc.execute("node.setValue", json!({ "group": group, "node": image, "id": "Image", "value": "f00d" })).is_err());
    let over = run(&mut doc, "node.addNode", json!({ "group": group, "kind": "alphaOver" }))["node"].clone();
    run(&mut doc, "node.link", json!({ "group": group, "from": made["renderLayers"], "fromSocket": "Image", "to": over, "toSocket": "Background" }));
    run(&mut doc, "node.link", json!({ "group": group, "from": image, "fromSocket": "Image", "to": over, "toSocket": "Foreground" }));
    run(&mut doc, "node.link", json!({ "group": group, "from": over, "fromSocket": "Image", "to": made["output"], "toSocket": "Image" }));
    let (width, height, rgba) = render(&mut doc);
    // The picture sits at the middle at its own size: its red pixel left of the centre.
    let at = |x: u32, y: u32| &rgba[((y * width + x) * 4) as usize..((y * width + x) * 4 + 4) as usize];
    let middle = (height - 1) / 2;
    assert_eq!(at(width / 2 - 1, middle), [255, 0, 0, 255]);
    assert_ne!(at(width / 2, middle)[..3], [255, 0, 0]);
    let reopened = SceneDocument::open("t.kscene", &doc.save_file(None).unwrap()).unwrap();
    assert!(reopened.inspect().unwrap()["images"].as_array().unwrap().iter().any(|item| item["hash"] == hash.as_str()));
}

#[test]
fn a_preview_is_small_and_can_show_the_viewer() {
    let mut doc = starter();
    run(&mut doc, "render.set", json!({ "width": 1920, "height": 1080 }));
    let made = run(&mut doc, "compositor.new", json!({}));
    let viewer = run(&mut doc, "node.addNode", json!({ "group": made["group"], "kind": "viewer" }))["node"].clone();
    let invert = run(&mut doc, "node.addNode", json!({ "group": made["group"], "kind": "invert" }))["node"].clone();
    run(
        &mut doc,
        "node.link",
        json!({ "group": made["group"], "from": made["renderLayers"], "fromSocket": "Image", "to": invert, "toSocket": "Color" }),
    );
    run(&mut doc, "node.link", json!({ "group": made["group"], "from": invert, "fromSocket": "Color", "to": viewer, "toSocket": "Image" }));
    let output = run(&mut doc, "compositor.preview", json!({ "size": 64 }));
    assert_eq!((output["width"].as_u64(), output["height"].as_u64()), (Some(64), Some(36)));
    let shown = run(&mut doc, "compositor.preview", json!({ "size": 64, "viewer": true }));
    let (plain, inverted) = (pixels(&unbase64(output["base64"].as_str().unwrap())).2, pixels(&unbase64(shown["base64"].as_str().unwrap())).2);
    // Inverted in linear light, the mostly dark render comes out mostly light.
    assert!(mean(&inverted) > mean(&plain) + 100.0, "{} and {}", mean(&plain), mean(&inverted));
}

#[test]
fn a_path_traced_render_comes_out_composited_once_done() {
    let mut doc = starter();
    run(&mut doc, "render.set", json!({ "width": 32, "height": 18 }));
    let plain = run(&mut doc, "render.trace.start", json!({ "samples": 2, "wait": true }));
    with_exposure(&mut doc, 2.0);
    let brighter = run(&mut doc, "render.trace.start", json!({ "samples": 2, "wait": true }));
    let (plain, brighter) = (pixels(&unbase64(plain["base64"].as_str().unwrap())).2, pixels(&unbase64(brighter["base64"].as_str().unwrap())).2);
    assert!(mean(&brighter) > mean(&plain) + 10.0, "{} against {}", mean(&brighter), mean(&plain));
}

/// A Cryptomatte node picks the cube from a point of the render and keeps only it; one undo
/// step takes the pick away.
#[test]
fn a_cryptomatte_node_picks_a_matte_from_the_render() {
    let mut doc = starter();
    let made = run(&mut doc, "compositor.new", json!({}));
    let group = made["group"].clone();
    let crypto = run(&mut doc, "node.addNode", json!({ "group": group, "kind": "cryptomatte" }))["node"].clone();
    run(&mut doc, "node.link", json!({ "group": group, "from": made["renderLayers"], "fromSocket": "Image", "to": crypto, "toSocket": "Image" }));
    run(&mut doc, "node.link", json!({ "group": group, "from": crypto, "fromSocket": "Image", "to": made["output"], "toSocket": "Image" }));
    let picked = run(&mut doc, "compositor.pickMatte", json!({ "node": crypto, "x": 0.5, "y": 0.5 }));
    assert_eq!(picked["name"], "Cube");
    assert_eq!(picked["matte"], "Cube");
    let (width, height, rgba) = render(&mut doc);
    let at = |x: u32, y: u32| &rgba[((y * width + x) * 4) as usize..((y * width + x) * 4 + 4) as usize];
    assert!(at(width / 2, height / 2)[3] > 250, "the cube is kept: {:?}", at(width / 2, height / 2));
    assert_eq!(at(0, 0)[3], 0, "the world is keyed out");
    // Picking it again with remove takes it away; nothing at all is there past the scene.
    let removed = run(&mut doc, "compositor.pickMatte", json!({ "node": crypto, "x": 0.5, "y": 0.5, "remove": true }));
    assert_eq!(removed["matte"], "");
    run(&mut doc, "edit.undo", json!({}));
    let groups = doc.inspect();
    assert!(serde_json::to_string(&groups).unwrap().contains("\"Matte ID\":\"Cube\""));
    assert!(doc.execute("compositor.pickMatte", json!({ "node": crypto, "x": 0.0, "y": 0.0 })).is_err());
}

/// A path-traced render gives the compositor its light passes: the diffuse colour pass shows
/// the cube's grey where it is.
#[test]
fn a_path_traced_render_gives_its_light_passes() {
    let mut doc = starter();
    run(&mut doc, "render.set", json!({ "width": 32, "height": 18 }));
    let made = run(&mut doc, "compositor.new", json!({}));
    run(
        &mut doc,
        "node.link",
        json!({ "group": made["group"], "from": made["renderLayers"], "fromSocket": "Diffuse Color", "to": made["output"], "toSocket": "Image" }),
    );
    let answer = run(&mut doc, "render.trace.start", json!({ "samples": 2, "wait": true }));
    let (width, height, rgba) = pixels(&unbase64(answer["base64"].as_str().unwrap()));
    let centre = &rgba[((height / 2 * width + width / 2) * 4) as usize..][..3];
    // The default material's base colour 0.8, less what its specular layer reflects.
    assert!(centre.iter().all(|value| (190..=235).contains(value)), "{centre:?}");
    assert!(rgba[..3].iter().all(|value| *value < 5), "nothing diffuse in the world: {:?}", &rgba[..3]);
}
