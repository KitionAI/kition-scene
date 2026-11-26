//! The viewport's basics by hand (plan scene-usable-by-hand, phase 1): what a click, a box, the
//! context-menu key and a number field's drag do to the selection and the scene, through the
//! engine's tools and commands as the editor drives them.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_kit::tools::{KitTools, PointerAction, ToolMods, ToolOverlay, ToolPointer};
use kition_scene_kit::SceneDocument;
use kition_scene_model::glam::{Vec3, Vec4};
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn starter() -> SceneDocument {
    let mut doc = SceneDocument::create("Basics.kscene", 800, 600).unwrap();
    run(&mut doc, "object.addCube", json!({ "name": "Other", "location": [4, 0, 0] }));
    run(&mut doc, "view.front", json!({}));
    run(&mut doc, "view.frameAll", json!({}));
    doc.mark_saved();
    doc
}

fn id(doc: &SceneDocument, name: &str) -> kition_scene_model::Id {
    doc.scene().object_named(name).unwrap_or_else(|| panic!("no {name}")).id
}

/// The selected objects' names, sorted.
fn selected(doc: &SceneDocument) -> Vec<String> {
    let inspected = doc.inspect().unwrap();
    let ids: Vec<u64> = inspected["selection"]["objects"].as_array().unwrap().iter().map(|id| id.as_u64().unwrap()).collect();
    let mut names: Vec<String> =
        doc.scene().objects.iter().filter(|object| ids.contains(&u64::from(object.id))).map(|object| object.name.clone()).collect();
    names.sort();
    names
}

fn active(doc: &SceneDocument) -> Option<String> {
    let active = doc.inspect().unwrap()["selection"]["active"].as_u64()?;
    doc.scene().objects.iter().find(|object| u64::from(object.id) == active).map(|object| object.name.clone())
}

/// A world point (in `name`'s own space) on the screen, in document pixels.
fn screen(doc: &SceneDocument, name: &str, local: Vec3) -> (f64, f64) {
    let world = doc.scene().world_matrix(id(doc, name)).unwrap();
    let projector = doc.session().editor.view.projector(doc.scene(), doc.width(), doc.height());
    let point = world.transform_point3(local);
    let at = projector.to_screen(projector.view_projection() * Vec4::new(point.x, point.y, point.z, 1.0));
    (f64::from(at.x), f64::from(at.y))
}

fn event(action: PointerAction, at: (f64, f64), mods: ToolMods) -> ToolPointer {
    ToolPointer { action, x: at.0, y: at.1, mods, pressure: 1.0, zoom: 1.0 }
}

fn click(doc: &mut SceneDocument, at: (f64, f64), mods: ToolMods) {
    doc.pointer(&event(PointerAction::Down, at, mods)).unwrap();
    doc.pointer(&event(PointerAction::Up, at, mods)).unwrap();
}

/// A drag from `from` to `to` in a few steps, with `mods` held.
fn drag(doc: &mut SceneDocument, from: (f64, f64), to: (f64, f64), mods: ToolMods) {
    doc.pointer(&event(PointerAction::Down, from, mods)).unwrap();
    for step in 1..=4 {
        let t = f64::from(step) / 4.0;
        doc.pointer(&event(PointerAction::Drag, (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t), mods)).unwrap();
    }
    doc.pointer(&event(PointerAction::Up, to, mods)).unwrap();
}

/// A box round `name`'s origin, `half` pixels each way.
fn around(doc: &SceneDocument, name: &str, half: f64) -> ((f64, f64), (f64, f64)) {
    let (x, y) = screen(doc, name, Vec3::ZERO);
    ((x - half, y - half), (x + half, y + half))
}

const SHIFT: ToolMods = ToolMods { shift: true, alt: false, command: false, control: false, space: false };
const COMMAND: ToolMods = ToolMods { shift: false, alt: false, command: true, control: false, space: false };
const BOTH: ToolMods = ToolMods { shift: true, alt: false, command: true, control: false, space: false };
const NONE: ToolMods = ToolMods { shift: false, alt: false, command: false, control: false, space: false };

/// A corner of the viewport with nothing drawn there.
const EMPTY: (f64, f64) = (12.0, 12.0);

