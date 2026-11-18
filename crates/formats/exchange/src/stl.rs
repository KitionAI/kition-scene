//! STL: triangles in world space, Z-up. Binary is written; binary and ASCII are read, with
//! exactly coincident corners joined into one mesh.

use kition_scene_model::glam::Vec3;
use kition_scene_model::{Id, Mesh, ObjectData, Scene, SceneError};

use crate::{Imported, ImportedObject, MAX_ITEMS, chosen, invalid, weld_exact, welded_triangles};

pub fn export(scene: &Scene, only: Option<&[Id]>) -> Result<Vec<u8>, SceneError> {
    let mut triangles: Vec<[Vec3; 4]> = Vec::new();
    for id in chosen(scene, only, false)? {
        let object = scene.object(id)?;
        if object.hidden {
            continue;
        }
        let Some(mut mesh) = object.evaluated_mesh(scene) else { continue };
        mesh.transform(scene.world_matrix(id)?);
        for ([a, b, c], _) in mesh.triangles() {
            let point = |index: u32| mesh.positions.get(index as usize).copied().unwrap_or(Vec3::ZERO);
            let (pa, pb, pc) = (point(a), point(b), point(c));
            triangles.push([(pb - pa).cross(pc - pa).normalize_or_zero(), pa, pb, pc]);
        }
    }
    let count = u32::try_from(triangles.len()).map_err(|_| invalid("too many triangles for STL"))?;
    let mut out = Vec::with_capacity(84 + triangles.len() * 50);
    let mut header = [b' '; 80];
    for (slot, byte) in header.iter_mut().zip(b"Kition Scene STL") {
        *slot = *byte;
    }
    out.extend(header);
    out.extend(count.to_le_bytes());
    for triangle in triangles {
        for vector in triangle {
            for value in vector.to_array() {
                out.extend(value.to_le_bytes());
            }
        }
        out.extend(0u16.to_le_bytes());
    }
    Ok(out)
}

fn f32_at(bytes: &[u8], at: usize) -> Option<f32> {
    let slice = bytes.get(at..at + 4)?;
    Some(f32::from_le_bytes([*slice.first()?, *slice.get(1)?, *slice.get(2)?, *slice.get(3)?]))
}

fn binary(bytes: &[u8]) -> Option<Result<Vec<Vec3>, SceneError>> {
    let count = u32::from_le_bytes([*bytes.get(80)?, *bytes.get(81)?, *bytes.get(82)?, *bytes.get(83)?]) as usize;
    if Some(bytes.len()) != count.checked_mul(50).and_then(|size| size.checked_add(84)) {
        return None;
    }
    if count > MAX_ITEMS {
        return Some(Err(invalid("the STL file has too many triangles")));
    }
    let mut corners = Vec::with_capacity(count * 3);
    for triangle in 0..count {
        let base = 84 + triangle * 50 + 12;
        for corner in 0..3 {
            let at = base + corner * 12;
            match (f32_at(bytes, at), f32_at(bytes, at + 4), f32_at(bytes, at + 8)) {
                (Some(x), Some(y), Some(z)) if x.is_finite() && y.is_finite() && z.is_finite() => corners.push(Vec3::new(x, y, z)),
                _ => return Some(Err(invalid(format!("triangle {triangle} has a corner that is not a finite number")))),
            }
        }
    }
    Some(Ok(corners))
}

fn ascii(bytes: &[u8]) -> Result<Vec<Vec3>, SceneError> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("not an STL file"))?;
    if !text.trim_start().starts_with("solid") {
        return Err(invalid("not an STL file"));
    }
    let mut corners = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let mut tokens = line.split_whitespace();
        if tokens.next() != Some("vertex") {
            continue;
        }
        let values: Vec<f32> = tokens
            .take(3)
            .map(str::parse)
            .collect::<Result<_, _>>()
            .map_err(|_| invalid(format!("line {}: a vertex is not three numbers", number + 1)))?;
        match values.as_slice() {
            [x, y, z] if x.is_finite() && y.is_finite() && z.is_finite() => corners.push(Vec3::new(*x, *y, *z)),
            _ => return Err(invalid(format!("line {}: a vertex is not three numbers", number + 1))),
        }
        if corners.len() > MAX_ITEMS {
            return Err(invalid("the STL file is too large"));
        }
    }
    if corners.len() % 3 != 0 {
        return Err(invalid("the STL file has a triangle with missing corners"));
    }
    Ok(corners)
}

pub fn import(bytes: &[u8]) -> Result<Imported, SceneError> {
    let corners = match binary(bytes) {
        Some(result) => result?,
        None => ascii(bytes)?,
    };
    let (positions, remap) = weld_exact(&corners);
    let triangles: Vec<[u32; 3]> = (0..corners.len() / 3)
        .map(|triangle| {
            let base = u32::try_from(triangle * 3).unwrap_or(u32::MAX);
            [base, base + 1, base + 2]
        })
        .collect();
    let mut mesh = Mesh { positions, faces: welded_triangles(&triangles, &remap), ..Default::default() };
    mesh.remove_unused_vertices();
    Ok(Imported {
        objects: vec![ImportedObject {
            name: "STL".into(),
            data: ObjectData::Mesh(mesh),
            location: Vec3::ZERO,
            rotation: Vec3::ZERO,
            scale: Vec3::ONE,
            parent: None,
            parent_inverse: kition_scene_model::glam::Mat4::IDENTITY,
            material: None,
        }],
        ..Imported::default()
    })
}
