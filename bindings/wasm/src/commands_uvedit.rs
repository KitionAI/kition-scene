//! UV editing and the second set of cutting tools, in edit mode: seams, unwrapping by seams,
//! pins, moving, turning, and scaling texture coordinates, the knife, and edge slide; and
//! `uv.layout`, which answers the edited mesh's texture coordinates for the UV editor and for
//! agents. The UV editor works on the mesh selection (the reference's "UV sync selection"): the
//! coordinates a command moves are the corners of selected faces at selected vertices. Each
//! change is one undo step; a failing command changes nothing.

use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};

use glam::{Mat3, Vec2, Vec3};
use kition_kit::KitCommand;
use kition_scene_model::{Mesh, unwrap::Method};
use serde_json::{Value, json};

use crate::params::{self, bad, schema as s};
use crate::{Mode, SceneDocument};

struct Spec {
    id: &'static str,
    title: &'static str,
    menu: &'static [&'static str],
    shortcut: Option<&'static str>,
    mutates: bool,
    schema: fn() -> Value,
}

const UV_MENU: &[&str] = &["Mesh", "UV"];
const MESH_MENU: &[&str] = &["Mesh"];

fn none() -> Value {
    s::of(vec![])
}

fn source_param() -> (&'static str, Value) {
    ("source", s::string("Optional UV layout source token; reject a stale drag without changing the scene"))
}

fn pivot() -> (&'static str, Value) {
    (
        "pivot",
        json!({ "type": "array", "items": { "type": "number" }, "description": "The point in UV space to turn or scale about [u, v]; the moved coordinates' centre when left out" }),
    )
}

static SPECS: &[Spec] = &[
    Spec { id: "mesh.markSeam", title: "Mark Seam", menu: UV_MENU, shortcut: None, mutates: true, schema: none },
    Spec { id: "mesh.clearSeam", title: "Clear Seam", menu: UV_MENU, shortcut: None, mutates: true, schema: none },
    Spec {
        id: "uv.unwrap",
        title: "Unwrap",
        menu: UV_MENU,
        shortcut: Some("U"),
        mutates: true,
        schema: || {
            s::of(vec![
                ("margin", s::number("Space between islands, as a fraction of the square (0.02)")),
                ("method", s::choice("How islands are flattened (angleBased)", &["angleBased", "conformal", "minimumStretch"])),
                ("iterations", s::integer("Minimum stretch iterations, 1 to 100 (30)")),
            ])
        },
    },
    Spec {
        id: "uv.liveUnwrap",
        title: "Live Unwrap",
        menu: UV_MENU,
        shortcut: None,
        mutates: false,
        schema: || {
            s::of(vec![
                ("on", s::boolean("Enable live relaxation while pinned UVs move")),
                ("method", s::choice("Live relaxation method", &["angleBased", "conformal", "minimumStretch"])),
                ("iterations", s::integer("Minimum stretch iterations, 1 to 100 (30)")),
            ])
        },
    },
    Spec {
        id: "uv.preview",
        title: "Preview UV Move",
        menu: &[],
        shortcut: None,
        mutates: false,
        schema: || {
            s::of(vec![
                ("offset", s::required(json!({"type":"array", "items":{"type":"number"}, "description":"UV drag offset [u, v]"}))),
                source_param(),
            ])
        },
    },
    Spec { id: "uv.stitch", title: "Stitch", menu: UV_MENU, shortcut: None, mutates: true, schema: none },
    Spec { id: "uv.pin", title: "Pin", menu: UV_MENU, shortcut: None, mutates: true, schema: none },
    Spec { id: "uv.unpin", title: "Unpin", menu: UV_MENU, shortcut: None, mutates: true, schema: none },
    Spec {
        id: "uv.translate",
        title: "Move UVs",
        menu: &[],
        shortcut: None,
        mutates: true,
        schema: || {
            s::of(vec![
                ("offset", s::required(json!({ "type": "array", "items": { "type": "number" }, "description": "How far in UV units [u, v]" }))),
                source_param(),
            ])
        },
    },
    Spec {
        id: "uv.rotate",
        title: "Rotate UVs",
        menu: &[],
        shortcut: None,
        mutates: true,
        schema: || s::of(vec![("angle", s::required(s::number("Degrees, counter-clockwise"))), pivot(), source_param()]),
    },
    Spec {
        id: "uv.scale",
        title: "Scale UVs",
        menu: &[],
        shortcut: None,
        mutates: true,
        schema: || s::of(vec![("factor", s::required(json!({ "type": "json", "description": "A number, or [u, v]" }))), pivot(), source_param()]),
    },
    Spec {
        id: "uv.layout",
        title: "UV Layout",
        menu: &[],
        shortcut: None,
        mutates: false,
        schema: || s::of(vec![("all", s::boolean("Every face, not only the selected ones (false)"))]),
    },
    Spec {
        id: "mesh.knife",
        title: "Knife",
        menu: MESH_MENU,
        shortcut: None,
        mutates: true,
        schema: || {
            s::of(vec![
                (
                    "points",
                    json!({ "type": "array", "items": { "type": "json" }, "description": "The cut line: view pixels [x, y], or points in the world [x, y, z], at least two" }),
                ),
                ("through", s::boolean("Cut faces turned away from the view too (false)")),
            ])
        },
    },
    Spec {
        id: "mesh.edgeSlide",
        title: "Edge Slide",
        menu: MESH_MENU,
        shortcut: None,
        mutates: true,
        schema: || s::of(vec![("factor", s::required(s::number("How far towards the next edge loop, -1 to 1")))]),
    },
];

