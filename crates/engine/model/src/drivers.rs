//! Drivers: a property's value worked out from other values instead of keyframes. A driver sits
//! on a channel (the channel's keys, when it has any, then map the driver's value as a curve
//! does a frame); its variables read a property, a transform channel, the angle between two
//! rotations, or a distance, and its kind averages, sums, takes the smallest or largest of
//! them, or evaluates an expression ([`crate::expression`]) over them and `frame`.
//!
//! The scene applies drivers after keyframes, whenever the frame changes and after every edit
//! ([`Scene::apply_drivers`]), a few passes so drivers reading driven values settle.

use std::collections::BTreeMap;

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::animation::{Property, read_at, write_at};
use crate::constraints::{Space, Target, TransformChannel};
use crate::error::{Result, SceneError};
use crate::scene::{Id, Scene};

/// How a driver combines its variables.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DriverKind {
    Average,
    Sum,
    Min,
    Max,
    /// Evaluates the driver's expression.
    #[default]
    Scripted,
}

/// The space a Transform variable reads in: world (constraints and parents included), the
/// transform's own values, or local (constraints included, parents not).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VariableSpace {
    #[default]
    World,
    Transform,
    Local,
}

/// Where a variable's value comes from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Source {
    /// A property of an object, bone, shape key, or material (as channels name them).
    SingleProperty {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Id>,
        property: Property,
        #[serde(default)]
        index: u8,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        path: String,
    },
    /// A location, rotation (XYZ Euler, radians), or scale component of an object or a bone;
    /// axis 3 of scale is the average (the cube root of the three multiplied).
    Transform {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Target>,
        #[serde(default)]
        channel: TransformChannel,
        #[serde(default)]
        axis: u8,
        #[serde(default)]
        space: VariableSpace,
    },
    /// The angle, in radians, between two objects' or bones' world rotations.
    RotationDifference {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        a: Option<Target>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        b: Option<Target>,
    },
    /// The distance between two objects' or bones' world locations.
    Distance {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        a: Option<Target>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        b: Option<Target>,
    },
}

impl Source {
    /// The objects the variable reads.
    pub fn objects(&self) -> Vec<Id> {
        match self {
            Self::SingleProperty { target, .. } => target.iter().copied().collect(),
            Self::Transform { target, .. } => target.iter().map(|target| target.object).collect(),
            Self::RotationDifference { a, b } | Self::Distance { a, b } => a.iter().chain(b.iter()).map(|target| target.object).collect(),
        }
    }

    /// Lets go of objects in `gone`.
    pub fn forget(&mut self, gone: &[Id]) {
        let lost = |slot: &mut Option<Target>| {
            if slot.as_ref().is_some_and(|target| gone.contains(&target.object)) {
                *slot = None;
            }
        };
        match self {
            Self::SingleProperty { target, .. } => {
                if target.is_some_and(|id| gone.contains(&id)) {
                    *target = None;
                }
            }
            Self::Transform { target, .. } => lost(target),
            Self::RotationDifference { a, b } | Self::Distance { a, b } => {
                lost(a);
                lost(b);
            }
        }
    }
}

/// One variable of a driver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Variable {
    pub name: String,
    #[serde(flatten)]
    pub source: Source,
}

/// A driver: how a channel's value is worked out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Driver {
    #[serde(default)]
    pub kind: DriverKind,
    /// The expression of a scripted driver.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub expression: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<Variable>,
}

impl Default for Driver {
    /// A scripted driver of one variable `var`, as the reference application makes them.
    fn default() -> Self {
        Self { kind: DriverKind::Scripted, expression: "var".into(), variables: Vec::new() }
    }
}

/// The most variables a driver may have.
pub const MAX_VARIABLES: usize = 64;

