//! Results computed in the right space, once (C8-scene-engine group 2, found by the independent
//! reading of C8-scene-read): a world offset or turn goes through the parent's frame (the parent's
//! world times the parent inverse), never through the object's own constraints; parenting and
//! clearing a parent keep the object where its own fields and its parents put it; Apply
//! Constraint bakes one constraint, once; a child of a scaled parent is scaled once; the promo
//! moves (hop, gather) plan in the world and key the parent's space; extrude adds `offset` to
//! the move along the normal on every path; a key's frame in tweak mode is the strip's action
//! frame for every key command; dynamic topology never runs on a mesh with multiresolution
//! levels.
//!
//! Cases `regressions/issue-parent-frame`, `issue-reparent-keeps-world`,
//! `issue-clear-parent-own-constraints`, `issue-apply-constraint-once`,
//! `issue-scale-selected-children`, `issue-promo-parent-space`, `issue-extrude-offset`,
//! `issue-interpolation-tweak-frame`, `issue-dyntopo-multires-levels`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use kition_scene_model::glam::{Mat4, Vec3};
use kition_scene_model::{Interpolation, Property, multires};
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn new_doc() -> SceneDocument {
    SceneDocument::create("World.kscene", 320, 180).unwrap()
}

fn id_of(doc: &SceneDocument, name: &str) -> u32 {
    doc.scene().object_named(name).unwrap_or_else(|| panic!("no {name}")).id
}

fn world(doc: &SceneDocument, name: &str) -> Mat4 {
    doc.scene().world_matrix(id_of(doc, name)).unwrap()
}

fn world_at(doc: &SceneDocument, name: &str) -> Vec3 {
    world(doc, name).transform_point3(Vec3::ZERO)
}

fn location(doc: &SceneDocument, name: &str) -> Vec3 {
    doc.scene().object_named(name).unwrap().location
}

fn rotation_degrees(doc: &SceneDocument, name: &str) -> Vec3 {
    let rotation = doc.scene().object_named(name).unwrap().rotation;
    Vec3::new(rotation.x.to_degrees(), rotation.y.to_degrees(), rotation.z.to_degrees())
}

#[track_caller]
fn near(actual: Vec3, expected: [f32; 3], what: &str) {
    let expected = Vec3::from_array(expected);
    assert!((actual - expected).length() < 1e-4, "{what}: {actual} != {expected}");
}

// --- issue-parent-frame -----------------------------------------------------------------

/// Cube (no parent, at the origin) turned 90 degrees about Z by a Copy Rotation of `Turned`.
fn turned_by_a_constraint() -> SceneDocument {
    let mut doc = new_doc();
    run(&mut doc, "object.addEmpty", json!({ "name": "Turned", "location": [0, 5, 0] }));
    run(&mut doc, "object.transform", json!({ "objects": ["Turned"], "rotation": [0, 0, 90] }));
    run(&mut doc, "constraint.add", json!({ "object": "Cube", "type": "copyRotation", "target": "Turned" }));
    // The constraint turns it: its X axis points along world Y.
    near(world(&doc, "Cube").transform_vector3(Vec3::X), [0.0, 1.0, 0.0], "constrained X axis");
    doc
}

#[test]
fn parent_frame_translate_ignores_the_own_constraints() {
    let mut doc = turned_by_a_constraint();
    run(&mut doc, "object.translate", json!({ "objects": ["Cube"], "offset": [1, 0, 0] }));
    // No parent: the parent frame is the identity, so location += [1, 0, 0]. (Through
    // world * local^-1 = Rz(90) the offset became Rz(-90) [1, 0, 0] = [0, -1, 0].)
    near(location(&doc, "Cube"), [1.0, 0.0, 0.0], "location");
}

#[test]
fn parent_frame_translate_turns_through_a_rotated_parent() {
    let mut doc = new_doc();
    run(&mut doc, "object.addEmpty", json!({ "name": "Pivot" }));
    run(&mut doc, "object.parent", json!({ "objects": ["Cube"], "parent": "Pivot" }));
    run(&mut doc, "object.transform", json!({ "objects": ["Pivot"], "rotation": [0, 0, 90] }));
    run(&mut doc, "object.translate", json!({ "objects": ["Cube"], "offset": [1, 0, 0] }));
    // Parent frame Rz(90) (parent inverse identity): the local offset is Rz(-90) [1, 0, 0] =
    // [0, -1, 0], which the parent turns back to world [1, 0, 0].
    near(location(&doc, "Cube"), [0.0, -1.0, 0.0], "location");
    near(world_at(&doc, "Cube"), [1.0, 0.0, 0.0], "world");
}

