//! Writes the conformance fixtures (KHP `spec/conformance.md`): `basic.kscene`, a new scene that
//! `scene_check` finds clean, and one scene per code of `agent/checks.json` on which the check
//! reports that code, each made from the new scene with the kit's own commands.
//!
//! `cargo run -p kition-scene-kit --example conformance -- conformance`

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

/// A 2 m cube's corners and its six faces, wound so they point outwards.
const CORNERS: [[f32; 3]; 8] = [
    [-1.0, -1.0, -1.0],
    [1.0, -1.0, -1.0],
    [1.0, 1.0, -1.0],
    [-1.0, 1.0, -1.0],
    [-1.0, -1.0, 1.0],
    [1.0, -1.0, 1.0],
    [1.0, 1.0, 1.0],
    [-1.0, 1.0, 1.0],
];
const OUTWARD: [[u32; 4]; 6] = [[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7]];

fn faces(reversed: impl Fn(usize) -> bool) -> Value {
    json!(
        OUTWARD
            .iter()
            .enumerate()
            .map(|(index, face)| {
                let mut face = face.to_vec();
                if reversed(index) {
                    face.reverse();
                }
                face
            })
            .collect::<Vec<_>>()
    )
}

/// A 2 × 2 red PNG, the picture of the textured fixture.
fn red_png() -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, 2, 2);
    encoder.set_color(png::ColorType::Rgba);
    encoder.write_header().and_then(|mut writer| writer.write_image_data(&[255, 0, 0, 255].repeat(4))).map_err(|error| error.to_string())?;
    Ok(out)
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            out.push(if i <= chunk.len() { char::from(TABLE.get(((n >> shift) & 63) as usize).copied().unwrap_or(b'A')) } else { '=' });
        }
    }
    out
}

/// The new scene (a cube, a camera, a light), changed by `steps`.
fn scene(steps: &[(&str, Value)]) -> Result<Vec<u8>, String> {
    let mut doc = SceneDocument::create("Scene.kscene", 1280, 720)?;
    for (command, params) in steps {
        doc.execute(command, params.clone()).map_err(|error| format!("{command}: {error}"))?;
    }
    doc.save(None)
}

fn main() -> Result<(), String> {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).ok_or("usage: conformance <conformance folder>")?);
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let geometry = |faces: Value| ("mesh.setGeometry", json!({ "object": "Cube", "positions": CORNERS, "faces": faces }));
    let fixtures: Vec<(&str, Vec<u8>)> = vec![
        ("basic.kscene", scene(&[])?),
        ("check-zeroScale.kscene", scene(&[("object.transform", json!({ "objects": "Cube", "scale": [1, 1, 0] }))])?),
        ("check-nodesError.kscene", {
            let mut doc = SceneDocument::create("Scene.kscene", 1280, 720)?;
            let made = doc.execute("nodes.newGroup", json!({ "object": "Cube" }))?;
            doc.execute("nodes.removeNodes", json!({ "nodes": [made.get("output").cloned().unwrap_or_default()] }))?;
            doc.save(None)?
        }),
        ("check-emptyMesh.kscene", scene(&[("mesh.setGeometry", json!({ "object": "Cube", "positions": [], "faces": [] }))])?),
        (
            "check-degenerateFaces.kscene",
            scene(&[(
                "mesh.setGeometry",
                json!({ "object": "Cube", "positions": [[0, 0, 0], [1, 0, 0], [2, 0, 0], [0, 1, 0]], "faces": [[0, 1, 2], [0, 1, 3]] }),
            )])?,
        ),
        ("check-insideOut.kscene", scene(&[geometry(faces(|_| true))])?),
        ("check-mixedNormals.kscene", scene(&[geometry(faces(|index| index == 0))])?),
        (
            "check-textureWithoutUvs.kscene",
            scene(&[
                ("material.setTexture", json!({ "material": "Material", "base64": base64(&red_png()?) })),
                ("uv.clear", json!({ "objects": "Cube" })),
            ])?,
        ),
        ("check-noCamera.kscene", scene(&[("edit.delete", json!({ "objects": "Camera" }))])?),
        ("check-noLight.kscene", scene(&[("edit.delete", json!({ "objects": "Light" }))])?),
        ("check-outOfShot.kscene", scene(&[("object.transform", json!({ "objects": "Cube", "location": [0, 0, 200] }))])?),
        ("check-unusedMaterial.kscene", scene(&[("material.new", json!({ "name": "Spare", "assign": false }))])?),
    ];
    for (name, bytes) in fixtures {
        std::fs::write(dir.join(name), bytes).map_err(|error| format!("{name}: {error}"))?;
    }
    Ok(())
}
