//! Compositing: renders pass through the compositing node group the render settings name, when
//! compositing is on (Render ▸ Compositing). `compositor.new` makes such a group (Render Layers
//! into the Group Output) and uses it; `nodes.*` edit its nodes; `compositor.set` picks another
//! group or turns compositing off; `compositor.setImage` gives an Image node a picture;
//! `compositor.pickMatte` adds the name under a point of the render to a Cryptomatte node's
//! matte (or takes it away); and
//! `compositor.preview` answers a small composited render, or what its Viewer node shows. Renders
//! (`render.image`, path tracing once done, `render.animation`, PNG exports) come out
//! composited. A group that reads the render's passes gets them: the geometric ones (normal,
//! vector, environment, ambient occlusion, Cryptomatte) traced for any render, the light ones
//! from path-traced renders only (a fast render's are black).

use std::collections::BTreeMap;

use kition_kit::KitCommand;
use kition_scene_compositor::{Composited, Image, Passes};
use kition_scene_model::node_tree::{NodeGroup, Tree};
use kition_scene_model::{Id, Scene};
use serde_json::{Value, json};

use crate::SceneDocument;
use crate::params::{self, bad, schema as s};

#[cfg(test)]
#[path = "commands_compositor_camera9_tests.rs"]
mod camera9;

const MENU: &[&str] = &["Render"];

/// The commands this file adds: id, title, whether it is in the menu, whether it changes the
/// scene, and its parameters.
type Entry = (&'static str, &'static str, bool, bool, fn() -> Value);

/// A compositing group named by its id or its name (the description says so: the premise guard
/// reads such a parameter's whole numbers as ids, `params::object_parameters`).
fn group_ref(description: &str) -> (&'static str, Value) {
    ("group", json!({ "type": "json", "description": format!("{description}: its id or name") }))
}

static ENTRIES: &[Entry] = &[
    ("compositor.new", "New Compositor", true, true, || {
        s::of(vec![("name", s::string("The group's name (Compositing when left out)")), ("use", s::boolean("Make renders pass through it (true)"))])
    }),
    ("compositor.set", "Compositing", false, true, || {
        s::of(vec![
            group_ref("The compositing group renders pass through: its id or name; null for none"),
            ("compositing", s::boolean("Whether renders pass through the group")),
        ])
    }),
    ("compositor.setImage", "Set Compositing Image", false, true, || {
        s::of(vec![
            group_ref("The compositing group (the one renders use when left out)"),
            ("node", s::required(s::integer("The Image node's id"))),
            ("path", s::string("An image file (PNG, JPEG, or Radiance HDR) to read")),
            ("base64", s::string("The image file's bytes, base64")),
            ("name", s::string("The image's name, with base64")),
            ("image", s::string("An image the scene has already, by its hash")),
            ("clear", s::boolean("Take the node's picture away")),
        ])
    }),
    ("compositor.pickMatte", "Pick Matte", false, true, || {
        s::of(vec![
            group_ref("The compositing group (the one renders use when left out)"),
            ("node", s::required(s::integer("The Cryptomatte node's id"))),
            ("x", s::required(s::number("Across the render, 0 (left) to 1 (right)"))),
            ("y", s::required(s::number("Down the render, 0 (top) to 1 (bottom)"))),
            ("remove", s::boolean("Take the name away from the matte instead (false)")),
        ])
    }),
    ("compositor.preview", "Preview Compositing", false, false, || {
        s::of(vec![
            ("size", s::integer("The longer side of the preview in pixels, 16 to 2048 (480)")),
            ("viewer", s::boolean("Answer what the Viewer node shows instead of the output (false)")),
            group_ref("A compositing group to preview other than the render's"),
        ])
    }),
];

/// The commands this file adds.
pub fn catalogue(doc: &SceneDocument) -> Vec<KitCommand> {
    let off = disabled(doc);
    ENTRIES
        .iter()
        .map(|(id, title, in_menu, mutates, schema)| KitCommand {
            id: (*id).into(),
            title: (*title).into(),
            menu: in_menu.then(|| MENU.iter().map(|item| (*item).to_string()).collect()),
            shortcut: None,
            params: None,
            schema: Some(schema()),
            mutates: *mutates,
            enabled: !off.iter().any(|other| other == id),
            ..Default::default()
        })
        .collect()
}

