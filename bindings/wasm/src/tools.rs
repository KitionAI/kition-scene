//! The viewport's tools, driven by the host's pointer: Select Box (click, Shift to toggle, Cmd/Ctrl
//! to pick by the origin, in edit mode to add the shortest path from the element clicked before;
//! drag for a box, Shift to add to the selection, Cmd/Ctrl to take from it, both to keep what is
//! in both), Select Circle (a press and a drag paint the selection with a circle; Shift adds,
//! Cmd/Ctrl takes out), Select Lasso (drag round what to take, as the box), Move, Rotate, and Scale (a press starts the modal transform,
//! `modal.rs`, and the release confirms it; a click selects as the Select tool's does), and Loop
//! Cut in edit mode (hover to see the loop, click to cut). A drag is one undo step; Escape cancels
//! it. While a transform runs, the pointer and the keys go to it. The context-menu key selects
//! the object under the pointer when it is not selected, before the editor opens its menu.
//! Picking uses rays (`select.rs`), never the software picture; while the editor's GPU viewport
//! draws, an event owes the software picture no repaint (`take_damage`).

use glam::Vec3;
use kition_kit::KitDocument;
use kition_kit::tools::{KitTools, PointerAction, ToolDamage, ToolInfo, ToolMods, ToolOverlay, ToolPointer, ToolResponse};
use kition_scene_model::Id;
use serde_json::{Value, json};

use crate::modal::{Kind, Started};
use crate::select::{Anchor, CIRCLE_PIXELS, MAX_LASSO_POINTS, Pick, Region};
use crate::{Mode, SceneDocument, Step};

/// The tools, in rail order.
const TOOLS: &[(&str, &str, &str, &str, usize)] = &[
    ("select", "Select Box", "W", "tool-selection", 0),
    ("selectCircle", "Select Circle", "W", "tool-ellipse", 0),
    ("selectLasso", "Select Lasso", "W", "tool-lasso", 0),
    ("move", "Move", "G", "tool-move", 1),
    ("rotate", "Rotate", "R", "tool-rotate", 2),
    ("scale", "Scale", "S", "tool-scale", 3),
    ("loopCut", "Loop Cut", "Shift+R", "tool-knife", 4),
    ("knife", "Knife", "K", "tool-line", 5),
    ("slide", "Edge Slide", "Shift+G", "tool-reshape", 6),
];

/// Tools that work on the mesh being edited; outside edit mode they do nothing. They are always
/// listed, since hosts read the tool list once when the document opens.
const EDIT_TOOLS: &[&str] = &["loopCut", "knife", "slide"];

/// Screen pixels of drag for a full slide (factor 1).
const SLIDE_PIXELS: f64 = 200.0;

/// An area of no document pixel: the software picture owes nothing for an event (the GPU
/// viewport shows it; the picture is drawn when asked for).
const NO_PIXELS: ToolDamage = ToolDamage { x0: -2.0, y0: -2.0, x1: -1.0, y1: -1.0 };

const ORANGE: [u8; 4] = [241, 88, 0, 255];
const YELLOW: [u8; 4] = [255, 214, 0, 255];

/// A drag of the Select tool (a box) or of Edge Slide (with the scene before it).
#[derive(Debug, Clone)]
struct Drag {
    start: (f64, f64),
    before: Option<Step>,
    moved: bool,
    /// What the drag shows now (the box, the readout): kept with the drag, so an answer to the
    /// tools' state while it is under way shows them too.
    overlays: Vec<ToolOverlay>,
}

impl Drag {
    fn new(start: (f64, f64), before: Option<Step>) -> Self {
        Self { start, before, moved: false, overlays: Vec::new() }
    }
}

/// The active tool and what it is doing.
#[derive(Debug, Clone)]
pub struct ToolState {
    tool: &'static str,
    drag: Option<Drag>,
    /// The edge a loop cut would cross, under the pointer.
    hover_edge: Option<(u32, u32)>,
    /// The knife's points so far, and where the pointer is.
    knife: Vec<(f64, f64)>,
    hover: Option<(f64, f64)>,
    /// Where the pointer last was over the viewport, for the context-menu key and a transform
    /// a key starts.
    pointer: Option<(f64, f64)>,
    /// The press under way already ended (a key's transform it confirmed, a transform cancelled
    /// while the button was held): its release does nothing.
    ignore_release: bool,
    /// The lasso's outline so far (Select Lasso's drag).
    lasso: Vec<(f64, f64)>,
    /// How Select Circle's drag under way takes what it paints.
    circle: Option<Pick>,
    /// The element the last click in edit mode took, for a Cmd/Ctrl+click's shortest path.
    anchor: Option<Anchor>,
    /// The zoom the pointer last came with (screen pixels per document pixel): Select Circle's
    /// circle keeps its size on the screen.
    zoom: f64,
}

