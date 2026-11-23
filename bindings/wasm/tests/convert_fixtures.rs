//! TEMPORARY (delete once the converted fixtures are committed): reads every `.kscene` fixture of
//! the previous container (`editor.json` version 4 with `replica.kdm`, the full snapshot), writes it
//! as a content file (its sealed content snapshot, docId and object ids kept) and prints it as
//! hex for the maintainer to commit. Fails on purpose so the CI log keeps the output.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::io::Read;

use kition_kit::kdm::{ActorId, KDM_VERSION, Replica};

const FIXTURES: &[&str] = &[
    "bindings/wasm/tests/fixtures/released-0.11.0.kscene",
    "bindings/wasm/tests/fixtures/released-0.12.0.kscene",
    "bindings/wasm/tests/fixtures/released-0.13.0.kscene",
    "bindings/wasm/tests/fixtures/released-0.15.0.kscene",
    "bindings/wasm/tests/fixtures/released-0.16.0.kscene",
    "bindings/wasm/tests/fixtures/released-0.17.0.kscene",
    "bindings/wasm/tests/fixtures/released-0.7.0.kscene",
    "bindings/wasm/tests/fixtures/released-0.8.0.kscene",
    "bindings/wasm/tests/fixtures/released-0.9.0-rig.kscene",
    "bindings/wasm/tests/fixtures/released-0.9.0.kscene",
    "cases/scene.agent.act/red-material/input/scene.kscene",
    "cases/scene.agent.context/three-objects-one-run/input/scene.kscene",
    "cases/scene.agent.conversation/modifier-stack/input/scene.kscene",
    "cases/scene.agent.observe/under-the-light/input/scene.kscene",
    "cases/scene.agent.recover/inside-out/input/cube.kscene",
    "conformance/basic.kscene",
    "conformance/check-degenerateFaces.kscene",
    "conformance/check-emptyMesh.kscene",
    "conformance/check-insideOut.kscene",
    "conformance/check-mixedNormals.kscene",
    "conformance/check-noCamera.kscene",
    "conformance/check-noLight.kscene",
    "conformance/check-nodesError.kscene",
    "conformance/check-outOfShot.kscene",
    "conformance/check-textureWithoutUvs.kscene",
    "conformance/check-unusedMaterial.kscene",
    "conformance/check-zeroScale.kscene",
    "web/public/sample/Sample.kscene",
];

fn entry(archive: &mut zip::ZipArchive<std::io::Cursor<Vec<u8>>>, name: &str) -> Option<Vec<u8>> {
    let mut file = archive.by_name(name).ok()?;
    let mut out = Vec::new();
    file.read_to_end(&mut out).unwrap();
    Some(out)
}

#[test]
fn convert_fixtures_to_content_files() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for path in FIXTURES {
        let bytes = std::fs::read(root.join(path)).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let envelope: serde_json::Value = serde_json::from_slice(&entry(&mut archive, "editor.json").unwrap()).unwrap();
        assert_eq!(envelope["version"], 4, "{path} is of the previous container");
        let snapshot = entry(&mut archive, "replica.kdm").unwrap();
        let mut replica = Replica::load(kition_scene_kdm::schema(), &snapshot, ActorId::random()).unwrap();
        let ids_before = replica.state_json().unwrap();
        let content = replica.content_snapshot().unwrap();
        let reloaded = Replica::load(kition_scene_kdm::schema(), &content, ActorId::random()).unwrap();
        assert_eq!(reloaded.state_json().unwrap(), ids_before, "{path}: the same document, the same ids");
        assert_eq!(Replica::snapshot_origins(&content).unwrap().len(), 1, "{path}: one change");
        let referenced = kition_scene_kdm::read::blob_hashes(&replica);
        let names: Vec<String> = archive.file_names().filter(|name| name.starts_with("blobs/")).map(str::to_owned).collect();
        let mut blobs = Vec::new();
        for name in names {
            let hash = name.trim_start_matches("blobs/").to_owned();
            if referenced.contains(&hash) {
                blobs.push((hash, entry(&mut archive, &name).unwrap()));
            }
        }
        let editor = envelope.get("editor").filter(|value| !value.is_null()).cloned();
        let file = kition_scene_kscene::File {
            meta: kition_scene_kscene::Meta {
                kdm: KDM_VERSION,
                schema_version: replica.schema().schema_version,
                doc_id: replica.doc_id().to_string(),
            },
            content,
            blobs,
            editor,
        };
        let out = kition_scene_kscene::write(&file).unwrap();
        let hex: String = out.iter().map(|byte| format!("{byte:02x}")).collect();
        println!("CONVERTED {path} {hex}");
    }
    panic!("TEMPORARY: the converted fixtures are printed above; commit them and delete this test");
}
