//! Files the released kits wrote (0.7.0: KDM schema version 2; 0.8.0 and 0.9.0 beside it) take
//! this version's new settings: physics on the scene and objects, particle systems with groomed
//! hair, node groups, and compositing groups, and keep them through a save and a reopen.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

const RELEASED: &[u8] = include_bytes!("fixtures/released-0.7.0.kscene");

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

#[test]
fn a_released_file_takes_fields_constraints_and_density_groups() {
    let mut doc = SceneDocument::open("Room.kscene", RELEASED).unwrap();
    run(&mut doc, "physics.rigidBody.add", json!({ "objects": "Cube", "deactivation": true, "startDeactivated": true }));
    run(&mut doc, "object.addCube", json!({ "name": "Second", "size": 1, "location": [3, 0, 0] }));
    run(&mut doc, "physics.rigidBody.add", json!({ "objects": "Second" }));
    run(&mut doc, "physics.constraint.connect", json!({ "objects": ["Cube", "Second"], "type": "hinge" }));
    run(&mut doc, "physics.field.new", json!({ "type": "turbulence", "strength": 2, "name": "Gust" }));
    run(&mut doc, "object.addGrid", json!({ "name": "Sheet", "size": 2 }));
    run(&mut doc, "physics.cloth.add", json!({ "objects": "Sheet", "selfCollision": true }));
    run(&mut doc, "vertexGroup.add", json!({ "object": "Sheet", "name": "Dense" }));
    run(&mut doc, "particle.add", json!({ "object": "Sheet", "densityGroup": "Dense" }));

    let saved = doc.save_file(None).unwrap();
    doc.mark_saved();
    let back = SceneDocument::open("Room.kscene", &saved).unwrap();
    let inspected = back.inspect().unwrap();
    let objects = inspected["objects"].as_array().unwrap();
    let id = |name: &str| objects.iter().find(|object| object["name"] == name).unwrap_or_else(|| panic!("no {name}"))["id"].clone();
    let physics = inspected["physics"]["objects"].as_array().unwrap();
    let of = |name: &str| physics.iter().find(|object| object["id"] == id(name)).unwrap_or_else(|| panic!("no physics on {name}")).clone();
    assert_eq!(of("Cube")["rigidBody"]["startDeactivated"], true);
    let constraint = of("Constraint")["rigidConstraint"].clone();
    assert_eq!(
        (constraint["kind"].as_str(), constraint["object1"].clone(), constraint["object2"].clone()),
        (Some("hinge"), id("Cube"), id("Second"))
    );
    assert_eq!((of("Gust")["field"]["kind"].as_str(), of("Gust")["field"]["strength"].as_f64()), (Some("turbulence"), Some(2.0)));
    assert_eq!(of("Sheet")["cloth"]["selfCollision"], true);
    assert_eq!(inspected["particles"][0]["densityGroup"], "Dense");
}

#[test]
fn a_released_file_takes_physics_particles_and_node_groups() {
    let mut doc = SceneDocument::open("Room.kscene", RELEASED).unwrap();
    run(&mut doc, "physics.world.set", json!({ "gravity": [0, 0, -5] }));
    run(&mut doc, "physics.rigidBody.add", json!({ "objects": "Cube" }));
    run(&mut doc, "object.addPlane", json!({ "name": "Floor", "size": 10, "location": [0, 0, -2] }));
    run(&mut doc, "physics.collision.add", json!({ "objects": "Floor" }));
    run(&mut doc, "particle.add", json!({ "object": "Floor", "kind": "hair" }));
    run(&mut doc, "node.newGroup", json!({ "object": "Floor", "name": "Lift" }));

    let saved = doc.save_file(None).unwrap();
    doc.mark_saved();
    let back = SceneDocument::open("Room.kscene", &saved).unwrap();
    let inspected = back.inspect().unwrap();
    let objects = inspected["objects"].as_array().unwrap();
    let named = |name: &str| objects.iter().find(|object| object["name"] == name).unwrap_or_else(|| panic!("no {name}"));
    let physics = &inspected["physics"];
    assert_eq!(physics["world"]["gravity"][2], -5.0, "{physics}");
    assert!(physics["objects"].as_array().is_some_and(|list| list.len() == 2), "{physics}");
    let systems = inspected["particles"].as_array().unwrap();
    assert!(systems.len() == 1 && systems[0]["object"] == named("Floor")["id"], "{systems:?}");
    assert_eq!(named("Floor")["modifiers"][0]["type"], "geometryNodes");
    assert!(inspected["nodeGroups"].as_array().is_some_and(|groups| groups.iter().any(|group| group["name"] == "Lift")));
    assert!(named("Cube")["id"].is_u64());
}