fn editing(doc: &SceneDocument) -> Option<u32> {
    match doc.session.mode {
        Mode::Edit(id) => Some(id),
        Mode::Object => None,
    }
}

fn enabled(doc: &SceneDocument, id: &str) -> bool {
    let Some(_) = editing(doc) else { return false };
    match id {
        "uv.layout" | "uv.liveUnwrap" | "mesh.knife" | "uv.unwrap" => true,
        _ => !doc.session.vertices.is_empty(),
    }
}

/// The commands this file adds.
pub fn catalogue(doc: &SceneDocument) -> Vec<KitCommand> {
    SPECS
        .iter()
        .map(|spec| KitCommand {
            id: spec.id.into(),
            title: spec.title.into(),
            menu: (!spec.menu.is_empty()).then(|| spec.menu.iter().map(|part| (*part).to_string()).collect()),
            shortcut: spec.shortcut.map(Into::into),
            params: None,
            schema: Some((spec.schema)()),
            mutates: spec.mutates,
            enabled: enabled(doc, spec.id),
            ..Default::default()
        })
        .collect()
}

/// The ones that cannot run now.
pub fn disabled(doc: &SceneDocument) -> Vec<String> {
    SPECS.iter().filter(|spec| !enabled(doc, spec.id)).map(|spec| spec.id.to_string()).collect()
}

/// Runs `command` if it is one of these; `None` for every other command.
pub fn execute(doc: &mut SceneDocument, command: &str, params: &Value) -> Option<Result<Value, String>> {
    let spec = SPECS.iter().find(|spec| spec.id == command)?;
    if !params.is_object() && !params.is_null() {
        return Some(Err(bad("the parameters must be an object")));
    }
    if !enabled(doc, spec.id) {
        return Some(Err(format!("not-allowed: {} works in edit mode{}", spec.title, if spec.id == "uv.layout" { "" } else { " on a selection" })));
    }
    if !spec.mutates {
        return Some(match command {
            "uv.liveUnwrap" => settings(doc, params),
            "uv.preview" => preview(doc, params),
            _ => layout(doc, params),
        });
    }
    let before = doc.step(spec.title);
    let session = doc.session.clone();
    let result = run(doc, command, params).and_then(|value| {
        doc.scene.validate().map_err(|error| format!("failed: the change would break the scene ({error})"))?;
        Ok(value)
    });
    Some(match result {
        Ok(value) => {
            doc.tidy_session();
            doc.committed(before).map(|()| value)
        }
        Err(error) => {
            doc.restore(before);
            doc.session = session;
            Err(error)
        }
    })
}

