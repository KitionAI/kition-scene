//! `.blend` files written here, field by field, with a small structure catalogue of their own:
//! the reader finds fields by name, so these exercise its real paths without the reference
//! application. Every truncation and many byte flips are refused or read without a panic.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_scene_model::{ObjectData, Scene};

/// A struct in the catalogue: its name and (type, field name) pairs.
type Struct = (&'static str, &'static [(&'static str, &'static str)]);

const STRUCTS: &[Struct] = &[
    ("ID", &[("void", "*lib"), ("char", "name[66]")]),
    ("ListBase", &[("void", "*first"), ("void", "*last")]),
    (
        "Object",
        &[
            ("ID", "id"),
            ("short", "type"),
            ("short", "rotmode"),
            ("int", "restrictflag"),
            ("void", "*data"),
            ("Object", "*parent"),
            ("float", "loc[3]"),
            ("float", "rot[3]"),
            ("float", "size[3]"),
            ("float", "parentinv[4][4]"),
            ("ListBase", "modifiers"),
        ],
    ),
    (
        "Mesh",
        &[
            ("ID", "id"),
            ("int", "totvert"),
            ("int", "totedge"),
            ("int", "totpoly"),
            ("int", "totloop"),
            ("int", "*poly_offset_indices"),
            ("AttributeStorage", "attribute_storage"),
        ],
    ),
    ("AttributeStorage", &[("Attribute", "*dna_attributes"), ("int", "dna_attributes_num"), ("int", "pad")]),
    ("Attribute", &[("char", "*name"), ("short", "data_type"), ("char", "domain"), ("char", "storage_type"), ("int", "pad"), ("void", "*data")]),
    ("AttributeArray", &[("void", "*data"), ("void", "*sharing_info"), ("int64_t", "size")]),
    ("ModifierData", &[("ModifierData", "*next"), ("ModifierData", "*prev"), ("int", "type"), ("int", "mode"), ("char", "name[64]")]),
    ("SubsurfModifierData", &[("ModifierData", "modifier"), ("short", "levels"), ("short", "pad")]),
    ("raw_data", &[]),
];

const PRIMITIVE_SIZES: &[(&str, u16)] = &[("char", 1), ("short", 2), ("int", 4), ("float", 4), ("void", 0), ("int64_t", 8)];

struct Writer {
    types: Vec<(String, u16)>,
    names: Vec<String>,
    structs: Vec<(u16, Vec<(u16, u16)>)>,
}

impl Writer {
    fn new() -> Self {
        let mut writer = Writer { types: Vec::new(), names: Vec::new(), structs: Vec::new() };
        for (name, size) in PRIMITIVE_SIZES {
            writer.types.push(((*name).into(), *size));
        }
        for (name, _) in STRUCTS {
            writer.types.push(((*name).into(), 0));
        }
        for (name, fields) in STRUCTS {
            let mut size = 0u16;
            let mut list = Vec::new();
            for (kind, field) in *fields {
                let type_index = writer.types.iter().position(|(known, _)| known == kind).unwrap() as u16;
                let name_index = writer.names.iter().position(|known| known == field).unwrap_or_else(|| {
                    writer.names.push((*field).into());
                    writer.names.len() - 1
                }) as u16;
                list.push((type_index, name_index));
                let count: u16 = field.split('[').skip(1).map(|part| part.trim_end_matches(']').parse::<u16>().unwrap()).product();
                let element = if field.starts_with('*') { 8 } else { writer.types[type_index as usize].1 };
                size += element * count.max(1);
            }
            let index = writer.types.iter().position(|(known, _)| known == name).unwrap();
            writer.types[index].1 = size;
            writer.structs.push((index as u16, list));
        }
        writer
    }

    fn dna(&self) -> Vec<u8> {
        let mut out = b"SDNANAME".to_vec();
        let pad = |out: &mut Vec<u8>| {
            while !out.len().is_multiple_of(4) {
                out.push(0);
            }
        };
        out.extend((self.names.len() as u32).to_le_bytes());
        for name in &self.names {
            out.extend(name.as_bytes());
            out.push(0);
        }
        pad(&mut out);
        out.extend(b"TYPE");
        out.extend((self.types.len() as u32).to_le_bytes());
        for (name, _) in &self.types {
            out.extend(name.as_bytes());
            out.push(0);
        }
        pad(&mut out);
        out.extend(b"TLEN");
        for (_, size) in &self.types {
            out.extend(size.to_le_bytes());
        }
        pad(&mut out);
        out.extend(b"STRC");
        out.extend((self.structs.len() as u32).to_le_bytes());
        for (kind, fields) in &self.structs {
            out.extend(kind.to_le_bytes());
            out.extend((fields.len() as u16).to_le_bytes());
            for (field_type, field_name) in fields {
                out.extend(field_type.to_le_bytes());
                out.extend(field_name.to_le_bytes());
            }
        }
        out
    }

