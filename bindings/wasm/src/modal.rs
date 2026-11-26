//! The modal transform: move, rotate and scale as Blender runs them, for objects, a mesh's
//! vertices, an armature's bones, a curve's points and a lattice's points. A key starts one (G, R,
//! S, and Shift+D for the copies it makes) and the pointer drives it without a button held; a
//! transform tool's press starts one that its release confirms. While it runs, X, Y and Z lock it
//! to an axis (Shift: to the plane across it; again: the object's own axes; a third time: free),
//! typed numbers give its value exactly, Ctrl steps it and Shift slows it down. Confirming makes
//! one undo step; cancelling leaves the scene as it was, with no step.
//!
//! It keeps what it moves as it was when it began (the objects' transforms, the points' places,
//! never a copy of the scene), and every update puts that back and applies the transform again
//! from it. Updates leave the document's revision alone: the editor asks for the GPU picture
//! with the answer (`modalStep`), and no software picture is drawn until the transform ends.
//!
//! The editor drives it with session commands that make no undo step of their own:
//! `modal.begin`, `modal.pointer`, `modal.key`, `modal.wheel`, `modal.confirm`, `modal.cancel`
//! and `modal.state`.

use std::collections::BTreeSet;

use glam::{Mat3, Vec3};
use kition_kit::KitCommand;
use kition_kit::tools::ToolOverlay;
use kition_scene_model::scene::{euler_matrix, matrix_euler};
use kition_scene_model::{Id, ObjectData};
use kition_scene_raster::{Projector, ViewState};
use serde_json::{Value, json};

use crate::params::{self, bad, schema as s};
use crate::{Mode, SceneDocument, Step};

/// Screen pixels a press may travel and still be a click.
const CLICK_PIXELS: f64 = 4.0;
/// Screen pixels round the pivot in which the pointer's direction turns nothing.
const PIVOT_PIXELS: f64 = 6.0;
/// What the pointer's motion counts for with Shift held.
const PRECISION: f64 = 0.1;
/// Ctrl's steps (with Shift: the finer ones): metres, degrees, scale.
const MOVE_STEPS: (f32, f32) = (1.0, 0.1);
const ROTATE_STEPS: (f32, f32) = (5.0, 1.0);
const SCALE_STEPS: (f32, f32) = (0.1, 0.01);
/// The longest number typed into one field.
const MAX_TYPED: usize = 24;
/// The axes' colours (the default theme's X, Y and Z).
const AXIS_COLORS: [[u8; 4]; 3] = [[255, 51, 82, 255], [139, 220, 0, 255], [40, 144, 255, 255]];
const DASH: [u8; 4] = [20, 20, 20, 220];
const AXIS_NAMES: [&str; 3] = ["X", "Y", "Z"];

/// What the transform does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Move,
    Rotate,
    Scale,
}

impl Kind {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "move" => Some(Kind::Move),
            "rotate" => Some(Kind::Rotate),
            "scale" => Some(Kind::Scale),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Move => "move",
            Kind::Rotate => "rotate",
            Kind::Scale => "scale",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Kind::Move => "Move",
            Kind::Rotate => "Rotate",
            Kind::Scale => "Scale",
        }
    }

    /// The fields typed numbers go to: a move takes one per free axis, a rotation one angle, a
    /// scale one per free axis.
    fn fields(self, constraint: Option<Constraint>) -> usize {
        match (self, constraint) {
            (Kind::Rotate, _) => 1,
            (_, None) => 3,
            (_, Some(constraint)) => constraint.free().len(),
        }
    }
}

/// Whose axes a constraint follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Global,
    /// Each object's own axes (in edit modes, the edited object's).
    Local,
}

impl Orientation {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "global" => Some(Orientation::Global),
            "local" => Some(Orientation::Local),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Orientation::Global => "global",
            Orientation::Local => "local",
        }
    }
}

/// An axis the transform keeps to, or (`plane`) the plane across it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Constraint {
    pub axis: usize,
    pub plane: bool,
    pub orientation: Orientation,
}

impl Constraint {
    /// The axes the transform may use, in order.
    fn free(&self) -> Vec<usize> {
        if self.plane { (0..3).filter(|axis| *axis != self.axis).collect() } else { vec![self.axis] }
    }

    /// As the header ends: ` along global X`, ` locking local Z`.
    fn text(&self) -> String {
        format!(" {} {} {}", if self.plane { "locking" } else { "along" }, self.orientation.name(), AXIS_NAMES.get(self.axis).copied().unwrap_or("Z"))
    }
}

/// How the transform began: by a key (the left button or Enter confirms it) or by a transform
/// tool's press (its release confirms it; a release where it was pressed is a click, which
/// selects when `select`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Started {
    Key,
    Press { select: bool },
}

/// An object's transform when the modal began.
#[derive(Debug, Clone)]
struct ObjectStart {
    id: Id,
    location: Vec3,
    rotation: Vec3,
    scale: Vec3,
}

/// What the modal moves, as it was when it began.
#[derive(Debug, Clone)]
enum Items {
    /// The selected objects (the top ones of a selected hierarchy).
    Objects(Vec<ObjectStart>),
    /// The edited mesh's selected vertices, and with proportional editing those that follow them
    /// part of the way (vertex, weight); `original` is every vertex's place.
    Vertices { object: Id, original: Vec<Vec3>, selected: Vec<u32>, followers: Vec<(u32, f32)> },
    /// An armature's selected bones, in edit mode (heads and tails) or pose mode (their pose).
    Bones { object: Id, original: ObjectData, pose: bool, selected: BTreeSet<String> },
    /// A curve's selected points and handles.
    Curve { object: Id, original: ObjectData, selected: BTreeSet<u32> },
    /// A lattice's selected points.
    Lattice { object: Id, original: ObjectData, selected: BTreeSet<u32> },
}

impl Items {
    /// The object whose axes are the local ones (the first object, or the edited one).
    fn first_object(&self) -> Option<Id> {
        match self {
            Items::Objects(objects) => objects.first().map(|start| start.id),
            Items::Vertices { object, .. } | Items::Bones { object, .. } | Items::Curve { object, .. } | Items::Lattice { object, .. } => {
                Some(*object)
            }
        }
    }

    /// The undo step's name.
    fn label(&self, kind: Kind) -> &'static str {
        match self {
            Items::Objects(_) | Items::Vertices { .. } => kind.title(),
            Items::Bones { .. } => "Transform Bones",
            Items::Curve { .. } | Items::Lattice { .. } => "Transform Points",
        }
    }
}

/// One typed number.
#[derive(Debug, Clone, Default, PartialEq)]
struct Field {
    text: String,
    edited: bool,
    negate: bool,
    inverse: bool,
}

impl Field {
    /// The value typed, or `None` when nothing was typed here.
    fn value(&self) -> Option<f32> {
        if !self.edited {
            return None;
        }
        let typed = if self.text.is_empty() || self.text == "." { 0.0 } else { self.text.parse::<f32>().ok().filter(|value| value.is_finite())? };
        let value = if self.inverse { if typed == 0.0 { 0.0 } else { 1.0 / typed } } else { typed };
        Some(if self.negate { -value } else { value })
    }

    /// As the header shows the field being typed: `[-(2|)] = -2`.
    fn shown(&self, value: &str) -> String {
        let (open, close) = match (self.negate, self.inverse) {
            (true, true) => ("-1/(", ")"),
            (true, false) => ("-(", ")"),
            (false, true) => ("1/(", ")"),
            (false, false) => ("", ""),
        };
        format!("[{open}{}|{close}] = {value}", self.text)
    }
}

/// The numbers typed: one field per free axis (or the angle), the one being typed `index`.
#[derive(Debug, Clone, Default)]
struct Numeric {
    on: bool,
    fields: [Field; 3],
    index: usize,
}

impl Numeric {
    fn current(&mut self) -> Option<&mut Field> {
        self.fields.get_mut(self.index)
    }

