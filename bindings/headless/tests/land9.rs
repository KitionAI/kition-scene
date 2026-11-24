//! C8-land-9c: SDK guard and receipt changes exercised through the real cube server.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
mod common;
use common::*;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn a_name_renamed_away_is_refused_before_the_kit() {
    for outside in [false, true] {
        let f = Fixture::new(3);
        let (details, basis) = f.inspect().await;
        let id = ids_by_name(&details)["C0"];
        if outside {
            f.colleague(&[("object.rename", json!({"object": id, "name": "Away"}))]);
        } else {
            f.call_by("other", "run", json!({"steps": [{"command":"object.rename", "params":{"object":id,"name":"Away"}}]})).await;
        }
        let before = std::fs::read(f.file()).unwrap();
        let refused = f.refused("run", json!({"basis":basis,"steps":[{"command":"object.hide","params":{"objects":["C0"]}}]})).await;
        assert_conflict(&refused, &id.to_string(), "relocated");
        assert_eq!(std::fs::read(f.file()).unwrap(), before);
        f.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn swapped_names_in_the_file_are_changed_not_renumbered() {
    let f = Fixture::new(3);
    let (details, basis) = f.inspect().await;
    let ids = ids_by_name(&details);
    f.colleague(&[
        ("object.rename", json!({"object":ids["C0"],"name":"Swap"})),
        ("object.rename", json!({"object":ids["C1"],"name":"C0"})),
        ("object.rename", json!({"object":ids["C0"],"name":"C1"})),
    ]);
    let refused = f.refused("run", json!({"basis":basis,"steps":[{"command":"object.rename","on":[ids["C0"]],"params":{"name":"Mine"}}]})).await;
    assert_conflict(&refused, &ids["C0"].to_string(), "changed");
    assert!(refused["others"].get("renumbered").is_none(), "{refused}");
    let refused = f.refused("run", json!({"basis":basis,"steps":[{"command":"object.hide","params":{"objects":["C0"]}}]})).await;
    assert!(refused["conflicts"].as_array().unwrap().iter().any(|c| c["reason"] == "relocated"), "{refused}");
    f.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pick_applies_when_a_parent_moves_or_is_deleted_but_the_target_stays_under_it() {
    for how in ["move", "delete"] {
        let f = Fixture::new(3);
        let (details, _) = f.inspect().await;
        let ids = ids_by_name(&details);
        let (child, parent) = (ids["C0"], ids["C1"]);
        f.ok("run", json!({"steps":[{"command":"object.parent","params":{"objects":[child],"parent":parent}}]})).await;
        let (_, basis) = f.inspect().await;
        match how {
            "move" => {
                f.colleague(&[("object.translate", json!({"objects":[parent],"offset":[0,0,0.5]}))]);
            }
            _ => {
                f.colleague(&[("edit.delete", json!({"objects":[parent]}))]);
            }
        }
        let doc = on_disk(&f.file());
        use kition_kit::KitDocument;
        assert_eq!(doc.under("object.rename", &json!({"pick":spot(0)})).unwrap().unwrap().objects, vec![json!(child)]);
        let ran = f.ok("run", json!({"basis":basis,"steps":[{"command":"object.rename","params":{"pick":spot(0),"name":"Picked"}}]})).await;
        assert_eq!(object_on_disk(&f.file(), child).unwrap()["name"], "Picked");
        println!("land9 parent {how}, target still at point: applied; others {}", ran["others"]);
        f.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn another_writers_parent_move_reports_what_it_carries() {
    let f = Fixture::new(6);
    let (details, _) = f.inspect().await;
    let ids = ids_by_name(&details);
    let kids: Vec<_> = (0..5).map(|i| ids[&format!("C{i}")]).collect();
    f.ok("run", json!({"steps":[{"command":"object.parent","params":{"objects":kids,"parent":ids["C5"]}}]})).await;
    let (_, basis) = f.inspect().await;
    f.colleague(&[("object.translate", json!({"objects":[ids["C5"]],"offset":[0,0,1]}))]);
    let ran = f.ok("run", json!({"basis":basis,"steps":[{"command":"object.rename","on":[ids["Cube"]],"params":{"name":"Seen"}}]})).await;
    assert_eq!(entry(&ran["others"]["changed"], &ids["C5"].to_string())["carries"], json!({"count":5,"what":"world transform"}));
    println!("land9 carries: {} bytes {}", ran["others"].to_string().len(), ran["others"]);
    f.colleague(&[("object.rename", json!({"object":ids["C5"],"name":"RenamedParent"}))]);
    let renamed =
        f.ok("run", json!({"basis":revision(&ran),"steps":[{"command":"object.rename","on":[ids["Cube"]],"params":{"name":"SeenAgain"}}]})).await;
    println!("land9 parent rename receipt: {}", renamed["others"]);

    f.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn two_hundred_equal_offsets_have_one_complete_footprint() {
    let f = Fixture::new(200);
    let (details, _) = f.inspect().await;
    let ids = ids_by_name(&details);
    let all: Vec<_> = (0..200).map(|i| ids[&format!("C{i}")]).collect();
    let ran = f.ok("run", json!({"steps":[{"command":"object.translate","params":{"objects":all,"offset":[0,0,1]}}]})).await;
    let changes = &ran["changes"];
    let footprint = &changes["footprint"][0];
    assert_eq!(footprint["ids"].as_array().unwrap().len(), 200, "{changes}");
    let actual: std::collections::BTreeSet<_> = footprint["ids"].as_array().unwrap().iter().map(|id| id.as_u64().unwrap()).collect();
    assert_eq!(actual, all.iter().copied().collect(), "every moved id, exactly once");
    assert_eq!(footprint["edits"], json!([{"field":"location.2","by":1}]));
    assert!(changes.get("truncated").is_none(), "{changes}");
    println!("land9 offset 200: {} bytes {}", changes.to_string().len(), changes);
    f.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn covered_replays_one_thousand_objects_at_a_place() {
    let f = Fixture::new(1000);
    let (details, _) = f.inspect().await;
    let ids = ids_by_name(&details);
    let all: Vec<_> = (0..1000).map(|i| ids[&format!("C{i}")]).collect();
    // All 1,000 bounds cover the same point; the first cube remains its picked target.
    f.ok("run", json!({"steps":[{"command":"object.transform","params":{"objects":all,"location":spot(0)}}]})).await;
    let (_, basis) = f.inspect().await;
    f.colleague(&[("object.translate", json!({"objects":all,"offset":[0,0,0.25]}))]);
    let start = std::time::Instant::now();
    f.ok("run", json!({"basis":basis,"steps":[{"command":"object.rename","params":{"pick":spot(0),"name":"Picked"}}]})).await;
    println!("land9 covered 1000: {:.1} ms", start.elapsed().as_secs_f64() * 1000.0);
    assert_eq!(object_on_disk(&f.file(), ids["C0"]).unwrap()["name"], "Picked");
    f.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_nested_boid_target_is_a_premise() {
    let f = Fixture::new(3);
    let (details, _) = f.inspect().await;
    let ids = ids_by_name(&details);
    f.ok("run", json!({"steps":[{"command":"particles.add","params":{"object":ids["C0"],"count":10}}]})).await;
    let (_, basis) = f.inspect().await;
    // A boid target is only stored by id: it has to exist (declarations_independent).
    f.colleague(&[("edit.delete", json!({"objects":[ids["C1"]]}))]);
    let before = std::fs::read(f.file()).unwrap();
    let refused = f.refused("run", json!({"basis":basis,"steps":[{"command":"particles.set","params":{"object":ids["C0"],"physics":"boids","boids":{"rules":[{"kind":"goal","target":ids["C1"]}]}}}]})).await;
    assert_conflict(&refused, &ids["C1"].to_string(), "deleted");
    assert_eq!(std::fs::read(f.file()).unwrap(), before);
    f.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn review_material_channel_edits_hold_the_material_id() {
    for command in ["graph.moveKey", "graph.addModifier"] {
        let f = Fixture::new(1);
        let (details, _) = f.inspect().await;
        let cube = ids_by_name(&details)["C0"];
        f.ok(
            "run",
            json!({"steps":[
                {"command":"material.new","params":{"name":"M","assign":false}},
                {"command":"material.assign","on":[cube],"params":{"material":"M"}},
                {"command":"anim.insertKey","on":[cube],"params":{"properties":["baseColor"],"frame":1}}
            ]}),
        )
        .await;
        let doc = on_disk(&f.file());
        use kition_kit::KitDocument;
        let material = doc.inspect().unwrap()["materials"].as_array().unwrap().iter().find(|m| m["name"] == "M").unwrap()["id"].clone();
        let (_, basis) = f.inspect().await;
        f.colleague(&[("graph.moveKey", json!({"target":material,"property":"baseColor","frame":1,"value":0.4}))]);
        let before = std::fs::read(f.file()).unwrap();
        let params = if command == "graph.moveKey" {
            json!({"target":material,"property":"baseColor","frame":1,"value":0.8})
        } else {
            json!({"target":material,"property":"baseColor","type":"noise"})
        };
        let refused = f.refused("run", json!({"basis":basis,"steps":[{"command":command,"params":params}]})).await;
        assert_conflict(&refused, &material.to_string(), "changed");
        assert_eq!(std::fs::read(f.file()).unwrap(), before);
        f.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn review_driver_variable_object_ids_are_premises() {
    use kition_kit::KitDocument;
    for (kind, member) in [("singleProperty", "object"), ("transform", "object"), ("distance", "a"), ("distance", "b"), ("rotationDifference", "a")] {
        // The session selects nothing: a reread that deletes a selected object is refused by name
        // (kition-sdk call-context.md, "Session Continuity On Reread"), and C1, the last cube
        // added, would be selected.
        let mut doc = scene_doc(2);
        doc.execute("select.none", json!({})).unwrap();
        let f = Fixture::from_bytes(&doc.save_file(None).unwrap(), Default::default());
        let (details, basis) = f.inspect().await;
        let ids = ids_by_name(&details);
        // A variable's source is only referenced: it has to exist (declarations_independent).
        f.colleague(&[("edit.delete", json!({"objects":[ids["C1"]]}))]);
        let mut variable = json!({"name":"v","type":kind});
        variable[member] = json!(ids["C1"]);
        let before = std::fs::read(f.file()).unwrap();
        let refused = f
            .refused(
                "run",
                json!({"basis":basis,"steps":[{"command":"driver.add","params":{
                    "object":ids["C0"],"property":"location","variables":[variable]
                }}]}),
            )
            .await;
        assert_conflict(&refused, &ids["C1"].to_string(), "deleted");
        assert_eq!(std::fs::read(f.file()).unwrap(), before);
        f.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn review_points_do_not_name_objects_but_scalar_references_do() {
    for form in ["point", "id", "deleted"] {
        let f = Fixture::new(1);
        let (details, basis) = f.inspect().await;
        let cube = ids_by_name(&details)["Cube"];
        if form == "deleted" {
            f.colleague(&[("edit.delete", json!({"objects":[cube]}))]);
        } else {
            f.colleague(&[("object.translate", json!({"objects":[cube],"offset":[0,0,1]}))]);
        }
        let target = if form == "point" { json!([cube, 0, 0]) } else { json!(cube) };
        let call = json!({"basis":basis,"steps":[{"command":"light.studio","params":{"target":target}}]});
        if form == "point" {
            f.ok("run", call).await;
        } else {
            let before = std::fs::read(f.file()).unwrap();
            let refused = f.refused("run", call).await;
            assert_conflict(&refused, &cube.to_string(), if form == "deleted" { "deleted" } else { "relocated" });
            assert_eq!(std::fs::read(f.file()).unwrap(), before);
        }
        f.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn review_typed_node_values_hold_ids_without_treating_numbers_as_ids() {
    use kition_kit::KitDocument;
    for command in ["nodes.setValue", "nodes.addNode", "nodes.addInput", "modifier.set", "modifier.add"] {
        let mut doc = scene_doc(2);
        let ids = ids_by_name(&doc.inspect().unwrap());
        let group = doc.execute("nodes.newGroup", json!({"name":"G","object":ids["C0"]})).unwrap()["group"].clone();
        doc.execute("nodes.addInput", json!({"group":group,"name":"Source","type":"object"})).unwrap();
        let node = doc.execute("nodes.addNode", json!({"group":group,"kind":"objectInfo"})).unwrap()["node"].clone();
        // The session selects nothing: a reread that deletes a selected object is refused by name
        // (kition-sdk call-context.md, "Session Continuity On Reread"), and C1 would be selected.
        doc.execute("select.none", json!({})).unwrap();
        let f = Fixture::from_bytes(&doc.save_file(None).unwrap(), Default::default());
        let (_, basis) = f.inspect().await;
        // An object a value names is stored by id: it has to exist (declarations_independent).
        f.colleague(&[("edit.delete", json!({"objects":[ids["C1"]]}))]);
        let params = match command {
            "nodes.setValue" => json!({"group":group,"node":node,"id":"Object","value":ids["C1"]}),
            "nodes.addNode" => json!({"group":group,"kind":"objectInfo","values":{"Object":ids["C1"]}}),
            "nodes.addInput" => json!({"group":group,"name":"Other","type":"object","default":ids["C1"]}),
            "modifier.set" => json!({"object":ids["C0"],"index":0,"inputs":{"Source":ids["C1"]}}),
            _ => json!({"object":ids["C0"],"type":"geometryNodes","group":group,"inputs":{"Source":ids["C1"]}}),
        };
        let before = std::fs::read(f.file()).unwrap();
        let refused = f.refused("run", json!({"basis":basis,"steps":[{"command":command,"params":params}]})).await;
        assert_conflict(&refused, &ids["C1"].to_string(), "deleted");
        assert_eq!(std::fs::read(f.file()).unwrap(), before);
        // A float socket with exactly the same number has no object premise.
        f.ok(
            "run",
            json!({"basis":basis,"steps":[{"command":"nodes.addInput","params":{
                "group":group,"name":"Number","type":"float","default":ids["C1"]
            }}]}),
        )
        .await;
        f.shutdown().await;
    }
}
