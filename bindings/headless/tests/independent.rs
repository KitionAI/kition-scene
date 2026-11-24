//! What the independent reading of every write command found (task card C8-scene-read,
//! `bindings/wasm/tests/declarations_independent.rs`), through the `kition-scene` executable and
//! the kits' MCP server: for each class of write the declarations let land on what the agent did
//! not observe, a colleague's program rewrites the file between the agent's inspect and its write
//! with `basis`; one change the write rests on must refuse it, one it does not rest on must not.
//! For each class of refusal no change needs, the unrelated change now applies.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::*;
use kition_kit::KitDocument;
use kition_kits::tools::CallOptions;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

/// The starter scene with cubes `C0`, `C1`, `C2`, then `setup` run on it, saved.
fn scene_with(setup: &[(&str, Value)]) -> Vec<u8> {
    let mut doc: SceneDocument = scene_doc(3);
    for (command, params) in setup {
        doc.execute(command, params.clone()).unwrap_or_else(|error| panic!("{command}: {error}"));
    }
    doc.save_file(None).unwrap()
}

fn step(command: &str, params: Value) -> Value {
    json!({ "command": command, "params": params })
}

/// Runs `write` with the `basis` of an inspect made before the colleague's `theirs`, on a fresh
/// copy of `bytes`: the answer, and the file's object `id` after it.
async fn after(bytes: &[u8], theirs: &[(&str, Value)], write: Value, id: &str) -> (Result<Value, Value>, Value) {
    let f = Fixture::from_bytes(bytes, CallOptions::default());
    let (details, basis) = f.inspect().await;
    let id = ids_by_name(&details)[id];
    f.colleague(theirs);
    // `write` is one step, or the steps of the run.
    let steps = if write.is_array() { write } else { json!([write]) };
    let answer = f.call("run", json!({ "basis": basis, "steps": steps })).await;
    let object = object_on_disk(&f.file(), id).unwrap_or(Value::Null);
    f.shutdown().await;
    (answer, object)
}

fn id_of(bytes: &[u8], name: &str) -> u64 {
    let doc = SceneDocument::open(FILE, bytes).unwrap();
    u64::from(doc.scene().object_named(name).unwrap().id)
}

fn refused_for(answer: &Result<Value, Value>, id: u64, reason: &str) {
    match answer {
        Err(refused) => {
            assert_conflict(refused, &id.to_string(), reason);
        }
        Ok(applied) => panic!("expected a refusal ({reason} on {id}), it applied: {applied}"),
    }
}

/// Class "a read that is not held: the object the selection or the active object names". A
/// command whose object parameter is left out acts on the active object (the selection): the
/// write held nothing, so the agent's rename of the active object overwrote the colleague's.
#[tokio::test(flavor = "multi_thread")]
async fn the_active_object_a_command_acts_on_is_held() {
    let bytes = scene_with(&[]);
    let (c0, c1) = (id_of(&bytes, "C0"), id_of(&bytes, "C1"));
    // A file holds no selection (the person's view state is not in the `.kscene`): the run
    // selects C0 itself, and the rename acts on the active object.
    let rename = json!([step("select.objects", json!({ "objects": [c0] })), step("object.rename", json!({ "name": "Mine" }))]);
    let (answer, object) = after(&bytes, &[("object.rename", json!({ "object": c0, "name": "Theirs" }))], rename.clone(), "C0").await;
    refused_for(&answer, c0, "changed");
    assert_eq!(object["name"], "Theirs", "the colleague's name kept");
    let (answer, object) = after(&bytes, &[("object.translate", json!({ "objects": [c1], "offset": [0, 0, 1] }))], rename, "C0").await;
    answer.unwrap_or_else(|error| panic!("an unrelated move refused the rename: {error}"));
    assert_eq!(object["name"], "Mine");
}