/// The ones that cannot run now: previews need a camera, and images a compositing group.
pub fn disabled(doc: &SceneDocument) -> Vec<String> {
    let mut out = Vec::new();
    if doc.scene.camera.is_none() {
        out.push("compositor.preview".to_owned());
    }
    if !doc.scene.node_groups.iter().any(|group| group.tree == Tree::Compositor) {
        out.push("compositor.setImage".to_owned());
    }
    let cryptomatte =
        doc.scene.node_groups.iter().any(|group| group.tree == Tree::Compositor && group.nodes.iter().any(|node| node.kind == "cryptomatte"));
    if !cryptomatte || doc.scene.camera.is_none() {
        out.push("compositor.pickMatte".to_owned());
    }
    out
}

/// Runs `command` if it is one of these; `None` for every other command.
pub fn execute(doc: &mut SceneDocument, command: &str, params: &Value) -> Option<Result<Value, String>> {
    let (_, title, _, mutates, _) = ENTRIES.iter().find(|entry| entry.0 == command)?;
    if !params.is_object() && !params.is_null() {
        return Some(Err(bad("the parameters must be an object")));
    }
    if !mutates {
        return Some(preview(doc, params));
    }
    let before = doc.step(title);
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

/// The compositing group `value` names (an id or a name).
fn named_group(scene: &Scene, value: &Value) -> Result<Id, String> {
    let group = match value {
        Value::String(name) => scene.node_groups.iter().find(|group| &group.name == name),
        other => {
            let id = other.as_u64().and_then(|id| Id::try_from(id).ok()).ok_or_else(|| bad("`group` must be an id or a name"))?;
            scene.node_group(id)
        }
    };
    let group = group.ok_or_else(|| format!("not-found: node group {value}"))?;
    if group.tree != Tree::Compositor {
        return Err(bad(format!("{} is not a compositing group", group.name)));
    }
    Ok(group.id)
}

/// The group a command names, else the one renders use, else the only compositing group.
fn target_group(scene: &Scene, params: &Value) -> Result<Id, String> {
    if let Some(value) = params.get("group").filter(|value| !value.is_null()) {
        return named_group(scene, value);
    }
    if let Some(id) = scene.render.compositor {
        return Ok(id);
    }
    let mut groups = scene.node_groups.iter().filter(|group| group.tree == Tree::Compositor);
    match (groups.next(), groups.next()) {
        (Some(only), None) => Ok(only.id),
        (None, _) => Err("not-found: there is no compositing group".into()),
        _ => Err(bad("`group` is required when there are several compositing groups")),
    }
}

fn run(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    match command {
        "compositor.new" => {
            let scene = &mut doc.scene;
            let name = params::string(params, "name")?.unwrap_or("Compositing");
            let name = kition_scene_model::scene::unique_name(name, |candidate| scene.node_groups.iter().any(|group| group.name == candidate));
            let (id, input, output) = (scene.take_id(), scene.take_id(), scene.take_id());
            scene.node_groups.push(NodeGroup::compositor(id, name.clone(), input, output));
            if params::boolean(params, "use")?.unwrap_or(true) {
                scene.render.compositor = Some(id);
                scene.render.compositing = true;
            }
            Ok(json!({ "group": id, "name": name, "renderLayers": input, "output": output, "used": scene.render.compositor == Some(id) }))
        }
        "compositor.set" => {
            let scene = &mut doc.scene;
            if let Some(value) = params.get("group") {
                scene.render.compositor = if value.is_null() { None } else { Some(named_group(scene, value)?) };
            }
            if let Some(on) = params::boolean(params, "compositing")? {
                scene.render.compositing = on;
            }
            Ok(json!({ "group": scene.render.compositor, "compositing": scene.render.compositing }))
        }
        "compositor.setImage" => {
            let group = target_group(&doc.scene, params)?;
            let node = params::integer(params, "node")?.ok_or_else(|| bad("`node` is required"))?;
            let kind = doc
                .scene
                .node_group(group)
                .and_then(|group| group.node(node))
                .map(|node| node.kind.clone())
                .ok_or_else(|| format!("not-found: node {node}"))?;
            if kind != "image" {
                return Err(bad(format!("node {node} is not an Image node")));
            }
            let (hash, name) = if params::boolean(params, "clear")?.unwrap_or(false) {
                (None, None)
            } else if let Some(hash) = params::string(params, "image")? {
                let image = doc.scene.image(hash).ok_or_else(|| format!("not-found: image {hash}"))?;
                (Some(hash.to_owned()), Some(image.name.clone()))
            } else {
                let (hash, name, _, _) = crate::commands_uv::load_image(doc, params, "name")?;
                (Some(hash), Some(name))
            };
            let target = doc.scene.node_group_mut(group).and_then(|group| group.node_mut(node)).ok_or_else(|| format!("not-found: node {node}"))?;
            match &hash {
                Some(hash) => target.values.insert("Image".into(), json!(hash)),
                None => target.values.remove("Image"),
            };
            doc.scene.prune_images();
            Ok(json!({ "group": group, "node": node, "image": hash, "name": name }))
        }
        "compositor.pickMatte" => {
            let group = target_group(&doc.scene, params)?;
            let node = params::integer(params, "node")?.ok_or_else(|| bad("`node` is required"))?;
            let (x, y) = (
                params::number(params, "x")?.ok_or_else(|| bad("`x` is required"))?,
                params::number(params, "y")?.ok_or_else(|| bad("`y` is required"))?,
            );
            let remove = params::boolean(params, "remove")?.unwrap_or(false);
            let found = doc.scene.node_group(group).and_then(|group| group.node(node)).ok_or_else(|| format!("not-found: node {node}"))?;
            if found.kind != "cryptomatte" {
                return Err(bad(format!("node {node} is not a Cryptomatte node")));
            }
            let layer = match found.values.get("Layer").and_then(Value::as_str) {
                Some("material") => 1,
                Some("asset") => 2,
                _ => 0,
            };
            let current = found.values.get("Matte ID").and_then(Value::as_str).unwrap_or_default().to_owned();
            let scene = crate::commands_particles::render_scene(doc, doc.scene.clone());
            let names = kition_scene_trace::passes::names_at(&scene, x, y)?;
            let name = names.and_then(|names| names.into_iter().nth(layer).flatten()).ok_or("not-found: nothing is there to pick")?;
            let mut picked: Vec<String> = current.split(',').map(|item| item.trim().to_owned()).filter(|item| !item.is_empty()).collect();
            picked.retain(|item| *item != name);
            if !remove {
                picked.push(name.clone());
            }
            let matte = picked.join(", ");
            let target = doc.scene.node_group_mut(group).and_then(|group| group.node_mut(node)).ok_or_else(|| format!("not-found: node {node}"))?;
            target.values.insert("Matte ID".into(), json!(matte));
            Ok(json!({ "group": group, "node": node, "name": name, "matte": matte }))
        }
        other => Err(format!("not-found: {other}")),
    }
}

/// A scene image as a compositing picture: linear, premultiplied.
fn scene_picture(scene: &Scene, hash: &str) -> Option<Image> {
    let image = scene.image(hash)?;
    if image.format == kition_scene_model::scene::ImageFormat::Hdr {
        let (width, height, pixels) = kition_scene_model::hdr::decode(&image.bytes)?;
        return Some(Image { width, height, pixels: pixels.into_iter().map(|[r, g, b]| [r, g, b, 1.0]).collect() });
    }
    let (width, height, bytes) = kition_scene_model::pixels::decode_rgba8(image)?;
    let pixels = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|[r, g, b, a]| {
            let alpha = f32::from(*a) / 255.0;
            let linear = |byte: u8| kition_scene_raster::draw::decode(byte) * alpha;
            [linear(*r), linear(*g), linear(*b), alpha]
        })
        .collect();
    Some(Image { width, height, pixels })
}

