//! The command bridge (`docs/design.md` section 7.2): writes a whole scene in a replica's
//! creating transaction ([`write_scene`]), or only what a command changed ([`write_changes`]),
//! the changed meshes and images as new blobs.

use std::collections::{BTreeMap, BTreeSet};

use kition_kit::kdm::{ObjectId, Tx};
use kition_kit::session::{BlobRef, sha256_hex};
use kition_scene_model::animation::{self, Channel, Timeline};
use kition_scene_model::glam::Mat4;
use kition_scene_model::nla::ActionBlend;
use kition_scene_model::{Id, Image, Material, Object, ObjectData, Scene};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::held::{self, Mint, Shape};
use crate::{KdmResult, hair_blob, kdm_err, mesh_blob, root_id};

/// A blob a write references, for the caller to put in the store with the transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewBlob {
    pub hash: String,
    pub bytes: Vec<u8>,
}

/// What a write needs written beside it.
#[derive(Debug, Clone, Default)]
pub struct Written {
    pub blobs: Vec<NewBlob>,
}

fn to_json(value: &impl Serialize) -> KdmResult<Value> {
    serde_json::to_value(value).map_err(kdm_err)
}

fn blob(bytes: Vec<u8>, mime: &str, written: &mut Written) -> KdmResult<Value> {
    let hash = sha256_hex(&bytes);
    let reference = BlobRef { hash: hash.clone(), mime: mime.to_owned(), size: bytes.len() as u64 };
    written.blobs.push(NewBlob { hash, bytes });
    to_json(&reference)
}

/// The scene as the replica stores it: `after` with every animated property put back to the value
/// `stored` holds for it (the view shows animated properties at the local frame, and an edit of
/// one without a key stays local), and the timeline at frame 1.
pub fn stored_form(after: &Scene, stored: &Scene) -> Scene {
    let mut out = after.clone();
    for (target, property, index, path) in after.animated_paths() {
        if let Some(value) = animation::read_at(stored, target, property, index, &path) {
            animation::write_at(&mut out, target, property, index, &path, value);
        }
    }
    out.animation.timeline.frame = Timeline::default().frame;
    out
}

fn timeline_json(timeline: &Timeline) -> KdmResult<Value> {
    let mut value = to_json(timeline)?;
    if let Some(fields) = value.as_object_mut() {
        fields.remove("frame");
    }
    Ok(value)
}

/// The scene's channels by target, each target's in scene order: one pass over the channels, so
/// a write never scans every channel once per object.
fn channels_by_target(scene: &Scene) -> BTreeMap<Id, Vec<&Channel>> {
    let mut by_target: BTreeMap<Id, Vec<&Channel>> = BTreeMap::new();
    for channel in &scene.animation.channels {
        by_target.entry(channel.target).or_default().push(channel);
    }
    by_target
}

/// The channels of `target` in a [`channels_by_target`] map (none when it has none).
fn channels_of<'a>(by_target: &'a BTreeMap<Id, Vec<&'a Channel>>, target: Id) -> &'a [&'a Channel] {
    by_target.get(&target).map_or(&[], Vec::as_slice)
}

