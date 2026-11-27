//! Selecting by hand (plan scene-usable-by-hand, phase 6): clicks, boxes, circles and lassos pick
//! with rays and draw no software picture; without X-ray only what shows is taken, with X-ray
//! what is hidden too; edit mode's loops, rings, linked parts, more and less and the shortest
//! path; X-ray as view state; an undo keeps the selection that still exists. None of it is an
//! undo step or an unsaved change.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::collections::BTreeSet;

use kition_kit::KitDocument;
use kition_kit::tools::{KitTools, PointerAction, ToolMods, ToolPointer};
use kition_scene_kit::SceneDocument;
use kition_scene_model::glam::{Vec3, Vec4};
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

/// The starter scene (cube, light, camera) in the opening view, framed, nothing selected, saved.
fn starter() -> SceneDocument {
    let mut doc = SceneDocument::create("Select.kscene", 800, 600).unwrap();
    run(&mut doc, "view.frameAll", json!({}));
    run(&mut doc, "edit.selectAll", json!({ "action": "deselect" }));
    doc.mark_saved();
    doc
}

fn screen(doc: &SceneDocument, point: Vec3) -> [f64; 2] {
    let projector = doc.session().editor.view.projector(doc.scene(), doc.width(), doc.height());
    let at = projector.to_screen(projector.view_projection() * Vec4::new(point.x, point.y, point.z, 1.0));
    [f64::from(at.x), f64::from(at.y)]
}

fn eye(doc: &SceneDocument) -> Vec3 {
    doc.session().editor.view.projector(doc.scene(), doc.width(), doc.height()).eye()
}

fn id(doc: &SceneDocument, name: &str) -> u32 {
    doc.scene().object_named(name).unwrap().id
}

fn selected(doc: &SceneDocument) -> BTreeSet<String> {
    doc.session().editor.selected.iter().map(|id| doc.scene().object(*id).unwrap().name.clone()).collect()
}

fn names(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

fn event(doc: &mut SceneDocument, action: PointerAction, at: [f64; 2], mods: ToolMods) {
    doc.pointer(&ToolPointer { action, x: at[0], y: at[1], mods, pressure: 1.0, zoom: 1.0 }).unwrap();
}

fn click(doc: &mut SceneDocument, at: [f64; 2], mods: ToolMods) {
    event(doc, PointerAction::Down, at, mods);
    event(doc, PointerAction::Up, at, mods);
}

fn drag(doc: &mut SceneDocument, from: [f64; 2], to: [f64; 2], mods: ToolMods) {
    event(doc, PointerAction::Down, from, mods);
    event(doc, PointerAction::Drag, [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0], mods);
    event(doc, PointerAction::Drag, to, mods);
    event(doc, PointerAction::Up, to, mods);
}

fn shift() -> ToolMods {
    ToolMods { shift: true, ..ToolMods::default() }
}

fn command() -> ToolMods {
    ToolMods { command: true, ..ToolMods::default() }
}

/// The edited mesh's world-space vertices.
fn world_vertices(doc: &SceneDocument, name: &str) -> Vec<Vec3> {
    let object = doc.scene().object_named(name).unwrap();
    let world = doc.scene().world_matrix(object.id).unwrap();
    object.mesh().unwrap().positions.iter().map(|point| world.transform_point3(*point)).collect()
}

fn vertices(doc: &SceneDocument) -> BTreeSet<u32> {
    doc.session().vertices.clone()
}

/// The step an undo would take back now.
fn history(doc: &SceneDocument) -> Value {
    doc.inspect().unwrap()["undo"].clone()
}

/// Nothing a selection does is an undo step or an unsaved change: the history is as `before`.
fn untouched(doc: &SceneDocument, before: &Value) {
    assert!(!doc.dirty(), "selecting leaves the document clean");
    assert_eq!(&history(doc), before, "selecting makes no undo step");
}

#[test]
fn clicks_and_boxes_pick_with_rays_and_draw_no_software_picture() {
    let mut doc = starter();
    let frames = doc.software_frames();
    let cube = screen(&doc, Vec3::ZERO);
    click(&mut doc, cube, ToolMods::default());
    assert_eq!(selected(&doc), names(&["Cube"]), "a click on the cube selects it");
    click(&mut doc, [4.0, 4.0], ToolMods::default());
    assert!(selected(&doc).is_empty(), "a click on nothing clears the selection");
    let light = screen(&doc, doc.scene().object_named("Light").unwrap().location);
    click(&mut doc, light, ToolMods::default());
    assert_eq!(selected(&doc), names(&["Light"]), "a click on the light's drawing selects it");
    drag(&mut doc, [2.0, 2.0], [798.0, 598.0], ToolMods::default());
    assert!(
        selected(&doc).contains("Cube") && selected(&doc).contains("Light"),
        "a box over everything takes the cube and the light: {:?}",
        selected(&doc)
    );
    drag(&mut doc, [cube[0] - 10.0, cube[1] - 10.0], [cube[0] + 10.0, cube[1] + 10.0], command());
    assert!(!selected(&doc).contains("Cube"), "Cmd/Ctrl+box takes the cube out");
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "mode.set", json!({ "mode": "edit" }));
    drag(&mut doc, [2.0, 2.0], [798.0, 598.0], ToolMods::default());
    assert!(!vertices(&doc).is_empty(), "a box in edit mode takes vertices");
    assert_eq!(doc.software_frames(), frames, "no click or box drew a software picture");
    doc.render(0.0, 0.0, 1.0, 80, 60).unwrap();
    assert_eq!(doc.software_frames(), frames + 1, "the agent's picture is drawn when asked for");
}

