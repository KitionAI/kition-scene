//! glTF animations: object transforms out and in.
//!
//! Out, as the reference application's exporter does by default: every frame from an object's
//! first key to its last is sampled, and its node's translation, rotation, and scale are
//! written with linear interpolation at `frame / fps` seconds. Properties glTF cannot animate
//! without extensions (hidden, light power and colour, lens, material colour) stay out.
//!
//! In: each animated node's translation, rotation, and scale are read at every time any of its
//! samplers has, and turned into location, rotation (XYZ Euler, unwrapped so it does not jump
//! by whole turns), and scale keys relative to the object's parent. Linear and cubic-spline
//! samplers become linear keys at their times, step samplers constant keys.

use std::collections::{BTreeMap, HashMap};

use kition_scene_model::glam::{Mat3, Mat4, Quat, Vec3};
use kition_scene_model::scene::matrix_euler;
use kition_scene_model::{Id, Interpolation, Mesh, ObjectData, Property, Scene, SceneError};
use serde_json::{Value, json};

use super::{Document, FLOAT, View, Writer, is_oriented, read_u32, to_gltf_world, usize_of, y_up};
use crate::{ImportedChannel, ImportedObject, MAX_ITEMS, invalid};

/// The most samples one exported animation may write per node.
const MAX_SAMPLES: i64 = 100_000;

/// A node's local matrix in glTF space, as the exporter writes it.
fn local_matrix(scene: &Scene, id: Id, exported: &HashMap<Id, usize>) -> Result<Mat4, SceneError> {
    let object = scene.object(id)?;
    let world = to_gltf_world(scene.world_matrix(id)?, is_oriented(&object.data));
    let mut parent = object.parent;
    while let Some(at) = parent {
        if exported.contains_key(&at) {
            break;
        }
        parent = scene.object(at)?.parent;
    }
    Ok(match parent {
        Some(parent) => to_gltf_world(scene.world_matrix(parent)?, is_oriented(&scene.object(parent)?.data)).inverse() * world,
        None => world,
    })
}

fn transform_channel(property: Property) -> bool {
    matches!(property, Property::Location | Property::Rotation | Property::Scale)
}

impl Writer {
    pub(super) fn floats(&mut self, values: &[f32], kind: &str, components: usize, bounds: bool) -> usize {
        let mut bytes = Vec::with_capacity(values.len() * 4);
        for value in values {
            bytes.extend(value.to_le_bytes());
        }
        let view = self.view(&bytes, None);
        let mut accessor = json!({ "bufferView": view, "componentType": FLOAT, "count": values.len() / components.max(1), "type": kind });
        if bounds
            && let (Some(min), Some(max), Value::Object(map)) =
                (values.iter().copied().reduce(f32::min), values.iter().copied().reduce(f32::max), &mut accessor)
        {
            map.insert("min".into(), json!([min]));
            map.insert("max".into(), json!([max]));
        }
        self.accessors.push(accessor);
        self.accessors.len() - 1
    }
}

