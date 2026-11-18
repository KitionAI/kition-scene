//! Sculpting: brushes that push, smooth, flatten, grab, and mask a mesh's vertices where they
//! touch, as the reference application's Sculpt Mode does. A stroke is a series of dabs along the
//! pointer's path; each dab works in the object's own space on the vertices within the brush's
//! radius, weighted by the falloff curve, the strength, the pen pressure, and one minus the
//! mask. A [`Stroke`] keeps what the dabs need between them (who neighbours whom, a grid to find
//! vertices fast, the grabbed vertices), built once when the stroke starts.

pub mod dyntopo;
pub mod face_sets;
mod grid;
pub mod remesh;

use std::collections::BTreeSet;

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::error::{Result, SceneError};
use crate::mesh::Mesh;
use crate::proportional::Falloff;

pub use grid::Grid;

/// What a brush does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BrushKind {
    /// Pushes the surface out along its mean normal (in with subtract).
    #[default]
    Draw,
    /// Builds the surface up towards a plane above it, in a square strip.
    ClayStrips,
    /// Moves each vertex towards the mean of its neighbours.
    Smooth,
    /// Drags the vertices under the brush where it first touched with the pointer.
    Grab,
    /// Pushes each vertex out along its own normal.
    Inflate,
    /// Draws a sharp groove and pinches the surface into it.
    Crease,
    /// Moves vertices onto the plane through the brush's area.
    Flatten,
    /// Pulls vertices towards the brush's centre (pushes them away with subtract).
    Pinch,
    /// Pulls the vertices under the brush along with the pointer, picking up new ones as it goes.
    SnakeHook,
    /// Paints the mask, which protects vertices from the other brushes.
    Mask,
    /// Paints a face set: the faces a stroke touches go into one new set.
    DrawFaceSets,
}

impl BrushKind {
    /// Every brush, in menu order.
    pub const ALL: [BrushKind; 11] = [
        BrushKind::Draw,
        BrushKind::ClayStrips,
        BrushKind::Smooth,
        BrushKind::Grab,
        BrushKind::Inflate,
        BrushKind::Crease,
        BrushKind::Flatten,
        BrushKind::Pinch,
        BrushKind::SnakeHook,
        BrushKind::Mask,
        BrushKind::DrawFaceSets,
    ];

    /// The id commands use.
    pub fn id(self) -> &'static str {
        match self {
            BrushKind::Draw => "draw",
            BrushKind::ClayStrips => "clayStrips",
            BrushKind::Smooth => "smooth",
            BrushKind::Grab => "grab",
            BrushKind::Inflate => "inflate",
            BrushKind::Crease => "crease",
            BrushKind::Flatten => "flatten",
            BrushKind::Pinch => "pinch",
            BrushKind::SnakeHook => "snakeHook",
            BrushKind::Mask => "mask",
            BrushKind::DrawFaceSets => "drawFaceSets",
        }
    }

    /// The brush with id `id`.
    pub fn from_id(id: &str) -> Option<BrushKind> {
        BrushKind::ALL.into_iter().find(|kind| kind.id() == id)
    }

    /// The strength a brush starts with in the reference application.
    pub fn default_strength(self) -> f32 {
        match self {
            BrushKind::Grab | BrushKind::SnakeHook | BrushKind::Mask | BrushKind::Crease | BrushKind::DrawFaceSets => 1.0,
            _ => 0.5,
        }
    }

    /// Whether the brush moves vertices with the pointer instead of dabbing in place.
    pub fn drags(self) -> bool {
        matches!(self, BrushKind::Grab | BrushKind::SnakeHook)
    }

    /// Whether dynamic topology refines under the brush (not for brushes that only paint, nor
    /// grab, which holds on to the vertices it took).
    pub fn refines(self) -> bool {
        !matches!(self, BrushKind::Grab | BrushKind::Mask | BrushKind::DrawFaceSets)
    }
}

