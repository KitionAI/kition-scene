//! Held objects (`docs/design.md` section 7.1): the ordered stacks of a record (an object's
//! modifiers, constraints, particle systems, keyframe channels, NLA tracks and bones, a material's
//! channels, the scene's actions and keying sets) are `list` fields whose items are records of
//! their own, so two people adding to one stack, or editing two items of it, both keep their
//! edits.
//!
//! Each item is the engine's JSON for it, split by a [`Shape`]: some members are registers of
//! their own (`name`, a channel's `keys`), the rest is one `data` register, and members holding
//! arrays of items are lists of records again (a track's strips, a bone's constraints). Writing
//! compares an item with the registers its record holds and sets only what differs; items are
//! matched to records by their [`Shape::key`] (with a count for repeats), so an edit of one item
//! never rewrites its neighbours.

use std::collections::{BTreeMap, BTreeSet};

use kition_kit::kdm::{KdmError, ObjectId, Replica, Tx};
use serde_json::{Map, Value};

use crate::{KdmResult, kdm_err};

/// How an item's JSON is stored in its record.
#[derive(Debug)]
pub struct Shape {
    /// The record's type.
    pub ty: &'static str,
    /// Members stored as registers of the same name.
    pub registers: &'static [&'static str],
    /// The register holding every other member, as one JSON object (`None`: there are none).
    pub rest: Option<&'static str>,
    /// Members holding arrays of items: list fields of the same name.
    pub lists: &'static [(&'static str, &'static Shape)],
    /// The members that tell an item from its siblings.
    pub key: &'static [&'static str],
}

/// A keyframe channel (an object's or a material's `animation`, an action's `channels`): the
/// keys are one register, as are the driver and the F-curve modifiers.
pub static CHANNEL: Shape = Shape {
    ty: "Channel",
    registers: &["property", "index", "path", "keys", "driver", "modifiers"],
    rest: None,
    lists: &[],
    key: &["property", "index", "path"],
};

pub static MODIFIER: Shape = Shape { ty: "Modifier", registers: &["name"], rest: Some("data"), lists: &[], key: &["name"] };

pub static CONSTRAINT: Shape = Shape { ty: "Constraint", registers: &["name"], rest: Some("data"), lists: &[], key: &["name"] };

pub static PARTICLE_SYSTEM: Shape = Shape { ty: "ParticleSystem", registers: &["name"], rest: Some("data"), lists: &[], key: &["name"] };

/// An NLA strip; a meta strip's children stay in its `data`.
pub static NLA_STRIP: Shape = Shape { ty: "NlaStrip", registers: &["name"], rest: Some("data"), lists: &[], key: &["name"] };

pub static NLA_TRACK: Shape =
    Shape { ty: "NlaTrack", registers: &["name", "muted", "solo"], rest: None, lists: &[("strips", &NLA_STRIP)], key: &["name"] };

pub static ACTION: Shape = Shape { ty: "Action", registers: &["name"], rest: None, lists: &[("channels", &CHANNEL)], key: &["name"] };

pub static KEYING_SET: Shape = Shape { ty: "KeyingSet", registers: &["name", "paths"], rest: None, lists: &[], key: &["name"] };

pub static BONE: Shape = Shape { ty: "Bone", registers: &["name"], rest: Some("data"), lists: &[("constraints", &CONSTRAINT)], key: &["name"] };

/// Every list of held records: its owner's type, its field, and the items' shape. Owners come
/// before the records they hold.
pub static LISTS: [(&str, &str, &Shape); 12] = [
    ("Scene", "actions", &ACTION),
    ("Scene", "keyingSets", &KEYING_SET),
    ("Object", "modifiers", &MODIFIER),
    ("Object", "constraints", &CONSTRAINT),
    ("Object", "particles", &PARTICLE_SYSTEM),
    ("Object", "animation", &CHANNEL),
    ("Object", "nla", &NLA_TRACK),
    ("Object", "bones", &BONE),
    ("Material", "animation", &CHANNEL),
    ("Action", "channels", &CHANNEL),
    ("NlaTrack", "strips", &NLA_STRIP),
    ("Bone", "constraints", &CONSTRAINT),
];

/// The most items one held list reads (a hostile replica's list is cut there).
pub const MAX_ITEMS: usize = 100_000;

/// What reads records: a replica, or a transaction.
pub trait Records {
    fn register(&self, id: &ObjectId, field: &str) -> Value;
    fn items(&self, id: &ObjectId, field: &str) -> KdmResult<Vec<ObjectId>>;
    fn alive(&self, id: &ObjectId) -> bool;
}

