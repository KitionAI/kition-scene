//! The 3D cursor and the transform's settings: where the cursor is, the point transforms turn and
//! scale about (the pivot point), whose axes the axis keys and the gizmo follow (the transform
//! orientation), and snapping (on or off, and what to). All of it is the person's view state
//! (`view.state`): never in the file, never an unsaved change, never an undo step.
//!
//! Session commands: `cursor.set` (a place, or the surface under a screen point), `cursor.reset`
//! (the world origin), `cursor.moveToSelected`, `cursor.moveToActive`, and `transform.set` (the
//! pivot point, the orientation, snapping). Moving the selection to the cursor is a change of the
//! scene, and runs as the object or mesh command that moves it (one undo step).

use std::collections::BTreeSet;

use glam::{Mat3, Vec3};
use kition_kit::KitCommand;
use kition_scene_model::Id;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::modal::Orientation;
use crate::params::{self, bad, schema as s};
use crate::{Mode, SceneDocument};

/// The point a rotation or a scale turns about.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PivotPoint {
    /// The middle of what is selected (the mean of the objects' origins or of the vertices).
    #[default]
    Median,
    /// The middle of the box round what is selected.
    BoundingBox,
    /// Each object about its own origin; in edit mode each connected part of the selection about
    /// its own middle.
    Individual,
    /// The 3D cursor.
    Cursor,
    /// The active object's origin (in edit mode, as Median: there is no active element).
    Active,
}

impl PivotPoint {
    pub fn name(self) -> &'static str {
        match self {
            PivotPoint::Median => "median",
            PivotPoint::BoundingBox => "boundingBox",
            PivotPoint::Individual => "individual",
            PivotPoint::Cursor => "cursor",
            PivotPoint::Active => "active",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        [PivotPoint::Median, PivotPoint::BoundingBox, PivotPoint::Individual, PivotPoint::Cursor, PivotPoint::Active]
            .into_iter()
            .find(|pivot| pivot.name() == name)
    }
}

/// What a snapping transform snaps to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SnapTarget {
    /// Steps (1 m, 5°, 0.1; with Shift a tenth).
    #[default]
    Increment,
    /// The nearest vertex under the pointer.
    Vertex,
    /// The nearest point of an edge under the pointer.
    Edge,
    /// The surface under the pointer.
    Face,
}

impl SnapTarget {
    pub fn name(self) -> &'static str {
        match self {
            SnapTarget::Increment => "increment",
            SnapTarget::Vertex => "vertex",
            SnapTarget::Edge => "edge",
            SnapTarget::Face => "face",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        [SnapTarget::Increment, SnapTarget::Vertex, SnapTarget::Edge, SnapTarget::Face].into_iter().find(|target| target.name() == name)
    }
}

/// The mean of `points`.
fn mean(points: &[Vec3]) -> Option<Vec3> {
    (!points.is_empty()).then(|| points.iter().copied().sum::<Vec3>() / points.len() as f32)
}

/// The middle of the box round `points`.
fn box_middle(points: &[Vec3]) -> Option<Vec3> {
    let first = *points.first()?;
    let (low, high) = points.iter().fold((first, first), |(low, high), point| (low.min(*point), high.max(*point)));
    Some((low + high) / 2.0)
}

/// A frame whose Z is `normal`, its X level where it can be.
pub(crate) fn normal_frame(normal: Vec3) -> Mat3 {
    let z = normal.normalize_or(Vec3::Z);
    let up = if z.z.abs() < 0.999 { Vec3::Z } else { Vec3::Y };
    let x = up.cross(z).normalize_or(Vec3::X);
    Mat3::from_cols(x, z.cross(x), z)
}

