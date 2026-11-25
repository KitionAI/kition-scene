//! Reading command parameters (hostile input: every number is checked) and describing them as
//! `ParameterSchemaSpec` for agents.

use glam::Vec3;
use kition_scene_model::{Id, Scene};
use serde_json::{Map, Value, json};

/// A parameter error, prefixed with its reason code.
pub fn bad(message: impl std::fmt::Display) -> String {
    format!("invalid-params: {message}")
}

/// A number parameter, if present.
pub fn number(params: &Value, key: &str) -> Result<Option<f32>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let number = value.as_f64().ok_or_else(|| bad(format!("`{key}` must be a number")))?;
            if !number.is_finite() || number.abs() > 1e9 {
                return Err(bad(format!("`{key}` must be a finite number")));
            }
            Ok(Some(number as f32))
        }
    }
}

/// A required number parameter.
pub fn need_number(params: &Value, key: &str) -> Result<f32, String> {
    number(params, key)?.ok_or_else(|| bad(format!("`{key}` is required")))
}

/// A whole-number parameter, if present.
pub fn integer(params: &Value, key: &str) -> Result<Option<u32>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            value.as_u64().and_then(|number| u32::try_from(number).ok()).map(Some).ok_or_else(|| bad(format!("`{key}` must be a whole number")))
        }
    }
}

/// A boolean parameter, if present.
pub fn boolean(params: &Value, key: &str) -> Result<Option<bool>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_bool().map(Some).ok_or_else(|| bad(format!("`{key}` must be true or false"))),
    }
}

/// A string parameter, if present.
pub fn string<'a>(params: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_str().map(Some).ok_or_else(|| bad(format!("`{key}` must be a string"))),
    }
}

/// A three-number parameter (`[x, y, z]` or `{x, y, z}`), if present.
pub fn vec3(params: &Value, key: &str) -> Result<Option<Vec3>, String> {
    let Some(value) = params.get(key).filter(|value| !value.is_null()) else { return Ok(None) };
    let parts: Vec<Option<f64>> = match value {
        Value::Array(items) if items.len() == 3 => items.iter().map(Value::as_f64).collect(),
        Value::Object(map) => ["x", "y", "z"].iter().map(|axis| map.get(*axis).and_then(Value::as_f64)).collect(),
        _ => return Err(bad(format!("`{key}` must be [x, y, z]"))),
    };
    match parts.as_slice() {
        [Some(x), Some(y), Some(z)] if [x, y, z].iter().all(|value| value.is_finite() && value.abs() <= 1e9) => {
            Ok(Some(Vec3::new(*x as f32, *y as f32, *z as f32)))
        }
        _ => Err(bad(format!("`{key}` must be three finite numbers"))),
    }
}

/// A colour parameter: `[r, g, b]` or `[r, g, b, a]` in 0..1 (linear), or `#rrggbb` (sRGB).
pub fn color(params: &Value, key: &str) -> Result<Option<[f32; 4]>, String> {
    let Some(value) = params.get(key).filter(|value| !value.is_null()) else { return Ok(None) };
    if let Some(text) = value.as_str() {
        let hex = text
            .strip_prefix('#')
            .filter(|hex| hex.len() == 6)
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .ok_or_else(|| bad(format!("`{key}` must be #rrggbb")))?;
        let [r, g, b] = kition_scene_raster::draw::hex(hex);
        return Ok(Some([r, g, b, 1.0]));
    }
    let items = value.as_array().ok_or_else(|| bad(format!("`{key}` must be [r, g, b] or #rrggbb")))?;
    let numbers: Vec<f32> =
        items.iter().filter_map(Value::as_f64).filter(|value| value.is_finite()).map(|value| value.clamp(0.0, 1.0) as f32).collect();
    match numbers.as_slice() {
        [r, g, b] if items.len() == 3 => Ok(Some([*r, *g, *b, 1.0])),
        [r, g, b, a] if items.len() == 4 => Ok(Some([*r, *g, *b, *a])),
        _ => Err(bad(format!("`{key}` must be [r, g, b] or [r, g, b, a] between 0 and 1"))),
    }
}