impl Default for ToolState {
    fn default() -> Self {
        Self {
            tool: "select",
            drag: None,
            hover_edge: None,
            knife: Vec::new(),
            hover: None,
            pointer: None,
            ignore_release: false,
            lasso: Vec::new(),
            circle: None,
            anchor: None,
            zoom: 1.0,
        }
    }
}

impl ToolState {
    /// Where the pointer last was over the viewport.
    pub(crate) fn pointer_at(&self) -> Option<(f64, f64)> {
        self.pointer
    }
}

fn cursor(tool: &str) -> String {
    match tool {
        "move" => "move",
        "loopCut" | "knife" | "selectCircle" | "selectLasso" => "crosshair",
        "slide" => "ew-resize",
        "rotate" | "scale" => "crosshair",
        _ => "default",
    }
    .into()
}

impl SceneDocument {
    /// The tool as inspect reports it, with the transform under way.
    pub(crate) fn tool_json(&self) -> Value {
        let tool = &self.tool;
        let modal = self.modal_answer();
        json!({
            "id": tool.tool,
            "busy": tool.drag.is_some() || !tool.knife.is_empty() || self.modal_started().is_some(),
            "knifePoints": tool.knife.len(),
            "modal": modal.get("kind").cloned().unwrap_or(Value::Null),
        })
    }

    pub(crate) fn projector(&self) -> kition_scene_raster::Projector {
        self.session.editor.view.projector(&self.scene, self.session.width, self.session.height)
    }

    /// The world-space centre of what the tools act on (selected objects' origins, or the
    /// selected vertices in edit mode).
    pub(crate) fn pivot(&self) -> Option<Vec3> {
        match self.session.mode {
            Mode::Object => {
                let points: Vec<Vec3> = self
                    .session
                    .editor
                    .selected
                    .iter()
                    .filter_map(|id| self.scene.world_matrix(*id).ok())
                    .map(|world| world.transform_point3(Vec3::ZERO))
                    .collect();
                (!points.is_empty()).then(|| points.iter().copied().sum::<Vec3>() / points.len() as f32)
            }
            Mode::Edit(id) => {
                let world = self.scene.world_matrix(id).ok()?;
                let mesh = self.scene.object(id).ok()?.mesh()?;
                let points: Vec<Vec3> = self
                    .session
                    .vertices
                    .iter()
                    .filter_map(|vertex| mesh.positions.get(*vertex as usize))
                    .map(|point| world.transform_point3(*point))
                    .collect();
                (!points.is_empty()).then(|| points.iter().copied().sum::<Vec3>() / points.len() as f32)
            }
        }
    }

    fn has_tool_selection(&self) -> bool {
        match self.session.mode {
            Mode::Object => !self.session.editor.selected.is_empty(),
            Mode::Edit(_) => !self.session.vertices.is_empty(),
        }
    }

    /// Selects what is under the pointer (or in the box from `start`) as `mods` say: a click sets
    /// the selection, with Shift toggles the object, with Cmd/Ctrl in object mode picks by the
    /// origin and in edit mode adds the shortest path from the element clicked before; a box sets
    /// it, with Shift adds to it, with Cmd/Ctrl takes from it, with both keeps what is in both.
    fn select_at(&mut self, start: (f64, f64), end: (f64, f64), mods: ToolMods) {
        let boxed = (end.0 - start.0).abs() > 4.0 || (end.1 - start.1).abs() > 4.0;
        if boxed {
            self.select_region(&Region::rect(start, end), Pick::of(mods, true));
            return;
        }
        let pick = Pick::of(mods, false);
        let (x, y) = (end.0 as f32, end.1 as f32);
        match self.session.mode {
            Mode::Object => {
                let found: Vec<Id> = self.object_by_ray(x, y, mods.command).into_iter().collect();
                let found = crate::commands_particles::owners(self, found);
                self.take_objects(found, pick);
            }
            Mode::Edit(_) => {
                let (found, anchor) = if mods.command && !mods.shift { self.path_by_ray(self.tool.anchor, x, y) } else { self.element_by_ray(x, y) };
                if anchor.is_some() || pick == Pick::Set {
                    self.tool.anchor = anchor;
                }
                let pick = if mods.command && !mods.shift { Pick::Extend } else { pick };
                pick.vertices(&mut self.session.vertices, found);
            }
        }
        self.touched();
    }