/// A sculpt brush and its settings. The defaults are the reference application's.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Brush {
    #[serde(default)]
    pub kind: BrushKind,
    /// The radius in screen pixels.
    #[serde(default = "fifty")]
    pub radius: f32,
    /// How strong a dab is at its centre, 0 to 1.
    #[serde(default = "half")]
    pub strength: f32,
    #[serde(default)]
    pub falloff: Falloff,
    /// Only vertices whose normals face the eye.
    #[serde(default)]
    pub front_faces_only: bool,
    /// Smooths after each dab, 0 to 1.
    #[serde(default)]
    pub auto_smooth: f32,
    /// The distance between dabs as a share of the radius, 0.01 to 5 (0.1).
    #[serde(default = "spacing")]
    pub spacing: f32,
    /// The opposite direction: in instead of out, unmask instead of mask, ...
    #[serde(default)]
    pub subtract: bool,
    /// Mirrored dabs across the object's own X, Y, Z (X by default, as in the reference).
    #[serde(default = "x_symmetry")]
    pub symmetry: [bool; 3],
    /// Pen pressure scales the strength.
    #[serde(default = "yes")]
    pub use_pressure: bool,
}

fn fifty() -> f32 {
    50.0
}
fn half() -> f32 {
    0.5
}
fn spacing() -> f32 {
    0.1
}
fn x_symmetry() -> [bool; 3] {
    [true, false, false]
}
fn yes() -> bool {
    true
}

impl Default for Brush {
    fn default() -> Self {
        Self {
            kind: BrushKind::Draw,
            radius: 50.0,
            strength: 0.5,
            falloff: Falloff::Smooth,
            front_faces_only: false,
            auto_smooth: 0.0,
            spacing: 0.1,
            subtract: false,
            symmetry: [true, false, false],
            use_pressure: true,
        }
    }
}

impl Brush {
    /// The brush `kind` with its default strength.
    pub fn of(kind: BrushKind) -> Self {
        Self { kind, strength: kind.default_strength(), ..Self::default() }
    }

    /// Checks the numbers.
    pub fn check(&self) -> Result<()> {
        let unit = |value: f32| value.is_finite() && (0.0..=1.0).contains(&value);
        if !(unit(self.strength) && unit(self.auto_smooth)) {
            return Err(SceneError::params("strength and auto smooth go from 0 to 1"));
        }
        if !(self.radius.is_finite() && (1.0..=2000.0).contains(&self.radius)) {
            return Err(SceneError::params("the radius goes from 1 to 2000 pixels"));
        }
        if !(self.spacing.is_finite() && (0.01..=5.0).contains(&self.spacing)) {
            return Err(SceneError::params("spacing goes from 0.01 to 5 (a share of the radius)"));
        }
        Ok(())
    }
}

/// One dab, in the object's own space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dab {
    /// Where the brush touches the surface.
    pub center: Vec3,
    /// The brush's radius there.
    pub radius: f32,
    /// Strength times pressure.
    pub strength: f32,
    /// Towards the eye (for front faces only, and the grab plane).
    pub toward_eye: Vec3,
    /// How far the pointer has dragged the grabbed vertices since the stroke started (grab), or
    /// since the last dab (snake hook).
    pub drag: Vec3,
}

/// A grabbed vertex: its index, its weight, and where it was when the stroke started.
type Grabbed = (u32, f32, Vec3);

/// What a stroke keeps between its dabs.
#[derive(Debug, Clone)]
pub struct Stroke {
    pub brush: Brush,
    /// Each vertex's neighbours along edges.
    neighbours: Csr,
    /// Each vertex's faces.
    faces: Csr,
    grid: Grid,
    /// How far vertices may have moved since the grid was built.
    moved: f32,
    /// Grab: for each mirrored copy of the brush, each grabbed vertex, its weight, and where it
    /// was when the stroke started.
    grabbed: Vec<(Vec3, Vec<Grabbed>)>,
    /// The previous dab's centre (clay strips' direction).
    last_center: Option<Vec3>,
    /// Vertices any dab touched (to keep the mesh check cheap).
    pub touched: BTreeSet<u32>,
    /// Vertices whose every face is hidden: brushes leave them alone.
    hidden: Vec<bool>,
    /// Dynamic topology, when on (the mesh is then triangles).
    pub dyntopo: Option<dyntopo::Dyntopo>,
    /// The face set a Draw Face Sets stroke paints.
    face_set: u32,
    /// Edges split and collapsed by dynamic topology so far.
    pub refined: (usize, usize),
}

