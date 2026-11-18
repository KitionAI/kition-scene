//! Unwrapping by seams: the chosen faces fall into charts (cut apart along seams), each chart is
//! flattened (angle-based, as a modeller unwraps by default: the angles a flattening can keep are
//! solved first and the coordinates rebuilt from them; or least-squares conformal maps, LSCM,
//! straight from the surface's triangles; two vertices pinned either way), scaled to its surface
//! area, turned to its smallest box, and the charts are packed into the unit square with a
//! margin.

use std::collections::{BTreeSet, HashMap};

use glam::{DVec2, DVec3, Vec2};

use crate::error::{Result, SceneError};
use crate::mesh::{Mesh, face_edges};

/// One unwrapped chart: its faces, and a texture coordinate per (face, corner).
#[derive(Debug, Clone)]
struct Chart {
    faces: Vec<usize>,
    /// Chart vertex of each (face, corner).
    corner_vertex: HashMap<(usize, usize), usize>,
    /// The mesh vertex each chart vertex came from.
    source: Vec<u32>,
    uv: Vec<DVec2>,
}

/// How charts are flattened.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Method {
    /// Angle-based flattening (`crate::abf`), then LSCM on the solved angles.
    #[default]
    AngleBased,
    /// LSCM on the surface's own triangles.
    Conformal,
    /// SLIM minimizing symmetric Dirichlet distortion (angles and areas).
    MinimumStretch,
}

/// What an unwrap did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unwrapped {
    pub charts: usize,
    pub faces: usize,
}

/// The largest number of chart vertices one unwrap solves.
const MAX_VERTICES: usize = 200_000;

/// Unwraps `faces` (every face when empty) by the mesh's seams, packing the charts into the unit
/// square with `margin` (a fraction of it) between them. Faces outside the set keep their
/// coordinates.
pub fn unwrap(mesh: &mut Mesh, faces: &BTreeSet<u32>, margin: f32) -> Result<Unwrapped> {
    unwrap_with(mesh, faces, margin, Method::Conformal)
}

/// [`unwrap`] with the flattening `method`.
pub fn unwrap_with(mesh: &mut Mesh, faces: &BTreeSet<u32>, margin: f32, method: Method) -> Result<Unwrapped> {
    if method == Method::MinimumStretch {
        return minimum_stretch(mesh, faces, margin, 30);
    }
    if !margin.is_finite() || !(0.0..0.5).contains(&margin) {
        return Err(SceneError::params("the margin must be between 0 and 0.5"));
    }
    let chosen: Vec<usize> = if faces.is_empty() { (0..mesh.faces.len()).collect() } else { faces.iter().map(|face| *face as usize).collect() };
    if let Some(bad) = chosen.iter().find(|face| **face >= mesh.faces.len()) {
        return Err(SceneError::not_found(format!("face {bad}")));
    }
    if chosen.is_empty() {
        return Err(SceneError::params("there are no faces to unwrap"));
    }
    let mut charts = charts(mesh, &chosen)?;
    for chart in &mut charts {
        flatten(mesh, chart, method == Method::AngleBased);
        fit_area(mesh, chart);
        turn_to_smallest_box(chart);
    }
    pack(&mut charts, f64::from(margin));
    if !mesh.has_uvs() || !mesh.uvs_aligned() {
        mesh.uvs.clear();
        mesh.fill_uvs();
    }
    for chart in &charts {
        for (&(face, corner), &vertex) in &chart.corner_vertex {
            if let (Some(slot), Some(uv)) = (mesh.uvs.get_mut(face).and_then(|uvs| uvs.get_mut(corner)), chart.uv.get(vertex)) {
                *slot = Vec2::new(uv.x as f32, uv.y as f32);
            }
        }
    }
    Ok(Unwrapped { charts: charts.len(), faces: chosen.len() })
}

/// One face's use of an edge: the face, the edge's first vertex, and the corners at its ends.
type EdgeUse = (usize, u32, usize, usize);

