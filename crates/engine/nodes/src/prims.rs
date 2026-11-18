//! The mesh primitive nodes. Their parameters follow the reference application's nodes (a
//! cube by vertex counts per axis, a cone with its base on the ground), which differ from the
//! Add ▸ Mesh operators; counts and bounds are checked against it in `tests/blender.rs`.

use std::collections::HashMap;
use std::f32::consts::TAU;

use glam::Vec3;
use kition_scene_model::Mesh;

use crate::field::Scalar;
use crate::geometry::{Attribute, Domain, MAX_ELEMENTS, MeshComponent};

/// How the ends of a cone or a circle are closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    None,
    Ngon,
    TriangleFan,
}

impl Fill {
    /// The fill a setting names.
    pub fn parse(name: &str) -> Self {
        match name {
            "none" => Self::None,
            "triangle_fan" => Self::TriangleFan,
            _ => Self::Ngon,
        }
    }
}

fn index(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn too_many(count: u64) -> Result<(), String> {
    if count > MAX_ELEMENTS as u64 { Err(format!("a primitive would have more than {MAX_ELEMENTS} vertices")) } else { Ok(()) }
}

/// A line of `count` points from `start`, `step` apart, joined by loose edges.
pub fn line_points(count: usize, start: Vec3, step: Vec3) -> Mesh {
    let mut mesh = Mesh { positions: (0..count).map(|at| start + step * at as f32).collect(), ..Mesh::default() };
    mesh.loose_edges = (1..count).map(|at| [index(at - 1), index(at)]).collect();
    mesh
}

/// The Mesh Line node: `count` points, `offset` apart or spread to `offset` as the end.
pub fn line(count: i32, start: Vec3, offset: Vec3, end_points: bool) -> Result<Mesh, String> {
    let count = usize::try_from(count).unwrap_or(0);
    too_many(count as u64)?;
    let step = if end_points { if count > 1 { (offset - start) / (count - 1) as f32 } else { Vec3::ZERO } } else { offset };
    Ok(line_points(count, start, step))
}

/// The Grid node: `x` × `y` vertices over `size_x` × `size_y`, facing up.
pub fn grid(size_x: f32, size_y: f32, x: i32, y: i32) -> Result<Mesh, String> {
    let (x, y) = (usize::try_from(x).unwrap_or(0), usize::try_from(y).unwrap_or(0));
    if x == 0 || y == 0 {
        return Ok(Mesh::default());
    }
    too_many(x as u64 * y as u64)?;
    let coordinate = |at: usize, count: usize, size: f32| if count > 1 { (at as f32 / (count - 1) as f32 - 0.5) * size } else { 0.0 };
    if x == 1 || y == 1 {
        let (count, along) = if x == 1 { (y, Vec3::Y * size_y) } else { (x, Vec3::X * size_x) };
        let step = if count > 1 { along / (count - 1) as f32 } else { Vec3::ZERO };
        return Ok(line_points(count, -step * (count.saturating_sub(1)) as f32 / 2.0, step));
    }
    let mut mesh = Mesh::default();
    for i in 0..x {
        for j in 0..y {
            mesh.positions.push(Vec3::new(coordinate(i, x, size_x), coordinate(j, y, size_y), 0.0));
        }
    }
    let at = |i: usize, j: usize| index(i * y + j);
    for i in 0..x - 1 {
        for j in 0..y - 1 {
            mesh.faces.push(vec![at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)]);
        }
    }
    mesh.uvs = mesh.faces.iter().map(|face| face.iter().map(|vertex| uv_of(&mesh.positions, *vertex, size_x, size_y)).collect()).collect();
    Ok(mesh)
}

fn uv_of(points: &[Vec3], vertex: u32, size_x: f32, size_y: f32) -> glam::Vec2 {
    let point = points.get(vertex as usize).copied().unwrap_or(Vec3::ZERO);
    let unit = |value: f32, size: f32| if size != 0.0 { value / size + 0.5 } else { 0.0 };
    glam::Vec2::new(unit(point.x, size_x), unit(point.y, size_y))
}