    /// Select Circle at the pointer: what the circle covers, taken as the drag's `pick`.
    fn circle_at(&mut self, at: (f64, f64), zoom: f64, pick: Pick) {
        let radius = (CIRCLE_PIXELS / if zoom.is_finite() && zoom > 0.0 { zoom } else { 1.0 }).clamp(1.0, 1000.0) as f32;
        self.select_region(&Region::Circle { x: at.0 as f32, y: at.1 as f32, radius }, pick);
    }

    /// The circle Select Circle paints with, at the pointer.
    fn circle_overlay(&self, at: (f64, f64), zoom: f64) -> ToolOverlay {
        let radius = (CIRCLE_PIXELS / if zoom.is_finite() && zoom > 0.0 { zoom } else { 1.0 }).clamp(1.0, 1000.0);
        let (x, y) = at;
        ToolOverlay::Path {
            d: format!("M {} {} A {radius} {radius} 0 1 0 {} {} A {radius} {radius} 0 1 0 {} {} Z", x - radius, y, x + radius, y, x - radius, y),
            color: [255, 255, 255, 220],
            width: 1.0,
            dashed: true,
        }
    }

    /// The edge under the pointer in edit mode: the nearest on screen within a few pixels.
    pub(crate) fn edge_at(&self, x: f32, y: f32) -> Option<(u32, u32)> {
        let Mode::Edit(id) = self.session.mode else { return None };
        let world = self.scene.world_matrix(id).ok()?;
        let mesh = self.scene.object(id).ok()?.mesh()?;
        let projector = self.projector();
        let matrix = projector.view_projection() * world;
        let screen = |index: u32| -> Option<(f32, f32)> {
            let clip = matrix * mesh.positions.get(index as usize)?.extend(1.0);
            if clip.w <= 0.0 || clip.z < 0.0 {
                return None;
            }
            let point = projector.to_screen(clip);
            Some((point.x, point.y))
        };
        let mut best: Option<(f32, (u32, u32))> = None;
        for [a, b] in mesh.edges() {
            let (Some(pa), Some(pb)) = (screen(a), screen(b)) else { continue };
            let (ex, ey) = (pb.0 - pa.0, pb.1 - pa.1);
            let length = ex * ex + ey * ey;
            let t = if length > 0.0 { (((x - pa.0) * ex + (y - pa.1) * ey) / length).clamp(0.0, 1.0) } else { 0.0 };
            let distance = (x - (pa.0 + ex * t)).hypot(y - (pa.1 + ey * t));
            if distance < 10.0 && best.is_none_or(|(nearest, _)| distance < nearest) {
                best = Some((distance, (a, b)));
            }
        }
        best.map(|(_, edge)| edge)
    }

    /// The loops a loop cut of `cuts` across `edge` would make, evenly spaced across the ring of
    /// edges it crosses: world-space segments.
    pub(crate) fn loop_segments(&self, edge: (u32, u32), cuts: u32) -> Vec<(Vec3, Vec3)> {
        let Mode::Edit(id) = self.session.mode else { return Vec::new() };
        let (Ok(world), Some(mesh)) = (self.scene.world_matrix(id), self.scene.object(id).ok().and_then(|object| object.mesh())) else {
            return Vec::new();
        };
        let Ok((ring, closed)) = kition_scene_model::ops::edge_ring(mesh, edge) else { return Vec::new() };
        let cuts = cuts.clamp(1, 64);
        let mut segments = Vec::new();
        for cut in 1..=cuts {
            let t = cut as f32 / (cuts + 1) as f32;
            let across = |(a, b): (u32, u32)| -> Option<Vec3> {
                let (low, high) = (a.min(b), a.max(b));
                Some(world.transform_point3(mesh.positions.get(low as usize)?.lerp(*mesh.positions.get(high as usize)?, t)))
            };
            let points: Vec<Vec3> = ring.iter().filter_map(|edge| across(*edge)).collect();
            segments.extend(points.windows(2).filter_map(|pair| if let [a, b] = pair { Some((*a, *b)) } else { None }));
            if closed && let (Some(first), Some(last)) = (points.first(), points.last()) {
                segments.push((*last, *first));
            }
        }
        segments
    }

