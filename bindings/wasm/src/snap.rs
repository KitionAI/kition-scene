//! Snapping a move to the scene's elements: the nearest vertex, the nearest point of an edge, or
//! the surface under the pointer, within a short reach of the pointer on the screen and only what
//! shows (in wireframe, what is behind a surface too). What it may snap to is found once, the
//! first time the move snaps (and again after the view changes): the visible meshes' vertices put
//! on the screen and into a grid of cells, so each pointer step looks at a few cells only. What the
//! transform moves is left out. The point of the moved selection that snaps is the one nearest the
//! target: in object mode the corners of the moved objects' boxes (an object without a mesh, its
//! origin), in edit mode the selected vertices.

use std::collections::{BTreeSet, HashMap};

use glam::Vec3;
use kition_scene_model::Id;
use kition_scene_raster::{Projector, Shading, ViewState};

use crate::SceneDocument;
use crate::cursor::SnapTarget;
use crate::ray::RayScene;

/// How near the pointer, in viewport pixels, a target must be.
const REACH: f32 = 30.0;
/// The grid's cell, in viewport pixels.
const CELL: f32 = 32.0;
/// The most points and edges looked at (the rest of a bigger scene is left out).
const MAX_POINTS: usize = 3_000_000;
const MAX_EDGES: usize = 3_000_000;
/// The most cells one edge is put in.
const MAX_CELLS: i64 = 2048;
/// The most points of the selection the snapping point is chosen from (more: the pivot).
const MAX_SOURCES: usize = 20_000;
/// The nearest candidates checked for whether they show, nearest first.
const MAX_CHECKS: usize = 8;

/// What a move may snap to, for one view.
pub(crate) struct SnapIndex {
    target: SnapTarget,
    view: (ViewState, u32, u32),
    /// Candidate points (world) and where they are on the screen.
    points: Vec<Vec3>,
    screen: Vec<[f32; 2]>,
    /// Candidate edges, as indices into `points`.
    edges: Vec<[u32; 2]>,
    /// Cell to the points (or with the edge target, the edges) in it.
    grid: HashMap<(i32, i32), Vec<u32>>,
    /// The moved selection's points (world, as it began), one of which snaps onto the target.
    sources: Vec<Vec3>,
    /// The surfaces that hide what is behind them, and the face target's surfaces.
    surfaces: RayScene,
    /// Wireframe: nothing hides anything.
    xray: bool,
}

impl std::fmt::Debug for SnapIndex {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SnapIndex")
            .field("target", &self.target)
            .field("points", &self.points.len())
            .field("edges", &self.edges.len())
            .finish()
    }
}

fn cell_of(at: [f32; 2]) -> (i32, i32) {
    ((at[0] / CELL).floor() as i32, (at[1] / CELL).floor() as i32)
}

/// The distance from `p` to the segment `a`–`b` on the screen, and how far along it (0 to 1).
fn to_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> (f32, f32) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = dx * dx + dy * dy;
    let t = if length > 1e-9 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length).clamp(0.0, 1.0) } else { 0.0 };
    let (x, y) = (a[0] + dx * t, a[1] + dy * t);
    ((p[0] - x).hypot(p[1] - y), t)
}

/// The point of segment `a`–`b` nearest the line from `origin` along `direction`.
fn nearest_on_segment(origin: Vec3, direction: Vec3, a: Vec3, b: Vec3) -> Vec3 {
    let along = b - a;
    let between = a - origin;
    let (aa, ad, dd) = (along.dot(along), along.dot(direction), direction.dot(direction));
    let (ab, db) = (along.dot(between), direction.dot(between));
    let denominator = aa * dd - ad * ad;
    let t = if denominator.abs() > 1e-12 { ((ad * db - dd * ab) / denominator).clamp(0.0, 1.0) } else { 0.0 };
    a + along * t
}

impl SnapIndex {
    /// Whether it was made for this view.
    pub(crate) fn fits(&self, view: &(ViewState, u32, u32)) -> bool {
        self.view == *view
    }

