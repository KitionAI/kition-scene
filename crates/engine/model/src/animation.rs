//! Keyframe animation: a timeline (frame range, frame rate, current frame) and channels of
//! keyframes, each animating one number of an object, its light or camera, or a material.
//!
//! Curves follow the reference application's defaults: Bézier interpolation with automatic,
//! clamped handles. Handles reach a third of the way to the neighbouring key; a key between two
//! others gets the slope `1.5 · (b·s₀ + a·s₁) / (a + b)` (where `a`, `b` are the frame gaps to
//! its neighbours and `s₀`, `s₁` the slopes towards them), the first and last keys and local
//! extremes are flat, and a handle never passes its neighbour's value (when one would, both
//! handles of the key take the smaller slope). Linear and constant interpolation are kept per
//! key, for the segment that starts at it. Before the first key and after the last the curve
//! holds its end value.
//!
//! Objects in the scene hold their values at the current frame (as the reference application's
//! objects do): changing the frame writes every animated value; editing a value without keying
//! it changes it only until the frame changes.
//!
//! A channel can carry a driver instead ([`crate::drivers`]): its value comes from other values,
//! and its keys (if any) map the driver's value. Actions are named sets of channels not tied to
//! an object, which the Action constraint plays and objects can take on; keying sets name what
//! a keyframe insertion keys.

use serde::{Deserialize, Serialize};

use crate::error::{Result, SceneError};
use crate::fmodifier::{self, FModifier};
use crate::scene::{Id, ObjectData, Scene};

/// The scene's frame range, frame rate, and current frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Timeline {
    pub start: i32,
    pub end: i32,
    /// Frames per second.
    pub fps: f32,
    /// The frame the scene's objects are at.
    pub frame: i32,
}

impl Default for Timeline {
    fn default() -> Self {
        Self { start: 1, end: 250, fps: 24.0, frame: 1 }
    }
}

/// The largest frame number.
pub const MAX_FRAME: i32 = 1_048_574;

/// The most modifiers on one channel.
pub const MAX_MODIFIERS: usize = 64;

/// How a curve runs from a key to the next.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Interpolation {
    #[default]
    Bezier,
    Linear,
    Constant,
}

impl Interpolation {
    /// The interpolation named `name`.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "bezier" => Some(Self::Bezier),
            "linear" => Some(Self::Linear),
            "constant" => Some(Self::Constant),
            _ => None,
        }
    }
}

/// What a channel animates. Each property has one channel per component (`index`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Property {
    /// An object's location, X Y Z.
    Location,
    /// An object's rotation, XYZ Euler, radians.
    Rotation,
    /// An object's scale.
    Scale,
    /// An object is hidden (1) or shown (0); always constant.
    Hidden,
    /// A light's power.
    Power,
    /// A light's colour, R G B.
    Color,
    /// A camera's focal length.
    Lens,
    /// A material's base colour, R G B A.
    BaseColor,
    /// A pose bone's location in its rest space, X Y Z (the channel's `path` names the bone).
    BoneLocation,
    /// A pose bone's rotation, XYZ Euler, radians.
    BoneRotation,
    /// A pose bone's scale.
    BoneScale,
    /// A mesh's shape key value, 0 to 1 (the channel's `path` names the key).
    ShapeKey,
}

impl Property {
    /// Every property, in the order inspect lists them.
    pub const ALL: [Property; 8] = [Self::Location, Self::Rotation, Self::Scale, Self::Hidden, Self::Power, Self::Color, Self::Lens, Self::BaseColor];

    /// The properties of bones and shape keys, which name their bone or key in `path`.
    pub const NAMED: [Property; 4] = [Self::BoneLocation, Self::BoneRotation, Self::BoneScale, Self::ShapeKey];

    /// The property named `name` (`location`, `baseColor`, `boneRotation`, ...).
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().chain(Self::NAMED).find(|property| property.name() == name)
    }

    /// Whether the channel names a bone or a shape key in its `path`.
    pub fn named(self) -> bool {
        Self::NAMED.contains(&self)
    }

    /// The name commands and inspect use.
    pub fn name(self) -> &'static str {
        match self {
            Self::Location => "location",
            Self::Rotation => "rotation",
            Self::Scale => "scale",
            Self::Hidden => "hidden",
            Self::Power => "power",
            Self::Color => "color",
            Self::Lens => "lens",
            Self::BaseColor => "baseColor",
            Self::BoneLocation => "boneLocation",
            Self::BoneRotation => "boneRotation",
            Self::BoneScale => "boneScale",
            Self::ShapeKey => "shapeKey",
        }
    }

    /// How many components the property has.
    pub fn components(self) -> u8 {
        match self {
            Self::Location | Self::Rotation | Self::Scale | Self::Color | Self::BoneLocation | Self::BoneRotation | Self::BoneScale => 3,
            Self::BaseColor => 4,
            Self::Hidden | Self::Power | Self::Lens | Self::ShapeKey => 1,
        }
    }

    /// Whether the property belongs to the object's material rather than the object.
    pub fn on_material(self) -> bool {
        self == Self::BaseColor
    }
}

