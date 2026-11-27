//! Selecting by hand, without the software picture: a click, a box, a circle and a lasso find
//! what they take with rays against the meshes as the viewport draws them (`ray.rs`), so the GPU
//! viewport needs no software frame for them. Without X-ray only what shows is taken (a ray from
//! the eye reaches it first); with X-ray (`view.toggleXray`, Alt+Z, and always in wireframe
//! shading) whatever lies in the region is, hidden or not.
//!
//! The selection is the person's session state: none of this is an undo step or an unsaved
//! change. Session commands: `select.circle` and `select.lasso` (a region of viewport pixels, in
//! object and edit mode), and in a mesh's edit mode `select.loop` (the edge loop, or in face mode
//! the face loop, through the edge under a pixel), `select.ring` (the ring of edges across it),
//! `select.linked` (the connected part under a pixel, or every part the selection touches),
//! `select.more` and `select.less` (one step of faces out or in); `view.toggleXray`. A Cmd/Ctrl
//! click in edit mode (the tools) adds the shortest path from the element clicked before.

use std::cmp::Ordering;
use std::collections::{BTreeSet, BinaryHeap, HashMap};
use std::sync::Arc;

use glam::{Mat4, Vec3};
use kition_kit::KitCommand;
use kition_scene_model::{Id, Mesh, mesh::face_edges};
use serde_json::{Value, json};

use crate::params::{self, bad, schema as s};
use crate::ray::RayScene;
use crate::{Mode, SceneDocument, SelectMode, Shading};

/// Screen pixels from a vertex within which a click takes it.
const VERTEX_PIXELS: f32 = 12.0;
/// Screen pixels from an edge within which a click takes it.
const EDGE_PIXELS: f32 = 10.0;
/// Screen pixels from a drawn line of an object without a surface (a light's rings, a camera's
/// pyramid, an empty's axes) within which a click takes it.
const LINE_PIXELS: f32 = 6.0;
/// Screen pixels from an object's origin within which a click takes it when nothing else is there.
const ORIGIN_PIXELS: f32 = 12.0;
/// Screen pixels from an object's origin within which a Cmd/Ctrl+click picks it by the origin.
const BY_ORIGIN_PIXELS: f32 = 75.0;
/// The most points of one mesh a region looks at (an even spread of a bigger one's vertices).
const SAMPLES_PER_MESH: usize = 4096;
/// Rays along each side of a region (a face bigger than the region has no vertex in it).
const GRID_SIDE: usize = 24;
/// The most visibility rays one region select casts for an edited mesh's vertices; past them the
/// vertices in the region are taken as they are (a mesh that big is not edited by hand).
const MAX_OCCLUSION_RAYS: usize = 250_000;
/// The most candidates a click tests for visibility, nearest first.
const CLICK_CANDIDATES: usize = 64;
/// The most layers a ray goes through for X-ray (objects behind objects).
const XRAY_LAYERS: usize = 16;
/// The most selected edges the Select menu's loops and rings start from.
const MAX_MENU_EDGES: usize = 2048;
/// The most points of a lasso.
pub(crate) const MAX_LASSO_POINTS: usize = 4096;
/// The circle's radius in screen pixels, at one device pixel per point.
pub(crate) const CIRCLE_PIXELS: f64 = 25.0;

/// How what a click or a region finds changes the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pick {
    /// What is found becomes the selection.
    Set,
    /// A click with Shift: the object becomes active and selected, or, already active, unselected.
    Toggle,
    /// A region with Shift: what is found is added.
    Extend,
    /// A region with Cmd/Ctrl: what is found is taken out.
    Subtract,
    /// A box with Shift and Cmd/Ctrl: only what is both selected and found stays.
    Intersect,
}

impl Pick {
    pub(crate) fn of(mods: kition_kit::tools::ToolMods, region: bool) -> Self {
        match (region, mods.shift, mods.command) {
            (false, true, _) => Pick::Toggle,
            (false, false, _) => Pick::Set,
            (true, true, true) => Pick::Intersect,
            (true, true, false) => Pick::Extend,
            (true, false, true) => Pick::Subtract,
            (true, false, false) => Pick::Set,
        }
    }

    fn parse(params: &Value) -> Result<Self, String> {
        match params::string(params, "mode")?.unwrap_or("set") {
            "set" => Ok(Pick::Set),
            "extend" => Ok(Pick::Extend),
            "subtract" => Ok(Pick::Subtract),
            other => Err(bad(format!("unknown mode {other}: set, extend or subtract"))),
        }
    }

    /// The vertex selection `selected` once `found` is taken in this way.
    pub(crate) fn vertices(self, selected: &mut BTreeSet<u32>, found: BTreeSet<u32>) {
        match self {
            Pick::Set => *selected = found,
            Pick::Toggle => {
                if !found.is_empty() && found.iter().all(|vertex| selected.contains(vertex)) {
                    selected.retain(|vertex| !found.contains(vertex));
                } else {
                    selected.extend(found);
                }
            }
            Pick::Extend => selected.extend(found),
            Pick::Subtract => selected.retain(|vertex| !found.contains(vertex)),
            Pick::Intersect => selected.retain(|vertex| found.contains(vertex)),
        }
    }
}

/// A region of viewport pixels: a box, a circle, a lasso.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Region {
    Rect { left: f32, top: f32, right: f32, bottom: f32 },
    Circle { x: f32, y: f32, radius: f32 },
    Lasso(Vec<(f32, f32)>),
}