impl Driver {
    /// Checks names and the expression.
    pub fn check(&self) -> Result<()> {
        if self.variables.len() > MAX_VARIABLES {
            return Err(SceneError::params(format!("a driver has at most {MAX_VARIABLES} variables")));
        }
        let mut names = Vec::new();
        for variable in &self.variables {
            if !crate::expression::valid_name(&variable.name) {
                return Err(SceneError::params(format!("`{}` cannot name a variable", variable.name)));
            }
            if names.contains(&variable.name.as_str()) {
                return Err(SceneError::params(format!("two variables are called `{}`", variable.name)));
            }
            names.push(variable.name.as_str());
            if let Source::Transform { axis, channel, .. } = &variable.source
                && (*axis > 3 || (*axis == 3 && *channel != TransformChannel::Scale))
            {
                return Err(SceneError::params("a transform variable's axis is 0 to 2 (3, the average, for scale)"));
            }
        }
        if self.kind == DriverKind::Scripted {
            crate::expression::parse(&self.expression).map_err(SceneError::params)?;
        }
        Ok(())
    }

    /// The driver's value in `scene` at `frame`.
    pub fn evaluate(&self, scene: &Scene, frame: f32) -> std::result::Result<f32, String> {
        let mut values = BTreeMap::new();
        for variable in &self.variables {
            values.insert(
                variable.name.clone(),
                f64::from(variable_value(scene, &variable.source).ok_or_else(|| format!("`{}` reads nothing", variable.name))?),
            );
        }
        let numbers: Vec<f64> = self.variables.iter().filter_map(|variable| values.get(&variable.name).copied()).collect();
        let empty = || "the driver has no variables".to_string();
        let value = match self.kind {
            DriverKind::Average => numbers.iter().sum::<f64>() / numbers.len().max(1) as f64,
            DriverKind::Sum => numbers.iter().sum(),
            DriverKind::Min => numbers.iter().copied().reduce(f64::min).ok_or_else(empty)?,
            DriverKind::Max => numbers.iter().copied().reduce(f64::max).ok_or_else(empty)?,
            DriverKind::Scripted => {
                values.insert("frame".into(), f64::from(frame));
                crate::expression::parse(&self.expression)?.eval(&values)?
            }
        };
        if matches!(self.kind, DriverKind::Average) && numbers.is_empty() {
            return Err(empty());
        }
        Ok(value as f32)
    }
}

/// The world matrix of an object or a bone.
fn world_of(scene: &Scene, target: &Target) -> Option<Mat4> {
    crate::constraints::target_world(scene, target)
}

fn variable_value(scene: &Scene, source: &Source) -> Option<f32> {
    match source {
        Source::SingleProperty { target, property, index, path } => read_at(scene, (*target)?, *property, *index, path),
        Source::Transform { target, channel, axis, space } => {
            let target = target.as_ref()?;
            let values = match space {
                VariableSpace::World => crate::constraint_math::channel_values(world_of(scene, target)?, *channel),
                VariableSpace::Local => {
                    let into = crate::constraints::target_space(scene, target, Space::Local)?;
                    crate::constraint_math::channel_values(into.inverse() * world_of(scene, target)?, *channel)
                }
                VariableSpace::Transform => {
                    let (location, rotation, scale) = match &target.bone {
                        None => {
                            let object = scene.object(target.object).ok()?;
                            (object.location, object.rotation, object.scale)
                        }
                        Some(bone) => {
                            let pose = crate::armature::armature_of(scene, target.object)?.bone(bone)?.pose;
                            (pose.location, pose.rotation, pose.scale)
                        }
                    };
                    match channel {
                        TransformChannel::Location => location,
                        TransformChannel::Rotation => rotation,
                        TransformChannel::Scale => scale,
                    }
                }
            };
            if *axis == 3 {
                // The reference application's average scale: the cube root of the volume.
                return Some((values.x * values.y * values.z).cbrt());
            }
            values.to_array().get(usize::from(*axis)).copied()
        }
        Source::RotationDifference { a, b } => {
            let turn = |target: &Option<Target>| -> Option<Quat> {
                let (_, rotation, _) = crate::constraint_math::split(world_of(scene, target.as_ref()?)?);
                Some(Quat::from_mat3(&rotation).normalize())
            };
            let (a, b) = (turn(a)?, turn(b)?);
            Some(2.0 * a.dot(b).abs().min(1.0).acos())
        }
        Source::Distance { a, b } => {
            let at = |target: &Option<Target>| -> Option<Vec3> { Some(world_of(scene, target.as_ref()?)?.w_axis.truncate()) };
            Some(at(a)?.distance(at(b)?))
        }
    }
}

