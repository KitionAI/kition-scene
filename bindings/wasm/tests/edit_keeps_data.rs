//! Data an edit used to lose (C8-scene-engine group 3, found by the independent reading of
//! C8-scene-read): geometry appended to a mesh (edit-mode Add, Duplicate, Join) keeps every
//! per-vertex and per-face list; Fill gives its face texture coordinates; edits that change the
//! faces carry the sculpt face sets and hidden faces; Apply Transform moves shape keys and
//! multiresolution detail with the mesh; deleting an object lets go of every reference to it;
//! removing a particle system or an action keeps the indexes that pointed past it.
//!
//! Cases `regressions/issue-append-keeps-data`, `issue-fill-uvs`, `issue-edit-face-sets`,
//! `issue-apply-transform-keys`, `issue-delete-object-references`, `issue-removed-index`,
//! `issue-action-remove-tweak`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use kition_scene_model::glam::{Mat4, Quat, Vec3};
use kition_scene_model::{Mesh, ObjectData, multires};
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn new_doc() -> SceneDocument {
    SceneDocument::create("Keeps.kscene", 320, 180).unwrap()
}

fn mesh<'a>(doc: &'a SceneDocument, name: &str) -> &'a Mesh {
    doc.scene().object_named(name).unwrap_or_else(|| panic!("no {name}")).mesh().unwrap()
}

fn reopened(doc: &SceneDocument) -> SceneDocument {
    SceneDocument::open("Keeps.kscene", &doc.save(None).unwrap()).unwrap()
}

fn group<'a>(mesh: &'a Mesh, name: &str) -> &'a [f32] {
    &mesh.groups.iter().find(|group| group.name == name).unwrap_or_else(|| panic!("no group {name}")).weights
}

fn key<'a>(mesh: &'a Mesh, name: &str) -> &'a [Vec3] {
    &mesh.shape_keys.iter().find(|key| key.name == name).unwrap_or_else(|| panic!("no key {name}")).offsets
}

/// The cube's top vertices (z > 0).
fn top(mesh: &Mesh) -> Vec<u32> {
    (0..mesh.vertex_count()).filter(|vertex| mesh.positions[*vertex as usize].z > 0.0).collect()
}

/// Cube with a vertex group `Top` (its top vertices at 0.5) and a shape key `Tall` (top up 1).
fn cube_with_group_and_key() -> SceneDocument {
    let mut doc = new_doc();
    let tops = top(mesh(&doc, "Cube"));
    run(&mut doc, "vertexGroup.assign", json!({ "object": "Cube", "name": "Top", "weight": 0.5, "vertices": tops }));
    run(&mut doc, "shapeKey.add", json!({ "object": "Cube", "name": "Tall" }));
    run(&mut doc, "shapeKey.offset", json!({ "object": "Cube", "name": "Tall", "offset": [0, 0, 1], "vertices": tops }));
    doc
}

fn edit(doc: &mut SceneDocument, name: &str) {
    run(doc, "select.objects", json!({ "objects": [name] }));
    run(doc, "mode.set", json!({ "mode": "edit" }));
}

fn object_mode(doc: &mut SceneDocument) {
    run(doc, "mode.set", json!({ "mode": "object" }));
}

// --- issue-append-keeps-data ------------------------------------------------------------

#[test]
fn append_add_in_edit_mode_keeps_groups_and_keys() {
    let mut doc = cube_with_group_and_key();
    edit(&mut doc, "Cube");
    run(&mut doc, "add.cube", json!({ "location": [4, 0, 0] }));
    object_mode(&mut doc);
    let mesh = mesh(&doc, "Cube");
    assert_eq!(mesh.positions.len(), 16);
    let weights = group(mesh, "Top");
    let offsets = key(mesh, "Tall");
    assert_eq!((weights.len(), offsets.len()), (16, 16));
    for vertex in 0..16 {
        let old = vertex < 8 && mesh.positions[vertex].z > 0.0;
        assert_eq!(weights[vertex], if old { 0.5 } else { 0.0 }, "weight of {vertex}");
        assert_eq!(offsets[vertex], if old { Vec3::Z } else { Vec3::ZERO }, "offset of {vertex}");
    }
}