/// Compressed rows: `items[starts[i]..starts[i + 1]]` belong to vertex `i`.
#[derive(Debug, Clone, Default)]
struct Csr {
    starts: Vec<u32>,
    items: Vec<u32>,
}

impl Csr {
    fn build(count: usize, pairs: impl Iterator<Item = (u32, u32)> + Clone) -> Self {
        let mut counts = vec![0u32; count + 1];
        for (from, _) in pairs.clone() {
            if let Some(slot) = counts.get_mut(from as usize + 1) {
                *slot += 1;
            }
        }
        for index in 1..counts.len() {
            let previous = counts.get(index - 1).copied().unwrap_or(0);
            if let Some(slot) = counts.get_mut(index) {
                *slot += previous;
            }
        }
        let mut items = vec![0u32; counts.last().copied().unwrap_or(0) as usize];
        let mut fill = counts.clone();
        for (from, to) in pairs {
            let Some(at) = fill.get_mut(from as usize) else { continue };
            if let Some(slot) = items.get_mut(*at as usize) {
                *slot = to;
            }
            *at += 1;
        }
        Self { starts: counts, items }
    }

    /// Sorts each row and drops repeats, closing the gaps.
    fn dedup_rows(&mut self) {
        let mut items = Vec::with_capacity(self.items.len());
        let mut starts = Vec::with_capacity(self.starts.len());
        starts.push(0u32);
        for row in self.starts.windows(2) {
            let [start, end] = row else { continue };
            let mut part: Vec<u32> = self.items.get(*start as usize..*end as usize).unwrap_or(&[]).to_vec();
            part.sort_unstable();
            part.dedup();
            items.extend(part);
            starts.push(u32::try_from(items.len()).unwrap_or(u32::MAX));
        }
        self.items = items;
        self.starts = starts;
    }

    fn row(&self, index: u32) -> &[u32] {
        let (Some(&start), Some(&end)) = (self.starts.get(index as usize), self.starts.get(index as usize + 1)) else { return &[] };
        self.items.get(start as usize..end as usize).unwrap_or(&[])
    }
}

/// The share of a dab a vertex at `distance` gets.
fn falloff(brush: &Brush, distance: f32, radius: f32) -> f32 {
    crate::proportional::weight(brush.falloff, distance, radius)
}

/// How much one dab of Draw displaces at full strength, as a share of the radius.
const DRAW_DEPTH: f32 = 0.05;

impl Stroke {
    /// Starts a stroke on `mesh` with `brush`; `cell` is about the brush's radius in the
    /// object's own space.
    pub fn new(mesh: &Mesh, brush: Brush, cell: f32) -> Self {
        let (neighbours, faces) = Self::adjacency(mesh);
        Self {
            brush,
            neighbours,
            faces,
            grid: Grid::build(&mesh.positions, cell),
            moved: 0.0,
            grabbed: Vec::new(),
            last_center: None,
            touched: BTreeSet::new(),
            hidden: face_sets::hidden_vertices(mesh),
            dyntopo: None,
            face_set: face_sets::next_id(mesh),
            refined: (0, 0),
        }
    }

    /// Who neighbours whom along edges, and each vertex's faces.
    fn adjacency(mesh: &Mesh) -> (Csr, Csr) {
        let count = mesh.positions.len();
        // Both directions of every edge, row by row; each (short) row is then deduplicated.
        let edge_pairs = mesh
            .faces
            .iter()
            .flat_map(|face| crate::mesh::face_edges(face).filter(|(a, b)| a != b).flat_map(|(a, b)| [(a, b), (b, a)]))
            .chain(mesh.loose_edges.iter().flat_map(|[a, b]| [(*a, *b), (*b, *a)]))
            .collect::<Vec<_>>();
        let mut neighbours = Csr::build(count, edge_pairs.iter().copied());
        neighbours.dedup_rows();
        let face_pairs: Vec<(u32, u32)> =
            mesh.faces.iter().enumerate().flat_map(|(index, face)| face.iter().map(move |vertex| (*vertex, index as u32))).collect();
        let faces = Csr::build(count, face_pairs.iter().copied());
        (neighbours, faces)
    }

