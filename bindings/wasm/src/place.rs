//! Places and premises: what a command names by where it is rather than by id, and what each call
//! reads or replaces of the objects it names (task cards C8-scene, C8-scene-2), for kition-sdk's
//! premise guard (`KitCommand::places`, `KitDocument::under`, `covered`, `derived`).
//!
//! Kition Scene's addresses besides ids:
//! - **A world point** (`pick: [x, y, z]`, on every command that takes `object` or `objects`): the
//!   object found there now ([`pick`]). `under` answers it with `covers` (no window can say what is
//!   at a point), and the guard asks [`covered`] whether the objects others moved, hid,
//!   reparented, constrained, deleted or whose mesh bounds changed were there at the agent's
//!   basis: the place is rebuilt from their values then and the point picked again.
//! - **Element indexes.** An edit-mode command's `elements` (`{object, vertices?, faces?}`) names
//!   vertices and faces of a mesh by index; they stay valid while the mesh's topology (vertex
//!   count, face count, which vertices each face has) is unchanged. Such a write's premise is the
//!   mesh's derived `~topology` ([`derived`]), not the mesh: a vertex moved elsewhere in the mesh
//!   keeps it, a subdivide refuses it.
//! - **Names.** An object, a material or a node group named by its name: the premise holds that
//!   object's `name` beside what the command needs of it, so a write by a name another object took
//!   since is refused.
//! - **Indexes into an object's lists** (a modifier, an object constraint, by `index`): the premise
//!   is that item's path (`modifiers.2`), so a change of another modifier keeps it and an insert
//!   above it (the list told whole) refuses it.
//!
//! What each call rests on besides its places ([`crate::rests`]) is stated for every write
//! command, read from its handler (C8-scene-read).
//!
//! Object positions in the object model (`"position": true`): `location`, `rotation`, `scale`,
//! `parent`, `parentInverse`. A command by id that does not depend on them
//! ([`POSITION_INDEPENDENT`], each read in its code) is not refused when others only moved its
//! target.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};

use glam::{Mat4, Vec3};
use kition_kit::{Covered, CoveredAsk, CoveredAt, Covers, DerivedValues, Premise, Under, Within};
use kition_scene_model::{Id, Object, ObjectData, Scene};
use serde_json::{Map, Value, json};

use crate::params::{self, bad};
use crate::rests::{self, Editor, Need, On};

/// The parameter naming an object by a world point.
pub const PICK: &str = "pick";

/// How near a world point an object without a surface (a light, a camera, an empty, a curve, a
/// text, a lattice, an armature) must have its origin to be picked, in metres.
pub const ORIGIN_REACH: f32 = 0.25;

/// The commands whose effect on the objects they name does not depend on where those objects are
/// (their location, rotation, scale or parent), read in their code: a name, visibility, a
/// material slot, a light's and a camera's settings.
pub const POSITION_INDEPENDENT: [&str; 7] =
    ["object.rename", "object.hide", "object.reveal", "material.assign", "material.clear", "light.set", "camera.set"];

/// What besides the `position` fields may move an object into or out of a world point: whether it
/// is hidden, its mesh's local bounds (`~bounds`, a derived value), and its constraints (a Child
/// Of or Copy Location constraint places it).
pub const COVERS: [&str; 3] = ["hidden", "~bounds", "constraints"];

/// World points and the outliner's insertion destination are places.
pub fn places(command: &str, schema: Option<&Value>) -> &'static [&'static str] {
    if command == "object.order" {
        &["pick", "index"]
    } else if command == "compositor.pickMatte" {
        // A point of the render through the scene camera (C8-scene-engine 5b).
        &["x", "y"]
    } else if schema.and_then(|schema| schema.get(PICK)).is_some() {
        &["pick"]
    } else {
        &[]
    }
}

/// The `pick` parameter's schema.
pub fn pick_schema() -> Value {
    json!({
        "type": "array",
        "items": { "type": "number" },
        "description": "Instead of `object`/`objects`: a world point [x, y, z] in metres; the command acts on the object there now (the smallest whose mesh's bounds hold the point, else the nearest light, camera, empty, curve, text, lattice or armature whose origin is within 0.25 m). A place: with `basis`, a write by it is refused when what is at the point is no longer what was there (moved away, another moved in, its parent moved)"
    })
}

/// Local bounds told for an object instead of read from its mesh (`context.covered`'s `~bounds`):
/// `None` inside for an object without a mesh.
type Told = HashMap<Id, Option<(Vec3, Vec3)>>;

