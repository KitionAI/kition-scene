//! An explicit parameter wins: a command that names an object, a group or faces acts on what it
//! names, or refuses with `not-allowed` saying what to do. It never acts on another object (the
//! edited mesh, the session's armature or mesh, the selection) and never answers success for
//! doing nothing (C8-scene-engine group 1, found by the independent reading of C8-scene-read).
//! Cases `regressions/issue-edit-mode-named-object`, `issue-bone-set-named-armature`,
//! `issue-delete-named-objects`, `issue-paint-named-mesh-group`, `issue-bisect-symmetrize-whole-mesh`,
//! `issue-elements-faces-exact`, `issue-armature-modifier-operand`, `issue-gates-named-objects`,
//! `issue-digit-names` and `issue-named-object-modes`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use kition_scene_model::{ModifierKind, ObjectData};
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn refused(doc: &mut SceneDocument, command: &str, params: Value) -> String {
    match doc.execute(command, params) {
        Ok(value) => panic!("{command} should be refused, answered {value}"),
        Err(error) => error,
    }
}

fn new_doc() -> SceneDocument {
    SceneDocument::create("Named.kscene", 320, 180).unwrap()
}

fn id_of(doc: &SceneDocument, name: &str) -> u32 {
    doc.scene().object_named(name).unwrap_or_else(|| panic!("no {name}")).id
}

fn mesh_of<'a>(doc: &'a SceneDocument, name: &str) -> &'a kition_scene_model::Mesh {
    doc.scene().object_named(name).unwrap().mesh().unwrap()
}

fn has(doc: &SceneDocument, name: &str) -> bool {
    doc.scene().object_named(name).is_some()
}

/// The roll of `bone` in armature `name`, degrees.
fn roll(doc: &SceneDocument, name: &str, bone: &str) -> f32 {
    let ObjectData::Armature(armature) = &doc.scene().object_named(name).unwrap().data else { panic!("{name} is no armature") };
    armature.bone(bone).unwrap().roll.to_degrees()
}

fn group_names(doc: &SceneDocument, name: &str) -> Vec<String> {
    mesh_of(doc, name).groups.iter().map(|group| group.name.clone()).collect()
}

// --- issue-edit-mode-named-object ---------------------------------------------------------

/// Edit mode on Cube; a second mesh Sphere to name.
fn editing_cube_with_sphere() -> SceneDocument {
    let mut doc = new_doc();
    run(&mut doc, "object.addUvSphere", json!({ "name": "Sphere", "location": [4, 0, 0] }));
    run(&mut doc, "mode.set", json!({ "mode": "edit", "object": "Cube" }));
    doc
}

#[test]
fn edit_mode_set_geometry_writes_the_named_mesh() {
    let mut doc = editing_cube_with_sphere();
    let cube = mesh_of(&doc, "Cube").clone();
    let triangle = json!({ "object": "Sphere", "positions": [[0, 0, 0], [1, 0, 0], [0, 1, 0]], "faces": [[0, 1, 2]] });
    let answer = run(&mut doc, "mesh.setGeometry", triangle);
    assert_eq!(answer["object"], id_of(&doc, "Sphere"));
    assert_eq!(mesh_of(&doc, "Sphere").positions.len(), 3, "the named mesh gets the geometry");
    assert_eq!(mesh_of(&doc, "Cube").positions, cube.positions, "the edited mesh is untouched");
    assert_eq!(doc.session().vertices.len(), 8, "the edit selection of the edited mesh stays");
}

#[test]
fn edit_mode_modifier_add_goes_on_the_named_object() {
    let mut doc = editing_cube_with_sphere();
    run(&mut doc, "modifier.add", json!({ "object": "Sphere", "type": "subdivision" }));
    assert_eq!(doc.scene().object_named("Sphere").unwrap().modifiers.len(), 1, "the named object gets the modifier");
    assert!(doc.scene().object_named("Cube").unwrap().modifiers.is_empty(), "the edited mesh gets none");
}

