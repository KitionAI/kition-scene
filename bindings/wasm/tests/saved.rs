//! In memory = saved (task card C8-scene): after each of a representative set of agent commands,
//! what the document holds is what its saved file opens as. The premise guard compares the
//! document the kit holds with the document the kit reads from a file another program saved
//! (kition-kits `outside_change`), so every difference between the two would read as a change by
//! others, and a write on an object that differs only so would be refused. Two things are
//! compared after each command: the object model (the replica, paired as the guard pairs two reads
//! of a file, `kition_kdm::reread::diff_paired`), and the engine's scene, which the next command
//! runs on (every value, floats exactly: a value that changes on its way through the file reads
//! as a mismatch).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use kition_kit::KitDocument;
use kition_kit::kdm::reread;
use kition_kit::session::model::Exchange;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

const NAME: &str = "Scene.kscene";

fn state(doc: &mut SceneDocument) -> Value {
    Exchange::new().replica(doc).unwrap().state_json().unwrap()
}

/// The leaves where two JSON values differ, `path: before -> after`.
fn leaves(path: &str, a: &Value, b: &Value, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for key in x.keys().chain(y.keys().filter(|key| !x.contains_key(*key))) {
                leaves(&format!("{path}.{key}"), x.get(key).unwrap_or(&Value::Null), y.get(key).unwrap_or(&Value::Null), out);
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (index, (p, q)) in x.iter().zip(y).enumerate() {
                leaves(&format!("{path}.{index}"), p, q, out);
            }
        }
        _ if a != b => {
            let cut = |value: &Value| value.to_string().chars().take(160).collect::<String>();
            out.push(format!("{path}: {} -> {}", cut(a), cut(b)));
        }
        _ => {}
    }
}

/// The mismatches between `doc` and its saved file opened again: the object model's, then the
/// engine scene's.
fn mismatches(doc: &mut SceneDocument) -> Vec<String> {
    let bytes = doc.save_file(Some(NAME)).unwrap();
    let mut again = SceneDocument::open(NAME, &bytes).unwrap();
    let schema = kition_scene_kdm::schema();
    let diff = reread::diff_paired(&schema, &state(doc), &state(&mut again)).to_json();
    let mut out = Vec::new();
    for list in ["created", "changed", "moved", "deleted"] {
        for entry in diff[list].as_array().into_iter().flatten() {
            out.push(format!("model {list} {} {} {}", entry["id"], entry["type"], entry["edits"]));
        }
    }
    let (memory, saved) = (serde_json::to_value(doc.scene()).unwrap(), serde_json::to_value(again.scene()).unwrap());
    let mut scene = Vec::new();
    leaves("scene", &memory, &saved, &mut scene);
    // The id allocator is each replica's own (an actor's range), never stored.
    out.extend(scene.into_iter().filter(|line| !line.starts_with("scene.allocator")).map(|line| format!("engine {line}")));
    out
}