/// For each of `selected` (vertices of `mesh`), the middle of the connected part of the selection
/// it belongs to (joined by edges whose ends are both selected), in the mesh's space.
pub(crate) fn island_middles(mesh: &kition_scene_model::Mesh, selected: &[u32]) -> Vec<Vec3> {
    let set: BTreeSet<u32> = selected.iter().copied().collect();
    let index: std::collections::HashMap<u32, usize> = selected.iter().enumerate().map(|(at, vertex)| (*vertex, at)).collect();
    let mut parent: Vec<usize> = (0..selected.len()).collect();
    fn root(parent: &mut [usize], mut at: usize) -> usize {
        while let Some(&up) = parent.get(at) {
            if up == at {
                break;
            }
            let grand = parent.get(up).copied().unwrap_or(up);
            if let Some(slot) = parent.get_mut(at) {
                *slot = grand;
            }
            at = up;
        }
        at
    }
    for [a, b] in mesh.edges() {
        if !(set.contains(&a) && set.contains(&b)) {
            continue;
        }
        let (Some(&ia), Some(&ib)) = (index.get(&a), index.get(&b)) else { continue };
        let (ra, rb) = (root(&mut parent, ia), root(&mut parent, ib));
        if ra != rb
            && let Some(slot) = parent.get_mut(ra)
        {
            *slot = rb;
        }
    }
    let mut sums: std::collections::HashMap<usize, (Vec3, f32)> = std::collections::HashMap::new();
    let roots: Vec<usize> = (0..selected.len()).map(|at| root(&mut parent, at)).collect();
    for (at, vertex) in selected.iter().enumerate() {
        let point = mesh.positions.get(*vertex as usize).copied().unwrap_or(Vec3::ZERO);
        let entry = sums.entry(roots.get(at).copied().unwrap_or(at)).or_insert((Vec3::ZERO, 0.0));
        entry.0 += point;
        entry.1 += 1.0;
    }
    roots.iter().map(|group| sums.get(group).map_or(Vec3::ZERO, |(sum, count)| *sum / count.max(1.0))).collect()
}

impl SceneDocument {
    /// The 3D cursor's place.
    pub(crate) fn cursor(&self) -> Vec3 {
        self.session.editor.cursor
    }

    /// The world points the pivot is found from in object or a mesh's edit mode (the visible
    /// selected objects' origins, or the selected vertices), and the active object's origin.
    fn pivot_points(&self) -> (Vec<Vec3>, Option<Vec3>) {
        match self.session.mode {
            Mode::Object => {
                let origin = |id: &Id| self.scene.world_matrix(*id).ok().map(|world| world.transform_point3(Vec3::ZERO));
                let points = self.session.editor.selected.iter().filter_map(origin).collect();
                let active = self.session.editor.active.filter(|id| self.session.editor.selected.contains(id)).as_ref().and_then(origin);
                (points, active)
            }
            Mode::Edit(id) => {
                let (Ok(world), Some(mesh)) = (self.scene.world_matrix(id), self.scene.object(id).ok().and_then(|object| object.mesh())) else {
                    return (Vec::new(), None);
                };
                let points = self
                    .session
                    .vertices
                    .iter()
                    .filter_map(|vertex| mesh.positions.get(*vertex as usize))
                    .map(|point| world.transform_point3(*point))
                    .collect();
                (points, None)
            }
        }
    }

    /// The point a transform of the selection turns and scales about, as the pivot point setting
    /// says (object or a mesh's edit mode; `median` for the other modes' own pivots), or `None`
    /// when nothing is selected.
    pub(crate) fn transform_pivot(&self, median: Option<Vec3>) -> Option<Vec3> {
        let pivot = self.session.editor.pivot;
        let in_own_mode = self.session.rig.is_some() || self.session.shape.is_some();
        if in_own_mode {
            let median = median?;
            return Some(if pivot == PivotPoint::Cursor { self.cursor() } else { median });
        }
        let (points, active) = self.pivot_points();
        let middle = mean(&points)?;
        Some(match pivot {
            PivotPoint::Median | PivotPoint::Individual => middle,
            PivotPoint::BoundingBox => box_middle(&points).unwrap_or(middle),
            PivotPoint::Cursor => self.cursor(),
            PivotPoint::Active => active.unwrap_or(middle),
        })
    }