/// The scene's object animation as one glTF animation, or `None` when nothing exported moves.
pub(super) fn export(scene: &Scene, ids: &[Id], exported: &HashMap<Id, usize>, writer: &mut Writer) -> Result<Option<Value>, SceneError> {
    let animation = &scene.animation;
    // Constrained objects move with whatever their targets follow: they are sampled over every
    // key in the scene (their constraints baked in, as the reference application's exporter does).
    let everything = animation
        .channels
        .iter()
        .filter_map(|channel| Some((channel.keys.first()?.frame, channel.keys.last()?.frame)))
        .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)));
    // Driven objects follow their drivers' sources the same way.
    let constrained = |id: Id| {
        scene.object(id).is_ok_and(|object| !object.constraints.is_empty())
            || animation.channels.iter().any(|channel| channel.target == id && channel.driver.is_some() && transform_channel(channel.property))
    };
    let keyed = animation.channels.iter().any(|channel| transform_channel(channel.property));
    let follows = everything.is_some() && ids.iter().any(|id| constrained(*id));
    let simulated = ids.iter().any(|id| scene.simulated_range(*id).is_some());
    if !(keyed || follows || simulated) {
        return Ok(None);
    }
    let fps = animation.timeline.fps;
    // The frames each node moves over: its own keys, and those of ancestors between it and its
    // nearest exported ancestor (which the node's local matrix then carries).
    let mut ranges: BTreeMap<usize, (f32, f32)> = BTreeMap::new();
    for (node, id) in ids.iter().enumerate() {
        let mut current = Some(*id);
        let mut first = true;
        while let Some(at) = current {
            if !first && exported.contains_key(&at) {
                break;
            }
            first = false;
            // Simulated bodies move over their bake.
            if let Some(span) = scene.simulated_range(at) {
                let range = ranges.entry(node).or_insert(span);
                range.0 = range.0.min(span.0);
                range.1 = range.1.max(span.1);
            }
            if let (true, Some(span)) = (constrained(at), everything) {
                let range = ranges.entry(node).or_insert(span);
                range.0 = range.0.min(span.0);
                range.1 = range.1.max(span.1);
            }
            for channel in animation.channels.iter().filter(|channel| channel.target == at && transform_channel(channel.property)) {
                if let (Some(start), Some(end)) = (channel.keys.first(), channel.keys.last()) {
                    let range = ranges.entry(node).or_insert((start.frame, end.frame));
                    range.0 = range.0.min(start.frame);
                    range.1 = range.1.max(end.frame);
                }
            }
            current = scene.object(at)?.parent;
        }
    }
    if ranges.is_empty() {
        return Ok(None);
    }
    // A transform-only copy of the scene to step through the frames.
    let mut stepper = scene.clone();
    for object in &mut stepper.objects {
        if let ObjectData::Mesh(mesh) = &mut object.data {
            *mesh = Mesh::default();
        }
        object.modifiers.clear();
    }
    let mut channels = Vec::new();
    let mut samplers = Vec::new();
    for (node, (start, end)) in ranges {
        let Some(id) = ids.get(node).copied() else { continue };
        let (first, last) = (start.floor() as i64, end.ceil() as i64);
        if last - first + 1 > MAX_SAMPLES {
            return Err(invalid("an animation is too long to export"));
        }
        let mut times = Vec::new();
        let (mut translations, mut rotations, mut scales) = (Vec::new(), Vec::new(), Vec::new());
        let mut previous: Option<Quat> = None;
        for frame in first..=last {
            stepper.go_to_frame(frame as f32);
            let (scale, mut rotation, translation) = local_matrix(&stepper, id, exported)?.to_scale_rotation_translation();
            // Neighbouring samples on the same side of the quaternion double cover.
            if previous.is_some_and(|previous| previous.dot(rotation) < 0.0) {
                rotation = -rotation;
            }
            previous = Some(rotation);
            times.push(frame as f32 / fps);
            translations.extend(translation.to_array());
            rotations.extend(rotation.normalize().to_array());
            scales.extend(scale.to_array());
        }
        let input = writer.floats(&times, "SCALAR", 1, true);
        let varies = |values: &[f32], width: usize| {
            values.chunks(width).any(|chunk| values.get(..width).is_some_and(|first| chunk.iter().zip(first).any(|(a, b)| (a - b).abs() > 1e-6)))
        };
        for (path, values, kind, width) in
            [("translation", &translations, "VEC3", 3), ("rotation", &rotations, "VEC4", 4), ("scale", &scales, "VEC3", 3)]
        {
            if !varies(values, width) {
                continue;
            }
            let output = writer.floats(values, kind, width, false);
            samplers.push(json!({ "input": input, "output": output, "interpolation": "LINEAR" }));
            channels.push(json!({ "sampler": samplers.len() - 1, "target": { "node": node, "path": path } }));
        }
    }
    if channels.is_empty() {
        return Ok(None);
    }
    Ok(Some(json!([{ "name": "Animation", "channels": channels, "samplers": samplers }])))
}

/// One sampler: times and values (cubic-spline values without their tangents).
pub(super) struct Sampler {
    pub(super) times: Vec<f32>,
    pub(super) values: Vec<Vec<f32>>,
    pub(super) step: bool,
}

impl Sampler {
    /// The value at `time`: held before the first and after the last key, linear (or held, for
    /// step samplers) between; quaternions are blended and renormalised.
    pub(super) fn at(&self, time: f32) -> Option<Vec<f32>> {
        let first = self.values.first()?;
        let last = self.values.last()?;
        if time <= *self.times.first()? {
            return Some(first.clone());
        }
        if time >= *self.times.last()? {
            return Some(last.clone());
        }
        let slot = self.times.iter().rposition(|at| *at <= time)?;
        let (from, to) = (self.values.get(slot)?, self.values.get(slot + 1)?);
        if self.step {
            return Some(from.clone());
        }
        let (t0, t1) = (*self.times.get(slot)?, *self.times.get(slot + 1)?);
        let t = if t1 > t0 { (time - t0) / (t1 - t0) } else { 1.0 };
        if from.len() == 4 {
            let a = Quat::from_slice(from);
            let mut b = Quat::from_slice(to);
            if a.dot(b) < 0.0 {
                b = -b;
            }
            return Some(a.slerp(b, t).normalize().to_array().to_vec());
        }
        Some(from.iter().zip(to).map(|(a, b)| a + (b - a) * t).collect())
    }
}

