//! Wavefront OBJ: positions, texture coordinates, n-gon faces, lines, objects, and material
//! names (no `.mtl`).
//! Written Y-up (x, z, -y) in world space, as the common modeller's default exporter does.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use kition_scene_model::glam::{Vec2, Vec3};
use kition_scene_model::{Id, Material, Mesh, ObjectData, Scene, SceneError};

use crate::{Imported, ImportedObject, MAX_ITEMS, chosen, from_y_up, invalid, to_y_up};

pub fn export(scene: &Scene, only: Option<&[Id]>) -> Result<Vec<u8>, SceneError> {
    let mut out = String::from("# Kition Scene\n");
    let mut offset = 1u64;
    let mut uv_offset = 1u64;
    for id in chosen(scene, only, false)? {
        let object = scene.object(id)?;
        if object.hidden {
            continue;
        }
        let Some(mut mesh) = object.evaluated_mesh(scene) else { continue };
        mesh.transform(scene.world_matrix(id)?);
        let _ = writeln!(out, "o {}", one_line(&object.name));
        for point in &mesh.positions {
            let point = to_y_up(*point);
            let _ = writeln!(out, "v {:.6} {:.6} {:.6}", point.x, point.y, point.z);
        }
        // One texture coordinate per face corner, in face order.
        let uvs = mesh.has_uvs() && mesh.uvs_aligned();
        if uvs {
            for uv in mesh.uvs.iter().flatten() {
                let _ = writeln!(out, "vt {:.6} {:.6}", uv.x, uv.y);
            }
        }
        let _ = writeln!(out, "s {}", if mesh.smooth { "1" } else { "0" });
        if let Some(material) = object.material.and_then(|material| scene.material(material).ok()) {
            let _ = writeln!(out, "usemtl {}", one_line(&material.name));
        }
        let mut corner = uv_offset;
        for face in &mesh.faces {
            out.push('f');
            for index in face {
                if uvs {
                    let _ = write!(out, " {}/{}", u64::from(*index) + offset, corner);
                    corner += 1;
                } else {
                    let _ = write!(out, " {}", u64::from(*index) + offset);
                }
            }
            out.push('\n');
        }
        uv_offset = corner;
        for [a, b] in &mesh.loose_edges {
            let _ = writeln!(out, "l {} {}", u64::from(*a) + offset, u64::from(*b) + offset);
        }
        offset += mesh.positions.len() as u64;
    }
    Ok(out.into_bytes())
}

fn one_line(name: &str) -> String {
    name.chars().map(|character| if character.is_control() { ' ' } else { character }).collect::<String>().trim().replace(' ', "_")
}

#[derive(Default)]
struct Group {
    name: String,
    faces: Vec<Vec<u32>>,
    /// Each face's corners' texture coordinate indices, where the file gives them.
    face_uvs: Vec<Vec<Option<u32>>>,
    lines: Vec<[u32; 2]>,
    material: Option<String>,
    smooth: bool,
}

/// The texture coordinate index of a face corner (`v/vt`, `v/vt/vn`), if it names one that is
/// there (files that name coordinates they do not have still open, without them).
fn uv_index(token: &str, count: usize, line: usize) -> Result<Option<u32>, SceneError> {
    match token.split('/').nth(1) {
        Some(part) if !part.is_empty() => Ok(resolve(part, count, line).ok()),
        _ => Ok(None),
    }
}

/// A 1-based (or negative, relative) OBJ index as a 0-based one.
fn index(token: &str, count: usize, line: usize) -> Result<u32, SceneError> {
    resolve(token.split('/').next().unwrap_or(""), count, line)
}

fn resolve(token: &str, count: usize, line: usize) -> Result<u32, SceneError> {
    let value: i64 = token.parse().map_err(|_| invalid(format!("line {line}: `{token}` is not an index")))?;
    let count = i64::try_from(count).unwrap_or(i64::MAX);
    let resolved = if value < 0 { count + value } else { value - 1 };
    if resolved < 0 || resolved >= count {
        return Err(invalid(format!("line {line}: item {value} is not there")));
    }
    u32::try_from(resolved).map_err(|_| invalid(format!("line {line}: vertex {value} is out of range")))
}