    fn typed(&mut self, character: char) -> bool {
        self.on = true;
        let Some(field) = self.current() else { return false };
        if field.text.len() >= MAX_TYPED || (character == '.' && field.text.contains('.')) {
            return true;
        }
        field.text.push(character);
        field.edited = true;
        true
    }

    fn negate(&mut self) {
        self.on = true;
        if let Some(field) = self.current() {
            field.negate = !field.negate;
            field.edited = true;
        }
    }

    fn inverse(&mut self) {
        self.on = true;
        if let Some(field) = self.current() {
            field.inverse = !field.inverse;
            field.edited = true;
        }
    }

    /// A character less; on an empty field, the field typed nothing; on a field never typed,
    /// numbers are no longer typed and the pointer gives the value again.
    fn backspace(&mut self) -> bool {
        if !self.on {
            return false;
        }
        let Some(field) = self.current() else { return false };
        if !field.edited {
            *self = Numeric::default();
        } else if field.text.pop().is_none() {
            *field = Field::default();
        }
        true
    }

    /// The next field (with Ctrl the one before), of `count`.
    fn tab(&mut self, back: bool, count: usize) -> bool {
        if !self.on || count == 0 {
            return false;
        }
        self.index = if back { (self.index + count - 1) % count } else { (self.index + 1) % count };
        true
    }

    /// Field `index`'s value with what an untyped field takes: the first field's (`all`, a scale
    /// typed once for every axis), else `none`.
    fn value(&self, index: usize, all: bool, none: f32) -> f32 {
        let own = self.fields.get(index).and_then(Field::value);
        let first = if all && index != self.index { self.fields.first().and_then(Field::value) } else { None };
        own.or(first).unwrap_or(none)
    }
}

/// The pointer's input before the view last changed under the transform.
#[derive(Debug, Clone, Copy)]
struct Base {
    /// A move's world offset.
    offset: Vec3,
    /// A scale's ratio.
    ratio: f64,
}

impl Default for Base {
    fn default() -> Self {
        Self { offset: Vec3::ZERO, ratio: 1.0 }
    }
}

/// The transform's value now: a move's offset (world, or along the constraint's axes in its
/// frame), a rotation's angle (radians), a scale's factors (along the frame's axes).
#[derive(Debug, Clone, Copy)]
struct Values {
    offset: Vec3,
    angle: f32,
    factors: Vec3,
}

impl Default for Values {
    fn default() -> Self {
        Self { offset: Vec3::ZERO, angle: 0.0, factors: Vec3::ONE }
    }
}

/// A transform of world points: `pivot + linear (p - pivot) + offset`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Change {
    pub kind: Kind,
    pub linear: Mat3,
    pub offset: Vec3,
    pub pivot: Vec3,
}

impl Change {
    pub fn point(&self, point: Vec3) -> Vec3 {
        self.pivot + self.linear * (point - self.pivot) + self.offset
    }
}

/// A transform under way.
#[derive(Debug, Clone)]
pub struct Modal {
    kind: Kind,
    items: Items,
    /// The point it turns and scales about (the selection's median), in world space.
    pivot: Vec3,
    /// The first object's own axes, for local constraints and the pointer's reading of them.
    local: Mat3,
    started: Started,
    /// Where the pointer's input starts, where it is with Shift's slowing, and where it really is.
    start: (f64, f64),
    virtual_at: (f64, f64),
    last: (f64, f64),
    /// The angle the pointer turned about the pivot on the screen, clockwise, in radians.
    turned: f64,
    base: Base,
    /// The view the pointer is read in, and the pivot's place in it.
    view: (ViewState, u32, u32),
    pivot_screen: (f64, f64),
    constraint: Option<Constraint>,
    numeric: Numeric,
    precision: bool,
    step: bool,
    /// The pointer went further than a click from where it was pressed.
    travelled: bool,
    /// Shift+D: the scene before its copies were made (the undo step is the copy's).
    duplicate: Option<Box<Step>>,
    values: Values,
    ratio: f64,
}

fn unit(axis: usize) -> Vec3 {
    match axis {
        0 => Vec3::X,
        1 => Vec3::Y,
        _ => Vec3::Z,
    }
}

fn component(value: Vec3, axis: usize) -> f32 {
    match axis {
        0 => value.x,
        1 => value.y,
        _ => value.z,
    }
}

fn with_component(mut value: Vec3, axis: usize, to: f32) -> Vec3 {
    match axis {
        0 => value.x = to,
        1 => value.y = to,
        _ => value.z = to,
    }
    value
}

/// A rotation matrix from any matrix with its axes (scale and shear taken out).
fn orthonormal(matrix: Mat3) -> Mat3 {
    let x = matrix.x_axis.normalize_or(Vec3::X);
    let y = (matrix.y_axis - x * x.dot(matrix.y_axis)).normalize_or(x.any_orthonormal_vector());
    Mat3::from_cols(x, y, x.cross(y))
}

/// `value` to the nearest multiple of `step`.
fn snapped(value: f32, step: f32) -> f32 {
    if step > 0.0 { (value / step).round() * step } else { value }
}

/// Without a negative zero (`-0.0000` in a header).
fn plain(value: f32) -> f32 {
    if value == 0.0 { 0.0 } else { value }
}

fn screen_of(projector: &Projector, point: Vec3) -> (f64, f64) {
    let at = projector.to_screen(projector.view_projection() * point.extend(1.0));
    (f64::from(at.x), f64::from(at.y))
}

/// The direction into the screen at `point`.
fn into_screen(projector: &Projector, point: Vec3) -> Vec3 {
    if projector.perspective {
        (point - projector.eye()).normalize_or(Vec3::NEG_Z)
    } else {
        (Mat3::from_mat4(projector.view.inverse()) * Vec3::NEG_Z).normalize_or(Vec3::NEG_Z)
    }
}

/// The world offset between the screen points `from` and `to`, in the plane facing the view
/// through `pivot`.
fn view_offset(projector: &Projector, pivot: Vec3, from: (f64, f64), to: (f64, f64)) -> Vec3 {
    let depth = projector.to_screen(projector.view_projection() * pivot.extend(1.0)).z;
    let offset = projector.unproject(to.0 as f32, to.1 as f32, depth) - projector.unproject(from.0 as f32, from.1 as f32, depth);
    if offset.is_finite() { offset } else { Vec3::ZERO }
}

/// How far along the line through `pivot` in `direction` the point under the screen point `at`
/// lies (the line's point nearest the pointer's ray); `None` where the line runs along the view.
fn along(projector: &Projector, pivot: Vec3, direction: Vec3, at: (f64, f64)) -> Option<f32> {
    let (origin, ray) = projector.ray(at.0 as f32, at.1 as f32);
    let between = pivot - origin;
    let cosine = direction.dot(ray);
    let across = 1.0 - cosine * cosine;
    if across < 1e-3 {
        return None;
    }
    let t = (cosine * ray.dot(between) - direction.dot(between)) / across;
    // The nearest point of the ray is behind the eye: the line goes off the view there.
    let s = ray.dot(between) + cosine * t;
    (t.is_finite() && s > 0.0).then_some(t)
}

/// Where the pointer's ray at `at` meets the plane through `pivot` across `normal`.
fn on_plane(projector: &Projector, pivot: Vec3, normal: Vec3, at: (f64, f64)) -> Option<Vec3> {
    let (origin, ray) = projector.ray(at.0 as f32, at.1 as f32);
    let facing = ray.dot(normal);
    if facing.abs() < 1e-3 {
        return None;
    }
    let s = (pivot - origin).dot(normal) / facing;
    let hit = origin + ray * s;
    (s > 0.0 && hit.is_finite()).then_some(hit)
}

