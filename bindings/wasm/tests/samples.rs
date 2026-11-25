//! Every committed `.kscene` sample (`conformance/`, `cases/`, `web/public/sample/` and the
//! `bindings/wasm/tests/fixtures/released-*` files) opens and saves to its own bytes (kition-sdk
//! data model 7.4; feature `scene.file.content-only`): its content is the canonical
//! `content.json` the kit writes, with only the blobs its state references and no other entry,
//! so the samples are the kit's own files. Opening runs no repair, so a sample of a broken scene
//! (the `check-*` fixtures) stays broken.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::path::{Path, PathBuf};

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The committed samples, by their path from the repository root, sorted.
fn samples() -> Vec<String> {
    let root = repo();
    let mut found = Vec::new();
    let mut stack: Vec<PathBuf> =
        ["conformance", "cases", "web/public/sample", "bindings/wasm/tests/fixtures"].iter().map(|dir| root.join(dir)).collect();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|extension| extension == "kscene") {
                found.push(path.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/"));
            }
        }
    }
    found.sort();
    found
}

/// The file the kit saves for `bytes` (a `.kscene`): opened, then saved.
fn kit_save(name: &str, bytes: &[u8]) -> Vec<u8> {
    let mut doc = SceneDocument::open(name, bytes).unwrap_or_else(|error| panic!("{name}: {error}"));
    doc.save_file(None).unwrap()
}

#[test]
fn every_committed_sample_opens_and_saves_to_its_own_bytes() {
    let samples = samples();
    assert!(samples.len() >= 28, "the samples: {samples:?}");
    for sample in samples {
        let bytes = std::fs::read(repo().join(&sample)).unwrap();
        let entries: Vec<String> = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap().file_names().map(str::to_owned).collect();
        for name in &entries {
            assert!(["mimetype", "meta.json", "content.json"].contains(&name.as_str()) || name.starts_with("blobs/"), "{sample} holds {name}");
        }
        assert!(kit_save(&sample, &bytes) == bytes, "{sample} saves to its own bytes");
    }
}

/// TEMPORARY (plain model sample conversion, deleted in the next commit): writes each converted
/// `content.json` (`tests/regen/<path with / as __>.json`, written by kition-sdk's
/// `tools/convert-content`) into its sample with the sample's own blobs, saves it through the kit
/// and prints the file as base64, then fails so CI shows the output.
#[test]
fn regenerate_samples() {
    let regen = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/regen");
    let mut printed = 0;
    for sample in samples() {
        let content = std::fs::read(regen.join(format!("{}.json", sample.replace('/', "__")))).unwrap();
        let mut file = kition_scene_kscene::read(&std::fs::read(repo().join(&sample)).unwrap()).unwrap();
        file.content = content;
        let bytes = kit_save(&sample, &kition_scene_kscene::write(&file).unwrap());
        println!("REGEN {sample} {}", base64(&bytes));
        printed += 1;
    }
    panic!("REGEN printed {printed} samples");
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let (a, b, c) = (chunk[0], chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0));
        let triple = (u32::from(a) << 16) | (u32::from(b) << 8) | u32::from(c);
        for (index, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            out.push(if index <= chunk.len() { char::from(TABLE[((triple >> shift) & 63) as usize]) } else { '=' });
        }
    }
    out
}
