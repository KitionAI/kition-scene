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
use kition_scene_model::{Id, ObjectData, Property};
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

/// What the modal does: a transform (move, rotate, scale), or one of edit mode's operations
/// that the pointer sizes (inset, bevel, a loop cut's choice of loop and then its slide).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Move,
    Rotate,
    Scale,
    /// The pointer's distance to the selection's centre is the thickness; with Ctrl, the depth.
    Inset,
    /// The pointer's distance from where it began is the width; the wheel adds segments.
    Bevel,
    /// The loop under the pointer is shown; the wheel and typed digits set the number of cuts.
    LoopCut,
    /// The cut loop slides along the edges it crosses.
    Slide,
}

impl Kind {
    /// One of the transforms (move, rotate, scale).
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
            Kind::Inset => "inset",
            Kind::Bevel => "bevel",
            Kind::LoopCut => "loopCut",
            Kind::Slide => "slide",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Kind::Move => "Move",
            Kind::Rotate => "Rotate",
            Kind::Scale => "Scale",
            Kind::Inset => "Inset Faces",
            Kind::Bevel => "Bevel Edges",
            Kind::LoopCut | Kind::Slide => "Loop Cut",
        }
    }

    /// A transform of what is selected, as opposed to an operation run again from the mesh as it
    /// was.
    fn transform(self) -> bool {
        matches!(self, Kind::Move | Kind::Rotate | Kind::Scale)
    }

    /// The fields typed numbers go to: a move takes one per free axis, a rotation one angle, a
    /// scale one per free axis; an inset its thickness and depth; the others one.
    fn fields(self, constraint: Option<Constraint>) -> usize {
        match (self, constraint) {
            (Kind::Rotate | Kind::Bevel | Kind::LoopCut | Kind::Slide, _) => 1,
            (Kind::Inset, _) => 2,
            (_, None) => 3,
            (_, Some(constraint)) => constraint.free().len(),
        }
    }
}

/// Whose axes a constraint follows (the transform orientation the person picks, kept in the view
/// state, is the first an axis key takes).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Orientation {
    #[default]
    Global,
    /// Each object's own axes (in edit modes, the edited object's).
    Local,
    /// The selected faces' mean normal as Z (an extrusion's move; in edit mode the selection's).
    Normal,
    /// The view's: X right, Y up, Z towards the eye, as the view was when the transform began.
    View,
}

impl Orientation {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "global" => Some(Orientation::Global),
            "local" => Some(Orientation::Local),
            "normal" => Some(Orientation::Normal),
            "view" => Some(Orientation::View),
            _ => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Orientation::Global => "global",
            Orientation::Local => "local",
            Orientation::Normal => "normal",
            Orientation::View => "view",
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

    /// As the header ends: ` along global X`, ` locking local Z`, ` normal` (along the normal).
    fn text(&self) -> String {
        if self.orientation == Orientation::Normal && !self.plane && self.axis == 2 {
            return " normal".into();
        }
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
    /// The edited mesh as it was, and its selected vertices, for an operation that runs again
    /// from them on every update (inset, bevel, a loop cut).
    Rerun { object: Id, original: ObjectData, selected: BTreeSet<u32> },
}

impl Items {
    /// The object whose axes are the local ones (the first object, or the edited one).
    fn first_object(&self) -> Option<Id> {
        match self {
            Items::Objects(objects) => objects.first().map(|start| start.id),
            Items::Vertices { object, .. }
            | Items::Bones { object, .. }
            | Items::Curve { object, .. }
            | Items::Lattice { object, .. }
            | Items::Rerun { object, .. } => Some(*object),
        }
    }

