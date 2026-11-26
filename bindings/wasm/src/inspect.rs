//! `inspect()`: the scene as an agent or the editor's panels read it. Rotations are in degrees;
//! `selection`, `view`, `mode`, and `tool` describe the session, not the file.

use glam::Mat4;
use kition_scene_model::{LightKind, ObjectData};
use kition_scene_raster::Shading;
use serde_json::{Map, Value, json};

use crate::{Mode, SceneDocument, SelectMode};

fn round(value: f32) -> f64 {
    (f64::from(value) * 10_000.0).round() / 10_000.0
}

fn triple(value: glam::Vec3) -> Value {
    json!([round(value.x), round(value.y), round(value.z)])
}

fn color(value: [f32; 4]) -> Value {
    json!([round(value[0]), round(value[1]), round(value[2]), round(value[3])])
}

/// The document as JSON.
/// An object's mesh after its modifiers, from the viewport's cache: evaluated again only when the
/// object or what it depends on changed (an inspect after every pointer event would otherwise
/// evaluate every modifier of the scene each time).
fn evaluated(doc: &SceneDocument, object: &kition_scene_model::Object) -> Option<std::sync::Arc<kition_scene_raster::Prepared>> {
    doc.meshes.borrow_mut().get(&doc.scene, object, false)
}

