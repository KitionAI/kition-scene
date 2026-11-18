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

use crate::hair_blob;
use crate::mesh_blob::MeshCache;
use crate::root_id;

fn field(replica: &Replica, id: &ObjectId, name: &str) -> Value {
    replica.get(id, name).unwrap_or(Value::Null)
}

fn parse<T: DeserializeOwned>(value: Value, what: &str) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| format!("damaged: {what}: {error}"))
}

fn parse_or<T: DeserializeOwned + Default>(value: Value, what: &str) -> Result<T, String> {
    if value.is_null() { Ok(T::default()) } else { parse(value, what) }
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
            _ => parse::<ObjectData>(field(replica, &id, "data"), &format!("{name}'s data"))?,
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
        object.modifiers = parse_or::<Vec<Modifier>>(field(replica, &id, "modifiers"), &format!("{name}'s modifiers"))?;
        object.constraints = parse_or::<Vec<kition_scene_model::Constraint>>(field(replica, &id, "constraints"), &format!("{name}'s constraints"))?;
        object.physics = match field(replica, &id, "physics") {
            Value::Null => None,
            value => Some(parse::<kition_scene_model::physics::ObjectPhysics>(value, &format!("{name}'s physics"))?),
        };
        if matches!(object.data, ObjectData::Mesh(_)) {
            object.particles = parse_or::<Vec<ParticleSystem>>(field(replica, &id, "particles"), &format!("{name}'s particle systems"))?;
            // Groomed hair whose blob has not arrived yet shows as generated until it does.
            if let Some(bytes) = blob_ref(&field(replica, &id, "hair")).and_then(|blob| blobs.get(&blob.hash)) {
                hair_blob::apply(&mut object, &bytes).map_err(|error| format!("damaged: {name}'s hair: {error}"))?;
            }
        }
        channels.extend(parse_or::<Vec<Channel>>(field(replica, &id, "animation"), &format!("{name}'s animation"))?);
        let nla = field(replica, &id, "nla");
        kition_scene_model::nla::Nla::check_json_structure(&nla).map_err(|reason| format!("damaged: {name}'s NLA: {reason}"))?;
        object.nla = parse_or::<kition_scene_model::nla::Nla>(nla, &format!("{name}'s NLA"))?;
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
        channels.extend(parse_or::<Vec<Channel>>(field(replica, &id, "animation"), &format!("{name}'s animation"))?);
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

    let actions = parse_or::<Vec<kition_scene_model::animation::Action>>(field(replica, &root, "actions"), "the actions")?;
    let keying_sets = parse_or::<kition_scene_model::animation::KeyingSets>(field(replica, &root, "keyingSets"), "the keying sets")?;
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