/// What a render gives the compositor beyond its picture and depth: the path tracer's light
/// passes, when it made them.
#[derive(Default)]
pub struct Extra {
    pub light: Option<BTreeMap<String, Vec<[f32; 4]>>>,
}

/// The render's passes `group` reads (none when it reads none): the geometric passes traced for
/// the scene, and the light passes `extra` brings.
fn layers(scene: &Scene, group: &NodeGroup, width: u32, height: u32, extra: &Extra) -> Result<Option<kition_scene_compositor::Layers>, String> {
    if !kition_scene_compositor::uses_passes(group) {
        return Ok(None);
    }
    let mut sized = scene.clone();
    sized.render.width = width;
    sized.render.height = height;
    let shapes = kition_scene_trace::passes::geometry(&sized)?;
    let mut passes = shapes.passes;
    if let Some(light) = &extra.light {
        passes.extend(light.iter().map(|(name, pixels)| (name.clone(), pixels.clone())));
    }
    use kition_scene_compositor::cryptomatte::{Layer, Layers as Crypto};
    let cryptomatte = Crypto {
        object: Layer::from_coverage(&shapes.objects),
        material: Layer::from_coverage(&shapes.materials),
        asset: Layer::from_coverage(&shapes.assets),
    };
    Ok(Some(kition_scene_compositor::Layers { passes, cryptomatte: Some(cryptomatte) }))
}

