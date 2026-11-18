//! Drivers, keying sets, actions, the new constraints, and dual-quaternion skinning through
//! commands, as the agent and the editor drive them; and that all of it survives saving.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn object<'a>(inspected: &'a Value, name: &str) -> &'a Value {
    inspected["objects"].as_array().unwrap().iter().find(|object| object["name"] == name).unwrap_or_else(|| panic!("no {name}"))
}

fn number(value: &Value) -> f64 {
    value.as_f64().unwrap_or_else(|| panic!("not a number: {value}"))
}

fn reopen(doc: &SceneDocument) -> SceneDocument {
    SceneDocument::open("A.kscene", &doc.save(None).unwrap()).unwrap()
}

#[test]
fn drivers_follow_their_sources_and_survive_saving() {
    let mut doc = SceneDocument::create("A.kscene", 1280, 720).unwrap();
    run(&mut doc, "add.empty", json!({ "name": "Handle", "location": [1, 0, 0] }));
    run(
        &mut doc,
        "driver.add",
        json!({
            "object": "Cube",
            "property": "location",
            "index": 2,
            "expression": "h * 2 + 0.5",
            "variables": [{ "name": "h", "type": "transform", "object": "Handle", "channel": "locationX" }]
        }),
    );
    let z = |doc: &SceneDocument| number(&object(&doc.inspect().unwrap(), "Cube")["location"][2]);
    assert!((z(&doc) - 2.5).abs() < 1e-4);
    // Moving the source moves the driven value with the same step.
    run(&mut doc, "object.transform", json!({ "objects": "Handle", "location": [3, 0, 0] }));
    assert!((z(&doc) - 6.5).abs() < 1e-4);
    let drivers = doc.inspect().unwrap()["animation"]["drivers"].clone();
    assert_eq!(drivers[0]["expression"], "h * 2 + 0.5");
    assert_eq!(drivers[0]["variables"][0]["channel"], "locationX");
    assert!((number(&drivers[0]["value"]) - 6.5).abs() < 1e-4);
    // A driven property takes no keys, and a keyed one is not driven over its keys.
    assert!(doc.execute("anim.insertKey", json!({ "objects": "Cube", "properties": ["location"] })).is_err());
    run(&mut doc, "anim.insertKey", json!({ "objects": "Handle", "properties": ["location"] }));
    assert!(doc.execute("driver.add", json!({ "object": "Handle", "property": "location", "index": 0 })).is_err());
    // A broken expression is refused; the driver stays as it was.
    assert!(doc.execute("driver.set", json!({ "object": "Cube", "property": "location", "index": 2, "expression": "h *" })).is_err());
    // The driver is saved; the stored location is the undriven one, so it reads the same back.
    let back = reopen(&doc);
    assert!((z(&back) - 6.5).abs() < 1e-4);
    assert_eq!(back.inspect().unwrap()["animation"]["drivers"], doc.inspect().unwrap()["animation"]["drivers"]);
    // Kinds other than scripted, a distance, and removal.
    run(
        &mut doc,
        "driver.set",
        json!({ "object": "Cube", "property": "location", "index": 2, "kind": "max", "variables": [
            { "name": "gap", "type": "distance", "a": "Handle", "b": "Light" },
            { "name": "base", "type": "singleProperty", "object": "Handle", "property": "location", "index": 0 }
        ] }),
    );
    let inspected = doc.inspect().unwrap();
    let handle = object(&inspected, "Handle")["location"].clone();
    let light = object(&inspected, "Light")["location"].clone();
    let gap = (0..3).map(|i| (number(&handle[i]) - number(&light[i])).powi(2)).sum::<f64>().sqrt();
    assert!((z(&doc) - gap.max(3.0)).abs() < 1e-3);
    run(&mut doc, "driver.remove", json!({ "object": "Cube", "property": "location" }));
    assert!(doc.inspect().unwrap()["animation"]["drivers"].as_array().unwrap().is_empty());
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(doc.inspect().unwrap()["animation"]["drivers"].as_array().unwrap().len(), 1);
}

