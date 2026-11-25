//! What each call reads or replaces (task card C8-scene-2): the kit states, per command family
//! whose code was read for it, the premises of a call (`KitDocument::under`'s `premises`: fields or
//! paths, and positions), so that a colleague's change inside them refuses the agent's write and a
//! change outside them applies. Each family: one write with `basis` after a change made in the
//! file inside its premise (refused, with the reason), and one after a change outside it (applied;
//! without the stated premise the default, every field and position of the target, refused it).
//! Names and indexes into an object's lists (modifiers, constraints) are places with premises: a
//! write by them holds that name or that item.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::*;
use serde_json::{Value, json};

const CUBES: usize = 8;

/// One family's pair: on a fresh 8-cube scene, the agent's `setup` (no basis), then an inspect;
/// the colleague's change `inside` (refused with `reason` on the object named `on`), and on another
/// fresh scene its change `outside` (applied). `write` and the changes are built from the scene's
/// ids by name.
struct Family {
    name: &'static str,
    setup: fn(&Ids) -> Vec<Value>,
    write: fn(&Ids) -> Value,
    inside: fn(&Ids) -> Vec<(&'static str, Value)>,
    on: &'static str,
    reason: &'static str,
    outside: fn(&Ids) -> Vec<(&'static str, Value)>,
}

type Ids = std::collections::BTreeMap<String, u64>;

async fn check(family: Family) {
    for inside in [true, false] {
        let f = Fixture::new(CUBES);
        let (details, _) = f.inspect().await;
        let ids = ids_by_name(&details);
        let setup = (family.setup)(&ids);
        if !setup.is_empty() {
            f.ok("run", json!({ "steps": setup })).await;
        }
        let (_, basis) = f.inspect().await;
        let change = if inside { (family.inside)(&ids) } else { (family.outside)(&ids) };
        f.colleague(&change);
        let answer = f.call("run", json!({ "basis": basis, "steps": [(family.write)(&ids)] })).await;
        if inside {
            let refused = answer.err().unwrap_or_else(|| panic!("{}: refused after {change:?}", family.name));
            let id = ids.get(family.on).map_or_else(|| family.on.to_owned(), u64::to_string);
            assert_conflict(&refused, &id, family.reason);
            println!("premises {}: inside refused: {}", family.name, refused["conflicts"]);
        } else {
            let ran = answer.unwrap_or_else(|error| panic!("{}: applied after {change:?}: {error}", family.name));
            println!("premises {}: outside applied; others {}", family.name, ran["others"]);
        }
        f.shutdown().await;
    }
}

fn none(_: &Ids) -> Vec<Value> {
    Vec::new()
}

#[tokio::test(flavor = "multi_thread")]
async fn transform_holds_the_targets_place_not_its_content() {
    check(Family {
        name: "transform",
        setup: none,
        write: |ids| json!({ "command": "object.translate", "params": { "objects": [ids["C2"]], "offset": [1, 0, 0] } }),
        inside: |ids| vec![("object.translate", json!({ "objects": [ids["C2"]], "offset": [0, 5, 0] }))],
        on: "C2",
        reason: "relocated",
        outside: |ids| {
            vec![("object.rename", json!({ "object": ids["C2"], "name": "Theirs" })), ("material.new", json!({ "name": "M", "assign": false }))]
        },
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn object_properties_hold_the_field_they_write() {
    check(Family {
        name: "object properties",
        setup: none,
        write: |ids| json!({ "command": "object.hide", "params": { "objects": [ids["C2"]] } }),
        inside: |ids| vec![("object.hide", json!({ "objects": [ids["C2"]] }))],
        on: "C2",
        reason: "changed",
        outside: |ids| {
            vec![
                ("object.rename", json!({ "object": ids["C2"], "name": "Theirs" })),
                ("object.translate", json!({ "objects": [ids["C2"]], "offset": [0, 5, 0] })),
            ]
        },
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_material_assignment_holds_the_slot_and_that_the_material_exists() {
    check(Family {
        name: "materials",
        setup: |_| {
            vec![
                json!({ "command": "material.new", "params": { "name": "M", "assign": false } }),
                json!({ "command": "material.new", "params": { "name": "N", "assign": false } }),
            ]
        },
        write: |ids| json!({ "command": "material.assign", "params": { "material": "M", "objects": [ids["C2"]] } }),
        inside: |ids| vec![("material.assign", json!({ "material": "N", "objects": [ids["C2"]] }))],
        on: "C2",
        reason: "changed",
        outside: |ids| {
            vec![("material.set", json!({ "material": "M", "roughness": 0.9 })), ("object.rename", json!({ "object": ids["C2"], "name": "Theirs" }))]
        },
    })
    .await;
}

/// The bevel at index 0 and the subdivision at index 1: the agent sets the subdivision's levels by
/// index. A modifier added and moved above it shifts it (the stack told whole): refused. The
/// bevel's width changed (`modifiers.0`): applied.
#[tokio::test(flavor = "multi_thread")]
async fn a_modifier_by_index_holds_that_item_and_an_insert_above_refuses_it() {
    check(Family {
        name: "modifiers",
        setup: |ids| {
            vec![
                json!({ "command": "modifier.add", "params": { "object": ids["C2"], "type": "bevel", "width": 0.1 } }),
                json!({ "command": "modifier.add", "params": { "object": ids["C2"], "type": "subdivision", "levels": 1 } }),
            ]
        },
        write: |ids| json!({ "command": "modifier.set", "params": { "object": ids["C2"], "index": 1, "levels": 2 } }),
        inside: |ids| {
            vec![
                ("modifier.add", json!({ "object": ids["C2"], "type": "solidify", "thickness": 0.1 })),
                ("modifier.move", json!({ "object": ids["C2"], "index": 2, "to": 0 })),
            ]
        },
        on: "C2",
        reason: "changed",
        outside: |ids| {
            vec![
                ("modifier.set", json!({ "object": ids["C2"], "index": 0, "width": 0.3 })),
                ("object.rename", json!({ "object": ids["C2"], "name": "Theirs" })),
            ]
        },
    })
    .await;
}

/// Two Copy Location constraints on C2 (targets C5 and C6): the agent sets the second's influence
/// by index. One added and moved above it: refused. The first's influence changed: applied.
#[tokio::test(flavor = "multi_thread")]
async fn a_constraint_by_index_holds_that_item_and_an_insert_above_refuses_it() {
    check(Family {
        name: "constraints",
        setup: |ids| {
            vec![
                json!({ "command": "constraint.add", "params": { "object": ids["C2"], "type": "copyLocation", "target": ids["C5"], "influence": 0.0 } }),
                json!({ "command": "constraint.add", "params": { "object": ids["C2"], "type": "copyLocation", "target": ids["C6"], "influence": 0.0 } }),
            ]
        },
        write: |ids| json!({ "command": "constraint.set", "params": { "object": ids["C2"], "index": 1, "influence": 0.0 } }),
        inside: |ids| {
            vec![
                ("constraint.add", json!({ "object": ids["C2"], "type": "copyLocation", "target": ids["C7"], "influence": 0.0 })),
                ("constraint.move", json!({ "object": ids["C2"], "index": 2, "to": 0 })),
            ]
        },
        on: "C2",
        reason: "changed",
        outside: |ids| {
            vec![("constraint.set", json!({ "object": ids["C2"], "index": 0, "influence": 0.0, "rename": "Theirs" })), ("object.rename", json!({ "object": ids["C2"], "name": "Theirs" }))]
        },
    })
    .await;
}

/// A vertex group lives in the mesh: the agent assigns vertex 0 to group G. A face subdivided:
/// the vertex indexes' topology changed, refused. The object renamed and moved: applied.
#[tokio::test(flavor = "multi_thread")]
async fn vertex_groups_hold_the_mesh() {
    check(Family {
        name: "vertex groups",
        setup: |ids| vec![json!({ "command": "vertexGroup.add", "params": { "object": ids["C2"], "name": "G" } })],
        write: |ids| json!({ "command": "vertexGroup.assign", "params": { "object": ids["C2"], "name": "G", "vertices": [0] } }),
        // An assignment names vertices by index: their topology, not where they are
        // (declarations_independent).
        inside: |ids| vec![("mesh.subdivide", json!({ "elements": { "object": ids["C2"], "faces": [0] } }))],
        on: "C2",
        reason: "changed",
        outside: |ids| {
            vec![
                ("object.rename", json!({ "object": ids["C2"], "name": "Theirs" })),
                ("object.translate", json!({ "objects": [ids["C2"]], "offset": [0, 5, 0] })),
            ]
        },
    })
    .await;
}

/// A transform key at frame 10 reads the object's channels and transform: another key inserted at
/// frame 20: refused. The object renamed: applied.
#[tokio::test(flavor = "multi_thread")]
async fn a_transform_key_holds_the_channels_and_the_transform() {
    check(Family {
        name: "keyframes",
        setup: none,
        write: |ids| json!({ "command": "animation.insertKey", "params": { "objects": [ids["C2"]], "frame": 10 } }),
        inside: |ids| vec![("animation.insertKey", json!({ "objects": [ids["C2"]], "frame": 20 }))],
        on: "C2",
        reason: "changed",
        outside: |ids| vec![("object.rename", json!({ "object": ids["C2"], "name": "Theirs" }))],
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn light_settings_hold_the_lights_data() {
    check(Family {
        name: "lights and cameras",
        setup: none,
        write: |ids| json!({ "command": "light.set", "params": { "object": ids["Light"], "power": 500 } }),
        inside: |ids| vec![("light.set", json!({ "object": ids["Light"], "color": [1, 0, 0] }))],
        on: "Light",
        reason: "changed",
        outside: |ids| {
            vec![
                ("object.rename", json!({ "object": ids["Light"], "name": "Theirs" })),
                ("object.translate", json!({ "objects": [ids["Light"]], "offset": [0, 1, 0] })),
            ]
        },
    })
    .await;
}

/// Parenting C2 to C5 keeps C2's world transform: it reads both objects' places. C5 moved: refused.
/// Both renamed: applied.
#[tokio::test(flavor = "multi_thread")]
async fn parenting_holds_the_childs_and_the_parents_places() {
    check(Family {
        name: "hierarchy",
        setup: none,
        write: |ids| json!({ "command": "object.parent", "params": { "objects": [ids["C2"]], "parent": ids["C5"] } }),
        inside: |ids| vec![("object.translate", json!({ "objects": [ids["C5"]], "offset": [0, 5, 0] }))],
        on: "C5",
        reason: "relocated",
        outside: |ids| {
            vec![
                ("object.rename", json!({ "object": ids["C2"], "name": "Theirs" })),
                ("object.rename", json!({ "object": ids["C5"], "name": "Parent" })),
            ]
        },
    })
    .await;
}

// ---- Names ----

/// A name holds the object it named at basis: swapping it or renaming it away is refused
/// as `relocated` before execution. Renaming a different object still permits the write.
#[tokio::test(flavor = "multi_thread")]
async fn a_write_by_name_holds_the_name() {
    for how in ["swap", "other", "away"] {
        let f = Fixture::new(CUBES);
        let (details, _) = f.inspect().await;
        let ids = ids_by_name(&details);
        let (c2, c3) = (ids["C2"], ids["C3"]);
        let bevel = |id: u64| json!({ "command": "modifier.add", "params": { "object": id, "type": "bevel" } });
        f.ok("run", json!({ "steps": [bevel(c2), bevel(c3)] })).await;
        let (_, basis) = f.inspect().await;
        let rename = |id: u64, name: &str| json!({ "command": "object.rename", "params": { "object": id, "name": name } });
        let steps = match how {
            "swap" => vec![rename(c2, "Swap"), rename(c3, "C2"), rename(c2, "C3")],
            "other" => vec![rename(ids["C4"], "Theirs")],
            _ => vec![rename(c2, "Theirs")],
        };
        f.call_by("agent-other", "run", json!({ "steps": steps })).await;
        let step = json!({ "command": "modifier.set", "params": { "object": "C2", "index": 0, "width": 0.3 } });
        let answer = f.call("run", json!({ "basis": basis, "steps": [step] })).await;
        match how {
            "swap" => {
                let refused = answer.unwrap_err();
                assert_conflict(&refused, &c3.to_string(), "relocated");
                println!("premises names: swapped by another agent: refused {}", refused["conflicts"]);
            }
            "other" => {
                let ran = answer.unwrap();
                println!("premises names: C4 renamed: applied; others {}", ran["others"]);
            }
            _ => {
                let refused = answer.unwrap_err();
                assert_conflict(&refused, &c2.to_string(), "relocated");
                println!("premises names: C2 renamed away: refused {}", refused["conflicts"]);
            }
        }
        f.shutdown().await;
    }
}

// ---- The object model's writable fields ----

/// An object's kind, parent and material are ids and an enum the kit's commands keep consistent
/// (a parent with its parent inverse, a material that exists, a kind with its data): the object
/// model marks them read-only (C8-scene-2), so a `scene_batch` that sets one is refused and
/// kition-env never samples a task that sets one to a number ("Change the parent of the object
/// named "Light" to 2."). The commands still set them (`object.parent`, `material.assign`).
#[tokio::test(flavor = "multi_thread")]
async fn a_batch_cannot_set_an_objects_kind_parent_or_material() {
    let f = Fixture::new(CUBES);
    let (details, _) = f.inspect().await;
    let ids = ids_by_name(&details);
    for (field, value) in [("parent", json!(2)), ("material", json!(1)), ("kind", json!("light"))] {
        let answer = f.call("batch", json!({ "ops": [{ "op": "set", "target": ids["C2"], "field": field, "value": value }] })).await;
        let refused = answer.err().unwrap_or_else(|| panic!("a batch set {field}"));
        println!("premises read-only: batch set {field}: {}", refused["error"]["message"]);
    }
    f.ok("run", json!({ "steps": [{ "command": "object.parent", "params": { "objects": [ids["C2"]], "parent": ids["C3"] } }] })).await;
    f.shutdown().await;
}

/// Raw `scene_batch` operations need a per-type `batchOps` premise contract (kition-sdk
/// call-context.md, "Isolated Planning And Raw Batches"). Every raw batch is followed by the
/// scene's repair, which lets go of references to objects, materials, images, node groups and
/// actions that are gone: a field whose value the repair reads that way (an object's `data`) has
/// no contract and is refused before anything runs; a field it never reads (a name) is direct; a
/// delete is held as the kit's own delete command holds it (`edit.delete`).
#[tokio::test(flavor = "multi_thread")]
async fn a_batch_runs_only_what_its_premise_contract_declares() {
    let f = Fixture::new(CUBES);
    let (details, basis) = f.inspect().await;
    let ids = ids_by_name(&details);
    let before = std::fs::read(f.file()).unwrap();
    // A valid value (a mesh object's data names its kind), so the model's scratch check passes and
    // the refusal is the missing contract's.
    let refused = f.refused("batch", json!({ "ops": [{ "op": "set", "target": ids["C2"], "field": "data", "value": { "type": "mesh" } }] })).await;
    // Refused before it runs: kition-kits first lets the model check the raw operation on a scratch
    // copy and answers that check's error when there is one, else that no contract is declared.
    println!("premises batchOps: data refused: {}", refused["error"]);
    assert!(matches!(refused["error"]["code"].as_str(), Some("conflict" | "invalid-op")), "{refused}");
    assert_eq!(std::fs::read(f.file()).unwrap(), before, "a refused batch writes nothing");
    f.ok("batch", json!({ "basis": basis, "ops": [{ "op": "set", "target": ids["C2"], "field": "name", "value": "Renamed" }] })).await;
    assert_eq!(object_on_disk(&f.file(), ids["C2"]).unwrap()["name"], "Renamed");
    f.ok("batch", json!({ "ops": [{ "op": "delete", "target": ids["C3"] }] })).await;
    assert!(object_on_disk(&f.file(), ids["C3"]).is_none());
    f.shutdown().await;
}