    /// The edit mode's normal: the selected faces' mean normal, else the selected vertices', in
    /// world space (`None` outside a mesh's edit mode, or with nothing selected).
    pub(crate) fn selection_normal(&self) -> Option<Vec3> {
        let Mode::Edit(id) = self.session.mode else { return None };
        let world = self.scene.world_matrix(id).ok()?;
        let mesh = self.scene.object(id).ok()?.mesh()?;
        let faces = crate::commands::faces_selected(&self.session, mesh);
        let local = if faces.is_empty() {
            let normals = mesh.vertex_normals();
            self.session.vertices.iter().filter_map(|vertex| normals.get(*vertex as usize)).copied().sum::<Vec3>()
        } else {
            faces.iter().filter_map(|face| mesh.faces.get(*face as usize)).map(|face| mesh.face_normal(face)).sum::<Vec3>()
        };
        let normal = world.inverse().transpose().transform_vector3(local);
        (normal.length_squared() > 1e-12 && normal.is_finite()).then(|| normal.normalize())
    }

    /// The axes of `orientation` for the selection now, in world space: the world's; the active
    /// object's own (in edit mode the edited object's); the selection's normal as Z (outside edit
    /// mode, the object's own); the view's (X right, Y up, Z towards the eye).
    pub(crate) fn orientation_frame(&self, orientation: Orientation) -> Mat3 {
        let own = || {
            let object = match self.session.mode {
                Mode::Edit(id) => Some(id),
                Mode::Object => self.session.editor.active.filter(|id| self.session.editor.selected.contains(id)).or(self
                    .session
                    .editor
                    .selected
                    .first()
                    .copied()),
            };
            object.and_then(|id| self.scene.world_matrix(id).ok()).map_or(Mat3::IDENTITY, |world| crate::modal::orthonormal(Mat3::from_mat4(world)))
        };
        match orientation {
            Orientation::Global => Mat3::IDENTITY,
            Orientation::Local => own(),
            Orientation::Normal => self.selection_normal().map_or_else(own, normal_frame),
            Orientation::View => crate::modal::orthonormal(Mat3::from_mat4(self.projector().view.inverse())),
        }
    }

    /// The transform's settings and the gizmo's place and axes, as `inspect` shows them.
    pub(crate) fn transform_json(&self) -> Value {
        let editor = &self.session.editor;
        let transforms = self.session.sculpt.is_none()
            && self.session.paint.is_none()
            && self.session.particle_edit.is_none()
            && !matches!(self.session.shape, Some(crate::commands_curve::ShapeSession::Text(_)));
        let median = match (&self.session.rig, &self.session.shape) {
            (Some(rig), _) => crate::commands_rig::pivot(self, rig),
            (None, Some(crate::commands_curve::ShapeSession::Curve(edit))) => crate::commands_curve::pivot(self, edit),
            (None, Some(crate::commands_curve::ShapeSession::Lattice(edit))) => crate::commands_lattice::pivot(self, edit),
            _ => None,
        };
        let gizmo = transforms.then(|| self.transform_pivot(median)).flatten().map(|centre| {
            let frame = self.orientation_frame(editor.orientation);
            json!({
                "centre": centre.to_array(),
                "axes": [frame.x_axis.to_array(), frame.y_axis.to_array(), frame.z_axis.to_array()],
            })
        });
        json!({
            "cursor": editor.cursor.to_array(),
            "pivot": editor.pivot.name(),
            "orientation": editor.orientation.name(),
            "snap": editor.snap,
            "snapTarget": editor.snap_target.name(),
            "gizmo": gizmo,
            // The selection's middle (world), and in edit mode the cursor in the edited object's
            // own space, for moving the selection to the cursor.
            "median": mean(&self.pivot_points().0).map(|point| point.to_array()),
            "cursorLocal": match self.session.mode {
                Mode::Edit(id) => self.scene.world_matrix(id).ok().map(|world| world.inverse().transform_point3(editor.cursor)).filter(|point| point.is_finite()).map(|point| point.to_array()),
                Mode::Object => None,
            },
        })
    }