impl Modal {
    /// The frame a constraint follows for an item whose own axes are `own`.
    fn frame(&self, own: Mat3) -> Mat3 {
        match self.constraint {
            Some(Constraint { orientation: Orientation::Local, .. }) => own,
            _ => Mat3::IDENTITY,
        }
    }

    /// Whether it would change anything now.
    fn changes(&self) -> bool {
        self.duplicate.is_some()
            || match self.kind {
                Kind::Move => self.values.offset != Vec3::ZERO,
                Kind::Rotate => self.values.angle != 0.0,
                Kind::Scale => self.values.factors != Vec3::ONE,
            }
    }

    /// The pointer pressed or released here is a click: no travel from where it began, no number
    /// typed, no axis chosen.
    fn click(&self, at: (f64, f64)) -> bool {
        !self.travelled && !self.numeric.on && self.constraint.is_none() && (at.0 - self.start.0).hypot(at.1 - self.start.1) <= CLICK_PIXELS
    }

    /// The value from the typed numbers or the pointer.
    fn solve(&self, projector: &Projector) -> (Values, f64) {
        let mut values = Values::default();
        let free = self.constraint.map(|constraint| constraint.free());
        let frame = self.frame(self.local);
        if self.numeric.on {
            match self.kind {
                Kind::Move => {
                    values.offset = match &free {
                        None => Vec3::new(self.numeric.value(0, false, 0.0), self.numeric.value(1, false, 0.0), self.numeric.value(2, false, 0.0)),
                        Some(free) => free
                            .iter()
                            .enumerate()
                            .fold(Vec3::ZERO, |offset, (index, axis)| with_component(offset, *axis, self.numeric.value(index, false, 0.0))),
                    };
                }
                Kind::Rotate => values.angle = self.numeric.value(0, false, 0.0).to_radians(),
                Kind::Scale => {
                    values.factors = match &free {
                        None => Vec3::new(self.numeric.value(0, true, 1.0), self.numeric.value(1, true, 1.0), self.numeric.value(2, true, 1.0)),
                        Some(free) => free
                            .iter()
                            .enumerate()
                            .fold(Vec3::ONE, |factors, (index, axis)| with_component(factors, *axis, self.numeric.value(index, true, 1.0))),
                    };
                }
            }
            return (values, self.ratio);
        }
        let step = self.step.then_some(self.precision);
        let pick = |steps: (f32, f32)| step.map(|fine| if fine { steps.1 } else { steps.0 });
        let mut ratio = self.ratio;
        match self.kind {
            Kind::Move => {
                let (from, to) = (self.start, self.virtual_at);
                let world = match self.constraint {
                    None => view_offset(projector, self.pivot, from, to),
                    Some(constraint) if !constraint.plane => {
                        let direction = (frame * unit(constraint.axis)).normalize_or(unit(constraint.axis));
                        match (along(projector, self.pivot, direction, from), along(projector, self.pivot, direction, to)) {
                            (Some(a), Some(b)) => direction * (b - a),
                            _ => direction * view_offset(projector, self.pivot, from, to).dot(direction),
                        }
                    }
                    Some(constraint) => {
                        let normal = (frame * unit(constraint.axis)).normalize_or(unit(constraint.axis));
                        match (on_plane(projector, self.pivot, normal, from), on_plane(projector, self.pivot, normal, to)) {
                            (Some(a), Some(b)) => b - a,
                            _ => {
                                let offset = view_offset(projector, self.pivot, from, to);
                                offset - normal * offset.dot(normal)
                            }
                        }
                    }
                };
                values.offset = match &free {
                    None => {
                        let world = world + self.base.offset;
                        match pick(MOVE_STEPS) {
                            Some(size) => Vec3::new(snapped(world.x, size), snapped(world.y, size), snapped(world.z, size)),
                            None => world,
                        }
                    }
                    Some(free) => {
                        let local = frame.transpose() * (world + self.base.offset);
                        free.iter().fold(Vec3::ZERO, |offset, axis| {
                            let value = component(local, *axis);
                            with_component(offset, *axis, pick(MOVE_STEPS).map_or(value, |size| snapped(value, size)))
                        })
                    }
                };
            }
            Kind::Rotate => {
                let turned = self.turned as f32;
                let angle = match self.constraint {
                    None => turned,
                    Some(constraint) => {
                        let axis = frame * unit(constraint.axis);
                        // Turned the way the pointer went, seen from where the view is.
                        if axis.dot(into_screen(projector, self.pivot)) < 0.0 { -turned } else { turned }
                    }
                };
                values.angle = match pick(ROTATE_STEPS) {
                    Some(size) => snapped(angle.to_degrees(), size).to_radians(),
                    None => angle,
                };
            }
            Kind::Scale => {
                let (px, py) = self.pivot_screen;
                let from = (self.start.0 - px, self.start.1 - py);
                let to = (self.virtual_at.0 - px, self.virtual_at.1 - py);
                let mut scale = to.0.hypot(to.1) / from.0.hypot(from.1).max(1.0);
                // Through the pivot to the other side: the scale turns negative.
                if from.0 * to.0 + from.1 * to.1 < 0.0 {
                    scale = -scale;
                }
                ratio = scale * self.base.ratio;
                let factor = match pick(SCALE_STEPS) {
                    Some(size) => snapped(ratio as f32, size),
                    None => ratio as f32,
                };
                values.factors = match &free {
                    None => Vec3::splat(factor),
                    Some(free) => free.iter().fold(Vec3::ONE, |factors, axis| with_component(factors, *axis, factor)),
                };
            }
        }
        (values, ratio)
    }

    /// The change for an item whose own axes are `own`.
    fn change(&self, own: Mat3, projector: &Projector) -> Change {
        let frame = self.frame(own);
        let values = self.values;
        let (linear, offset) = match self.kind {
            Kind::Move => (Mat3::IDENTITY, if self.constraint.is_some() { frame * values.offset } else { values.offset }),
            Kind::Rotate => {
                let axis = match self.constraint {
                    None => into_screen(projector, self.pivot),
                    Some(constraint) => (frame * unit(constraint.axis)).normalize_or(Vec3::Z),
                };
                (Mat3::from_axis_angle(axis, values.angle), Vec3::ZERO)
            }
            Kind::Scale => (frame * Mat3::from_diagonal(values.factors) * frame.transpose(), Vec3::ZERO),
        };
        Change { kind: self.kind, linear, offset, pivot: self.pivot }
    }

    /// Takes an axis key: the axis (or with `plane` the plane across it) in the global frame,
    /// pressed again the local one, a third time free again.
    fn press_axis(&mut self, axis: usize, plane: bool) {
        self.constraint = match self.constraint {
            Some(current) if current.axis == axis && current.plane == plane => match current.orientation {
                Orientation::Global => Some(Constraint { orientation: Orientation::Local, ..current }),
                Orientation::Local => None,
            },
            _ => Some(Constraint { axis, plane, orientation: Orientation::Global }),
        };
        self.fit_fields();
    }

    fn fit_fields(&mut self) {
        let count = self.kind.fields(self.constraint);
        if self.numeric.index >= count {
            self.numeric.index = count.saturating_sub(1);
        }
    }

    /// The pointer's input starts again from where it is (another kind, a view that changed).
    fn restart_input(&mut self) {
        self.start = self.last;
        self.virtual_at = self.last;
        self.turned = 0.0;
        self.base = Base::default();
        self.ratio = 1.0;
    }

    /// One typed field as the header shows it.
    fn field_text(&self, index: usize, value: f32, unit: &str) -> String {
        let shown = format!("{:.4}{unit}", plain(value));
        match self.numeric.fields.get(index) {
            Some(field) if self.numeric.on && index == self.numeric.index && field.edited => field.shown(&shown),
            _ if self.numeric.on && index == self.numeric.index => format!("[|] = {shown}"),
            _ => shown,
        }
    }

