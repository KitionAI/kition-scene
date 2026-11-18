//! The second set of Edit Mode commands (extrude individual faces, dissolve, bridge, bisect,
//! separate, symmetrize, spin, vertex bevel, rotate), proportional editing, and the follow-on
//! moves proportional editing adds to `mesh.translate`, `mesh.scale`, and the viewport tools.
//! `commands.rs` lists these specs after its own and hands these commands here.

use std::collections::BTreeSet;

use glam::{Mat3, Vec3};
use kition_scene_model::proportional::{Falloff, Proportional};
use kition_scene_model::{Mesh, ObjectData, ops, ops2, proportional};
use serde_json::{Value, json};

use crate::commands::{Needs, Spec, edit_mesh, selected_faces, spec};
use crate::params::{self, bad, schema as s};
use crate::{Mode, SceneDocument};

fn none() -> Value {
    s::of(vec![])
}

pub static SPECS: &[Spec] = &[
    spec("mesh.extrudeIndividual", "Extrude Individual Faces", &["Mesh", "Extrude"], None, true, Needs::EditSelection, || {
        s::of(vec![("distance", s::number("How far each face moves along its own normal, metres (1)"))])
    }),
    spec("mesh.dissolveVertices", "Dissolve Vertices", &["Mesh", "Dissolve"], None, true, Needs::EditSelection, none),
    spec("mesh.dissolveEdges", "Dissolve Edges", &["Mesh", "Dissolve"], None, true, Needs::EditSelection, none),
    spec("mesh.dissolveFaces", "Dissolve Faces", &["Mesh", "Dissolve"], None, true, Needs::EditSelection, none),
    spec("mesh.bridge", "Bridge Edge Loops", &["Mesh"], None, true, Needs::EditSelection, none),
    spec("mesh.bisect", "Bisect", &["Mesh"], None, true, Needs::EditMode, || {
        s::of(vec![
            ("point", s::vec3("A point on the cutting plane, in the object's own axes (the selection's centre)")),
            ("normal", s::vec3("The plane's normal, in the object's own axes ([0, 0, 1])")),
            ("clearInner", s::boolean("Delete what lies behind the plane")),
            ("clearOuter", s::boolean("Delete what lies in front of the plane")),
            ("fill", s::boolean("Close the cut with a face (with clearing)")),
        ])
    }),
    spec("mesh.separate", "Separate Selection", &["Mesh"], Some("P"), true, Needs::EditSelection, || {
        s::of(vec![("name", s::string("The new object's name"))])
    }),
    spec("mesh.symmetrize", "Symmetrize", &["Mesh", "Mirror"], None, true, Needs::EditMode, || {
        s::of(vec![
            ("axis", s::choice("The axis whose plane the mesh is mirrored across (x)", &["x", "y", "z"])),
            ("direction", s::choice("Which side replaces the other (positive)", &["positive", "negative"])),
        ])
    }),
    spec("mesh.spin", "Spin", &["Mesh"], None, true, Needs::EditSelection, || {
        s::of(vec![
            ("steps", s::integer("Steps (12)")),
            ("angle", s::number("Degrees to sweep (90); 360 closes the sweep")),
            ("axis", s::vec3("The axis to sweep round ([0, 0, 1])")),
            ("center", s::vec3("A point on the axis, in the object's own axes (the origin)")),
        ])
    }),
    spec("mesh.bevelVertices", "Bevel Vertices", &["Mesh"], Some("Cmd+Shift+B"), true, Needs::EditSelection, || {
        s::of(vec![("width", s::number("How far along each edge the cut is, metres (0.1)"))])
    }),
    spec("mesh.rotate", "Rotate Elements", &["Mesh", "Transform"], None, true, Needs::EditSelection, || {
        s::of(vec![
            ("axis", s::required(s::choice("The object's axis to turn round", &["x", "y", "z"]))),
            ("angle", s::required(s::number("Degrees, counter-clockwise looking down the axis"))),
            ("pivot", s::vec3("The point to turn about; the selection's centre when left out")),
        ])
    }),
    spec("mesh.proportional", "Proportional Editing", &["Mesh"], Some("O"), false, Needs::Always, || {
        s::of(vec![
            ("on", s::boolean("On or off; toggles when left out")),
            ("radius", s::number("How far, in metres, unselected vertices follow (1)")),
            ("falloff", s::choice("How the pull fades with distance", &["smooth", "sphere", "root", "linear", "sharp", "constant"])),
            ("connected", s::boolean("Only vertices joined to the selection by edges follow, by their distance along the edges")),
        ])
    }),
];

