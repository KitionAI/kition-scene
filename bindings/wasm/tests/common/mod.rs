//! The Scene kit's adapter for kition-sdk's convergence suite: replicas of a scene edited through
//! the engine's own commands.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use kition_convergence::{Blobs, KitAdapter, Result, Rng, Script};
use kition_kit::KitDocument;
use kition_kit::kdm::{ActorId, Replica};
use kition_kit::session::KitSessionDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

pub const NAME: &str = "Scene.kscene";

pub struct SceneKit;

pub fn decode_blobs(bytes: &[u8]) -> Result<Blobs> {
    let mut out = Blobs::new();
    for (hash, bytes) in kition_scene_kdm::blobs::decode_bundle(bytes)? {
        out.insert(hash, bytes);
    }
    Ok(out)
}

pub fn encode_blobs(blobs: &Blobs) -> Vec<u8> {
    kition_convergence::wasm::encode_blobs(blobs)
}

fn exec(doc: &mut SceneDocument, command: &str, params: Value) -> Result<Value> {
    doc.execute(command, params).map_err(|error| format!("{command}: {error}"))
}

fn attempt(doc: &mut SceneDocument, command: &str, params: Value) -> bool {
    doc.execute(command, params).is_ok()
}

/// The scene's objects as (id, type), in outliner order.
fn objects(doc: &SceneDocument) -> Vec<(u64, String)> {
    let inspected = doc.inspect().unwrap_or(Value::Null);
    inspected["objects"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|object| Some((object["id"].as_u64()?, object["type"].as_str()?.to_owned())))
        .collect()
}

fn meshes(doc: &SceneDocument) -> Vec<u64> {
    objects(doc).into_iter().filter(|(_, kind)| kind == "mesh").map(|(id, _)| id).collect()
}

fn probe(doc: &SceneDocument) -> Result<u64> {
    meshes(doc).first().copied().ok_or_else(|| "the scene has no mesh".to_owned())
}