/// The edited mesh, to change.
fn mesh_mut(doc: &mut SceneDocument) -> Result<&mut Mesh, String> {
    let id = editing(doc).ok_or("not-allowed: not in edit mode")?;
    doc.scene.object_mut(id).map_err(|error| error.to_string())?.mesh_mut().ok_or_else(|| "unsupported: the edited object is not a mesh".to_string())
}

fn uv_pair(value: &Value, key: &str) -> Result<Vec2, String> {
    let items = value.as_array().ok_or_else(|| bad(format!("`{key}` must be [u, v]")))?;
    match items.as_slice() {
        [u, v] => {
            let (u, v) = (
                u.as_f64().ok_or_else(|| bad(format!("`{key}` must be numbers")))?,
                v.as_f64().ok_or_else(|| bad(format!("`{key}` must be numbers")))?,
            );
            if !(u.is_finite() && v.is_finite() && u.abs() < 1e6 && v.abs() < 1e6) {
                return Err(bad(format!("`{key}` must be finite")));
            }
            Ok(Vec2::new(u as f32, v as f32))
        }
        _ => Err(bad(format!("`{key}` must be [u, v]"))),
    }
}

/// The (face, corner) pairs a UV command moves: corners of selected faces at selected vertices.
fn moving_corners(mesh: &Mesh, vertices: &BTreeSet<u32>) -> Vec<(usize, usize)> {
    let faces = kition_scene_model::ops::faces_within(mesh, vertices);
    faces
        .iter()
        .filter_map(|face| mesh.faces.get(*face as usize).map(|corners| (*face as usize, corners)))
        .flat_map(|(face, corners)| corners.iter().enumerate().filter(|(_, vertex)| vertices.contains(vertex)).map(move |(corner, _)| (face, corner)))
        .collect()
}

fn transform_uvs(doc: &mut SceneDocument, change: impl Fn(Vec2, Vec2) -> Vec2, params: &Value) -> Result<Value, String> {
    check_source(doc, params)?;
    let vertices = doc.session.vertices.clone();
    let given_pivot = match params.get("pivot").filter(|value| !value.is_null()) {
        Some(value) => Some(uv_pair(value, "pivot")?),
        None => None,
    };
    let live = doc.session.editor.uv_live_unwrap;
    let method = doc.session.editor.uv_unwrap_method;
    let iterations = doc.session.editor.uv_unwrap_iterations;
    let mesh = mesh_mut(doc)?;
    transform_mesh(mesh, &vertices, change, given_pivot, live, method, iterations)
}

fn transform_mesh(
    mesh: &mut Mesh,
    vertices: &BTreeSet<u32>,
    change: impl Fn(Vec2, Vec2) -> Vec2,
    given_pivot: Option<Vec2>,
    live: bool,
    method: Method,
    iterations: u32,
) -> Result<Value, String> {
    if !mesh.has_uvs() || !mesh.uvs_aligned() {
        return Err("not-allowed: the mesh has no texture coordinates; unwrap it first".into());
    }
    let corners = if live {
        mesh.faces
            .iter()
            .enumerate()
            .flat_map(|(face, list)| list.iter().enumerate().filter(|(_, vertex)| vertices.contains(vertex)).map(move |(corner, _)| (face, corner)))
            .collect()
    } else {
        moving_corners(mesh, vertices)
    };
    if corners.is_empty() {
        return Err("not-allowed: select faces whose coordinates to change".into());
    }
    let centre =
        given_pivot.unwrap_or_else(|| corners.iter().map(|(face, corner)| mesh.corner_uv(*face, *corner)).sum::<Vec2>() / corners.len() as f32);
    let original = live.then(|| mesh.clone());
    for (face, corner) in &corners {
        if let Some(uv) = mesh.uvs.get_mut(*face).and_then(|list| list.get_mut(*corner)) {
            *uv = change(*uv, centre);
        }
    }
    let charts = if let Some(original) = original {
        kition_scene_model::unwrap::live_unwrap(mesh, &original, vertices, method, iterations).map_err(|error| error.to_string())?
    } else {
        0
    };
    Ok(json!({ "corners": corners.len(), "charts": charts }))
}

