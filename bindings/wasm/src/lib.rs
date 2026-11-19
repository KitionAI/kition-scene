//! The Scene kit's engine behind Kition's `KitDocument` contract, on engine contract 2: a `.kscene`
//! document is a KDM replica (`kition-scene-kdm`) with the editing session (view, selection,
//! mode), rendered by the software viewport, driven by commands (people's menus and the agent
//! alike) and by engine tools (the viewport's pointer). Commands run on the scene (the view) and
//! each becomes one transaction of its origin; undo, redo, and a collaborator's changes come back
//! through the replica (`docs/design.md` section 7).
#![forbid(unsafe_code)]

pub mod acts;
mod check;
mod collab;
mod commands;
mod commands_anim;
mod commands_anim2;
mod commands_boids;
mod commands_compositor;
mod commands_curve;
mod commands_field_weights;
mod commands_lattice;
mod commands_model;
mod commands_multires;
mod commands_nla;
mod commands_nodes;
mod commands_particles;
mod commands_physics;
mod commands_presence;
mod commands_promo;
mod commands_render;
mod commands_rig;
mod commands_rig2;
mod commands_sculpt;
mod commands_sculpt2;
mod commands_uv;
mod commands_uvedit;
mod commands_world;
mod context;
pub mod described;
mod files;
mod formats;
pub mod gl;
mod inspect;
pub mod params;
mod place;
mod render;
mod rests;
mod tools;
mod viewport_trace;

use std::cell::RefCell;
use std::collections::BTreeSet;

use kition_kit::kdm::{ActorId, DocId, Origin, Replica, UndoScope};
use kition_kit::session::{Damage, KitSession, KitSessionDocument};
use kition_kit::{KitCommand, KitDocument};
use kition_scene_kdm::blobs::SharedBlobs;
use kition_scene_kdm::{SceneModel, ids, read, write};
use kition_scene_model::{Id, ObjectData, Scene};
use kition_scene_raster::{Canvas, EditOverlay, Overlays, Shading, ViewState};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use tools::ToolState;

/// Commands whose parameters name files the engine reads through lent files.
pub const SANDBOXED: &[&str] = &["compositor.setImage", "file.import", "material.setTexture", "modifier.setImage", "world.setImage"];
/// The places in [`KitDocument::inspect`] that are the editor's state, not the scene's, as JSON
/// Pointers: the viewport (`/view`), the mode and what it edits (`/mode`), the selection
/// (`/selection`: objects, the active one, the edited mesh's vertices and faces), the active tool,
/// the current frame (`/animation/frame`) and the selected objects' key frames; and by name, each
/// animated object's `keyedNow` (which properties have a key on the current frame), the one
/// name in an object's entry that is editor state. They are no part of what `view.describe`
/// answers an agent (`scene_inspect`'s details), so orbiting, selecting, entering edit mode or
/// going to another frame changes nothing the agent reads of the scene, while the scene's own
/// fields of the same names (the render's colour `view`, a modifier's `mode`, a key's `frame`)
/// stay. `kition-scene` passes them to kition-headless as its volatile keys.
pub const EDITOR_STATE: &[&str] = &["/selection", "/view", "/mode", "/tool", "/animation/frame", "/animation/selectedKeyFrames", "keyedNow"];

/// The viewport size a document opens with when it is made without one, or opened, until the
/// editor says how large its canvas is.
pub const DEFAULT_VIEWPORT: (u32, u32) = (1280, 720);
/// The largest viewport side.
const MAX_VIEWPORT: u32 = 4096;
/// The origin of the host's own person (contract 1's `execute`).
pub fn person() -> Origin {
    Origin::person("person:local")
}

/// Object or edit mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Object,
    /// Editing this mesh object's vertices and faces.
    Edit(Id),
}

/// What clicks select in edit mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SelectMode {
    #[default]
    Vertex,
    /// An edge is selected when both its ends are.
    Edge,
    Face,
}

/// The editor state saved with the file (but never part of the scene, and never dirtying it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Editor {
    /// UV options are local editor preferences, saved without dirtying the replica.
    #[serde(default)]
    pub uv_live_unwrap: bool,
    #[serde(default)]
    pub uv_unwrap_method: kition_scene_model::unwrap::Method,
    #[serde(default = "uv_iterations")]
    pub uv_unwrap_iterations: u32,
    #[serde(default)]
    pub view: ViewState,
    #[serde(default)]
    pub shading: Shading,
    #[serde(default = "yes")]
    pub grid: bool,
    /// Every overlay (grid, lights, cameras, empties, origins, outlines, edit marks).
    #[serde(default = "yes")]
    pub overlays: bool,
    #[serde(default)]
    pub selected: Vec<Id>,
    #[serde(default)]
    pub active: Option<Id>,
    /// The current frame: each person's own, never part of the replica.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<i32>,
    /// The viewport's size when saved, so a reopened document shows what was saved until the
    /// editor says how large its canvas is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<[u32; 2]>,
}

fn uv_iterations() -> u32 {
    30
}

fn yes() -> bool {
    true
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            uv_live_unwrap: false,
            uv_unwrap_method: Default::default(),
            uv_unwrap_iterations: uv_iterations(),
            view: ViewState::default(),
            shading: Shading::Solid,
            grid: true,
            overlays: true,
            selected: Vec::new(),
            active: None,
            frame: None,
            viewport: None,
        }
    }
}

