//! Drivers, keying sets, and actions.
//!
//! A driver works a property out from other values ([`kition_scene_model::drivers`]): `driver.add`
//! puts one on a property of an object, a bone (`bone`), a shape key (`shapeKey`), or the
//! object's material (`baseColor`), and `driver.set` changes its kind, expression, and
//! variables. Angles in drivers are radians, as in the reference application's expressions.
//!
//! Keying sets name what Insert Keyframe keys: the built-in ones (location, rotation, scale, and
//! their mixes, every property already keyed, a whole character) or the scene's own, which list
//! properties of named objects and bones. The active one is what `animation.insertKey` and
//! `pose.insertKey` key when they are given no properties.
//!
//! Actions are named sets of keys no object owns: `action.new` stashes an object's keys in one,
//! `action.assign` gives an object (or an armature's bones) an action's keys, and the Action
//! constraint plays one.

use kition_kit::KitCommand;
use kition_scene_model::animation::{Action, ActionChannel, BuiltinKeyingSet, KeyingPath, KeyingSet, KeyingSetRef, Property};
use kition_scene_model::constraints::{ConstraintKind, Target, TransformChannel};
use kition_scene_model::drivers::{Driver, DriverKind, Source, Variable, VariableSpace};
use kition_scene_model::{Channel, Id, ObjectData, Scene};
use serde_json::{Value, json};

use crate::params::{self, bad, schema as s};
use crate::{Mode, SceneDocument};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Needs {
    Always,
    /// An object to work on: the active one, or one named.
    Object,
}

struct Spec {
    id: &'static str,
    title: &'static str,
    menu: &'static [&'static str],
    needs: Needs,
    schema: fn() -> Value,
}

const DRIVERS: &[&str] = &["Object", "Animation", "Drivers"];
const KEYING: &[&str] = &["Object", "Animation", "Keying Sets"];
const ACTIONS: &[&str] = &["Object", "Animation", "Actions"];

const PROPERTIES: &[&str] =
    &["location", "rotation", "scale", "hidden", "power", "color", "lens", "baseColor", "boneLocation", "boneRotation", "boneScale", "shapeKey"];
const CHANNELS: &[&str] =
    &["locationX", "locationY", "locationZ", "rotationX", "rotationY", "rotationZ", "scaleX", "scaleY", "scaleZ", "scaleAverage"];

fn driven_fields() -> Vec<(&'static str, Value)> {
    vec![
        (
            "object",
            s::object("The object (or the armature of `bone`, the mesh of `shapeKey`, the owner of the material); the active object when left out"),
        ),
        (
            "property",
            s::required(s::choice(
                "The driven property (bone ones name `bone`, shapeKey names `shapeKey`, baseColor is the object's material's)",
                PROPERTIES,
            )),
        ),
        ("index", s::integer("The component (x 0, y 1, z 2, alpha 3); every component when left out")),
        ("bone", s::string("The bone of a bone property")),
        ("shapeKey", s::string("The shape key of a shapeKey property")),
    ]
}

fn driver_fields() -> Vec<(&'static str, Value)> {
    vec![
        ("kind", s::choice("How the variables combine (scripted evaluates `expression`)", &["average", "sum", "min", "max", "scripted"])),
        (
            "expression",
            s::string(
                "scripted: Python-style arithmetic over the variables and `frame` (+ - * / // % **, comparisons, and, or, not, a if c else b, sin, cos, min, max, clamp, lerp, radians, ...); angles in radians",
            ),
        ),
        (
            "variables",
            json!({ "type": "array", "items": { "type": "object", "additionalProperties": true, "properties": {
                "object": s::object("The source object by id or name"),
                "a": s::object("The first source object by id or name"),
                "b": s::object("The second source object by id or name")
            } }, "description": "The variables, replacing any: { name, type: singleProperty, object, property, index, bone?, shapeKey? } | { name, type: transform, object, bone?, channel: locationX..scaleZ or scaleAverage, space: world, transform, or local } | { name, type: distance or rotationDifference, a, aBone?, b, bBone? }" }),
        ),
    ]
}

