//! Damaged and hostile files are refused with an error, never a panic.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use kition_scene_exchange::{Format, Imported, ImportedObject, export, import, place};
use kition_scene_model::glam::Vec3;
use kition_scene_model::{ObjectData, Scene};
use serde_json::json;

fn glb(json: &serde_json::Value, bin: &[u8]) -> Vec<u8> {
    let mut text = serde_json::to_vec(json).unwrap();
    while !text.len().is_multiple_of(4) {
        text.push(b' ');
    }
    let mut out = Vec::new();
    out.extend(b"glTF");
    out.extend(2u32.to_le_bytes());
    out.extend(((12 + 8 + text.len() + 8 + bin.len()) as u32).to_le_bytes());
    out.extend((text.len() as u32).to_le_bytes());
    out.extend(0x4E4F_534Au32.to_le_bytes());
    out.extend(&text);
    out.extend((bin.len() as u32).to_le_bytes());
    out.extend(0x004E_4942u32.to_le_bytes());
    out.extend(bin);
    out
}

fn triangle(accessor: serde_json::Value, view: serde_json::Value, bin_length: usize) -> serde_json::Value {
    json!({
        "asset": { "version": "2.0" },
        "nodes": [{ "mesh": 0 }],
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0 } }] }],
        "accessors": [accessor],
        "bufferViews": [view],
        "buffers": [{ "byteLength": bin_length }],
    })
}

#[test]
fn every_truncation_of_a_glb_is_refused_or_read() {
    let bytes = export(&Scene::starter(), Format::Glb, None).unwrap();
    for cut in (0..bytes.len()).step_by(7) {
        let _ = import(&bytes[..cut], Format::Glb);
    }
    assert!(import(&bytes[..bytes.len() / 2], Format::Glb).is_err());
    assert!(import(b"glTF", Format::Glb).is_err());
    assert!(import(b"{}", Format::Gltf).is_err());
}

#[test]
fn accessors_past_their_buffers_are_refused() {
    let bin = vec![0u8; 36];
    let good = triangle(json!({ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3" }), json!({ "buffer": 0, "byteLength": 36 }), 36);
    assert!(import(&glb(&good, &bin), Format::Glb).is_ok());
    for (accessor, view) in [
        (json!({ "bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3" }), json!({ "buffer": 0, "byteLength": 36 })),
        (json!({ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "byteOffset": 4 }), json!({ "buffer": 0, "byteLength": 36 })),
        (json!({ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3" }), json!({ "buffer": 0, "byteOffset": 8, "byteLength": 36 })),
        (json!({ "bufferView": 0, "componentType": 5126, "count": 4_000_000_000u64, "type": "VEC3" }), json!({ "buffer": 0, "byteLength": 36 })),
        (
            json!({ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3" }),
            json!({ "buffer": 0, "byteLength": 18_446_744_073_709_551_615u64 }),
        ),
        (json!({ "bufferView": 7, "componentType": 5126, "count": 3, "type": "VEC3" }), json!({ "buffer": 0, "byteLength": 36 })),
        (json!({ "bufferView": 0, "componentType": 9999, "count": 3, "type": "VEC3" }), json!({ "buffer": 0, "byteLength": 36 })),
    ] {
        let file = glb(&triangle(accessor.clone(), view.clone(), 36), &bin);
        assert!(import(&file, Format::Glb).is_err(), "{accessor} {view}");
    }
    // Indices that name missing vertices, and NaN positions.
    let mut with_indices = good.clone();
    with_indices["meshes"][0]["primitives"][0]["indices"] = json!(1);
    with_indices["accessors"].as_array_mut().unwrap().push(json!({ "bufferView": 1, "componentType": 5125, "count": 3, "type": "SCALAR" }));
    with_indices["bufferViews"].as_array_mut().unwrap().push(json!({ "buffer": 0, "byteOffset": 36, "byteLength": 12 }));
    with_indices["buffers"][0]["byteLength"] = json!(48);
    let mut bin = vec![0u8; 36];
    bin.extend([0u32, 1, 99].iter().flat_map(|value| value.to_le_bytes()));
    assert!(import(&glb(&with_indices, &bin), Format::Glb).is_err());
    let mut nan = vec![0u8; 36];
    nan[0..4].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(import(&glb(&good, &nan), Format::Glb).is_err());
}

#[test]
fn node_cycles_and_external_buffers_are_refused() {
    let cycle = json!({ "asset": { "version": "2.0" }, "scenes": [{ "nodes": [0] }], "nodes": [{ "children": [1] }, { "children": [0] }] });
    assert!(import(&serde_json::to_vec(&cycle).unwrap(), Format::Gltf).is_err());
    let external = json!({ "asset": { "version": "2.0" }, "buffers": [{ "uri": "scene.bin", "byteLength": 4 }] });
    let error = import(&serde_json::to_vec(&external).unwrap(), Format::Gltf).unwrap_err();
    assert!(error.to_string().contains("scene.bin"), "{error}");
}

#[test]
fn bad_obj_and_stl_are_refused() {
    assert!(import(b"v 0 0 0\nf 1 2 3\n", Format::Obj).is_err());
    assert!(import(b"v 0 0 nan\n", Format::Obj).is_err());
    assert!(import(b"v 0 0\n", Format::Obj).is_err());
    assert!(import(&[0xff, 0xfe], Format::Obj).is_err());
    assert!(import(b"", Format::Obj).is_err());
    assert!(import(b"not an stl", Format::Stl).is_err());
    let mut binary = vec![0u8; 84];
    binary[80..84].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(import(&binary, Format::Stl).is_err());
    assert!(import(b"solid x\nvertex 0 0 0\nvertex 1 0 0\n", Format::Stl).is_err());
}

#[test]
fn place_refuses_inconsistent_imports() {
    let object = |parent| ImportedObject {
        name: "A".into(),
        data: ObjectData::Empty { size: 1.0 },
        location: Vec3::ZERO,
        rotation: Vec3::ZERO,
        scale: Vec3::ONE,
        parent,
        parent_inverse: kition_scene_model::glam::Mat4::IDENTITY,
        material: None,
    };
    let mut scene = Scene::empty();
    assert!(place(&mut scene, Imported { objects: vec![object(Some(1)), object(Some(0))], ..Imported::default() }).is_err());
    assert!(place(&mut scene, Imported { objects: vec![object(Some(5))], ..Imported::default() }).is_err());
    assert!(scene.objects.is_empty());
    let roots = place(&mut scene, Imported { objects: vec![object(None), object(Some(0))], ..Imported::default() }).unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(scene.objects[1].name, "A.001");
}