#[test]
fn drag_overlays_stay_while_the_tools_state_is_asked() {
    // The editor asks the tools for their state after every change; before, that answer dropped
    // the drag's readout and dashed line, so they never showed while rotating or scaling.
    let mut doc = starter();
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    doc.select_tool("rotate", 1.0).unwrap();
    let (x, y) = screen(&doc, "Cube", Vec3::ZERO);
    doc.pointer(&event(PointerAction::Down, (x + 60.0, y), NONE)).unwrap();
    doc.pointer(&event(PointerAction::Drag, (x + 40.0, y - 50.0), NONE)).unwrap();
    let state = doc.state(x + 40.0, y - 50.0, NONE, 1.0);
    assert!(state.busy);
    assert!(
        state.overlays.iter().any(|overlay| matches!(overlay, ToolOverlay::Measure { text, .. } if text.starts_with("Rotation:"))),
        "{:?}",
        state.overlays
    );
    assert!(state.overlays.iter().any(|overlay| matches!(overlay, ToolOverlay::Line { dashed: true, .. })), "{:?}", state.overlays);
    doc.pointer(&event(PointerAction::Up, (x + 40.0, y - 50.0), NONE)).unwrap();
    assert!(doc.state(x, y, NONE, 1.0).overlays.is_empty(), "the readout goes with the drag");
    // The Select tool's box likewise.
    doc.select_tool("select", 1.0).unwrap();
    doc.pointer(&event(PointerAction::Down, EMPTY, NONE)).unwrap();
    doc.pointer(&event(PointerAction::Drag, (120.0, 90.0), NONE)).unwrap();
    let state = doc.state(120.0, 90.0, NONE, 1.0);
    assert!(state.overlays.iter().any(|overlay| matches!(overlay, ToolOverlay::Marquee { .. })), "{:?}", state.overlays);
    doc.pointer(&event(PointerAction::Up, (120.0, 90.0), NONE)).unwrap();
}

#[test]
fn a_click_with_a_transform_tool_selects() {
    // Before, Move, Rotate and Scale kept the selection on a click: another object stayed
    // unselected, and a click on nothing cleared nothing.
    let mut doc = starter();
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    for tool in ["move", "rotate", "scale"] {
        doc.select_tool(tool, 1.0).unwrap();
        let undo = doc.inspect().unwrap()["undo"].clone();
        let at = screen(&doc, "Other", Vec3::ZERO);
        click(&mut doc, at, NONE);
        assert_eq!(selected(&doc), ["Other"], "{tool}: a click on another object selects it");
        assert_eq!(active(&doc).as_deref(), Some("Other"));
        click(&mut doc, EMPTY, NONE);
        assert!(selected(&doc).is_empty(), "{tool}: a click on nothing clears the selection");
        let at = screen(&doc, "Cube", Vec3::ZERO);
        click(&mut doc, at, NONE);
        let at = screen(&doc, "Other", Vec3::ZERO);
        click(&mut doc, at, SHIFT);
        assert_eq!(selected(&doc), ["Cube", "Other"], "{tool}: Shift adds");
        // A press that moves a pixel or two is a click, not a move.
        let (x, y) = screen(&doc, "Cube", Vec3::ZERO);
        doc.pointer(&event(PointerAction::Down, (x, y), NONE)).unwrap();
        doc.pointer(&event(PointerAction::Drag, (x + 2.0, y + 1.0), NONE)).unwrap();
        doc.pointer(&event(PointerAction::Up, (x + 2.0, y + 1.0), NONE)).unwrap();
        assert_eq!(selected(&doc), ["Cube"]);
        assert_eq!(doc.scene().object_named("Cube").unwrap().location, Vec3::ZERO, "{tool}");
        assert_eq!(doc.inspect().unwrap()["undo"], undo, "{tool}: clicks leave no undo step");
    }
    // A drag still moves the selection, one step.
    doc.select_tool("move", 1.0).unwrap();
    let (x, y) = screen(&doc, "Cube", Vec3::ZERO);
    drag(&mut doc, (x, y), (x + 80.0, y), NONE);
    assert!(doc.scene().object_named("Cube").unwrap().location.x > 0.5);
    assert_eq!(doc.inspect().unwrap()["undo"], "Move");
}