static SPECS: &[Spec] = &[
    Spec {
        id: "driver.add",
        title: "Add Driver",
        menu: DRIVERS,
        needs: Needs::Object,
        schema: || {
            let mut fields = driven_fields();
            fields.extend(driver_fields());
            s::of(fields)
        },
    },
    Spec {
        id: "driver.set",
        title: "Change Driver",
        menu: &[],
        needs: Needs::Object,
        schema: || {
            let mut fields = driven_fields();
            fields.extend(driver_fields());
            s::of(fields)
        },
    },
    Spec { id: "driver.remove", title: "Remove Driver", menu: DRIVERS, needs: Needs::Object, schema: || s::of(driven_fields()) },
    Spec {
        id: "keyingSet.new",
        title: "Add Keying Set",
        menu: KEYING,
        needs: Needs::Always,
        schema: || s::of(vec![("name", s::string("Its name (Keying Set)"))]),
    },
    Spec {
        id: "keyingSet.remove",
        title: "Remove Keying Set",
        menu: &[],
        needs: Needs::Always,
        schema: || s::of(vec![("name", s::string("The keying set; the active one when left out"))]),
    },
    Spec {
        id: "keyingSet.addPaths",
        title: "Add to Keying Set",
        menu: KEYING,
        needs: Needs::Always,
        schema: || {
            s::of(vec![
                ("name", s::string("The keying set; the active one of the scene's own when left out")),
                ("objects", s::objects("The objects; the selection when left out")),
                (
                    "properties",
                    json!({ "type": "array", "items": { "type": "string", "enum": PROPERTIES }, "description": "The properties (location, rotation, and scale when left out; bone ones with `bones`)" }),
                ),
                (
                    "bones",
                    json!({ "type": "array", "items": { "type": "string" }, "description": "Bones of the (one) armature named; its location, rotation, and scale" }),
                ),
            ])
        },
    },
    Spec {
        id: "keyingSet.removePath",
        title: "Remove from Keying Set",
        menu: &[],
        needs: Needs::Always,
        schema: || {
            s::of(vec![
                ("name", s::string("The keying set; the active one when left out")),
                ("index", s::required(s::integer("The path's position, from 0"))),
            ])
        },
    },
    Spec {
        id: "keyingSet.setActive",
        title: "Active Keying Set",
        menu: &[],
        needs: Needs::Always,
        schema: || {
            s::of(vec![(
                "name",
                s::string(
                    "location, rotation, scale, locRot, locScale, locRotScale, rotScale, available, wholeCharacter, or the name of one of the scene's own; null for none",
                ),
            )])
        },
    },
    Spec {
        id: "action.new",
        title: "New Action",
        menu: ACTIONS,
        needs: Needs::Always,
        schema: || {
            s::of(vec![
                ("name", s::string("Its name (Action)")),
                ("object", s::object("An object whose keys (an armature's bones' too) the action takes; an empty action when left out")),
            ])
        },
    },
    Spec {
        id: "action.assign",
        title: "Assign Action",
        menu: ACTIONS,
        needs: Needs::Object,
        schema: || {
            s::of(vec![
                ("objects", s::objects("The objects; the selection when left out")),
                (
                    "action",
                    s::required(s::string("The action; its keys replace each object's (bone keys go to an armature's bones of the same names)")),
                ),
            ])
        },
    },
    Spec {
        id: "action.remove",
        title: "Remove Action",
        menu: &[],
        needs: Needs::Always,
        schema: || s::of(vec![("name", s::required(s::string("The action")))]),
    },
    Spec {
        id: "action.rename",
        title: "Rename Action",
        menu: &[],
        needs: Needs::Always,
        schema: || s::of(vec![("name", s::required(s::string("The action"))), ("to", s::required(s::string("The new name")))]),
    },
];

fn spec_of(id: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|spec| spec.id == id)
}

fn enabled(doc: &SceneDocument, needs: Needs) -> bool {
    let object_mode = doc.session.mode == Mode::Object;
    match needs {
        Needs::Always => object_mode,
        Needs::Object => object_mode && (doc.session.editor.active.is_some() || !doc.session.editor.selected.is_empty()),
    }
}

/// The commands, with whether each can run now.
pub fn catalogue(doc: &SceneDocument) -> Vec<KitCommand> {
    SPECS
        .iter()
        .map(|spec| KitCommand {
            id: spec.id.into(),
            title: spec.title.into(),
            menu: (!spec.menu.is_empty()).then(|| spec.menu.iter().map(|part| (*part).to_string()).collect()),
            shortcut: None,
            params: None,
            schema: Some((spec.schema)()),
            mutates: true,
            enabled: enabled(doc, spec.needs),
            ..Default::default()
        })
        .collect()
}