/// Minimum-stretch unwrap with a bounded iteration count. Pinned charts stay in their existing
/// coordinate system; only free charts are packed. All chart solutions are validated before any
/// corner is changed, including when a later chart is degenerate.
pub fn minimum_stretch(mesh: &mut Mesh, faces: &BTreeSet<u32>, margin: f32, iterations: u32) -> Result<Unwrapped> {
    if !margin.is_finite() || !(0.0..0.5).contains(&margin) || !(1..=100).contains(&iterations) {
        return Err(SceneError::params("minimum stretch needs a margin in [0, 0.5) and 1 to 100 iterations"));
    }
    mesh.validate()?;
    let chosen: Vec<usize> = if faces.is_empty() { (0..mesh.faces.len()).collect() } else { faces.iter().map(|face| *face as usize).collect() };
    if chosen.is_empty() || chosen.iter().any(|face| *face >= mesh.faces.len()) {
        return Err(SceneError::params("minimum stretch needs valid selected faces"));
    }
    check_live_size(mesh, &chosen)?;
    let mut charts = charts(mesh, &chosen)?;
    check_chart_size(&charts)?;
    let mut free = Vec::new();
    for (index, chart) in charts.iter_mut().enumerate() {
        let mut points: Vec<DVec3> = chart.source.iter().map(|vertex| position(mesh, *vertex)).collect();
        let triangles = crate::abf::chart_triangles(mesh, &chart.faces, &chart.corner_vertex);
        let pins = chart_pins(mesh, chart)?;
        if pins.len() == 1 {
            // One pin fixes translation but leaves UV scale free. Keep the current chart's
            // area as its frame rather than allowing a change of model units to resize it.
            let current = chart_coordinates(mesh, chart);
            let (mut flat, mut surface) = (0.0, 0.0);
            for &[a, b, c] in &triangles {
                if let (Some(pa), Some(pb), Some(pc), Some(ua), Some(ub), Some(uc)) =
                    (points.get(a), points.get(b), points.get(c), current.get(a), current.get(b), current.get(c))
                {
                    surface += (*pb - *pa).cross(*pc - *pa).length();
                    flat += (*ub - *ua).perp_dot(*uc - *ua).abs();
                }
            }
            if surface > 1e-14 {
                let scale = (flat.max(1e-14) / surface).sqrt();
                for point in &mut points {
                    *point *= scale;
                }
            }
        }
        chart.uv = crate::minimum_stretch::solve(&points, &triangles, &pins, iterations)?;
        if pins.is_empty() {
            fit_area(mesh, chart);
            turn_to_smallest_box(chart);
            free.push(index);
        }
    }
    let mut packed: Vec<Chart> = free.iter().filter_map(|index| charts.get(*index).cloned()).collect();
    pack(&mut packed, f64::from(margin));
    let fixed: Vec<DVec2> =
        charts.iter().enumerate().filter(|(index, _)| free.binary_search(index).is_err()).flat_map(|(_, chart)| chart.uv.iter().copied()).collect();
    if !fixed.is_empty() && !packed.is_empty() {
        let (low, high) = bounds(&fixed);
        let gap = f64::from(margin);
        // Keep pinned islands' frame and leave them room. Try the four rectangular regions
        // around their combined bounds; when they fill the tile, use the adjacent tile.
        let regions = [
            (DVec2::ZERO, DVec2::new((low.x - gap).clamp(0.0, 1.0), 1.0)),
            (DVec2::new((high.x + gap).clamp(0.0, 1.0), 0.0), DVec2::ONE),
            (DVec2::ZERO, DVec2::new(1.0, (low.y - gap).clamp(0.0, 1.0))),
            (DVec2::new(0.0, (high.y + gap).clamp(0.0, 1.0)), DVec2::ONE),
        ];
        let best = regions.iter().max_by(|(a, b), (c, d)| (*b - *a).min_element().total_cmp(&(*d - *c).min_element()));
        let (offset, scale) = match best {
            Some((a, b)) if (*b - *a).min_element() > 1e-5 => (*a, (*b - *a).min_element()),
            _ => (DVec2::new(high.x.max(1.0) + gap, 0.0), 1.0),
        };
        for chart in &mut packed {
            for uv in &mut chart.uv {
                *uv = *uv * scale + offset;
            }
        }
    }
    for (index, chart) in free.into_iter().zip(packed) {
        if let Some(slot) = charts.get_mut(index) {
            *slot = chart;
        }
    }
    validate_charts(&charts)?;
    write_charts(mesh, &charts);
    Ok(Unwrapped { charts: charts.len(), faces: chosen.len() })
}

fn check_live_size(mesh: &Mesh, chosen: &[usize]) -> Result<()> {
    let corners = chosen.iter().try_fold(0usize, |sum, face| sum.checked_add(mesh.faces.get(*face).map_or(0, Vec::len)));
    if chosen.len() > 40_000 || corners.is_none_or(|count| count > 120_000) {
        return Err(SceneError::params("minimum stretch and live unwrap support up to 40000 faces and 120000 corners per operation"));
    }
    Ok(())
}

fn check_chart_size(charts: &[Chart]) -> Result<()> {
    if charts.len() > 128 || charts.iter().map(|chart| chart.source.len()).sum::<usize>() > 20_000 {
        return Err(SceneError::params("minimum stretch and live unwrap support up to 128 islands and 20000 chart vertices per operation"));
    }
    Ok(())
}

fn chart_coordinates(mesh: &Mesh, chart: &Chart) -> Vec<DVec2> {
    let mut coordinates = vec![DVec2::ZERO; chart.source.len()];
    for &face in &chart.faces {
        for corner in 0..mesh.faces.get(face).map_or(0, Vec::len) {
            if let Some(&vertex) = chart.corner_vertex.get(&(face, corner))
                && let Some(slot) = coordinates.get_mut(vertex)
            {
                *slot = mesh.corner_uv(face, corner).as_dvec2();
            }
        }
    }
    coordinates
}

/// Existing UV splits remain island boundaries during live editing, even without marked seams.
fn uv_boundaries(mesh: &Mesh) -> Vec<[u32; 2]> {
    let mut uses = std::collections::BTreeMap::<[u32; 2], Vec<(Vec2, Vec2)>>::new();
    for (face, vertices) in mesh.faces.iter().enumerate() {
        for (corner, (a, b)) in face_edges(vertices).enumerate() {
            let p = mesh.corner_uv(face, corner);
            let q = mesh.corner_uv(face, (corner + 1) % vertices.len());
            uses.entry([a.min(b), a.max(b)]).or_default().push(if a < b { (p, q) } else { (q, p) });
        }
    }
    uses.into_iter()
        .filter_map(|(edge, list)| match list.as_slice() {
            [(a, b), (c, d)] if a.distance_squared(*c) > 1e-12 || b.distance_squared(*d) > 1e-12 => Some(edge),
            _ => None,
        })
        .collect()
}

