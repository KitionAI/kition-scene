//! Object parameters (task card C8-scene-2): kition-sdk reads a whole number in a parameter other
//! than a command's targets as an object's id only where the command marks the parameter
//! (`KitCommand::objects`, `"x-khp-role": "object"`). A parameter that takes an object, a
//! material, a node group or a node by id and is not marked silently loses its premise: an agent's
//! write with `basis` would not be refused after others changed what it names. This test walks
//! every command's schema, its parameters and their members one level deep (as far as the guard
//! reads), and fails for an integer (or a `oneOf` with an integer) whose name or description says
//! it names such a thing and which is neither marked, nor the targets, nor on the allow-list of
//! counts and indexes below.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeSet;

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::Value;

/// Integer parameters whose name or description reads like an id but which name no object of the
/// scene's id space, with the reason.
const NOT_IDS: [(&str, &str); 6] = [
    ("chain", "a count of bones in an IK chain"),
    ("rings", "a sphere's ring count (its description says `pole`)"),
    ("volumeBounces", "a count of light bounces in a volume"),
    ("childCount", "a count of hair children per strand"),
    ("count", "a count of particles"),
    ("faceSet", "a face set's number inside one mesh, not a tool id"),
];

/// Also not ids, by command: an index into a list the object holds.
const INDEXES: [(&str, &str, &str); 3] = [
    ("nodes.newGroup", "modifier", "the Geometry Nodes modifier's index in the object's stack"),
    ("nla.makeMeta", "first", "a strip's index in its NLA track"),
    ("nla.makeMeta", "last", "a strip's index in its NLA track"),
];

/// A point's coordinates are not ids. The scalar form is guarded by `under`, tested below
/// and through the headless server, including a deleted scalar id.
const CONDITIONAL_OBJECTS: [(&str, &str); 6] = [
    ("camera.move", "target"),
    ("camera.move", "to"),
    ("light.studio", "target"),
    ("object.hop", "to"),
    ("object.gather", "center"),
    ("object.wave", "origin"),
];

// commands_particles::system_of resolves these inside an Object's JSON particles array,
// not among model records. Their integer form is a local index, not a tool id.
const LOCAL_NAMES: [(&str, &str); 5] = [
    ("particles.set", "system"),
    ("particles.remove", "system"),
    ("particles.convert", "system"),
    ("particles.stats", "system"),
    ("mode.particleEdit", "system"),
];

fn takes_integer(node: &Value) -> bool {
    node.get("type").and_then(Value::as_str) == Some("integer")
        || node.get("items").is_some_and(takes_integer)
        || node.get("oneOf").and_then(Value::as_array).is_some_and(|branches| branches.iter().any(takes_integer))
}

/// Words that say a parameter may name an object, a material, a node group or a node.
fn reads_like_an_id(name: &str, node: &Value) -> bool {
    const WORDS: [&str; 22] = [
        "id",
        "ids",
        "object",
        "objects",
        "material",
        "materials",
        "node",
        "nodes",
        "group",
        "camera",
        "armature",
        "parent",
        "target",
        "image",
        "collider",
        "colliders",
        "operand",
        "bone",
        "shader",
        "volume",
        "pole",
        "shape",
    ];
    let text = format!("{name} {node}").to_lowercase();
    text.split(|c: char| !c.is_ascii_alphanumeric()).any(|word| WORDS.contains(&word))
}

#[test]
fn every_parameter_that_takes_an_id_is_marked_as_naming_objects() {
    let doc = SceneDocument::create("Scene.kscene", 320, 180).unwrap();
    let allow: Vec<_> = [("sculpt.faceSetVisibility", "faceSet")].into_iter().chain(CONDITIONAL_OBJECTS).collect();
    kition_kit::testing::check_id_params(&doc.commands(), &allow).unwrap();
    let not_ids: BTreeSet<&str> = NOT_IDS.iter().map(|(name, _)| *name).collect();
    let mut missed = Vec::new();
    let mut marked = 0;
    for command in doc.commands() {
        let Some(schema) = command.schema.as_ref() else { continue };
        let properties = schema.get("properties").unwrap_or(schema).as_object().cloned().unwrap_or_default();
        for (name, node) in &properties {
            if Some(name.as_str()) == command.targets || command.places.contains(&name.as_str()) {
                continue;
            }
            if CONDITIONAL_OBJECTS.contains(&(command.id.as_str(), name.as_str())) {
                continue;
            }
            // Keep the older, broader vocabulary audit beside the SDK's scanner. Each id member
            // must now have its own mark, including `elements.object`.
            let members: Vec<(String, Value)> = node
                .get("properties")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .map(|(key, member)| (format!("{name}.{key}"), member.clone()))
                .collect();
            let wanted = std::iter::once((name.clone(), node.clone()))
                .chain(members)
                .filter(|(path, node)| takes_integer(node) && reads_like_an_id(path.rsplit('.').next().unwrap(), node))
                .filter(|(path, _)| !not_ids.contains(path.as_str()))
                .filter(|(path, _)| !INDEXES.iter().any(|(id, parameter, _)| *id == command.id && parameter == path))
                .map(|(path, _)| path)
                .collect::<Vec<_>>();
            if wanted.is_empty() {
                continue;
            }
            if wanted.iter().all(|path| command.objects.contains(&path.as_str())) {
                marked += 1;
            } else {
                missed.push(format!("{} {name}: {node}", command.id));
            }
        }
    }
    assert!(missed.is_empty(), "{} parameters take an id and are not marked (`KitCommand::objects`):\n{}", missed.len(), missed.join("\n"));
    println!("{marked} parameters marked as naming objects");
    assert!(marked > 50, "{marked}");
}