    /// Refines the mesh under a dab with dynamic topology, and what the stroke keeps with it.
    fn refine(&mut self, mesh: &mut Mesh, centers: &[Vec3], radius: f32) {
        let Some(settings) = self.dyntopo.filter(|settings| settings.enabled && self.brush.kind.refines()) else { return };
        let longest = settings.edge_length(radius, self.brush.radius);
        let mut changed = false;
        for center in centers {
            let refined = dyntopo::refine(mesh, *center, radius, longest, settings.refine);
            if !refined.changed() {
                continue;
            }
            changed = true;
            self.refined.0 += refined.split;
            self.refined.1 += refined.collapsed;
            if let Some(remap) = refined.remap {
                self.touched = self.touched.iter().filter_map(|vertex| remap.get(*vertex as usize).copied().flatten()).collect();
            }
        }
        if changed {
            let (neighbours, faces) = Self::adjacency(mesh);
            self.neighbours = neighbours;
            self.faces = faces;
            self.grid = Grid::build(&mesh.positions, self.grid.cell());
            self.moved = 0.0;
            self.hidden = face_sets::hidden_vertices(mesh);
        }
    }

    /// The normal at a vertex: the area-weighted mean of its faces' normals.
    fn vertex_normal(&self, mesh: &Mesh, vertex: u32) -> Vec3 {
        let mut sum = Vec3::ZERO;
        for face in self.faces.row(vertex) {
            if let Some(corners) = mesh.faces.get(*face as usize) {
                sum += crate::mesh::newell_vector(&mesh.face_positions(corners));
            }
        }
        sum.normalize_or_zero()
    }

    /// The vertices within `radius` of `center`, with their falloff weights (mask, facing, and
    /// strength not yet applied).
    fn within(&mut self, mesh: &Mesh, center: Vec3, radius: f32) -> Vec<(u32, f32)> {
        if self.moved > self.grid.cell() * 0.5 {
            self.grid = Grid::build(&mesh.positions, self.grid.cell());
            self.moved = 0.0;
        }
        let mut out = Vec::new();
        self.grid.query(center, radius + self.moved, |vertex| {
            let Some(point) = mesh.positions.get(vertex as usize) else { return };
            let distance = point.distance(center);
            if distance <= radius {
                let weight = falloff(&self.brush, distance, radius);
                if weight > 0.0 {
                    out.push((vertex, weight));
                }
            }
        });
        out
    }

    /// Applies one dab (and its mirrored copies) to `mesh`.
    pub fn dab(&mut self, mesh: &mut Mesh, dab: &Dab) {
        if self.dyntopo.is_some() {
            let centers: Vec<Vec3> = self.copies(dab).into_iter().map(|(copy, _)| copy.center).collect();
            self.refine(mesh, &centers, dab.radius);
        }
        for (copy, mirror) in self.copies(dab) {
            self.one_dab(mesh, &copy, mirror);
        }
        self.last_center = Some(dab.center);
    }

    /// The dab and its mirrored copies, each with its mirror.
    fn copies(&self, dab: &Dab) -> Vec<(Dab, Vec3)> {
        let mut out = Vec::new();
        let [mx, my, mz] = self.brush.symmetry;
        for flip_x in if mx { [false, true].as_slice() } else { [false].as_slice() } {
            for flip_y in if my { [false, true].as_slice() } else { [false].as_slice() } {
                for flip_z in if mz { [false, true].as_slice() } else { [false].as_slice() } {
                    let mirror = Vec3::new(if *flip_x { -1.0 } else { 1.0 }, if *flip_y { -1.0 } else { 1.0 }, if *flip_z { -1.0 } else { 1.0 });
                    let copy = Dab { center: dab.center * mirror, toward_eye: dab.toward_eye * mirror, drag: dab.drag * mirror, ..*dab };
                    // A mirrored copy whose centre lands on the original's (on the plane) is
                    // the same dab: skip it.
                    if mirror != Vec3::ONE && copy.center.distance(dab.center) < dab.radius * 0.1 {
                        continue;
                    }
                    out.push((copy, mirror));
                }
            }
        }
        out
    }