    /// The target nearest the pointer `at` that shows, and the selection's point (as it began)
    /// that snaps onto it: the one nearest the target once moved by `offset` (world).
    pub(crate) fn find(&self, projector: &Projector, at: (f64, f64), offset: Vec3) -> Option<(Vec3, Vec3)> {
        let pointer = [at.0 as f32, at.1 as f32];
        if !(pointer[0].is_finite() && pointer[1].is_finite()) {
            return None;
        }
        let target = match self.target {
            SnapTarget::Increment => return None,
            SnapTarget::Face => self.surfaces.at(projector, pointer[0], pointer[1]).map(|hit| hit.point)?,
            SnapTarget::Vertex => {
                let mut near: Vec<(f32, u32)> = self
                    .nearby(pointer)
                    .filter_map(|index| {
                        let screen = self.screen.get(index as usize)?;
                        let distance = (screen[0] - pointer[0]).hypot(screen[1] - pointer[1]);
                        (distance <= REACH).then_some((distance, index))
                    })
                    .collect();
                near.sort_by(|a, b| a.0.total_cmp(&b.0));
                near.iter()
                    .take(MAX_CHECKS)
                    .filter_map(|(_, index)| self.points.get(*index as usize).copied())
                    .find(|point| self.shows(projector, *point))?
            }
            SnapTarget::Edge => {
                let mut seen = BTreeSet::new();
                let mut near: Vec<(f32, u32)> = self
                    .nearby(pointer)
                    .filter(|index| seen.insert(*index))
                    .filter_map(|index| {
                        let [a, b] = *self.edges.get(index as usize)?;
                        let (sa, sb) = (self.screen.get(a as usize)?, self.screen.get(b as usize)?);
                        let (distance, _) = to_segment(pointer, *sa, *sb);
                        (distance <= REACH).then_some((distance, index))
                    })
                    .collect();
                near.sort_by(|a, b| a.0.total_cmp(&b.0));
                let (origin, direction) = projector.ray(pointer[0], pointer[1]);
                near.iter()
                    .take(MAX_CHECKS)
                    .filter_map(|(_, index)| {
                        let [a, b] = *self.edges.get(*index as usize)?;
                        Some(nearest_on_segment(origin, direction, *self.points.get(a as usize)?, *self.points.get(b as usize)?))
                    })
                    .find(|point| point.is_finite() && self.shows(projector, *point))?
            }
        };
        let source = self
            .sources
            .iter()
            .copied()
            .min_by(|a, b| (*a + offset - target).length_squared().total_cmp(&(*b + offset - target).length_squared()))?;
        Some((target, source))
    }

    /// What the cells round `pointer` hold.
    fn nearby(&self, pointer: [f32; 2]) -> impl Iterator<Item = u32> + '_ {
        let (cx, cy) = cell_of(pointer);
        let reach = (REACH / CELL).ceil() as i32;
        (cy - reach..=cy + reach)
            .flat_map(move |y| (cx - reach..=cx + reach).map(move |x| (x, y)))
            .filter_map(|cell| self.grid.get(&cell))
            .flatten()
            .copied()
    }

    fn shows(&self, projector: &Projector, point: Vec3) -> bool {
        self.xray || self.surfaces.is_empty() || self.surfaces.shows(projector, point)
    }
}

impl SceneDocument {
    /// What the move under way may snap to now (see the module's description).
    pub(crate) fn snap_index(&self, modal: &crate::modal::Modal) -> SnapIndex {
        let projector = self.projector();
        let view = (self.session.editor.view.clone(), self.session.width, self.session.height);
        let target = modal.snap_target();
        let (moved, edited, kept_out) = modal.moved_parts();
        // The moved objects and what hangs from them move.
        let moved: BTreeSet<Id> = self
            .scene
            .objects
            .iter()
            .map(|object| object.id)
            .filter(|id| {
                let mut at = Some(*id);
                let mut depth = 0;
                while let Some(current) = at {
                    if moved.contains(&current) {
                        return true;
                    }
                    at = self.scene.object(current).ok().and_then(|object| object.parent);
                    depth += 1;
                    if depth > 256 {
                        break;
                    }
                }
                false
            })
            .collect();
        let meshes = self.ray_meshes();
        let (width, height) = (self.session.width as f32, self.session.height as f32);
        let mut points = Vec::new();
        let mut screen = Vec::new();
        let mut edges = Vec::new();
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        let view_projection = projector.view_projection();
        if matches!(target, SnapTarget::Vertex | SnapTarget::Edge) {
            for (id, prepared, world) in &meshes {
                if moved.contains(id) || points.len() >= MAX_POINTS {
                    continue;
                }
                let own_mesh = edited == Some(*id);
                let mut slot = vec![u32::MAX; prepared.mesh.positions.len()];
                for (vertex, local) in prepared.mesh.positions.iter().enumerate() {
                    if points.len() >= MAX_POINTS {
                        break;
                    }
                    if own_mesh && u32::try_from(vertex).is_ok_and(|vertex| kept_out.contains(&vertex)) {
                        continue;
                    }
                    let point = world.transform_point3(*local);
                    let clip = view_projection * point.extend(1.0);
                    if clip.w <= 1e-6 || clip.w.is_nan() || !point.is_finite() {
                        continue;
                    }
                    let at = projector.to_screen(clip);
                    let Ok(index) = u32::try_from(points.len()) else { break };
                    if let Some(entry) = slot.get_mut(vertex) {
                        *entry = index;
                    }
                    points.push(point);
                    screen.push([at.x, at.y]);
                    let on_screen = at.x >= -REACH && at.y >= -REACH && at.x <= width + REACH && at.y <= height + REACH;
                    if target == SnapTarget::Vertex && on_screen {
                        grid.entry(cell_of([at.x, at.y])).or_default().push(index);
                    }
                }
                if target == SnapTarget::Edge {
                    for [a, b] in prepared.mesh.edges() {
                        if edges.len() >= MAX_EDGES {
                            break;
                        }
                        let (Some(&ia), Some(&ib)) = (slot.get(a as usize), slot.get(b as usize)) else { continue };
                        if ia == u32::MAX || ib == u32::MAX {
                            continue;
                        }
                        let (Some(sa), Some(sb)) = (screen.get(ia as usize).copied(), screen.get(ib as usize).copied()) else { continue };
                        let low = [sa[0].min(sb[0]).max(-REACH), sa[1].min(sb[1]).max(-REACH)];
                        let high = [sa[0].max(sb[0]).min(width + REACH), sa[1].max(sb[1]).min(height + REACH)];
                        if low[0] > high[0] || low[1] > high[1] {
                            continue;
                        }
                        let (c0, c1) = (cell_of(low), cell_of(high));
                        let cells = (i64::from(c1.0) - i64::from(c0.0) + 1) * (i64::from(c1.1) - i64::from(c0.1) + 1);
                        if cells > MAX_CELLS {
                            continue;
                        }
                        let Ok(index) = u32::try_from(edges.len()) else { break };
                        edges.push([ia, ib]);
                        for y in c0.1..=c1.1 {
                            for x in c0.0..=c1.0 {
                                grid.entry((x, y)).or_default().push(index);
                            }
                        }
                    }
                }
            }
        }
        let sources = self.snap_sources(modal, &meshes);
        let skip_edited = edited;
        let surfaces = self.ray_scene(&|id| moved.contains(&id) || Some(id) == skip_edited);
        SnapIndex { target, view, points, screen, edges, grid, sources, surfaces, xray: self.session.editor.shading == Shading::Wireframe }
    }

