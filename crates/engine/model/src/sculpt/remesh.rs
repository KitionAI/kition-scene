//! Voxel remeshing: the mesh's surface rebuilt as an even quad mesh at a chosen voxel size, as
//! the reference application's Voxel Remesh does. The surface is sampled on a grid of points
//! (the distance to the nearest triangle near the surface, signed by that triangle's normal, and
//! inside or outside by a flood fill from the grid's border elsewhere), and surface nets put one
//! vertex in every cell the surface crosses and one quad across every grid edge it crosses.

use std::collections::VecDeque;

use glam::Vec3;

use crate::error::{Result, SceneError};
use crate::mesh::{Bounds, Mesh};

/// The most grid points a remesh may sample.
pub const MAX_POINTS: u64 = 12_000_000;

/// The closest point to `p` on triangle `a b c`.
fn closest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

struct Lattice {
    origin: Vec3,
    size: f32,
    dims: [usize; 3],
}

impl Lattice {
    fn index(&self, x: usize, y: usize, z: usize) -> usize {
        (z * self.dims[1] + y) * self.dims[0] + x
    }

    fn point(&self, x: usize, y: usize, z: usize) -> Vec3 {
        self.origin + Vec3::new(x as f32, y as f32, z as f32) * self.size
    }

    fn count(&self) -> usize {
        self.dims[0] * self.dims[1] * self.dims[2]
    }
}