/// The world bounds of the mesh's local bounding box (no modifiers). Live picks and replay
/// transform the same eight corners: replay only knows the declared local `~bounds`.
fn bounds(scene: &Scene, id: Id, told: &Told) -> Option<(Vec3, Vec3)> {
    let object = scene.object(id).ok()?;
    let world = scene.world_matrix(id).ok()?;
    let (lo, hi) = match told.get(&id) {
        Some(Some(bounds)) => *bounds,
        Some(None) => return None,
        None => match &object.data {
            ObjectData::Mesh(mesh) => {
                let first = *mesh.positions.first()?;
                mesh.positions.iter().fold((first, first), |(lo, hi), point| (lo.min(*point), hi.max(*point)))
            }
            _ => return None,
        },
    };
    let mut points = (0..8).map(|n: u8| {
        let corner = Vec3::new(if n & 1 == 0 { lo.x } else { hi.x }, if n & 2 == 0 { lo.y } else { hi.y }, if n & 4 == 0 { lo.z } else { hi.z });
        world.transform_point3(corner)
    });
    let first = points.next()?;
    Some(points.fold((first, first), |(lo, hi), point| (lo.min(point), hi.max(point))))
}

/// The object at world point `at`: the smallest (by bounds volume, then list order) visible mesh
/// whose world bounds hold it, else the visible object without a mesh whose origin is nearest,
/// within [`ORIGIN_REACH`].
pub fn pick(scene: &Scene, at: Vec3) -> Option<Id> {
    pick_with(scene, at, &Told::new())
}

/// [`pick`] with some objects' local bounds told.
fn pick_with(scene: &Scene, at: Vec3, told: &Told) -> Option<Id> {
    let mut best: Option<(f32, Id)> = None;
    for object in scene.objects.iter().filter(|object| !object.hidden) {
        let Some((lo, hi)) = bounds(scene, object.id, told) else { continue };
        // A flat mesh (a plane) has no thickness: a millimetre's slack holds a point on it.
        let slack = Vec3::splat(1e-3);
        if at.cmpge(lo - slack).all() && at.cmple(hi + slack).all() {
            let size = (hi - lo).max(slack);
            let volume = size.x * size.y * size.z;
            if best.is_none_or(|(smallest, _)| volume < smallest) {
                best = Some((volume, object.id));
            }
        }
    }
    if let Some((_, id)) = best {
        return Some(id);
    }
    let meshless = |object: &&Object| match told.get(&object.id) {
        Some(bounds) => bounds.is_none(),
        None => !matches!(object.data, ObjectData::Mesh(_)),
    };
    let mut nearest: Option<(f32, Id)> = None;
    for object in scene.objects.iter().filter(|object| !object.hidden).filter(meshless) {
        let Ok(world) = scene.world_matrix(object.id) else { continue };
        let distance = world.transform_point3(Vec3::ZERO).distance(at);
        if distance <= ORIGIN_REACH && nearest.is_none_or(|(near, _)| distance < near) {
            nearest = Some((distance, object.id));
        }
    }
    nearest.map(|(_, id)| id)
}

/// `params` with `pick` resolved to the object there now, written into the command's `objects`
/// (a list) or `object` parameter (`schema` says which it takes). Refused when the step also names
/// its objects, or nothing is at the point.
pub fn resolve_pick(scene: &Scene, schema: Option<&Value>, params: &Value) -> Result<Value, String> {
    let Some(given) = params.get(PICK).filter(|value| !value.is_null()) else { return Ok(params.clone()) };
    let at = params::vec3(params, PICK)?.ok_or_else(|| bad(format!("`pick` is {given}: give a world point [x, y, z] in metres")))?;
    let key = ["objects", "object"].into_iter().find(|key| schema.and_then(|schema| schema.get(*key)).is_some());
    let key = key.ok_or_else(|| bad("`pick` names the object a command acts on: this command takes no object"))?;
    if params.get(key).is_some_and(|value| !value.is_null()) {
        return Err(bad(format!("`pick` and `{key}` both name what the command acts on: give one")));
    }
    let id = pick(scene, at).ok_or_else(|| {
        bad(format!("`pick` is [{}, {}, {}]: no object is there; name the object by its id as scene_inspect lists it", at.x, at.y, at.z))
    })?;
    let mut params = params.clone();
    if let Some(map) = params.as_object_mut() {
        map.remove(PICK);
        map.insert(key.to_owned(), if key == "objects" { json!([id]) } else { json!(id) });
    }
    Ok(params)
}