    /// The yellow loops of [`Self::loop_segments`] on the screen.
    pub(crate) fn loop_lines(&self, edge: (u32, u32), cuts: u32) -> Vec<ToolOverlay> {
        let projector = self.projector();
        let matrix = projector.view_projection();
        let screen = |point: Vec3| -> Option<(f64, f64)> {
            let clip = matrix * point.extend(1.0);
            if clip.w <= 0.0 {
                return None;
            }
            let at = projector.to_screen(clip);
            Some((f64::from(at.x), f64::from(at.y)))
        };
        self.loop_segments(edge, cuts)
            .into_iter()
            .filter_map(|(a, b)| {
                let (a, b) = (screen(a)?, screen(b)?);
                Some(ToolOverlay::Line { ax: a.0, ay: a.1, bx: b.0, by: b.1, color: YELLOW, dashed: false })
            })
            .collect()
    }

    fn loop_overlays(&self) -> Vec<ToolOverlay> {
        match self.tool.hover_edge {
            Some(edge) if self.modal_started().is_none() => self.loop_lines(edge, 1),
            _ => Vec::new(),
        }
    }

    /// Cuts along the knife's points (and forgets them).
    fn confirm_knife(&mut self) -> Result<(), String> {
        let points: Vec<[f64; 2]> = std::mem::take(&mut self.tool.knife).into_iter().map(|(x, y)| [x, y]).collect();
        if points.len() >= 2 {
            self.execute("mesh.knife", json!({ "points": points }))?;
        }
        Ok(())
    }

    /// The knife's line so far, a rubber band to the pointer, and its points.
    fn knife_overlays(&self) -> Vec<ToolOverlay> {
        let mut points = self.tool.knife.clone();
        let placed = points.len();
        if let (Some(hover), true) = (self.tool.hover, placed > 0) {
            points.push(hover);
        }
        let mut overlays: Vec<ToolOverlay> = points
            .windows(2)
            .enumerate()
            .filter_map(|(index, pair)| match pair {
                [a, b] => Some(ToolOverlay::Line { ax: a.0, ay: a.1, bx: b.0, by: b.1, color: YELLOW, dashed: index + 1 >= placed }),
                _ => None,
            })
            .collect();
        overlays.extend(self.tool.knife.iter().map(|(x, y)| ToolOverlay::Anchor { x: *x, y: *y, color: ORANGE, filled: true, size: 6.0 }));
        overlays
    }

    /// Before the context menu opens at the pointer (the right button, or the menu key): in object
    /// mode an object under the pointer that is not selected becomes the selection, as a click
    /// would make it; a selected one keeps the selection as it is, and nothing under the pointer
    /// changes nothing.
    fn select_for_menu(&mut self) {
        let Some(at) = self.tool.pointer else { return };
        if self.tool.drag.is_some() || self.modal_started().is_some() || self.session.mode != Mode::Object || self.special_session() {
            return;
        }
        let found = self.object_by_ray(at.0 as f32, at.1 as f32, false);
        let Some(found) = crate::commands_particles::owners(self, found.into_iter().collect()).last().copied() else { return };
        if !self.session.editor.selected.contains(&found) {
            self.session.editor.selected = vec![found];
            self.session.editor.active = Some(found);
            self.touched();
        }
    }

    /// Whether a mode with its own pointer handling is on (sculpt, weight paint, Particle Edit,
    /// a curve's, text's or lattice's edit mode, an armature's edit or pose mode).
    fn special_session(&self) -> bool {
        let session = &self.session;
        session.sculpt.is_some() || session.paint.is_some() || session.particle_edit.is_some() || session.shape.is_some() || session.rig.is_some()
    }