/// A list of whole numbers, if present.
pub fn indices(params: &Value, key: &str) -> Result<Option<Vec<u32>>, String> {
    let Some(value) = params.get(key).filter(|value| !value.is_null()) else { return Ok(None) };
    let items = value.as_array().ok_or_else(|| bad(format!("`{key}` must be a list of whole numbers")))?;
    items
        .iter()
        .map(|item| {
            item.as_u64().and_then(|number| u32::try_from(number).ok()).ok_or_else(|| bad(format!("`{key}` must be a list of whole numbers")))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// The parameters that name an object, a material, a node group or a node by its id (or name).
const ID_KEYS: [&str; 16] = [
    "object",
    "objects",
    "material",
    "parent",
    "target",
    "operand",
    "armature",
    "camera",
    "focusObject",
    "group",
    "node",
    "nodes",
    "from",
    "to",
    "colliders",
    "instanceObject",
];

/// The kinds of id a parameter of [`ID_KEYS`] names: a material's `material`, a node group's
/// `group`, a node's `node`/`nodes`, an animation channel's `target` (an object or a material),
/// a node link's or a camera move's `from`/`to` (a node or an object); every other one an object.
fn is_id_for(scene: &Scene, key: &str, id: Id) -> bool {
    let object = || scene.object(id).is_ok();
    let material = || scene.materials.iter().any(|material| material.id == id);
    let group = || scene.node_groups.iter().any(|group| group.id == id);
    let node = || scene.node_groups.iter().any(|group| group.nodes.iter().any(|node| node.id == id));
    match key {
        "material" => material(),
        "group" => group(),
        "node" | "nodes" => node(),
        "target" => object() || material(),
        "from" | "to" => object() || node(),
        _ => object(),
    }
}

/// Whether the command's schema says parameter `key` is only a string: a name (a bone's, a
/// vertex group's, an action's new name), never an id.
fn names_only(schema: Option<&Value>, key: &str) -> bool {
    schema.map(parameters).and_then(|properties| properties.get(key)).and_then(|node| node.get("type")).and_then(Value::as_str) == Some("string")
}

/// An id as receipts and `others` print it may come back as its digits (`"4111466496"`): such a
/// string, in a parameter that names objects, materials, node groups or nodes (or an item of its
/// list), is that id when the scene has an id of the kind the parameter names, so every command
/// takes an id as a number or as its digits (task card C8-scene). Any other string (a name) stays,
/// and so does every string of a parameter the command's `schema` gives as a string only (a
/// vertex group's or a bone's name): a material called "12" is found by its name though an object
/// has id 12 (C8-scene-engine).
pub fn ids_as_numbers(scene: &Scene, schema: Option<&Value>, params: &Value) -> Value {
    let mut params = params.clone();
    if let Some(map) = params.as_object_mut() {
        for key in ID_KEYS {
            if names_only(schema, key) {
                continue;
            }
            let as_number = |value: &mut Value| {
                if let Some(id) = value.as_str().and_then(digits_id).filter(|id| is_id_for(scene, key, *id)) {
                    *value = Value::from(id);
                }
            };
            match map.get_mut(key) {
                Some(Value::Array(items)) => items.iter_mut().for_each(as_number),
                Some(value) => as_number(value),
                None => {}
            }
        }
    }
    params
}

/// The id a string of digits spells, if it is one.
fn digits_id(text: &str) -> Option<Id> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse::<Id>().ok()
}

/// An object named by id (a number, or its digits as receipts print it) or by name (a string).
pub fn resolve(scene: &Scene, value: &Value) -> Result<Id, String> {
    if let Some(id) = value.as_str().and_then(digits_id).filter(|id| scene.object(*id).is_ok()) {
        return Ok(id);
    }
    match value {
        Value::Number(number) => {
            let id = number.as_u64().and_then(|id| u32::try_from(id).ok()).ok_or_else(|| bad("an object id is a whole number"))?;
            scene.object(id).map(|object| object.id).map_err(|_| format!("not-found: no object {id}"))
        }
        Value::String(name) => scene.object_named(name).map(|object| object.id).ok_or_else(|| format!("not-found: no object named {name}")),
        _ => Err(bad("an object is named by its id or its name")),
    }
}

/// Parameters whose schema says `id` but which name no object of the scene's id space: a face
/// set's id is a number inside one mesh (`sculpt.faceSetVisibility`).
const NOT_OBJECT_IDS: [&str; 1] = ["faceSet"];

/// These unions take either one object or a world point. Marking the whole parameter would
/// turn integer coordinates into ids; `place::under` holds only the scalar object form.
const POINT_OBJECTS: [(&str, &str); 6] = [
    ("camera.move", "target"),
    ("camera.move", "to"),
    ("light.studio", "target"),
    ("object.hop", "to"),
    ("object.gather", "center"),
    ("object.wave", "origin"),
];

pub fn point_objects(command: &str) -> impl Iterator<Item = &'static str> + '_ {
    POINT_OBJECTS.iter().filter_map(move |(id, path)| (*id == command).then_some(*path))
}

/// Whether this schema describes an integer id (possibly in an array or union).
fn names_ids(node: &Value) -> bool {
    fn takes_integer(node: &Value) -> bool {
        node.get("type").and_then(Value::as_str) == Some("integer")
            || node.get("items").is_some_and(takes_integer)
            || node.get("oneOf").and_then(Value::as_array).is_some_and(|branches| branches.iter().any(takes_integer))
    }
    takes_integer(node)
        && node
            .to_string()
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|word| word.eq_ignore_ascii_case("id") || word.eq_ignore_ascii_case("ids"))
}

/// A command's parameters: its `ParameterSchemaSpec`, or the `properties` of a JSON Schema
/// object. A parameter itself named `properties` (`animation.insertKey`'s) is no JSON Schema's: taking
/// it for one hid the command's `objects` from its object parameters (C8-scene-read).
pub fn parameters(schema: &Value) -> &Value {
    match schema.get("properties") {
        Some(properties) if schema.get("type").and_then(Value::as_str) == Some("object") => properties,
        _ => schema,
    }
}

/// The precise object members, including members of arrays (`boids.rules[].target`).
/// Containers are not marked: their counts, indexes and weights are not ids.
pub fn object_parameters(command: &str, schema: Option<&Value>, targets: Option<&str>) -> &'static [&'static str] {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock, PoisonError};
    static LISTS: OnceLock<Mutex<HashMap<Vec<String>, &'static [&'static str]>>> = OnceLock::new();
    fn walk(node: &Value, path: String, found: &mut Vec<String>) {
        if names_ids(node) || (matches!(path.as_str(), "group" | "shader" | "volume") && node.get("type").and_then(Value::as_str) == Some("json")) {
            found.push(path.clone());
        }
        if let Some(properties) = node.get("properties").and_then(Value::as_object) {
            for (name, child) in properties {
                walk(child, format!("{path}.{name}"), found);
            }
        }
        if let Some(items) = node.get("items").filter(|items| items.get("properties").is_some()) {
            walk(items, format!("{path}[]"), found);
        }
    }
    let Some(properties) = schema.map(parameters).and_then(Value::as_object) else { return &[] };
    let mut found = Vec::new();
    for (name, node) in properties {
        if Some(name.as_str()) != targets && !NOT_OBJECT_IDS.contains(&name.as_str()) && !point_objects(command).any(|path| path == name) {
            walk(node, name.clone(), &mut found);
        }
    }
    let mut lists = LISTS.get_or_init(Mutex::default).lock().unwrap_or_else(PoisonError::into_inner);
    lists.entry(found.clone()).or_insert_with(|| {
        let names: Vec<&'static str> = found.into_iter().map(|name| &*Box::leak(name.into_boxed_str())).collect();
        Box::leak(names.into_boxed_slice())
    })
}