/// Class "a value naming an object, not held": `anim.insertKey`'s `objects` take ids the
/// declarations did not mark, and only a key of the transform was stated, so a key of a light's
/// power by id held nothing of the light: the value the colleague changed was keyed where the agent
/// did not look.
#[tokio::test(flavor = "multi_thread")]
async fn the_objects_a_key_names_by_id_are_held() {
    let bytes = scene_with(&[]);
    let (light, c1) = (id_of(&bytes, "Light"), id_of(&bytes, "C1"));
    let key = step("anim.insertKey", json!({ "objects": [light], "properties": ["power"], "frame": 1 }));
    let (answer, _) = after(&bytes, &[("light.set", json!({ "object": light, "power": 5 }))], key.clone(), "Light").await;
    refused_for(&answer, light, "changed");
    let (answer, object) = after(&bytes, &[("object.translate", json!({ "objects": [c1], "offset": [0, 0, 1] }))], key, "Light").await;
    answer.unwrap_or_else(|error| panic!("an unrelated move refused the key: {error}"));
    assert_eq!(object["animated"], json!(["power"]), "{object}");
}

/// Class "another object read through a reference, not held": a key of `baseColor` goes to the
/// material the object's slot names; the slot the colleague changed made the key land on
/// another material.
#[tokio::test(flavor = "multi_thread")]
async fn the_material_a_colour_key_goes_to_is_held() {
    let bytes = scene_with(&[
        ("material.new", json!({ "name": "Red", "assign": false })),
        ("material.new", json!({ "name": "Blue", "assign": false })),
        ("material.assign", json!({ "material": "Red", "objects": ["C0"] })),
    ]);
    let (c0, c1) = (id_of(&bytes, "C0"), id_of(&bytes, "C1"));
    let key = step("anim.insertKey", json!({ "objects": [c0], "properties": ["baseColor"], "frame": 1 }));
    let (answer, _) = after(&bytes, &[("material.assign", json!({ "material": "Blue", "objects": [c0] }))], key.clone(), "C0").await;
    refused_for(&answer, c0, "changed");
    let (answer, _) = after(&bytes, &[("object.rename", json!({ "object": c1, "name": "Other" }))], key, "C0").await;
    answer.unwrap_or_else(|error| panic!("an unrelated rename refused the key: {error}"));
}

/// Class "a default taken from elsewhere": `light.studio` aims its lights from the scene's camera
/// (the root's `camera`); the colleague who made another camera the scene's camera had the agent's
/// lights placed for a view it never saw.
#[tokio::test(flavor = "multi_thread")]
async fn the_scene_camera_a_light_rig_is_aimed_from_is_held() {
    let bytes = scene_with(&[("add.camera", json!({ "name": "Side", "location": [10, 0, 1] }))]);
    let c1 = id_of(&bytes, "C1");
    let side = id_of(&bytes, "Side");
    let studio = step("light.studio", json!({ "target": [0, 0, 0] }));
    let (answer, _) = after(&bytes, &[("object.setCamera", json!({ "object": side }))], studio.clone(), "C0").await;
    match &answer {
        Err(refused) => assert_eq!(refused["error"]["code"], "conflict", "{refused}"),
        Ok(applied) => panic!("expected a refusal after the scene camera changed: {applied}"),
    }
    let (answer, _) = after(&bytes, &[("object.rename", json!({ "object": c1, "name": "Other" }))], studio, "C0").await;
    answer.unwrap_or_else(|error| panic!("an unrelated rename refused the light rig: {error}"));
}

/// Class "a read that is not held: where an object's parent is": a knife by screen points cuts
/// the mesh where the object stands in the world, its parent's place included; the colleague's
/// move of the parent made the cut cross another part of the mesh.
#[tokio::test(flavor = "multi_thread")]
async fn the_parent_a_knife_projects_through_is_held() {
    let bytes = scene_with(&[
        ("add.empty", json!({ "name": "Rig" })),
        ("add.cube", json!({ "name": "Held", "size": 1 })),
        ("object.parent", json!({ "objects": ["Held"], "parent": "Rig" })),
        ("edit.delete", json!({ "objects": ["Cube"] })),
    ]);
    let (rig, held, c1) = (id_of(&bytes, "Rig"), id_of(&bytes, "Held"), id_of(&bytes, "C1"));
    // The screen points are in the 320 × 180 view the scene was made in; a file holds no view
    // (the person's view state), so the run sizes its own.
    let knife = json!([
        step("view.resize", json!({ "width": 320, "height": 180 })),
        step("mesh.knife", json!({ "elements": { "object": held }, "points": [[150, 60], [170, 120]] })),
    ]);
    let (answer, _) = after(&bytes, &[("object.transform", json!({ "objects": [rig], "location": [0.2, 0, 0] }))], knife.clone(), "Held").await;
    refused_for(&answer, rig, "relocated");
    let (answer, _) = after(&bytes, &[("object.transform", json!({ "objects": [c1], "location": [0, 9, 0] }))], knife, "Held").await;
    answer.unwrap_or_else(|error| panic!("an unrelated move refused the knife: {error}"));
}