impl Records for Replica {
    fn register(&self, id: &ObjectId, field: &str) -> Value {
        self.get(id, field).unwrap_or(Value::Null)
    }
    fn items(&self, id: &ObjectId, field: &str) -> KdmResult<Vec<ObjectId>> {
        self.list(id, field)
    }
    fn alive(&self, id: &ObjectId) -> bool {
        self.exists(id)
    }
}

impl Records for Tx<'_> {
    fn register(&self, id: &ObjectId, field: &str) -> Value {
        self.get(id, field).unwrap_or(Value::Null)
    }
    fn items(&self, id: &ObjectId, field: &str) -> KdmResult<Vec<ObjectId>> {
        self.list_items(id, field)
    }
    fn alive(&self, id: &ObjectId) -> bool {
        self.exists(id)
    }
}

/// An item's registers (null members left out) and its held lists, from its JSON.
/// An item's registers and its held lists (each its field, its items' shape and its items).
type Split<'a> = (Map<String, Value>, Vec<(&'static str, &'a Shape, Vec<Value>)>);

fn split<'a>(value: &Value, shape: &'a Shape) -> KdmResult<Split<'a>> {
    let Value::Object(members) = value else { return Err(kdm_err(format!("a {} is not a JSON object", shape.ty))) };
    let mut members = members.clone();
    let mut lists = Vec::with_capacity(shape.lists.len());
    for (name, inner) in shape.lists {
        let items = match members.remove(*name) {
            Some(Value::Array(items)) => items,
            None | Some(Value::Null) => Vec::new(),
            Some(_) => return Err(kdm_err(format!("a {}'s `{name}` is not an array", shape.ty))),
        };
        lists.push((*name, *inner, items));
    }
    let mut fields = Map::new();
    for name in shape.registers {
        if let Some(value) = members.remove(*name)
            && !value.is_null()
        {
            fields.insert((*name).to_owned(), value);
        }
    }
    match shape.rest {
        Some(rest) => {
            fields.insert(rest.to_owned(), Value::Object(members));
        }
        None => {
            if let Some(name) = members.keys().next() {
                return Err(kdm_err(format!("a {} has a member `{name}` its record has no field for", shape.ty)));
            }
        }
    }
    Ok((fields, lists))
}

/// An item's key among its siblings, from its JSON.
fn key_of(value: &Value, shape: &Shape) -> String {
    shape.key.iter().map(|name| value.get(*name).map_or_else(String::new, Value::to_string)).collect::<Vec<_>>().join("\u{1f}")
}

/// A record's JSON without its held lists.
fn flat(records: &dyn Records, id: &ObjectId, shape: &Shape) -> Value {
    let mut members = match shape.rest.map(|rest| records.register(id, rest)) {
        Some(Value::Object(members)) => members,
        _ => Map::new(),
    };
    for name in shape.registers {
        let value = records.register(id, name);
        if !value.is_null() {
            members.insert((*name).to_owned(), value);
        }
    }
    Value::Object(members)
}

/// The record's key among its siblings.
fn record_key(records: &dyn Records, id: &ObjectId, shape: &Shape) -> String {
    key_of(&flat(records, id, shape), shape)
}

/// A held record's JSON, its held lists in it.
pub fn item(records: &dyn Records, id: &ObjectId, shape: &Shape) -> KdmResult<Value> {
    let mut value = flat(records, id, shape);
    if let Value::Object(members) = &mut value {
        for (name, inner) in shape.lists {
            members.insert((*name).to_owned(), Value::Array(read(records, id, name, inner)?));
        }
    }
    Ok(value)
}

/// The engine's JSON for items of a held list as the object model gives them (each its record's
/// fields: `name`, `data`, its own held lists), or as the engine writes them; `None` when an item
/// is neither (an id).
pub fn engine_items(value: &Value, shape: &Shape) -> Option<Vec<Value>> {
    match value {
        Value::Null => Some(Vec::new()),
        Value::Array(items) => items.iter().map(|item| engine_item(item, shape)).collect(),
        _ => None,
    }
}