#[test]
fn edit_mode_shading_acts_on_the_named_objects() {
    let mut doc = editing_cube_with_sphere();
    run(&mut doc, "object.shadeSmooth", json!({ "objects": ["Sphere"] }));
    assert!(mesh_of(&doc, "Sphere").smooth);
    let cube_smooth = mesh_of(&doc, "Cube").smooth;
    let answer = run(&mut doc, "object.shadeFlat", json!({ "objects": ["Sphere"] }));
    assert_eq!(answer["objects"], json!([id_of(&doc, "Sphere")]));
    assert!(!mesh_of(&doc, "Sphere").smooth, "the named mesh is shaded flat");
    assert_eq!(mesh_of(&doc, "Cube").smooth, cube_smooth, "the edited mesh is untouched");
}

#[test]
fn edit_mode_uv_projection_acts_on_the_named_mesh() {
    let mut doc = editing_cube_with_sphere();
    let (cube, sphere) = (mesh_of(&doc, "Cube").uvs.clone(), mesh_of(&doc, "Sphere").uvs.clone());
    run(&mut doc, "uv.planarProject", json!({ "objects": ["Sphere"], "axis": "x" }));
    assert_eq!(mesh_of(&doc, "Cube").uvs, cube, "the edited mesh's UVs stay");
    assert_ne!(mesh_of(&doc, "Sphere").uvs, sphere, "the named mesh is projected");
    run(&mut doc, "uv.clear", json!({ "objects": ["Sphere"] }));
    assert!(mesh_of(&doc, "Sphere").uvs.is_empty());
    assert_eq!(mesh_of(&doc, "Cube").uvs, cube, "uv.clear leaves the edited mesh's UVs");
}

#[test]
fn edit_mode_recalculate_normals_acts_on_the_named_mesh() {
    let mut doc = editing_cube_with_sphere();
    // Sphere's faces turned inside out; the edited Cube's turned too, so a wrong target shows.
    run(&mut doc, "mesh.flip", json!({}));
    let cube = mesh_of(&doc, "Cube").faces.clone();
    let mut inside_out = mesh_of(&doc, "Sphere").clone();
    for face in &mut inside_out.faces {
        face.reverse();
    }
    let positions: Vec<[f32; 3]> = inside_out.positions.iter().map(|point| point.to_array()).collect();
    run(&mut doc, "mode.set", json!({ "mode": "object" }));
    run(&mut doc, "mesh.setGeometry", json!({ "object": "Sphere", "positions": positions, "faces": inside_out.faces }));
    run(&mut doc, "mode.set", json!({ "mode": "edit", "object": "Cube" }));
    run(&mut doc, "mesh.recalculateNormals", json!({ "objects": ["Sphere"] }));
    assert_eq!(mesh_of(&doc, "Cube").faces, cube, "the edited mesh keeps its faces");
    assert_ne!(mesh_of(&doc, "Sphere").faces, inside_out.faces, "the named mesh is turned outside");
}

#[test]
fn edit_mode_naming_the_edited_mesh_acts_as_before() {
    let mut doc = editing_cube_with_sphere();
    run(&mut doc, "modifier.add", json!({ "object": "Cube", "type": "subdivision" }));
    assert_eq!(doc.scene().object_named("Cube").unwrap().modifiers.len(), 1);
    let triangle = json!({ "object": "Cube", "positions": [[0, 0, 0], [1, 0, 0], [0, 1, 0]], "faces": [[0, 1, 2]] });
    run(&mut doc, "mesh.setGeometry", triangle);
    assert_eq!(mesh_of(&doc, "Cube").positions.len(), 3);
    assert!(doc.session().vertices.is_empty(), "the edited mesh's selection is cleared with its old vertices");
}

// --- issue-bone-set-named-armature --------------------------------------------------------

#[test]
fn bone_set_changes_the_named_armature_while_another_is_posed() {
    let mut doc = new_doc();
    run(&mut doc, "object.addArmature", json!({ "name": "Rig" }));
    run(&mut doc, "object.addArmature", json!({ "name": "Other", "location": [3, 0, 0] }));
    run(&mut doc, "mode.pose", json!({ "object": "Rig", "on": true }));
    run(&mut doc, "bone.set", json!({ "object": "Other", "bone": "Bone", "roll": 45 }));
    assert!((roll(&doc, "Other", "Bone") - 45.0).abs() < 1e-3, "the named armature's bone turns");
    assert!(roll(&doc, "Rig", "Bone").abs() < 1e-3, "the posed armature's bone stays");
    // Without `object` the session's armature is still the one.
    run(&mut doc, "bone.set", json!({ "bone": "Bone", "roll": 10 }));
    assert!((roll(&doc, "Rig", "Bone") - 10.0).abs() < 1e-3);
}

