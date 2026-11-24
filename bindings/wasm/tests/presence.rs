//! Presence: what a person shares (KDM ids, a scene point under the pointer, edited vertices) and
//! how another view places and draws it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn starter() -> Vec<u8> {
    SceneDocument::create("Room.kscene", 640, 400).unwrap().save_file(None).unwrap()
}

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

#[test]
fn a_selection_travels_as_kdm_ids_and_comes_back_outlined_in_the_peers_colour() {
    let bytes = starter();
    let mut alice = SceneDocument::open("Room.kscene", &bytes).unwrap();
    let mut bob = SceneDocument::open("Room.kscene", &bytes).unwrap();
    for doc in [&mut alice, &mut bob] {
        run(doc, "view.resize", json!({ "width": 640, "height": 400 }));
        run(doc, "select.none", json!({}));
    }
    run(&mut alice, "select.objects", json!({ "objects": "Cube" }));
    let shared = run(&mut alice, "presence.local", json!({ "pointer": [320, 200] }));
    let ids = shared["selection"].as_array().unwrap();
    assert_eq!(ids.len(), 1);
    assert!(ids[0].as_str().unwrap().len() > 8, "a KDM object id, not a key: {shared}");
    let pointer = shared["viewport"]["pointer"].as_array().unwrap();
    let point: Vec<f64> = pointer.iter().map(|value| value.as_f64().unwrap()).collect();
    assert!(point.iter().all(|value| value.abs() <= 1.01), "the pointer lands on the cube: {point:?}");

    // Bob's view: Alice outlined in her colour, her pointer and label placed.
    let before = bob.render(0.0, 0.0, 1.0, 640, 400).unwrap();
    let placed = run(
        &mut bob,
        "presence.peers",
        json!({ "peers": [{ "principal": "u:alice", "name": "Alice", "color": "#00a0ff", "selection": ids, "viewport": shared["viewport"] }] }),
    );
    let peer = &placed["peers"][0];
    assert_eq!(peer["name"], "Alice");
    assert_eq!(peer["objects"].as_array().unwrap().len(), 1);
    assert!(peer["label"].is_array() && peer["pointer"].is_array(), "{placed}");
    let after = bob.render(0.0, 0.0, 1.0, 640, 400).unwrap();
    assert_ne!(before, after, "the outline is drawn");
    let blue = after.chunks(4).filter(|pixel| pixel[2] > 200 && pixel[0] < 40).count();
    assert!(blue > 100, "the outline is in Alice's colour ({blue} pixels)");
    assert!(!bob.dirty(), "presence never dirties the scene");

    // The agent is a peer like anyone; unknown ids are ignored; a bad pointer is refused.
    let placed = run(
        &mut bob,
        "presence.peers",
        json!({ "peers": [{ "principal": "agent", "name": "Agent", "color": "#7c5cff", "selection": ["nope"], "agent": { "origin": "agent" } }] }),
    );
    assert_eq!(placed["peers"][0]["agent"], true);
    assert_eq!(placed["peers"][0]["objects"].as_array().unwrap().len(), 0);
    assert!(bob.execute("presence.local", json!({ "pointer": [1, "x"] })).is_err());
}

#[test]
fn edit_mode_shares_the_mesh_and_its_vertices() {
    let mut doc = SceneDocument::open("Room.kscene", &starter()).unwrap();
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "mode.set", json!({ "mode": "edit" }));
    run(&mut doc, "mesh.select", json!({ "vertices": [0, 1] }));
    let shared = run(&mut doc, "presence.local", json!({}));
    assert!(shared["viewport"]["editing"].is_string());
    assert_eq!(shared["viewport"]["vertices"], json!([0, 1]));
    assert!(shared["viewport"].get("pointer").is_none());
    let placed = run(
        &mut doc,
        "presence.peers",
        json!({ "peers": [{ "principal": "u:bob", "name": "Bob", "color": "#e33", "selection": [], "viewport": shared["viewport"] }] }),
    );
    assert_eq!(placed["peers"][0]["vertices"].as_array().unwrap().len(), 2, "{placed}");
}