fn engine_item(item: &Value, shape: &Shape) -> Option<Value> {
    let Value::Object(fields) = item else { return None };
    let mut members = match shape.rest.and_then(|rest| fields.get(rest)) {
        Some(Value::Object(rest)) => rest.clone(),
        Some(_) => return None,
        None => fields.iter().filter(|(name, _)| !name.starts_with('$')).map(|(name, value)| (name.clone(), value.clone())).collect(),
    };
    for name in shape.registers {
        if let Some(value) = fields.get(*name).filter(|value| !value.is_null()) {
            members.insert((*name).to_owned(), value.clone());
        }
    }
    for (name, inner) in shape.lists {
        if let Some(value) = fields.get(*name) {
            members.insert((*name).to_owned(), Value::Array(engine_items(value, inner)?));
        }
    }
    Some(Value::Object(members))
}

/// The items of `owner`'s list `field`, in order, as JSON (records that are gone are skipped).
pub fn read(records: &dyn Records, owner: &ObjectId, field: &str, shape: &Shape) -> KdmResult<Vec<Value>> {
    Ok(entries(records, owner, field, shape)?.into_iter().map(|(_, value)| value).collect())
}

/// The records of `owner`'s list `field` with their JSON, in order.
pub fn entries(records: &dyn Records, owner: &ObjectId, field: &str, shape: &Shape) -> KdmResult<Vec<(ObjectId, Value)>> {
    let ids = records.items(owner, field)?;
    if ids.len() > MAX_ITEMS {
        return Err(kdm_err(format!("damaged: a list of {} records holds {} of them", shape.ty, ids.len())));
    }
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if records.alive(&id) {
            let value = item(records, &id, shape)?;
            out.push((id, value));
        }
    }
    Ok(out)
}

/// How new records get their ids: random (a person's command), or derived from where they go,
/// so every replica's repair makes the same record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mint {
    Random,
    Derived,
}

/// A stable number for a text (FNV-1a).
pub(crate) fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3))
}

fn create(tx: &mut Tx<'_>, shape: &Shape, value: &Value, mint: Mint, seed: &str) -> KdmResult<ObjectId> {
    let (fields, lists) = split(value, shape)?;
    let id = match mint {
        Mint::Random => tx.create(shape.ty, fields)?,
        Mint::Derived => {
            let base = format!("held-{:016x}", fnv(seed));
            let mut attempt = 0u32;
            loop {
                let candidate = if attempt == 0 { ObjectId::from_string(&base) } else { ObjectId::from_string(format!("{base}-{attempt}")) };
                match tx.create_with_id(shape.ty, &candidate, fields.clone()) {
                    Ok(id) => break id,
                    Err(KdmError::InvalidValue { field, .. }) if field == "$id" && attempt < 1024 => attempt += 1,
                    Err(error) => return Err(error),
                }
            }
        }
    };
    for (name, inner, items) in lists {
        for (index, item) in items.iter().enumerate() {
            let child = create(tx, inner, item, mint, &format!("{seed}/{name}/{index}/{}", key_of(item, inner)))?;
            tx.list_insert(&id, name, index, &child)?;
        }
    }
    Ok(id)
}

/// Deletes a held record and every record it holds.
pub fn delete(tx: &mut Tx<'_>, id: &ObjectId, shape: &Shape) -> KdmResult<()> {
    for (name, inner) in shape.lists {
        for child in tx.list_items(id, name)? {
            if tx.exists(&child) {
                delete(tx, &child, inner)?;
            }
        }
    }
    if tx.exists(id) {
        tx.delete(id)?;
    }
    Ok(())
}

/// Deletes every record `owner` holds in its lists `fields` (before `owner` itself goes).
pub fn delete_all(tx: &mut Tx<'_>, owner: &ObjectId, fields: &[(&str, &Shape)]) -> KdmResult<()> {
    for (field, shape) in fields {
        for id in tx.list_items(owner, field)? {
            if tx.exists(&id) {
                delete(tx, &id, shape)?;
            }
        }
    }
    Ok(())
}

/// Makes record `id` hold `value`: sets the registers that differ and writes its held lists.
pub fn update(tx: &mut Tx<'_>, id: &ObjectId, shape: &Shape, value: &Value, mint: Mint) -> KdmResult<()> {
    let (fields, lists) = split(value, shape)?;
    for name in shape.registers.iter().copied().chain(shape.rest) {
        let wanted = fields.get(name).cloned().unwrap_or(Value::Null);
        if tx.get(id, name)? != wanted {
            tx.set(id, name, wanted)?;
        }
    }
    for (name, inner, items) in lists {
        write(tx, id, name, inner, &items, mint)?;
    }
    Ok(())
}

