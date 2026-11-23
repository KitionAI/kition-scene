//! The post-merge repair (`docs/design.md` section 7.3). Every replica runs it after a merge, an
//! undo, or a redo; it reads only the state, so it writes the same for the same state, and it
//! writes only what is wrong.

use std::collections::{BTreeMap, BTreeSet};

use kition_kit::kdm::{ObjectId, Tx};
use serde_json::Value;

use crate::held::{self, Mint};
use crate::{KdmResult, root_id};

/// The list fields of the root and the type each lists (the root's held lists are
/// [`held::repair`]'s).
const LISTS: [(&str, &str); 4] = [("objects", "Object"), ("materials", "Material"), ("images", "Image"), ("nodeGroups", "NodeGroup")];

/// The material fields that name an image by hash.
const IMAGE_SLOTS: [&str; 3] = ["baseColorTexture", "metallicRoughnessTexture", "normalTexture"];

fn key_of(tx: &Tx<'_>, id: &ObjectId) -> Option<u64> {
    tx.get(id, "key").ok().and_then(|value| value.as_u64())
}

/// The objects one constraint reads: its target and pole, and an Armature constraint's bones.
fn aimed_at(constraint: &Value) -> Vec<u64> {
    let mut out: Vec<u64> =
        ["target", "pole"].iter().filter_map(|slot| constraint.get(*slot).and_then(|target| target.get("object")).and_then(Value::as_u64)).collect();
    out.extend(
        constraint.get("targets").and_then(Value::as_array).into_iter().flatten().filter_map(|entry| entry.get("object").and_then(Value::as_u64)),
    );
    out
}

/// Clears a constraint's target and pole when they name no object, and drops an Armature
/// constraint's bones of objects that are gone.
fn forget_missing(constraint: &mut Value, keys: &BTreeMap<u64, ObjectId>) {
    if let Some(entries) = constraint.get_mut("targets").and_then(Value::as_array_mut) {
        entries.retain(|entry| entry.get("object").and_then(Value::as_u64).is_none_or(|object| keys.contains_key(&object)));
    }
    for slot in ["target", "pole"] {
        let object = constraint.get(slot).and_then(|target| target.get("object")).and_then(Value::as_u64);
        if let (Some(object), Some(map)) = (object, constraint.as_object_mut())
            && !keys.contains_key(&object)
        {
            map.insert(slot.into(), Value::Null);
        }
    }
}

/// An object's constraint records and its bones', each with the record holding it in its
/// `constraints` list and its JSON.
fn constraint_records(tx: &Tx<'_>, id: &ObjectId) -> KdmResult<Vec<(ObjectId, ObjectId, Value)>> {
    let mut out: Vec<(ObjectId, ObjectId, Value)> =
        held::entries(tx, id, "constraints", &held::CONSTRAINT)?.into_iter().map(|(record, value)| (id.clone(), record, value)).collect();
    for bone in tx.list_items(id, "bones")? {
        if tx.exists(&bone) {
            out.extend(held::entries(tx, &bone, "constraints", &held::CONSTRAINT)?.into_iter().map(|(record, value)| (bone.clone(), record, value)));
        }
    }
    Ok(out)
}

fn break_constraint_cycles(tx: &mut Tx<'_>, keys: &BTreeMap<u64, ObjectId>, parents: &BTreeMap<u64, u64>) -> KdmResult<bool> {
    let mut wrote = false;
    // Targets that a peer deleted go (the constraint stays, with no target).
    for id in keys.values() {
        for (_, record, value) in constraint_records(tx, id)? {
            let mut fixed = value.clone();
            forget_missing(&mut fixed, keys);
            if fixed != value {
                held::update(tx, &record, &held::CONSTRAINT, &fixed, Mint::Derived)?;
                wrote = true;
            }
        }
    }
    // Each pass breaks one cycle; there are at most as many as objects.
    for _ in 0..=keys.len() {
        let mut edges: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
        for (key, id) in keys {
            let entry = edges.entry(*key).or_default();
            if let Some(parent) = parents.get(key) {
                entry.insert(*parent);
            }
            for (_, _, constraint) in constraint_records(tx, id)? {
                for target in aimed_at(&constraint) {
                    if target != *key && keys.contains_key(&target) {
                        entry.insert(target);
                    }
                }
            }
        }
        let Some(cycle) = find_cycle(&edges) else { break };
        let Some(highest) = cycle.iter().max().copied() else { break };
        let position = cycle.iter().position(|key| *key == highest).unwrap_or(0);
        let next = cycle.get((position + 1) % cycle.len()).copied().unwrap_or(highest);
        let Some(id) = keys.get(&highest) else { break };
        // The object's own constraints on the next object go; when the edge is a bone's, that
        // bone's constraints on it go.
        let records = constraint_records(tx, id)?;
        let own: Vec<&(ObjectId, ObjectId, Value)> =
            records.iter().filter(|(owner, _, value)| owner == id && aimed_at(value).contains(&next)).collect();
        let aiming: Vec<&(ObjectId, ObjectId, Value)> =
            if own.is_empty() { records.iter().filter(|(_, _, value)| aimed_at(value).contains(&next)).collect() } else { own };
        if aiming.is_empty() {
            break;
        }
        for (owner, record, _) in aiming {
            held::remove_entry(tx, owner, "constraints", record)?;
            held::delete(tx, record, &held::CONSTRAINT)?;
        }
        wrote = true;
    }
    Ok(wrote)
}

