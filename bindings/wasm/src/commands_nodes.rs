//! Node groups: making and removing groups, adding, moving, and removing nodes, setting their
//! values, linking sockets, the group's inputs and outputs, and zones' items (Geometry Nodes ▸
//! ...). A Group node (`group`) runs another geometry group, named by its `group` value, with
//! that group's interface as its sockets; a group may not use itself. Adding a zone's input
//! (`repeatInput`, `simulationInput`) adds its output too, paired and linked. A Geometry Nodes
//! modifier names a geometry group (`modifier.set` with `group` and `inputs`); renders pass
//! through the compositing group the render settings name (`compositor.*`); a material's surface
//! comes from the shader group it names (`shader.*`). These same commands edit the nodes of every
//! kind of group. Each change is one undo step; a failing command changes nothing.

use std::collections::BTreeMap;

use kition_kit::KitCommand;
use kition_scene_model::compositor_kinds::COMPOSITOR_KINDS;
use kition_scene_model::node_tree::{self, GroupSocket, Link, Node, NodeGroup, NodeKind, SocketType, Tree};
use kition_scene_model::shader_kinds::SHADER_KINDS;
use kition_scene_model::{Id, Modifier, ModifierKind, Scene};
use serde_json::{Value, json};

use crate::SceneDocument;
use crate::params::{self, bad, schema as s};

const MENU: &[&str] = &["Geometry Nodes"];

/// The commands this file adds: id, title, whether it is in the menu, whether it changes the
/// scene, and its parameters.
type Entry = (&'static str, &'static str, bool, bool, fn() -> Value);

fn group_ref() -> (&'static str, Value) {
    ("group", json!({ "type": "json", "description": "The group's id or name; the only group when left out" }))
}

/// A node named by its id (the description says so: the premise guard reads such a parameter's
/// whole numbers as ids, `params::object_parameters`).
fn node_ref(description: &str) -> Value {
    s::integer(&format!("{description}: its id"))
}

fn types() -> Vec<&'static str> {
    vec!["geometry", "float", "int", "bool", "vector", "rotation", "object", "material", "string"]
}

/// The types a group's outputs and a zone's items take.
fn item_types() -> Vec<&'static str> {
    vec!["geometry", "float", "int", "bool", "vector", "rotation"]
}

static ENTRIES: &[Entry] = &[
    ("node.newGroup", "New Node Group", true, true, || {
        s::of(vec![
            ("name", s::string("The group's name (Geometry Nodes, Shader, or Volume when left out)")),
            ("tree", s::choice("geometry (the default): a Geometry Nodes group; shader: a material's surface and volume", &["geometry", "shader"])),
            ("object", s::object("geometry: add a Geometry Nodes modifier using the group to this mesh object")),
            ("modifier", s::integer("With object: the Geometry Nodes modifier (its index) to use the group instead of adding one")),
            (
                "material",
                json!({ "oneOf": [{ "type": "integer" }, { "type": "string" }], "description": "shader: the material (id or name) the group gives its surface (a Principled BSDF into the Material Output's Surface)" }),
            ),
            ("world", s::boolean("shader: the group is the world's volume (a Principled Volume into the Material Output's Volume)")),
        ])
    }),
    ("node.removeGroup", "Remove Node Group", false, true, || s::of(vec![group_ref()])),
    ("node.renameGroup", "Rename Node Group", false, true, || s::of(vec![group_ref(), ("name", s::required(s::string("The new name")))])),
    ("node.addNode", "Add Node", false, true, || {
        s::of(vec![
            group_ref(),
            ("kind", s::required(s::choice("The node's kind (node.describe lists their sockets)", &kinds()))),
            ("location", json!({ "type": "array", "items": { "type": "number" }, "description": "[x, y] in the editor's units" })),
            (
                "values",
                json!({ "type": "object", "additionalProperties": true, "description": "Socket and setting values by id, as node.setValue takes them" }),
            ),
        ])
    }),
    ("node.removeNodes", "Remove Nodes", false, true, || s::of(vec![group_ref(), ("nodes", s::required(s::indices("The nodes' ids")))])),
    ("node.moveNodes", "Move Nodes", false, true, || {
        s::of(vec![
            group_ref(),
            ("nodes", s::required(s::indices("The nodes' ids"))),
            ("offset", json!({ "type": "array", "items": { "type": "number" }, "description": "[dx, dy] to move each by" })),
            ("location", json!({ "type": "array", "items": { "type": "number" }, "description": "[x, y] to put the (one) node at" })),
        ])
    }),
    ("node.setValue", "Set Node Value", false, true, || {
        s::of(vec![
            group_ref(),
            ("node", s::required(node_ref("The node's id"))),
            ("id", s::required(s::string("An input socket's or a setting's id"))),
            (
                "value",
                json!({ "type": "json", "description": "The value: a number, [x, y, z], a boolean, a setting's choice, an object or material id, a text; null clears it" }),
            ),
        ])
    }),
    ("node.link", "Link Sockets", false, true, || {
        s::of(vec![
            group_ref(),
            ("from", s::required(node_ref("The node whose output to link"))),
            ("fromSocket", s::required(s::string("The output socket's id"))),
            ("to", s::required(node_ref("The node whose input to link"))),
            ("toSocket", s::required(s::string("The input socket's id; a link already there is replaced unless the input takes several"))),
        ])
    }),
    ("node.unlink", "Unlink Sockets", false, true, || {
        s::of(vec![
            group_ref(),
            ("to", s::required(node_ref("The node whose input to unlink"))),
            ("toSocket", s::required(s::string("The input socket's id"))),
            ("from", node_ref("Only the link from this node")),
            ("fromSocket", s::string("Only the link from this output socket")),
        ])
    }),
    ("node.addInput", "Add Group Input", false, true, || {
        s::of(vec![
            group_ref(),
            ("name", s::required(s::string("The input's name, unique in the group"))),
            ("type", s::required(s::choice("What the input carries", &types()))),
            ("default", json!({ "type": "json", "description": "The value a modifier uses when it sets none" })),
        ])
    }),
    ("node.removeInput", "Remove Group Input", false, true, || s::of(vec![group_ref(), ("name", s::required(s::string("The input's name")))])),
    ("node.addOutput", "Add Group Output", false, true, || {
        s::of(vec![
            group_ref(),
            ("name", s::required(s::string("The output's name, unique in the group (a Group node using the group gives it)"))),
            ("type", s::required(s::choice("What the output carries", &item_types()))),
        ])
    }),
    ("node.removeOutput", "Remove Group Output", false, true, || s::of(vec![group_ref(), ("name", s::required(s::string("The output's name")))])),
    ("node.addZoneItem", "Add Zone Item", false, true, || {
        s::of(vec![
            group_ref(),
            ("node", s::required(node_ref("Either node of the zone"))),
            ("name", s::required(s::string("The item's name, unique in the zone: a socket on both of its nodes"))),
            ("type", s::required(s::choice("What the item carries", &item_types()))),
        ])
    }),
    ("node.removeZoneItem", "Remove Zone Item", false, true, || {
        s::of(vec![group_ref(), ("node", s::required(node_ref("Either node of the zone"))), ("name", s::required(s::string("The item's name")))])
    }),
    ("node.describe", "Describe Node Kinds", false, false, || {
        s::of(vec![
            ("kind", s::string("One kind; every kind when left out")),
            ("tree", s::choice("Only the kinds a geometry, a compositing, or a shader group takes", &["geometry", "compositor", "shader"])),
            group_ref(),
        ])
    }),
];

