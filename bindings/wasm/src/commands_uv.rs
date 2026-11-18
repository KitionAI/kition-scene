//! Texture coordinate and image commands: the UV projections and unwrap (Mesh ▸ UV) and a
//! material's image slots. They join the catalogue in `commands.rs` and run the same way: each
//! change is one undo step, and a failing command changes nothing.

use std::collections::BTreeSet;

use kition_kit::KitCommand;
use kition_scene_model::uv::{Axis, Projection};
use kition_scene_model::{Id, Image, ModifierKind, Scene, TextureSlot};
use serde_json::{Value, json};

use crate::params::{self, bad, schema as s};
use crate::{Mode, SceneDocument};

struct Spec {
    id: &'static str,
    title: &'static str,
    menu: &'static [&'static str],
    schema: fn() -> Value,
}

const UV_MENU: &[&str] = &["Mesh", "UV"];

fn size_only() -> Value {
    s::of(vec![
        ("size", s::number("Metres one repeat of the texture covers (2)")),
        ("objects", s::objects("The mesh objects, in object mode; the selection when left out")),
    ])
}

fn targets_only() -> Value {
    s::of(vec![("objects", s::objects("The mesh objects, in object mode; the selection when left out"))])
}

static SPECS: &[Spec] = &[
    Spec {
        id: "uv.smartProject",
        title: "Smart UV Project",
        menu: UV_MENU,
        schema: || {
            s::of(vec![
                ("angleLimit", s::number("Faces whose normals differ by more than this many degrees go to separate islands (66)")),
                ("margin", s::number("Space between islands, as a fraction of the texture (0.02)")),
                ("objects", s::objects("The mesh objects, in object mode; the selection when left out")),
            ])
        },
    },
    Spec { id: "uv.cubeProject", title: "Cube Projection", menu: UV_MENU, schema: size_only },
    Spec { id: "uv.cylinderProject", title: "Cylinder Projection", menu: UV_MENU, schema: targets_only },
    Spec { id: "uv.sphereProject", title: "Sphere Projection", menu: UV_MENU, schema: targets_only },
    Spec {
        id: "uv.planarProject",
        title: "Planar Projection",
        menu: UV_MENU,
        schema: || {
            s::of(vec![
                ("axis", s::choice("The local axis to project along (z)", &["x", "y", "z"])),
                ("size", s::number("Metres one repeat of the texture covers (2)")),
                ("objects", s::objects("The mesh objects, in object mode; the selection when left out")),
            ])
        },
    },
    Spec { id: "uv.reset", title: "Reset", menu: UV_MENU, schema: targets_only },
    Spec { id: "uv.clear", title: "Clear UVs", menu: UV_MENU, schema: targets_only },
    Spec {
        id: "material.setTexture",
        title: "Set Material Image",
        menu: &[],
        schema: || {
            s::of(vec![
                (
                    "material",
                    s::required(json!({ "oneOf": [{ "type": "integer" }, { "type": "string" }], "description": "The material's id or name" })),
                ),
                ("slot", s::choice("The image slot (baseColor)", &["baseColor", "metallicRoughness", "normal"])),
                ("path", s::string("A .png or .jpg file, relative to this scene's folder")),
                ("base64", s::string("The image's bytes, instead of a path")),
                ("name", s::string("The image's name, with base64 bytes")),
                ("clear", s::boolean("Take the image out of the slot")),
            ])
        },
    },
    Spec {
        id: "modifier.setImage",
        title: "Set Displace Image",
        menu: &[],
        schema: || {
            s::of(vec![
                ("object", s::object("The object; the active one when left out")),
                ("index", s::integer("The Displace modifier's position in the stack, from 0")),
                ("name", s::string("The Displace modifier's name (instead of index)")),
                ("path", s::string("A .png or .jpg file, relative to this scene's folder")),
                ("base64", s::string("The image's bytes, instead of a path")),
                ("imageName", s::string("The image's name, with base64 bytes")),
                ("clear", s::boolean("Take the image off the modifier")),
            ])
        },
    },
];