#[test]
fn append_duplicate_in_edit_mode_copies_weights_keys_and_uvs() {
    let mut doc = cube_with_group_and_key();
    let before = mesh(&doc, "Cube").clone();
    let face = before.faces.iter().position(|face| before.face_normal(face).z > 0.9).unwrap();
    edit(&mut doc, "Cube");
    run(&mut doc, "mesh.select", json!({ "faces": [face] }));
    run(&mut doc, "edit.duplicate", json!({ "offset": [0, 0, 2] }));
    object_mode(&mut doc);
    let mesh = mesh(&doc, "Cube");
    assert_eq!(mesh.positions.len(), 12);
    assert_eq!(group(mesh, "Top")[8..], [0.5; 4], "the copies keep their weights");
    assert_eq!(key(mesh, "Tall")[8..], [Vec3::Z; 4], "the copies keep their shape offsets");
    let copy = mesh.uvs.last().unwrap();
    assert_eq!(copy, &before.uvs[face], "the copied face keeps its texture coordinates");
}

#[test]
fn append_join_carries_groups_and_keys_by_name() {
    let mut doc = cube_with_group_and_key();
    run(&mut doc, "add.cube", json!({ "name": "Other", "location": [4, 0, 0] }));
    let other_top = top(mesh(&doc, "Other"));
    run(&mut doc, "vertexGroup.assign", json!({ "object": "Other", "name": "Top", "weight": 1.0, "vertices": other_top }));
    run(&mut doc, "vertexGroup.assign", json!({ "object": "Other", "name": "Side", "weight": 0.25, "vertices": [0] }));
    run(&mut doc, "shapeKey.add", json!({ "object": "Other", "name": "Tall" }));
    run(&mut doc, "shapeKey.offset", json!({ "object": "Other", "name": "Tall", "offset": [0, 0, 2], "vertices": other_top }));
    run(&mut doc, "select.objects", json!({ "objects": ["Other", "Cube"] }));
    let joined = run(&mut doc, "object.join", json!({}));
    assert_eq!(joined["object"], json!(doc.scene().object_named("Cube").unwrap().id));
    let mesh = mesh(&doc, "Cube");
    assert_eq!(mesh.positions.len(), 16);
    let (top_weights, side, tall) = (group(mesh, "Top"), group(mesh, "Side"), key(mesh, "Tall"));
    for vertex in 0..16 {
        let up = mesh.positions[vertex].z > 0.0;
        let (weight, offset) = match (vertex < 8, up) {
            (true, true) => (0.5, Vec3::Z),
            (false, true) => (1.0, Vec3::Z * 2.0),
            _ => (0.0, Vec3::ZERO),
        };
        assert_eq!(top_weights[vertex], weight, "Top of {vertex}");
        assert_eq!(tall[vertex], offset, "Tall of {vertex}");
        assert_eq!(side[vertex], if vertex == 8 { 0.25 } else { 0.0 }, "Side of {vertex}");
    }
    // It is stored that way.
    let again = reopened(&doc);
    assert_eq!(mesh.groups, again.scene().object_named("Cube").unwrap().mesh().unwrap().groups);
}

/// Cube made of two loose parts (the second added in edit mode at x = 4), with a face set per
/// part (1 and 2) and the second part's faces hidden.
fn two_part_cube_with_face_sets() -> SceneDocument {
    let mut doc = new_doc();
    edit(&mut doc, "Cube");
    run(&mut doc, "add.cube", json!({ "location": [4, 0, 0] }));
    object_mode(&mut doc);
    run(&mut doc, "mode.sculpt", json!({}));
    run(&mut doc, "sculpt.faceSetsFromLooseParts", json!({}));
    run(&mut doc, "sculpt.faceSetVisibility", json!({ "action": "hide", "faceSet": 2 }));
    run(&mut doc, "mode.sculpt", json!({ "on": false }));
    let mesh = mesh(&doc, "Cube");
    assert_eq!(mesh.face_sets, [1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2]);
    assert_eq!(mesh.hidden, [false, false, false, false, false, false, true, true, true, true, true, true]);
    doc
}

#[test]
fn append_add_in_edit_mode_keeps_face_sets() {
    let mut doc = two_part_cube_with_face_sets();
    edit(&mut doc, "Cube");
    run(&mut doc, "add.cube", json!({ "location": [0, 4, 0] }));
    object_mode(&mut doc);
    let mesh = mesh(&doc, "Cube");
    assert_eq!(mesh.face_sets, [1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 0, 0, 0, 0, 0, 0]);
    assert_eq!(mesh.hidden[12..], [false; 6]);
    assert_eq!(mesh.hidden[6..12], [true; 6]);
}