/// The editing session.
#[derive(Debug, Clone)]
pub struct Session {
    pub editor: Editor,
    pub width: u32,
    pub height: u32,
    pub mode: Mode,
    pub select_mode: SelectMode,
    /// Selected vertices of the mesh in edit mode; a face is selected when all its vertices are.
    pub vertices: BTreeSet<u32>,
    /// The faces a command's `elements.faces` names, for that one command: then exactly these are
    /// the selected faces (the vertex selection holds their corners). `None` otherwise.
    pub faces: Option<BTreeSet<u32>>,
    /// Proportional editing in edit mode, when on.
    pub proportional: Option<kition_scene_model::proportional::Proportional>,
    /// An armature in edit or pose mode (the document is otherwise in object mode).
    pub rig: Option<commands_rig::RigSession>,
    /// A mesh in weight paint mode (the document is otherwise in object mode).
    pub paint: Option<commands_rig2::PaintSession>,
    /// Co-editing presence: the other people, and this person's pointer.
    pub presence: commands_presence::PresenceSession,
    /// A curve's points or a text's letters being edited (the document is otherwise in object
    /// mode).
    pub shape: Option<commands_curve::ShapeSession>,
    /// A mesh in sculpt mode (the document is otherwise in object mode).
    pub sculpt: Option<commands_sculpt::SculptSession>,
    /// The physics bake this person shows, and one under way (never saved).
    pub physics: commands_physics::PhysicsSession,
    /// A hair system being groomed (the document is otherwise in object mode).
    pub particle_edit: Option<commands_particles::ParticleEditSession>,
    /// The host's colour scheme, which the viewport's background and grid follow (`view.scheme`).
    /// The editor's, never the file's: it is not saved and leaves the document as it is.
    pub scheme: kition_scene_raster::Scheme,
}

/// The state a command started from, to put back if it fails (the replica keeps the history).
#[derive(Debug, Clone)]
struct Step {
    scene: Scene,
    selected: Vec<Id>,
    active: Option<Id>,
    mode: Mode,
    vertices: BTreeSet<u32>,
    label: String,
}

struct Cached {
    revision: u64,
    width: u32,
    height: u32,
    canvas: Canvas,
    rgba: Vec<u8>,
}

/// An open scene.
pub struct SceneDocument {
    /// The view: the replica's scene at the local frame, as the last command left it.
    scene: Scene,
    session: Session,
    /// The scene's KDM replica (engine contract 2); its model is the scene as stored.
    replica: KitSession<SceneModel>,
    /// The meshes and images the replica references, shared with the replica's store.
    blobs: SharedBlobs,
    /// Which versions brought which blobs, for `blobsSince`.
    blob_log: collab::BlobLog,
    /// Who the running command acts for: the person, or an agent through `execute_as`.
    origin: Origin,
    /// The labels of the steps each scope can undo and redo, newest last.
    history: History,
    /// The snapshot the document was opened from and the replica's version then: a save with no
    /// change since writes those bytes again, so an unchanged file never changes.
    opened: Option<(kition_kit::kdm::Version, Vec<u8>)>,
    /// Moves with every change, the view's included (it asks for a new picture).
    revision: u64,
    /// Moves with changes to the scene only (they need saving).
    content: u64,
    saved: u64,
    cache: RefCell<Option<Cached>>,
    /// Evaluated meshes and shadow maps kept between pictures (orbiting reuses them).
    meshes: RefCell<kition_scene_raster::MeshCache>,
    tool: ToolState,
    /// The path-traced render under way (session state, never saved).
    trace: Option<kition_scene_trace::Tracer>,
    /// The progressive viewport, independent of the final render and never saved.
    viewport_trace: RefCell<viewport_trace::State>,
    /// Particles worked out for pictures (never saved).
    particles: RefCell<commands_particles::Derived>,
    /// The other modes' sessions of the editor states `editor_state` answered lately, for
    /// `restore_editor_state` (never saved).
    kept_modes: RefCell<Vec<(Value, context::Modes)>>,
    /// The editor states `editor_state` answered lately: `restore_editor_state` puts one of them
    /// back as it is, and checks any other against the scene first (never saved).
    answered: RefCell<Vec<Value>>,
}

/// One undo step as the kit recorded it: its label, and what it wrote.
#[derive(Debug, Clone, Default)]
struct Recorded {
    label: String,
    touched: rests::Touched,
}

/// The undo and redo steps of one scope, newest last.
#[derive(Debug, Clone, Default)]
struct Labels {
    undo: Vec<Recorded>,
    redo: Vec<Recorded>,
}

/// The steps per undo scope (the replica keeps the steps; inspect names them, and an undo's
/// premises are what they wrote).
#[derive(Debug, Clone, Default)]
pub(crate) struct History {
    person: Labels,
    agent: Labels,
    /// The replica keeps a step the kit recorded nothing of (a model batch's): what an undo or a
    /// redo takes back is not known.
    untold: bool,
}

impl History {
    fn of(&mut self, scope: UndoScope) -> &mut Labels {
        match scope {
            UndoScope::Agent => &mut self.agent,
            _ => &mut self.person,
        }
    }

    /// What every step of `scope` an undo (`undo`) or a redo may take back wrote, newest first;
    /// `None` when it is not known.
    pub(crate) fn steps(&self, scope: UndoScope, undo: bool) -> Option<Vec<&rests::Touched>> {
        if self.untold {
            return None;
        }
        let labels = match scope {
            UndoScope::Agent => &self.agent,
            _ => &self.person,
        };
        Some((if undo { &labels.undo } else { &labels.redo }).iter().rev().map(|step| &step.touched).collect())
    }
}