/// One keyframe.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Keyframe {
    pub frame: f32,
    pub value: f32,
    /// How the curve runs to the next key.
    #[serde(default)]
    pub interpolation: Interpolation,
}

/// The keyframes of one component of one property, or its driver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    /// The object, or for material properties the material.
    pub target: Id,
    pub property: Property,
    pub index: u8,
    /// The bone or shape key a named property belongs to; empty otherwise.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
    /// Sorted by frame, one key per frame. A driven channel's keys map the driver's value (and
    /// may be none).
    #[serde(default)]
    pub keys: Vec<Keyframe>,
    /// Works the value out from other values instead of the frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<crate::drivers::Driver>,
    /// F-curve modifiers, in stack order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<FModifier>,
}

/// One component of a property in an action, which names no object: bone and shape key
/// properties name theirs in `path`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionChannel {
    pub property: Property,
    pub index: u8,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
    pub keys: Vec<Keyframe>,
    /// F-curve modifiers, in stack order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<FModifier>,
}

/// A named set of channels any object can play (stashed from an object, taken on by one, or
/// played by an Action constraint).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    pub name: String,
    #[serde(default)]
    pub channels: Vec<ActionChannel>,
}

impl Action {
    /// The value of a component at `frame`, if the action keys it.
    pub fn value(&self, property: Property, index: u8, path: &str, frame: f32) -> Option<f32> {
        let channel = self.channels.iter().find(|channel| channel.property == property && channel.index == index && channel.path == path)?;
        evaluate_keys(&channel.keys, &channel.modifiers, property, frame)
    }

    /// The transform the action gives at `frame`: an object's (`bone` none) or a bone's pose,
    /// unkeyed components at rest.
    pub fn matrix_at(&self, frame: f32, bone: Option<&str>) -> glam::Mat4 {
        let (properties, path) = match bone {
            None => ([Property::Location, Property::Rotation, Property::Scale], ""),
            Some(name) => ([Property::BoneLocation, Property::BoneRotation, Property::BoneScale], name),
        };
        let read =
            |property: Property, rest: f32| glam::Vec3::from_array([0u8, 1, 2].map(|index| self.value(property, index, path, frame).unwrap_or(rest)));
        let (location, rotation, scale) = (read(properties[0], 0.0), read(properties[1], 0.0), read(properties[2], 1.0));
        glam::Mat4::from_translation(location) * glam::Mat4::from_mat3(crate::scene::euler_matrix(rotation)) * glam::Mat4::from_scale(scale)
    }

    /// The first and last keyed frames.
    pub fn range(&self) -> Option<(f32, f32)> {
        self.channels
            .iter()
            .filter_map(|channel| Some((channel.keys.first()?.frame, channel.keys.last()?.frame)))
            .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
    }
}

/// The keying sets every scene has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BuiltinKeyingSet {
    Location,
    Rotation,
    Scale,
    LocRot,
    LocScale,
    LocRotScale,
    RotScale,
    /// Every property that already has keys.
    Available,
    /// Every bone of an armature: location, rotation, and scale.
    WholeCharacter,
}

impl BuiltinKeyingSet {
    pub const ALL: [Self; 9] = [
        Self::Location,
        Self::Rotation,
        Self::Scale,
        Self::LocRot,
        Self::LocScale,
        Self::LocRotScale,
        Self::RotScale,
        Self::Available,
        Self::WholeCharacter,
    ];

    /// The name commands use.
    pub fn name(self) -> &'static str {
        match self {
            Self::Location => "location",
            Self::Rotation => "rotation",
            Self::Scale => "scale",
            Self::LocRot => "locRot",
            Self::LocScale => "locScale",
            Self::LocRotScale => "locRotScale",
            Self::RotScale => "rotScale",
            Self::Available => "available",
            Self::WholeCharacter => "wholeCharacter",
        }
    }

    /// The transform properties the set keys (none for Available and Whole Character).
    pub fn transforms(self) -> &'static [Property] {
        match self {
            Self::Location => &[Property::Location],
            Self::Rotation => &[Property::Rotation],
            Self::Scale => &[Property::Scale],
            Self::LocRot => &[Property::Location, Property::Rotation],
            Self::LocScale => &[Property::Location, Property::Scale],
            Self::LocRotScale => &[Property::Location, Property::Rotation, Property::Scale],
            Self::RotScale => &[Property::Rotation, Property::Scale],
            Self::Available | Self::WholeCharacter => &[],
        }
    }
}

/// One property a keying set keys: every component of `property` of `target` (and `path`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyingPath {
    pub target: Id,
    pub property: Property,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
}

/// A keying set of the scene's own: named properties of named objects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyingSet {
    pub name: String,
    #[serde(default)]
    pub paths: Vec<KeyingPath>,
}

/// A keying set: one every scene has, or one of the scene's own by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyingSetRef {
    Builtin(BuiltinKeyingSet),
    Custom(String),
}

/// The scene's keying sets and the active one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyingSets {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sets: Vec<KeyingSet>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<KeyingSetRef>,
}

impl KeyingSets {
    pub fn is_empty(&self) -> bool {
        self.sets.is_empty() && self.active.is_none()
    }
}