// --- issue-fill-uvs ---------------------------------------------------------------------

#[test]
fn fill_uvs_a_hole_on_a_mesh_with_texture_coordinates() {
    let mut doc = new_doc();
    let before = mesh(&doc, "Cube").clone();
    assert!(before.has_uvs());
    let corners = before.faces[0].clone();
    run(&mut doc, "mesh.deleteFaces", json!({ "elements": { "object": "Cube", "faces": [0] } }));
    let filled = run(&mut doc, "mesh.fill", json!({ "elements": { "object": "Cube", "vertices": corners.clone() }, "vertices": corners }));
    let mesh = mesh(&doc, "Cube");
    assert_eq!(mesh.faces.len(), 6, "{filled}");
    assert!(mesh.uvs_aligned());
}

// --- issue-edit-face-sets ---------------------------------------------------------------

#[test]
fn edit_face_sets_follow_a_deleted_face() {
    let mut doc = two_part_cube_with_face_sets();
    run(&mut doc, "mesh.deleteFaces", json!({ "elements": { "object": "Cube", "faces": [0] } }));
    let mesh = mesh(&doc, "Cube");
    assert_eq!(mesh.face_sets, [1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2]);
    assert_eq!(mesh.hidden, [false, false, false, false, false, true, true, true, true, true, true]);
    let again = reopened(&doc);
    assert_eq!(again.scene().object_named("Cube").unwrap().mesh().unwrap().face_sets, mesh.face_sets, "stored");
}

#[test]
fn edit_face_sets_follow_subdivide_and_extrude() {
    let mut doc = two_part_cube_with_face_sets();
    let second = mesh(&doc, "Cube").faces[6..].iter().position(|_| true).unwrap() + 6;
    run(&mut doc, "mesh.subdivide", json!({ "elements": { "object": "Cube", "faces": [second] } }));
    let after = mesh(&doc, "Cube").clone();
    assert_eq!(after.face_sets.len(), after.faces.len());
    for (index, face) in after.faces.iter().enumerate() {
        let x = after.face_center(face).x;
        assert_eq!(after.face_sets[index], if x > 2.0 { 2 } else { 1 }, "face {index} at x {x}");
        assert_eq!(after.hidden[index], x > 2.0, "face {index} at x {x}");
    }
    let first_top = after.faces.iter().position(|face| after.face_normal(face).z > 0.9 && after.face_center(face).x < 2.0).unwrap();
    run(&mut doc, "mesh.extrude", json!({ "elements": { "object": "Cube", "faces": [first_top] }, "distance": 1 }));
    let after = mesh(&doc, "Cube");
    assert_eq!(after.face_sets.len(), after.faces.len());
    for (index, face) in after.faces.iter().enumerate() {
        let x = after.face_center(face).x;
        assert_eq!(after.face_sets[index], if x > 2.0 { 2 } else { 1 }, "face {index} at x {x}");
    }
}

// --- issue-apply-transform-keys ---------------------------------------------------------

#[test]
fn apply_transform_scales_shape_keys() {
    let mut doc = new_doc();
    let offsets = vec![[0.0, 0.0, 1.0]; 8];
    run(&mut doc, "shapeKey.add", json!({ "object": "Cube", "name": "Up", "offsets": offsets }));
    run(&mut doc, "object.transform", json!({ "objects": ["Cube"], "scale": [2, 2, 2], "rotation": [90, 0, 0] }));
    run(&mut doc, "object.applyTransform", json!({ "objects": ["Cube"] }));
    let mesh = mesh(&doc, "Cube");
    for offset in key(mesh, "Up") {
        // Rx(90) turns +Z to -Y; the scale doubles it.
        assert!((*offset - Vec3::new(0.0, -2.0, 0.0)).length() < 1e-5, "{offset}");
    }
}

fn detailed_cube() -> Mesh {
    let mut mesh = kition_scene_model::primitives::Primitive::Cube { size: 2.0 }.build().unwrap();
    multires::subdivide(&mut mesh).unwrap();
    multires::subdivide(&mut mesh).unwrap();
    for (level, offsets) in mesh.multires.iter_mut().enumerate() {
        for (index, offset) in offsets.iter_mut().enumerate() {
            let t = (index * 7 + level * 3) as f32;
            *offset = Vec3::new((t * 0.37).sin(), (t * 0.11).cos(), (t * 0.23).sin()) * 0.05;
        }
    }
    mesh
}