impl SceneDocument {
    /// A document over a replica and its blobs, with the editor state saved beside it.
    fn attach(replica: KitSession<SceneModel>, blobs: SharedBlobs, editor: Option<Value>) -> Self {
        kition_scene_nodes::install();
        let mut editor: Editor = editor.and_then(|value| serde_json::from_value(value).ok()).unwrap_or_default();
        let mut doc = Self {
            scene: Scene::empty(),
            session: Session {
                editor: Editor::default(),
                width: DEFAULT_VIEWPORT.0,
                height: DEFAULT_VIEWPORT.1,
                mode: Mode::Object,
                select_mode: SelectMode::Vertex,
                vertices: BTreeSet::new(),
                faces: None,
                proportional: None,
                rig: None,
                paint: None,
                presence: commands_presence::PresenceSession::default(),
                shape: None,
                sculpt: None,
                physics: commands_physics::PhysicsSession::default(),
                particle_edit: None,
                scheme: kition_scene_raster::Scheme::Light,
            },
            replica,
            blobs,
            blob_log: collab::BlobLog::default(),
            origin: person(),
            history: History::default(),
            opened: None,
            revision: 0,
            content: 0,
            saved: 0,
            cache: RefCell::new(None),
            meshes: RefCell::new(kition_scene_raster::MeshCache::new()),
            tool: ToolState::default(),
            trace: None,
            viewport_trace: RefCell::default(),
            particles: RefCell::default(),
            kept_modes: RefCell::default(),
            answered: RefCell::default(),
        };
        let frame = editor.frame.take();
        if let Some([width, height]) = editor.viewport.take().filter(|[w, h]| (1..=MAX_VIEWPORT).contains(w) && (1..=MAX_VIEWPORT).contains(h)) {
            doc.session.width = width;
            doc.session.height = height;
        }
        doc.session.editor = editor;
        doc.view_from_replica(frame);
        doc
    }

    /// Makes the view the replica's scene again, at `frame` (the current one when `None`), with
    /// this replica's ids to mint, and drops session state that no longer applies.
    fn view_from_replica(&mut self, frame: Option<i32>) {
        let frame = frame.unwrap_or(self.scene.animation.timeline.frame);
        let mut scene = self.replica.model().stored.clone();
        let actor = self.replica.replica().actor();
        ids::allocate(self.replica.replica(), actor, &mut scene);
        scene.go_to_frame(frame as f32);
        self.scene = scene;
        self.tidy_session();
    }

    /// The replica changed under the view (an undo, a redo, a collaborator's changes, late
    /// blobs): the view follows.
    fn replica_changed(&mut self) {
        self.view_from_replica(None);
        self.content += 1;
        self.revision += 1;
    }

    /// The replica's actor.
    pub fn actor(&self) -> ActorId {
        self.replica.replica().actor()
    }

