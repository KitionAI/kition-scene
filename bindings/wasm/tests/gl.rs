//! The GPU viewport's protocol: the frame description and the geometry buffers.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use kition_scene_kit::gl::{MESH_MAGIC, WORLD_MAGIC, mesh_header};
use kition_scene_model::hdr;
use serde_json::{Value, json};

fn doc() -> SceneDocument {
    let doc = SceneDocument::create("Untitled scene.kscene", 1280, 720).unwrap();
    SceneDocument::open("t.kscene", &doc.save(None).unwrap()).unwrap()
}

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn object(frame: &Value, id: u64) -> &Value {
    frame["objects"].as_array().unwrap().iter().find(|object| object["id"] == id).unwrap()
}

#[test]
fn the_frame_describes_view_objects_and_overlays() {
    let mut doc = doc();
    let frame = doc.gl_scene();
    assert_eq!(frame["supported"], true);
    assert_eq!(frame["shading"], "solid");
    assert_eq!(frame["viewMatrix"].as_array().unwrap().len(), 16);
    assert_eq!(frame["projection"].as_array().unwrap().len(), 16);
    // The cube is drawn and selected (active); the light and camera are gizmo lines.
    let objects = frame["objects"].as_array().unwrap();
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0]["class"], 2);
    assert!(frame["gizmos"]["positions"].as_array().unwrap().len() >= 6 * 2 * 3);
    assert_eq!(frame["gizmos"]["positions"].as_array().unwrap().len(), frame["gizmos"]["colors"].as_array().unwrap().len());
    assert_eq!(frame["origins"].as_array().unwrap().len(), 1);
    // Edit mode lists the selected vertices and faces.
    run(&mut doc, "mode.set", json!({ "mode": "edit" }));
    let frame = doc.gl_scene();
    assert_eq!(frame["edit"]["vertices"].as_array().unwrap().len(), 8);
    assert_eq!(frame["edit"]["faces"].as_array().unwrap().len(), 6);
}

#[test]
fn keys_change_with_geometry_but_not_with_moves_or_views() {
    let mut doc = doc();
    let first = doc.gl_scene();
    let cube = first["objects"][0]["id"].as_u64().unwrap();
    let key = object(&first, cube)["key"].clone();
    run(&mut doc, "object.translate", json!({ "offset": [1, 0, 0] }));
    run(&mut doc, "view.orbit", json!({ "turn": 30 }));
    let moved = doc.gl_scene();
    assert_eq!(object(&moved, cube)["key"], key, "a move keeps the geometry");
    assert_ne!(object(&moved, cube)["world"], object(&first, cube)["world"]);
    assert_ne!(moved["viewMatrix"], first["viewMatrix"]);
    run(&mut doc, "modifier.add", json!({ "type": "subdivision" }));
    assert_ne!(object(&doc.gl_scene(), cube)["key"], key, "a modifier changes the geometry");
}

#[test]
fn geometry_buffers_hold_every_triangle_corner() {
    let doc = doc();
    let cube = doc.gl_scene()["objects"][0]["id"].as_u64().unwrap() as u32;
    let bytes = doc.gl_mesh(cube, false).unwrap();
    assert_eq!(&bytes[0..4], MESH_MAGIC);
    let (corners, vertices, edges, flags) = mesh_header(&bytes).unwrap();
    // 6 quads make 12 triangles; 8 vertices, 12 edges; the cube is closed.
    assert_eq!((corners, vertices, edges, flags), (36, 8, 24, 1));
    assert_eq!(bytes.len(), 20 + 36 * 9 * 4 + 8 * 3 * 4 + 24 * 4);
    // Every corner's normal is a unit face normal, pointing out of the cube.
    for corner in 0..36usize {
        let at = 20 + corner * 9 * 4;
        let read = |offset: usize| f32::from_le_bytes(bytes[at + offset * 4..at + offset * 4 + 4].try_into().unwrap());
        let (position, normal) = ([read(0), read(1), read(2)], [read(3), read(4), read(5)]);
        let dot: f32 = position.iter().zip(normal).map(|(p, n)| p * n).sum();
        assert!((normal.iter().map(|n| n * n).sum::<f32>() - 1.0).abs() < 1e-5 && dot > 0.99, "corner {corner}");
    }
    let tangents = doc.gl_mesh(cube, true).unwrap();
    assert_eq!(mesh_header(&tangents).unwrap().3, 3);
    assert_eq!(tangents.len(), bytes.len() + 36 * 4 * 4);
    assert!(doc.gl_mesh(999_999, false).is_err());
}