    fn one_dab(&mut self, mesh: &mut Mesh, dab: &Dab, mirror: Vec3) {
        let brush = self.brush;
        let sign = if brush.subtract { -1.0 } else { 1.0 };
        let kind = brush.kind;
        // Grab keeps the vertices each copy of the brush took at the first dab.
        if kind == BrushKind::Grab {
            let index = match self.grabbed.iter().position(|(key, _)| *key == mirror) {
                Some(index) => index,
                None => {
                    let taken = self.weighted(mesh, dab);
                    let taken =
                        taken.into_iter().filter_map(|(vertex, weight)| Some((vertex, weight, *mesh.positions.get(vertex as usize)?))).collect();
                    self.grabbed.push((mirror, taken));
                    self.grabbed.len() - 1
                }
            };
            let Some((_, grabbed)) = self.grabbed.get(index) else { return };
            let grabbed = grabbed.clone();
            let mut moved = self.moved;
            for (vertex, weight, start) in &grabbed {
                if let Some(point) = mesh.positions.get_mut(*vertex as usize) {
                    let target = *start + dab.drag * *weight;
                    moved = moved.max(point.distance(target));
                    *point = target;
                    self.touched.insert(*vertex);
                }
            }
            self.moved = moved;
            return;
        }
        let touched = self.weighted(mesh, dab);
        if touched.is_empty() {
            return;
        }
        if kind == BrushKind::DrawFaceSets {
            face_sets::tidy(mesh);
            if mesh.face_sets.len() != mesh.faces.len() {
                mesh.face_sets = vec![0; mesh.faces.len()];
            }
            for (vertex, _) in &touched {
                for face in self.faces.row(*vertex) {
                    if let Some(set) = mesh.face_sets.get_mut(*face as usize) {
                        *set = self.face_set;
                    }
                }
                self.touched.insert(*vertex);
            }
            return;
        }
        if kind == BrushKind::Mask {
            if mesh.mask.len() != mesh.positions.len() {
                mesh.mask = vec![0.0; mesh.positions.len()];
            }
            for (vertex, weight) in &touched {
                if let Some(value) = mesh.mask.get_mut(*vertex as usize) {
                    *value = (*value + sign * weight).clamp(0.0, 1.0);
                }
            }
            return;
        }
        // The area under the brush: its weighted centre and normal.
        let mut area_center = Vec3::ZERO;
        let mut area_normal = Vec3::ZERO;
        let mut total = 0.0;
        for (vertex, weight) in &touched {
            if let Some(point) = mesh.positions.get(*vertex as usize) {
                area_center += *point * *weight;
                area_normal += self.vertex_normal(mesh, *vertex) * *weight;
                total += *weight;
            }
        }
        if total <= 0.0 {
            return;
        }
        area_center /= total;
        let area_normal = area_normal.normalize_or_zero();
        let depth = dab.radius * DRAW_DEPTH;
        // Clay strips work in a square aligned with the stroke.
        let along = self.last_center.map(|last| dab.center - last).filter(|step| step.length_squared() > 1e-12).unwrap_or(Vec3::X);
        let side_u = (along - area_normal * along.dot(area_normal)).normalize_or_zero();
        let side_u = if side_u == Vec3::ZERO { area_normal.any_orthonormal_vector() } else { side_u };
        let side_v = area_normal.cross(side_u);
        let moves: Vec<(u32, Vec3)> = touched
            .iter()
            .filter_map(|(vertex, weight)| {
                let point = *mesh.positions.get(*vertex as usize)?;
                let offset = match kind {
                    BrushKind::Draw => area_normal * (sign * weight * depth),
                    BrushKind::Inflate => self.vertex_normal(mesh, *vertex) * (sign * weight * depth),
                    BrushKind::Smooth => (self.neighbour_mean(mesh, *vertex)? - point) * (weight * if brush.subtract { -0.5 } else { 1.0 }),
                    BrushKind::Flatten => {
                        let height = (point - area_center).dot(area_normal);
                        -area_normal * height * weight
                    }
                    BrushKind::ClayStrips => {
                        // A square footprint: the farther side measured, not the distance.
                        let local = point - dab.center;
                        let square = local.dot(side_u).abs().max(local.dot(side_v).abs());
                        let weight = falloff(&brush, square, dab.radius) * dab.strength;
                        let plane = area_center + area_normal * (sign * depth * 2.0);
                        let height = (point - plane).dot(area_normal) * sign;
                        // Only what lies under the plane rises to it.
                        if height >= 0.0 {
                            return None;
                        }
                        -area_normal * (height * sign) * weight
                    }
                    BrushKind::Crease => {
                        let groove = area_normal * (sign * weight * depth);
                        let to_center = dab.center - point;
                        let flat = to_center - area_normal * to_center.dot(area_normal);
                        groove + flat * (weight * 0.25)
                    }
                    BrushKind::Pinch => {
                        let to_center = dab.center - point;
                        let flat = to_center - area_normal * to_center.dot(area_normal);
                        flat * (sign * weight * 0.5)
                    }
                    BrushKind::SnakeHook => dab.drag * weight,
                    BrushKind::Grab | BrushKind::Mask | BrushKind::DrawFaceSets => return None,
                };
                Some((*vertex, offset))
            })
            .collect();
        for (vertex, offset) in &moves {
            if let Some(point) = mesh.positions.get_mut(*vertex as usize) {
                *point += *offset;
                self.moved = self.moved.max(offset.length());
                self.touched.insert(*vertex);
            }
        }
        if brush.auto_smooth > 0.0 && kind != BrushKind::Smooth {
            let smoothed: Vec<(u32, Vec3)> = touched
                .iter()
                .filter_map(|(vertex, weight)| {
                    let point = *mesh.positions.get(*vertex as usize)?;
                    let mean = self.neighbour_mean(mesh, *vertex)?;
                    Some((*vertex, (mean - point) * (weight / dab.strength.max(1e-6) * brush.auto_smooth).min(1.0)))
                })
                .collect();
            for (vertex, offset) in smoothed {
                if let Some(point) = mesh.positions.get_mut(vertex as usize) {
                    *point += offset;
                }
            }
        }
    }