    /// Places the cursor on the surface under the viewport's pixel `at`, or where nothing is
    /// there, on the plane through the cursor facing the view.
    fn place_cursor(&mut self, at: (f32, f32)) -> Result<Vec3, String> {
        let projector = self.projector();
        let placed = match self.surface_at(at.0, at.1, &|_| false) {
            Some(hit) => hit.point,
            None => {
                let cursor = self.cursor();
                let (origin, direction) = projector.ray(at.0, at.1);
                let facing = crate::modal::into_screen(&projector, cursor);
                let across = direction.dot(facing);
                let distance = if across.abs() > 1e-6 { (cursor - origin).dot(facing) / across } else { f32::NAN };
                let point = origin + direction * distance;
                if distance.is_finite() && distance > 0.0 && point.is_finite() {
                    point
                } else {
                    let depth = projector.to_screen(projector.view_projection() * cursor.extend(1.0)).z;
                    projector.unproject(at.0, at.1, depth)
                }
            }
        };
        if !placed.is_finite() {
            return Err(bad("the point under the pointer is not in the view"));
        }
        self.set_cursor(placed);
        Ok(placed)
    }

    fn set_cursor(&mut self, at: Vec3) {
        if self.session.editor.cursor != at {
            self.session.editor.cursor = at;
            self.touched();
        }
    }
}

fn command(id: &str, title: &str, menu: &[&str], schema: Value) -> KitCommand {
    KitCommand {
        id: id.into(),
        title: title.into(),
        menu: (!menu.is_empty()).then(|| menu.iter().map(|part| (*part).to_string()).collect()),
        shortcut: None,
        params: None,
        schema: Some(schema),
        mutates: false,
        enabled: true,
        ..Default::default()
    }
}

/// The cursor's and the transform settings' commands: session commands (no undo step).
pub fn catalogue(_doc: &SceneDocument) -> Vec<KitCommand> {
    vec![
        command(
            "cursor.set",
            "Set 3D Cursor",
            &[],
            s::of(vec![
                ("location", s::vec3("Where to put the cursor, in metres")),
                (
                    "at",
                    json!({ "type": "array", "items": { "type": "number" }, "description": "A viewport pixel [x, y]: the cursor goes onto the surface there, or where there is none, onto the plane through the cursor facing the view" }),
                ),
            ]),
        ),
        command("cursor.reset", "Cursor to World Origin", &[], s::of(vec![])),
        command("cursor.moveToSelected", "Cursor to Selected", &[], s::of(vec![])),
        command("cursor.moveToActive", "Cursor to Active", &[], s::of(vec![])),
        command(
            "transform.set",
            "Transform Settings",
            &[],
            s::of(vec![
                (
                    "pivot",
                    s::choice(
                        "The point rotations and scales turn about: the selection's median (the default), its bounding box's middle, each object's own origin, the 3D cursor, the active object",
                        &["median", "boundingBox", "individual", "cursor", "active"],
                    ),
                ),
                ("orientation", s::choice("Whose axes the axis keys and the gizmo follow (global)", &["global", "local", "normal", "view"])),
                ("snap", s::boolean("Transforms snap (Ctrl inverts it while held); false")),
                ("snapTarget", s::choice("What they snap to (increment)", &["increment", "vertex", "edge", "face"])),
            ]),
        ),
    ]
}

/// The ones that cannot run now.
pub fn disabled(doc: &SceneDocument) -> Vec<String> {
    let mut ids = Vec::new();
    if doc.transform_pivot(None).is_none() || doc.session.rig.is_some() || doc.session.shape.is_some() {
        ids.push("cursor.moveToSelected".to_owned());
    }
    if doc.session.mode != Mode::Object || doc.session.editor.active.is_none() {
        ids.push("cursor.moveToActive".to_owned());
    }
    ids
}

/// Runs one of the cursor's or the transform settings' commands; `None` for any other.
pub fn execute(doc: &mut SceneDocument, command: &str, params: &Value) -> Option<Result<Value, String>> {
    Some(match command {
        "cursor.set" => set(doc, params),
        "cursor.reset" => {
            doc.set_cursor(Vec3::ZERO);
            Ok(json!({ "cursor": doc.cursor().to_array() }))
        }
        "cursor.moveToSelected" => match doc.pivot_points().0 {
            points if points.is_empty() => Err("not-allowed: select an object first".into()),
            points => {
                let middle = mean(&points).unwrap_or(Vec3::ZERO);
                doc.set_cursor(middle);
                Ok(json!({ "cursor": middle.to_array() }))
            }
        },
        "cursor.moveToActive" => {
            let active = doc.session.editor.active.and_then(|id| doc.scene.world_matrix(id).ok()).map(|world| world.transform_point3(Vec3::ZERO));
            match active {
                Some(point) if doc.session.mode == Mode::Object => {
                    doc.set_cursor(point);
                    Ok(json!({ "cursor": point.to_array() }))
                }
                _ => Err("not-allowed: there is no active object".into()),
            }
        }
        "transform.set" => settings(doc, params),
        _ => return None,
    })
}