/// The centre of the selected vertices (or the origin).
fn centre(mesh: &Mesh, vertices: &BTreeSet<u32>) -> Vec3 {
    let points: Vec<Vec3> = vertices.iter().filter_map(|vertex| mesh.positions.get(*vertex as usize)).copied().collect();
    if points.is_empty() { Vec3::ZERO } else { points.iter().copied().sum::<Vec3>() / points.len() as f32 }
}

/// Unselected vertices follow `change` (in the object's own axes) by their proportional weight,
/// when proportional editing is on. Weights come from where the vertices are now, so call this
/// before the selection moves.
pub fn follow(doc: &mut SceneDocument, change: &dyn Fn(Vec3) -> Vec3) -> Result<(), String> {
    let Some(settings) = doc.session.proportional else { return Ok(()) };
    edit_mesh(doc, |mesh, vertices| {
        let weights = proportional::weights(mesh, vertices, settings);
        proportional::apply(mesh, &weights, change);
        Ok(())
    })
}

/// [`follow`] for a scale by `factor` about `pivot` (or the selection's centre).
pub fn follow_scale(doc: &mut SceneDocument, factor: Vec3, pivot: Option<Vec3>) -> Result<(), String> {
    if doc.session.proportional.is_none() {
        return Ok(());
    }
    let Mode::Edit(id) = doc.session.mode else { return Ok(()) };
    let selection = doc.session.vertices.clone();
    let centre = match pivot {
        Some(pivot) => pivot,
        None => doc.scene.object(id).ok().and_then(|object| object.mesh()).map_or(Vec3::ZERO, |mesh| centre(mesh, &selection)),
    };
    follow(doc, &|point| centre + (point - centre) * factor)
}

/// Whether `mesh.extrude` should extrude loose vertices and edges: edit mode with a selection
/// that holds no whole face.
pub fn extrudes_loose(doc: &SceneDocument) -> bool {
    let Mode::Edit(id) = doc.session.mode else { return false };
    let Some(mesh) = doc.scene.object(id).ok().and_then(|object| object.mesh()) else { return false };
    !doc.session.vertices.is_empty() && crate::commands::faces_selected(&doc.session, mesh).is_empty()
}

/// `mesh.extrude` with only vertices and edges selected: `offset`, or `distance` (1) along the
/// selection's mean normal (up when it has none).
pub fn extrude_loose(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let distance = params::number(params, "distance")?.unwrap_or(1.0);
    let offset = params::vec3(params, "offset")?.unwrap_or(Vec3::ZERO);
    let made = edit_mesh(doc, |mesh, vertices| {
        // As on faces: `distance` along the mean normal, plus `offset`.
        let normals = mesh.vertex_normals();
        let mean = vertices.iter().filter_map(|vertex| normals.get(*vertex as usize)).copied().sum::<Vec3>().normalize_or_zero();
        let along = (if mean == Vec3::ZERO { Vec3::Z } else { mean }) * distance;
        ops2::extrude_loose(mesh, vertices, along + offset).map_err(|error| error.to_string())
    })?;
    doc.session.vertices = made;
    Ok(json!({ "vertices": doc.session.vertices.len() }))
}

fn axis_vector(name: Option<&str>) -> Result<Vec3, String> {
    match name.unwrap_or("x") {
        "x" => Ok(Vec3::X),
        "y" => Ok(Vec3::Y),
        "z" => Ok(Vec3::Z),
        other => Err(bad(format!("the axis must be x, y, or z, not {other}"))),
    }
}

/// Commands that only change the session.
pub fn run_view(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    match command {
        "mesh.proportional" => {
            let on = params::boolean(params, "on")?.unwrap_or(doc.session.proportional.is_none());
            let mut settings = doc.session.proportional.unwrap_or_default();
            if let Some(radius) = params::number(params, "radius")? {
                if radius <= 0.0 {
                    return Err(bad("`radius` must be positive"));
                }
                settings.radius = radius;
            }
            if let Some(name) = params::string(params, "falloff")? {
                settings.falloff = serde_json::from_value::<Falloff>(json!(name)).map_err(|_| bad(format!("unknown falloff {name}")))?;
            }
            if let Some(connected) = params::boolean(params, "connected")? {
                settings.connected = connected;
            }
            doc.session.proportional = on.then_some(settings);
            // A drag under way follows the new radius at once (the wheel resizes it mid-drag).
            doc.redrag();
            doc.touched();
            Ok(json!({ "proportional": doc.session.proportional.map(proportional_json) }))
        }
        other => Err(format!("unknown-command: {other}")),
    }
}

