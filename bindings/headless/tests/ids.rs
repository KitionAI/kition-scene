//! One id space at the agent's tools (task card C8-scene): `scene_inspect` lists objects and
//! materials by the engine's ids, commands take them, and receipts, `others`, conflicts and
//! `scene_query` name objects by the same ids (the object model's tool ids, KitObjectModel OM9),
//! numbers as inspect lists them. An id as receipts print it works in the next call unchanged, in
//! `on` and in parameters, as a number or as its digits. The ids are stored in the file, so they
//! survive save, reopen and another program writing the file.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::*;
use serde_json::{Value, json};

/// A receipt names the moved object by the id inspect lists, a number, and says nothing else
/// changed: before, objects were named by KDM object ids (UUIDs no command takes) and every first
/// write of an actor added a change of the scene's `keys` register.
#[tokio::test(flavor = "multi_thread")]
async fn a_receipt_names_the_object_by_the_id_inspect_lists() {
    let f = Fixture::new(3);
    let (details, basis) = f.inspect().await;
    let c0 = ids_by_name(&details)["C0"];
    let ran =
        f.ok("run", json!({ "basis": basis, "steps": [{ "command": "object.translate", "on": [c0], "params": { "offset": [0, 0, 1] } }] })).await;
    let changes = &ran["changes"];
    assert_eq!(changes["changed"].as_array().unwrap().len(), 1, "{changes}");
    assert_eq!(changes["changed"][0]["id"], json!(c0), "{changes}");
    assert_eq!(changes["changed"][0]["type"], "Object");
    assert_eq!(changes["changed"][0]["edits"], json!([{ "field": "location.2", "before": 0, "after": 1 }]), "{changes}");
    f.shutdown().await;
}

/// The id a receipt prints is taken back as it is, a number or its digits, in `on` and in a
/// command's parameter; the rename's receipt names the object by it.
#[tokio::test(flavor = "multi_thread")]
async fn an_id_as_receipts_print_it_works_in_on_and_in_parameters() {
    let f = Fixture::new(2);
    let (details, _) = f.inspect().await;
    let c0 = ids_by_name(&details)["C0"];
    for (n, step) in [
        json!({ "command": "object.rename", "params": { "object": c0, "name": "A" } }),
        json!({ "command": "object.rename", "params": { "object": c0.to_string(), "name": "B" } }),
        json!({ "command": "object.rename", "on": [c0.to_string()], "params": { "name": "C" } }),
        json!({ "command": "object.hide", "params": { "objects": [c0.to_string()] } }),
        json!({ "command": "object.reveal", "params": { "objects": c0.to_string() } }),
    ]
    .into_iter()
    .enumerate()
    {
        let ran = f.ok("run", json!({ "steps": [step] })).await;
        assert_eq!(ids(&ran["changes"]["changed"]), [c0.to_string()], "step {n}: {ran}");
    }
    assert_eq!(object_on_disk(&f.file(), c0).unwrap()["name"], "C");
    f.shutdown().await;
}

/// `scene_query` takes the id inspect lists and answers the object with it; its material is the
/// material's id, which `scene_query` takes too.
#[tokio::test(flavor = "multi_thread")]
async fn query_takes_and_answers_the_ids_inspect_lists() {
    let f = Fixture::new(1);
    let (details, _) = f.inspect().await;
    let cube = ids_by_name(&details)["Cube"];
    let material = details["materials"][0]["id"].as_u64().unwrap();
    let got = f.ok("query", json!({ "objects": [cube] })).await;
    let object = &got["objects"][0];
    assert_eq!(object["id"], json!(cube), "{got}");
    assert_eq!(object["fields"]["material"], json!(material), "{got}");
    let got = f.ok("query", json!({ "objects": [material] })).await;
    assert_eq!(got["objects"][0]["type"], "Material", "{got}");
    f.shutdown().await;
}

/// A colleague's change made in the file is told in `others` by the id inspect lists, and only
/// that object: the colleague's engine opening the file as a new actor adds no change of the
/// scene's own registers.
#[tokio::test(flavor = "multi_thread")]
async fn others_names_a_colleagues_object_by_its_id() {
    let f = Fixture::new(3);
    let (details, basis) = f.inspect().await;
    let names = ids_by_name(&details);
    f.colleague(&[("object.rename", json!({ "object": names["C1"], "name": "X" }))]);
    let ran =
        f.ok("run", json!({ "basis": basis, "steps": [{ "command": "object.rename", "params": { "object": names["C0"], "name": "Mine" } }] })).await;
    let others = &ran["others"];
    assert_eq!(others["by"], json!(["outside"]), "{ran}");
    assert_eq!(others["changed"].as_array().unwrap().len(), 1, "{others}");
    assert_eq!(others["changed"][0]["id"], json!(names["C1"]), "{others}");
    assert!(others.get("unnamed").is_none(), "{others}");
    f.shutdown().await;
}

/// The ids survive saving, reopening and another program writing the file again.
#[tokio::test(flavor = "multi_thread")]
async fn the_ids_survive_reopening_and_a_rewrite() {
    let f = Fixture::new(2);
    let (details, _) = f.inspect().await;
    let before: Value = json!(ids_by_name(&details));
    rewrite(&f.file());
    f.colleague(&[]);
    let (details, _) = f.inspect().await;
    assert_eq!(json!(ids_by_name(&details)), before);
    f.shutdown().await;
}