/// Sets `key` of a JSON object.
fn put(value: &mut Value, key: &str, item: Value) {
    if let Some(map) = value.as_object_mut() {
        map.insert(key.to_owned(), item);
    }
}

fn kinds() -> Vec<&'static str> {
    node_tree::NODE_KINDS.iter().chain(COMPOSITOR_KINDS).chain(SHADER_KINDS).map(|kind| kind.id).collect()
}

/// The commands this file adds.
pub fn catalogue(doc: &SceneDocument) -> Vec<KitCommand> {
    let off = disabled(doc);
    ENTRIES
        .iter()
        .map(|(id, title, in_menu, mutates, schema)| KitCommand {
            id: (*id).into(),
            title: (*title).into(),
            menu: in_menu.then(|| MENU.iter().map(|item| (*item).to_string()).collect()),
            shortcut: None,
            params: None,
            schema: Some(schema()),
            mutates: *mutates,
            enabled: !off.iter().any(|other| other == id),
            ..Default::default()
        })
        .collect()
}

/// The ones that cannot run now: those that need a group when there is none.
pub fn disabled(doc: &SceneDocument) -> Vec<String> {
    if doc.scene.node_groups.is_empty() {
        ENTRIES.iter().map(|entry| entry.0).filter(|id| !matches!(*id, "node.newGroup" | "node.describe")).map(str::to_owned).collect()
    } else {
        Vec::new()
    }
}

/// Runs `command` if it is one of these; `None` for every other command.
pub fn execute(doc: &mut SceneDocument, command: &str, params: &Value) -> Option<Result<Value, String>> {
    let (_, title, _, mutates, _) = ENTRIES.iter().find(|entry| entry.0 == command)?;
    if !params.is_object() && !params.is_null() {
        return Some(Err(bad("the parameters must be an object")));
    }
    if !mutates {
        return Some(describe(&doc.scene, params));
    }
    let before = doc.step(title);
    let session = doc.session.clone();
    let result = run(doc, command, params).and_then(|value| {
        doc.scene.validate().map_err(|error| format!("failed: the change would break the scene ({error})"))?;
        Ok(value)
    });
    Some(match result {
        Ok(value) => {
            doc.tidy_session();
            doc.committed(before).map(|()| value)
        }
        Err(error) => {
            doc.restore(before);
            doc.session = session;
            Err(error)
        }
    })
}

/// The group `params` names.
/// A shader group named by `value` (an id or a name), or `None` for null.
pub fn shader_ref(scene: &Scene, value: &Value) -> Result<Option<Id>, String> {
    let group = match value {
        Value::Null => return Ok(None),
        Value::String(name) => scene.node_groups.iter().find(|group| &group.name == name).ok_or_else(|| format!("not-found: node group {name}"))?,
        value => {
            let id = value.as_u64().and_then(|id| Id::try_from(id).ok()).ok_or_else(|| bad("a shader group is an id or a name"))?;
            scene.node_group(id).ok_or_else(|| format!("not-found: node group {id}"))?
        }
    };
    if group.tree != Tree::Shader {
        return Err(bad(format!("{} is not a shader group", group.name)));
    }
    Ok(Some(group.id))
}

fn group_id(scene: &Scene, params: &Value) -> Result<Id, String> {
    match params.get("group") {
        None | Some(Value::Null) => match scene.node_groups.as_slice() {
            [only] => Ok(only.id),
            [] => Err("not-found: there is no node group".into()),
            _ => Err(bad("`group` is required when there are several node groups")),
        },
        Some(Value::String(name)) => {
            scene.node_groups.iter().find(|group| &group.name == name).map(|group| group.id).ok_or_else(|| format!("not-found: node group {name}"))
        }
        Some(value) => {
            let id = value.as_u64().and_then(|id| Id::try_from(id).ok()).ok_or_else(|| bad("`group` must be an id or a name"))?;
            scene.node_group(id).map(|group| group.id).ok_or_else(|| format!("not-found: node group {id}"))
        }
    }
}

fn group_mut<'a>(scene: &'a mut Scene, params: &Value) -> Result<&'a mut NodeGroup, String> {
    let id = group_id(scene, params)?;
    scene.node_group_mut(id).ok_or_else(|| format!("not-found: node group {id}"))
}

fn node_id(params: &Value, key: &str) -> Result<Option<Id>, String> {
    params::integer(params, key)
}

fn need_node(params: &Value, key: &str) -> Result<Id, String> {
    node_id(params, key)?.ok_or_else(|| bad(format!("`{key}` is required")))
}

