//! `check()`: what is probably wrong with a scene, errors first, each with a stable
//! `cube.<name>` code, the object it is about, and the command that fixes it where one does.

use glam::{Mat4, Vec3};
use kition_kit::diagnostics::{Diagnostic, Severity, Target};
use kition_scene_model::{Mesh, ObjectData, Scene, ops};
use serde_json::json;

fn on(id: u32) -> Target {
    Target { object_id: Some(id.to_string()), ..Default::default() }
}

/// The signed volume of a mesh (positive when its faces point outwards).
fn volume(mesh: &Mesh) -> f32 {
    mesh.faces
        .iter()
        .map(|face| {
            let points = mesh.face_positions(face);
            let first = points.first().copied().unwrap_or(Vec3::ZERO);
            points.windows(2).skip(1).map(|pair| if let [b, c] = pair { first.dot(b.cross(*c)) } else { 0.0 }).sum::<f32>()
        })
        .sum::<f32>()
        / 6.0
}

/// The scene's diagnostics.
pub fn check(scene: &Scene) -> Vec<Diagnostic> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut notes = Vec::new();
    for object in &scene.objects {
        if object.scale.to_array().iter().any(|value| value.abs() < 1e-8) {
            errors.push(
                Diagnostic::new(
                    "scene.zeroScale",
                    Severity::Error,
                    format!("{} has a scale of zero on an axis, so it is flat or invisible", object.name),
                )
                .on(on(object.id))
                .fix("object.transform", Some(json!({ "objects": object.id, "scale": [1, 1, 1] }))),
            );
        }
        if let Some(error) = kition_scene_model::modifiers::nodes_error(object, scene) {
            errors.push(
                Diagnostic::new("scene.nodesError", Severity::Error, format!("{}'s geometry nodes cannot run: {error}", object.name))
                    .on(on(object.id)),
            );
        }
        let ObjectData::Mesh(mesh) = &object.data else { continue };
        if mesh.positions.is_empty() {
            warnings.push(Diagnostic::new("scene.emptyMesh", Severity::Warning, format!("{} has no geometry", object.name)).on(on(object.id)));
            continue;
        }
        let degenerate =
            mesh.faces.iter().filter(|face| kition_scene_model::mesh::newell_vector(&mesh.face_positions(face)).length() < 1e-10).count();
        if degenerate > 0 {
            warnings.push(
                Diagnostic::new("scene.degenerateFaces", Severity::Warning, format!("{} has {degenerate} faces with no area", object.name))
                    .on(on(object.id)),
            );
        }
        let closed = !mesh.faces.is_empty() && kition_scene_model::modifiers::boundary_edges(mesh).is_empty();
        if closed && volume(mesh) < 0.0 {
            warnings.push(
                Diagnostic::new(
                    "scene.insideOut",
                    Severity::Warning,
                    format!("{}'s faces point inwards, so it shades and exports inside out", object.name),
                )
                .on(on(object.id))
                .fix("mesh.recalculateNormals", Some(json!({ "objects": object.id }))),
            );
        } else if closed {
            let mut oriented = mesh.clone();
            ops::recalculate_outside(&mut oriented);
            if oriented.faces != mesh.faces {
                warnings.push(
                    Diagnostic::new(
                        "scene.mixedNormals",
                        Severity::Warning,
                        format!("Some of {}'s faces point the other way from their neighbours", object.name),
                    )
                    .on(on(object.id))
                    .fix("mesh.recalculateNormals", Some(json!({ "objects": object.id }))),
                );
            }
        }
    }
    let visible_meshes: Vec<_> = scene.objects.iter().filter(|object| !object.hidden && object.has_geometry()).collect();
    match scene.camera {
        None if !visible_meshes.is_empty() => warnings.push(
            Diagnostic::new("scene.noCamera", Severity::Warning, "The scene has no camera, so it cannot be rendered")
                .fix("object.addCamera", Some(json!({ "location": [7.36, -6.93, 4.96], "rotation": [63.6, 0, 46.7] }))),
        ),
        Some(camera) => {
            if let Some(view) = kition_scene_raster::view::camera_view(scene, camera) {
                let projector = view.fit(scene.render.width, scene.render.height, scene.render.width, scene.render.height);
                for object in &visible_meshes {
                    let Ok(world) = scene.world_matrix(object.id) else { continue };
                    let Some(mesh) = object.evaluated_mesh(scene) else { continue };
                    if !in_view(&projector, &mesh, world) {
                        notes.push(
                            Diagnostic::new(
                                "scene.outOfShot",
                                Severity::Info,
                                format!("{} is outside the camera's view, so renders will not show it", object.name),
                            )
                            .on(on(object.id))
                            .fix("object.lookAt", Some(json!({ "object": camera, "target": world.transform_point3(Vec3::ZERO).to_array() }))),
                        );
                    }
                }
            }
        }
        None => {}
    }
    let lights = scene.objects.iter().any(|object| !object.hidden && matches!(object.data, ObjectData::Light(_)));
    if !lights && !visible_meshes.is_empty() && scene.world.strength * scene.world.color.max_element() < 0.2 {
        warnings.push(
            Diagnostic::new("scene.noLight", Severity::Warning, "Nothing lights the scene, so renders come out dark")
                .fix("object.addLight", Some(json!({ "kind": "sun", "rotation": [40, 0, 30] }))),
        );
    }
    for object in &scene.objects {
        let textured = object.material.and_then(|id| scene.material(id).ok()).is_some_and(|material| material.images().next().is_some());
        if textured && object.mesh().is_some_and(|mesh| !mesh.faces.is_empty() && !mesh.has_uvs()) {
            warnings.push(
                Diagnostic::new(
                    "scene.textureWithoutUvs",
                    Severity::Warning,
                    format!("{} has an image material but no texture coordinates, so the image shows as one colour", object.name),
                )
                .on(on(object.id))
                .fix("uv.smartProject", Some(json!({ "objects": object.id }))),
            );
        }
    }
    for material in &scene.materials {
        if !scene.objects.iter().any(|object| object.material == Some(material.id)) {
            notes.push(
                Diagnostic::new("scene.unusedMaterial", Severity::Info, format!("Material {} is not used by any object", material.name))
                    .fix("material.delete", Some(json!({ "material": material.id }))),
            );
        }
    }
    errors.extend(warnings);
    errors.extend(notes);
    errors
}

/// Whether any vertex of the mesh projects inside the camera's frame (a cheap test that errs
/// towards "in view" for meshes larger than the frame).
fn in_view(projector: &kition_scene_raster::Projector, mesh: &Mesh, world: Mat4) -> bool {
    let matrix = projector.view_projection() * world;
    let bounds = mesh.bounds(Mat4::IDENTITY);
    let corners = (0..8).map(|bits| {
        Vec3::new(
            if bits & 1 == 0 { bounds.min.x } else { bounds.max.x },
            if bits & 2 == 0 { bounds.min.y } else { bounds.max.y },
            if bits & 4 == 0 { bounds.min.z } else { bounds.max.z },
        )
    });
    let (mut left, mut right, mut below, mut above, mut behind) = (true, true, true, true, true);
    for corner in corners {
        let clip = matrix * corner.extend(1.0);
        left &= clip.x < -clip.w;
        right &= clip.x > clip.w;
        below &= clip.y < -clip.w;
        above &= clip.y > clip.w;
        behind &= clip.z < 0.0;
    }
    !(left || right || below || above || behind)
}