/// A cycle in a dependency graph, in order, if there is one.
fn find_cycle(edges: &BTreeMap<u64, BTreeSet<u64>>) -> Option<Vec<u64>> {
    let mut state: BTreeMap<u64, u8> = BTreeMap::new();
    for start in edges.keys().copied() {
        if state.get(&start).copied().unwrap_or(0) != 0 {
            continue;
        }
        let mut path: Vec<u64> = vec![start];
        let mut pending: Vec<Vec<u64>> = vec![edges.get(&start).map(|set| set.iter().copied().collect()).unwrap_or_default()];
        state.insert(start, 1);
        while let Some(next) = pending.last_mut().map(Vec::pop) {
            match next {
                Some(node) => match state.get(&node).copied().unwrap_or(0) {
                    1 => {
                        let position = path.iter().position(|key| *key == node)?;
                        return Some(path.get(position..)?.to_vec());
                    }
                    0 => {
                        state.insert(node, 1);
                        path.push(node);
                        pending.push(edges.get(&node).map(|set| set.iter().copied().collect()).unwrap_or_default());
                    }
                    _ => {}
                },
                None => {
                    if let Some(done) = path.pop() {
                        state.insert(done, 2);
                    }
                    pending.pop();
                }
            }
        }
    }
    None
}

/// Drops NLA strips whose action is gone, and ends tweak mode that no longer fits the tracks
/// (see [`repair`]). It reads the `nla` tracks, the `tweak` register, and the `animation`
/// channels; tweak mode and the keys it put aside always come from the same peer.
fn repair_nla(tx: &mut Tx<'_>, keys: &BTreeMap<u64, ObjectId>) -> KdmResult<bool> {
    use kition_scene_model::Channel;
    use kition_scene_model::animation::in_nla;
    use kition_scene_model::nla::{Nla, Tweak};
    let root = root_id();
    let names: BTreeSet<String> =
        tx.list_items(&root, "actions")?.iter().filter_map(|id| tx.get(id, "name").ok()?.as_str().map(str::to_owned)).collect();
    let names: BTreeSet<&str> = names.iter().map(String::as_str).collect();
    let mut wrote = false;
    for (key, id) in keys {
        // Strips playing an action that is gone go.
        let value = serde_json::json!({ "tracks": held::read(tx, id, "nla", &held::NLA_TRACK)? });
        Nla::check_json_structure(&value).map_err(crate::kdm_err)?;
        let mut nla = serde_json::from_value::<Nla>(value).unwrap_or_default();
        nla.check_structure().map_err(crate::kdm_err)?;
        if nla.retain_actions(&names) {
            let tracks = nla.tracks.iter().map(serde_json::to_value).collect::<Result<Vec<Value>, _>>().map_err(crate::kdm_err)?;
            held::write(tx, id, "nla", &held::NLA_TRACK, &tracks, Mint::Derived)?;
            wrote = true;
        }
        // Tweak mode holds while a strip still plays its action (found again if a peer moved
        // it); otherwise it ends and the keys put aside come back as the object's own.
        let value = tx.get(id, "tweak")?;
        if value.is_null() {
            continue;
        }
        let Ok(mut tweak) = serde_json::from_value::<Tweak>(value) else { continue };
        let plays = |track: usize, strip: usize| {
            nla.tracks.get(track).and_then(|found| found.strips.get(strip)).is_some_and(|found| found.action == tweak.action)
        };
        if names.contains(tweak.action.as_str()) && plays(tweak.track, tweak.strip) {
            continue;
        }
        let found = names
            .contains(tweak.action.as_str())
            .then(|| {
                nla.tracks
                    .iter()
                    .enumerate()
                    .flat_map(|(track, found)| found.strips.iter().enumerate().map(move |(strip, played)| (track, strip, played)))
                    .find(|(_, _, played)| played.action == tweak.action)
                    .map(|(track, strip, _)| (track, strip))
            })
            .flatten();
        match found {
            Some((track, strip)) => {
                tweak.track = track;
                tweak.strip = strip;
                tx.set(id, "tweak", serde_json::to_value(&tweak).map_err(crate::kdm_err)?)?;
            }
            None => {
                let target = u32::try_from(*key).unwrap_or_default();
                let mut items = held::read(tx, id, "animation", &held::CHANNEL)?;
                for item in &mut items {
                    if let Some(members) = item.as_object_mut() {
                        members.insert("target".into(), Value::from(target));
                    }
                }
                let mut channels: Vec<Channel> = serde_json::from_value(Value::Array(items)).unwrap_or_default();
                channels.retain(|channel| !in_nla(channel));
                channels.extend(tweak.stash.into_iter().map(|channel| Channel {
                    target,
                    property: channel.property,
                    index: channel.index,
                    path: channel.path,
                    keys: channel.keys,
                    driver: None,
                    modifiers: channel.modifiers,
                }));
                let mut items = Vec::with_capacity(channels.len());
                for channel in &channels {
                    let mut item = serde_json::to_value(channel).map_err(crate::kdm_err)?;
                    if let Some(members) = item.as_object_mut() {
                        members.remove("target");
                    }
                    items.push(item);
                }
                held::write(tx, id, "animation", &held::CHANNEL, &items, Mint::Derived)?;
                tx.set(id, "tweak", Value::Null)?;
            }
        }
        wrote = true;
    }
    Ok(wrote)
}

