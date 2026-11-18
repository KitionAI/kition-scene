//! Tangent frames for normal maps, the way MikkTSpace builds them: every triangle's tangent
//! and bitangent come from how its texture coordinates run across it; corners that share a
//! vertex, a texture coordinate, and a normal (no seam between them) share a frame, the
//! triangles' contributions weighted by the corner's angle; the tangent is then made
//! perpendicular to the normal, and its sign says whether the bitangent is N × T or its
//! opposite (mirrored UVs).

use std::collections::HashMap;

use glam::{Vec2, Vec3, Vec4};

use crate::mesh::Mesh;

/// Corners sharing a frame: the vertex, and the bits of the texture coordinate and the normal.
type FrameKey = (u32, [u32; 2], [u32; 3]);

/// Per face, per corner: the tangent (xyz, unit, perpendicular to the corner's normal) and the
/// bitangent sign (w, ±1). Empty when the mesh has no texture coordinates.
pub fn corner_tangents(mesh: &Mesh) -> Vec<Vec<Vec4>> {
    if !mesh.has_uvs() || !mesh.uvs_aligned() {
        return Vec::new();
    }
    let face_normals: Vec<Vec3> = mesh.faces.iter().map(|face| mesh.face_normal(face)).collect();
    let vertex_normals = if mesh.smooth { mesh.vertex_normals() } else { Vec::new() };
    // Smooth by angle: corners have normals of their own.
    let split = if mesh.smooth && mesh.smooth_angle.is_some() { mesh.corner_normals() } else { Vec::new() };
    let normal_at = |face: usize, corner: usize, vertex: u32| -> Vec3 {
        if let Some(normal) = split.get(face).and_then(|normals| normals.get(corner)) {
            return *normal;
        }
        if mesh.smooth { vertex_normals.get(vertex as usize).copied().unwrap_or(Vec3::Z) } else { face_normals.get(face).copied().unwrap_or(Vec3::Z) }
    };
    // Corners that share a frame: same vertex, texture coordinate, and normal.
    let key = |face: usize, corner: usize, vertex: u32| -> FrameKey {
        let uv = mesh.corner_uv(face, corner);
        let normal = normal_at(face, corner, vertex);
        (vertex, [uv.x.to_bits(), uv.y.to_bits()], normal.to_array().map(|value| (value + 0.0).to_bits()))
    };
    let mut sums: HashMap<FrameKey, (Vec3, Vec3)> = HashMap::new();
    for (corners, face) in mesh.corner_triangles() {
        let face = face as usize;
        let Some(vertices) = mesh.faces.get(face) else { continue };
        let vertex = |corner: u32| vertices.get(corner as usize).copied().unwrap_or(0);
        let point = |corner: u32| mesh.positions.get(vertex(corner) as usize).copied().unwrap_or(Vec3::ZERO);
        let uv = |corner: u32| mesh.corner_uv(face, corner as usize);
        let [a, b, c] = corners;
        let (e1, e2) = (point(b) - point(a), point(c) - point(a));
        let (d1, d2): (Vec2, Vec2) = (uv(b) - uv(a), uv(c) - uv(a));
        let determinant = d1.x * d2.y - d2.x * d1.y;
        if determinant.abs() < 1e-20 || !determinant.is_finite() {
            continue;
        }
        let tangent = (e1 * d2.y - e2 * d1.y) / determinant;
        let bitangent = (e2 * d1.x - e1 * d2.x) / determinant;
        for (corner, (previous, next)) in [(a, (c, b)), (b, (a, c)), (c, (b, a))] {
            let weight = (point(previous) - point(corner)).angle_between(point(next) - point(corner));
            if !weight.is_finite() {
                continue;
            }
            let entry = sums.entry(key(face, corner as usize, vertex(corner))).or_insert((Vec3::ZERO, Vec3::ZERO));
            entry.0 += tangent.normalize_or_zero() * weight;
            entry.1 += bitangent.normalize_or_zero() * weight;
        }
    }
    mesh.faces
        .iter()
        .enumerate()
        .map(|(face, vertices)| {
            vertices
                .iter()
                .enumerate()
                .map(|(corner, vertex)| {
                    let normal = normal_at(face, corner, *vertex);
                    let (tangent, bitangent) = sums.get(&key(face, corner, *vertex)).copied().unwrap_or((Vec3::ZERO, Vec3::ZERO));
                    let mut tangent = (tangent - normal * normal.dot(tangent)).normalize_or_zero();
                    if tangent == Vec3::ZERO {
                        // No texture direction here: any perpendicular will do.
                        tangent = normal.any_orthonormal_vector();
                    }
                    let sign = if normal.cross(tangent).dot(bitangent) < 0.0 { -1.0 } else { 1.0 };
                    tangent.extend(sign)
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::Primitive;

    #[test]
    fn a_plane_tangent_follows_u() {
        let plane = Primitive::Plane { size: 2.0 }.build().unwrap();
        let tangents = corner_tangents(&plane);
        for tangent in tangents.iter().flatten() {
            assert!(tangent.truncate().abs_diff_eq(Vec3::X, 1e-5), "{tangent}");
            assert_eq!(tangent.w, 1.0);
        }
    }

    #[test]
    fn mirrored_coordinates_flip_the_sign() {
        let mut plane = Primitive::Plane { size: 2.0 }.build().unwrap();
        for uv in plane.uvs.iter_mut().flatten() {
            uv.y = 1.0 - uv.y;
        }
        assert!(corner_tangents(&plane).iter().flatten().all(|tangent| tangent.w == -1.0));
    }

    #[test]
    fn every_tangent_is_perpendicular_to_its_normal() {
        let sphere = Primitive::UvSphere { segments: 12, rings: 6, radius: 1.0 }.build().unwrap();
        let normals = sphere.vertex_normals();
        let tangents = corner_tangents(&sphere);
        for (face, list) in tangents.iter().enumerate() {
            for (corner, tangent) in list.iter().enumerate() {
                let normal = normals[sphere.faces[face][corner] as usize];
                assert!(tangent.truncate().dot(normal).abs() < 1e-4);
                assert!((tangent.truncate().length() - 1.0).abs() < 1e-4);
            }
        }
    }
}