#[test]
fn box_select_adds_with_shift_and_takes_away_with_command() {
    // Before, Shift with a box toggled: what was selected and inside the box was unselected.
    let mut doc = starter();
    doc.select_tool("select", 1.0).unwrap();
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    let (from, to) = around(&doc, "Other", 40.0);
    drag(&mut doc, from, to, SHIFT);
    assert_eq!(selected(&doc), ["Cube", "Other"], "Shift adds what the box finds");
    let (from, to) = around(&doc, "Cube", 40.0);
    drag(&mut doc, from, to, SHIFT);
    assert_eq!(selected(&doc), ["Cube", "Other"], "Shift never takes away");
    drag(&mut doc, from, to, COMMAND);
    assert_eq!(selected(&doc), ["Other"], "Cmd/Ctrl takes away what the box finds");
    run(&mut doc, "select.objects", json!({ "objects": ["Cube", "Other"] }));
    drag(&mut doc, from, to, BOTH);
    assert_eq!(selected(&doc), ["Cube"], "Shift and Cmd/Ctrl keep what is in both");
    drag(&mut doc, from, to, NONE);
    assert_eq!(selected(&doc), ["Cube"], "a plain box sets the selection");
    // Shift with a click still toggles, as in Blender.
    let at = screen(&doc, "Other", Vec3::ZERO);
    click(&mut doc, at, SHIFT);
    assert_eq!((selected(&doc), active(&doc).as_deref()), (vec!["Cube".to_owned(), "Other".to_owned()], Some("Other")));
    let at = screen(&doc, "Other", Vec3::ZERO);
    click(&mut doc, at, SHIFT);
    assert_eq!(selected(&doc), ["Cube"]);
}

#[test]
fn a_command_click_picks_by_the_origin() {
    // Blender's Ctrl+click (Cmd on a Mac) picks the object whose origin is nearest the pointer,
    // not the surface under it. Before, it was a plain click.
    let mut doc = starter();
    doc.select_tool("select", 1.0).unwrap();
    let (x, y) = screen(&doc, "Light", Vec3::ZERO);
    let near = (x + 30.0, y + 30.0);
    click(&mut doc, near, NONE);
    assert!(selected(&doc).is_empty(), "a plain click beside the light picks nothing");
    click(&mut doc, near, COMMAND);
    assert_eq!(selected(&doc), ["Light"], "Cmd/Ctrl picks the light by its origin");
    let at = screen(&doc, "Other", Vec3::new(0.5, 0.0, 0.5));
    click(&mut doc, at, BOTH);
    assert_eq!(selected(&doc), ["Light", "Other"], "with Shift it adds, by the origin too");
}

#[test]
fn lights_and_cameras_are_picked_on_their_drawn_lines() {
    // Before, only a click within 12 pixels of the origin picked a light, a camera or an empty.
    let mut doc = starter();
    run(&mut doc, "select.objects", json!({ "objects": "Light" }));
    run(&mut doc, "view.frameSelected", json!({}));
    run(&mut doc, "edit.deselectAll", json!({}));
    doc.select_tool("select", 1.0).unwrap();
    let origin = screen(&doc, "Light", Vec3::ZERO);
    let ring = screen(&doc, "Light", Vec3::new(0.2, 0.0, 0.0));
    assert!((ring.0 - origin.0).hypot(ring.1 - origin.1) > 20.0, "the ring is drawn away from the origin ({origin:?}, {ring:?})");
    click(&mut doc, ring, NONE);
    assert_eq!(selected(&doc), ["Light"], "a click on the light's ring picks it");
    click(&mut doc, EMPTY, NONE);
    assert!(selected(&doc).is_empty());
    // The camera, on its frame's corner.
    run(&mut doc, "select.objects", json!({ "objects": "Camera" }));
    run(&mut doc, "view.frameSelected", json!({}));
    run(&mut doc, "edit.deselectAll", json!({}));
    let camera = doc.scene().object_named("Camera").unwrap();
    let kition_scene_model::ObjectData::Camera(data) = &camera.data else { panic!("not a camera") };
    let aspect = doc.scene().render.width as f32 / doc.scene().render.height as f32;
    let (w, h) = data.frame_half_size(aspect);
    // Halfway along the pyramid's edge from the origin to the frame's corner.
    let at = screen(&doc, "Camera", Vec3::new(w, h, -1.0) * 0.5);
    let origin = screen(&doc, "Camera", Vec3::ZERO);
    assert!((at.0 - origin.0).hypot(at.1 - origin.1) > 20.0, "the edge is drawn away from the origin ({origin:?}, {at:?})");
    assert!(at.0 > 0.0 && at.1 > 0.0 && at.0 < 800.0 && at.1 < 600.0, "the edge is in the view ({at:?})");
    click(&mut doc, at, NONE);
    assert_eq!(selected(&doc), ["Camera"], "a click on the camera's pyramid picks it");
}