/// The Cube node: a box of `size` with `counts` vertices along each axis (an axis with one
/// vertex flattens the box into a grid, two into a line).
pub fn cube(size: Vec3, counts: [i32; 3]) -> Result<Mesh, String> {
    let counts = counts.map(|count| usize::try_from(count).unwrap_or(0));
    if counts.contains(&0) {
        return Ok(Mesh::default());
    }
    too_many(counts.iter().map(|count| *count as u64).product())?;
    let wide: Vec<usize> = (0..3).filter(|axis| counts.get(*axis).copied().unwrap_or(1) > 1).collect();
    let size_at = |axis: usize| size.to_array().get(axis).copied().unwrap_or(1.0);
    let count_at = |axis: usize| counts.get(axis).copied().unwrap_or(1);
    match wide.as_slice() {
        [] => return Ok(Mesh { positions: vec![Vec3::ZERO], ..Mesh::default() }),
        [axis] => {
            let mut step = Vec3::ZERO;
            if let Some(slot) = step.as_mut().get_mut(*axis) {
                *slot = size_at(*axis) / (count_at(*axis) - 1) as f32;
            }
            return Ok(line_points(count_at(*axis), -step * (count_at(*axis) - 1) as f32 / 2.0, step));
        }
        [a, b] => {
            let flat = grid(size_at(*a), size_at(*b), i32::try_from(count_at(*a)).unwrap_or(2), i32::try_from(count_at(*b)).unwrap_or(2))?;
            let mut mesh = flat;
            for point in &mut mesh.positions {
                let mut out = Vec3::ZERO;
                let (u, v) = (point.x, point.y);
                if let Some(slot) = out.as_mut().get_mut(*a) {
                    *slot = u;
                }
                if let Some(slot) = out.as_mut().get_mut(*b) {
                    *slot = v;
                }
                *point = out;
            }
            // A grid across X and Z faces −Y when its axes run that way round: keep it facing
            // the positive third axis.
            if *a == 0 && *b == 2 {
                mesh.flip_all();
            }
            return Ok(mesh);
        }
        _ => {}
    }
    // The surface's lattice points by layer (z), row (y), and column (x); then the sides'
    // faces bottom, front, top, back, left, right, as the reference orders them.
    let [nx, ny, nz] = counts;
    let surface = |i: usize, j: usize, k: usize| i == 0 || j == 0 || k == 0 || i + 1 == nx || j + 1 == ny || k + 1 == nz;
    let mut ids: HashMap<[usize; 3], u32> = HashMap::new();
    let mut mesh = Mesh::default();
    let at = |value: usize, count: usize, size: f32| (value as f32 / (count - 1) as f32 - 0.5) * size;
    for k in 0..nz {
        for j in 0..ny {
            for i in 0..nx {
                if surface(i, j, k) {
                    ids.insert([i, j, k], index(mesh.positions.len()));
                    mesh.positions.push(Vec3::new(at(i, nx, size.x), at(j, ny, size.y), at(k, nz, size.z)));
                }
            }
        }
    }
    let id = |lattice: [usize; 3]| ids.get(&lattice).copied().unwrap_or(0);
    let quads = |outer: usize, inner: usize, corners: &dyn Fn(usize, usize) -> [[usize; 3]; 4], mesh: &mut Mesh| {
        for a in 0..outer - 1 {
            for b in 0..inner - 1 {
                mesh.faces.push(corners(a, b).iter().map(|lattice| id(*lattice)).collect());
            }
        }
    };
    let (top, back, right) = (nz - 1, ny - 1, nx - 1);
    quads(ny, nx, &|j, i| [[i, j, 0], [i, j + 1, 0], [i + 1, j + 1, 0], [i + 1, j, 0]], &mut mesh);
    quads(nz, nx, &|k, i| [[i, 0, k], [i + 1, 0, k], [i + 1, 0, k + 1], [i, 0, k + 1]], &mut mesh);
    quads(ny, nx, &|j, i| [[i, j, top], [i + 1, j, top], [i + 1, j + 1, top], [i, j + 1, top]], &mut mesh);
    quads(nz, nx, &|k, i| [[i, back, k], [i, back, k + 1], [i + 1, back, k + 1], [i + 1, back, k]], &mut mesh);
    quads(nz, ny, &|k, j| [[0, j, k], [0, j, k + 1], [0, j + 1, k + 1], [0, j + 1, k]], &mut mesh);
    quads(nz, ny, &|k, j| [[right, j, k], [right, j + 1, k], [right, j + 1, k + 1], [right, j, k + 1]], &mut mesh);
    Ok(mesh)
}

