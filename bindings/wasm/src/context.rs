//! The editor state a call reads and leaves behind, as an engine API that is no edit (kition-sdk's
//! explicit call context, task card C4-scene): what the editor shows and selects (the object
//! selection and the active object, the mode and the edited mesh's selected vertices, the element
//! kind clicks select, the other modes' sessions, proportional editing, the current frame, the
//! viewport and its shading). A host saves it before an agent's run and puts it back after
//! (`editor_state`, `restore_editor_state`); `set_context` sets the selection from a step's `on`
//! and the current frame from its `at.time`. None of it moves the revision, the dirty state or the
//! undo history.
//!
//! An agent's command that names its objects, its frame or its mesh `elements` leaves all of it as
//! it was, also when it fails (`SceneDocument::execute_as`); a command whose work is moving the
//! editor (`select.*`, `mode.*`, `frame.*`, `view.*`) keeps its effect. `elements` (an edit-mode
//! command's `{object, vertices?, faces?}`) runs the command in that mesh's edit mode on those
//! elements and puts the mode back, so an agent extrudes a face in one step.

use std::collections::BTreeSet;

use kition_kit::CallContext;
use kition_scene_model::{Id, Scene};
use kition_scene_raster::{Shading, ViewState};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{Mode, SceneDocument, SelectMode, commands_curve, commands_particles, commands_rig, commands_rig2, commands_sculpt, params};

/// The editor state as a host sees it (opaque to it): everything but the other modes' sessions,
/// which [`Modes`] holds and `other` names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CallState {
    selected: Vec<Id>,
    active: Option<Id>,
    /// The mesh in edit mode.
    editing: Option<Id>,
    vertices: Vec<u32>,
    select_mode: SelectMode,
    frame: i32,
    view: ViewState,
    shading: Shading,
    grid: bool,
    overlays: bool,
    viewport: [u32; 2],
    /// Which other mode is on, and on which object (`"sculpt 4"`), when one is.
    other: Option<String>,
}

/// The other modes' sessions: an armature's edit or pose mode, weight paint, a curve's, text's or
/// lattice's edit mode, sculpt, Particle Edit, and proportional editing.
#[derive(Debug, Clone, Default)]
pub(crate) struct Modes {
    rig: Option<commands_rig::RigSession>,
    paint: Option<commands_rig2::PaintSession>,
    shape: Option<commands_curve::ShapeSession>,
    sculpt: Option<commands_sculpt::SculptSession>,
    particle_edit: Option<commands_particles::ParticleEditSession>,
    proportional: Option<kition_scene_model::proportional::Proportional>,
}

/// Everything [`SceneDocument::put_back`] puts back.
#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    state: CallState,
    modes: Modes,
}

/// How many snapshots of the other modes' sessions `editor_state` keeps for `restore_editor_state`
/// (a host restores the one it saved before a call).
const KEPT: usize = 8;
/// How many editor states `editor_state` remembers having answered, so `restore_editor_state`
/// tells its own (put back as they are) from another document's (checked first): more than the
/// states one call's steps pass through.
const ANSWERED: usize = 256;

impl SceneDocument {
    /// What the editor shows and selects now.
    pub(crate) fn snapshot(&self) -> Snapshot {
        let session = &self.session;
        let other = match (&session.rig, &session.paint, &session.shape, &session.sculpt, &session.particle_edit) {
            (Some(rig), ..) => Some(format!("{} {}", if rig.pose { "pose" } else { "armatureEdit" }, rig.object)),
            (_, Some(paint), ..) => Some(format!("weightPaint {}", paint.object)),
            (_, _, Some(shape), ..) => Some(match shape {
                commands_curve::ShapeSession::Curve(edit) => format!("curveEdit {}", edit.object),
                commands_curve::ShapeSession::Text(edit) => format!("textEdit {}", edit.object),
                commands_curve::ShapeSession::Lattice(edit) => format!("latticeEdit {}", edit.object),
            }),
            (_, _, _, Some(sculpt), _) => Some(format!("sculpt {}", sculpt.object)),
            (.., Some(groom)) => Some(format!("particleEdit {}", groom.object)),
            _ => None,
        };
        Snapshot {
            state: CallState {
                selected: session.editor.selected.clone(),
                active: session.editor.active,
                editing: match session.mode {
                    Mode::Edit(id) => Some(id),
                    Mode::Object => None,
                },
                vertices: session.vertices.iter().copied().collect(),
                select_mode: session.select_mode,
                frame: self.scene.animation.timeline.frame,
                view: session.editor.view.clone(),
                shading: session.editor.shading,
                grid: session.editor.grid,
                overlays: session.editor.overlays,
                viewport: [session.width, session.height],
                other,
            },
            modes: Modes {
                rig: session.rig.clone(),
                paint: session.paint.clone(),
                shape: session.shape.clone(),
                sculpt: session.sculpt.clone(),
                particle_edit: session.particle_edit.clone(),
                proportional: session.proportional,
            },
        }
    }