/// The commands that cannot run now.
pub fn disabled(doc: &SceneDocument) -> Vec<String> {
    SPECS.iter().filter(|spec| !enabled(doc, spec.needs)).map(|spec| spec.id.to_string()).collect()
}

/// Runs one of these commands, or answers `None` when `command` is not one.
pub fn execute(doc: &mut SceneDocument, command: &str, params: &Value) -> Option<Result<Value, String>> {
    let spec = spec_of(command)?;
    if !params.is_object() && !params.is_null() {
        return Some(Err(bad("the parameters must be an object")));
    }
    let named = params.get("object").or_else(|| params.get("objects")).is_some_and(|value| !value.is_null());
    if !(enabled(doc, spec.needs) || (named && enabled(doc, Needs::Always))) {
        return Some(Err(format!("not-allowed: {} cannot run now", spec.title)));
    }
    let before = doc.step(spec.title);
    let session = doc.session.clone();
    Some(match run(doc, command, params) {
        Ok(value) => {
            doc.tidy_session();
            if let Err(error) = doc.scene.validate() {
                doc.restore(before);
                doc.session = session;
                return Some(Err(format!("not-allowed: {}", error.to_string().replace("invalid-file: ", ""))));
            }
            doc.committed(before).map(|()| value)
        }
        Err(error) => {
            doc.restore(before);
            doc.session = session;
            Err(error)
        }
    })
}

fn object_param(doc: &SceneDocument, params: &Value, key: &str) -> Result<Id, String> {
    match params.get(key).filter(|value| !value.is_null()) {
        Some(value) => params::resolve(&doc.scene, value),
        None => doc.session.editor.active.ok_or_else(|| format!("not-allowed: there is no active object; name `{key}`")),
    }
}

fn property_of(text: &str) -> Result<Property, String> {
    Property::parse(text).ok_or_else(|| bad(format!("unknown property {text}")))
}

/// The channels (target, property, index, path) a driver command names.
fn driven(doc: &SceneDocument, params: &Value) -> Result<Vec<(Id, Property, u8, String)>, String> {
    let object = object_param(doc, params, "object")?;
    let property = property_of(params::string(params, "property")?.ok_or_else(|| bad("`property` is required"))?)?;
    let path = match property {
        Property::BoneLocation | Property::BoneRotation | Property::BoneScale => {
            params::string(params, "bone")?.ok_or_else(|| bad("a bone property names its `bone`"))?.to_string()
        }
        Property::ShapeKey => params::string(params, "shapeKey")?.ok_or_else(|| bad("a shapeKey property names its `shapeKey`"))?.to_string(),
        _ => String::new(),
    };
    let target = if property.on_material() {
        doc.scene.object(object).map_err(|error| error.to_string())?.material.ok_or_else(|| bad("the object has no material"))?
    } else {
        object
    };
    let indices: Vec<u8> = match params::integer(params, "index")? {
        Some(index) if index < u32::from(property.components()) => vec![index as u8],
        Some(index) => return Err(bad(format!("{} has no component {index}", property.name()))),
        None => (0..property.components()).collect(),
    };
    for index in &indices {
        if kition_scene_model::animation::read_at(&doc.scene, target, property, *index, &path).is_none() {
            return Err(bad(format!("that has no {}", property.name())));
        }
    }
    Ok(indices.into_iter().map(|index| (target, property, index, path.clone())).collect())
}

fn target_from(doc: &SceneDocument, item: &Value, object_key: &str, bone_key: &str) -> Result<Option<Target>, String> {
    let Some(value) = item.get(object_key).filter(|value| !value.is_null()) else { return Ok(None) };
    let object = params::resolve(&doc.scene, value)?;
    let bone = item.get(bone_key).and_then(Value::as_str).map(str::to_string);
    if let Some(name) = &bone {
        let armature = kition_scene_model::armature::armature_of(&doc.scene, object).ok_or_else(|| bad(format!("`{bone_key}` needs an armature")))?;
        armature.bone(name).ok_or_else(|| format!("not-found: no bone {name}"))?;
    }
    Ok(Some(Target { object, bone }))
}