#[test]
fn counts_and_indexes_are_not_marked() {
    let doc = SceneDocument::create("Scene.kscene", 320, 180).unwrap();
    for command in doc.commands() {
        for (name, _) in NOT_IDS {
            assert!(!command.objects.contains(&name), "{} marks {name}", command.id);
        }
        for (id, name, _) in INDEXES {
            if command.id == id {
                assert!(!command.objects.contains(&name), "{id} marks {name}");
            }
        }
        // An element index list is never an object parameter.
        for name in ["index", "vertices", "faces", "levels", "steps", "segments", "frame"] {
            if command.objects.contains(&name) {
                panic!("{} marks {name}", command.id);
            }
        }
    }
}

#[test]
fn nested_object_members_are_marked_at_the_member() {
    let doc = SceneDocument::create("Scene.kscene", 320, 180).unwrap();
    let commands = doc.commands();
    for (id, path) in [
        ("mesh.extrude", "elements.object"),
        ("particles.set", "boids.rules[].target"),
        ("constraint.add", "targets[].object"),
        ("nodes.link", "from"),
        ("nodes.link", "to"),
        ("nodes.link", "group"),
    ] {
        let command = commands.iter().find(|command| command.id == id).unwrap_or_else(|| panic!("no {id}"));
        assert!(command.objects.contains(&path), "{id} misses {path}: {:?}", command.objects);
    }
}

#[test]
fn object_parameters_declare_the_type_the_command_resolves() {
    let doc = SceneDocument::create("Scene.kscene", 320, 180).unwrap();
    for command in doc.commands() {
        for path in command.targets.into_iter().chain(command.objects.iter().copied()) {
            assert!(command.object_types.iter().any(|(name, types)| *name == path && !types.is_empty()), "{} lacks the types for {path}", command.id);
        }
        if command.targets.is_some() {
            assert_eq!(command.name_field, Some("name"), "{}", command.id);
        }
    }
}

#[test]
fn types_follow_the_command_resolver_instead_of_the_parameter_spelling() {
    let doc = SceneDocument::create("Scene.kscene", 320, 180).unwrap();
    let commands = doc.commands();
    for (id, path, expected) in [
        ("graph.moveKey", "target", &["Object", "Material"][..]),
        ("graph.deleteKey", "target", &["Object", "Material"][..]),
        ("graph.setInterpolation", "target", &["Object", "Material"][..]),
        ("graph.addModifier", "target", &["Object", "Material"][..]),
        ("graph.setModifier", "target", &["Object", "Material"][..]),
        ("graph.removeModifier", "target", &["Object", "Material"][..]),
        ("camera.move", "to", &["Object"][..]),
        ("object.hop", "to", &["Object"][..]),
        ("shapeKey.add", "from", &["Object"][..]),
    ] {
        let command = commands.iter().find(|command| command.id == id).unwrap();
        assert_eq!(command.object_types.iter().find(|(name, _)| *name == path).map(|(_, types)| *types), Some(expected), "{id} {path}");
    }
}

#[test]
fn driver_variable_references_are_marked_without_their_component_indexes() {
    let doc = SceneDocument::create("Scene.kscene", 320, 180).unwrap();
    for command in doc.commands().into_iter().filter(|command| matches!(command.id.as_str(), "driver.add" | "driver.set")) {
        for path in ["variables[].object", "variables[].a", "variables[].b"] {
            assert!(command.objects.contains(&path), "{} misses {path}", command.id);
        }
        assert!(!command.objects.contains(&"variables"));
        assert!(!command.objects.contains(&"variables[].index"));
    }
}