    /// The undo step's name.
    fn label(&self, kind: Kind) -> &'static str {
        match self {
            Items::Objects(_) | Items::Vertices { .. } | Items::Rerun { .. } => kind.title(),
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
/// frame), a rotation's angle (radians), a scale's factors (along the frame's axes); an
/// operation's amounts (an inset's thickness and depth, a bevel's width, a slide's factor) and
/// count (a bevel's segments, a loop cut's cuts).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Values {
    pub offset: Vec3,
    pub angle: f32,
    pub factors: Vec3,
    pub amounts: [f32; 2],
    pub count: u32,
}

impl Default for Values {
    fn default() -> Self {
        Self { offset: Vec3::ZERO, angle: 0.0, factors: Vec3::ONE, amounts: [0.0, 0.0], count: 1 }
    }
}

/// An extrusion under way: its geometry is made (at no distance) and the modal moves it. The
/// scene before it, for the step; whether it extruded loose vertices and edges (then the move is
/// free) or faces (then it keeps to their mean normal, `normal` in the object's own axes).
#[derive(Debug, Clone)]
struct Extrusion {
    before: Box<Step>,
    loose: bool,
    normal: Vec3,
}

/// What a confirmed transform was, for Adjust Last Operation to run it again with other values.
#[derive(Debug, Clone)]
pub(crate) struct Replay {
    pub kind: Kind,
    pub duplicate: bool,
    /// A free rotation's axis (the view's, when it was confirmed).
    pub view_axis: Vec3,
    /// The normal frame an extrusion's move kept to.
    pub normal: Mat3,
    /// The view's frame when it was confirmed (the view orientation).
    pub view_frame: Mat3,
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
    /// E: the extrusion the move carries.
    extrusion: Option<Extrusion>,
    /// The frame of `Orientation::Normal` (the extruded faces' normal as Z), world space.
    normal: Mat3,
    /// Values given outright (Adjust Last Operation's replay), and a free rotation's axis.
    fixed: Option<(Values, Option<Vec3>)>,
    /// An inset's Ctrl: the pointer sizes the depth. The amounts the pointer starts from.
    depth: bool,
    base_amounts: [f32; 2],
    /// A bevel's segments (the wheel) and a loop cut's cuts (the wheel), before typing.
    count: u32,
    /// The edge a loop cut crosses, and for its slide the edge's direction on the screen (from
    /// its lower-numbered end).
    edge: Option<(u32, u32)>,
    edge_screen: (f64, f64),
    /// The selection as it was before the modal (for Adjust Last Operation).
    selected_before: crate::operation::Selected,
    /// The orientation an axis key takes first (the person's transform orientation), and the
    /// view's axes when the transform began (the view orientation's).
    preferred: Orientation,
    view_frame: Mat3,
    /// Individual origins: each object turns and scales about its own origin; each selected
    /// vertex about the middle of its connected part of the selection (`origins`, world space, in
    /// the order of the selected vertices).
    individual: bool,
    origins: Vec<Vec3>,
    /// Snapping as the person set it (Shift+Tab during the transform toggles it; Ctrl inverts it
    /// while held), and what to.
    snap_on: bool,
    snap_target: crate::cursor::SnapTarget,
    /// What snapping to elements looks at, made the first time it snaps; and the point it
    /// snapped to last (none when it did not).
    snap_index: Option<std::sync::Arc<crate::snap::SnapIndex>>,
    snapped: Option<Vec3>,
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
pub(crate) fn orthonormal(matrix: Mat3) -> Mat3 {
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
pub(crate) fn into_screen(projector: &Projector, point: Vec3) -> Vec3 {
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

/// What a move snaps from: the objects it moves, or the edited mesh's selected vertices.
pub(crate) enum SnapItems {
    Objects(Vec<Id>),
    Vertices(Id, Vec<u32>),
    Other,
}

impl Modal {
    /// What snapping snaps to.
    pub(crate) fn snap_target(&self) -> crate::cursor::SnapTarget {
        self.snap_target
    }

    /// The point it turns about.
    pub(crate) fn pivot_point(&self) -> Vec3 {
        self.pivot
    }

    /// What it moves, which snapping leaves out: the objects whose data or place it changes, the
    /// edited mesh, and that mesh's vertices that move (the selected ones and those that follow).
    pub(crate) fn moved_parts(&self) -> (BTreeSet<Id>, Option<Id>, BTreeSet<u32>) {
        match &self.items {
            Items::Objects(objects) => (objects.iter().map(|start| start.id).collect(), None, BTreeSet::new()),
            Items::Vertices { object, selected, followers, .. } => {
                (BTreeSet::new(), Some(*object), selected.iter().copied().chain(followers.iter().map(|(vertex, _)| *vertex)).collect())
            }
            Items::Bones { object, .. } | Items::Curve { object, .. } | Items::Lattice { object, .. } | Items::Rerun { object, .. } => {
                (std::iter::once(*object).collect(), None, BTreeSet::new())
            }
        }
    }

    /// What snaps onto the target.
    pub(crate) fn snap_items(&self) -> SnapItems {
        match &self.items {
            Items::Objects(objects) => SnapItems::Objects(objects.iter().map(|start| start.id).collect()),
            Items::Vertices { object, selected, .. } => SnapItems::Vertices(*object, selected.clone()),
            _ => SnapItems::Other,
        }
    }

    /// The frame a constraint follows for an item whose own axes are `own`.
    fn frame(&self, own: Mat3) -> Mat3 {
        match self.constraint {
            Some(Constraint { orientation: Orientation::Local, .. }) => own,
            Some(Constraint { orientation: Orientation::Normal, .. }) => self.normal,
            Some(Constraint { orientation: Orientation::View, .. }) => self.view_frame,
            _ => Mat3::IDENTITY,
        }
    }

    /// Whether the transform snaps now: the person's setting, inverted while Ctrl is held.
    fn snapping(&self) -> bool {
        self.step != self.snap_on
    }

    /// Whether it steps now (snapping to increments; a rotation or a scale steps when it snaps
    /// to elements too, which only a move does).
    fn stepping(&self) -> bool {
        self.snapping() && (self.snap_target == crate::cursor::SnapTarget::Increment || self.kind != Kind::Move)
    }

    /// Whether it snaps to the scene's elements now (a move, the pointer giving the value).
    fn snapping_to_elements(&self) -> bool {
        self.kind == Kind::Move
            && self.snapping()
            && self.snap_target != crate::cursor::SnapTarget::Increment
            && !self.numeric.on
            && self.fixed.is_none()
    }

    /// Whether it would change anything now (an operation always does, once it runs).
    fn changes(&self) -> bool {
        self.duplicate.is_some()
            || self.extrusion.is_some()
            || match self.kind {
                Kind::Move => self.values.offset != Vec3::ZERO,
                Kind::Rotate => self.values.angle != 0.0,
                Kind::Scale => self.values.factors != Vec3::ONE,
                Kind::Inset | Kind::Bevel | Kind::Slide => true,
                Kind::LoopCut => false,
            }
    }

    /// The pointer pressed or released here is a click: no travel from where it began, no number
    /// typed, no axis chosen.
    fn click(&self, at: (f64, f64)) -> bool {
        !self.travelled && !self.numeric.on && self.constraint.is_none() && (at.0 - self.start.0).hypot(at.1 - self.start.1) <= CLICK_PIXELS
    }

    /// The pointer's distance to the pivot on the screen now and where its input started, in
    /// world units at the pivot (per object scale `scale`).
    fn pivot_distances(&self, projector: &Projector, scale: f32) -> (f32, f32) {
        let (px, py) = self.pivot_screen;
        let size = projector.pixel_size_at(self.pivot) / scale.max(1e-6);
        let now = (self.virtual_at.0 - px).hypot(self.virtual_at.1 - py) as f32 * size;
        let start = (self.start.0 - px).hypot(self.start.1 - py) as f32 * size;
        (now, start)
    }

    /// An operation's value from the typed numbers or the pointer (`scale`: the edited object's
    /// largest scale, which the amounts are measured in).
    fn solve_operation(&self, projector: &Projector, scale: f32) -> Values {
        let mut values = Values { count: self.count, amounts: self.base_amounts, ..Values::default() };
        let typed = |index: usize| self.numeric.fields.get(index).and_then(Field::value);
        if self.numeric.on {
            match self.kind {
                Kind::Inset => values.amounts = [typed(0).unwrap_or(0.0).max(0.0), typed(1).unwrap_or(0.0)],
                Kind::Bevel => values.amounts[0] = typed(0).unwrap_or(0.0).max(0.0),
                // Cuts are whole, from 1 to 64; nothing typed yet keeps the wheel's.
                Kind::LoopCut => values.count = typed(0).map_or(self.count, |cuts| cuts.round().clamp(1.0, 64.0) as u32),
                // Typed as Blender reads a slide: -1 to 1 across the edges, 0 the middle.
                Kind::Slide => values.amounts[0] = (0.5 + typed(0).unwrap_or(0.0) * 0.5).clamp(0.0, 1.0),
                Kind::Move | Kind::Rotate | Kind::Scale => {}
            }
            return values;
        }
        let step = self.step.then_some(self.precision);
        match self.kind {
            Kind::Inset => {
                let (now, start) = self.pivot_distances(projector, scale);
                if self.depth {
                    values.amounts[1] = self.base_amounts[1] + (now - start);
                } else {
                    // Towards the centre is thicker.
                    values.amounts[0] = (self.base_amounts[0] - (now - start)).max(0.0);
                }
            }
            Kind::Bevel => {
                let (now, start) = self.pivot_distances(projector, scale);
                let width = (self.base_amounts[0] + (now - start)).max(0.0);
                values.amounts[0] = match step {
                    Some(fine) => snapped(width, if fine { 0.01 } else { 0.1 }),
                    None => width,
                };
            }
            Kind::Slide => {
                let (ex, ey) = self.edge_screen;
                let length = ex * ex + ey * ey;
                let (dx, dy) = (self.virtual_at.0 - self.start.0, self.virtual_at.1 - self.start.1);
                let along = if length > 1e-6 { (dx * ex + dy * ey) / length } else { 0.0 };
                let factor = (0.5 + along as f32).clamp(0.0, 1.0);
                values.amounts[0] = match step {
                    Some(fine) => snapped(factor, if fine { 0.01 } else { 0.1 }),
                    None => factor,
                };
            }
            Kind::LoopCut | Kind::Move | Kind::Rotate | Kind::Scale => {}
        }
        values
    }

    /// The value from the typed numbers or the pointer.
    fn solve(&self, projector: &Projector) -> (Values, f64) {
        if let Some((values, _)) = self.fixed {
            return (values, self.ratio);
        }
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
                Kind::Inset | Kind::Bevel | Kind::LoopCut | Kind::Slide => {}
            }
            return (values, self.ratio);
        }
        let step = self.stepping().then_some(self.precision);
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
            Kind::Inset | Kind::Bevel | Kind::LoopCut | Kind::Slide => {}
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
                    None => self.fixed.and_then(|(_, axis)| axis).unwrap_or_else(|| into_screen(projector, self.pivot)),
                    Some(constraint) => (frame * unit(constraint.axis)).normalize_or(Vec3::Z),
                };
                (Mat3::from_axis_angle(axis, values.angle), Vec3::ZERO)
            }
            Kind::Scale => (frame * Mat3::from_diagonal(values.factors) * frame.transpose(), Vec3::ZERO),
            Kind::Inset | Kind::Bevel | Kind::LoopCut | Kind::Slide => (Mat3::IDENTITY, Vec3::ZERO),
        };
        Change { kind: self.kind, linear, offset, pivot: self.pivot }
    }