/// References in values whose types are decided by the node or group interface. Marking the
/// whole JSON parameter as an object would also hold ordinary numbers, vectors and settings.
pub fn value_objects(scene: &Scene, command: &str, params: &Value) -> Vec<Id> {
    let id = |value: &Value| value.as_u64().and_then(|id| Id::try_from(id).ok());
    let is_object = |kind: SocketType| matches!(kind, SocketType::Object | SocketType::Material);
    if command == "node.addInput" {
        return params
            .get("default")
            .and_then(id)
            .filter(|_| matches!(params.get("type").and_then(Value::as_str), Some("object" | "material")))
            .into_iter()
            .collect();
    }
    if matches!(command, "modifier.add" | "modifier.set") {
        let existing =
            params.get("object").and_then(|value| params::resolve(scene, value).ok()).and_then(|object| scene.object(object).ok()).and_then(
                |object| {
                    params
                        .get("index")
                        .and_then(Value::as_u64)
                        .and_then(|index| usize::try_from(index).ok())
                        .and_then(|index| object.modifiers.get(index))
                        .or_else(|| {
                            params.get("name").and_then(Value::as_str).and_then(|name| object.modifiers.iter().find(|modifier| modifier.name == name))
                        })
                },
            );
        let group = match (command, existing.map(|modifier| &modifier.kind)) {
            ("modifier.add", _) if params.get("type").and_then(Value::as_str) == Some("geometryNodes") => params.get("group").and_then(id),
            ("modifier.set", Some(ModifierKind::GeometryNodes { group, .. })) => params.get("group").and_then(id).or(*group),
            _ => None,
        }
        .and_then(|group| scene.node_group(group));
        return group
            .into_iter()
            .flat_map(|group| group.inputs.iter())
            .filter(|socket| is_object(socket.kind))
            .filter_map(|socket| params.get("inputs").and_then(|inputs| inputs.get(&socket.name)).and_then(id))
            .collect();
    }
    if !matches!(command, "node.setValue" | "node.addNode") {
        return Vec::new();
    }
    let Some(group) = group_id(scene, params).ok().and_then(|group| scene.node_group(group)) else { return Vec::new() };
    let node = if command == "node.setValue" {
        let Some(node) = params.get("node").and_then(id).and_then(|node| group.node(node)) else { return Vec::new() };
        node.clone()
    } else {
        let Some(kind) = params.get("kind").and_then(Value::as_str) else { return Vec::new() };
        Node {
            id: 0,
            kind: kind.to_owned(),
            location: [0.0, 0.0],
            values: params
                .get("values")
                .and_then(Value::as_object)
                .map(|values| values.iter().map(|(key, value)| (key.clone(), value.clone())).collect())
                .unwrap_or_default(),
        }
    };
    let values: Vec<(&str, &Value)> = if command == "node.setValue" {
        params.get("id").and_then(Value::as_str).zip(params.get("value")).into_iter().collect()
    } else {
        params.get("values").and_then(Value::as_object).into_iter().flatten().map(|(key, value)| (key.as_str(), value)).collect()
    };
    let sockets = group.inputs_of(&node, &scene.node_groups);
    values
        .into_iter()
        .filter(|(key, _)| (node.kind == "group" && *key == "group") || sockets.iter().any(|(name, kind, _)| name == key && is_object(*kind)))
        .filter_map(|(_, value)| id(value))
        .collect()
}

fn location(params: &Value, key: &str) -> Result<Option<[f32; 2]>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) if items.len() == 2 => {
            let at = |index: usize| {
                items.get(index).and_then(Value::as_f64).map(|value| value as f32).filter(|value| value.is_finite() && value.abs() < 1e6)
            };
            match (at(0), at(1)) {
                (Some(x), Some(y)) => Ok(Some([x, y])),
                _ => Err(bad(format!("`{key}` must be two finite numbers"))),
            }
        }
        Some(_) => Err(bad(format!("`{key}` must be [x, y]"))),
    }
}

/// Checks a node value: a setting takes one of its choices, a socket (its kind's, or one its
/// group's interface or its zone's items give it) a value of its type, and a Group node's
/// `group` a geometry group that does not use the group the node is in.
fn checked_value(group: &NodeGroup, groups: &[NodeGroup], node: &Node, kind: &NodeKind, id: &str, value: &Value) -> Result<Value, String> {
    if node.kind == "group" && id == "group" {
        let used = value.as_u64().and_then(|id| Id::try_from(id).ok()).ok_or_else(|| bad("`group` takes a node group's id"))?;
        let target = groups.iter().find(|other| other.id == used).ok_or_else(|| format!("not-found: node group {used}"))?;
        if target.tree != Tree::Geometry {
            return Err(bad(format!("{} is not a geometry node group", target.name)));
        }
        if used == group.id || node_tree::uses_group(groups, used, group.id) {
            return Err(bad(format!("{} would use itself", group.name)));
        }
        return Ok(json!(used));
    }
    if node_tree::has_dynamic_sockets(&node.kind) && matches!(id, "items" | "output") {
        return Err(bad("a zone's pair and items change through node.addNode, node.addZoneItem, and node.removeZoneItem"));
    }
    if let Some(setting) = kind.settings.iter().find(|setting| setting.id == id) {
        let text = match value {
            Value::Bool(flag) => flag.to_string(),
            Value::String(text) => text.clone(),
            _ => return Err(bad(format!("`{id}` takes one of {}", setting.choices.join(", ")))),
        };
        if !setting.choices.contains(&text.as_str()) {
            return Err(bad(format!("`{id}` takes one of {}", setting.choices.join(", "))));
        }
        return Ok(if setting.choices == ["false", "true"] { Value::Bool(text == "true") } else { Value::String(text) });
    }
    let kind_of = match kind.inputs.iter().find(|socket| socket.id == id) {
        Some(socket) => node_tree::socket_type(node, kind, socket),
        None => match group.inputs_of(node, groups).into_iter().find(|(socket, _, _)| socket == id) {
            Some((_, kind_of, _)) => kind_of,
            None => return Err(bad(format!("{} has no input or setting {id}", kind.title))),
        },
    };
    let number = |value: &Value| value.as_f64().filter(|number| number.is_finite() && number.abs() < 1e9);
    let ok = match kind_of {
        SocketType::Float | SocketType::Int => number(value).is_some(),
        SocketType::Bool => value.is_boolean(),
        SocketType::Vector | SocketType::Rotation => {
            value.as_array().is_some_and(|items| items.len() == 3 && items.iter().all(|item| number(item).is_some()))
        }
        SocketType::String => value.is_string(),
        SocketType::Object | SocketType::Material => value.as_u64().is_some(),
        SocketType::Color => value
            .as_array()
            .is_some_and(|items| (3..=4).contains(&items.len()) && items.iter().all(|item| number(item).is_some_and(|number| number >= 0.0))),
        SocketType::Curves => return checked_curves(value),
        SocketType::Ramp => return checked_ramp(value),
        SocketType::Geometry | SocketType::Any | SocketType::Shader => false,
    };
    if !ok {
        return Err(bad(format!("`{id}` of {} takes a {kind_of:?} value", kind.title).to_lowercase()));
    }
    if kind_of == SocketType::Color
        && let Some([r, g, b]) = value.as_array().map(Vec::as_slice)
    {
        return Ok(json!([r, g, b, 1.0]));
    }
    Ok(value.clone())
}