#[track_caller]
fn same_points(actual: &[Vec3], expected: &[Vec3], what: &str) {
    assert_eq!(actual.len(), expected.len(), "{what}");
    for (index, point) in expected.iter().enumerate() {
        let nearest = actual.iter().map(|other| other.distance(*point)).fold(f32::MAX, f32::min);
        assert!(nearest < 1e-4, "{what}: point {index} {point} is {nearest} from the nearest");
    }
}

#[test]
fn apply_transform_keeps_multires_detail_where_it_was() {
    for (what, transform) in [
        ("scale and turn", Mat4::from_scale_rotation_translation(Vec3::new(2.0, 1.0, 0.5), Quat::from_rotation_z(0.7), Vec3::new(1.0, 2.0, 3.0))),
        ("mirror", Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0))),
    ] {
        let mut mesh = detailed_cube();
        let shown: Vec<Vec<Vec3>> = (1..=2).map(|level| multires::evaluate(&mesh, level).positions).collect();
        mesh.transform(transform);
        assert_eq!(multires::levels(&mesh), 2, "{what}");
        for (level, before) in shown.iter().enumerate() {
            let expected: Vec<Vec3> = before.iter().map(|point| transform.transform_point3(*point)).collect();
            let actual = multires::evaluate(&mesh, level as u32 + 1).positions;
            same_points(&actual, &expected, &format!("{what}, level {}", level + 1));
        }
    }
}

// --- issue-delete-object-references ------------------------------------------------------

fn modifier_objects(doc: &SceneDocument, name: &str) -> Vec<Option<u32>> {
    use kition_scene_model::modifiers::ModifierKind;
    doc.scene()
        .object_named(name)
        .unwrap()
        .modifiers
        .iter()
        .map(|modifier| match &modifier.kind {
            ModifierKind::Boolean { object, .. }
            | ModifierKind::Armature { object, .. }
            | ModifierKind::Shrinkwrap { object, .. }
            | ModifierKind::Lattice { object, .. } => *object,
            other => panic!("unexpected modifier {other:?}"),
        })
        .collect()
}

#[test]
fn delete_object_references_in_modifiers_let_go() {
    let mut doc = new_doc();
    run(&mut doc, "add.cube", json!({ "name": "Tool", "location": [0.5, 0, 0] }));
    run(&mut doc, "add.lattice", json!({ "resolution": 2, "scale": [3, 3, 3] }));
    run(&mut doc, "add.armature", json!({ "name": "Rig" }));
    run(&mut doc, "modifier.add", json!({ "object": "Cube", "type": "boolean", "operand": "Tool" }));
    run(&mut doc, "modifier.add", json!({ "object": "Cube", "type": "shrinkwrap", "operand": "Tool" }));
    run(&mut doc, "modifier.add", json!({ "object": "Cube", "type": "lattice", "operand": "Lattice" }));
    run(&mut doc, "modifier.add", json!({ "object": "Cube", "type": "armature", "operand": "Rig" }));
    assert!(modifier_objects(&doc, "Cube").iter().all(Option::is_some));
    run(&mut doc, "edit.delete", json!({ "objects": ["Tool", "Lattice", "Rig"] }));
    assert_eq!(modifier_objects(&doc, "Cube"), [None, None, None, None], "the modifiers name no object, as after clearing their field");
    let again = reopened(&doc);
    assert_eq!(modifier_objects(&again, "Cube"), [None, None, None, None]);
}

#[test]
fn delete_object_references_in_curves_and_texts_let_go() {
    use kition_scene_model::{Curve, Scene, Text};
    let mut scene = Scene::empty();
    let profile = scene.add_object("Profile", ObjectData::Curve(Curve::default()));
    let path = scene.add_object("Path", ObjectData::Curve(Curve { bevel_object: Some(profile), taper_object: Some(profile), ..Curve::default() }));
    let text = scene.add_object("Text", ObjectData::Text(Text { follow_curve: Some(profile), ..Text::default() }));
    scene.validate().unwrap();
    scene.remove_objects(&[profile]).unwrap();
    let ObjectData::Curve(curve) = &scene.object(path).unwrap().data else { panic!() };
    assert_eq!((curve.bevel_object, curve.taper_object), (None, None));
    let ObjectData::Text(text) = &scene.object(text).unwrap().data else { panic!() };
    assert_eq!(text.follow_curve, None);
}

