//! What `ids.rs` and `outside.rs` share: scenes of cubes built with this kit's engine, the
//! `kition-scene` executable driven through the kits' MCP server's tools (`kition_kits::tools`, the
//! library `kition-kits-mcp` serves) as an agent drives it, and two programs other than that
//! server that change the file: a colleague's (this kit's engine, an instance of its own, opening
//! the file, running commands and saving it) and a bare rewriter (the `.kscene` ZIP written again
//! with its entries in another order, no engine).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, dead_code)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use kition_kit::KitDocument;
use kition_kits::tools::{self, CallOptions};
use kition_kits::{KitDefinition, KitManager};
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

pub const ME: &str = "agent-me";
pub const FILE: &str = "Scene.kscene";

pub fn kit() -> &'static KitDefinition {
    KitDefinition::from_manifest_text(include_str!("../../../../kit.json")).unwrap()
}

/// Where cube `C<n>` of [`scene`] stands: rows of ten, 3 m apart along x (from x = 3) and y, so
/// no two cubes' bounds (2 m wide) touch each other or the starter Cube's at the origin.
pub fn spot(n: usize) -> [f64; 3] {
    [3.0 * (n % 10) as f64 + 3.0, 3.0 * (n / 10) as f64, 0.0]
}

/// The starter scene (Cube at the origin, Light, Camera) with `cubes` more cubes `C0`, `C1`, …
/// at [`spot`]s.
pub fn scene_doc(cubes: usize) -> SceneDocument {
    let mut doc = SceneDocument::create(FILE, 320, 180).unwrap();
    for n in 0..cubes {
        doc.execute("object.addCube", json!({ "name": format!("C{n}"), "location": spot(n) })).unwrap();
    }
    doc
}

pub fn scene(cubes: usize) -> Vec<u8> {
    scene_doc(cubes).save_file(None).unwrap()
}

/// Sets the file's modification time an hour before its own, so that only its content tells the
/// server it changed.
fn backdate(file: &Path, before: std::time::SystemTime) {
    std::fs::File::options().write(true).open(file).unwrap().set_modified(before - Duration::from_secs(3600)).unwrap();
}

/// A colleague's program: this kit's engine, its own instance, opening the file from disk,
/// running `steps` (commands and parameters) and saving the file back.
pub fn colleague(file: &Path, steps: &[(&str, Value)]) -> Vec<Value> {
    let at = std::fs::metadata(file).unwrap().modified().unwrap();
    let mut doc = SceneDocument::open(FILE, &std::fs::read(file).unwrap()).unwrap();
    let answers = steps
        .iter()
        .map(|(command, params)| doc.execute(command, params.clone()).unwrap_or_else(|error| panic!("colleague's {command}: {error}")))
        .collect();
    std::fs::write(file, doc.save_file(Some(FILE)).unwrap()).unwrap();
    backdate(file, at);
    answers
}

/// Another program writing the `.kscene` ZIP again entry for entry, in reverse order and stored
/// without compression: other bytes, the same content.
pub fn rewrite(file: &Path) {
    let at = std::fs::metadata(file).unwrap().modified().unwrap();
    let bytes = std::fs::read(file).unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut entries = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        let mut content = Vec::new();
        entry.read_to_end(&mut content).unwrap();
        entries.push((entry.name().to_owned(), content));
    }
    // The `mimetype` entry stays first, as the format asks.
    let (first, rest) = entries.split_at(1);
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, content) in first.iter().chain(rest.iter().rev()) {
        out.start_file(name.as_str(), stored).unwrap();
        out.write_all(content).unwrap();
    }
    let written = out.finish().unwrap().into_inner();
    assert_ne!(written, std::fs::read(file).unwrap(), "the rewrite gives other bytes");
    std::fs::write(file, written).unwrap();
    backdate(file, at);
}

/// The file as a new engine reads it.
pub fn on_disk(file: &Path) -> SceneDocument {
    SceneDocument::open(FILE, &std::fs::read(file).unwrap()).unwrap()
}

/// Object `id` as a new engine's inspect of the file lists it (`None` when the file has none).
pub fn object_on_disk(file: &Path, id: u64) -> Option<Value> {
    on_disk(file).inspect().unwrap()["objects"].as_array().unwrap().iter().find(|object| object["id"] == id).cloned()
}

/// The ids of the objects an inspect lists, by name.
pub fn ids_by_name(inspected: &Value) -> std::collections::BTreeMap<String, u64> {
    inspected["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|object| (object["name"].as_str().unwrap().to_owned(), object["id"].as_u64().unwrap()))
        .collect()
}

static WORKSPACES: AtomicUsize = AtomicUsize::new(0);

/// A fresh, empty directory of this test process.
fn workspace() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kition-scene-agent-{}-{}", std::process::id(), WORKSPACES.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::canonicalize(dir).unwrap()
}