/// Drops the group's links that no longer join sockets its nodes have (after a node's sockets
/// changed: a setting, a Group node's group, a zone's items, a group's interface).
fn drop_unfitting(scene: &mut Scene) {
    let groups = scene.node_groups.clone();
    for group in &mut scene.node_groups {
        group.links = kition_scene_kdm::nodes::fitting_links(group, &groups);
    }
}

/// A zone's output node, from either of its nodes.
fn zone_output(group: &NodeGroup, id: Id) -> Result<Id, String> {
    let node = group.node(id).ok_or_else(|| format!("not-found: node {id}"))?;
    if node_tree::is_zone_output(&node.kind) {
        return Ok(node.id);
    }
    group.zone_output_of(node).map(|output| output.id).ok_or_else(|| bad(format!("node {id} is not a zone's node")))
}

fn scene_has_image<'a>(hashes: &'a [String], hash: &str) -> Option<&'a String> {
    hashes.iter().find(|item| item.as_str() == hash)
}

/// Checks RGB Curves' curves: `c`, `r`, `g`, and `b` (any of them), each 2 to 64 `[x, y]`
/// points with x from 0 to 1, which it sorts by x.
fn checked_curves(value: &Value) -> Result<Value, String> {
    let map = value.as_object().ok_or_else(|| bad("`curves` takes { \"c\": [[x, y], ...], \"r\": ..., \"g\": ..., \"b\": ... }"))?;
    let mut out = serde_json::Map::new();
    for (key, points) in map {
        if !matches!(key.as_str(), "c" | "r" | "g" | "b") {
            return Err(bad(format!("`curves` has no curve {key} (c, r, g, b)")));
        }
        let mut list: Vec<[f64; 2]> = Vec::new();
        for point in points.as_array().ok_or_else(|| bad(format!("curve {key} must be a list of [x, y]")))? {
            let Some([x, y]) = point.as_array().map(Vec::as_slice) else {
                return Err(bad(format!("curve {key}'s points are [x, y]")));
            };
            let (Some(x), Some(y)) = (x.as_f64(), y.as_f64()) else {
                return Err(bad(format!("curve {key}'s points are numbers")));
            };
            if !((0.0..=1.0).contains(&x) && y.is_finite() && y.abs() < 1e3) {
                return Err(bad(format!("curve {key}'s points have x from 0 to 1")));
            }
            list.push([x, y]);
        }
        if !(2..=64).contains(&list.len()) {
            return Err(bad(format!("curve {key} takes 2 to 64 points")));
        }
        list.sort_by(|a, b| a[0].total_cmp(&b[0]));
        out.insert(key.clone(), json!(list));
    }
    Ok(Value::Object(out))
}

/// Checks a Color Ramp's stops: `{ "elements": [[position, [r, g, b, a]], ...] }`, 1 to 32 stops
/// with positions from 0 to 1, which it sorts by position.
fn checked_ramp(value: &Value) -> Result<Value, String> {
    let usage = "`ramp` takes { \"elements\": [[position, [r, g, b, a]], ...] }";
    let elements = value.get("elements").and_then(Value::as_array).ok_or_else(|| bad(usage))?;
    if !(1..=32).contains(&elements.len()) {
        return Err(bad("a color ramp takes 1 to 32 stops"));
    }
    let mut stops: Vec<(f64, [f64; 4])> = Vec::new();
    for element in elements {
        let Some([position, color]) = element.as_array().map(Vec::as_slice) else { return Err(bad(usage)) };
        let position = position.as_f64().filter(|position| (0.0..=1.0).contains(position)).ok_or_else(|| bad("a stop's position is from 0 to 1"))?;
        let channels: Vec<f64> = color.as_array().into_iter().flatten().filter_map(Value::as_f64).filter(|channel| channel.is_finite()).collect();
        let color = match channels.as_slice() {
            [r, g, b] => [*r, *g, *b, 1.0],
            [r, g, b, a] => [*r, *g, *b, *a],
            _ => return Err(bad("a stop's colour is [r, g, b] or [r, g, b, a]")),
        };
        stops.push((position, color));
    }
    stops.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(json!({ "elements": stops.iter().map(|(position, color)| json!([position, color])).collect::<Vec<_>>() }))
}