pub fn inspect(doc: &SceneDocument) -> Value {
    let scene = &doc.scene;
    let empty = kition_scene_model::Mesh::default();
    let objects: Vec<Value> = scene
        .objects
        .iter()
        .map(|object| {
            let mut entry = json!({
                "id": object.id,
                "name": object.name,
                "type": object.data.kind(),
                "location": triple(object.location),
                "rotation": triple(glam::Vec3::new(object.rotation.x.to_degrees(), object.rotation.y.to_degrees(), object.rotation.z.to_degrees())),
                "scale": triple(object.scale),
            });
            let Some(map) = entry.as_object_mut() else { return entry };
            if let Some(parent) = object.parent {
                map.insert("parent".into(), json!(parent));
            }
            if object.hidden {
                map.insert("hidden".into(), json!(true));
            }
            if let Some(material) = object.material {
                map.insert("material".into(), json!(material));
            }
            if let Ok(world) = scene.world_matrix(object.id) {
                map.insert("worldLocation".into(), triple(world.transform_point3(glam::Vec3::ZERO)));
            }
            match &object.data {
                ObjectData::Mesh(mesh) => {
                    let prepared = evaluated(doc, object);
                    let evaluated = prepared.as_deref().map_or(&empty, |prepared| &prepared.mesh);
                    let size = evaluated.bounds(Mat4::from_scale(object.scale)).size();
                    map.insert(
                        "mesh".into(),
                        json!({
                            "vertices": mesh.positions.len(),
                            "edges": mesh.edges().len(),
                            "faces": mesh.faces.len(),
                            "smooth": mesh.smooth,
                            "smoothAngle": mesh.smooth_angle.map(|angle| round(angle.to_degrees())),
                            "uvs": mesh.has_uvs(),
                            "evaluated": { "vertices": evaluated.positions.len(), "faces": evaluated.faces.len() },
                        }),
                    );
                    if let Some(fields) = map.get_mut("mesh").and_then(Value::as_object_mut) {
                        let levels = kition_scene_model::multires::levels(mesh);
                        if levels > 0 {
                            fields.insert("multiresLevels".into(), json!(levels));
                        }
                        let sets = kition_scene_model::sculpt::face_sets::ids(mesh);
                        if !sets.is_empty() {
                            fields.insert("faceSets".into(), json!(sets.len()));
                        }
                        let hidden = kition_scene_model::sculpt::face_sets::hidden(mesh).iter().filter(|flag| **flag).count();
                        if hidden > 0 {
                            fields.insert("hiddenFaces".into(), json!(hidden));
                        }
                    }
                    map.insert("dimensions".into(), triple(size));
                    if !object.modifiers.is_empty() {
                        map.insert("modifiers".into(), Value::Array(object.modifiers.iter().map(crate::commands::modifier_json).collect()));
                    }
                }
                ObjectData::Curve(curve) => {
                    let prepared = evaluated(doc, object);
                    let evaluated = prepared.as_deref().map_or(&empty, |prepared| &prepared.mesh);
                    map.insert("curve".into(), crate::commands_curve::curve_json(curve, evaluated));
                    map.insert("dimensions".into(), triple(evaluated.bounds(Mat4::from_scale(object.scale)).size()));
                    if !object.modifiers.is_empty() {
                        map.insert("modifiers".into(), Value::Array(object.modifiers.iter().map(crate::commands::modifier_json).collect()));
                    }
                }
                ObjectData::Text(text) => {
                    let prepared = evaluated(doc, object);
                    let evaluated = prepared.as_deref().map_or(&empty, |prepared| &prepared.mesh);
                    map.insert("text".into(), crate::commands_curve::text_json(text, evaluated));
                    map.insert("dimensions".into(), triple(evaluated.bounds(Mat4::from_scale(object.scale)).size()));
                    if !object.modifiers.is_empty() {
                        map.insert("modifiers".into(), Value::Array(object.modifiers.iter().map(crate::commands::modifier_json).collect()));
                    }
                }
                ObjectData::Surface(surface) => {
                    let prepared = evaluated(doc, object);
                    let evaluated = prepared.as_deref().map_or(&empty, |prepared| &prepared.mesh);
                    // Each patch's grid, orders and resolutions; its control points are left out.
                    let patches = surface
                        .patches
                        .iter()
                        .map(|patch| {
                            let mut fields = match serde_json::to_value(patch) {
                                Ok(Value::Object(fields)) => fields,
                                _ => Map::new(),
                            };
                            fields.remove("points");
                            fields.insert("pointCount".into(), json!(patch.points.len()));
                            Value::Object(fields)
                        })
                        .collect();
                    map.insert("surface".into(), json!({ "patches": Value::Array(patches), "smooth": surface.smooth, "faces": evaluated.faces.len() }));
                    map.insert("dimensions".into(), triple(evaluated.bounds(Mat4::from_scale(object.scale)).size()));
                    if !object.modifiers.is_empty() {
                        map.insert("modifiers".into(), Value::Array(object.modifiers.iter().map(crate::commands::modifier_json).collect()));
                    }
                }
                ObjectData::Light(light) => {
                    let kind = match light.kind {
                        LightKind::Point => "point",
                        LightKind::Sun => "sun",
                        LightKind::Spot => "spot",
                        LightKind::Area => "area",
                    };
                    map.insert(
                        "light".into(),
                        json!({ "kind": kind, "color": triple(light.color), "power": round(light.power), "radius": round(light.radius), "spotSize": round(light.spot_size.to_degrees()) }),
                    );
                }
                ObjectData::Camera(camera) => {
                    map.insert(
                        "camera".into(),
                        json!({
                            "lens": round(camera.lens),
                            "sensor": round(camera.sensor),
                            "sensorHeight": round(camera.sensor_height),
                            "sensorFit": camera.sensor_fit,
                            "clipStart": round(camera.clip_start),
                            "clipEnd": round(camera.clip_end),
                            "orthographic": camera.orthographic,
                            "orthoScale": round(camera.ortho_scale),
                            "depthOfField": camera.dof.enabled,
                            "fStop": round(camera.dof.f_stop),
                            "focusDistance": round(camera.dof.focus_distance),
                            "focusObject": camera.dof.focus_object,
                            "blades": camera.dof.blades,
                            "bladeRotation": round(camera.dof.rotation.to_degrees()),
                            "ratio": round(camera.dof.ratio),
                        }),
                    );
                }
                ObjectData::Empty { size } => {
                    map.insert("size".into(), json!(round(*size)));
                }
                ObjectData::Lattice(lattice) => {
                    map.insert("lattice".into(), crate::commands_lattice::lattice_json(lattice));
                }
                // Bones are listed by the rigging commands' part of inspect.
                ObjectData::Armature(_) => {}
            }
            entry
        })
        .collect();
    let materials: Vec<Value> = scene
        .materials
        .iter()
        .map(|material| {
            json!({
                "id": material.id,
                "name": material.name,
                "baseColor": color(material.base_color),
                "metallic": round(material.metallic),
                "roughness": round(material.roughness),
                "emission": triple(material.emission),
                "emissionStrength": round(material.emission_strength),
                "baseColorTexture": material.base_color_texture,
                "metallicRoughnessTexture": material.metallic_roughness_texture,
                "normalTexture": material.normal_texture,
                "normalStrength": material.normal_strength,
                "transmission": round(material.transmission),
                "ior": round(material.ior),
                "alphaMode": match material.alpha_mode {
                    kition_scene_model::AlphaMode::Opaque => "opaque",
                    kition_scene_model::AlphaMode::Clip => "clip",
                    kition_scene_model::AlphaMode::Blend => "blend",
                },
                "alphaCutoff": round(material.alpha_cutoff),
                "shader": material.shader,
                "users": scene.objects.iter().filter(|object| object.material == Some(material.id)).count(),
            })
        })
        .collect();
    let view = &doc.session.editor.view;
    let mut selection = json!({ "objects": doc.session.editor.selected, "active": doc.session.editor.active });
    if let (Mode::Edit(id), Some(map)) = (doc.session.mode, selection.as_object_mut()) {
        let faces = scene
            .object(id)
            .ok()
            .and_then(|object| object.mesh())
            .map(|mesh| kition_scene_model::ops::faces_within(mesh, &doc.session.vertices))
            .unwrap_or_default();
        map.insert("vertices".into(), json!(doc.session.vertices));
        map.insert("faces".into(), json!(faces));
    }
    let (mode, editing) = match doc.session.mode {
        Mode::Object => ("object", Value::Null),
        Mode::Edit(id) => ("edit", json!(id)),
    };
    json!({
        "canUndo": doc.can_step(None, true),
        "canRedo": doc.can_step(None, false),
        "undo": doc.step_label(kition_kit::kdm::UndoScope::Person, true),
        "redo": doc.step_label(kition_kit::kdm::UndoScope::Person, false),
        "objects": objects,
        "materials": materials,
        "images": crate::commands_uv::images_json(scene),
        "nodeGroups": scene.node_groups.iter().map(|group| crate::commands_nodes::group_json(group, &scene.node_groups)).collect::<Vec<_>>(),
        "camera": scene.camera,
        "world": {
            "color": triple(scene.world.color),
            "strength": round(scene.world.strength),
            "image": scene.world.image,
            "rotation": round(scene.world.rotation.to_degrees()),
            "volume": scene.world.volume,
        },
        "render": {
            "width": scene.render.width,
            "height": scene.render.height,
            "compositor": scene.render.compositor,
            "compositing": scene.render.compositing,
            "motionBlur": scene.render.motion_blur.enabled,
            "shutter": round(scene.render.motion_blur.shutter),
            "shutterPosition": match scene.render.motion_blur.position {
                kition_scene_model::ShutterPosition::Center => "center",
                kition_scene_model::ShutterPosition::Start => "start",
                kition_scene_model::ShutterPosition::End => "end",
            },
            "ao": scene.render.ambient_occlusion.enabled,
            "aoDistance": scene.render.ambient_occlusion.distance.map(round),
            "aoStrength": round(scene.render.ambient_occlusion.strength),
            "view": scene.render.color_management.view.name(),
            "look": scene.render.color_management.look.name(),
            "exposure": round(scene.render.color_management.exposure),
        },
        "units": "metres, Z up, rotations in degrees (XYZ Euler)",
        "mode": { "mode": mode, "object": editing, "proportional": doc.session.proportional.map(crate::commands_model::proportional_json), "select": match doc.session.select_mode { SelectMode::Vertex => "vertex", SelectMode::Edge => "edge", SelectMode::Face => "face" } },
        "selection": selection,
        "view": {
            "target": triple(view.target),
            "distance": round(view.distance),
            "tilt": round(view.tilt.to_degrees()),
            "turn": round(view.turn.to_degrees()),
            "perspective": view.perspective,
            "camera": view.camera,
            "rendered": (doc.session.editor.shading == Shading::Rendered).then(|| doc.viewport_trace.borrow().status(doc.revision)),
            "shading": match doc.session.editor.shading { Shading::Wireframe => "wireframe", Shading::Solid => "solid", Shading::Material => "material", Shading::Rendered => "rendered" },
            "grid": doc.session.editor.grid,
            "overlays": doc.session.editor.overlays,
            "width": doc.session.width,
            "height": doc.session.height,
        },
        "tool": doc.tool_json(),
    })
}
