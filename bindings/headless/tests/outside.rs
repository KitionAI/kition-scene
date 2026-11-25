//! The premise guard and receipts when the `.kscene` file itself changes (task card C8-scene): a
//! colleague's program, an engine of this kit that shares nothing with the server under test,
//! opens the scene, edits it and writes it back, so the server sees only a changed file. The agent
//! works through the kits' MCP server's tools (`kition_kits::tools::call_as`, the library
//! `kition-kits-mcp` serves) on the `kition-scene` executable, holding the ids and the `revision`
//! of an inspect made before the colleague's edit.
//!
//! Kition Scene's addresses (C8-scene-2): objects by id (tool ids, stored in the file), each write holding
//! what its command reads or replaces; object names (the premise holds the name of the object a
//! name names now); a mesh's vertices and faces by index (`elements`, whose premise is the mesh's
//! derived `~topology`); an object by a world point (`pick`, a place told by `covers`: the guard
//! asks the kit what was at the point at `basis`). An object's location, rotation, scale, parent and
//! parent inverse are its `position` fields: a colleague's change of one is a move (`relocated`),
//! which a position-independent command by id (a name, a material slot) is not refused for.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::collections::BTreeMap;
use std::time::Instant;

use common::*;
use kition_kits::tools::{CallOptions, Guard};
use serde_json::{Value, json};

const CUBES: usize = 8;

fn rename(object: impl Into<Value>, name: &str) -> Value {
    json!({ "command": "object.rename", "params": { "object": object.into(), "name": name } })
}

fn rename_at(at: [f64; 3], name: &str) -> Value {
    json!({ "command": "object.rename", "params": { "pick": at, "name": name } })
}

fn extrude(object: u64, face: u64) -> Value {
    json!({ "command": "mesh.extrude", "params": { "elements": { "object": object, "faces": [face] }, "distance": 0.5 } })
}

/// The faces of mesh `id` in the file, as a new engine reads it (vertex indexes per face).
fn faces_on_disk(file: &std::path::Path, id: u64) -> Vec<Vec<u32>> {
    let doc = on_disk(file);
    doc.scene().object(u32::try_from(id).unwrap()).unwrap().mesh().unwrap().faces.iter().map(|face| face.to_vec()).collect()
}

fn location_on_disk(file: &std::path::Path, id: u64) -> Value {
    object_on_disk(file, id).unwrap()["location"].clone()
}

fn name_on_disk(file: &std::path::Path, id: u64) -> String {
    object_on_disk(file, id).unwrap()["name"].as_str().unwrap().to_owned()
}

/// The conflicts' reasons, joined.
fn reasons(refused: &Value) -> String {
    refused["conflicts"].as_array().unwrap().iter().map(|c| c["reason"].as_str().unwrap().to_owned()).collect::<Vec<_>>().join("+")
}

// ---- 1. An unrelated object ----

/// The colleague renames C5 and moves C6; the agent's writes on C2 with `basis` apply (a rename,
/// and a move by id through `on`), and `others` names the colleague's objects only.
#[tokio::test(flavor = "multi_thread")]
async fn s1_a_colleagues_change_of_another_object_applies_and_others_names_it() {
    let f = Fixture::new(CUBES);
    let (details, basis) = f.inspect().await;
    let names = ids_by_name(&details);
    let (mine, renamed, moved) = (names["C2"], names["C5"], names["C6"]);
    f.colleague(&[("object.rename", json!({ "object": renamed, "name": "Theirs" }))]);
    let ran = f.ok("run", json!({ "basis": basis, "steps": [rename(mine, "Mine")] })).await;
    let others = &ran["others"];
    assert_eq!(others["by"], json!(["outside"]), "{ran}");
    assert_eq!(ids(&others["changed"]), [renamed.to_string()], "{others}");
    assert!(others.get("unnamed").is_none(), "{others}");
    println!("s1: others for one unrelated rename: {} bytes: {others}", others.to_string().len());
    f.colleague(&[("object.translate", json!({ "objects": [moved], "offset": [0, 0, 2] }))]);
    let ran = f
        .ok("run", json!({ "basis": revision(&ran), "steps": [{ "command": "object.translate", "on": [mine], "params": { "offset": [1, 0, 0] } }] }))
        .await;
    assert_eq!(ids(&ran["others"]["changed"]), [moved.to_string()], "{ran}");
    println!("s1: others for one unrelated move: {} bytes: {}", ran["others"].to_string().len(), ran["others"]);
    assert_eq!(name_on_disk(&f.file(), mine), "Mine");
    assert_eq!(location_on_disk(&f.file(), mine), json!([10.0, 0.0, 0.0]));
    f.shutdown().await;
}

// ---- 2. The target's same field ----

/// The colleague renames the target, then the agent renames it with the `basis` of an inspect that
/// showed the old name: refused as `changed`, nothing written; the retry after an inspect applies.
/// The same for the target's location along one axis: refused as `relocated` (C8-fix-8: a leaf of a
/// position field, `location.1`, is a move; it was `changed`).
#[tokio::test(flavor = "multi_thread")]
async fn s2_a_colleagues_change_of_the_targets_field_is_refused_and_the_retry_applies() {
    let f = Fixture::new(CUBES);
    let (details, basis) = f.inspect().await;
    let mine = ids_by_name(&details)["C2"];
    f.colleague(&[("object.rename", json!({ "object": mine, "name": "Theirs" }))]);
    let before = std::fs::read(f.file()).unwrap();
    let refused = f.refused("run", json!({ "basis": basis, "steps": [rename(mine, "Mine")] })).await;
    assert_conflict(&refused, &mine.to_string(), "changed");
    assert_eq!(std::fs::read(f.file()).unwrap(), before, "nothing written");
    println!("s2: refused changed: {} bytes: {refused}", refused.to_string().len());
    let (_, basis) = f.inspect().await;
    f.ok("run", json!({ "basis": basis, "steps": [rename(mine, "Mine")] })).await;
    assert_eq!(name_on_disk(&f.file(), mine), "Mine");

    let (_, basis) = f.inspect().await;
    f.colleague(&[("object.transform", json!({ "objects": [mine], "location": [9, 9, 0] }))]);
    // C2 stood at [9, 0, 0]: one coordinate changes.
    let step = json!({ "command": "object.transform", "on": [mine], "params": { "location": [1, 1, 0] } });
    let refused = f.refused("run", json!({ "basis": basis, "steps": [step] })).await;
    assert_conflict(&refused, &mine.to_string(), "relocated");
    println!("s2: location: refused {}", refused["conflicts"]);
    assert_eq!(location_on_disk(&f.file(), mine), json!([9.0, 9.0, 0.0]), "the colleague's location kept");
    f.shutdown().await;
}