fn run(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    match command {
        "mesh.markSeam" | "mesh.clearSeam" => {
            let vertices = doc.session.vertices.clone();
            let mesh = mesh_mut(doc)?;
            let edges: Vec<(u32, u32)> =
                mesh.edges().into_iter().filter(|[a, b]| vertices.contains(a) && vertices.contains(b)).map(|[a, b]| (a, b)).collect();
            if edges.is_empty() {
                return Err("not-allowed: select edges".into());
            }
            let changed = kition_scene_model::seams::mark(mesh, &edges, command == "mesh.markSeam").map_err(|error| error.to_string())?;
            Ok(json!({ "edges": changed, "seams": mesh.seams.len() }))
        }
        "uv.unwrap" => {
            let margin = params::number(params, "margin")?.unwrap_or(0.02);
            let method = parse_method(params)?.unwrap_or(Method::AngleBased);
            let iterations = iterations(params)?.unwrap_or(30);
            let (vertices, named) = (doc.session.vertices.clone(), doc.session.faces.clone());
            let mesh = mesh_mut(doc)?;
            let mut faces = crate::commands::named_or_within(mesh, &vertices, named.as_ref());
            if faces.is_empty() {
                faces = (0..u32::try_from(mesh.faces.len()).unwrap_or(0)).collect();
            }
            let done = if method == Method::MinimumStretch {
                kition_scene_model::unwrap::minimum_stretch(mesh, &faces, margin, iterations)
            } else {
                kition_scene_model::unwrap::unwrap_with(mesh, &faces, margin, method)
            }
            .map_err(|error| error.to_string())?;
            doc.session.editor.uv_unwrap_method = method;
            doc.session.editor.uv_unwrap_iterations = iterations;
            Ok(json!({ "charts": done.charts, "faces": done.faces }))
        }
        "uv.stitch" => {
            let vertices = doc.session.vertices.clone();
            let mesh = mesh_mut(doc)?;
            let stitched = kition_scene_model::stitch::stitch(mesh, &vertices).map_err(|error| error.to_string())?;
            Ok(json!({ "edges": stitched }))
        }
        "uv.pin" | "uv.unpin" => {
            let vertices = doc.session.vertices.clone();
            let mesh = mesh_mut(doc)?;
            let changed = kition_scene_model::seams::pin(mesh, &vertices, command == "uv.pin").map_err(|error| error.to_string())?;
            Ok(json!({ "vertices": changed, "pinned": mesh.pinned.len() }))
        }
        "uv.translate" => {
            let offset = uv_pair(params.get("offset").ok_or_else(|| bad("`offset` is required"))?, "offset")?;
            transform_uvs(doc, |uv, _| uv + offset, params)
        }
        "uv.rotate" => {
            let angle = params::need_number(params, "angle")?.to_radians();
            let (sin, cos) = angle.sin_cos();
            transform_uvs(
                doc,
                |uv, centre| {
                    let d = uv - centre;
                    centre + Vec2::new(d.x * cos - d.y * sin, d.x * sin + d.y * cos)
                },
                params,
            )
        }
        "uv.scale" => {
            let factor = match params.get("factor") {
                Some(Value::Number(_)) => Vec2::splat(params::need_number(params, "factor")?),
                Some(value) => uv_pair(value, "factor")?,
                None => return Err(bad("`factor` is required")),
            };
            transform_uvs(doc, |uv, centre| centre + (uv - centre) * factor, params)
        }
        "mesh.knife" => knife(doc, params),
        "mesh.edgeSlide" => {
            let factor = params::need_number(params, "factor")?;
            let vertices = doc.session.vertices.clone();
            let mesh = mesh_mut(doc)?;
            let moved = kition_scene_model::knife::edge_slide(mesh, &vertices, factor).map_err(|error| error.to_string())?;
            Ok(json!({ "moved": moved }))
        }
        other => Err(format!("unknown-command: {other}")),
    }
}

