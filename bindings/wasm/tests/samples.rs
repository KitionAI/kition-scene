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