    fn respond(&self) -> ToolResponse {
        let mut overlays = Vec::new();
        if self.tool.tool == "loopCut" {
            overlays = self.loop_overlays();
        }
        if self.tool.tool == "knife" {
            overlays = self.knife_overlays();
        }
        if let Some(drag) = &self.tool.drag {
            overlays.extend(drag.overlays.iter().cloned());
        }
        if self.tool.tool == "selectCircle"
            && let Some(at) = self.tool.hover
            && self.modal_started().is_none()
        {
            overlays.push(self.circle_overlay(at, self.tool.zoom));
        }
        if self.tool.lasso.len() >= 2 {
            let mut d = String::new();
            for (index, (x, y)) in self.tool.lasso.iter().enumerate() {
                d.push_str(&format!("{}{x} {y} ", if index == 0 { "M " } else { "L " }));
            }
            d.push('Z');
            overlays.push(ToolOverlay::Path { d, color: [255, 255, 255, 220], width: 1.0, dashed: true });
        }
        overlays.extend(self.modal_overlays());
        let busy = self.tool.drag.is_some()
            || !self.tool.knife.is_empty()
            || !self.tool.lasso.is_empty()
            || self.tool.circle.is_some()
            || self.modal_started().is_some();
        // Text edit mode takes typed text.
        let wants_text = crate::commands_curve::typing(self);
        ToolResponse { overlays, cursor: if wants_text { "text".into() } else { cursor(self.tool.tool) }, busy, wants_text, ..Default::default() }
    }

    /// A pointer event while a transform runs: moving drives it; with a key's transform the left
    /// button confirms it (a press where it began, with nothing changed yet, goes to the tool
    /// instead, as if the key had only picked the tool); with a press's transform the release
    /// confirms it, or, where it was pressed, is a click.
    fn modal_event(&mut self, event: &ToolPointer) -> Result<ToolResponse, String> {
        let at = (event.x, event.y);
        match (event.action, self.modal_started()) {
            (PointerAction::Move | PointerAction::Drag, _) => {
                self.modal_pointer(at, event.mods.shift, event.mods.command);
            }
            (PointerAction::Down, Some(Started::Key)) => {
                self.modal_pointer(at, event.mods.shift, event.mods.command);
                // An operation confirms where it is (a loop cut goes on to its slide).
                if self.modal_operates() {
                    self.modal_confirm()?;
                    self.tool.ignore_release = true;
                    return Ok(self.respond());
                }
                if self.modal_click(at) || !self.modal_changes() {
                    // Nothing done yet (pressed where the key was, or the value is none): the key
                    // only picked the tool, whose press this is.
                    self.modal_cancel()?;
                    return self.pointer(event);
                }
                self.modal_confirm()?;
                self.tool.ignore_release = true;
            }
            (PointerAction::Up, Some(Started::Press { select })) => {
                // The release's place counts: the page's last pointer step may still be on its way.
                self.modal_pointer(at, event.mods.shift, event.mods.command);
                // The Loop Cut tool's press slides the loop it cut; the release ends the slide
                // (where it was pressed: in the middle).
                if self.modal_operates() {
                    self.modal_confirm()?;
                } else if self.modal_click(at) {
                    self.modal_cancel()?;
                    if select {
                        self.select_at(at, at, event.mods);
                    }
                } else {
                    self.modal_confirm()?;
                }
            }
            _ => {}
        }
        Ok(self.respond())
    }

    /// Ends the transform under way from a key or a command while the button may be held: the
    /// release that follows does nothing.
    fn modal_end(&mut self, confirm: bool) -> Result<(), String> {
        let pressed = matches!(self.modal_started(), Some(Started::Press { .. }));
        if confirm {
            self.modal_confirm()?;
        } else {
            self.modal_cancel()?;
        }
        if pressed {
            self.tool.ignore_release = true;
        }
        Ok(())
    }
}

impl KitTools for SceneDocument {
    fn tools(&self) -> Vec<ToolInfo> {
        TOOLS
            .iter()
            .map(|(id, label, shortcut, icon, group)| ToolInfo {
                id: (*id).into(),
                label: (*label).into(),
                shortcut: Some((*shortcut).into()),
                icon: (*icon).into(),
                group: *group,
            })
            .collect()
    }

    fn tool(&self) -> String {
        self.tool.tool.into()
    }