/// Held more than read: a rigid body's settings rest on the object's `physics` alone, not on its
/// name or where it is; a colleague's rename or move of it refused the write for nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_physics_setting_is_not_refused_for_a_rename_or_a_move() {
    let bytes = scene_with(&[]);
    let c0 = id_of(&bytes, "C0");
    let body = step("physics.rigidBody.add", json!({ "objects": [c0], "mass": 2 }));
    let (answer, object) = after(&bytes, &[("object.rename", json!({ "object": c0, "name": "Theirs" }))], body.clone(), "C0").await;
    answer.unwrap_or_else(|error| panic!("a rename refused the rigid body: {error}"));
    assert_eq!(object["name"], "Theirs");
    let (answer, _) = after(&bytes, &[("object.translate", json!({ "objects": [c0], "offset": [0, 0, 1] }))], body.clone(), "C0").await;
    answer.unwrap_or_else(|error| panic!("a move refused the rigid body: {error}"));
    let (answer, _) = after(&bytes, &[("physics.rigidBody.add", json!({ "objects": [c0], "mass": 5 }))], body, "C0").await;
    refused_for(&answer, c0, "changed");
}

/// C8-scene-engine 4a: an `elements` edit works on the positions of the vertices it names, but
/// holds only the mesh's `~topology` (`src/rests.rs` `elements`): a colleague's move of a vertex of
/// face 0 applies under the agent's extrude of face 0, which then extrudes from where it never
/// saw the face. Wanted: the move of a vertex of face 0 refuses the extrude, the move of a vertex
/// elsewhere in the mesh (7, outside face 0 of the default cube: 0, 1, 3, 2) still applies (as
/// `outside.rs` s11's `VertexTarget` asks).
///
/// The SDK at 33b14be cannot express it: a mesh is one stored field (`mesh`, a blob reference),
/// so a premise on a path inside it (`mesh.positions.0`) overlaps every change of `mesh` (the
/// guard's `overlaps` on the changed field `mesh`) and is touched by the move of vertex 7 too; a
/// derived value (`KitDocument::derived`) has a fixed name per type, declared on the schema and
/// read for every object of the type, a function of the object's own fields alone, so it cannot
/// be "the positions of the vertices this call names" (a value per call or per element group).
/// Missing: a premise on a derived value with an argument (`~positions[0,1,2,3]`, the kit asked
/// for its value at basis and now), or change sets that name element paths inside a blob field.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "waiting for kition-sdk: a premise on part of a blob field or a derived value with arguments (the positions of the vertices an elements edit names)"]
async fn waiting_for_sdk_an_elements_edit_holds_the_positions_of_the_vertices_it_names() {
    let bytes = scene_with(&[]);
    let c0 = id_of(&bytes, "C0");
    let faces = {
        let doc = SceneDocument::open(FILE, &bytes).unwrap();
        doc.scene().object(u32::try_from(c0).unwrap()).unwrap().mesh().unwrap().faces.clone()
    };
    assert!(faces[0].contains(&0) && !faces[0].contains(&7), "face 0 is {:?}", faces[0]);
    let extrude = step("mesh.extrude", json!({ "elements": { "object": c0, "faces": [0] }, "distance": 0.5 }));
    let mv = |vertex: u32| ("mesh.translate", json!({ "elements": { "object": c0, "vertices": [vertex] }, "offset": [0, 0, 0.25] }));
    let (answer, _) = after(&bytes, &[mv(7)], extrude.clone(), "C0").await;
    answer.unwrap_or_else(|error| panic!("a move of vertex 7, outside face 0, refused the extrude: {error}"));
    let (answer, _) = after(&bytes, &[mv(0)], extrude, "C0").await;
    refused_for(&answer, c0, "changed");
}