/// Makes `owner`'s list `field` hold `items`, in order: items match records by key (the n-th
/// item of a key the n-th record of it), then the items left match the records left in order (an
/// item renamed in place stays its record); matched records change only what differs, new items
/// get records, and records no item matches go.
pub fn write(tx: &mut Tx<'_>, owner: &ObjectId, field: &str, shape: &Shape, items: &[Value], mint: Mint) -> KdmResult<()> {
    let records: Vec<ObjectId> = tx.list_items(owner, field)?.into_iter().filter(|id| tx.exists(id)).collect();
    let mut by_key: BTreeMap<String, Vec<ObjectId>> = BTreeMap::new();
    for id in &records {
        by_key.entry(record_key(tx, id, shape)).or_default().push(id.clone());
    }
    // By key first.
    let mut taken: BTreeMap<String, usize> = BTreeMap::new();
    let mut matched: Vec<Option<ObjectId>> = Vec::with_capacity(items.len());
    let mut kept = BTreeSet::new();
    let mut keys = Vec::with_capacity(items.len());
    for item in items {
        let key = key_of(item, shape);
        let count = taken.entry(key.clone()).or_insert(0);
        let found = by_key.get(&key).and_then(|ids| ids.get(*count)).cloned();
        *count += 1;
        if let Some(id) = &found {
            kept.insert(id.clone());
        }
        matched.push(found);
        keys.push((key, *taken.get(&key_of(item, shape)).unwrap_or(&0)));
    }
    // Then the rest in order.
    let spare: Vec<ObjectId> = records.iter().filter(|id| !kept.contains(*id)).cloned().collect();
    let mut spare = spare.into_iter();
    for slot in matched.iter_mut().filter(|slot| slot.is_none()) {
        let Some(id) = spare.next() else { break };
        kept.insert(id.clone());
        *slot = Some(id);
    }
    let mut wanted = Vec::with_capacity(items.len());
    for ((item, found), (key, count)) in items.iter().zip(matched).zip(keys) {
        let id = match found {
            Some(id) => {
                update(tx, &id, shape, item, mint)?;
                id
            }
            None => {
                let id = create(tx, shape, item, mint, &format!("{owner}/{field}/{key}/{count}"))?;
                let length = tx.list_items(owner, field)?.len();
                tx.list_insert(owner, field, length, &id)?;
                id
            }
        };
        wanted.push(id);
    }
    for id in records.iter().filter(|id| !kept.contains(*id)) {
        remove_entry(tx, owner, field, id)?;
        delete(tx, id, shape)?;
    }
    order(tx, owner, field, &wanted)
}

/// Takes every entry of `id` out of `owner`'s list `field`.
pub fn remove_entry(tx: &mut Tx<'_>, owner: &ObjectId, field: &str, id: &ObjectId) -> KdmResult<()> {
    while let Some(index) = tx.list_items(owner, field)?.iter().position(|other| other == id) {
        tx.list_remove(owner, field, index)?;
    }
    Ok(())
}

/// Puts `owner`'s list `field` in the order of `wanted`, moving as few entries as it takes.
pub fn order(tx: &mut Tx<'_>, owner: &ObjectId, field: &str, wanted: &[ObjectId]) -> KdmResult<()> {
    let mut current = tx.list_items(owner, field)?;
    for (index, id) in wanted.iter().enumerate() {
        if current.get(index) == Some(id) {
            continue;
        }
        let Some(from) = current.iter().position(|other| other == id) else { continue };
        tx.list_move(owner, field, from, index)?;
        let moved = current.remove(from);
        current.insert(index.min(current.len()), moved);
    }
    Ok(())
}