/// The scene camera's lens, for Defocus.
fn lens(scene: &Scene) -> Option<kition_scene_compositor::Lens> {
    let id = scene.camera?;
    let object = scene.object(id).ok()?;
    let kition_scene_model::ObjectData::Camera(camera) = &object.data else { return None };
    let focus = match camera.dof.focus_object.and_then(|target| scene.world_matrix(target).ok()) {
        Some(target) => {
            let eye = scene.world_matrix(id).ok()?;
            let forward = -eye.z_axis.truncate().normalize_or_zero();
            (target.w_axis.truncate() - eye.w_axis.truncate()).dot(forward).max(1e-3)
        }
        None => camera.dof.focus_distance,
    };
    let aspect = scene.render.width.max(1) as f32 / scene.render.height.max(1) as f32;
    // Defocus converts a physical blur diameter to pixels across the horizontal gate.
    let sensor_width = if camera.orthographic { camera.sensor } else { 2.0 * camera.lens * camera.frame_half_size(aspect).0 };
    Some(kition_scene_compositor::Lens { focal_length: camera.lens, sensor_width, focus_distance: focus })
}

/// Runs `group` on a render's linear, premultiplied pixels.
fn run_group(
    scene: &Scene,
    group: &NodeGroup,
    width: u32,
    height: u32,
    pixels: Vec<[f32; 4]>,
    depth: Option<&[f32]>,
    extra: &Extra,
) -> Result<Composited, String> {
    let image = Image { width: width as usize, height: height as usize, pixels };
    let pictures: BTreeMap<String, Image> = group
        .nodes
        .iter()
        .filter(|node| node.kind == "image")
        .filter_map(|node| node.values.get("Image").and_then(Value::as_str))
        .filter_map(|hash| Some((hash.to_owned(), scene_picture(scene, hash)?)))
        .collect();
    let images = |hash: &str| pictures.get(hash).cloned();
    let layers = layers(scene, group, width, height, extra)?;
    kition_scene_compositor::composite(group, &Passes { image: &image, depth, images: &images, layers: layers.as_ref(), lens: lens(scene) })
}

/// A render's pixels (linear, premultiplied) passed through the scene's compositing group when
/// compositing is on; as they are otherwise (or when the group has no output).
pub fn composite(
    scene: &Scene,
    width: u32,
    height: u32,
    pixels: Vec<[f32; 4]>,
    depth: Option<&[f32]>,
    extra: &Extra,
) -> Result<Vec<[f32; 4]>, String> {
    let Some(group) = scene.render.active_compositor().and_then(|id| scene.node_group(id)) else { return Ok(pixels) };
    let composited = run_group(scene, group, width, height, pixels.clone(), depth, extra)?;
    Ok(composited.image.map_or(pixels, |image| image.pixels))
}