fn read_f32(bytes: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

/// The paint block of a geometry buffer: (value, tint) per corner.
fn paint_of(bytes: &[u8]) -> Vec<(f32, [f32; 3])> {
    let (corners, _, _, flags) = mesh_header(bytes).unwrap();
    assert_eq!(flags & 4, 4, "the buffer carries paint");
    let start = 20 + corners as usize * 9 * 4 + if flags & 2 != 0 { corners as usize * 16 } else { 0 };
    (0..corners as usize)
        .map(|corner| {
            let at = start + corner * 16;
            (read_f32(bytes, at), [read_f32(bytes, at + 4), read_f32(bytes, at + 8), read_f32(bytes, at + 12)])
        })
        .collect()
}

#[test]
fn only_more_lights_than_it_shades_with_leave_the_gpu_viewport() {
    let mut doc = doc();
    run(&mut doc, "view.material", json!({}));
    for index in 0..7 {
        run(&mut doc, "add.light", json!({ "name": format!("Extra {index}"), "location": [index, 3, 3] }));
    }
    // The starter light and seven more: eight.
    assert_eq!(doc.gl_scene()["supported"], true);
    run(&mut doc, "add.light", json!({ "name": "Ninth", "location": [0, -3, 3] }));
    let frame = doc.gl_scene();
    assert_eq!(frame["supported"], false);
    assert_eq!(frame["reason"], "more than eight lights");
    run(&mut doc, "view.solid", json!({}));
    assert_eq!(doc.gl_scene()["supported"], true, "solid shading uses no lights");
}

#[test]
fn looking_through_a_camera_gives_its_view_projection_and_frame() {
    let mut doc = doc();
    let orbit = doc.gl_scene();
    assert!(orbit["camera"].is_null());
    let lines = orbit["gizmos"]["positions"].as_array().unwrap().len();
    run(&mut doc, "view.camera", json!({}));
    let frame = doc.gl_scene();
    assert_eq!(frame["supported"], true);
    let camera = &frame["camera"];
    assert_eq!(camera["viewMatrix"], frame["viewMatrix"]);
    assert_eq!(camera["projection"].as_array().unwrap().len(), 16);
    let rect: Vec<f64> = camera["frame"].as_array().unwrap().iter().map(|value| value.as_f64().unwrap()).collect();
    // The 16:9 render frame fills the 16:9 picture.
    assert!(rect[0].abs() < 1.0 && (rect[2] - 1280.0).abs() < 1.0, "{rect:?}");
    // The WebGL projection maps the near plane to −1 and the far plane to 1.
    let projection: Vec<f32> = camera["projection"].as_array().unwrap().iter().map(|value| value.as_f64().unwrap() as f32).collect();
    let near = camera["near"].as_f64().unwrap() as f32;
    let z = |depth: f32| (projection[10] * -depth + projection[14]) / (projection[11] * -depth + projection[15]);
    assert!((z(near) + 1.0).abs() < 1e-3, "{}", z(near));
    // The camera looked through draws no pyramid.
    assert!(frame["gizmos"]["positions"].as_array().unwrap().len() < lines);
}

#[test]
fn sculpt_mode_paints_clay_masks_face_sets_and_hides_faces_on_the_gpu() {
    let mut doc = doc();
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "mode.sculpt", json!({ "on": true }));
    let cube = doc.gl_scene()["objects"][0]["id"].as_u64().unwrap();
    let before = object(&doc.gl_scene(), cube)["key"].clone();
    assert_eq!(object(&doc.gl_scene(), cube)["paint"], 1, "sculpt mode draws clay");
    run(&mut doc, "sculpt.maskFill", json!({}));
    let frame = doc.gl_scene();
    assert_eq!(frame["supported"], true);
    assert_ne!(object(&frame, cube)["key"], before, "the mask is part of the geometry");
    let painted = paint_of(&doc.gl_mesh(cube as u32, false).unwrap());
    assert_eq!(painted.len(), 36);
    assert!(painted.iter().all(|(value, tint)| *value == 1.0 && *tint == [1.0; 3]), "a full mask, no face sets");
    run(&mut doc, "sculpt.maskClear", json!({}));
    run(&mut doc, "mode.sculpt", json!({ "on": false }));
    // Face sets tint, hidden faces have no triangles.
    run(&mut doc, "add.grid", json!({ "name": "Clay", "xSubdivisions": 20, "ySubdivisions": 20, "size": 2 }));
    run(&mut doc, "view.top", json!({}));
    run(&mut doc, "view.frameSelected", json!({}));
    run(&mut doc, "mode.sculpt", json!({ "on": true }));
    run(&mut doc, "sculpt.gesture", json!({ "action": "faceSet", "points": [[640, 0], [1280, 720]] }));
    run(&mut doc, "sculpt.gesture", json!({ "action": "hide", "points": [[0, 0], [560, 720]] }));
    let frame = doc.gl_scene();
    assert_eq!(frame["supported"], true);
    let clay = frame["objects"].as_array().unwrap().iter().find(|object| object["paint"] == 1).unwrap()["id"].as_u64().unwrap() as u32;
    let bytes = doc.gl_mesh(clay, false).unwrap();
    let corners = mesh_header(&bytes).unwrap().0;
    assert!(corners > 0 && corners < 20 * 20 * 6, "hidden faces are left out: {corners} corners");
    let painted = paint_of(&bytes);
    assert!(painted.iter().any(|(_, tint)| *tint != [1.0; 3]) && painted.iter().any(|(_, tint)| *tint == [1.0; 3]), "some faces tinted");
}

