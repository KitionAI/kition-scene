//! Texture coordinate and image commands.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn starter() -> SceneDocument {
    let mut doc = SceneDocument::create("Untitled.kscene", 320, 200).unwrap();
    doc.reopen("t.kscene").unwrap()
}

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
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

fn red_png() -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, 2, 2);
    encoder.set_color(png::ColorType::Rgba);
    encoder.write_header().unwrap().write_image_data(&[255, 0, 0, 255].repeat(4)).unwrap();
    out
}

#[test]
fn projections_run_on_the_selection() {
    let mut doc = starter();
    assert_eq!(doc.inspect().unwrap()["objects"][0]["mesh"]["uvs"], true, "a new cube has the reference layout");
    run(&mut doc, "uv.clear", json!({}));
    assert_eq!(doc.inspect().unwrap()["objects"][0]["mesh"]["uvs"], false);
    run(&mut doc, "uv.smartProject", json!({ "angleLimit": 45 }));
    assert_eq!(doc.inspect().unwrap()["objects"][0]["mesh"]["uvs"], true);
    run(&mut doc, "mode.set", json!({ "mode": "edit" }));
    run(&mut doc, "mesh.select", json!({ "faces": [0] }));
    let changed = run(&mut doc, "uv.planarProject", json!({ "axis": "x" }));
    assert_eq!(changed["changed"][0]["faces"], 1);
    for command in ["uv.cubeProject", "uv.cylinderProject", "uv.sphereProject", "uv.reset"] {
        run(&mut doc, command, json!({}));
    }
    assert!(doc.execute("uv.planarProject", json!({ "axis": "w" })).is_err());
    assert!(doc.execute("uv.smartProject", json!({ "angleLimit": 500 })).is_err());
    // Edits keep coordinates.
    run(&mut doc, "mesh.extrude", json!({ "distance": 0.5 }));
    run(&mut doc, "mesh.bevel", json!({ "width": 0.05 }));
    assert_eq!(doc.inspect().unwrap()["objects"][0]["mesh"]["uvs"], true);
}

#[test]
fn images_go_into_material_slots_and_the_file() {
    let mut doc = starter();
    let set = run(&mut doc, "material.setTexture", json!({ "material": "Material", "base64": base64(&red_png()), "name": "red.png" }));
    assert_eq!((set["width"].as_u64(), set["height"].as_u64()), (Some(2), Some(2)));
    let inspected = doc.inspect().unwrap();
    assert_eq!(inspected["materials"][0]["baseColorTexture"], set["image"]);
    assert_eq!(inspected["images"][0]["name"], "red.png");

    // The material viewport shows it: the cube in the middle turns red.
    run(&mut doc, "view.shading", json!({ "mode": "material" }));
    let (width, height) = (doc.width(), doc.height());
    let pixels = doc.render(0.0, 0.0, 1.0, width, height).unwrap();
    let centre = ((height / 2 * width + width / 2) * 4) as usize;
    let [r, g, b] = [pixels[centre], pixels[centre + 1], pixels[centre + 2]];
    assert!(r > g + 40 && r > b + 40, "the textured cube is red: {r} {g} {b}");

    // Saved and reopened, the image is still there; cleared, it goes.
    let reopened = doc.reopen("t.kscene").unwrap();
    assert_eq!(reopened.inspect().unwrap()["images"][0]["hash"], set["image"]);
    run(&mut doc, "material.setTexture", json!({ "material": "Material", "clear": true }));
    assert_eq!(doc.inspect().unwrap()["images"], json!([]));
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(doc.inspect().unwrap()["images"][0]["hash"], set["image"]);
}

#[test]
fn bad_images_are_refused_and_change_nothing() {
    let mut doc = starter();
    let before = doc.save_file(None).unwrap();
    for params in [
        json!({ "material": "Material", "base64": base64(b"not an image at all") }),
        json!({ "material": "Material", "base64": "@@@" }),
        json!({ "material": "Nope", "base64": base64(&red_png()) }),
        json!({ "material": "Material", "slot": "displacement", "base64": base64(&red_png()) }),
        json!({ "material": "Material" }),
        json!({ "material": "Material", "base64": base64(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 1, 2, 3]) }),
    ] {
        assert!(doc.execute("material.setTexture", params.clone()).is_err(), "{params} was accepted");
    }
    assert_eq!(doc.save_file(None).unwrap(), before);
}

#[test]
fn an_image_material_without_coordinates_is_reported() {
    let mut doc = starter();
    run(&mut doc, "material.setTexture", json!({ "material": "Material", "base64": base64(&red_png()) }));
    run(&mut doc, "uv.clear", json!({ "objects": "Cube" }));
    let found = doc.check().into_iter().find(|diagnostic| diagnostic.code == "scene.textureWithoutUvs").expect("reported");
    let fix = found.fix.unwrap();
    run(&mut doc, &fix.command, fix.params.unwrap());
    assert!(doc.check().iter().all(|diagnostic| diagnostic.code != "scene.textureWithoutUvs"));
}