fn enabled(doc: &SceneDocument, id: &str) -> bool {
    if id == "modifier.setImage" {
        return doc.scene.objects.iter().any(|object| object.modifiers.iter().any(|modifier| matches!(modifier.kind, ModifierKind::Displace { .. })));
    }
    if id.starts_with("uv.") {
        return match doc.session.mode {
            Mode::Edit(_) => true,
            Mode::Object => doc.session.editor.selected.iter().any(|id| doc.scene.object(*id).is_ok_and(|object| object.mesh().is_some())),
        };
    }
    !doc.scene.materials.is_empty()
}

/// Whether the call names what it acts on, so the gate's look at the selection does not apply: a
/// UV command's `objects` in object mode holding a mesh.
fn names_targets(doc: &SceneDocument, id: &str, params: &Value) -> bool {
    id.starts_with("uv.")
        && doc.session.mode == Mode::Object
        && params::objects(&doc.scene, params, "objects")
            .ok()
            .flatten()
            .is_some_and(|ids| ids.iter().any(|id| doc.scene.object(*id).is_ok_and(|object| object.mesh().is_some())))
}

/// Whether the object `modifier.setImage` changes (`object`, else the active one) has a Displace
/// modifier: the gate looks at that object, not at any object of the scene.
fn displace_target(doc: &SceneDocument, params: &Value) -> bool {
    let target = match params.get("object").filter(|value| !value.is_null()) {
        // A name that finds no object is the command's own not-found.
        Some(value) => match params::resolve(&doc.scene, value) {
            Ok(id) => Some(id),
            Err(_) => return true,
        },
        None => doc.session.editor.active,
    };
    target
        .and_then(|id| doc.scene.object(id).ok())
        .is_some_and(|object| object.modifiers.iter().any(|modifier| matches!(modifier.kind, ModifierKind::Displace { .. })))
}

/// The commands this file adds.
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
            enabled: enabled(doc, spec.id),
            ..Default::default()
        })
        .collect()
}

/// The ones that cannot run now.
pub fn disabled(doc: &SceneDocument) -> Vec<String> {
    SPECS.iter().filter(|spec| !enabled(doc, spec.id)).map(|spec| spec.id.to_string()).collect()
}