    /// The scene.
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// The session.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// What a call falls back to when it leaves its object parameter out ([`rests::Editor`]): the
    /// selection, the active object, and each mode's session object.
    fn rests_editor(&self) -> rests::Editor<'_> {
        let session = &self.session;
        rests::Editor {
            selected: &session.editor.selected,
            active: session.editor.active,
            edited: match session.mode {
                Mode::Edit(id) => Some(id),
                Mode::Object => None,
            },
            sculpt: session.sculpt.as_ref().map(|sculpt| sculpt.object),
            shape: session.shape.as_ref().map(|shape| match shape {
                commands_curve::ShapeSession::Curve(edit) => edit.object,
                commands_curve::ShapeSession::Text(edit) => edit.object,
                commands_curve::ShapeSession::Lattice(edit) => edit.object,
            }),
            rig: session.rig.as_ref().map(|rig| rig.object),
            posed: session.rig.as_ref().filter(|rig| rig.pose).map(|rig| (rig.object, rig.active.is_some())),
            paint: session.paint.as_ref().map(|paint| paint.object),
            groom: session.particle_edit.as_ref().map(|groom| (groom.object, groom.system)),
            history: Some(&self.history),
        }
    }

    fn step(&self, label: &str) -> Step {
        Step {
            scene: self.scene.clone(),
            selected: self.session.editor.selected.clone(),
            active: self.session.editor.active,
            mode: self.session.mode,
            vertices: self.session.vertices.clone(),
            label: label.to_string(),
        }
    }

    fn restore(&mut self, step: Step) -> Step {
        let current = self.step(&step.label);
        self.scene = step.scene;
        self.session.editor.selected = step.selected;
        self.session.editor.active = step.active;
        self.session.mode = step.mode;
        self.session.vertices = step.vertices;
        self.tidy_session();
        current
    }

    /// Records the change the running command made (from `before` to the view) as one
    /// transaction of its origin. A change the replica refuses is put back and fails the command.
    fn committed(&mut self, before: Step) -> Result<(), String> {
        // Driven values follow what the command changed.
        let frame = self.scene.animation.timeline.frame as f32;
        self.scene.apply_drivers(frame);
        // Multiresolution offsets that no longer fit their mesh (its topology changed) are none:
        // the file stores none for them, and the scene in memory is the scene its file opens as.
        for object in &mut self.scene.objects {
            if let kition_scene_model::ObjectData::Mesh(mesh) = &mut object.data
                && !mesh.multires.is_empty()
                && kition_scene_model::multires::levels(mesh) == 0
            {
                mesh.multires.clear();
            }
        }
        let stored = self.replica.model().stored.clone();
        // The actor's `keys` register moves only when the command minted ids: an actor that
        // minted none (a rename, a move) writes nothing there, so receipts and others' `others`
        // name only what the command changed.
        let unminted = ids::next(self.replica.replica(), self.actor(), &stored);
        let next = self.scene.allocator.0.map(|(next, _)| next).filter(|next| *next != unminted);
        let after = write::stored_form(&self.scene, &stored);
        let key = ids::key_of(self.actor());
        let origin = self.origin.clone().with_label(before.label.clone());
        let label = before.label.clone();
        let version = self.replica.version();
        // What the step writes, for an undo's premises (C8-scene-engine 5a).
        let touched = UndoScope::of(origin.kind).map(|_| rests::Touched::between(&stored, &after));
        let recorded = self.replica.record(&origin, |tx| Ok(write::write_changes(tx, &stored, &after, next.map(|next| (key.as_str(), next)))?));
        match recorded {
            Ok(written) => {
                let mut added = Vec::new();
                for blob in written.blobs {
                    if self.blobs.insert(&blob.hash, &blob.bytes)? {
                        added.push(blob.hash);
                    }
                }
                self.blob_log.record(self.replica.version(), added);
                self.replica.model_mut().adopt(after);
                commands_physics::tidy(self);
                if self.replica.version() != version
                    && let Some(scope) = UndoScope::of(origin.kind)
                {
                    let labels = self.history.of(scope);
                    labels.undo.push(Recorded { label, touched: touched.unwrap_or_default() });
                    labels.redo.clear();
                }
                self.content += 1;
                self.revision += 1;
                Ok(())
            }
            Err(error) => {
                self.restore(before);
                self.touched();
                Err(format!("failed: the scene's replica refused the change ({error})"))
            }
        }
    }

    /// Undoes (or redoes) the last step of `scope`; answers whether there was one.
    fn step_history(&mut self, scope: UndoScope, undo: bool) -> Result<bool, String> {
        let (done, _) = if undo { self.replica.undo_in(scope) } else { self.replica.redo_in(scope) }.map_err(|error| error.to_string())?;
        if done {
            let labels = self.history.of(scope);
            if undo {
                if let Some(label) = labels.undo.pop() {
                    labels.redo.push(label);
                }
            } else if let Some(label) = labels.redo.pop() {
                labels.undo.push(label);
            }
            self.replica_changed();
        }
        Ok(done)
    }

    /// Whether `scope` (either, when `None`) has a step to undo or redo.
    fn can_step(&self, scope: Option<UndoScope>, undo: bool) -> bool {
        match (scope, undo) {
            (None, true) => self.replica.can_undo_in(UndoScope::Person) || self.replica.can_undo_in(UndoScope::Agent),
            (None, false) => self.replica.can_redo_in(UndoScope::Person) || self.replica.can_redo_in(UndoScope::Agent),
            (Some(scope), true) => self.replica.can_undo_in(scope),
            (Some(scope), false) => self.replica.can_redo_in(scope),
        }
    }

    /// The label of the step `scope` would undo (or redo) next.
    fn step_label(&self, scope: UndoScope, undo: bool) -> Option<String> {
        let labels = match scope {
            UndoScope::Agent => &self.history.agent,
            _ => &self.history.person,
        };
        if undo { labels.undo.last() } else { labels.redo.last() }.map(|step| step.label.clone())
    }

    /// Something only the picture depends on changed (view, selection, mode).
    fn touched(&mut self) {
        self.revision += 1;
    }

    /// Drops session references to objects and vertices that no longer exist.
    fn tidy_session(&mut self) {
        let scene = &self.scene;
        self.session.editor.selected.retain(|id| scene.object(*id).is_ok());
        if self.session.editor.active.is_some_and(|id| scene.object(id).is_err()) {
            self.session.editor.active = self.session.editor.selected.last().copied();
        }
        if let Mode::Edit(id) = self.session.mode {
            match scene.object(id).ok().and_then(|object| object.mesh()) {
                Some(mesh) => {
                    let count = mesh.vertex_count();
                    self.session.vertices.retain(|vertex| *vertex < count);
                }
                None => {
                    self.session.mode = Mode::Object;
                    self.session.vertices.clear();
                }
            }
        }
        if self.session.editor.view.camera.is_some_and(|id| !matches!(scene.object(id).map(|object| &object.data), Ok(ObjectData::Camera(_)))) {
            self.session.editor.view.camera = None;
        }
        commands_rig::tidy(self);
        commands_rig2::tidy(self);
        commands_curve::tidy(self);
        commands_sculpt::tidy(self);
        commands_physics::tidy(self);
        commands_particles::tidy(self);
    }

    fn overlays(&self) -> Overlays {
        if !self.session.editor.overlays {
            return Overlays { scheme: self.session.scheme, hide_extras: true, ..Overlays::default() };
        }
        let edit = match self.session.mode {
            Mode::Edit(object) => {
                let faces = self
                    .scene
                    .object(object)
                    .ok()
                    .and_then(|object| object.mesh())
                    .map(|mesh| kition_scene_model::ops::faces_within(mesh, &self.session.vertices))
                    .unwrap_or_default();
                Some(EditOverlay { object, vertices: self.session.vertices.clone(), faces })
            }
            Mode::Object => None,
        };
        Overlays {
            scheme: self.session.scheme,
            grid: self.session.editor.grid,
            selected: self.session.editor.selected.iter().copied().collect(),
            active: self.session.editor.active,
            edit,
            bones: crate::commands_rig::bone_overlay(self),
            weights: crate::commands_rig2::weight_overlay(self),
            peers: crate::commands_presence::outlines(self),
            curve: crate::commands_curve::curve_overlay(self),
            caret: crate::commands_curve::caret_overlay(self),
            sculpt: crate::commands_sculpt::overlay(self),
            hide_extras: false,
        }
    }

    /// Runs `draw` with the viewport picture for the current revision, rendering it first if
    /// needed.
    fn with_frame<T>(&self, draw: impl FnOnce(&Cached) -> T) -> T {
        let mut cache = self.cache.borrow_mut();
        let fresh = cache
            .as_ref()
            .is_some_and(|cached| cached.revision == self.revision && cached.width == self.session.width && cached.height == self.session.height);
        if !fresh {
            let shown = commands_particles::viewport_scene(self);
            if self.session.editor.shading != Shading::Rendered {
                self.viewport_trace.borrow_mut().current = None;
            }
            let viewport = self.viewport_trace.borrow();
            let picture = viewport.picture(self.revision);
            let canvas = kition_scene_raster::render_viewport_picture(
                shown.as_ref().unwrap_or(&self.scene),
                &self.session.editor.view,
                self.session.width,
                self.session.height,
                self.session.editor.shading,
                &self.overlays(),
                &mut self.meshes.borrow_mut(),
                picture,
            );
            let rgba = canvas.to_rgba8();
            *cache = Some(Cached { revision: self.revision, width: self.session.width, height: self.session.height, canvas, rgba });
        }
        match cache.as_ref() {
            Some(cached) => draw(cached),
            None => draw(&Cached { revision: 0, width: 0, height: 0, canvas: Canvas::new(0, 0, [0.0; 4]), rgba: Vec::new() }),
        }
    }

    /// The editor state as saved with the file.
    fn editor_value(&self) -> Option<Value> {
        let editor = Editor {
            frame: Some(self.scene.animation.timeline.frame),
            viewport: Some([self.session.width, self.session.height]),
            ..self.session.editor.clone()
        };
        serde_json::to_value(&editor).ok()
    }

    /// A PNG of the scene rendered through its camera (and its compositing group, when
    /// compositing is on).
    pub fn render_png(&self, samples: u32) -> Result<Vec<u8>, String> {
        let scene = commands_particles::render_scene(self, self.scene.clone());
        commands_compositor::camera_png(self, &scene, samples)
    }
}