fn chart_pins(mesh: &Mesh, chart: &Chart) -> Result<Vec<(usize, DVec2)>> {
    let pinned: BTreeSet<u32> = mesh.pinned.iter().copied().collect();
    let mut out = std::collections::BTreeMap::new();
    for &face in &chart.faces {
        for corner in 0..mesh.faces.get(face).map_or(0, Vec::len) {
            let Some(&vertex) = chart.corner_vertex.get(&(face, corner)) else { continue };
            if chart.source.get(vertex).is_some_and(|source| pinned.contains(source)) {
                let uv = mesh.corner_uv(face, corner).as_dvec2();
                if out.get(&vertex).is_some_and(|previous: &DVec2| previous.distance(uv) > 1e-7) {
                    return Err(SceneError::params("a pinned vertex has discontinuous UVs inside a chart; mark its boundary as seams"));
                }
                out.insert(vertex, uv);
            }
        }
    }
    Ok(out.into_iter().collect())
}

fn validate_charts(charts: &[Chart]) -> Result<()> {
    if charts.iter().flat_map(|chart| &chart.uv).any(|uv| !uv.is_finite() || uv.abs().max_element() >= 1e6) {
        return Err(SceneError::params("the unwrap produced coordinates outside the supported range"));
    }
    Ok(())
}

fn write_charts(mesh: &mut Mesh, charts: &[Chart]) {
    if !mesh.has_uvs() || !mesh.uvs_aligned() {
        mesh.uvs.clear();
        mesh.fill_uvs();
    }
    for chart in charts {
        for (&(face, corner), &vertex) in &chart.corner_vertex {
            if let (Some(slot), Some(uv)) = (mesh.uvs.get_mut(face).and_then(|uvs| uvs.get_mut(corner)), chart.uv.get(vertex)) {
                *slot = uv.as_vec2();
            }
        }
    }
}

/// Relax only charts containing a moved pin. Other islands and every pin retain their exact
/// coordinates. Live transforms use this without packing, in the same transaction as the move.
pub fn live_unwrap(mesh: &mut Mesh, original: &Mesh, moved: &BTreeSet<u32>, method: Method, iterations: u32) -> Result<usize> {
    mesh.validate()?;
    original.validate()?;
    if !(1..=100).contains(&iterations) {
        return Err(SceneError::params("live unwrap needs 1 to 100 iterations"));
    }
    let chosen: Vec<usize> = (0..mesh.faces.len()).collect();
    check_live_size(mesh, &chosen)?;
    if original.faces != mesh.faces || original.positions != mesh.positions || !original.uvs_aligned() {
        return Err(SceneError::params("the live unwrap source must have the same geometry and aligned UVs"));
    }
    let cuts = uv_boundaries(original);
    let mut charts = charts_with_cuts(mesh, &chosen, &cuts)?;
    check_chart_size(&charts)?;
    let mut changed = Vec::new();
    for chart in &mut charts {
        if !chart.source.iter().any(|vertex| moved.contains(vertex) && mesh.pinned.contains(vertex)) {
            continue;
        }
        let pins = chart_pins(mesh, chart)?;
        // Observed in the UV Editor: a single moving pin translates its entire island,
        // retaining the old frame. Two or more pins constrain a fresh relaxation.
        if pins.len() == 1 {
            if let Some(&(pin, target)) = pins.first() {
                let previous = chart_coordinates(original, chart);
                let delta = target - previous.get(pin).copied().unwrap_or_default();
                chart.uv = previous.into_iter().map(|uv| uv + delta).collect();
                changed.push(chart.clone());
            }
            continue;
        }
        let points: Vec<DVec3> = chart.source.iter().map(|vertex| position(mesh, *vertex)).collect();
        chart.uv = match method {
            Method::MinimumStretch => {
                let triangles = crate::abf::chart_triangles(mesh, &chart.faces, &chart.corner_vertex);
                crate::minimum_stretch::solve(&points, &triangles, &pins, iterations)?
            }
            Method::Conformal => lscm(&points, &triangles(mesh, chart), &pins),
            Method::AngleBased => {
                let triangles = crate::abf::chart_triangles(mesh, &chart.faces, &chart.corner_vertex);
                lscm_shapes(points.len(), &crate::abf::shapes(&points, &triangles), &pins)
            }
        };
        changed.push(chart.clone());
    }
    validate_charts(&changed)?;
    write_charts(mesh, &changed);
    Ok(changed.len())
}

/// Splits the chosen faces into charts: faces join across an edge they share unless it is a
/// seam, and a vertex is split into one chart vertex per fan of corners joined that way.
fn charts(mesh: &Mesh, chosen: &[usize]) -> Result<Vec<Chart>> {
    charts_with_cuts(mesh, chosen, &[])
}