#[test]
fn while_the_gpu_viewport_draws_an_event_owes_the_software_picture_nothing() {
    let mut doc = starter();
    let cube = screen(&doc, Vec3::ZERO);
    click(&mut doc, cube, ToolMods::default());
    assert!(doc.take_damage().is_none(), "without the GPU viewport the picture repaints");
    doc.set_gl_shown(true);
    click(&mut doc, [4.0, 4.0], ToolMods::default());
    let damage = doc.take_damage().expect("an area");
    assert!(damage.x1 <= 0.0 && damage.y1 <= 0.0 && damage.x1 > damage.x0, "an area outside the picture: {damage:?}");
    doc.set_gl_shown(false);
    click(&mut doc, cube, ToolMods::default());
    assert!(doc.take_damage().is_none());
}

/// A small cube straight behind the starter cube, seen from the eye.
fn hidden_cube(doc: &mut SceneDocument) {
    let behind = (Vec3::ZERO - eye(doc)).normalize() * 6.0;
    run(doc, "object.addCube", json!({ "name": "Behind", "size": 0.5, "location": [behind.x, behind.y, behind.z] }));
    run(doc, "edit.selectAll", json!({ "action": "deselect" }));
    doc.mark_saved();
}

#[test]
fn without_xray_a_box_takes_only_what_shows_and_with_xray_what_is_hidden_too() {
    let mut doc = starter();
    hidden_cube(&mut doc);
    let before = history(&doc);
    let cube = screen(&doc, Vec3::ZERO);
    click(&mut doc, cube, ToolMods::default());
    assert_eq!(selected(&doc), names(&["Cube"]), "a click takes the cube in front");
    drag(&mut doc, [cube[0] - 40.0, cube[1] - 40.0], [cube[0] + 40.0, cube[1] + 40.0], ToolMods::default());
    assert_eq!(selected(&doc), names(&["Cube"]), "the box leaves the hidden cube");
    run(&mut doc, "view.toggleXray", json!({}));
    drag(&mut doc, [cube[0] - 40.0, cube[1] - 40.0], [cube[0] + 40.0, cube[1] + 40.0], ToolMods::default());
    assert_eq!(selected(&doc), names(&["Cube", "Behind"]), "with X-ray the box takes the hidden cube too");
    run(&mut doc, "view.toggleXray", json!({}));
    run(&mut doc, "view.shading", json!({ "mode": "wireframe" }));
    drag(&mut doc, [cube[0] - 40.0, cube[1] - 40.0], [cube[0] + 40.0, cube[1] + 40.0], ToolMods::default());
    assert_eq!(selected(&doc), names(&["Cube", "Behind"]), "wireframe selects through");
    untouched(&doc, &before);
}