pub struct Fixture {
    pub kits: KitManager,
    pub options: CallOptions,
    pub workspace: PathBuf,
}

impl Fixture {
    pub fn new(cubes: usize) -> Self {
        Self::with(cubes, CallOptions::default())
    }

    pub fn with(cubes: usize, options: CallOptions) -> Self {
        Self::from_bytes(&scene(cubes), options)
    }

    pub fn from_bytes(bytes: &[u8], options: CallOptions) -> Self {
        let workspace = workspace();
        std::fs::write(workspace.join(FILE), bytes).unwrap();
        let kits = KitManager::new(Some(workspace.clone()), vec![(kit(), Some(PathBuf::from(env!("CARGO_BIN_EXE_kition-scene"))))]);
        Self { kits, options, workspace }
    }

    pub fn file(&self) -> PathBuf {
        self.workspace.join(FILE)
    }

    pub fn colleague(&self, steps: &[(&str, Value)]) -> Vec<Value> {
        colleague(&self.file(), steps)
    }

    pub async fn call(&self, action: &str, mut input: Value) -> Result<Value, Value> {
        input["path"] = json!(FILE);
        match tools::call_as(&self.kits, &format!("scene_{action}"), &input, self.options, ME).await {
            Some(Ok(output)) => Ok(output.output),
            Some(Err(error)) => Err(error.structured()),
            None => panic!("scene_{action} is no tool"),
        }
    }

    /// A call by another agent of the same server (`other`), as a second MCP connection makes it.
    pub async fn call_by(&self, other: &str, action: &str, mut input: Value) -> Value {
        input["path"] = json!(FILE);
        match tools::call_as(&self.kits, &format!("scene_{action}"), &input, self.options, other).await {
            Some(Ok(output)) => output.output,
            Some(Err(error)) => panic!("{other}'s scene_{action}: {}", error.structured()),
            None => panic!("scene_{action} is no tool"),
        }
    }

    pub async fn ok(&self, action: &str, input: Value) -> Value {
        self.call(action, input).await.unwrap_or_else(|error| panic!("scene_{action}: {error}"))
    }

    pub async fn refused(&self, action: &str, input: Value) -> Value {
        match self.call(action, input).await {
            Err(error) => error,
            Ok(answer) => panic!("scene_{action} was expected to be refused: {answer}"),
        }
    }

    /// The scene as the agent inspects it (`details`), and the answer's `revision`.
    pub async fn inspect(&self) -> (Value, String) {
        let seen = self.ok("inspect", json!({})).await;
        (seen["details"].clone(), revision(&seen))
    }

    pub async fn shutdown(self) {
        self.kits.shutdown().await;
        let _ = std::fs::remove_dir_all(&self.workspace);
    }
}

pub fn revision(answer: &Value) -> String {
    answer["revision"].as_str().unwrap_or_else(|| panic!("a revision: {answer}")).to_owned()
}

/// An id as an answer gives it, as text: an integer tool id is a number in receipts, `others` and
/// conflicts, a string elsewhere.
pub fn id_text(id: &Value) -> Option<String> {
    id.as_str().map(str::to_owned).or_else(|| id.as_u64().map(|id| id.to_string()))
}

/// The ids a change list names.
pub fn ids(list: &Value) -> Vec<String> {
    list.as_array().map(|items| items.iter().filter_map(|item| id_text(&item["id"])).collect()).unwrap_or_default()
}

/// The entry of `list` naming `id`.
pub fn entry<'a>(list: &'a Value, id: &str) -> &'a Value {
    list.as_array()
        .and_then(|items| items.iter().find(|item| id_text(&item["id"]).as_deref() == Some(id)))
        .unwrap_or_else(|| panic!("no {id} in {list}"))
}

/// A refusal of a write whose premises others changed, naming `id` with `reason`.
pub fn assert_conflict<'a>(refused: &'a Value, id: &str, reason: &str) -> &'a Value {
    assert_eq!(refused["error"]["code"], "conflict", "{refused}");
    assert_eq!(refused["error"]["retryable"], true, "{refused}");
    assert!(refused["revision"].is_string(), "{refused}");
    let found =
        refused["conflicts"].as_array().and_then(|conflicts| conflicts.iter().find(|conflict| id_text(&conflict["id"]).as_deref() == Some(id)));
    let found = found.unwrap_or_else(|| panic!("no conflict on {id}: {refused}"));
    assert_eq!(found["reason"], reason, "{refused}");
    found
}

/// The median of `samples`, in milliseconds.
pub fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples.get(samples.len() / 2).copied().unwrap_or(f64::NAN)
}