/// The colleague hides the target; the agent hides it too, naming it in `objects` (one id, a name,
/// or a list: declared as the command's targets since C8-fix-8). Given as a number, as its digits,
/// or in `on`, the write is refused as `changed` (its premise is the target's `hidden`,
/// C8-scene-2). The colleague's rename of the target alone is outside that premise: the same write
/// as a number applies.
#[tokio::test(flavor = "multi_thread")]
async fn s2b_an_id_as_a_number_in_objects_is_a_premise() {
    let f = Fixture::new(CUBES);
    let (details, basis) = f.inspect().await;
    let mine = ids_by_name(&details)["C2"];
    f.colleague(&[("object.hide", json!({ "objects": [mine] }))]);
    for (how, step) in [
        ("digits in objects", json!({ "command": "object.hide", "params": { "objects": [mine.to_string()] } })),
        ("on", json!({ "command": "object.hide", "on": [mine] })),
        ("a number in objects", json!({ "command": "object.hide", "params": { "objects": [mine] } })),
    ] {
        let refused = f.refused("run", json!({ "basis": basis, "steps": [step] })).await;
        assert_conflict(&refused, &mine.to_string(), "changed");
        println!("s2b: {how}: refused");
    }
    f.shutdown().await;
    let f = Fixture::new(CUBES);
    let (details, basis) = f.inspect().await;
    let mine = ids_by_name(&details)["C2"];
    f.colleague(&[("object.rename", json!({ "object": mine, "name": "Theirs" }))]);
    let ran = f.ok("run", json!({ "basis": basis, "steps": [{ "command": "object.hide", "params": { "objects": [mine] } }] })).await;
    assert_eq!(ids(&ran["others"]["changed"]), [mine.to_string()], "{ran}");
    println!("s2b: a number in objects after a rename: applied");
    f.shutdown().await;
}

// ---- 3. The target deleted ----

#[tokio::test(flavor = "multi_thread")]
async fn s3_a_colleague_deleting_the_target_is_refused_as_deleted() {
    let f = Fixture::new(CUBES);
    let (details, basis) = f.inspect().await;
    let mine = ids_by_name(&details)["C2"];
    f.colleague(&[("edit.delete", json!({ "objects": [mine] }))]);
    let before = std::fs::read(f.file()).unwrap();
    let refused = f.refused("run", json!({ "basis": basis, "steps": [rename(mine, "Mine")] })).await;
    assert_conflict(&refused, &mine.to_string(), "deleted");
    assert_eq!(std::fs::read(f.file()).unwrap(), before, "nothing written");
    println!("s3: refused deleted: {}", refused["conflicts"]);
    f.shutdown().await;
}

// ---- 4. The target moved ----
//
// C8-fix-8: a change of one coordinate of a `position` json field is told at its leaf
// (`location.0`) and is a move, as a move along three axes (told whole) is. The scenarios below
// move along one axis.

/// The colleague moves the target C2 30 m along x; the agent renames it by id (`object.rename`,
/// declared position-independent): applied, `others` telling the move.
#[tokio::test(flavor = "multi_thread")]
async fn s4a_a_rename_by_id_after_a_move_of_the_target_applies() {
    let f = Fixture::new(CUBES);
    let (details, basis) = f.inspect().await;
    let mine = ids_by_name(&details)["C2"];
    f.colleague(&[("object.translate", json!({ "objects": [mine], "offset": [30, 0, 0] }))]);
    let ran = f.ok("run", json!({ "basis": basis, "steps": [rename(mine, "Mine")] })).await;
    assert_eq!(ids(&ran["others"]["changed"]), [mine.to_string()], "{ran}");
    println!("s4a: rename by id after a move along x: applied; others {}", ran["others"]);
    // Moved along all three axes, the move is told whole and is a move: the rename applies.
    let (_, basis) = f.inspect().await;
    f.colleague(&[("object.translate", json!({ "objects": [mine], "offset": [1, 1, 1] }))]);
    let ran = f.ok("run", json!({ "basis": basis, "steps": [rename(mine, "Mine")] })).await;
    println!("s4a: rename by id after a move along x, y and z: applied; others {}", ran["others"]);
    f.shutdown().await;
}

/// The same move; a move by id relative to where the target was (`object.translate` through
/// `on`) is refused as `relocated` (it was `changed`).
#[tokio::test(flavor = "multi_thread")]
async fn s4b_a_move_by_id_after_a_move_of_the_target_is_refused() {
    let f = Fixture::new(CUBES);
    let (details, basis) = f.inspect().await;
    let mine = ids_by_name(&details)["C2"];
    f.colleague(&[("object.translate", json!({ "objects": [mine], "offset": [30, 0, 0] }))]);
    let refused = f
        .refused("run", json!({ "basis": basis, "steps": [{ "command": "object.translate", "on": [mine], "params": { "offset": [1, 0, 0] } }] }))
        .await;
    assert_conflict(&refused, &mine.to_string(), "relocated");
    assert_eq!(location_on_disk(&f.file(), mine), json!([39.0, 0.0, 0.0]), "nothing written");
    println!("s4b: translate by id refused: {}", refused["conflicts"]);
    f.shutdown().await;
}

/// The same move; a write by the world point where the target stood (`pick`): refused as
/// `relocated` by the guard, naming the target and where it went (`now`); nothing written.
#[tokio::test(flavor = "multi_thread")]
async fn s4c_a_write_by_point_after_a_move_of_the_target_is_refused() {
    let f = Fixture::new(CUBES);
    let (details, basis) = f.inspect().await;
    let mine = ids_by_name(&details)["C2"];
    f.colleague(&[("object.translate", json!({ "objects": [mine], "offset": [30, 0, 0] }))]);
    let before = std::fs::read(f.file()).unwrap();
    let refused = f.refused("run", json!({ "basis": basis, "steps": [rename_at(spot(2), "Picked")] })).await;
    let conflict = assert_conflict(&refused, &mine.to_string(), "relocated");
    assert_eq!(conflict["now"]["location"][0].as_f64(), Some(39.0), "{refused}");
    assert_eq!(std::fs::read(f.file()).unwrap(), before, "nothing written");
    println!("s4c: by point: refused, {} bytes: {}", refused.to_string().len(), refused["conflicts"]);
    f.shutdown().await;
}

/// The colleague moves C2 away and C3 to where C2 stood (along x both); the agent's write by that
/// point, which names C3 now, is refused as `relocated` naming C3. Moved along all three axes, the
/// same.
#[tokio::test(flavor = "multi_thread")]
async fn s4d_a_write_by_point_after_another_object_took_the_place_is_refused() {
    let f = Fixture::new(CUBES);
    let (details, basis) = f.inspect().await;
    let names = ids_by_name(&details);
    let (mine, other) = (names["C2"], names["C3"]);
    f.colleague(&[
        ("object.translate", json!({ "objects": [mine], "offset": [60, 0, 0] })),
        ("object.translate", json!({ "objects": [other], "offset": [-3, 0, 0] })),
    ]);
    let refused = f.refused("run", json!({ "basis": basis, "steps": [rename_at(spot(2), "Picked")] })).await;
    let conflict = assert_conflict(&refused, &other.to_string(), "relocated");
    assert_eq!(conflict["place"]["under"], json!([other]), "{refused}");
    assert_eq!(name_on_disk(&f.file(), other), "C3", "nothing written");
    println!("s4d: by point, another object moved there along x: refused {}", refused["conflicts"]);
    let (_, basis) = f.inspect().await;
    let c4 = names["C4"];
    f.colleague(&[
        ("object.translate", json!({ "objects": [other], "offset": [0, 30, 1] })),
        ("object.transform", json!({ "objects": [c4], "location": [spot(2)[0], spot(2)[1], 0.25] })),
    ]);
    let before = std::fs::read(f.file()).unwrap();
    let refused = f.refused("run", json!({ "basis": basis, "steps": [rename_at(spot(2), "Picked2")] })).await;
    assert_eq!(refused["error"]["code"], "conflict", "{refused}");
    assert_eq!(std::fs::read(f.file()).unwrap(), before, "nothing written");
    println!("s4d: by point, another object moved there along x, y and z: refused {}", refused["conflicts"]);
    f.shutdown().await;
}