/// The starter cube in edit mode, nothing selected, and its vertex farthest from the eye (the one
/// the cube hides).
fn editing_cube(doc: &mut SceneDocument) -> u32 {
    run(doc, "select.objects", json!({ "objects": "Cube" }));
    run(doc, "view.frameSelected", json!({}));
    run(doc, "mode.set", json!({ "mode": "edit" }));
    run(doc, "mesh.select", json!({ "vertices": [] }));
    let eye = eye(doc);
    let points = world_vertices(doc, "Cube");
    let far = (0..points.len()).max_by(|a, b| points[*a].distance(eye).total_cmp(&points[*b].distance(eye))).unwrap();
    doc.mark_saved();
    far as u32
}

#[test]
fn edit_mode_takes_the_vertices_that_show_and_x_ray_takes_them_all() {
    let mut doc = starter();
    let hidden = editing_cube(&mut doc);
    let before = history(&doc);
    drag(&mut doc, [2.0, 2.0], [798.0, 598.0], ToolMods::default());
    assert_eq!(vertices(&doc).len(), 7, "the box takes the seven vertices that show: {:?}", vertices(&doc));
    assert!(!vertices(&doc).contains(&hidden));
    let at = screen(&doc, world_vertices(&doc, "Cube")[hidden as usize]);
    run(&mut doc, "mesh.select", json!({ "vertices": [] }));
    click(&mut doc, at, ToolMods::default());
    assert!(vertices(&doc).is_empty(), "a click where the hidden vertex lies takes nothing: {:?}", vertices(&doc));
    run(&mut doc, "view.toggleXray", json!({ "value": true }));
    click(&mut doc, at, ToolMods::default());
    assert_eq!(vertices(&doc), BTreeSet::from([hidden]), "with X-ray the click takes it");
    drag(&mut doc, [2.0, 2.0], [798.0, 598.0], ToolMods::default());
    assert_eq!(vertices(&doc).len(), 8, "with X-ray the box takes all eight");
    run(&mut doc, "view.toggleXray", json!({ "value": false }));
    // A face: the one nearest the eye, clicked in its middle.
    run(&mut doc, "mesh.selectFaceMode", json!({}));
    run(&mut doc, "mesh.select", json!({ "vertices": [] }));
    let points = world_vertices(&doc, "Cube");
    let mesh = doc.scene().object_named("Cube").unwrap().mesh().unwrap().clone();
    let eye = eye(&doc);
    let middle = |face: &Vec<u32>| face.iter().map(|vertex| points[*vertex as usize]).sum::<Vec3>() / face.len() as f32;
    let near = mesh.faces.iter().min_by(|a, b| middle(a).distance(eye).total_cmp(&middle(b).distance(eye))).unwrap().clone();
    let at = screen(&doc, middle(&near));
    click(&mut doc, at, ToolMods::default());
    assert_eq!(vertices(&doc), near.iter().copied().collect::<BTreeSet<u32>>(), "a click in a face takes its corners");
    untouched(&doc, &before);
}

/// A UV sphere in edit mode (32 segments, 16 rings), nothing selected, and its equator.
fn editing_sphere(doc: &mut SceneDocument) -> Vec<u32> {
    run(doc, "edit.delete", json!({ "objects": ["Cube"] }));
    run(doc, "object.addUvSphere", json!({ "name": "Ball", "radius": 2, "location": [0, 0, 0] }));
    run(doc, "view.frameSelected", json!({}));
    run(doc, "mode.set", json!({ "mode": "edit" }));
    run(doc, "mesh.select", json!({ "vertices": [] }));
    doc.mark_saved();
    let points = world_vertices(doc, "Ball");
    (0..points.len() as u32).filter(|index| points[*index as usize].z.abs() < 1e-4).collect()
}