/// The representative commands, in order, on one document: what agents do most (add, move, turn,
/// scale by fractions, parent, materials, modifiers, edit-mode modelling by elements, keys,
/// lights, cameras, curves, text, armatures, constraints, UVs, smooth shading, duplicates and
/// deletes).
fn steps() -> Vec<(&'static str, Value)> {
    vec![
        ("object.addUvSphere", json!({ "name": "Ball", "location": [0.1, 0.2, 0.3], "radius": 0.7 })),
        ("object.addCylinder", json!({ "name": "Post", "location": [-2.5, 1.0 / 3.0, 0.0], "rotation": [10, 20, 33.3] })),
        ("object.translate", json!({ "objects": ["Ball"], "offset": [0.1, 0.1, 0.1] })),
        ("object.rotate", json!({ "objects": ["Ball"], "axis": "z", "angle": 17.5 })),
        ("object.scale", json!({ "objects": ["Post"], "factor": 1.1 })),
        (
            "object.transform",
            json!({ "objects": ["Cube"], "location": [1.234_567, -0.000_1, 2.0], "rotation": [0.1, 45, 359.9], "scale": [0.3, 0.3, 0.3] }),
        ),
        ("object.parent", json!({ "objects": ["Ball"], "parent": "Post" })),
        ("material.new", json!({ "name": "Red", "baseColor": [0.9, 0.1, 0.05], "metallic": 0.3, "roughness": 0.45, "assign": false })),
        ("material.assign", json!({ "material": "Red", "objects": ["Ball", "Post"] })),
        ("material.set", json!({ "material": "Red", "emission": [0.1, 0.2, 0.3], "emissionStrength": 1.5, "ior": 1.45 })),
        ("modifier.add", json!({ "object": "Post", "type": "bevel", "width": 0.05, "segments": 3 })),
        ("modifier.add", json!({ "object": "Cube", "type": "subdivision", "levels": 2 })),
        ("mesh.extrude", json!({ "elements": { "object": "Cube", "faces": [0] }, "distance": 0.37 })),
        ("mesh.subdivide", json!({ "elements": { "object": "Cube", "faces": [1, 2] } })),
        ("mesh.bevel", json!({ "elements": { "object": "Post", "faces": [0] }, "width": 0.1, "segments": 2 })),
        ("mesh.translate", json!({ "elements": { "object": "Ball", "vertices": [0, 1, 2] }, "offset": [0.01, 0.02, 0.03] })),
        ("mesh.deleteFaces", json!({ "elements": { "object": "Ball", "faces": [4] } })),
        ("object.shadeSmooth", json!({ "objects": ["Ball"] })),
        ("uv.smartProject", json!({ "objects": ["Post"] })),
        ("animation.insertKey", json!({ "objects": ["Ball"], "properties": ["location", "rotation"], "frame": 1 })),
        ("object.translate", json!({ "objects": ["Ball"], "offset": [0.5, 0.0, 0.25] })),
        ("animation.insertKey", json!({ "objects": ["Ball"], "properties": ["location"], "frame": 30 })),
        ("light.set", json!({ "object": "Light", "power": 750.5, "color": [1.0, 0.9, 0.8] })),
        ("camera.set", json!({ "object": "Camera", "lens": 35.5, "fStop": 1.8, "depthOfField": true })),
        ("object.addText", json!({ "name": "Title", "body": "Hello", "location": [0, 0, 2.5] })),
        ("object.addCurveBezier", json!({ "name": "Path", "location": [1, 1, 0] })),
        ("object.addArmature", json!({ "name": "Rig", "location": [0, -2, 0] })),
        ("constraint.add", json!({ "object": "Post", "type": "copyLocation", "target": "Cube", "influence": 0.35 })),
        ("edit.duplicate", json!({ "objects": ["Post"], "offset": [0.75, 0, 0] })),
        ("edit.delete", json!({ "objects": ["Path"] })),
    ]
}

/// The mismatches known and kept by design (docs/design.md section 7.1, "The current frame is
/// session state"), printed as `KNOWN` and not counted: an edit of an animated property without a
/// key stays in the view until the frame changes, as in the reference application, and is never
/// stored: a location, rotation or scale of an object its keys, action or NLA animate. The object models
/// are equal.
fn known(doc: &SceneDocument, line: &str) -> bool {
    let Some(rest) = line.strip_prefix("engine scene.objects.") else { return false };
    let mut parts = rest.split('.');
    let (Some(index), Some(property)) = (parts.next().and_then(|index| index.parse::<usize>().ok()), parts.next()) else { return false };
    let Some(object) = doc.scene().objects.get(index) else { return false };
    ["location", "rotation", "scale"].contains(&property)
        && doc
            .scene()
            .animated_paths()
            .iter()
            .any(|(target, animated, _, _)| *target == object.id && format!("{animated:?}").to_lowercase() == property)
}

/// Each step's mismatches; none is wanted. Prints every one (`MISMATCH <step>: <line>`) and the
/// count, then asserts none.
#[test]
fn the_scene_in_memory_is_the_scene_its_saved_file_opens_as() {
    let mut doc = SceneDocument::create(NAME, 320, 180).unwrap();
    let mut found = Vec::new();
    for line in mismatches(&mut doc) {
        found.push(format!("open/save: {line}"));
    }
    let mut seen = std::collections::BTreeSet::new();
    for (command, params) in steps() {
        doc.execute(command, params.clone()).unwrap_or_else(|error| panic!("{command}: {error}"));
        for line in mismatches(&mut doc) {
            if !seen.insert(line.clone()) {
                continue;
            }
            if known(&doc, &line) {
                eprintln!("KNOWN {command}: {line}");
                continue;
            }
            found.push(format!("{command}: {line}"));
        }
    }
    for line in &found {
        eprintln!("MISMATCH {line}");
    }
    eprintln!("saved: {} mismatches over {} commands", found.len(), steps().len());
    assert!(found.is_empty(), "{} mismatches", found.len());
}

