//! The scene a replica holds (`docs/design.md` section 7.1), as stored: animated properties at
//! their stored values and the timeline at frame 1 (the current frame is each person's own).

use std::collections::BTreeSet;
use std::sync::Arc;

use kition_kit::kdm::{ObjectId, Replica};
use kition_kit::session::{BlobRef, BlobSource};
use kition_scene_model::animation::{Animation, Channel, Timeline};
use kition_scene_model::glam::{Mat4, Vec3};
use kition_scene_model::particles::ParticleSystem;
use kition_scene_model::{Id, Image, ImageFormat, Material, Mesh, Modifier, Object, ObjectData, RenderSettings, Scene, World};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::mesh_blob::MeshCache;
use crate::root_id;
use crate::{hair_blob, held};

fn field(replica: &Replica, id: &ObjectId, name: &str) -> Value {
    replica.get(id, name).unwrap_or(Value::Null)
}

fn parse<T: DeserializeOwned>(value: Value, what: &str) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| format!("damaged: {what}: {error}"))
}

fn parse_or<T: DeserializeOwned + Default>(value: Value, what: &str) -> Result<T, String> {
    if value.is_null() { Ok(T::default()) } else { parse(value, what) }
}

/// The items of `owner`'s held list `field`, as JSON.
fn list(replica: &Replica, owner: &ObjectId, field: &str, shape: &held::Shape, name: &str) -> Result<Vec<Value>, String> {
    held::read(replica, owner, field, shape).map_err(|error| format!("damaged: {name}'s {field}: {error}"))
}

/// The keyframe channels `owner` (whose engine id is `target`) holds.
fn channels_of(replica: &Replica, owner: &ObjectId, target: Id, name: &str) -> Result<Vec<Channel>, String> {
    let mut items = list(replica, owner, "animation", &held::CHANNEL, name)?;
    for item in &mut items {
        if let Some(members) = item.as_object_mut() {
            members.insert("target".into(), Value::from(target));
        }
    }
    parse(Value::Array(items), &format!("{name}'s animation"))
}

fn key(value: &Value) -> Option<Id> {
    value.as_u64().and_then(|key| u32::try_from(key).ok())
}

fn blob_ref(value: &Value) -> Option<BlobRef> {
    serde_json::from_value(value.clone()).ok()
}

/// Every blob hash the replica's records name.
pub fn blob_hashes(replica: &Replica) -> BTreeSet<String> {
    let root = root_id();
    let mut out = BTreeSet::new();
    for id in replica.list(&root, "objects").unwrap_or_default() {
        for name in ["mesh", "hair"] {
            if let Some(blob) = blob_ref(&field(replica, &id, name)) {
                out.insert(blob.hash);
            }
        }
    }
    for id in replica.list(&root, "images").unwrap_or_default() {
        if let Some(blob) = blob_ref(&field(replica, &id, "bytes")) {
            out.insert(blob.hash);
        }
    }
    out
}

/// Each object's KDM id by its key (presence shares selections as KDM ids).
pub fn object_ids(replica: &Replica) -> std::collections::BTreeMap<Id, ObjectId> {
    let root = root_id();
    replica
        .list(&root, "objects")
        .unwrap_or_default()
        .into_iter()
        .filter_map(|id| key(&field(replica, &id, "key")).map(|object| (object, id)))
        .collect()
}