/// The post-merge repair of held lists; answers whether it wrote anything. Entries of records that
/// are gone, of another type, or listed before (in this list or, in id order of owners, another)
/// go; records no list holds go (with what they hold); a name two peers gave two items of one list
/// at once is made unique on the later item; two channels of one component (two peers keying it at
/// once) become the first, with the keys of both (the first's key wins on a frame both keyed).
pub fn repair(tx: &mut Tx<'_>) -> KdmResult<bool> {
    let mut wrote = false;
    // Records whose owner a pass deletes are found unheld by the next one.
    for _ in 0..4 {
        let mut held: BTreeSet<ObjectId> = BTreeSet::new();
        for (owner_type, field, shape) in &LISTS {
            let mut owners = tx.objects_of_type(owner_type);
            owners.sort();
            for owner in owners {
                let items = tx.list_items(&owner, field)?;
                for index in (0..items.len()).rev() {
                    let Some(id) = items.get(index) else { continue };
                    let first = items.iter().position(|other| other == id) == Some(index);
                    let fits = tx.exists(id) && tx.object_type(id).ok().as_deref() == Some(shape.ty);
                    if !fits || !first || held.contains(id) {
                        tx.list_remove(&owner, field, index)?;
                        wrote = true;
                    }
                }
                held.extend(tx.list_items(&owner, field)?);
            }
        }
        let mut deleted = false;
        let types: BTreeSet<&str> = LISTS.iter().map(|(_, _, shape)| shape.ty).collect();
        for ty in types {
            for id in tx.objects_of_type(ty) {
                if !held.contains(&id) {
                    tx.delete(&id)?;
                    deleted = true;
                }
            }
        }
        wrote |= deleted;
        if !deleted {
            break;
        }
    }
    for (owner_type, field, shape) in &LISTS {
        let mut owners = tx.objects_of_type(owner_type);
        owners.sort();
        for owner in owners {
            if shape.ty == CHANNEL.ty {
                wrote |= merge_channels(tx, &owner, field)?;
            } else if shape.key == ["name"] {
                wrote |= unique_names(tx, &owner, field, shape)?;
            }
        }
    }
    Ok(wrote)
}

/// Later items of a name taken in `owner`'s list `field` get a name of their own.
fn unique_names(tx: &mut Tx<'_>, owner: &ObjectId, field: &str, shape: &Shape) -> KdmResult<bool> {
    let items = tx.list_items(owner, field)?;
    let names: Vec<String> = items.iter().map(|id| flat(tx, id, shape).get("name").and_then(Value::as_str).unwrap_or_default().to_owned()).collect();
    let mut taken: BTreeSet<String> = BTreeSet::new();
    let mut wrote = false;
    for (index, id) in items.iter().enumerate() {
        let Some(name) = names.get(index) else { continue };
        if taken.insert(name.clone()) {
            continue;
        }
        let fresh =
            kition_scene_model::scene::unique_name(name, |candidate| taken.contains(candidate) || names.iter().any(|other| other == candidate));
        taken.insert(fresh.clone());
        tx.set(id, "name", Value::String(fresh))?;
        wrote = true;
    }
    Ok(wrote)
}

/// Channels of one component in `owner`'s list `field` become the first of them, with every key
/// (on a frame several key, the first channel's).
fn merge_channels(tx: &mut Tx<'_>, owner: &ObjectId, field: &str) -> KdmResult<bool> {
    let items = tx.list_items(owner, field)?;
    let mut first: BTreeMap<String, ObjectId> = BTreeMap::new();
    let mut wrote = false;
    for id in items {
        let key = record_key(tx, &id, &CHANNEL);
        let Some(keep) = first.get(&key).cloned() else {
            first.insert(key, id);
            continue;
        };
        let mut keys = match tx.get(&keep, "keys")? {
            Value::Array(keys) => keys,
            _ => Vec::new(),
        };
        let frame = |key: &Value| key.get("frame").and_then(Value::as_f64);
        if let Value::Array(more) = tx.get(&id, "keys")? {
            for key in more {
                let Some(at) = frame(&key) else { continue };
                if !keys.iter().any(|kept| frame(kept) == Some(at)) {
                    keys.push(key);
                }
            }
        }
        keys.sort_by(|a, b| frame(a).unwrap_or(0.0).total_cmp(&frame(b).unwrap_or(0.0)));
        let keys = Value::Array(keys);
        if tx.get(&keep, "keys")? != keys {
            tx.set(&keep, "keys", keys)?;
        }
        remove_entry(tx, owner, field, &id)?;
        delete(tx, &id, &CHANNEL)?;
        wrote = true;
    }
    Ok(wrote)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use serde_json::json;

    use super::*;

    #[test]
    fn split_keeps_every_member() {
        let bone = json!({ "name": "Arm", "head": [0, 0, 0], "constraints": [{ "name": "IK", "type": "ik" }] });
        let (fields, lists) = split(&bone, &BONE).unwrap();
        assert_eq!(fields.get("name"), Some(&json!("Arm")));
        assert_eq!(fields.get("data"), Some(&json!({ "head": [0, 0, 0] })));
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].2, vec![json!({ "name": "IK", "type": "ik" })]);
    }

    #[test]
    fn a_member_without_a_field_is_refused() {
        assert!(split(&json!({ "name": "Walk", "channels": [], "extra": 1 }), &ACTION).is_err());
        assert_eq!(key_of(&json!({ "property": "location", "index": 2 }), &CHANNEL), "\"location\"\u{1f}2\u{1f}");
    }
}