    /// The header: what the transform does now, as Blender's viewport header shows it.
    fn header(&self, proportional: Option<f32>) -> String {
        let constraint = self.constraint.map(|constraint| constraint.text()).unwrap_or_default();
        let free = self.constraint.map(|constraint| constraint.free());
        let mut text = proportional.map(|radius| format!("Proportional Size: {:.4} m   ", plain(radius))).unwrap_or_default();
        match self.kind {
            Kind::Move => {
                let offset = self.values.offset;
                match &free {
                    None => {
                        let parts: Vec<String> = (0..3).map(|axis| self.field_text(axis, component(offset, axis), " m")).collect();
                        text.push_str(&format!(
                            "Dx: {}   Dy: {}   Dz: {} ({:.4} m)",
                            parts.first().cloned().unwrap_or_default(),
                            parts.get(1).cloned().unwrap_or_default(),
                            parts.get(2).cloned().unwrap_or_default(),
                            plain(offset.length())
                        ));
                    }
                    Some(free) => {
                        let parts: Vec<String> = free
                            .iter()
                            .enumerate()
                            .map(|(index, axis)| format!("D: {}", self.field_text(index, component(offset, *axis), " m")))
                            .collect();
                        text.push_str(&format!("{} ({:.4} m){constraint}", parts.join("   "), plain(offset.length())));
                    }
                }
            }
            Kind::Rotate => {
                let degrees = self.values.angle.to_degrees();
                let shown = format!("{:.2}°", plain(degrees));
                let value = match self.numeric.fields.first() {
                    Some(field) if self.numeric.on && field.edited => field.shown(&shown),
                    _ if self.numeric.on => format!("[|] = {shown}"),
                    _ => shown,
                };
                text.push_str(&format!("Rotation: {value}{constraint}"));
            }
            Kind::Scale => {
                let factors = self.values.factors;
                match &free {
                    None => {
                        let parts: Vec<String> = (0..3).map(|axis| self.field_text(axis, component(factors, axis), "")).collect();
                        text.push_str(&format!(
                            "Scale X: {}   Y: {}   Z: {}",
                            parts.first().cloned().unwrap_or_default(),
                            parts.get(1).cloned().unwrap_or_default(),
                            parts.get(2).cloned().unwrap_or_default()
                        ));
                    }
                    Some(free) => {
                        let parts: Vec<String> =
                            free.iter().enumerate().map(|(index, axis)| self.field_text(index, component(factors, *axis), "")).collect();
                        text.push_str(&format!("Scale: {}{constraint}", parts.join(" : ")));
                    }
                }
            }
        }
        text
    }
}

impl SceneDocument {
    /// The transform under way, as the editor draws it (`{ "active": false }` without one).
    pub(crate) fn modal_answer(&self) -> Value {
        let Some(modal) = &self.modal else { return json!({ "active": false }) };
        let projector = self.projector();
        let lines: Vec<Value> = self
            .modal_lines(modal, &projector)
            .into_iter()
            .map(|(axis, direction, screen)| json!({ "axis": axis, "direction": direction.to_array(), "screen": screen.map(|(ax, ay, bx, by)| [ax, ay, bx, by]) }))
            .collect();
        let values = match modal.kind {
            Kind::Move => json!(modal.values.offset.to_array()),
            Kind::Rotate => json!(modal.values.angle.to_degrees()),
            Kind::Scale => json!(modal.values.factors.to_array()),
        };
        let pivot_screen = screen_of(&projector, modal.pivot);
        json!({
            "active": true,
            "kind": modal.kind.name(),
            "started": match modal.started { Started::Key => "key", Started::Press { .. } => "press" },
            "header": modal.header(self.modal_radius(modal)),
            "changed": modal.changes(),
            "values": values,
            "constraint": modal.constraint.map(|constraint| json!({
                "axis": AXIS_NAMES.get(constraint.axis).copied().unwrap_or("Z").to_ascii_lowercase(),
                "plane": constraint.plane,
                "orientation": constraint.orientation.name(),
            })),
            "numeric": modal.numeric.on.then(|| json!({
                "index": modal.numeric.index,
                "texts": modal.numeric.fields.iter().take(modal.kind.fields(modal.constraint)).map(|field| field.text.clone()).collect::<Vec<_>>(),
            })),
            "precision": modal.precision,
            "step": modal.step,
            "pivot": modal.pivot.to_array(),
            "pivotScreen": [pivot_screen.0, pivot_screen.1],
            "pointer": [modal.last.0, modal.last.1],
            "lines": lines,
            "dashed": modal.kind != Kind::Move,
            "proportional": self.modal_radius(modal),
            "duplicate": modal.duplicate.is_some(),
        })
    }

    /// Proportional editing's radius in world space, when vertices follow the transform.
    fn modal_radius(&self, modal: &Modal) -> Option<f32> {
        let Items::Vertices { object, .. } = &modal.items else { return None };
        let settings = self.session.proportional?;
        let world = self.scene.world_matrix(*object).ok()?;
        Some(settings.radius * world.to_scale_rotation_translation().0.abs().max_element())
    }

    /// The first item's own axes, in world space.
    fn modal_local_frame(&self, modal: &Modal) -> Mat3 {
        modal.items.first_object().and_then(|id| self.scene.world_matrix(id).ok()).map_or(Mat3::IDENTITY, |world| orthonormal(Mat3::from_mat4(world)))
    }

    /// What a transform would move now, as it is, and the point it turns about; `None` when
    /// nothing is selected or the mode moves nothing (sculpt, weight paint, Particle Edit, text).
    fn modal_items(&self) -> Option<(Items, Vec3)> {
        let session = &self.session;
        if session.sculpt.is_some() || session.paint.is_some() || session.particle_edit.is_some() {
            return None;
        }
        if let Some(rig) = &session.rig {
            if rig.selected.is_empty() {
                return None;
            }
            let original = self.scene.object(rig.object).ok()?.data.clone();
            let pivot = crate::commands_rig::pivot(self, rig)?;
            return Some((Items::Bones { object: rig.object, original, pose: rig.pose, selected: rig.selected.clone() }, pivot));
        }
        match &session.shape {
            Some(crate::commands_curve::ShapeSession::Curve(edit)) => {
                if edit.selected.is_empty() {
                    return None;
                }
                let original = self.scene.object(edit.object).ok()?.data.clone();
                let pivot = crate::commands_curve::pivot(self, edit)?;
                return Some((Items::Curve { object: edit.object, original, selected: edit.selected.clone() }, pivot));
            }
            Some(crate::commands_curve::ShapeSession::Lattice(edit)) => {
                if edit.selected.is_empty() {
                    return None;
                }
                let original = self.scene.object(edit.object).ok()?.data.clone();
                let pivot = crate::commands_lattice::pivot(self, edit)?;
                return Some((Items::Lattice { object: edit.object, original, selected: edit.selected.clone() }, pivot));
            }
            Some(crate::commands_curve::ShapeSession::Text(_)) => return None,
            None => {}
        }
        let pivot = self.pivot()?;
        match session.mode {
            Mode::Object => {
                let visible: Vec<Id> =
                    session.editor.selected.iter().copied().filter(|id| self.scene.object(*id).is_ok_and(|object| !object.hidden)).collect();
                let objects: Vec<ObjectStart> = crate::commands::top_level(&self.scene, &visible)
                    .into_iter()
                    .filter_map(|id| {
                        let object = self.scene.object(id).ok()?;
                        Some(ObjectStart { id, location: object.location, rotation: object.rotation, scale: object.scale })
                    })
                    .collect();
                (!objects.is_empty()).then_some((Items::Objects(objects), pivot))
            }
            Mode::Edit(object) => {
                let mesh = self.scene.object(object).ok()?.mesh()?;
                let selected: Vec<u32> = session.vertices.iter().copied().filter(|vertex| (*vertex as usize) < mesh.positions.len()).collect();
                if selected.is_empty() {
                    return None;
                }
                let followers = session
                    .proportional
                    .map(|settings| kition_scene_model::proportional::weights(mesh, &selected.iter().copied().collect(), settings))
                    .unwrap_or_default();
                Some((Items::Vertices { object, original: mesh.positions.clone(), selected, followers }, pivot))
            }
        }
    }