/// The Mesh Circle node.
pub fn circle(vertices: i32, radius: f32, fill: Fill) -> Result<Mesh, String> {
    let count = usize::try_from(vertices).unwrap_or(0);
    if count < 3 {
        return Ok(Mesh::default());
    }
    too_many(count as u64)?;
    let mut mesh = Mesh { positions: ring(count, radius, 0.0), ..Mesh::default() };
    let all: Vec<u32> = (0..count).map(index).collect();
    match fill {
        Fill::None => mesh.loose_edges = (0..count).map(|at| [index(at), index((at + 1) % count)]).collect(),
        Fill::Ngon => mesh.faces.push(all),
        Fill::TriangleFan => {
            mesh.positions.push(Vec3::ZERO);
            let centre = index(count);
            mesh.faces = (0..count).map(|at| vec![index(at), index((at + 1) % count), centre]).collect();
        }
    }
    Ok(mesh)
}

fn ring(count: usize, radius: f32, z: f32) -> Vec<Vec3> {
    (0..count)
        .map(|at| {
            let angle = TAU * at as f32 / count as f32;
            Vec3::new(radius * angle.cos(), radius * angle.sin(), z)
        })
        .collect()
}

/// A cone's or cylinder's parameters.
pub struct Cone {
    pub vertices: i32,
    pub side_segments: i32,
    pub fill_segments: i32,
    pub radius_top: f32,
    pub radius_bottom: f32,
    pub depth: f32,
    pub fill: Fill,
    /// Where the bottom sits (the cylinder is centred, the cone stands on the ground).
    pub bottom: f32,
}