/// Whether renders of `scene` pass through a group that reads the light passes (a path-traced
/// render then keeps them).
pub fn wants_light_passes(scene: &Scene) -> bool {
    scene.render.active_compositor().and_then(|id| scene.node_group(id)).is_some_and(kition_scene_compositor::uses_light_passes)
}

/// Whether renders of `scene` pass through a compositing group.
pub fn compositing(scene: &Scene) -> bool {
    scene.render.active_compositor().is_some_and(|id| scene.node_group(id).is_some())
}

/// Linear, premultiplied pixels as straight sRGB RGBA8, through the render's colour management
/// (`display`), which comes after compositing as in the reference.
pub fn to_rgba8(pixels: &[[f32; 4]], display: &kition_scene_model::color::ColorManagement) -> Vec<u8> {
    let mut out = Vec::with_capacity(pixels.len() * 4);
    for pixel in pixels {
        let alpha = if pixel[3].is_finite() { pixel[3].clamp(0.0, 1.0) } else { 0.0 };
        let straight = |value: f32| if alpha > 0.0 { value / alpha } else { 0.0 };
        let [r, g, b] = display.apply([straight(pixel[0]), straight(pixel[1]), straight(pixel[2])]);
        out.extend([
            kition_scene_raster::draw::encode(r),
            kition_scene_raster::draw::encode(g),
            kition_scene_raster::draw::encode(b),
            (alpha * 255.0).round() as u8,
        ]);
    }
    out
}

/// A fast render of `scene` through its camera, passed through its compositing group when
/// compositing is on, as a PNG.
pub fn camera_png(doc: &SceneDocument, scene: &Scene, samples: u32) -> Result<Vec<u8>, String> {
    let options = kition_scene_raster::RenderOptions { samples, ..Default::default() };
    if !compositing(scene) {
        let canvas = kition_scene_raster::render_camera_cached(scene, &options, &mut doc.meshes.borrow_mut())?;
        return crate::encode_png(canvas.width, canvas.height, &canvas.to_rgba8());
    }
    let canvas = kition_scene_raster::render_camera_passes(scene, &options, &mut doc.meshes.borrow_mut())?;
    let pixels = composite(scene, canvas.width, canvas.height, canvas.color.clone(), Some(&canvas.depth), &Extra::default())?;
    crate::encode_png(canvas.width, canvas.height, &to_rgba8(&pixels, &scene.render.color_management))
}

/// A small composited render (or what the Viewer shows) as a PNG.
fn preview(doc: &SceneDocument, params: &Value) -> Result<Value, String> {
    let side = params::integer(params, "size")?.unwrap_or(480).clamp(16, 2048);
    let viewer = params::boolean(params, "viewer")?.unwrap_or(false);
    let mut scene = crate::commands_particles::render_scene(doc, doc.scene.clone());
    let group_id = match params.get("group").filter(|value| !value.is_null()) {
        Some(value) => named_group(&scene, value)?,
        None => scene.render.compositor.ok_or("not-found: the render uses no compositing group")?,
    };
    let (width, height) = (scene.render.width, scene.render.height);
    let scale = side as f32 / width.max(height).max(1) as f32;
    if scale < 1.0 {
        scene.render.width = ((width as f32 * scale).round() as u32).max(1);
        scene.render.height = ((height as f32 * scale).round() as u32).max(1);
    }
    let options = kition_scene_raster::RenderOptions { samples: 1, ..Default::default() };
    let canvas = kition_scene_raster::render_camera_passes(&scene, &options, &mut doc.meshes.borrow_mut())?;
    let group = scene.node_group(group_id).ok_or("not-found: the compositing group")?;
    let composited = run_group(&scene, group, canvas.width, canvas.height, canvas.color.clone(), Some(&canvas.depth), &Extra::default())?;
    let shown = if viewer { composited.viewer.or(composited.image) } else { composited.image };
    let pixels = shown.map_or_else(|| canvas.color.clone(), |image| image.pixels);
    let png = crate::encode_png(canvas.width, canvas.height, &to_rgba8(&pixels, &scene.render.color_management))?;
    Ok(json!({
        "mime": "image/png",
        "extension": "png",
        "width": canvas.width,
        "height": canvas.height,
        "group": group_id,
        "base64": crate::commands::base64(&png),
    }))
}