/// The scene's timeline, every channel, its actions, and its keying sets.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Animation {
    #[serde(default)]
    pub timeline: Timeline,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<Channel>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
    #[serde(default, skip_serializing_if = "KeyingSets::is_empty")]
    pub keying_sets: KeyingSets,
}

impl Animation {
    /// Whether nothing differs from a new scene's (so the file can leave it out).
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The action called `name`.
    pub fn action(&self, name: &str) -> Option<&Action> {
        self.actions.iter().find(|action| action.name == name)
    }
}

/// The neighbours of key `slot`; on a cyclic curve the first and last keys see the keys across
/// the cycle, moved by the cycle's period and change in value.
fn neighbours(keys: &[Keyframe], slot: usize, cyclic: bool) -> (Option<Keyframe>, Option<Keyframe>) {
    let mut previous = slot.checked_sub(1).and_then(|before| keys.get(before)).copied();
    let mut next = keys.get(slot + 1).copied();
    if cyclic
        && keys.len() >= 2
        && let (Some(first), Some(last)) = (keys.first(), keys.last())
    {
        let (period, change) = (last.frame - first.frame, last.value - first.value);
        if slot == 0 {
            previous = keys.len().checked_sub(2).and_then(|at| keys.get(at)).map(|key| Keyframe {
                frame: key.frame - period,
                value: key.value - change,
                ..*key
            });
        }
        if slot + 1 == keys.len() {
            next = keys.get(1).map(|key| Keyframe { frame: key.frame + period, value: key.value + change, ..*key });
        }
    }
    (previous, next)
}

/// The handles of key `slot`: (left slope, right slope). The x-reach of each handle is a third
/// of the gap on its side.
fn slopes(keys: &[Keyframe], slot: usize, cyclic: bool) -> (f32, f32) {
    let (Some(previous), Some(current), Some(next)) = (neighbours(keys, slot, cyclic).0, keys.get(slot), neighbours(keys, slot, cyclic).1) else {
        return (0.0, 0.0);
    };
    let (a, b) = (current.frame - previous.frame, next.frame - current.frame);
    if a <= 0.0 || b <= 0.0 {
        return (0.0, 0.0);
    }
    let (s0, s1) = ((current.value - previous.value) / a, (next.value - current.value) / b);
    // A local extreme (or a flat side) stays flat.
    if s0 * s1 <= 0.0 {
        return (0.0, 0.0);
    }
    let mut slope = 1.5 * (b * s0 + a * s1) / (a + b);
    // Neither handle may pass its neighbour's value.
    let reach_left = (current.value - previous.value).abs() / (a / 3.0);
    let reach_right = (next.value - current.value).abs() / (b / 3.0);
    let limit = reach_left.min(reach_right);
    if slope.abs() > limit {
        slope = limit.copysign(slope);
    }
    (slope, slope)
}

/// The value of keys sorted by frame at `frame` (held before the first and after the last);
/// `cyclic` keys smooth their ends across the cycle.
pub fn curve_value(keys: &[Keyframe], property: Property, frame: f32, cyclic: bool) -> Option<f32> {
    let first = keys.first()?;
    let last = keys.last()?;
    if frame <= first.frame || frame.is_nan() {
        return Some(first.value);
    }
    if frame >= last.frame {
        return Some(last.value);
    }
    let slot = keys.iter().rposition(|key| key.frame <= frame)?;
    let (Some(from), Some(to)) = (keys.get(slot), keys.get(slot + 1)) else { return Some(last.value) };
    let gap = to.frame - from.frame;
    if gap <= 0.0 {
        return Some(to.value);
    }
    let t = (frame - from.frame) / gap;
    if property == Property::Hidden {
        return Some(from.value);
    }
    Some(match from.interpolation {
        Interpolation::Constant => from.value,
        Interpolation::Linear => from.value + (to.value - from.value) * t,
        Interpolation::Bezier => {
            // Handle x-reaches of a third of the gap make x linear in t.
            let (_, out_slope) = slopes(keys, slot, cyclic);
            let (in_slope, _) = slopes(keys, slot + 1, cyclic);
            let p1 = from.value + out_slope * gap / 3.0;
            let p2 = to.value - in_slope * gap / 3.0;
            let u = 1.0 - t;
            u * u * u * from.value + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * to.value
        }
    })
}

/// The value of a curve with modifiers at `frame`, or `None` when it has no keys.
pub fn evaluate_keys(keys: &[Keyframe], modifiers: &[FModifier], property: Property, frame: f32) -> Option<f32> {
    let (first, last) = (keys.first()?, keys.last()?);
    if modifiers.is_empty() {
        return curve_value(keys, property, frame, false);
    }
    let span = fmodifier::Span { first_frame: first.frame, last_frame: last.frame, first_value: first.value, last_value: last.value };
    let (time, offset) = fmodifier::time(modifiers, Some(span), frame);
    let value = curve_value(keys, property, time, fmodifier::cyclic(modifiers))? + offset;
    Some(fmodifier::value(modifiers, time, value))
}

impl Channel {
    /// The channel's value at `frame`, or `None` when it has no keys.
    pub fn evaluate(&self, frame: f32) -> Option<f32> {
        evaluate_keys(&self.keys, &self.modifiers, self.property, frame)
    }