fn read_floats(document: &Document<'_>, index: usize, components: usize) -> Result<Vec<Vec<f32>>, SceneError> {
    let View { data, stride, count, component, components: found } = document.accessor(index)?;
    if component != FLOAT || found != components {
        return Err(invalid(format!("animation accessor {index} is not float data of the right shape")));
    }
    let mut out = Vec::with_capacity(count);
    for element in 0..count {
        let at = element * stride;
        let mut value = Vec::with_capacity(components);
        for slot in 0..components {
            let number = read_u32(data, at + slot * 4).map(f32::from_bits).filter(|value| value.is_finite());
            value.push(number.ok_or_else(|| invalid(format!("animation accessor {index} holds a value that is not a finite number")))?);
        }
        out.push(value);
    }
    Ok(out)
}

/// A `weights` sampler: `components` scalars (one per morph target) for each time.
pub(super) fn weights_sampler(document: &Document<'_>, sampler: &Value, components: usize) -> Result<Sampler, SceneError> {
    let input = usize_of(sampler, "input").ok_or_else(|| invalid("an animation sampler has no input"))?;
    let output = usize_of(sampler, "output").ok_or_else(|| invalid("an animation sampler has no output"))?;
    let times: Vec<f32> = read_floats(document, input, 1)?.into_iter().filter_map(|value| value.first().copied()).collect();
    if times.windows(2).any(|pair| matches!(pair, [a, b] if b < a)) {
        return Err(invalid("animation times are not in order"));
    }
    let interpolation = sampler.get("interpolation").and_then(Value::as_str).unwrap_or("LINEAR");
    let scalars: Vec<f32> = read_floats(document, output, 1)?.into_iter().filter_map(|value| value.first().copied()).collect();
    let mut values: Vec<Vec<f32>> = scalars.chunks(components.max(1)).map(<[f32]>::to_vec).collect();
    if interpolation == "CUBICSPLINE" {
        values = values.chunks(3).filter_map(|triple| triple.get(1).cloned()).collect();
    }
    if values.len() != times.len() || times.is_empty() || values.iter().any(|value| value.len() != components.max(1)) {
        return Err(invalid("a weights sampler's times and values do not match"));
    }
    Ok(Sampler { times, values, step: interpolation == "STEP" })
}

pub(super) fn sampler(document: &Document<'_>, sampler: &Value, components: usize) -> Result<Sampler, SceneError> {
    let input = usize_of(sampler, "input").ok_or_else(|| invalid("an animation sampler has no input"))?;
    let output = usize_of(sampler, "output").ok_or_else(|| invalid("an animation sampler has no output"))?;
    let times: Vec<f32> = read_floats(document, input, 1)?.into_iter().filter_map(|value| value.first().copied()).collect();
    if times.windows(2).any(|pair| matches!(pair, [a, b] if b < a)) {
        return Err(invalid("animation times are not in order"));
    }
    let interpolation = sampler.get("interpolation").and_then(Value::as_str).unwrap_or("LINEAR");
    let mut values = read_floats(document, output, components)?;
    if interpolation == "CUBICSPLINE" {
        // In-tangent, value, out-tangent per key: keep the values.
        values = values.chunks(3).filter_map(|triple| triple.get(1).cloned()).collect();
    }
    if values.len() != times.len() || times.is_empty() {
        return Err(invalid("an animation sampler's times and values do not match"));
    }
    Ok(Sampler { times, values, step: interpolation == "STEP" })
}

/// Turns a node's local glTF matrix into its object's local matrix in the scene: both ends of
/// a parent relation are converted by the same change of axes, so only the node's own matrix
/// matters (cameras and lights keep looking down their own -Z).
fn scene_local(local: Mat4, oriented: bool, parent_oriented: Option<bool>) -> Mat4 {
    let c = y_up();
    match (parent_oriented, oriented) {
        (None, true) | (Some(false), true) => c.inverse() * local,
        (None, false) | (Some(false), false) => c.inverse() * local * c,
        (Some(true), true) => local,
        (Some(true), false) => local * c,
    }
}

/// Unwraps `angle` to the turn nearest `previous`.
fn unwrap(angle: f32, previous: f32) -> f32 {
    let turn = std::f32::consts::TAU;
    angle + ((previous - angle) / turn).round() * turn
}