// --- issue-delete-named-objects -----------------------------------------------------------

#[test]
fn delete_named_objects_in_mesh_edit_mode_deletes_them() {
    let mut doc = editing_cube_with_sphere();
    let cube = mesh_of(&doc, "Cube").clone();
    run(&mut doc, "edit.delete", json!({ "objects": ["Sphere"] }));
    assert!(!has(&doc, "Sphere"), "the named object is deleted");
    assert_eq!(mesh_of(&doc, "Cube").positions, cube.positions, "the edited mesh keeps its vertices");
    assert_eq!(doc.session().mode, kition_scene_kit::Mode::Edit(id_of(&doc, "Cube")), "edit mode on Cube goes on");
    // Naming the edited mesh deletes it, and edit mode ends.
    run(&mut doc, "edit.delete", json!({ "objects": ["Cube"] }));
    assert!(!has(&doc, "Cube"));
    assert_eq!(doc.session().mode, kition_scene_kit::Mode::Object);
}

#[test]
fn delete_named_objects_in_curve_lattice_and_armature_edit() {
    let mut doc = new_doc();
    run(&mut doc, "object.addLattice", json!({ "name": "Cage" }));
    run(&mut doc, "mode.shape", json!({}));
    // A plain Delete in lattice edit deletes nothing and keeps the lattice (modifiers3.rs, older
    // than this case, holds it answering success).
    run(&mut doc, "edit.delete", json!({}));
    assert!(has(&doc, "Cage"));
    run(&mut doc, "edit.delete", json!({ "objects": ["Cube"] }));
    assert!(!has(&doc, "Cube"), "the named object goes in lattice edit");
    assert!(has(&doc, "Cage"));
    run(&mut doc, "mode.shape", json!({}));
    run(&mut doc, "object.addCurveBezier", json!({ "name": "Path" }));
    run(&mut doc, "mode.shape", json!({}));
    run(&mut doc, "edit.delete", json!({ "objects": ["Cage"] }));
    assert!(!has(&doc, "Cage"), "the named object goes in curve edit");
    let ObjectData::Curve(curve) = &doc.scene().object_named("Path").unwrap().data else { panic!() };
    assert_eq!(curve.splines.len(), 1, "the edited curve keeps its points");
    run(&mut doc, "mode.shape", json!({}));
    run(&mut doc, "object.addArmature", json!({ "name": "Rig" }));
    run(&mut doc, "mode.armature", json!({}));
    run(&mut doc, "bone.select", json!({ "bones": ["Bone"] }));
    run(&mut doc, "edit.delete", json!({ "objects": ["Path"] }));
    assert!(!has(&doc, "Path"), "the named object goes in armature edit");
    let ObjectData::Armature(armature) = &doc.scene().object_named("Rig").unwrap().data else { panic!() };
    assert_eq!(armature.bones.len(), 1, "the selected bone stays");
}

#[test]
fn delete_in_sculpt_and_particle_edit_keeps_the_object_being_worked_on() {
    let mut doc = new_doc();
    run(&mut doc, "mode.sculpt", json!({ "object": "Cube", "on": true }));
    let error = refused(&mut doc, "edit.delete", json!({}));
    assert!(error.starts_with("not-allowed"), "{error}");
    assert!(has(&doc, "Cube"), "sculpt mode does not delete the sculpted mesh");
    run(&mut doc, "mode.sculpt", json!({ "on": false }));
    run(&mut doc, "object.addUvSphere", json!({ "name": "Ball" }));
    run(&mut doc, "particle.add", json!({ "kind": "hair", "hairLength": 1, "count": 20 }));
    run(&mut doc, "mode.particleEdit", json!({}));
    let error = refused(&mut doc, "edit.delete", json!({}));
    assert!(error.starts_with("not-allowed"), "{error}");
    assert!(has(&doc, "Ball"), "Particle Edit does not delete the emitter");
    run(&mut doc, "edit.delete", json!({ "objects": ["Cube"] }));
    assert!(!has(&doc, "Cube") && has(&doc, "Ball"), "a named object goes");
}

// --- issue-paint-named-mesh-group ---------------------------------------------------------