/// C8-scene-engine 4b, class "a driven value read, its driver's sources not held": `C0`'s location z
/// follows `Handle`'s location x, and `Handle` is keyed from x 0 (frame 1) to x 4 (frame 19). The
/// agent's hop of `C1` onto `C0` lands at frame 19, on top of `C0` as the driver puts it there; the
/// colleague who cleared `Handle`'s keys changed only `Handle` (`C0` and its stored place are as
/// they were at frame 1), so the hop landed 4 m lower than the agent planned. The driver's source is
/// held now; an unrelated move still applies.
#[tokio::test(flavor = "multi_thread")]
async fn a_driven_value_rests_on_the_drivers_sources() {
    let bytes = scene_with(&[
        ("add.empty", json!({ "name": "Handle", "location": [0, 5, 0] })),
        ("anim.insertKey", json!({ "objects": ["Handle"], "properties": ["location"], "frame": 1 })),
        ("frame.set", json!({ "frame": 19 })),
        ("object.transform", json!({ "objects": ["Handle"], "location": [4, 5, 0] })),
        ("anim.insertKey", json!({ "objects": ["Handle"], "properties": ["location"], "frame": 19 })),
        ("frame.set", json!({ "frame": 1 })),
        (
            "driver.add",
            json!({ "object": "C0", "property": "location", "index": 2, "expression": "h",
                    "variables": [{ "name": "h", "type": "transform", "object": "Handle", "channel": "locationX" }] }),
        ),
    ]);
    let (c0, c1, c2, handle) = (id_of(&bytes, "C0"), id_of(&bytes, "C1"), id_of(&bytes, "C2"), id_of(&bytes, "Handle"));
    let hop = step("object.hop", json!({ "objects": [c1], "to": c0, "start": 1, "duration": 18 }));
    let (answer, _) = after(&bytes, &[("anim.clear", json!({ "objects": [handle] }))], hop.clone(), "C1").await;
    refused_for(&answer, handle, "changed");
    let (answer, _) = after(&bytes, &[("object.translate", json!({ "objects": [c2], "offset": [0, 0, 1] }))], hop, "C1").await;
    answer.unwrap_or_else(|error| panic!("an unrelated move refused the hop: {error}"));
}

/// One session kind of C8-scene-engine 4c: on the starter scene with `setup` run (its last step
/// selects the session's object `object`), an agent's run enters the session on the active object
/// and then writes in it (`run`). The session is editor state no host sets for a call: the
/// kit answers `under` for each step on the document as it was before the run (object mode), so
/// the session's object is held by the step that enters it. `theirs` changes that object (the
/// write is refused for it); a move of `C1` is unrelated (the write applies).
async fn session_holds_its_object(setup: &[(&str, Value)], object: &str, run: Vec<Value>, theirs: (&str, Value)) {
    let bytes = scene_with(setup);
    let (held, c1) = (id_of(&bytes, object), id_of(&bytes, "C1"));
    // A file holds no selection (it is the person's view state, never in the `.kscene`): the run
    // selects the session's object itself, by id, before it enters the session.
    let run: Vec<Value> = std::iter::once(step("select.objects", json!({ "objects": [held] }))).chain(run).collect();
    let run = |basis: &str| json!({ "basis": basis, "steps": run.clone() });
    let f = Fixture::from_bytes(&bytes, CallOptions::default());
    let (_, basis) = f.inspect().await;
    f.colleague(std::slice::from_ref(&theirs));
    let answer = f.call("run", run(&basis)).await;
    f.shutdown().await;
    refused_for(&answer, held, "changed");
    let f = Fixture::from_bytes(&bytes, CallOptions::default());
    let (_, basis) = f.inspect().await;
    f.colleague(&[("object.translate", json!({ "objects": [c1], "offset": [0, 0, 1] }))]);
    let answer = f.call("run", run(&basis)).await;
    f.shutdown().await;
    answer.unwrap_or_else(|error| panic!("an unrelated move refused the session's write: {error}"));
}

fn select(name: &str) -> (&'static str, Value) {
    ("select.objects", json!({ "objects": [name] }))
}