fn variable_from(doc: &SceneDocument, item: &Value) -> Result<Variable, String> {
    let name = item.get("name").and_then(Value::as_str).unwrap_or("var").to_string();
    let source = match item.get("type").and_then(Value::as_str).unwrap_or("singleProperty") {
        "singleProperty" => {
            let property = property_of(item.get("property").and_then(Value::as_str).unwrap_or("location"))?;
            let path = item.get("bone").or_else(|| item.get("shapeKey")).and_then(Value::as_str).unwrap_or_default().to_string();
            let mut target = item.get("object").filter(|value| !value.is_null()).map(|value| params::resolve(&doc.scene, value)).transpose()?;
            if property.on_material() {
                target = target.and_then(|object| doc.scene.object(object).ok()).and_then(|object| object.material);
            }
            let index = item.get("index").and_then(Value::as_u64).unwrap_or(0).min(3) as u8;
            Source::SingleProperty { target, property, index, path }
        }
        "transform" => {
            let channel = item.get("channel").and_then(Value::as_str).unwrap_or("locationX");
            let position = CHANNELS.iter().position(|known| *known == channel).ok_or_else(|| bad(format!("unknown channel {channel}")))?;
            let (channel, axis) = match position {
                9 => (TransformChannel::Scale, 3),
                _ => (
                    [TransformChannel::Location, TransformChannel::Rotation, TransformChannel::Scale].get(position / 3).copied().unwrap_or_default(),
                    (position % 3) as u8,
                ),
            };
            let space = match item.get("space").and_then(Value::as_str).unwrap_or("world") {
                "world" => VariableSpace::World,
                "transform" => VariableSpace::Transform,
                "local" => VariableSpace::Local,
                other => return Err(bad(format!("unknown space {other}"))),
            };
            Source::Transform { target: target_from(doc, item, "object", "bone")?, channel, axis, space }
        }
        kind @ ("distance" | "rotationDifference") => {
            let (a, b) = (target_from(doc, item, "a", "aBone")?, target_from(doc, item, "b", "bBone")?);
            if kind == "distance" { Source::Distance { a, b } } else { Source::RotationDifference { a, b } }
        }
        other => return Err(bad(format!("unknown variable type {other}"))),
    };
    Ok(Variable { name, source })
}

/// Applies `kind`, `expression`, and `variables` from `params` to a driver.
fn configure(doc: &SceneDocument, driver: &mut Driver, params: &Value) -> Result<(), String> {
    if let Some(kind) = params::string(params, "kind")? {
        driver.kind = match kind {
            "average" => DriverKind::Average,
            "sum" => DriverKind::Sum,
            "min" => DriverKind::Min,
            "max" => DriverKind::Max,
            "scripted" => DriverKind::Scripted,
            other => return Err(bad(format!("unknown driver kind {other}"))),
        };
    }
    if let Some(expression) = params::string(params, "expression")? {
        driver.expression = expression.to_string();
    }
    if let Some(value) = params.get("variables").filter(|value| !value.is_null()) {
        let items = value.as_array().ok_or_else(|| bad("`variables` is a list"))?;
        driver.variables = items.iter().map(|item| variable_from(doc, item)).collect::<Result<_, _>>()?;
    }
    driver.check().map_err(|error| error.to_string())
}

fn keying_ref(scene: &Scene, name: &str) -> Result<KeyingSetRef, String> {
    if let Some(builtin) = BuiltinKeyingSet::ALL.iter().find(|builtin| builtin.name() == name) {
        return Ok(KeyingSetRef::Builtin(*builtin));
    }
    if scene.animation.keying_sets.sets.iter().any(|set| set.name == name) {
        return Ok(KeyingSetRef::Custom(name.to_string()));
    }
    Err(format!("not-found: no keying set {name}"))
}

/// The scene's own keying set `params` names, or the active one.
fn own_set(scene: &Scene, params: &Value) -> Result<String, String> {
    match params::string(params, "name")? {
        Some(name) if scene.animation.keying_sets.sets.iter().any(|set| set.name == name) => Ok(name.to_string()),
        Some(name) => Err(format!("not-found: no keying set {name} of the scene's own")),
        None => match &scene.animation.keying_sets.active {
            Some(KeyingSetRef::Custom(name)) => Ok(name.clone()),
            _ => Err("not-allowed: name a keying set of the scene's own (or make one active)".into()),
        },
    }
}