#[test]
fn paint_on_a_named_mesh_uses_its_own_groups() {
    let mut doc = new_doc();
    run(&mut doc, "object.addUvSphere", json!({ "name": "Ball", "location": [4, 0, 0] }));
    run(&mut doc, "vertexGroup.add", json!({ "object": "Cube", "name": "Arm" }));
    run(&mut doc, "vertexGroup.add", json!({ "object": "Ball", "name": "Leg" }));
    run(&mut doc, "mode.weightPaint", json!({ "object": "Cube", "on": true }));
    let painted = run(&mut doc, "paint.vertices", json!({ "object": "Ball", "vertices": [0] }));
    assert_ne!(painted["group"], "Arm", "the session's group is not Ball's: {painted}");
    assert!(!group_names(&doc, "Ball").contains(&"Arm".to_string()), "Ball gets no group named after Cube's: {:?}", group_names(&doc, "Ball"));
    // On the session's own mesh the paint group stays the default.
    let own = run(&mut doc, "paint.vertices", json!({ "vertices": [0] }));
    assert_eq!(own["group"], "Arm");
}

#[test]
fn paint_normalize_all_on_a_named_mesh_locks_none_of_the_session_groups() {
    let mut doc = new_doc();
    run(&mut doc, "object.addUvSphere", json!({ "name": "Ball", "location": [4, 0, 0] }));
    run(&mut doc, "vertexGroup.add", json!({ "object": "Cube", "name": "Arm" }));
    // Ball has its own group called Arm too, with weight 0.25, and a second group with 0.25:
    // with Cube's Arm taken for Ball's locked group, Ball's Arm would stay 0.25.
    run(&mut doc, "paint.vertices", json!({ "object": "Ball", "group": "Arm", "vertices": [0], "weight": 0.25 }));
    run(&mut doc, "paint.vertices", json!({ "object": "Ball", "group": "Other", "vertices": [0], "weight": 0.25 }));
    run(&mut doc, "mode.weightPaint", json!({ "object": "Cube", "on": true }));
    run(&mut doc, "paint.normalizeAll", json!({ "object": "Ball" }));
    let mesh = mesh_of(&doc, "Ball");
    let arm = mesh.groups.iter().find(|group| group.name == "Arm").unwrap().weights[0];
    let other = mesh.groups.iter().find(|group| group.name == "Other").unwrap().weights[0];
    assert!((arm + other - 1.0).abs() < 1e-4 && (arm - other).abs() < 1e-4, "both groups share the vertex evenly: {arm} {other}");
}

// --- issue-bisect-symmetrize-whole-mesh -----------------------------------------------------

#[test]
fn bisect_and_symmetrize_say_they_act_on_the_whole_mesh() {
    let doc = new_doc();
    let catalogue = doc.commands();
    for id in ["mesh.bisect", "mesh.symmetrize"] {
        let command = catalogue.iter().find(|command| command.id == id).unwrap();
        let description = &command.description;
        assert!(description.contains("whole"), "{id} says it acts on the whole mesh, whatever is selected: {description}");
    }
}

// --- issue-elements-faces-exact -----------------------------------------------------------

/// The default cube's top and bottom faces (the faces whose corners all share one z).
fn top_and_bottom(doc: &SceneDocument) -> Vec<usize> {
    let mesh = mesh_of(doc, "Cube");
    (0..mesh.faces.len())
        .filter(|face| {
            let corners = &mesh.faces[*face];
            corners.iter().all(|corner| (mesh.positions[*corner as usize].z - mesh.positions[corners[0] as usize].z).abs() < 1e-6)
        })
        .collect()
}

#[test]
fn elements_faces_select_only_the_named_faces() {
    let mut doc = new_doc();
    let faces = top_and_bottom(&doc);
    assert_eq!(faces.len(), 2);
    run(&mut doc, "mesh.inset", json!({ "elements": { "object": "Cube", "faces": faces }, "thickness": 0.2 }));
    // Two faces inset make 2 x 4 new rim faces: 6 + 8 = 14; all six would make 30.
    assert_eq!(mesh_of(&doc, "Cube").faces.len(), 14, "only the two named faces are inset");
    let mut doc = new_doc();
    let faces = top_and_bottom(&doc);
    run(&mut doc, "mesh.deleteFaces", json!({ "elements": { "object": "Cube", "faces": faces } }));
    assert_eq!(mesh_of(&doc, "Cube").faces.len(), 4, "the two named faces go, the four sides stay");
}