#[cfg(test)]
thread_local! {
    /// How many objects' and materials' registers were worked out on this thread (tests count
    /// the work a write does).
    static FIELDS_BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn count_fields_built() {
    #[cfg(test)]
    FIELDS_BUILT.with(|built| built.set(built.get() + 1));
}

/// The lists an object holds records in, with their items' shapes.
pub const OBJECT_LISTS: [(&str, &Shape); 6] = [
    ("modifiers", &held::MODIFIER),
    ("constraints", &held::CONSTRAINT),
    ("particles", &held::PARTICLE_SYSTEM),
    ("animation", &held::CHANNEL),
    ("nla", &held::NLA_TRACK),
    ("bones", &held::BONE),
];

/// The lists a material holds records in.
pub const MATERIAL_LISTS: [(&str, &Shape); 1] = [("animation", &held::CHANNEL)];

/// The lists the scene holds records in (besides objects, materials, images and node groups).
pub const SCENE_LISTS: [(&str, &Shape); 2] = [("actions", &held::ACTION), ("keyingSets", &held::KEYING_SET)];

/// Each item of `values` as JSON.
fn items<T: Serialize>(values: &[T]) -> KdmResult<Vec<Value>> {
    values.iter().map(to_json).collect()
}

/// Channels as their records hold them: the target is the record holding them.
fn channel_items(channels: &[&Channel]) -> KdmResult<Vec<Value>> {
    channels
        .iter()
        .map(|channel| {
            let mut value = to_json(channel)?;
            if let Some(members) = value.as_object_mut() {
                members.remove("target");
            }
            Ok(value)
        })
        .collect()
}

/// An object's registers, but its mesh, hair and text: name, kind, data (without an armature's
/// bones and a text's body, which are records and text of their own), transform, links, physics,
/// the blend of its own keys under the NLA, tweak mode.
fn object_fields(object: &Object) -> KdmResult<BTreeMap<&'static str, Value>> {
    count_fields_built();
    let data = match &object.data {
        ObjectData::Mesh(_) => json!({ "type": "mesh" }),
        other => {
            let mut data = to_json(other)?;
            if let Some(members) = data.as_object_mut() {
                match other {
                    ObjectData::Armature(_) => members.remove("bones"),
                    ObjectData::Text(_) => members.remove("body"),
                    _ => None,
                };
            }
            data
        }
    };
    let mut fields = BTreeMap::new();
    fields.insert("key", Value::from(object.id));
    fields.insert("name", Value::String(object.name.clone()));
    fields.insert("kind", Value::String(object.data.kind().to_owned()));
    fields.insert("data", data);
    fields.insert("location", to_json(&object.location)?);
    fields.insert("rotation", to_json(&object.rotation)?);
    fields.insert("scale", to_json(&object.scale)?);
    fields.insert("parent", object.parent.map_or(Value::Null, Value::from));
    fields.insert("parentInverse", if object.parent_inverse == Mat4::IDENTITY { Value::Null } else { to_json(&object.parent_inverse)? });
    fields.insert("hidden", Value::Bool(object.hidden));
    fields.insert("material", object.material.map_or(Value::Null, Value::from));
    fields.insert(
        "physics",
        match &object.physics {
            Some(physics) if !physics.is_empty() => to_json(physics)?,
            _ => Value::Null,
        },
    );
    fields.insert("nlaAction", if object.nla.action == ActionBlend::default() { Value::Null } else { to_json(&object.nla.action)? });
    // Tweak mode (with the keys it put aside) is its own register: a peer's edit of the tracks
    // must never take it away from the keys it goes with.
    fields.insert("tweak", object.nla.tweak.as_ref().map_or(Ok(Value::Null), to_json)?);
    Ok(fields)
}

/// An object's held lists, in [`OBJECT_LISTS`] order: its modifiers, constraints, particle
/// systems, keyframe channels (`channels`, its own), NLA tracks, and an armature's bones.
fn object_lists(object: &Object, channels: &[&Channel]) -> KdmResult<[Vec<Value>; 6]> {
    let bones = match &object.data {
        ObjectData::Armature(armature) => items(&armature.bones)?,
        _ => Vec::new(),
    };
    Ok([
        items(&object.modifiers)?,
        items(&object.constraints)?,
        items(&object.particles)?,
        channel_items(channels)?,
        items(&object.nla.tracks)?,
        bones,
    ])
}

/// A text object's body; empty for other objects.
fn body(object: &Object) -> &str {
    match &object.data {
        ObjectData::Text(text) => &text.body,
        _ => "",
    }
}

/// Makes the `text` field of `id`, which holds `old`, hold `new`: what lies between their common
/// start and end is deleted and typed, so a peer's typing elsewhere in it stays.
fn write_text(tx: &mut Tx<'_>, id: &ObjectId, old: &str, new: &str) -> KdmResult<()> {
    let old: Vec<char> = old.chars().collect();
    let new: Vec<char> = new.chars().collect();
    let length = tx.text_len(id, "text")?;
    if length != old.len() {
        if length > 0 {
            tx.text_delete(id, "text", 0, length)?;
        }
        if !new.is_empty() {
            tx.text_insert(id, "text", 0, &new.iter().collect::<String>())?;
        }
        return Ok(());
    }
    let start = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let room = old.len().min(new.len()) - start;
    let end = old.iter().rev().zip(new.iter().rev()).take(room).take_while(|(a, b)| a == b).count();
    let removed = old.len() - start - end;
    if removed > 0 {
        tx.text_delete(id, "text", start, removed)?;
    }
    let typed: String = new.get(start..new.len() - end).unwrap_or_default().iter().collect();
    if !typed.is_empty() {
        tx.text_insert(id, "text", start, &typed)?;
    }
    Ok(())
}