/// The same check on the released files the repository keeps (`tests/fixtures/released-*.kscene`,
/// written by earlier versions): open, save and open again, then a move, a material change and a
/// face extruded on each one's first mesh.
#[test]
fn released_files_in_memory_are_what_their_saved_files_open_as() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    let mut found = Vec::new();
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().filter_map(Result::ok).map(|entry| entry.path()).collect();
    files.retain(|path| path.extension().is_some_and(|extension| extension == "kscene"));
    files.sort();
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let mut doc = SceneDocument::open(NAME, &std::fs::read(path).unwrap()).unwrap();
        for line in mismatches(&mut doc) {
            if known(&doc, &line) {
                eprintln!("KNOWN {name} open/save: {line}");
                continue;
            }
            found.push(format!("{name} open/save: {line}"));
        }
        let mesh = doc.scene().objects.iter().find(|object| object.mesh().is_some_and(|mesh| !mesh.faces.is_empty())).map(|object| object.id);
        let material = doc.scene().materials.first().map(|material| material.id);
        let mut steps = Vec::new();
        if let Some(mesh) = mesh {
            steps.push(("object.translate", json!({ "objects": [mesh], "offset": [0.1, 0.2, 0.3] })));
            steps.push(("mesh.extrude", json!({ "elements": { "object": mesh, "faces": [0] }, "distance": 0.25 })));
        }
        if let Some(material) = material {
            steps.push(("material.set", json!({ "material": material, "roughness": 0.33, "baseColor": [0.2, 0.4, 0.6] })));
        }
        for (command, params) in steps {
            doc.execute(command, params).unwrap_or_else(|error| panic!("{name} {command}: {error}"));
            for line in mismatches(&mut doc) {
                if known(&doc, &line) {
                    eprintln!("KNOWN {name} {command}: {line}");
                    continue;
                }
                found.push(format!("{name} {command}: {line}"));
            }
        }
    }
    for line in &found {
        eprintln!("MISMATCH {line}");
    }
    eprintln!("saved: {} mismatches over {} released files", found.len(), files.len());
    assert!(found.is_empty(), "{} mismatches", found.len());
}

/// The derived values (`model.derived`: each mesh's `topology` and `bounds`, C8-scene-2) after each
/// step and on each released file equal those of its saved file opened again: the premise guard
/// keeps the values read from the document the kit holds as `before` and compares those read from
/// a file another program saved, so a difference would read as others' change of `~topology` and
/// refuse every write by element index on that mesh.
#[test]
fn derived_values_in_memory_are_those_of_the_saved_file() {
    let derived = |doc: &SceneDocument| serde_json::to_value(doc.derived(&[], &[]).unwrap()).unwrap();
    let mut found = Vec::new();
    let mut check = |label: &str, doc: &mut SceneDocument| {
        let again = SceneDocument::open(NAME, &doc.save_file(Some(NAME)).unwrap()).unwrap();
        let mut lines = Vec::new();
        leaves("derived", &derived(doc), &derived(&again), &mut lines);
        found.extend(lines.into_iter().map(|line| format!("{label}: {line}")));
    };
    let mut doc = SceneDocument::create(NAME, 320, 180).unwrap();
    check("open/save", &mut doc);
    for (command, params) in steps() {
        doc.execute(command, params.clone()).unwrap_or_else(|error| panic!("{command}: {error}"));
        check(command, &mut doc);
    }
    let values = derived(&doc);
    assert!(values.as_array().unwrap().iter().any(|object| object["values"]["topology"].is_string()), "{values}");
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().filter_map(Result::ok).map(|entry| entry.path()).collect();
    files.retain(|path| path.extension().is_some_and(|extension| extension == "kscene"));
    files.sort();
    for path in &files {
        let mut doc = SceneDocument::open(NAME, &std::fs::read(path).unwrap()).unwrap();
        check(&path.file_name().unwrap().to_string_lossy(), &mut doc);
    }
    for line in &found {
        eprintln!("MISMATCH {line}");
    }
    eprintln!("saved: {} derived mismatches over {} commands and {} released files", found.len(), steps().len(), files.len());
    assert!(found.is_empty(), "{} mismatches", found.len());
}