fn selection(doc: &SceneDocument, params: &Value) -> Result<Vec<Id>, String> {
    match params::objects(&doc.scene, params, "objects")? {
        Some(ids) => Ok(ids),
        None if !doc.session.editor.selected.is_empty() => Ok(doc.session.editor.selected.clone()),
        None => Err("not-allowed: select an object first, or name `objects`".into()),
    }
}

fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    kition_scene_model::scene::unique_name(base, taken)
}

fn run(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    match command {
        "driver.add" | "driver.set" => {
            let channels = driven(doc, params)?;
            for (target, property, index, path) in &channels {
                let existing = doc.scene.animation.channels.iter().position(|channel| {
                    channel.target == *target && channel.property == *property && channel.index == *index && channel.path == *path
                });
                let mut driver = match existing.and_then(|slot| doc.scene.animation.channels.get(slot)) {
                    Some(channel) if channel.driver.is_none() => {
                        return Err(bad(format!("{} has keyframes; clear them before driving it", property.name())));
                    }
                    Some(channel) => channel.driver.clone().unwrap_or_default(),
                    None if command == "driver.set" => return Err(format!("not-found: no driver on {}", property.name())),
                    // A new driver has a variable `var` to fill in, unless the command gives its
                    // own expression without one.
                    None => {
                        let expression = params::string(params, "expression")?.unwrap_or("var");
                        let wants_var = kition_scene_model::expression::parse(expression)
                            .map(|parsed| parsed.names())
                            .unwrap_or_default()
                            .iter()
                            .any(|name| name == "var");
                        Driver {
                            variables: if wants_var {
                                vec![Variable {
                                    name: "var".into(),
                                    source: Source::SingleProperty { target: None, property: Property::Location, index: 0, path: String::new() },
                                }]
                            } else {
                                Vec::new()
                            },
                            ..Driver::default()
                        }
                    }
                };
                configure(doc, &mut driver, params)?;
                match existing.and_then(|slot| doc.scene.animation.channels.get_mut(slot)) {
                    Some(channel) => channel.driver = Some(driver),
                    None => doc.scene.animation.channels.push(Channel {
                        target: *target,
                        property: *property,
                        index: *index,
                        path: path.clone(),
                        keys: Vec::new(),
                        driver: Some(driver),
                        modifiers: Vec::new(),
                    }),
                }
            }
            let frame = doc.scene.animation.timeline.frame as f32;
            doc.scene.apply_drivers(frame);
            Ok(json!({ "drivers": channels.len() }))
        }
        "driver.remove" => {
            let channels = driven(doc, params)?;
            let before = doc.scene.animation.channels.len();
            doc.scene.animation.channels.retain(|channel| {
                channel.driver.is_none()
                    || !channels.iter().any(|(target, property, index, path)| {
                        channel.target == *target && channel.property == *property && channel.index == *index && channel.path == *path
                    })
            });
            let removed = before - doc.scene.animation.channels.len();
            if removed == 0 {
                return Err("not-allowed: nothing there is driven".into());
            }
            Ok(json!({ "removed": removed }))
        }
        "keyingSet.new" => {
            let base = params::string(params, "name")?.unwrap_or("Keying Set");
            let sets = &doc.scene.animation.keying_sets.sets;
            let name =
                unique(base, |name| sets.iter().any(|set| set.name == name) || BuiltinKeyingSet::ALL.iter().any(|builtin| builtin.name() == name));
            doc.scene.animation.keying_sets.sets.push(KeyingSet { name: name.clone(), paths: Vec::new() });
            doc.scene.animation.keying_sets.active = Some(KeyingSetRef::Custom(name.clone()));
            Ok(json!({ "name": name }))
        }
        "keyingSet.remove" => {
            let name = own_set(&doc.scene, params)?;
            let sets = &mut doc.scene.animation.keying_sets;
            sets.sets.retain(|set| set.name != name);
            if sets.active == Some(KeyingSetRef::Custom(name.clone())) {
                sets.active = None;
            }
            Ok(json!({ "removed": name }))
        }
        "keyingSet.addPaths" => {
            let name = own_set(&doc.scene, params)?;
            let objects = selection(doc, params)?;
            let bones: Option<Vec<String>> = match params.get("bones").filter(|value| !value.is_null()) {
                Some(Value::Array(items)) => Some(
                    items
                        .iter()
                        .map(|item| item.as_str().map(str::to_string).ok_or_else(|| bad("bones are named by strings")))
                        .collect::<Result<_, _>>()?,
                ),
                Some(_) => return Err(bad("`bones` is a list of names")),
                None => None,
            };
            let properties: Vec<Property> = match params.get("properties").filter(|value| !value.is_null()) {
                Some(Value::Array(items)) => items.iter().map(|item| property_of(item.as_str().unwrap_or_default())).collect::<Result<_, _>>()?,
                Some(_) => return Err(bad("`properties` is a list")),
                None if bones.is_some() => vec![Property::BoneLocation, Property::BoneRotation, Property::BoneScale],
                None => vec![Property::Location, Property::Rotation, Property::Scale],
            };
            let mut added = Vec::new();
            for object in &objects {
                match &bones {
                    Some(names) => {
                        for bone in names {
                            for property in properties.iter().filter(|property| property.named() && **property != Property::ShapeKey) {
                                added.push(KeyingPath { target: *object, property: *property, path: bone.clone() });
                            }
                        }
                    }
                    None => {
                        for property in &properties {
                            let target = if property.on_material() {
                                doc.scene.object(*object).ok().and_then(|object| object.material).ok_or_else(|| bad("an object has no material"))?
                            } else {
                                *object
                            };
                            added.push(KeyingPath { target, property: *property, path: String::new() });
                        }
                    }
                }
            }
            let set = doc.scene.animation.keying_sets.sets.iter_mut().find(|set| set.name == name).ok_or("not-found: the keying set")?;
            let mut count = 0;
            for path in added {
                if !set.paths.contains(&path) {
                    set.paths.push(path);
                    count += 1;
                }
            }
            Ok(json!({ "name": name, "added": count }))
        }
        "keyingSet.removePath" => {
            let name = own_set(&doc.scene, params)?;
            let index = params::integer(params, "index")?.ok_or_else(|| bad("`index` is required"))? as usize;
            let set = doc.scene.animation.keying_sets.sets.iter_mut().find(|set| set.name == name).ok_or("not-found: the keying set")?;
            if index >= set.paths.len() {
                return Err(format!("not-found: the keying set has no path {index}"));
            }
            set.paths.remove(index);
            Ok(json!({ "name": name, "paths": set.paths.len() }))
        }
        "keyingSet.setActive" => {
            let active = match params.get("name") {
                None | Some(Value::Null) => None,
                Some(Value::String(name)) => Some(keying_ref(&doc.scene, name)?),
                Some(_) => return Err(bad("`name` is a keying set's name or null")),
            };
            doc.scene.animation.keying_sets.active = active;
            Ok(json!({ "active": params.get("name").cloned().unwrap_or(Value::Null) }))
        }
        "action.new" => {
            let base = params::string(params, "name")?.unwrap_or("Action");
            let name = unique(base, |name| doc.scene.animation.actions.iter().any(|action| action.name == name));
            let mut channels = Vec::new();
            if let Some(value) = params.get("object").filter(|value| !value.is_null()) {
                let object = params::resolve(&doc.scene, value)?;
                channels = doc
                    .scene
                    .animation
                    .channels
                    .iter()
                    .filter(|channel| {
                        channel.target == object && channel.driver.is_none() && !channel.keys.is_empty() && channel.property != Property::ShapeKey
                    })
                    .map(|channel| ActionChannel {
                        property: channel.property,
                        index: channel.index,
                        path: channel.path.clone(),
                        keys: channel.keys.clone(),
                        modifiers: Vec::new(),
                    })
                    .collect();
                if channels.is_empty() {
                    return Err("not-allowed: the object has no keys to take".into());
                }
            }
            let count = channels.len();
            doc.scene.animation.actions.push(Action { name: name.clone(), channels });
            Ok(json!({ "name": name, "channels": count }))
        }
        "action.assign" => {
            let name = params::string(params, "action")?.ok_or_else(|| bad("`action` is required"))?;
            let action = doc.scene.animation.action(name).cloned().ok_or_else(|| format!("not-found: no action {name}"))?;
            let objects = selection(doc, params)?;
            if objects.iter().any(|object| doc.scene.tweaked_strip(*object).is_some()) {
                return Err("not-allowed: leave tweak mode first".into());
            }
            let mut keyed = 0;
            for object in objects {
                let armature = kition_scene_model::armature::armature_of(&doc.scene, object).cloned();
                // The object's own keys go; the action's take their place.
                doc.scene
                    .animation
                    .channels
                    .retain(|channel| channel.driver.is_some() || channel.target != object || channel.property == Property::ShapeKey);
                for channel in &action.channels {
                    let fits = match channel.property {
                        Property::BoneLocation | Property::BoneRotation | Property::BoneScale => {
                            armature.as_ref().is_some_and(|armature| armature.bone(&channel.path).is_some())
                        }
                        property => kition_scene_model::animation::read_at(&doc.scene, object, property, channel.index, "").is_some(),
                    };
                    if !fits || doc.scene.driver_of(object, channel.property, channel.index, &channel.path).is_some() {
                        continue;
                    }
                    doc.scene.animation.channels.push(Channel {
                        target: object,
                        property: channel.property,
                        index: channel.index,
                        path: channel.path.clone(),
                        keys: channel.keys.clone(),
                        driver: None,
                        modifiers: Vec::new(),
                    });
                    keyed += 1;
                }
            }
            let frame = doc.scene.animation.timeline.frame as f32;
            doc.scene.go_to_frame(frame);
            Ok(json!({ "action": name, "channels": keyed }))
        }
        "action.remove" | "action.rename" => {
            let name = params::string(params, "name")?.ok_or_else(|| bad("`name` is required"))?.to_string();
            if doc.scene.animation.action(&name).is_none() {
                return Err(format!("not-found: no action {name}"));
            }
            let to = if command == "action.rename" {
                let to = params::string(params, "to")?.ok_or_else(|| bad("`to` is required"))?.trim().to_string();
                if to.is_empty() || doc.scene.animation.actions.iter().any(|action| action.name == to) {
                    return Err(bad(format!("an action is already called {to}, or the name is empty")));
                }
                Some(to)
            } else {
                None
            };
            match &to {
                Some(to) => {
                    if let Some(action) = doc.scene.animation.actions.iter_mut().find(|action| action.name == name) {
                        action.name = to.clone();
                    }
                }
                None => doc.scene.animation.actions.retain(|action| action.name != name),
            }
            // Strips follow the rename, or go with a removed action.
            doc.scene.nla_follow_action(&name, to.as_deref());
            // Action constraints follow the rename, or let go of a removed action.
            for object in &mut doc.scene.objects {
                let mut stacks: Vec<&mut Vec<kition_scene_model::Constraint>> = vec![&mut object.constraints];
                if let ObjectData::Armature(armature) = &mut object.data {
                    stacks.extend(armature.bones.iter_mut().map(|bone| &mut bone.constraints));
                }
                for stack in stacks {
                    for constraint in stack.iter_mut() {
                        if let ConstraintKind::Action { action: Some(played), .. } = &mut constraint.kind
                            && *played == name
                        {
                            match &to {
                                Some(to) => *played = to.clone(),
                                None => constraint.kind = clear_action(&constraint.kind),
                            }
                        }
                    }
                }
            }
            Ok(json!({ "name": to.unwrap_or(name) }))
        }
        other => Err(format!("unknown-command: {other}")),
    }
}