    fn sdna(&self, name: &str) -> u32 {
        self.structs.iter().position(|(kind, _)| self.types[*kind as usize].0 == name).unwrap() as u32
    }
}

fn block(out: &mut Vec<u8>, code: &[u8; 4], sdna: u32, old: u64, body: &[u8], count: u64) {
    out.extend(code);
    out.extend(sdna.to_le_bytes());
    out.extend(old.to_le_bytes());
    out.extend((body.len() as u64).to_le_bytes());
    out.extend(count.to_le_bytes());
    out.extend(body);
}

fn floats(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|value| value.to_le_bytes()).collect()
}

fn ints(values: &[i32]) -> Vec<u8> {
    values.iter().flat_map(|value| value.to_le_bytes()).collect()
}

fn id(name: &str) -> Vec<u8> {
    let mut out = 0u64.to_le_bytes().to_vec();
    let mut text = name.as_bytes().to_vec();
    text.resize(66, 0);
    out.extend(text);
    out
}

/// A file with a parent empty and a child triangle mesh with a subdivision modifier.
fn sample() -> Vec<u8> {
    let writer = Writer::new();
    let mut out = b"BLENDER17-01v0502".to_vec();
    let identity: Vec<f32> = (0..16).map(|index| if index % 5 == 0 { 1.0 } else { 0.0 }).collect();
    let object = |name: &str, kind: i16, data: u64, parent: u64, loc: [f32; 3], modifiers: u64| {
        let mut body = id(name);
        body.extend(kind.to_le_bytes());
        body.extend(1i16.to_le_bytes());
        body.extend(0i32.to_le_bytes());
        body.extend(data.to_le_bytes());
        body.extend(parent.to_le_bytes());
        body.extend(floats(&loc));
        body.extend(floats(&[0.0, 0.0, 0.5]));
        body.extend(floats(&[2.0, 2.0, 2.0]));
        body.extend(floats(&identity));
        body.extend(modifiers.to_le_bytes());
        body.extend(modifiers.to_le_bytes());
        body
    };
    block(&mut out, b"OB\0\0", writer.sdna("Object"), 0x100, &object("OBPivot", 0, 0, 0, [0.0, 0.0, 1.0], 0), 1);
    block(&mut out, b"OB\0\0", writer.sdna("Object"), 0x200, &object("OBTri", 1, 0x300, 0x100, [1.0, 0.0, 0.0], 0x900), 1);
    let mut subsurf = 0u64.to_le_bytes().to_vec();
    subsurf.extend(0u64.to_le_bytes());
    subsurf.extend(1i32.to_le_bytes());
    subsurf.extend(1i32.to_le_bytes());
    let mut name = b"Subdivision".to_vec();
    name.resize(64, 0);
    subsurf.extend(name);
    subsurf.extend(2i16.to_le_bytes());
    subsurf.extend(0i16.to_le_bytes());
    block(&mut out, b"DATA", writer.sdna("SubsurfModifierData"), 0x900, &subsurf, 1);
    let mut mesh = id("METri");
    mesh.extend(ints(&[3, 3, 1, 3]));
    mesh.extend(0x400u64.to_le_bytes());
    mesh.extend(0x500u64.to_le_bytes());
    mesh.extend(ints(&[2, 0]));
    block(&mut out, b"ME\0\0", writer.sdna("Mesh"), 0x300, &mesh, 1);
    block(&mut out, b"DATA", writer.sdna("raw_data"), 0x400, &ints(&[0, 3]), 1);
    let mut attributes = Vec::new();
    for (name, data) in [(0x600u64, 0x700u64), (0x610, 0x710)] {
        attributes.extend(name.to_le_bytes());
        attributes.extend(0i16.to_le_bytes());
        attributes.extend([0u8, 0u8]);
        attributes.extend(0i32.to_le_bytes());
        attributes.extend(data.to_le_bytes());
    }
    block(&mut out, b"DATA", writer.sdna("Attribute"), 0x500, &attributes, 2);
    block(&mut out, b"DATA", writer.sdna("raw_data"), 0x600, b"position\0", 1);
    block(&mut out, b"DATA", writer.sdna("raw_data"), 0x610, b".corner_vert\0", 1);
    for (old, data) in [(0x700u64, 0x800u64), (0x710, 0x810)] {
        let mut array = data.to_le_bytes().to_vec();
        array.extend(0u64.to_le_bytes());
        array.extend(3i64.to_le_bytes());
        block(&mut out, b"DATA", writer.sdna("AttributeArray"), old, &array, 1);
    }
    block(&mut out, b"DATA", writer.sdna("raw_data"), 0x800, &floats(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]), 1);
    block(&mut out, b"DATA", writer.sdna("raw_data"), 0x810, &ints(&[0, 1, 2]), 1);
    block(&mut out, b"DNA1", 0, 0x1, &writer.dna(), 1);
    block(&mut out, b"ENDB", 0, 0, &[], 0);
    out
}