/// A 2 × 2 PNG of a colour from `seed`.
fn png(seed: u32) -> Vec<u8> {
    let [r, g, b, _] = seed.to_le_bytes();
    let rgba: Vec<u8> = (0..4).flat_map(|_| [r, g, b, 255]).collect();
    kition_scene_kit::encode_png(2, 2, &rgba).unwrap()
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

/// The scene a document shows, without session state (selection, view, mode, tool).
fn content(doc: &SceneDocument) -> Value {
    let inspected = doc.inspect().unwrap_or(Value::Null);
    json!({ "objects": inspected["objects"], "materials": inspected["materials"], "camera": inspected["camera"], "world": inspected["world"], "render": inspected["render"], "particles": inspected["particles"] })
}

impl KitAdapter for SceneKit {
    type Doc = SceneDocument;

    fn name(&self) -> &str {
        "kit.scene"
    }

    fn schema(&self) -> &str {
        kition_scene_kdm::SCHEMA
    }

    fn create(&self) -> Result<SceneDocument> {
        let mut doc = SceneDocument::create(NAME, 320, 180)?;
        exec(&mut doc, "add.uvSphere", json!({ "name": "Ball", "location": [3, 0, 1], "segments": 8, "rings": 4 }))?;
        exec(&mut doc, "material.new", json!({ "name": "Red", "baseColor": "#cc3322" }))?;
        exec(&mut doc, "add.cylinder", json!({ "name": "Post", "location": [-3, 0, 1], "vertices": 8 }))?;
        exec(&mut doc, "object.parent", json!({ "objects": ["Post"], "parent": "Cube" }))?;
        exec(&mut doc, "modifier.add", json!({ "object": "Ball", "type": "subdivision" }))?;
        exec(&mut doc, "select.none", json!({}))?;
        Ok(doc)
    }

    fn join(&self, snapshot: &[u8], blobs: &Blobs) -> Result<SceneDocument> {
        SceneDocument::open_with_replica(NAME, &[], snapshot, &encode_blobs(blobs))
    }

    fn version(&self, doc: &mut SceneDocument) -> Result<Vec<u8>> {
        Ok(kition_kit::session::__version(doc))
    }

    fn export_changes(&self, doc: &mut SceneDocument, since: &[u8]) -> Result<Vec<u8>> {
        kition_kit::session::__export_changes(doc, since)
    }

    fn import_changes(&self, doc: &mut SceneDocument, bytes: &[u8]) -> Result<()> {
        KitSessionDocument::import_changes(doc, bytes).map(|_| ())
    }

    fn snapshot(&self, doc: &mut SceneDocument) -> Result<Vec<u8>> {
        kition_kit::session::__snapshot(doc)
    }

    fn state(&self, doc: &mut SceneDocument) -> Result<Value> {
        let snapshot = kition_kit::session::__snapshot(doc)?;
        let replica = Replica::load(kition_scene_kdm::schema(), &snapshot, ActorId::random()).map_err(|error| error.to_string())?;
        replica.state_json().map_err(|error| error.to_string())
    }

    fn undo(&self, doc: &mut SceneDocument) -> Result<bool> {
        KitSessionDocument::undo(doc).map(|(done, _)| done)
    }

    fn redo(&self, doc: &mut SceneDocument) -> Result<bool> {
        KitSessionDocument::redo(doc).map(|(done, _)| done)
    }

    fn random_edit(&self, doc: &mut SceneDocument, rng: &mut Rng) -> Result<Option<String>> {
        let all = objects(doc);
        let meshes = meshes(doc);
        let target = rng.pick(&all).map(|(id, _)| *id);
        let mesh = rng.pick(&meshes).copied();
        let value = rng.next_u64() % 10_000;
        let small = (value % 9) as f64 - 4.0;
        let (label, done) = match (rng.below(18), target, mesh) {
            (0 | 1, _, _) => ("add cube", attempt(doc, "add.cube", json!({ "location": [small, (value % 5) as f64, 0] }))),
            (2, _, _) => ("add sphere", attempt(doc, "add.icoSphere", json!({ "location": [0, small, 1], "subdivisions": 1 }))),
            (3 | 4, Some(id), _) => (
                "move",
                {
                    let _ = doc.execute("select.objects", json!({ "objects": [id] }));
                    true
                } && attempt(doc, "object.translate", json!({ "objects": [id], "offset": [small, 0.5, 0] })),
            ),
            (5, Some(id), _) => (
                "rotate",
                doc.execute("select.objects", json!({ "objects": [id] })).is_ok()
                    && attempt(doc, "object.rotate", json!({ "objects": [id], "axis": "z", "angle": small * 10.0 })),
            ),
            (6, Some(id), _) => (
                "scale",
                doc.execute("select.objects", json!({ "objects": [id] })).is_ok()
                    && attempt(doc, "object.scale", json!({ "objects": [id], "factor": 1.0 + (value % 3) as f64 * 0.1 })),
            ),
            (7, Some(id), _) if all.len() > 3 => ("delete", attempt(doc, "edit.delete", json!({ "objects": [id] }))),
            (8, Some(id), _) => ("rename", attempt(doc, "object.rename", json!({ "object": id, "name": format!("Thing {value}") }))),
            (9, _, Some(id)) => ("modifier", attempt(doc, "modifier.add", json!({ "object": id, "type": "bevel" }))),
            (10, _, _) => {
                ("material", attempt(doc, "material.set", json!({ "material": "Red", "baseColor": [((value % 7) as f64) / 7.0, 0.2, 0.3] })))
            }
            (11, Some(a), _) => {
                let Some(&(b, _)) = rng.pick(&all) else { return Ok(None) };
                if a == b {
                    return Ok(None);
                }
                let _ = doc.execute("select.objects", json!({ "objects": [a, b] }));
                ("parent", attempt(doc, "object.parent", json!({ "objects": [a], "parent": b })))
            }
            (12, _, Some(id)) => {
                // An edit-mode step: extrude every face of a mesh a little.
                let ok = attempt(doc, "mode.set", json!({ "mode": "edit", "object": id }))
                    && attempt(doc, "select.all", json!({ "action": "select" }))
                    && attempt(doc, "mesh.subdivide", json!({}));
                let _ = doc.execute("mode.set", json!({ "mode": "object" }));
                ("subdivide", ok)
            }
            (13, Some(id), _) => ("key", attempt(doc, "anim.insertKey", json!({ "objects": [id], "frame": 1 + (value % 40) as i64 }))),
            (14, _, _) => (
                "add curve",
                attempt(doc, "add.curveBezier", json!({ "location": [small, 0, 2] }))
                    && attempt(doc, "curve.set", json!({ "bevelDepth": 0.05 + (value % 3) as f64 * 0.05 })),
            ),
            (15, _, _) => (
                "add text",
                attempt(doc, "add.text", json!({ "body": format!("T{value}"), "location": [0, small, 0] }))
                    && attempt(doc, "text.set", json!({ "extrude": 0.02 })),
            ),
            (16, Some(collider), Some(id)) => (
                "particles",
                attempt(doc, "particles.add", json!({ "object": id, "count": 50 + value % 100, "seed": value % 7, "kind": "emitter" }))
                    && (collider == id || attempt(doc, "particles.set", json!({ "object": id, "colliders": [collider] }))),
            ),
            (17, _, Some(id)) => {
                // Hair, groomed with a comb stroke across the view.
                let ok = attempt(doc, "particles.add", json!({ "object": id, "kind": "hair", "count": 30, "hairLength": 0.5 }))
                    && attempt(doc, "mode.particleEdit", json!({ "object": id }))
                    && attempt(doc, "particleEdit.stroke", json!({ "kind": "comb", "radius": 200, "points": [[60, 90], [160, 80], [260, 70]] }));
                let _ = doc.execute("mode.particleEdit", json!({ "on": false }));
                ("hair", ok)
            }
            _ => return Ok(None),
        };
        let _ = doc.execute("select.none", json!({}));
        Ok(done.then(|| label.to_owned()))
    }

    fn script(&self, doc: &mut SceneDocument, step: &Script) -> Result<bool> {
        match step {
            Script::SetField { slot: 0, value } => {
                let id = probe(doc)?;
                exec(doc, "object.transform", json!({ "objects": [id], "location": [1 + (*value % 40), 0, 0] }))?;
            }
            Script::SetField { value, .. } => {
                let id = probe(doc)?;
                exec(doc, "object.transform", json!({ "objects": [id], "rotation": [0, 0, 1 + (*value % 300)] }))?;
            }
            Script::Create => {
                exec(doc, "add.cube", json!({ "location": [5, 5, 0] }))?;
            }
            Script::Delete => {
                let id = probe(doc)?;
                exec(doc, "edit.delete", json!({ "objects": [id] }))?;
            }
            Script::Duplicate => {
                let id = probe(doc)?;
                exec(doc, "select.objects", json!({ "objects": [id] }))?;
                exec(doc, "edit.duplicate", json!({ "offset": [1, 0, 0] }))?;
            }
            Script::Reorder { which } => {
                let all = objects(doc);
                let id = all.iter().rev().nth(usize::from(*which)).map(|(id, _)| *id).ok_or("no object")?;
                exec(doc, "object.order", json!({ "object": id, "index": 0 }))?;
            }
            Script::Reparent { which } => {
                let meshes = meshes(doc);
                let (Some(&a), Some(&b)) = (meshes.first(), meshes.get(1)) else { return Ok(false) };
                let (child, parent) = if *which == 0 { (a, b) } else { (b, a) };
                exec(doc, "select.objects", json!({ "objects": [child, parent] }))?;
                exec(doc, "object.parent", json!({ "objects": [child], "parent": parent }))?;
            }
            Script::InsertBlob { seed } => {
                exec(
                    doc,
                    "material.setTexture",
                    json!({ "material": "Red", "slot": "baseColor", "base64": base64(&png(*seed)), "name": format!("picture-{seed}.png") }),
                )?;
            }
            _ => return Ok(false),
        }
        exec(doc, "select.none", json!({}))?;
        Ok(true)
    }

    /// The view matches a fresh replica of the same state (ids are the scene's own keys, the
    /// same on every replica).
    fn check_view(&self, doc: &mut SceneDocument) -> Result<()> {
        let snapshot = kition_kit::session::__snapshot(doc)?;
        let blobs = self.blobs(doc)?;
        let fresh = self.join(&snapshot, &blobs)?;
        let (mine, theirs) = (content(doc), content(&fresh));
        if mine != theirs {
            return Err(format!(
                "the view differs from a fresh replica's: {}",
                kition_convergence::state::first_difference(&mine, &theirs).unwrap_or_default()
            ));
        }
        Ok(())
    }

    fn blobs(&self, doc: &mut SceneDocument) -> Result<Blobs> {
        decode_blobs(&doc.blobs_bundle())
    }

    fn add_blobs(&self, doc: &mut SceneDocument, blobs: &Blobs) -> Result<()> {
        doc.add_blobs(&encode_blobs(blobs)).map(|_| ())
    }

    fn export_file(&self, doc: &mut SceneDocument) -> Result<Option<Vec<u8>>> {
        doc.save(None).map(Some)
    }

    /// A nudge of one object and the repaint of the viewport.
    fn keystroke(&self, doc: &mut SceneDocument, rng: &mut Rng) -> Result<()> {
        if let Some(&(id, _)) = rng.pick(&objects(doc)) {
            let _ = doc.execute("mode.set", json!({ "mode": "object" }));
            exec(doc, "select.objects", json!({ "objects": [id] }))?;
            exec(doc, "object.translate", json!({ "objects": [id], "offset": [0.1, 0, 0] }))?;
        }
        let (w, h) = (doc.width(), doc.height());
        doc.render(0.0, 0.0, 1.0, w, h).map(|_| ())
    }
}