    fn select_tool(&mut self, id: &str, _zoom: f64) -> Result<ToolResponse, String> {
        let Some((found, ..)) = TOOLS.iter().find(|(tool, ..)| *tool == id) else {
            return Err(format!("not-found: no tool {id}"));
        };
        // A transform a tool's press started ends with the tool; a key's goes on.
        if matches!(self.modal_started(), Some(Started::Press { .. })) {
            self.modal_cancel()?;
        }
        self.tool = ToolState { tool: found, pointer: self.tool.pointer, anchor: self.tool.anchor, zoom: self.tool.zoom, ..ToolState::default() };
        Ok(self.respond())
    }

    fn pointer(&mut self, event: &ToolPointer) -> Result<ToolResponse, String> {
        if !(event.x.is_finite() && event.y.is_finite()) {
            return Err("invalid-params: the pointer position is not a number".into());
        }
        self.tool.pointer = Some((event.x, event.y));
        if self.modal_started().is_some() {
            return self.modal_event(event);
        }
        if event.action == PointerAction::Up && std::mem::take(&mut self.tool.ignore_release) {
            return Ok(self.respond());
        }
        if event.action == PointerAction::Down {
            self.tool.ignore_release = false;
        }
        // Sculpt, Particle Edit, and weight paint take the pointer for the brush.
        if let Some(response) = crate::commands_particles::pointer(self, event) {
            return Ok(response);
        }
        if let Some(response) = crate::commands_sculpt::pointer(self, event) {
            return Ok(response);
        }
        if let Some(response) = crate::commands_rig2::pointer(self, event) {
            return Ok(response);
        }
        // A curve or text in its edit mode takes the pointer for its points.
        if let Some(response) = crate::commands_curve::pointer(self, self.tool.tool, event) {
            return Ok(response);
        }
        // An armature in edit or pose mode takes the pointer for its bones.
        if let Some(response) = crate::commands_rig::pointer(self, self.tool.tool, event) {
            return Ok(response);
        }
        let at = (event.x, event.y);
        let mods = event.mods;
        self.tool.pointer = Some(at);
        if EDIT_TOOLS.contains(&self.tool.tool) && !matches!(self.session.mode, Mode::Edit(_)) {
            self.tool.knife.clear();
            return Ok(self.respond());
        }
        match (self.tool.tool, event.action) {
            ("knife", PointerAction::Down) => {
                let at = if mods.command { snap_angle(self.tool.knife.last().copied(), at) } else { at };
                if self.tool.knife.len() < 256 && self.tool.knife.last() != Some(&at) {
                    self.tool.knife.push(at);
                }
            }
            ("knife", PointerAction::Move | PointerAction::Drag) => {
                self.tool.hover = Some(if mods.command { snap_angle(self.tool.knife.last().copied(), at) } else { at });
            }
            ("knife", PointerAction::DoubleClick) => {
                self.confirm_knife()?;
            }
            ("slide", PointerAction::Down) => {
                self.tool.drag = Some(Drag::new(at, Some(self.step("Edge Slide"))));
            }
            ("slide", PointerAction::Drag) => {
                if let Some(mut drag) = self.tool.drag.take() {
                    let factor = ((at.0 - drag.start.0) / SLIDE_PIXELS).clamp(-1.0, 1.0) as f32;
                    if let Some(before) = &drag.before {
                        self.scene = before.scene.clone();
                    }
                    if let Mode::Edit(id) = self.session.mode {
                        let vertices = self.session.vertices.clone();
                        if let Some(mesh) = self.scene.object_mut(id).ok().and_then(|object| object.mesh_mut()) {
                            let _ = kition_scene_model::knife::edge_slide(mesh, &vertices, factor);
                        }
                    }
                    self.touched();
                    drag.moved = true;
                    drag.overlays = vec![ToolOverlay::Measure { x: at.0, y: at.1, text: format!("Factor: {factor:.3}") }];
                    self.tool.drag = Some(drag);
                    return Ok(self.respond());
                }
            }
            ("slide", PointerAction::Up) => {
                if let Some(Drag { before: Some(before), moved: true, .. }) = self.tool.drag.take()
                    && self.scene != before.scene
                {
                    if self.scene.validate().is_ok() {
                        self.committed(before)?;
                    } else {
                        self.restore(before);
                        self.touched();
                    }
                }
            }
            ("loopCut", PointerAction::Move | PointerAction::Drag) => {
                self.tool.hover_edge = self.edge_at(at.0 as f32, at.1 as f32);
            }
            // The press cuts the loop under the pointer and slides it until the release, the same
            // modal as Ctrl+R's after its click.
            ("loopCut", PointerAction::Down) => {
                if self.edge_at(at.0 as f32, at.1 as f32).is_some() && self.modal_begin_operation("loopCut", Some(at), Started::Press { select: false }).is_ok() {
                    self.modal_confirm()?;
                    self.tool.hover_edge = None;
                }
            }
            ("selectCircle", PointerAction::Move) => {
                self.tool.hover = Some(at);
                self.tool.zoom = event.zoom;
            }
            // A press paints with the circle (Set clears first; Shift adds, Cmd/Ctrl takes out),
            // and the drag goes on adding or taking out.
            ("selectCircle", PointerAction::Down) => {
                self.tool.hover = Some(at);
                self.tool.zoom = event.zoom;
                let pick = if mods.command { Pick::Subtract } else if mods.shift { Pick::Extend } else { Pick::Set };
                self.circle_at(at, event.zoom, pick);
                self.tool.circle = Some(if pick == Pick::Subtract { Pick::Subtract } else { Pick::Extend });
            }
            ("selectCircle", PointerAction::Drag) => {
                self.tool.hover = Some(at);
                if let Some(pick) = self.tool.circle {
                    self.circle_at(at, event.zoom, pick);
                }
            }
            ("selectCircle", PointerAction::Up) => {
                self.tool.circle = None;
            }
            ("selectLasso", PointerAction::Down) => {
                self.tool.lasso = vec![at];
            }
            ("selectLasso", PointerAction::Drag) => {
                let far = self.tool.lasso.last().is_none_or(|last| (last.0 - at.0).hypot(last.1 - at.1) >= 2.0);
                if !self.tool.lasso.is_empty() && far && self.tool.lasso.len() < MAX_LASSO_POINTS {
                    self.tool.lasso.push(at);
                }
                return Ok(self.respond());
            }
            // A lasso of a few points is a click.
            ("selectLasso", PointerAction::Up) => {
                let outline = std::mem::take(&mut self.tool.lasso);
                if outline.len() >= 3 {
                    let points = outline.iter().map(|(x, y)| (*x as f32, *y as f32)).collect();
                    self.select_region(&Region::Lasso(points), Pick::of(mods, true));
                } else {
                    self.select_at(at, at, mods);
                }
            }
            ("select", PointerAction::Down) => {
                self.tool.drag = Some(Drag::new(at, None));
            }
            ("select", PointerAction::Drag) => {
                if let Some(drag) = &mut self.tool.drag {
                    let (x0, y0) = drag.start;
                    drag.overlays = vec![ToolOverlay::Marquee { x0, y0, x1: at.0, y1: at.1 }];
                    return Ok(self.respond());
                }
            }
            ("select", PointerAction::Up) => {
                if let Some(drag) = self.tool.drag.take() {
                    self.select_at(drag.start, at, mods);
                }
            }
            // A press starts the transform; its release (in `modal_event`) confirms it, or selects
            // as a click.
            ("move" | "rotate" | "scale", PointerAction::Down) => {
                if !self.has_tool_selection() {
                    self.select_at(at, at, ToolMods::default());
                }
                if let Some(kind) = Kind::parse(self.tool.tool) {
                    // Nothing to move: the release is a click on nothing.
                    let _ = self.modal_begin(kind, Some(at), Started::Press { select: true }, None, false);
                }
            }
            // A press that started no transform (nothing to move): a click on nothing.
            ("move" | "rotate" | "scale", PointerAction::Up) => self.select_at(at, at, mods),
            (_, PointerAction::DoubleClick)
                // A double click on a mesh in object mode edits it.
                if self.session.mode == Mode::Object => {
                    self.select_at(at, at, ToolMods::default());
                    if self.session.editor.active.is_some_and(|id| self.scene.object(id).is_ok_and(|object| object.mesh().is_some())) {
                        self.execute("mode.set", json!({ "mode": "edit" }))?;
                    }
                }
            _ => {}
        }
        Ok(self.respond())
    }

