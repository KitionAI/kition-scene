//! Places (task card C8-scene): what each command names by where it is, and what the kit answers
//! a host's `context.under` with. A world point (`pick`) names the object there now, the smallest
//! mesh whose bounds hold it, else the nearest object without a mesh within 0.25 m; an edit-mode
//! command's `elements` name vertices and faces of a mesh by index, held by the mesh's derived
//! `~topology` rather than as a place. `under` answers a pick with the object there, `covers` and
//! the call's premises, and every other call with its premises. The object model marks where an
//! object is (`location`, `rotation`, `scale`, `parent`, `parentInverse`) as positions.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use kition_kit::{Covers, KitDocument, Premise, Under};
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn scene() -> (SceneDocument, u64, u64, u64) {
    let mut doc = SceneDocument::create("Scene.kscene", 320, 180).unwrap();
    let small = doc.execute("object.addCube", json!({ "name": "Small", "size": 0.5, "location": [0.25, 0, 0] })).unwrap();
    let small = small["object"].as_u64().or_else(|| small["objects"][0].as_u64()).unwrap_or_else(|| panic!("{small}"));
    let ids: std::collections::BTreeMap<String, u64> = doc.inspect().unwrap()["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| (o["name"].as_str().unwrap().to_owned(), o["id"].as_u64().unwrap()))
        .collect();
    (doc, ids["Cube"], small, ids["Light"])
}

#[test]
fn pick_names_the_smallest_mesh_holding_the_point_else_the_nearest_origin() {
    let (mut doc, cube, small, light) = scene();
    let objects = |doc: &SceneDocument, at: Value| doc.under("object.rename", &json!({ "pick": at })).unwrap().unwrap().objects;
    assert_eq!(objects(&doc, json!([0.3, 0, 0])), [json!(small)], "inside both cubes: the smaller");
    assert_eq!(objects(&doc, json!([-0.8, 0, 0])), [json!(cube)]);
    let at = doc.inspect().unwrap()["objects"].as_array().unwrap().iter().find(|o| o["id"] == light).unwrap()["worldLocation"].clone();
    assert_eq!(objects(&doc, at.clone()), [json!(light)], "a light by its origin");
    assert!(objects(&doc, json!([50, 50, 50])).is_empty(), "nothing there");
    doc.execute("object.rename", json!({ "pick": [0.3, 0, 0], "name": "Picked" })).unwrap();
    assert_eq!(doc.inspect().unwrap()["objects"].as_array().unwrap().iter().find(|o| o["id"] == small).unwrap()["name"], "Picked");
    let error = doc.execute("object.rename", json!({ "pick": [50, 50, 50], "name": "X" })).unwrap_err();
    assert!(error.contains("no object is there"), "{error}");
    let error = doc.execute("object.rename", json!({ "pick": [0.3, 0, 0], "object": cube, "name": "X" })).unwrap_err();
    assert!(error.contains("give one"), "{error}");
}

#[test]
fn under_answers_a_pick_with_covers_and_premises_and_elements_and_names_with_their_premises() {
    let (doc, cube, small, _) = scene();
    let covers = Some(Covers { fields: vec!["hidden".into(), "~bounds".into(), "constraints".into()] });
    let premise = |id: u64, fields: &[&str]| Premise::on(json!(id)).fields(fields.iter().copied()).positions(Vec::<String>::new());
    // A world point: the object there now, `covers`, and what the call needs of that object.
    assert_eq!(
        doc.under("object.hide", &json!({ "pick": [0.3, 0, 0] })).unwrap(),
        Some(Under { objects: vec![json!(small)], covers: covers.clone(), premises: vec![premise(small, &["hidden"])], ..Default::default() })
    );
    // A point where nothing is: no object, still `covers`.
    assert_eq!(doc.under("object.hide", &json!({ "pick": [50, 50, 50] })).unwrap(), Some(Under { covers, ..Default::default() }));
    // Element indexes: the mesh's derived topology and no position; by name, the name too.
    assert_eq!(
        doc.under("mesh.extrude", &json!({ "elements": { "object": cube, "faces": [2] } })).unwrap(),
        Some(Under { premises: vec![premise(cube, &["~topology"])], ..Default::default() })
    );
    assert_eq!(
        doc.under("mesh.extrude", &json!({ "elements": { "object": "Cube", "faces": [2] } })).unwrap(),
        Some(Under { premises: vec![premise(cube, &["name", "~topology"])], ..Default::default() })
    );
    // By id: what the command reads or replaces; by name: that name besides it.
    assert_eq!(
        doc.under("object.hide", &json!({ "objects": [cube] })).unwrap(),
        Some(Under { premises: vec![premise(cube, &["hidden"])], ..Default::default() })
    );
    assert_eq!(
        doc.under("object.hide", &json!({ "objects": ["Cube"] })).unwrap(),
        Some(Under { premises: vec![premise(cube, &["hidden", "name"])], ..Default::default() })
    );
}

#[test]
fn commands_declare_pick_as_their_place_and_position_independence() {
    let (doc, ..) = scene();
    let commands: std::collections::BTreeMap<String, kition_kit::KitCommand> = doc.commands().into_iter().map(|c| (c.id.clone(), c)).collect();
    // Element indexes are no place: their premise is the mesh's derived topology.
    assert!(commands["mesh.extrude"].places.is_empty());
    assert_eq!(commands["object.rename"].places, ["pick"]);
    assert!(commands["object.rename"].schema.as_ref().unwrap().get("pick").is_some());
    assert!(commands["object.addCube"].places.is_empty());
    for id in ["object.rename", "object.hide", "object.reveal", "material.assign", "material.clear", "light.set", "camera.set"] {
        assert!(commands[id].position_independent, "{id}");
    }
    for id in ["object.translate", "object.transform", "mesh.extrude", "modifier.add", "material.set"] {
        assert!(!commands[id].position_independent, "{id}");
    }
    let schema: Value = serde_json::from_str(kition_scene_kdm::SCHEMA).unwrap();
    let fields = &schema["types"]["Object"]["fields"];
    for field in ["location", "rotation", "scale", "parent", "parentInverse"] {
        assert_eq!(fields[field]["position"], true, "{field}");
    }
    for field in ["name", "mesh", "material", "modifiers", "data", "hidden"] {
        assert!(fields[field].get("position").is_none(), "{field}");
    }
}

#[test]
fn named_model_types_declare_their_instruction_label() {
    let schema: Value = serde_json::from_str(kition_scene_kdm::SCHEMA).unwrap();
    for kind in ["Object", "Material", "NodeGroup", "Image"] {
        assert_eq!(schema["types"][kind]["label"], "name", "{kind}");
    }
}

#[test]
fn carried_counts_active_dependencies_once_including_hidden_descendants() {
    let (mut doc, cube, child, target) = scene();
    doc.execute("object.parent", json!({"objects":[child],"parent":cube})).unwrap();
    doc.execute("object.parent", json!({"objects":[cube],"parent":target})).unwrap();
    doc.execute("object.hide", json!({"objects":[child]})).unwrap();
    doc.execute("constraint.add", json!({"object":cube,"type":"copyLocation","target":target})).unwrap();
    let moved = json!({"id":target,"fields":["location.2"],"edits":[{"field":"location.2","before":0,"after":1}]});
    let carried = doc.carried(&[json!(target)], std::slice::from_ref(&moved)).unwrap();
    assert_eq!(carried.len(), 1);
    assert_eq!(carried[0].carries.count, 2, "a parent that is also a target counts each dependent once");

    doc.execute("select.objects", json!({"objects":[cube]})).unwrap();
    doc.execute("object.clearParent", json!({"objects":[cube]})).unwrap();
    doc.execute("constraint.set", json!({"object":cube,"index":0,"enabled":false})).unwrap();
    assert!(doc.carried(&[json!(target)], std::slice::from_ref(&moved)).unwrap().is_empty(), "a disabled constraint carries nothing");
    doc.execute("constraint.set", json!({"object":cube,"index":0,"enabled":true,"influence":0})).unwrap();
    assert!(doc.carried(&[json!(target)], &[moved]).unwrap().is_empty(), "zero influence carries nothing");
}

#[test]
fn carries_require_a_transform_field_in_the_changed_entry() {
    let (mut doc, cube, child, _) = scene();
    doc.execute("object.parent", json!({"objects":[child],"parent":cube})).unwrap();
    for field in ["location", "location.2", "rotation.0", "scale.1", "parent", "parentInverse.3", "constraints.0.influence"] {
        for change in [json!({"id":cube,"fields":[field]}), json!({"id":cube.to_string(),"edits":[{"field":field}]})] {
            assert_eq!(doc.carried(&[json!(cube)], &[change]).unwrap()[0].carries.count, 1, "{field}");
        }
    }
    for field in ["name", "hidden", "data", "mesh", "material", "animation", "modifiers", "constraints.0.name"] {
        assert!(doc.carried(&[json!(cube)], &[json!({"id":cube,"fields":[field]})]).unwrap().is_empty(), "{field}");
    }
    assert!(
        doc.carried(&[json!(cube)], &[json!({"id":cube,"fields":["constraints"],"edits":[{"field":"constraints.0.name","before":"A","after":"B"}]})])
            .unwrap()
            .is_empty(),
        "expanded edits distinguish constraint labels from the whole constraints field"
    );
    assert!(doc.carried(&[json!(cube)], &[]).unwrap().is_empty());
    assert!(doc.carried(&[json!(cube)], &[json!({"id":cube,"from":{"index":0},"to":{"index":1}})]).unwrap().is_empty());
    assert!(doc.carried(&[json!(cube)], &[json!({"id":child,"fields":["location"]})]).unwrap().is_empty());
}

#[test]
fn covered_rebuilds_nested_parents_and_visibility_without_editing_the_document() {
    use kition_kit::CoveredAsk;
    let (mut doc, cube, child, root) = scene();
    doc.execute("object.parent", json!({"objects":[child],"parent":cube})).unwrap();
    doc.execute("object.parent", json!({"objects":[cube],"parent":root})).unwrap();
    doc.execute("constraint.add", json!({"object":cube,"type":"copyLocation","target":root,"influence":0})).unwrap();
    let old_root = doc.scene().object(root as u32).unwrap().location;
    doc.execute("object.translate", json!({"objects":[root],"offset":[100,0,0]})).unwrap();
    doc.execute("object.hide", json!({"objects":[child]})).unwrap();
    let before = doc.inspect().unwrap();
    let revision = doc.revision();
    let asked = vec![
        CoveredAsk { id: json!(root), before: serde_json::from_value(json!({"location":old_root})).unwrap(), ..Default::default() },
        CoveredAsk { id: json!(child), before: serde_json::from_value(json!({"hidden":false})).unwrap(), ..Default::default() },
    ];
    let covered = doc.covered("object.rename", &json!({"pick":[0.3,0,0]}), &asked).unwrap().unwrap();
    assert_eq!(covered.under, vec![json!(child)]);
    assert!(covered.covered.iter().all(|entry| entry.covered == Some(true)));
    assert_eq!(doc.inspect().unwrap(), before);
    assert_eq!(doc.revision(), revision);

    let material = doc.execute("material.new", json!({"name":"Shared","assign":false})).unwrap()["material"].clone();
    doc.execute("material.assign", json!({"objects":[cube,child],"material":material})).unwrap();
    assert!(doc.carried(&[material], &[]).unwrap().is_empty(), "shared material data carries no object world transform");
}

#[test]
fn live_pick_and_covered_replay_agree_on_rotated_mesh_bounds() {
    use kition_kit::CoveredAsk;
    let (mut doc, ..) = scene();
    let sphere = doc
        .execute("object.addIcoSphere", json!({"name":"Rotated","radius":1,"subdivisions":1,"location":[10,0,0],"rotation":[0,0,45]}))
        .unwrap()["object"]
        .clone();
    let bounds = doc.derived(std::slice::from_ref(&sphere), &["bounds".into()]).unwrap()[0].values["bounds"].clone();
    let asked = CoveredAsk { id: sphere, before: serde_json::from_value(json!({"~bounds":bounds})).unwrap(), ..Default::default() };
    let params = json!({"pick":[11.15,0,0]});
    let live = doc.under("object.rename", &params).unwrap().unwrap();
    let replay = doc.covered("object.rename", &params, &[asked]).unwrap().unwrap();
    assert_eq!(replay.under, live.objects, "unchanged geometry must pick the same object in both hooks");
}