#[test]
fn keying_sets_choose_what_insert_keyframe_keys() {
    let mut doc = SceneDocument::create("A.kscene", 1280, 720).unwrap();
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "keyingSet.setActive", json!({ "name": "locRot" }));
    run(&mut doc, "anim.insertKey", json!({}));
    let properties = |doc: &SceneDocument| object(&doc.inspect().unwrap(), "Cube")["animated"].clone();
    assert_eq!(properties(&doc), json!(["location", "rotation"]));
    // Available keys only what is keyed already.
    run(&mut doc, "anim.insertKey", json!({ "keyingSet": "available", "frame": 10 }));
    assert_eq!(doc.inspect().unwrap()["animation"]["keyFrames"], json!([1.0, 10.0]));
    // A set of the scene's own: the light's power, whatever is selected.
    let made = run(&mut doc, "keyingSet.new", json!({ "name": "Lighting" }));
    assert_eq!(made["name"], "Lighting");
    run(&mut doc, "keyingSet.addPaths", json!({ "objects": "Light", "properties": ["power", "color"] }));
    run(&mut doc, "anim.insertKey", json!({ "frame": 5 }));
    let light = object(&doc.inspect().unwrap(), "Light")["animated"].clone();
    assert_eq!(light, json!(["power", "color"]));
    let sets = doc.inspect().unwrap()["animation"]["keyingSets"].clone();
    assert_eq!(sets["active"], "Lighting");
    assert_eq!(sets["sets"][0]["paths"].as_array().unwrap().len(), 2);
    let renamed = run(&mut doc, "keyingSet.new", json!({ "name": "location" }))["name"].clone();
    assert_ne!(renamed, "location", "a built-in's name is not taken");
    run(&mut doc, "keyingSet.remove", json!({ "name": renamed }));
    run(&mut doc, "keyingSet.setActive", json!({ "name": "Lighting" }));
    let back = reopen(&doc);
    assert_eq!(back.inspect().unwrap()["animation"]["keyingSets"], doc.inspect().unwrap()["animation"]["keyingSets"]);
    run(&mut doc, "keyingSet.removePath", json!({ "name": "Lighting", "index": 1 }));
    run(&mut doc, "keyingSet.remove", json!({ "name": "Lighting" }));
    assert!(doc.inspect().unwrap()["animation"]["keyingSets"]["active"].is_null());

    // Whole Character keys every bone of an armature in pose mode.
    run(&mut doc, "add.armature", json!({ "name": "Rig" }));
    run(&mut doc, "mode.armature", json!({}));
    run(&mut doc, "bone.select", json!({ "bones": "Bone" }));
    run(&mut doc, "bone.extrude", json!({}));
    run(&mut doc, "mode.armature", json!({}));
    run(&mut doc, "mode.pose", json!({ "object": "Rig" }));
    run(&mut doc, "bone.select", json!({ "bones": "Bone" }));
    let keyed = run(&mut doc, "pose.insertKey", json!({ "keyingSet": "wholeCharacter", "frame": 1 }));
    assert_eq!(keyed["keys"], 18, "two bones, three properties, three components");
}

#[test]
fn actions_stash_assign_and_play_through_a_constraint() {
    let mut doc = SceneDocument::create("A.kscene", 1280, 720).unwrap();
    run(&mut doc, "anim.insertKey", json!({ "objects": "Cube", "properties": ["location"], "frame": 1 }));
    run(&mut doc, "object.transform", json!({ "objects": "Cube", "location": [0, 0, 4] }));
    run(&mut doc, "anim.insertKey", json!({ "objects": "Cube", "properties": ["location"], "frame": 11 }));
    let made = run(&mut doc, "action.new", json!({ "name": "Rise", "object": "Cube" }));
    assert_eq!(made["channels"], 3);
    // Another object takes the action's keys.
    run(&mut doc, "add.empty", json!({ "name": "Twin", "location": [5, 0, 0] }));
    run(&mut doc, "action.assign", json!({ "objects": "Twin", "action": "Rise" }));
    run(&mut doc, "frame.set", json!({ "frame": 11 }));
    assert!((number(&object(&doc.inspect().unwrap(), "Twin")["location"][2]) - 4.0).abs() < 1e-4);
    run(&mut doc, "frame.set", json!({ "frame": 1 }));

    // An Action constraint plays it from a handle's height: half way up at 0.5.
    run(&mut doc, "add.empty", json!({ "name": "Handle", "location": [0, 3, 0.5] }));
    run(&mut doc, "add.empty", json!({ "name": "Puppet", "location": [0, -3, 0] }));
    run(
        &mut doc,
        "constraint.add",
        json!({ "object": "Puppet", "type": "action", "target": "Handle", "action": "Rise", "channel": "locationZ", "min": 0, "max": 1, "frameStart": 1, "frameEnd": 11, "mix": "replace" }),
    );
    let puppet = object(&doc.inspect().unwrap(), "Puppet").clone();
    assert!((number(&puppet["worldLocation"][2]) - 2.0).abs() < 1e-3, "{}", puppet["worldLocation"]);
    assert_eq!(puppet["constraints"][0]["channel"], "locationZ");
    // Renaming follows into the constraint; removing lets it go.
    run(&mut doc, "action.rename", json!({ "name": "Rise", "to": "Lift" }));
    assert_eq!(object(&doc.inspect().unwrap(), "Puppet")["constraints"][0]["action"], "Lift");
    let back = reopen(&doc);
    assert_eq!(back.inspect().unwrap()["animation"]["actions"], doc.inspect().unwrap()["animation"]["actions"]);
    assert!((number(&object(&back.inspect().unwrap(), "Puppet")["worldLocation"][2]) - 2.0).abs() < 1e-3);
    run(&mut doc, "action.remove", json!({ "name": "Lift" }));
    assert!(object(&doc.inspect().unwrap(), "Puppet")["constraints"][0].get("action").is_none());
}