#[test]
fn parent_frame_rotate_ignores_the_own_constraints() {
    let mut doc = turned_by_a_constraint();
    run(&mut doc, "object.rotate", json!({ "objects": ["Cube"], "axis": "x", "angle": 90 }));
    // No parent: the rotation fields become Rx(90) = [90, 0, 0] degrees. (Through Rz(90) the
    // axis became Rz(-90) X = -Y: [0, -90, 0].)
    near(rotation_degrees(&doc, "Cube"), [90.0, 0.0, 0.0], "rotation");
}

#[test]
fn parent_frame_look_at_ignores_the_own_constraints() {
    let mut doc = turned_by_a_constraint();
    run(&mut doc, "object.lookAt", json!({ "object": "Cube", "target": [0, -5, 0] }));
    // From the origin to [0, -5, 0]: the object's -Z axis (through its own fields, no parent)
    // points along world -Y. (Through Rz(90) it pointed along Rz(-90) (-Y) = -X.)
    let local = doc.scene().object_named("Cube").unwrap().local_matrix();
    near(local.transform_vector3(Vec3::NEG_Z), [0.0, -1.0, 0.0], "-Z axis");
    near(location(&doc, "Cube"), [0.0, 0.0, 0.0], "location");
}

#[test]
fn parent_frame_look_at_keeps_the_location_a_constraint_moves() {
    let mut doc = new_doc();
    run(&mut doc, "object.addEmpty", json!({ "name": "Spot", "location": [3, 0, 0] }));
    run(&mut doc, "constraint.add", json!({ "object": "Cube", "type": "copyLocation", "target": "Spot", "offset": true }));
    run(&mut doc, "object.lookAt", json!({ "object": "Cube", "target": [3, -5, 0] }));
    // Aimed from where it is seen ([3, 0, 0]) along -Y; its location field stays [0, 0, 0] (the
    // constraint adds the 3 m), so it is still seen at [3, 0, 0].
    near(location(&doc, "Cube"), [0.0, 0.0, 0.0], "location");
    near(world_at(&doc, "Cube"), [3.0, 0.0, 0.0], "world");
    near(world(&doc, "Cube").transform_vector3(Vec3::NEG_Z), [0.0, -1.0, 0.0], "-Z axis");
}

#[test]
fn parent_frame_camera_move_ignores_the_track_constraint() {
    let mut doc = new_doc();
    run(&mut doc, "object.transform", json!({ "objects": ["Camera"], "location": [0, -10, 0], "rotation": [0, 0, 0] }));
    run(&mut doc, "constraint.add", json!({ "object": "Camera", "type": "trackTo", "target": "Cube" }));
    run(&mut doc, "camera.move", json!({ "preset": "pushIn", "target": "Cube", "start": 1, "duration": 10, "amount": 0.5 }));
    // A push in from [0, -10, 0] towards the cube at the origin stays on the Y axis: every
    // location key has x = 0 and z = 0 (no parent, so the keys are world places). Through
    // world * local^-1 (the Track To turn about the camera) they swung off the axis.
    let camera = id_of(&doc, "Camera");
    let channels: Vec<_> = doc
        .scene()
        .animation
        .channels
        .iter()
        .filter(|channel| channel.target == camera && channel.property == Property::Location && channel.index != 1)
        .collect();
    assert_eq!(channels.len(), 2, "x and z location channels");
    for channel in channels {
        for key in &channel.keys {
            assert!(key.value.abs() < 1e-3, "location[{}] at frame {} is {}", channel.index, key.frame, key.value);
        }
    }
}

// --- issue-reparent-keeps-world -----------------------------------------------------------

/// Cube parented to P1 at the origin, then P1 moved to [5, 0, 0]; Cube's location [1, 0, 0]:
/// Cube is seen at [6, 0, 0].
fn under_a_moved_parent() -> SceneDocument {
    let mut doc = new_doc();
    run(&mut doc, "object.addEmpty", json!({ "name": "P1" }));
    run(&mut doc, "object.addEmpty", json!({ "name": "P2", "location": [0, 0, 2] }));
    run(&mut doc, "object.transform", json!({ "objects": ["Cube"], "location": [1, 0, 0] }));
    run(&mut doc, "object.parent", json!({ "objects": ["Cube"], "parent": "P1" }));
    run(&mut doc, "object.translate", json!({ "objects": ["P1"], "offset": [5, 0, 0] }));
    near(world_at(&doc, "Cube"), [6.0, 0.0, 0.0], "before");
    doc
}