/// The sphere's edge nearest the eye whose ends `keep` takes, at its middle on the screen.
fn edge_facing(doc: &SceneDocument, keep: impl Fn(Vec3, Vec3) -> bool) -> [f64; 2] {
    let points = world_vertices(doc, "Ball");
    let mesh = doc.scene().object_named("Ball").unwrap().mesh().unwrap().clone();
    let eye = eye(doc);
    let [a, b] = mesh
        .edges()
        .into_iter()
        .filter(|[a, b]| keep(points[*a as usize], points[*b as usize]))
        .min_by(|x, y| {
            let mid = |[a, b]: &[u32; 2]| (points[*a as usize] + points[*b as usize]) / 2.0;
            mid(x).distance(eye).total_cmp(&mid(y).distance(eye))
        })
        .unwrap();
    screen(doc, (points[a as usize] + points[b as usize]) / 2.0)
}

#[test]
fn loops_rings_and_face_loops_follow_the_quads() {
    let mut doc = starter();
    let equator = editing_sphere(&mut doc);
    let before = history(&doc);
    assert_eq!(equator.len(), 32);
    let along = edge_facing(&doc, |a, b| a.z.abs() < 1e-4 && b.z.abs() < 1e-4);
    run(&mut doc, "select.loop", json!({ "position": along }));
    assert_eq!(vertices(&doc), equator.iter().copied().collect::<BTreeSet<u32>>(), "Alt+click: the equator's loop");
    let points = world_vertices(&doc, "Ball");
    let across = edge_facing(&doc, |a, b| (a.z.abs() < 1e-4) != (b.z.abs() < 1e-4) && a.z.max(b.z) > 0.0 && a.z.min(b.z) > -1e-4);
    run(&mut doc, "select.ring", json!({ "position": across }));
    let ring = vertices(&doc);
    assert_eq!(ring.len(), 64, "Ctrl+Alt+click: the ring across the quads above the equator");
    assert!(ring.iter().all(|vertex| points[*vertex as usize].z > -1e-4));
    run(&mut doc, "mesh.selectFaceMode", json!({}));
    run(&mut doc, "select.loop", json!({ "position": across }));
    assert_eq!(vertices(&doc), ring, "in face mode Alt+click takes the band of faces across the edge");
    run(&mut doc, "mesh.selectVertexMode", json!({}));
    run(&mut doc, "mesh.select", json!({ "vertices": [] }));
    run(&mut doc, "select.loop", json!({ "position": along, "extend": true }));
    run(&mut doc, "select.ring", json!({ "position": across, "extend": true }));
    assert_eq!(vertices(&doc), ring, "extend adds to the selection");
    untouched(&doc, &before);
}