fn material_fields(material: &Material) -> KdmResult<BTreeMap<&'static str, Value>> {
    count_fields_built();
    let mut data = to_json(material)?;
    if let Some(map) = data.as_object_mut() {
        map.remove("id");
        map.remove("name");
    }
    let mut fields = BTreeMap::new();
    fields.insert("key", Value::from(material.id));
    fields.insert("name", Value::String(material.name.clone()));
    fields.insert("data", data);
    Ok(fields)
}

fn image_fields(image: &Image, written: &mut Written) -> KdmResult<Map<String, Value>> {
    let mut fields = Map::new();
    fields.insert("hash".into(), Value::String(image.hash.clone()));
    fields.insert("name".into(), Value::String(image.name.clone()));
    fields.insert("format".into(), to_json(&image.format)?);
    fields.insert("bytes".into(), blob(image.bytes.as_ref().clone(), image.format.mime(), written)?);
    Ok(fields)
}

fn to_map(fields: BTreeMap<&'static str, Value>) -> Map<String, Value> {
    fields.into_iter().filter(|(_, value)| !value.is_null()).map(|(key, value)| (key.to_owned(), value)).collect()
}

/// The object's groomed hair by system name (empty when no system is groomed).
fn groomed(object: &Object) -> Vec<(&str, &[Vec<kition_scene_model::glam::Vec3>])> {
    object.particles.iter().filter(|system| !system.groomed.is_empty()).map(|system| (system.name.as_str(), system.groomed.as_slice())).collect()
}

/// The object's `hair` register: a blob of its groomed strands, or null when none are groomed.
fn hair_value(object: &Object, written: &mut Written) -> KdmResult<Value> {
    let groomed = groomed(object);
    if groomed.is_empty() {
        return Ok(Value::Null);
    }
    blob(hair_blob::encode(&groomed).map_err(kdm_err)?, hair_blob::MIME, written)
}

/// Creates an object's record with what it holds, at the end of the scene's `objects`.
fn add_object(tx: &mut Tx<'_>, object: &Object, channels: &[&Channel], written: &mut Written) -> KdmResult<ObjectId> {
    let mut fields = to_map(object_fields(object)?);
    if let ObjectData::Mesh(mesh) = &object.data {
        fields.insert("mesh".into(), blob(mesh_blob::encode(mesh), mesh_blob::MIME, written)?);
    }
    let hair = hair_value(object, written)?;
    if !hair.is_null() {
        fields.insert("hair".into(), hair);
    }
    let id = tx.create("Object", fields)?;
    for ((field, shape), values) in OBJECT_LISTS.iter().zip(object_lists(object, channels)?) {
        held::write(tx, &id, field, shape, &values, Mint::Random)?;
    }
    write_text(tx, &id, "", body(object))?;
    let length = tx.list_items(&root_id(), "objects")?.len();
    tx.list_insert(&root_id(), "objects", length, &id)?;
    Ok(id)
}

/// Creates a material's record with its channels, at the end of the scene's `materials`.
fn add_material(tx: &mut Tx<'_>, material: &Material, channels: &[&Channel]) -> KdmResult<ObjectId> {
    let id = tx.create("Material", to_map(material_fields(material)?))?;
    held::write(tx, &id, "animation", &held::CHANNEL, &channel_items(channels)?, Mint::Random)?;
    let length = tx.list_items(&root_id(), "materials")?.len();
    tx.list_insert(&root_id(), "materials", length, &id)?;
    Ok(id)
}