    /// The vertices under a dab with their full weights: falloff, strength, one minus the mask,
    /// and zero for back faces when the brush takes front faces only.
    fn weighted(&mut self, mesh: &Mesh, dab: &Dab) -> Vec<(u32, f32)> {
        let found = self.within(mesh, dab.center, dab.radius);
        found
            .into_iter()
            .filter_map(|(vertex, weight)| {
                if self.hidden.get(vertex as usize).copied().unwrap_or(false) {
                    return None;
                }
                if self.brush.front_faces_only && self.vertex_normal(mesh, vertex).dot(dab.toward_eye) <= 0.0 {
                    return None;
                }
                let mask = if self.brush.kind == BrushKind::Mask { 0.0 } else { mesh.mask.get(vertex as usize).copied().unwrap_or(0.0) };
                let weight = weight * dab.strength * (1.0 - mask);
                (weight > 0.0).then_some((vertex, weight))
            })
            .collect()
    }

    fn neighbour_mean(&self, mesh: &Mesh, vertex: u32) -> Option<Vec3> {
        let row = self.neighbours.row(vertex);
        if row.is_empty() {
            return None;
        }
        let sum = row.iter().filter_map(|other| mesh.positions.get(*other as usize)).copied().sum::<Vec3>();
        Some(sum / row.len() as f32)
    }
}

/// What a mask command does to the whole mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskAction {
    Invert,
    Clear,
    Fill,
    Smooth,
}