/// Edit mode without `elements`: the run's `mesh.extrude` extrudes the edited mesh's selection,
/// a mesh no step names.
#[tokio::test(flavor = "multi_thread")]
async fn an_edit_mode_session_holds_its_mesh() {
    let theirs = ("mesh.subdivide", json!({ "elements": { "object": "C0" } }));
    let run = vec![step("mode.set", json!({ "mode": "edit" })), step("mesh.extrude", json!({ "distance": 0.5 }))];
    session_holds_its_object(&[select("C0")], "C0", run, theirs).await;
}

/// Armature edit mode: `bone.add` adds to the session's armature.
#[tokio::test(flavor = "multi_thread")]
async fn an_armature_edit_session_holds_its_armature() {
    let setup = [("add.armature", json!({ "name": "Arm", "location": [-3, 0, 0] })), select("Arm")];
    let theirs = ("bone.set", json!({ "object": "Arm", "bone": "Bone", "tail": [0, 0, 3] }));
    let run = vec![step("mode.armature", json!({})), step("bone.add", json!({ "name": "B2", "head": [0, 0, 1], "tail": [0, 0, 2] }))];
    session_holds_its_object(&setup, "Arm", run, theirs).await;
}

/// Pose mode: `pose.set` poses the session's armature's bones.
#[tokio::test(flavor = "multi_thread")]
async fn a_pose_session_holds_its_armature() {
    let setup = [("add.armature", json!({ "name": "Arm", "location": [-3, 0, 0] })), select("Arm")];
    let theirs = ("bone.set", json!({ "object": "Arm", "bone": "Bone", "tail": [0, 0, 3] }));
    let run = vec![step("mode.pose", json!({})), step("pose.set", json!({ "bones": ["Bone"], "rotation": [0, 0, 45] }))];
    session_holds_its_object(&setup, "Arm", run, theirs).await;
}

/// Weight paint: `paint.stroke` paints the session's mesh where it is in the view.
#[tokio::test(flavor = "multi_thread")]
async fn a_weight_paint_session_holds_its_mesh() {
    let setup = [("vertexGroup.add", json!({ "object": "C0", "name": "G1" })), select("C0")];
    let theirs = ("vertexGroup.add", json!({ "object": "C0", "name": "G2" }));
    let run = vec![step("mode.weightPaint", json!({})), step("paint.stroke", json!({ "points": [[160, 90], [170, 95]], "group": "G1" }))];
    session_holds_its_object(&setup, "C0", run, theirs).await;
}

/// Curve point edit: `curve.translate` moves the session's selected points.
#[tokio::test(flavor = "multi_thread")]
async fn a_curve_edit_session_holds_its_curve() {
    let setup = [("add.curveBezier", json!({ "name": "Path", "location": [0, 3, 0] })), select("Path")];
    let theirs = ("curve.set", json!({ "object": "Path", "resolution": 6 }));
    let run =
        vec![step("mode.shape", json!({})), step("curve.select", json!({ "points": [0] })), step("curve.translate", json!({ "offset": [0, 0, 1] }))];
    session_holds_its_object(&setup, "Path", run, theirs).await;
}

/// Text edit: `text.delete` deletes before the session's caret.
#[tokio::test(flavor = "multi_thread")]
async fn a_text_edit_session_holds_its_text() {
    let setup = [("add.text", json!({ "name": "Label", "location": [0, -3, 0] })), select("Label")];
    let theirs = ("text.set", json!({ "object": "Label", "body": "Other" }));
    let run = vec![step("mode.shape", json!({})), step("text.delete", json!({ "count": 1 }))];
    session_holds_its_object(&setup, "Label", run, theirs).await;
}

/// Lattice edit: `lattice.movePoints` moves the session's selected points.
#[tokio::test(flavor = "multi_thread")]
async fn a_lattice_edit_session_holds_its_lattice() {
    let setup = [("add.lattice", json!({ "name": "Cage", "location": [0, 0, -3] })), select("Cage")];
    let theirs = ("lattice.set", json!({ "object": "Cage", "interpolation": "linear" }));
    let run = vec![
        step("mode.shape", json!({})),
        step("lattice.select", json!({ "points": [0] })),
        step("lattice.movePoints", json!({ "offset": [0, 0, 1] })),
    ];
    session_holds_its_object(&setup, "Cage", run, theirs).await;
}