/// The Cone and Cylinder nodes, with their Top, Side, and Bottom face selections stored as
/// face attributes under `names` (top, side, bottom).
pub fn cone(cone: &Cone, names: [&str; 3]) -> Result<MeshComponent, String> {
    let count = usize::try_from(cone.vertices).unwrap_or(0);
    let sides = usize::try_from(cone.side_segments).unwrap_or(0).max(1);
    let fills = usize::try_from(cone.fill_segments).unwrap_or(0).max(1);
    if count < 3 {
        return Ok(MeshComponent::default());
    }
    too_many(count as u64 * (sides as u64 + 2 * fills as u64 + 1))?;
    let mut mesh = Mesh::default();
    let mut kinds: Vec<usize> = Vec::new();
    // Side rings from the top down; a ring of radius zero is one apex vertex.
    let mut rings: Vec<Vec<u32>> = Vec::new();
    for level in 0..=sides {
        let t = level as f32 / sides as f32;
        let radius = cone.radius_top + (cone.radius_bottom - cone.radius_top) * t;
        let z = cone.bottom + cone.depth * (1.0 - t);
        let start = mesh.positions.len();
        if radius == 0.0 {
            mesh.positions.push(Vec3::new(0.0, 0.0, z));
            rings.push(vec![index(start); count]);
        } else {
            mesh.positions.extend(ring(count, radius, z));
            rings.push((start..start + count).map(index).collect());
        }
    }
    for pair in rings.windows(2) {
        let [upper, lower] = pair else { continue };
        for at in 0..count {
            let next = (at + 1) % count;
            let corners = [upper.get(at), lower.get(at), lower.get(next), upper.get(next)];
            let mut face: Vec<u32> = corners.into_iter().flatten().copied().collect();
            face.dedup();
            if face.first() == face.last() && face.len() > 1 {
                face.pop();
            }
            if face.len() >= 3 {
                mesh.faces.push(face);
                kinds.push(1);
            }
        }
    }
    // The caps: rings inward from the rim, then an n-gon or a fan.
    for (cap, radius, z, outer) in
        [(0, cone.radius_top, cone.bottom + cone.depth, rings.first().cloned()), (2, cone.radius_bottom, cone.bottom, rings.last().cloned())]
    {
        let Some(mut outer) = outer else { continue };
        if radius == 0.0 || cone.fill == Fill::None {
            continue;
        }
        let up = cap == 0;
        let inner_rings = if cone.fill == Fill::TriangleFan { fills } else { fills - 1 };
        for step in 1..=inner_rings {
            let factor = 1.0 - step as f32 / fills as f32;
            let start = mesh.positions.len();
            let inner: Vec<u32> = if factor <= 0.0 {
                mesh.positions.push(Vec3::new(0.0, 0.0, z));
                vec![index(start); count]
            } else {
                mesh.positions.extend(ring(count, radius * factor, z));
                (start..start + count).map(index).collect()
            };
            for at in 0..count {
                let next = (at + 1) % count;
                let corners = [outer.get(at), outer.get(next), inner.get(next), inner.get(at)];
                let mut face: Vec<u32> = corners.into_iter().flatten().copied().collect();
                face.dedup();
                if face.first() == face.last() && face.len() > 1 {
                    face.pop();
                }
                if !up {
                    face.reverse();
                }
                if face.len() >= 3 {
                    mesh.faces.push(face);
                    kinds.push(cap);
                }
            }
            outer = inner;
        }
        if cone.fill == Fill::Ngon {
            let mut face = outer;
            if !up {
                face.reverse();
            }
            mesh.faces.push(face);
            kinds.push(cap);
        }
    }
    let mut component = MeshComponent::new(mesh);
    for (slot, name) in names.iter().enumerate() {
        if name.is_empty() {
            continue;
        }
        let values = kinds.iter().map(|kind| Scalar::Bool(*kind == [0, 1, 2].get(slot).copied().unwrap_or(9))).collect();
        component.attributes.insert((*name).to_owned(), Attribute { domain: Domain::Face, values });
    }
    Ok(component)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outward(mesh: &Mesh) -> bool {
        let centre = mesh.positions.iter().sum::<Vec3>() / mesh.positions.len() as f32;
        mesh.faces.iter().all(|face| mesh.face_normal(face).dot(mesh.face_center(face) - centre) > -1e-4)
    }

    #[test]
    fn cubes_count_their_lattice_surface() {
        let mesh = cube(Vec3::ONE, [3, 4, 3]).unwrap();
        assert_eq!((mesh.positions.len(), mesh.faces.len()), (34, 32));
        assert!(outward(&mesh));
        let flat = cube(Vec3::ONE, [3, 3, 1]).unwrap();
        assert_eq!((flat.positions.len(), flat.faces.len()), (9, 4));
    }

    #[test]
    fn cones_and_cylinders_match_the_reference_counts() {
        let cylinder = |side, fill_segments, fill| Cone {
            vertices: 32,
            side_segments: side,
            fill_segments,
            radius_top: 1.0,
            radius_bottom: 1.0,
            depth: 2.0,
            fill,
            bottom: -1.0,
        };
        let count = |cone: &Cone| {
            let component = super::cone(cone, ["", "", ""]).unwrap();
            assert!(outward(&component.mesh));
            (component.mesh.positions.len(), component.mesh.faces.len())
        };
        assert_eq!(count(&cylinder(1, 1, Fill::Ngon)), (64, 34));
        assert_eq!(count(&cylinder(3, 2, Fill::Ngon)), (192, 162));
        assert_eq!(count(&cylinder(1, 2, Fill::TriangleFan)), (130, 160));
        assert_eq!(count(&cylinder(1, 1, Fill::None)), (64, 32));
        let cone = |top, side, fill_segments, fill| Cone {
            vertices: 32,
            side_segments: side,
            fill_segments,
            radius_top: top,
            radius_bottom: 1.0,
            depth: 2.0,
            fill,
            bottom: 0.0,
        };
        assert_eq!(count(&cone(0.0, 1, 1, Fill::Ngon)), (33, 33));
        assert_eq!(count(&cone(0.5, 2, 3, Fill::Ngon)), (224, 194));
        assert_eq!(count(&cone(0.0, 1, 1, Fill::TriangleFan)), (34, 64));
    }

    #[test]
    fn grids_lines_and_circles() {
        let flat = grid(2.0, 1.0, 5, 4).unwrap();
        assert_eq!((flat.positions.len(), flat.faces.len(), flat.edges().len()), (20, 12, 31));
        assert_eq!(grid(1.0, 1.0, 5, 1).unwrap().loose_edges.len(), 4);
        let mesh = line(5, Vec3::ZERO, Vec3::Z, true).unwrap();
        assert!((mesh.bounds(glam::Mat4::IDENTITY).max.z - 1.0).abs() < 1e-6);
        assert_eq!(circle(32, 1.0, Fill::TriangleFan).unwrap().faces.len(), 32);
    }
}