fn knife(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let id = editing(doc).ok_or("not-allowed: not in edit mode")?;
    let through = params::boolean(params, "through")?.unwrap_or(false);
    let projector = doc.session.editor.view.projector(&doc.scene, doc.session.width, doc.session.height);
    let world = doc.scene.world_matrix(id).map_err(|error| error.to_string())?;
    let inverse = world.inverse();
    let matrix = projector.view_projection() * world;
    let to_screen = |point: Vec3| -> Option<Vec2> {
        let clip = matrix * point.extend(1.0);
        if clip.w <= 1e-6 {
            return None;
        }
        let screen = projector.to_screen(clip);
        Some(Vec2::new(screen.x, screen.y))
    };
    let points = params.get("points").and_then(Value::as_array).ok_or_else(|| bad("`points` is a list of [x, y] pixels or [x, y, z] points"))?;
    if points.len() < 2 || points.len() > 257 {
        return Err(bad("`points` needs 2 to 257 points"));
    }
    let mut line = Vec::with_capacity(points.len());
    for point in points {
        let numbers: Vec<f32> = point
            .as_array()
            .map(|items| items.iter().filter_map(Value::as_f64).filter(|value| value.is_finite()).map(|value| value as f32).collect())
            .unwrap_or_default();
        match numbers.as_slice() {
            [x, y] => line.push(Vec2::new(*x, *y)),
            [x, y, z] => {
                let screen = to_screen(inverse.transform_point3(Vec3::new(*x, *y, *z))).ok_or_else(|| bad("a knife point is behind the view"))?;
                line.push(screen);
            }
            _ => return Err(bad("each point is [x, y] or [x, y, z]")),
        }
    }
    let turn = Mat3::from_mat4(inverse);
    let ray = |screen: Vec2| -> (Vec3, Vec3) {
        let (origin, direction) = projector.ray(screen.x, screen.y);
        (inverse.transform_point3(origin), (turn * direction).normalize_or_zero())
    };
    let view = kition_scene_model::knife::View { project: &to_screen, ray: &ray };
    let mesh = doc.scene.object_mut(id).map_err(|error| error.to_string())?.mesh_mut().ok_or("unsupported: the edited object is not a mesh")?;
    let made = kition_scene_model::knife::knife(mesh, &line, &view, through).map_err(|error| error.to_string())?;
    if made.is_empty() {
        return Err("not-allowed: the knife line crosses no edge".into());
    }
    doc.session.vertices = made.clone();
    Ok(json!({ "vertices": made }))
}

/// The edited mesh's texture coordinates: each face (selected ones, or all) with its vertices,
/// coordinates, and whether it is selected; the seams and pins; and the base colour image.
fn layout(doc: &SceneDocument, params: &Value) -> Result<Value, String> {
    let id = editing(doc).ok_or("not-allowed: not in edit mode")?;
    let object = doc.scene.object(id).map_err(|error| error.to_string())?;
    let mesh = object.mesh().ok_or("unsupported: the edited object is not a mesh")?;
    layout_mesh(doc, id, mesh, params)
}

fn layout_mesh(doc: &SceneDocument, id: u32, mesh: &Mesh, params: &Value) -> Result<Value, String> {
    let object = doc.scene.object(id).map_err(|error| error.to_string())?;
    let all = params::boolean(params, "all")?.unwrap_or(false);
    let selected = crate::commands::faces_selected(&doc.session, mesh);
    const MAX_FACES: usize = 100_000;
    let mut faces = Vec::new();
    for (index, corners) in mesh.faces.iter().enumerate() {
        let chosen = selected.contains(&u32::try_from(index).unwrap_or(u32::MAX));
        if !(all || chosen) {
            continue;
        }
        if faces.len() >= MAX_FACES {
            break;
        }
        let uvs: Vec<[f32; 2]> = (0..corners.len()).map(|corner| mesh.corner_uv(index, corner).to_array()).collect();
        faces.push(json!({ "face": index, "vertices": corners, "uvs": uvs, "selected": chosen }));
    }
    let image = object.material.and_then(|material| doc.scene.material(material).ok()).and_then(|material| material.base_color_texture.clone());
    Ok(json!({
        "source": source_token(doc)?,
        "object": id,
        "hasUvs": mesh.has_uvs() && mesh.uvs_aligned(),
        "faces": faces,
        "selectedVertices": doc.session.vertices,
        "seams": mesh.seams,
        "pinned": mesh.pinned,
        "image": image,
        "liveUnwrap": doc.session.editor.uv_live_unwrap,
        "method": doc.session.editor.uv_unwrap_method,
        "iterations": doc.session.editor.uv_unwrap_iterations,
    }))
}