#[test]
fn reparent_keeps_the_world_place() {
    let mut doc = under_a_moved_parent();
    run(&mut doc, "object.parent", json!({ "objects": ["Cube"], "parent": "P2" }));
    // Keep Transform: the fields take the world transform [6, 0, 0], the parent inverse is
    // inverse(P2 world) = T(0, 0, -2), so the world stays [6, 0, 0]. (It became [1, 0, 0].)
    near(world_at(&doc, "Cube"), [6.0, 0.0, 0.0], "world");
    near(location(&doc, "Cube"), [6.0, 0.0, 0.0], "location");
    assert_eq!(doc.scene().object_named("Cube").unwrap().parent, Some(id_of(&doc, "P2")));
}

#[test]
fn reparent_an_unparented_object_keeps_its_fields() {
    let mut doc = new_doc();
    run(&mut doc, "object.addEmpty", json!({ "name": "P2", "location": [0, 0, 2] }));
    run(&mut doc, "object.transform", json!({ "objects": ["Cube"], "location": [1, 0, 0], "rotation": [0, 0, 200] }));
    run(&mut doc, "object.parent", json!({ "objects": ["Cube"], "parent": "P2" }));
    // No old parent: the fields are untouched (200 degrees stays 200) and the world stays.
    near(location(&doc, "Cube"), [1.0, 0.0, 0.0], "location");
    near(rotation_degrees(&doc, "Cube"), [0.0, 0.0, 200.0], "rotation");
    near(world_at(&doc, "Cube"), [1.0, 0.0, 0.0], "world");
}

#[test]
fn reparent_on_delete_keeps_the_child_in_place() {
    let mut doc = new_doc();
    run(&mut doc, "object.addEmpty", json!({ "name": "GP" }));
    run(&mut doc, "object.addEmpty", json!({ "name": "P" }));
    run(&mut doc, "object.parent", json!({ "objects": ["P"], "parent": "GP" }));
    run(&mut doc, "object.transform", json!({ "objects": ["Cube"], "location": [1, 0, 0] }));
    run(&mut doc, "object.parent", json!({ "objects": ["Cube"], "parent": "P" }));
    run(&mut doc, "object.translate", json!({ "objects": ["P"], "offset": [5, 0, 0] }));
    near(world_at(&doc, "Cube"), [6.0, 0.0, 0.0], "before");
    run(&mut doc, "edit.delete", json!({ "objects": ["P"] }));
    // The child moves to GP keeping [6, 0, 0]. (It jumped back to [1, 0, 0].)
    assert_eq!(doc.scene().object_named("Cube").unwrap().parent, Some(id_of(&doc, "GP")));
    near(world_at(&doc, "Cube"), [6.0, 0.0, 0.0], "world");
}

#[test]
fn reparent_to_armature_keeps_a_parented_mesh_in_place() {
    let mut doc = under_a_moved_parent();
    run(&mut doc, "object.addArmature", json!({ "name": "Armature", "location": [0, 3, 0] }));
    run(&mut doc, "object.parentArmature", json!({ "armature": "Armature", "objects": ["Cube"], "weights": "none" }));
    // Cube was seen at [6, 0, 0]; it stays there under the armature. (It went to [1, 0, 0].)
    near(world_at(&doc, "Cube"), [6.0, 0.0, 0.0], "world");
}

// --- issue-clear-parent-own-constraints ----------------------------------------------------

#[test]
fn clear_parent_without_a_parent_changes_nothing() {
    let mut doc = new_doc();
    run(&mut doc, "object.addEmpty", json!({ "name": "Spot", "location": [3, 0, 0] }));
    run(&mut doc, "constraint.add", json!({ "object": "Cube", "type": "copyLocation", "target": "Spot", "offset": true }));
    near(world_at(&doc, "Cube"), [3.0, 0.0, 0.0], "before");
    run(&mut doc, "object.clearParent", json!({ "objects": ["Cube"] }));
    // No parent: location stays [0, 0, 0] and the constraint keeps it at [3, 0, 0]. (The
    // constraint's [3, 0, 0] went into the location and the constraint added 3 more: [6, 0, 0].)
    near(location(&doc, "Cube"), [0.0, 0.0, 0.0], "location");
    near(world_at(&doc, "Cube"), [3.0, 0.0, 0.0], "world");
}