    /// Starts a transform of the selection at screen point `at` (the pointer's last place when
    /// `None`): `started` says what ends it, `constraint` the axis it keeps to from the start,
    /// and `duplicate` copies the selection first and moves the copies (Shift+D). A transform
    /// under way is cancelled first, as is a number field's preview.
    pub(crate) fn modal_begin(
        &mut self,
        kind: Kind,
        at: Option<(f64, f64)>,
        started: Started,
        constraint: Option<Constraint>,
        duplicate: bool,
    ) -> Result<(), String> {
        if self.modal.is_some() {
            self.modal_cancel()?;
        }
        if let Some(before) = self.field_preview.take() {
            self.scene = before.scene;
        }
        let at = at
            .or(self.tool.pointer_at())
            .filter(|(x, y)| x.is_finite() && y.is_finite())
            .unwrap_or((f64::from(self.session.width) / 2.0, f64::from(self.session.height) / 2.0));
        let copied = if duplicate { Some(Box::new(self.duplicate_for_modal()?)) } else { None };
        let Some((items, pivot)) = self.modal_items() else {
            if let Some(before) = copied {
                self.restore(*before);
                self.touched();
            }
            return Err("not-allowed: select something to transform first".into());
        };
        let projector = self.projector();
        let local =
            items.first_object().and_then(|id| self.scene.world_matrix(id).ok()).map_or(Mat3::IDENTITY, |world| orthonormal(Mat3::from_mat4(world)));
        let mut modal = Modal {
            kind,
            items,
            pivot,
            local,
            started,
            start: at,
            virtual_at: at,
            last: at,
            turned: 0.0,
            base: Base::default(),
            view: (self.session.editor.view.clone(), self.session.width, self.session.height),
            pivot_screen: screen_of(&projector, pivot),
            constraint,
            numeric: Numeric::default(),
            precision: false,
            step: false,
            travelled: false,
            duplicate: copied,
            values: Values::default(),
            ratio: 1.0,
        };
        modal.fit_fields();
        self.modal = Some(modal);
        if duplicate {
            // The copies show at once (they are new objects or vertices).
            self.touched();
        }
        Ok(())
    }

    /// Shift+D's copies, made in the scene for the transform that follows: answers the scene
    /// before them, the undo step's.
    fn duplicate_for_modal(&mut self) -> Result<Step, String> {
        let session = &self.session;
        if session.rig.is_some() || session.shape.is_some() || session.sculpt.is_some() || session.paint.is_some() || session.particle_edit.is_some()
        {
            return Err("not-allowed: Duplicate and move works on objects and a mesh's elements".into());
        }
        let before = self.step("Duplicate");
        let made = crate::commands::run_change(self, "edit.duplicate", &json!({})).and_then(|_| {
            self.tidy_session();
            self.scene.validate().map_err(|error| format!("failed: the copies would break the scene ({error})"))
        });
        match made {
            Ok(()) => Ok(before),
            Err(error) => {
                self.restore(before);
                Err(error)
            }
        }
    }

    /// Puts what the transform moves back as it was when it began.
    fn modal_restore(&mut self, items: &Items) {
        match items {
            Items::Objects(objects) => {
                for start in objects {
                    if let Ok(object) = self.scene.object_mut(start.id) {
                        object.location = start.location;
                        object.rotation = start.rotation;
                        object.scale = start.scale;
                    }
                }
            }
            Items::Vertices { object, original, selected, followers } => {
                if let Some(mesh) = self.scene.object_mut(*object).ok().and_then(|object| object.mesh_mut()) {
                    for vertex in selected.iter().copied().chain(followers.iter().map(|(vertex, _)| *vertex)) {
                        if let (Some(point), Some(was)) = (mesh.positions.get_mut(vertex as usize), original.get(vertex as usize)) {
                            *point = *was;
                        }
                    }
                }
            }
            Items::Bones { object, original, .. } | Items::Curve { object, original, .. } | Items::Lattice { object, original, .. } => {
                if let Ok(object) = self.scene.object_mut(*object) {
                    object.data = original.clone();
                }
            }
        }
    }

    /// Works the value out again and applies it to what the transform moves, from where it was.
    fn modal_apply(&mut self, modal: &mut Modal) {
        let projector = self.projector();
        let (values, ratio) = modal.solve(&projector);
        modal.values = values;
        modal.ratio = ratio;
        self.modal_restore(&modal.items);
        match &modal.items {
            Items::Objects(objects) => {
                for start in objects {
                    let (Ok(world), Ok(parent)) = (self.scene.world_matrix(start.id), self.scene.parent_frame(start.id)) else { continue };
                    let own = orthonormal(Mat3::from_mat4(world));
                    let change = modal.change(own, &projector);
                    let origin = world.transform_point3(Vec3::ZERO);
                    let moved = parent.inverse().transform_vector3(change.point(origin) - origin);
                    let parent_axes = orthonormal(Mat3::from_mat4(parent));
                    let (rotation, scale) = match modal.kind {
                        Kind::Move => (start.rotation, start.scale),
                        Kind::Rotate => {
                            let turned = parent_axes.transpose() * change.linear * parent_axes * euler_matrix(start.rotation);
                            (matrix_euler(orthonormal(turned)), start.scale)
                        }
                        Kind::Scale => {
                            let aligned = modal.constraint.is_none_or(|constraint| constraint.orientation == Orientation::Local)
                                || (own.x_axis - Vec3::X).length() + (own.y_axis - Vec3::Y).length() + (own.z_axis - Vec3::Z).length() < 1e-5;
                            if aligned {
                                (start.rotation, start.scale * values.factors)
                            } else {
                                // Scaled along axes the object's own do not follow: its new axes and
                                // their lengths.
                                let matrix = parent_axes.transpose()
                                    * change.linear
                                    * parent_axes
                                    * euler_matrix(start.rotation)
                                    * Mat3::from_diagonal(start.scale);
                                let axes = orthonormal(matrix);
                                let lengths = Vec3::new(matrix.x_axis.length(), matrix.y_axis.dot(axes.y_axis), matrix.z_axis.dot(axes.z_axis));
                                (matrix_euler(axes), lengths)
                            }
                        }
                    };
                    if let Ok(object) = self.scene.object_mut(start.id)
                        && moved.is_finite()
                        && rotation.is_finite()
                        && scale.is_finite()
                    {
                        object.location = start.location + moved;
                        object.rotation = rotation;
                        object.scale = scale;
                    }
                }
            }
            Items::Vertices { object, original, selected, followers } => {
                let Ok(world) = self.scene.world_matrix(*object) else { return };
                let inverse = world.inverse();
                let change = modal.change(orthonormal(Mat3::from_mat4(world)), &projector);
                if let Some(mesh) = self.scene.object_mut(*object).ok().and_then(|object| object.mesh_mut()) {
                    for vertex in selected {
                        if let (Some(point), Some(was)) = (mesh.positions.get_mut(*vertex as usize), original.get(*vertex as usize)) {
                            *point = inverse.transform_point3(change.point(world.transform_point3(*was)));
                        }
                    }
                    for (vertex, weight) in followers {
                        if let (Some(point), Some(was)) = (mesh.positions.get_mut(*vertex as usize), original.get(*vertex as usize)) {
                            let before = world.transform_point3(*was);
                            *point = inverse.transform_point3(before + (change.point(before) - before) * *weight);
                        }
                    }
                }
            }
            Items::Bones { object, original, pose, selected } => {
                let Ok(world) = self.scene.world_matrix(*object) else { return };
                let change = modal.change(orthonormal(Mat3::from_mat4(world)), &projector);
                crate::commands_rig::transform_bones(self, *object, original, *pose, selected, &change);
            }
            Items::Curve { object, original, selected } => {
                let (Ok(world), ObjectData::Curve(curve)) = (self.scene.world_matrix(*object), original) else { return };
                let inverse = world.inverse();
                let change = modal.change(orthonormal(Mat3::from_mat4(world)), &projector);
                let mut curve = curve.clone();
                crate::commands_curve::transform_parts(&mut curve, selected, |local| {
                    inverse.transform_point3(change.point(world.transform_point3(local)))
                });
                if let Ok(object) = self.scene.object_mut(*object) {
                    object.data = ObjectData::Curve(curve);
                }
            }
            Items::Lattice { object, original, selected } => {
                let (Ok(world), ObjectData::Lattice(lattice)) = (self.scene.world_matrix(*object), original) else { return };
                let inverse = world.inverse();
                let change = modal.change(orthonormal(Mat3::from_mat4(world)), &projector);
                let mut lattice = lattice.clone();
                for index in selected {
                    if let Some(point) = lattice.points.get_mut(*index as usize) {
                        *point = inverse.transform_point3(change.point(world.transform_point3(*point)));
                    }
                }
                if let Ok(object) = self.scene.object_mut(*object) {
                    object.data = ObjectData::Lattice(lattice);
                }
            }
        }
    }