#[test]
fn the_new_constraints_through_commands() {
    let mut doc = SceneDocument::create("A.kscene", 1280, 720).unwrap();
    run(&mut doc, "add.empty", json!({ "name": "Goal", "location": [0, 0, 6] }));
    // Stretch To takes the distance now as its rest length, then stretches.
    run(&mut doc, "constraint.add", json!({ "object": "Cube", "type": "stretchTo", "target": "Goal" }));
    let cube = object(&doc.inspect().unwrap(), "Cube").clone();
    assert!((number(&cube["constraints"][0]["restLength"]) - 6.0).abs() < 1e-4);
    run(&mut doc, "object.transform", json!({ "objects": "Goal", "location": [0, 0, 12] }));
    // Applied, the stretch is the cube's own scale: twice as long along the goal's line, the
    // other axes thinner by the square root.
    run(&mut doc, "constraint.apply", json!({ "object": "Cube", "index": 0 }));
    let scale = object(&doc.inspect().unwrap(), "Cube")["scale"].clone();
    let mut sizes: Vec<f64> = (0..3).map(|i| number(&scale[i]).abs()).collect();
    sizes.sort_by(f64::total_cmp);
    assert!((sizes[2] - 2.0).abs() < 1e-3 && (sizes[0] - 0.5f64.sqrt()).abs() < 1e-3, "{scale}");
    run(&mut doc, "object.transform", json!({ "objects": "Cube", "rotation": [0, 0, 0], "scale": [1, 1, 1] }));

    // Floor keeps the cube above the goal's plane.
    run(&mut doc, "constraint.add", json!({ "object": "Cube", "type": "floor", "target": "Goal", "offset": 0.5 }));
    assert!((number(&object(&doc.inspect().unwrap(), "Cube")["worldLocation"][2]) - 12.5).abs() < 1e-4);
    run(&mut doc, "constraint.set", json!({ "object": "Cube", "index": 0, "axis": "-z" }));
    assert!((number(&object(&doc.inspect().unwrap(), "Cube")["worldLocation"][2]) - 0.0).abs() < 1e-4);
    run(&mut doc, "constraint.remove", json!({ "object": "Cube", "index": 0 }));

    // Transformation: the goal's height turns the cube, in degrees.
    run(
        &mut doc,
        "constraint.add",
        json!({ "object": "Cube", "type": "transform", "target": "Goal", "mapTo": "rotation", "fromMax": [0, 0, 12], "toMax": [0, 0, 90], "mapping": ["x", "y", "z"], "ownerSpace": "local" }),
    );
    let transform = object(&doc.inspect().unwrap(), "Cube")["constraints"][0].clone();
    assert_eq!(transform["toMax"][2], 90.0);
    assert_eq!(transform["ownerSpace"], "local");
    run(&mut doc, "constraint.remove", json!({ "object": "Cube", "index": 0 }));
    // Bones-only spaces and chain constraints are refused on objects.
    assert!(doc.execute("constraint.add", json!({ "object": "Cube", "type": "copyLocation", "target": "Goal", "ownerSpace": "pose" })).is_err());
    assert!(doc.execute("constraint.add", json!({ "object": "Cube", "type": "splineIk" })).is_err());

    // Armature: the cube follows a bone like a skinned vertex, and with dual quaternions.
    run(&mut doc, "add.armature", json!({ "name": "Rig", "location": [3, 0, 0] }));
    run(
        &mut doc,
        "constraint.add",
        json!({ "object": "Cube", "type": "armature", "targets": [{ "object": "Rig", "bone": "Bone", "weight": 1 }], "preserveVolume": true }),
    );
    let before = object(&doc.inspect().unwrap(), "Cube")["worldLocation"].clone();
    run(&mut doc, "mode.pose", json!({ "object": "Rig" }));
    run(&mut doc, "bone.select", json!({ "bones": "Bone" }));
    run(&mut doc, "pose.set", json!({ "rotation": [0, 0, 90] }));
    run(&mut doc, "mode.pose", json!({ "on": false }));
    let after = object(&doc.inspect().unwrap(), "Cube")["worldLocation"].clone();
    assert!((number(&after[0]) - number(&before[0])).abs() > 1.0, "{before} -> {after}");
    let armature = object(&doc.inspect().unwrap(), "Cube")["constraints"][0].clone();
    assert_eq!(armature["targets"][0]["name"], "Rig");
    assert_eq!(armature["targets"][0]["bone"], "Bone");

    // Spline IK on a bone along a path curve.
    run(&mut doc, "add.curvePath", json!({ "name": "Path", "location": [0, 5, 0] }));
    run(&mut doc, "constraint.add", json!({ "object": "Rig", "bone": "Bone", "type": "splineIk", "target": "Path", "chain": 1 }));
    assert!(doc.execute("constraint.set", json!({ "object": "Rig", "bone": "Bone", "index": 0, "target": "Cube" })).is_err(), "a curve only");
    let bones = run(&mut doc, "file.export", json!({ "format": "glb" }));
    assert!(bones["base64"].is_string());

    // Everything survives saving.
    let back = reopen(&doc);
    assert_eq!(object(&back.inspect().unwrap(), "Cube")["constraints"], object(&doc.inspect().unwrap(), "Cube")["constraints"]);
    assert_eq!(object(&back.inspect().unwrap(), "Rig")["boneConstraints"], object(&doc.inspect().unwrap(), "Rig")["boneConstraints"]);
}