/// Types resolved by the command implementation, for the target and every object member.
pub fn object_types(
    command: &str,
    objects: &'static [&'static str],
    targets: Option<&'static str>,
) -> &'static [(&'static str, &'static [&'static str])] {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock, PoisonError};
    type Types = &'static [(&'static str, &'static [&'static str])];
    type Interned = HashMap<Vec<(&'static str, &'static [&'static str])>, Types>;
    static LISTS: OnceLock<Mutex<Interned>> = OnceLock::new();
    let types: Vec<_> = targets
        .into_iter()
        .chain(objects.iter().copied())
        .chain(point_objects(command))
        .map(|path| {
            let types: &'static [&'static str] = match path.rsplit('.').next().unwrap_or(path) {
                "material" => &["Material"],
                "group" | "shader" | "volume" => &["NodeGroup"],
                "node" | "nodes" => &["Node"],
                "from" | "to" if command.starts_with("node.") => &["Node"],
                "target" if command.starts_with("graph.") => &["Object", "Material"],
                "image" => &["Image"],
                _ => &["Object"],
            };
            (path, types)
        })
        .collect();
    let mut lists = LISTS.get_or_init(Mutex::default).lock().unwrap_or_else(PoisonError::into_inner);
    lists.entry(types.clone()).or_insert_with(|| Box::leak(types.into_boxed_slice()))
}