    /// Takes an axis key: the axis (or with `plane` the plane across it) in the global frame,
    /// pressed again the local one, a third time free again.
    fn press_axis(&mut self, axis: usize, plane: bool) {
        self.constraint = next_constraint(self.constraint, self.preferred, axis, plane);
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
            Kind::Inset => {
                let [thickness, depth] = self.values.amounts;
                text.push_str(&format!("Thickness: {}, Depth: {}", self.field_text(0, thickness, " m"), self.field_text(1, depth, " m")));
            }
            Kind::Bevel => {
                text.push_str(&format!("Offset: {}, Segments: {}", self.field_text(0, self.values.amounts[0], " m"), self.values.count));
            }
            Kind::LoopCut => {
                let cuts = match self.numeric.fields.first() {
                    Some(field) if self.numeric.on && field.edited => field.shown(&self.values.count.to_string()),
                    _ => self.values.count.to_string(),
                };
                text.push_str(&format!("Cuts: {cuts}"));
            }
            Kind::Slide => {
                text.push_str(&format!("Edge Slide: {}", self.field_text(0, self.values.amounts[0] * 2.0 - 1.0, "")));
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
            Kind::Inset => json!({ "thickness": modal.values.amounts[0], "depth": modal.values.amounts[1] }),
            Kind::Bevel => json!({ "width": modal.values.amounts[0], "segments": modal.values.count }),
            Kind::LoopCut => json!({ "cuts": modal.values.count }),
            Kind::Slide => json!({ "factor": modal.values.amounts[0], "cuts": modal.values.count }),
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
            "dashed": matches!(modal.kind, Kind::Rotate | Kind::Scale | Kind::Inset | Kind::Bevel),
            "wheel": matches!(modal.kind, Kind::Bevel | Kind::LoopCut),
            "extrude": modal.extrusion.is_some(),
            "edge": modal.edge.map(|(a, b)| [a, b]),
            // A loop cut's loops before its click: world segments, and the same on the screen.
            "loops": match (modal.kind, modal.edge) {
                (Kind::LoopCut, Some(edge)) => self.loop_segments(edge, modal.values.count).into_iter().map(|(a, b)| [a.x, a.y, a.z, b.x, b.y, b.z]).collect::<Vec<_>>(),
                _ => Vec::new(),
            },
            "loopScreen": match (modal.kind, modal.edge) {
                (Kind::LoopCut, Some(edge)) => self
                    .loop_lines(edge, modal.values.count)
                    .into_iter()
                    .filter_map(|line| match line {
                        ToolOverlay::Line { ax, ay, bx, by, .. } => Some([ax, ay, bx, by]),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            },
            "proportional": self.modal_radius(modal),
            "proportionalPixels": self.modal_radius(modal).map(|radius| f64::from(radius / projector.pixel_size_at(modal.pivot).max(1e-9))).filter(|pixels| pixels.is_finite()),
            "duplicate": modal.duplicate.is_some(),
            "snap": {
                "on": modal.snapping(),
                "target": modal.snap_target.name(),
                "point": modal.snapped.map(|point| point.to_array()),
                "screen": modal.snapped.map(|point| { let (x, y) = screen_of(&projector, point); [x, y] }),
            },
            "orientation": modal.preferred.name(),
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
            let pivot = self.transform_pivot(crate::commands_rig::pivot(self, rig))?;
            return Some((Items::Bones { object: rig.object, original, pose: rig.pose, selected: rig.selected.clone() }, pivot));
        }
        match &session.shape {
            Some(crate::commands_curve::ShapeSession::Curve(edit)) => {
                if edit.selected.is_empty() {
                    return None;
                }
                let original = self.scene.object(edit.object).ok()?.data.clone();
                let pivot = self.transform_pivot(crate::commands_curve::pivot(self, edit))?;
                return Some((Items::Curve { object: edit.object, original, selected: edit.selected.clone() }, pivot));
            }
            Some(crate::commands_curve::ShapeSession::Lattice(edit)) => {
                if edit.selected.is_empty() {
                    return None;
                }
                let original = self.scene.object(edit.object).ok()?.data.clone();
                let pivot = self.transform_pivot(crate::commands_lattice::pivot(self, edit))?;
                return Some((Items::Lattice { object: edit.object, original, selected: edit.selected.clone() }, pivot));
            }
            Some(crate::commands_curve::ShapeSession::Text(_)) => return None,
            None => {}
        }
        let pivot = self.transform_pivot(None)?;
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
        let at = self.modal_at(at);
        let selected_before = crate::operation::Selected::of(self);
        let copied = if duplicate { Some(Box::new(self.duplicate_for_modal()?)) } else { None };
        let Some((items, pivot)) = self.modal_items() else {
            if let Some(before) = copied {
                self.restore(*before);
                self.touched();
            }
            return Err("not-allowed: select something to transform first".into());
        };
        let mut modal = self.new_modal(kind, items, pivot, at, started, selected_before);
        modal.constraint = constraint;
        modal.duplicate = copied;
        self.modal_settle(&mut modal);
        modal.fit_fields();
        self.modal = Some(modal);
        if duplicate {
            // The copies show at once (they are new objects or vertices).
            self.touched();
        }
        Ok(())
    }

    /// Where a modal begins: `at`, else the pointer's last place, else the viewport's middle.
    fn modal_at(&self, at: Option<(f64, f64)>) -> (f64, f64) {
        at.or(self.tool.pointer_at())
            .filter(|(x, y)| x.is_finite() && y.is_finite())
            .unwrap_or((f64::from(self.session.width) / 2.0, f64::from(self.session.height) / 2.0))
    }

    /// A modal of `kind` over `items` about `pivot`, begun at `at`, with nothing done yet.
    fn new_modal(
        &self,
        kind: Kind,
        items: Items,
        pivot: Vec3,
        at: (f64, f64),
        started: Started,
        selected_before: crate::operation::Selected,
    ) -> Modal {
        let projector = self.projector();
        let local =
            items.first_object().and_then(|id| self.scene.world_matrix(id).ok()).map_or(Mat3::IDENTITY, |world| orthonormal(Mat3::from_mat4(world)));
        Modal {
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
            constraint: None,
            numeric: Numeric::default(),
            precision: false,
            step: false,
            travelled: false,
            duplicate: None,
            values: Values::default(),
            ratio: 1.0,
            extrusion: None,
            normal: Mat3::IDENTITY,
            fixed: None,
            depth: false,
            base_amounts: [0.0, 0.0],
            count: 1,
            edge: None,
            edge_screen: (0.0, 0.0),
            selected_before,
            preferred: self.session.editor.orientation,
            view_frame: orthonormal(Mat3::from_mat4(projector.view.inverse())),
            individual: false,
            origins: Vec::new(),
            snap_on: self.session.editor.snap,
            snap_target: self.session.editor.snap_target,
            snap_index: None,
            snapped: None,
        }
    }

    /// The pivot point's per-item parts and the orientation's frames for a transform about to
    /// begin: individual origins (each object's own, each connected part of the vertices'), and
    /// the selection's normal for the normal orientation.
    fn modal_settle(&self, modal: &mut Modal) {
        if self.session.editor.pivot == crate::cursor::PivotPoint::Individual && modal.kind.transform() {
            match &modal.items {
                Items::Objects(_) => modal.individual = true,
                Items::Vertices { object, original, selected, .. } => {
                    if let (Ok(world), Some(mesh)) =
                        (self.scene.world_matrix(*object), self.scene.object(*object).ok().and_then(|object| object.mesh()))
                    {
                        let mut kept = mesh.clone();
                        kept.positions.clone_from(original);
                        modal.origins =
                            crate::cursor::island_middles(&kept, selected).into_iter().map(|point| world.transform_point3(point)).collect();
                        modal.individual = true;
                    }
                }
                _ => {}
            }
        }
        if modal.extrusion.is_none()
            && (modal.preferred == Orientation::Normal || modal.constraint.is_some_and(|constraint| constraint.orientation == Orientation::Normal))
        {
            modal.normal = self.orientation_frame(Orientation::Normal);
        }
    }

    /// Starts one of edit mode's operations where the pointer is (`at`; its last place when
    /// `None`): `extrude` makes the extrusion and moves it (along the faces' normal, or freely
    /// for loose vertices and edges), `inset` and `bevel` size it by the pointer, `loopCut` shows
    /// the loop under the pointer until a click cuts it and slides it. A modal under way is
    /// cancelled first.
    pub(crate) fn modal_begin_operation(&mut self, name: &str, at: Option<(f64, f64)>, started: Started) -> Result<(), String> {
        if self.modal.is_some() {
            self.modal_cancel()?;
        }
        if let Some(before) = self.field_preview.take() {
            self.scene = before.scene;
        }
        let Mode::Edit(object) = self.session.mode else {
            return Err("not-allowed: the edit mode's operations work on the mesh being edited".into());
        };
        if self.session.rig.is_some() || self.session.shape.is_some() {
            return Err("not-allowed: the edit mode's operations work on a mesh".into());
        }
        let at = self.modal_at(at);
        let selected_before = crate::operation::Selected::of(self);
        let original = self.scene.object(object).map_err(|error| error.to_string())?.data.clone();
        let ObjectData::Mesh(mesh) = &original else { return Err("unsupported: the edited object is not a mesh".into()) };
        let faces = crate::commands::faces_selected(&self.session, mesh);
        let has_edges = mesh.edges().into_iter().any(|[a, b]| self.session.vertices.contains(&a) && self.session.vertices.contains(&b));
        match name {
            "extrude" => {
                if self.session.vertices.is_empty() {
                    return Err("not-allowed: select something to extrude".into());
                }
                let loose = faces.is_empty();
                let normal_local = mesh
                    .faces
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| faces.contains(&(*index as u32)))
                    .map(|(_, face)| mesh.face_normal(face))
                    .sum::<Vec3>()
                    .normalize_or_zero();
                let before = self.step("Extrude");
                let made = crate::commands::run_change(self, "mesh.extrude", &json!({ "distance": 0.0 })).and_then(|_| {
                    self.tidy_session();
                    self.scene.validate().map_err(|error| format!("failed: the extrusion would break the scene ({error})"))
                });
                if let Err(error) = made {
                    self.restore(before);
                    self.touched();
                    return Err(error);
                }
                let Some((mut items, pivot)) = self.modal_items() else {
                    self.restore(before);
                    self.touched();
                    return Err("not-allowed: nothing was extruded".into());
                };
                if let Items::Vertices { followers, .. } = &mut items {
                    followers.clear();
                }
                let mut modal = self.new_modal(Kind::Move, items, pivot, at, started, selected_before);
                if !loose && normal_local != Vec3::ZERO {
                    let world = self.scene.world_matrix(object).map_err(|error| error.to_string())?;
                    let direction = world.transform_vector3(normal_local).normalize_or(Vec3::Z);
                    let x = direction.any_orthonormal_vector();
                    modal.normal = Mat3::from_cols(x, direction.cross(x), direction);
                    modal.constraint = Some(Constraint { axis: 2, plane: false, orientation: Orientation::Normal });
                }
                modal.extrusion = Some(Extrusion { before: Box::new(before), loose, normal: normal_local });
                modal.fit_fields();
                self.modal = Some(modal);
                // The new geometry shows at once.
                self.touched();
                return Ok(());
            }
            "inset" if faces.is_empty() => return Err("not-allowed: select faces to inset".into()),
            "bevel" if !has_edges => return Err("not-allowed: select edges to bevel".into()),
            "inset" | "bevel" | "loopCut" => {}
            other => return Err(bad(format!("`kind` {other} is not an operation of edit mode"))),
        }
        let kind = match name {
            "inset" => Kind::Inset,
            "bevel" => Kind::Bevel,
            _ => Kind::LoopCut,
        };
        let pivot = if kind == Kind::LoopCut { Vec3::ZERO } else { self.pivot().ok_or("not-allowed: select something first")? };
        let items = Items::Rerun { object, original, selected: self.session.vertices.clone() };
        let mut modal = self.new_modal(kind, items, pivot, at, started, selected_before);
        if kind == Kind::LoopCut {
            modal.edge = self.edge_at(at.0 as f32, at.1 as f32);
        }
        self.modal_apply(&mut modal);
        self.modal = Some(modal);
        self.touched();
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
            Items::Rerun { object, original, selected } => {
                if let Ok(object) = self.scene.object_mut(*object) {
                    object.data = original.clone();
                }
                self.session.vertices = selected.clone();
                self.session.faces = None;
            }
        }
    }