fn charts_with_cuts(mesh: &Mesh, chosen: &[usize], cuts: &[[u32; 2]]) -> Result<Vec<Chart>> {
    let chosen_set: BTreeSet<usize> = chosen.iter().copied().collect();
    let seams: BTreeSet<[u32; 2]> = mesh.seams.iter().chain(cuts).copied().collect();
    // Corners are (face, corner); union-find over them and over faces.
    let mut corner_ids: HashMap<(usize, usize), usize> = HashMap::new();
    let mut corners: Vec<(usize, usize)> = Vec::new();
    for &face in chosen {
        for corner in 0..mesh.faces.get(face).map_or(0, Vec::len) {
            corner_ids.insert((face, corner), corners.len());
            corners.push((face, corner));
        }
    }
    let mut corner_sets = UnionFind::new(corners.len());
    let mut face_sets = UnionFind::new(mesh.faces.len());
    // Each edge's uses: (face, corner at its first end, corner at its second end) by direction.
    let mut uses: HashMap<[u32; 2], Vec<EdgeUse>> = HashMap::new();
    for &face in chosen {
        let Some(vertices) = mesh.faces.get(face) else { continue };
        let count = vertices.len();
        for (corner, (a, b)) in face_edges(vertices).enumerate() {
            uses.entry([a.min(b), a.max(b)]).or_default().push((face, a, corner, (corner + 1) % count));
        }
    }
    for (edge, list) in &uses {
        if seams.contains(edge) {
            continue;
        }
        let [first, second] = list.as_slice() else { continue };
        if !(chosen_set.contains(&first.0) && chosen_set.contains(&second.0)) {
            continue;
        }
        face_sets.join(first.0, second.0);
        // Join the corners at the same vertex across the edge.
        let vertex_at = |use_: &EdgeUse, vertex: u32| if use_.1 == vertex { use_.2 } else { use_.3 };
        for vertex in [edge[0], edge[1]] {
            let (Some(&left), Some(&right)) =
                (corner_ids.get(&(first.0, vertex_at(first, vertex))), corner_ids.get(&(second.0, vertex_at(second, vertex))))
            else {
                continue;
            };
            corner_sets.join(left, right);
        }
    }
    let mut by_root: HashMap<usize, Chart> = HashMap::new();
    let mut order: Vec<usize> = Vec::new();
    let mut vertex_of_group: HashMap<usize, (usize, usize)> = HashMap::new();
    for &face in chosen {
        let root = face_sets.find(face);
        let chart = by_root.entry(root).or_insert_with(|| {
            order.push(root);
            Chart { faces: Vec::new(), corner_vertex: HashMap::new(), source: Vec::new(), uv: Vec::new() }
        });
        chart.faces.push(face);
        let Some(vertices) = mesh.faces.get(face) else { continue };
        for (corner, vertex) in vertices.iter().enumerate() {
            let Some(&id) = corner_ids.get(&(face, corner)) else { continue };
            let group = corner_sets.find(id);
            let local = match vertex_of_group.get(&group) {
                Some(&(owner, local)) if owner == root => local,
                _ => {
                    let local = chart.source.len();
                    chart.source.push(*vertex);
                    vertex_of_group.insert(group, (root, local));
                    local
                }
            };
            chart.corner_vertex.insert((face, corner), local);
        }
    }
    let charts: Vec<Chart> = order.into_iter().filter_map(|root| by_root.remove(&root)).collect();
    if charts.iter().map(|chart| chart.source.len()).sum::<usize>() > MAX_VERTICES {
        return Err(SceneError::params("the mesh is too large to unwrap"));
    }
    Ok(charts)
}

fn position(mesh: &Mesh, vertex: u32) -> DVec3 {
    mesh.positions.get(vertex as usize).map_or(DVec3::ZERO, |point| point.as_dvec3())
}

/// The chart's triangles as chart vertices, each with a weight: a quad counts both of its
/// diagonals at half weight, so the flattening does not depend on which diagonal splits it.
fn triangles(mesh: &Mesh, chart: &Chart) -> Vec<([usize; 3], f64)> {
    let mut out = Vec::new();
    let wanted: BTreeSet<usize> = chart.faces.iter().copied().collect();
    let local = |face: usize, corner: usize| chart.corner_vertex.get(&(face, corner)).copied();
    let mut quads = BTreeSet::new();
    for &face in &chart.faces {
        if mesh.faces.get(face).map(Vec::len) == Some(4)
            && let (Some(a), Some(b), Some(c), Some(d)) = (local(face, 0), local(face, 1), local(face, 2), local(face, 3))
        {
            out.extend([([a, b, c], 0.5), ([a, c, d], 0.5), ([a, b, d], 0.5), ([b, c, d], 0.5)]);
            quads.insert(face);
        }
    }
    for (corners, face) in mesh.corner_triangles() {
        let face = face as usize;
        if !wanted.contains(&face) || quads.contains(&face) {
            continue;
        }
        if let (Some(a), Some(b), Some(c)) = (local(face, corners[0] as usize), local(face, corners[1] as usize), local(face, corners[2] as usize)) {
            out.push(([a, b, c], 1.0));
        }
    }
    out
}