fn parse_method(params: &Value) -> Result<Option<Method>, String> {
    Ok(match params::string(params, "method")? {
        None => None,
        Some("angleBased") => Some(Method::AngleBased),
        Some("conformal") => Some(Method::Conformal),
        Some("minimumStretch") => Some(Method::MinimumStretch),
        Some(_) => return Err(bad("`method` must be angleBased, conformal, or minimumStretch")),
    })
}

fn iterations(params: &Value) -> Result<Option<u32>, String> {
    let value = params::integer(params, "iterations")?;
    if value.is_some_and(|value| !(1..=100).contains(&value)) {
        return Err(bad("`iterations` must be 1 to 100"));
    }
    Ok(value)
}

fn settings(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let on = params::boolean(params, "on")?.unwrap_or_else(|| {
        if params.get("method").is_none() && params.get("iterations").is_none() {
            !doc.session.editor.uv_live_unwrap
        } else {
            doc.session.editor.uv_live_unwrap
        }
    });
    let method = parse_method(params)?.unwrap_or(doc.session.editor.uv_unwrap_method);
    let iterations = iterations(params)?.unwrap_or(doc.session.editor.uv_unwrap_iterations);
    doc.session.editor.uv_live_unwrap = on;
    doc.session.editor.uv_unwrap_method = method;
    doc.session.editor.uv_unwrap_iterations = iterations;
    doc.touched();
    Ok(json!({ "on": on, "method": method, "iterations": iterations }))
}

fn preview(doc: &SceneDocument, params: &Value) -> Result<Value, String> {
    check_source(doc, params)?;
    let id = editing(doc).ok_or("not-allowed: not in edit mode")?;
    let mut mesh = doc.scene.object(id).map_err(|error| error.to_string())?.mesh().ok_or("unsupported: not a mesh")?.clone();
    let offset = uv_pair(params.get("offset").ok_or_else(|| bad("`offset` is required"))?, "offset")?;
    transform_mesh(
        &mut mesh,
        &doc.session.vertices,
        |uv, _| uv + offset,
        None,
        doc.session.editor.uv_live_unwrap,
        doc.session.editor.uv_unwrap_method,
        doc.session.editor.uv_unwrap_iterations,
    )?;
    mesh.validate().map_err(|error| error.to_string())?;
    layout_mesh(doc, id, &mesh, &json!({ "all": true }))
}

/// A UV drag depends on this mesh, its selection and its UV preferences, never on viewport
/// accumulation, camera navigation or other objects' display revisions.
fn source_token(doc: &SceneDocument) -> Result<String, String> {
    let id = editing(doc).ok_or("not-allowed: not in edit mode")?;
    let mesh = doc.scene.object(id).map_err(|error| error.to_string())?.mesh().ok_or("unsupported: not a mesh")?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut hash);
    for point in &mesh.positions {
        point.to_array().map(f32::to_bits).hash(&mut hash);
    }
    mesh.faces.hash(&mut hash);
    for face in &mesh.uvs {
        for point in face {
            point.to_array().map(f32::to_bits).hash(&mut hash);
        }
    }
    mesh.seams.hash(&mut hash);
    mesh.pinned.hash(&mut hash);
    doc.session.vertices.hash(&mut hash);
    doc.session.editor.uv_live_unwrap.hash(&mut hash);
    (doc.session.editor.uv_unwrap_method as u8).hash(&mut hash);
    doc.session.editor.uv_unwrap_iterations.hash(&mut hash);
    Ok(format!("{:016x}", hash.finish()))
}

fn check_source(doc: &SceneDocument, params: &Value) -> Result<(), String> {
    if let Some(source) = params::string(params, "source")?
        && source != source_token(doc)?
    {
        return Err("not-allowed: the UV layout changed during this drag; start a new drag".into());
    }
    Ok(())
}