/// Repairs the replica's state; answers whether it wrote anything.
pub fn repair(tx: &mut Tx<'_>) -> KdmResult<bool> {
    let root = root_id();
    if !tx.exists(&root) {
        return Ok(false);
    }
    let mut wrote = false;

    // Lists: entries of deleted records go, a record listed twice keeps its first place, and
    // records no list names go.
    for (field, ty) in LISTS {
        let items = tx.list_items(&root, field)?;
        let mut seen = BTreeSet::new();
        for index in (0..items.len()).rev() {
            let Some(id) = items.get(index) else { continue };
            let first = items.iter().position(|other| other == id) == Some(index);
            if !tx.exists(id) || !first {
                tx.list_remove(&root, field, index)?;
                wrote = true;
            } else {
                seen.insert(id.clone());
            }
        }
        for id in tx.objects_of_type(ty) {
            if !seen.contains(&id) {
                tx.delete(&id)?;
                wrote = true;
            }
        }
    }

    // Held lists: their entries, the records they hold, and names and channels two peers added
    // at once.
    wrote |= held::repair(tx)?;

    // Keys (the records' tool ids, one id space) found twice: the first record in list order keeps
    // its key, the others get one from a range no actor mints from, derived from their record id.
    // A record with no key (a batch created it: `key` is read-only to batches) gets one the same
    // way. Node groups and nodes take part with their keys as they are; one without a key gets
    // one (links and modifiers name nodes and groups by key, so a repeated one is left alone).
    let mut used: BTreeSet<u64> = BTreeSet::new();
    let groups: Vec<ObjectId> = tx.list_items(&root, "nodeGroups")?;
    let mut nodes: Vec<ObjectId> = Vec::new();
    for group in &groups {
        nodes.extend(tx.list_items(group, "nodes")?);
    }
    for id in groups.iter().chain(&nodes) {
        if let Some(key) = key_of(tx, id) {
            used.insert(key);
        }
    }
    let mut keyless: Vec<ObjectId> = groups.into_iter().chain(nodes).filter(|id| key_of(tx, id).is_none()).collect();
    for field in ["objects", "materials"] {
        for id in tx.list_items(&root, field)? {
            match key_of(tx, &id) {
                Some(key) if used.insert(key) => continue,
                Some(_) => {}
                None => {
                    keyless.push(id);
                    continue;
                }
            }
            keyless.push(id);
        }
    }
    for id in keyless {
        let floor = u64::from(crate::ids::REKEY_FLOOR);
        let mut fresh = floor + held::fnv(id.as_str()) % 0x0FFF_FFFF;
        while used.contains(&fresh) {
            fresh = floor + (fresh + 1 - floor) % 0x0FFF_FFFF;
        }
        used.insert(fresh);
        tx.set(&id, "key", Value::from(fresh))?;
        wrote = true;
    }

    // Node groups: their lists, links that no longer fit, and what their nodes name.
    wrote |= crate::nodes::repair(tx)?;

    // Objects: parents, materials, and the camera that are gone are cleared; a parent cycle is
    // broken at its highest key.
    let objects = tx.list_items(&root, "objects")?;
    let keys: BTreeMap<u64, ObjectId> = objects.iter().filter_map(|id| key_of(tx, id).map(|key| (key, id.clone()))).collect();
    let materials: BTreeSet<u64> = tx.list_items(&root, "materials")?.iter().filter_map(|id| key_of(tx, id)).collect();
    let mut parents: BTreeMap<u64, u64> = BTreeMap::new();
    for (key, id) in &keys {
        if let Some(parent) = tx.get(id, "parent")?.as_u64() {
            if parent == *key || !keys.contains_key(&parent) {
                tx.set(id, "parent", Value::Null)?;
                tx.set(id, "parentInverse", Value::Null)?;
                wrote = true;
            } else {
                parents.insert(*key, parent);
            }
        }
        if let Some(material) = tx.get(id, "material")?.as_u64()
            && !materials.contains(&material)
        {
            tx.set(id, "material", Value::Null)?;
            wrote = true;
        }
    }
    for start in keys.keys() {
        let mut path = vec![*start];
        let mut at = *start;
        while let Some(next) = parents.get(&at).copied() {
            if let Some(position) = path.iter().position(|key| *key == next) {
                let cycle = path.get(position..).unwrap_or_default();
                if let Some(highest) = cycle.iter().max().copied()
                    && let Some(id) = keys.get(&highest)
                {
                    tx.set(id, "parent", Value::Null)?;
                    tx.set(id, "parentInverse", Value::Null)?;
                    parents.remove(&highest);
                    wrote = true;
                }
                break;
            }
            path.push(next);
            at = next;
        }
    }
    // Constraints: a cycle through targets (two people adding constraints at once, each fine
    // alone) is broken at its highest key, whose constraints on the next object in the cycle go.
    wrote |= break_constraint_cycles(tx, &keys, &parents)?;
    let cameras: Vec<u64> = objects
        .iter()
        .filter(|id| tx.get(id, "kind").ok().and_then(|kind| kind.as_str().map(str::to_owned)).as_deref() == Some("camera"))
        .filter_map(|id| key_of(tx, id))
        .collect();
    if let Some(camera) = tx.get(&root, "camera")?.as_u64()
        && !cameras.contains(&camera)
    {
        tx.set(&root, "camera", cameras.first().map_or(Value::Null, |first| Value::from(*first)))?;
        wrote = true;
    }

    // Cameras: a focus object that is gone lets go (the camera focuses at its distance again).
    for id in &objects {
        let Value::Object(mut data) = tx.get(id, "data")? else { continue };
        let Some(Value::Object(dof)) = data.get_mut("dof") else { continue };
        if let Some(focus) = dof.get("focusObject").and_then(Value::as_u64)
            && !keys.contains_key(&focus)
        {
            dof.remove("focusObject");
            tx.set(id, "data", Value::Object(data))?;
            wrote = true;
        }
    }

    // Armatures: a bone drawn by an object that is gone gets its octahedron back.
    for (key, id) in &keys {
        for (record, mut bone) in held::entries(tx, id, "bones", &held::BONE)? {
            let drawn_by = bone.get("shape").and_then(|shape| shape.get("object")).and_then(Value::as_u64);
            if let Some(map) = bone.as_object_mut()
                && map.contains_key("shape")
                && drawn_by.is_none_or(|object| object == *key || !keys.contains_key(&object))
            {
                map.remove("shape");
                held::update(tx, &record, &held::BONE, &bone, Mint::Derived)?;
                wrote = true;
            }
        }
    }

    // Modifiers (a boolean's or shrinkwrap's mesh, the lattice, the armature), a curve's bevel
    // and taper objects and the curve a text follows let go of an object that is gone, as
    // deleting it does.
    for id in &objects {
        let gone = |value: Option<&Value>| value.is_some_and(|value| value.as_u64().is_some_and(|other| !keys.contains_key(&other)));
        for (record, mut modifier) in held::entries(tx, id, "modifiers", &held::MODIFIER)? {
            if let Some(map) = modifier.as_object_mut()
                && gone(map.get("object"))
            {
                map.remove("object");
                held::update(tx, &record, &held::MODIFIER, &modifier, Mint::Derived)?;
                wrote = true;
            }
        }
        if let Value::Object(mut data) = tx.get(id, "data")? {
            let mut changed = false;
            for slot in ["bevelObject", "taperObject", "followCurve"] {
                if gone(data.get(slot)) {
                    data.remove(slot);
                    changed = true;
                }
            }
            if changed {
                tx.set(id, "data", Value::Object(data))?;
                wrote = true;
            }
        }
    }

    // Particle systems let go of deleted colliders, instances and boid targets, and self references.
    for (key, id) in &keys {
        for (record, mut system) in held::entries(tx, id, "particles", &held::PARTICLE_SYSTEM)? {
            let Some(map) = system.as_object_mut() else { continue };
            let mut changed = false;
            let alive = |value: &Value| value.as_u64().is_some_and(|other| other != *key && keys.contains_key(&other));
            if let Some(Value::Array(colliders)) = map.get_mut("colliders") {
                let before = colliders.len();
                colliders.retain(alive);
                changed |= colliders.len() != before;
            }
            if let Some(rules) = map.get_mut("boids").and_then(|boids| boids.get_mut("rules")).and_then(Value::as_array_mut) {
                for rule in rules.iter_mut().filter_map(Value::as_object_mut) {
                    if rule.get("target").is_some_and(|target| !alive(target)) {
                        rule.remove("target");
                        changed = true;
                    }
                }
            }
            if map.get("instanceObject").is_some_and(|source| !alive(source)) {
                map.remove("instanceObject");
                changed = true;
            }
            if changed {
                held::update(tx, &record, &held::PARTICLE_SYSTEM, &system, Mint::Derived)?;
                wrote = true;
            }
        }
    }

    // Rigid body constraints: objects that are gone (or the constraint's own) let go.
    for (key, id) in &keys {
        let Value::Object(mut physics) = tx.get(id, "physics")? else { continue };
        let Some(Value::Object(constraint)) = physics.get_mut("rigidConstraint") else { continue };
        let mut changed = false;
        for slot in ["object1", "object2"] {
            if constraint.get(slot).is_some_and(|value| value.as_u64().is_none_or(|other| other == *key || !keys.contains_key(&other))) {
                constraint.remove(slot);
                changed = true;
            }
        }
        if changed {
            tx.set(id, "physics", Value::Object(physics))?;
            wrote = true;
        }
    }

    // NLA: strips playing an action that is gone (a peer removed or renamed it) go; tweak mode
    // on such a strip ends, the keys put aside coming back as the object's own.
    wrote |= repair_nla(tx, &keys)?;

    // Materials: image slots naming an image no record holds are cleared.
    let images: BTreeSet<String> =
        tx.list_items(&root, "images")?.iter().filter_map(|id| tx.get(id, "hash").ok()?.as_str().map(str::to_owned)).collect();
    for id in tx.list_items(&root, "materials")? {
        let Value::Object(mut data) = tx.get(&id, "data")? else { continue };
        let mut changed = false;
        for slot in IMAGE_SLOTS {
            if let Some(Value::String(hash)) = data.get(slot)
                && !images.contains(hash)
            {
                data.insert(slot.to_owned(), Value::Null);
                changed = true;
            }
        }
        if changed {
            tx.set(&id, "data", Value::Object(data))?;
            wrote = true;
        }
    }
    // The world: an image no record holds is let go (the world colour shows again).
    if let Value::Object(mut world) = tx.get(&root, "world")?
        && let Some(Value::String(hash)) = world.get("image")
        && !images.contains(hash)
    {
        world.remove("image");
        tx.set(&root, "world", Value::Object(world))?;
        wrote = true;
    }
    Ok(wrote)
}