    /// Adds or replaces the key at `frame`.
    pub fn insert(&mut self, frame: f32, value: f32, interpolation: Interpolation) {
        match self.keys.iter().position(|key| key.frame >= frame - 1e-4) {
            Some(slot) if self.keys.get(slot).is_some_and(|key| (key.frame - frame).abs() <= 1e-4) => {
                if let Some(key) = self.keys.get_mut(slot) {
                    key.value = value;
                }
            }
            Some(slot) => self.keys.insert(slot, Keyframe { frame, value, interpolation }),
            None => self.keys.push(Keyframe { frame, value, interpolation }),
        }
    }
}

/// Whether an object's channel is one of its own keys the NLA plays (its active action): keyed,
/// not driven, not a shape key's (shape keys animate outside the object's NLA).
pub fn in_nla(channel: &Channel) -> bool {
    channel.driver.is_none() && channel.property != Property::ShapeKey && !channel.property.on_material()
}

/// A number of a property, read from the scene now.
pub fn read(scene: &Scene, target: Id, property: Property, index: u8) -> Option<f32> {
    read_at(scene, target, property, index, "")
}

/// A number of a property of a bone or shape key `path` (empty for other properties).
pub fn read_at(scene: &Scene, target: Id, property: Property, index: u8, path: &str) -> Option<f32> {
    if property.named() {
        return crate::armature::read_named(scene, target, property, index, path);
    }
    let slot = usize::from(index);
    if property.on_material() {
        return scene.material(target).ok()?.base_color.get(slot).copied();
    }
    let object = scene.object(target).ok()?;
    match (property, &object.data) {
        (Property::Location, _) => object.location.to_array().get(slot).copied(),
        (Property::Rotation, _) => object.rotation.to_array().get(slot).copied(),
        (Property::Scale, _) => object.scale.to_array().get(slot).copied(),
        (Property::Hidden, _) => Some(if object.hidden { 1.0 } else { 0.0 }),
        (Property::Power, ObjectData::Light(light)) => Some(light.power),
        (Property::Color, ObjectData::Light(light)) => light.color.to_array().get(slot).copied(),
        (Property::Lens, ObjectData::Camera(camera)) => Some(camera.lens),
        _ => None,
    }
}

/// Writes a number of a property. Returns whether the target has the property.
pub fn write(scene: &mut Scene, target: Id, property: Property, index: u8, value: f32) -> bool {
    write_at(scene, target, property, index, "", value)
}

/// Writes a number of a property of a bone or shape key `path` (empty for other properties).
pub fn write_at(scene: &mut Scene, target: Id, property: Property, index: u8, path: &str, value: f32) -> bool {
    if property.named() {
        return crate::armature::write_named(scene, target, property, index, path, value);
    }
    let slot = usize::from(index);
    if property.on_material() {
        return scene
            .material_mut(target)
            .ok()
            .and_then(|material| material.base_color.get_mut(slot))
            .map(|component| *component = value.clamp(0.0, 1.0))
            .is_some();
    }
    let Ok(object) = scene.object_mut(target) else { return false };
    let set = |vector: &mut glam::Vec3| {
        let mut array = vector.to_array();
        let found = array.get_mut(slot).map(|component| *component = value).is_some();
        *vector = glam::Vec3::from_array(array);
        found
    };
    match (property, &mut object.data) {
        (Property::Location, _) => set(&mut object.location),
        (Property::Rotation, _) => set(&mut object.rotation),
        (Property::Scale, _) => set(&mut object.scale),
        (Property::Hidden, _) => {
            object.hidden = value >= 0.5;
            true
        }
        (Property::Power, ObjectData::Light(light)) => {
            light.power = value.max(0.0);
            true
        }
        (Property::Color, ObjectData::Light(light)) => set(&mut light.color),
        (Property::Lens, ObjectData::Camera(camera)) => {
            camera.lens = value.clamp(1.0, 5000.0);
            true
        }
        _ => false,
    }
}

impl Scene {
    /// Writes every animated value at `frame` into the scene, and makes it the current frame
    /// when it is a whole frame.
    pub fn go_to_frame(&mut self, frame: f32) {
        let layered: std::collections::BTreeSet<Id> = self.objects.iter().filter(|object| !object.nla.is_empty()).map(|object| object.id).collect();
        let mut values: Vec<(Id, Property, u8, String, f32)> = self
            .animation
            .channels
            .iter()
            .filter(|channel| channel.driver.is_none() && !(layered.contains(&channel.target) && in_nla(channel)))
            .filter_map(|channel| Some((channel.target, channel.property, channel.index, channel.path.clone(), channel.evaluate(frame)?)))
            .collect();
        for object in self.objects.iter().filter(|object| layered.contains(&object.id)) {
            let own: Vec<&Channel> = self.animation.channels.iter().filter(|channel| channel.target == object.id && in_nla(channel)).collect();
            values.extend(
                object
                    .nla
                    .evaluate(&own, &self.animation.actions, frame)
                    .into_iter()
                    .map(|(property, index, path, value)| (object.id, property, index, path, value)),
            );
        }
        for (target, property, index, path, value) in values {
            write_at(self, target, property, index, &path, value);
        }
        if frame.is_finite() {
            self.apply_drivers(frame);
        }
        if frame.fract() == 0.0 && frame.is_finite() {
            self.animation.timeline.frame = (frame as i32).clamp(-MAX_FRAME, MAX_FRAME);
        }
        if frame.is_finite() {
            self.physics_cache.frame = Some(frame);
        }
    }