#[test]
fn clear_parent_bakes_the_parent_not_the_own_constraints() {
    let mut doc = under_a_moved_parent();
    run(&mut doc, "object.addEmpty", json!({ "name": "Spot", "location": [3, 0, 0] }));
    run(&mut doc, "constraint.add", json!({ "object": "Cube", "type": "copyLocation", "target": "Spot", "offset": true }));
    // Before its constraint Cube is at [6, 0, 0]; Copy Location with offset adds [3, 0, 0].
    near(world_at(&doc, "Cube"), [9.0, 0.0, 0.0], "before");
    run(&mut doc, "object.clearParent", json!({ "objects": ["Cube"] }));
    // Keep Transform: location = parent frame * local = [6, 0, 0]; the world stays [9, 0, 0].
    // (Location became [9, 0, 0] and the world [12, 0, 0].)
    near(location(&doc, "Cube"), [6.0, 0.0, 0.0], "location");
    near(world_at(&doc, "Cube"), [9.0, 0.0, 0.0], "world");
}

// --- issue-apply-constraint-once ----------------------------------------------------------

#[test]
fn apply_constraint_later_one_does_not_apply_the_earlier_again() {
    let mut doc = new_doc();
    run(&mut doc, "object.addEmpty", json!({ "name": "Spot", "location": [5, 0, 0] }));
    run(&mut doc, "object.transform", json!({ "objects": ["Cube"], "location": [1, 0, 0], "rotation": [0, 0, 30] }));
    run(&mut doc, "constraint.add", json!({ "object": "Cube", "type": "copyLocation", "target": "Spot", "offset": true }));
    run(
        &mut doc,
        "constraint.add",
        json!({ "object": "Cube", "type": "limitRotation", "limit": [false, false, true], "min": [0, 0, 0], "max": [0, 0, 10] }),
    );
    // [1, 0, 0] + Spot's [5, 0, 0] = [6, 0, 0]; the turn limited from 30 to 10 degrees.
    near(world_at(&doc, "Cube"), [6.0, 0.0, 0.0], "before");
    run(&mut doc, "constraint.apply", json!({ "object": "Cube", "index": 1 }));
    // Apply bakes the Limit Rotation's effect into the fields (rotation 30 -> 10); the Copy
    // Location stays on the stack and adds Spot's 5 m once: location [1, 0, 0], world [6, 0, 0].
    // (The location took the stack's [6, 0, 0] and the Copy Location added 5 again: [11, 0, 0].)
    let cube = doc.scene().object_named("Cube").unwrap();
    assert_eq!(cube.constraints.len(), 1, "only the applied constraint goes");
    near(location(&doc, "Cube"), [1.0, 0.0, 0.0], "location");
    near(rotation_degrees(&doc, "Cube"), [0.0, 0.0, 10.0], "rotation");
    near(world_at(&doc, "Cube"), [6.0, 0.0, 0.0], "world");
}

#[test]
fn apply_constraint_first_one_still_bakes() {
    let mut doc = new_doc();
    run(&mut doc, "object.addEmpty", json!({ "name": "Spot", "location": [5, 0, 0] }));
    run(&mut doc, "object.transform", json!({ "objects": ["Cube"], "location": [1, 0, 0] }));
    run(&mut doc, "constraint.add", json!({ "object": "Cube", "type": "copyLocation", "target": "Spot", "offset": true }));
    run(&mut doc, "constraint.apply", json!({ "object": "Cube", "index": 0 }));
    // [1, 0, 0] + [5, 0, 0] baked into the location; the stack is empty.
    near(location(&doc, "Cube"), [6.0, 0.0, 0.0], "location");
    assert!(doc.scene().object_named("Cube").unwrap().constraints.is_empty());
}

// --- issue-scale-selected-children --------------------------------------------------------