fn run(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    let scene = &mut doc.scene;
    match command {
        "node.newGroup" if params::string(params, "tree")? == Some("shader") => {
            let world = params::boolean(params, "world")?.unwrap_or(false);
            let material = match params.get("material").filter(|value| !value.is_null()) {
                Some(value) => Some(crate::commands::material_ref_value(scene, value)?),
                None => None,
            };
            if world && material.is_some() {
                return Err(bad("a shader group is for a material or for the world, not both"));
            }
            let name = params::string(params, "name")?.unwrap_or(if world { "Volume" } else { "Shader" });
            let name = kition_scene_model::scene::unique_name(name, |candidate| scene.node_groups.iter().any(|group| group.name == candidate));
            let (id, first, output) = (scene.take_id(), scene.take_id(), scene.take_id());
            let mut group = NodeGroup::shader(id, name.clone(), first, output);
            if world {
                // A Principled Volume into the Volume, nothing in the Surface.
                if let Some(node) = group.nodes.iter_mut().find(|node| node.id == first) {
                    node.kind = "principledVolume".into();
                }
                group.links = vec![Link { from: first, from_socket: "Volume".into(), to: output, to_socket: "Volume".into() }];
                scene.world.volume = Some(id);
            }
            scene.node_groups.push(group);
            if let Some(material) = material {
                scene.material_mut(material).map_err(|error| error.to_string())?.shader = Some(id);
            }
            Ok(json!({ "group": id, "name": name, "output": output, "node": first, "material": material, "world": world }))
        }
        "node.newGroup" => {
            if !matches!(params::string(params, "tree")?, None | Some("geometry")) {
                return Err(bad("`tree` is geometry or shader"));
            }
            let name = params::string(params, "name")?.unwrap_or("Geometry Nodes");
            let name = kition_scene_model::scene::unique_name(name, |candidate| scene.node_groups.iter().any(|group| group.name == candidate));
            let target = match params.get("object").filter(|value| !value.is_null()) {
                Some(value) => Some(params::resolve(scene, value)?),
                None => None,
            };
            let (id, input, output) = (scene.take_id(), scene.take_id(), scene.take_id());
            scene.node_groups.push(NodeGroup::new(id, name.clone(), input, output));
            let mut out = json!({ "group": id, "name": name, "input": input, "output": output });
            if let Some(target) = target {
                let existing = params::integer(params, "modifier")?;
                let object = scene.object_mut(target).map_err(|error| error.to_string())?;
                if object.mesh().is_none() {
                    return Err(bad(format!("{} is not a mesh", object.name)));
                }
                if let Some(index) = existing {
                    let modifier = object.modifiers.get_mut(index as usize).ok_or("not-found: the modifier")?;
                    let ModifierKind::GeometryNodes { group, .. } = &mut modifier.kind else {
                        return Err(bad("that modifier is not a Geometry Nodes modifier"));
                    };
                    *group = Some(id);
                    put(&mut out, "object", json!(target));
                    put(&mut out, "modifier", json!(index));
                    return Ok(out);
                }
                let mut modifier = Modifier::new(ModifierKind::GeometryNodes { group: Some(id), inputs: BTreeMap::new() });
                modifier.name =
                    kition_scene_model::scene::unique_name(&modifier.name, |name| object.modifiers.iter().any(|other| other.name == name));
                object.modifiers.push(modifier);
                put(&mut out, "object", json!(target));
                put(&mut out, "modifier", json!(object.modifiers.len() - 1));
            }
            Ok(out)
        }
        "node.removeGroup" => {
            let id = group_id(scene, params)?;
            scene.node_groups.retain(|group| group.id != id);
            // Group nodes using it let go of it, and their links to its sockets go.
            for group in &mut scene.node_groups {
                for node in &mut group.nodes {
                    if node_tree::used_group(node) == Some(id) {
                        node.values.remove("group");
                    }
                }
            }
            drop_unfitting(scene);
            if scene.render.compositor == Some(id) {
                scene.render.compositor = None;
            }
            for material in &mut scene.materials {
                if material.shader == Some(id) {
                    material.shader = None;
                }
            }
            if scene.world.volume == Some(id) {
                scene.world.volume = None;
            }
            for object in &mut scene.objects {
                for modifier in &mut object.modifiers {
                    if let ModifierKind::GeometryNodes { group, .. } = &mut modifier.kind
                        && *group == Some(id)
                    {
                        *group = None;
                    }
                }
            }
            Ok(json!({ "group": id }))
        }
        "node.renameGroup" => {
            let id = group_id(scene, params)?;
            let name = params::string(params, "name")?.filter(|name| !name.trim().is_empty()).ok_or_else(|| bad("`name` must not be empty"))?;
            if scene.node_groups.iter().any(|group| group.id != id && group.name == name) {
                return Err(bad(format!("a node group is already called {name}")));
            }
            let group = scene.node_group_mut(id).ok_or("not-found: the node group")?;
            group.name = name.to_owned();
            Ok(json!({ "group": id, "name": name }))
        }
        "node.addNode" => {
            let kind_id = params::string(params, "kind")?.ok_or_else(|| bad("`kind` is required"))?;
            let kind = node_tree::node_kind(kind_id).ok_or_else(|| bad(format!("unknown node kind {kind_id}")))?;
            if node_tree::is_zone_output(kind_id) {
                return Err(bad("add a zone by its input kind (repeatInput, simulationInput): its output comes with it"));
            }
            let at = location(params, "location")?.unwrap_or([0.0, 0.0]);
            let id = scene.take_id();
            let output_kind = node_tree::zone_output_kind(kind_id);
            let output_id = output_kind.map(|_| scene.take_id());
            let groups = scene.node_groups.clone();
            let group = group_mut(scene, params)?;
            if !node_tree::in_tree(kind_id, group.tree) {
                return Err(bad(format!("a {} group takes no {} nodes", group.tree.id(), kind.title)));
            }
            if group.nodes.len() + 2 > node_tree::MAX_NODES {
                return Err(bad(format!("a node group holds at most {} nodes", node_tree::MAX_NODES)));
            }
            let mut node = Node { id, kind: kind_id.to_owned(), location: at, values: BTreeMap::new() };
            let mut extra = Vec::new();
            if let (Some(output_kind), Some(output_id)) = (output_kind, output_id) {
                // The zone's output beside it, holding the zone's one geometry item, linked.
                node.values.insert("output".into(), json!(output_id));
                extra.push(Node { id: output_id, kind: output_kind.to_owned(), location: [at[0] + 300.0, at[1]], values: BTreeMap::new() });
                group.links.push(Link { from: id, from_socket: "Geometry".into(), to: output_id, to_socket: "Geometry".into() });
            }
            if let Some(values) = params.get("values").and_then(Value::as_object) {
                // Settings and the used group first: they decide the sockets the other values set.
                let first = |key: &str| key == "group" || kind.settings.iter().any(|setting| setting.id == key);
                let (settings, sockets): (Vec<_>, Vec<_>) = values.iter().partition(|(key, _)| first(key));
                let probe = {
                    let mut probe = group.clone();
                    probe.nodes.extend(extra.iter().cloned());
                    probe
                };
                for (key, value) in settings.into_iter().chain(sockets) {
                    let value = checked_value(&probe, &groups, &node, kind, key, value)?;
                    node.values.insert(key.clone(), value);
                }
            }
            group.nodes.push(node);
            group.nodes.extend(extra);
            let mut out = json!({ "group": group.id, "node": id });
            if let Some(output_id) = output_id {
                put(&mut out, "output", json!(output_id));
            }
            Ok(out)
        }
        "node.removeNodes" => {
            let mut ids = params::indices(params, "nodes")?.ok_or_else(|| bad("`nodes` is required"))?;
            let group = group_mut(scene, params)?;
            for id in &ids {
                if group.node(*id).is_none() {
                    return Err(format!("not-found: node {id}"));
                }
            }
            // A zone's two nodes go together.
            let partners: Vec<Id> = ids
                .iter()
                .filter_map(|id| {
                    let node = group.node(*id)?;
                    if node_tree::is_zone_output(&node.kind) {
                        group.zone_input_of(node).map(|input| input.id)
                    } else {
                        group.zone_output_of(node).map(|output| output.id)
                    }
                })
                .collect();
            for partner in partners {
                if !ids.contains(&partner) {
                    ids.push(partner);
                }
            }
            group.nodes.retain(|node| !ids.contains(&node.id));
            group.links.retain(|link| !ids.contains(&link.from) && !ids.contains(&link.to));
            Ok(json!({ "group": group.id, "removed": ids }))
        }
        "node.moveNodes" => {
            let ids = params::indices(params, "nodes")?.ok_or_else(|| bad("`nodes` is required"))?;
            let offset = location(params, "offset")?;
            let place = location(params, "location")?;
            let group = group_mut(scene, params)?;
            for id in &ids {
                let node = group.node_mut(*id).ok_or_else(|| format!("not-found: node {id}"))?;
                match (offset, place) {
                    (Some([dx, dy]), _) => node.location = [node.location[0] + dx, node.location[1] + dy],
                    (None, Some(at)) => node.location = at,
                    (None, None) => return Err(bad("give `offset` or `location`")),
                }
            }
            Ok(json!({ "group": group.id, "moved": ids.len() }))
        }
        "node.setValue" => {
            let doc_images: Vec<String> = scene.images.iter().map(|image| image.hash.clone()).collect();
            let id = need_node(params, "node")?;
            let key = params::string(params, "id")?.ok_or_else(|| bad("`id` is required"))?.to_owned();
            let value = params.get("value").cloned().unwrap_or(Value::Null);
            let groups = scene.node_groups.clone();
            let group = group_mut(scene, params)?;
            let node = group.node(id).ok_or_else(|| format!("not-found: node {id}"))?.clone();
            let kind = node_tree::node_kind(&node.kind).ok_or("failed: the node's kind is unknown")?;
            if value.is_null() && node_tree::has_dynamic_sockets(&node.kind) && matches!(key.as_str(), "items" | "output") {
                return Err(bad("a zone's pair and items change through node.addNode, node.addZoneItem, and node.removeZoneItem"));
            }
            let checked = if value.is_null() { Value::Null } else { checked_value(group, &groups, &node, kind, &key, &value)? };
            if matches!(node.kind.as_str(), "image" | "imageTexture")
                && key == "Image"
                && checked.as_str().is_some_and(|hash| !hash.is_empty() && scene_has_image(&doc_images, hash).is_none())
            {
                return Err(format!("not-found: image {checked} (compositor.setImage and shader.setImage add one)"));
            }
            let node = group.node_mut(id).ok_or_else(|| format!("not-found: node {id}"))?;
            if checked.is_null() {
                node.values.remove(&key);
            } else {
                node.values.insert(key.clone(), checked);
            }
            // A setting can hide sockets: links into or out of hidden ones go.
            let snapshot = node.clone();
            if kind.settings.iter().any(|setting| setting.id == key) {
                group.links.retain(|link| {
                    let hidden = |id: Id, socket: &str, inputs: bool| {
                        id == snapshot.id && {
                            let sockets = if inputs { kind.inputs } else { kind.outputs };
                            sockets.iter().find(|item| item.id == socket).is_some_and(|item| !node_tree::socket_shown(&snapshot, kind, item))
                        }
                    };
                    !hidden(link.to, &link.to_socket, true) && !hidden(link.from, &link.from_socket, false)
                });
            }
            let group_id = group.id;
            // A Group node's sockets follow its group: links to sockets it no longer has go.
            if snapshot.kind == "group" && key == "group" {
                drop_unfitting(scene);
            }
            Ok(json!({ "group": group_id, "node": id, "id": key }))
        }
        "node.link" => {
            let (from, to) = (need_node(params, "from")?, need_node(params, "to")?);
            let from_socket = params::string(params, "fromSocket")?.ok_or_else(|| bad("`fromSocket` is required"))?.to_owned();
            let to_socket = params::string(params, "toSocket")?.ok_or_else(|| bad("`toSocket` is required"))?.to_owned();
            let groups = scene.node_groups.clone();
            let group = group_mut(scene, params)?;
            let (Some(source), Some(target)) = (group.node(from), group.node(to)) else {
                return Err("not-found: a node of the link".into());
            };
            let Some((_, out_type)) = group.outputs_of(source, &groups).into_iter().find(|(id, _)| *id == from_socket) else {
                return Err(bad(format!("node {from} has no output {from_socket}")));
            };
            let Some((_, in_type, multi)) = group.inputs_of(target, &groups).into_iter().find(|(id, _, _)| *id == to_socket) else {
                return Err(bad(format!("node {to} has no input {to_socket}")));
            };
            let geometry = |kind: SocketType| kind == SocketType::Geometry;
            let reference = |kind: SocketType| matches!(kind, SocketType::Object | SocketType::Material | SocketType::String | SocketType::Curves);
            let surface = |kind: SocketType| kind == SocketType::Shader;
            if matches!(in_type, SocketType::Curves | SocketType::Ramp)
                || geometry(out_type) != geometry(in_type)
                || surface(out_type) != surface(in_type)
                || (reference(out_type) || reference(in_type)) && out_type != in_type
            {
                return Err(bad(format!("a {out_type:?} output cannot link into a {in_type:?} input").to_lowercase()));
            }
            if from == to || kition_scene_kdm::nodes::reaches(&group.links, to, from) {
                return Err(bad("the link would make a cycle"));
            }
            let link = Link { from, from_socket, to, to_socket };
            if !multi {
                group.links.retain(|other| !(other.to == link.to && other.to_socket == link.to_socket));
            }
            if !group.links.contains(&link) {
                group.links.push(link);
            }
            Ok(json!({ "group": group.id, "links": group.links.len() }))
        }
        "node.unlink" => {
            let to = need_node(params, "to")?;
            let to_socket = params::string(params, "toSocket")?.ok_or_else(|| bad("`toSocket` is required"))?.to_owned();
            let from = node_id(params, "from")?;
            let from_socket = params::string(params, "fromSocket")?.map(str::to_owned);
            let group = group_mut(scene, params)?;
            let before = group.links.len();
            group.links.retain(|link| {
                !(link.to == to
                    && link.to_socket == to_socket
                    && from.is_none_or(|from| link.from == from)
                    && from_socket.as_ref().is_none_or(|socket| &link.from_socket == socket))
            });
            Ok(json!({ "group": group.id, "removed": before - group.links.len() }))
        }
        "node.addInput" | "node.addOutput" => {
            let outputs = command == "node.addOutput";
            let name = params::string(params, "name")?.filter(|name| !name.trim().is_empty()).ok_or_else(|| bad("`name` is required"))?.to_owned();
            let allowed = if outputs { item_types() } else { types() };
            let text = params::string(params, "type")?.ok_or_else(|| bad("`type` is required"))?;
            let kind: SocketType = serde_json::from_value(json!(text))
                .ok()
                .filter(|_| allowed.contains(&text))
                .ok_or_else(|| bad(format!("`type` takes one of {}", allowed.join(", "))))?;
            let default = if outputs { Value::Null } else { params.get("default").cloned().unwrap_or(Value::Null) };
            let group = group_mut(scene, params)?;
            let what = if outputs { "outputs" } else { "inputs" };
            if group.tree != Tree::Geometry {
                return Err(bad(format!("a {} group takes no {what}", group.tree.id())));
            }
            let sockets = if outputs { &mut group.outputs } else { &mut group.inputs };
            if name == "Geometry" || sockets.iter().any(|socket| socket.name == name) {
                return Err(bad(format!("the group already has an {} called {name}", &what[..what.len() - 1])));
            }
            if sockets.len() >= node_tree::MAX_ITEMS {
                return Err(bad(format!("a node group takes at most {} {what}", node_tree::MAX_ITEMS)));
            }
            sockets.push(GroupSocket { name: name.clone(), kind, default });
            Ok(json!({ "group": group.id, "name": name }))
        }
        "node.removeInput" | "node.removeOutput" => {
            let outputs = command == "node.removeOutput";
            let name = params::string(params, "name")?.ok_or_else(|| bad("`name` is required"))?.to_owned();
            let group = group_mut(scene, params)?;
            let sockets = if outputs { &mut group.outputs } else { &mut group.inputs };
            let before = sockets.len();
            sockets.retain(|socket| socket.name != name);
            if sockets.len() == before {
                return Err(format!("not-found: {} {name}", if outputs { "output" } else { "input" }));
            }
            let id = group.id;
            // Links to the socket inside the group, and to Group nodes' sockets elsewhere, go.
            drop_unfitting(scene);
            Ok(json!({ "group": id, "name": name }))
        }
        "node.addZoneItem" => {
            let node = need_node(params, "node")?;
            let name = params::string(params, "name")?.filter(|name| !name.trim().is_empty()).ok_or_else(|| bad("`name` is required"))?.to_owned();
            let text = params::string(params, "type")?.ok_or_else(|| bad("`type` is required"))?;
            let kind: SocketType = serde_json::from_value(json!(text))
                .ok()
                .filter(|_| item_types().contains(&text))
                .ok_or_else(|| bad(format!("`type` takes one of {}", item_types().join(", "))))?;
            let group = group_mut(scene, params)?;
            let output = zone_output(group, node)?;
            let target = group.node_mut(output).ok_or("not-found: the zone's output")?;
            let mut items = node_tree::items_of(target);
            if node_tree::RESERVED_ITEM_NAMES.contains(&name.as_str()) || items.iter().any(|item| item.name == name) {
                return Err(bad(format!("the zone already has a socket called {name}")));
            }
            if items.len() >= node_tree::MAX_ITEMS {
                return Err(bad(format!("a zone carries at most {} items", node_tree::MAX_ITEMS)));
            }
            items.push(node_tree::ZoneItem { name: name.clone(), kind });
            target.values.insert("items".into(), json!(items));
            Ok(json!({ "group": group.id, "node": output, "name": name }))
        }
        "node.removeZoneItem" => {
            let node = need_node(params, "node")?;
            let name = params::string(params, "name")?.ok_or_else(|| bad("`name` is required"))?.to_owned();
            let group = group_mut(scene, params)?;
            let output = zone_output(group, node)?;
            let target = group.node_mut(output).ok_or("not-found: the zone's output")?;
            let mut items = node_tree::items_of(target);
            let before = items.len();
            items.retain(|item| item.name != name);
            if items.len() == before {
                return Err(format!("not-found: zone item {name}"));
            }
            target.values.insert("items".into(), json!(items));
            let id = group.id;
            drop_unfitting(scene);
            Ok(json!({ "group": id, "node": output, "name": name }))
        }
        other => Err(format!("not-found: {other}")),
    }
}