#[test]
fn more_less_linked_and_the_shortest_path() {
    let mut doc = starter();
    let equator = editing_sphere(&mut doc);
    let points = world_vertices(&doc, "Ball");
    let eye = eye(&doc);
    let front = *equator.iter().min_by(|a, b| points[**a as usize].distance(eye).total_cmp(&points[**b as usize].distance(eye))).unwrap();
    run(&mut doc, "mesh.select", json!({ "vertices": [front] }));
    run(&mut doc, "select.more", json!({}));
    assert_eq!(vertices(&doc).len(), 9, "More: the four faces round the vertex");
    run(&mut doc, "select.less", json!({}));
    assert_eq!(vertices(&doc), BTreeSet::from([front]), "Less takes them back");
    // The shortest path: a click, then Cmd/Ctrl+click three steps along the equator.
    let angle = |vertex: u32| points[vertex as usize].y.atan2(points[vertex as usize].x);
    let step = std::f32::consts::TAU / 32.0;
    let third = *equator
        .iter()
        .min_by(|a, b| {
            ((angle(**a) - angle(front)).rem_euclid(std::f32::consts::TAU) - 3.0 * step)
                .abs()
                .total_cmp(&((angle(**b) - angle(front)).rem_euclid(std::f32::consts::TAU) - 3.0 * step).abs())
        })
        .unwrap();
    run(&mut doc, "mesh.select", json!({ "vertices": [] }));
    let at = screen(&doc, points[front as usize]);
    click(&mut doc, at, ToolMods::default());
    assert_eq!(vertices(&doc), BTreeSet::from([front]));
    let at = screen(&doc, points[third as usize]);
    click(&mut doc, at, command());
    let path = vertices(&doc);
    assert_eq!(path.len(), 4, "Cmd/Ctrl+click adds the shortest path: {path:?}");
    assert!(path.iter().all(|vertex| equator.contains(vertex)), "along the equator");
    // Linked: two cubes joined into one mesh.
    run(&mut doc, "mode.set", json!({ "mode": "object" }));
    run(&mut doc, "edit.delete", json!({ "objects": ["Ball"] }));
    run(&mut doc, "object.addCube", json!({ "name": "Left", "location": [-2, 0, 0], "size": 1 }));
    run(&mut doc, "object.addCube", json!({ "name": "Right", "location": [2, 0, 0], "size": 1 }));
    run(&mut doc, "select.objects", json!({ "objects": ["Left", "Right"] }));
    run(&mut doc, "object.join", json!({}));
    let joined = doc.session().editor.active.unwrap();
    let name = doc.scene().object(joined).unwrap().name.clone();
    run(&mut doc, "view.frameSelected", json!({}));
    run(&mut doc, "mode.set", json!({ "mode": "edit" }));
    run(&mut doc, "mesh.select", json!({ "vertices": [0] }));
    run(&mut doc, "select.linked", json!({}));
    let first = vertices(&doc);
    assert_eq!(first.len(), 8, "Ctrl+L: the part the selection touches");
    let points = world_vertices(&doc, &name);
    let eye = self::eye(&doc);
    let other = (0..16u32)
        .filter(|vertex| !first.contains(vertex))
        .min_by(|a, b| points[*a as usize].distance(eye).total_cmp(&points[*b as usize].distance(eye)))
        .unwrap();
    let at = screen(&doc, points[other as usize]);
    run(&mut doc, "select.linked", json!({ "position": at }));
    assert_eq!(vertices(&doc).len(), 16, "L: the part under the pointer");
    run(&mut doc, "select.linked", json!({ "position": at, "deselect": true }));
    assert_eq!(vertices(&doc), first, "Shift+L takes it out");
}

#[test]
fn circles_and_lassos_select_in_object_and_edit_mode() {
    let mut doc = starter();
    let before = history(&doc);
    let cube = screen(&doc, Vec3::ZERO);
    let light = screen(&doc, doc.scene().object_named("Light").unwrap().location);
    run(&mut doc, "select.circle", json!({ "position": cube, "radius": 10 }));
    assert_eq!(selected(&doc), names(&["Cube"]));
    run(&mut doc, "select.circle", json!({ "position": light, "radius": 10, "mode": "extend" }));
    assert_eq!(selected(&doc), names(&["Cube", "Light"]));
    run(&mut doc, "select.circle", json!({ "position": cube, "radius": 10, "mode": "subtract" }));
    assert_eq!(selected(&doc), names(&["Light"]));
    let around = |at: [f64; 2], size: f64| {
        json!([[at[0] - size, at[1] - size], [at[0] + size, at[1] - size], [at[0] + size, at[1] + size], [at[0] - size, at[1] + size]])
    };
    run(&mut doc, "select.lasso", json!({ "points": around(cube, 20.0) }));
    assert_eq!(selected(&doc), names(&["Cube"]));
    assert!(doc.execute("select.lasso", json!({ "points": [[0, 0], [1, 1]] })).is_err(), "a lasso needs three points");
    assert!(doc.execute("select.circle", json!({ "position": cube, "radius": -1 })).is_err());
    // The tools: Select Lasso's drag and Select Circle's press.
    doc.select_tool("selectLasso", 1.0).unwrap();
    run(&mut doc, "edit.selectAll", json!({ "action": "deselect" }));
    event(&mut doc, PointerAction::Down, [cube[0] - 25.0, cube[1] - 25.0], ToolMods::default());
    for at in [[cube[0] + 25.0, cube[1] - 25.0], [cube[0] + 25.0, cube[1] + 25.0], [cube[0] - 25.0, cube[1] + 25.0]] {
        event(&mut doc, PointerAction::Drag, at, ToolMods::default());
    }
    event(&mut doc, PointerAction::Up, [cube[0] - 25.0, cube[1] + 25.0], ToolMods::default());
    assert_eq!(selected(&doc), names(&["Cube"]), "Select Lasso takes what it goes round");
    doc.select_tool("selectCircle", 1.0).unwrap();
    click(&mut doc, light, shift());
    assert_eq!(selected(&doc), names(&["Cube", "Light"]), "Select Circle with Shift adds");
    click(&mut doc, light, ToolMods::default());
    assert_eq!(selected(&doc), names(&["Light"]), "Select Circle's press sets");
    // Edit mode.
    let hidden = editing_cube(&mut doc);
    let at = screen(&doc, world_vertices(&doc, "Cube")[hidden as usize]);
    run(&mut doc, "select.circle", json!({ "position": at, "radius": 6 }));
    assert!(vertices(&doc).is_empty(), "the circle leaves the hidden vertex");
    run(&mut doc, "select.circle", json!({ "position": [400, 300], "radius": 1000 }));
    assert_eq!(vertices(&doc).len(), 7);
    untouched(&doc, &before);
}