#[test]
fn scale_children_of_a_selected_parent_scaled_once() {
    let mut doc = new_doc();
    run(&mut doc, "object.addEmpty", json!({ "name": "P" }));
    run(&mut doc, "object.transform", json!({ "objects": ["Cube"], "location": [1, 0, 0] }));
    run(&mut doc, "object.parent", json!({ "objects": ["Cube"], "parent": "P" }));
    run(&mut doc, "object.scale", json!({ "objects": ["P", "Cube"], "factor": 2 }));
    // P's scale 2 carries the child once: Cube's own scale stays 1, its world scale is 2 and
    // it is seen at [2, 0, 0]. (Its own scale doubled too: world scale 4.)
    near(doc.scene().object_named("P").unwrap().scale, [2.0, 2.0, 2.0], "P scale");
    near(doc.scene().object_named("Cube").unwrap().scale, [1.0, 1.0, 1.0], "Cube scale");
    near(world(&doc, "Cube").transform_vector3(Vec3::X), [2.0, 0.0, 0.0], "Cube world X axis");
    near(world_at(&doc, "Cube"), [2.0, 0.0, 0.0], "Cube world");
}

// --- issue-promo-parent-space --------------------------------------------------------------

/// Cube (2 m, at the origin) parented to Big, which is then scaled by 2: the cube is seen 4 m
/// wide at the origin, its location field [0, 0, 0].
fn under_a_scaled_parent() -> SceneDocument {
    let mut doc = new_doc();
    run(&mut doc, "object.addEmpty", json!({ "name": "Big" }));
    run(&mut doc, "object.parent", json!({ "objects": ["Cube"], "parent": "Big" }));
    run(&mut doc, "object.scale", json!({ "objects": ["Big"], "factor": 2 }));
    doc
}

#[test]
fn promo_hop_lands_on_the_world_point_under_a_scaled_parent() {
    let mut doc = under_a_scaled_parent();
    let answer = run(&mut doc, "object.hop", json!({ "objects": ["Cube"], "to": [4, 0, 0], "start": 1, "duration": 10, "impact": false }));
    assert_eq!(answer["landed"], 11.0);
    // The centre goes from [0, 0, 0] to [4, 0, 0] in the world: through the parent's scale 2 the
    // location key at the landing is [2, 0, 0]. (The world shift was keyed as the location: the
    // cube landed at [8, 0, 0].)
    let landed = doc.scene().at_frame(11.0);
    let cube = id_of(&doc, "Cube");
    near(landed.object(cube).unwrap().location, [2.0, 0.0, 0.0], "location at landing");
    near(landed.world_matrix(cube).unwrap().transform_point3(Vec3::ZERO), [4.0, 0.0, 0.0], "world at landing");
}

#[test]
fn promo_gather_measures_and_keys_in_the_world() {
    let mut doc = new_doc();
    run(&mut doc, "object.addEmpty", json!({ "name": "Hub" }));
    run(&mut doc, "object.parent", json!({ "objects": ["Cube"], "parent": "Hub" }));
    run(&mut doc, "object.translate", json!({ "objects": ["Hub"], "offset": [10, 0, 0] }));
    near(world_at(&doc, "Cube"), [10.0, 0.0, 0.0], "before");
    run(&mut doc, "object.gather", json!({ "objects": ["Cube"], "center": [0, 0, 0], "radius": 2, "arc": 0, "start": 1, "duration": 10 }));
    // Seen 10 m from the centre at angle 0; one object, so its slot is angle 0 on the 2 m ring:
    // it ends at world [2, 0, 0], location [-8, 0, 0] (the parent adds 10). It starts where it
    // is. (Its location [0, 0, 0] was taken as the world place: radius 0 at the centre, and the
    // ring place [2, 0, 0] keyed as the location put it at [12, 0, 0].)
    let cube = id_of(&doc, "Cube");
    let first = doc.scene().at_frame(1.0);
    near(first.world_matrix(cube).unwrap().transform_point3(Vec3::ZERO), [10.0, 0.0, 0.0], "world at start");
    let last = doc.scene().at_frame(11.0);
    near(last.object(cube).unwrap().location, [-8.0, 0.0, 0.0], "location at the end");
    near(last.world_matrix(cube).unwrap().transform_point3(Vec3::ZERO), [2.0, 0.0, 0.0], "world at the end");
}

// --- issue-extrude-offset ------------------------------------------------------------------