    /// The view changed under the transform (navigation): what the pointer did so far stays, and
    /// its input starts again from where it is, read in the new view.
    fn modal_follow_view(&mut self, modal: &mut Modal) {
        let now = (self.session.editor.view.clone(), self.session.width, self.session.height);
        if modal.view == now {
            return;
        }
        let offset = modal.change(self.modal_local_frame(modal), &self.projector()).offset;
        let (ratio, turned) = (modal.ratio, modal.turned);
        modal.restart_input();
        modal.base = Base { offset, ratio };
        modal.turned = turned;
        modal.view = now;
        modal.pivot_screen = screen_of(&self.projector(), modal.pivot);
    }

    /// The pointer moved to `at` (`precision`: Shift held, `step`: Ctrl or Cmd held). Answers
    /// whether a transform is under way.
    pub(crate) fn modal_pointer(&mut self, at: (f64, f64), precision: bool, step: bool) -> bool {
        let Some(mut modal) = self.modal.take() else { return false };
        if at.0.is_finite() && at.1.is_finite() {
            self.modal_follow_view(&mut modal);
            let factor = if precision { PRECISION } else { 1.0 };
            modal.virtual_at = (modal.virtual_at.0 + (at.0 - modal.last.0) * factor, modal.virtual_at.1 + (at.1 - modal.last.1) * factor);
            let (px, py) = modal.pivot_screen;
            let (before, after) = ((modal.last.0 - px, modal.last.1 - py), (at.0 - px, at.1 - py));
            // Right by the pivot the direction means nothing: no turn is read there.
            if before.0.hypot(before.1) >= PIVOT_PIXELS && after.0.hypot(after.1) >= PIVOT_PIXELS {
                let mut turn = after.1.atan2(after.0) - before.1.atan2(before.0);
                if turn > std::f64::consts::PI {
                    turn -= std::f64::consts::TAU;
                } else if turn < -std::f64::consts::PI {
                    turn += std::f64::consts::TAU;
                }
                modal.turned += turn * factor;
            }
            modal.last = at;
            if (at.0 - modal.start.0).hypot(at.1 - modal.start.1) > CLICK_PIXELS {
                modal.travelled = true;
            }
            modal.precision = precision;
            modal.step = step;
            self.modal_apply(&mut modal);
        }
        self.modal = Some(modal);
        true
    }

    /// A key during the transform: X, Y, Z (with `shift` the plane across), C clears the axis,
    /// G, R and S switch to that transform, digits, `.`, `-` (negate), `/` (inverse),
    /// Backspace and Tab (with `ctrl` back) type the value. Answers whether it was taken.
    pub(crate) fn modal_key(&mut self, key: &str, shift: bool, ctrl: bool) -> bool {
        let Some(mut modal) = self.modal.take() else { return false };
        let lower = key.to_ascii_lowercase();
        let count = modal.kind.fields(modal.constraint);
        let taken = match lower.as_str() {
            "x" => {
                modal.press_axis(0, shift);
                true
            }
            "y" => {
                modal.press_axis(1, shift);
                true
            }
            "z" => {
                modal.press_axis(2, shift);
                true
            }
            "c" => {
                modal.constraint = None;
                modal.fit_fields();
                true
            }
            "g" | "r" | "s" => {
                let kind = match lower.as_str() {
                    "g" => Kind::Move,
                    "r" => Kind::Rotate,
                    _ => Kind::Scale,
                };
                if kind != modal.kind {
                    modal.kind = kind;
                    modal.numeric = Numeric::default();
                    modal.restart_input();
                    modal.fit_fields();
                }
                true
            }
            "backspace" => modal.numeric.backspace(),
            "tab" => modal.numeric.tab(ctrl, count),
            "-" | "minus" | "subtract" => {
                modal.numeric.negate();
                true
            }
            "/" | "divide" => {
                modal.numeric.inverse();
                true
            }
            other => match other.chars().collect::<Vec<char>>().as_slice() {
                [character] if character.is_ascii_digit() || *character == '.' => modal.numeric.typed(*character),
                _ => false,
            },
        };
        if taken {
            self.modal_apply(&mut modal);
        }
        self.modal = Some(modal);
        taken
    }

    /// The wheel during the transform (`delta` in pixels, down positive): with proportional
    /// editing it resizes the radius. Answers whether it was taken (otherwise the view zooms).
    pub(crate) fn modal_wheel(&mut self, delta: f64) -> bool {
        let Some(modal) = &self.modal else { return false };
        let (Items::Vertices { .. }, Some(mut settings)) = (&modal.items, self.session.proportional) else { return false };
        if !delta.is_finite() {
            return false;
        }
        settings.radius = (settings.radius * (-delta.clamp(-400.0, 400.0) * 0.0015).exp() as f32).clamp(1e-3, 1e6);
        self.session.proportional = Some(settings);
        self.modal_refollow();
        true
    }

    /// Proportional editing's settings changed: the vertices that follow are found again.
    pub(crate) fn modal_refollow(&mut self) {
        let Some(mut modal) = self.modal.take() else { return };
        self.modal_restore(&modal.items);
        if let Items::Vertices { object, selected, followers, .. } = &mut modal.items {
            let set: BTreeSet<u32> = selected.iter().copied().collect();
            *followers = match (self.session.proportional, self.scene.object(*object).ok().and_then(|object| object.mesh())) {
                (Some(settings), Some(mesh)) => kition_scene_model::proportional::weights(mesh, &set, settings),
                _ => Vec::new(),
            };
        }
        self.modal_apply(&mut modal);
        self.modal = Some(modal);
    }

    /// Ends the transform as it is now: one undo step (Shift+D's copies with it), none when it
    /// changes nothing. Answers whether there was one.
    pub(crate) fn modal_confirm(&mut self) -> Result<bool, String> {
        let Some(mut modal) = self.modal.take() else { return Ok(false) };
        if !modal.changes() {
            self.modal_restore(&modal.items);
            self.touched();
            return Ok(true);
        }
        // The scene before the transform, for the step (taken now, not when it began).
        self.modal_restore(&modal.items);
        let before = match modal.duplicate.take() {
            Some(before) => *before,
            None => self.step(modal.items.label(modal.kind)),
        };
        self.modal_apply(&mut modal);
        if let Err(error) = self.scene.validate() {
            self.restore(before);
            self.touched();
            return Err(format!("failed: the transform would break the scene ({error})"));
        }
        self.committed(before)?;
        Ok(true)
    }