/// An image of the scene as a PNG at most `side` pixels on its longer side (a high dynamic range
/// image clipped to what a screen shows), for thumbnails; `None` when there is no such image.
pub fn image_preview(scene: &kition_scene_model::Scene, hash: &str, side: u32) -> Option<Vec<u8>> {
    let image = scene.image(hash)?;
    let (width, height, rgba) = kition_scene_raster::texture::decode_rgba(image)?;
    let side = side.clamp(1, 1024) as usize;
    let step = width.max(height).div_ceil(side).max(1);
    let (out_width, out_height) = (width.div_ceil(step), height.div_ceil(step));
    let mut out = Vec::with_capacity(out_width * out_height * 4);
    for y in 0..out_height {
        for x in 0..out_width {
            let at = ((y * step).min(height - 1) * width + (x * step).min(width - 1)) * 4;
            out.extend(rgba.get(at..at + 4).unwrap_or(&[0, 0, 0, 255]));
        }
    }
    encode_png(u32::try_from(out_width).ok()?, u32::try_from(out_height).ok()?, &out).ok()
}

/// RGBA8 as a PNG file.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|error| format!("failed: {error}"))?;
        writer.write_image_data(rgba).map_err(|error| format!("failed: {error}"))?;
    }
    Ok(out)
}

impl KitDocument for SceneDocument {
    /// Opens a `.kscene` file: its replica as a new actor of the same document.
    fn open(_name: &str, bytes: &[u8]) -> Result<Self, String> {
        let file = kition_scene_kscene::read(bytes).map_err(|error| error.to_string())?;
        let blobs = SharedBlobs::default();
        for (hash, bytes) in &file.blobs {
            blobs.insert(hash, bytes)?;
        }
        let replica = KitSession::open_snapshot(&file.snapshot, Box::new(blobs.clone()), ActorId::random())
            .map_err(|error| format!("invalid-file: {error}"))?;
        let version = replica.version();
        let mut doc = Self::attach(replica, blobs, file.editor);
        doc.opened = Some((version, file.snapshot));
        Ok(doc)
    }

    /// A new document: the starter scene, written in the replica's creating transaction.
    fn create(_name: &str, width: u32, height: u32) -> Result<Self, String> {
        let scene = Scene::starter();
        let blobs = SharedBlobs::default();
        let (replica, written) = Replica::create_with(kition_scene_kdm::schema(), DocId::new(), ActorId::random(), &Origin::system("create"), |tx| {
            write::write_scene(tx, &scene)
        })
        .map_err(|error| error.to_string())?;
        for blob in written.blobs {
            blobs.insert(&blob.hash, &blob.bytes)?;
        }
        let replica = KitSession::from_replica(replica, Box::new(blobs.clone())).map_err(|error| error.to_string())?;
        let cube = scene.object_named("Cube").map(|object| object.id);
        let editor = Editor { selected: cube.into_iter().collect(), active: cube, ..Editor::default() };
        let mut doc = Self::attach(replica, blobs, serde_json::to_value(editor).ok());
        // The size the host asks for, until the editor says how large its canvas is.
        if (1..=MAX_VIEWPORT).contains(&width) && (1..=MAX_VIEWPORT).contains(&height) {
            doc.session.width = width;
            doc.session.height = height;
        }
        Ok(doc)
    }

    fn width(&self) -> u32 {
        self.session.width
    }