#[test]
fn a_released_file_takes_multiresolution_levels_face_sets_and_hiding() {
    let mut doc = SceneDocument::open("Room.kscene", RELEASED).unwrap();
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "multires.subdivide", json!({}));
    run(&mut doc, "mode.sculpt", json!({}));
    run(&mut doc, "sculpt.stroke", json!({ "kind": "inflate", "points": [[0, 0, 1.0]], "worldRadius": 1, "strength": 1 }));
    run(&mut doc, "sculpt.faceSetVisibility", json!({ "action": "isolate", "faceSet": 0 }));
    run(&mut doc, "sculpt.faceSetsFromLooseParts", json!({}));
    let before = doc.inspect().unwrap()["objects"].as_array().unwrap().iter().find(|object| object["name"] == "Cube").unwrap().clone();

    let back = SceneDocument::open("Room.kscene", &doc.save_file(None).unwrap()).unwrap();
    let inspected = back.inspect().unwrap();
    let cube = inspected["objects"].as_array().unwrap().iter().find(|object| object["name"] == "Cube").unwrap();
    assert_eq!(cube["mesh"]["multiresLevels"], 1, "{cube}");
    assert_eq!(cube["mesh"]["faceSets"], 1);
    assert_eq!(cube["dimensions"], before["dimensions"]);
    assert_eq!(cube["modifiers"][0]["type"], "multires");
}

#[test]
fn released_files_take_a_compositing_group_and_keep_it() {
    let files: [(&str, &[u8]); 3] = [
        ("0.7.0", RELEASED),
        ("0.8.0", include_bytes!("fixtures/released-0.8.0.kscene")),
        ("0.9.0", include_bytes!("fixtures/released-0.9.0.kscene")),
    ];
    for (version, bytes) in files {
        let mut doc = SceneDocument::open("Room.kscene", bytes).unwrap_or_else(|error| panic!("{version}: {error}"));
        let made = run(&mut doc, "compositor.new", json!({ "name": "Grade" }));
        let glare = run(&mut doc, "node.addNode", json!({ "group": made["group"], "kind": "glare", "values": { "Type": "bloom" } }))["node"].clone();
        run(
            &mut doc,
            "node.link",
            json!({ "group": made["group"], "from": made["renderLayers"], "fromSocket": "Image", "to": glare, "toSocket": "Image" }),
        );
        run(
            &mut doc,
            "node.link",
            json!({ "group": made["group"], "from": glare, "fromSocket": "Image", "to": made["output"], "toSocket": "Image" }),
        );
        let saved = doc.save_file(None).unwrap();
        doc.mark_saved();
        let back = SceneDocument::open("Room.kscene", &saved).unwrap();
        let inspected = back.inspect().unwrap();
        assert_eq!(inspected["render"]["compositor"], made["group"], "{version}");
        let group = inspected["nodeGroups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|group| group["name"] == "Grade")
            .unwrap_or_else(|| panic!("{version}: no group"));
        assert_eq!(group["tree"], "compositor", "{version}");
        assert_eq!(group["links"].as_array().unwrap().len(), 2, "{version}");
        assert!(group["nodes"].as_array().unwrap().iter().any(|node| node["kind"] == "glare" && node["values"]["Type"] == "bloom"), "{version}");
    }
}