    /// Ends the transform, putting everything back; Shift+D's copies stay, where they were made,
    /// as their own step. Answers whether there was one.
    pub(crate) fn modal_cancel(&mut self) -> Result<bool, String> {
        let Some(mut modal) = self.modal.take() else { return Ok(false) };
        self.modal_restore(&modal.items);
        match modal.duplicate.take() {
            Some(before) => self.committed(*before)?,
            None => self.touched(),
        }
        Ok(true)
    }

    /// How the transform began, when one is under way.
    pub(crate) fn modal_started(&self) -> Option<Started> {
        self.modal.as_ref().map(|modal| modal.started)
    }

    /// Whether the transform changes anything now.
    pub(crate) fn modal_changes(&self) -> bool {
        self.modal.as_ref().is_some_and(Modal::changes)
    }

    /// Whether a release at `at` ends the transform as a click.
    pub(crate) fn modal_click(&self, at: (f64, f64)) -> bool {
        self.modal.as_ref().is_some_and(|modal| modal.click(at))
    }

    /// The transform as the tools draw it on the picture: the header at the pointer, the dashed
    /// line from the pivot for a rotation or a scale, the axes it keeps to.
    pub(crate) fn modal_overlays(&self) -> Vec<ToolOverlay> {
        let Some(modal) = &self.modal else { return Vec::new() };
        if matches!(modal.started, Started::Press { .. }) && !modal.travelled && !modal.numeric.on {
            return Vec::new();
        }
        let projector = self.projector();
        let (px, py) = screen_of(&projector, modal.pivot);
        let mut overlays = Vec::new();
        for (axis, _, screen) in self.modal_lines(modal, &projector) {
            let Some((ax, ay, bx, by)) = screen else { continue };
            let color = AXIS_COLORS.get(axis).copied().unwrap_or([255, 255, 255, 255]);
            overlays.push(ToolOverlay::Line { ax, ay, bx, by, color, dashed: false });
        }
        if modal.kind != Kind::Move {
            overlays.push(ToolOverlay::Line { ax: px, ay: py, bx: modal.last.0, by: modal.last.1, color: DASH, dashed: true });
        }
        overlays.push(ToolOverlay::Measure { x: modal.last.0, y: modal.last.1, text: modal.header(self.modal_radius(modal)) });
        overlays
    }

    /// The axes the transform keeps to, each with its world direction and, when it does not run
    /// along the view, the line through the pivot across the picture.
    #[allow(clippy::type_complexity)]
    fn modal_lines(&self, modal: &Modal, projector: &Projector) -> Vec<(usize, Vec3, Option<(f64, f64, f64, f64)>)> {
        let Some(constraint) = modal.constraint else { return Vec::new() };
        let frame = modal.frame(self.modal_local_frame(modal));
        let axes = if constraint.plane { constraint.free() } else { vec![constraint.axis] };
        let (px, py) = screen_of(projector, modal.pivot);
        let reach = f64::from(self.session.width.max(self.session.height)) * 2.0;
        let size = projector.pixel_size_at(modal.pivot).max(1e-9);
        axes.into_iter()
            .map(|axis| {
                let direction = (frame * unit(axis)).normalize_or(unit(axis));
                let (ex, ey) = screen_of(projector, modal.pivot + direction * size * 40.0);
                let (dx, dy) = (ex - px, ey - py);
                let length = dx.hypot(dy);
                let screen = (length > 1e-3 && length.is_finite() && px.is_finite() && py.is_finite()).then(|| {
                    let (ux, uy) = (dx / length * reach, dy / length * reach);
                    (px - ux, py - uy, px + ux, py + uy)
                });
                (axis, direction, screen)
            })
            .collect()
    }

    /// Whether the transform still has what it began with: the mode it began in and the selection
    /// it moves (another route may change them). One that does not is cancelled.
    fn modal_still_fits(&mut self) {
        let Some(modal) = &self.modal else { return };
        let fits = match &modal.items {
            Items::Objects(objects) => {
                self.session.mode == Mode::Object && objects.iter().all(|start| self.session.editor.selected.contains(&start.id))
            }
            Items::Vertices { object, .. } => self.session.mode == Mode::Edit(*object),
            Items::Bones { object, .. } => self.session.rig.as_ref().is_some_and(|rig| rig.object == *object),
            Items::Curve { object, .. } | Items::Lattice { object, .. } => self.session.shape.as_ref().is_some_and(|shape| match shape {
                crate::commands_curve::ShapeSession::Curve(edit) => edit.object == *object,
                crate::commands_curve::ShapeSession::Lattice(edit) => edit.object == *object,
                crate::commands_curve::ShapeSession::Text(_) => false,
            }),
        };
        if !fits {
            let _ = self.modal_cancel();
        }
    }

    /// `modalStep`, the editor's quick route for the pointer and keys during a transform: runs
    /// `modal.pointer`, `modal.key` or `modal.state` without moving the document's revision (no
    /// software picture, no inspect) and answers the transform, with the GPU viewport's frame
    /// when `frame` and a transform is under way.
    pub fn modal_step(&mut self, command: &str, params: &str, frame: bool) -> Result<String, String> {
        let params: Value = serde_json::from_str(params).map_err(|error| bad(format!("the parameters are not JSON ({error})")))?;
        let answer = match command {
            "modal.pointer" => pointer(self, &params, false)?,
            "modal.key" => key(self, &params, false)?,
            "modal.state" => self.modal_answer(),
            other => return Err(format!("unknown-command: {other} is not a quick step of the transform")),
        };
        let mut answer = answer;
        if frame
            && self.modal.is_some()
            && let Some(fields) = answer.as_object_mut()
        {
            fields.insert("frame".into(), self.gl_scene());
        }
        Ok(answer.to_string())
    }
}

/// The screen point `[x, y]` of parameter `name`, if given.
fn point(params: &Value, name: &str) -> Result<Option<(f64, f64)>, String> {
    let Some(value) = params.get(name).filter(|value| !value.is_null()) else { return Ok(None) };
    match value.as_array().map(|items| items.iter().map(Value::as_f64).collect::<Vec<_>>()).as_deref() {
        Some([Some(x), Some(y)]) if x.is_finite() && y.is_finite() && x.abs() <= 1e7 && y.abs() <= 1e7 => Ok(Some((*x, *y))),
        _ => Err(bad(format!("`{name}` must be [x, y] in viewport pixels"))),
    }
}

fn command(id: &str, title: &str, schema: Value) -> KitCommand {
    KitCommand {
        id: id.into(),
        title: title.into(),
        menu: None,
        shortcut: None,
        params: None,
        schema: Some(schema),
        mutates: false,
        enabled: true,
        ..Default::default()
    }
}

/// The transform's commands: session commands for the editor (no undo step of their own;
/// `modal.confirm` makes the transform's).
pub fn catalogue(_doc: &SceneDocument) -> Vec<KitCommand> {
    vec![
        command(
            "modal.begin",
            "Begin Transform",
            s::of(vec![
                ("kind", s::required(s::choice("What the transform does", &["move", "rotate", "scale"]))),
                (
                    "at",
                    json!({ "type": "array", "items": { "type": "number" }, "description": "Where the pointer is, [x, y] in viewport pixels; its last place when left out" }),
                ),
                ("axis", s::choice("An axis to keep to from the start (a handle, a normal); free when left out", &["x", "y", "z"])),
                ("plane", s::boolean("Keep to the plane across `axis` instead (false)")),
                ("orientation", s::choice("Whose axes `axis` is (global)", &["global", "local"])),
                ("duplicate", s::boolean("Copy the selection first and move the copies (Shift+D; false)")),
                ("press", s::boolean("A press started it: the release confirms it, and a release where it began is a click (false)")),
            ]),
        ),
        command(
            "modal.pointer",
            "Transform Pointer",
            s::of(vec![
                (
                    "at",
                    s::required(
                        json!({ "type": "array", "items": { "type": "number" }, "description": "Where the pointer is, [x, y] in viewport pixels" }),
                    ),
                ),
                ("precision", s::boolean("Shift held: the pointer's motion counts for a tenth")),
                ("step", s::boolean("Ctrl or Cmd held: the value goes in steps (1 m, 5°, 0.1; with precision 0.1 m, 1°, 0.01)")),
            ]),
        ),
        command(
            "modal.key",
            "Transform Key",
            s::of(vec![
                ("key", s::required(s::string("x, y, z, c, g, r, s, a digit, ., -, /, Backspace or Tab"))),
                ("shift", s::boolean("Shift held: X, Y and Z lock the plane across the axis")),
                ("ctrl", s::boolean("Ctrl held: Tab goes back a field")),
            ]),
        ),
        command("modal.wheel", "Transform Wheel", s::of(vec![("delta", s::required(s::number("The wheel's turn in pixels, down positive")))])),
        command("modal.confirm", "Confirm Transform", s::of(vec![])),
        command("modal.cancel", "Cancel Transform", s::of(vec![])),
        command("modal.state", "Transform State", s::of(vec![])),
    ]
}