// ---- 5. The target mesh's topology ----

/// The colleague subdivides the target's mesh (6 faces become 24), or deletes one of its faces:
/// the agent's extrude of face 0 by index (`elements`) is refused as `changed` on the mesh's
/// topology (`~topology`, C8-fix-8; it was `relocated` on the whole mesh), and
/// the saved file proves nothing was written; a rename of the object by id applies.
#[tokio::test(flavor = "multi_thread")]
async fn s5_a_topology_change_refuses_a_write_by_element_index() {
    for theirs in [json!({ "command": "mesh.subdivide" }), json!({ "command": "mesh.deleteFaces", "faces": [3] })] {
        let f = Fixture::new(CUBES);
        let (details, basis) = f.inspect().await;
        let mine = ids_by_name(&details)["C2"];
        let command = theirs["command"].as_str().unwrap();
        let mut elements = json!({ "object": mine });
        if let Some(faces) = theirs.get("faces") {
            elements["faces"] = faces.clone();
        }
        f.colleague(&[(command, json!({ "elements": elements }))]);
        let before = std::fs::read(f.file()).unwrap();
        let faces = faces_on_disk(&f.file(), mine);
        let refused = f.refused("run", json!({ "basis": basis, "steps": [extrude(mine, 0)] })).await;
        let conflict = assert_conflict(&refused, &mine.to_string(), "changed");
        assert!(refused["others"]["changed"][0]["fields"].as_array().unwrap().contains(&json!("~topology")), "{refused}");
        assert_eq!(std::fs::read(f.file()).unwrap(), before, "{command}: nothing written");
        assert_eq!(faces_on_disk(&f.file(), mine), faces);
        println!("s5 {command}: extrude by face index refused relocated: {conflict}");
        let ran = f.ok("run", json!({ "basis": basis, "steps": [rename(mine, "Mine")] })).await;
        println!("s5 {command}: rename by id applied; others {} bytes", ran["others"].to_string().len());
        f.shutdown().await;
    }
}

/// The colleague moves one vertex of the target's mesh (its topology stays): the agent's extrude
/// of face 0 still names the same face, and applies (its premise is the mesh's topology,
/// `~topology`, a derived value, C8-fix-8).
#[tokio::test(flavor = "multi_thread")]
async fn s5b_a_vertex_move_keeps_a_write_by_element_index() {
    let f = Fixture::new(CUBES);
    let (details, basis) = f.inspect().await;
    let mine = ids_by_name(&details)["C2"];
    f.colleague(&[("mesh.translate", json!({ "elements": { "object": mine, "vertices": [7] }, "offset": [0, 0, 0.25] }))]);
    let ran = f.ok("run", json!({ "basis": basis, "steps": [extrude(mine, 0)] })).await;
    println!("s5b: extrude after a vertex move: applied; others {}", ran["others"]);
    f.shutdown().await;
}

// ---- 6. The target's parent ----

/// C2 is C3's child. The colleague moves C3 20 m along y (C2's world position changes, its stored
/// location does not): a write by the point where C2 stood is refused as `relocated` on C3 (what
/// was at the point derives its place from it, `context.covered`); nothing is written. A move of
/// C2 by id applies: C2's own fields did not change, and `others` names C3 alone, so the agent is
/// told how many objects depend on that parent through `carries`. The colleague then reparents C2 to C4: C2 keeps its
/// world place (Keep Transform, C8-scene-engine), so a write by C2's point still names C2 and applies.
#[tokio::test(flavor = "multi_thread")]
async fn s6_a_parents_move_refuses_a_write_by_point_a_reparent_in_place_does_not() {
    let f = Fixture::new(CUBES);
    let (details, _) = f.inspect().await;
    let names = ids_by_name(&details);
    let (mine, parent, other) = (names["C2"], names["C3"], names["C4"]);
    f.ok("run", json!({ "steps": [{ "command": "object.parent", "params": { "objects": [mine], "parent": parent } }] })).await;
    let (_, basis) = f.inspect().await;
    f.colleague(&[("object.translate", json!({ "objects": [parent], "offset": [0, 20, 0] }))]);
    let before = std::fs::read(f.file()).unwrap();
    let at_point = f.refused("run", json!({ "basis": basis, "steps": [rename_at(spot(2), "Picked")] })).await;
    assert_conflict(&at_point, &parent.to_string(), "relocated");
    assert_eq!(std::fs::read(f.file()).unwrap(), before, "nothing written");
    println!("s6: parent moved: by point refused: {}", at_point["conflicts"]);
    let by_id =
        f.ok("run", json!({ "basis": basis, "steps": [{ "command": "object.translate", "on": [mine], "params": { "offset": [0, 0, 1] } }] })).await;
    assert_eq!(ids(&by_id["others"]["changed"]), [parent.to_string()], "{by_id}");
    println!("s6: parent moved: translate by id applied; others {}", by_id["others"]);
    let (_, basis) = f.inspect().await;
    f.colleague(&[("object.parent", json!({ "objects": [mine], "parent": other }))]);
    // changed by C8-scene-engine: the reparent used to make C2 jump (its world became its local
    // fields), so the point no longer named it and the write was refused; C2 now stays put.
    let kept = f.ok("run", json!({ "basis": basis, "steps": [rename_at([spot(2)[0], spot(2)[1] + 20.0, 1.0], "Picked")] })).await;
    assert_eq!(kept["completed"][0]["result"]["object"], json!(mine), "{kept}");
    println!("s6: reparented in place: by point applied; others {}", kept["others"]);
    f.shutdown().await;
}

// ---- 7. Names and indexes that name another object now ----