impl Region {
    /// The box between two corners.
    pub(crate) fn rect(a: (f64, f64), b: (f64, f64)) -> Self {
        let (ax, ay, bx, by) = (a.0 as f32, a.1 as f32, b.0 as f32, b.1 as f32);
        Region::Rect { left: ax.min(bx), top: ay.min(by), right: ax.max(bx), bottom: ay.max(by) }
    }

    /// The box round it.
    fn bounds(&self) -> (f32, f32, f32, f32) {
        match self {
            Region::Rect { left, top, right, bottom } => (*left, *top, *right, *bottom),
            Region::Circle { x, y, radius } => (x - radius, y - radius, x + radius, y + radius),
            Region::Lasso(points) => {
                points.iter().fold((f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY), |(l, t, r, b), (x, y)| {
                    (l.min(*x), t.min(*y), r.max(*x), b.max(*y))
                })
            }
        }
    }

    fn contains(&self, x: f32, y: f32) -> bool {
        if !(x.is_finite() && y.is_finite()) {
            return false;
        }
        match self {
            Region::Rect { left, top, right, bottom } => x >= *left && x <= *right && y >= *top && y <= *bottom,
            Region::Circle { x: cx, y: cy, radius } => (x - cx).hypot(y - cy) <= *radius,
            Region::Lasso(points) => {
                // Even-odd: a ray to the right crosses the outline an odd number of times.
                let mut inside = false;
                let count = points.len();
                for index in 0..count {
                    let (Some(&(ax, ay)), Some(&(bx, by))) = (points.get(index), points.get((index + 1) % count)) else { continue };
                    if (ay > y) != (by > y) && x < ax + (y - ay) / (by - ay) * (bx - ax) {
                        inside = !inside;
                    }
                }
                inside
            }
        }
    }

    /// Points spread over it, for rays (at most [`GRID_SIDE`] along each side, at least 2 px apart).
    fn grid(&self) -> Vec<(f32, f32)> {
        let (left, top, right, bottom) = self.bounds();
        if !(left.is_finite() && top.is_finite() && right.is_finite() && bottom.is_finite()) {
            return Vec::new();
        }
        let steps = |length: f32| ((length / 2.0).ceil() as usize).clamp(1, GRID_SIDE);
        let (across, down) = (steps(right - left), steps(bottom - top));
        let mut points = Vec::with_capacity(across * down);
        for row in 0..down {
            for column in 0..across {
                let x = left + (right - left) * (column as f32 + 0.5) / across as f32;
                let y = top + (bottom - top) * (row as f32 + 0.5) / down as f32;
                if self.contains(x, y) {
                    points.push((x, y));
                }
            }
        }
        points
    }

    fn valid(&self) -> bool {
        let (left, top, right, bottom) = self.bounds();
        [left, top, right, bottom].iter().all(|value| value.is_finite() && value.abs() <= 1e7)
    }
}

/// A point of world space on the screen (x, y in pixels), unless it is behind the eye.
fn on_screen(projector: &kition_scene_raster::Projector, point: Vec3) -> Option<(f32, f32)> {
    let clip = projector.view_projection() * point.extend(1.0);
    if clip.w <= 0.0 || clip.z < 0.0 {
        return None;
    }
    let at = projector.to_screen(clip);
    (at.x.is_finite() && at.y.is_finite()).then_some((at.x, at.y))
}

/// Where along the screen segment `a`–`b` the point nearest (`x`, `y`) is, and how far it is.
fn nearest_on_segment(a: (f32, f32), b: (f32, f32), x: f32, y: f32) -> (f32, f32) {
    let (ex, ey) = (b.0 - a.0, b.1 - a.1);
    let length = ex * ex + ey * ey;
    let t = if length > 0.0 { (((x - a.0) * ex + (y - a.1) * ey) / length).clamp(0.0, 1.0) } else { 0.0 };
    (t, (x - (a.0 + ex * t)).hypot(y - (a.1 + ey * t)))
}

fn edge_key(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}

/// A mesh's connections, for walking it.
struct Topology {
    /// The faces of each edge.
    edge_faces: HashMap<(u32, u32), Vec<usize>>,
    /// The edges of each vertex.
    vertex_edges: HashMap<u32, Vec<(u32, u32)>>,
    /// The faces of each vertex.
    vertex_faces: HashMap<u32, Vec<usize>>,
}

impl Topology {
    fn of(mesh: &Mesh) -> Self {
        let mut edge_faces: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
        let mut vertex_faces: HashMap<u32, Vec<usize>> = HashMap::new();
        for (index, face) in mesh.faces.iter().enumerate() {
            for (a, b) in face_edges(face) {
                if a != b {
                    edge_faces.entry(edge_key(a, b)).or_default().push(index);
                }
            }
            for vertex in face {
                vertex_faces.entry(*vertex).or_default().push(index);
            }
        }
        let mut vertex_edges: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
        for [a, b] in mesh.edges() {
            let key = edge_key(a, b);
            vertex_edges.entry(a).or_default().push(key);
            vertex_edges.entry(b).or_default().push(key);
        }
        Self { edge_faces, vertex_edges, vertex_faces }
    }

    fn faces_of(&self, edge: (u32, u32)) -> &[usize] {
        self.edge_faces.get(&edge).map_or(&[], Vec::as_slice)
    }