    /// Every property the frame sets: keyed and NLA-animated ones (drivers' too), as
    /// (target, property, component, bone or shape key).
    pub fn animated_paths(&self) -> Vec<(Id, Property, u8, String)> {
        let mut paths: Vec<(Id, Property, u8, String)> =
            self.animation.channels.iter().map(|channel| (channel.target, channel.property, channel.index, channel.path.clone())).collect();
        for object in self.objects.iter().filter(|object| !object.nla.is_empty()) {
            let own: Vec<&Channel> = self.animation.channels.iter().filter(|channel| channel.target == object.id && in_nla(channel)).collect();
            for (property, index, path, _) in object.nla.evaluate(&own, &self.animation.actions, self.animation.timeline.frame as f32) {
                let entry = (object.id, property, index, path);
                if !paths.contains(&entry) {
                    paths.push(entry);
                }
            }
        }
        paths
    }

    /// The scene as it is at `frame` (an evaluated copy).
    pub fn at_frame(&self, frame: f32) -> Scene {
        let mut scene = self.clone();
        scene.go_to_frame(frame);
        scene
    }

    /// The channel of a target's property component, if keyed.
    pub fn channel(&self, target: Id, property: Property, index: u8) -> Option<&Channel> {
        self.channel_at(target, property, index, "")
    }

    /// The channel of a property component of bone or shape key `path`, if keyed.
    pub fn channel_at(&self, target: Id, property: Property, index: u8, path: &str) -> Option<&Channel> {
        self.animation
            .channels
            .iter()
            .find(|channel| channel.target == target && channel.property == property && channel.index == index && channel.path == path)
    }

    /// Keys every component of `property` of `target` at `frame` with its value now. Returns
    /// how many keys were set.
    pub fn insert_key(&mut self, target: Id, property: Property, frame: f32) -> Result<usize> {
        self.insert_key_at(target, property, "", frame)
    }

    /// [`Scene::insert_key`] for a property of bone or shape key `path`.
    pub fn insert_key_at(&mut self, target: Id, property: Property, path: &str, frame: f32) -> Result<usize> {
        if !frame.is_finite() || frame.abs() > MAX_FRAME as f32 {
            return Err(SceneError::params("the frame is out of range"));
        }
        // In tweak mode a key goes at the strip's action frame.
        let frame = if property.on_material() || property == Property::ShapeKey { frame } else { self.key_time(target, frame) };
        if !frame.is_finite() || frame.abs() > MAX_FRAME as f32 {
            return Err(SceneError::params("the frame is out of range"));
        }
        let mut count = 0;
        for index in 0..property.components() {
            let Some(value) = read_at(self, target, property, index, path) else {
                return Err(SceneError::params(format!("this has no {} to key", property.name())));
            };
            let interpolation = if property == Property::Hidden { Interpolation::Constant } else { Interpolation::Bezier };
            if self.driver_of(target, property, index, path).is_some() {
                return Err(SceneError::params(format!("a driver sets this {}; it takes no keys", property.name())));
            }
            match self
                .animation
                .channels
                .iter_mut()
                .find(|channel| channel.target == target && channel.property == property && channel.index == index && channel.path == path)
            {
                Some(channel) => channel.insert(frame, value, interpolation),
                None => self.animation.channels.push(Channel {
                    target,
                    property,
                    index,
                    path: path.to_string(),
                    keys: vec![Keyframe { frame, value, interpolation }],
                    driver: None,
                    modifiers: Vec::new(),
                }),
            }
            count += 1;
        }
        Ok(count)
    }