    /// Puts back what [`SceneDocument::snapshot`] took, leaving out the objects and vertices the
    /// scene no longer has; no edit.
    pub(crate) fn put_back(&mut self, snapshot: &Snapshot) {
        let (state, modes) = (&snapshot.state, &snapshot.modes);
        let session = &mut self.session;
        session.editor.selected.clone_from(&state.selected);
        session.editor.active = state.active;
        session.mode = state.editing.map_or(Mode::Object, Mode::Edit);
        session.vertices = state.vertices.iter().copied().collect::<BTreeSet<u32>>();
        session.select_mode = state.select_mode;
        session.editor.view.clone_from(&state.view);
        session.editor.shading = state.shading;
        session.editor.grid = state.grid;
        session.editor.overlays = state.overlays;
        let [width, height] = state.viewport;
        if (1..=crate::MAX_VIEWPORT).contains(&width) && (1..=crate::MAX_VIEWPORT).contains(&height) {
            (session.width, session.height) = (width, height);
        }
        session.rig.clone_from(&modes.rig);
        session.paint.clone_from(&modes.paint);
        session.shape.clone_from(&modes.shape);
        session.sculpt.clone_from(&modes.sculpt);
        session.particle_edit.clone_from(&modes.particle_edit);
        session.proportional = modes.proportional;
        if self.scene.animation.timeline.frame != state.frame {
            self.scene.go_to_frame(state.frame as f32);
        }
        self.tidy_session();
        self.repaint();
    }

    /// The viewport's picture is drawn again, without moving the revision (no edit).
    fn repaint(&self) {
        *self.cache.borrow_mut() = None;
        self.viewport_trace.borrow_mut().current = None;
    }

    /// [`kition_kit::KitDocument::editor_state`]: the snapshot as JSON, its other modes' sessions
    /// kept beside the document for [`SceneDocument::restore_state`].
    pub(crate) fn state_value(&self) -> Value {
        let snapshot = self.snapshot();
        let value = serde_json::to_value(&snapshot.state).unwrap_or(Value::Null);
        {
            let mut answered = self.answered.borrow_mut();
            answered.retain(|saved| saved != &value);
            answered.push(value.clone());
            if answered.len() > ANSWERED {
                answered.remove(0);
            }
        }
        if snapshot.state.other.is_some() || snapshot.modes.proportional.is_some() {
            let mut kept = self.kept_modes.borrow_mut();
            kept.retain(|(saved, _)| saved != &value);
            kept.push((value.clone(), snapshot.modes));
            if kept.len() > KEPT {
                kept.remove(0);
            }
        }
        value
    }

    /// [`kition_kit::KitDocument::restore_editor_state`].
    pub(crate) fn restore_state(&mut self, saved: &Value) -> Result<(), String> {
        if saved.is_null() {
            return Ok(());
        }
        let state: CallState = serde_json::from_value(saved.clone())
            .map_err(|error| format!("invalid-params: this is not the editor state this kit answered ({error})"))?;
        // A state another document answered (the session before its file was read again) is put
        // back only when everything it names is in this scene; none is silently left out.
        if !self.answered.borrow().contains(saved)
            && let Some(reason) = unresolved(&self.scene, &state)
        {
            return Err(reason);
        }
        let modes = self.kept_modes.borrow().iter().rev().find(|(kept, _)| kept == saved).map(|(_, modes)| modes.clone()).unwrap_or_default();
        self.put_back(&Snapshot { state, modes });
        Ok(())
    }