    fn height(&self) -> u32 {
        self.session.height
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn dirty(&self) -> bool {
        self.content != self.saved
    }

    fn render(&self, x: f64, y: f64, scale: f64, width: u32, height: u32) -> Result<Vec<u8>, String> {
        if !(x.is_finite() && y.is_finite() && scale.is_finite() && scale > 0.0)
            || width > kition_kit::MAX_RENDER_SIDE
            || height > kition_kit::MAX_RENDER_SIDE
        {
            return Err("invalid-params: the render area is out of range".into());
        }
        Ok(self.with_frame(|cached| {
            kition_kit::raster::render(
                kition_kit::raster::Frame { rgba: &cached.rgba, width: cached.width, height: cached.height },
                x,
                y,
                scale,
                width,
                height,
            )
        }))
    }

    fn render_area(&self, x0: i32, y0: i32, x1: i32, y1: i32, width: u32, height: u32) -> Result<Vec<u8>, String> {
        if width > kition_kit::MAX_RENDER_SIDE || height > kition_kit::MAX_RENDER_SIDE {
            return Err("invalid-params: the render area is too large".into());
        }
        Ok(self.with_frame(|cached| {
            kition_kit::raster::render_area(
                kition_kit::raster::Frame { rgba: &cached.rgba, width: cached.width, height: cached.height },
                x0,
                y0,
                x1,
                y1,
                width,
                height,
            )
        }))
    }

    fn execute(&mut self, command: &str, params: Value) -> Result<Value, String> {
        commands::execute(self, command, &params).map_err(typed)
    }

    fn inspect(&self) -> Result<Value, String> {
        let mut inspected = inspect::inspect(self);
        commands_anim::extend_inspect(self, &mut inspected);
        commands_anim2::extend_inspect(self, &mut inspected);
        commands_nla::extend_inspect(self, &mut inspected);
        commands_rig::extend_inspect(self, &mut inspected);
        commands_rig2::extend_inspect(self, &mut inspected);
        commands_sculpt::extend_inspect(self, &mut inspected);
        commands_particles::extend_inspect(self, &mut inspected);
        commands_curve::extend_inspect(self, &mut inspected);
        commands_physics::extend_inspect(self, &mut inspected);
        Ok(inspected)
    }

    fn commands(&self) -> Vec<KitCommand> {
        let mut commands: Vec<kition_kit::KitCommand> = { commands::catalogue(self) };
        described::fill(&mut commands);
        kition_kit::files::declare(&mut commands, files::FILES);
        for command in &mut commands {
            command.reads = acts::reads(&command.id);
            command.targets = acts::targets(command.schema.as_ref());
            // A command that names its objects may name them by a world point instead (a place).
            if let Some(schema) = command.schema.as_mut().and_then(Value::as_object_mut)
                && (schema.contains_key("object") || schema.contains_key("objects"))
            {
                schema.insert(place::PICK.into(), place::pick_schema());
            }
            command.places = place::places(&command.id, command.schema.as_ref());
            command.objects = params::object_parameters(&command.id, command.schema.as_ref(), command.targets);
            command.object_types = params::object_types(&command.id, command.objects, command.targets);
            command.name_field = command.targets.map(|_| "name");
            command.name_params = params::name_parameters(&command.id, command.object_types, command.targets);
            command.position_independent = place::POSITION_INDEPENDENT.contains(&command.id.as_str());
        }
        commands
    }

    fn disabled_commands(&self) -> Vec<String> {
        commands::disabled(self)
    }

    fn batch_applied(&mut self) {
        // The batch is a step of the replica the kit wrote nothing of.
        self.history.untold = true;
        self.replica_changed();
    }

    /// The replica's snapshot, the blobs it references, and the editor state.
    fn save(&self, _name: Option<&str>) -> Result<Vec<u8>, String> {
        let snapshot = match &self.opened {
            Some((version, bytes)) if *version == self.replica.version() => bytes.clone(),
            _ => self.replica.snapshot().map_err(|error| error.to_string())?,
        };
        let blobs = read::blob_hashes(self.replica.replica())
            .into_iter()
            .filter_map(|hash| kition_kit::session::BlobSource::get(&self.blobs, &hash).map(|bytes| (hash, bytes.to_vec())))
            .collect();
        kition_scene_kscene::write(&kition_scene_kscene::File { snapshot, blobs, editor: self.editor_value() }).map_err(|error| error.to_string())
    }

    fn mark_saved(&mut self) {
        self.saved = self.content;
    }

    /// Takes another program's save of this clean session's file (`fresh`, the file read again)
    /// into the live replica instead of replacing it: only the file's difference is imported,
    /// outside both undo histories, so the session's own steps stay undoable and keep the file's
    /// edits; ids, the editor state and the blobs stay ([`SceneDocument::reread_file`]).
    fn reconcile_reread(&mut self, fresh: &Self) -> Result<bool, String> {
        self.reread_file(fresh)
    }

    fn check(&self) -> Vec<kition_kit::diagnostics::Diagnostic> {
        check::check(&self.scene)
    }

    /// The object selection and the active object, the mode and the edited mesh's selected
    /// vertices (and the other modes' sessions), the current frame, and the viewport, as an opaque
    /// snapshot (`context::CallState`).
    fn editor_state(&self) -> Value {
        self.state_value()
    }

    /// Puts back what [`KitDocument::editor_state`] answered, leaving out what the scene no longer
    /// has (a call may delete what it selected); a state this document did not answer (the live
    /// session's, on its file read again) is refused by name when the scene lacks what it names
    /// (`deleted selected object 4`). No edit.
    fn restore_editor_state(&mut self, saved: &Value) -> Result<(), String> {
        self.restore_state(saved)
    }

    /// Sets what a call acts on and when: `on`, object ids or names as scene_inspect lists them,
    /// becomes the selection (the last one the active object); `time`, seconds from the first
    /// frame as `file.render`'s `target.time` counts them, the current frame (the frame that starts
    /// at or before it). A scene has no parts. An object or a time the scene lacks is refused,
    /// saying what is valid, and nothing changes. No edit.
    fn set_context(&mut self, context: &kition_kit::CallContext) -> Result<(), String> {
        self.set_call_context(context).map_err(typed)
    }

    /// What a call depends on ([`place::under`]): a world point's object and `covers`, and the
    /// premises the kit states for the commands whose code was read for them; no edit.
    fn under(&self, command: &str, params: &Value) -> Result<Option<kition_kit::Under>, String> {
        let schema = commands::schema_of(self, command);
        let objects = schema.map_or(&[][..], |schema| params::object_parameters(command, Some(schema), acts::targets(Some(schema))));
        let under = place::under(&self.scene, self.rests_editor(), command, schema, objects, params).map_err(typed)?;
        commands_compositor::under(self, command, under, params).map_err(typed)
    }

    /// [`KitDocument::under`] for one step of a run, also on the copy a premise plan reads after
    /// the run's earlier steps. A step that rests on nothing beyond the guard's defaults answers an
    /// empty `Under` rather than none: kition-headless answers none as `command_disabled` ("names
    /// nothing this kit can tell by its place"), which a planned read takes as unavailable and
    /// refuses as unknown (a curve's `curve.select` after `mode.shape`). Both say the same to the
    /// guard: no premise of the kit's own.
    fn under_step(&self, command: &str, params: &Value, _step: &kition_kit::PremiseStep) -> Result<Option<kition_kit::Under>, String> {
        Ok(Some(self.under(command, params)?.unwrap_or_default()))
    }

    /// Whether the objects others changed were at a world point at the agent's basis
    /// ([`place::covered`]); no edit.
    fn covered(&self, command: &str, params: &Value, objects: &[kition_kit::CoveredAsk]) -> Result<Option<kition_kit::CoveredAt>, String> {
        if let Some(answer) = commands_compositor::covered(self, command, params, objects).map_err(typed)? {
            return Ok(answer);
        }
        place::covered(&self.scene, params, objects).map_err(typed)
    }

    /// Objects whose world transforms depend on each asked object, including constrained objects.
    fn carried(&self, ids: &[Value], changes: &[Value]) -> Result<Vec<kition_kit::Carried>, String> {
        Ok(ids
            .iter()
            .filter_map(|value| {
                let id = value.as_u64().or_else(|| value.as_str().and_then(|text| text.parse().ok())).and_then(|id| u32::try_from(id).ok())?;
                if !place::carries_transform(id, changes) {
                    return None;
                }
                self.scene.object(id).ok()?;
                let count = place::carried(&self.scene, &[id]);
                (count > 0).then(|| kition_kit::Carried {
                    id: Value::from(id),
                    carries: kition_kit::Carries { count: count as u64, what: "world transform".into() },
                })
            })
            .collect())
    }

    /// A mesh's topology and local bounds ([`place::derived`]), computed now; no edit.
    fn derived(&self, ids: &[Value], names: &[String]) -> Result<Vec<kition_kit::DerivedValues>, String> {
        place::derived(&self.scene, ids, names).map_err(typed)
    }
}

/// A refusal with its reason typed (`kition_kit::failure`), read from the reason code the kit's
/// messages start with, so a host answers its error code without reading the wording: an unknown
/// command is `command_not_found`; wrong parameters, and an object, material or element the scene
/// lacks, `invalid_params`; a command that cannot run now (nothing selected, the wrong mode)
/// `command_disabled`. Other failures (a change the scene refuses, a damaged file) keep no reason.
pub(crate) fn typed(error: String) -> String {
    use kition_kit::failure::{CommandFailure, FAILURE_SEPARATOR, failure};
    // Already typed: a reason given, or a refusal of the parameters with its errors
    // (`kition_kit::schema::refusal`, which hosts read as `invalid_params`).
    if error.contains(FAILURE_SEPARATOR) || kition_kit::schema::parse_refusal(&error).is_some() {
        return error;
    }
    let reason = match error.split_once(':').map(|(code, _)| code) {
        Some("unknown-command") => CommandFailure::NotFound,
        Some("invalid-params" | "not-found") => CommandFailure::InvalidParams,
        Some("not-allowed") => CommandFailure::Disabled,
        _ => return error,
    };
    failure(reason, error)
}

impl KitSessionDocument for SceneDocument {
    type Model = SceneModel;