    /// The edge a loop goes on to at `vertex` from `edge`: across a vertex with four edges, the
    /// one that shares no face with it; along an open border, the border's other edge there.
    /// None where the loop stops (a pole, a corner it cannot pass, a wire).
    fn loop_step(&self, edge: (u32, u32), vertex: u32, border: bool) -> Option<(u32, u32)> {
        let edges = self.vertex_edges.get(&vertex)?;
        let faces = self.faces_of(edge);
        let candidates: Vec<(u32, u32)> = if border {
            edges.iter().copied().filter(|other| *other != edge && self.faces_of(*other).len() == 1).collect()
        } else {
            if edges.len() != 4 || faces.len() != 2 {
                return None;
            }
            edges
                .iter()
                .copied()
                .filter(|other| *other != edge && self.faces_of(*other).len() == 2 && !self.faces_of(*other).iter().any(|face| faces.contains(face)))
                .collect()
        };
        match candidates.as_slice() {
            [only] => Some(*only),
            _ => None,
        }
    }
}

/// The edge loop through `start`: both ways from it until it stops or comes round.
pub(crate) fn edge_loop(mesh: &Mesh, start: (u32, u32)) -> Vec<(u32, u32)> {
    loop_in(&Topology::of(mesh), start)
}

/// [`edge_loop`] in a mesh's connections taken once.
fn loop_in(topology: &Topology, start: (u32, u32)) -> Vec<(u32, u32)> {
    let start = edge_key(start.0, start.1);
    let border = topology.faces_of(start).len() == 1;
    let mut out = vec![start];
    let mut seen = BTreeSet::from([start]);
    let limit = topology.edge_faces.len() + 1;
    for toward in [start.1, start.0] {
        let (mut edge, mut vertex) = (start, toward);
        for _ in 0..limit {
            let Some(next) = topology.loop_step(edge, vertex, border) else { break };
            if !seen.insert(next) {
                break;
            }
            out.push(next);
            vertex = if next.0 == vertex { next.1 } else { next.0 };
            edge = next;
        }
    }
    out
}

/// The faces of the face loop across `edge`: the quads its edge ring runs through.
pub(crate) fn face_loop(mesh: &Mesh, edge: (u32, u32)) -> BTreeSet<u32> {
    let Ok((ring, _)) = kition_scene_model::ops::edge_ring(mesh, edge) else { return BTreeSet::new() };
    let ring: BTreeSet<(u32, u32)> = ring.into_iter().map(|(a, b)| edge_key(a, b)).collect();
    mesh.faces
        .iter()
        .enumerate()
        .filter(|(_, face)| face_edges(face).filter(|(a, b)| ring.contains(&edge_key(*a, *b))).count() >= 2)
        .map(|(index, _)| u32::try_from(index).unwrap_or(u32::MAX))
        .collect()
}

/// Every vertex connected to `seeds` by edges.
pub(crate) fn linked_vertices(mesh: &Mesh, seeds: &BTreeSet<u32>) -> BTreeSet<u32> {
    let topology = Topology::of(mesh);
    let mut found: BTreeSet<u32> = seeds.iter().copied().filter(|vertex| *vertex < mesh.vertex_count()).collect();
    let mut queue: Vec<u32> = found.iter().copied().collect();
    while let Some(vertex) = queue.pop() {
        for (a, b) in topology.vertex_edges.get(&vertex).map_or(&[][..], Vec::as_slice) {
            let other = if *a == vertex { *b } else { *a };
            if found.insert(other) {
                queue.push(other);
            }
        }
    }
    found
}

/// One step out: every vertex of a face that has a selected vertex (Blender's face step), and the
/// ends of a loose edge from a selected vertex.
pub(crate) fn grown(mesh: &Mesh, selected: &BTreeSet<u32>) -> BTreeSet<u32> {
    let topology = Topology::of(mesh);
    let mut out = selected.clone();
    for vertex in selected {
        for face in topology.vertex_faces.get(vertex).map_or(&[][..], Vec::as_slice) {
            if let Some(corners) = mesh.faces.get(*face) {
                out.extend(corners.iter().copied());
            }
        }
        for edge in topology.vertex_edges.get(vertex).map_or(&[][..], Vec::as_slice) {
            if topology.faces_of(*edge).is_empty() {
                out.extend([edge.0, edge.1]);
            }
        }
    }
    out
}

/// One step in: the selected vertices all of whose faces are selected whole (a loose vertex
/// stays while every edge's other end is selected).
pub(crate) fn shrunk(mesh: &Mesh, selected: &BTreeSet<u32>) -> BTreeSet<u32> {
    let topology = Topology::of(mesh);
    selected
        .iter()
        .copied()
        .filter(|vertex| {
            let faces = topology.vertex_faces.get(vertex).map_or(&[][..], Vec::as_slice);
            let whole = faces.iter().all(|face| mesh.faces.get(*face).is_some_and(|corners| corners.iter().all(|corner| selected.contains(corner))));
            let edges = topology.vertex_edges.get(vertex).map_or(&[][..], Vec::as_slice);
            let ends = edges.iter().all(|(a, b)| selected.contains(a) && selected.contains(b));
            if faces.is_empty() { ends } else { whole }
        })
        .collect()
}

/// A distance for the path search's queue (nearest first).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Queued(f32, u32);

impl Eq for Queued {}

impl Ord for Queued {
    fn cmp(&self, other: &Self) -> Ordering {
        other.0.total_cmp(&self.0).then_with(|| other.1.cmp(&self.1))
    }
}