    /// What of the editor state now `scene` no longer has ([`unresolved`]).
    pub(crate) fn unresolved_in(&self, scene: &Scene) -> Option<String> {
        unresolved(scene, &self.snapshot().state)
    }

    /// [`kition_kit::KitDocument::set_context`]: `on` (object ids or names, as scene_inspect lists
    /// them) becomes the selection, the last one the active object; `time` (seconds from the first
    /// frame, as `file.render`'s `target.time`) the current frame. Everything is checked before
    /// anything changes. A scene has no parts.
    pub(crate) fn set_call_context(&mut self, context: &CallContext) -> Result<(), String> {
        if let Some(part) = &context.part {
            return Err(format!(
                "invalid-params: `part` is {part}: a 3D scene has no parts (no slides, pages or sequences); name the objects a step acts on with `on` and its time with `at.time`"
            ));
        }
        let mut on: Vec<Id> = Vec::new();
        for given in &context.on {
            let id = params::resolve(&self.scene, given).map_err(|reason| {
                let reason = reason.split_once(": ").map_or(reason.as_str(), |(_, reason)| reason).to_owned();
                format!(
                    "invalid-params: `on` holds {given}: {reason}; give an object's `id` or `name` as scene_inspect lists them ({})",
                    object_list(&self.scene)
                )
            })?;
            on.retain(|other| *other != id);
            on.push(id);
        }
        let frame = context.time.map(|time| crate::render::frame_at_time(&self.scene, time, "at.time")).transpose()?;
        if !on.is_empty() {
            self.session.editor.active = on.last().copied();
            self.session.editor.selected = on;
        }
        if let Some(frame) = frame
            && frame != self.scene.animation.timeline.frame
        {
            self.scene.go_to_frame(frame as f32);
        }
        self.repaint();
        Ok(())
    }
}

/// What of `state`, an editor state this kit answered, `scene` no longer has: the first selected
/// object, active object, mesh in edit mode (or selected vertex of it), object another mode edits
/// or camera the viewport looks through that is gone, as a reason (`deleted active object 4`).
/// `None` when everything resolves.
pub(crate) fn unresolved(scene: &Scene, state: &CallState) -> Option<String> {
    let exists = |id: Id| scene.object(id).is_ok();
    if let Some(gone) = state.selected.iter().find(|id| !exists(**id)) {
        return Some(format!("deleted selected object {gone}"));
    }
    if let Some(gone) = state.active.filter(|id| !exists(*id)) {
        return Some(format!("deleted active object {gone}"));
    }
    if let Some(editing) = state.editing {
        let Some(mesh) = scene.object(editing).ok().and_then(|object| object.mesh()) else {
            return Some(format!("deleted edited mesh {editing}"));
        };
        if let Some(gone) = state.vertices.iter().find(|vertex| **vertex >= mesh.vertex_count()) {
            return Some(format!("deleted selected vertex {gone} of mesh {editing}"));
        }
    }
    if let Some((mode, id)) = state.other.as_deref().and_then(|other| other.split_once(' '))
        && let Ok(id) = id.parse::<Id>()
        && !exists(id)
    {
        return Some(format!("deleted {mode} object {id}"));
    }
    if let Some(camera) = state.view.camera
        && !matches!(scene.object(camera).map(|object| &object.data), Ok(kition_scene_model::ObjectData::Camera(_)))
    {
        return Some(format!("deleted view camera {camera}"));
    }
    None
}

/// The scene's objects as `Name (id)`, the first twelve.
fn object_list(scene: &Scene) -> String {
    let mut names: Vec<String> = scene.objects.iter().take(12).map(|object| format!("{} ({})", object.name, object.id)).collect();
    if scene.objects.len() > 12 {
        names.push(format!("and {} more", scene.objects.len() - 12));
    }
    if names.is_empty() { "it has none".to_owned() } else { names.join(", ") }
}

