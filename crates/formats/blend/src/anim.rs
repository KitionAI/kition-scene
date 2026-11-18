//! Object animation: the keyframes of an object's action on its location, XYZ Euler rotation,
//! scale, and viewport visibility. Actions are read in both layouts the reference application
//! writes: layered actions (layers, strips, and one channel bag per slot) and the older flat
//! list of curves.

use kition_scene_exchange::ImportedChannel;
use kition_scene_model::{Interpolation, Property};

use crate::file::{Block, File, View};

/// The most curves and keys read from one action.
const MAX_CURVES: usize = 10_000;
const MAX_KEYS: usize = 100_000;

/// The blocks an array of pointers names (`count` of them).
fn pointed<'a>(file: &'a File, view: &View<'a>, field: &str, count: usize) -> Vec<&'a Block> {
    let Some(array) = view.target(field) else { return Vec::new() };
    file.bytes(array)
        .as_chunks::<8>()
        .0
        .iter()
        .take(count.min(MAX_CURVES))
        .filter_map(|bytes| file.block(u64::from_le_bytes(*bytes), array.index))
        .collect()
}

fn count(view: &View<'_>, field: &str) -> usize {
    usize::try_from(view.int(field).unwrap_or(0)).unwrap_or(0)
}

/// The curves of the action an object's animation data names, for the slot it uses.
fn curves<'a>(file: &'a File, animation: &View<'a>) -> Vec<View<'a>> {
    let Some(action) = animation.follow("action") else { return Vec::new() };
    let slot = animation.int("slot_handle");
    let mut out = Vec::new();
    // Layered: layers → strips → the strip's keyframe data → the slot's channel bag.
    let data = pointed(file, &action, "strip_keyframe_data_array", count(&action, "strip_keyframe_data_array_num"));
    for layer in pointed(file, &action, "layer_array", count(&action, "layer_array_num")) {
        let Some(layer) = file.view_in("ActionLayer", layer, 0) else { continue };
        for strip in pointed(file, &layer, "strip_array", count(&layer, "strip_array_num")) {
            let Some(strip) = file.view_in("ActionStrip", strip, 0) else { continue };
            let Some(keyframes) = usize::try_from(strip.int("data_index").unwrap_or(-1)).ok().and_then(|index| data.get(index)) else { continue };
            let Some(keyframes) = file.view_in("ActionStripKeyframeData", keyframes, 0) else { continue };
            for bag in pointed(file, &keyframes, "channelbag_array", count(&keyframes, "channelbag_array_num")) {
                let Some(bag) = file.view_in("ActionChannelBag", bag, 0) else { continue };
                if slot.is_some() && bag.int("slot_handle") != slot {
                    continue;
                }
                for curve in pointed(file, &bag, "fcurve_array", count(&bag, "fcurve_array_num")) {
                    if let Some(curve) = file.view_in("FCurve", curve, 0) {
                        out.push(curve);
                    }
                }
            }
        }
    }
    // The older flat list.
    let mut next = action.nested("curves").and_then(|list| list.target("first"));
    while let Some(block) = next {
        if out.len() >= MAX_CURVES {
            break;
        }
        let Some(curve) = file.view_in("FCurve", block, 0) else { break };
        next = curve.target("next");
        out.push(curve);
    }
    out
}

/// The keyframed channels of `object` (index `index` among the imported objects), with times in
/// seconds at `fps` frames a second. Rotations are read only for XYZ Euler objects.
pub fn read(file: &File, object: &View<'_>, index: usize, fps: f32) -> Vec<ImportedChannel> {
    let Some(animation) = object.follow("adt") else { return Vec::new() };
    let euler = object.int("rotmode").unwrap_or(1) == 1;
    let mut out = Vec::new();
    for curve in curves(file, &animation) {
        let path = curve
            .target("rna_path")
            .map(|block| {
                let bytes = file.bytes(block);
                let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(bytes.len());
                String::from_utf8_lossy(bytes.get(..end).unwrap_or_default()).into_owned()
            })
            .unwrap_or_default();
        let property = match path.as_str() {
            "location" => Property::Location,
            "rotation_euler" if euler => Property::Rotation,
            "scale" => Property::Scale,
            "hide_viewport" => Property::Hidden,
            _ => continue,
        };
        let Ok(component) = u8::try_from(curve.int("array_index").unwrap_or(0)) else { continue };
        if component >= property.components() {
            continue;
        }
        let total = usize::try_from(curve.int("totvert").unwrap_or(0)).unwrap_or(0).min(MAX_KEYS);
        let Some(block) = curve.target("bezt") else { continue };
        let mut keys = Vec::with_capacity(total);
        let mut interpolation = None;
        for slot in 0..total {
            let Some(key) = file.view_in("BezTriple", block, slot) else { break };
            let vec = key.floats("vec").unwrap_or_default();
            let (Some(frame), Some(value)) = (vec.get(3).copied(), vec.get(4).copied()) else { continue };
            if !(frame.is_finite() && value.is_finite()) {
                continue;
            }
            // One interpolation a channel: the first key's.
            interpolation.get_or_insert(match key.int("ipo").unwrap_or(2) {
                0 => Interpolation::Constant,
                1 => Interpolation::Linear,
                _ => Interpolation::Bezier,
            });
            keys.push((frame / fps, value));
        }
        keys.sort_by(|a, b| a.0.total_cmp(&b.0));
        if keys.is_empty() {
            continue;
        }
        let interpolation = if property == Property::Hidden { Interpolation::Constant } else { interpolation.unwrap_or_default() };
        out.push(ImportedChannel { object: index, property, index: component, path: String::new(), keys, interpolation });
    }
    out
}