fn clear_action(kind: &ConstraintKind) -> ConstraintKind {
    let mut kind = kind.clone();
    if let ConstraintKind::Action { action, .. } = &mut kind {
        *action = None;
    }
    kind
}

/// The keying set `animation.insertKey` or `pose.insertKey` uses: the one named, or with no
/// properties given the active one.
pub fn keying_set_for(doc: &SceneDocument, params: &Value) -> Result<Option<KeyingSetRef>, String> {
    if let Some(name) = params::string(params, "keyingSet")? {
        return keying_ref(&doc.scene, name).map(Some);
    }
    if params.get("properties").is_some_and(|value| !value.is_null()) {
        return Ok(None);
    }
    Ok(doc.scene.animation.keying_sets.active.clone())
}

fn keying_name(set: &KeyingSetRef) -> String {
    match set {
        KeyingSetRef::Builtin(builtin) => builtin.name().to_string(),
        KeyingSetRef::Custom(name) => name.clone(),
    }
}

fn target_json(scene: &Scene, target: &Option<Target>) -> Value {
    match target {
        Some(target) => {
            json!({ "object": target.object, "name": scene.object(target.object).map(|object| object.name.clone()).unwrap_or_default(), "bone": target.bone })
        }
        None => Value::Null,
    }
}

fn variable_json(scene: &Scene, variable: &Variable) -> Value {
    let mut value = match &variable.source {
        Source::SingleProperty { target, property, index, path } => {
            json!({ "type": "singleProperty", "object": target, "property": property.name(), "index": index, "path": path })
        }
        Source::Transform { target, channel, axis, space } => {
            let index = match channel {
                TransformChannel::Location => 0,
                TransformChannel::Rotation => 3,
                TransformChannel::Scale => 6,
            } + usize::from(*axis);
            let space = match space {
                VariableSpace::World => "world",
                VariableSpace::Transform => "transform",
                VariableSpace::Local => "local",
            };
            json!({ "type": "transform", "target": target_json(scene, target), "channel": CHANNELS.get(index).copied().unwrap_or("locationX"), "space": space })
        }
        Source::Distance { a, b } => json!({ "type": "distance", "a": target_json(scene, a), "b": target_json(scene, b) }),
        Source::RotationDifference { a, b } => json!({ "type": "rotationDifference", "a": target_json(scene, a), "b": target_json(scene, b) }),
    };
    if let Some(fields) = value.as_object_mut() {
        fields.insert("name".into(), json!(variable.name));
    }
    value
}