/// The schema of an edit-mode command's `elements`.
pub(crate) fn elements_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "description": "Runs the command in this mesh's edit mode on these elements and puts the mode and the selections back afterwards, so no `mode.set` or `mesh.select` step is needed: `object` (the mesh, an id or a name), `vertices` and `faces` (indices from 0, as `mesh.select` takes them; every vertex when both are left out)",
        "properties": {
            "object": { "oneOf": [{ "type": "integer", "description": "the object's id" }, { "type": "string", "description": "the object's name" }], "description": "The mesh object" },
            "vertices": { "type": "array", "items": { "type": "integer" }, "description": "Vertex indices, from 0" },
            "faces": { "type": "array", "items": { "type": "integer" }, "description": "Face indices, from 0" }
        }
    })
}

/// Runs `command` with `elements` given: in edit mode on that mesh with those vertices (and the
/// faces' corners) selected, then puts the mode, the selections and everything else the editor
/// shows back, also when the command fails. Every element is checked before anything changes.
pub(crate) fn with_elements(doc: &mut SceneDocument, command: &str, params: &Value, elements: &Value) -> Result<Value, String> {
    let bad = |message: String| params::bad(message);
    let fields = elements.as_object().ok_or_else(|| bad(format!("`elements` is {elements}: give {{object, vertices?, faces?}}")))?;
    if let Some(other) = fields.keys().find(|key| !matches!(key.as_str(), "object" | "vertices" | "faces")) {
        return Err(bad(format!("`elements` has no `{other}`: it is {{object, vertices?, faces?}}")));
    }
    let object = fields
        .get("object")
        .filter(|value| !value.is_null())
        .ok_or_else(|| bad("`elements.object` names the mesh to edit (an id or a name)".to_owned()))?;
    let id = params::resolve(&doc.scene, object)?;
    let found = doc.scene.object(id).map_err(|error| error.to_string())?;
    let mesh = found.mesh().ok_or_else(|| bad(format!("`elements.object` is {}, which is not a mesh", found.name)))?;
    let (count, faces) = (mesh.vertex_count(), mesh.faces.len());
    let mut chosen: BTreeSet<u32> = BTreeSet::new();
    let mut named_faces: BTreeSet<u32> = BTreeSet::new();
    for vertex in params::indices(elements, "vertices")?.unwrap_or_default() {
        if vertex >= count {
            return Err(bad(format!("`elements.vertices` holds {vertex}: {} has vertices 0 to {}", found.name, count.saturating_sub(1))));
        }
        chosen.insert(vertex);
    }
    for face in params::indices(elements, "faces")?.unwrap_or_default() {
        let corners = mesh
            .faces
            .get(face as usize)
            .ok_or_else(|| bad(format!("`elements.faces` holds {face}: {} has faces 0 to {}", found.name, faces.saturating_sub(1))))?;
        chosen.extend(corners.iter().copied());
        named_faces.insert(face);
    }
    // Named faces are exactly the selected faces (with the faces the named vertices close): a
    // face whose corners all belong to named faces is not selected unless it is named.
    let faces_named = fields.get("faces").is_some_and(|value| !value.is_null());
    if faces_named {
        let vertices: BTreeSet<u32> = params::indices(elements, "vertices")?.unwrap_or_default().into_iter().collect();
        named_faces.extend(kition_scene_model::ops::faces_within(mesh, &vertices));
    }
    if fields.get("vertices").is_none() && fields.get("faces").is_none() {
        chosen = (0..count).collect();
    }
    let mut params = params.clone();
    if let Some(map) = params.as_object_mut() {
        map.remove("elements");
    }
    let kept = doc.snapshot();
    let session = &mut doc.session;
    (session.rig, session.paint, session.shape, session.sculpt, session.particle_edit) = (None, None, None, None, None);
    session.mode = Mode::Edit(id);
    session.vertices = chosen;
    session.faces = faces_named.then_some(named_faces);
    // Faces alone are a face selection, as in face select mode.
    if faces_named && fields.get("vertices").is_none_or(Value::is_null) {
        session.select_mode = SelectMode::Face;
    }
    if !session.editor.selected.contains(&id) {
        session.editor.selected.push(id);
    }
    session.editor.active = Some(id);
    let result = crate::commands::execute(doc, command, &params);
    doc.session.faces = None;
    doc.put_back(&kept);
    result
}