// --- issue-armature-modifier-operand ------------------------------------------------------

#[test]
fn modifier_set_changes_an_armature_modifiers_armature() {
    let mut doc = new_doc();
    run(&mut doc, "object.addArmature", json!({ "name": "Rig" }));
    run(&mut doc, "object.addArmature", json!({ "name": "Rig2", "location": [3, 0, 0] }));
    run(&mut doc, "modifier.add", json!({ "object": "Cube", "type": "armature", "operand": "Rig" }));
    let armature_of = |doc: &SceneDocument| match &doc.scene().object_named("Cube").unwrap().modifiers[0].kind {
        ModifierKind::Armature { object, .. } => *object,
        other => panic!("{other:?}"),
    };
    assert_eq!(armature_of(&doc), Some(id_of(&doc, "Rig")), "modifier.add takes the armature as `operand`");
    run(&mut doc, "modifier.set", json!({ "object": "Cube", "index": 0, "operand": "Rig2" }));
    assert_eq!(armature_of(&doc), Some(id_of(&doc, "Rig2")), "modifier.set changes the armature");
    let error = refused(&mut doc, "modifier.set", json!({ "object": "Cube", "index": 0, "operand": "Camera" }));
    assert!(error.starts_with("invalid-params"), "a camera is no armature: {error}");
}

// --- issue-gates-named-objects ------------------------------------------------------------

#[test]
fn gates_named_objects_run_without_a_selection() {
    let mut doc = new_doc();
    run(&mut doc, "object.addEmpty", json!({ "name": "Tag" }));
    run(&mut doc, "edit.deselectAll", json!({}));
    assert!(doc.session().editor.selected.is_empty());
    let before = doc.scene().object_named("Cube").unwrap().location;
    run(&mut doc, "object.translate", json!({ "objects": ["Cube"], "offset": [1, 0, 0] }));
    assert!((doc.scene().object_named("Cube").unwrap().location.x - before.x - 1.0).abs() < 1e-5);
    run(&mut doc, "object.rotate", json!({ "objects": ["Cube"], "axis": "z", "angle": 10 }));
    run(&mut doc, "object.scale", json!({ "objects": ["Cube"], "factor": 2 }));
    run(&mut doc, "object.applyTransform", json!({ "objects": ["Cube"] }));
    run(&mut doc, "object.originToGeometry", json!({ "objects": ["Cube"] }));
    run(&mut doc, "object.parent", json!({ "objects": ["Tag"], "parent": "Cube" }));
    run(&mut doc, "object.clearParent", json!({ "objects": ["Tag"] }));
    run(&mut doc, "object.hide", json!({ "objects": ["Tag"] }));
    run(&mut doc, "mesh.recalculateNormals", json!({ "objects": ["Cube"] }));
    run(&mut doc, "object.shadeSmooth", json!({ "objects": ["Cube"] }));
    run(&mut doc, "material.new", json!({ "name": "Red" }));
    run(&mut doc, "edit.deselectAll", json!({}));
    run(&mut doc, "material.clear", json!({ "objects": ["Cube"] }));
    run(&mut doc, "uv.cubeProject", json!({ "objects": ["Cube"] }));
    run(&mut doc, "object.addCurveBezier", json!({ "name": "Path" }));
    run(&mut doc, "edit.deselectAll", json!({}));
    run(&mut doc, "object.convertToMesh", json!({ "objects": ["Path"] }));
    run(&mut doc, "object.addArmature", json!({ "name": "Rig" }));
    run(&mut doc, "edit.deselectAll", json!({}));
    run(&mut doc, "object.parentArmature", json!({ "objects": ["Cube"], "armature": "Rig", "weights": "empty" }));
}

#[test]
fn gates_pose_commands_naming_bones_run_without_a_bone_selection() {
    let mut doc = new_doc();
    run(&mut doc, "object.addArmature", json!({ "name": "Rig" }));
    run(&mut doc, "mode.pose", json!({ "on": true }));
    assert!(doc.session().rig.as_ref().unwrap().selected.is_empty());
    run(&mut doc, "pose.insertKey", json!({ "bones": ["Bone"] }));
    run(&mut doc, "pose.clear", json!({ "all": true }));
    run(&mut doc, "pose.deleteKey", json!({ "bones": ["Bone"] }));
}