fn socket_json(node: Option<&Node>, kind: &NodeKind, socket: &node_tree::Socket) -> Value {
    let mut value = json!({ "id": socket.id, "name": socket.name, "type": socket.kind, "default": socket.default.json() });
    if let Some(implicit) = socket.implicit {
        put(&mut value, "implicit", json!(implicit));
    }
    if socket.multi {
        put(&mut value, "multi", json!(true));
    }
    if let Some((setting, values)) = socket.when {
        put(&mut value, "when", json!({ setting: values }));
    }
    if let Some(node) = node {
        put(&mut value, "type", json!(node_tree::socket_type(node, kind, socket)));
    }
    value
}

/// The trees whose groups take nodes of `kind`.
fn trees_of(kind: &NodeKind) -> Vec<&'static str> {
    [Tree::Geometry, Tree::Compositor, Tree::Shader].into_iter().filter(|tree| node_tree::in_tree(kind.id, *tree)).map(Tree::id).collect()
}

/// A node kind as the agent and the editor read it.
pub fn kind_json(kind: &NodeKind) -> Value {
    json!({
        "id": kind.id,
        "title": kind.title,
        "category": kind.category,
        "inputs": kind.inputs.iter().map(|socket| socket_json(None, kind, socket)).collect::<Vec<_>>(),
        "outputs": kind.outputs.iter().map(|socket| socket_json(None, kind, socket)).collect::<Vec<_>>(),
        "settings": kind.settings.iter().map(|setting| json!({ "id": setting.id, "choices": setting.choices, "default": setting.default })).collect::<Vec<_>>(),
        "trees": trees_of(kind),
    })
}