/// Flattens a chart by LSCM. Pinned mesh vertices keep their coordinates when at least two are
/// in the chart; otherwise the two chart vertices farthest apart are pinned.
fn flatten(mesh: &Mesh, chart: &mut Chart, angle_based: bool) {
    let count = chart.source.len();
    let triangles = triangles(mesh, chart);
    let points: Vec<DVec3> = chart.source.iter().map(|vertex| position(mesh, *vertex)).collect();
    // The chart's current coordinates, for pinned vertices.
    let mut current = vec![DVec2::ZERO; count];
    for (&(face, corner), &vertex) in &chart.corner_vertex {
        if let Some(slot) = current.get_mut(vertex) {
            let uv = mesh.corner_uv(face, corner);
            *slot = DVec2::new(f64::from(uv.x), f64::from(uv.y));
        }
    }
    let pinned_set: BTreeSet<u32> = mesh.pinned.iter().copied().collect();
    let mut pins: Vec<(usize, DVec2)> = (0..count)
        .filter(|vertex| chart.source.get(*vertex).is_some_and(|source| pinned_set.contains(source)))
        .map(|vertex| (vertex, current.get(vertex).copied().unwrap_or_default()))
        .collect();
    if pins.len() < 2 {
        let (a, b) = farthest_pair(&points);
        let distance = points.get(a).zip(points.get(b)).map_or(1.0, |(p, q)| p.distance(*q)).max(1e-9);
        pins = vec![(a, DVec2::ZERO), (b, DVec2::new(distance, 0.0))];
    }
    chart.uv = if angle_based {
        let plain = crate::abf::chart_triangles(mesh, &chart.faces, &chart.corner_vertex);
        let shapes = crate::abf::shapes(&points, &plain);
        lscm_shapes(count, &shapes, &pins)
    } else {
        lscm(&points, &triangles, &pins)
    };
}

/// The two points farthest apart (approximately: the ends of the longest axis, refined once).
pub(crate) fn farthest_pair(points: &[DVec3]) -> (usize, usize) {
    let mut best = (0, 0, -1.0f64);
    for axis in 0..3 {
        let value = |point: &DVec3| point.to_array().get(axis).copied().unwrap_or(0.0);
        let low = points.iter().enumerate().min_by(|a, b| value(a.1).total_cmp(&value(b.1))).map_or(0, |(index, _)| index);
        let high = points.iter().enumerate().max_by(|a, b| value(a.1).total_cmp(&value(b.1))).map_or(0, |(index, _)| index);
        let distance = points.get(low).zip(points.get(high)).map_or(0.0, |(p, q)| p.distance(*q));
        if distance > best.2 {
            best = (low, high, distance);
        }
    }
    if best.0 == best.1 {
        return (0, usize::from(points.len() > 1));
    }
    (best.0, best.1)
}

/// Least-squares conformal maps: for each triangle in its own plane, the complex expression of
/// the Cauchy–Riemann equations, summed over triangles weighted by area, minimised with the
/// pinned vertices fixed (conjugate gradients on the normal equations).
fn lscm(points: &[DVec3], triangles: &[([usize; 3], f64)], pins: &[(usize, DVec2)]) -> Vec<DVec2> {
    let mut shapes = Vec::with_capacity(triangles.len());
    for &([a, b, c], weight) in triangles {
        let (Some(pa), Some(pb), Some(pc)) = (points.get(a), points.get(b), points.get(c)) else { continue };
        // The triangle in its own plane: a at the origin, b on the x axis.
        let ab = *pb - *pa;
        let ac = *pc - *pa;
        let x_axis = ab.normalize_or_zero();
        let normal = ab.cross(ac);
        if normal.length() < 2e-14 || x_axis == DVec3::ZERO {
            continue;
        }
        let y_axis = normal.normalize().cross(x_axis);
        shapes.push(([a, b, c], [DVec2::ZERO, DVec2::new(ab.length(), 0.0), DVec2::new(ac.dot(x_axis), ac.dot(y_axis))], weight));
    }
    lscm_shapes(points.len(), &shapes, pins)
}

/// A triangle of chart vertices, its shape in its own plane, and its weight.
pub(crate) type Shape = ([usize; 3], [DVec2; 3], f64);

/// LSCM over triangles of given planar shapes (from the surface, or from solved angles).
pub(crate) fn lscm_shapes(count: usize, triangles: &[Shape], pins: &[(usize, DVec2)]) -> Vec<DVec2> {
    let pinned: HashMap<usize, DVec2> = pins.iter().copied().collect();
    // Free vertices get unknowns 2k (u) and 2k+1 (v).
    let mut slot = vec![usize::MAX; count];
    let mut free = 0;
    for (vertex, entry) in slot.iter_mut().enumerate() {
        if !pinned.contains_key(&vertex) {
            *entry = free;
            free += 1;
        }
    }
    // Rows: (entries, right-hand side).
    let mut rows: Vec<(Vec<(usize, f64)>, f64)> = Vec::with_capacity(triangles.len() * 2);
    for &([a, b, c], local, weight) in triangles {
        let area = (local[1] - local[0]).perp_dot(local[2] - local[0]).abs() / 2.0;
        if !area.is_finite() || area < 1e-14 {
            continue;
        }
        // W_j = p_{j+2} - p_{j+1} (as complex numbers).
        let w = [local[2] - local[1], local[0] - local[2], local[1] - local[0]];
        let scale = weight.sqrt() / (2.0 * area).sqrt();
        let vertices = [a, b, c];
        let mut real = (Vec::with_capacity(6), 0.0);
        let mut imaginary = (Vec::with_capacity(6), 0.0);
        for (vertex, weight) in vertices.iter().zip(w) {
            let (wr, wi) = (weight.x * scale, weight.y * scale);
            // Re(W U) = wr u - wi v; Im(W U) = wi u + wr v.
            match pinned.get(vertex) {
                Some(uv) => {
                    real.1 -= wr * uv.x - wi * uv.y;
                    imaginary.1 -= wi * uv.x + wr * uv.y;
                }
                None => {
                    let k = slot.get(*vertex).copied().unwrap_or(0);
                    real.0.push((2 * k, wr));
                    real.0.push((2 * k + 1, -wi));
                    imaginary.0.push((2 * k, wi));
                    imaginary.0.push((2 * k + 1, wr));
                }
            }
        }
        rows.push(real);
        rows.push(imaginary);
    }
    let unknowns = 2 * free;
    // Start from a planar projection so CG converges fast.
    let mut x = vec![0.0; unknowns];
    let solution = conjugate_gradient(&rows, unknowns, &mut x);
    let mut out = vec![DVec2::ZERO; count];
    for (vertex, uv) in out.iter_mut().enumerate() {
        *uv = match pinned.get(&vertex) {
            Some(fixed) => *fixed,
            None => {
                let k = slot.get(vertex).copied().unwrap_or(0);
                DVec2::new(solution.get(2 * k).copied().unwrap_or(0.0), solution.get(2 * k + 1).copied().unwrap_or(0.0))
            }
        };
    }
    out
}