/// Drivers on the animation channels, actions, and keying sets in `inspect()`.
pub fn extend_inspect(doc: &SceneDocument, inspected: &mut Value) {
    let scene = &doc.scene;
    let frame = scene.animation.timeline.frame as f32;
    let drivers: Vec<Value> = scene
        .animation
        .channels
        .iter()
        .filter_map(|channel| {
            let driver = channel.driver.as_ref()?;
            let kind = match driver.kind {
                DriverKind::Average => "average",
                DriverKind::Sum => "sum",
                DriverKind::Min => "min",
                DriverKind::Max => "max",
                DriverKind::Scripted => "scripted",
            };
            let (value, error) = match driver.evaluate(scene, frame) {
                Ok(value) => (json!((f64::from(value) * 10_000.0).round() / 10_000.0), Value::Null),
                Err(error) => (Value::Null, json!(error)),
            };
            let owner = if channel.property.on_material() { "material" } else { "object" };
            let mut entry = json!({
                "property": channel.property.name(),
                "index": channel.index,
                "kind": kind,
                "expression": driver.expression,
                "variables": driver.variables.iter().map(|variable| variable_json(scene, variable)).collect::<Vec<_>>(),
                "value": value,
                "error": error,
                "mapped": !channel.keys.is_empty(),
            });
            if let Some(fields) = entry.as_object_mut() {
                fields.insert(owner.into(), json!(channel.target));
                if !channel.path.is_empty() {
                    fields.insert("path".into(), json!(channel.path));
                }
            }
            Some(entry)
        })
        .collect();
    let actions: Vec<Value> = scene
        .animation
        .actions
        .iter()
        .map(|action| {
            let mut properties: Vec<&str> = action.channels.iter().map(|channel| channel.property.name()).collect();
            properties.dedup();
            let range = action.range().map(|(start, end)| json!([start, end])).unwrap_or(Value::Null);
            json!({ "name": action.name, "channels": action.channels.len(), "properties": properties, "range": range })
        })
        .collect();
    let sets: Vec<Value> = scene
        .animation
        .keying_sets
        .sets
        .iter()
        .map(|set| {
            let paths: Vec<Value> = set
                .paths
                .iter()
                .map(|path| {
                    let name = scene
                        .object(path.target)
                        .map(|object| object.name.clone())
                        .or_else(|_| scene.material(path.target).map(|material| material.name.clone()))
                        .unwrap_or_default();
                    json!({ "target": path.target, "name": name, "property": path.property.name(), "path": path.path })
                })
                .collect();
            json!({ "name": set.name, "paths": paths })
        })
        .collect();
    let Some(animation) = inspected.get_mut("animation").and_then(Value::as_object_mut) else { return };
    animation.insert("drivers".into(), json!(drivers));
    animation.insert("actions".into(), json!(actions));
    animation.insert(
        "keyingSets".into(),
        json!({
            "sets": sets,
            "active": scene.animation.keying_sets.active.as_ref().map(keying_name),
            "builtins": BuiltinKeyingSet::ALL.iter().map(|builtin| builtin.name()).collect::<Vec<_>>(),
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_schema_is_well_formed() {
        for spec in SPECS {
            kition_kit::schema::validate(&(spec.schema)()).unwrap_or_else(|error| panic!("{}: {error}", spec.id));
        }
    }
}