/// The ones that cannot run now (none: each answers what it did).
pub fn disabled(_doc: &SceneDocument) -> Vec<String> {
    Vec::new()
}

/// Runs one of the transform's commands; `None` for any other.
pub fn execute(doc: &mut SceneDocument, command: &str, params: &Value) -> Option<Result<Value, String>> {
    if !command.starts_with("modal.") {
        return None;
    }
    doc.modal_still_fits();
    Some(match command {
        "modal.begin" => begin(doc, params),
        "modal.pointer" => pointer(doc, params, true),
        "modal.key" => key(doc, params, true),
        "modal.wheel" => wheel(doc, params),
        "modal.confirm" => doc.modal_confirm().map(|ended| json!({ "confirmed": ended, "active": false })),
        "modal.cancel" => doc.modal_cancel().map(|ended| json!({ "cancelled": ended, "active": false })),
        "modal.state" => Ok(doc.modal_answer()),
        other => Err(format!("unknown-command: {other}")),
    })
}

fn begin(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let kind = params::string(params, "kind")?.ok_or_else(|| bad("`kind` is required"))?;
    let kind = Kind::parse(kind).ok_or_else(|| bad("`kind` must be move, rotate or scale"))?;
    let at = point(params, "at")?;
    let axis = match params::string(params, "axis")? {
        None => None,
        Some("x") => Some(0),
        Some("y") => Some(1),
        Some("z") => Some(2),
        Some(_) => return Err(bad("`axis` must be x, y or z")),
    };
    let orientation = match params::string(params, "orientation")? {
        None => Orientation::Global,
        Some(name) => Orientation::parse(name).ok_or_else(|| bad("`orientation` must be global or local"))?,
    };
    let plane = params::boolean(params, "plane")?.unwrap_or(false);
    let constraint = axis.map(|axis| Constraint { axis, plane, orientation });
    let duplicate = params::boolean(params, "duplicate")?.unwrap_or(false);
    let started = if params::boolean(params, "press")?.unwrap_or(false) { Started::Press { select: false } } else { Started::Key };
    doc.modal_begin(kind, at, started, constraint, duplicate)?;
    Ok(doc.modal_answer())
}

/// `modal.pointer`; `touch` moves the revision (the command route: the software picture follows).
fn pointer(doc: &mut SceneDocument, params: &Value, touch: bool) -> Result<Value, String> {
    let at = point(params, "at")?.ok_or_else(|| bad("`at` is required"))?;
    let precision = params::boolean(params, "precision")?.unwrap_or(false);
    let step = params::boolean(params, "step")?.unwrap_or(false);
    if doc.modal_pointer(at, precision, step) && touch {
        doc.touched();
    }
    Ok(doc.modal_answer())
}

fn key(doc: &mut SceneDocument, params: &Value, touch: bool) -> Result<Value, String> {
    let name = params::string(params, "key")?.ok_or_else(|| bad("`key` is required"))?;
    let shift = params::boolean(params, "shift")?.unwrap_or(false);
    let ctrl = params::boolean(params, "ctrl")?.unwrap_or(false);
    let taken = doc.modal_key(name, shift, ctrl);
    if taken && touch {
        doc.touched();
    }
    let mut answer = doc.modal_answer();
    if let Some(fields) = answer.as_object_mut() {
        fields.insert("taken".into(), json!(taken));
    }
    Ok(answer)
}

fn wheel(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let delta = params::number(params, "delta")?.ok_or_else(|| bad("`delta` is required"))?;
    let taken = doc.modal_wheel(f64::from(delta));
    if taken {
        doc.touched();
    }
    let mut answer = doc.modal_answer();
    if let Some(fields) = answer.as_object_mut() {
        fields.insert("taken".into(), json!(taken));
    }
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_numbers_read_as_blender_reads_them() {
        let mut numeric = Numeric::default();
        assert!(numeric.typed('2'));
        assert_eq!(numeric.value(0, false, 0.0), 2.0);
        numeric.negate();
        assert_eq!(numeric.value(0, false, 0.0), -2.0, "minus negates the whole number, at any time");
        assert!(numeric.typed('.'));
        assert!(numeric.typed('5'));
        assert_eq!(numeric.value(0, false, 0.0), -2.5);
        assert!(numeric.typed('.'), "a second point is taken and ignored");
        assert_eq!(numeric.fields[0].text, "2.5");
        assert!(numeric.tab(false, 3));
        assert_eq!(numeric.index, 1);
        assert_eq!(numeric.value(1, false, 0.0), 0.0, "an untyped field is nothing");
        assert_eq!(numeric.value(1, true, 1.0), -2.5, "a scale's untyped field takes the first");
        assert!(numeric.tab(true, 3));
        assert_eq!(numeric.index, 0);
        assert!(numeric.backspace());
        assert_eq!(numeric.fields[0].text, "2.");
        assert!(numeric.backspace() && numeric.backspace());
        assert!(numeric.fields[0].edited, "an emptied field still counts as typed (zero)");
        assert!(numeric.backspace());
        assert!(!numeric.fields[0].edited, "Backspace on an empty field forgets it");
        assert!(numeric.backspace());
        assert!(!numeric.on, "and once more stops typing");
        numeric.inverse();
        assert!(numeric.typed('4'));
        assert_eq!(numeric.value(0, false, 0.0), 0.25, "slash takes the inverse");
    }

    #[test]
    fn axis_keys_cycle_global_local_free() {
        let mut constraint = None;
        let press = |current: Option<Constraint>, axis: usize, plane: bool| match current {
            Some(c) if c.axis == axis && c.plane == plane => match c.orientation {
                Orientation::Global => Some(Constraint { orientation: Orientation::Local, ..c }),
                Orientation::Local => None,
            },
            _ => Some(Constraint { axis, plane, orientation: Orientation::Global }),
        };
        constraint = press(constraint, 0, false);
        assert_eq!(constraint.map(|c| c.text()), Some(" along global X".to_owned()));
        constraint = press(constraint, 0, false);
        assert_eq!(constraint.map(|c| c.text()), Some(" along local X".to_owned()));
        constraint = press(constraint, 0, false);
        assert_eq!(constraint, None);
        constraint = press(constraint, 2, true);
        assert_eq!(constraint.map(|c| (c.text(), c.free())), Some((" locking global Z".to_owned(), vec![0, 1])));
        constraint = press(constraint, 1, false);
        assert_eq!(constraint.map(|c| c.free()), Some(vec![1]), "another axis starts global again");
    }

    #[test]
    fn a_field_shows_what_is_typed_and_its_value() {
        let field = Field { text: "2".into(), edited: true, negate: true, inverse: false };
        assert_eq!(field.shown("-2.0000 m"), "[-(2|)] = -2.0000 m");
        assert_eq!(field.value(), Some(-2.0));
        assert_eq!(Field::default().value(), None);
    }
}