#[test]
fn the_context_menu_key_selects_an_unselected_object_under_the_pointer() {
    // Before, a right click opened the menu over whatever was selected, or nothing.
    let mut doc = starter();
    run(&mut doc, "select.objects", json!({ "objects": "Other" }));
    let cube = screen(&doc, "Cube", Vec3::ZERO);
    doc.pointer(&event(PointerAction::Move, cube, NONE)).unwrap();
    doc.key("ContextMenu", NONE, 1.0).unwrap();
    assert_eq!(selected(&doc), ["Cube"], "the object under the pointer becomes the selection");
    assert_eq!(active(&doc).as_deref(), Some("Cube"));
    run(&mut doc, "select.objects", json!({ "objects": ["Cube", "Other"] }));
    doc.pointer(&event(PointerAction::Move, cube, NONE)).unwrap();
    doc.key("ContextMenu", NONE, 1.0).unwrap();
    assert_eq!(selected(&doc), ["Cube", "Other"], "a selected object keeps the selection");
    doc.pointer(&event(PointerAction::Move, EMPTY, NONE)).unwrap();
    doc.key("ContextMenu", NONE, 1.0).unwrap();
    assert_eq!(selected(&doc), ["Cube", "Other"], "nothing under the pointer changes nothing");
    assert!(!doc.dirty());
}

#[test]
fn a_command_that_changes_nothing_leaves_the_file_saved() {
    // Before, Cmd+A (Apply Transform) on an object with nothing to apply made the file unsaved
    // while Undo stayed grey: the kit counted a change the replica never wrote.
    let mut doc = starter();
    let undo = doc.inspect().unwrap()["undo"].clone();
    run(&mut doc, "object.transform", json!({ "objects": "Cube", "location": [0, 0, 0] }));
    assert!(!doc.dirty(), "the same location again is no change");
    let _ = doc.execute("object.applyTransform", json!({ "objects": "Cube" }));
    assert!(!doc.dirty(), "nothing to apply is no change");
    assert_eq!(doc.inspect().unwrap()["undo"], undo);
    run(&mut doc, "object.transform", json!({ "objects": "Cube", "location": [1, 0, 0] }));
    assert!(doc.dirty(), "a real change is unsaved");
}

#[test]
fn a_number_field_drag_previews_without_an_undo_step() {
    // The properties' X, Y and Z labels drag the value: the viewport follows at once, and only
    // the value the drag ends on is a step. Before, the object stayed put and the drag committed
    // nothing.
    let mut doc = starter();
    let undo = doc.inspect().unwrap()["undo"].clone();
    let location = |doc: &SceneDocument| doc.scene().object_named("Cube").unwrap().location;
    let revision = doc.revision();
    run(&mut doc, "object.previewTransform", json!({ "objects": "Cube", "location": [1, 0, 0] }));
    assert_eq!(location(&doc), Vec3::new(1.0, 0.0, 0.0));
    assert!(doc.revision() > revision, "the picture is drawn again");
    run(&mut doc, "object.previewTransform", json!({ "objects": "Cube", "location": [2, 0, 0] }));
    assert_eq!(location(&doc), Vec3::new(2.0, 0.0, 0.0));
    assert!(!doc.dirty(), "a preview is no unsaved change");
    assert_eq!(doc.inspect().unwrap()["undo"], undo, "a preview is no undo step");
    run(&mut doc, "object.previewTransform", json!({ "end": true }));
    assert_eq!(location(&doc), Vec3::ZERO, "ending the preview puts the cube back");
    // The drag's last value commits as one step.
    run(&mut doc, "object.previewTransform", json!({ "objects": "Cube", "location": [3, 0, 0] }));
    run(&mut doc, "object.transform", json!({ "objects": "Cube", "location": [2.5, 0, 0] }));
    assert_eq!(location(&doc), Vec3::new(2.5, 0.0, 0.0));
    assert_eq!(doc.inspect().unwrap()["undo"], "Set Transform");
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(location(&doc), Vec3::ZERO, "one undo takes the whole drag back");
    assert_eq!(doc.inspect().unwrap()["undo"], undo);
    // Any other change ends a preview first: the scene it changes is the scene as it was.
    run(&mut doc, "object.previewTransform", json!({ "objects": "Cube", "scale": [3, 3, 3] }));
    run(&mut doc, "object.transform", json!({ "objects": "Other", "location": [5, 0, 0] }));
    assert_eq!(doc.scene().object_named("Cube").unwrap().scale, Vec3::ONE);
    let saved = doc.save_file(None).unwrap();
    let back = SceneDocument::open("Basics.kscene", &saved).unwrap();
    assert_eq!(back.scene().object_named("Cube").unwrap().scale, Vec3::ONE, "a preview is never saved");
    assert!(doc.execute("object.previewTransform", json!({ "objects": "Cube", "scale": [f64::NAN, 1, 1] })).is_err());
    assert_eq!(doc.scene().object_named("Cube").unwrap().scale, Vec3::ONE, "a refused preview changes nothing");
}