/// The surface of `mesh` (its closed parts) rebuilt at `voxel` size. Keeps the shading flag;
/// texture coordinates, groups, keys, seams, and the mask do not carry over.
pub fn voxel_remesh(mesh: &Mesh, voxel: f32) -> Result<Mesh> {
    if !(voxel.is_finite() && voxel > 0.0) {
        return Err(SceneError::params("the voxel size must be a positive number"));
    }
    let bounds = mesh.positions.iter().fold(Bounds::EMPTY, |bounds, point| bounds.with(*point));
    if bounds.is_empty() || mesh.faces.is_empty() {
        return Err(SceneError::params("the mesh has no faces to remesh"));
    }
    let pad = voxel * 2.0;
    let origin = bounds.min - Vec3::splat(pad);
    let extent = bounds.size() + Vec3::splat(pad * 2.0);
    let dims_f = (extent / voxel).ceil() + Vec3::ONE;
    let total = f64::from(dims_f.x) * f64::from(dims_f.y) * f64::from(dims_f.z);
    if !total.is_finite() || total > MAX_POINTS as f64 {
        return Err(SceneError::params(format!(
            "a voxel size of {voxel} needs {total:.0} samples; the most is {MAX_POINTS} (use a larger voxel size)"
        )));
    }
    let lattice = Lattice { origin, size: voxel, dims: [dims_f.x as usize, dims_f.y as usize, dims_f.z as usize] };
    let count = lattice.count();
    // Distances near the surface, signed by the nearest triangle's side.
    let band = voxel * 1.5;
    let mut distance = vec![f32::INFINITY; count];
    let mut sign = vec![0i8; count];
    let triangles = mesh.triangles();
    for (triangle, _) in &triangles {
        let [ia, ib, ic] = *triangle;
        let (Some(&a), Some(&b), Some(&c)) = (mesh.positions.get(ia as usize), mesh.positions.get(ib as usize), mesh.positions.get(ic as usize))
        else {
            continue;
        };
        let normal = (b - a).cross(c - a).normalize_or_zero();
        if normal == Vec3::ZERO {
            continue;
        }
        let low = (a.min(b).min(c) - Vec3::splat(band) - origin) / voxel;
        let high = (a.max(b).max(c) + Vec3::splat(band) - origin) / voxel;
        let clamp = |value: f32, axis: usize| (value.max(0.0) as usize).min(lattice.dims.get(axis).copied().unwrap_or(1).saturating_sub(1));
        for z in clamp(low.z.floor(), 2)..=clamp(high.z.ceil(), 2) {
            for y in clamp(low.y.floor(), 1)..=clamp(high.y.ceil(), 1) {
                for x in clamp(low.x.floor(), 0)..=clamp(high.x.ceil(), 0) {
                    let point = lattice.point(x, y, z);
                    let nearest = closest_on_triangle(point, a, b, c);
                    let gap = point.distance(nearest);
                    let slot = lattice.index(x, y, z);
                    if let Some(stored) = distance.get_mut(slot)
                        && gap < *stored
                    {
                        *stored = gap;
                        if let Some(side) = sign.get_mut(slot) {
                            *side = if (point - nearest).dot(normal) >= 0.0 { 1 } else { -1 };
                        }
                    }
                }
            }
        }
    }
    // Away from the surface: outside where the border reaches without crossing it.
    let mut outside = vec![false; count];
    let mut queue = VecDeque::new();
    let [nx, ny, nz] = lattice.dims;
    let near = |slot: usize| distance.get(slot).is_some_and(|gap| *gap <= band);
    for z in 0..nz {
        for y in 0..ny {
            for x in 0..nx {
                let border = x == 0 || y == 0 || z == 0 || x + 1 == nx || y + 1 == ny || z + 1 == nz;
                let slot = lattice.index(x, y, z);
                if border
                    && !near(slot)
                    && let Some(flag) = outside.get_mut(slot)
                {
                    *flag = true;
                    queue.push_back((x, y, z));
                }
            }
        }
    }
    while let Some((x, y, z)) = queue.pop_front() {
        let steps: [(isize, isize, isize); 6] = [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)];
        for (dx, dy, dz) in steps {
            let (Some(px), Some(py), Some(pz)) = (x.checked_add_signed(dx), y.checked_add_signed(dy), z.checked_add_signed(dz)) else { continue };
            if px >= nx || py >= ny || pz >= nz {
                continue;
            }
            let slot = lattice.index(px, py, pz);
            if near(slot) || outside.get(slot).copied().unwrap_or(true) {
                continue;
            }
            if let Some(flag) = outside.get_mut(slot) {
                *flag = true;
            }
            queue.push_back((px, py, pz));
        }
    }
    let field: Vec<f32> = (0..count)
        .map(|slot| {
            let gap = distance.get(slot).copied().unwrap_or(f32::INFINITY);
            if gap <= band {
                gap * f32::from(sign.get(slot).copied().unwrap_or(1))
            } else if outside.get(slot).copied().unwrap_or(true) {
                band
            } else {
                -band
            }
        })
        .collect();
    let value = |x: usize, y: usize, z: usize| field.get(lattice.index(x, y, z)).copied().unwrap_or(band);
    // One vertex per cell the surface crosses, at the mean of its edges' crossings.
    let cells = [nx.saturating_sub(1), ny.saturating_sub(1), nz.saturating_sub(1)];
    let cell_index = |x: usize, y: usize, z: usize| (z * cells[1] + y) * cells[0] + x;
    let mut vertex_of = vec![u32::MAX; cells[0] * cells[1] * cells[2]];
    let mut out = Mesh { smooth: mesh.smooth, smooth_angle: mesh.smooth_angle, ..Mesh::default() };
    const CORNERS: [(usize, usize, usize); 8] = [(0, 0, 0), (1, 0, 0), (0, 1, 0), (1, 1, 0), (0, 0, 1), (1, 0, 1), (0, 1, 1), (1, 1, 1)];
    const EDGES: [(usize, usize); 12] = [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)];
    for z in 0..cells[2] {
        for y in 0..cells[1] {
            for x in 0..cells[0] {
                let values = CORNERS.map(|(dx, dy, dz)| value(x + dx, y + dy, z + dz));
                let inside = values.iter().filter(|value| **value < 0.0).count();
                if inside == 0 || inside == 8 {
                    continue;
                }
                let mut sum = Vec3::ZERO;
                let mut crossings: f32 = 0.0;
                for (a, b) in EDGES {
                    let (Some(&va), Some(&vb)) = (values.get(a), values.get(b)) else { continue };
                    if (va < 0.0) == (vb < 0.0) {
                        continue;
                    }
                    let (Some(&(ax, ay, az)), Some(&(bx, by, bz))) = (CORNERS.get(a), CORNERS.get(b)) else { continue };
                    let t = (va / (va - vb)).clamp(0.0, 1.0);
                    let pa = lattice.point(x + ax, y + ay, z + az);
                    let pb = lattice.point(x + bx, y + by, z + bz);
                    sum += pa.lerp(pb, t);
                    crossings += 1.0;
                }
                if let Some(slot) = vertex_of.get_mut(cell_index(x, y, z)) {
                    *slot = out.vertex_count();
                }
                out.positions.push(sum / crossings.max(1.0));
            }
        }
    }
    // One quad across every grid edge with a sign change, between the four cells round it.
    let cell_vertex = |x: isize, y: isize, z: isize| -> Option<u32> {
        if x < 0 || y < 0 || z < 0 {
            return None;
        }
        let (x, y, z) = (x as usize, y as usize, z as usize);
        if x >= cells[0] || y >= cells[1] || z >= cells[2] {
            return None;
        }
        vertex_of.get(cell_index(x, y, z)).copied().filter(|vertex| *vertex != u32::MAX)
    };
    for z in 0..nz {
        for y in 0..ny {
            for x in 0..nx {
                let here = value(x, y, z) < 0.0;
                let (xi, yi, zi) = (x as isize, y as isize, z as isize);
                // Along X: the cells around the edge differ in Y and Z.
                if x + 1 < nx && here != (value(x + 1, y, z) < 0.0) {
                    let quad = [cell_vertex(xi, yi - 1, zi - 1), cell_vertex(xi, yi, zi - 1), cell_vertex(xi, yi, zi), cell_vertex(xi, yi - 1, zi)];
                    push_quad(&mut out, quad, here);
                }
                if y + 1 < ny && here != (value(x, y + 1, z) < 0.0) {
                    let quad = [cell_vertex(xi - 1, yi, zi - 1), cell_vertex(xi - 1, yi, zi), cell_vertex(xi, yi, zi), cell_vertex(xi, yi, zi - 1)];
                    push_quad(&mut out, quad, here);
                }
                if z + 1 < nz && here != (value(x, y, z + 1) < 0.0) {
                    let quad = [cell_vertex(xi - 1, yi - 1, zi), cell_vertex(xi, yi - 1, zi), cell_vertex(xi, yi, zi), cell_vertex(xi - 1, yi, zi)];
                    push_quad(&mut out, quad, here);
                }
            }
        }
    }
    if out.faces.is_empty() {
        return Err(SceneError::params("the remesh found no closed surface at this voxel size"));
    }
    out.remove_unused_vertices();
    crate::ops::recalculate_outside(&mut out);
    out.validate()?;
    Ok(out)
}