    /// The moved selection's points one of which snaps (see the module's description).
    fn snap_sources(&self, modal: &crate::modal::Modal, meshes: &[(Id, std::sync::Arc<kition_scene_raster::Prepared>, glam::Mat4)]) -> Vec<Vec3> {
        let mut sources = Vec::new();
        match modal.snap_items() {
            crate::modal::SnapItems::Objects(ids) => {
                for id in ids {
                    let Ok(world) = self.scene.world_matrix(id) else { continue };
                    match meshes.iter().find(|(drawn, ..)| *drawn == id) {
                        Some((_, prepared, _)) if !prepared.mesh.positions.is_empty() => {
                            let bounds = prepared.mesh.bounds(glam::Mat4::IDENTITY);
                            for corner in 0..8 {
                                let pick = |bit: usize, low: f32, high: f32| if corner & bit == 0 { low } else { high };
                                let local = Vec3::new(
                                    pick(1, bounds.min.x, bounds.max.x),
                                    pick(2, bounds.min.y, bounds.max.y),
                                    pick(4, bounds.min.z, bounds.max.z),
                                );
                                sources.push(world.transform_point3(local));
                            }
                        }
                        _ => sources.push(world.transform_point3(Vec3::ZERO)),
                    }
                    if sources.len() > MAX_SOURCES {
                        break;
                    }
                }
            }
            crate::modal::SnapItems::Vertices(object, vertices) => {
                if vertices.len() <= MAX_SOURCES
                    && let (Ok(world), Some(mesh)) =
                        (self.scene.world_matrix(object), self.scene.object(object).ok().and_then(|object| object.mesh()))
                {
                    sources.extend(
                        vertices.iter().filter_map(|vertex| mesh.positions.get(*vertex as usize)).map(|point| world.transform_point3(*point)),
                    );
                }
            }
            crate::modal::SnapItems::Other => {}
        }
        if sources.is_empty() || sources.len() > MAX_SOURCES {
            sources = vec![modal.pivot_point()];
        }
        sources
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_segment_is_measured_to_its_nearest_point() {
        let (distance, along) = to_segment([5.0, 3.0], [0.0, 0.0], [10.0, 0.0]);
        assert!((distance - 3.0).abs() < 1e-6 && (along - 0.5).abs() < 1e-6);
        let (beyond, end) = to_segment([14.0, 3.0], [0.0, 0.0], [10.0, 0.0]);
        assert!((beyond - 5.0).abs() < 1e-6 && (end - 1.0).abs() < 1e-6);
    }

    #[test]
    fn the_nearest_point_of_an_edge_to_a_ray() {
        let point = nearest_on_segment(Vec3::new(0.3, 0.0, 5.0), Vec3::NEG_Z, Vec3::ZERO, Vec3::X);
        assert!((point - Vec3::new(0.3, 0.0, 0.0)).length() < 1e-6);
    }
}
