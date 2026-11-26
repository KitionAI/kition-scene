//! The viewport's tools, driven by the host's pointer: Select (click, Shift to toggle, Cmd/Ctrl
//! to pick by the origin; drag for a box, Shift to add to the selection, Cmd/Ctrl to take from it,
//! both to keep what is in both), Move, Rotate, and Scale (drag; Shift for fine steps, Cmd to snap;
//! a click selects as the Select tool's does), and Loop Cut in edit mode (hover to see the loop,
//! click to cut). A drag is one undo step; Escape cancels it. The context-menu key selects the
//! object under the pointer when it is not selected, before the editor opens its menu.

use std::collections::BTreeSet;

use glam::{Mat3, Vec3};
use kition_kit::KitDocument;
use kition_kit::tools::{KitTools, PointerAction, ToolInfo, ToolMods, ToolOverlay, ToolPointer, ToolResponse};
use kition_scene_model::Id;
use serde_json::{Value, json};

use crate::commands::{rotate_objects, translate_objects};
use crate::{Mode, SceneDocument, SelectMode, Step};

/// The tools, in rail order.
const TOOLS: &[(&str, &str, &str, &str, usize)] = &[
    ("select", "Select", "W", "tool-selection", 0),
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

/// Screen pixels a press may travel and still be a click.
const CLICK_PIXELS: f64 = 4.0;

/// Screen pixels from an object's origin within which a Cmd/Ctrl+click picks it by the origin.
const ORIGIN_PIXELS: f32 = 75.0;

const ORANGE: [u8; 4] = [241, 88, 0, 255];
const YELLOW: [u8; 4] = [255, 214, 0, 255];

#[derive(Debug, Clone)]
struct Drag {
    start: (f64, f64),
    last: (f64, f64),
    /// The selection's centre in world space, and on screen.
    pivot: Vec3,
    pivot_screen: (f32, f32),
    before: Step,
    moved: bool,
    /// What the drag shows now (the box, the readout, the dashed line): kept with the drag, so an
    /// answer to the tools' state while it is under way shows them too.
    overlays: Vec<ToolOverlay>,
}

impl Drag {
    fn new(start: (f64, f64), pivot: Vec3, pivot_screen: (f32, f32), before: Step) -> Self {
        Self { start, last: start, pivot, pivot_screen, before, moved: false, overlays: Vec::new() }
    }

    /// Whether the pointer, now at `at`, stayed close enough to the press for a click.
    fn click(&self, at: (f64, f64)) -> bool {
        (at.0 - self.start.0).hypot(at.1 - self.start.1) <= CLICK_PIXELS
    }
}

/// How a click or a box changes the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pick {
    /// What is found becomes the selection.
    Set,
    /// A click with Shift: the object becomes active and selected, or, already active, unselected.
    Toggle,
    /// A box with Shift: what is found is added.
    Extend,
    /// A box with Cmd/Ctrl: what is found is taken out.
    Subtract,
    /// A box with Shift and Cmd/Ctrl: only what is both selected and found stays.
    Intersect,
}

impl Pick {
    fn of(mods: ToolMods, boxed: bool) -> Self {
        match (boxed, mods.shift, mods.command) {
            (false, true, _) => Pick::Toggle,
            (false, false, _) => Pick::Set,
            (true, true, true) => Pick::Intersect,
            (true, true, false) => Pick::Extend,
            (true, false, true) => Pick::Subtract,
            (true, false, false) => Pick::Set,
        }
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
    /// Where the pointer last was over the viewport, for the context-menu key.
    pointer: Option<(f64, f64)>,
}

impl Default for ToolState {
    fn default() -> Self {
        Self { tool: "select", drag: None, hover_edge: None, knife: Vec::new(), hover: None, pointer: None }
    }
}

impl ToolState {
    /// The tool as inspect reports it.
    pub fn describe(&self) -> Value {
        json!({ "id": self.tool, "busy": self.drag.is_some() || !self.knife.is_empty(), "knifePoints": self.knife.len() })
    }
}

fn cursor(tool: &str) -> String {
    match tool {
        "move" => "move",
        "loopCut" | "knife" => "crosshair",
        "slide" => "ew-resize",
        "rotate" | "scale" => "crosshair",
        _ => "default",
    }
    .into()
}

impl SceneDocument {
    fn projector(&self) -> kition_scene_raster::Projector {
        self.session.editor.view.projector(&self.scene, self.session.width, self.session.height)
    }