/// Proportional settings as inspect shows them.
pub fn proportional_json(settings: Proportional) -> Value {
    json!({ "radius": settings.radius, "falloff": serde_json::to_value(settings.falloff).unwrap_or(Value::Null), "connected": settings.connected })
}

/// Commands that change the scene (one undo step each; `commands.rs` rolls back failures).
pub fn run_change(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    match command {
        "mesh.extrudeIndividual" => {
            let distance = params::number(params, "distance")?.unwrap_or(1.0);
            let moved = edit_mesh(doc, |mesh, vertices| {
                let faces = selected_faces(mesh, vertices);
                if faces.is_empty() {
                    return Err("not-allowed: select faces to extrude".into());
                }
                ops2::extrude_individual(mesh, &faces, distance).map_err(|error| error.to_string())
            })?;
            doc.session.vertices = moved;
            Ok(json!({ "vertices": doc.session.vertices.len() }))
        }
        "mesh.dissolveVertices" => {
            edit_mesh(doc, |mesh, vertices| ops2::dissolve_vertices(mesh, vertices).map_err(|error| error.to_string()))?;
            doc.session.vertices.clear();
            Ok(json!({ "dissolved": "vertices" }))
        }
        "mesh.dissolveEdges" => {
            edit_mesh(doc, |mesh, vertices| {
                let edges = ops2::selected_edges(mesh, vertices);
                if edges.is_empty() {
                    return Err("not-allowed: select edges to dissolve".into());
                }
                ops2::dissolve_edges(mesh, &edges).map_err(|error| error.to_string())
            })?;
            doc.session.vertices.clear();
            Ok(json!({ "dissolved": "edges" }))
        }
        "mesh.dissolveFaces" => {
            let made = edit_mesh(doc, |mesh, vertices| {
                let faces = selected_faces(mesh, vertices);
                if faces.len() < 2 {
                    return Err("not-allowed: select neighbouring faces to dissolve".into());
                }
                let made = ops2::dissolve_faces(mesh, &faces).map_err(|error| error.to_string())?;
                Ok(ops::face_vertices(mesh, &made.into_iter().collect()))
            })?;
            doc.session.vertices = made;
            Ok(json!({ "vertices": doc.session.vertices.len() }))
        }
        "mesh.bridge" => {
            let made = edit_mesh(doc, |mesh, vertices| {
                let made = ops2::bridge(mesh, vertices).map_err(|error| error.to_string())?;
                Ok(made.len())
            })?;
            Ok(json!({ "faces": made }))
        }
        "mesh.bisect" => {
            let point = params::vec3(params, "point")?;
            let normal = params::vec3(params, "normal")?.unwrap_or(Vec3::Z);
            let clear_inner = params::boolean(params, "clearInner")?.unwrap_or(false);
            let clear_outer = params::boolean(params, "clearOuter")?.unwrap_or(false);
            let fill = params::boolean(params, "fill")?.unwrap_or(false);
            if clear_inner && clear_outer {
                return Err(bad("clear one side at most"));
            }
            let cut = edit_mesh(doc, |mesh, vertices| {
                let point = point.unwrap_or_else(|| centre(mesh, vertices));
                ops2::bisect(mesh, point, normal, clear_inner, clear_outer, fill).map_err(|error| error.to_string())
            })?;
            doc.session.vertices = cut;
            Ok(json!({ "cut": doc.session.vertices.len() }))
        }
        "mesh.separate" => {
            let Mode::Edit(id) = doc.session.mode else { return Err("not-allowed: not in edit mode".into()) };
            let selection = doc.session.vertices.clone();
            let original = doc.scene.object(id).map_err(|error| error.to_string())?.clone();
            let mesh = original.mesh().ok_or("unsupported: the edited object is not a mesh")?;
            let part = ops2::split_off(mesh, &selection);
            if part.positions.is_empty() {
                return Err("not-allowed: nothing is selected".into());
            }
            let name = params::string(params, "name")?.map_or_else(|| original.name.clone(), str::to_string);
            let new_id = doc.scene.add_object(&name, ObjectData::Mesh(part));
            let object = doc.scene.object_mut(new_id).map_err(|error| error.to_string())?;
            object.location = original.location;
            object.rotation = original.rotation;
            object.scale = original.scale;
            object.parent = original.parent;
            object.parent_inverse = original.parent_inverse;
            object.material = original.material;
            object.modifiers = original.modifiers.clone();
            let name = object.name.clone();
            edit_mesh(doc, |mesh, vertices| {
                ops2::remove_split(mesh, vertices);
                Ok(())
            })?;
            doc.session.vertices.clear();
            Ok(json!({ "object": new_id, "name": name }))
        }
        "mesh.symmetrize" => {
            let axis = match params::string(params, "axis")?.unwrap_or("x") {
                "x" => 0,
                "y" => 1,
                "z" => 2,
                other => return Err(bad(format!("the axis must be x, y, or z, not {other}"))),
            };
            let positive = match params::string(params, "direction")?.unwrap_or("positive") {
                "positive" => true,
                "negative" => false,
                other => return Err(bad(format!("the direction must be positive or negative, not {other}"))),
            };
            edit_mesh(doc, |mesh, _| ops2::symmetrize(mesh, axis, positive).map_err(|error| error.to_string()))?;
            doc.session.vertices.clear();
            Ok(json!({ "symmetrized": true }))
        }
        "mesh.spin" => {
            let steps = params::integer(params, "steps")?.unwrap_or(12);
            let angle = params::number(params, "angle")?.unwrap_or(90.0).to_radians();
            let axis = params::vec3(params, "axis")?.unwrap_or(Vec3::Z);
            let center = params::vec3(params, "center")?.unwrap_or(Vec3::ZERO);
            let made = edit_mesh(doc, |mesh, vertices| ops2::spin(mesh, vertices, steps, angle, axis, center).map_err(|error| error.to_string()))?;
            doc.session.vertices = made;
            Ok(json!({ "vertices": doc.session.vertices.len() }))
        }
        "mesh.bevelVertices" => {
            let width = params::number(params, "width")?.unwrap_or(0.1);
            let made = edit_mesh(doc, |mesh, vertices| {
                let faces = ops2::bevel_vertices(mesh, vertices, width).map_err(|error| error.to_string())?;
                Ok(ops::face_vertices(mesh, &faces.into_iter().collect()))
            })?;
            doc.session.vertices = made;
            Ok(json!({ "vertices": doc.session.vertices.len() }))
        }
        "mesh.rotate" => {
            let axis = axis_vector(params::string(params, "axis")?)?;
            let angle = params::need_number(params, "angle")?.to_radians();
            let pivot = params::vec3(params, "pivot")?;
            let Mode::Edit(id) = doc.session.mode else { return Err("not-allowed: not in edit mode".into()) };
            let selection = doc.session.vertices.clone();
            let centre_point = match pivot {
                Some(pivot) => pivot,
                None => doc.scene.object(id).ok().and_then(|object| object.mesh()).map_or(Vec3::ZERO, |mesh| centre(mesh, &selection)),
            };
            let turn = Mat3::from_axis_angle(axis, angle);
            let change = move |point: Vec3| centre_point + turn * (point - centre_point);
            follow(doc, &change)?;
            edit_mesh(doc, |mesh, vertices| {
                for vertex in vertices {
                    if let Some(point) = mesh.positions.get_mut(*vertex as usize) {
                        *point = change(*point);
                    }
                }
                Ok(())
            })?;
            Ok(json!({ "rotated": doc.session.vertices.len() }))
        }
        other => Err(format!("unknown-command: {other}")),
    }
}

/// Reads a `.blend` file's objects into the scene (with their modifiers). Returns the new
/// top-level objects.
pub fn import_blend(doc: &mut SceneDocument, bytes: &[u8]) -> Result<Vec<kition_scene_model::Id>, String> {
    let file = kition_scene_blend::read(bytes).map_err(|error| error.to_string())?;
    kition_scene_blend::place(&mut doc.scene, file).map_err(|error| error.to_string())
}