/// The objects, materials and node groups a call names by their names (not by id), by the id the
/// name gives now: in its object parameters (`object`, `objects`, `elements.object`, and every
/// parameter `objects` lists), `material`, and `group`.
fn named_by_name(scene: &Scene, params: &Value, objects: &[&str]) -> Vec<Id> {
    let mut found = Vec::new();
    let mut values: Vec<(&str, &Value)> = Vec::new();
    for key in ["object", "objects"].into_iter().chain(objects.iter().copied()) {
        values.extend(params::object_values(params, key).into_iter().map(|value| (key, value)));
    }
    for (key, value) in values {
        let Some(name) = value.as_str().filter(|name| !name.is_empty() && !name.bytes().all(|byte| byte.is_ascii_digit())) else { continue };
        let id = match key {
            "material" => scene.materials.iter().find(|material| material.name == name).map(|material| material.id),
            "group" | "shader" | "volume" => scene.node_groups.iter().find(|group| group.name == name).map(|group| group.id),
            _ => scene.object_named(name).map(|object| object.id),
        };
        found.extend(id);
    }
    found
}

/// `params` with the object parameter a world point stands for naming no object.
fn named_nothing(schema: Option<&Value>, params: &Value) -> Value {
    let mut params = params.clone();
    let key = ["objects", "object"].into_iter().find(|key| schema.and_then(|schema| schema.get(*key)).is_some());
    if let (Some(key), Some(map)) = (key, params.as_object_mut()) {
        map.remove(PICK);
        map.insert(key.to_owned(), json!([]));
    }
    params
}

/// What a call `command` with `params` depends on (`KitDocument::under`): for a world point
/// (`pick`), the object there now and `covers` (the guard asks [`covered`]); for every call, what
/// [`rests::rests`] says it rests on, and for every object, material or node group named by its
/// name, that name besides it (the guard's whole default when no rule names it). `schema` is the
/// command's, `objects` its object parameters, `editor` what a left-out parameter falls back to.
/// `None` when there is nothing to say (the guard's default holds).
pub fn under(
    scene: &Scene,
    editor: Editor,
    command: &str,
    schema: Option<&Value>,
    objects: &[&str],
    params: &Value,
) -> Result<Option<Under>, String> {
    let picked = params::vec3(params, PICK)?.map(|at| pick(scene, at));
    // The call as it runs: a world point written into its object parameter.
    let resolved = match picked {
        Some(Some(_)) => resolve_pick(scene, schema, params).unwrap_or_else(|_| params.clone()),
        // Nothing at the point: the call names no object (it is refused), not the selection.
        Some(None) => named_nothing(schema, params),
        None => params.clone(),
    };
    let mut found: BTreeMap<On, Need> = rests::rests(scene, editor, command, &resolved).unwrap_or_default();
    let mut keys: Vec<&str> = objects.to_vec();
    for key in params::point_objects(command) {
        keys.push(key);
    }
    for id in named_by_name(scene, params, &keys) {
        match found.get_mut(&On::Id(id)) {
            Some(need) => {
                if let Some(fields) = need.fields.as_mut() {
                    fields.insert("name".to_owned());
                }
            }
            // Not stated: every field (its name too) and every position, as the guard holds an
            // object named by id.
            None => {
                found.insert(On::Id(id), Need::all());
            }
        }
    }
    let premises: Vec<Premise> = found.iter().map(|(on, need)| need.premise(on)).collect();
    if command == "object.order"
        && let Some(index) = params::integer(params, "index")?
    {
        let id = match resolved.get("object").filter(|value| !value.is_null()) {
            // Nothing at a world point names no source.
            Some(value) if value.is_array() => None,
            Some(value) => Some(params::resolve(scene, value)?),
            None => editor.active,
        };
        let source = scene.objects.iter().position(|object| Some(object.id) == id);
        // The engine removes the source first. Express the destination gap in the list
        // before that removal, as required by the guard's insertion-window contract.
        let destination = (index as usize).min(scene.objects.len().saturating_sub(1));
        let gap = destination.saturating_add(usize::from(source.is_some_and(|source| source <= destination)));
        return Ok(Some(Under {
            objects: picked.flatten().map(|id| vec![json!(id)]).unwrap_or_default(),
            covers: picked.map(|_| Covers { fields: COVERS.iter().map(|field| (*field).to_owned()).collect() }),
            premises,
            within: vec![Within { parent: json!("scene"), field: "objects".into(), from: gap, to: gap, ..Default::default() }],
            ..Default::default()
        }));
    }
    if let Some(picked) = picked {
        let objects = picked.map(|id| vec![json!(id)]).unwrap_or_default();
        return Ok(Some(Under {
            objects,
            covers: Some(Covers { fields: COVERS.iter().map(|field| (*field).to_owned()).collect() }),
            premises,
            ..Default::default()
        }));
    }
    // A literal point in these unions names no scene object. Say so even without other
    // premises, or the guard must treat an unrelated structural edit as an unknown place.
    let literal_point = params::point_objects(command).any(|key| params.get(key).is_some_and(Value::is_array));
    Ok((!premises.is_empty() || literal_point).then(|| Under { premises, ..Default::default() }))
}