    fn key(&mut self, key: &str, mods: ToolMods, _zoom: f64) -> Result<ToolResponse, String> {
        // A transform under way takes the keys: Escape cancels it, Enter confirms it, the others
        // go to it (Backspace and Tab edit a typed value).
        if self.modal_started().is_some() {
            match key {
                "Escape" => self.modal_end(false)?,
                "Enter" => self.modal_end(true)?,
                _ => {
                    self.modal_key(key, mods.shift, mods.command);
                }
            }
            return Ok(self.respond());
        }
        if crate::commands_curve::text_key(self, key)? {
            return Ok(self.respond());
        }
        if self.tool.tool == "knife" && !self.tool.knife.is_empty() {
            match key {
                "Enter" => {
                    self.confirm_knife()?;
                    return Ok(self.respond());
                }
                "Escape" => {
                    self.tool.knife.clear();
                    return Ok(self.respond());
                }
                "Backspace" => {
                    self.tool.knife.pop();
                    return Ok(self.respond());
                }
                _ => {}
            }
        }
        match key {
            "Escape" if crate::commands_curve::cancel(self) => {}
            "Escape" if crate::commands_sculpt::cancel(self) => {}
            "Escape" if crate::commands_particles::cancel(self) => {}
            "Escape" if crate::commands_rig2::cancel(self) => {}
            "Escape" if crate::commands_rig::cancel(self) => {}
            "Escape" if !self.tool.lasso.is_empty() || self.tool.circle.is_some() => {
                self.tool.lasso.clear();
                self.tool.circle = None;
            }
            "Escape" => {
                if let Some(Drag { before: Some(before), moved: true, .. }) = self.tool.drag.take() {
                    self.restore(before);
                    self.touched();
                }
            }
            "ContextMenu" => self.select_for_menu(),
            "Delete" | "Backspace" if self.tool.drag.is_none() && self.has_tool_selection() => {
                self.execute("edit.delete", json!({}))?;
            }
            _ => {}
        }
        Ok(self.respond())
    }