#[test]
fn the_armature_modifier_blends_dual_quaternions_on_request() {
    let mut doc = SceneDocument::create("A.kscene", 1280, 720).unwrap();
    run(&mut doc, "edit.delete", json!({ "objects": "Cube" }));
    run(&mut doc, "add.cylinder", json!({ "name": "Column", "vertices": 8, "radius": 0.2, "depth": 2, "location": [0, 0, 1] }));
    run(&mut doc, "select.objects", json!({ "objects": "Column" }));
    run(&mut doc, "mode.toggle", json!({}));
    run(&mut doc, "select.all", json!({}));
    run(&mut doc, "mesh.subdivide", json!({}));
    run(&mut doc, "mesh.subdivide", json!({}));
    run(&mut doc, "mode.toggle", json!({}));
    run(&mut doc, "add.armature", json!({}));
    run(&mut doc, "mode.armature", json!({}));
    run(&mut doc, "bone.select", json!({ "bones": "Bone" }));
    run(&mut doc, "bone.extrude", json!({}));
    run(&mut doc, "mode.armature", json!({}));
    run(&mut doc, "select.objects", json!({ "objects": ["Column", "Armature"] }));
    run(&mut doc, "object.parentArmature", json!({ "weights": "automatic" }));
    run(&mut doc, "mode.pose", json!({ "object": "Armature" }));
    run(&mut doc, "bone.select", json!({ "bones": "Bone.001" }));
    run(&mut doc, "pose.set", json!({ "rotation": [120, 0, 0] }));
    run(&mut doc, "mode.pose", json!({ "on": false }));
    let width = |doc: &SceneDocument| {
        let column = object(&doc.inspect().unwrap(), "Column").clone();
        (0..3).map(|axis| number(&column["dimensions"][axis])).collect::<Vec<_>>()
    };
    let linear = width(&doc);
    run(&mut doc, "modifier.set", json!({ "object": "Column", "index": 0, "preserveVolume": true }));
    assert_eq!(object(&doc.inspect().unwrap(), "Column")["modifiers"][0]["preserveVolume"], true);
    let kept = width(&doc);
    let apart = linear.iter().zip(&kept).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(apart > 1e-3, "bending the joint blends differently: {linear:?} and {kept:?}");
}

#[test]
fn a_released_090_file_takes_drivers_actions_and_keying_sets() {
    let mut doc = SceneDocument::open("Room.kscene", include_bytes!("fixtures/released-0.9.0-rig.kscene")).unwrap();
    run(&mut doc, "keyingSet.new", json!({ "name": "Mine" }));
    run(&mut doc, "keyingSet.addPaths", json!({ "objects": "Ball" }));
    run(&mut doc, "anim.insertKey", json!({ "objects": "Ball", "properties": ["location"] }));
    run(&mut doc, "action.new", json!({ "name": "Bounce", "object": "Ball" }));
    run(&mut doc, "driver.add", json!({ "object": "Cube", "property": "scale", "index": 0, "expression": "2" }));
    run(&mut doc, "constraint.add", json!({ "object": "Rig", "bone": "Bone", "type": "stretchTo", "target": "Ball", "volume": "none" }));
    let back = reopen(&doc);
    let inspected = back.inspect().unwrap();
    assert_eq!(inspected["animation"]["keyingSets"]["sets"][0]["name"], "Mine");
    assert_eq!(inspected["animation"]["actions"][0]["name"], "Bounce");
    assert_eq!(inspected["animation"]["drivers"][0]["expression"], "2");
    assert!((number(&object(&inspected, "Cube")["scale"][0]) - 2.0).abs() < 1e-5);
    assert_eq!(object(&inspected, "Rig")["boneConstraints"]["Bone"][0]["type"], "stretchTo");
}