pub fn import(bytes: &[u8]) -> Result<Imported, SceneError> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("the OBJ file is not UTF-8 text"))?;
    let uses_objects = text.lines().any(|line| line.trim_start().starts_with("o ") || line.trim() == "o");
    let mut positions: Vec<Vec3> = Vec::new();
    let mut coordinates: Vec<Vec2> = Vec::new();
    let mut groups: Vec<Group> = vec![Group { name: "Object".into(), ..Default::default() }];
    let mut items = 0usize;
    for (number, line) in text.lines().enumerate() {
        let number = number + 1;
        let line = line.split('#').next().unwrap_or("").trim();
        let mut tokens = line.split_whitespace();
        let Some(keyword) = tokens.next() else { continue };
        items += 1;
        if items > MAX_ITEMS {
            return Err(invalid("the OBJ file is too large"));
        }
        let current = groups.len() - 1;
        match keyword {
            "v" => {
                let values: Vec<f32> = tokens
                    .take(3)
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .map_err(|_| invalid(format!("line {number}: a vertex is not three numbers")))?;
                match values.as_slice() {
                    [x, y, z] if x.is_finite() && y.is_finite() && z.is_finite() => positions.push(from_y_up(Vec3::new(*x, *y, *z))),
                    _ => return Err(invalid(format!("line {number}: a vertex is not three numbers"))),
                }
            }
            "vt" => {
                let values: Vec<f32> = tokens
                    .take(2)
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .map_err(|_| invalid(format!("line {number}: a texture coordinate is not numbers")))?;
                match values.as_slice() {
                    [u, v] if u.is_finite() && v.is_finite() => coordinates.push(Vec2::new(*u, *v)),
                    [u] if u.is_finite() => coordinates.push(Vec2::new(*u, 0.0)),
                    _ => return Err(invalid(format!("line {number}: a texture coordinate is not numbers"))),
                }
            }
            "o" | "g" if (keyword == "o") == uses_objects => {
                let name = tokens.collect::<Vec<_>>().join(" ");
                let group = Group { name: if name.is_empty() { "Object".into() } else { name }, ..Default::default() };
                let last_empty = groups.last().is_some_and(|group| group.faces.is_empty() && group.lines.is_empty());
                if last_empty {
                    if let Some(last) = groups.last_mut() {
                        *last = group;
                    }
                } else {
                    groups.push(group);
                }
            }
            "f" => {
                let mut cleaned: Vec<u32> = Vec::new();
                let mut cleaned_uvs: Vec<Option<u32>> = Vec::new();
                for token in tokens {
                    let corner = index(token, positions.len(), number)?;
                    let uv = uv_index(token, coordinates.len(), number)?;
                    if !cleaned.contains(&corner) {
                        cleaned.push(corner);
                        cleaned_uvs.push(uv);
                    }
                }
                if cleaned.len() >= 3
                    && let Some(group) = groups.get_mut(current)
                {
                    group.faces.push(cleaned);
                    group.face_uvs.push(cleaned_uvs);
                }
            }
            "l" => {
                let points: Vec<u32> = tokens.map(|token| index(token, positions.len(), number)).collect::<Result<_, _>>()?;
                if let Some(group) = groups.get_mut(current) {
                    for pair in points.windows(2) {
                        if let [a, b] = pair
                            && a != b
                        {
                            group.lines.push([*a, *b]);
                        }
                    }
                }
            }
            "usemtl" => {
                if let Some(group) = groups.get_mut(current)
                    && group.material.is_none()
                {
                    group.material = Some(tokens.collect::<Vec<_>>().join(" "));
                }
            }
            "s" => {
                if let Some(group) = groups.get_mut(current) {
                    group.smooth = !matches!(tokens.next(), Some("off" | "0") | None);
                }
            }
            _ => {}
        }
    }
    let mut out = Imported::default();
    let mut material_index: BTreeMap<String, usize> = BTreeMap::new();
    for group in groups {
        if group.faces.is_empty() && group.lines.is_empty() {
            continue;
        }
        // Each object keeps only the vertices it uses, in first-use order.
        let mut remap: HashMap<u32, u32> = HashMap::new();
        let mut mesh = Mesh { smooth: group.smooth, ..Default::default() };
        let mut local = |index: u32, mesh: &mut Mesh| -> u32 {
            *remap.entry(index).or_insert_with(|| {
                mesh.positions.push(positions.get(index as usize).copied().unwrap_or(Vec3::ZERO));
                mesh.vertex_count() - 1
            })
        };
        for face in &group.faces {
            let corners = face.iter().map(|index| local(*index, &mut mesh)).collect();
            mesh.faces.push(corners);
        }
        if group.face_uvs.iter().flatten().any(Option::is_some) {
            mesh.uvs = group
                .face_uvs
                .iter()
                .map(|corners| corners.iter().map(|uv| uv.and_then(|uv| coordinates.get(uv as usize).copied()).unwrap_or(Vec2::ZERO)).collect())
                .collect();
        }
        for [a, b] in &group.lines {
            let edge = [local(*a, &mut mesh), local(*b, &mut mesh)];
            mesh.loose_edges.push(edge);
        }
        let material = group.material.map(|name| {
            *material_index.entry(name.clone()).or_insert_with(|| {
                out.materials.push(Material::new(0, name));
                out.materials.len() - 1
            })
        });
        out.objects.push(ImportedObject {
            name: group.name,
            data: ObjectData::Mesh(mesh),
            location: Vec3::ZERO,
            rotation: Vec3::ZERO,
            scale: Vec3::ONE,
            parent: None,
            parent_inverse: kition_scene_model::glam::Mat4::IDENTITY,
            material,
        });
    }
    if out.objects.is_empty() && positions.is_empty() {
        return Err(invalid("the OBJ file has no geometry"));
    }
    Ok(out)
}