    fn text(&mut self, text: &str, _zoom: f64) -> Result<ToolResponse, String> {
        crate::commands_curve::type_text(self, text)?;
        Ok(self.respond())
    }

    fn state(&mut self, x: f64, y: f64, _mods: ToolMods, zoom: f64) -> ToolResponse {
        if self.tool.tool == "loopCut" && x.is_finite() && y.is_finite() {
            self.tool.hover_edge = self.edge_at(x as f32, y as f32);
        }
        if self.tool.tool == "selectCircle" && x.is_finite() && y.is_finite() {
            self.tool.hover = Some((x, y));
            self.tool.zoom = zoom;
        }
        self.respond()
    }

    /// While the editor's GPU viewport draws, an event owes the software picture nothing: it is
    /// drawn when something asks for it (the agent's screenshot, the software viewport).
    fn take_damage(&mut self) -> Option<ToolDamage> {
        self.gl_shown.then_some(NO_PIXELS)
    }
}

/// With Cmd held the knife's next point keeps to 45° steps on screen from the last one (its
/// distance along that direction kept).
pub(crate) fn snap_angle(last: Option<(f64, f64)>, at: (f64, f64)) -> (f64, f64) {
    let Some((x0, y0)) = last else { return at };
    let (dx, dy) = (at.0 - x0, at.1 - y0);
    if dx.hypot(dy) < 1e-9 {
        return at;
    }
    let step = std::f64::consts::FRAC_PI_4;
    let angle = (dy.atan2(dx) / step).round() * step;
    let (sin, cos) = angle.sin_cos();
    let length = dx * cos + dy * sin;
    (x0 + length * cos, y0 + length * sin)
}

#[cfg(test)]
mod snap_tests {
    use super::snap_angle;

    #[test]
    fn the_knife_snaps_to_45_degree_steps() {
        let close = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9;
        assert!(close(snap_angle(Some((0.0, 0.0)), (10.0, 1.0)), (10.0, 0.0)));
        assert!(close(snap_angle(Some((0.0, 0.0)), (10.0, 9.0)), (9.5, 9.5)));
        assert!(close(snap_angle(Some((5.0, 5.0)), (5.5, -20.0)), (5.0, -20.0)));
        assert_eq!(snap_angle(None, (3.0, 4.0)), (3.0, 4.0));
    }
}