fn push_quad(mesh: &mut Mesh, quad: [Option<u32>; 4], inside_first: bool) {
    let [Some(a), Some(b), Some(c), Some(d)] = quad else { return };
    let mut face = vec![a, b, c, d];
    if !inside_first {
        face.reverse();
    }
    let distinct: std::collections::BTreeSet<_> = face.iter().collect();
    if distinct.len() == 4 {
        mesh.faces.push(face);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modifiers::boundary_edges;
    use crate::primitives::Primitive;

    #[test]
    fn a_cube_remeshes_to_a_closed_quad_surface_of_the_same_size() {
        let cube = Primitive::Cube { size: 2.0 }.build().unwrap();
        let out = voxel_remesh(&cube, 0.1).unwrap();
        assert!(out.faces.iter().all(|face| face.len() == 4));
        assert!(boundary_edges(&out).is_empty(), "closed");
        let size = out.bounds(glam::Mat4::IDENTITY).size();
        assert!((size - Vec3::splat(2.0)).abs().max_element() < 0.1, "{size}");
        // About (2 / 0.1)² quads per side.
        assert!((1800..3000).contains(&out.faces.len()), "{}", out.faces.len());
    }

    #[test]
    fn a_sphere_stays_round() {
        let sphere = Primitive::UvSphere { segments: 32, rings: 16, radius: 1.0 }.build().unwrap();
        let out = voxel_remesh(&sphere, 0.05).unwrap();
        for point in &out.positions {
            assert!((point.length() - 1.0).abs() < 0.05, "{point}");
        }
        let mut check = out.clone();
        crate::ops::recalculate_outside(&mut check);
        assert_eq!(check.faces, out.faces);
    }

    #[test]
    fn too_fine_a_voxel_is_refused() {
        let cube = Primitive::Cube { size: 2.0 }.build().unwrap();
        assert!(voxel_remesh(&cube, 0.0005).is_err());
        assert!(voxel_remesh(&cube, -1.0).is_err());
    }
}