    /// Keys what keying set `set` names at `frame`: for a set every scene has, the given objects
    /// (or, with `bones`, those bones of an armature); for one of the scene's own, its paths.
    /// Returns how many keys were set.
    pub fn insert_keying_set(&mut self, set: &KeyingSetRef, objects: &[Id], bones: Option<(Id, &[String])>, frame: f32) -> Result<usize> {
        let bone_property = |property: Property| match property {
            Property::Location => Property::BoneLocation,
            Property::Rotation => Property::BoneRotation,
            _ => Property::BoneScale,
        };
        let mut wanted: Vec<(Id, Property, String)> = Vec::new();
        match set {
            KeyingSetRef::Custom(name) => {
                let set = self
                    .animation
                    .keying_sets
                    .sets
                    .iter()
                    .find(|set| &set.name == name)
                    .ok_or_else(|| SceneError::params(format!("there is no keying set {name}")))?;
                wanted.extend(set.paths.iter().map(|path| (path.target, path.property, path.path.clone())));
            }
            KeyingSetRef::Builtin(BuiltinKeyingSet::Available) => {
                let mine = |channel: &&Channel| {
                    channel.driver.is_none()
                        && match bones {
                            Some((rig, names)) => channel.target == rig && names.contains(&channel.path),
                            None => objects.contains(&channel.target) && !channel.property.named(),
                        }
                };
                for channel in self.animation.channels.iter().filter(mine) {
                    let entry = (channel.target, channel.property, channel.path.clone());
                    if !wanted.contains(&entry) {
                        wanted.push(entry);
                    }
                }
            }
            KeyingSetRef::Builtin(BuiltinKeyingSet::WholeCharacter) => {
                let rigs: Vec<Id> = match bones {
                    Some((rig, _)) => vec![rig],
                    None => objects.iter().copied().filter(|id| crate::armature::armature_of(self, *id).is_some()).collect(),
                };
                for rig in rigs {
                    let names: Vec<String> = crate::armature::armature_of(self, rig)
                        .map(|armature| armature.bones.iter().map(|bone| bone.name.clone()).collect())
                        .unwrap_or_default();
                    for name in names {
                        for property in [Property::BoneLocation, Property::BoneRotation, Property::BoneScale] {
                            wanted.push((rig, property, name.clone()));
                        }
                    }
                }
            }
            KeyingSetRef::Builtin(builtin) => match bones {
                Some((rig, names)) => {
                    for name in names {
                        wanted.extend(builtin.transforms().iter().map(|property| (rig, bone_property(*property), name.clone())));
                    }
                }
                None => {
                    for object in objects {
                        wanted.extend(builtin.transforms().iter().map(|property| (*object, *property, String::new())));
                    }
                }
            },
        }
        if wanted.is_empty() {
            return Err(SceneError::params("the keying set keys nothing here"));
        }
        let mut count = 0;
        for (target, property, path) in wanted {
            count += self.insert_key_at(target, property, &path, frame)?;
        }
        Ok(count)
    }

    /// Removes the keys of `property` of `target` at `frame`. Returns how many were removed.
    pub fn delete_key(&mut self, target: Id, property: Property, frame: f32) -> usize {
        self.delete_key_at(target, property, None, frame)
    }

    /// [`Scene::delete_key`] for one bone or shape key `path` (every one when `None`).
    pub fn delete_key_at(&mut self, target: Id, property: Property, path: Option<&str>, frame: f32) -> usize {
        let frame = if property.on_material() || property == Property::ShapeKey { frame } else { self.key_time(target, frame) };
        let mut count = 0;
        for channel in self.animation.channels.iter_mut().filter(|channel| {
            channel.driver.is_none() && channel.target == target && channel.property == property && path.is_none_or(|path| channel.path == path)
        }) {
            let before = channel.keys.len();
            channel.keys.retain(|key| (key.frame - frame).abs() > 1e-4);
            count += before - channel.keys.len();
        }
        self.animation.channels.retain(|channel| !channel.keys.is_empty() || channel.driver.is_some());
        count
    }

    /// Removes every keyed channel of `target` (all properties, or one); drivers stay.
    pub fn clear_animation(&mut self, target: Id, property: Option<Property>) -> usize {
        let before = self.animation.channels.len();
        self.animation.channels.retain(|channel| {
            channel.driver.is_some() || !(channel.target == target && property.is_none_or(|property| channel.property == property))
        });
        before - self.animation.channels.len()
    }

    /// Drops channels whose targets are gone (or whose bone or shape key is).
    pub fn prune_animation(&mut self) {
        let objects: std::collections::BTreeSet<Id> = self.objects.iter().map(|object| object.id).collect();
        let materials: std::collections::BTreeSet<Id> = self.materials.iter().map(|material| material.id).collect();
        self.animation
            .channels
            .retain(|channel| if channel.property.on_material() { materials.contains(&channel.target) } else { objects.contains(&channel.target) });
        let gone: Vec<usize> = self
            .animation
            .channels
            .iter()
            .enumerate()
            .filter(|(_, channel)| {
                channel.property.named() && read_at(self, channel.target, channel.property, channel.index, &channel.path).is_none()
            })
            .map(|(slot, _)| slot)
            .collect();
        for slot in gone.into_iter().rev() {
            if slot < self.animation.channels.len() {
                self.animation.channels.remove(slot);
            }
        }
        // Drivers and keying sets let go of objects that are gone.
        let missing: Vec<Id> = self
            .animation
            .channels
            .iter()
            .filter_map(|channel| channel.driver.as_ref())
            .flat_map(|driver| driver.variables.iter().flat_map(|variable| variable.source.objects()))
            .filter(|id| !objects.contains(id) && !materials.contains(id))
            .collect();
        if !missing.is_empty() {
            for driver in self.animation.channels.iter_mut().filter_map(|channel| channel.driver.as_mut()) {
                for variable in &mut driver.variables {
                    variable.source.forget(&missing);
                }
            }
        }
        for set in &mut self.animation.keying_sets.sets {
            set.paths.retain(|path| objects.contains(&path.target) || materials.contains(&path.target));
        }
    }

    /// Copies `from`'s object channels to `to` (a duplicate).
    pub fn copy_animation(&mut self, from: Id, to: Id) {
        let copies: Vec<Channel> = self
            .animation
            .channels
            .iter()
            .filter(|channel| channel.target == from && !channel.property.on_material())
            .map(|channel| Channel { target: to, ..channel.clone() })
            .collect();
        self.animation.channels.extend(copies);
    }