    fn session(&self) -> &KitSession<SceneModel> {
        &self.replica
    }

    fn session_mut(&mut self) -> &mut KitSession<SceneModel> {
        &mut self.replica
    }

    /// Runs a command as `origin`: the replica records it as theirs, and it undoes in their
    /// scope. Contract 1's `execute` is this with the host's person.
    ///
    /// An agent's command (not a person's) given what it acts on or when (`objects`, `object`,
    /// `elements`, `frame`, `time`) leaves the selection, the active object, the mode, the current
    /// frame and the viewport as they were, whatever it moved on the way, also when it fails; only
    /// a command whose work is moving them (`select.*`, `mode.*`, `frame.*`, `view.*`, …) keeps its
    /// effect. A person's commands keep the editor's own behaviour (a duplicate selects the copies).
    fn execute_as(&mut self, command: &str, params: Value, origin: &Origin) -> Result<Value, String> {
        let previous = std::mem::replace(&mut self.origin, origin.clone());
        let kept = (origin.kind != kition_kit::kdm::OriginKind::Person && acts::names_what_or_when(&params) && !acts::moves_the_editor(command))
            .then(|| self.snapshot());
        let result = commands::execute(self, command, &params);
        if let Some(kept) = kept {
            self.put_back(&kept);
        }
        self.origin = previous;
        result.map_err(typed)
    }