/// Runs `command` if it is one of these; `None` for every other command.
pub fn execute(doc: &mut SceneDocument, command: &str, params: &Value) -> Option<Result<Value, String>> {
    let spec = SPECS.iter().find(|spec| spec.id == command)?;
    if !params.is_object() && !params.is_null() {
        return Some(Err(bad("the parameters must be an object")));
    }
    let allowed =
        if spec.id == "modifier.setImage" { displace_target(doc, params) } else { enabled(doc, spec.id) || names_targets(doc, spec.id, params) };
    if !allowed {
        return Some(Err(format!("not-allowed: {} cannot run now", spec.title)));
    }
    let before = doc.step(spec.title);
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

/// The meshes a UV command covers, each with the faces it covers (empty: all).
fn uv_targets(doc: &SceneDocument, params: &Value) -> Result<Vec<(Id, BTreeSet<u32>)>, String> {
    match doc.session.mode {
        // Named meshes win: the edited one with its selected faces, any other one whole.
        Mode::Edit(id) if params.get("objects").is_some_and(|value| !value.is_null()) => {
            let ids = params::objects(&doc.scene, params, "objects")?.unwrap_or_default();
            let mut meshes = Vec::new();
            for named in ids {
                let object = doc.scene.object(named).map_err(|error| error.to_string())?;
                let mesh = object.mesh().ok_or_else(|| bad(format!("{} is not a mesh", object.name)))?;
                let faces = if named == id { crate::commands::faces_selected(&doc.session, mesh) } else { BTreeSet::new() };
                meshes.push((named, faces));
            }
            Ok(meshes)
        }
        Mode::Edit(id) => {
            let mesh = doc.scene.object(id).ok().and_then(|object| object.mesh()).ok_or("unsupported: the edited object is not a mesh")?;
            let faces = crate::commands::faces_selected(&doc.session, mesh);
            Ok(vec![(id, faces)])
        }
        Mode::Object => {
            let ids = match params::objects(&doc.scene, params, "objects")? {
                Some(ids) => ids,
                None => doc.session.editor.selected.clone(),
            };
            let meshes: Vec<(Id, BTreeSet<u32>)> = ids
                .into_iter()
                .filter(|id| doc.scene.object(*id).is_ok_and(|object| object.mesh().is_some()))
                .map(|id| (id, BTreeSet::new()))
                .collect();
            if meshes.is_empty() {
                return Err("not-allowed: select a mesh object".into());
            }
            Ok(meshes)
        }
    }
}

fn run(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    if command == "material.setTexture" {
        return set_texture(doc, params);
    }
    if command == "modifier.setImage" {
        return set_displace_image(doc, params);
    }
    let size = params::number(params, "size")?.unwrap_or(2.0);
    if size.is_nan() || size <= 0.0 {
        return Err(bad("`size` must be positive"));
    }
    let projection = match command {
        "uv.cubeProject" => Some(Projection::Cube),
        "uv.cylinderProject" => Some(Projection::Cylinder),
        "uv.sphereProject" => Some(Projection::Sphere),
        "uv.reset" => Some(Projection::Reset),
        "uv.planarProject" => Some(Projection::Planar(match params::string(params, "axis")?.unwrap_or("z") {
            "x" => Axis::X,
            "y" => Axis::Y,
            "z" => Axis::Z,
            other => return Err(bad(format!("`axis` must be x, y, or z, not {other}"))),
        })),
        _ => None,
    };
    let angle_limit = params::number(params, "angleLimit")?.unwrap_or(66.0);
    let margin = params::number(params, "margin")?.unwrap_or(0.02);
    if !(0.0..=180.0).contains(&angle_limit) || !(0.0..=0.5).contains(&margin) {
        return Err(bad("`angleLimit` must be 0 to 180 degrees and `margin` 0 to 0.5"));
    }
    let mut changed = Vec::new();
    for (id, faces) in uv_targets(doc, params)? {
        let mesh = doc.scene.object_mut(id).map_err(|error| error.to_string())?.mesh_mut().ok_or("unsupported: not a mesh")?;
        match (command, projection) {
            ("uv.clear", _) => mesh.uvs.clear(),
            ("uv.smartProject", _) => kition_scene_model::uv::smart_project(mesh, &faces, angle_limit.to_radians(), margin),
            (_, Some(projection)) => kition_scene_model::uv::project(mesh, &faces, projection, size),
            (other, None) => return Err(format!("unknown-command: {other}")),
        }
        changed.push(json!({ "object": id, "faces": if faces.is_empty() { mesh.faces.len() } else { faces.len() } }));
    }
    Ok(json!({ "changed": changed }))
}

fn material_of(scene: &Scene, params: &Value) -> Result<Id, String> {
    match params.get("material") {
        Some(Value::Number(number)) => {
            let id = number.as_u64().and_then(|id| u32::try_from(id).ok()).ok_or_else(|| bad("`material` must be an id or a name"))?;
            scene.material(id).map(|material| material.id).map_err(|_| format!("not-found: no material {id}"))
        }
        Some(Value::String(name)) => scene
            .materials
            .iter()
            .find(|material| &material.name == name)
            .map(|material| material.id)
            .ok_or_else(|| format!("not-found: no material named {name}")),
        _ => Err(bad("`material` is required (an id or a name)")),
    }
}

fn set_texture(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let material = material_of(&doc.scene, params)?;
    let slot_name = params::string(params, "slot")?.unwrap_or("baseColor");
    let slot =
        TextureSlot::parse(slot_name).ok_or_else(|| bad(format!("`slot` must be baseColor, metallicRoughness, or normal, not {slot_name}")))?;
    if params::boolean(params, "clear")?.unwrap_or(false) {
        doc.scene.material_mut(material).map_err(|error| error.to_string())?.set_texture(slot, None);
        doc.scene.prune_images();
        return Ok(json!({ "material": material, "slot": slot_name, "image": Value::Null }));
    }
    let (hash, image_name, width, height) = load_image(doc, params, "name")?;
    doc.scene.material_mut(material).map_err(|error| error.to_string())?.set_texture(slot, Some(hash.clone()));
    doc.scene.prune_images();
    Ok(json!({ "material": material, "slot": slot_name, "image": hash, "name": image_name, "width": width, "height": height }))
}

/// Reads the image a command names by `path` or `base64` (named `name_key` then), and adds it.
pub(crate) fn load_image(doc: &mut SceneDocument, params: &Value, name_key: &str) -> Result<(String, String, usize, usize), String> {
    let (bytes, name) = match (params::string(params, "path")?, params::string(params, "base64")?) {
        (Some(path), _) => {
            let name = path.rsplit(['/', '\\']).next().unwrap_or(path).to_string();
            (kition_kit::host::read(path).map_err(|error| format!("unavailable: {error}"))?, name)
        }
        (None, Some(text)) => (decode_base64(text)?, params::string(params, name_key)?.unwrap_or("Image").to_string()),
        (None, None) => return Err(bad("give `path`, `base64`, or `clear`")),
    };
    let image = Image::new(name, bytes).map_err(|error| error.to_string())?;
    let Some((width, height, _)) = kition_scene_raster::texture::decode_rgba(&image) else {
        return Err(bad("the image cannot be read (a PNG or JPEG up to 8192 pixels on a side)"));
    };
    let (hash, image_name) = (image.hash.clone(), image.name.clone());
    doc.scene.add_image(image);
    Ok((hash, image_name, width, height))
}

fn set_displace_image(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let (id, index) = crate::commands::find_modifier(doc, params)?;
    let image = if params::boolean(params, "clear")?.unwrap_or(false) { None } else { Some(load_image(doc, params, "imageName")?) };
    let object = doc.scene.object_mut(id).map_err(|error| error.to_string())?;
    let Some(ModifierKind::Displace { image: slot, .. }) = object.modifiers.get_mut(index).map(|modifier| &mut modifier.kind) else {
        return Err(bad("that modifier is not a Displace modifier"));
    };
    *slot = image.as_ref().map(|(hash, ..)| hash.clone());
    doc.scene.prune_images();
    Ok(match image {
        Some((hash, name, width, height)) => json!({ "object": id, "index": index, "image": hash, "name": name, "width": width, "height": height }),
        None => json!({ "object": id, "index": index, "image": Value::Null }),
    })
}

pub(crate) fn decode_base64(text: &str) -> Result<Vec<u8>, String> {
    let value = |byte: u8| -> Option<u32> {
        match byte {
            b'A'..=b'Z' => Some(u32::from(byte - b'A')),
            b'a'..=b'z' => Some(u32::from(byte - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(byte - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let clean: Vec<u8> = text.bytes().filter(|byte| !byte.is_ascii_whitespace() && *byte != b'=').collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut triple = 0u32;
        for (index, byte) in chunk.iter().enumerate() {
            triple |= value(*byte).ok_or_else(|| bad("`base64` is not base64"))? << (18 - 6 * index);
        }
        out.extend(triple.to_be_bytes().iter().skip(1).take(chunk.len().saturating_sub(1)));
    }
    Ok(out)
}

/// The images a scene keeps, for inspect.
pub fn images_json(scene: &Scene) -> Value {
    Value::Array(
        scene
            .images
            .iter()
            .map(|image| {
                let users: Vec<Id> =
                    scene.materials.iter().filter(|material| material.images().any(|hash| *hash == image.hash)).map(|material| material.id).collect();
                json!({ "hash": image.hash, "name": image.name, "format": image.format.extension(), "bytes": image.bytes.len(), "materials": users })
            })
            .collect(),
    )
}