/// Particle Edit: `particleEdit.reset` clears the groom of the session's system.
#[tokio::test(flavor = "multi_thread")]
async fn a_particle_edit_session_holds_its_emitter() {
    let setup = [("particles.add", json!({ "object": "C0", "kind": "hair", "count": 20 })), select("C0")];
    let theirs = ("particles.set", json!({ "object": "C0", "count": 30 }));
    let run = vec![step("mode.particleEdit", json!({})), step("particleEdit.reset", json!({}))];
    session_holds_its_object(&setup, "C0", run, theirs).await;
}

/// Sculpt: `sculpt.maskInvert` inverts the session's mesh's mask.
#[tokio::test(flavor = "multi_thread")]
async fn a_sculpt_session_holds_its_mesh() {
    let theirs = ("mesh.subdivide", json!({ "elements": { "object": "C0" } }));
    let run = vec![step("mode.sculpt", json!({})), step("sculpt.maskInvert", json!({}))];
    session_holds_its_object(&[select("C0")], "C0", run, theirs).await;
}

/// C8-scene-engine 4c, what the session holds is exact: a run that enters edit mode and moves the
/// selected vertices (`mesh.translate` without `elements`) rests on the mesh's topology only, as
/// the same move by `elements` does, so a colleague's rename of the mesh should not refuse it.
///
/// The SDK at 33b14be asks the kit what each step rests on (`context.under`) with the document as
/// it was before the run, one step at a time: a session an earlier step of the run enters is not
/// on when the later step is asked about, so the kit cannot say which object the step acts on
/// and the step that enters the session holds its object whole (every field: the rename refuses
/// the run). Missing: `context.under` asked with the steps of the run before it (or on the
/// document after them), as the run would leave the editor for that step.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "waiting for kition-sdk: context.under asked with the editor state the run's earlier steps leave (a session entered in the same run)"]
async fn waiting_for_sdk_a_step_in_a_session_holds_only_what_it_reads() {
    let bytes = scene_with(&[select("C0")]);
    let c0 = id_of(&bytes, "C0");
    let run = vec![step("mode.set", json!({ "mode": "edit" })), step("mesh.translate", json!({ "offset": [0, 0, 1] }))];
    let f = Fixture::from_bytes(&bytes, CallOptions::default());
    let (_, basis) = f.inspect().await;
    f.colleague(&[("object.rename", json!({ "object": c0, "name": "Theirs" }))]);
    let answer = f.call("run", json!({ "basis": basis, "steps": run })).await;
    f.shutdown().await;
    answer.unwrap_or_else(|error| panic!("a rename refused a move of the edited mesh's vertices: {error}"));
}

/// C8-scene-engine 5a: an agent's `edit.undo` takes back the agent's last step; it held the whole
/// scene (`("scene", "*", "*")`), so any colleague's change refused it. It holds the objects the
/// steps it may take back wrote: the agent's run renames `C0` and takes the rename back; a
/// colleague's move of `C1` since the agent's inspect keeps the run, the colleague's rename of `C0`
/// refuses it.
#[tokio::test(flavor = "multi_thread")]
async fn an_undo_holds_the_objects_its_step_wrote() {
    let bytes = scene_with(&[]);
    let (c0, c1) = (id_of(&bytes, "C0"), id_of(&bytes, "C1"));
    let run = vec![step("object.rename", json!({ "object": c0, "name": "Mine" })), step("edit.undo", json!({}))];
    let undo = |theirs: (&'static str, Value)| {
        let (bytes, run) = (bytes.clone(), run.clone());
        async move {
            let f = Fixture::from_bytes(&bytes, CallOptions::default());
            let (_, basis) = f.inspect().await;
            f.colleague(&[theirs]);
            let answer = f.call("run", json!({ "basis": basis, "steps": run })).await;
            let name = object_on_disk(&f.file(), c0).unwrap()["name"].clone();
            f.shutdown().await;
            (answer, name)
        }
    };
    let (answer, name) = undo(("object.translate", json!({ "objects": [c1], "offset": [0, 0, 1] }))).await;
    answer.unwrap_or_else(|error| panic!("an unrelated move refused the undo: {error}"));
    assert_eq!(name, "C0", "the undo took the rename back");
    let (answer, name) = undo(("object.rename", json!({ "object": c0, "name": "Theirs" }))).await;
    refused_for(&answer, c0, "changed");
    assert_eq!(name, "Theirs");
}