/// Whether `id`'s place in the world derives from `from`'s: `from` is `id`, one of its parents,
/// or (through them) the target of one of their constraints.
pub(crate) fn derives_from(scene: &Scene, id: Id, from: Id) -> bool {
    let mut seen = BTreeSet::new();
    let mut queue = vec![id];
    while let Some(at) = queue.pop() {
        if at == from {
            return true;
        }
        if !seen.insert(at) || seen.len() > scene.objects.len() {
            continue;
        }
        let Ok(object) = scene.object(at) else { continue };
        queue.extend(object.parent);
        queue.extend(
            object
                .constraints
                .iter()
                .filter(|constraint| constraint.enabled && constraint.influence > 0.0)
                .flat_map(|constraint| constraint.kind.targets())
                .map(|target| target.object),
        );
    }
    false
}

/// What the world point a call names picked at the agent's basis (`KitDocument::covered`): the
/// scene copied with every asked object's values then put in at once (location, rotation, scale,
/// parent, parent inverse, hidden, constraints, and its mesh's local bounds `~bounds`; a deleted
/// one put back as an object of those values, a created one left out), and the point picked
/// there. An object is covered when what is picked derives its place from it ([`derives_from`]:
/// it, a child of it, an object constrained to it). `None` for a call without a point; an object
/// some of whose values are not known answers `null`. The document does not change.
pub fn covered(scene: &Scene, params: &Value, objects: &[CoveredAsk]) -> Result<Option<CoveredAt>, String> {
    let Some(at) = params::vec3(params, PICK)? else { return Ok(None) };
    let mut before = scene.clone();
    let mut told = Told::new();
    let id_of = |value: &Value| value.as_u64().or_else(|| value.as_str().and_then(|text| text.parse().ok())).and_then(|id| u32::try_from(id).ok());
    for asked in objects {
        let id = id_of(&asked.id).ok_or_else(|| bad(format!("{} is no object id", asked.id)))?;
        // Created since the basis: not there then.
        if asked.created {
            before.objects.retain(|object| object.id != id);
            continue;
        }
        if before.object(id).is_err() {
            before.objects.push(Object::new(id, String::new(), ObjectData::Empty { size: 0.0 }));
            told.insert(id, None);
        }
        let Some(object) = before.objects.iter_mut().find(|object| object.id == id) else { continue };
        let was = &asked.before;
        put_back_place(object, was)?;
        if let Some(bounds) = was.get("~bounds") {
            told.insert(id, local_bounds_of(bounds));
        }
    }
    // A parent a deleted child names may be gone too: a cycle or a dangling parent is no place.
    let picked = pick_with(&before, at, &told);
    let covered = objects
        .iter()
        .map(|asked| {
            let covered = match (asked.unknown.is_empty(), id_of(&asked.id)) {
                (true, Some(id)) => Some(picked.is_some_and(|picked| derives_from(&before, picked, id))),
                _ => None,
            };
            Covered { id: asked.id.clone(), covered, ..Default::default() }
        })
        .collect();
    Ok(Some(CoveredAt { under: picked.map(|id| vec![json!(id)]).unwrap_or_default(), covered }))
}