/// A name swap by another agent or in the file is a content change: a rename by id is
/// refused as `changed`, never `unknown` for renumbering. A write by the name is refused
/// as `relocated`, because it now names the other object. Neither writes anything.
#[tokio::test(flavor = "multi_thread")]
async fn s7a_a_write_by_a_name_another_object_took_is_refused() {
    let swap = |c2: u64, c3: u64| {
        [
            ("object.rename", json!({ "object": c2, "name": "Swap" })),
            ("object.rename", json!({ "object": c3, "name": "C2" })),
            ("object.rename", json!({ "object": c2, "name": "C3" })),
        ]
    };
    for how in ["another agent", "the file", "the file, cubes told apart"] {
        let f = Fixture::new(CUBES);
        let (details, _) = f.inspect().await;
        let names = ids_by_name(&details);
        let (c2, c3) = (names["C2"], names["C3"]);
        if how == "the file, cubes told apart" {
            f.ok("run", json!({ "steps": [{ "command": "modifier.add", "params": { "object": c2, "type": "bevel" } }] })).await;
        }
        let (_, basis) = f.inspect().await;
        if how == "another agent" {
            let steps: Vec<Value> = swap(c2, c3).into_iter().map(|(command, params)| json!({ "command": command, "params": params })).collect();
            f.call_by("agent-other", "run", json!({ "steps": steps })).await;
        } else {
            f.colleague(&swap(c2, c3));
        }
        let by_id = f.refused("run", json!({ "basis": basis, "steps": [rename(c3, "Mine")] })).await;
        assert_conflict(&by_id, &c3.to_string(), "changed");
        assert!(by_id["others"].get("renumbered").is_none(), "{by_id}");
        let refused = f.refused("run", json!({ "basis": basis, "steps": [rename("C2", "Mine")] })).await;
        assert_conflict(&refused, &c3.to_string(), "relocated");
        if how != "another agent" {
            assert_eq!(name_on_disk(&f.file(), c3), "C2", "nothing written");
        }
        println!("s7a: by name after a swap by {how}: refused {}", refused["conflicts"]);
        f.shutdown().await;
    }
}

/// The colleague moves C2's second modifier to the top of its stack; the agent sets modifier
/// index 0 (which named the first) on C2 by id: refused as `changed` (C2's stack changed), and the
/// colleague's order is kept.
#[tokio::test(flavor = "multi_thread")]
async fn s7b_a_write_by_a_modifier_index_after_a_reorder_is_refused() {
    let f = Fixture::new(CUBES);
    let (details, _) = f.inspect().await;
    let mine = ids_by_name(&details)["C2"];
    f.ok(
        "run",
        json!({ "steps": [
            { "command": "modifier.add", "params": { "object": mine, "type": "bevel", "width": 0.1 } },
            { "command": "modifier.add", "params": { "object": mine, "type": "subdivision", "levels": 1 } },
        ] }),
    )
    .await;
    let (_, basis) = f.inspect().await;
    f.colleague(&[("modifier.move", json!({ "object": mine, "index": 1, "to": 0 }))]);
    let refused = f
        .refused("run", json!({ "basis": basis, "steps": [{ "command": "modifier.set", "params": { "object": mine, "index": 0, "width": 0.3 } }] }))
        .await;
    assert_conflict(&refused, &mine.to_string(), "changed");
    println!("s7b: modifier index after a reorder: refused {}", refused["conflicts"]);
    f.shutdown().await;
}

// ---- 8. Guard modes ----

#[tokio::test(flavor = "multi_thread")]
async fn s8_epoch_refuses_an_unrelated_change_and_off_overwrites_the_target() {
    let f = Fixture::with(CUBES, CallOptions { guard: Guard::Epoch, ..CallOptions::default() });
    let (details, basis) = f.inspect().await;
    let names = ids_by_name(&details);
    f.colleague(&[("object.rename", json!({ "object": names["C5"], "name": "Theirs" }))]);
    let refused = f.refused("run", json!({ "basis": basis, "steps": [rename(names["C2"], "Mine")] })).await;
    assert_eq!(refused["error"]["code"], "conflict", "{refused}");
    assert_eq!(name_on_disk(&f.file(), names["C2"]), "C2", "nothing written");
    f.shutdown().await;

    let f = Fixture::with(CUBES, CallOptions { guard: Guard::Off, ..CallOptions::default() });
    let (details, basis) = f.inspect().await;
    let mine = ids_by_name(&details)["C2"];
    f.colleague(&[("object.rename", json!({ "object": mine, "name": "Theirs" }))]);
    let ran = f.ok("run", json!({ "basis": basis, "steps": [rename(mine, "Mine")] })).await;
    assert_eq!(ids(&ran["others"]["changed"]), [mine.to_string()], "{ran}");
    assert_eq!(name_on_disk(&f.file(), mine), "Mine", "the colleague's name overwritten");
    f.shutdown().await;
}

// ---- 9. Saved again unchanged ----

/// The file saved again with no change, by the colleague's engine and by a bare rewrite (the ZIP's
/// entries in another order, stored): no notice, no refusal, nothing in `others`.
#[tokio::test(flavor = "multi_thread")]
async fn s9_a_file_saved_again_unchanged_is_no_change() {
    for how in ["engine", "rewrite"] {
        let f = Fixture::new(CUBES);
        let (details, basis) = f.inspect().await;
        let mine = ids_by_name(&details)["C2"];
        let bytes = std::fs::read(f.file()).unwrap();
        let resave = || if how == "engine" { drop(f.colleague(&[])) } else { rewrite(&f.file()) };
        resave();
        println!("s9 {how}: same bytes {}", std::fs::read(f.file()).unwrap() == bytes);
        let ran = f.ok("run", json!({ "basis": basis, "steps": [rename(mine, "Mine")] })).await;
        assert!(ran.get("others").is_none_or(Value::is_null), "{how}: {ran}");
        resave();
        let seen = f.ok("inspect", json!({})).await;
        assert!(seen.get("changedOutside").is_none_or(|told| told.is_null() || told == false), "{how}: {seen}");
        let ran = f.ok("run", json!({ "basis": revision(&seen), "steps": [rename_at(spot(2), "Again")] })).await;
        assert!(ran.get("others").is_none_or(Value::is_null), "{how}: {ran}");
        f.shutdown().await;
    }
}

// ---- 10. False refusals ----