fn set(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let location = params::vec3(params, "location")?;
    let at = match params.get("at").filter(|value| !value.is_null()) {
        None => None,
        Some(value) => match value.as_array().map(|items| items.iter().map(Value::as_f64).collect::<Vec<_>>()).as_deref() {
            Some([Some(x), Some(y)]) if x.is_finite() && y.is_finite() && x.abs() <= 1e7 && y.abs() <= 1e7 => Some((*x as f32, *y as f32)),
            _ => return Err(bad("`at` must be [x, y] in viewport pixels")),
        },
    };
    match (location, at) {
        (Some(_), Some(_)) => Err(bad("give `location` or `at`, not both")),
        (Some(location), None) => {
            if !location.is_finite() || location.abs().max_element() > 1e7 {
                return Err(bad("`location` must be finite and within 10,000 km"));
            }
            doc.set_cursor(location);
            Ok(json!({ "cursor": location.to_array() }))
        }
        (None, Some(at)) => {
            let placed = doc.place_cursor(at)?;
            Ok(json!({ "cursor": placed.to_array() }))
        }
        (None, None) => Err(bad("`location` or `at` is required")),
    }
}

fn settings(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let pivot = params::string(params, "pivot")?
        .map(|name| PivotPoint::parse(name).ok_or_else(|| bad("`pivot` must be median, boundingBox, individual, cursor or active")))
        .transpose()?;
    let orientation = params::string(params, "orientation")?
        .map(|name| Orientation::parse(name).ok_or_else(|| bad("`orientation` must be global, local, normal or view")))
        .transpose()?;
    let snap = params::boolean(params, "snap")?;
    let target = params::string(params, "snapTarget")?
        .map(|name| SnapTarget::parse(name).ok_or_else(|| bad("`snapTarget` must be increment, vertex, edge or face")))
        .transpose()?;
    let before = doc.session.editor.clone();
    let editor = &mut doc.session.editor;
    if let Some(pivot) = pivot {
        editor.pivot = pivot;
    }
    if let Some(orientation) = orientation {
        editor.orientation = orientation;
    }
    if let Some(snap) = snap {
        editor.snap = snap;
    }
    if let Some(target) = target {
        editor.snap_target = target;
    }
    if doc.session.editor != before {
        doc.touched();
    }
    let editor = &doc.session.editor;
    Ok(
        json!({ "pivot": editor.pivot.name(), "orientation": editor.orientation.name(), "snap": editor.snap, "snapTarget": editor.snap_target.name() }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn island_middles_are_each_connected_part_s() {
        let mesh = kition_scene_model::Mesh {
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::new(5.0, 0.0, 0.0), Vec3::new(7.0, 0.0, 0.0)],
            faces: Vec::new(),
            loose_edges: vec![[0, 1], [2, 3]],
            ..Default::default()
        };
        let middles = island_middles(&mesh, &[0, 1, 2, 3]);
        assert_eq!(middles, vec![Vec3::new(0.5, 0.0, 0.0), Vec3::new(0.5, 0.0, 0.0), Vec3::new(6.0, 0.0, 0.0), Vec3::new(6.0, 0.0, 0.0)]);
    }

    #[test]
    fn a_normal_frame_keeps_its_x_level() {
        let frame = normal_frame(Vec3::new(1.0, 0.0, 1.0));
        assert!(frame.x_axis.z.abs() < 1e-6);
        assert!((frame.z_axis - Vec3::new(1.0, 0.0, 1.0).normalize()).length() < 1e-6);
        assert!((frame.x_axis.cross(frame.y_axis) - frame.z_axis).length() < 1e-5);
    }
}