impl Scene {
    /// Writes every driven value: each driver's value (through its channel's keys, when it has
    /// any) at `frame`. Drivers reading driven values settle over a few passes. Answers the
    /// drivers that could not be evaluated, with why, by channel position.
    pub fn apply_drivers(&mut self, frame: f32) -> Vec<(usize, String)> {
        let driven: Vec<usize> =
            self.animation.channels.iter().enumerate().filter(|(_, channel)| channel.driver.is_some()).map(|(slot, _)| slot).collect();
        let mut failed = Vec::new();
        if driven.is_empty() {
            return failed;
        }
        for pass in 0..4 {
            let mut moved = false;
            failed.clear();
            for slot in &driven {
                let Some(channel) = self.animation.channels.get(*slot) else { continue };
                let Some(driver) = &channel.driver else { continue };
                let (target, property, index, path) = (channel.target, channel.property, channel.index, channel.path.clone());
                let value = match driver.evaluate(self, frame) {
                    Ok(value) if channel.keys.is_empty() => value,
                    Ok(value) => channel.evaluate(value).unwrap_or(value),
                    Err(error) => {
                        failed.push((*slot, error));
                        continue;
                    }
                };
                let before = read_at(self, target, property, index, &path);
                if before.is_none_or(|before| (before - value).abs() > 1e-7) {
                    write_at(self, target, property, index, &path, value);
                    moved = true;
                }
            }
            if !moved || pass == 3 {
                break;
            }
        }
        failed
    }

    /// The channel driving a property, if a driver drives it.
    pub fn driver_of(&self, target: Id, property: Property, index: u8, path: &str) -> Option<&crate::animation::Channel> {
        self.animation.channels.iter().find(|channel| {
            channel.driver.is_some() && channel.target == target && channel.property == property && channel.index == index && channel.path == path
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::Channel;

    #[test]
    fn a_driver_follows_another_object() {
        let mut scene = Scene::starter();
        let cube = scene.object_named("Cube").unwrap().id;
        let light = scene.object_named("Light").unwrap().id;
        scene.object_mut(light).unwrap().location = Vec3::new(1.0, 2.0, 3.0);
        scene.animation.channels.push(Channel {
            target: cube,
            property: Property::Location,
            index: 2,
            path: String::new(),
            keys: Vec::new(),
            driver: Some(Driver {
                kind: DriverKind::Scripted,
                expression: "height * 2 + frame".into(),
                variables: vec![Variable {
                    name: "height".into(),
                    source: Source::Transform {
                        target: Some(Target { object: light, bone: None }),
                        channel: TransformChannel::Location,
                        axis: 2,
                        space: VariableSpace::World,
                    },
                }],
            }),
            modifiers: Vec::new(),
        });
        scene.validate_animation().unwrap();
        scene.go_to_frame(4.0);
        assert!((scene.object(cube).unwrap().location.z - 10.0).abs() < 1e-5);
        // An average of a property.
        scene.animation.channels[0].driver = Some(Driver {
            kind: DriverKind::Average,
            expression: String::new(),
            variables: vec![
                Variable {
                    name: "a".into(),
                    source: Source::SingleProperty { target: Some(light), property: Property::Location, index: 0, path: String::new() },
                },
                Variable {
                    name: "b".into(),
                    source: Source::SingleProperty { target: Some(light), property: Property::Location, index: 1, path: String::new() },
                },
            ],
        });
        scene.apply_drivers(1.0);
        assert!((scene.object(cube).unwrap().location.z - 1.5).abs() < 1e-5);
        // Keys on the driven channel map the driver's value.
        scene.animation.channels[0].keys = vec![
            crate::animation::Keyframe { frame: 0.0, value: 0.0, interpolation: crate::animation::Interpolation::Linear },
            crate::animation::Keyframe { frame: 2.0, value: 10.0, interpolation: crate::animation::Interpolation::Linear },
        ];
        scene.apply_drivers(1.0);
        assert!((scene.object(cube).unwrap().location.z - 7.5).abs() < 1e-4);
    }
}