/// Runs a mask command on the whole mesh.
pub fn mask(mesh: &mut Mesh, action: MaskAction) {
    let count = mesh.positions.len();
    match action {
        MaskAction::Clear => mesh.mask.clear(),
        MaskAction::Fill => mesh.mask = vec![1.0; count],
        MaskAction::Invert => {
            if mesh.mask.len() != count {
                mesh.mask = vec![0.0; count];
            }
            for value in &mut mesh.mask {
                *value = 1.0 - *value;
            }
        }
        MaskAction::Smooth => {
            if mesh.mask.len() != count {
                return;
            }
            let stroke = Stroke::new(mesh, Brush::default(), 1.0);
            let next: Vec<f32> = (0..count as u32)
                .map(|vertex| {
                    let row = stroke.neighbours.row(vertex);
                    let own = mesh.mask.get(vertex as usize).copied().unwrap_or(0.0);
                    if row.is_empty() {
                        return own;
                    }
                    let mean = row.iter().filter_map(|other| mesh.mask.get(*other as usize)).sum::<f32>() / row.len() as f32;
                    (own + mean) * 0.5
                })
                .collect();
            mesh.mask = next;
        }
    }
    if mesh.mask.iter().all(|value| *value == 0.0) {
        mesh.mask.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::Primitive;

    fn plane() -> Mesh {
        Primitive::Grid { x_subdivisions: 40, y_subdivisions: 40, size: 2.0 }.build().unwrap()
    }

    fn dab_at(center: Vec3, radius: f32) -> Dab {
        Dab { center, radius, strength: 1.0, toward_eye: Vec3::Z, drag: Vec3::ZERO }
    }

    #[test]
    fn draw_raises_the_middle_and_leaves_the_rest() {
        let mut mesh = plane();
        let mut stroke = Stroke::new(&mesh, Brush { symmetry: [false; 3], ..Brush::of(BrushKind::Draw) }, 0.3);
        stroke.dab(&mut mesh, &dab_at(Vec3::ZERO, 0.3));
        let middle = mesh.positions.iter().map(|point| point.z).fold(f32::MIN, f32::max);
        assert!(middle > 0.0, "{middle}");
        assert!(mesh.positions.iter().filter(|point| point.truncate().length() > 0.31).all(|point| point.z == 0.0));
        let mut down = plane();
        let mut stroke = Stroke::new(&down, Brush { subtract: true, symmetry: [false; 3], ..Brush::of(BrushKind::Draw) }, 0.3);
        stroke.dab(&mut down, &dab_at(Vec3::ZERO, 0.3));
        assert!(down.positions.iter().map(|point| point.z).fold(f32::MAX, f32::min) < 0.0);
    }

    #[test]
    fn the_mask_protects() {
        let mut mesh = plane();
        mask(&mut mesh, MaskAction::Fill);
        let mut stroke = Stroke::new(&mesh, Brush { symmetry: [false; 3], ..Brush::of(BrushKind::Draw) }, 0.3);
        stroke.dab(&mut mesh, &dab_at(Vec3::ZERO, 0.3));
        assert!(mesh.positions.iter().all(|point| point.z == 0.0));
        mask(&mut mesh, MaskAction::Invert);
        assert!(mesh.mask.is_empty(), "an all-zero mask is no mask");
    }

    #[test]
    fn symmetry_mirrors_across_x() {
        let mut mesh = plane();
        let mut stroke = Stroke::new(&mesh, Brush::of(BrushKind::Draw), 0.3);
        stroke.dab(&mut mesh, &dab_at(Vec3::new(0.5, 0.0, 0.0), 0.2));
        let at = |x: f32| mesh.positions.iter().find(|point| (point.x - x).abs() < 1e-4 && point.y.abs() < 1e-4).unwrap().z;
        assert!(at(0.5) > 0.0);
        assert!((at(0.5) - at(-0.5)).abs() < 1e-6);
    }

    #[test]
    fn grab_moves_what_it_took() {
        let mut mesh = plane();
        let mut stroke = Stroke::new(&mesh, Brush { symmetry: [false; 3], ..Brush::of(BrushKind::Grab) }, 0.3);
        stroke.dab(&mut mesh, &dab_at(Vec3::ZERO, 0.3));
        stroke.dab(&mut mesh, &Dab { drag: Vec3::new(0.0, 0.0, 0.5), ..dab_at(Vec3::ZERO, 0.3) });
        let top = mesh.positions.iter().map(|point| point.z).fold(f32::MIN, f32::max);
        assert!((top - 0.5).abs() < 1e-5, "{top}");
    }

    #[test]
    fn smooth_and_flatten_take_a_bump_down() {
        let mut mesh = plane();
        let mut stroke = Stroke::new(&mesh, Brush { strength: 1.0, symmetry: [false; 3], ..Brush::of(BrushKind::Draw) }, 0.3);
        for _ in 0..10 {
            stroke.dab(&mut mesh, &dab_at(Vec3::ZERO, 0.3));
        }
        let peak = |mesh: &Mesh| mesh.positions.iter().map(|point| point.z).fold(f32::MIN, f32::max);
        let before = peak(&mesh);
        for kind in [BrushKind::Smooth, BrushKind::Flatten] {
            let mut copy = mesh.clone();
            let mut stroke = Stroke::new(&copy, Brush { strength: 1.0, symmetry: [false; 3], ..Brush::of(kind) }, 0.3);
            for _ in 0..5 {
                stroke.dab(&mut copy, &dab_at(Vec3::ZERO, 0.3));
            }
            assert!(peak(&copy) < before, "{kind:?}: {} vs {before}", peak(&copy));
        }
    }

    #[test]
    fn dynamic_topology_adds_detail_where_the_brush_goes() {
        let mut mesh = Primitive::UvSphere { segments: 16, rings: 8, radius: 1.0 }.build().unwrap();
        dyntopo::triangulate(&mut mesh);
        let before = mesh.faces.len();
        let mut stroke = Stroke::new(&mesh, Brush { symmetry: [false; 3], ..Brush::of(BrushKind::Draw) }, 0.3);
        stroke.dyntopo = Some(dyntopo::Dyntopo { enabled: true, mode: dyntopo::DetailMode::Brush, ..Default::default() });
        for step in 0..6 {
            let center = Vec3::new(step as f32 * 0.05, -1.0, 0.0);
            stroke.dab(&mut mesh, &Dab { center, radius: 0.4, strength: 1.0, toward_eye: Vec3::NEG_Y, drag: Vec3::ZERO });
        }
        mesh.validate().unwrap();
        assert!(mesh.faces.len() > before * 2, "{} faces from {before}", mesh.faces.len());
        assert!(stroke.refined.0 > 0);
        assert!(mesh.positions.iter().map(|point| -point.y).fold(f32::MIN, f32::max) > 1.0, "the new detail is drawn out");
    }

    #[test]
    fn draw_face_sets_paints_one_new_set_and_hidden_faces_stay() {
        let mut mesh = plane();
        let mut stroke = Stroke::new(&mesh, Brush { symmetry: [false; 3], ..Brush::of(BrushKind::DrawFaceSets) }, 0.3);
        stroke.dab(&mut mesh, &dab_at(Vec3::ZERO, 0.3));
        let ids = face_sets::ids(&mesh);
        assert_eq!(ids.into_iter().collect::<Vec<_>>(), vec![1]);
        face_sets::visibility(&mut mesh, 1, face_sets::Visibility::Isolate);
        let mut stroke = Stroke::new(&mesh, Brush { symmetry: [false; 3], ..Brush::of(BrushKind::Draw) }, 0.3);
        stroke.dab(&mut mesh, &dab_at(Vec3::new(0.8, 0.8, 0.0), 0.15));
        assert!(mesh.positions.iter().all(|point| point.z == 0.0), "hidden vertices stay");
    }

    #[test]
    fn every_brush_keeps_the_mesh_valid() {
        for kind in BrushKind::ALL {
            let mut mesh = Primitive::UvSphere { segments: 24, rings: 12, radius: 1.0 }.build().unwrap();
            let mut stroke = Stroke::new(&mesh, Brush::of(kind), 0.4);
            for step in 0..8 {
                let center = Vec3::new(0.0, -1.0, 0.0) + Vec3::X * (step as f32 * 0.05);
                stroke.dab(&mut mesh, &Dab { center, radius: 0.4, strength: 1.0, toward_eye: Vec3::NEG_Y, drag: Vec3::new(0.0, -0.05, 0.0) });
            }
            mesh.validate().unwrap();
            assert!(mesh.positions.iter().all(|point| point.is_finite()), "{kind:?}");
        }
    }
}