    fn import_changes(&mut self, bytes: &[u8]) -> Result<Damage, String> {
        let damage = self.replica.import_changes(bytes).map_err(|error| error.to_string())?;
        self.replica_changed();
        Ok(damage)
    }

    fn undo(&mut self) -> Result<(bool, Damage), String> {
        self.undo_in(UndoScope::Person)
    }

    fn redo(&mut self) -> Result<(bool, Damage), String> {
        self.redo_in(UndoScope::Person)
    }

    fn undo_in(&mut self, scope: UndoScope) -> Result<(bool, Damage), String> {
        let done = self.step_history(scope, true)?;
        Ok((done, if done { Damage::Full } else { Damage::None }))
    }

    fn redo_in(&mut self, scope: UndoScope) -> Result<(bool, Damage), String> {
        let done = self.step_history(scope, false)?;
        Ok((done, if done { Damage::Full } else { Damage::None }))
    }
}

kition_kit::export_kit_document!(SceneDocument as "SceneDocument");
kition_kit::export_kit_session!(SceneDocument as "SceneDocument");
kition_kit::export_kit_tools!(SceneDocument as "SceneDocument");

/// The Scene kit's extras on the exported class.
#[cfg(target_arch = "wasm32")]
mod web {
    use kition_kit::wasm_bindgen::prelude::*;

    use crate::__kition_kit_export::WebDocument;

    #[wasm_bindgen(js_class = "SceneDocument", wasm_bindgen = kition_kit::wasm_bindgen)]
    impl WebDocument {
        /// The scene rendered through its camera, as PNG bytes (`samples` per side, 1 to 4).
        #[wasm_bindgen(js_name = renderImage)]
        pub fn render_image(&self, samples: u32) -> Result<Vec<u8>, JsError> {
            self.0.render_png(samples).map_err(|error| JsError::new(&error))
        }

        /// Advances the path-traced render by about `budget_ms` milliseconds; answers its progress
        /// as JSON.
        #[wasm_bindgen(js_name = traceStep)]
        pub fn trace_step(&mut self, budget_ms: f64) -> Result<String, JsError> {
            self.0.trace_step(budget_ms).map(|value| value.to_string()).map_err(|error| JsError::new(&error))
        }

        /// The path-traced picture so far, as PNG bytes.
        #[wasm_bindgen(js_name = traceImage)]
        pub fn trace_image(&self) -> Result<Vec<u8>, JsError> {
            self.0.trace_png().map_err(|error| JsError::new(&error))
        }

        /// Starts tracing one shard of a render split across workers (JSON: `index`, `count`,
        /// and `render.trace.start`'s options); answers its progress as JSON.
        #[wasm_bindgen(js_name = traceShard)]
        pub fn trace_shard(&mut self, json: &str) -> Result<String, JsError> {
            self.0.trace_shard(json).map(|value| value.to_string()).map_err(|error| JsError::new(&error))
        }

        /// The rows of the render under way so far, for `traceMerge`.
        #[wasm_bindgen(js_name = traceSums)]
        pub fn trace_sums(&self) -> Result<Vec<u8>, JsError> {
            self.0.trace_sums().map_err(|error| JsError::new(&error))
        }

        /// Merges a shard's `traceSums` into the render started with `collect`; answers its
        /// progress as JSON.
        #[wasm_bindgen(js_name = traceMerge)]
        pub fn trace_merge(&mut self, sums: &[u8]) -> Result<String, JsError> {
            self.0.trace_merge(sums).map(|value| value.to_string()).map_err(|error| JsError::new(&error))
        }

        /// The scene as it is now, as file bytes, for workers to open (nothing is marked saved).
        #[wasm_bindgen(js_name = traceSnapshot)]
        pub fn trace_snapshot(&self) -> Result<Vec<u8>, JsError> {
            kition_kit::KitDocument::save(&self.0, None).map_err(|error| JsError::new(&error))
        }

        /// An image's encoded bytes by hash (empty when the scene has no such image).
        #[wasm_bindgen(js_name = imageBytes)]
        pub fn image_bytes(&self, hash: &str) -> Vec<u8> {
            self.0.scene().image(hash).map(|image| image.bytes.to_vec()).unwrap_or_default()
        }

        /// One frame of the GPU viewport as JSON: view, objects, materials, lights, overlays.
        #[wasm_bindgen(js_name = glScene)]
        pub fn gl_scene(&self) -> String {
            self.0.gl_scene().to_string()
        }

        /// One object's geometry for the GPU viewport, as the viewport draws it.
        #[wasm_bindgen(js_name = glMesh)]
        pub fn gl_mesh(&self, id: u32, tangents: bool) -> Result<Vec<u8>, JsError> {
            self.0.gl_mesh(id, tangents).map_err(|error| JsError::new(&error))
        }

        /// The world image for the GPU viewport as linear floats (empty without one).
        #[wasm_bindgen(js_name = glWorld)]
        pub fn gl_world(&self) -> Vec<u8> {
            self.0.gl_world()
        }

        #[wasm_bindgen(js_name = imagePreview)]
        pub fn image_preview(&self, hash: &str, side: u32) -> Vec<u8> {
            crate::image_preview(self.0.scene(), hash, side).unwrap_or_default()
        }
    }
}