#[test]
fn a_written_file_reads_back() {
    let file = kition_scene_blend::read(&sample()).unwrap();
    assert_eq!(file.imported.objects.len(), 2);
    let tri = &file.imported.objects[1];
    assert_eq!(tri.name, "Tri");
    assert_eq!(tri.parent, Some(0));
    let ObjectData::Mesh(mesh) = &tri.data else { panic!("not a mesh") };
    assert_eq!((mesh.positions.len(), mesh.faces.len()), (3, 1));
    assert_eq!(file.modifiers[1].len(), 1);
    assert_eq!(file.modifiers[1][0].kind, kition_scene_model::ModifierKind::Subdivision { levels: 2 });

    let mut scene = Scene::empty();
    kition_scene_blend::place(&mut scene, file).unwrap();
    let child = scene.object_named("Tri").unwrap();
    let world = scene.world_matrix(child.id).unwrap();
    // The parent sits at (0, 0, 1) turned 0.5 rad round Z and scaled 2; the child at (1, 0, 0)
    // in its space.
    let expected = kition_scene_model::glam::Vec3::new(0.5f32.cos() * 2.0, 0.5f32.sin() * 2.0, 1.0);
    assert!(world.transform_point3(kition_scene_model::glam::Vec3::ZERO).abs_diff_eq(expected, 1e-5));
    assert_eq!(child.modifiers.len(), 1);
}

#[test]
fn compressed_files_read_like_plain_ones() {
    // A zstd frame with no compression (raw blocks), written by hand.
    let plain = sample();
    let mut frame = vec![0x28, 0xB5, 0x2F, 0xFD, 0x00];
    // Window descriptor (needed without single-segment), then raw blocks of up to 128 KiB.
    frame.push(0x58);
    for (index, chunk) in plain.chunks(100_000).enumerate() {
        let last = (index + 1) * 100_000 >= plain.len();
        let header = (chunk.len() as u32) << 3 | u32::from(last);
        frame.extend(&header.to_le_bytes()[..3]);
        frame.extend(chunk);
    }
    let a = kition_scene_blend::read(&plain).unwrap();
    let b = kition_scene_blend::read(&frame).unwrap();
    assert_eq!(a.imported.objects, b.imported.objects);
}

#[test]
fn other_files_are_refused() {
    for bytes in [&b""[..], b"BLENDER", b"BLENDER_v405", b"BLENDER-V405", b"BLENDER17-01V0502", b"PK\x03\x04", b"\x28\xb5\x2f\xfdgarbage"] {
        assert!(kition_scene_blend::read(bytes).is_err(), "{bytes:?}");
    }
    let mut without_catalogue = b"BLENDER17-01v0502".to_vec();
    block(&mut without_catalogue, b"ENDB", 0, 0, &[], 0);
    assert!(kition_scene_blend::read(&without_catalogue).is_err());
}

#[test]
fn damaged_files_never_panic() {
    let file = sample();
    for cut in 0..file.len() {
        let _ = kition_scene_blend::read(&file[..cut]);
    }
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    for _ in 0..4000 {
        let mut copy = file.clone();
        for _ in 0..4 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let at = (seed % copy.len() as u64) as usize;
            copy[at] = (seed >> 32) as u8;
        }
        let _ = kition_scene_blend::read(&copy);
    }
}