#[test]
fn a_multiresolution_level_is_drawn_in_place_of_the_mesh() {
    let mut doc = doc();
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "modifier.add", json!({ "type": "multires" }));
    run(&mut doc, "mode.sculpt", json!({ "on": true }));
    let base = doc.gl_scene();
    let cube = base["objects"][0]["id"].as_u64().unwrap() as u32;
    assert_eq!(mesh_header(&doc.gl_mesh(cube, false).unwrap()).unwrap().1, 8);
    run(&mut doc, "multires.subdivide", json!({}));
    let frame = doc.gl_scene();
    assert_eq!(frame["supported"], true);
    // Level 1 of a cube: 26 vertices.
    assert_eq!(mesh_header(&doc.gl_mesh(cube, false).unwrap()).unwrap().1, 26);
}

#[test]
fn weight_paint_colours_ride_in_the_geometry() {
    let mut doc = doc();
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "mode.weightPaint", json!({}));
    run(&mut doc, "paint.group", json!({ "name": "Glow" }));
    let frame = doc.gl_scene();
    assert_eq!(frame["supported"], true);
    let cube = frame["objects"][0]["id"].as_u64().unwrap();
    assert_eq!(object(&frame, cube)["paint"], 2);
    let blank = paint_of(&doc.gl_mesh(cube as u32, false).unwrap());
    assert!(blank.iter().all(|(value, _)| *value == 0.0));
    run(&mut doc, "paint.vertices", json!({ "group": "Glow", "vertices": [0, 1], "weight": 0.75 }));
    let painted = paint_of(&doc.gl_mesh(cube as u32, false).unwrap());
    assert!(painted.iter().any(|(value, _)| (*value - 0.75).abs() < 1e-6), "the stroke's weight");
}