/// Solves the normal equations AᵀA x = Aᵀb of the rows by conjugate gradients.
pub(crate) fn conjugate_gradient(rows: &[(Vec<(usize, f64)>, f64)], unknowns: usize, x: &mut [f64]) -> Vec<f64> {
    let apply = |vector: &[f64]| -> Vec<f64> {
        // AᵀA v, one row at a time.
        let mut out = vec![0.0; unknowns];
        for (entries, _) in rows {
            let dot: f64 = entries.iter().map(|(column, value)| value * vector.get(*column).copied().unwrap_or(0.0)).sum();
            for (column, value) in entries {
                if let Some(slot) = out.get_mut(*column) {
                    *slot += value * dot;
                }
            }
        }
        out
    };
    let mut rhs = vec![0.0; unknowns];
    for (entries, b) in rows {
        for (column, value) in entries {
            if let Some(slot) = rhs.get_mut(*column) {
                *slot += value * b;
            }
        }
    }
    let product = apply(x);
    let mut residual: Vec<f64> = rhs.iter().zip(&product).map(|(b, ax)| b - ax).collect();
    let mut direction = residual.clone();
    let mut old = dot(&residual, &residual);
    let scale = dot(&rhs, &rhs).max(1e-30);
    for _ in 0..(unknowns * 2).clamp(100, 20_000) {
        if old <= scale * 1e-24 {
            break;
        }
        let towards = apply(&direction);
        let curvature = dot(&direction, &towards);
        if curvature.abs() < 1e-300 {
            break;
        }
        let step = old / curvature;
        for (value, d) in x.iter_mut().zip(&direction) {
            *value += step * d;
        }
        for (r, t) in residual.iter_mut().zip(&towards) {
            *r -= step * t;
        }
        let new = dot(&residual, &residual);
        let ratio = new / old;
        for (d, r) in direction.iter_mut().zip(&residual) {
            *d = r + ratio * *d;
        }
        old = new;
    }
    x.to_vec()
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Scales a chart so its texture area matches its surface area.
fn fit_area(mesh: &Mesh, chart: &mut Chart) {
    let mut surface = 0.0;
    let mut flat = 0.0;
    let points: Vec<DVec3> = chart.source.iter().map(|vertex| position(mesh, *vertex)).collect();
    for ([a, b, c], weight) in triangles(mesh, chart) {
        if let (Some(pa), Some(pb), Some(pc), Some(ua), Some(ub), Some(uc)) =
            (points.get(a), points.get(b), points.get(c), chart.uv.get(a), chart.uv.get(b), chart.uv.get(c))
        {
            surface += weight * (*pb - *pa).cross(*pc - *pa).length() / 2.0;
            flat += weight * ((*ub - *ua).perp_dot(*uc - *ua)).abs() / 2.0;
        }
    }
    let factor = if flat > 1e-18 { (surface / flat).sqrt() } else { 1.0 };
    for uv in &mut chart.uv {
        *uv *= factor;
    }
}

/// Turns a chart to the angle (in 1° steps, refined to 0.1°) giving the smallest box, then moves
/// it to the origin.
fn turn_to_smallest_box(chart: &mut Chart) {
    let area_at = |angle: f64| -> f64 {
        let (sin, cos) = angle.sin_cos();
        let (mut min, mut max) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
        for uv in &chart.uv {
            let turned = DVec2::new(uv.x * cos - uv.y * sin, uv.x * sin + uv.y * cos);
            min = min.min(turned);
            max = max.max(turned);
        }
        let size = max - min;
        size.x * size.y
    };
    let mut best = (0.0f64, area_at(0.0));
    for step in 1..90 {
        let angle = f64::from(step).to_radians();
        let area = area_at(angle);
        if area < best.1 - 1e-12 {
            best = (angle, area);
        }
    }
    for step in -10..=10 {
        let angle = best.0 + f64::from(step).to_radians() / 10.0;
        let area = area_at(angle);
        if area < best.1 - 1e-12 {
            best = (angle, area);
        }
    }
    let (sin, cos) = best.0.sin_cos();
    for uv in &mut chart.uv {
        *uv = DVec2::new(uv.x * cos - uv.y * sin, uv.x * sin + uv.y * cos);
    }
    // Wider than tall.
    let (min, max) = bounds(&chart.uv);
    if max.y - min.y > max.x - min.x {
        for uv in &mut chart.uv {
            *uv = DVec2::new(uv.y, -uv.x);
        }
    }
    let (min, _) = bounds(&chart.uv);
    for uv in &mut chart.uv {
        *uv -= min;
    }
}

fn bounds(points: &[DVec2]) -> (DVec2, DVec2) {
    points.iter().fold((DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)), |(min, max), point| (min.min(*point), max.max(*point)))
}