    /// The command and parameters an operation's modal runs now (`None` for a transform, and
    /// for a loop cut before it has its loop).
    fn modal_operation(modal: &Modal) -> Option<(&'static str, Value)> {
        let values = modal.values;
        match modal.kind {
            Kind::Inset => Some(("mesh.inset", json!({ "thickness": values.amounts[0], "depth": values.amounts[1] }))),
            Kind::Bevel => Some(("mesh.bevel", json!({ "width": values.amounts[0], "segments": values.count }))),
            Kind::Slide => {
                let (a, b) = modal.edge?;
                Some(("mesh.loopCut", json!({ "edge": [a, b], "cuts": values.count, "factor": values.amounts[0] })))
            }
            Kind::LoopCut | Kind::Move | Kind::Rotate | Kind::Scale => None,
        }
    }

    /// The extrusion's command: along the normal when the move kept to it, else the move's offset
    /// in the object's own axes.
    fn extrusion_params(&self, modal: &Modal, extrusion: &Extrusion) -> Value {
        let Some(object) = modal.items.first_object() else { return json!({ "distance": 0.0 }) };
        let world = self.scene.world_matrix(object).unwrap_or(glam::Mat4::IDENTITY);
        let projector = self.projector();
        let offset = world.inverse().transform_vector3(modal.change(orthonormal(Mat3::from_mat4(world)), &projector).offset);
        let offset = if offset.is_finite() { offset } else { Vec3::ZERO };
        let along_normal = !extrusion.loose
            && modal.constraint.is_some_and(|constraint| constraint.orientation == Orientation::Normal && !constraint.plane && constraint.axis == 2);
        if along_normal {
            json!({ "distance": plain(offset.dot(extrusion.normal)) })
        } else {
            json!({ "distance": 0.0, "offset": offset.to_array().map(plain) })
        }
    }