/// The scene the replica holds. A mesh or image whose blob has not arrived yet reads as empty;
/// it fills in when its blob comes (the document reads the replica again then).
pub fn read_scene(replica: &Replica, blobs: &dyn BlobSource, meshes: &mut MeshCache) -> Result<Scene, String> {
    let root = root_id();
    if !replica.exists(&root) {
        return Err("damaged: the replica has no scene".into());
    }
    let mut scene = Scene::empty();
    scene.world = parse_or::<World>(field(replica, &root, "world"), "the world")?;
    scene.render = parse_or::<RenderSettings>(field(replica, &root, "render"), "the render settings")?;
    scene.camera = key(&field(replica, &root, "camera"));
    scene.physics = parse_or::<kition_scene_model::physics::PhysicsWorld>(field(replica, &root, "physics"), "the physics world")?;
    // The current frame is each person's own: the replica keeps the range and the rate.
    let timeline = match field(replica, &root, "timeline") {
        Value::Object(mut fields) => {
            fields.insert("frame".into(), Value::from(Timeline::default().frame));
            parse::<Timeline>(Value::Object(fields), "the timeline")?
        }
        _ => Timeline::default(),
    };
    let mut channels: Vec<Channel> = Vec::new();
    let mut named = BTreeSet::new();

    for id in replica.list(&root, "objects").map_err(|error| error.to_string())? {
        let Some(object_key) = key(&field(replica, &id, "key")) else { continue };
        let name = field(replica, &id, "name").as_str().unwrap_or("Object").to_owned();
        let data = match field(replica, &id, "kind").as_str() {
            Some("mesh") => {
                let mesh = match blob_ref(&field(replica, &id, "mesh")) {
                    Some(blob) => {
                        named.insert(blob.hash.clone());
                        match blobs.get(&blob.hash) {
                            Some(bytes) => meshes.get(&blob.hash, &bytes).map_err(|error| error.to_string())?.as_ref().clone(),
                            None => Mesh::default(),
                        }
                    }
                    None => Mesh::default(),
                };
                ObjectData::Mesh(mesh)
            }
            kind => {
                let mut data = field(replica, &id, "data");
                if let Some(members) = data.as_object_mut() {
                    match kind {
                        Some("armature") => {
                            members.insert("bones".into(), Value::Array(list(replica, &id, "bones", &held::BONE, &name)?));
                        }
                        Some("text") => {
                            members.insert("body".into(), Value::String(replica.text(&id, "text").unwrap_or_default()));
                        }
                        _ => {}
                    }
                }
                parse::<ObjectData>(data, &format!("{name}'s data"))?
            }
        };
        let mut object = Object::new(object_key, name.clone(), data);
        object.location = parse_or::<Vec3>(field(replica, &id, "location"), "a location")?;
        object.rotation = parse_or::<Vec3>(field(replica, &id, "rotation"), "a rotation")?;
        object.scale = match field(replica, &id, "scale") {
            Value::Null => Vec3::ONE,
            value => parse::<Vec3>(value, "a scale")?,
        };
        object.parent = key(&field(replica, &id, "parent"));
        object.parent_inverse = match field(replica, &id, "parentInverse") {
            Value::Null => Mat4::IDENTITY,
            value => parse::<Mat4>(value, "a parent inverse")?,
        };
        object.hidden = field(replica, &id, "hidden").as_bool().unwrap_or(false);
        object.material = key(&field(replica, &id, "material"));
        object.modifiers =
            parse::<Vec<Modifier>>(Value::Array(list(replica, &id, "modifiers", &held::MODIFIER, &name)?), &format!("{name}'s modifiers"))?;
        object.constraints = parse::<Vec<kition_scene_model::Constraint>>(
            Value::Array(list(replica, &id, "constraints", &held::CONSTRAINT, &name)?),
            &format!("{name}'s constraints"),
        )?;
        object.physics = match field(replica, &id, "physics") {
            Value::Null => None,
            value => Some(parse::<kition_scene_model::physics::ObjectPhysics>(value, &format!("{name}'s physics"))?),
        };
        if matches!(object.data, ObjectData::Mesh(_)) {
            object.particles = parse::<Vec<ParticleSystem>>(
                Value::Array(list(replica, &id, "particles", &held::PARTICLE_SYSTEM, &name)?),
                &format!("{name}'s particle systems"),
            )?;
            // Groomed hair whose blob has not arrived yet shows as generated until it does.
            if let Some(bytes) = blob_ref(&field(replica, &id, "hair")).and_then(|blob| blobs.get(&blob.hash)) {
                hair_blob::apply(&mut object, &bytes).map_err(|error| format!("damaged: {name}'s hair: {error}"))?;
            }
        }
        channels.extend(channels_of(replica, &id, object_key, &name)?);
        let mut nla = serde_json::Map::new();
        nla.insert("tracks".into(), Value::Array(list(replica, &id, "nla", &held::NLA_TRACK, &name)?));
        let action = field(replica, &id, "nlaAction");
        if !action.is_null() {
            nla.insert("action".into(), action);
        }
        let nla = Value::Object(nla);
        kition_scene_model::nla::Nla::check_json_structure(&nla).map_err(|reason| format!("damaged: {name}'s NLA: {reason}"))?;
        object.nla = parse::<kition_scene_model::nla::Nla>(nla, &format!("{name}'s NLA"))?;
        object.nla.tweak = match field(replica, &id, "tweak") {
            Value::Null => None,
            value => Some(parse::<kition_scene_model::nla::Tweak>(value, &format!("{name}'s tweak mode"))?),
        };
        scene.objects.push(object);
    }

    for id in replica.list(&root, "materials").map_err(|error| error.to_string())? {
        let Some(material_key) = key(&field(replica, &id, "key")) else { continue };
        let name = field(replica, &id, "name").as_str().unwrap_or("Material").to_owned();
        let mut material: Material = match field(replica, &id, "data") {
            Value::Object(mut fields) => {
                fields.insert("id".into(), Value::from(material_key));
                fields.insert("name".into(), Value::String(name.clone()));
                parse(Value::Object(fields), &format!("material {name}"))?
            }
            _ => Material::new(material_key, name.clone()),
        };
        material.id = material_key;
        channels.extend(channels_of(replica, &id, material_key, &name)?);
        scene.materials.push(material);
    }

    for id in replica.list(&root, "images").map_err(|error| error.to_string())? {
        let Some(hash) = field(replica, &id, "hash").as_str().map(str::to_owned) else { continue };
        let name = field(replica, &id, "name").as_str().unwrap_or("Image").to_owned();
        let format: ImageFormat = parse(field(replica, &id, "format"), &format!("image {name}'s format"))?;
        let bytes =
            blob_ref(&field(replica, &id, "bytes")).and_then(|blob| blobs.get(&blob.hash)).map(|bytes| Arc::new(bytes.to_vec())).unwrap_or_default();
        scene.images.push(Image { hash, name, format, bytes });
    }
    meshes.retain(&named);
    // Colliders and instanced objects a peer deleted at the same time let go (repair writes it).
    let ids: Vec<Id> = scene.objects.iter().map(|object| object.id).collect();
    for object in &mut scene.objects {
        let own = object.id;
        for system in &mut object.particles {
            system.colliders.retain(|collider| *collider != own && ids.contains(collider));
            system.instance_object = system.instance_object.filter(|source| *source != own && ids.contains(source));
        }
    }
    // Rigid body constraints naming an object a peer deleted (or their own) let go of it too.
    for object in &mut scene.objects {
        let own = object.id;
        let Some(constraint) = object.physics.as_mut().and_then(|physics| physics.rigid_constraint.as_mut()) else { continue };
        for slot in [&mut constraint.object1, &mut constraint.object2] {
            *slot = slot.filter(|id| *id != own && ids.contains(id));
        }
    }
    scene.node_groups = crate::nodes::read_groups(replica)?;

    let actions = parse::<Vec<kition_scene_model::animation::Action>>(
        Value::Array(list(replica, &root, "actions", &held::ACTION, "the scene")?),
        "the actions",
    )?;
    let keying_sets = kition_scene_model::animation::KeyingSets {
        sets: parse(Value::Array(list(replica, &root, "keyingSets", &held::KEYING_SET, "the scene")?), "the keying sets")?,
        active: match field(replica, &root, "activeKeyingSet") {
            Value::Null => None,
            value => Some(parse(value, "the active keying set")?),
        },
    };
    scene.animation = Animation { timeline, channels, actions, keying_sets };
    let highest = scene
        .objects
        .iter()
        .map(|object| object.id)
        .chain(scene.materials.iter().map(|material| material.id))
        .chain(crate::nodes::ids(&scene))
        .max()
        .unwrap_or(0);
    scene.next_id = highest.saturating_add(1).max(1);
    Ok(scene)
}