#[test]
fn extrude_offset_adds_to_the_move_along_the_normal() {
    let mut doc = new_doc();
    run(&mut doc, "object.addPlane", json!({ "name": "Floor", "location": [0, 0, 5] }));
    run(&mut doc, "mode.set", json!({ "mode": "edit", "object": "Floor" }));
    let before = doc.scene().object_named("Floor").unwrap().mesh().unwrap().positions.clone();
    run(&mut doc, "mesh.extrude", json!({ "elements": { "object": "Floor", "vertices": [0, 1] }, "distance": 2, "offset": [1, 0, 0] }));
    // The plane's vertex normals are +Z: each new vertex is its source + [0, 0, 1] * 2 +
    // [1, 0, 0] = source + [1, 0, 2], as on a face. (`offset` replaced the 2 m along the normal:
    // source + [1, 0, 0].)
    let after = &doc.scene().object_named("Floor").unwrap().mesh().unwrap().positions;
    assert_eq!(after.len(), before.len() + 2);
    let made: Vec<Vec3> = after[before.len()..].to_vec();
    for source in [before[0], before[1]] {
        let wanted = source + Vec3::new(1.0, 0.0, 2.0);
        assert!(made.iter().any(|point| (*point - wanted).length() < 1e-4), "no new vertex at {wanted}: {made:?}");
    }
}

// --- issue-interpolation-tweak-frame -------------------------------------------------------

#[test]
fn interpolation_tweak_frame_maps_through_the_strip() {
    let mut doc = new_doc();
    run(&mut doc, "frame.set", json!({ "frame": 1 }));
    run(&mut doc, "animation.insertKey", json!({ "objects": "Cube", "properties": ["location"] }));
    run(&mut doc, "frame.set", json!({ "frame": 11 }));
    run(&mut doc, "object.transform", json!({ "objects": "Cube", "location": [0, 0, 2] }));
    run(&mut doc, "animation.insertKey", json!({ "objects": "Cube", "properties": ["location"] }));
    run(&mut doc, "animation.setInterpolation", json!({ "objects": "Cube", "interpolation": "linear" }));
    run(&mut doc, "nla.pushDown", json!({ "object": "Cube", "name": "Rise" }));
    run(&mut doc, "nla.setStrip", json!({ "object": "Cube", "track": 0, "strip": 0, "start": 21, "scale": 2 }));
    run(&mut doc, "nla.tweak", json!({ "object": "Cube", "track": 0, "strip": 0 }));
    // Strip from scene frame 21 at scale 2 over action 1..11: scene frame 21 is action frame
    // (21 - 21) / 2 + 1 = 1, as insertKey and deleteKey map it. (Frame 21 was compared with the
    // action's frames 1 and 11 as is: no key, refused.)
    run(&mut doc, "animation.setInterpolation", json!({ "objects": "Cube", "frame": 21, "interpolation": "constant" }));
    let cube = id_of(&doc, "Cube");
    for channel in doc.scene().animation.channels.iter().filter(|channel| channel.target == cube) {
        for key in &channel.keys {
            let wanted = if (key.frame - 1.0).abs() < 1e-4 { Interpolation::Constant } else { Interpolation::Linear };
            assert_eq!(key.interpolation, wanted, "key at action frame {}", key.frame);
        }
    }
}

// --- issue-dyntopo-multires-levels -----------------------------------------------------------

#[test]
fn dyntopo_levels_never_retopologise() {
    let mut doc = new_doc();
    run(&mut doc, "object.addUvSphere", json!({ "name": "Ball", "location": [6, 0, 0] }));
    run(&mut doc, "mode.sculpt", json!({ "on": true, "object": "Cube" }));
    run(&mut doc, "sculpt.dyntopo", json!({ "enabled": true }));
    // The guard on subdividing reads the session: Ball is not sculpted, so it gets levels.
    run(&mut doc, "multires.subdivide", json!({ "object": "Ball" }));
    run(&mut doc, "modifier.set", json!({ "object": "Ball", "index": 0, "sculptLevels": 0 }));
    run(&mut doc, "mode.sculpt", json!({ "on": true, "object": "Ball" }));
    let base = doc.scene().object_named("Ball").unwrap().mesh().unwrap().clone();
    assert_eq!(multires::levels(&base), 1);
    run(&mut doc, "sculpt.stroke", json!({ "points": [[6, 0, 1.0]], "worldRadius": 0.6, "strength": 1 }));
    // Ball has a multiresolution level: the stroke sculpts its base without dynamic topology,
    // so the base keeps its 482 vertices' count and its faces, and the level stays. (With its
    // sculpt level at 0 the stroke re-cut the base under the level.)
    let after = doc.scene().object_named("Ball").unwrap().mesh().unwrap();
    assert_eq!(after.positions.len(), base.positions.len(), "base vertex count");
    assert_eq!(after.faces, base.faces, "base faces");
    assert_eq!(multires::levels(after), 1, "levels");
}