#[test]
fn particles_are_drawn_as_the_meshes_the_software_viewport_makes() {
    let mut doc = doc();
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "particles.add", json!({ "kind": "hair", "hairLength": 1, "count": 100 }));
    let frame = doc.gl_scene();
    assert_eq!(frame["supported"], true);
    let objects = frame["objects"].as_array().unwrap();
    assert_eq!(objects.len(), 2, "the cube and its hair");
    let hair = objects[1]["id"].as_u64().unwrap() as u32;
    let (corners, ..) = mesh_header(&doc.gl_mesh(hair, false).unwrap()).unwrap();
    assert!(corners >= 100 * 3, "{corners} corners of ribbons");
}

#[test]
fn the_material_view_carries_shadow_plans_and_the_world_image() {
    let mut doc = doc();
    assert!(doc.gl_scene()["shadowKey"].is_null(), "solid shading has no shadows");
    run(&mut doc, "view.material", json!({}));
    let frame = doc.gl_scene();
    let shadows = frame["shadows"].as_array().unwrap();
    assert_eq!(shadows.len(), frame["lights"].as_array().unwrap().len());
    // The starter light is a point light: a cube of six pictures of 512 texels.
    let faces = shadows[0]["faces"].as_array().unwrap();
    assert_eq!(faces.len(), 6);
    assert_eq!(faces[0]["size"], 512);
    assert_eq!(shadows[0]["taps"], 2);
    assert!(shadows[0]["centre"].is_array());
    // Orbiting keeps the shadows; moving a caster changes them.
    let key = frame["shadowKey"].clone();
    run(&mut doc, "view.orbit", json!({ "turn": 30 }));
    assert_eq!(doc.gl_scene()["shadowKey"], key);
    run(&mut doc, "object.translate", json!({ "offset": [0, 0, 1] }));
    assert_ne!(doc.gl_scene()["shadowKey"], key);
    // A world image: its irradiance and turn in the frame, its texels from glWorld.
    assert!(frame["environment"].is_null());
    assert!(doc.gl_world().is_empty());
    let sky = hdr::encode(8, 4, &[[0.2, 0.4, 2.0]; 32]);
    run(&mut doc, "world.setImage", json!({ "base64": base64(&sky), "name": "Sky.hdr", "rotation": 90, "strength": 1.5 }));
    let frame = doc.gl_scene();
    assert_eq!(frame["supported"], true);
    assert_eq!(frame["environment"]["coefficients"].as_array().unwrap().len(), 27);
    assert_eq!(frame["environment"]["turn"].as_array().unwrap().len(), 9);
    let world = doc.gl_world();
    assert_eq!(&world[0..4], WORLD_MAGIC);
    assert_eq!(world.len(), 12 + 8 * 4 * 16);
    // Linear texels with the strength in.
    assert!((read_f32(&world, 12 + 8) - 3.0).abs() < 0.05, "{}", read_f32(&world, 12 + 8));
}

#[test]
fn edit_modes_draw_their_points_handles_and_bones_over_everything() {
    let mut doc = doc();
    run(&mut doc, "add.curveBezier", json!({ "name": "Path" }));
    run(&mut doc, "mode.shape", json!({}));
    let frame = doc.gl_scene();
    assert_eq!(frame["supported"], true);
    // A Bézier curve's two points: each a dot and two handle dots, with handle lines.
    assert_eq!(frame["dots"]["sizes"].as_array().unwrap().len(), 6);
    assert_eq!(frame["front"]["positions"].as_array().unwrap().len(), 4 * 2 * 3);
    run(&mut doc, "mode.shape", json!({}));
    run(&mut doc, "add.armature", json!({ "name": "Rig" }));
    run(&mut doc, "mode.armature", json!({}));
    let frame = doc.gl_scene();
    assert_eq!(frame["supported"], true);
    assert_eq!(frame["dots"]["sizes"].as_array().unwrap().len(), 2, "the bone's head and tail");
    assert_eq!(frame["front"]["positions"].as_array().unwrap().len(), 12 * 2 * 3, "the bone in front");
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let triple = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        for index in 0..4 {
            out.push(if index <= chunk.len() { TABLE[((triple >> (18 - 6 * index)) & 63) as usize] as char } else { '=' });
        }
    }
    out
}