#[test]
fn point_or_object_parameters_use_conditional_premises_instead_of_marking_coordinates() {
    let doc = SceneDocument::create("Scene.kscene", 320, 180).unwrap();
    let commands = doc.commands();
    for (id, path) in [
        ("camera.move", "target"),
        ("camera.move", "to"),
        ("light.studio", "target"),
        ("object.hop", "to"),
        ("object.gather", "center"),
        ("object.wave", "origin"),
    ] {
        let command = commands.iter().find(|command| command.id == id).unwrap();
        assert!(!command.objects.contains(&path), "{id} {path} would mark coordinates as ids");
        assert!(command.name_params.contains(&(path, "name")), "{id} {path} guards scalar names");
        assert!(command.schema.as_ref().unwrap()[path].get("x-khp-name").is_none(), "{id} {path} needs no inline workaround");
        let scalar = serde_json::json!({path: 1});
        assert!(doc.under(id, &scalar).unwrap().unwrap().premises.iter().any(|p| p.id == 1), "{id} {path}");
        let point = doc.under(id, &serde_json::json!({path: [1, 0, 0]})).unwrap().unwrap();
        // A point names no object: no premise on an id its coordinates spell (the call still rests
        // on the scene camera, the selection or the timeline: declarations_independent).
        assert!(!point.premises.iter().any(|p| p.id == 1 || p.id == 0) && point.objects.is_empty() && point.within.is_empty(), "{id} {path}");
    }
}

fn name_declaration_gaps(commands: &[kition_kit::KitCommand]) -> Vec<String> {
    fn takes_string(node: &Value) -> bool {
        node["type"] == "string"
            || node.get("items").is_some_and(takes_string)
            || node.get("oneOf").and_then(Value::as_array).is_some_and(|items| items.iter().any(takes_string))
    }
    fn walk(command: &kition_kit::KitCommand, node: &Value, path: &str, gaps: &mut Vec<String>) {
        let description = node["description"].as_str().unwrap_or_default();
        // Audited against params::resolve, material_ref_value, group_id/shader_group and
        // the scalar forms of point_or_object. Nodes, graph targets and image hashes take ids.
        let names = !LOCAL_NAMES.contains(&(command.id.as_str(), path))
            && ((takes_integer(node) && takes_string(node))
                || (node["type"] == "json" && description.contains("id or name"))
                || CONDITIONAL_OBJECTS.contains(&(command.id.as_str(), path)));
        if names {
            let declared =
                if command.targets == Some(path) { command.name_field == Some("name") } else { command.name_params.contains(&(path, "name")) };
            if !declared || !command.object_types.iter().any(|(parameter, types)| *parameter == path && !types.is_empty()) {
                gaps.push(format!("{} {path}", command.id));
            }
        }
        if let Some(properties) = node.get("properties").and_then(Value::as_object) {
            for (name, child) in properties {
                walk(command, child, &format!("{path}.{name}"), gaps);
            }
        }
        if let Some(items) = node.get("items").filter(|items| items.get("properties").is_some()) {
            walk(command, items, &format!("{path}[]"), gaps);
        }
    }
    let mut gaps = Vec::new();
    for command in commands {
        if let Some(properties) = command.schema.as_ref().and_then(|schema| schema.get("properties").unwrap_or(schema).as_object()) {
            for (name, node) in properties {
                walk(command, node, name, &mut gaps);
            }
        }
    }
    gaps
}

#[test]
fn every_parameter_that_takes_a_name_is_declared() {
    let doc = SceneDocument::create("Scene.kscene", 320, 180).unwrap();
    let commands = doc.commands();
    let gaps = name_declaration_gaps(&commands);
    assert!(gaps.is_empty(), "undeclared name parameters:\n{}", gaps.join("\n"));
    let count: usize = commands.iter().map(|command| command.name_params.len()).sum();
    println!("{count} non-target name parameters declared");
    for (id, path) in LOCAL_NAMES {
        let command = commands.iter().find(|command| command.id == id).unwrap();
        assert!(!command.name_params.iter().any(|(parameter, _)| *parameter == path), "{id} {path} is a local selector");
        assert!(!command.object_types.iter().any(|(parameter, _)| *parameter == path), "{id} {path} names no model record");
    }

    let mut missing = commands.clone();
    missing.iter_mut().find(|command| command.id == "material.set").unwrap().name_params = &[("shader", "name")];
    assert_eq!(name_declaration_gaps(&missing), ["material.set material"], "the scanner must catch a removed declaration");
    let mut missing = commands;
    missing.iter_mut().find(|command| command.id == "light.studio").unwrap().name_params = &[];
    assert_eq!(name_declaration_gaps(&missing), ["light.studio target"], "the scanner must also catch a removed union declaration");
}