/// A group as inspect shows it: its nodes with the sockets their settings, their group, or their
/// zone show, and its links (`groups` are the scene's).
pub fn group_json(group: &NodeGroup, groups: &[NodeGroup]) -> Value {
    let nodes: Vec<Value> = group
        .nodes
        .iter()
        .map(|node| {
            let mut value = json!({ "id": node.id, "kind": node.kind, "location": node.location, "values": node.values });
            if let Some(kind) = node_tree::node_kind(&node.kind) {
                put(&mut value, "title", json!(kind.title));
                let shown = |sockets: &[node_tree::Socket]| -> Vec<Value> {
                    sockets.iter().filter(|socket| node_tree::socket_shown(node, kind, socket)).map(|socket| socket_json(Some(node), kind, socket)).collect()
                };
                let (mut inputs, mut outputs) = (shown(kind.inputs), shown(kind.outputs));
                if node_tree::has_dynamic_sockets(&node.kind) {
                    // Sockets that follow the group's interface, the used group's, or the zone's items.
                    let defaults: BTreeMap<String, Value> = match node_tree::used_group(node).and_then(|id| groups.iter().find(|other| other.id == id)) {
                        Some(used) => used.inputs.iter().map(|input| (input.name.clone(), input.default.clone())).collect(),
                        None => BTreeMap::new(),
                    };
                    let known_in: Vec<String> = kind.inputs.iter().map(|socket| socket.id.to_owned()).collect();
                    let known_out: Vec<String> = kind.outputs.iter().map(|socket| socket.id.to_owned()).collect();
                    let entry = |id: &str, kind: SocketType| {
                        json!({ "id": id, "name": id, "type": kind, "default": defaults.get(id).cloned().unwrap_or(Value::Null), "dynamic": true })
                    };
                    inputs.extend(group.inputs_of(node, groups).into_iter().filter(|(id, _, _)| !known_in.contains(id)).map(|(id, kind, _)| entry(&id, kind)));
                    outputs = if node.kind == "groupInput" { Vec::new() } else { outputs };
                    outputs.extend(group.outputs_of(node, groups).into_iter().filter(|(id, _)| !known_out.contains(id) || node.kind == "groupInput").map(|(id, kind)| entry(&id, kind)));
                }
                put(&mut value, "inputs", json!(inputs));
                put(&mut value, "outputs", json!(outputs));
                let settings: Vec<Value> = kind
                    .settings
                    .iter()
                    .map(|setting| json!({ "id": setting.id, "choices": setting.choices, "value": node_tree::setting(node, kind, setting.id) }))
                    .collect();
                put(&mut value, "settings", json!(settings));
                put(&mut value, "category", json!(kind.category));
                if let Some(used) = node_tree::used_group(node).and_then(|id| groups.iter().find(|other| other.id == id)) {
                    put(&mut value, "title", json!(used.name));
                }
                if node_tree::zone_output_kind(&node.kind).is_some() || node_tree::is_zone_output(&node.kind) {
                    let pair = if node_tree::is_zone_output(&node.kind) { group.zone_input_of(node) } else { group.zone_output_of(node) };
                    put(&mut value, "zone", json!({ "pair": pair.map(|other| other.id), "items": group.zone_items(node) }));
                }
            }
            value
        })
        .collect();
    json!({ "id": group.id, "name": group.name, "tree": group.tree.id(), "inputs": group.inputs, "outputs": group.outputs, "nodes": nodes, "links": group.links })
}