/// What `compositor.pickMatte` meets at its point (C8-scene-engine 5b): besides the position fields,
/// what decides where a surface is and whether the camera sees it, for `covers`.
const MATTE_COVERS: [&str; 7] = ["hidden", "constraints", "data", "modifiers", "mesh", "particles", "hair"];

/// The object (an emitter for its particles) whose surface the scene camera sees at the point
/// `params` names (`x`, `y`), in `scene`, as `compositor.pickMatte` renders it.
fn matte_hit(doc: &SceneDocument, scene: &Scene, params: &Value) -> Result<Option<Id>, String> {
    let (x, y) =
        (params::number(params, "x")?.ok_or_else(|| bad("`x` is required"))?, params::number(params, "y")?.ok_or_else(|| bad("`y` is required"))?);
    let rendered = crate::commands_particles::render_scene(doc, scene.clone());
    let hit = kition_scene_trace::passes::object_at(&rendered, x, y)?;
    Ok(hit.and_then(|id| crate::commands_particles::owners(doc, vec![id]).first().copied()))
}

/// What a call rests on (`KitDocument::under`) for the command that names a point of the render
/// (`compositor.pickMatte`, [`matte_under`]); any other command's `base` as it is.
pub fn under(doc: &SceneDocument, command: &str, base: Option<kition_kit::Under>, params: &Value) -> Result<Option<kition_kit::Under>, String> {
    match command {
        "compositor.pickMatte" => matte_under(doc, base, params),
        _ => Ok(base),
    }
}

/// What the point of the render a call names met at the agent's basis (`KitDocument::covered`,
/// [`matte_covered`]); `None` for a command that names no such point.
#[allow(clippy::type_complexity)]
pub fn covered(
    doc: &SceneDocument,
    command: &str,
    params: &Value,
    objects: &[kition_kit::CoveredAsk],
) -> Result<Option<Option<kition_kit::CoveredAt>>, String> {
    match command {
        "compositor.pickMatte" => matte_covered(doc, params, objects).map(Some),
        _ => Ok(None),
    }
}

/// What a `compositor.pickMatte` call rests on (C8-scene-engine 5b): a place in space, told by
/// `covers` as a world point is ([`crate::place::under`]): the object under the point now, its
/// name, its material slot and that material's name, and its parents' names (the asset layer reads
/// the topmost one); the rest (`base`: the node, its group, the scene's camera and render size)
/// as [`crate::rests`] says. When what is under the point cannot be told (no camera), every
/// object and material.
fn matte_under(doc: &SceneDocument, base: Option<kition_kit::Under>, params: &Value) -> Result<Option<kition_kit::Under>, String> {
    use crate::rests::{Need, On};
    let mut under = base.unwrap_or_default();
    let scene = &doc.scene;
    let mut found: BTreeMap<On, Need> = BTreeMap::new();
    match matte_hit(doc, scene, params) {
        Ok(hit) => {
            let mut at = hit;
            let mut fields: &[&str] = &["material", "name"];
            let mut seen = 0;
            while let Some(id) = at.filter(|_| seen <= scene.objects.len()) {
                seen += 1;
                let mut need = Need::of(fields);
                need.positions = Some(["parent".to_owned()].into());
                found.insert(On::Id(id), need);
                if let Some(material) = scene.object(id).ok().and_then(|object| object.material).filter(|_| fields.contains(&"material")) {
                    found.insert(On::Id(material), Need::of(&["name"]));
                }
                fields = &["name"];
                at = scene.object(id).ok().and_then(|object| object.parent);
            }
            under.objects = hit.map(|id| vec![json!(id)]).unwrap_or_default();
            under.covers = Some(kition_kit::Covers { fields: MATTE_COVERS.iter().map(|field| (*field).to_owned()).collect() });
        }
        // Nothing can be picked (the call is refused): what it would read, whole.
        Err(_) => {
            for object in &scene.objects {
                found.insert(On::Id(object.id), Need::all());
            }
            for material in &scene.materials {
                found.insert(On::Id(material.id), Need::of(&["name"]));
            }
        }
    }
    let premises: Vec<kition_kit::Premise> = found.iter().map(|(on, need)| need.premise(on)).collect();
    under.premises.retain(|premise| !premises.iter().any(|mine| mine.id == premise.id));
    under.premises.extend(premises);
    Ok(Some(under))
}