/// C8-scene-engine 5a, across runs: the agent renames `C0` in one run, a colleague moves `C1`, and
/// the agent's next run (with the first run's revision as `basis`) undoes the rename. Wanted: the
/// undo takes the rename back (it holds `C0`, which the colleague did not change).
///
/// The SDK at 33b14be reads the file again when another program changed it (kition-headless
/// `reload` replaces the open document): the agent's undo history is gone, the undo has nothing
/// to take back and the rename stays. Missing: a reload that keeps the document's undo history
/// (the file's changes merged into the open replica), or the history kept with the file.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "waiting for kition-sdk: a document read again after another program's change keeps its undo history"]
async fn waiting_for_sdk_an_undo_after_a_colleagues_change_takes_back_the_agents_step() {
    let bytes = scene_with(&[]);
    let (c0, c1) = (id_of(&bytes, "C0"), id_of(&bytes, "C1"));
    let f = Fixture::from_bytes(&bytes, CallOptions::default());
    let ran = f.ok("run", json!({ "steps": [step("object.rename", json!({ "object": c0, "name": "Mine" }))] })).await;
    f.colleague(&[("object.translate", json!({ "objects": [c1], "offset": [0, 0, 1] }))]);
    let answer = f.call("run", json!({ "basis": revision(&ran), "steps": [step("edit.undo", json!({}))] })).await;
    let name = object_on_disk(&f.file(), c0).unwrap()["name"].clone();
    f.shutdown().await;
    answer.unwrap_or_else(|error| panic!("an unrelated move refused the undo: {error}"));
    assert_eq!(name, "C0", "the undo took the rename back");
}

/// C8-scene-engine 5b: `compositor.pickMatte` adds the name under a point of the render through the
/// scene camera; it held every object and material (any change refused it). It holds the object
/// under the point now and what its name is read from, and tells the place with `covers`: the
/// objects others moved or changed in what decides what the ray meets are put back as they were
/// at the agent's basis and the ray is cast again. The colleague's move of the cube under the point
/// refuses the pick; the move of a cube elsewhere does not.
#[tokio::test(flavor = "multi_thread")]
async fn a_matte_pick_holds_what_is_under_the_point() {
    let mut doc: SceneDocument = scene_doc(3);
    let made = doc.execute("compositor.new", json!({})).unwrap();
    let group = made["group"].clone();
    let crypto = doc.execute("nodes.addNode", json!({ "group": group, "kind": "cryptomatte" })).unwrap()["node"].clone();
    let bytes = doc.save_file(None).unwrap();
    let (cube, c1) = (id_of(&bytes, "Cube"), id_of(&bytes, "C1"));
    let pick = step("compositor.pickMatte", json!({ "group": group, "node": crypto, "x": 0.5, "y": 0.5 }));
    let (answer, _) = after(&bytes, &[("object.translate", json!({ "objects": [cube], "offset": [0, 0, 50] }))], pick.clone(), "Cube").await;
    match &answer {
        Err(refused) => assert_eq!(refused["error"]["code"], "conflict", "{refused}"),
        Ok(applied) => panic!("expected a refusal after the cube under the point moved away: {applied}"),
    }
    // A cube moved between the camera and the cube under the point: the ray meets it first now.
    let camera = doc.scene().object(doc.scene().camera.unwrap()).unwrap().location;
    let between = camera * 0.5;
    let (answer, _) =
        after(&bytes, &[("object.transform", json!({ "objects": [c1], "location": [between.x, between.y, between.z] }))], pick.clone(), "Cube").await;
    refused_for(&answer, c1, "relocated");
    let (answer, _) = after(&bytes, &[("object.translate", json!({ "objects": [c1], "offset": [0, 0, 1] }))], pick, "Cube").await;
    let applied = answer.unwrap_or_else(|error| panic!("an unrelated move refused the pick: {error}"));
    assert_eq!(applied["completed"][0]["result"]["name"], "Cube", "{applied}");
}