    /// Works the value out again and applies it to what the transform moves, from where it was.
    fn modal_apply(&mut self, modal: &mut Modal) {
        let projector = self.projector();
        if !modal.kind.transform() {
            let scale = modal
                .items
                .first_object()
                .and_then(|id| self.scene.world_matrix(id).ok())
                .map_or(1.0, |world| world.to_scale_rotation_translation().0.abs().max_element());
            modal.values = match modal.fixed {
                Some((values, _)) => values,
                None => modal.solve_operation(&projector, scale),
            };
            self.modal_restore(&modal.items);
            if let Some((command, params)) = Self::modal_operation(modal) {
                let ran = crate::commands::run_change(self, command, &params).and_then(|_| {
                    self.tidy_session();
                    self.scene.validate().map_err(|error| error.to_string())
                });
                // An amount the operation cannot take (a bevel wider than its faces) shows the mesh
                // as it was.
                if ran.is_err() {
                    self.modal_restore(&modal.items);
                }
            }
            return;
        }
        let (mut values, ratio) = modal.solve(&projector);
        // What it moves as it began, which snapping measures from.
        self.modal_restore(&modal.items);
        modal.snapped = None;
        if modal.snapping_to_elements() {
            let view = (self.session.editor.view.clone(), self.session.width, self.session.height);
            if !modal.snap_index.as_ref().is_some_and(|index| index.fits(&view)) {
                modal.snap_index = Some(std::sync::Arc::new(self.snap_index(modal)));
            }
            let frame = modal.frame(modal.local);
            let raw = if modal.constraint.is_some() { frame * values.offset } else { values.offset };
            if let Some((target, source)) = modal.snap_index.as_ref().and_then(|index| index.find(&projector, modal.last, raw)) {
                let world = target - source;
                values.offset = match modal.constraint.map(|constraint| constraint.free()) {
                    None => world,
                    Some(free) => {
                        let local = frame.transpose() * world;
                        free.iter().fold(Vec3::ZERO, |offset, axis| with_component(offset, *axis, component(local, *axis)))
                    }
                };
                modal.snapped = Some(target);
            }
        }
        modal.values = values;
        modal.ratio = ratio;
        match &modal.items {
            Items::Objects(objects) => {
                for start in objects {
                    let (Ok(world), Ok(parent)) = (self.scene.world_matrix(start.id), self.scene.parent_frame(start.id)) else { continue };
                    let own = orthonormal(Mat3::from_mat4(world));
                    let origin = world.transform_point3(Vec3::ZERO);
                    let mut change = modal.change(own, &projector);
                    if modal.individual {
                        change.pivot = origin;
                    }
                    let moved = parent.inverse().transform_vector3(change.point(origin) - origin);
                    let parent_axes = orthonormal(Mat3::from_mat4(parent));
                    let (rotation, scale) = match modal.kind {
                        Kind::Move | Kind::Inset | Kind::Bevel | Kind::LoopCut | Kind::Slide => (start.rotation, start.scale),
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
                    for (index, vertex) in selected.iter().enumerate() {
                        let own = match modal.origins.get(index) {
                            Some(origin) if modal.individual => Change { pivot: *origin, ..change },
                            _ => change,
                        };
                        if let (Some(point), Some(was)) = (mesh.positions.get_mut(*vertex as usize), original.get(*vertex as usize)) {
                            *point = inverse.transform_point3(own.point(world.transform_point3(*was)));
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
            Items::Rerun { .. } => {}
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
        modal.base_amounts = modal.values.amounts;
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
            match modal.kind {
                // An inset's Ctrl sizes the depth instead, from where the pointer is when it changes.
                Kind::Inset => {
                    if step != modal.depth {
                        modal.depth = step;
                        modal.base_amounts = modal.values.amounts;
                        modal.start = at;
                        modal.virtual_at = at;
                    }
                }
                // A loop cut shows the loop across the edge under the pointer.
                Kind::LoopCut => modal.edge = self.edge_at(at.0 as f32, at.1 as f32),
                _ => modal.step = step,
            }
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
        let transform = modal.kind.transform();
        // An extrusion's move stays a move.
        let switches = transform && modal.extrusion.is_none();
        let taken = match lower.as_str() {
            "x" | "y" | "z" | "c" if !transform => false,
            "g" | "r" | "s" if !switches => false,
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
            // Shift+Tab turns snapping on or off, for this transform and the next.
            "tab" if shift => {
                modal.snap_on = !modal.snap_on;
                self.session.editor.snap = modal.snap_on;
                true
            }
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
        // A bevel's segments and a loop cut's cuts: one a notch, up the wheel more.
        if let Some(mut modal) = self.modal.take() {
            if matches!(modal.kind, Kind::Bevel | Kind::LoopCut) && delta.is_finite() {
                if delta != 0.0 {
                    let up = delta < 0.0;
                    let limit = if modal.kind == Kind::Bevel { 100 } else { 64 };
                    modal.count = if up { (modal.count + 1).min(limit) } else { modal.count.saturating_sub(1).max(1) };
                    // A typed number of cuts gives way to the wheel.
                    if modal.kind == Kind::LoopCut {
                        modal.numeric = Numeric::default();
                    }
                    self.modal_apply(&mut modal);
                }
                self.modal = Some(modal);
                return true;
            }
            self.modal = Some(modal);
        }
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
        // An extrusion: the scene before it, and the extrude command as the move left it.
        if let Some(extrusion) = modal.extrusion.take() {
            let params = self.extrusion_params(&modal, &extrusion);
            self.restore(*extrusion.before);
            self.touched();
            crate::commands::execute(self, "mesh.extrude", &params)?;
            return Ok(true);
        }
        match modal.kind {
            // The loop is chosen: it is cut, and slides with the pointer until a click.
            Kind::LoopCut => {
                match modal.edge {
                    Some(edge) => self.modal_start_slide(modal, edge),
                    None => {
                        self.modal_restore(&modal.items);
                        self.touched();
                    }
                }
                return Ok(true);
            }
            Kind::Inset | Kind::Bevel | Kind::Slide => {
                self.modal_restore(&modal.items);
                self.touched();
                if let Some((command, params)) = Self::modal_operation(&modal) {
                    crate::commands::execute(self, command, &params)?;
                }
                return Ok(true);
            }
            Kind::Move | Kind::Rotate | Kind::Scale => {}
        }
        if !modal.changes() {
            self.modal_restore(&modal.items);
            self.touched();
            return Ok(true);
        }
        // The scene before the transform, for the step (taken now, not when it began).
        self.modal_restore(&modal.items);
        let duplicated = modal.duplicate.is_some();
        let before = match modal.duplicate.take() {
            Some(before) => *before,
            None => self.step(modal.items.label(modal.kind)),
        };
        self.modal_apply(&mut modal);
        if self.session.editor.auto_key
            && let Items::Objects(objects) = &modal.items
        {
            self.modal_auto_keys(modal.kind, objects);
        }
        if let Err(error) = self.scene.validate() {
            self.restore(before);
            self.touched();
            return Err(format!("failed: the transform would break the scene ({error})"));
        }
        let content = self.content;
        self.committed(before)?;
        if self.content != content {
            let projector = self.projector();
            let view_axis = modal.fixed.and_then(|(_, axis)| axis).unwrap_or_else(|| into_screen(&projector, modal.pivot));
            let replay = Replay { kind: modal.kind, duplicate: duplicated, view_axis, normal: modal.normal, view_frame: modal.view_frame };
            crate::operation::record_transform(self, replay, transform_params(&modal), modal.selected_before.clone());
        }
        Ok(true)
    }

    /// Auto-keying: the moved objects get keys at the current frame, in the transform's step, on
    /// what the transform is about (a move's location, a rotation's, a scale's) and on any other of
    /// the three it changed (a rotation about the cursor moves the objects too). A property a
    /// driver sets takes no key, and is passed over.
    fn modal_auto_keys(&mut self, kind: Kind, objects: &[ObjectStart]) {
        let frame = self.scene.animation.timeline.frame as f32;
        let own = match kind {
            Kind::Rotate => Property::Rotation,
            Kind::Scale => Property::Scale,
            _ => Property::Location,
        };
        for start in objects {
            let Ok(object) = self.scene.object(start.id) else { continue };
            let changed = |from: Vec3, to: Vec3| (from - to).abs().max_element() > 1e-6;
            let mut wanted = vec![own];
            for (property, from, to) in [
                (Property::Location, start.location, object.location),
                (Property::Rotation, start.rotation, object.rotation),
                (Property::Scale, start.scale, object.scale),
            ] {
                if property != own && changed(from, to) {
                    wanted.push(property);
                }
            }
            for property in wanted {
                self.scene.insert_key(start.id, property, frame).ok();
            }
        }
    }

    /// A loop cut's loop is chosen: it is cut in the middle, and the modal slides it.
    fn modal_start_slide(&mut self, mut modal: Modal, edge: (u32, u32)) {
        let edge = (edge.0.min(edge.1), edge.0.max(edge.1));
        let projector = self.projector();
        let ends = match &modal.items {
            Items::Rerun { object, original: ObjectData::Mesh(mesh), .. } => self.scene.world_matrix(*object).ok().and_then(|world| {
                let (a, b) = (mesh.positions.get(edge.0 as usize)?, mesh.positions.get(edge.1 as usize)?);
                Some((screen_of(&projector, world.transform_point3(*a)), screen_of(&projector, world.transform_point3(*b))))
            }),
            _ => None,
        };
        modal.edge_screen = ends.map_or((0.0, 0.0), |(a, b)| (b.0 - a.0, b.1 - a.1));
        modal.count = modal.values.count;
        modal.kind = Kind::Slide;
        modal.edge = Some(edge);
        modal.numeric = Numeric::default();
        modal.base_amounts = [0.5, 0.0];
        modal.start = modal.last;
        modal.virtual_at = modal.last;
        modal.travelled = false;
        self.modal_apply(&mut modal);
        self.modal = Some(modal);
        self.touched();
    }

    /// Ends the transform, putting everything back; Shift+D's copies stay, where they were made,
    /// as their own step. Answers whether there was one.
    pub(crate) fn modal_cancel(&mut self) -> Result<bool, String> {
        let Some(mut modal) = self.modal.take() else { return Ok(false) };
        // An extrusion's geometry stays where it was made, as Blender leaves it (one Ctrl+Z more
        // takes it back).
        if let Some(extrusion) = modal.extrusion.take() {
            let params = if extrusion.loose { json!({ "distance": 0.0, "offset": [0.0, 0.0, 0.0] }) } else { json!({ "distance": 0.0 }) };
            self.restore(*extrusion.before);
            self.touched();
            crate::commands::execute(self, "mesh.extrude", &params)?;
            return Ok(true);
        }
        // A loop already cut stays, in the middle.
        if modal.kind == Kind::Slide {
            modal.fixed = None;
            modal.values.amounts[0] = 0.5;
            self.modal_restore(&modal.items);
            self.touched();
            if let Some((command, params)) = Self::modal_operation(&modal) {
                crate::commands::execute(self, command, &params)?;
            }
            return Ok(true);
        }
        self.modal_restore(&modal.items);
        match modal.duplicate.take() {
            Some(before) => self.committed(*before)?,
            None => self.touched(),
        }
        Ok(true)
    }

    /// Whether the modal under way is one of edit mode's operations (or an extrusion's move):
    /// a click confirms it wherever it is.
    pub(crate) fn modal_operates(&self) -> bool {
        self.modal.as_ref().is_some_and(|modal| !modal.kind.transform() || modal.extrusion.is_some())
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
        if matches!(modal.kind, Kind::Rotate | Kind::Scale | Kind::Inset | Kind::Bevel) {
            overlays.push(ToolOverlay::Line { ax: px, ay: py, bx: modal.last.0, by: modal.last.1, color: DASH, dashed: true });
        }
        if modal.kind == Kind::LoopCut
            && let Some(edge) = modal.edge
        {
            overlays.extend(self.loop_lines(edge, modal.values.count));
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
            Items::Rerun { object, .. } => self.session.mode == Mode::Edit(*object),
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

/// The constraint an axis key (`axis`, with `plane` the plane across it) leaves after `current`:
/// first along the person's orientation `preferred`; pressed again, the other of global and local
/// (local after global, global after any other); a third time, free. A constraint along another
/// orientation (an extrusion's normal) pressed goes to `preferred`.
fn next_constraint(current: Option<Constraint>, preferred: Orientation, axis: usize, plane: bool) -> Option<Constraint> {
    let second = if preferred == Orientation::Global { Orientation::Local } else { Orientation::Global };
    match current {
        Some(current) if current.axis == axis && current.plane == plane => {
            if current.orientation == preferred {
                Some(Constraint { orientation: second, ..current })
            } else if current.orientation == second {
                None
            } else {
                Some(Constraint { orientation: preferred, ..current })
            }
        }
        _ => Some(Constraint { axis, plane, orientation: preferred }),
    }
}

/// The axis a transform's parameters name (`none`, or for a rotation `view`: free).
fn axis_index(params: &Value) -> Result<Option<usize>, String> {
    match params::string(params, "axis")? {
        None | Some("none") | Some("view") => Ok(None),
        Some("x") => Ok(Some(0)),
        Some("y") => Ok(Some(1)),
        Some("z") => Ok(Some(2)),
        Some(_) => Err(bad("`axis` must be none, x, y or z")),
    }
}

/// A confirmed transform's values as Adjust Last Operation shows them: the move (in the axes of
/// the orientation when an axis is kept to), the angle in degrees, the scale; the axis, the plane
/// and the orientation.
fn transform_params(modal: &Modal) -> Value {
    let axis = modal.constraint.and_then(|constraint| AXIS_NAMES.get(constraint.axis)).map(|name| name.to_ascii_lowercase());
    let free = if modal.kind == Kind::Rotate { "view" } else { "none" };
    let mut params = json!({
        "axis": axis.unwrap_or_else(|| free.into()),
        "orientation": modal.constraint.map_or("global", |constraint| constraint.orientation.name()),
    });
    if let Some(fields) = params.as_object_mut() {
        let round = |value: f32| plain((value * 1e5).round() / 1e5);
        match modal.kind {
            Kind::Move => {
                fields.insert("move".into(), json!(modal.values.offset.to_array().map(round)));
            }
            Kind::Rotate => {
                fields.insert("angle".into(), json!(round(modal.values.angle.to_degrees())));
            }
            _ => {
                fields.insert("scale".into(), json!(modal.values.factors.to_array().map(round)));
            }
        }
        if modal.kind != Kind::Rotate {
            fields.insert("plane".into(), json!(modal.constraint.is_some_and(|constraint| constraint.plane)));
        }
    }
    params
}

/// The parameters of a confirmed transform's replay (Adjust Last Operation's fields).
pub(crate) fn replay_schema(kind: Kind) -> Value {
    let orientation = ("orientation", s::choice("Whose axes the axis is (global)", &["global", "local", "normal", "view"]));
    match kind {
        Kind::Rotate => s::of(vec![
            ("angle", s::number("Degrees, counter-clockwise looking down the axis (0)")),
            ("axis", s::choice("The axis to turn about; view: the line of sight (view)", &["view", "x", "y", "z"])),
            orientation,
        ]),
        Kind::Scale => s::of(vec![
            ("scale", s::vec3("Scale along X, Y and Z (1)")),
            ("axis", s::choice("The axis it keeps to (none)", &["none", "x", "y", "z"])),
            ("plane", s::boolean("Keep to the plane across the axis instead (false)")),
            orientation,
        ]),
        _ => s::of(vec![
            ("move", s::vec3("How far along X, Y and Z, in metres; along the orientation's axes when it keeps to an axis")),
            ("axis", s::choice("The axis it keeps to (none)", &["none", "x", "y", "z"])),
            ("plane", s::boolean("Keep to the plane across the axis instead (false)")),
            orientation,
        ]),
    }
}

impl SceneDocument {
    /// Runs a confirmed transform again with `params` (its replay's fields), from the selection as
    /// it is now, as one undo step (Adjust Last Operation).
    pub(crate) fn modal_replay(&mut self, replay: &Replay, params: &Value) -> Result<(), String> {
        let axis = axis_index(params)?;
        let orientation = match params::string(params, "orientation")? {
            None => Orientation::Global,
            Some(name) => Orientation::parse(name).ok_or_else(|| bad("`orientation` must be global, local, normal or view"))?,
        };
        let plane = replay.kind != Kind::Rotate && params::boolean(params, "plane")?.unwrap_or(false);
        let constraint = axis.map(|axis| Constraint { axis, plane, orientation });
        let mut values = Values::default();
        match replay.kind {
            Kind::Move => {
                let offset = params::vec3(params, "move")?.unwrap_or(Vec3::ZERO);
                values.offset = match constraint {
                    Some(constraint) => constraint.free().iter().fold(Vec3::ZERO, |kept, axis| with_component(kept, *axis, component(offset, *axis))),
                    None => offset,
                };
            }
            Kind::Rotate => values.angle = params::number(params, "angle")?.unwrap_or(0.0).to_radians(),
            _ => {
                let factors = params::vec3(params, "scale")?.unwrap_or(Vec3::ONE);
                values.factors = match constraint {
                    Some(constraint) => constraint.free().iter().fold(Vec3::ONE, |kept, axis| with_component(kept, *axis, component(factors, *axis))),
                    None => factors,
                };
            }
        }
        if !(values.offset.is_finite() && values.angle.is_finite() && values.factors.is_finite()) {
            return Err(bad("the values must be numbers"));
        }
        self.modal_begin(replay.kind, None, Started::Key, constraint, replay.duplicate)?;
        let Some(mut modal) = self.modal.take() else { return Err("not-allowed: nothing to transform".into()) };
        modal.normal = replay.normal;
        modal.view_frame = replay.view_frame;
        modal.fixed = Some((values, Some(replay.view_axis)));
        self.modal_apply(&mut modal);
        self.modal = Some(modal);
        self.modal_confirm()?;
        Ok(())
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
                (
                    "kind",
                    s::required(s::choice(
                        "What the modal does: a transform, or (in a mesh's edit mode) an extrusion that moves, an inset or bevel the pointer sizes, a loop cut",
                        &["move", "rotate", "scale", "extrude", "inset", "bevel", "loopCut"],
                    )),
                ),
                (
                    "at",
                    json!({ "type": "array", "items": { "type": "number" }, "description": "Where the pointer is, [x, y] in viewport pixels; its last place when left out" }),
                ),
                ("axis", s::choice("An axis to keep to from the start (a handle, a normal); free when left out", &["x", "y", "z"])),
                ("plane", s::boolean("Keep to the plane across `axis` instead (false)")),
                ("orientation", s::choice("Whose axes `axis` is (the person's transform orientation)", &["global", "local", "normal", "view"])),
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
                (
                    "step",
                    s::boolean(
                        "Ctrl or Cmd held: snapping is inverted (with increments the value goes in steps of 1 m, 5°, 0.1; with precision 0.1 m, 1°, 0.01)",
                    ),
                ),
            ]),
        ),
        command(
            "modal.key",
            "Transform Key",
            s::of(vec![
                ("key", s::required(s::string("x, y, z, c, g, r, s, a digit, ., -, /, Backspace or Tab"))),
                ("shift", s::boolean("Shift held: X, Y and Z lock the plane across the axis; Shift+Tab turns snapping on or off")),
                ("ctrl", s::boolean("Ctrl held: Tab goes back a field")),
            ]),
        ),
        command(
            "modal.wheel",
            "Transform Wheel",
            s::of(vec![(
                "delta",
                s::required(s::number("The wheel's turn in pixels, down positive (a bevel's segments, a loop cut's cuts: one less)")),
            )]),
        ),
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
        // A loop cut's first click goes on to its slide: the answer is the modal still under way.
        "modal.confirm" => doc.modal_confirm().map(|ended| {
            let mut answer = doc.modal_answer();
            if let Some(fields) = answer.as_object_mut() {
                fields.insert("confirmed".into(), json!(ended));
            }
            answer
        }),
        "modal.cancel" => doc.modal_cancel().map(|ended| json!({ "cancelled": ended, "active": false })),
        "modal.state" => Ok(doc.modal_answer()),
        other => Err(format!("unknown-command: {other}")),
    })
}

fn begin(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let name = params::string(params, "kind")?.ok_or_else(|| bad("`kind` is required"))?;
    let at = point(params, "at")?;
    if matches!(name, "extrude" | "inset" | "bevel" | "loopCut") {
        let started = if params::boolean(params, "press")?.unwrap_or(false) { Started::Press { select: false } } else { Started::Key };
        doc.modal_begin_operation(name, at, started)?;
        return Ok(doc.modal_answer());
    }
    let kind = Kind::parse(name).ok_or_else(|| bad("`kind` must be move, rotate, scale, extrude, inset, bevel or loopCut"))?;
    let axis = match params::string(params, "axis")? {
        None => None,
        Some("x") => Some(0),
        Some("y") => Some(1),
        Some("z") => Some(2),
        Some(_) => return Err(bad("`axis` must be x, y or z")),
    };
    let orientation = match params::string(params, "orientation")? {
        None => doc.session.editor.orientation,
        Some(name) => Orientation::parse(name).ok_or_else(|| bad("`orientation` must be global, local, normal or view"))?,
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
        assert_eq!(numeric.value(1, true, 1.0), 1.0, "the untyped field being typed is its own (one)");
        assert_eq!(numeric.value(2, true, 1.0), -2.5, "a scale's other untyped field takes the first");
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
                Orientation::Normal | Orientation::View => Some(Constraint { orientation: Orientation::Global, ..c }),
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
    fn axis_keys_start_with_the_persons_orientation() {
        let view = next_constraint(None, Orientation::View, 0, false);
        assert_eq!(view.map(|c| c.orientation), Some(Orientation::View));
        let again = next_constraint(view, Orientation::View, 0, false);
        assert_eq!(again.map(|c| c.orientation), Some(Orientation::Global), "again: global");
        assert_eq!(next_constraint(again, Orientation::View, 0, false), None, "a third time: free");
        let global = next_constraint(None, Orientation::Global, 2, false);
        let local = next_constraint(global, Orientation::Global, 2, false);
        assert_eq!(local.map(|c| c.orientation), Some(Orientation::Local));
        assert_eq!(next_constraint(local, Orientation::Global, 2, false), None);
        let normal = Some(Constraint { axis: 2, plane: false, orientation: Orientation::Normal });
        assert_eq!(
            next_constraint(normal, Orientation::Global, 2, false).map(|c| c.orientation),
            Some(Orientation::Global),
            "an extrusion's normal pressed: global"
        );
        assert_eq!(view.map(|c| c.text()), Some(" along view X".to_owned()));
    }

    #[test]
    fn a_field_shows_what_is_typed_and_its_value() {
        let field = Field { text: "2".into(), edited: true, negate: true, inverse: false };
        assert_eq!(field.shown("-2.0000 m"), "[-(2|)] = -2.0000 m");
        assert_eq!(field.value(), Some(-2.0));
        assert_eq!(Field::default().value(), None);
    }
}