/// Puts `was`'s values of what places an object back into it: location, rotation, scale, parent,
/// parent inverse, hidden, constraints (those `was` has).
pub(crate) fn put_back_place(object: &mut Object, was: &Map<String, Value>) -> Result<(), String> {
    let parse = |name: &str| was.get(name).map(|value| serde_json::from_value::<Vec3>(value.clone()));
    if let Some(location) = parse("location") {
        object.location = location.map_err(|error| bad(format!("location: {error}")))?;
    }
    if let Some(rotation) = parse("rotation") {
        object.rotation = rotation.map_err(|error| bad(format!("rotation: {error}")))?;
    }
    if let Some(scale) = was.get("scale") {
        object.scale =
            if scale.is_null() { Vec3::ONE } else { serde_json::from_value(scale.clone()).map_err(|error| bad(format!("scale: {error}")))? };
    }
    if let Some(parent) = was.get("parent") {
        object.parent = parent.as_u64().and_then(|parent| u32::try_from(parent).ok());
    }
    if let Some(inverse) = was.get("parentInverse") {
        object.parent_inverse = if inverse.is_null() {
            Mat4::IDENTITY
        } else {
            serde_json::from_value(inverse.clone()).map_err(|error| bad(format!("parentInverse: {error}")))?
        };
    }
    if let Some(hidden) = was.get("hidden") {
        object.hidden = hidden.as_bool().unwrap_or(false);
    }
    if let Some(constraints) = was.get("constraints") {
        object.constraints = if constraints.is_null() {
            Vec::new()
        } else {
            serde_json::from_value(constraints.clone()).map_err(|error| bad(format!("constraints: {error}")))?
        };
    }
    Ok(())
}

fn local_bounds_of(value: &Value) -> Option<(Vec3, Vec3)> {
    let pair: [Vec3; 2] = serde_json::from_value(value.clone()).ok()?;
    Some((pair[0], pair[1]))
}

/// The derived values of objects (`KitDocument::derived`), by id (every object when `ids` is
/// empty), each a function of the object's own mesh: `topology`, its vertex and face counts and a
/// hash of its faces (`"8v6f:4e593f69"`: element indexes stay valid while it is unchanged), and
/// `bounds`, its local bounds `[[min x, y, z], [max x, y, z]]`; `null` for an object without a mesh.
pub fn derived(scene: &Scene, ids: &[Value], names: &[String]) -> Result<Vec<DerivedValues>, String> {
    let wanted = |name: &str| names.is_empty() || names.iter().any(|wanted| wanted == name);
    let chosen: Vec<&Object> = if ids.is_empty() {
        scene.objects.iter().collect()
    } else {
        ids.iter()
            .map(|id| params::resolve(scene, id).and_then(|id| scene.object(id).map_err(|error| error.to_string())))
            .collect::<Result<Vec<_>, _>>()?
    };
    Ok(chosen
        .into_iter()
        .map(|object| {
            let mut values = Map::new();
            let mesh = match &object.data {
                ObjectData::Mesh(mesh) => Some(mesh),
                _ => None,
            };
            if wanted("topology") {
                let topology = mesh.map_or(Value::Null, |mesh| {
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    mesh.faces.hash(&mut hasher);
                    json!(format!("{}v{}f:{:08x}", mesh.positions.len(), mesh.faces.len(), hasher.finish() & 0xffff_ffff))
                });
                values.insert("topology".into(), topology);
            }
            if wanted("bounds") {
                let bounds = mesh.and_then(|mesh| {
                    let first = *mesh.positions.first()?;
                    let (lo, hi) = mesh.positions.iter().fold((first, first), |(lo, hi), point| (lo.min(*point), hi.max(*point)));
                    Some(json!([[lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]]))
                });
                values.insert("bounds".into(), bounds.unwrap_or(Value::Null));
            }
            DerivedValues { id: json!(object.id), values }
        })
        .collect())
}

/// How many objects beyond `ids` moved in the world with them: their descendants, and the objects
/// constrained to any of them (C8-scene-2). Their own fields did not change, so neither the
/// receipt nor `others` lists them; transform results and `KitDocument::carried` report the count.
pub fn carried(scene: &Scene, ids: &[Id]) -> usize {
    scene.objects.iter().filter(|object| !ids.contains(&object.id) && ids.iter().any(|id| derives_from(scene, object.id, *id))).count()
}

/// Scene-list reordering is not a world move. These fields feed the owner's world matrix;
/// its name, visibility, geometry and shading do not move its descendants.
pub fn carries_transform(id: Id, changes: &[Value]) -> bool {
    changes.iter().any(|change| {
        let same =
            change.get("id").is_some_and(|value| value.as_u64().or_else(|| value.as_str().and_then(|text| text.parse().ok())) == Some(u64::from(id)));
        if !same {
            return false;
        }
        let mut paths: Vec<_> = change
            .get("edits")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|edit| edit.get("field").and_then(Value::as_str))
            .collect();
        if paths.is_empty() {
            paths.extend(change.get("fields").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str));
        }
        paths.into_iter().any(|path| match path.split('.').next() {
            Some("constraints") => path.rsplit('.').next() != Some("name"),
            Some("location" | "rotation" | "scale" | "parent" | "parentInverse") => true,
            _ => false,
        })
    })
}