/// Packs charts (each already at the origin) into the unit square: tallest first, in shelves,
/// with the whole layout scaled to fit and `margin` between charts.
fn pack(charts: &mut [Chart], margin: f64) {
    let sizes: Vec<DVec2> = charts.iter().map(|chart| bounds(&chart.uv).1.max(DVec2::splat(1e-12))).collect();
    let total: f64 = sizes.iter().map(|size| size.x * size.y).sum();
    let mut order: Vec<usize> = (0..charts.len()).collect();
    order.sort_by(|a, b| sizes.get(*b).map_or(0.0, |size| size.y).total_cmp(&sizes.get(*a).map_or(0.0, |size| size.y)));
    // Try shelf widths around the square root of the total area; keep the smallest square.
    let widest = sizes.iter().map(|size| size.x).fold(0.0f64, f64::max);
    let mut best: Option<(f64, Vec<DVec2>)> = None;
    for step in 0..24 {
        let width = (total.sqrt() * (0.7 + 0.05 * f64::from(step))).max(widest);
        let gap = margin * width;
        let (mut x, mut y, mut shelf) = (0.0f64, 0.0f64, 0.0f64);
        let mut offsets = vec![DVec2::ZERO; charts.len()];
        let mut used_width = 0.0f64;
        for &index in &order {
            let Some(size) = sizes.get(index) else { continue };
            if x > 0.0 && x + size.x > width {
                x = 0.0;
                y += shelf + gap;
                shelf = 0.0;
            }
            if let Some(offset) = offsets.get_mut(index) {
                *offset = DVec2::new(x, y);
            }
            x += size.x + gap;
            used_width = used_width.max(x - gap);
            shelf = shelf.max(size.y);
        }
        let side = used_width.max(y + shelf);
        if best.as_ref().is_none_or(|(smallest, _)| side < *smallest) {
            best = Some((side, offsets));
        }
    }
    let Some((side, offsets)) = best else { return };
    // The margin also keeps charts off the square's edge.
    let border = margin * side;
    let scale = 1.0 / (side + 2.0 * border).max(1e-12);
    for (chart, offset) in charts.iter_mut().zip(offsets) {
        for uv in &mut chart.uv {
            *uv = (*uv + offset + DVec2::splat(border)) * scale;
        }
    }
}

/// Distortion of texture coordinates against the surface, over `faces` (all when empty): the
/// mean absolute difference of triangle angles (radians), and the mean absolute log of each
/// triangle's area ratio against the whole set's.
pub fn distortion(mesh: &Mesh, faces: &BTreeSet<u32>) -> (f64, f64) {
    let mut angle_error = 0.0;
    let mut angles = 0usize;
    let mut ratios: Vec<(f64, f64)> = Vec::new();
    for (corners, face) in mesh.corner_triangles() {
        if !faces.is_empty() && !faces.contains(&face) {
            continue;
        }
        let Some(vertices) = mesh.faces.get(face as usize) else { continue };
        let p: Vec<DVec3> = corners.iter().map(|corner| position(mesh, vertices.get(*corner as usize).copied().unwrap_or(0))).collect();
        let q: Vec<DVec2> = corners.iter().map(|corner| mesh.corner_uv(face as usize, *corner as usize).as_dvec2()).collect();
        let (Some(p0), Some(p1), Some(p2), Some(q0), Some(q1), Some(q2)) = (p.first(), p.get(1), p.get(2), q.first(), q.get(1), q.get(2)) else {
            continue;
        };
        let surface = (*p1 - *p0).cross(*p2 - *p0).length() / 2.0;
        let flat = (*q1 - *q0).perp_dot(*q2 - *q0).abs() / 2.0;
        if surface < 1e-14 {
            continue;
        }
        ratios.push((surface, flat));
        let surface_angles = [(*p1 - *p0).angle_between(*p2 - *p0), (*p2 - *p1).angle_between(*p0 - *p1), (*p0 - *p2).angle_between(*p1 - *p2)];
        let flat_angle = |a: DVec2, b: DVec2, c: DVec2| {
            let (u, v) = (b - a, c - a);
            if u.length_squared() < 1e-30 || v.length_squared() < 1e-30 { 0.0 } else { u.angle_to(v).abs() }
        };
        let flat_angles = [flat_angle(*q0, *q1, *q2), flat_angle(*q1, *q2, *q0), flat_angle(*q2, *q0, *q1)];
        for (s, f) in surface_angles.iter().zip(flat_angles) {
            angle_error += (s - f).abs();
            angles += 1;
        }
    }
    let total_surface: f64 = ratios.iter().map(|(surface, _)| surface).sum();
    let total_flat: f64 = ratios.iter().map(|(_, flat)| flat).sum();
    let area_error = if total_flat > 0.0 && !ratios.is_empty() {
        ratios.iter().map(|(surface, flat)| ((flat / total_flat).max(1e-12) / (surface / total_surface)).ln().abs()).sum::<f64>()
            / ratios.len() as f64
    } else {
        f64::INFINITY
    };
    (if angles > 0 { angle_error / angles as f64 } else { 0.0 }, area_error)
}

struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(count: usize) -> Self {
        Self { parent: (0..count).collect() }
    }

    fn find(&mut self, mut item: usize) -> usize {
        let mut root = item;
        while let Some(&parent) = self.parent.get(root) {
            if parent == root {
                break;
            }
            root = parent;
        }
        while let Some(&parent) = self.parent.get(item) {
            if parent == root {
                break;
            }
            if let Some(slot) = self.parent.get_mut(item) {
                *slot = root;
            }
            item = parent;
        }
        root
    }

    fn join(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b
            && let Some(slot) = self.parent.get_mut(a)
        {
            *slot = b;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::Primitive;
    use crate::seams;

    /// A cube cut into a cross: every edge but the bottom face's four and the top-front one.
    pub(crate) fn cross_seams(mesh: &Mesh) -> Vec<(u32, u32)> {
        mesh.edges()
            .into_iter()
            .filter(|[a, b]| {
                let (pa, pb) = (mesh.positions[*a as usize], mesh.positions[*b as usize]);
                let bottom = pa.z < 0.0 && pb.z < 0.0;
                let top_front = pa.z > 0.0 && pb.z > 0.0 && pa.y < 0.0 && pb.y < 0.0;
                !(bottom || top_front)
            })
            .map(|[a, b]| (a, b))
            .collect()
    }

    #[test]
    fn a_cut_cube_unfolds_flat_without_distortion() {
        let mut cube = Primitive::Cube { size: 2.0 }.build().unwrap();
        let cuts = cross_seams(&cube);
        assert_eq!(cuts.len(), 7);
        seams::mark(&mut cube, &cuts, true).unwrap();
        let done = unwrap(&mut cube, &BTreeSet::new(), 0.02).unwrap();
        assert_eq!(done, Unwrapped { charts: 1, faces: 6 });
        cube.validate().unwrap();
        let (angle, area) = distortion(&cube, &BTreeSet::new());
        assert!(angle < 1e-3 && area < 1e-3, "angle {angle} area {area}");
        for uv in cube.uvs.iter().flatten() {
            assert!((0.0..=1.0).contains(&uv.x) && (0.0..=1.0).contains(&uv.y), "{uv}");
        }
    }

    #[test]
    fn every_seam_gives_every_face_its_own_chart() {
        let mut cube = Primitive::Cube { size: 2.0 }.build().unwrap();
        let all: Vec<(u32, u32)> = cube.edges().into_iter().map(|[a, b]| (a, b)).collect();
        seams::mark(&mut cube, &all, true).unwrap();
        let done = unwrap(&mut cube, &BTreeSet::new(), 0.05).unwrap();
        assert_eq!(done.charts, 6);
        let (angle, area) = distortion(&cube, &BTreeSet::new());
        assert!(angle < 1e-3 && area < 1e-3);
    }

    #[test]
    fn pinned_vertices_keep_their_coordinates_and_unpinned_charts_relax() {
        let mut plane = Primitive::Grid { x_subdivisions: 4, y_subdivisions: 4, size: 2.0 }.build().unwrap();
        // Shear the grid's coordinates, pin two corners, and unwrap: the grid relaxes to squares.
        for uv in plane.uvs.iter_mut().flatten() {
            *uv = Vec2::new(uv.x + uv.y * 0.5, uv.y);
        }
        plane.pinned = vec![0, 4];
        unwrap(&mut plane, &BTreeSet::new(), 0.0).unwrap();
        let (angle, _) = distortion(&plane, &BTreeSet::new());
        assert!(angle < 1e-3, "{angle}");
    }

    #[test]
    fn a_sphere_cut_along_a_meridian_unwraps_in_one_chart() {
        let mut sphere = Primitive::UvSphere { segments: 16, rings: 8, radius: 1.0 }.build().unwrap();
        let meridian: Vec<(u32, u32)> = sphere
            .edges()
            .into_iter()
            .filter(|[a, b]| {
                let (pa, pb) = (sphere.positions[*a as usize], sphere.positions[*b as usize]);
                pa.x.abs() < 1e-4 && pb.x.abs() < 1e-4 && pa.y > -1e-4 && pb.y > -1e-4
            })
            .map(|[a, b]| (a, b))
            .collect();
        assert_eq!(meridian.len(), 8);
        seams::mark(&mut sphere, &meridian, true).unwrap();
        let done = unwrap(&mut sphere, &BTreeSet::new(), 0.02).unwrap();
        assert_eq!(done.charts, 1);
        sphere.validate().unwrap();
        let (angle, area) = distortion(&sphere, &BTreeSet::new());
        assert!(angle < 0.2, "angle {angle}");
        assert!(area.is_finite(), "area {area}");
    }
}
