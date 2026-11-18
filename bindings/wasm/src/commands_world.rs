//! The world image: an equirectangular picture (PNG, JPEG, or Radiance HDR) the world shows
//! behind the scene and lights it with, in place of its colour (Render ▸ World Image). A change is
//! one undo step, and a failing command changes nothing.

use kition_kit::KitCommand;
use kition_scene_model::Image;
use serde_json::{Value, json};

use crate::SceneDocument;
use crate::params::{self, bad, schema as s};

const TITLE: &str = "World Image";

fn schema() -> Value {
    s::of(vec![
        ("path", s::string("An equirectangular .hdr, .png, or .jpg, relative to this scene's folder")),
        ("base64", s::string("The image's bytes, instead of a path")),
        ("name", s::string("The image's name, with base64")),
        ("rotation", s::number("Degrees the image is turned about the up axis")),
        ("strength", s::number("How strongly the world lights the scene")),
        ("clear", s::boolean("Remove the image: the world colour shows and lights again")),
    ])
}

/// The commands this file adds.
pub fn catalogue(_doc: &SceneDocument) -> Vec<KitCommand> {
    vec![KitCommand {
        id: "world.setImage".into(),
        title: TITLE.into(),
        menu: Some(vec!["Render".into()]),
        shortcut: None,
        params: None,
        schema: Some(schema()),
        mutates: true,
        enabled: true,
        ..Default::default()
    }]
}

/// The ones that cannot run now (none).
pub fn disabled(_doc: &SceneDocument) -> Vec<String> {
    Vec::new()
}

/// Runs `command` if it is one of these; `None` for every other command.
pub fn execute(doc: &mut SceneDocument, command: &str, params: &Value) -> Option<Result<Value, String>> {
    if command != "world.setImage" {
        return None;
    }
    if !params.is_object() && !params.is_null() {
        return Some(Err(bad("the parameters must be an object")));
    }
    let before = doc.step(TITLE);
    let session = doc.session.clone();
    let result = set_image(doc, params).and_then(|value| {
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

fn set_image(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    if let Some(rotation) = params::number(params, "rotation")? {
        doc.scene.world.rotation = rotation.to_radians().rem_euclid(std::f32::consts::TAU);
    }
    if let Some(strength) = params::number(params, "strength")? {
        doc.scene.world.strength = strength.max(0.0);
    }
    if params::boolean(params, "clear")?.unwrap_or(false) {
        doc.scene.world.image = None;
        doc.scene.prune_images();
        return Ok(json!({ "image": Value::Null }));
    }
    let source = match (params::string(params, "path")?, params::string(params, "base64")?) {
        (Some(path), _) => {
            let name = path.rsplit(['/', '\\']).next().unwrap_or(path).to_string();
            Some((kition_kit::host::read(path).map_err(|error| format!("unavailable: {error}"))?, name))
        }
        (None, Some(text)) => Some((crate::commands_uv::decode_base64(text)?, params::string(params, "name")?.unwrap_or("World").to_string())),
        (None, None) => None,
    };
    let Some((bytes, name)) = source else {
        if params.get("rotation").is_some() || params.get("strength").is_some() {
            return Ok(json!({ "image": doc.scene.world.image }));
        }
        return Err(bad("give `path`, `base64`, `clear`, or a new `rotation` or `strength`"));
    };
    let image = Image::new(name, bytes).map_err(|error| error.to_string())?;
    let Some((width, height, _)) = kition_scene_raster::texture::decode_linear(&image, true) else {
        return Err(bad("the image cannot be read (an .hdr, PNG, or JPEG up to 8192 pixels on a side)"));
    };
    let hash = doc.scene.add_image(image);
    doc.scene.world.image = Some(hash.clone());
    doc.scene.prune_images();
    Ok(json!({ "image": hash, "width": width, "height": height }))
}