impl PartialOrd for Queued {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The shortest way from node `from` to node `to` over `neighbours` (with each step's length):
/// the nodes along it, both ends included.
fn shortest(nodes: u32, from: u32, to: u32, neighbours: impl Fn(u32) -> Vec<(u32, f32)>) -> Option<Vec<u32>> {
    if from >= nodes || to >= nodes {
        return None;
    }
    let mut best: HashMap<u32, (f32, u32)> = HashMap::from([(from, (0.0, from))]);
    let mut queue = BinaryHeap::from([Queued(0.0, from)]);
    while let Some(Queued(distance, node)) = queue.pop() {
        if node == to {
            break;
        }
        if best.get(&node).is_some_and(|(known, _)| *known < distance) {
            continue;
        }
        for (next, length) in neighbours(node) {
            let through = distance + length.max(0.0);
            if best.get(&next).is_none_or(|(known, _)| through < *known) {
                best.insert(next, (through, node));
                queue.push(Queued(through, next));
            }
        }
    }
    best.get(&to)?;
    let mut path = vec![to];
    let mut at = to;
    while at != from {
        at = best.get(&at)?.1;
        path.push(at);
        if path.len() > nodes as usize + 1 {
            return None;
        }
    }
    path.reverse();
    Some(path)
}

/// The vertices of the shortest path along edges from vertex `from` to vertex `to`.
pub(crate) fn vertex_path(mesh: &Mesh, from: u32, to: u32) -> Option<Vec<u32>> {
    let topology = Topology::of(mesh);
    let at = |vertex: u32| mesh.positions.get(vertex as usize).copied().unwrap_or(Vec3::ZERO);
    shortest(mesh.vertex_count(), from, to, |vertex| {
        topology
            .vertex_edges
            .get(&vertex)
            .map(|edges| {
                edges
                    .iter()
                    .map(|(a, b)| {
                        let other = if *a == vertex { *b } else { *a };
                        (other, at(vertex).distance(at(other)))
                    })
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// The faces of the shortest path across shared edges from face `from` to face `to`.
pub(crate) fn face_path(mesh: &Mesh, from: u32, to: u32) -> Option<Vec<u32>> {
    let topology = Topology::of(mesh);
    let centers: Vec<Vec3> = mesh.faces.iter().map(|face| mesh.face_center(face)).collect();
    let center = |face: u32| centers.get(face as usize).copied().unwrap_or(Vec3::ZERO);
    let count = u32::try_from(mesh.faces.len()).unwrap_or(u32::MAX);
    shortest(count, from, to, |face| {
        let Some(corners) = mesh.faces.get(face as usize) else { return Vec::new() };
        face_edges(corners)
            .flat_map(|(a, b)| topology.faces_of(edge_key(a, b)).iter().copied())
            .filter_map(|other| u32::try_from(other).ok())
            .filter(|other| *other != face)
            .map(|other| (other, center(face).distance(center(other))))
            .collect()
    })
}

/// The element a click in edit mode took, for a later Cmd/Ctrl+click's shortest path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Anchor {
    Vertex { object: Id, vertex: u32 },
    Face { object: Id, face: u32 },
}

impl SceneDocument {
    /// Whether region selects take what is hidden too: X-ray on, or wireframe shading.
    pub(crate) fn xray_shown(&self) -> bool {
        self.session.editor.xray || self.session.editor.shading == Shading::Wireframe
    }

    /// The drawn meshes for rays, kept while the revision stays (a transform under way moves
    /// objects without moving it: then they are taken afresh).
    pub(crate) fn pick_rays(&self) -> Arc<RayScene> {
        if self.modal.is_some() {
            return Arc::new(self.ray_scene(&|_| false));
        }
        let mut cache = self.ray_cache.borrow_mut();
        if let Some((revision, rays)) = cache.as_ref()
            && *revision == self.revision
        {
            return Arc::clone(rays);
        }
        let rays = Arc::new(self.ray_scene(&|_| false));
        *cache = Some((self.revision, Arc::clone(&rays)));
        rays
    }

    /// The object a click at pixel (`x`, `y`) takes: with `by_origin` (Cmd/Ctrl) the one whose
    /// origin is nearest; otherwise an object without a surface whose drawn lines pass within a
    /// few pixels and show there (nearest first), else the surface under the pixel, else the
    /// object whose origin is nearest within a few pixels.
    pub(crate) fn object_by_ray(&self, x: f32, y: f32, by_origin: bool) -> Option<Id> {
        let projector = self.projector();
        if by_origin {
            return kition_scene_raster::pick_origin(&self.scene, &projector, x, y, BY_ORIGIN_PIXELS);
        }
        let rays = self.pick_rays();
        let through = self.session.editor.view.camera;
        let mut best: Option<(f32, Id)> = None;
        for object in self.scene.objects.iter().filter(|object| !object.hidden && !object.has_geometry() && through != Some(object.id)) {
            let Ok(world) = self.scene.world_matrix(object.id) else { continue };
            for [a, b] in kition_scene_raster::gizmo_segments(&self.scene, &object.data) {
                let (a, b) = (world.transform_point3(a), world.transform_point3(b));
                let (Some(sa), Some(sb)) = (on_screen(&projector, a), on_screen(&projector, b)) else { continue };
                let (t, distance) = nearest_on_segment(sa, sb, x, y);
                if !distance.is_finite() || distance > LINE_PIXELS || best.is_some_and(|(nearest, _)| nearest <= distance) {
                    continue;
                }
                if rays.shows(&projector, a.lerp(b, t)) {
                    best = Some((distance, object.id));
                }
            }
        }
        if let Some((_, id)) = best {
            return Some(id);
        }
        if let Some(hit) = rays.at(&projector, x, y) {
            return Some(hit.object);
        }
        kition_scene_raster::pick_origin(&self.scene, &projector, x, y, ORIGIN_PIXELS)
    }

    /// The objects a region takes: a mesh with a point in it that shows (any point, with X-ray),
    /// or whose surface a ray through it meets first; an object without a mesh by its origin.
    pub(crate) fn objects_in_region(&self, region: &Region) -> Vec<Id> {
        if !region.valid() {
            return Vec::new();
        }
        let projector = self.projector();
        let rays = self.pick_rays();
        let xray = self.xray_shown();
        let (left, top, right, bottom) = region.bounds();
        let mut found: BTreeSet<Id> = BTreeSet::new();
        for (id, prepared, world) in rays.meshes() {
            let positions = &prepared.mesh.positions;
            if positions.is_empty() || !Self::box_reaches(&projector, positions, world, (left, top, right, bottom)) {
                continue;
            }
            let stride = positions.len().div_ceil(SAMPLES_PER_MESH).max(1);
            for point in positions.iter().step_by(stride) {
                let point = world.transform_point3(*point);
                let Some((x, y)) = on_screen(&projector, point) else { continue };
                if region.contains(x, y) && (xray || rays.shows(&projector, point)) {
                    found.insert(id);
                    break;
                }
            }
        }
        for (x, y) in region.grid() {
            if xray {
                for _ in 0..XRAY_LAYERS {
                    let Some(hit) = rays.at_except(&projector, x, y, &|id| found.contains(&id)) else { break };
                    found.insert(hit.object);
                }
            } else if let Some(hit) = rays.at(&projector, x, y) {
                found.insert(hit.object);
            }
        }
        for object in self.scene.objects.iter().filter(|object| !object.hidden && object.mesh().is_none()) {
            if let Ok(world) = self.scene.world_matrix(object.id)
                && let Some((x, y)) = on_screen(&projector, world.transform_point3(Vec3::ZERO))
                && region.contains(x, y)
            {
                found.insert(object.id);
            }
        }
        found.into_iter().collect()
    }

    /// Whether the box round `positions` (in the space `world` maps from) reaches the screen
    /// rectangle `bounds` (or comes behind the eye, where it cannot be told).
    fn box_reaches(projector: &kition_scene_raster::Projector, positions: &[Vec3], world: Mat4, bounds: (f32, f32, f32, f32)) -> bool {
        let Some(first) = positions.first() else { return false };
        let (low, high) = positions.iter().fold((*first, *first), |(low, high), point| (low.min(*point), high.max(*point)));
        let mut screen = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
        for corner in 0..8 {
            let point = Vec3::new(
                if corner & 1 == 0 { low.x } else { high.x },
                if corner & 2 == 0 { low.y } else { high.y },
                if corner & 4 == 0 { low.z } else { high.z },
            );
            let Some((x, y)) = on_screen(projector, world.transform_point3(point)) else { return true };
            screen = (screen.0.min(x), screen.1.min(y), screen.2.max(x), screen.3.max(y));
        }
        let (left, top, right, bottom) = bounds;
        screen.0 <= right && screen.2 >= left && screen.1 <= bottom && screen.3 >= top
    }

    /// The edited mesh, its world matrix and its object.
    fn mesh_being_edited(&self) -> Option<(Id, Mat4, &Mesh)> {
        let Mode::Edit(id) = self.session.mode else { return None };
        let world = self.scene.world_matrix(id).ok()?;
        let mesh = self.scene.object(id).ok()?.mesh()?;
        Some((id, world, mesh))
    }

    /// The edited mesh's vertices in a region that show (all of them, with X-ray).
    pub(crate) fn vertices_in_region(&self, region: &Region) -> BTreeSet<u32> {
        let Some((_, world, mesh)) = self.mesh_being_edited() else { return BTreeSet::new() };
        if !region.valid() {
            return BTreeSet::new();
        }
        let projector = self.projector();
        let xray = self.xray_shown();
        let rays = if xray { None } else { Some(self.pick_rays()) };
        let mut cast = 0usize;
        let mut found = BTreeSet::new();
        for (index, point) in mesh.positions.iter().enumerate() {
            let point = world.transform_point3(*point);
            let Some((x, y)) = on_screen(&projector, point) else { continue };
            if !region.contains(x, y) {
                continue;
            }
            if let Some(rays) = &rays
                && cast < MAX_OCCLUSION_RAYS
            {
                cast += 1;
                if !rays.shows(&projector, point) {
                    continue;
                }
            }
            found.insert(u32::try_from(index).unwrap_or(u32::MAX));
        }
        found
    }

    /// The vertices a region takes in the select mode: in face mode the corners of the faces whose
    /// every corner it takes.
    pub(crate) fn elements_in_region(&self, region: &Region) -> BTreeSet<u32> {
        let vertices = self.vertices_in_region(region);
        match (self.session.select_mode, self.mesh_being_edited()) {
            (SelectMode::Face, Some((_, _, mesh))) => {
                kition_scene_model::ops::face_vertices(mesh, &kition_scene_model::ops::faces_within(mesh, &vertices))
            }
            _ => vertices,
        }
    }

    /// The edited mesh's vertex nearest pixel (`x`, `y`) within a few pixels that shows (any,
    /// with X-ray).
    pub(crate) fn vertex_by_ray(&self, x: f32, y: f32) -> Option<u32> {
        let (_, world, mesh) = self.mesh_being_edited()?;
        let projector = self.projector();
        let mut near: Vec<(f32, u32, Vec3)> = mesh
            .positions
            .iter()
            .enumerate()
            .filter_map(|(index, point)| {
                let point = world.transform_point3(*point);
                let (sx, sy) = on_screen(&projector, point)?;
                let distance = (sx - x).hypot(sy - y);
                (distance <= VERTEX_PIXELS).then(|| (distance, u32::try_from(index).unwrap_or(u32::MAX), point))
            })
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        if self.xray_shown() {
            return near.first().map(|(_, vertex, _)| *vertex);
        }
        let rays = self.pick_rays();
        near.into_iter().take(CLICK_CANDIDATES).find(|(_, _, point)| rays.shows(&projector, *point)).map(|(_, vertex, _)| vertex)
    }

    /// The edited mesh's edge nearest pixel (`x`, `y`) within a few pixels whose nearest point
    /// shows (any, with X-ray).
    pub(crate) fn edge_by_ray(&self, x: f32, y: f32) -> Option<(u32, u32)> {
        let (_, world, mesh) = self.mesh_being_edited()?;
        let projector = self.projector();
        let point = |index: u32| mesh.positions.get(index as usize).map(|point| world.transform_point3(*point));
        let mut near: Vec<(f32, (u32, u32), Vec3)> = mesh
            .edges()
            .into_iter()
            .filter_map(|[a, b]| {
                let (pa, pb) = (point(a)?, point(b)?);
                let (sa, sb) = (on_screen(&projector, pa)?, on_screen(&projector, pb)?);
                let (t, distance) = nearest_on_segment(sa, sb, x, y);
                (distance <= EDGE_PIXELS).then(|| (distance, (a, b), pa.lerp(pb, t)))
            })
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        if self.xray_shown() {
            return near.first().map(|(_, edge, _)| *edge);
        }
        let rays = self.pick_rays();
        near.into_iter().take(CLICK_CANDIDATES).find(|(_, _, point)| rays.shows(&projector, *point)).map(|(_, edge, _)| edge)
    }

    /// The edited mesh's face under pixel (`x`, `y`): the first surface there, when it is the
    /// edited mesh's (with X-ray, its nearest face there, whatever is in front).
    pub(crate) fn face_by_ray(&self, x: f32, y: f32) -> Option<u32> {
        let (id, _, mesh) = self.mesh_being_edited()?;
        let projector = self.projector();
        let rays = self.pick_rays();
        let hit = if self.xray_shown() { rays.at_except(&projector, x, y, &|other| other != id) } else { rays.at(&projector, x, y) }?;
        (hit.object == id && (hit.face as usize) < mesh.faces.len()).then_some(hit.face)
    }

    /// The vertices a click at pixel (`x`, `y`) takes in the select mode, and the element taken.
    pub(crate) fn element_by_ray(&self, x: f32, y: f32) -> (BTreeSet<u32>, Option<Anchor>) {
        let Some((id, world, mesh)) = self.mesh_being_edited() else { return (BTreeSet::new(), None) };
        match self.session.select_mode {
            SelectMode::Vertex => match self.vertex_by_ray(x, y) {
                Some(vertex) => (BTreeSet::from([vertex]), Some(Anchor::Vertex { object: id, vertex })),
                None => (BTreeSet::new(), None),
            },
            SelectMode::Edge => match self.edge_by_ray(x, y) {
                Some((a, b)) => {
                    // The end nearer the click anchors a path.
                    let projector = self.projector();
                    let distance = |vertex: u32| {
                        mesh.positions
                            .get(vertex as usize)
                            .and_then(|point| on_screen(&projector, world.transform_point3(*point)))
                            .map_or(f32::INFINITY, |(sx, sy)| (sx - x).hypot(sy - y))
                    };
                    let vertex = if distance(a) <= distance(b) { a } else { b };
                    (BTreeSet::from([a, b]), Some(Anchor::Vertex { object: id, vertex }))
                }
                None => (BTreeSet::new(), None),
            },
            SelectMode::Face => match self.face_by_ray(x, y) {
                Some(face) => (
                    mesh.faces.get(face as usize).map(|corners| corners.iter().copied().collect()).unwrap_or_default(),
                    Some(Anchor::Face { object: id, face }),
                ),
                None => (BTreeSet::new(), None),
            },
        }
    }

    /// What a Cmd/Ctrl+click at pixel (`x`, `y`) adds in edit mode: the shortest path from the
    /// element `from` (clicked before) to the one under the pointer, or that one alone.
    pub(crate) fn path_by_ray(&self, from: Option<Anchor>, x: f32, y: f32) -> (BTreeSet<u32>, Option<Anchor>) {
        let (found, anchor) = self.element_by_ray(x, y);
        let Some((_, _, mesh)) = self.mesh_being_edited() else { return (found, anchor) };
        let path = match (from, anchor) {
            (Some(Anchor::Vertex { object, vertex: start }), Some(Anchor::Vertex { object: now, vertex: end })) if object == now => {
                vertex_path(mesh, start, end).map(|path| path.into_iter().collect::<BTreeSet<u32>>())
            }
            (Some(Anchor::Face { object, face: start }), Some(Anchor::Face { object: now, face: end })) if object == now => {
                face_path(mesh, start, end).map(|faces| {
                    let faces: BTreeSet<u32> = faces.into_iter().collect();
                    kition_scene_model::ops::face_vertices(mesh, &faces)
                })
            }
            _ => None,
        };
        let mut all = found;
        all.extend(path.unwrap_or_default());
        (all, anchor)
    }
}

// ------------------------------------------------------------------------- the commands

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

fn position_schema(what: &str) -> Value {
    s::number_pair(what)
}

/// The selection's session commands.
pub fn catalogue(doc: &SceneDocument) -> Vec<KitCommand> {
    let off = disabled(doc);
    let mode = || {
        s::choice(
            "How what is found changes the selection: set (it becomes the selection), extend (added), subtract (taken out); set",
            &["set", "extend", "subtract"],
        )
    };
    let mut commands = vec![
        command(
            "select.circle",
            "Circle Select",
            &[],
            s::of(vec![
                ("position", s::required(position_schema("The circle's middle, a viewport pixel [x, y]"))),
                ("radius", s::number("The circle's radius in viewport pixels (25)")),
                ("mode", mode()),
            ]),
        ),
        command(
            "select.lasso",
            "Lasso Select",
            &[],
            s::of(vec![
                (
                    "points",
                    s::required(
                        json!({ "type": "array", "items": { "type": "array", "items": { "type": "number" } }, "description": "The lasso's outline, viewport pixels [[x, y], …] (at least 3)" }),
                    ),
                ),
                ("mode", mode()),
            ]),
        ),
        command(
            "select.loop",
            "Select Edge Loops",
            &["Edit", "Select"],
            s::of(vec![
                (
                    "position",
                    position_schema("A viewport pixel [x, y] near an edge of the edited mesh; left out, the loops through every selected edge"),
                ),
                ("extend", s::boolean("Add to the selection instead of replacing it")),
            ]),
        ),
        command(
            "select.ring",
            "Select Edge Rings",
            &["Edit", "Select"],
            s::of(vec![
                (
                    "position",
                    position_schema("A viewport pixel [x, y] near an edge of the edited mesh; left out, the rings across every selected edge"),
                ),
                ("extend", s::boolean("Add to the selection instead of replacing it")),
            ]),
        ),
        command(
            "select.linked",
            "Select Linked",
            &["Edit", "Select"],
            s::of(vec![
                ("position", position_schema("A viewport pixel [x, y]: the connected part under it; left out, every part the selection touches")),
                ("deselect", s::boolean("Take the part under `position` out of the selection")),
            ]),
        ),
        command("select.more", "Select More", &["Edit", "Select"], s::of(vec![])),
        command("select.less", "Select Less", &["Edit", "Select"], s::of(vec![])),
        command("view.toggleXray", "Toggle X-Ray", &["View"], s::of(vec![("value", s::boolean("On or off; left out, the other way"))])),
    ];
    for command in &mut commands {
        command.enabled = !off.contains(&command.id);
    }
    commands
}

/// The ones that cannot run now.
pub fn disabled(doc: &SceneDocument) -> Vec<String> {
    let edit = matches!(doc.session.mode, Mode::Edit(_));
    let regions = doc.session.mode == Mode::Object && !regions_elsewhere(doc) || edit;
    let mut ids = Vec::new();
    if !regions {
        ids.extend(["select.circle", "select.lasso"].map(str::to_owned));
    }
    if !edit {
        ids.extend(["select.loop", "select.ring", "select.linked", "select.more", "select.less"].map(str::to_owned));
    }
    ids
}

/// Whether a mode with its own selection is on (an armature's, a curve's, sculpt, ...).
fn regions_elsewhere(doc: &SceneDocument) -> bool {
    let session = &doc.session;
    session.sculpt.is_some() || session.paint.is_some() || session.particle_edit.is_some() || session.shape.is_some() || session.rig.is_some()
}

/// Runs one of the selection's commands; `None` for any other.
pub fn execute(doc: &mut SceneDocument, command: &str, params: &Value) -> Option<Result<Value, String>> {
    Some(match command {
        "select.circle" => circle_select(doc, params),
        "select.lasso" => lasso_select(doc, params),
        "select.loop" => loop_select(doc, params, false),
        "select.ring" => loop_select(doc, params, true),
        "select.linked" => linked_select(doc, params),
        "select.more" => step_select(doc, true),
        "select.less" => step_select(doc, false),
        "view.toggleXray" => toggle_xray(doc, params),
        _ => return None,
    })
}

fn pixel(params: &Value, key: &str) -> Result<Option<(f32, f32)>, String> {
    match params::number_pair(params, key)? {
        Some([x, y]) if x.is_finite() && y.is_finite() && x.abs() <= 1e7 && y.abs() <= 1e7 => Ok(Some((x, y))),
        Some(_) => Err(bad(format!("`{key}` is not a viewport pixel"))),
        None => Ok(None),
    }
}

/// What the selection is now, as the commands answer it.
fn selection_answer(doc: &SceneDocument) -> Value {
    match doc.session.mode {
        Mode::Edit(_) => json!({ "vertices": doc.session.vertices.len() }),
        Mode::Object => json!({ "objects": doc.session.editor.selected.clone(), "active": doc.session.editor.active }),
    }
}

impl SceneDocument {
    /// Takes what `region` finds into the selection as `pick` says (objects or the edited mesh's
    /// elements, by the mode).
    pub(crate) fn select_region(&mut self, region: &Region, pick: Pick) {
        match self.session.mode {
            Mode::Object => {
                let found = self.objects_in_region(region);
                let found = crate::commands_particles::owners(self, found);
                self.take_objects(found, pick);
            }
            Mode::Edit(_) => {
                let found = self.elements_in_region(region);
                pick.vertices(&mut self.session.vertices, found);
            }
        }
        self.touched();
    }

    /// The object selection once `found` is taken in as `pick` says.
    pub(crate) fn take_objects(&mut self, found: Vec<Id>, pick: Pick) {
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
}

fn circle_select(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let (x, y) = pixel(params, "position")?.ok_or_else(|| bad("`position` is required"))?;
    let radius = params::number(params, "radius")?.unwrap_or(CIRCLE_PIXELS as f32);
    if !(radius.is_finite() && radius > 0.0 && radius <= 1e5) {
        return Err(bad("`radius` must be a positive number of pixels"));
    }
    let pick = Pick::parse(params)?;
    if !disabled(doc).iter().all(|id| id != "select.circle") {
        return Err("not-allowed: Circle Select works in object and edit mode".into());
    }
    doc.select_region(&Region::Circle { x, y, radius }, pick);
    Ok(selection_answer(doc))
}

fn lasso_select(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let points = params.get("points").and_then(Value::as_array).ok_or_else(|| bad("`points` is required: [[x, y], …]"))?;
    if points.len() < 3 || points.len() > MAX_LASSO_POINTS {
        return Err(bad(format!("a lasso has 3 to {MAX_LASSO_POINTS} points")));
    }
    let mut outline = Vec::with_capacity(points.len());
    for point in points {
        match point.as_array().map(|pair| pair.iter().map(Value::as_f64).collect::<Vec<_>>()).as_deref() {
            Some([Some(x), Some(y)]) if x.is_finite() && y.is_finite() && x.abs() <= 1e7 && y.abs() <= 1e7 => outline.push((*x as f32, *y as f32)),
            _ => return Err(bad("each lasso point is a viewport pixel [x, y]")),
        }
    }
    let pick = Pick::parse(params)?;
    if !disabled(doc).iter().all(|id| id != "select.lasso") {
        return Err("not-allowed: Lasso Select works in object and edit mode".into());
    }
    doc.select_region(&Region::Lasso(outline), pick);
    Ok(selection_answer(doc))
}

/// `select.loop` and `select.ring`: the loop (in face mode the face loop) or the ring through
/// the edge nearest the pixel.
fn loop_select(doc: &mut SceneDocument, params: &Value, ring: bool) -> Result<Value, String> {
    let at = pixel(params, "position")?;
    let extend = params::boolean(params, "extend")?.unwrap_or(false);
    let Some((_, _, mesh)) = doc.mesh_being_edited() else { return Err("not-allowed: not in edit mode".into()) };
    let Some((x, y)) = at else {
        // From the menu: the loops (or rings) through every selected edge.
        let selected = &doc.session.vertices;
        let edges: Vec<(u32, u32)> = mesh
            .edges()
            .into_iter()
            .filter(|[a, b]| selected.contains(a) && selected.contains(b))
            .take(MAX_MENU_EDGES)
            .map(|[a, b]| (a, b))
            .collect();
        if edges.is_empty() {
            return Err("not-allowed: select an edge first".into());
        }
        let topology = Topology::of(mesh);
        let mut found = BTreeSet::new();
        for edge in edges {
            if ring {
                let (crossed, _) = kition_scene_model::ops::edge_ring(mesh, edge).map_err(|error| format!("failed: {error}"))?;
                found.extend(crossed.into_iter().flat_map(|(a, b)| [a, b]));
            } else {
                found.extend(loop_in(&topology, edge).into_iter().flat_map(|(a, b)| [a, b]));
            }
        }
        doc.session.vertices.extend(found);
        doc.touched();
        return Ok(json!({ "vertices": doc.session.vertices.len() }));
    };
    let Some(edge) = doc.edge_by_ray(x, y) else { return Err("not-found: no edge there".into()) };
    let found: BTreeSet<u32> = if ring {
        let (edges, _) = kition_scene_model::ops::edge_ring(mesh, edge).map_err(|error| format!("failed: {error}"))?;
        edges.into_iter().flat_map(|(a, b)| [a, b]).collect()
    } else if doc.session.select_mode == SelectMode::Face {
        kition_scene_model::ops::face_vertices(mesh, &face_loop(mesh, edge))
    } else {
        edge_loop(mesh, edge).into_iter().flat_map(|(a, b)| [a, b]).collect()
    };
    if extend {
        doc.session.vertices.extend(found);
    } else {
        doc.session.vertices = found;
    }
    doc.touched();
    Ok(json!({ "vertices": doc.session.vertices.len() }))
}

/// `select.linked`: the connected part under a pixel (taken out with `deselect`), or every part
/// the selection touches.
fn linked_select(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let at = pixel(params, "position")?;
    let deselect = params::boolean(params, "deselect")?.unwrap_or(false);
    let Some((_, _, mesh)) = doc.mesh_being_edited() else { return Err("not-allowed: not in edit mode".into()) };
    let seeds: BTreeSet<u32> = match at {
        Some((x, y)) => {
            let (found, _) = doc.element_by_ray(x, y);
            let found = if found.is_empty() {
                doc.face_by_ray(x, y)
                    .and_then(|face| mesh.faces.get(face as usize))
                    .map(|corners| corners.iter().copied().collect())
                    .unwrap_or_default()
            } else {
                found
            };
            if found.is_empty() {
                return Err("not-found: nothing of the mesh there".into());
            }
            found
        }
        None => doc.session.vertices.clone(),
    };
    let linked = linked_vertices(mesh, &seeds);
    if deselect {
        doc.session.vertices.retain(|vertex| !linked.contains(vertex));
    } else {
        doc.session.vertices.extend(linked);
    }
    doc.touched();
    Ok(json!({ "vertices": doc.session.vertices.len() }))
}

/// `select.more` and `select.less`.
fn step_select(doc: &mut SceneDocument, more: bool) -> Result<Value, String> {
    let Some((_, _, mesh)) = doc.mesh_being_edited() else { return Err("not-allowed: not in edit mode".into()) };
    let next = if more { grown(mesh, &doc.session.vertices) } else { shrunk(mesh, &doc.session.vertices) };
    doc.session.vertices = next;
    doc.touched();
    Ok(json!({ "vertices": doc.session.vertices.len() }))
}

fn toggle_xray(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let on = params::boolean(params, "value")?.unwrap_or(!doc.session.editor.xray);
    if doc.session.editor.xray != on {
        doc.session.editor.xray = on;
        doc.touched();
    }
    Ok(json!({ "xray": on }))
}