#[test]
fn delete_object_references_dangling_ones_are_refused() {
    use kition_scene_model::{Curve, Modifier, ModifierKind, Scene};
    let mut scene = Scene::empty();
    let cube = scene.add_object("Cube", ObjectData::Mesh(Mesh::default()));
    let curve = scene.add_object("Curve", ObjectData::Curve(Curve::default()));
    let missing = scene.next_id + 5;
    scene.validate().unwrap();
    let mut broken = scene.clone();
    broken.object_mut(cube).unwrap().modifiers.push(Modifier::new(ModifierKind::Lattice { object: Some(missing), strength: 1.0 }));
    assert!(broken.validate().is_err(), "a modifier naming a missing object");
    let mut broken = scene.clone();
    if let ObjectData::Curve(data) = &mut broken.object_mut(curve).unwrap().data {
        data.bevel_object = Some(missing);
    }
    assert!(broken.validate().is_err(), "a curve beveled by a missing object");
}

// --- issue-removed-index ----------------------------------------------------------------

#[test]
fn removed_index_particle_edit_keeps_its_system() {
    let mut doc = new_doc();
    for name in ["A", "B", "C"] {
        run(&mut doc, "particles.add", json!({ "object": "Cube", "kind": "hair", "name": name, "count": 20 }));
    }
    run(&mut doc, "mode.particleEdit", json!({ "object": "Cube", "system": "B" }));
    let edited = |doc: &SceneDocument| doc.session().particle_edit.as_ref().map(|session| session.system);
    assert_eq!(edited(&doc), Some(1));
    run(&mut doc, "particles.remove", json!({ "object": "Cube", "system": "A" }));
    let systems: Vec<String> = doc.scene().object_named("Cube").unwrap().particles.iter().map(|system| system.name.clone()).collect();
    assert_eq!(systems, ["B", "C"]);
    assert_eq!(edited(&doc), Some(0), "Particle Edit still grooms B");
    // Removing the groomed system ends Particle Edit.
    run(&mut doc, "particles.remove", json!({ "object": "Cube", "system": "B" }));
    assert_eq!(edited(&doc), None, "B is gone: Particle Edit ends rather than grooming C");
}

fn keyed(doc: &mut SceneDocument, name: &str, top: f32) {
    run(doc, "frame.set", json!({ "frame": 1 }));
    run(doc, "object.transform", json!({ "objects": "Cube", "location": [0, 0, 0] }));
    run(doc, "anim.insertKey", json!({ "objects": "Cube", "properties": ["location"] }));
    run(doc, "frame.set", json!({ "frame": 11 }));
    run(doc, "object.transform", json!({ "objects": "Cube", "location": [0, 0, top] }));
    run(doc, "anim.insertKey", json!({ "objects": "Cube", "properties": ["location"] }));
    run(doc, "nla.pushDown", json!({ "object": "Cube", "name": name }));
}

#[test]
fn removed_index_action_before_the_tweaked_strip() {
    let mut doc = new_doc();
    keyed(&mut doc, "Rise", 2.0);
    keyed(&mut doc, "Fall", -2.0);
    run(&mut doc, "nla.addStrip", json!({ "object": "Cube", "action": "Fall", "track": 0, "start": 30 }));
    let strips = |doc: &SceneDocument| -> Vec<String> {
        doc.scene().object_named("Cube").unwrap().nla.tracks[0].strips.iter().map(|strip| strip.action.clone()).collect()
    };
    assert_eq!(strips(&doc), ["Rise", "Fall"]);
    run(&mut doc, "nla.tweak", json!({ "object": "Cube", "track": 0, "strip": 1 }));
    run(&mut doc, "action.remove", json!({ "name": "Rise" }));
    assert_eq!(strips(&doc), ["Fall"]);
    let tweak = doc.scene().object_named("Cube").unwrap().nla.tweak.clone().unwrap();
    assert_eq!((tweak.track, tweak.strip, tweak.action.as_str()), (0, 0, "Fall"), "still tweaking the Fall strip");
}