    /// The frames that have a key on any of `targets` (all objects and materials when empty),
    /// sorted.
    pub fn key_frames(&self, targets: &[Id]) -> Vec<f32> {
        let mut frames: Vec<f32> = self
            .animation
            .channels
            .iter()
            .filter(|channel| targets.is_empty() || targets.contains(&channel.target))
            .flat_map(|channel| {
                // A tweaked strip's keys show at their scene frames.
                let strip = if in_nla(channel) { self.tweaked_strip(channel.target) } else { None };
                channel.keys.iter().map(move |key| strip.map_or(key.frame, |strip| strip.scene_time(key.frame)))
            })
            .collect();
        frames.sort_by(f32::total_cmp);
        frames.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
        frames
    }

    /// Checks the timeline and channels of a scene read from a file.
    pub fn validate_animation(&self) -> Result<()> {
        let timeline = &self.animation.timeline;
        if !(timeline.fps.is_finite() && timeline.fps > 0.0 && timeline.fps <= 1000.0) {
            return Err(SceneError::InvalidFile("the frame rate is out of range".into()));
        }
        if timeline.start > timeline.end || timeline.start.abs() > MAX_FRAME || timeline.end.abs() > MAX_FRAME || timeline.frame.abs() > MAX_FRAME {
            return Err(SceneError::InvalidFile("the frame range is out of range".into()));
        }
        let mut seen = std::collections::BTreeSet::new();
        for channel in &self.animation.channels {
            if channel.index >= channel.property.components()
                || channel.property.named() == channel.path.is_empty()
                || !seen.insert((channel.target, channel.property, channel.index, channel.path.clone()))
            {
                return Err(SceneError::InvalidFile(format!("a {} channel is repeated or out of range", channel.property.name())));
            }
            if read_at(self, channel.target, channel.property, channel.index, &channel.path).is_none() {
                return Err(SceneError::InvalidFile(format!("a {} channel names something that does not have it", channel.property.name())));
            }
            if (channel.keys.is_empty() && channel.driver.is_none())
                || channel.keys.iter().any(|key| !(key.frame.is_finite() && key.value.is_finite()) || key.frame.abs() > MAX_FRAME as f32)
            {
                return Err(SceneError::InvalidFile("a keyframe is not a finite number".into()));
            }
            if channel.keys.windows(2).any(|pair| matches!(pair, [a, b] if b.frame <= a.frame)) {
                return Err(SceneError::InvalidFile("keyframes are not in order".into()));
            }
            if let Some(driver) = &channel.driver {
                driver.check().map_err(|error| SceneError::InvalidFile(error.to_string().replace("invalid-params: ", "")))?;
            }
            if channel.modifiers.len() > MAX_MODIFIERS || channel.modifiers.iter().any(|modifier| modifier.check().is_err()) {
                return Err(SceneError::InvalidFile(format!("a {} channel's modifiers are out of range", channel.property.name())));
            }
        }
        let mut names = std::collections::BTreeSet::new();
        for action in &self.animation.actions {
            if action.name.trim().is_empty() || action.name.len() > 128 || !names.insert(action.name.as_str()) {
                return Err(SceneError::InvalidFile(format!("action `{}` is unnamed, too long, or repeated", action.name)));
            }
            let mut seen = std::collections::BTreeSet::new();
            for channel in &action.channels {
                if channel.index >= channel.property.components()
                    || channel.property.named() == channel.path.is_empty()
                    || channel.property.on_material()
                    || !seen.insert((channel.property, channel.index, channel.path.as_str()))
                    || channel.keys.is_empty()
                    || channel.keys.iter().any(|key| !(key.frame.is_finite() && key.value.is_finite()) || key.frame.abs() > MAX_FRAME as f32)
                    || channel.keys.windows(2).any(|pair| matches!(pair, [a, b] if b.frame <= a.frame))
                    || channel.modifiers.len() > MAX_MODIFIERS
                    || channel.modifiers.iter().any(|modifier| modifier.check().is_err())
                {
                    return Err(SceneError::InvalidFile(format!("action `{}` has a channel out of range", action.name)));
                }
            }
        }
        let mut names = std::collections::BTreeSet::new();
        for set in &self.animation.keying_sets.sets {
            let builtin = BuiltinKeyingSet::ALL.iter().any(|builtin| builtin.name() == set.name);
            if set.name.trim().is_empty() || set.name.len() > 128 || builtin || !names.insert(set.name.as_str()) {
                return Err(SceneError::InvalidFile(format!("keying set `{}` is unnamed, too long, repeated, or a built-in's name", set.name)));
            }
            if set
                .paths
                .iter()
                .any(|path| path.property.named() == path.path.is_empty() || read_at(self, path.target, path.property, 0, &path.path).is_none())
            {
                return Err(SceneError::InvalidFile(format!("keying set `{}` names something that does not have the property", set.name)));
            }
        }
        if let Some(KeyingSetRef::Custom(name)) = &self.animation.keying_sets.active
            && !names.contains(name.as_str())
        {
            return Err(SceneError::InvalidFile(format!("the active keying set `{name}` does not exist")));
        }
        self.validate_nla()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(keys: &[(f32, f32)]) -> Channel {
        Channel {
            target: 1,
            property: Property::Location,
            index: 0,
            path: String::new(),
            keys: keys.iter().map(|(frame, value)| Keyframe { frame: *frame, value: *value, interpolation: Interpolation::Bezier }).collect(),
            driver: None,
            modifiers: Vec::new(),
        }
    }

    /// Samples the reference application gives for the same keys (Blender 5.2, auto-clamped
    /// Bézier keys inserted on location X).
    #[test]
    fn bezier_curves_match_the_reference() {
        type Points = &'static [(f32, f32)];
        let cases: &[(Points, Points)] = &[
            (&[(1.0, 0.0), (20.0, 5.0)], &[(2.0, 0.0401), (5.0, 0.5715), (7.5, 1.3552), (10.0, 2.3028), (13.0, 3.4641), (15.0, 4.1435), (25.0, 5.0)]),
            (&[(1.0, 0.0), (20.0, 5.0), (40.0, 2.0)], &[(25.0, 4.5312), (30.0, 3.5), (35.0, 2.4687), (45.0, 2.0)]),
            (
                &[(1.0, 0.0), (10.0, 3.0), (40.0, 9.0)],
                &[
                    (2.0, 0.0581),
                    (5.0, 0.8028),
                    (7.5, 1.8423),
                    (13.0, 4.2708),
                    (15.0, 5.0203),
                    (20.0, 6.5726),
                    (25.0, 7.7019),
                    (30.0, 8.453),
                    (35.0, 8.8707),
                ],
            ),
            (
                &[(1.0, 0.0), (5.0, 4.0), (40.0, 6.0), (45.0, 0.0)],
                &[
                    (2.0, 0.5929),
                    (7.5, 4.3987),
                    (10.0, 4.7405),
                    (13.0, 5.0818),
                    (15.0, 5.2711),
                    (20.0, 5.6268),
                    (25.0, 5.8426),
                    (30.0, 5.9534),
                    (35.0, 5.9942),
                ],
            ),
        ];
        for (keys, samples) in cases {
            let curve = channel(keys);
            for (frame, expected) in *samples {
                let got = curve.evaluate(*frame).unwrap();
                assert!((got - expected).abs() < 2e-3, "{keys:?} at {frame}: {got} against {expected}");
            }
        }
    }

    #[test]
    fn linear_and_constant_segments() {
        let mut curve = channel(&[(0.0, 0.0), (10.0, 10.0), (20.0, 0.0)]);
        curve.keys[0].interpolation = Interpolation::Linear;
        curve.keys[1].interpolation = Interpolation::Constant;
        assert!((curve.evaluate(2.5).unwrap() - 2.5).abs() < 1e-6);
        assert_eq!(curve.evaluate(15.0), Some(10.0));
        assert_eq!(curve.evaluate(20.0), Some(0.0));
    }

    #[test]
    fn keys_replace_on_the_same_frame_and_stay_sorted() {
        let mut curve = channel(&[(10.0, 1.0)]);
        curve.insert(5.0, 2.0, Interpolation::Bezier);
        curve.insert(10.0, 3.0, Interpolation::Bezier);
        curve.insert(20.0, 4.0, Interpolation::Bezier);
        assert_eq!(curve.keys.iter().map(|key| (key.frame, key.value)).collect::<Vec<_>>(), vec![(5.0, 2.0), (10.0, 3.0), (20.0, 4.0)]);
    }

    #[test]
    fn the_scene_moves_with_the_frame() {
        let mut scene = Scene::starter();
        let cube = scene.object_named("Cube").unwrap().id;
        scene.insert_key(cube, Property::Location, 1.0).unwrap();
        scene.object_mut(cube).unwrap().location.x = 4.0;
        scene.insert_key(cube, Property::Location, 21.0).unwrap();
        scene.validate_animation().unwrap();
        let middle = scene.at_frame(11.0);
        assert!((middle.object(cube).unwrap().location.x - 2.0).abs() < 1e-4);
        scene.go_to_frame(1.0);
        assert_eq!(scene.object(cube).unwrap().location.x, 0.0);
        assert_eq!(scene.animation.timeline.frame, 1);
        assert_eq!(scene.key_frames(&[cube]), vec![1.0, 21.0]);
        assert_eq!(scene.delete_key(cube, Property::Location, 21.0), 3);
        scene.remove_objects(&[cube]).unwrap();
        scene.prune_animation();
        assert!(scene.animation.channels.is_empty());
    }

    #[test]
    fn material_and_light_channels() {
        let mut scene = Scene::starter();
        let light = scene.object_named("Light").unwrap().id;
        let material = scene.materials[0].id;
        assert_eq!(scene.insert_key(light, Property::Power, 1.0).unwrap(), 1);
        assert_eq!(scene.insert_key(material, Property::BaseColor, 1.0).unwrap(), 4);
        assert!(scene.insert_key(light, Property::Lens, 1.0).is_err());
        scene.validate_animation().unwrap();
    }
}