/// Receipts in space (task card C8-scene, part C): the bytes of `changes` (default cap, and
/// `changes: "all"`) for writes on a scene, and what they say. Printed as a table; asserted: what
/// a receipt names.
#[tokio::test(flavor = "multi_thread")]
async fn receipts_in_space() {
    use kition_kit::KitDocument;
    let mut doc = kition_scene_kit::SceneDocument::create(FILE, 320, 180).unwrap();
    doc.execute("add.empty", json!({ "name": "Parent5" })).unwrap();
    doc.execute("add.empty", json!({ "name": "Parent200", "location": [0, 50, 0] })).unwrap();
    for n in 0..200 {
        doc.execute(
            "add.cube",
            json!({ "name": format!("K{n}"), "size": 0.2, "location": [0.5 * (n % 20) as f64, 50.0 + 0.5 * (n / 20) as f64, 0] }),
        )
        .unwrap();
    }
    for n in 0..5 {
        doc.execute("add.cube", json!({ "name": format!("F{n}"), "size": 0.2, "location": [n as f64, -5, 0] })).unwrap();
    }
    let kids200: Vec<String> = (0..200).map(|n| format!("K{n}")).collect();
    let kids5: Vec<String> = (0..5).map(|n| format!("F{n}")).collect();
    doc.execute("object.parent", json!({ "objects": kids200, "parent": "Parent200" })).unwrap();
    doc.execute("object.parent", json!({ "objects": kids5, "parent": "Parent5" })).unwrap();
    // A 2,000-face grid (50 x 40) and its 2,091 vertices; a material shared by 50 cubes.
    doc.execute("add.grid", json!({ "name": "Grid", "xSubdivisions": 50, "ySubdivisions": 40, "location": [0, -20, 0] })).unwrap();
    doc.execute("material.new", json!({ "name": "Shared", "assign": false })).unwrap();
    let fifty: Vec<String> = (0..50).map(|n| format!("K{n}")).collect();
    doc.execute("material.assign", json!({ "material": "Shared", "objects": fifty })).unwrap();
    let bytes = doc.save(None).unwrap();
    let f = Fixture::from_bytes(&bytes, kition_kits::tools::CallOptions::default());
    let (details, _) = f.inspect().await;
    let names = ids_by_name(&details);
    let grid = &details["objects"].as_array().unwrap().iter().find(|o| o["name"] == "Grid").unwrap()["mesh"];
    println!("receipts: grid {} faces, {} vertices", grid["faces"], grid["vertices"]);
    let cube = names["Cube"];
    let rows: Vec<(&str, Value)> = vec![
        ("move one object", json!({ "command": "object.translate", "on": [cube], "params": { "offset": [1, 0, 0] } })),
        ("move a parent of 5", json!({ "command": "object.translate", "on": [names["Parent5"]], "params": { "offset": [0, 0, 1] } })),
        ("move a parent of 200", json!({ "command": "object.translate", "on": [names["Parent200"]], "params": { "offset": [0, 0, 1] } })),
        (
            "move 5 objects by one offset",
            json!({ "command": "object.translate", "params": { "objects": (0..5).map(|n| format!("F{n}")).collect::<Vec<_>>(), "offset": [0, 0, 1] } }),
        ),
        (
            "move 200 objects by one offset",
            json!({ "command": "object.translate", "params": { "objects": (0..200).map(|n| format!("K{n}")).collect::<Vec<_>>(), "offset": [0, 0, 1] } }),
        ),
        (
            "move one vertex",
            json!({ "command": "mesh.translate", "params": { "elements": { "object": cube, "vertices": [0] }, "offset": [0, 0, 0.1] } }),
        ),
        (
            "move 5 of 2,091 vertices",
            json!({ "command": "mesh.translate", "params": { "elements": { "object": names["Grid"], "vertices": [0, 1, 2, 3, 4] }, "offset": [0, 0, 0.1] } }),
        ),
        (
            "move 2,000 of 2,091 vertices",
            json!({ "command": "mesh.translate", "params": { "elements": { "object": names["Grid"], "vertices": (0..2000).collect::<Vec<u32>>() }, "offset": [0, 0, 0.1] } }),
        ),
        ("subdivide a 6-face cube", json!({ "command": "mesh.subdivide", "params": { "elements": { "object": cube } } })),
        ("subdivide a 2,000-face grid", json!({ "command": "mesh.subdivide", "params": { "elements": { "object": names["Grid"] } } })),
        ("a material shared by 50 objects", json!({ "command": "material.set", "params": { "material": "Shared", "roughness": 0.2 } })),
        ("reparent one object", json!({ "command": "object.parent", "params": { "objects": [cube], "parent": names["Parent5"] } })),
    ];
    println!("receipts: write | bytes | bytes (all) | objects named | what it says");
    for (name, step) in rows {
        let ran = f.ok("run", json!({ "steps": [step.clone()] })).await;
        f.ok("run", json!({ "steps": [{ "command": "edit.undo", "params": { "scope": "agent" } }] })).await;
        let all = f.ok("run", json!({ "steps": [step], "changes": "all" })).await;
        f.ok("run", json!({ "steps": [{ "command": "edit.undo", "params": { "scope": "agent" } }] })).await;
        let changes = &ran["changes"];
        let named: usize = ["created", "changed", "moved", "deleted"].iter().map(|list| changes[*list].as_array().map_or(0, Vec::len)).sum();
        let says: Vec<String> = changes["changed"]
            .as_array()
            .unwrap()
            .iter()
            .take(2)
            .map(|c| format!("{} {}", c["id"], c["edits"].to_string().chars().take(120).collect::<String>()))
            .collect();
        println!(
            "receipts: {name} | {} | {} | {named} | {}{}",
            changes.to_string().len(),
            all["changes"].to_string().len(),
            says.join("; "),
            changes.get("footprint").map(|f| format!(" footprint {f}")).unwrap_or_default()
        );
        if name.starts_with("move a parent") || name.starts_with("a material shared") {
            assert_eq!(named, 1, "{name}: the children and the material's users are not named: {changes}");
        }
    }
    f.shutdown().await;
}