#[test]
fn gates_still_refuse_with_nothing_named() {
    let mut doc = new_doc();
    run(&mut doc, "edit.deselectAll", json!({}));
    let error = refused(&mut doc, "object.translate", json!({ "offset": [1, 0, 0] }));
    assert!(error.starts_with("not-allowed"), "{error}");
    run(&mut doc, "mode.set", json!({ "mode": "edit", "object": "Cube" }));
    let error = refused(&mut doc, "object.translate", json!({ "objects": ["Cube"], "offset": [1, 0, 0] }));
    assert!(error.starts_with("not-allowed"), "objects move in object mode: {error}");
}

// --- issue-digit-names --------------------------------------------------------------------

#[test]
fn digit_names_are_ids_only_of_their_own_kind() {
    let mut doc = new_doc();
    let cube = id_of(&doc, "Cube");
    // A material named after the cube's id is found by its name.
    run(&mut doc, "material.new", json!({ "name": cube.to_string() }));
    run(&mut doc, "material.set", json!({ "material": cube.to_string(), "roughness": 0.25 }));
    let material = doc.scene().materials.iter().find(|material| material.name == cube.to_string()).unwrap();
    assert!((material.roughness - 0.25).abs() < 1e-6);
    // A vertex group named after the cube's id is a name, never an id.
    let painted = run(&mut doc, "paint.vertices", json!({ "object": "Cube", "group": cube.to_string(), "vertices": [0] }));
    assert_eq!(painted["group"], cube.to_string());
    assert!(group_names(&doc, "Cube").contains(&cube.to_string()));
    // An object's id as digits is still that object.
    run(&mut doc, "object.translate", json!({ "objects": [cube.to_string()], "offset": [0, 0, 1] }));
}

// --- issue-named-object-modes -------------------------------------------------------------

#[test]
fn modes_a_refused_curve_set_leaves_the_selection_as_it_was() {
    let mut doc = new_doc();
    run(&mut doc, "object.addCurveBezier", json!({ "name": "Path" }));
    let (selected, active) = (doc.session().editor.selected.clone(), doc.session().editor.active);
    assert!(doc.execute("curve.set", json!({ "object": "Cube", "resolution": 4 })).is_err());
    assert_eq!(
        (doc.session().editor.selected.clone(), doc.session().editor.active),
        (selected.clone(), active),
        "a refused curve.set changes no selection"
    );
    assert!(doc.execute("lattice.set", json!({ "object": "Cube", "resolution": 3 })).is_err());
    assert_eq!(
        (doc.session().editor.selected.clone(), doc.session().editor.active),
        (selected, active),
        "a refused lattice.set changes no selection"
    );
}

#[test]
fn modes_lattice_set_on_another_lattice_keeps_the_edited_selection() {
    let mut doc = new_doc();
    run(&mut doc, "object.addLattice", json!({ "name": "B" }));
    run(&mut doc, "object.addLattice", json!({ "name": "A", "resolution": 4 }));
    run(&mut doc, "mode.shape", json!({}));
    run(&mut doc, "edit.selectAll", json!({}));
    let selected = |doc: &SceneDocument| {
        let mode = doc.inspect().unwrap()["mode"].clone();
        assert_eq!(mode["mode"], "latticeEdit");
        mode["points"].as_array().unwrap().len()
    };
    assert_eq!(selected(&doc), 64);
    run(&mut doc, "lattice.set", json!({ "object": "B", "resolution": 2 }));
    assert_eq!(selected(&doc), 64, "A's selection stays whole");
}

#[test]
fn modes_particles_and_remesh_naming_an_object_wait_for_object_mode() {
    let mut doc = new_doc();
    run(&mut doc, "mode.set", json!({ "mode": "edit", "object": "Cube" }));
    let error = refused(&mut doc, "particle.add", json!({ "object": "Cube" }));
    assert!(error.starts_with("not-allowed"), "{error}");
    assert!(doc.scene().object_named("Cube").unwrap().particles.is_empty());
    let error = refused(&mut doc, "sculpt.remesh", json!({ "object": "Cube" }));
    assert!(error.starts_with("not-allowed"), "{error}");
    assert_eq!(mesh_of(&doc, "Cube").positions.len(), 8, "the edited mesh is not remeshed");
}