/// What the point a `compositor.pickMatte` call names met at the agent's basis
/// (`KitDocument::covered`, C8-scene-engine 5b): the scene with every asked object's values then
/// put back (where it was, whether hidden, its constraints, its data and modifiers; one created
/// since left out), and the ray cast again. An object whose mesh, hair or particle systems others
/// changed, or that they deleted, cannot be put back as it was (its mesh and hair are blobs this
/// document no longer holds): it answers `null`, and the guard refuses the write.
fn matte_covered(doc: &SceneDocument, params: &Value, objects: &[kition_kit::CoveredAsk]) -> Result<Option<kition_kit::CoveredAt>, String> {
    let mut before = doc.scene.clone();
    let id_of = |value: &Value| value.as_u64().or_else(|| value.as_str().and_then(|text| text.parse().ok())).and_then(|id| u32::try_from(id).ok());
    let mut unknown: Vec<Id> = Vec::new();
    for asked in objects {
        let id = id_of(&asked.id).ok_or_else(|| bad(format!("{} is no object id", asked.id)))?;
        if asked.created {
            before.objects.retain(|object| object.id != id);
            continue;
        }
        let was = &asked.before;
        let blobs = ["mesh", "hair", "particles"].iter().any(|field| was.contains_key(*field));
        let Some(object) = before.objects.iter_mut().find(|object| object.id == id).filter(|_| !asked.deleted && !blobs && asked.unknown.is_empty())
        else {
            // A material, a node group: no surface of its own (`covers` names none of theirs).
            if before.object(id).is_ok() || asked.deleted || blobs || !asked.unknown.is_empty() {
                unknown.push(id);
            }
            continue;
        };
        crate::place::put_back_place(object, was)?;
        if let Some(data) = was.get("data").filter(|_| !matches!(object.data, kition_scene_model::ObjectData::Mesh(_))) {
            object.data = serde_json::from_value(data.clone()).map_err(|error| bad(format!("data: {error}")))?;
        }
        if let Some(modifiers) = was.get("modifiers") {
            object.modifiers = if modifiers.is_null() {
                Vec::new()
            } else {
                serde_json::from_value(modifiers.clone()).map_err(|error| bad(format!("modifiers: {error}")))?
            };
        }
    }
    let hit = if unknown.is_empty() { matte_hit(doc, &before, params)? } else { None };
    let covered = objects
        .iter()
        .map(|asked| {
            let covered = match id_of(&asked.id) {
                Some(id) if !unknown.contains(&id) && unknown.is_empty() => Some(hit.is_some_and(|hit| crate::place::derives_from(&before, hit, id))),
                _ => None,
            };
            kition_kit::Covered { id: asked.id.clone(), covered, ..Default::default() }
        })
        .collect();
    Ok(Some(kition_kit::CoveredAt { under: hit.map(|id| vec![json!(id)]).unwrap_or_default(), covered }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_schema_is_well_formed() {
        for (id, _, _, _, schema) in super::ENTRIES {
            kition_kit::schema::validate(&schema()).unwrap_or_else(|error| panic!("{id}: {error}"));
        }
    }
}