/// Name-taking resolvers are scene objects, materials and node groups. Node references and
/// images take ids/hashes; graph channels take numeric object/material ids only.
pub fn name_parameters(
    command: &str,
    types: &'static [(&'static str, &'static [&'static str])],
    targets: Option<&str>,
) -> &'static [(&'static str, &'static str)] {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock, PoisonError};
    type Names = &'static [(&'static str, &'static str)];
    type Interned = HashMap<Vec<(&'static str, &'static str)>, Names>;
    static LISTS: OnceLock<Mutex<Interned>> = OnceLock::new();
    let names: Vec<_> = types
        .iter()
        .filter(|(path, types)| {
            Some(*path) != targets
                && !(command.starts_with("graph.") && *path == "target")
                && types.iter().any(|ty| matches!(*ty, "Object" | "Material" | "NodeGroup"))
        })
        .map(|(path, _)| (*path, "name"))
        .collect();
    let mut lists = LISTS.get_or_init(Mutex::default).lock().unwrap_or_else(PoisonError::into_inner);
    lists.entry(names.clone()).or_insert_with(|| Box::leak(names.into_boxed_slice()))
}

/// Values named by an object member path; arrays of ids and arrays along the path are flattened.
pub fn object_values<'a>(params: &'a Value, path: &str) -> Vec<&'a Value> {
    let mut values = vec![params];
    for segment in path.split('.') {
        let (name, items) = segment.strip_suffix("[]").map_or((segment, false), |name| (name, true));
        values = values
            .into_iter()
            .filter_map(|value| value.get(name))
            .flat_map(|value| if items { value.as_array().map(|items| items.iter().collect()).unwrap_or_default() } else { vec![value] })
            .collect();
    }
    values
        .into_iter()
        .flat_map(|value| match value {
            Value::Array(items) => items.iter().collect(),
            other => vec![other],
        })
        .collect()
}

/// The objects a command names with `key` (one or a list), if it names any.
pub fn objects(scene: &Scene, params: &Value, key: &str) -> Result<Option<Vec<Id>>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => items.iter().map(|item| resolve(scene, item)).collect::<Result<Vec<_>, _>>().map(Some),
        Some(value) => resolve(scene, value).map(|id| Some(vec![id])),
    }
}

/// Schema builders.
pub mod schema {
    use super::*;

    pub fn number(description: &str) -> Value {
        json!({ "type": "number", "description": description })
    }

    pub fn integer(description: &str) -> Value {
        json!({ "type": "integer", "description": description })
    }

    pub fn boolean(description: &str) -> Value {
        json!({ "type": "boolean", "description": description })
    }

    pub fn string(description: &str) -> Value {
        json!({ "type": "string", "description": description })
    }

    pub fn choice(description: &str, values: &[&str]) -> Value {
        json!({ "type": "string", "description": description, "enum": values })
    }

    pub fn vec3(description: &str) -> Value {
        json!({ "type": "array", "items": { "type": "number" }, "description": format!("{description} [x, y, z]") })
    }

    pub fn color(description: &str) -> Value {
        json!({ "oneOf": [
            { "type": "array", "items": { "type": "number" }, "description": "[r, g, b] or [r, g, b, a], linear, 0 to 1" },
            { "type": "string", "description": "#rrggbb (sRGB)" }
        ], "description": description })
    }

    pub fn indices(description: &str) -> Value {
        json!({ "type": "array", "items": { "type": "integer" }, "description": description })
    }

    pub fn object(description: &str) -> Value {
        json!({ "oneOf": [{ "type": "integer", "description": "the object's id" }, { "type": "string", "description": "the object's name" }], "description": description })
    }

    pub fn objects(description: &str) -> Value {
        json!({ "oneOf": [
            { "type": "integer", "description": "an object's id" },
            { "type": "string", "description": "an object's name" },
            { "type": "array", "items": { "oneOf": [{ "type": "integer" }, { "type": "string" }] }, "description": "ids or names" }
        ], "description": description })
    }

    pub fn required(mut node: Value) -> Value {
        if let Some(object) = node.as_object_mut() {
            object.insert("required".into(), Value::Bool(true));
        }
        node
    }

    /// A `ParameterSchemaSpec` from named properties.
    pub fn of(fields: Vec<(&str, Value)>) -> Value {
        let mut map = Map::new();
        for (key, node) in fields {
            map.insert(key.to_string(), node);
        }
        Value::Object(map)
    }
}