fn describe(scene: &Scene, params: &Value) -> Result<Value, String> {
    if let Some(kind) = params::string(params, "kind")? {
        let kind = node_tree::node_kind(kind).ok_or_else(|| bad(format!("unknown node kind {kind}")))?;
        return Ok(kind_json(kind));
    }
    let tree = match params::string(params, "tree")? {
        Some(text) => Some(Tree::parse(text).ok_or_else(|| bad("`tree` is geometry, compositor, or shader"))?),
        None => None,
    };
    let kinds: Vec<Value> = match tree {
        Some(tree) => node_tree::kinds_of(tree).into_iter().map(kind_json).collect(),
        None => node_tree::NODE_KINDS.iter().chain(COMPOSITOR_KINDS).chain(SHADER_KINDS).map(kind_json).collect(),
    };
    let mut out = json!({ "kinds": kinds });
    if params.get("group").is_some_and(|value| !value.is_null()) {
        let id = group_id(scene, params)?;
        if let Some(group) = scene.node_group(id) {
            put(&mut out, "group", group_json(group, &scene.node_groups));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_schema_is_well_formed() {
        for (id, _, _, _, schema) in super::ENTRIES {
            kition_kit::schema::validate(&schema()).unwrap_or_else(|error| panic!("{id}: {error}"));
        }
    }
}