#[test]
fn xray_is_view_state_and_the_gpu_frame_shows_it() {
    let mut doc = starter();
    assert_eq!(doc.inspect().unwrap()["view"]["xray"], json!(false));
    assert_eq!(run(&mut doc, "view.toggleXray", json!({}))["xray"], json!(true));
    assert_eq!(doc.inspect().unwrap()["view"]["xray"], json!(true));
    assert_eq!(doc.gl_scene()["xray"], json!(true));
    assert_eq!(doc.view_state()["xray"], json!(true));
    untouched(&doc, &json!(null));
    let state = doc.view_state();
    let mut other = SceneDocument::create("Other.kscene", 800, 600).unwrap();
    assert!(other.restore_view_state(&state));
    assert_eq!(other.inspect().unwrap()["view"]["xray"], json!(true), "the view state carries it");
    run(&mut doc, "view.toggleXray", json!({ "value": false }));
    assert_eq!(doc.gl_scene()["xray"], json!(false));
    let commands = doc.commands();
    for id in ["select.circle", "select.lasso", "select.loop", "select.ring", "select.linked", "select.more", "select.less", "view.toggleXray"] {
        let command = commands.iter().find(|command| command.id == id).unwrap_or_else(|| panic!("{id} is listed"));
        assert!(!command.mutates, "{id} changes no document");
    }
}

#[test]
fn an_undo_keeps_the_selection_that_still_exists() {
    let mut doc = starter();
    run(&mut doc, "select.objects", json!({ "objects": ["Light", "Cube"] }));
    run(&mut doc, "object.translate", json!({ "objects": ["Cube"], "offset": [1, 0, 0] }));
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(selected(&doc), names(&["Cube", "Light"]), "the objects stay selected");
    assert_eq!(doc.session().editor.active, Some(id(&doc, "Cube")), "and the active one stays");
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "mode.set", json!({ "mode": "edit" }));
    run(&mut doc, "mesh.selectFaceMode", json!({}));
    run(&mut doc, "mesh.select", json!({ "faces": [0] }));
    let face = vertices(&doc);
    run(&mut doc, "mesh.translate", json!({ "offset": [0, 0, 1] }));
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(vertices(&doc), face, "the face stays selected: the mesh has as many elements");
    run(&mut doc, "edit.redo", json!({}));
    assert_eq!(vertices(&doc), face, "and after a redo");
    run(&mut doc, "mesh.extrude", json!({ "distance": 1 }));
    assert!(!vertices(&doc).is_empty());
    run(&mut doc, "edit.undo", json!({}));
    assert!(vertices(&doc).is_empty(), "an undo that changes the element count clears the elements");
}