/// One round of s10: the xorshift step.
fn xorshift(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

/// Fifty rounds on 24 cubes: the colleague changes a cube picked at random (a fixed seed) among the
/// 23 other than the agent's target C7, then the agent writes C7 with the revision of its last
/// answer, by id (`object.rename`) and by its world point (`pick`). `change` is what the colleague
/// does: `rename` (no position field) or `move` (0.5 m up: a position field). Answers the refusals
/// (by id, by point).
async fn fifty_rounds(guard: Guard, change: &str) -> (usize, usize) {
    const ROUNDS: u64 = 50;
    let f = Fixture::with(24, CallOptions { guard, ..CallOptions::default() });
    let (details, mut basis) = f.inspect().await;
    let names = ids_by_name(&details);
    let mine = names["C7"];
    let others: Vec<u64> = (0..24).filter(|n| *n != 7).map(|n| names[&format!("C{n}")]).collect();
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    let (mut by_id, mut by_point) = (0, 0);
    for round in 0..ROUNDS {
        let theirs = others[usize::try_from(xorshift(&mut seed) % 23).unwrap()];
        match change {
            "rename" => f.colleague(&[("object.rename", json!({ "object": theirs, "name": format!("R{round}") }))]),
            _ => f.colleague(&[("object.translate", json!({ "objects": [theirs], "offset": [0, 0, 0.5] }))]),
        };
        // Both writes hold the revision from before the colleague's change.
        let seen = basis.clone();
        for (write, count) in [(rename(mine, &format!("A{round}")), &mut by_id), (rename_at(spot(7), &format!("B{round}")), &mut by_point)] {
            match f.call("run", json!({ "basis": seen, "steps": [write] })).await {
                Ok(ran) => basis = revision(&ran),
                Err(error) => {
                    assert_eq!(error["error"]["code"], "conflict", "{error}");
                    *count += 1;
                    basis = revision(&error);
                }
            }
        }
    }
    f.shutdown().await;
    (by_id, by_point)
}

/// The colleague renames another cube each round: premise refuses no write, by id or by point;
/// epoch every one.
#[tokio::test(flavor = "multi_thread")]
async fn s10_fifty_unrelated_renames_refuse_nothing_under_premise_and_everything_under_epoch() {
    let premise = fifty_rounds(Guard::Premise, "rename").await;
    let epoch = fifty_rounds(Guard::Epoch, "rename").await;
    println!("s10 rename others: premise (by id, by point) {premise:?}; epoch {epoch:?}");
    assert_eq!((premise, epoch), ((0, 0), (50, 50)));
}

/// The colleague moves another cube 0.5 m up each round: premise refuses no write, by id or by
/// point (the guard sees the move and asks the kit whether the cube was at the point,
/// C8-fix-8); epoch every one.
#[tokio::test(flavor = "multi_thread")]
async fn s10b_fifty_unrelated_moves_refuse_nothing_under_premise_and_everything_under_epoch() {
    let premise = fifty_rounds(Guard::Premise, "move").await;
    let epoch = fifty_rounds(Guard::Epoch, "move").await;
    println!("s10b move others: premise (by id, by point) {premise:?}; epoch {epoch:?}");
    assert_eq!((premise, epoch), ((0, 0), (50, 50)));
}

/// Fifty rounds as [`fifty_rounds`], by each addressing form (C8-scene-2): the agent writes C7 with
/// the revision of its last answer by id (`object.rename`), by its world point (`pick`), by face
/// index (`mesh.extrude` of face 0, `elements`), and by its name (`material.assign` of "C7", which
/// keeps its name). Answers the refusals per form.
async fn fifty_rounds_by_form(guard: Guard, change: &str) -> [usize; 4] {
    const ROUNDS: u64 = 50;
    let f = Fixture::with(24, CallOptions { guard, ..CallOptions::default() });
    f.ok("run", json!({ "steps": [{ "command": "material.new", "params": { "name": "M", "assign": false } }] })).await;
    let (details, mut basis) = f.inspect().await;
    let names = ids_by_name(&details);
    let mine = names["C7"];
    let others: Vec<u64> = (0..24).filter(|n| *n != 7).map(|n| names[&format!("C{n}")]).collect();
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    let mut refused = [0usize; 4];
    for round in 0..ROUNDS {
        let theirs = others[usize::try_from(xorshift(&mut seed) % 23).unwrap()];
        match change {
            "rename" => f.colleague(&[("object.rename", json!({ "object": theirs, "name": format!("R{round}") }))]),
            _ => f.colleague(&[("object.translate", json!({ "objects": [theirs], "offset": [0, 0, 0.5] }))]),
        };
        let seen = basis.clone();
        let writes = [
            rename(mine, "C7"),
            rename_at(spot(7), "C7"),
            json!({ "command": "mesh.extrude", "params": { "elements": { "object": mine, "faces": [0] }, "distance": 0.01 } }),
            json!({ "command": "material.assign", "params": { "material": "M", "objects": ["C7"] } }),
        ];
        for (form, write) in writes.into_iter().enumerate() {
            match f.call("run", json!({ "basis": seen, "steps": [write] })).await {
                Ok(ran) => basis = revision(&ran),
                Err(error) => {
                    assert_eq!(error["error"]["code"], "conflict", "{error}");
                    refused[form] += 1;
                    basis = revision(&error);
                }
            }
        }
    }
    f.shutdown().await;
    refused
}

/// [`fifty_rounds_by_form`] with renames and with moves of other cubes: premise refuses no write by
/// any form; epoch every one.
#[tokio::test(flavor = "multi_thread")]
async fn s10c_fifty_unrelated_changes_by_every_addressing_form() {
    for change in ["rename", "move"] {
        let premise = fifty_rounds_by_form(Guard::Premise, change).await;
        let epoch = fifty_rounds_by_form(Guard::Epoch, change).await;
        println!("s10c {change} others: refused (by id, by point, by face index, by name): premise {premise:?}; epoch {epoch:?}");
        assert_eq!((premise, epoch), ([0; 4], [50; 4]), "{change}");
    }
}

// ---- 11. Mixed editing ----

/// One row of s11's table: rounds, applied, refused necessary, refused false, applied wrongly,
/// failed at the kit, the refusals' reasons.
type Cell = (usize, usize, usize, usize, usize, usize, BTreeMap<String, usize>);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Theirs {
    /// Another cube moved 0.5 m up.
    MoveOther,
    /// The target moved 30 m along x (away from its point).
    MoveTarget,
    /// The target's mesh subdivided (6 faces to 24).
    TopologyTarget,
    /// Another cube's mesh subdivided.
    TopologyOther,
    /// Another cube deleted.
    DeleteOther,
    /// Another cube parented to a third.
    ReparentOther,
    /// The material every cube shares changed (roughness).
    SharedMaterial,
    /// Another cube moved to the target's point (C8-fix-8).
    MoveIntoTarget,
    /// The target's parent (C11) moved 30 m along y (C8-fix-8).
    MoveParent,
    /// One vertex of the target's mesh moved 0.25 m up (C8-fix-8).
    VertexTarget,
    /// The target's name swapped with another cube's (C8-scene-2).
    RenameSwap,
    /// The target turned 30 degrees about z (C8-scene-2).
    RotateTarget,
    /// A modifier added to the target and moved to the top of its stack (C8-scene-2).
    InsertModifierAbove,
}

/// Fifty rounds of mixed editing, each on a fresh scene of 12 cubes sharing one material, the
/// target C5 (C11's child, with a bevel modifier): the agent inspects; the colleague makes one
/// change ([`Theirs`], picked by a fixed, printed seed); the agent, each with the revision of its
/// inspect, (D) renames the object named "C5" by that name, (A) renames the target by id
/// (`object.rename`, position-independent), (B) renames the object at the target's world point
/// (`pick`), (C) extrudes face 0 of the target's mesh by index (`elements`), (E) sets the width of
/// the target's modifier 0 by index. A refusal is necessary when what the write names changed, or
/// what it replaces: (A) the target deleted (never in this mix) or its name changed (the field a
/// rename replaces: a lost update); (B) the point naming another object than the target, or none,
/// or the target's name changed; (C) the target's faces (its topology) changed; (D) the name
/// naming another object than the target, or none; (E) the modifier at index 0 is another (its
/// name differs).
/// Prints the table (their change, write, rounds, applied, refused necessary, refused false,
/// applied wrongly, failed at the kit, the reasons); asserts no write is applied wrongly.
#[tokio::test(flavor = "multi_thread")]
async fn s11_fifty_rounds_of_mixed_editing() {
    const ROUNDS: usize = 50;
    const N: usize = 12;
    const MINE: usize = 5;
    const SEED: u64 = 0x9E37_79B9_7F4A_7C15;
    let kinds = [
        Theirs::MoveOther,
        Theirs::MoveTarget,
        Theirs::TopologyTarget,
        Theirs::TopologyOther,
        Theirs::DeleteOther,
        Theirs::ReparentOther,
        Theirs::SharedMaterial,
        Theirs::MoveIntoTarget,
        Theirs::MoveParent,
        Theirs::VertexTarget,
        Theirs::RenameSwap,
        Theirs::RotateTarget,
        Theirs::InsertModifierAbove,
    ];
    println!("s11: seed {SEED:#x}");
    let mut doc = scene_doc(N);
    {
        use kition_kit::KitDocument;
        doc.execute("material.new", json!({ "name": "Shared", "assign": false })).unwrap();
        let all: Vec<String> = (0..N).map(|n| format!("C{n}")).collect();
        doc.execute("material.assign", json!({ "material": "Shared", "objects": all })).unwrap();
        // The target is C11's child (its world place unchanged), for `MoveParent`.
        doc.execute("object.parent", json!({ "objects": [format!("C{MINE}")], "parent": format!("C{}", N - 1) })).unwrap();
        doc.execute("modifier.add", json!({ "object": format!("C{MINE}"), "type": "bevel", "width": 0.1 })).unwrap();
        // The session selects nothing: a reread that deletes a selected object is refused by name
        // (kition-sdk call-context.md, "Session Continuity On Reread"), and `DeleteOther` may
        // delete the object the setup left selected.
        doc.execute("edit.deselectAll", json!({})).unwrap();
    }
    let bytes = doc.save_file(None).unwrap();
    // Per their change and write: rounds, applied, refused necessary, refused false, applied
    // wrongly, failed at the kit (not refused by the guard: the step itself found nothing to act
    // on, nothing written), the refusals' reasons.
    let mut table: BTreeMap<(Theirs, char), Cell> = BTreeMap::new();
    let mut seed = SEED;
    let mut next = |below: usize| usize::try_from(xorshift(&mut seed) % u64::try_from(below).unwrap()).unwrap();
    for _ in 0..ROUNDS {
        let kind = kinds[next(kinds.len())];
        let f = Fixture::from_bytes(&bytes, CallOptions::default());
        let (details, basis) = f.inspect().await;
        let names = ids_by_name(&details);
        let mine = names[&format!("C{MINE}")];
        let pick = |k: usize| names[&format!("C{}", if k >= MINE { k + 1 } else { k })];
        let other = pick(next(N - 1));
        let third = loop {
            let candidate = pick(next(N - 1));
            if candidate != other {
                break candidate;
            }
        };
        let theirs: Vec<(&str, Value)> = match kind {
            Theirs::MoveOther => vec![("object.translate", json!({ "objects": [other], "offset": [0, 0, 0.5] }))],
            Theirs::MoveTarget => vec![("object.translate", json!({ "objects": [mine], "offset": [30, 0, 0] }))],
            Theirs::TopologyTarget => vec![("mesh.subdivide", json!({ "elements": { "object": mine } }))],
            Theirs::TopologyOther => vec![("mesh.subdivide", json!({ "elements": { "object": other } }))],
            Theirs::DeleteOther => vec![("edit.delete", json!({ "objects": [other] }))],
            Theirs::ReparentOther => vec![("object.parent", json!({ "objects": [other], "parent": third }))],
            Theirs::SharedMaterial => vec![("material.set", json!({ "material": "Shared", "roughness": 0.9 }))],
            Theirs::MoveIntoTarget => vec![("object.transform", json!({ "objects": [other], "location": spot(MINE) }))],
            Theirs::MoveParent => vec![("object.translate", json!({ "objects": [names[&format!("C{}", N - 1)]], "offset": [0, 30, 0] }))],
            Theirs::VertexTarget => vec![("mesh.translate", json!({ "elements": { "object": mine, "vertices": [7] }, "offset": [0, 0, 0.25] }))],
            Theirs::RenameSwap => {
                let swapped =
                    details["objects"].as_array().unwrap().iter().find(|object| object["id"] == other).unwrap()["name"].as_str().unwrap().to_owned();
                vec![
                    ("object.rename", json!({ "object": mine, "name": "Swap" })),
                    ("object.rename", json!({ "object": other, "name": format!("C{MINE}") })),
                    ("object.rename", json!({ "object": mine, "name": swapped })),
                ]
            }
            Theirs::RotateTarget => vec![("object.rotate", json!({ "objects": [mine], "axis": "z", "angle": 30 }))],
            Theirs::InsertModifierAbove => vec![
                ("modifier.add", json!({ "object": mine, "type": "solidify", "thickness": 0.05 })),
                ("modifier.move", json!({ "object": mine, "index": 1, "to": 0 })),
            ],
        };
        let faces = faces_on_disk(&f.file(), mine);
        let first_modifier =
            |file: &std::path::Path| on_disk(file).scene().object(u32::try_from(mine).unwrap()).unwrap().modifiers.first().map(|m| m.name.clone());
        let modifier = first_modifier(&f.file());
        let renamed = || name_on_disk(&f.file(), mine) != format!("C{MINE}");
        f.colleague(&theirs);
        let renamed = renamed();
        let (now_faces, now_there, now_named, now_modifier) = {
            let doc = on_disk(&f.file());
            let there = kition_scene_kit_pick(&doc, spot(MINE));
            let named = doc.scene().object_named(&format!("C{MINE}")).map(|object| u64::from(object.id));
            (faces_on_disk(&f.file(), mine), there, named, first_modifier(&f.file()))
        };
        let d = f.call("run", json!({ "basis": basis, "steps": [rename(format!("C{MINE}"), "D")] })).await;
        let a = f.call("run", json!({ "basis": basis, "steps": [rename(mine, "A")] })).await;
        let b = f.call("run", json!({ "basis": basis, "steps": [rename_at(spot(MINE), "B")] })).await;
        let c = f.call("run", json!({ "basis": basis, "steps": [extrude(mine, 0)] })).await;
        let e = f
            .call("run", json!({ "basis": basis, "steps": [{ "command": "modifier.set", "params": { "object": mine, "index": 0, "width": 0.3 } }] }))
            .await;
        for (write, answer, necessary) in [
            ('A', a, renamed),
            ('B', b, now_there != Some(mine) || renamed),
            ('C', c, now_faces != faces),
            ('D', d, now_named != Some(mine)),
            ('E', e, now_modifier != modifier),
        ] {
            let cell = table.entry((kind, write)).or_default();
            cell.0 += 1;
            match answer {
                Ok(_) if necessary => {
                    cell.4 += 1;
                    println!("s11: {write} applied wrongly after {theirs:?}");
                }
                Ok(_) => cell.1 += 1,
                Err(error) if error["error"]["code"] != "conflict" => {
                    cell.5 += 1;
                    println!("s11: {write} failed at the kit after {theirs:?} (necessary {necessary}): {}", error["error"]["message"]);
                }
                Err(error) => {
                    *cell.6.entry(reasons(&error)).or_default() += 1;
                    if necessary {
                        cell.2 += 1;
                    } else {
                        cell.3 += 1;
                        println!("s11: {write} refused falsely after {theirs:?}: {} (others: {})", error["conflicts"], error["others"]);
                    }
                }
            }
        }
        f.shutdown().await;
    }
    println!("s11: their change | write | rounds | applied | refused, necessary | refused, false | applied, wrongly | failed at the kit | reasons");
    let mut wrong = 0;
    let mut false_refusals = 0;
    let mut kit_failures = 0;
    for ((kind, write), (rounds, applied, needed, needless, wrongly, failed, reasons)) in &table {
        println!("s11: {kind:?} | {write} | {rounds} | {applied} | {needed} | {needless} | {wrongly} | {failed} | {reasons:?}");
        wrong += wrongly;
        false_refusals += needless;
        kit_failures += failed;
    }
    assert_eq!(wrong, 0, "no write applied whose premise changed");
    assert_eq!(false_refusals, 0, "no write refused whose premise still holds");
    assert_eq!(kit_failures, 0, "no write failed after passing the guard");
}

/// The object a new engine finds at world point `at` of `doc` (the kit's own `pick`), by id.
fn kition_scene_kit_pick(doc: &kition_scene_kit::SceneDocument, at: [f64; 3]) -> Option<u64> {
    use kition_kit::KitDocument;
    let under = doc.under("object.rename", &json!({ "pick": at })).unwrap()?;
    under.objects.first().and_then(Value::as_u64)
}

// ---- Timings ----

/// The median time of an agent's write (dev profile, ten runs each): plain (no `basis`), with
/// `basis` and nothing changed, after an unrelated change made in the file, and refused after a
/// change of its target made in the file, on the 8-cube scene and on the repository's largest
/// real file (`bindings/wasm/tests/fixtures/released-0.12.0.kscene`), and on a 200-cube scene.
/// Printed, not asserted.
#[tokio::test(flavor = "multi_thread")]
async fn timings_of_a_write() {
    let large = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../wasm/tests/fixtures/released-0.12.0.kscene")).unwrap();
    for (name, bytes) in [("8 cubes", scene(CUBES)), ("released-0.12.0.kscene", large), ("200 cubes", scene(200))] {
        let f = Fixture::from_bytes(&bytes, CallOptions::default());
        let (details, _) = f.inspect().await;
        let objects = details["objects"].as_array().unwrap();
        let vertices: u64 = objects.iter().filter_map(|object| object["mesh"]["vertices"].as_u64()).sum();
        let (mine, theirs) = (objects[0]["id"].as_u64().unwrap(), objects[1]["id"].as_u64().unwrap());
        let [mut plain, mut based, mut unrelated, mut refused] = std::array::from_fn::<Vec<f64>, 4, _>(|_| Vec::new());
        for round in 0..10u32 {
            let start = Instant::now();
            f.ok("run", json!({ "steps": [rename(mine, &format!("P{round}"))] })).await;
            plain.push(start.elapsed().as_secs_f64() * 1000.0);
            let (_, basis) = f.inspect().await;
            let start = Instant::now();
            let ran = f.ok("run", json!({ "basis": basis, "steps": [rename(mine, &format!("B{round}"))] })).await;
            based.push(start.elapsed().as_secs_f64() * 1000.0);
            f.colleague(&[("object.rename", json!({ "object": theirs, "name": format!("T{round}") }))]);
            let start = Instant::now();
            let ran = f.ok("run", json!({ "basis": revision(&ran), "steps": [rename(mine, &format!("U{round}"))] })).await;
            unrelated.push(start.elapsed().as_secs_f64() * 1000.0);
            f.colleague(&[("object.rename", json!({ "object": mine, "name": format!("C{round}") }))]);
            let start = Instant::now();
            let answer = f.call("run", json!({ "basis": revision(&ran), "steps": [rename(mine, &format!("R{round}"))] })).await;
            refused.push(start.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(answer.as_ref().err().map(|error| error["error"]["code"].clone()), Some(json!("conflict")), "{name}: {answer:?}");
        }
        println!(
            "timings ({name}: {} objects, {vertices} vertices): plain {:.1} ms, with basis {:.1} ms, after an unrelated file change {:.1} ms, refused {:.1} ms",
            objects.len(),
            median(plain),
            median(based),
            median(unrelated),
            median(refused)
        );
        f.shutdown().await;
    }
}

// ---- C8-fix-8, C8-scene-2: receipts in space; timings ----

/// Receipt bytes and wording (printed): a subdivide of a cube; a move of 1 and of 2,000 vertices;
/// a move of a parent of 5 and of 200 children (with the command's result, which says how many
/// objects moved with it, `carried`), the same parent move made by a colleague in the file as the
/// next write's `others` tells it; a move of 200 objects by one offset; a change of a material 50
/// cubes share, and the same change by a colleague as `others` tells it.
#[tokio::test(flavor = "multi_thread")]
async fn c8f8_receipts_in_space() {
    let show = |name: &str, what: &Value| {
        let text = what.to_string();
        let excerpt: String = text.chars().take(700).collect();
        println!("c8f8 receipt: {name}: {} bytes: {excerpt}{}", text.len(), if text.len() > 700 { "…" } else { "" });
        if text.len() > 700 {
            println!("c8f8 receipt: {name}: entries {}, ends: …{}", what["changed"].as_array().map_or(0, Vec::len), &text[text.len() - 160..]);
        }
    };
    let f = Fixture::new(CUBES);
    let (details, _) = f.inspect().await;
    let mine = ids_by_name(&details)["C2"];
    let ran = f.ok("run", json!({ "steps": [{ "command": "mesh.subdivide", "params": { "elements": { "object": mine } } }] })).await;
    show("subdivide a cube", &ran["changes"]);
    let ran = f.ok("run", json!({ "steps": [{ "command": "mesh.translate", "params": { "elements": { "object": mine, "vertices": [7] }, "offset": [0, 0, 0.25] } }] })).await;
    show("move 1 vertex", &ran["changes"]);
    // A dense mesh: subdivided until it has at least 2,000 vertices.
    let mut count = 0u64;
    while count < 2000 {
        f.ok("run", json!({ "steps": [{ "command": "mesh.subdivide", "params": { "elements": { "object": mine } } }] })).await;
        let (details, _) = f.inspect().await;
        count = details["objects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|object| object["id"] == mine)
            .and_then(|object| object["mesh"]["vertices"].as_u64())
            .unwrap();
    }
    let vertices: Vec<u64> = (0..2000).collect();
    let ran = f.ok("run", json!({ "steps": [{ "command": "mesh.translate", "params": { "elements": { "object": mine, "vertices": vertices }, "offset": [0, 0, 0.1] } }] })).await;
    show(&format!("move 2,000 vertices (of {count})"), &ran["changes"]);
    f.shutdown().await;
    for children in [5usize, 200] {
        let f = Fixture::with(children + 1, CallOptions::default());
        let (details, _) = f.inspect().await;
        let names = ids_by_name(&details);
        let parent = names[&format!("C{children}")];
        let kids: Vec<u64> = (0..children).map(|n| names[&format!("C{n}")]).collect();
        f.ok("run", json!({ "steps": [{ "command": "object.parent", "params": { "objects": kids, "parent": parent } }] })).await;
        let ran = f.ok("run", json!({ "steps": [{ "command": "object.translate", "params": { "objects": [parent], "offset": [0, 0, 1] } }] })).await;
        show(&format!("move a parent of {children} children"), &ran["changes"]);
        println!("c8f8 receipt: move a parent of {children} children: result {}", ran["completed"][0]["result"]);
        assert_eq!(ran["completed"][0]["result"]["carried"]["count"], children, "{ran}");
        f.colleague(&[("object.translate", json!({ "objects": [parent], "offset": [0, 0, 1] }))]);
        let ran = f.ok("run", json!({ "basis": revision(&ran), "steps": [rename(names["C0"], "Seen")] })).await;
        show(&format!("others: a colleague moved a parent of {children} children"), &ran["others"]);
        assert_eq!(entry(&ran["others"]["changed"], &parent.to_string())["carries"], json!({ "count": children, "what": "world transform" }));
        f.shutdown().await;
    }
    let f = Fixture::with(200, CallOptions::default());
    let (details, _) = f.inspect().await;
    let names = ids_by_name(&details);
    let all: Vec<u64> = (0..200).map(|n| names[&format!("C{n}")]).collect();
    let ran = f.ok("run", json!({ "steps": [{ "command": "object.translate", "params": { "objects": all, "offset": [0, 0, 1] } }] })).await;
    show("move 200 objects by one offset", &ran["changes"]);
    f.shutdown().await;
    let f = Fixture::with(50, CallOptions::default());
    let all: Vec<String> = (0..50).map(|n| format!("C{n}")).collect();
    f.ok(
        "run",
        json!({ "steps": [
            { "command": "material.new", "params": { "name": "Shared", "assign": false } },
            { "command": "material.assign", "params": { "material": "Shared", "objects": all } },
        ] }),
    )
    .await;
    let ran = f.ok("run", json!({ "steps": [{ "command": "material.set", "params": { "material": "Shared", "roughness": 0.9 } }] })).await;
    show("change a material 50 cubes share", &ran["changes"]);
    f.colleague(&[("material.set", json!({ "material": "Shared", "roughness": 0.2 }))]);
    let (details, _) = f.inspect().await;
    let c0 = ids_by_name(&details)["C0"];
    let ran = f.ok("run", json!({ "basis": revision(&ran), "steps": [rename(c0, "Seen")] })).await;
    show("others: a colleague changed a material 50 cubes share", &ran["others"]);
    f.shutdown().await;
}

/// The median time (ten runs each, dev profile) of an agent's write on the 8-cube scene, the
/// repository's largest real file (`released-0.12.0.kscene`) and a 200-cube scene: plain (no
/// `basis`); with `basis`, nothing changed; after an unrelated change made in the file; refused
/// after a change of its target made in the file; by world point after an unrelated move made in
/// the file (`context.covered` asked); by face index after a vertex move made in the file (the
/// derived values read for the change set and the premise). And the first write a fresh server
/// makes on the file, which reads every object's derived values once, against the second. Printed,
/// not asserted.
#[tokio::test(flavor = "multi_thread")]
async fn c8f8_timings_of_writes_that_ask_the_kit() {
    let large = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../wasm/tests/fixtures/released-0.12.0.kscene")).unwrap();
    for (name, bytes) in [("8 cubes", scene(CUBES)), ("released-0.12.0.kscene", large), ("200 cubes", scene(200))] {
        let f = Fixture::from_bytes(&bytes, CallOptions::default());
        let (details, _) = f.inspect().await;
        let objects = details["objects"].as_array().unwrap();
        let mesh = |object: &&Value| object["mesh"]["faces"].as_u64().is_some_and(|faces| faces > 0);
        let meshes: Vec<&Value> = objects.iter().filter(mesh).collect();
        let (mine, theirs) =
            (meshes[0]["id"].as_u64().unwrap(), objects.iter().find(|object| object["id"] != meshes[0]["id"]).unwrap()["id"].as_u64().unwrap());
        let at: Vec<f64> = meshes[0]["worldLocation"].as_array().unwrap().iter().map(|value| value.as_f64().unwrap()).collect();
        let at = [at[0], at[1], at[2]];
        // The first write of a fresh server, and the second.
        let start = Instant::now();
        let ran = f.ok("run", json!({ "steps": [rename(mine, "First")] })).await;
        let first = start.elapsed().as_secs_f64() * 1000.0;
        let start = Instant::now();
        f.ok("run", json!({ "basis": revision(&ran), "steps": [rename(mine, "Second")] })).await;
        let second = start.elapsed().as_secs_f64() * 1000.0;
        let [mut plain, mut based, mut unrelated, mut refused, mut by_point, mut by_face] = std::array::from_fn::<Vec<f64>, 6, _>(|_| Vec::new());
        for round in 0..10u32 {
            let start = Instant::now();
            let mut ran = f.ok("run", json!({ "steps": [rename(mine, &format!("P{round}"))] })).await;
            plain.push(start.elapsed().as_secs_f64() * 1000.0);
            let start = Instant::now();
            ran = f.ok("run", json!({ "basis": revision(&ran), "steps": [rename(mine, &format!("B{round}"))] })).await;
            based.push(start.elapsed().as_secs_f64() * 1000.0);
            f.colleague(&[("object.rename", json!({ "object": theirs, "name": format!("T{round}") }))]);
            let start = Instant::now();
            ran = f.ok("run", json!({ "basis": revision(&ran), "steps": [rename(mine, &format!("U{round}"))] })).await;
            unrelated.push(start.elapsed().as_secs_f64() * 1000.0);
            f.colleague(&[("object.rename", json!({ "object": mine, "name": format!("C{round}") }))]);
            let start = Instant::now();
            let answer = f.call("run", json!({ "basis": revision(&ran), "steps": [rename(mine, &format!("R{round}"))] })).await;
            refused.push(start.elapsed().as_secs_f64() * 1000.0);
            let error = answer.unwrap_err();
            assert_eq!(error["error"]["code"], "conflict", "{name}: {error}");
            f.colleague(&[("object.translate", json!({ "objects": [theirs], "offset": [0, 0, 0.01] }))]);
            let start = Instant::now();
            ran = f.ok("run", json!({ "basis": revision(&error), "steps": [rename_at(at, &format!("Q{round}"))] })).await;
            by_point.push(start.elapsed().as_secs_f64() * 1000.0);
            f.colleague(&[("mesh.translate", json!({ "elements": { "object": mine, "vertices": [0] }, "offset": [0, 0, 0.001] }))]);
            let start = Instant::now();
            f.ok("run", json!({ "basis": revision(&ran), "steps": [{ "command": "mesh.extrude", "params": { "elements": { "object": mine, "faces": [0] }, "distance": 0.001 } }] })).await;
            by_face.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        let vertices: u64 = objects.iter().filter_map(|object| object["mesh"]["vertices"].as_u64()).sum();
        println!(
            "c8f8 timings ({name}: {} objects, {vertices} vertices): plain {:.1} ms, with basis {:.1} ms, after an unrelated file change {:.1} ms, refused {:.1} ms, by point after an unrelated move {:.1} ms, by face index after a vertex move {:.1} ms; first write {first:.1} ms, second {second:.1} ms",
            objects.len(),
            median(plain),
            median(based),
            median(unrelated),
            median(refused),
            median(by_point),
            median(by_face)
        );
        f.shutdown().await;
    }
}