/// The keyframes of the file's animations, for objects `node_of[i]` is the node of.
pub(super) fn import(document: &Document<'_>, node_of: &[usize], objects: &[ImportedObject]) -> Result<Vec<ImportedChannel>, SceneError> {
    let animations = document.list("animations");
    if animations.is_empty() {
        return Ok(Vec::new());
    }
    let nodes = document.list("nodes");
    let object_of: HashMap<usize, usize> = node_of.iter().enumerate().map(|(object, node)| (*node, object)).collect();
    // Per node: path -> sampler (the last animation that animates it wins).
    let mut animated: BTreeMap<usize, BTreeMap<String, Sampler>> = BTreeMap::new();
    let mut total = 0usize;
    for animation in animations {
        let samplers = animation.get("samplers").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
        for channel in animation.get("channels").and_then(Value::as_array).map_or(&[][..], Vec::as_slice) {
            let Some(target) = channel.get("target") else { continue };
            let (Some(node), Some(path)) = (usize_of(target, "node"), target.get("path").and_then(Value::as_str)) else { continue };
            let components = match path {
                "translation" | "scale" => 3,
                "rotation" => 4,
                _ => continue,
            };
            let index = usize_of(channel, "sampler").ok_or_else(|| invalid("an animation channel has no sampler"))?;
            let found =
                sampler(document, samplers.get(index).ok_or_else(|| invalid(format!("animation sampler {index} is not there")))?, components)?;
            total = total.saturating_add(found.times.len());
            if total > MAX_ITEMS {
                return Err(invalid("the file's animations are too large"));
            }
            animated.entry(node).or_default().insert(path.to_string(), found);
        }
    }
    let mut out = Vec::new();
    for (node, paths) in animated {
        let Some(&object) = object_of.get(&node) else { continue };
        let Some(imported) = objects.get(object) else { continue };
        // A local matrix with shear is held through the parent inverse; leave it still.
        if imported.parent_inverse != Mat4::IDENTITY {
            continue;
        }
        let rest = nodes.get(node).map(super::node_matrix).transpose()?.unwrap_or(Mat4::IDENTITY);
        let (rest_scale, rest_rotation, rest_translation) = rest.to_scale_rotation_translation();
        let oriented = is_oriented(&imported.data);
        let parent_oriented = imported.parent.and_then(|parent| objects.get(parent)).map(|parent| is_oriented(&parent.data));
        let mut times: Vec<f32> = paths.values().flat_map(|sampler| sampler.times.iter().copied()).collect();
        times.sort_by(f32::total_cmp);
        times.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        let step = paths.values().all(|sampler| sampler.step);
        let mut keys: Vec<[f32; 9]> = Vec::with_capacity(times.len());
        let mut previous: Option<Vec3> = None;
        for time in &times {
            let vec3 = |path: &str, rest: Vec3| paths.get(path).and_then(|sampler| sampler.at(*time)).map_or(rest, |value| Vec3::from_slice(&value));
            let rotation =
                paths.get("rotation").and_then(|sampler| sampler.at(*time)).map_or(rest_rotation, |value| Quat::from_slice(&value).normalize());
            let local = Mat4::from_scale_rotation_translation(vec3("scale", rest_scale), rotation, vec3("translation", rest_translation));
            let (scale, rotation, location) = scene_local(local, oriented, parent_oriented).to_scale_rotation_translation();
            let mut euler = matrix_euler(Mat3::from_quat(rotation));
            if let Some(previous) = previous {
                euler = Vec3::new(unwrap(euler.x, previous.x), unwrap(euler.y, previous.y), unwrap(euler.z, previous.z));
            }
            previous = Some(euler);
            keys.push([location.x, location.y, location.z, euler.x, euler.y, euler.z, scale.x, scale.y, scale.z]);
        }
        let interpolation = if step { Interpolation::Constant } else { Interpolation::Linear };
        for (slot, (property, index)) in [Property::Location, Property::Rotation, Property::Scale]
            .iter()
            .flat_map(|property| (0..3u8).map(move |index| (*property, index)))
            .enumerate()
        {
            let values: Vec<(f32, f32)> = times.iter().zip(&keys).filter_map(|(time, key)| Some((*time, *key.get(slot)?))).collect();
            let Some(&(_, first)) = values.first() else { continue };
            if values.iter().all(|(_, value)| (value - first).abs() < 1e-6) {
                continue;
            }
            out.push(ImportedChannel { object, property, index, path: String::new(), keys: values, interpolation });
        }
    }
    Ok(out)
}