    /// The world-space centre of what the tools act on (selected objects' origins, or the
    /// selected vertices in edit mode).
    fn pivot(&self) -> Option<Vec3> {
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
    /// origin; a box sets it, with Shift adds to it, with Cmd/Ctrl takes from it, with both keeps
    /// what is in both.
    fn select_at(&mut self, start: (f64, f64), end: (f64, f64), mods: ToolMods) {
        let projector = self.projector();
        let boxed = (end.0 - start.0).abs() > 4.0 || (end.1 - start.1).abs() > 4.0;
        let pick = Pick::of(mods, boxed);
        let through = self.session.editor.view.camera;
        let rect = [start.0 as f32, start.1 as f32, end.0 as f32, end.1 as f32];
        match self.session.mode {
            Mode::Object => {
                let found: Vec<Id> = self.with_frame(|cached| {
                    if boxed {
                        let mut ids = BTreeSet::new();
                        let [x0, y0, x1, y1] = rect;
                        let (left, right, top, bottom) =
                            (x0.min(x1).max(0.0) as u32, x0.max(x1).max(0.0) as u32, y0.min(y1).max(0.0) as u32, y0.max(y1).max(0.0) as u32);
                        for y in top..bottom.min(cached.height) {
                            for x in left..right.min(cached.width) {
                                if let Some(hit) = kition_scene_raster::hit(&cached.canvas, x, y) {
                                    ids.insert(hit.object);
                                }
                            }
                        }
                        for object in self.scene.objects.iter().filter(|object| !object.hidden && object.mesh().is_none()) {
                            if let Ok(world) = self.scene.world_matrix(object.id) {
                                let screen = projector.to_screen(projector.view_projection() * world.transform_point3(Vec3::ZERO).extend(1.0));
                                if screen.x >= x0.min(x1)
                                    && screen.x <= x0.max(x1)
                                    && screen.y >= y0.min(y1)
                                    && screen.y <= y0.max(y1)
                                    && screen.z >= 0.0
                                {
                                    ids.insert(object.id);
                                }
                            }
                        }
                        ids.into_iter().collect()
                    } else if mods.command {
                        kition_scene_raster::pick_origin(&self.scene, &projector, end.0 as f32, end.1 as f32, ORIGIN_PIXELS).into_iter().collect()
                    } else {
                        kition_scene_raster::pick_object(&self.scene, &cached.canvas, &projector, end.0 as f32, end.1 as f32, through)
                            .into_iter()
                            .collect()
                    }
                });
                let found = crate::commands_particles::owners(self, found);
                let editor = &mut self.session.editor;
                match pick {
                    Pick::Set => {
                        editor.active = found.last().copied();
                        editor.selected = found;
                    }
                    Pick::Toggle => {
                        for id in found {
                            if editor.selected.contains(&id) && editor.active == Some(id) {
                                editor.selected.retain(|other| *other != id);
                                editor.active = editor.selected.last().copied();
                            } else {
                                editor.selected.retain(|other| *other != id);
                                editor.selected.push(id);
                                editor.active = Some(id);
                            }
                        }
                    }
                    Pick::Extend => {
                        for id in found {
                            if !editor.selected.contains(&id) {
                                editor.selected.push(id);
                            }
                        }
                        if editor.active.is_none() {
                            editor.active = editor.selected.last().copied();
                        }
                    }
                    Pick::Subtract => {
                        editor.selected.retain(|id| !found.contains(id));
                        if editor.active.is_some_and(|id| !editor.selected.contains(&id)) {
                            editor.active = editor.selected.last().copied();
                        }
                    }
                    Pick::Intersect => {
                        editor.selected.retain(|id| found.contains(id));
                        if editor.active.is_some_and(|id| !editor.selected.contains(&id)) {
                            editor.active = editor.selected.last().copied();
                        }
                    }
                }
            }
            Mode::Edit(id) => {
                let (Ok(world), Some(mesh)) = (self.scene.world_matrix(id), self.scene.object(id).ok().and_then(|object| object.mesh()).cloned())
                else {
                    return;
                };
                let found: BTreeSet<u32> = self.with_frame(|cached| {
                    if boxed {
                        let vertices = kition_scene_raster::vertices_in_rect(&cached.canvas, &projector, &mesh, world, rect);
                        match self.session.select_mode {
                            // Edges whose two ends are in the box are selected with their ends.
                            SelectMode::Vertex | SelectMode::Edge => vertices.into_iter().collect(),
                            // Faces whose every corner is in the box.
                            SelectMode::Face => {
                                let inside: BTreeSet<u32> = vertices.into_iter().collect();
                                kition_scene_model::ops::face_vertices(&mesh, &kition_scene_model::ops::faces_within(&mesh, &inside))
                            }
                        }
                    } else {
                        match self.session.select_mode {
                            SelectMode::Vertex => {
                                kition_scene_raster::pick_vertex(&cached.canvas, &projector, &mesh, world, end.0 as f32, end.1 as f32, 12.0)
                                    .into_iter()
                                    .collect()
                            }
                            SelectMode::Edge => self.edge_at(end.0 as f32, end.1 as f32).map(|(a, b)| BTreeSet::from([a, b])).unwrap_or_default(),
                            SelectMode::Face => kition_scene_raster::hit(&cached.canvas, end.0.max(0.0) as u32, end.1.max(0.0) as u32)
                                .filter(|hit| hit.object == id)
                                .and_then(|hit| mesh.faces.get(hit.face as usize))
                                .map(|face| face.iter().copied().collect())
                                .unwrap_or_default(),
                        }
                    }
                });
                match pick {
                    Pick::Set => self.session.vertices = found,
                    Pick::Toggle => {
                        let all_in = !found.is_empty() && found.iter().all(|vertex| self.session.vertices.contains(vertex));
                        if all_in {
                            self.session.vertices.retain(|vertex| !found.contains(vertex));
                        } else {
                            self.session.vertices.extend(found);
                        }
                    }
                    Pick::Extend => self.session.vertices.extend(found),
                    Pick::Subtract => self.session.vertices.retain(|vertex| !found.contains(vertex)),
                    Pick::Intersect => self.session.vertices.retain(|vertex| found.contains(vertex)),
                }
            }
        }
        self.touched();
    }

    /// Applies a drag of the move, rotate, or scale tool, from the state before it began.
    /// Applies the drag under way again where the pointer last was (after its settings changed).
    pub(crate) fn redrag(&mut self) {
        if let Some(drag) = self.tool.drag.take() {
            if drag.moved {
                let _ = self.apply_drag(&drag, drag.last, ToolMods::default());
            }
            self.tool.drag = Some(drag);
        }
    }

    fn apply_drag(&mut self, drag: &Drag, at: (f64, f64), mods: ToolMods) -> Option<String> {
        let projector = self.projector();
        let original = &drag.before.scene;
        let mut scene = original.clone();
        let fine = if mods.shift { 0.1 } else { 1.0 };
        let (dx, dy) = ((at.0 - drag.start.0) as f32 * fine, (at.1 - drag.start.1) as f32 * fine);
        let pivot_depth = projector.to_screen(projector.view_projection() * drag.pivot.extend(1.0)).z;
        let view_axis = (projector.eye() - drag.pivot).normalize_or_zero();
        let view_axis = if projector.perspective { view_axis } else { Mat3::from_mat4(projector.view.inverse()) * Vec3::Z };
        let (sx, sy) = drag.pivot_screen;
        let status;
        let change: Box<dyn Fn(Vec3) -> Vec3> = match self.tool.tool {
            "move" => {
                let a = projector.unproject(sx, sy, pivot_depth);
                let b = projector.unproject(sx + dx, sy + dy, pivot_depth);
                let mut delta = b - a;
                if mods.command {
                    delta = (delta / 0.25).round() * 0.25;
                }
                status = format!("D: {:.3}  {:.3}  {:.3} m", delta.x, delta.y, delta.z);
                Box::new(move |point| point + delta)
            }
            "rotate" => {
                let begin = ((drag.start.1 as f32 - sy).atan2(drag.start.0 as f32 - sx), (at.1 as f32 - sy).atan2(at.0 as f32 - sx));
                let mut angle = -(begin.1 - begin.0) * fine;
                if mods.command {
                    angle = (angle / 5f32.to_radians()).round() * 5f32.to_radians();
                }
                status = format!("Rotation: {:.1}°", angle.to_degrees());
                let rotation = Mat3::from_axis_angle(view_axis, angle);
                let pivot = drag.pivot;
                if self.session.mode == Mode::Object {
                    let ids = drag.before.selected.clone();
                    if rotate_objects(&mut scene, &ids, rotation).is_err() {
                        return None;
                    }
                }
                Box::new(move |point| pivot + rotation * (point - pivot))
            }
            "scale" => {
                let from = ((drag.start.0 as f32 - sx).hypot(drag.start.1 as f32 - sy)).max(1.0);
                let to = (at.0 as f32 - sx).hypot(at.1 as f32 - sy);
                let mut factor = 1.0 + (to / from - 1.0) * fine;
                if mods.command {
                    factor = (factor * 10.0).round() / 10.0;
                }
                status = format!("Scale: {factor:.3}");
                let pivot = drag.pivot;
                if self.session.mode == Mode::Object {
                    for id in &drag.before.selected {
                        if let Ok(object) = scene.object_mut(*id) {
                            object.scale *= factor;
                        }
                    }
                }
                Box::new(move |point| pivot + (point - pivot) * factor)
            }
            _ => return None,
        };
        match self.session.mode {
            Mode::Object => {
                // Each top-level selected object's origin goes where the change takes it.
                for id in drag.before.selected.clone() {
                    let Ok(world) = original.world_matrix(id) else { continue };
                    let origin = world.transform_point3(Vec3::ZERO);
                    let target = change(origin);
                    let _ = translate_objects(&mut scene, &[id], target - origin);
                }
            }
            Mode::Edit(id) => {
                let Ok(world) = original.world_matrix(id) else { return None };
                let inverse = world.inverse();
                let vertices = drag.before.vertices.clone();
                // With proportional editing, nearby vertices follow part of the way.
                let followers = self
                    .session
                    .proportional
                    .and_then(|settings| {
                        original
                            .object(id)
                            .ok()
                            .and_then(|object| object.mesh())
                            .map(|mesh| kition_scene_model::proportional::weights(mesh, &vertices, settings))
                    })
                    .unwrap_or_default();
                if let Some(mesh) = scene.object_mut(id).ok().and_then(|object| object.mesh_mut()) {
                    for vertex in vertices {
                        if let Some(point) = mesh.positions.get_mut(vertex as usize) {
                            *point = inverse.transform_point3(change(world.transform_point3(*point)));
                        }
                    }
                    for (vertex, weight) in &followers {
                        if let Some(point) = mesh.positions.get_mut(*vertex as usize) {
                            let before = world.transform_point3(*point);
                            *point = inverse.transform_point3(before + (change(before) - before) * *weight);
                        }
                    }
                }
            }
        }
        self.scene = scene;
        self.touched();
        Some(status)
    }

    /// The proportional editing radius round the drag's pivot, as a circle on the screen.
    fn proportional_circle(&self, drag: &Drag) -> Option<ToolOverlay> {
        let settings = self.session.proportional?;
        let Mode::Edit(id) = self.session.mode else { return None };
        let world = self.scene.world_matrix(id).ok()?;
        let scale = world.to_scale_rotation_translation().0.abs().max_element().max(1e-6);
        let projector = self.projector();
        let pixel = projector.pixel_size_at(drag.pivot).max(1e-9);
        let radius = f64::from(settings.radius * scale / pixel);
        if !radius.is_finite() || radius > 100_000.0 {
            return None;
        }
        let (cx, cy) = (f64::from(drag.pivot_screen.0), f64::from(drag.pivot_screen.1));
        let mut path = String::new();
        for step in 0..=48 {
            let angle = std::f64::consts::TAU * f64::from(step) / 48.0;
            let (x, y) = (cx + radius * angle.cos(), cy + radius * angle.sin());
            path.push_str(&format!("{}{x:.1} {y:.1}", if step == 0 { "M" } else { " L" }));
        }
        Some(ToolOverlay::Path { d: path, color: [60, 60, 66, 200], width: 1.0, dashed: true })
    }

    /// The edge under the pointer in edit mode: the nearest on screen within a few pixels.
    fn edge_at(&self, x: f32, y: f32) -> Option<(u32, u32)> {
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

    fn loop_overlays(&self) -> Vec<ToolOverlay> {
        let (Some(edge), Mode::Edit(id)) = (self.tool.hover_edge, self.session.mode) else { return Vec::new() };
        let (Ok(world), Some(mesh)) = (self.scene.world_matrix(id), self.scene.object(id).ok().and_then(|object| object.mesh())) else {
            return Vec::new();
        };
        let Ok((ring, closed)) = kition_scene_model::ops::edge_ring(mesh, edge) else { return Vec::new() };
        let projector = self.projector();
        let matrix = projector.view_projection() * world;
        let middle = |(a, b): (u32, u32)| -> Option<(f64, f64)> {
            let point = (mesh.positions.get(a as usize)? + *mesh.positions.get(b as usize)?) * 0.5;
            let clip = matrix * point.extend(1.0);
            if clip.w <= 0.0 {
                return None;
            }
            let screen = projector.to_screen(clip);
            Some((f64::from(screen.x), f64::from(screen.y)))
        };
        let points: Vec<(f64, f64)> = ring.iter().filter_map(|edge| middle(*edge)).collect();
        let mut overlays: Vec<ToolOverlay> = points
            .windows(2)
            .filter_map(|pair| {
                if let [a, b] = pair { Some(ToolOverlay::Line { ax: a.0, ay: a.1, bx: b.0, by: b.1, color: YELLOW, dashed: false }) } else { None }
            })
            .collect();
        if closed && let (Some(first), Some(last)) = (points.first(), points.last()) {
            overlays.push(ToolOverlay::Line { ax: last.0, ay: last.1, bx: first.0, by: first.1, color: YELLOW, dashed: false });
        }
        overlays
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
        if self.tool.drag.is_some() || self.session.mode != Mode::Object || self.special_session() {
            return;
        }
        let projector = self.projector();
        let through = self.session.editor.view.camera;
        let found =
            self.with_frame(|cached| kition_scene_raster::pick_object(&self.scene, &cached.canvas, &projector, at.0 as f32, at.1 as f32, through));
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
        let busy = self.tool.drag.is_some() || !self.tool.knife.is_empty();
        // Text edit mode takes typed text.
        let wants_text = crate::commands_curve::typing(self);
        ToolResponse { overlays, cursor: if wants_text { "text".into() } else { cursor(self.tool.tool) }, busy, wants_text, ..Default::default() }
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
        self.tool = ToolState { tool: found, ..ToolState::default() };
        Ok(self.respond())
    }

    fn pointer(&mut self, event: &ToolPointer) -> Result<ToolResponse, String> {
        if !(event.x.is_finite() && event.y.is_finite()) {
            return Err("invalid-params: the pointer position is not a number".into());
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
                self.tool.drag = Some(Drag::new(at, Vec3::ZERO, (0.0, 0.0), self.step("Edge Slide")));
            }
            ("slide", PointerAction::Drag) => {
                if let Some(mut drag) = self.tool.drag.take() {
                    let factor = ((at.0 - drag.start.0) / SLIDE_PIXELS).clamp(-1.0, 1.0) as f32;
                    self.scene = drag.before.scene.clone();
                    if let Mode::Edit(id) = self.session.mode {
                        let vertices = self.session.vertices.clone();
                        if let Some(mesh) = self.scene.object_mut(id).ok().and_then(|object| object.mesh_mut()) {
                            let _ = kition_scene_model::knife::edge_slide(mesh, &vertices, factor);
                        }
                    }
                    self.touched();
                    drag.last = at;
                    drag.moved = true;
                    drag.overlays = vec![ToolOverlay::Measure { x: at.0, y: at.1, text: format!("Factor: {factor:.3}") }];
                    self.tool.drag = Some(drag);
                    return Ok(self.respond());
                }
            }
            ("slide", PointerAction::Up) => {
                if let Some(drag) = self.tool.drag.take()
                    && drag.moved
                    && self.scene != drag.before.scene
                {
                    if self.scene.validate().is_ok() {
                        self.committed(drag.before)?;
                    } else {
                        self.restore(drag.before);
                        self.touched();
                    }
                }
            }
            ("loopCut", PointerAction::Move | PointerAction::Drag) => {
                self.tool.hover_edge = self.edge_at(at.0 as f32, at.1 as f32);
            }
            ("loopCut", PointerAction::Down) => {
                if let Some((a, b)) = self.edge_at(at.0 as f32, at.1 as f32) {
                    self.execute("mesh.loopCut", json!({ "edge": [a, b], "cuts": 1 }))?;
                    self.tool.hover_edge = None;
                }
            }
            ("select", PointerAction::Down) => {
                self.tool.drag = Some(Drag::new(at, Vec3::ZERO, (0.0, 0.0), self.step("Select")));
            }
            ("select", PointerAction::Drag) => {
                if let Some(drag) = &mut self.tool.drag {
                    drag.last = at;
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
            ("move" | "rotate" | "scale", PointerAction::Down) => {
                if !self.has_tool_selection() {
                    self.select_at(at, at, ToolMods::default());
                }
                if let Some(pivot) = self.pivot() {
                    let projector = self.projector();
                    let screen = projector.to_screen(projector.view_projection() * pivot.extend(1.0));
                    let label = match self.tool.tool {
                        "move" => "Move",
                        "rotate" => "Rotate",
                        _ => "Scale",
                    };
                    self.tool.drag = Some(Drag::new(at, pivot, (screen.x, screen.y), self.step(label)));
                }
            }
            ("move" | "rotate" | "scale", PointerAction::Drag) => {
                if let Some(mut drag) = self.tool.drag.take() {
                    let status = self.apply_drag(&drag, at, mods);
                    drag.last = at;
                    drag.moved = true;
                    let mut overlays = Vec::new();
                    if self.tool.tool != "move" {
                        overlays.push(ToolOverlay::Line {
                            ax: f64::from(drag.pivot_screen.0),
                            ay: f64::from(drag.pivot_screen.1),
                            bx: at.0,
                            by: at.1,
                            color: ORANGE,
                            dashed: true,
                        });
                    }
                    if let Some(text) = status {
                        overlays.push(ToolOverlay::Measure { x: at.0, y: at.1, text });
                    }
                    if let Some(circle) = self.proportional_circle(&drag) {
                        overlays.push(circle);
                    }
                    drag.overlays = overlays;
                    self.tool.drag = Some(drag);
                    return Ok(self.respond());
                }
            }
            ("move" | "rotate" | "scale", PointerAction::Up) => {
                match self.tool.drag.take() {
                    // A click, as with the Select tool: it selects what is under the pointer, or
                    // clears the selection on nothing (Blender's transform tools select so too).
                    Some(drag) if drag.click(at) => {
                        if self.scene != drag.before.scene {
                            self.restore(drag.before);
                        }
                        self.select_at(at, at, mods);
                    }
                    Some(drag) if drag.moved && self.scene != drag.before.scene => {
                        if self.scene.validate().is_ok() {
                            self.committed(drag.before)?;
                        } else {
                            self.restore(drag.before);
                            self.touched();
                        }
                    }
                    Some(_) => {}
                    // A press that started no drag (nothing to move): a click on nothing.
                    None => self.select_at(at, at, mods),
                }
            }
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

    fn key(&mut self, key: &str, _mods: ToolMods, _zoom: f64) -> Result<ToolResponse, String> {
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
            "Escape" => {
                if let Some(drag) = self.tool.drag.take()
                    && drag.moved
                {
                    self.restore(drag.before);
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

    fn state(&mut self, x: f64, y: f64, _mods: ToolMods, _zoom: f64) -> ToolResponse {
        if self.tool.tool == "loopCut" && x.is_finite() && y.is_finite() {
            self.tool.hover_edge = self.edge_at(x as f32, y as f32);
        }
        self.respond()
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