/// The scene's registers: world, render settings, timeline (without the frame), camera, physics
/// world, and the active keying set.
fn root_fields(scene: &Scene) -> KdmResult<[(&'static str, Value); 6]> {
    Ok([
        ("world", to_json(&scene.world)?),
        ("render", to_json(&scene.render)?),
        ("timeline", timeline_json(&scene.animation.timeline)?),
        ("camera", scene.camera.map_or(Value::Null, Value::from)),
        ("physics", if scene.physics.is_default() { Value::Null } else { to_json(&scene.physics)? }),
        ("activeKeyingSet", scene.animation.keying_sets.active.as_ref().map_or(Ok(Value::Null), to_json)?),
    ])
}

/// The scene's held lists, in [`SCENE_LISTS`] order.
fn root_lists(scene: &Scene) -> KdmResult<[Vec<Value>; 2]> {
    Ok([items(&scene.animation.actions)?, items(&scene.animation.keying_sets.sets)?])
}

/// Writes `scene` as a new replica's content (the creating transaction).
pub fn write_scene(tx: &mut Tx<'_>, scene: &Scene) -> KdmResult<Written> {
    let mut written = Written::default();
    let root: Map<String, Value> =
        root_fields(scene)?.into_iter().filter(|(_, value)| !value.is_null()).map(|(name, value)| (name.to_owned(), value)).collect();
    let root_id = tx.create_with_id("Scene", &root_id(), root)?;
    for ((field, shape), values) in SCENE_LISTS.iter().zip(root_lists(scene)?) {
        held::write(tx, &root_id, field, shape, &values, Mint::Random)?;
    }
    let channels = channels_by_target(scene);
    for object in &scene.objects {
        add_object(tx, object, channels_of(&channels, object.id), &mut written)?;
    }
    for material in &scene.materials {
        add_material(tx, material, channels_of(&channels, material.id))?;
    }
    for (index, image) in scene.images.iter().enumerate() {
        let id = tx.create("Image", image_fields(image, &mut written)?)?;
        tx.list_insert(&root_id, "images", index, &id)?;
    }
    crate::nodes::write_groups(tx, scene)?;
    Ok(written)
}

/// The records of list `field` by their key (`key` for objects and materials, `hash` for
/// images); a key listed twice maps to its first record.
fn records(tx: &Tx<'_>, field: &str, key: &str) -> KdmResult<(Vec<ObjectId>, BTreeMap<String, ObjectId>)> {
    let ids = tx.list_items(&root_id(), field)?;
    let mut by_key = BTreeMap::new();
    for id in &ids {
        let value = tx.get(id, key)?;
        let name = match value {
            Value::String(text) => text,
            other => other.to_string(),
        };
        by_key.entry(name).or_insert_with(|| id.clone());
    }
    Ok((ids, by_key))
}

fn set_changed(tx: &mut Tx<'_>, id: &ObjectId, before: &BTreeMap<&'static str, Value>, after: &BTreeMap<&'static str, Value>) -> KdmResult<()> {
    for (name, value) in after {
        if before.get(name) != Some(value) {
            tx.set(id, name, value.clone())?;
        }
    }
    Ok(())
}

/// Writes the held lists of `id` (`fields`) that differ between `before` and `after`.
fn write_lists<const N: usize>(
    tx: &mut Tx<'_>,
    id: &ObjectId,
    fields: &[(&str, &Shape); N],
    before: [Vec<Value>; N],
    after: [Vec<Value>; N],
) -> KdmResult<()> {
    for (((field, shape), old), new) in fields.iter().zip(before).zip(after) {
        if old != new {
            held::write(tx, id, field, shape, &new, Mint::Random)?;
        }
    }
    Ok(())
}

/// Deletes a record of the scene's list `field` with what it holds (`fields`).
fn delete_record(tx: &mut Tx<'_>, field: &str, id: &ObjectId, fields: &[(&str, &Shape)]) -> KdmResult<()> {
    held::delete_all(tx, id, fields)?;
    held::remove_entry(tx, &root_id(), field, id)?;
    tx.delete(id)
}

/// Writes what changed between `before` (the scene the replica holds) and `after` (both in
/// stored form, [`stored_form`]): new, changed, deleted, and reordered objects, materials, and
/// images, what they hold, and the root's settings and lists. `issued` is the actor's `keys` entry
/// and the next id its range mints, written when it moved.
pub fn write_changes(tx: &mut Tx<'_>, before: &Scene, after: &Scene, issued: Option<(&str, Id)>) -> KdmResult<Written> {
    let mut written = Written::default();
    let root = root_id();

    // The root's settings and lists.
    for ((name, old), (_, new)) in root_fields(before)?.into_iter().zip(root_fields(after)?) {
        if old != new {
            tx.set(&root, name, new)?;
        }
    }
    if before.animation.actions != after.animation.actions || before.animation.keying_sets.sets != after.animation.keying_sets.sets {
        write_lists(tx, &root, &SCENE_LISTS, root_lists(before)?, root_lists(after)?)?;
    }
    if let Some((key, next)) = issued {
        let current = tx.map_get(&root, "keys", key)?.as_u64();
        if current.is_none_or(|current| current < u64::from(next)) {
            tx.map_set(&root, "keys", key, crate::ids::issued(next))?;
        }
    }

    // Objects. One that is equal and keeps equal keys has equal registers: it is skipped
    // without working them out, so a command's write costs what it changed, not the scene's
    // size (every key of every object, serialised twice, made baking keys quadratic).
    let old_channels = channels_by_target(before);
    let new_channels = channels_by_target(after);
    let (_, by_key) = records(tx, "objects", "key")?;
    let old_objects: BTreeMap<Id, &Object> = before.objects.iter().map(|object| (object.id, object)).collect();
    let new_keys: BTreeSet<Id> = after.objects.iter().map(|object| object.id).collect();
    for object in old_objects.values().filter(|object| !new_keys.contains(&object.id)) {
        if let Some(id) = by_key.get(&object.id.to_string()) {
            delete_record(tx, "objects", id, &OBJECT_LISTS)?;
        }
    }
    let mut wanted = Vec::with_capacity(after.objects.len());
    for object in &after.objects {
        let id = match (old_objects.get(&object.id), by_key.get(&object.id.to_string())) {
            (Some(old), Some(id)) if *old == object && channels_of(&old_channels, object.id) == channels_of(&new_channels, object.id) => id.clone(),
            (Some(old), Some(id)) => {
                let (old_keys, new_keys) = (channels_of(&old_channels, object.id), channels_of(&new_channels, object.id));
                set_changed(tx, id, &object_fields(old)?, &object_fields(object)?)?;
                write_lists(tx, id, &OBJECT_LISTS, object_lists(old, old_keys)?, object_lists(object, new_keys)?)?;
                if body(old) != body(object) {
                    write_text(tx, id, body(old), body(object))?;
                }
                if let (ObjectData::Mesh(old_mesh), ObjectData::Mesh(new_mesh)) = (&old.data, &object.data) {
                    if old_mesh != new_mesh {
                        let reference = blob(mesh_blob::encode(new_mesh), mesh_blob::MIME, &mut written)?;
                        tx.set(id, "mesh", reference)?;
                    }
                } else if let ObjectData::Mesh(new_mesh) = &object.data {
                    let reference = blob(mesh_blob::encode(new_mesh), mesh_blob::MIME, &mut written)?;
                    tx.set(id, "mesh", reference)?;
                } else if matches!(old.data, ObjectData::Mesh(_)) {
                    tx.set(id, "mesh", Value::Null)?;
                }
                if groomed(old) != groomed(object) {
                    let hair = hair_value(object, &mut written)?;
                    tx.set(id, "hair", hair)?;
                }
                id.clone()
            }
            _ => add_object(tx, object, channels_of(&new_channels, object.id), &mut written)?,
        };
        wanted.push(id);
    }
    held::order(tx, &root, "objects", &wanted)?;

    // Materials.
    let (_, by_key) = records(tx, "materials", "key")?;
    let old_materials: BTreeMap<Id, &Material> = before.materials.iter().map(|material| (material.id, material)).collect();
    let new_keys: BTreeSet<Id> = after.materials.iter().map(|material| material.id).collect();
    for material in old_materials.values().filter(|material| !new_keys.contains(&material.id)) {
        if let Some(id) = by_key.get(&material.id.to_string()) {
            delete_record(tx, "materials", id, &MATERIAL_LISTS)?;
        }
    }
    let mut wanted = Vec::with_capacity(after.materials.len());
    for material in &after.materials {
        let id = match (old_materials.get(&material.id), by_key.get(&material.id.to_string())) {
            (Some(old), Some(id)) if *old == material && channels_of(&old_channels, material.id) == channels_of(&new_channels, material.id) => {
                id.clone()
            }
            (Some(old), Some(id)) => {
                let (old_keys, new_keys) = (channels_of(&old_channels, material.id), channels_of(&new_channels, material.id));
                set_changed(tx, id, &material_fields(old)?, &material_fields(material)?)?;
                write_lists(tx, id, &MATERIAL_LISTS, [channel_items(old_keys)?], [channel_items(new_keys)?])?;
                id.clone()
            }
            _ => add_material(tx, material, channels_of(&new_channels, material.id))?,
        };
        wanted.push(id);
    }
    held::order(tx, &root, "materials", &wanted)?;

    // Images: by hash, never edited in place.
    let (_, by_hash) = records(tx, "images", "hash")?;
    let new_hashes: BTreeSet<&str> = after.images.iter().map(|image| image.hash.as_str()).collect();
    for image in &before.images {
        if !new_hashes.contains(image.hash.as_str())
            && let Some(id) = by_hash.get(&image.hash)
        {
            delete_record(tx, "images", id, &[])?;
        }
    }
    let mut wanted = Vec::with_capacity(after.images.len());
    for image in &after.images {
        let id = match by_hash.get(&image.hash) {
            Some(id) => {
                if before.images.iter().find(|old| old.hash == image.hash).is_some_and(|old| old.name != image.name) {
                    tx.set(id, "name", Value::String(image.name.clone()))?;
                }
                id.clone()
            }
            None => {
                let id = tx.create("Image", image_fields(image, &mut written)?)?;
                let length = tx.list_items(&root, "images")?.len();
                tx.list_insert(&root, "images", length, &id)?;
                id
            }
        };
        wanted.push(id);
    }
    held::order(tx, &root, "images", &wanted)?;
    crate::nodes::write_group_changes(tx, before, after)?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

    use kition_kit::kdm::{ActorId, DocId, Origin, Replica};
    use kition_scene_model::animation::Property;
    use kition_scene_model::glam::Vec3;
    use kition_scene_model::{Id, Primitive, Scene};

    use super::{FIELDS_BUILT, stored_form, write_changes, write_scene};
    use crate::schema;

    fn built() -> usize {
        FIELDS_BUILT.with(std::cell::Cell::get)
    }

    /// `objects` cubes, each keyed on location and scale on `frames` frames.
    fn keyed_scene(objects: usize, frames: usize) -> (Scene, Vec<Id>) {
        let mut scene = Scene::starter();
        let mut ids = Vec::new();
        for index in 0..objects {
            let mesh = Primitive::Cube { size: 0.2 }.build().unwrap();
            let id = scene.add_object(&format!("C{index}"), kition_scene_model::ObjectData::Mesh(mesh));
            for frame in 0..frames {
                scene.object_mut(id).unwrap().location = Vec3::new(frame as f32, index as f32, 0.0);
                scene.insert_key(id, Property::Location, frame as f32).unwrap();
                scene.insert_key(id, Property::Scale, frame as f32).unwrap();
            }
            ids.push(id);
        }
        (scene, ids)
    }

    /// The registers a write works out to key one object once more, in a scene of `objects`
    /// objects keyed on `frames` frames each.
    fn fields_built_to_key_one(objects: usize, frames: usize) -> usize {
        let (scene, ids) = keyed_scene(objects, frames);
        let mut replica =
            Replica::create_with(schema(), DocId::new(), ActorId::random(), &Origin::system("create"), |tx| write_scene(tx, &scene)).unwrap().0;
        let before = stored_form(&scene, &scene);
        let mut after = before.clone();
        let target = ids[objects / 2];
        let frame = frames as f32 + 1.0;
        after.object_mut(target).unwrap().location = Vec3::new(9.0, 9.0, 9.0);
        after.insert_key(target, Property::Location, frame).unwrap();
        after.insert_key(target, Property::Scale, frame).unwrap();
        let after = stored_form(&after, &before);
        let start = built();
        let version = replica.version();
        replica.transact(&Origin::person("p"), |tx| write_changes(tx, &before, &after, None)).unwrap();
        assert_ne!(replica.version(), version, "the key was written");
        built() - start
    }

    /// Inserting a key works out the registers of the keyed object only, however many keys
    /// the other objects hold: baking keys object by object was quadratic in the scene's keys.
    #[test]
    fn a_key_insert_works_out_only_the_keyed_object() {
        let small = fields_built_to_key_one(8, 2);
        let large = fields_built_to_key_one(64, 200);
        assert_eq!(small, 2, "the keyed object's registers, before and after");
        assert_eq!(large, small, "more keys on other objects add no work");
    }
}
