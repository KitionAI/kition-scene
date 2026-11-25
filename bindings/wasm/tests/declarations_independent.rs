//! The independent reading of every write command (task card C8-scene-read): for each of the 257
//! commands the kit offers that change the scene, its handler and the engine operation behind it
//! were read, before the declarations (`src/place.rs`, `src/acts.rs`, `src/reads.json`, the object
//! parameter tables in `src/params.rs`) were looked at, for which parameters it resolves and how,
//! what it reads of the scene and of the editor, and what it writes. Each row is one code path of a
//! command on [`fixture`], written from that reading, with what a write with `basis` should hold:
//! the editor state the path reads (`selection`: the selection or the active object, `time`:
//! the current frame, `mode`: the mode and what it selects) and, for each object, the fields held
//! (`*`: every field, `-`: none, only that it is there) and the positions held (`*`: where it is
//! matters, `-`: it does not). A command declares the editor state it reads once: the union of its
//! rows'. What a write holds is computed as the guard does ([`held`]): the
//! command's target parameter and the objects its declared object parameters name (by id, and
//! by name where the command declares names), each on every field, then replaced by `under`'s
//! premises, and the objects a place names.
//!
//! The rules the rows follow (what a write rests on):
//! - what decides what the call acts on: the object a parameter, the selection or the active
//!   object names (its existence, and its `name` when named by name), the item an index names (a
//!   modifier's path `modifiers.1`; a mesh's `~topology` for element indexes);
//! - the values the call reads to compute what it writes: the fields read, the positions when it
//!   reads where the object is (in world space: its own and every ancestor's, with `constraints`);
//!   another object used as an operand, a parent, a target, a material, a node group;
//! - the field it replaces whole (a lost update otherwise): `name` for a rename;
//! - not: what only refuses the call (the scene's validation, a kind check), what only makes a new
//!   name unique, what only shapes the receipt, and the drivers every commit re-evaluates.
//!
//! Where the declarations before this card (C8-land-12) held otherwise, the row says what (`was`:
//! object fields/positions). Of the 257 commands, 190 differed (74%): 166 held less than the
//! handler reads (a write that can land on what the agent did not observe), 135 held more (a
//! refusal no change needs), 111 both. By class (a command may be in several):
//! - the selection or the active object a command acts on when its parameter is left out, not
//!   held: 132; the mode's session object or another default the handler takes (the scene's
//!   active keying set, the scene camera, the current frame's picture): 15;
//! - a read that is not held: the scene's own fields (camera, timeline, keying sets, actions,
//!   render, world, physics): 39; an ancestor's place where the result projects through the world:
//!   15; a field of the object itself (its constraints, a modifier by index, the stack): 11;
//! - a value naming another object, not held (an object parameter no table marked, `anim.*`'s
//!   `objects`; the material a colour key goes to through the slot; a parent by default): 20;
//! - editor state read but not declared: the UV projections' mode and selection (5), the current
//!   frame of imports, particle conversion, matte picks and the sculpt picture commands (8);
//! - held more: every field of an object whose few fields the call reads, 130; where it is, for a
//!   call that does not depend on it, 116; fields the call does not read (a whole stack for one
//!   item, the mesh for its topology), 9.
//!
//! Rows normalized or pinned after review say so; the editor state the code scan finds on paths
//! the readings did not count is [`SCANNED`].
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::{BTreeMap, BTreeSet};

use kition_kit::{KitCommand, KitDocument};
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn id(answer: &Value, key: &str) -> u64 {
    answer[key].as_u64().unwrap_or_else(|| panic!("no {key} in {answer}"))
}

/// The scene the rows run on, by role:
/// - `$Cube`: the default cube, material `$Mat`, modifiers Bevel (0) and Solidify (1), selected and active;
/// - `$Ball`: a UV sphere at x 3, material `$Glass`;
/// - `$Rig`: an empty at z 2, parent of `$Child`, a cube at z 3;
/// - `$Light`, `$Camera` (the scene's camera), `$Arm` (an armature), `$Path` (a Bezier curve),
///   `$Label` (a text), `$Cage` (a lattice);
/// - `$G`: a geometry node group with its input and output nodes `$GIn`, `$GOut`; `$Comp`: the
///   compositor's group, its Render Layers node `$CompIn`;
/// - `scene`: the scene itself (its `objects` list, `world`, `render`, `camera`, `timeline`,
///   `physics`, `actions`, `keyingSets`).
pub fn fixture() -> (SceneDocument, Vec<(&'static str, u64)>) {
    let mut doc = SceneDocument::create("Scene.kscene", 320, 180).unwrap();
    let named = |doc: &SceneDocument, name: &str| -> u64 {
        doc.inspect().unwrap()["objects"].as_array().unwrap().iter().find(|o| o["name"] == name).unwrap()["id"].as_u64().unwrap()
    };
    let cube = named(&doc, "Cube");
    let light = named(&doc, "Light");
    let camera = named(&doc, "Camera");
    let mut ex = |c: &str, p: Value| doc.execute(c, p.clone()).unwrap_or_else(|e| panic!("{c} {p}: {e}"));
    let ball = id(&ex("object.addUvSphere", json!({ "name": "Ball", "location": [3, 0, 0] })), "object");
    let rig = id(&ex("object.addEmpty", json!({ "name": "Rig", "location": [0, 0, 2] })), "object");
    let child = id(&ex("object.addCube", json!({ "name": "Child", "location": [0, 0, 3], "size": 0.5 })), "object");
    ex("object.parent", json!({ "objects": [child], "parent": rig }));
    let arm = id(&ex("object.addArmature", json!({ "name": "Arm", "location": [-3, 0, 0] })), "object");
    let path = id(&ex("object.addCurveBezier", json!({ "name": "Path", "location": [0, 3, 0] })), "object");
    let label = id(&ex("object.addText", json!({ "name": "Label", "location": [0, -3, 0] })), "object");
    let cage = id(&ex("object.addLattice", json!({ "name": "Cage", "location": [0, 0, -3] })), "object");
    let mat = id(&ex("material.new", json!({ "name": "Mat", "assign": false })), "material");
    let glass = id(&ex("material.new", json!({ "name": "Glass", "assign": false })), "material");
    ex("material.assign", json!({ "material": mat, "objects": [cube] }));
    ex("material.assign", json!({ "material": glass, "objects": [ball] }));
    ex("modifier.add", json!({ "object": cube, "type": "bevel" }));
    ex("modifier.add", json!({ "object": cube, "type": "solidify" }));
    let made = ex("node.newGroup", json!({ "name": "G" }));
    let (g, g_in, g_out) = (id(&made, "group"), id(&made, "input"), id(&made, "output"));
    let made = ex("compositor.new", json!({}));
    let (comp, comp_in) = (id(&made, "group"), id(&made, "renderLayers"));
    ex("select.objects", json!({ "objects": [cube] }));
    let roles = vec![
        ("$Cube", cube),
        ("$Ball", ball),
        ("$Rig", rig),
        ("$Child", child),
        ("$Light", light),
        ("$Camera", camera),
        ("$Arm", arm),
        ("$Path", path),
        ("$Label", label),
        ("$Cage", cage),
        ("$Mat", mat),
        ("$Glass", glass),
        ("$G", g),
        ("$GIn", g_in),
        ("$GOut", g_out),
        ("$Comp", comp),
        ("$CompIn", comp_in),
    ];
    (doc, roles)
}

/// [`fixture`] with driven values (C8-scene-engine 4b): `$Child`'s location z follows `$Ball`'s world
/// location x, and `$Light`'s power `$Ball`'s location x (its own value); `$Ball` keyed at frame 1.
fn driven() -> (SceneDocument, Vec<(&'static str, u64)>) {
    let (mut doc, roles) = fixture();
    let ball = roles.iter().find(|(role, _)| *role == "$Ball").unwrap().1;
    for (command, params) in [
        ("animation.insertKey", json!({ "objects": [ball], "properties": ["location"], "frame": 1 })),
        (
            "driver.add",
            json!({ "object": "Child", "property": "location", "index": 2, "expression": "b",
                    "variables": [{ "name": "b", "type": "transform", "object": ball, "channel": "locationX", "space": "world" }] }),
        ),
        (
            "driver.add",
            json!({ "object": "Light", "property": "power", "expression": "b * 100",
                    "variables": [{ "name": "b", "type": "singleProperty", "object": ball, "property": "location" }] }),
        ),
    ] {
        doc.execute(command, params.clone()).unwrap_or_else(|e| panic!("{command} {params}: {e}"));
    }
    (doc, roles)
}

/// Rows on [`driven`] (C8-scene-engine 4b, pinned after review: the readings did not hold the drivers
/// every commit re-evaluates). A call that reads a driven value rests on the driver's variable
/// sources: their value (a world place: its own, its constraints, every ancestor's) and, when the
/// call evaluates the driven object at another frame (it reads its `animation`), their keys.
const DRIVEN: &[Row] = &[
    // The driven location is what the commit gives `$Child` after the move.
    r("object.translate", r##"{"objects": ["$Child"], "offset": [0, 0, 1]}"##, "", &[("$Child", "-", "*"), ("$Ball", "constraints", "*")]),
    // A rename reads no driven value.
    r("object.rename", r##"{"object": "$Child", "name": "Kid"}"##, "", &[("$Child", "name", "-")]),
    // The power the commit gives the light follows `$Ball`'s own location.
    r("light.set", r##"{"object": "$Light", "power": 500}"##, "", &[("$Light", "data", "-"), ("$Ball", "-", "*")]),
    // The landing on `$Child` at frame 19: where `$Ball`'s keys put it then.
    r(
        "object.hop",
        r##"{"objects": ["$Cube"], "to": "$Child", "start": 1}"##,
        "",
        &[
            ("$Cube", "animation,constraints,hidden,mesh,modifiers,nla,tweak", "*"),
            ("$Child", "animation,constraints,hidden,mesh,modifiers,nla", "*"),
            ("$Rig", "constraints", "*"),
            ("$Ball", "animation,constraints,nla", "*"),
            ("scene", "timeline", "-"),
        ],
    ),
];

/// [`fixture`] with `steps` run on it (roles in place of ids): a mode's session entered.
fn session(steps: &[(&str, Value)]) -> (SceneDocument, Vec<(&'static str, u64)>) {
    let (mut doc, roles) = fixture();
    for (command, params) in steps {
        let params = with_roles(params, &roles);
        doc.execute(command, params.clone()).unwrap_or_else(|e| panic!("{command} {params}: {e}"));
    }
    (doc, roles)
}

/// `$Cube` in edit mode, every vertex selected.
fn edit_mode() -> (SceneDocument, Vec<(&'static str, u64)>) {
    session(&[("mode.set", json!({ "mode": "edit", "object": "$Cube" }))])
}

/// `$Arm` in armature edit mode, its bone `Bone` selected.
fn armature_edit() -> (SceneDocument, Vec<(&'static str, u64)>) {
    session(&[("mode.armature", json!({ "object": "$Arm" })), ("bone.select", json!({ "bones": ["Bone"] }))])
}

/// `$Arm` in pose mode, its bone `Bone` selected.
fn pose_mode() -> (SceneDocument, Vec<(&'static str, u64)>) {
    session(&[("mode.pose", json!({ "object": "$Arm" })), ("bone.select", json!({ "bones": ["Bone"] }))])
}

/// `$Cube` in weight paint mode, with a group `G1`.
fn weight_paint() -> (SceneDocument, Vec<(&'static str, u64)>) {
    session(&[("vertexGroup.add", json!({ "object": "$Cube", "name": "G1" })), ("mode.weightPaint", json!({ "object": "$Cube" }))])
}

/// `$Path` in curve edit mode, its point 0 selected.
fn curve_edit() -> (SceneDocument, Vec<(&'static str, u64)>) {
    session(&[("select.objects", json!({ "objects": ["$Path"] })), ("mode.shape", json!({})), ("curve.select", json!({ "points": [0] }))])
}

/// `$Label` in text edit mode, the caret at the end.
fn text_edit() -> (SceneDocument, Vec<(&'static str, u64)>) {
    session(&[("select.objects", json!({ "objects": ["$Label"] })), ("mode.shape", json!({}))])
}

/// `$Cage` in lattice edit mode, its point 0 selected.
fn lattice_edit() -> (SceneDocument, Vec<(&'static str, u64)>) {
    session(&[("select.objects", json!({ "objects": ["$Cage"] })), ("mode.shape", json!({})), ("lattice.select", json!({ "points": [0] }))])
}

/// `$Cube`'s hair system 0 in Particle Edit.
fn particle_edit() -> (SceneDocument, Vec<(&'static str, u64)>) {
    session(&[("particle.add", json!({ "object": "$Cube", "kind": "hair", "count": 20 })), ("mode.particleEdit", json!({ "object": "$Cube" }))])
}

/// `$Cube` in sculpt mode.
fn sculpt_mode() -> (SceneDocument, Vec<(&'static str, u64)>) {
    session(&[("mode.sculpt", json!({ "object": "$Cube" }))])
}

// Rows in each mode's session (C8-scene-engine 4c): the object-mode fixture could not enter them,
// and its rows of these paths held nothing. Each row is "from the reading" (the line of its
// path), or "pinned after review" where the reading leaves the normalization open.

/// Edit mode without `elements` (reading 2, 3, 5, 7): the edited mesh and its selected elements.
/// The element edits hold the mesh's `~topology`, as their `elements` paths do (normalized after
/// review: `mesh` -> `~topology`).
const EDIT_MODE: &[Row] = &[
    r("mesh.extrude", r##"{"distance": 0.5}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("mesh.inset", r##"{"thickness": 0.1}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("mesh.bevel", r##"{"width": 0.1}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("mesh.subdivide", r##"{}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("mesh.loopCut", r##"{"edge": [0, 1]}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("mesh.merge", r##"{}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("mesh.deleteFaces", r##"{}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("mesh.deleteVertices", r##"{}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("mesh.fill", r##"{}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("mesh.flip", r##"{}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("mesh.translate", r##"{"offset": [0, 0, 1]}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("mesh.scale", r##"{"factor": 2}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("mesh.extrudeIndividual", r##"{"distance": 0.5}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("mesh.dissolveVertices", r##"{}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("mesh.dissolveEdges", r##"{}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("mesh.dissolveFaces", r##"{}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("mesh.bridge", r##"{}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("mesh.bisect", r##"{}"##, "mode", &[("$Cube", "~topology", "-")]),
    // separate copies the mesh's place, material and modifiers, and its name for the new one.
    r("mesh.separate", r##"{}"##, "mode", &[("$Cube", "material,modifiers,name,~topology", "*")]),
    r("mesh.symmetrize", r##"{"axis": "x"}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("mesh.spin", r##"{"steps": 4}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("mesh.bevelVertices", r##"{}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("mesh.rotate", r##"{"axis": "z", "angle": 45}"##, "mode", &[("$Cube", "~topology", "-")]),
    // knife: pixel points through the view and the mesh's world place.
    r("mesh.knife", r##"{"points": [[150, 60], [170, 120]]}"##, "mode", &[("$Cube", "constraints,~topology", "*")]),
    r("mesh.edgeSlide", r##"{"factor": 0.5}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("mesh.markSeam", r##"{}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("mesh.clearSeam", r##"{}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("uv.unwrap", r##"{}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("uv.stitch", r##"{}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("uv.pin", r##"{}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("uv.unpin", r##"{}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("uv.translate", r##"{"offset": [0.1, 0]}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("uv.rotate", r##"{"angle": 90}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("uv.scale", r##"{"factor": 2}"##, "mode", &[("$Cube", "~topology", "-")]),
    // The projections read the faces' positions; reset only each face's corner count; clear nothing.
    r("uv.smartProject", r##"{}"##, "mode", &[("$Cube", "mesh", "-")]),
    r("uv.cubeProject", r##"{}"##, "mode", &[("$Cube", "mesh", "-")]),
    r("uv.reset", r##"{}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("uv.clear", r##"{}"##, "mode", &[("$Cube", "-", "-")]),
    // Shading and normals: the edited mesh's blob (`objects` ignored).
    r("object.shadeSmooth", r##"{}"##, "mode", &[("$Cube", "mesh", "-")]),
    r("mesh.recalculateNormals", r##"{}"##, "mode", &[("$Cube", "mesh", "-")]),
    r("mesh.setGeometry", r##"{"positions": [[0, 0, 0], [1, 0, 0], [0, 1, 0]], "faces": [[0, 1, 2]]}"##, "mode", &[("$Cube", "mesh", "-")]),
    // modifier.add: the edited mesh (`object` ignored); its modifiers' names only made unique.
    r("modifier.add", r##"{"type": "bevel"}"##, "mode", &[("$Cube", "-", "-")]),
    // The edit selection's vertices by index.
    r("vertexGroup.assign", r##"{"name": "G1", "weight": 0.5}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("vertexGroup.unassign", r##"{"name": "G1"}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("shapeKey.offset", r##"{"name": "K", "offset": [0, 0, 1]}"##, "mode", &[("$Cube", "mesh", "-")]),
    // A primitive joins the edited mesh, placed through its world transform.
    r("object.addPlane", r##"{"size": 2}"##, "mode", &[("$Cube", "constraints,mesh", "*")]),
    // material.new assigns the new material to the edited mesh.
    r("material.new", r##"{"name": "M2"}"##, "mode", &[("$Cube", "material", "-")]),
    // delete and duplicate the selected elements (pinned after review: `mesh` -> `~topology`, as
    // the element edits).
    r("edit.delete", r##"{}"##, "mode", &[("$Cube", "~topology", "-")]),
    r("edit.duplicate", r##"{}"##, "mode", &[("$Cube", "~topology", "-")]),
];

/// Armature edit mode (reading 5): the session's armature's bones.
const ARMATURE_EDIT: &[Row] = &[
    r("bone.add", r##"{"name": "B2", "head": [0, 0, 1], "tail": [0, 0, 2]}"##, "mode", &[("$Arm", "data", "-")]),
    r("bone.extrude", r##"{"length": 0.5}"##, "mode", &[("$Arm", "data", "-")]),
    // bone.set: the session's armature (`object` ignored), its active bone.
    r("bone.set", r##"{"roll": 10}"##, "mode", &[("$Arm", "data", "-")]),
    // delete: the selected bones, and the channels of the deleted bones pruned (reading 6).
    r("edit.delete", r##"{}"##, "mode", &[("$Arm", "animation,data", "-")]),
    // duplicate: copies of the selected bones (reading 5).
    r("edit.duplicate", r##"{}"##, "mode", &[("$Arm", "data", "-")]),
];

/// Pose mode (reading 5): the session's armature; keys by the tweak-mapped current frame.
const POSE_MODE: &[Row] = &[
    r("pose.set", r##"{"bones": ["Bone"], "rotation": [0, 0, 45]}"##, "mode", &[("$Arm", "data", "-")]),
    r("pose.set", r##"{"rotation": [0, 0, 45]}"##, "mode", &[("$Arm", "data", "-")]),
    r("pose.clear", r##"{"all": true}"##, "mode", &[("$Arm", "data", "-")]),
    // insertKey: the bones' pose values keyed into the armature's channels at the tweak-mapped frame.
    r("pose.insertKey", r##"{"properties": ["rotation"]}"##, "mode,time", &[("$Arm", "animation,data,nla,tweak", "-")]),
    // No properties: the scene's active keying set when it has one.
    r("pose.insertKey", r##"{}"##, "mode,time", &[("$Arm", "animation,data,nla,tweak", "-"), ("scene", "keyingSets", "-")]),
    r("pose.deleteKey", r##"{}"##, "mode,time", &[("$Arm", "animation,nla,tweak", "-")]),
    r("bone.set", r##"{"roll": 10}"##, "mode", &[("$Arm", "data", "-")]),
    // A constraint on the active bone (in the armature's data); the target only has to exist.
    r("constraint.add", r##"{"type": "copyLocation", "target": "$Ball"}"##, "mode", &[("$Arm", "data", "-"), ("$Ball", "-", "-")]),
];

/// Weight paint (reading 5): the session's mesh, where it is in the view, its evaluated mesh.
const WEIGHT_PAINT: &[Row] = &[
    r(
        "paint.gradient",
        r##"{"from": [10, 10], "to": [100, 100], "kind": "linear", "group": "G1"}"##,
        "mode",
        &[("$Cube", "constraints,mesh,modifiers", "*")],
    ),
    r("paint.stroke", r##"{"points": [[10, 10], [50, 50]], "group": "G1"}"##, "mode", &[("$Cube", "constraints,mesh,modifiers", "*")]),
    r("paint.normalizeAll", r##"{}"##, "mode", &[("$Cube", "mesh", "-")]),
    r("paint.vertices", r##"{"vertices": [0], "weight": 1}"##, "mode", &[("$Cube", "~topology", "-")]),
];

/// Curve point edit (reading 6): the session's curve and its selected points.
const CURVE_EDIT: &[Row] = &[
    r("curve.translate", r##"{"offset": [0, 0, 1]}"##, "mode", &[("$Path", "data", "-")]),
    r("curve.extrude", r##"{}"##, "mode", &[("$Path", "data", "-")]),
    r("curve.subdivide", r##"{}"##, "mode", &[("$Path", "data", "-")]),
    r("curve.toggleCyclic", r##"{}"##, "mode", &[("$Path", "data", "-")]),
    r("curve.setHandleType", r##"{"type": "vector"}"##, "mode", &[("$Path", "data", "-")]),
    r("curve.deletePoints", r##"{}"##, "mode", &[("$Path", "data", "-")]),
    r("curve.set", r##"{"resolution": 6}"##, "mode", &[("$Path", "data", "-")]),
    r("curve.setSplines", r##"{"splines": [{"kind": "poly", "points": [[0, 0, 0], [1, 0, 0]]}]}"##, "mode", &[("$Path", "data", "-")]),
    // delete: the selected points (`objects` ignored).
    r("edit.delete", r##"{}"##, "mode", &[("$Path", "data", "-")]),
];

/// Text edit (reading 6): the session's text and its caret.
const TEXT_EDIT: &[Row] = &[
    r("text.delete", r##"{"count": 1}"##, "mode", &[("$Label", "data", "-")]),
    r("text.insert", r##"{"text": "x"}"##, "mode", &[("$Label", "data", "-")]),
    r("text.set", r##"{"body": "Other"}"##, "mode", &[("$Label", "data", "-")]),
    // delete: one letter after the caret.
    r("edit.delete", r##"{}"##, "mode", &[("$Label", "data", "-")]),
];

/// Lattice edit (reading 6): the session's lattice and its selected points.
const LATTICE_EDIT: &[Row] = &[
    r("lattice.set", r##"{"interpolation": "linear"}"##, "mode", &[("$Cage", "data", "-")]),
    r("lattice.movePoints", r##"{"offset": [0, 0, 1]}"##, "mode", &[("$Cage", "data", "-")]),
    // delete does nothing in lattice edit (it answers success): it reads only that the session is on.
    r("edit.delete", r##"{}"##, "mode", &[]),
];

/// Particle Edit (reading 6): the session's emitter, its world and evaluated mesh, the groom.
const PARTICLE_EDIT: &[Row] = &[
    r("particleEdit.stroke", r##"{"points": [[100, 100], [120, 100]]}"##, "mode", &[("$Cube", "constraints,hair,mesh,modifiers,particles.0", "*")]),
    r("particleEdit.reset", r##"{}"##, "mode", &[("$Cube", "hair", "-")]),
];

/// Sculpt (reading 3): the session's mesh.
const SCULPT_MODE: &[Row] = &[
    r("sculpt.maskInvert", r##"{}"##, "mode", &[("$Cube", "mesh", "-")]),
    r("sculpt.remesh", r##"{}"##, "mode", &[("$Cube", "mesh", "-")]),
    r("sculpt.stroke", r##"{"points": [[0, 0, 1], [0.2, 0, 1]], "worldRadius": 0.25}"##, "mode", &[("$Cube", "constraints,mesh,modifiers", "*")]),
    r("multires.subdivide", r##"{}"##, "mode", &[("$Cube", "mesh,modifiers", "-")]),
];

/// Commands with a path the object-mode fixture cannot run that reads the mode or what it selects
/// (edit mode without `elements`: the edited mesh and its selected elements; a curve, text or
/// lattice session's object), from the readings: they add `mode` to what the command reads.
const MODE_PATHS: &[&str] = &[
    "mesh.extrude",
    "mesh.inset",
    "mesh.bevel",
    "mesh.subdivide",
    "mesh.loopCut",
    "mesh.merge",
    "mesh.deleteFaces",
    "mesh.deleteVertices",
    "mesh.fill",
    "mesh.flip",
    "mesh.translate",
    "mesh.scale",
    "mesh.extrudeIndividual",
    "mesh.dissolveVertices",
    "mesh.dissolveEdges",
    "mesh.dissolveFaces",
    "mesh.bridge",
    "mesh.bisect",
    "mesh.separate",
    "mesh.symmetrize",
    "mesh.spin",
    "mesh.bevelVertices",
    "mesh.rotate",
    "mesh.knife",
    "mesh.edgeSlide",
    "mesh.markSeam",
    "mesh.clearSeam",
    "uv.unwrap",
    "uv.stitch",
    "uv.pin",
    "uv.unpin",
    "uv.translate",
    "uv.rotate",
    "uv.scale",
    "uv.smartProject",
    "uv.cubeProject",
    "uv.cylinderProject",
    "uv.sphereProject",
    "uv.planarProject",
    "uv.reset",
    "uv.clear",
    "curve.set",
    "curve.setSplines",
    "text.set",
    "lattice.set",
    "modifier.add",
    "vertexGroup.assign",
    "vertexGroup.unassign",
    "shapeKey.offset",
];

/// Editor state the readings did not count and the code scan (`tests/acts.rs`) finds, pinned after
/// review: the current frame the scene is evaluated at again after the change (`action.assign`, the
/// drivers, the graph's key edits) or that a default takes (`camera.handheld` shares
/// `camera.shake`'s `frame`); the mode the families' gate reads (the curve, text and lattice
/// family refuses its commands in an edit session); the active object a sculpt command takes when
/// no sculpt session names one ("the sculpted or active one when left out"); one state the scan
/// finds on a path the call never takes (`sculpt.showAllFaces`' time).
const SCANNED: &[(&str, &str)] = &[
    ("action.assign", "time"),
    ("driver.add", "time"),
    ("driver.set", "time"),
    ("camera.handheld", "time"),
    ("object.addCurveBezier", "mode"),
    ("object.addCurveCircle", "mode"),
    ("object.addCurvePath", "mode"),
    ("object.addText", "mode"),
    ("object.addLattice", "mode"),
    ("object.convertToMesh", "mode"),
    ("sculpt.stroke", "selection"),
    ("sculpt.gesture", "selection"),
    ("sculpt.faceSetVisibility", "selection"),
    ("sculpt.hideFaceSet", "selection"),
    ("sculpt.isolateFaceSet", "selection"),
    ("sculpt.showAllFaces", "selection"),
    // The scan follows `visibility_command` as a whole, whose face-set lookup draws the picture;
    // showAll never reaches it at run time (commands_sculpt2.rs:422): declared, not read.
    ("sculpt.showAllFaces", "time"),
];

/// One code path of a command: its parameters (roles in place of ids), the editor state it reads
/// (sorted, comma-separated), and what a write with `basis` holds of each object (role, fields,
/// positions).
struct Row {
    command: &'static str,
    params: &'static str,
    editor: &'static str,
    holds: &'static [(&'static str, &'static str, &'static str)],
}

const fn r(command: &'static str, params: &'static str, editor: &'static str, holds: &'static [(&'static str, &'static str, &'static str)]) -> Row {
    Row { command, params, editor, holds }
}

fn with_roles(value: &Value, roles: &[(&str, u64)]) -> Value {
    match value {
        Value::String(text) => roles.iter().find(|(role, _)| role == text).map_or_else(|| value.clone(), |(_, id)| json!(id)),
        Value::Array(items) => Value::Array(items.iter().map(|item| with_roles(item, roles)).collect()),
        Value::Object(fields) => Value::Object(fields.iter().map(|(key, item)| (key.clone(), with_roles(item, roles))).collect()),
        _ => value.clone(),
    }
}

/// The values a declared object parameter (`a.b`, `items[].c`) holds in `params`.
fn values_at<'a>(params: &'a Value, path: &str) -> Vec<&'a Value> {
    let mut current = vec![params];
    for part in path.split('.') {
        let key = part.trim_end_matches("[]");
        let mut next = Vec::new();
        for value in current {
            match value.get(key) {
                Some(Value::Array(items)) if part.ends_with("[]") => next.extend(items.iter()),
                Some(item) => next.push(item),
                None => {}
            }
        }
        current = next;
    }
    current.into_iter().flat_map(|value| if let Value::Array(items) = value { items.iter().collect() } else { vec![value] }).collect()
}

/// The object a name names, of the types a parameter declares.
fn by_name(doc: &SceneDocument, name: &str, types: &[&str]) -> Option<u64> {
    let scene = doc.scene();
    let object = types.contains(&"Object").then(|| scene.object_named(name).map(|object| u64::from(object.id))).flatten();
    let material = types.contains(&"Material").then(|| scene.materials.iter().find(|m| m.name == name).map(|m| u64::from(m.id))).flatten();
    let group = types.contains(&"NodeGroup").then(|| scene.node_groups.iter().find(|g| g.name == name).map(|g| u64::from(g.id))).flatten();
    object.or(material).or(group)
}

fn spelled(set: Option<&Vec<String>>) -> String {
    let Some(items) = set else { return "*".to_owned() };
    let set: BTreeSet<&String> = items.iter().collect();
    if set.is_empty() { "-".to_owned() } else { set.into_iter().cloned().collect::<Vec<_>>().join(",") }
}

/// What a write with `basis` of `command` with `params` holds, as the guard computes it: the editor
/// state the command declares, and for each object its fields and positions.
fn held(doc: &SceneDocument, command: &KitCommand, params: &Value, names: &BTreeMap<u64, &str>) -> (String, BTreeMap<String, (String, String)>) {
    let mut editor: Vec<&str> = command.reads.iter().map(|state| state.as_str()).collect();
    if kition_scene_kit::acts::reads_mode(&command.id) {
        editor.push("mode");
    }
    editor.sort_unstable();
    let name = |id: u64| names.get(&id).map_or_else(|| id.to_string(), |name| (*name).to_owned());
    let mut holds: BTreeMap<String, (String, String)> = BTreeMap::new();
    let types_of = |path: &str| -> Vec<&str> {
        command.object_types.iter().find(|(at, _)| *at == path).map_or_else(|| vec!["Object"], |(_, types)| types.to_vec())
    };
    let resolve = |value: &Value, path: &str, names_allowed: bool| -> Option<u64> {
        match value {
            Value::Number(number) => number.as_u64(),
            Value::String(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => text.parse().ok(),
            Value::String(text) if names_allowed => by_name(doc, text, &types_of(path)),
            _ => None,
        }
    };
    if let Some(target) = command.targets {
        for value in values_at(params, target) {
            if let Some(id) = resolve(value, target, command.name_field.is_some()) {
                holds.insert(name(id), ("*".to_owned(), if command.position_independent { "-" } else { "*" }.to_owned()));
            }
        }
    }
    for path in command.objects {
        let names_allowed = command.name_params.iter().any(|(at, _)| at == path);
        for value in values_at(params, path) {
            if let Some(id) = resolve(value, path, names_allowed) {
                holds.insert(name(id), ("*".to_owned(), "*".to_owned()));
            }
        }
    }
    if let Some(under) = doc.under(&command.id, params).unwrap_or_else(|e| panic!("{}: {e}", command.id)) {
        for object in &under.objects {
            if let Some(id) = object.as_u64() {
                holds.entry(name(id)).or_insert_with(|| ("*".to_owned(), "*".to_owned()));
            }
        }
        for premise in &under.premises {
            let object = premise.id.as_u64().map_or_else(|| premise.id.as_str().unwrap_or_default().to_owned(), name);
            holds.insert(object, (spelled(premise.fields.as_ref()), spelled(premise.positions.as_ref())));
        }
    }
    (editor.join(","), holds)
}

/// A reading's fields of `object` in the replica's shape (schema version 1): an armature's bones
/// and a text's body are records and text beside its `data`, so reading the data reads them too.
fn with_held_fields(doc: &SceneDocument, roles: &[(&str, u64)], object: &str, fields: &str) -> String {
    if !fields.split(',').any(|field| field == "data") {
        return fields.to_owned();
    }
    let Some(id) = roles.iter().find(|(role, _)| *role == object).and_then(|(_, id)| u32::try_from(*id).ok()) else { return fields.to_owned() };
    let more = match doc.scene().object(id).map(|object| &object.data) {
        Ok(kition_scene_model::ObjectData::Armature(_)) => "bones",
        Ok(kition_scene_model::ObjectData::Text(_)) => "text",
        _ => return fields.to_owned(),
    };
    let mut all: BTreeSet<&str> = fields.split(',').collect();
    all.insert(more);
    all.into_iter().collect::<Vec<_>>().join(",")
}

/// Whether `held` (fields, positions) holds at least what `needed` does, as the guard reads a
/// premise: `*` every field (derived values too), `-` none.
fn covers(held: &(String, String), needed: &(String, String)) -> bool {
    let fields = |text: &str| -> BTreeSet<String> { if text == "-" { BTreeSet::new() } else { text.split(',').map(str::to_owned).collect() } };
    let fields_ok = held.0 == "*" || (needed.0 != "*" && fields(&needed.0).is_subset(&fields(&held.0)));
    fields_ok && (held.1 == "*" || needed.1 == "-")
}

/// Which way `holds` differs from `expected`: (holds less somewhere, holds more somewhere).
fn ways(holds: &BTreeMap<String, (String, String)>, expected: &BTreeMap<String, (String, String)>) -> (bool, bool) {
    let none = ("-".to_owned(), "-".to_owned());
    let less = expected.iter().any(|(object, need)| holds.get(object).is_none_or(|held| !covers(held, need)));
    let more = holds.iter().any(|(object, held)| {
        expected.get(object).is_none_or(|need| held != need && covers(held, need)) || (expected.get(object).is_none() && held != &none)
    });
    let more = more || holds.iter().any(|(object, held)| expected.get(object).is_some_and(|need| !covers(need, held)));
    (less, more)
}

/// Every write command's rows hold what its reading says, and every write command has a row.
/// A scene the rows run on, by role ([`fixture`], or one built on it), and its rows.
type Fixture = (&'static str, fn() -> (SceneDocument, Vec<(&'static str, u64)>), &'static [Row]);

/// The fixtures and their rows: the object-mode [`fixture`] with [`ROWS`], then the scenes the
/// other rows need (a driven value, each mode's session).
const FIXTURES: &[Fixture] = &[
    ("object mode", fixture, ROWS),
    ("driven", driven, DRIVEN),
    ("edit mode", edit_mode, EDIT_MODE),
    ("armature edit", armature_edit, ARMATURE_EDIT),
    ("pose", pose_mode, POSE_MODE),
    ("weight paint", weight_paint, WEIGHT_PAINT),
    ("curve edit", curve_edit, CURVE_EDIT),
    ("text edit", text_edit, TEXT_EDIT),
    ("lattice edit", lattice_edit, LATTICE_EDIT),
    ("Particle Edit", particle_edit, PARTICLE_EDIT),
    ("sculpt", sculpt_mode, SCULPT_MODE),
];

#[test]
fn every_write_rests_on_what_its_handler_reads() {
    let commands = fixture().0.commands();
    let mut wrong = Vec::new();
    let mut differing = BTreeSet::new();
    let (mut less, mut more) = (BTreeSet::new(), BTreeSet::new());
    // The editor state a command reads is declared once for the command: the union of its paths'.
    let mut editors: BTreeMap<&str, (BTreeSet<&str>, String)> = BTreeMap::new();
    let mut rows: BTreeSet<&str> = BTreeSet::new();
    let mut count = 0;
    for (scene, build, table) in FIXTURES {
        let (doc, roles) = build();
        let names: BTreeMap<u64, &str> = roles.iter().map(|(role, id)| (*id, *role)).collect();
        for row in *table {
            count += 1;
            rows.insert(row.command);
            let command = commands.iter().find(|c| c.id == row.command).unwrap_or_else(|| panic!("no command {}", row.command));
            let params = with_roles(&serde_json::from_str(row.params).unwrap_or_else(|e| panic!("{} {}: {e}", row.command, row.params)), &roles);
            let (editor, holds) = held(&doc, command, &params, &names);
            let entry = editors.entry(row.command).or_insert_with(|| (BTreeSet::new(), editor));
            entry.0.extend(row.editor.split(',').filter(|state| !state.is_empty()));
            let expected: BTreeMap<String, (String, String)> = row
                .holds
                .iter()
                .map(|(object, fields, at)| ((*object).to_owned(), (with_held_fields(&doc, &roles, object, fields), (*at).to_owned())))
                .collect();
            if holds != expected {
                differing.insert(row.command);
                let (fewer, wider) = ways(&holds, &expected);
                if fewer {
                    less.insert(row.command);
                }
                if wider {
                    more.insert(row.command);
                }
                wrong.push(format!("{} {} ({scene}):\n  holds   {holds:?}\n  reading {expected:?}", row.command, row.params));
            }
        }
    }
    for command in MODE_PATHS {
        editors.get_mut(command).unwrap_or_else(|| panic!("{command}: no row")).0.insert("mode");
    }
    for (command, state) in SCANNED {
        editors.get_mut(command).unwrap_or_else(|| panic!("{command}: no row")).0.insert(state);
    }
    for (command, (read, declared)) in &editors {
        let read = read.iter().copied().collect::<Vec<_>>().join(",");
        if &read != declared {
            differing.insert(command);
            let declared: BTreeSet<&str> = declared.split(',').filter(|state| !state.is_empty()).collect();
            let read: BTreeSet<&str> = read.split(',').filter(|state| !state.is_empty()).collect();
            if !read.is_subset(&declared) {
                less.insert(command);
            }
            if !declared.is_subset(&read) {
                more.insert(command);
            }
            wrong.push(format!("{command}: reads {declared:?}, its reading {read:?}"));
        }
    }
    let writes: BTreeSet<&str> = commands.iter().filter(|c| c.mutates).map(|c| c.id.as_str()).collect();
    for missing in writes.difference(&rows) {
        wrong.push(format!("{missing}: a write command with no reading"));
    }
    for gone in rows.difference(&writes) {
        wrong.push(format!("{gone}: a reading of no write command"));
    }
    println!("{}", wrong.join("\n"));
    println!(
        "rows {}; commands differing {} of {}: holding less {}, holding more {}, both {}",
        count,
        differing.len(),
        writes.len(),
        less.len(),
        more.len(),
        less.intersection(&more).count()
    );
    println!("LESS {less:?}");
    println!("MORE {more:?}");
    assert_eq!(writes.len(), 257, "the write commands read");
    assert!(wrong.is_empty(), "{} rows differ from the readings", wrong.len());
}

const ROWS: &[Row] = &[
    // ---- Reading 1: edit.undo .. object.lookAt ----
    // edit.undo: scope param else the origin's kind; inverts that scope's last transaction in the replica (rests on the whole history).
    // (C8-scene-engine 5a: on what that transaction wrote; `tests/undo_premises.rs` checks steps of both scopes.)
    // was: {}
    // corrected by C8-scene-engine: an undo or a redo holds what the steps it may take back (or replay)
    // wrote, recorded per scope as each step is written; the agents' scope (an agent's undo with no
    // `scope`) and the person's redo have none on this fixture (bindings/wasm/src/rests.rs:727,
    // bindings/wasm/src/lib.rs:290)
    r("edit.undo", r##"{}"##, "", &[]),
    // was: {}
    // corrected by C8-scene-engine: an undo or a redo holds what the steps it may take back (or replay)
    // wrote, recorded per scope as each step is written; the agents' scope (an agent's undo with no
    // `scope`) and the person's redo have none on this fixture (bindings/wasm/src/rests.rs:727,
    // bindings/wasm/src/lib.rs:290)
    r("edit.undo", r##"{"scope": "agent"}"##, "", &[]),
    // edit.redo: same as edit.undo, replaying the scope's last undone transaction.
    // was: {}
    // corrected by C8-scene-engine: an undo or a redo holds what the steps it may take back (or replay)
    // wrote, recorded per scope as each step is written; the agents' scope (an agent's undo with no
    // `scope`) and the person's redo have none on this fixture (bindings/wasm/src/rests.rs:727,
    // bindings/wasm/src/lib.rs:290)
    r("edit.redo", r##"{}"##, "", &[]),
    // was: {}
    // corrected by C8-scene-engine: an undo or a redo holds what the steps it may take back (or replay)
    // wrote, recorded per scope as each step is written; the agents' scope (an agent's undo with no
    // `scope`) and the person's redo have none on this fixture (bindings/wasm/src/rests.rs:727,
    // bindings/wasm/src/lib.rs:290)
    r("edit.redo", r##"{"scope": "person"}"##, "", &[]),
    // object.addPlane: builds the primitive from params; the mode decides whether it becomes a new object (object mode,
    // placed by params else the origin) or joins the edited mesh (edit mode: reads the edited mesh's world transform
    // and mesh; not set up by the fixture, no row). New name only deduped, appended object holds nothing. (C8-scene-engine 4c: the session rows run on its fixture: [`FIXTURES`].)
    r("object.addPlane", r##"{"size": 2, "location": [1, 0, 0]}"##, "mode", &[]),
    r("object.addPlane", r##"{"name": "Floor"}"##, "mode", &[]),
    // object.addBackdrop: object mode only (gate); mesh from params, placement from params.
    r("object.addBackdrop", r##"{"width": 20, "height": 8}"##, "", &[]),
    // object.addCube: shares the primitive branch with object.addPlane (edit-mode path: no row).
    r("object.addCube", r##"{"size": 1, "location": [0, 0, 1]}"##, "mode", &[]),
    // object.addCircle: shares the primitive branch with object.addPlane.
    r("object.addCircle", r##"{"vertices": 16, "radius": 1, "fill": "ngon"}"##, "mode", &[]),
    // object.addUvSphere: shares the primitive branch with object.addPlane.
    r("object.addUvSphere", r##"{"segments": 16, "rings": 8}"##, "mode", &[]),
    // object.addIcoSphere: shares the primitive branch with object.addPlane.
    r("object.addIcoSphere", r##"{"subdivisions": 2}"##, "mode", &[]),
    // object.addCylinder: shares the primitive branch with object.addPlane.
    r("object.addCylinder", r##"{"vertices": 12, "depth": 2}"##, "mode", &[]),
    // object.addCone: shares the primitive branch with object.addPlane.
    r("object.addCone", r##"{"radius1": 1, "radius2": 0}"##, "mode", &[]),
    // object.addTorus: shares the primitive branch with object.addPlane.
    r("object.addTorus", r##"{"majorRadius": 1, "minorRadius": 0.25}"##, "mode", &[]),
    // object.addGrid: shares the primitive branch with object.addPlane.
    r("object.addGrid", r##"{"xSubdivisions": 4, "ySubdivisions": 4}"##, "mode", &[]),
    // object.addLight: object mode only (gate); light data from params.
    r("object.addLight", r##"{"kind": "sun", "power": 3}"##, "", &[]),
    // object.addEmpty: object mode only (gate); size from params.
    r("object.addEmpty", r##"{"size": 1, "location": [0, 0, 1]}"##, "", &[]),
    // object.addCamera: object mode only; reads scene.camera (none -> the new camera becomes the scene camera).
    // was: {}
    r("object.addCamera", r##"{"lens": 35, "location": [7, -7, 5]}"##, "", &[("scene", "camera", "-")]),
    // object.transform: objects else the selection; sets the given local location/rotation/scale (replaced, not read).
    r("object.transform", r##"{"objects": ["$Cube"], "location": [1, 2, 3]}"##, "", &[("$Cube", "-", "*")]),
    // was: {}
    r("object.transform", r##"{"location": [1, 2, 3]}"##, "selection", &[("$Cube", "-", "*")]),
    r("object.transform", r##"{"objects": ["Cube"], "scale": [2, 2, 2]}"##, "", &[("$Cube", "name", "*")]),
    // object.translate: objects else the selection (gate needs a selection even with objects: not a read); world-space
    // offset through world * local^-1, so world transform of each top-level target (ancestors, constraints).
    // was: {$Cube -/*}
    // corrected by C8-scene-engine: the offset goes through the parent frame (parent world * parentInverse), not the own constraints (bindings/wasm/src/commands.rs:2607)
    r("object.translate", r##"{"objects": ["$Cube"], "offset": [1, 0, 0]}"##, "", &[("$Cube", "-", "*")]),
    // was: {}
    // corrected by C8-scene-engine: the offset goes through the parent frame (parent world * parentInverse), not the own constraints (bindings/wasm/src/commands.rs:2607)
    r("object.translate", r##"{"offset": [1, 0, 0]}"##, "selection", &[("$Cube", "-", "*")]),
    // was: {$Cube name/*}
    // corrected by C8-scene-engine: the offset goes through the parent frame (parent world * parentInverse), not the own constraints (bindings/wasm/src/commands.rs:2607)
    r("object.translate", r##"{"objects": ["Cube"], "offset": [1, 0, 0]}"##, "", &[("$Cube", "name", "*")]),
    // pinned after review: the world offset is turned into the parent's space as the parent is when
    // the step runs, so the child moves by it wherever the parent went (outside.rs s6, C8-scene); the
    // reading held `("$Rig", "constraints", "*")`.
    // was: {$Child -/*}
    // corrected by C8-scene-engine: the offset goes through the parent frame (parent world * parentInverse), not the own constraints (bindings/wasm/src/commands.rs:2607)
    r("object.translate", r##"{"objects": ["$Child"], "offset": [1, 0, 0]}"##, "", &[("$Child", "-", "*")]),
    // object.rotate: as translate, world axis turned into the parent's axes; reads rotation, world transform.
    // was: {$Cube -/*}
    // corrected by C8-scene-engine: the axis goes through the parent frame (parent world * parentInverse), not the own constraints (bindings/wasm/src/commands.rs:2617)
    r("object.rotate", r##"{"objects": ["$Cube"], "axis": "z", "angle": 90}"##, "", &[("$Cube", "-", "*")]),
    // was: {}
    // corrected by C8-scene-engine: the axis goes through the parent frame (parent world * parentInverse), not the own constraints (bindings/wasm/src/commands.rs:2617)
    r("object.rotate", r##"{"axis": "z", "angle": 90}"##, "selection", &[("$Cube", "-", "*")]),
    // was: {$Cube name/*}
    // corrected by C8-scene-engine: the axis goes through the parent frame (parent world * parentInverse), not the own constraints (bindings/wasm/src/commands.rs:2617)
    r("object.rotate", r##"{"objects": ["Cube"], "axis": "x", "angle": 45}"##, "", &[("$Cube", "name", "*")]),
    // pinned after review: as object.translate, the world axis is turned through the parent as it is
    // when the step runs; the reading held `("$Rig", "constraints", "*")`.
    // was: {$Child -/*}
    // corrected by C8-scene-engine: the axis goes through the parent frame (parent world * parentInverse), not the own constraints (bindings/wasm/src/commands.rs:2617)
    r("object.rotate", r##"{"objects": ["$Child"], "axis": "z", "angle": 90}"##, "", &[("$Child", "-", "*")]),
    // object.scale: objects else the selection; local scale *= factor (own scale only, no world, no top-level filter).
    r("object.scale", r##"{"objects": ["$Cube"], "factor": 2}"##, "", &[("$Cube", "-", "*")]),
    // was: {}
    r("object.scale", r##"{"factor": [1, 2, 1]}"##, "selection", &[("$Cube", "-", "*")]),
    r("object.scale", r##"{"objects": ["Cube"], "factor": 2}"##, "", &[("$Cube", "name", "*")]),
    // object.applyTransform: objects else the selection; meshes only (others skipped); reads local loc/rot/scale and
    // the mesh, rewrites both (children's parentInverse too; the fixture's $Cube has none).
    // was: {$Cube */*}
    r("object.applyTransform", r##"{"objects": ["$Cube"]}"##, "", &[("$Cube", "mesh", "*")]),
    // was: {}
    r("object.applyTransform", r##"{}"##, "selection", &[("$Cube", "mesh", "*")]),
    // was: {$Cube */*}
    r("object.applyTransform", r##"{"objects": ["Cube"], "location": true}"##, "", &[("$Cube", "mesh,name", "*")]),
    // was: {$Rig */*}
    r("object.applyTransform", r##"{"objects": ["$Rig"]}"##, "", &[("$Rig", "-", "-")]),
    // object.originToGeometry: objects else the selection; meshes only; reads the base mesh's bounds and the local
    // matrix, rewrites the mesh positions and the location.
    // was: {$Cube */*}
    r("object.originToGeometry", r##"{"objects": ["$Cube"]}"##, "", &[("$Cube", "mesh", "*")]),
    // was: {}
    r("object.originToGeometry", r##"{}"##, "selection", &[("$Cube", "mesh", "*")]),
    // was: {$Cube */*}
    r("object.originToGeometry", r##"{"objects": ["Cube"]}"##, "", &[("$Cube", "mesh,name", "*")]),
    // object.order: object else the active object; moves it to an index of scene.objects.
    // pinned after review: the list the destination falls in is held by the insertion window
    // (`within`, scene.objects from..to), not as a premise on the whole list, which every insertion
    // anywhere would touch; the reading held `("scene", "objects", "-")` on each row.
    // was: {$Cube -/-}
    r("object.order", r##"{"object": "$Cube", "index": 0}"##, "", &[("$Cube", "-", "-")]),
    // was: {}
    r("object.order", r##"{"index": 2}"##, "selection", &[("$Cube", "-", "-")]),
    // was: {$Cube name/-}
    r("object.order", r##"{"object": "Cube", "index": 0}"##, "", &[("$Cube", "name", "-")]),
    // object.rename: object else the active object; replaces name (deduped only against other objects' names).
    r("object.rename", r##"{"object": "$Cube", "name": "Box"}"##, "", &[("$Cube", "name", "-")]),
    // was: {}
    r("object.rename", r##"{"name": "Box"}"##, "selection", &[("$Cube", "name", "-")]),
    r("object.rename", r##"{"object": "Cube", "name": "Box"}"##, "", &[("$Cube", "name", "-")]),
    // object.parent: parent else the active object; objects else the selection minus the parent; parentInverse =
    // inverse(world of the parent); each child's parent/parentInverse replaced (its own fields untouched).
    // was: {$Cube -/*, $Rig -/*}
    r("object.parent", r##"{"objects": ["$Cube"], "parent": "$Rig"}"##, "", &[("$Cube", "-", "*"), ("$Rig", "constraints", "*")]),
    // was: {$Ball -/*}
    r("object.parent", r##"{"objects": ["$Ball"]}"##, "selection", &[("$Ball", "-", "*"), ("$Cube", "constraints", "*")]),
    // was: {$Rig -/*}
    r("object.parent", r##"{"parent": "$Rig"}"##, "selection", &[("$Cube", "-", "*"), ("$Rig", "constraints", "*")]),
    // was: {$Cube name/*, $Rig name/*}
    r("object.parent", r##"{"objects": ["Cube"], "parent": "Rig"}"##, "", &[("$Cube", "name", "*"), ("$Rig", "constraints,name", "*")]),
    // object.clearParent: objects else the selection; local fields = decomposed world matrix (ancestors, constraints).
    // was: {$Cube -/*}
    // corrected by C8-scene-engine: Keep Transform bakes the parent frame, not the own constraints (crates/engine/model/src/scene.rs:1181)
    r("object.clearParent", r##"{"objects": ["$Cube"]}"##, "", &[("$Cube", "-", "*")]),
    // was: {}
    // corrected by C8-scene-engine: Keep Transform bakes the parent frame, not the own constraints (crates/engine/model/src/scene.rs:1181)
    r("object.clearParent", r##"{}"##, "selection", &[("$Cube", "-", "*")]),
    // was: {$Cube name/*}
    // corrected by C8-scene-engine: Keep Transform bakes the parent frame, not the own constraints (crates/engine/model/src/scene.rs:1181)
    r("object.clearParent", r##"{"objects": ["Cube"]}"##, "", &[("$Cube", "name", "*")]),
    // was: {$Child -/*}
    // corrected by C8-scene-engine: Keep Transform bakes the parent frame, not the own constraints (crates/engine/model/src/scene.rs:1181)
    r("object.clearParent", r##"{"objects": ["$Child"]}"##, "", &[("$Child", "-", "*"), ("$Rig", "constraints", "*")]),
    // object.hide: objects else the selection; sets hidden (replaced); gate refuses in sculpt / without a selection.
    r("object.hide", r##"{"objects": ["$Cube"]}"##, "", &[("$Cube", "hidden", "-")]),
    // was: {}
    r("object.hide", r##"{}"##, "selection", &[("$Cube", "hidden", "-")]),
    r("object.hide", r##"{"objects": ["Cube"]}"##, "", &[("$Cube", "hidden,name", "-")]),
    // object.reveal: objects (only the hidden ones among them) else every hidden object; reads hidden.
    r("object.reveal", r##"{"objects": ["$Cube"]}"##, "", &[("$Cube", "hidden", "-")]),
    r("object.reveal", r##"{"objects": ["Cube"]}"##, "", &[("$Cube", "hidden,name", "-")]),
    // was: {}
    r(
        "object.reveal",
        r##"{}"##,
        "",
        &[
            ("$Arm", "hidden", "-"),
            ("$Ball", "hidden", "-"),
            ("$Cage", "hidden", "-"),
            ("$Camera", "hidden", "-"),
            ("$Child", "hidden", "-"),
            ("$Cube", "hidden", "-"),
            ("$Label", "hidden", "-"),
            ("$Light", "hidden", "-"),
            ("$Path", "hidden", "-"),
            ("$Rig", "hidden", "-"),
        ],
    ),
    // object.shadeSmooth: the mode decides (edit mode: the edited mesh, objects ignored; object mode: objects else the
    // selection); writes smooth/smooth_angle into the mesh blob. Edit-mode path: no row.
    // was: {$Cube */*}
    r("object.shadeSmooth", r##"{"objects": ["$Cube"]}"##, "mode", &[("$Cube", "mesh", "-")]),
    // was: {}
    r("object.shadeSmooth", r##"{}"##, "mode,selection", &[("$Cube", "mesh", "-")]),
    // was: {$Cube */*}
    r("object.shadeSmooth", r##"{"objects": ["Cube"]}"##, "mode", &[("$Cube", "mesh,name", "-")]),
    // object.shadeSmoothByAngle: shares the shade branch with object.shadeSmooth, angle from params.
    // was: {$Cube */*}
    r("object.shadeSmoothByAngle", r##"{"objects": ["$Cube"], "angle": 30}"##, "mode", &[("$Cube", "mesh", "-")]),
    // was: {}
    r("object.shadeSmoothByAngle", r##"{"angle": 45}"##, "mode,selection", &[("$Cube", "mesh", "-")]),
    // was: {$Cube */*}
    r("object.shadeSmoothByAngle", r##"{"objects": ["Cube"]}"##, "mode", &[("$Cube", "mesh,name", "-")]),
    // object.shadeFlat: shares the shade branch with object.shadeSmooth.
    // was: {$Cube */*}
    r("object.shadeFlat", r##"{"objects": ["$Cube"]}"##, "mode", &[("$Cube", "mesh", "-")]),
    // was: {}
    r("object.shadeFlat", r##"{}"##, "mode,selection", &[("$Cube", "mesh", "-")]),
    // was: {$Cube */*}
    r("object.shadeFlat", r##"{"objects": ["Cube"]}"##, "mode", &[("$Cube", "mesh,name", "-")]),
    // object.join: no params; the active mesh is the target, the other selected meshes are joined into it (world
    // transforms of all, their meshes). In the fixture only $Cube is selected (the call would be refused).
    // was: {}
    r("object.join", r##"{}"##, "selection", &[("$Cube", "constraints,mesh", "*")]),
    // object.setCamera: object else the active object; must be a camera; replaces scene.camera.
    // was: {$Camera */*}
    r("object.setCamera", r##"{"object": "$Camera"}"##, "", &[("$Camera", "-", "-"), ("scene", "camera", "-")]),
    // was: {}
    r("object.setCamera", r##"{}"##, "selection", &[("$Cube", "-", "-"), ("scene", "camera", "-")]),
    // was: {$Camera */*}
    r("object.setCamera", r##"{"object": "Camera"}"##, "", &[("$Camera", "name", "-"), ("scene", "camera", "-")]),
    // object.lookAt: object else the active object (any kind); target is a world point; reads its world transform
    // (ancestors, constraints), rewrites location/rotation in the parent's space, keeps scale.
    // was: {$Camera */*}
    r("object.lookAt", r##"{"object": "$Camera", "target": [0, 0, 0]}"##, "", &[("$Camera", "constraints", "*")]),
    // was: {}
    r("object.lookAt", r##"{"target": [0, 0, 5]}"##, "selection", &[("$Cube", "constraints", "*")]),
    // was: {$Camera */*}
    r("object.lookAt", r##"{"object": "Camera", "target": [0, 0, 0]}"##, "", &[("$Camera", "constraints,name", "*")]),
    // was: {$Child */*}
    r("object.lookAt", r##"{"object": "$Child", "target": [1, 1, 1]}"##, "", &[("$Child", "constraints", "*"), ("$Rig", "constraints", "*")]),
    // ---- Reading 2 (27 commands). Edit-mode paths without `elements` (the edited mesh + its edit
    // selection; Mode::Edit) cannot be set up by the fixture (object mode): no rows for them (C8-scene-engine 4c: the session rows run on its fixture: [`FIXTURES`]). The
    // proportional-editing tool setting (session.proportional) is read by mesh.translate / mesh.scale
    // in every path but is none of mode/selection/time, so it is not in the editor column.

    // mesh.extrude: elements -> edit mode on elements.object (id-or-name), vertex set = given vertices + every corner of the given faces; extrudes faces_within(set) (reads positions, face normals) or, with no whole face, extrudes loose vertices/edges. Local only.
    r("mesh.extrude", r##"{"elements": {"object": "$Cube", "faces": [0]}, "distance": 0.5}"##, "", &[("$Cube", "~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    r("mesh.extrude", r##"{"elements": {"object": "Cube", "faces": [0]}, "distance": 0.5}"##, "", &[("$Cube", "name,~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    // (loose path, {"elements": {"object": "$Cube", "vertices": [0, 1]}}: same holdings, no extra row)
    // mesh.inset: elements as above; insets faces_within(set) by positions and face normals. Local.
    r("mesh.inset", r##"{"elements": {"object": "$Cube", "faces": [0]}, "thickness": 0.1}"##, "", &[("$Cube", "~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    r("mesh.inset", r##"{"elements": {"object": "Cube", "faces": [0]}, "thickness": 0.1}"##, "", &[("$Cube", "name,~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    // mesh.bevel: elements as above; bevels every mesh edge with both ends in the set (positions read). Local.
    r("mesh.bevel", r##"{"elements": {"object": "$Cube", "faces": [0]}, "width": 0.1}"##, "", &[("$Cube", "~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    r("mesh.bevel", r##"{"elements": {"object": "Cube", "faces": [0]}, "width": 0.1}"##, "", &[("$Cube", "name,~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    // mesh.subdivide: elements as above; splits faces_within(set) at edge midpoints/centres (positions read), neighbours re-cornered.
    r("mesh.subdivide", r##"{"elements": {"object": "$Cube", "faces": [0]}}"##, "", &[("$Cube", "~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    r("mesh.subdivide", r##"{"elements": {"object": "Cube", "faces": [0]}}"##, "", &[("$Cube", "name,~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    // mesh.loopCut: elements only enters edit mode (the selection is ignored); `edge` = vertex indexes; walks the quad ring, new vertices lerp the crossed edges' positions.
    r("mesh.loopCut", r##"{"elements": {"object": "$Cube", "faces": [0]}, "edge": [0, 1]}"##, "", &[("$Cube", "~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    r("mesh.loopCut", r##"{"elements": {"object": "Cube", "faces": [0]}, "edge": [0, 1]}"##, "", &[("$Cube", "name,~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    // mesh.merge: elements as above; merges the set at its centroid (positions read), reindexes.
    r("mesh.merge", r##"{"elements": {"object": "$Cube", "faces": [0]}}"##, "", &[("$Cube", "~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    r("mesh.merge", r##"{"elements": {"object": "Cube", "faces": [0]}}"##, "", &[("$Cube", "name,~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    // mesh.deleteFaces: elements as above; deletes faces_within(set) by index only (no positions read).
    r("mesh.deleteFaces", r##"{"elements": {"object": "$Cube", "faces": [0]}}"##, "", &[("$Cube", "~topology", "-")]),
    r("mesh.deleteFaces", r##"{"elements": {"object": "Cube", "faces": [0]}}"##, "", &[("$Cube", "name,~topology", "-")]),
    // mesh.deleteVertices: elements as above; deletes the set's vertices and the faces/edges using them, by index only.
    r("mesh.deleteVertices", r##"{"elements": {"object": "$Cube", "faces": [0]}}"##, "", &[("$Cube", "~topology", "-")]),
    r("mesh.deleteVertices", r##"{"elements": {"object": "Cube", "faces": [0]}}"##, "", &[("$Cube", "name,~topology", "-")]),
    // mesh.fill: elements as above; `vertices` absent -> corners = the set ordered by angle around its centroid (positions read); given -> those indexes in order (index only).
    r("mesh.fill", r##"{"elements": {"object": "$Cube", "faces": [0]}}"##, "", &[("$Cube", "~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    r("mesh.fill", r##"{"elements": {"object": "Cube", "faces": [0]}}"##, "", &[("$Cube", "name,~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    r("mesh.fill", r##"{"elements": {"object": "$Cube", "faces": [0]}, "vertices": [0, 1, 2]}"##, "", &[("$Cube", "~topology", "-")]),
    // mesh.flip: elements as above; reverses faces_within(set) by index only.
    r("mesh.flip", r##"{"elements": {"object": "$Cube", "faces": [0]}}"##, "", &[("$Cube", "~topology", "-")]),
    r("mesh.flip", r##"{"elements": {"object": "Cube", "faces": [0]}}"##, "", &[("$Cube", "name,~topology", "-")]),
    // mesh.recalculateNormals: object mode -> `objects` (id-or-name) else the selection; the mode decides between that and the edited mesh. Reads the whole mesh (windings, signed volume). Takes no `elements`. Gate: Needs::Mesh (selection/active must hold a mesh even when objects is given; refuses only).
    // was: {$Cube */*}
    r("mesh.recalculateNormals", r##"{"objects": ["$Cube"]}"##, "mode", &[("$Cube", "mesh", "-")]),
    // was: {$Cube */*}
    r("mesh.recalculateNormals", r##"{"objects": ["Cube"]}"##, "mode", &[("$Cube", "mesh,name", "-")]),
    // was: {}
    r("mesh.recalculateNormals", r##"{}"##, "mode,selection", &[("$Cube", "mesh", "-")]),
    // mesh.translate: elements as above; adds `offset` (local axes) to the set's positions by index; proportional (tool) also reads unselected positions when on (off in the fixture).
    r("mesh.translate", r##"{"elements": {"object": "$Cube", "faces": [0]}, "offset": [0, 0, 1]}"##, "", &[("$Cube", "~topology", "-")]),
    r("mesh.translate", r##"{"elements": {"object": "Cube", "faces": [0]}, "offset": [0, 0, 1]}"##, "", &[("$Cube", "name,~topology", "-")]),
    // mesh.scale: elements as above; pivot absent -> centroid of the set's local positions (positions read); pivot given -> scales relative to it by index.
    r("mesh.scale", r##"{"elements": {"object": "$Cube", "faces": [0]}, "factor": 2}"##, "", &[("$Cube", "~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    r("mesh.scale", r##"{"elements": {"object": "Cube", "faces": [0]}, "factor": 2}"##, "", &[("$Cube", "name,~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    r(
        "mesh.scale",
        r##"{"elements": {"object": "$Cube", "faces": [0]}, "factor": [1, 1, 2], "pivot": [0, 0, 0]}"##,
        "",
        &[("$Cube", "~topology", "-")],
    ),
    // mesh.setGeometry: the mode decides the target (edit mode: the edited mesh, `object` ignored); object mode -> `object` (id-or-name) else the active object. Replaces the mesh whole, keeping smooth/smoothAngle.
    // was: {$Cube */*}
    r(
        "mesh.setGeometry",
        r##"{"object": "$Cube", "positions": [[0, 0, 0], [1, 0, 0], [0, 1, 0]], "faces": [[0, 1, 2]]}"##,
        "mode",
        &[("$Cube", "mesh", "-")],
    ),
    // was: {$Cube */*}
    r(
        "mesh.setGeometry",
        r##"{"object": "Cube", "positions": [[0, 0, 0], [1, 0, 0], [0, 1, 0]], "faces": [[0, 1, 2]]}"##,
        "mode",
        &[("$Cube", "mesh,name", "-")],
    ),
    // was: {}
    r("mesh.setGeometry", r##"{"positions": [[0, 0, 0], [1, 0, 0], [0, 1, 0]], "faces": [[0, 1, 2]]}"##, "mode,selection", &[("$Cube", "mesh", "-")]),
    // modifier.remove: `object` (id-or-name, any kind) else the active object; `index` into its modifiers (wins), else the first with that `name`.
    r("modifier.remove", r##"{"object": "$Cube", "index": 0}"##, "", &[("$Cube", "modifiers.0", "-")]),
    r("modifier.remove", r##"{"object": "Cube", "index": 0}"##, "", &[("$Cube", "modifiers.0,name", "-")]),
    // was: {}
    r("modifier.remove", r##"{"index": 0}"##, "selection", &[("$Cube", "modifiers.0", "-")]),
    r("modifier.remove", r##"{"object": "$Cube", "name": "Solidify"}"##, "", &[("$Cube", "modifiers", "-")]),
    // modifier.apply: target as modifier.remove; evaluates the raw mesh through every enabled modifier 0..i, then modifier i, replaces the mesh and drops modifiers 0..=i. (Bevel/Solidify read no other object, no transform.)
    // was: {$Cube */*}
    r("modifier.apply", r##"{"object": "$Cube", "index": 0}"##, "", &[("$Cube", "mesh,modifiers.0", "-")]),
    // was: {$Cube */*}
    r("modifier.apply", r##"{"object": "$Cube", "index": 1}"##, "", &[("$Cube", "mesh,modifiers.0,modifiers.1", "-")]),
    // was: {$Cube */*}
    r("modifier.apply", r##"{"object": "Cube", "index": 0}"##, "", &[("$Cube", "mesh,modifiers.0,name", "-")]),
    // was: {}
    r("modifier.apply", r##"{"index": 0}"##, "selection", &[("$Cube", "mesh,modifiers.0", "-")]),
    // was: {$Cube */*}
    r("modifier.apply", r##"{"object": "$Cube", "name": "Solidify"}"##, "", &[("$Cube", "mesh,modifiers", "-")]),
    // modifier.move: target as modifier.remove; reorders the list (`to` checked against its length).
    r("modifier.move", r##"{"object": "$Cube", "index": 0, "to": 1}"##, "", &[("$Cube", "modifiers", "-")]),
    r("modifier.move", r##"{"object": "Cube", "index": 0, "to": 1}"##, "", &[("$Cube", "modifiers,name", "-")]),
    // was: {}
    r("modifier.move", r##"{"index": 0, "to": 1}"##, "selection", &[("$Cube", "modifiers", "-")]),
    // material.new: a new material (name made unique); assign (default true) -> the mode decides the targets: object mode the selection (geometry objects only), edit mode the edited mesh; their `material` replaced.
    // was: {}
    r("material.new", r##"{"name": "Red", "baseColor": [1, 0, 0]}"##, "mode,selection", &[("$Cube", "material", "-")]),
    r("material.new", r##"{"name": "Red", "assign": false}"##, "", &[]),
    // material.set: `material` = material id or exact name; writes the named settings into its data (replaced whole), `name` for a rename; `shader` = node-group id or name (Shader tree only: a kind check).
    r("material.set", r##"{"material": "$Mat", "roughness": 0.2}"##, "", &[("$Mat", "data", "-")]),
    r("material.set", r##"{"material": "Mat", "roughness": 0.2}"##, "", &[("$Mat", "data,name", "-")]),
    // was: {$Mat data,name/-}
    r("material.set", r##"{"material": "$Mat", "name": "Red"}"##, "", &[("$Mat", "name", "-")]),
    // was: {$G */*, $Mat data/-}
    r("material.set", r##"{"material": "$Mat", "shader": "$G"}"##, "", &[("$Mat", "data", "-"), ("$G", "-", "-")]),
    // was: {$G */*, $Mat data/-}
    r("material.set", r##"{"material": "$Mat", "shader": "G"}"##, "", &[("$Mat", "data", "-"), ("$G", "name", "-")]),
    // material.assign: `material` id or name; `objects` (id-or-name, any kind) else the selection; each object's `material` replaced.
    r("material.assign", r##"{"material": "$Glass", "objects": ["$Cube"]}"##, "", &[("$Glass", "-", "-"), ("$Cube", "material", "-")]),
    r("material.assign", r##"{"material": "Glass", "objects": ["Cube"]}"##, "", &[("$Glass", "name", "-"), ("$Cube", "material,name", "-")]),
    // was: {$Glass -/-}
    r("material.assign", r##"{"material": "$Glass"}"##, "selection", &[("$Glass", "-", "-"), ("$Cube", "material", "-")]),
    // material.clear: `objects` (id-or-name) else the selection; each `material` set to none. Gate: Needs::Selection (a non-empty selection even when objects is given; refuses only).
    r("material.clear", r##"{"objects": ["$Ball"]}"##, "", &[("$Ball", "material", "-")]),
    r("material.clear", r##"{"objects": ["Ball"]}"##, "", &[("$Ball", "material,name", "-")]),
    // was: {}
    r("material.clear", r##"{}"##, "selection", &[("$Cube", "material", "-")]),
    // material.delete: `material` id or name; its users = every object whose `material` is it (refuse without force, else released); removes it, its animation and images only it used.
    // was: {$Glass */*}
    r(
        "material.delete",
        r##"{"material": "$Glass", "force": true}"##,
        "",
        &[
            ("$Glass", "-", "-"),
            ("$Cube", "material", "-"),
            ("$Ball", "material", "-"),
            ("$Rig", "material", "-"),
            ("$Child", "material", "-"),
            ("$Light", "material", "-"),
            ("$Camera", "material", "-"),
            ("$Arm", "material", "-"),
            ("$Path", "material", "-"),
            ("$Label", "material", "-"),
            ("$Cage", "material", "-"),
        ],
    ),
    // was: {$Glass */*}
    r(
        "material.delete",
        r##"{"material": "Glass"}"##,
        "",
        &[
            ("$Glass", "name", "-"),
            ("$Cube", "material", "-"),
            ("$Ball", "material", "-"),
            ("$Rig", "material", "-"),
            ("$Child", "material", "-"),
            ("$Light", "material", "-"),
            ("$Camera", "material", "-"),
            ("$Arm", "material", "-"),
            ("$Path", "material", "-"),
            ("$Label", "material", "-"),
            ("$Cage", "material", "-"),
        ],
    ),
    // light.set: `object` (id-or-name) else the active object (must be a light: a kind check); the named settings into its data.
    r("light.set", r##"{"object": "$Light", "power": 500}"##, "", &[("$Light", "data", "-")]),
    r("light.set", r##"{"object": "Light", "power": 500}"##, "", &[("$Light", "data,name", "-")]),
    // was: {}
    r("light.set", r##"{"power": 500}"##, "selection", &[("$Cube", "data", "-")]),
    // camera.set: `object` (id-or-name) else the active object (camera: a kind check); the named settings into its data (clipEnd clamped by its clipStart); `focusObject` = another object (id-or-name, any kind) stored by id.
    r("camera.set", r##"{"object": "$Camera", "lens": 35}"##, "", &[("$Camera", "data", "-")]),
    r("camera.set", r##"{"object": "Camera", "lens": 35}"##, "", &[("$Camera", "data,name", "-")]),
    // was: {}
    r("camera.set", r##"{"lens": 35}"##, "selection", &[("$Cube", "data", "-")]),
    // was: {$Ball */*, $Camera data/-}
    r("camera.set", r##"{"object": "$Camera", "focusObject": "$Ball"}"##, "", &[("$Camera", "data", "-"), ("$Ball", "-", "-")]),
    // was: {$Ball */*, $Camera data/-}
    r("camera.set", r##"{"object": "$Camera", "focusObject": "Ball"}"##, "", &[("$Camera", "data", "-"), ("$Ball", "name", "-")]),
    // world.set: the named settings into scene.world; `volume` = node-group id or name (Shader tree only: a kind check).
    // was: {}
    r("world.set", r##"{"strength": 0.5}"##, "", &[("scene", "world", "-")]),
    // was: {$G */*}
    r("world.set", r##"{"volume": "$G"}"##, "", &[("scene", "world", "-"), ("$G", "-", "-")]),
    // was: {$G */*}
    r("world.set", r##"{"volume": "G"}"##, "", &[("scene", "world", "-"), ("$G", "name", "-")]),
    // render.set: the named settings into scene.render (the size check reads the other side).
    // was: {}
    r("render.set", r##"{"width": 640}"##, "", &[("scene", "render", "-")]),
    // file.import: appends the file's objects/materials/images; keyframes land by scene.timeline fps, objects take their values at the current frame; scene.camera set only when it has none. Gate: object mode (refuses only).
    // was: {}
    r("file.import", r##"{"path": "model.glb"}"##, "time", &[("scene", "camera,timeline", "-")]),
    // was: {}
    r("file.import", r##"{"base64": "AA==", "format": "obj"}"##, "time", &[("scene", "camera,timeline", "-")]),
    // was: {}
    r("file.import", r##"{"path": "model.blend"}"##, "time", &[("scene", "camera,timeline", "-")]),
    // ---- Reading 3: UV, UV edit, images, sculpt, multires ----
    // Correction to reading-3: only the commands_uvedit mutating commands get `elements` in their schema
    // (commands.rs:733-741). For uv.* projections, material.setTexture, modifier.setImage, sculpt.* and
    // multires.*, `elements` is refused as an unknown parameter (commands.rs:808) before with_elements runs.
    // So those have no elements row.
    //
    // uv.* projections, object mode: the `enabled` gate needs a mesh in the selection even when `objects`
    // is given. That is a gate that only refuses, so not a read. The edit-mode path (the edited mesh and
    // its edit-selection; `objects` ignored) cannot be set up by the fixture (no elements on these): no row. (C8-scene-engine 4c: the session rows run on its fixture: [`FIXTURES`].)

    // uv.smartProject: objects id-or-name list (else the selection), filtered to meshes; reads each mesh's faces/positions/uvs (local)
    // was: {$Cube */*}
    r("uv.smartProject", r##"{"objects": ["$Cube"]}"##, "", &[("$Cube", "mesh", "-")]),
    // was: {}
    r("uv.smartProject", r##"{}"##, "selection", &[("$Cube", "mesh", "-")]),
    // was: {$Cube */*}
    r("uv.smartProject", r##"{"objects": ["Cube"]}"##, "", &[("$Cube", "mesh,name", "-")]),
    // uv.cubeProject: as smartProject; face normals, local positions, whole-mesh local bounds centre
    // was: {$Cube */*}
    r("uv.cubeProject", r##"{"objects": ["$Cube"], "size": 2}"##, "", &[("$Cube", "mesh", "-")]),
    // was: {}
    r("uv.cubeProject", r##"{"size": 2}"##, "selection", &[("$Cube", "mesh", "-")]),
    // was: {$Cube */*}
    r("uv.cubeProject", r##"{"objects": ["Cube"]}"##, "", &[("$Cube", "mesh,name", "-")]),
    // uv.cylinderProject: local positions, bounds centre and z range
    // was: {$Cube */*}
    r("uv.cylinderProject", r##"{"objects": ["$Cube"]}"##, "", &[("$Cube", "mesh", "-")]),
    // was: {}
    r("uv.cylinderProject", r##"{}"##, "selection", &[("$Cube", "mesh", "-")]),
    // was: {$Cube */*}
    r("uv.cylinderProject", r##"{"objects": ["Cube"]}"##, "", &[("$Cube", "mesh,name", "-")]),
    // uv.sphereProject: local positions around the bounds centre
    // was: {$Cube */*}
    r("uv.sphereProject", r##"{"objects": ["$Cube"]}"##, "", &[("$Cube", "mesh", "-")]),
    // was: {}
    r("uv.sphereProject", r##"{}"##, "selection", &[("$Cube", "mesh", "-")]),
    // was: {$Cube */*}
    r("uv.sphereProject", r##"{"objects": ["Cube"]}"##, "", &[("$Cube", "mesh,name", "-")]),
    // uv.planarProject: local positions along the local axis param, bounds centre
    // was: {$Cube */*}
    r("uv.planarProject", r##"{"objects": ["$Cube"], "axis": "z"}"##, "", &[("$Cube", "mesh", "-")]),
    // was: {}
    r("uv.planarProject", r##"{"axis": "x"}"##, "selection", &[("$Cube", "mesh", "-")]),
    // was: {$Cube */*}
    r("uv.planarProject", r##"{"objects": ["Cube"]}"##, "", &[("$Cube", "mesh,name", "-")]),
    // uv.reset: only each face's corner count decides the coordinates
    // was: {$Cube */*}
    r("uv.reset", r##"{"objects": ["$Cube"]}"##, "", &[("$Cube", "~topology", "-")]),
    // was: {}
    r("uv.reset", r##"{}"##, "selection", &[("$Cube", "~topology", "-")]),
    // was: {$Cube */*}
    r("uv.reset", r##"{"objects": ["Cube"]}"##, "", &[("$Cube", "name,~topology", "-")]),
    // uv.clear: empties the UVs; nothing of the mesh decides the result
    // was: {$Cube */*}
    r("uv.clear", r##"{"objects": ["$Cube"]}"##, "", &[("$Cube", "-", "-")]),
    // was: {}
    r("uv.clear", r##"{}"##, "selection", &[("$Cube", "-", "-")]),
    // was: {$Cube */*}
    r("uv.clear", r##"{"objects": ["Cube"]}"##, "", &[("$Cube", "name", "-")]),
    // material.setTexture: material = id, or the first material with that name; one slot set (or cleared). The image
    // prune runs on the scene as it is at write time, so it holds nothing.
    // was: {$Mat */*}
    r("material.setTexture", r##"{"material": "$Mat", "path": "wood.png"}"##, "", &[("$Mat", "-", "-")]),
    // was: {$Mat */*}
    r("material.setTexture", r##"{"material": "Mat", "slot": "normal", "path": "wood.png"}"##, "", &[("$Mat", "name", "-")]),
    // was: {$Mat */*}
    r("material.setTexture", r##"{"material": "$Mat", "clear": true}"##, "", &[("$Mat", "-", "-")]),
    // modifier.setImage: object id-or-name (else active); modifier by index, or the first one with that name; must be Displace
    // was: {$Cube */*}
    r("modifier.setImage", r##"{"object": "$Cube", "index": 0, "path": "height.png"}"##, "", &[("$Cube", "modifiers.0", "-")]),
    // was: {}
    r("modifier.setImage", r##"{"index": 0, "clear": true}"##, "selection", &[("$Cube", "modifiers.0", "-")]),
    // was: {$Cube */*}
    r("modifier.setImage", r##"{"object": "Cube", "index": 0, "path": "height.png"}"##, "", &[("$Cube", "modifiers.0,name", "-")]),
    // was: {$Cube */*}
    r("modifier.setImage", r##"{"object": "$Cube", "name": "Bevel", "path": "height.png"}"##, "", &[("$Cube", "modifiers", "-")]),
    // UV edit commands: all need edit mode, so in the fixture only the `elements` path runs (elements.object is
    // id-or-name; the elements become the edit-selection and are put back afterwards). The edit-mode path without
    // elements reads the edited mesh and its edit-selection (mode): no row.
    // mesh.markSeam: edges with both ends among the chosen vertices; sets seams by vertex index
    r("mesh.markSeam", r##"{"elements": {"object": "$Cube", "faces": [0]}}"##, "", &[("$Cube", "~topology", "-")]),
    r("mesh.markSeam", r##"{"elements": {"object": "Cube", "faces": [0]}}"##, "", &[("$Cube", "name,~topology", "-")]),
    // mesh.clearSeam: shares the arm of markSeam
    r("mesh.clearSeam", r##"{"elements": {"object": "$Cube", "faces": [0]}}"##, "", &[("$Cube", "~topology", "-")]),
    r("mesh.clearSeam", r##"{"elements": {"object": "Cube", "faces": [0]}}"##, "", &[("$Cube", "name,~topology", "-")]),
    // uv.unwrap: faces within the chosen vertices; reads positions, faces, seams, pins, and pinned UVs
    r("uv.unwrap", r##"{"elements": {"object": "$Cube", "faces": [0]}, "method": "angleBased"}"##, "", &[("$Cube", "~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    r("uv.unwrap", r##"{"elements": {"object": "Cube"}}"##, "", &[("$Cube", "name,~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    // uv.stitch: edges among the chosen vertices; islands over the whole mesh's UVs
    r("uv.stitch", r##"{"elements": {"object": "$Cube", "faces": [0, 1]}}"##, "", &[("$Cube", "~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    r("uv.stitch", r##"{"elements": {"object": "Cube", "faces": [0, 1]}}"##, "", &[("$Cube", "name,~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    // uv.pin: pins the chosen vertex indexes
    r("uv.pin", r##"{"elements": {"object": "$Cube", "faces": [0]}}"##, "", &[("$Cube", "~topology", "-")]),
    r("uv.pin", r##"{"elements": {"object": "Cube", "faces": [0]}}"##, "", &[("$Cube", "name,~topology", "-")]),
    // uv.unpin: shares the arm of uv.pin
    r("uv.unpin", r##"{"elements": {"object": "$Cube", "faces": [0]}}"##, "", &[("$Cube", "~topology", "-")]),
    r("uv.unpin", r##"{"elements": {"object": "Cube", "faces": [0]}}"##, "", &[("$Cube", "name,~topology", "-")]),
    // uv.translate: offset added to the current UVs of the chosen corners (live unwrap re-solves charts: editor tool setting)
    r("uv.translate", r##"{"elements": {"object": "$Cube", "faces": [0]}, "offset": [0.1, 0]}"##, "", &[("$Cube", "~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    r("uv.translate", r##"{"elements": {"object": "Cube", "faces": [0]}, "offset": [0.1, 0]}"##, "", &[("$Cube", "name,~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    // uv.rotate: shares transform_uvs; pivot = the corners' mean UV unless given
    r("uv.rotate", r##"{"elements": {"object": "$Cube", "faces": [0]}, "angle": 90}"##, "", &[("$Cube", "~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    r("uv.rotate", r##"{"elements": {"object": "$Cube", "faces": [0]}, "angle": 90, "pivot": [0.5, 0.5]}"##, "", &[("$Cube", "~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    // uv.scale: shares transform_uvs
    r("uv.scale", r##"{"elements": {"object": "$Cube", "faces": [0]}, "factor": 2}"##, "", &[("$Cube", "~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    r("uv.scale", r##"{"elements": {"object": "Cube", "faces": [0]}, "factor": [2, 1]}"##, "", &[("$Cube", "name,~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    // mesh.knife: cuts the edited mesh along a screen line (view projector, editor state); world points and rays go
    // through the target's world matrix. The selection is not read.
    // was: {$Cube ~topology/*}
    r(
        "mesh.knife",
        r##"{"elements": {"object": "$Cube", "faces": [0]}, "points": [[100, 90], [220, 90]]}"##,
        "",
        &[("$Cube", "constraints,~topology", "*")],
    ), // normalized after review: `mesh` -> `~topology`
    // was: {$Cube ~topology/*}
    r(
        "mesh.knife",
        r##"{"elements": {"object": "$Cube", "faces": [0]}, "points": [[-2, 0, 0], [2, 0, 0]], "through": true}"##,
        "",
        &[("$Cube", "constraints,~topology", "*")],
    ), // normalized after review: `mesh` -> `~topology`
    // was: {$Cube name,~topology/*}
    r(
        "mesh.knife",
        r##"{"elements": {"object": "Cube", "faces": [0]}, "points": [[100, 90], [220, 90]]}"##,
        "",
        &[("$Cube", "constraints,name,~topology", "*")],
    ), // normalized after review: `mesh` -> `~topology`
    // mesh.edgeSlide: edges among the chosen vertices slide along neighbouring rails (positions, faces)
    r("mesh.edgeSlide", r##"{"elements": {"object": "$Cube", "faces": [0]}, "factor": 0.5}"##, "", &[("$Cube", "~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    r("mesh.edgeSlide", r##"{"elements": {"object": "Cube", "faces": [0]}, "factor": 0.5}"##, "", &[("$Cube", "name,~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    // Sculpt: every command but remesh needs a sculpt session. Its object (`mode`) is the target, and no parameter
    // names it. The fixture is not in sculpt mode, so these rows read as if `mode.sculpt` were on `$Cube`.
    // The sculpt level comes from the first enabled Multires modifier (modifiers).
    // Pixel paths read the viewport picture: every visible object is drawn at the current frame and can occlude.
    // sculpt.stroke: pixels -> the surface under the pointer in the picture (occluders, frame), the world transform, the mesh/level
    // was: {}
    r(
        "sculpt.stroke",
        r##"{"points": [[160, 90], [170, 95]]}"##,
        "mode,time",
        &[
            ("$Cube", "constraints,mesh,modifiers", "*"),
            ("$Ball", "*", "*"),
            ("$Child", "*", "*"),
            ("$Rig", "constraints", "*"),
            ("$Path", "*", "*"),
            ("$Label", "*", "*"),
        ],
    ),
    // sculpt.stroke: world points -> dab centres directly through the world matrix; the view gives only the eye
    // was: {}
    r("sculpt.stroke", r##"{"points": [[0, 0, 1], [0.2, 0, 1]], "worldRadius": 0.25}"##, "mode", &[("$Cube", "constraints,mesh,modifiers", "*")]),
    // sculpt.maskInvert: 1 - the current mask
    // was: {}
    r("sculpt.maskInvert", r##"{}"##, "mode", &[("$Cube", "mesh", "-")]),
    // sculpt.maskClear: empties the mask; nothing read
    // was: {}
    r("sculpt.maskClear", r##"{}"##, "mode", &[("$Cube", "-", "-")]),
    // sculpt.maskFill: a mask of 1 per vertex (vertex count)
    // was: {}
    r("sculpt.maskFill", r##"{}"##, "mode", &[("$Cube", "~topology", "-")]),
    // sculpt.maskSmooth: the mask averaged over the neighbours
    // was: {}
    r("sculpt.maskSmooth", r##"{}"##, "mode", &[("$Cube", "mesh", "-")]),
    // sculpt.dyntopo: enabled -> the mesh is triangulated (the level-count check only refuses); disabled -> session settings only
    // was: {}
    r("sculpt.dyntopo", r##"{"enabled": true}"##, "mode", &[("$Cube", "mesh", "-")]),
    // was: {}
    r("sculpt.dyntopo", r##"{"enabled": false}"##, "mode", &[("$Cube", "-", "-")]),
    // sculpt.detailFlood: edge length from the view's pixel size at the world bounds centre (world matrix, scale), then refine
    // was: {}
    r("sculpt.detailFlood", r##"{}"##, "mode", &[("$Cube", "constraints,mesh", "*")]),
    // sculpt.gesture: vertices projected through the world matrix into the box/lasso; mask/hide/face set
    // was: {}
    r("sculpt.gesture", r##"{"action": "mask", "points": [[0, 0], [160, 90]]}"##, "mode", &[("$Cube", "constraints,mesh", "*")]),
    // sculpt.gesture frontFacesOnly: plus the picture's depth (occluders at the current frame)
    // was: {}
    r(
        "sculpt.gesture",
        r##"{"action": "hide", "shape": "box", "points": [[0, 0], [160, 90]], "frontFacesOnly": true}"##,
        "mode,time",
        &[
            ("$Cube", "constraints,mesh", "*"),
            ("$Ball", "*", "*"),
            ("$Child", "*", "*"),
            ("$Rig", "constraints", "*"),
            ("$Path", "*", "*"),
            ("$Label", "*", "*"),
        ],
    ),
    // sculpt.faceSetsFromMask: faces wholly masked (and not hidden) become a new set
    // was: {}
    r("sculpt.faceSetsFromMask", r##"{}"##, "mode", &[("$Cube", "mesh", "-")]),
    // sculpt.faceSetsFromVisible: the faces not hidden become a new set
    // was: {}
    r("sculpt.faceSetsFromVisible", r##"{}"##, "mode", &[("$Cube", "mesh", "-")]),
    // sculpt.faceSetsFromLooseParts: one set per connected part
    // was: {}
    r("sculpt.faceSetsFromLooseParts", r##"{}"##, "mode", &[("$Cube", "mesh", "-")]),
    // sculpt.faceSetsClear: empties the face sets; nothing read
    // was: {}
    r("sculpt.faceSetsClear", r##"{}"##, "mode", &[("$Cube", "-", "-")]),
    // sculpt.faceSetVisibility: by face-set number (face sets, hidden flags for toggle)
    // was: {}
    r("sculpt.faceSetVisibility", r##"{"action": "hide", "faceSet": 1}"##, "mode", &[("$Cube", "mesh", "-")]),
    // sculpt.faceSetVisibility: by the face under a pixel (picture: occluders, frame; level face origins via modifiers)
    // was: {}
    r(
        "sculpt.faceSetVisibility",
        r##"{"action": "isolate", "x": 160, "y": 90}"##,
        "mode,time",
        &[
            ("$Cube", "constraints,mesh,modifiers", "*"),
            ("$Ball", "*", "*"),
            ("$Child", "*", "*"),
            ("$Rig", "constraints", "*"),
            ("$Path", "*", "*"),
            ("$Label", "*", "*"),
        ],
    ),
    // sculpt.faceSetVisibility: neither faceSet nor x/y -> the face under the sculpt session's hover point
    // was: {}
    r(
        "sculpt.faceSetVisibility",
        r##"{"action": "toggle"}"##,
        "mode,time",
        &[
            ("$Cube", "constraints,mesh,modifiers", "*"),
            ("$Ball", "*", "*"),
            ("$Child", "*", "*"),
            ("$Rig", "constraints", "*"),
            ("$Path", "*", "*"),
            ("$Label", "*", "*"),
        ],
    ),
    // sculpt.faceSetVisibility showAll: every face shown; nothing read
    // was: {}
    r("sculpt.faceSetVisibility", r##"{"action": "showAll"}"##, "mode", &[("$Cube", "-", "-")]),
    // sculpt.hideFaceSet: shares visibility_command, always with the hover point
    // was: {}
    r(
        "sculpt.hideFaceSet",
        r##"{}"##,
        "mode,time",
        &[
            ("$Cube", "constraints,mesh,modifiers", "*"),
            ("$Ball", "*", "*"),
            ("$Child", "*", "*"),
            ("$Rig", "constraints", "*"),
            ("$Path", "*", "*"),
            ("$Label", "*", "*"),
        ],
    ),
    // sculpt.isolateFaceSet: shares visibility_command, always with the hover point
    // was: {}
    r(
        "sculpt.isolateFaceSet",
        r##"{}"##,
        "mode,time",
        &[
            ("$Cube", "constraints,mesh,modifiers", "*"),
            ("$Ball", "*", "*"),
            ("$Child", "*", "*"),
            ("$Rig", "constraints", "*"),
            ("$Path", "*", "*"),
            ("$Label", "*", "*"),
        ],
    ),
    // sculpt.showAllFaces: every face shown; nothing read
    // was: {}
    r("sculpt.showAllFaces", r##"{}"##, "mode", &[("$Cube", "-", "-")]),
    // sculpt.remesh: object id-or-name (gate bypassed), else the sculpted object, else the active mesh; voxel remesh of the local mesh
    // was: {$Cube */*}
    r("sculpt.remesh", r##"{"object": "$Cube", "voxelSize": 0.1}"##, "", &[("$Cube", "mesh", "-")]),
    // was: {}
    r("sculpt.remesh", r##"{"voxelSize": 0.1}"##, "mode,selection", &[("$Cube", "mesh", "-")]),
    // was: {$Cube */*}
    r("sculpt.remesh", r##"{"object": "Cube"}"##, "", &[("$Cube", "mesh,name", "-")]),
    // Multires: object mode only (elements are not in the schema). object id-or-name, else the sculpted object, else
    // the active one. It uses the first Multires modifier. The dyntopo check only refuses.
    // multires.subdivide: appends a Multires when there is none, adds a level, sets the levels
    // was: {$Cube */*}
    r("multires.subdivide", r##"{"object": "$Cube"}"##, "", &[("$Cube", "mesh,modifiers", "-")]),
    // was: {}
    r("multires.subdivide", r##"{}"##, "mode,selection", &[("$Cube", "mesh,modifiers", "-")]),
    // was: {$Cube */*}
    r("multires.subdivide", r##"{"object": "Cube"}"##, "", &[("$Cube", "mesh,modifiers,name", "-")]),
    // multires.unsubdivide: a lower base recovered from the mesh; the modifier must exist (its settings stay)
    // was: {$Cube */*}
    r("multires.unsubdivide", r##"{"object": "$Cube"}"##, "", &[("$Cube", "mesh,modifiers", "-")]),
    // was: {}
    r("multires.unsubdivide", r##"{}"##, "mode,selection", &[("$Cube", "mesh,modifiers", "-")]),
    // was: {$Cube */*}
    r("multires.unsubdivide", r##"{"object": "Cube"}"##, "", &[("$Cube", "mesh,modifiers,name", "-")]),
    // multires.rebuildSubdivisions: every lower base recovered; levels set on the first Multires
    // was: {$Cube */*}
    r("multires.rebuildSubdivisions", r##"{"object": "$Cube"}"##, "", &[("$Cube", "mesh,modifiers", "-")]),
    // was: {}
    r("multires.rebuildSubdivisions", r##"{}"##, "mode,selection", &[("$Cube", "mesh,modifiers", "-")]),
    // was: {$Cube */*}
    r("multires.rebuildSubdivisions", r##"{"object": "Cube"}"##, "", &[("$Cube", "mesh,modifiers,name", "-")]),
    // multires.deleteHigher: keeps the first Multires' sculptLevels levels
    // was: {$Cube */*}
    r("multires.deleteHigher", r##"{"object": "$Cube"}"##, "", &[("$Cube", "mesh,modifiers", "-")]),
    // was: {}
    r("multires.deleteHigher", r##"{}"##, "mode,selection", &[("$Cube", "mesh,modifiers", "-")]),
    // was: {$Cube */*}
    r("multires.deleteHigher", r##"{"object": "Cube"}"##, "", &[("$Cube", "mesh,modifiers,name", "-")]),
    // multires.applyBase: base fitted to the mesh's own highest level (the modifier only has to exist)
    // was: {$Cube */*}
    r("multires.applyBase", r##"{"object": "$Cube"}"##, "", &[("$Cube", "mesh", "-")]),
    // was: {}
    r("multires.applyBase", r##"{}"##, "mode,selection", &[("$Cube", "mesh", "-")]),
    // was: {$Cube */*}
    r("multires.applyBase", r##"{"object": "Cube"}"##, "", &[("$Cube", "mesh,name", "-")]),
    // multires.reshape: `from` id-or-name of another mesh; its local base positions become the highest level
    // was: {$Ball */*, $Cube */*}
    r("multires.reshape", r##"{"object": "$Cube", "from": "$Ball"}"##, "", &[("$Cube", "mesh", "-"), ("$Ball", "mesh", "-")]),
    // was: {$Ball */*}
    r("multires.reshape", r##"{"from": "$Ball"}"##, "mode,selection", &[("$Cube", "mesh", "-"), ("$Ball", "mesh", "-")]),
    // was: {$Ball */*, $Cube */*}
    r("multires.reshape", r##"{"object": "Cube", "from": "Ball"}"##, "", &[("$Cube", "mesh,name", "-"), ("$Ball", "mesh,name", "-")]),
    // ---- reading 4: anim, frame.range, driver, keyingSet, action, nla, graph ----
    // Common to the nla/graph family: after success the wrapper re-evaluates the scene at the current frame
    // (go_to_frame) — treated like the commit's driver re-evaluation: not `time`, not held. action.assign does the
    // same inside its handler (only to apply the keys it just wrote); also not counted.
    // anim.*: gate needs object mode, no rig session and a non-empty selection unless `objects` is named (gate only).

    // animation.insertKey: objects=id-or-name else selection; frame else current frame; with properties given (no
    // keyingSet) keys each property's CURRENT value (location/rotation/scale: positions) into the object's
    // animation at the tweak-mapped frame (nla + tweak).
    r("animation.insertKey", r##"{"objects": ["$Cube"], "properties": ["location"], "frame": 1}"##, "", &[("$Cube", "animation,nla,tweak", "*")]),
    r("animation.insertKey", r##"{"objects": ["Cube"], "properties": ["location"], "frame": 1}"##, "", &[("$Cube", "animation,name,nla,tweak", "*")]),
    // properties absent: the scene's active keying set decides what is keyed (none -> location, rotation, scale).
    // was: {$Cube animation,nla,tweak/*}
    r("animation.insertKey", r##"{"objects": ["$Cube"]}"##, "time", &[("$Cube", "animation,nla,tweak", "*"), ("scene", "keyingSets", "-")]),
    // was: {}
    r("animation.insertKey", r##"{}"##, "selection,time", &[("$Cube", "animation,nla,tweak", "*"), ("scene", "keyingSets", "-")]),
    // baseColor: keyed on the object's material (slot read), at the scene frame (no tweak map); value from the material's data.
    // was: {}
    r(
        "animation.insertKey",
        r##"{"objects": ["$Cube"], "properties": ["baseColor"], "frame": 1}"##,
        "",
        &[("$Cube", "material", "-"), ("$Mat", "animation,data", "-")],
    ),
    // power of a light: value from the light's data.
    // was: {}
    r("animation.insertKey", r##"{"objects": ["$Light"], "properties": ["power"], "frame": 1}"##, "", &[("$Light", "animation,data,nla,tweak", "-")]),
    // builtin keying set named (builtin names resolve without reading the scene's own sets).
    // was: {}
    r("animation.insertKey", r##"{"objects": ["$Cube"], "keyingSet": "location", "frame": 1}"##, "", &[("$Cube", "animation,nla,tweak", "*")]),
    // wholeCharacter: every bone of the armature (bone names and poses in data).
    // was: {}
    r(
        "animation.insertKey",
        r##"{"objects": ["$Arm"], "keyingSet": "wholeCharacter", "frame": 1}"##,
        "",
        &[("$Arm", "animation,data,nla,tweak", "-")],
    ),
    // animation.deleteKey: objects else selection; frame else current; default properties include baseColor, so the
    // material's keys on that frame go too; frame tweak-mapped for object properties. Values are not read.
    // was: {}
    r(
        "animation.deleteKey",
        r##"{"objects": ["$Cube"], "frame": 1}"##,
        "",
        &[("$Cube", "animation,material,nla,tweak", "-"), ("$Mat", "animation", "-")],
    ),
    // was: {$Cube */*}
    r(
        "animation.deleteKey",
        r##"{"objects": ["Cube"], "frame": 1}"##,
        "",
        &[("$Cube", "animation,material,name,nla,tweak", "-"), ("$Mat", "animation", "-")],
    ),
    // was: {}
    r("animation.deleteKey", r##"{}"##, "selection,time", &[("$Cube", "animation,material,nla,tweak", "-"), ("$Mat", "animation", "-")]),
    // was: {}
    r("animation.deleteKey", r##"{"objects": ["$Cube"], "properties": ["location"], "frame": 1}"##, "", &[("$Cube", "animation,nla,tweak", "-")]),
    // animation.clear: without properties every keyed channel of the object and its material's baseColor channels.
    // was: {}
    r("animation.clear", r##"{"objects": ["$Cube"]}"##, "", &[("$Cube", "animation,material", "-"), ("$Mat", "animation", "-")]),
    // was: {$Cube */*}
    r("animation.clear", r##"{"objects": ["Cube"]}"##, "", &[("$Cube", "animation,material,name", "-"), ("$Mat", "animation", "-")]),
    // was: {}
    r("animation.clear", r##"{}"##, "selection", &[("$Cube", "animation,material", "-"), ("$Mat", "animation", "-")]),
    // was: {}
    r("animation.clear", r##"{"objects": ["$Cube"], "properties": ["location"]}"##, "", &[("$Cube", "animation", "-")]),
    // animation.setInterpolation: keys of the named properties (default all 8, baseColor on the material); frame only
    // filters (absent = every key, not the current frame); no tweak map.
    // was: {}
    r(
        "animation.setInterpolation",
        r##"{"objects": ["$Cube"], "interpolation": "linear"}"##,
        "",
        &[("$Cube", "animation,material", "-"), ("$Mat", "animation", "-")],
    ),
    // was: {$Cube */*}
    r(
        "animation.setInterpolation",
        r##"{"objects": ["Cube"], "interpolation": "linear"}"##,
        "",
        &[("$Cube", "animation,material,name", "-"), ("$Mat", "animation", "-")],
    ),
    // was: {}
    r(
        "animation.setInterpolation",
        r##"{"interpolation": "linear"}"##,
        "selection",
        &[("$Cube", "animation,material", "-"), ("$Mat", "animation", "-")],
    ),
    // was: {}
    r(
        "animation.setInterpolation",
        r##"{"objects": ["$Cube"], "properties": ["location"], "frame": 1, "interpolation": "constant"}"##,
        "",
        // corrected by C8-scene-engine: `frame` is tweak-mapped as insertKey/deleteKey map it (bindings/wasm/src/commands_anim.rs:383)
        &[("$Cube", "animation,nla,tweak", "-")],
    ),
    // frame.range: start/end/fps of the timeline (the others kept, checked together).
    // was: {}
    r("frame.range", r##"{"start": 1, "end": 120}"##, "", &[("scene", "timeline", "-")]),
    // driver.add: object=id-or-name else the ACTIVE object; the channel (existing keyed one refuses, existing
    // driver is changed); the property's existence is only checked. Variable sources are referenced (the
    // driven value is the commit's driver re-evaluation: not held).
    // was: {$Cube */*}
    r("driver.add", r##"{"object": "$Cube", "property": "location", "index": 0, "expression": "frame"}"##, "", &[("$Cube", "animation", "-")]),
    // was: {$Cube */*}
    r("driver.add", r##"{"object": "Cube", "property": "location", "index": 0, "expression": "frame"}"##, "", &[("$Cube", "animation,name", "-")]),
    // was: {}
    r("driver.add", r##"{"property": "location", "index": 0, "expression": "frame"}"##, "selection", &[("$Cube", "animation", "-")]),
    // was: {$Cube */*}
    r("driver.add", r##"{"object": "$Cube", "property": "baseColor", "index": 0}"##, "", &[("$Cube", "material", "-"), ("$Mat", "animation", "-")]),
    // was: {$Arm */*}
    r("driver.add", r##"{"object": "$Arm", "property": "boneRotation", "bone": "Bone", "index": 2}"##, "", &[("$Arm", "animation,data", "-")]),
    // was: {$Ball */*, $Cube */*}
    r(
        "driver.add",
        r##"{"object": "$Cube", "property": "location", "index": 2, "variables": [{"name": "var", "type": "transform", "object": "$Ball", "channel": "locationX", "space": "world"}]}"##,
        "",
        &[("$Cube", "animation", "-"), ("$Ball", "-", "-")],
    ),
    // was: {$Ball */*, $Cube */*}
    r(
        "driver.add",
        r##"{"object": "$Cube", "property": "location", "index": 2, "variables": [{"name": "var", "type": "singleProperty", "object": "Ball", "property": "location"}]}"##,
        "",
        &[("$Cube", "animation", "-"), ("$Ball", "name", "-")],
    ),
    // driver.set: shares the handler with driver.add; needs an existing driver.
    // was: {$Cube */*}
    r("driver.set", r##"{"object": "$Cube", "property": "location", "index": 0, "expression": "frame * 2"}"##, "", &[("$Cube", "animation", "-")]),
    // was: {$Cube */*}
    r("driver.set", r##"{"object": "Cube", "property": "location", "index": 0, "kind": "sum"}"##, "", &[("$Cube", "animation,name", "-")]),
    // was: {}
    r("driver.set", r##"{"property": "location", "index": 0, "kind": "sum"}"##, "selection", &[("$Cube", "animation", "-")]),
    // was: {$Ball */*, $Cube */*, $Light */*}
    r(
        "driver.set",
        r##"{"object": "$Cube", "property": "location", "index": 0, "variables": [{"name": "d", "type": "distance", "a": "$Ball", "b": "$Light"}]}"##,
        "",
        &[("$Cube", "animation", "-"), ("$Ball", "-", "-"), ("$Light", "-", "-")],
    ),
    // driver.remove: the driver channels of the property (material's for baseColor).
    // was: {$Cube */*}
    r("driver.remove", r##"{"object": "$Cube", "property": "location"}"##, "", &[("$Cube", "animation", "-")]),
    // was: {$Cube */*}
    r("driver.remove", r##"{"object": "Cube", "property": "location"}"##, "", &[("$Cube", "animation,name", "-")]),
    // was: {}
    r("driver.remove", r##"{"property": "location"}"##, "selection", &[("$Cube", "animation", "-")]),
    // was: {$Cube */*}
    r("driver.remove", r##"{"object": "$Cube", "property": "baseColor"}"##, "", &[("$Cube", "material", "-"), ("$Mat", "animation", "-")]),
    // keyingSet.new: appends a set (name made unique: not held) AND makes it the active one (replaces `active`).
    // was: {}
    r("keyingSet.new", r##"{"name": "KS"}"##, "", &[("scene", "keyingSets", "-")]),
    // keyingSet.remove: the scene's own set by name, else the active custom one.
    // was: {}
    r("keyingSet.remove", r##"{"name": "KS"}"##, "", &[("scene", "keyingSets", "-")]),
    // was: {}
    r("keyingSet.remove", r##"{}"##, "", &[("scene", "keyingSets", "-")]),
    // keyingSet.addPaths: objects id-or-name else selection; the objects are only referenced (no property check);
    // baseColor stores the object's material.
    // was: {}
    r("keyingSet.addPaths", r##"{"name": "KS", "objects": ["$Cube"]}"##, "", &[("scene", "keyingSets", "-"), ("$Cube", "-", "-")]),
    // was: {$Cube */*}
    r("keyingSet.addPaths", r##"{"name": "KS", "objects": ["Cube"]}"##, "", &[("scene", "keyingSets", "-"), ("$Cube", "name", "-")]),
    // was: {}
    r("keyingSet.addPaths", r##"{"name": "KS"}"##, "selection", &[("scene", "keyingSets", "-"), ("$Cube", "-", "-")]),
    // was: {}
    r(
        "keyingSet.addPaths",
        r##"{"name": "KS", "objects": ["$Cube"], "properties": ["baseColor"]}"##,
        "",
        &[("scene", "keyingSets", "-"), ("$Cube", "material", "-"), ("$Mat", "-", "-")],
    ),
    // was: {}
    r("keyingSet.addPaths", r##"{"name": "KS", "objects": ["$Arm"], "bones": ["Bone"]}"##, "", &[("scene", "keyingSets", "-"), ("$Arm", "-", "-")]),
    // keyingSet.removePath: index into the set's paths.
    // was: {}
    r("keyingSet.removePath", r##"{"name": "KS", "index": 0}"##, "", &[("scene", "keyingSets", "-")]),
    // keyingSet.setActive: builtin name or one of the scene's own; replaces `active`.
    // was: {}
    r("keyingSet.setActive", r##"{"name": "locRotScale"}"##, "", &[("scene", "keyingSets", "-")]),
    // action.new: empty action appended (name made unique: not held); with `object` (id-or-name, never the
    // active object) a copy of its keyed channels.
    r("action.new", r##"{"name": "Act"}"##, "", &[]),
    // was: {$Cube */*}
    r("action.new", r##"{"name": "Act", "object": "$Cube"}"##, "", &[("$Cube", "animation", "-")]),
    // was: {$Cube */*}
    r("action.new", r##"{"name": "Act", "object": "Cube"}"##, "", &[("$Cube", "animation,name", "-")]),
    // action.assign: action by name; objects else the SELECTION; replaces the object's own keys (drivers kept and
    // block channels; tweak only refuses); bone channels need the armature's bones (data).
    // was: {$Cube */*}
    r("action.assign", r##"{"objects": ["$Cube"], "action": "Act"}"##, "", &[("$Cube", "animation", "-"), ("scene", "actions", "-")]),
    // was: {$Cube */*}
    r("action.assign", r##"{"objects": ["Cube"], "action": "Act"}"##, "", &[("$Cube", "animation,name", "-"), ("scene", "actions", "-")]),
    // was: {}
    r("action.assign", r##"{"action": "Act"}"##, "selection", &[("$Cube", "animation", "-"), ("scene", "actions", "-")]),
    // was: {$Arm */*}
    r("action.assign", r##"{"objects": ["$Arm"], "action": "Act"}"##, "", &[("$Arm", "animation,data", "-"), ("scene", "actions", "-")]),
    // action.remove / action.rename: the action by name; strips, tweaks and Action constraints of every object
    // follow (they are rewritten, not read to decide; not held per object).
    // was: {}
    r("action.remove", r##"{"name": "Act"}"##, "", &[("scene", "actions", "-")]),
    // was: {}
    r("action.rename", r##"{"name": "Act", "to": "Run"}"##, "", &[("scene", "actions", "-")]),
    // nla.*: object=id-or-name else the active object; tweak checks only refuse.
    // nla.pushDown: the object's keyed channels become an action; its default name comes from the object's name.
    // was: {$Cube */*}
    r("nla.pushDown", r##"{"object": "$Cube"}"##, "", &[("$Cube", "animation,name,nla", "-")]),
    // was: {$Cube */*}
    r("nla.pushDown", r##"{"object": "$Cube", "name": "Act"}"##, "", &[("$Cube", "animation,nla", "-")]),
    // was: {$Cube */*}
    r("nla.pushDown", r##"{"object": "Cube", "name": "Act"}"##, "", &[("$Cube", "animation,name,nla", "-")]),
    // was: {}
    r("nla.pushDown", r##"{"name": "Act"}"##, "selection", &[("$Cube", "animation,nla", "-")]),
    // nla.addTrack: index (clamped) into the tracks, else on top.
    // was: {$Cube */*}
    r("nla.addTrack", r##"{"object": "$Cube"}"##, "", &[("$Cube", "nla", "-")]),
    // was: {$Cube */*}
    r("nla.addTrack", r##"{"object": "Cube", "index": 0}"##, "", &[("$Cube", "name,nla", "-")]),
    // was: {}
    r("nla.addTrack", r##"{}"##, "selection", &[("$Cube", "nla", "-")]),
    // nla.removeTrack / setTrack: track index.
    // was: {$Cube */*}
    r("nla.removeTrack", r##"{"object": "$Cube", "track": 0}"##, "", &[("$Cube", "nla", "-")]),
    // was: {}
    r("nla.removeTrack", r##"{"track": 0}"##, "selection", &[("$Cube", "nla", "-")]),
    // was: {$Cube */*}
    r("nla.setTrack", r##"{"object": "$Cube", "track": 0, "muted": true}"##, "", &[("$Cube", "nla", "-")]),
    // was: {}
    r("nla.setTrack", r##"{"track": 0, "solo": true}"##, "selection", &[("$Cube", "nla", "-")]),
    // nla.addStrip: action by name (its keyed range sets the strip's frames); start else the current frame;
    // track index else the top track with room, else a new one.
    // was: {$Cube */*}
    r("nla.addStrip", r##"{"object": "$Cube", "action": "Act", "start": 1}"##, "", &[("$Cube", "nla", "-"), ("scene", "actions", "-")]),
    // was: {$Cube */*}
    r("nla.addStrip", r##"{"object": "$Cube", "action": "Act"}"##, "time", &[("$Cube", "nla", "-"), ("scene", "actions", "-")]),
    // was: {$Cube */*}
    r(
        "nla.addStrip",
        r##"{"object": "Cube", "action": "Act", "track": 0, "start": 1}"##,
        "",
        &[("$Cube", "name,nla", "-"), ("scene", "actions", "-")],
    ),
    // was: {}
    r("nla.addStrip", r##"{"action": "Act", "start": 1}"##, "selection", &[("$Cube", "nla", "-"), ("scene", "actions", "-")]),
    // nla.setStrip / addTransition / makeMeta / dissolveMeta / removeStrip: track + strip indexes into the NLA.
    // was: {$Cube */*}
    r("nla.setStrip", r##"{"object": "$Cube", "track": 0, "strip": 0, "scale": 2}"##, "", &[("$Cube", "nla", "-")]),
    // was: {}
    r("nla.setStrip", r##"{"track": 0, "strip": 0, "toTrack": 1}"##, "selection", &[("$Cube", "nla", "-")]),
    // was: {$Cube */*}
    r("nla.addTransition", r##"{"object": "$Cube", "track": 0, "strip": 0}"##, "", &[("$Cube", "nla", "-")]),
    // was: {}
    r("nla.addTransition", r##"{"track": 0, "strip": 0}"##, "selection", &[("$Cube", "nla", "-")]),
    // was: {$Cube */*}
    r("nla.makeMeta", r##"{"object": "$Cube", "track": 0, "first": 0, "last": 1}"##, "", &[("$Cube", "nla", "-")]),
    // was: {}
    r("nla.makeMeta", r##"{"track": 0, "first": 0, "last": 1}"##, "selection", &[("$Cube", "nla", "-")]),
    // was: {$Cube */*}
    r("nla.dissolveMeta", r##"{"object": "$Cube", "track": 0, "strip": 0}"##, "", &[("$Cube", "nla", "-")]),
    // was: {}
    r("nla.dissolveMeta", r##"{"track": 0, "strip": 0}"##, "selection", &[("$Cube", "nla", "-")]),
    // was: {$Cube */*}
    r("nla.removeStrip", r##"{"object": "$Cube", "track": 0, "strip": 0}"##, "", &[("$Cube", "nla", "-")]),
    // was: {$Cube */*}
    r("nla.removeStrip", r##"{"object": "Cube", "track": 0, "strip": 0}"##, "", &[("$Cube", "name,nla", "-")]),
    // was: {}
    r("nla.removeStrip", r##"{"track": 0, "strip": 0}"##, "selection", &[("$Cube", "nla", "-")]),
    // nla.setAction: blend settings of the object's own keys (merged over the current ones); no tweak check.
    // was: {$Cube */*}
    r("nla.setAction", r##"{"object": "$Cube", "influence": 0.5}"##, "", &[("$Cube", "nla", "-")]),
    // was: {}
    r("nla.setAction", r##"{"blend": "add"}"##, "selection", &[("$Cube", "nla", "-")]),
    // nla.tweak: the strip's action's keys replace the object's own (stashed in tweak); drivers block channels.
    // was: {$Cube */*}
    r("nla.tweak", r##"{"object": "$Cube", "track": 0, "strip": 0}"##, "", &[("$Cube", "animation,nla,tweak", "-"), ("scene", "actions", "-")]),
    // was: {$Cube */*}
    r(
        "nla.tweak",
        r##"{"object": "$Cube", "track": 0, "strip": 0, "upperStack": true}"##,
        "",
        &[("$Cube", "animation,nla,tweak", "-"), ("scene", "actions", "-")],
    ),
    // was: {}
    r("nla.tweak", r##"{"track": 0, "strip": 0}"##, "selection", &[("$Cube", "animation,nla,tweak", "-"), ("scene", "actions", "-")]),
    // nla.tweakExit: the object's keys go back into the strip's action (with its channels the object cannot
    // take), the stash comes back.
    // was: {$Cube */*}
    r("nla.tweakExit", r##"{"object": "$Cube"}"##, "", &[("$Cube", "animation,nla,tweak", "-"), ("scene", "actions", "-")]),
    // was: {$Cube */*}
    r("nla.tweakExit", r##"{"object": "Cube"}"##, "", &[("$Cube", "animation,name,nla,tweak", "-"), ("scene", "actions", "-")]),
    // was: {}
    r("nla.tweakExit", r##"{}"##, "selection", &[("$Cube", "animation,nla,tweak", "-"), ("scene", "actions", "-")]),
    // graph.*: target = integer id of an object or material (no names, no selection); the exact channel
    // (target, property, index, path) — keyed or driver — and its modifier stack.
    // was: {$Cube */*}
    r("graph.addModifier", r##"{"target": "$Cube", "property": "location", "index": 0, "type": "noise"}"##, "", &[("$Cube", "animation", "-")]),
    // was: {$Mat */*}
    r("graph.addModifier", r##"{"target": "$Mat", "property": "baseColor", "index": 0, "type": "cycles"}"##, "", &[("$Mat", "animation", "-")]),
    // was: {$Cube */*}
    r(
        "graph.setModifier",
        r##"{"target": "$Cube", "property": "location", "index": 0, "modifier": 0, "strength": 2}"##,
        "",
        &[("$Cube", "animation", "-")],
    ),
    // was: {$Mat */*}
    r(
        "graph.setModifier",
        r##"{"target": "$Mat", "property": "baseColor", "index": 0, "modifier": 0, "moveTo": 1}"##,
        "",
        &[("$Mat", "animation", "-")],
    ),
    // was: {$Cube */*}
    r("graph.removeModifier", r##"{"target": "$Cube", "property": "location", "index": 0, "modifier": 0}"##, "", &[("$Cube", "animation", "-")]),
    // was: {$Mat */*}
    r("graph.removeModifier", r##"{"target": "$Mat", "property": "baseColor", "index": 0, "modifier": 0}"##, "", &[("$Mat", "animation", "-")]),
    // ---- reading 5: rigging, vertex groups, shape keys, graph keys, constraints, weight paint ----
    // Session-only paths (armature edit/pose session, weight paint session, mesh edit mode) cannot be set up
    // by the object-mode fixture (C8-scene-engine 4c: the session rows run on its fixture: [`FIXTURES`]). For commands that run ONLY in such a session a row is still given (every
    // write command needs one): its editor is `mode` and it holds no object, because the object acted on
    // is the session's (no parameter names it).

    // edit.duplicate: objects id-or-name else the selection; the mode picks objects vs vertices vs bones; clones the whole object (location + offset, its animation)
    r("edit.duplicate", r##"{"objects": ["$Cube"], "offset": [1, 0, 0]}"##, "mode", &[("$Cube", "*", "*")]),
    r("edit.duplicate", r##"{"objects": ["Cube"]}"##, "mode", &[("$Cube", "*", "*")]),
    // was: {}
    r("edit.duplicate", r##"{}"##, "mode,selection", &[("$Cube", "*", "*")]),
    // edit.duplicate in mesh edit mode (edited mesh + edit-selection) and in armature edit mode (selected bones): not set up by the fixture, no row. (C8-scene-engine 4c: the session rows run on its fixture: [`FIXTURES`].)

    // object.addArmature: a new armature at `location` (default origin, not the cursor); the object-mode gate only refuses
    r("object.addArmature", r##"{"name": "Arm2", "location": [1, 0, 0]}"##, "", &[]),
    // bone.add: armature EDIT session only; the armature is the session's, bones added in armature space (reads parent bone tail)
    r("bone.add", r##"{"name": "B2", "head": [0, 0, 1], "tail": [0, 0, 2], "parent": "Bone", "connected": true}"##, "mode", &[]),
    // bone.extrude: armature EDIT session only; extrudes the session's selected bones
    r("bone.extrude", r##"{"length": 0.5}"##, "mode", &[]),
    // bone.set: the session's armature when one is on (even over `object`), else object id-or-name, else the active object; bone else the session's active bone; reads/writes the armature's data
    // was: {$Arm */*}
    r("bone.set", r##"{"object": "$Arm", "bone": "Bone", "roll": 45}"##, "mode", &[("$Arm", "data", "-")]),
    // was: {$Arm */*}
    r("bone.set", r##"{"object": "Arm", "bone": "Bone", "roll": 45}"##, "mode", &[("$Arm", "data,name", "-")]),
    // absent object: the active object ($Cube here, refused as not an armature)
    // was: {}
    r("bone.set", r##"{"bone": "Bone", "roll": 45}"##, "mode,selection", &[("$Cube", "data", "-")]),
    // rename: also the rig's bone channels (and groups of meshes deformed by it: none in the fixture)
    // was: {$Arm */*}
    r("bone.set", r##"{"object": "$Arm", "bone": "Bone", "name": "Spine"}"##, "mode", &[("$Arm", "animation,data", "-")]),
    // custom shape: another object, only its existence (and a geometry kind check)
    // was: {$Arm */*, $Ball */*}
    r("bone.set", r##"{"object": "$Arm", "bone": "Bone", "shape": "$Ball"}"##, "mode", &[("$Arm", "data", "-"), ("$Ball", "-", "-")]),
    // pose.set: POSE session only; bones given or the session's selection; writes bone poses
    r("pose.set", r##"{"bones": ["Bone"], "rotation": [0, 0, 45]}"##, "mode", &[]),
    // pose.clear: POSE session only (gate needs a selected bone even with bones/all)
    r("pose.clear", r##"{"all": true}"##, "mode", &[]),
    // pose.insertKey: POSE session only; frame else the current frame; with no properties the active keying set
    r("pose.insertKey", r##"{"bones": ["Bone"], "properties": ["location"], "frame": 5}"##, "mode", &[]),
    // was: {}
    r("pose.insertKey", r##"{"bones": ["Bone"]}"##, "mode,time", &[("scene", "keyingSets", "-")]),
    // pose.deleteKey: POSE session only; frame else the current frame
    r("pose.deleteKey", r##"{"bones": ["Bone"], "frame": 5}"##, "mode", &[]),
    r("pose.deleteKey", r##"{"bones": ["Bone"]}"##, "mode,time", &[]),
    // object.parentArmature: armature id-or-name else active; objects id-or-name else selection (meshes); parents (inverse of the armature's world), Armature modifier, bone-heat groups from both world transforms and the armature's bones
    // (gate: active must be an armature and a mesh selected, even with both params: refusal only)
    // was: {$Arm */*, $Cube */*}
    r(
        "object.parentArmature",
        r##"{"armature": "$Arm", "objects": ["$Cube"], "weights": "automatic"}"##,
        "",
        &[("$Arm", "constraints,data", "*"), ("$Cube", "constraints,mesh,modifiers", "*")],
    ),
    // was: {$Arm */*, $Cube */*}
    r(
        "object.parentArmature",
        r##"{"armature": "Arm", "objects": ["Cube"], "weights": "automatic"}"##,
        "",
        &[("$Arm", "constraints,data,name", "*"), ("$Cube", "constraints,mesh,modifiers,name", "*")],
    ),
    // weights none: no bind; set_parent reads only the armature's world (the child's world is computed but unused)
    // was: {$Arm */*, $Cube */*}
    r(
        "object.parentArmature",
        r##"{"armature": "$Arm", "objects": ["$Cube"], "weights": "none"}"##,
        "",
        &[("$Arm", "constraints", "*"), ("$Cube", "modifiers", "-")],
    ),
    // weights empty: missing groups per deforming bone, zero weights (bone names, vertex count)
    // was: {$Arm */*, $Cube */*}
    r(
        "object.parentArmature",
        r##"{"armature": "$Arm", "objects": ["$Cube"], "weights": "empty"}"##,
        "",
        &[("$Arm", "constraints,data", "*"), ("$Cube", "mesh,modifiers", "-")],
    ),
    // was: {$Arm */*}
    r(
        "object.parentArmature",
        r##"{"armature": "$Arm"}"##,
        "selection",
        &[("$Arm", "constraints,data", "*"), ("$Cube", "constraints,mesh,modifiers", "*")],
    ),
    // armature absent: the active object ($Cube, refused as not an armature) in the armature role
    // was: {$Cube */*}
    r("object.parentArmature", r##"{"objects": ["$Cube"]}"##, "selection", &[("$Cube", "constraints,data,mesh,modifiers", "*")]),
    // vertexGroup.add: object id-or-name else the edited mesh / active mesh; a zero group sized to the vertex count
    // was: {$Cube mesh/-}
    r("vertexGroup.add", r##"{"object": "$Cube", "name": "G1"}"##, "", &[("$Cube", "~topology", "-")]),
    // was: {$Cube mesh,name/-}
    r("vertexGroup.add", r##"{"object": "Cube", "name": "G1"}"##, "", &[("$Cube", "name,~topology", "-")]),
    // was: {}
    r("vertexGroup.add", r##"{"name": "G1"}"##, "mode,selection", &[("$Cube", "~topology", "-")]),
    // vertexGroup.remove: the group by name in the mesh
    r("vertexGroup.remove", r##"{"object": "$Cube", "name": "G1"}"##, "", &[("$Cube", "mesh", "-")]),
    r("vertexGroup.remove", r##"{"object": "Cube", "name": "G1"}"##, "", &[("$Cube", "mesh,name", "-")]),
    // was: {}
    r("vertexGroup.remove", r##"{"name": "G1"}"##, "mode,selection", &[("$Cube", "mesh", "-")]),
    // vertexGroup.assign: vertices by index (else the edit selection, edit mode only: no row); weight replaces
    // was: {$Cube mesh/-}
    r("vertexGroup.assign", r##"{"object": "$Cube", "name": "G1", "vertices": [0, 1], "weight": 0.5}"##, "", &[("$Cube", "~topology", "-")]),
    // was: {$Cube mesh,name/-}
    r("vertexGroup.assign", r##"{"object": "Cube", "name": "G1", "vertices": [0, 1]}"##, "", &[("$Cube", "name,~topology", "-")]),
    // was: {}
    r("vertexGroup.assign", r##"{"name": "G1", "vertices": [0, 1]}"##, "mode,selection", &[("$Cube", "~topology", "-")]),
    // vertexGroup.unassign: shares the handler with assign (weight 0; the group must exist: refusal only)
    // was: {$Cube mesh/-}
    r("vertexGroup.unassign", r##"{"object": "$Cube", "name": "G1", "vertices": [0, 1]}"##, "", &[("$Cube", "~topology", "-")]),
    // was: {$Cube mesh,name/-}
    r("vertexGroup.unassign", r##"{"object": "Cube", "name": "G1", "vertices": [0, 1]}"##, "", &[("$Cube", "name,~topology", "-")]),
    // was: {}
    r("vertexGroup.unassign", r##"{"name": "G1", "vertices": [0, 1]}"##, "mode,selection", &[("$Cube", "~topology", "-")]),
    // shapeKey.add: object id-or-name else edited/active mesh; from = another mesh (offsets = its positions - the basis), else offsets, else zeros
    // was: {$Cube mesh/-}
    r("shapeKey.add", r##"{"object": "$Cube", "name": "K"}"##, "", &[("$Cube", "~topology", "-")]),
    // was: {$Cube mesh,name/-}
    r("shapeKey.add", r##"{"object": "Cube", "name": "K"}"##, "", &[("$Cube", "name,~topology", "-")]),
    // was: {}
    r("shapeKey.add", r##"{"name": "K"}"##, "mode,selection", &[("$Cube", "~topology", "-")]),
    // was: {$Child */*, $Cube mesh/-}
    r("shapeKey.add", r##"{"object": "$Cube", "from": "$Child"}"##, "", &[("$Child", "mesh", "-"), ("$Cube", "mesh", "-")]),
    // was: {$Child */*, $Cube mesh/-}
    r("shapeKey.add", r##"{"object": "$Cube", "from": "Child"}"##, "", &[("$Child", "mesh,name", "-"), ("$Cube", "mesh", "-")]),
    // shapeKey.remove: the key by name; prunes its channel from the object's animation
    // was: {$Cube */*}
    r("shapeKey.remove", r##"{"object": "$Cube", "name": "K"}"##, "", &[("$Cube", "animation,mesh", "-")]),
    // was: {$Cube */*}
    r("shapeKey.remove", r##"{"object": "Cube", "name": "K"}"##, "", &[("$Cube", "animation,mesh,name", "-")]),
    // was: {}
    r("shapeKey.remove", r##"{"name": "K"}"##, "mode,selection", &[("$Cube", "animation,mesh", "-")]),
    // shapeKey.set: the key by name; value / rename (renames its channels)
    // was: {$Cube animation,mesh/-}
    r("shapeKey.set", r##"{"object": "$Cube", "name": "K", "value": 0.5}"##, "", &[("$Cube", "mesh", "-")]),
    // was: {$Cube animation,mesh,name/-}
    r("shapeKey.set", r##"{"object": "Cube", "name": "K", "value": 0.5}"##, "", &[("$Cube", "mesh,name", "-")]),
    // was: {}
    r("shapeKey.set", r##"{"name": "K", "value": 0.5}"##, "mode,selection", &[("$Cube", "mesh", "-")]),
    r("shapeKey.set", r##"{"object": "$Cube", "name": "K", "rename": "Smile"}"##, "", &[("$Cube", "animation,mesh", "-")]),
    // shapeKey.offset: adds to the key's offsets of vertices by index (edit-selection path: edit mode only, no row)
    r("shapeKey.offset", r##"{"object": "$Cube", "name": "K", "offset": [0, 0, 1], "vertices": [0]}"##, "", &[("$Cube", "mesh", "-")]),
    r("shapeKey.offset", r##"{"object": "Cube", "name": "K", "offset": [0, 0, 1], "vertices": [0]}"##, "", &[("$Cube", "mesh,name", "-")]),
    // was: {}
    r("shapeKey.offset", r##"{"name": "K", "offset": [0, 0, 1], "vertices": [0]}"##, "mode,selection", &[("$Cube", "mesh", "-")]),
    // shapeKey.insertKey: keys the key's current value; frame else the current frame
    r("shapeKey.insertKey", r##"{"object": "$Cube", "name": "K", "frame": 5}"##, "", &[("$Cube", "animation,mesh", "-")]),
    r("shapeKey.insertKey", r##"{"object": "$Cube", "name": "K"}"##, "time", &[("$Cube", "animation,mesh", "-")]),
    r("shapeKey.insertKey", r##"{"object": "Cube", "name": "K", "frame": 5}"##, "", &[("$Cube", "animation,mesh,name", "-")]),
    // was: {}
    r("shapeKey.insertKey", r##"{"name": "K", "frame": 5}"##, "mode,selection", &[("$Cube", "animation,mesh", "-")]),
    // graph.moveKey: target by id only (object or material); edits one key, then re-evaluates the scene at the current frame (handler-explicit go_to_frame: counted as time)
    // was: {$Cube */*}
    r(
        "graph.moveKey",
        r##"{"target": "$Cube", "property": "location", "index": 0, "frame": 1, "toFrame": 5}"##,
        "time",
        &[("$Cube", "animation", "-")],
    ),
    // was: {$Mat */*}
    r(
        "graph.moveKey",
        r##"{"target": "$Mat", "property": "baseColor", "index": 0, "frame": 1, "value": 0.5}"##,
        "time",
        &[("$Mat", "animation", "-")],
    ),
    // graph.deleteKey: shares fn graph
    // was: {$Cube */*}
    r("graph.deleteKey", r##"{"target": "$Cube", "property": "location", "index": 0, "frame": 1}"##, "time", &[("$Cube", "animation", "-")]),
    // graph.setInterpolation: shares fn graph
    // was: {$Cube */*}
    r(
        "graph.setInterpolation",
        r##"{"target": "$Cube", "property": "location", "index": 0, "frame": 1, "interpolation": "linear"}"##,
        "time",
        &[("$Cube", "animation", "-")],
    ),
    // constraint.add: owner object id-or-name else (pose session's armature or) the active object; bone else the pose session's active bone (mode read even with object); target another object id-or-name
    // was: {$Ball */*, $Cube constraints/*}
    r(
        "constraint.add",
        r##"{"object": "$Cube", "type": "copyLocation", "target": "$Ball"}"##,
        "mode",
        &[("$Ball", "-", "-"), ("$Cube", "constraints", "-")],
    ),
    // was: {$Ball */*, $Cube constraints,name/*}
    r(
        "constraint.add",
        r##"{"object": "Cube", "type": "copyLocation", "target": "Ball"}"##,
        "mode",
        &[("$Ball", "name", "-"), ("$Cube", "constraints,name", "-")],
    ),
    // was: {}
    r("constraint.add", r##"{"type": "limitRotation"}"##, "mode,selection", &[("$Cube", "constraints", "-")]),
    // Child Of: inverse from the target's world
    // was: {$Ball */*, $Cube constraints/*}
    r(
        "constraint.add",
        r##"{"object": "$Cube", "type": "childOf", "target": "$Ball"}"##,
        "mode",
        &[("$Ball", "constraints", "*"), ("$Cube", "constraints", "-")],
    ),
    // a parented target: its ancestor too
    // was: {$Child */*, $Cube constraints/*}
    r(
        "constraint.add",
        r##"{"object": "$Cube", "type": "childOf", "target": "$Child"}"##,
        "mode",
        &[("$Child", "constraints", "*"), ("$Cube", "constraints", "-"), ("$Rig", "constraints", "*")],
    ),
    // Stretch To: rest length from the owner's and the target's worlds
    // was: {$Ball */*, $Cube constraints/*}
    r(
        "constraint.add",
        r##"{"object": "$Cube", "type": "stretchTo", "target": "$Ball"}"##,
        "mode",
        &[("$Ball", "constraints", "*"), ("$Cube", "constraints", "*")],
    ),
    // a bone constraint: the bone's stack is in the armature's data
    // was: {$Arm */*, $Ball */*}
    r(
        "constraint.add",
        r##"{"object": "$Arm", "bone": "Bone", "type": "ik", "target": "$Ball"}"##,
        "mode",
        &[("$Arm", "data", "-"), ("$Ball", "-", "-")],
    ),
    // Action: the action by name in the scene's actions
    // was: {$Cube constraints/*}
    r(
        "constraint.add",
        r##"{"object": "$Cube", "type": "action", "action": "Act"}"##,
        "mode",
        &[("$Cube", "constraints", "-"), ("scene", "actions", "-")],
    ),
    // constraint.set: the constraint by index or name in the owner's stack
    // was: {$Cube constraints.0/*}
    r("constraint.set", r##"{"object": "$Cube", "index": 0, "influence": 0.5}"##, "mode", &[("$Cube", "constraints.0", "-")]),
    // was: {$Cube constraints.0,name/*}
    r("constraint.set", r##"{"object": "Cube", "index": 0, "influence": 0.5}"##, "mode", &[("$Cube", "constraints.0,name", "-")]),
    // was: {}
    r("constraint.set", r##"{"index": 0, "influence": 0.5}"##, "mode,selection", &[("$Cube", "constraints.0", "-")]),
    // was: {$Cube constraints/*}
    r("constraint.set", r##"{"object": "$Cube", "name": "Copy Location", "influence": 0.5}"##, "mode", &[("$Cube", "constraints", "-")]),
    // was: {$Ball */*, $Cube constraints.0/*}
    r("constraint.set", r##"{"object": "$Cube", "index": 0, "target": "$Ball"}"##, "mode", &[("$Ball", "-", "-"), ("$Cube", "constraints.0", "-")]),
    // constraint.remove: removes one entry of the stack (the list shifts)
    // was: {$Cube constraints.0/*}
    r("constraint.remove", r##"{"object": "$Cube", "index": 0}"##, "mode", &[("$Cube", "constraints", "-")]),
    // was: {$Cube constraints.0,name/*}
    r("constraint.remove", r##"{"object": "Cube", "index": 0}"##, "mode", &[("$Cube", "constraints,name", "-")]),
    // was: {}
    r("constraint.remove", r##"{"index": 0}"##, "mode,selection", &[("$Cube", "constraints", "-")]),
    // constraint.move: reorders the stack
    // was: {$Cube */*}
    r("constraint.move", r##"{"object": "$Cube", "index": 0, "to": 1}"##, "mode", &[("$Cube", "constraints", "-")]),
    // was: {$Cube */*}
    r("constraint.move", r##"{"object": "Cube", "index": 0, "to": 1}"##, "mode", &[("$Cube", "constraints,name", "-")]),
    // was: {}
    r("constraint.move", r##"{"index": 0, "to": 1}"##, "mode,selection", &[("$Cube", "constraints", "-")]),
    // constraint.apply: the owner's world with the stack up to the constraint (and its targets) becomes its transform
    // was: {$Cube */*}
    r("constraint.apply", r##"{"object": "$Cube", "index": 0}"##, "mode", &[("$Cube", "constraints", "*")]),
    // was: {$Cube */*}
    r("constraint.apply", r##"{"object": "Cube", "index": 0}"##, "mode", &[("$Cube", "constraints,name", "*")]),
    // was: {}
    r("constraint.apply", r##"{"index": 0}"##, "mode,selection", &[("$Cube", "constraints", "*")]),
    // a parented owner: its ancestor's world too
    // was: {$Child */*}
    r("constraint.apply", r##"{"object": "$Child", "index": 0}"##, "mode", &[("$Child", "constraints", "*"), ("$Rig", "constraints", "*")]),
    // constraint.setInverse: the Child Of's inverse from its target's world (the fixture has no constraint: no target to hold)
    // was: {$Cube */*}
    r("constraint.setInverse", r##"{"object": "$Cube", "index": 0}"##, "mode", &[("$Cube", "constraints.0", "-")]),
    // was: {$Cube */*}
    r("constraint.setInverse", r##"{"object": "Cube", "index": 0}"##, "mode", &[("$Cube", "constraints.0,name", "-")]),
    // was: {}
    r("constraint.setInverse", r##"{"index": 0}"##, "mode,selection", &[("$Cube", "constraints.0", "-")]),
    // constraint.clearInverse: identity inverse (kind check only)
    // was: {$Cube */*}
    r("constraint.clearInverse", r##"{"object": "$Cube", "index": 0}"##, "mode", &[("$Cube", "constraints.0", "-")]),
    // was: {$Cube */*}
    r("constraint.clearInverse", r##"{"object": "Cube", "index": 0}"##, "mode", &[("$Cube", "constraints.0,name", "-")]),
    // was: {}
    r("constraint.clearInverse", r##"{"index": 0}"##, "mode,selection", &[("$Cube", "constraints.0", "-")]),
    // paint.gradient: WEIGHT PAINT session only; the session's mesh, its world, evaluated mesh and the view
    r("paint.gradient", r##"{"from": [10, 10], "to": [100, 100], "kind": "linear", "group": "G1"}"##, "mode", &[]),
    // paint.stroke: WEIGHT PAINT session only; as paint.gradient with the brush and X mirror
    r("paint.stroke", r##"{"points": [[10, 10], [50, 50]], "group": "G1"}"##, "mode", &[]),
    // paint.vertices: object id-or-name else the paint session's mesh else the active mesh; group else the session's paint group; draw replaces, add/subtract read the weight
    // was: {$Cube */*}
    r("paint.vertices", r##"{"object": "$Cube", "group": "G1", "vertices": [0], "weight": 0.5}"##, "", &[("$Cube", "~topology", "-")]),
    // was: {$Cube */*}
    r("paint.vertices", r##"{"object": "Cube", "group": "G1", "vertices": [0]}"##, "", &[("$Cube", "name,~topology", "-")]),
    // was: {$Cube */*}
    r("paint.vertices", r##"{"object": "$Cube", "group": "G1", "vertices": [0], "mode": "add"}"##, "", &[("$Cube", "mesh", "-")]),
    // was: {$Cube */*}
    r("paint.vertices", r##"{"object": "$Cube", "vertices": [0]}"##, "mode", &[("$Cube", "~topology", "-")]),
    // was: {}
    r("paint.vertices", r##"{"group": "G1", "vertices": [0]}"##, "mode,selection", &[("$Cube", "~topology", "-")]),
    // paint.normalizeAll: object id-or-name else paint session's / active mesh; every group's weights; locked group from the paint session
    // was: {$Cube */*}
    r("paint.normalizeAll", r##"{"object": "$Cube"}"##, "mode", &[("$Cube", "mesh", "-")]),
    // was: {$Cube */*}
    r("paint.normalizeAll", r##"{"object": "Cube", "lockActive": false}"##, "", &[("$Cube", "mesh,name", "-")]),
    // was: {$Cube */*}
    r("paint.normalizeAll", r##"{"object": "$Cube", "lockActive": false}"##, "", &[("$Cube", "mesh", "-")]),
    // was: {}
    r("paint.normalizeAll", r##"{}"##, "mode,selection", &[("$Cube", "mesh", "-")]),
    // ---- reading 6: edit.delete, particles, particle edit, curves, texts, lattices ----

    // edit.delete: Object mode; `objects` id-or-name list, else the selection; removes them, reparents their children to the
    // grandparent keeping each child's world place (reads the child's and the deleted parent's world transform), clears every
    // reference (constraints, particles, physics, camera focus, bone shapes); a deleted scene camera is replaced by the FIRST
    // camera in scene.objects order. In Edit mode / a curve, text, lattice or armature session it ignores `objects` and deletes
    // the session's selected elements (the select mode decides faces vs vertices).
    // was: {$Cube */*}
    r("edit.delete", r##"{"objects": ["$Cube"]}"##, "", &[("$Cube", "-", "-")]),
    // was: {}
    r("edit.delete", r##"{}"##, "selection", &[("$Cube", "-", "-")]),
    // was: {$Cube */*}
    r("edit.delete", r##"{"objects": ["Cube"]}"##, "", &[("$Cube", "name", "-")]),
    // A parent's deletion moves its child to stay where it is in the world: the child's and the parent's world transform.
    // was: {$Rig */*}
    r("edit.delete", r##"{"objects": ["$Rig"]}"##, "", &[("$Rig", "constraints", "*"), ("$Child", "constraints", "*")]),
    // The scene camera's deletion picks the first remaining camera in the objects' order.
    // was: {$Camera */*}
    r("edit.delete", r##"{"objects": ["$Camera"]}"##, "", &[("$Camera", "-", "-"), ("scene", "camera,objects", "-")]),
    // `elements`: edit mode on the faces' corners; the editor's select mode decides deleteFaces vs deleteVertices.
    r("edit.delete", r##"{"elements": {"object": "$Cube", "faces": [0]}}"##, "mode", &[("$Cube", "~topology", "-")]),
    // particle.add: `object` id-or-name (a mesh), else the active mesh; appends a new system (its name only made unique);
    // colliders / instanceObject / boid targets are id-or-name and only stored (they must exist and not be the emitter).
    // was: {$Cube */*}
    r("particle.add", r##"{"object": "$Cube", "kind": "hair"}"##, "", &[("$Cube", "-", "-")]),
    // was: {}
    r("particle.add", r##"{"kind": "emitter"}"##, "selection", &[("$Cube", "-", "-")]),
    // was: {$Cube */*}
    r("particle.add", r##"{"object": "Cube"}"##, "", &[("$Cube", "name", "-")]),
    // was: {$Ball */*, $Child */*, $Cube */*}
    r(
        "particle.add",
        r##"{"object": "$Cube", "renderAs": "object", "instanceObject": "$Ball", "colliders": ["$Child"]}"##,
        "",
        &[("$Cube", "-", "-"), ("$Ball", "-", "-"), ("$Child", "-", "-")],
    ),
    // particle.set: object as above; `system` index (default 0) or name into target.particles; merges the given settings
    // into that system's current ones; a change of kind/seed/count/emitFrom/distribution/segments/densityGroup drops the groom.
    // was: {$Cube */*}
    r("particle.set", r##"{"object": "$Cube", "system": 0, "size": 0.1}"##, "", &[("$Cube", "particles.0", "-")]),
    // was: {$Cube */*}
    r("particle.set", r##"{"object": "$Cube", "system": 0, "count": 10}"##, "", &[("$Cube", "hair,particles.0", "-")]),
    // was: {}
    r("particle.set", r##"{"size": 0.1}"##, "selection", &[("$Cube", "particles.0", "-")]),
    // was: {$Cube */*}
    r("particle.set", r##"{"object": "Cube", "size": 0.1}"##, "", &[("$Cube", "name,particles.0", "-")]),
    // was: {$Cube */*}
    r("particle.set", r##"{"object": "$Cube", "system": "ParticleSystem", "size": 0.1}"##, "", &[("$Cube", "particles", "-")]),
    // was: {$Ball */*, $Cube */*}
    r("particle.set", r##"{"object": "$Cube", "colliders": ["$Ball"]}"##, "", &[("$Cube", "particles.0", "-"), ("$Ball", "-", "-")]),
    // particle.remove: object as above; removes system <i> (and its groom); later indexes shift.
    // was: {$Cube */*}
    r("particle.remove", r##"{"object": "$Cube", "system": 0}"##, "", &[("$Cube", "particles.0", "-")]),
    // was: {}
    // corrected by C8-scene-engine: removing a system moves Particle Edit's index past it (or ends it), so it reads the mode (bindings/wasm/src/commands_particles.rs:641)
    r("particle.remove", r##"{}"##, "mode,selection", &[("$Cube", "particles.0", "-")]),
    // was: {$Cube */*}
    r("particle.remove", r##"{"object": "Cube"}"##, "", &[("$Cube", "name,particles.0", "-")]),
    // was: {$Cube */*}
    r("particle.remove", r##"{"object": "$Cube", "system": "ParticleSystem"}"##, "", &[("$Cube", "particles", "-")]),
    // particle.convert: bakes system <i> at the CURRENT frame into a new world-space mesh object: the emitter's evaluated mesh
    // (modifiers), world transform, hidden flag, material, the system and its groom, the emitter's name (new object's name);
    // every other non-hidden object with collision physics or a force field acts on emitter particles.
    // was: {$Cube */*}
    r(
        "particle.convert",
        r##"{"object": "$Cube", "system": 0}"##,
        "time",
        &[
            ("$Cube", "constraints,hair,hidden,material,mesh,modifiers,name,particles.0", "*"),
            ("$Ball", "hidden,physics", "-"),
            ("$Rig", "hidden,physics", "-"),
            ("$Child", "hidden,physics", "-"),
            ("$Light", "hidden,physics", "-"),
            ("$Camera", "hidden,physics", "-"),
            ("$Arm", "hidden,physics", "-"),
            ("$Path", "hidden,physics", "-"),
            ("$Label", "hidden,physics", "-"),
            ("$Cage", "hidden,physics", "-"),
        ],
    ),
    // was: {}
    r(
        "particle.convert",
        r##"{}"##,
        "selection,time",
        &[
            ("$Cube", "constraints,hair,hidden,material,mesh,modifiers,name,particles.0", "*"),
            ("$Ball", "hidden,physics", "-"),
            ("$Rig", "hidden,physics", "-"),
            ("$Child", "hidden,physics", "-"),
            ("$Light", "hidden,physics", "-"),
            ("$Camera", "hidden,physics", "-"),
            ("$Arm", "hidden,physics", "-"),
            ("$Path", "hidden,physics", "-"),
            ("$Label", "hidden,physics", "-"),
            ("$Cage", "hidden,physics", "-"),
        ],
    ),
    // was: {$Cube */*}
    r(
        "particle.convert",
        r##"{"object": "Cube"}"##,
        "time",
        &[
            ("$Cube", "constraints,hair,hidden,material,mesh,modifiers,name,particles.0", "*"),
            ("$Ball", "hidden,physics", "-"),
            ("$Rig", "hidden,physics", "-"),
            ("$Child", "hidden,physics", "-"),
            ("$Light", "hidden,physics", "-"),
            ("$Camera", "hidden,physics", "-"),
            ("$Arm", "hidden,physics", "-"),
            ("$Path", "hidden,physics", "-"),
            ("$Label", "hidden,physics", "-"),
            ("$Cage", "hidden,physics", "-"),
        ],
    ),
    // particleEdit.stroke: Particle Edit only (no object param): the session names object and system; reads the viewport
    // view, the emitter's world transform, evaluated mesh and groom. The fixture cannot start Particle Edit (Cube has no hair
    // system): the session's emitter (constraints,hair,mesh,modifiers,particles.<i>, positions *) cannot be set up here. (C8-scene-engine 4c: the session rows run on its fixture: [`FIXTURES`].)
    r("particleEdit.stroke", r##"{"points": [[100, 100], [120, 100]]}"##, "mode", &[]),
    // particleEdit.reset: Particle Edit only; clears the session system's groom (session emitter: hair). Not set up here. (C8-scene-engine 4c: the session rows run on its fixture: [`FIXTURES`].)
    r("particleEdit.reset", r##"{}"##, "mode", &[]),
    // object.addCurveBezier / object.addCurveCircle / object.addCurvePath / object.addText / object.addLattice: no object params; location defaults to the
    // world origin; the new object is appended and its name only made unique.
    r("object.addCurveBezier", r##"{"radius": 1, "location": [1, 0, 0]}"##, "", &[]),
    r("object.addCurveCircle", r##"{"radius": 1}"##, "", &[]),
    r("object.addCurvePath", r##"{"radius": 1}"##, "", &[]),
    r("object.addText", r##"{"body": "Hi", "size": 1}"##, "", &[]),
    r("object.addLattice", r##"{"resolution": 3}"##, "", &[]),
    // curve.set: `object` id-or-name, else the edited curve, else the active curve; merges settings into the curve's data
    // (resolution goes to every spline, cyclic to every spline).
    // was: {$Path */*}
    r("curve.set", r##"{"object": "$Path", "bevelDepth": 0.1}"##, "", &[("$Path", "data", "-")]),
    // was: {}
    r("curve.set", r##"{"bevelDepth": 0.1}"##, "selection", &[("$Cube", "data", "-")]),
    // was: {$Path */*}
    r("curve.set", r##"{"object": "Path", "resolution": 6}"##, "", &[("$Path", "data,name", "-")]),
    // text.set: `object` id-or-name, else the edited text, else the active text; merges settings into the text's data.
    // was: {$Label */*}
    r("text.set", r##"{"object": "$Label", "size": 2}"##, "", &[("$Label", "data", "-")]),
    // was: {}
    r("text.set", r##"{"size": 2}"##, "selection", &[("$Cube", "data", "-")]),
    // was: {$Label */*}
    r("text.set", r##"{"object": "Label", "body": "Hello"}"##, "", &[("$Label", "data,name", "-")]),
    // object.convertToMesh: `objects` id-or-name, else the selection; each curve/text becomes its evaluated mesh (data with
    // modifiers, bevel/taper objects, followed curve -- none in the fixture) and loses its modifiers; others are skipped.
    // The enable gate reads the selection even when `objects` is given (only refuses: not a read).
    // was: {$Path */*}
    r("object.convertToMesh", r##"{"objects": ["$Path"]}"##, "", &[("$Path", "data,modifiers", "-")]),
    // was: {$Label */*, $Path */*}
    r("object.convertToMesh", r##"{"objects": ["$Path", "$Label"]}"##, "", &[("$Path", "data,modifiers", "-"), ("$Label", "data,modifiers", "-")]),
    // was: {}
    r("object.convertToMesh", r##"{}"##, "selection", &[("$Cube", "data,modifiers", "-")]),
    // was: {$Path */*}
    r("object.convertToMesh", r##"{"objects": ["Path"]}"##, "", &[("$Path", "data,modifiers,name", "-")]),
    // curve.setSplines: `object` as curve.set; replaces the curve's splines (other curve settings kept).
    // was: {$Path */*}
    r(
        "curve.setSplines",
        r##"{"object": "$Path", "splines": [{"type": "poly", "points": [{"co": [0, 0, 0]}, {"co": [1, 0, 0]}]}]}"##,
        "",
        &[("$Path", "data", "-")],
    ),
    // was: {}
    r(
        "curve.setSplines",
        r##"{"splines": [{"type": "bezier", "points": [{"co": [0, 0, 0]}, {"co": [1, 0, 0]}]}]}"##,
        "selection",
        &[("$Cube", "data", "-")],
    ),
    // was: {$Path */*}
    r(
        "curve.setSplines",
        r##"{"object": "Path", "splines": [{"type": "poly", "points": [{"co": [0, 0, 0]}]}]}"##,
        "",
        &[("$Path", "data,name", "-")],
    ),
    // curve.translate / extrude / subdivide / toggleCyclic / setHandleType / deletePoints: curve edit only (no object param):
    // the session names the curve and its selected points/handles; they read and write the curve's data (local axes). The
    // fixture is in object mode: the session's curve (data, positions -) cannot be set up here. (C8-scene-engine 4c: the session rows run on its fixture: [`FIXTURES`].)
    r("curve.translate", r##"{"offset": [0, 0, 1]}"##, "mode", &[]),
    r("curve.extrude", r##"{"offset": [1, 0, 0]}"##, "mode", &[]),
    r("curve.subdivide", r##"{}"##, "mode", &[]),
    r("curve.toggleCyclic", r##"{}"##, "mode", &[]),
    r("curve.setHandleType", r##"{"type": "vector"}"##, "mode", &[]),
    r("curve.deletePoints", r##"{}"##, "mode", &[]),
    // text.insert: `object` id-or-name, else the edited text, else the active text; types at the session's caret when the text
    // edit session is on that same text, else appends at the end of the body.
    // was: {$Label */*}
    r("text.insert", r##"{"object": "$Label", "text": "x"}"##, "mode", &[("$Label", "data", "-")]),
    // was: {}
    r("text.insert", r##"{"text": "x"}"##, "mode,selection", &[("$Cube", "data", "-")]),
    // was: {$Label */*}
    r("text.insert", r##"{"object": "Label", "text": "x"}"##, "mode", &[("$Label", "data,name", "-")]),
    // text.delete: text edit only (no object param): the session's text and caret. Not set up here. (C8-scene-engine 4c: the session rows run on its fixture: [`FIXTURES`].)
    r("text.delete", r##"{"count": 1}"##, "mode", &[]),
    // lattice.set: `object` id-or-name, else the edited lattice, else the active lattice; a new resolution resamples the
    // current point offsets, reset puts the rest points back.
    // was: {$Cage */*}
    r("lattice.set", r##"{"object": "$Cage", "resolution": 3}"##, "", &[("$Cage", "data", "-")]),
    // was: {}
    r("lattice.set", r##"{"interpolation": "linear"}"##, "selection", &[("$Cube", "data", "-")]),
    // was: {$Cage */*}
    r("lattice.set", r##"{"object": "Cage", "reset": true}"##, "", &[("$Cage", "data,name", "-")]),
    // lattice.movePoints: `object` as lattice.set; `points` index the lattice's points (else the edit session's selection on
    // that same lattice); adds offset in the lattice's own axes.
    // was: {$Cage */*}
    r("lattice.movePoints", r##"{"object": "$Cage", "points": [0], "offset": [0, 0, 1]}"##, "", &[("$Cage", "data", "-")]),
    // was: {$Cage */*}
    r("lattice.movePoints", r##"{"object": "$Cage", "offset": [0, 0, 1]}"##, "mode", &[("$Cage", "data", "-")]),
    // was: {}
    r("lattice.movePoints", r##"{"points": [0], "offset": [0, 0, 1]}"##, "selection", &[("$Cube", "data", "-")]),
    // was: {$Cage */*}
    r("lattice.movePoints", r##"{"object": "Cage", "points": [0], "offset": [0, 0, 1]}"##, "", &[("$Cage", "data,name", "-")]),
    // ---- reading 7 ----
    // modifier.add: object = id-or-name else the active mesh (the edit-mode path, which ignores `object`, cannot be set up: no row); appends a modifier; operand/group stored by id, kind checks only refuse (C8-scene-engine 4c: the session rows run on its fixture: [`FIXTURES`].)
    // was: {$Cube kind,modifiers/-}
    r("modifier.add", r##"{"object": "$Cube", "type": "subdivision"}"##, "", &[("$Cube", "-", "-")]),
    // was: {$Cube kind,modifiers,name/-}
    r("modifier.add", r##"{"object": "Cube", "type": "subdivision"}"##, "", &[("$Cube", "name", "-")]),
    // was: {}
    r("modifier.add", r##"{"type": "subdivision"}"##, "selection", &[("$Cube", "-", "-")]),
    // was: {$Ball */*, $Cube kind,modifiers/-}
    r("modifier.add", r##"{"object": "$Cube", "type": "boolean", "operand": "$Ball"}"##, "", &[("$Cube", "-", "-"), ("$Ball", "-", "-")]),
    // was: {$Ball */*, $Cube kind,modifiers/-}
    r("modifier.add", r##"{"object": "$Cube", "type": "boolean", "operand": "Ball"}"##, "", &[("$Cube", "-", "-"), ("$Ball", "name", "-")]),
    // was: {$Cube kind,modifiers/-, $G */*}
    r("modifier.add", r##"{"object": "$Cube", "type": "geometryNodes", "group": "$G"}"##, "", &[("$Cube", "-", "-"), ("$G", "-", "-")]),
    // modifier.set: object = id-or-name else the active object; modifier = index into modifiers or the first with that name; settings merge into that modifier's kind
    r("modifier.set", r##"{"object": "$Cube", "index": 0, "width": 0.2}"##, "", &[("$Cube", "modifiers.0", "-")]),
    r("modifier.set", r##"{"object": "Cube", "index": 0, "width": 0.2}"##, "", &[("$Cube", "modifiers.0,name", "-")]),
    // was: {}
    r("modifier.set", r##"{"index": 0, "width": 0.2}"##, "selection", &[("$Cube", "modifiers.0", "-")]),
    r("modifier.set", r##"{"object": "$Cube", "name": "Bevel", "width": 0.2}"##, "", &[("$Cube", "modifiers", "-")]),
    // was: {$Cube modifiers/-}
    r("modifier.set", r##"{"object": "$Cube", "index": 1, "rename": "Shell", "enabled": false}"##, "", &[("$Cube", "modifiers.1", "-")]),
    // was: {$Ball */*, $Cube modifiers.0/-}
    r("modifier.set", r##"{"object": "$Cube", "index": 0, "operand": "$Ball"}"##, "", &[("$Cube", "modifiers.0", "-"), ("$Ball", "-", "-")]),
    // mesh.* (rows below): elements.object = id-or-name, faces/vertices = element indexes; runs in that mesh's edit mode and restores it.
    // The plain edit-mode paths (the edited mesh + its edit selection) cannot be set up in object mode: no rows. (C8-scene-engine 4c: the session rows run on its fixture: [`FIXTURES`].)
    // mesh.extrudeIndividual: extrudes the selected faces along their own normals
    r("mesh.extrudeIndividual", r##"{"elements": {"object": "$Cube", "faces": [0]}, "distance": 0.5}"##, "", &[("$Cube", "~topology", "-")]),
    r("mesh.extrudeIndividual", r##"{"elements": {"object": "Cube", "faces": [0]}}"##, "", &[("$Cube", "name,~topology", "-")]),
    // mesh.dissolveVertices / Edges / Faces: dissolve the selected elements
    r("mesh.dissolveVertices", r##"{"elements": {"object": "$Cube", "faces": [0]}}"##, "", &[("$Cube", "~topology", "-")]),
    r("mesh.dissolveEdges", r##"{"elements": {"object": "$Cube", "faces": [0]}}"##, "", &[("$Cube", "~topology", "-")]),
    r("mesh.dissolveFaces", r##"{"elements": {"object": "$Cube", "faces": [0, 1]}}"##, "", &[("$Cube", "~topology", "-")]),
    // mesh.bridge: bridges the selected loops
    r("mesh.bridge", r##"{"elements": {"object": "$Cube", "faces": [0]}}"##, "", &[("$Cube", "~topology", "-")]),
    // mesh.bisect: cuts the WHOLE mesh by a local plane; the default point is the selection's centre (positions)
    r("mesh.bisect", r##"{"elements": {"object": "$Cube", "faces": [0]}}"##, "", &[("$Cube", "~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    r(
        "mesh.bisect",
        r##"{"elements": {"object": "$Cube", "faces": [0]}, "point": [0, 0, 0], "normal": [1, 0, 0]}"##,
        "",
        &[("$Cube", "~topology", "-")],
    ), // normalized after review: `mesh` -> `~topology`
    // mesh.separate: splits the selection off into a new object, copying the position fields, material and modifiers; its name defaults to the original's
    // was: {$Cube */*}
    r("mesh.separate", r##"{"elements": {"object": "$Cube", "faces": [0]}}"##, "", &[("$Cube", "material,modifiers,name,~topology", "*")]), // normalized after review: `mesh` -> `~topology`
    // was: {$Cube */*}
    r("mesh.separate", r##"{"elements": {"object": "$Cube", "faces": [0]}, "name": "Part"}"##, "", &[("$Cube", "material,modifiers,~topology", "*")]), // normalized after review: `mesh` -> `~topology`
    // mesh.symmetrize: mirrors the whole mesh (ignores the elements' selection)
    r("mesh.symmetrize", r##"{"elements": {"object": "$Cube", "faces": [0]}, "axis": "x"}"##, "", &[("$Cube", "~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    // mesh.spin: sweeps the selection round a local axis through `center` (default: the local origin)
    r("mesh.spin", r##"{"elements": {"object": "$Cube", "faces": [0]}, "steps": 4}"##, "", &[("$Cube", "~topology", "-")]),
    // mesh.bevelVertices: bevels the selected vertices
    r("mesh.bevelVertices", r##"{"elements": {"object": "$Cube", "vertices": [0]}}"##, "", &[("$Cube", "~topology", "-")]),
    // mesh.rotate: turns the selection about `pivot` (default: the selection's centre, from positions); proportional editing (session tool state) can also move unselected vertices
    r("mesh.rotate", r##"{"elements": {"object": "$Cube", "faces": [0]}, "axis": "z", "angle": 45}"##, "", &[("$Cube", "~topology", "-")]), // normalized after review: `mesh` -> `~topology`
    r(
        "mesh.rotate",
        r##"{"elements": {"object": "$Cube", "faces": [0]}, "axis": "z", "angle": 45, "pivot": [0, 0, 0]}"##,
        "",
        &[("$Cube", "~topology", "-")],
    ),
    // world.setImage: sets world.image/rotation/strength (the image from a host path or base64)
    // was: {}
    r("world.setImage", r##"{"base64": "iVBORw0KGgo=", "name": "Sky"}"##, "", &[("scene", "world", "-")]),
    // was: {}
    r("world.setImage", r##"{"clear": true}"##, "", &[("scene", "world", "-")]),
    // was: {}
    r("world.setImage", r##"{"rotation": 90, "strength": 2}"##, "", &[("scene", "world", "-")]),
    // node.newGroup: appends a group; geometry: may add a Geometry Nodes modifier to `object` (id-or-name) or set modifiers.<modifier>; shader: sets the material's (id-or-name) shader or world.volume
    r("node.newGroup", r##"{"name": "H"}"##, "", &[]),
    // was: {$Cube */*}
    r("node.newGroup", r##"{"name": "H", "object": "$Cube"}"##, "", &[("$Cube", "-", "-")]),
    // was: {$Cube */*}
    r("node.newGroup", r##"{"name": "H", "object": "Cube"}"##, "", &[("$Cube", "name", "-")]),
    // was: {$Cube */*}
    r("node.newGroup", r##"{"name": "H", "object": "$Cube", "modifier": 0}"##, "", &[("$Cube", "modifiers.0", "-")]),
    // was: {$Mat */*}
    r("node.newGroup", r##"{"tree": "shader", "material": "$Mat"}"##, "", &[("$Mat", "data", "-")]),
    // was: {$Mat */*}
    r("node.newGroup", r##"{"tree": "shader", "material": "Mat"}"##, "", &[("$Mat", "data,name", "-")]),
    // was: {}
    r("node.newGroup", r##"{"tree": "shader", "world": true}"##, "", &[("scene", "world", "-")]),
    // nodes.* below: group = id, or a name among all groups, or the only group when absent (the fixture has two groups, so the absent path only refuses: no row).
    // node.removeGroup: removes the group; the references to it (Group nodes, render.compositor, materials, world, modifiers) are cleared as they are at write time
    // was: {$G */*}
    r("node.removeGroup", r##"{"group": "$G"}"##, "", &[("$G", "-", "-")]),
    // was: {$G */*}
    r("node.removeGroup", r##"{"group": "G"}"##, "", &[("$G", "name", "-")]),
    // node.renameGroup: replaces the name
    // was: {$G */*}
    r("node.renameGroup", r##"{"group": "$G", "name": "H"}"##, "", &[("$G", "name", "-")]),
    // was: {$G */*}
    r("node.renameGroup", r##"{"group": "G", "name": "H"}"##, "", &[("$G", "name", "-")]),
    // node.addNode: appends a node of `kind`; the values are checked against the kind (tree and count checks only refuse)
    // was: {$G */*}
    r("node.addNode", r##"{"group": "$G", "kind": "setPosition", "location": [0, 0]}"##, "", &[("$G", "-", "-")]),
    // was: {$G */*}
    r("node.addNode", r##"{"group": "G", "kind": "setPosition"}"##, "", &[("$G", "name", "-")]),
    // node.removeNodes: removes the nodes by id (and their zone partners) and their links
    // was: {$G */*, $GOut */*}
    r("node.removeNodes", r##"{"group": "$G", "nodes": ["$GOut"]}"##, "", &[("$G", "-", "-"), ("$GOut", "*", "-")]), // node roles put in after review (the reading used a placeholder id and held the group's `nodes` for it): the node named by id
    // node.moveNodes: offset moves from each node's location; location replaces it
    // was: {$G */*, $GOut */*}
    r("node.moveNodes", r##"{"group": "$G", "nodes": ["$GOut"], "offset": [10, 0]}"##, "", &[("$G", "-", "-"), ("$GOut", "*", "-")]), // node roles put in after review (the reading used a placeholder id and held the group's `nodes` for it): the node named by id
    // was: {$G */*, $GOut */*}
    r("node.moveNodes", r##"{"group": "$G", "nodes": ["$GOut"], "location": [10, 0]}"##, "", &[("$G", "-", "-"), ("$GOut", "*", "-")]), // node roles put in after review (the reading used a placeholder id and held the group's `nodes` for it): the node named by id
    // node.setValue: sets one value of a node, checked against its kind (and the group interface for dynamic sockets)
    // was: {$G */*, $GOut */*}
    r(
        "node.setValue",
        r##"{"group": "$G", "node": "$GOut", "id": "Offset", "value": [0, 0, 1]}"##,
        "",
        &[("$G", "inputs", "-"), ("$GOut", "*", "-")],
    ), // node roles put in after review (the reading used a placeholder id and held the group's `nodes` for it): the node named by id
    // node.link: socket types from the nodes (and the group interface), a cycle check on the links, and a single input's old link replaced
    // was: {$G */*, $GIn */*, $GOut */*}
    r(
        "node.link",
        r##"{"group": "$G", "from": "$GIn", "fromSocket": "Geometry", "to": "$GOut", "toSocket": "Geometry"}"##,
        "",
        &[("$G", "inputs,links,outputs", "-"), ("$GOut", "*", "-"), ("$GIn", "*", "-")],
    ), // node roles put in after review (the reading used a placeholder id and held the group's `nodes` for it): the node named by id
    // node.unlink: removes the matching links
    // was: {$G */*, $GOut */*}
    r("node.unlink", r##"{"group": "$G", "to": "$GOut", "toSocket": "Geometry"}"##, "", &[("$G", "links", "-"), ("$GOut", "*", "-")]), // node roles put in after review (the reading used a placeholder id and held the group's `nodes` for it): the node named by id
    // node.addInput / addOutput: append an interface socket (the duplicate check only refuses); the default is stored as given
    // was: {$G */*}
    r("node.addInput", r##"{"group": "$G", "name": "Size", "type": "float", "default": 1}"##, "", &[("$G", "-", "-")]),
    // was: {$Ball */*, $G */*}
    r("node.addInput", r##"{"group": "$G", "name": "Target", "type": "object", "default": "$Ball"}"##, "", &[("$G", "-", "-"), ("$Ball", "-", "-")]),
    // was: {$G */*}
    r("node.addOutput", r##"{"group": "$G", "name": "Mask", "type": "float"}"##, "", &[("$G", "-", "-")]),
    // node.removeInput / removeOutput: remove the socket by name; the links that no longer fit go (as they are at write time)
    // was: {$G */*}
    r("node.removeInput", r##"{"group": "$G", "name": "Size"}"##, "", &[("$G", "inputs", "-")]),
    // was: {$G */*}
    r("node.removeOutput", r##"{"group": "$G", "name": "Mask"}"##, "", &[("$G", "outputs", "-")]),
    // node.addZoneItem / removeZoneItem: the zone's output node (from either zone node) gets or loses an item
    // was: {$G */*, $GOut */*}
    r("node.addZoneItem", r##"{"group": "$G", "node": "$GOut", "name": "V", "type": "float"}"##, "", &[("$G", "nodes", "-"), ("$GOut", "*", "-")]), // node roles put in after review (the reading used a placeholder id and held the group's `nodes` for it): the node named by id
    // was: {$G */*, $GOut */*}
    r("node.removeZoneItem", r##"{"group": "$G", "node": "$GOut", "name": "V"}"##, "", &[("$G", "nodes", "-"), ("$GOut", "*", "-")]), // node roles put in after review (the reading used a placeholder id and held the group's `nodes` for it): the node named by id
    // compositor.new: appends a compositing group; with use (default), sets render.compositor and compositing
    // was: {}
    r("compositor.new", r##"{}"##, "", &[("scene", "render", "-")]),
    r("compositor.new", r##"{"use": false}"##, "", &[]),
    // compositor.set: group = id or name (compositing tree only) or null; compositing flag
    // was: {$Comp */*}
    r("compositor.set", r##"{"group": "$Comp"}"##, "", &[("$Comp", "-", "-"), ("scene", "render", "-")]),
    // was: {$Comp */*}
    r("compositor.set", r##"{"group": "Compositing"}"##, "", &[("$Comp", "name", "-"), ("scene", "render", "-")]),
    // was: {}
    r("compositor.set", r##"{"group": null, "compositing": false}"##, "", &[("scene", "render", "-")]),
    // compositor.setImage: group = given, else render.compositor, else the only compositing group; node = an Image node's id; the image from base64/path/hash
    // was: {$Comp */*, $CompIn */*}
    r(
        "compositor.setImage",
        r##"{"group": "$Comp", "node": "$CompIn", "base64": "iVBORw0KGgo="}"##,
        "",
        &[("$Comp", "-", "-"), ("$CompIn", "*", "-")],
    ), // node roles put in after review (the reading used a placeholder id and held the group's `nodes` for it): the node named by id
    // was: {$Comp */*, $CompIn */*}
    r("compositor.setImage", r##"{"group": "Compositing", "node": "$CompIn", "clear": true}"##, "", &[("$Comp", "name", "-"), ("$CompIn", "*", "-")]), // node roles put in after review (the reading used a placeholder id and held the group's `nodes` for it): the node named by id
    // was: {$CompIn */*}
    r(
        "compositor.setImage",
        r##"{"node": "$CompIn", "base64": "iVBORw0KGgo="}"##,
        "",
        &[("$Comp", "-", "-"), ("scene", "render", "-"), ("$CompIn", "*", "-")],
    ), // node roles put in after review (the reading used a placeholder id and held the group's `nodes` for it): the node named by id
    // compositor.pickMatte: the name under (x, y) of a render through scene.camera of the whole scene as it stands, added to (or taken from) the Cryptomatte node's Matte ID
    // was: {$Comp */*, $CompIn */*}
    r(
        "compositor.pickMatte",
        r##"{"group": "$Comp", "node": "$CompIn", "x": 0.5, "y": 0.5}"##,
        // corrected after review: the render is of the scene at the current frame (`time`); the node named by id
        "time",
        // corrected by C8-scene-engine: what is under the point is a place told by `covers` (the guard
        // asks the kit to cast the ray again with what others moved or changed put back); held: the
        // object under it now (`$Cube`), the fields its names are read from (its name and material
        // slot, its parents' names through `parent`), its material's name, and the camera and the
        // render size the ray is cast through; the world and lights decide no hit
        // (bindings/wasm/src/commands_compositor.rs:489, bindings/wasm/src/rests.rs:1433)
        &[("$Comp", "-", "-"), ("$CompIn", "*", "-"), ("scene", "camera,render", "-"), ("$Cube", "material,name", "parent"), ("$Mat", "name", "-")],
    ),
    // camera.move: camera = id-or-name else scene.camera; start = param else the current frame; the camera at start (lens, clip, local and world transform); target/to = an object (world bbox centre at start, evaluated mesh + modifiers, hidden) or a point; keys location/rotation (lens), widens timeline.end; whip turns on render.motion_blur
    // was: {$Cube */*}
    r(
        "camera.move",
        r##"{"preset": "pushIn", "target": "$Cube"}"##,
        "time",
        &[
            ("$Camera", "animation,constraints,data,nla,tweak", "*"),
            ("$Cube", "animation,constraints,hidden,mesh,modifiers,nla", "*"),
            ("scene", "camera,timeline", "-"),
        ],
    ),
    // was: {$Cube */*}
    r(
        "camera.move",
        r##"{"preset": "pushIn", "target": "Cube", "start": 1}"##,
        "",
        &[
            ("$Camera", "animation,constraints,data,nla,tweak", "*"),
            ("$Cube", "animation,constraints,hidden,mesh,modifiers,name,nla", "*"),
            ("scene", "camera,timeline", "-"),
        ],
    ),
    // was: {$Camera */*}
    r(
        "camera.move",
        r##"{"preset": "orbit", "target": [0, 0, 0], "camera": "$Camera", "start": 1}"##,
        "",
        &[("$Camera", "animation,constraints,data,nla,tweak", "*"), ("scene", "timeline", "-")],
    ),
    // was: {$Camera */*, $Child */*}
    r(
        "camera.move",
        r##"{"preset": "pushIn", "target": "$Child", "camera": "Camera", "start": 1}"##,
        "",
        &[
            ("$Camera", "animation,constraints,data,name,nla,tweak", "*"),
            ("$Child", "animation,constraints,hidden,mesh,modifiers,nla", "*"),
            ("$Rig", "constraints", "*"),
            ("scene", "timeline", "-"),
        ],
    ),
    // was: {$Ball */*}
    r(
        "camera.move",
        r##"{"preset": "whip", "to": "$Ball", "start": 1}"##,
        "",
        &[
            ("$Camera", "animation,constraints,data,nla,tweak", "*"),
            ("$Ball", "animation,constraints,hidden,mesh,modifiers,nla", "*"),
            ("scene", "camera,render,timeline", "-"),
        ],
    ),
    // was: {$Ball */*}
    r(
        "camera.move",
        r##"{"preset": "follow", "target": "$Ball", "start": 1}"##,
        "",
        &[
            ("$Camera", "animation,constraints,data,nla,tweak", "*"),
            ("$Ball", "animation,constraints,hidden,mesh,modifiers,nla", "*"),
            ("scene", "camera,timeline", "-"),
        ],
    ),
    // camera.shake: camera = id-or-name else scene.camera (must be a camera); frame = param else the current frame; overlays on the camera's local location/rotation evaluated at each frame between its nearest keys
    // was: {}
    r("camera.shake", r##"{}"##, "time", &[("$Camera", "animation,nla,tweak", "*"), ("scene", "camera,timeline", "-")]),
    // was: {$Camera */*}
    r("camera.shake", r##"{"camera": "$Camera", "frame": 1}"##, "", &[("$Camera", "animation,nla,tweak", "*"), ("scene", "timeline", "-")]),
    // was: {$Camera */*}
    r("camera.shake", r##"{"camera": "Camera", "frame": 1}"##, "", &[("$Camera", "animation,name,nla,tweak", "*"), ("scene", "timeline", "-")]),
    // camera.handheld: as shake; start/end default to the timeline's start/end (not the current frame)
    // was: {}
    r("camera.handheld", r##"{}"##, "", &[("$Camera", "animation,nla,tweak", "*"), ("scene", "camera,timeline", "-")]),
    // was: {$Camera */*}
    r(
        "camera.handheld",
        r##"{"camera": "$Camera", "start": 1, "end": 48}"##,
        "",
        &[("$Camera", "animation,nla,tweak", "*"), ("scene", "timeline", "-")],
    ),
    // light.studio: target = an object (world bbox centre in the scene as it stands) or a point; aimed from scene.camera's world place; makes or reuses Key/Fill/Rim Light; sets world.strength, render AO and colour management
    // was: {$Cube */*}
    r(
        "light.studio",
        r##"{"target": "$Cube"}"##,
        "",
        &[("$Cube", "constraints,hidden,mesh,modifiers", "*"), ("$Camera", "constraints", "*"), ("scene", "camera,render,world", "-")],
    ),
    // was: {$Cube */*}
    r(
        "light.studio",
        r##"{"target": "Cube", "preset": "soft"}"##,
        "",
        &[("$Cube", "constraints,hidden,mesh,modifiers,name", "*"), ("$Camera", "constraints", "*"), ("scene", "camera,render,world", "-")],
    ),
    // was: {}
    r("light.studio", r##"{"target": [0, 0, 0], "distance": 4}"##, "", &[("$Camera", "constraints", "*"), ("scene", "camera,render,world", "-")]),
    // object.drop: objects = ids-or-names else the selection; start = param else the current frame; keys from each object's local location/scale at start
    // was: {$Cube */*}
    r("object.drop", r##"{"objects": ["$Cube"], "start": 1}"##, "", &[("$Cube", "animation,nla,tweak", "*"), ("scene", "timeline", "-")]),
    // was: {$Cube */*}
    r("object.drop", r##"{"objects": ["Cube"], "start": 1}"##, "", &[("$Cube", "animation,name,nla,tweak", "*"), ("scene", "timeline", "-")]),
    // was: {}
    r("object.drop", r##"{}"##, "selection,time", &[("$Cube", "animation,nla,tweak", "*"), ("scene", "timeline", "-")]),
    // object.pop: as drop, keys scale from each local scale at start
    // was: {$Ball */*, $Cube */*}
    r(
        "object.pop",
        r##"{"objects": ["$Cube", "$Ball"], "start": 1}"##,
        "",
        &[("$Cube", "animation,nla,tweak", "*"), ("$Ball", "animation,nla,tweak", "*"), ("scene", "timeline", "-")],
    ),
    // was: {}
    r("object.pop", r##"{}"##, "selection,time", &[("$Cube", "animation,nla,tweak", "*"), ("scene", "timeline", "-")]),
    // object.hop: the objects' world bounds at start; to = a point, or an object (id-or-name) whose world bounds at the landing frame it lands on
    // was: {$Cube */*}
    r(
        "object.hop",
        r##"{"objects": ["$Cube"], "to": [3, 0, 0], "start": 1}"##,
        "",
        &[("$Cube", "animation,constraints,hidden,mesh,modifiers,nla,tweak", "*"), ("scene", "timeline", "-")],
    ),
    // was: {$Ball */*, $Cube */*}
    r(
        "object.hop",
        r##"{"objects": ["$Cube"], "to": "$Ball", "start": 1}"##,
        "",
        &[
            ("$Cube", "animation,constraints,hidden,mesh,modifiers,nla,tweak", "*"),
            ("$Ball", "animation,constraints,hidden,mesh,modifiers,nla", "*"),
            ("scene", "timeline", "-"),
        ],
    ),
    // was: {$Ball */*}
    r(
        "object.hop",
        r##"{"to": "Ball"}"##,
        "selection,time",
        &[
            ("$Cube", "animation,constraints,hidden,mesh,modifiers,nla,tweak", "*"),
            ("$Ball", "animation,constraints,hidden,mesh,modifiers,name,nla", "*"),
            ("scene", "timeline", "-"),
        ],
    ),
    // object.gather: center = a point or an object's world bbox centre at start; each object's local location/rotation at start; radius 0 packs by world bounds
    // was: {$Ball */*, $Cube */*}
    r(
        "object.gather",
        r##"{"objects": ["$Cube", "$Ball"], "center": [0, 0, 0], "start": 1}"##,
        "",
        &[("$Cube", "animation,nla,tweak", "*"), ("$Ball", "animation,nla,tweak", "*"), ("scene", "timeline", "-")],
    ),
    // was: {$Ball */*, $Cube */*, $Rig */*}
    r(
        "object.gather",
        r##"{"objects": ["$Cube", "$Ball"], "center": "$Rig", "radius": 0, "start": 1}"##,
        "",
        &[
            ("$Cube", "animation,constraints,hidden,mesh,modifiers,nla,tweak", "*"),
            ("$Ball", "animation,constraints,hidden,mesh,modifiers,nla,tweak", "*"),
            ("$Rig", "animation,constraints,hidden,nla", "*"),
            ("scene", "timeline", "-"),
        ],
    ),
    // was: {}
    r("object.gather", r##"{"center": [0, 0, 0]}"##, "selection,time", &[("$Cube", "animation,nla,tweak", "*"), ("scene", "timeline", "-")]),
    // object.wave: origin = a point or an object's world bbox centre at start; each object's world bbox centre; scale: its local scale; color: its material's base colour (keys on the material)
    // was: {$Cube */*}
    r(
        "object.wave",
        r##"{"objects": ["$Cube"], "origin": [0, 0, 0], "start": 1}"##,
        "",
        &[("$Cube", "animation,constraints,hidden,mesh,modifiers,nla,tweak", "*"), ("scene", "timeline", "-")],
    ),
    // was: {$Ball */*, $Cube */*}
    r(
        "object.wave",
        r##"{"objects": ["$Cube"], "origin": "$Ball", "property": "color", "color": "#ff0000", "start": 1}"##,
        "",
        &[
            ("$Cube", "animation,constraints,hidden,material,mesh,modifiers,nla", "*"),
            ("$Mat", "animation,data", "-"),
            ("$Ball", "animation,constraints,hidden,mesh,modifiers,nla", "*"),
            ("scene", "timeline", "-"),
        ],
    ),
    // was: {}
    r(
        "object.wave",
        r##"{"origin": [0, 0, 0]}"##,
        "selection,time",
        &[("$Cube", "animation,constraints,hidden,mesh,modifiers,nla,tweak", "*"), ("scene", "timeline", "-")],
    ),
    // physics.world.set: no object params; merges the given keys into scene.physics and range-checks the merged world
    // was: {}
    r("physics.world.set", r##"{"gravity": [0, 0, -5], "substeps": 20}"##, "", &[("scene", "physics", "-")]),
    // physics.rigidBody.add: objects = id-or-name, else the selection; creates or merges into target.physics.rigidBody (mesh-only kind check refuses only); reads only target.physics; position-free
    // was: {$Cube */*}
    r("physics.rigidBody.add", r##"{"objects": ["$Cube"], "type": "active", "mass": 2}"##, "", &[("$Cube", "physics", "-")]),
    // was: {}
    r("physics.rigidBody.add", r##"{"type": "active", "mass": 2}"##, "selection", &[("$Cube", "physics", "-")]),
    // was: {$Cube */*}
    r("physics.rigidBody.add", r##"{"objects": ["Cube"], "type": "active", "mass": 2}"##, "", &[("$Cube", "name,physics", "-")]),
    // physics.rigidBody.set: objects = id-or-name, else the selection; merges into the existing target.physics.rigidBody (missing -> not-found); reads only target.physics; position-free
    // was: {$Cube */*}
    r("physics.rigidBody.set", r##"{"objects": ["$Cube"], "mass": 2}"##, "", &[("$Cube", "physics", "-")]),
    // was: {}
    r("physics.rigidBody.set", r##"{"mass": 2}"##, "selection", &[("$Cube", "physics", "-")]),
    // was: {$Cube */*}
    r("physics.rigidBody.set", r##"{"objects": ["Cube"], "mass": 2}"##, "", &[("$Cube", "name,physics", "-")]),
    // physics.rigidBody.remove: objects = id-or-name, else the selection; clears target.physics.rigidBody; prune drops physics when nothing is left; reads only target.physics; position-free
    // was: {$Cube */*}
    r("physics.rigidBody.remove", r##"{"objects": ["$Cube"]}"##, "", &[("$Cube", "physics", "-")]),
    // was: {}
    r("physics.rigidBody.remove", r##"{}"##, "selection", &[("$Cube", "physics", "-")]),
    // was: {$Cube */*}
    r("physics.rigidBody.remove", r##"{"objects": ["Cube"]}"##, "", &[("$Cube", "name,physics", "-")]),
    // physics.cloth.add: objects = id-or-name, else the selection; creates or merges target.physics.cloth; refuses on a soft body; pinGroup is a stored name, not checked against the mesh (not held); reads only target.physics; position-free
    // was: {$Cube */*}
    r("physics.cloth.add", r##"{"objects": ["$Cube"], "quality": 10, "pinGroup": "Pin"}"##, "", &[("$Cube", "physics", "-")]),
    // was: {}
    r("physics.cloth.add", r##"{"quality": 10, "pinGroup": "Pin"}"##, "selection", &[("$Cube", "physics", "-")]),
    // was: {$Cube */*}
    r("physics.cloth.add", r##"{"objects": ["Cube"], "quality": 10, "pinGroup": "Pin"}"##, "", &[("$Cube", "name,physics", "-")]),
    // physics.cloth.set: objects = id-or-name, else the selection; merges into the existing target.physics.cloth; refuses on a soft body; reads only target.physics; position-free
    // was: {$Cube */*}
    r("physics.cloth.set", r##"{"objects": ["$Cube"], "tension": 20}"##, "", &[("$Cube", "physics", "-")]),
    // was: {}
    r("physics.cloth.set", r##"{"tension": 20}"##, "selection", &[("$Cube", "physics", "-")]),
    // was: {$Cube */*}
    r("physics.cloth.set", r##"{"objects": ["Cube"], "tension": 20}"##, "", &[("$Cube", "name,physics", "-")]),
    // physics.cloth.remove: objects = id-or-name, else the selection; clears target.physics.cloth (+prune); reads only target.physics; position-free
    // was: {$Cube */*}
    r("physics.cloth.remove", r##"{"objects": ["$Cube"]}"##, "", &[("$Cube", "physics", "-")]),
    // was: {}
    r("physics.cloth.remove", r##"{}"##, "selection", &[("$Cube", "physics", "-")]),
    // was: {$Cube */*}
    r("physics.cloth.remove", r##"{"objects": ["Cube"]}"##, "", &[("$Cube", "name,physics", "-")]),
    // physics.softBody.add: objects = id-or-name, else the selection; creates or merges target.physics.softBody; refuses on cloth; goalGroup is a stored name, not checked (not held); reads only target.physics; position-free
    // was: {$Cube */*}
    r("physics.softBody.add", r##"{"objects": ["$Cube"], "mass": 2, "goalGroup": "Goal"}"##, "", &[("$Cube", "physics", "-")]),
    // was: {}
    r("physics.softBody.add", r##"{"mass": 2, "goalGroup": "Goal"}"##, "selection", &[("$Cube", "physics", "-")]),
    // was: {$Cube */*}
    r("physics.softBody.add", r##"{"objects": ["Cube"], "mass": 2, "goalGroup": "Goal"}"##, "", &[("$Cube", "name,physics", "-")]),
    // physics.softBody.set: objects = id-or-name, else the selection; merges into the existing target.physics.softBody; refuses on cloth; reads only target.physics; position-free
    // was: {$Cube */*}
    r("physics.softBody.set", r##"{"objects": ["$Cube"], "goalDefault": 0.5}"##, "", &[("$Cube", "physics", "-")]),
    // was: {}
    r("physics.softBody.set", r##"{"goalDefault": 0.5}"##, "selection", &[("$Cube", "physics", "-")]),
    // was: {$Cube */*}
    r("physics.softBody.set", r##"{"objects": ["Cube"], "goalDefault": 0.5}"##, "", &[("$Cube", "name,physics", "-")]),
    // physics.softBody.remove: objects = id-or-name, else the selection; clears target.physics.softBody (+prune); reads only target.physics; position-free
    // was: {$Cube */*}
    r("physics.softBody.remove", r##"{"objects": ["$Cube"]}"##, "", &[("$Cube", "physics", "-")]),
    // was: {}
    r("physics.softBody.remove", r##"{}"##, "selection", &[("$Cube", "physics", "-")]),
    // was: {$Cube */*}
    r("physics.softBody.remove", r##"{"objects": ["Cube"]}"##, "", &[("$Cube", "name,physics", "-")]),
    // physics.collision.add: objects = id-or-name, else the selection; creates or merges target.physics.collision (no .set: add acts as set); reads only target.physics; position-free
    // was: {$Cube */*}
    r("physics.collision.add", r##"{"objects": ["$Cube"], "thickness": 0.05}"##, "", &[("$Cube", "physics", "-")]),
    // was: {}
    r("physics.collision.add", r##"{"thickness": 0.05}"##, "selection", &[("$Cube", "physics", "-")]),
    // was: {$Cube */*}
    r("physics.collision.add", r##"{"objects": ["Cube"], "thickness": 0.05}"##, "", &[("$Cube", "name,physics", "-")]),
    // physics.collision.remove: objects = id-or-name, else the selection; clears target.physics.collision (+prune); reads only target.physics; position-free
    // was: {$Cube */*}
    r("physics.collision.remove", r##"{"objects": ["$Cube"]}"##, "", &[("$Cube", "physics", "-")]),
    // was: {}
    r("physics.collision.remove", r##"{}"##, "selection", &[("$Cube", "physics", "-")]),
    // was: {$Cube */*}
    r("physics.collision.remove", r##"{"objects": ["Cube"]}"##, "", &[("$Cube", "name,physics", "-")]),
    // physics.field.add: objects = id-or-name, else the selection; creates or merges target.physics.field; any object kind; reads only target.physics; position-free
    // was: {$Cube */*}
    r("physics.field.add", r##"{"objects": ["$Cube"], "type": "wind", "strength": 2}"##, "", &[("$Cube", "physics", "-")]),
    // was: {}
    r("physics.field.add", r##"{"type": "wind", "strength": 2}"##, "selection", &[("$Cube", "physics", "-")]),
    // was: {$Cube */*}
    r("physics.field.add", r##"{"objects": ["Cube"], "type": "wind", "strength": 2}"##, "", &[("$Cube", "name,physics", "-")]),
    // physics.field.set: objects = id-or-name, else the selection; merges into the existing target.physics.field; reads only target.physics; position-free
    // was: {$Cube */*}
    r("physics.field.set", r##"{"objects": ["$Cube"], "strength": 2}"##, "", &[("$Cube", "physics", "-")]),
    // was: {}
    r("physics.field.set", r##"{"strength": 2}"##, "selection", &[("$Cube", "physics", "-")]),
    // was: {$Cube */*}
    r("physics.field.set", r##"{"objects": ["Cube"], "strength": 2}"##, "", &[("$Cube", "name,physics", "-")]),
    // physics.field.remove: objects = id-or-name, else the selection; clears target.physics.field (+prune); reads only target.physics; position-free
    // was: {$Cube */*}
    r("physics.field.remove", r##"{"objects": ["$Cube"]}"##, "", &[("$Cube", "physics", "-")]),
    // was: {}
    r("physics.field.remove", r##"{}"##, "selection", &[("$Cube", "physics", "-")]),
    // was: {$Cube */*}
    r("physics.field.remove", r##"{"objects": ["Cube"]}"##, "", &[("$Cube", "name,physics", "-")]),
    // physics.constraint.remove: objects = id-or-name, else the selection; clears target.physics.rigidConstraint (+prune); reads only target.physics; position-free
    // was: {$Cube */*}
    r("physics.constraint.remove", r##"{"objects": ["$Cube"]}"##, "", &[("$Cube", "physics", "-")]),
    // was: {}
    r("physics.constraint.remove", r##"{}"##, "selection", &[("$Cube", "physics", "-")]),
    // was: {$Cube */*}
    r("physics.constraint.remove", r##"{"objects": ["Cube"]}"##, "", &[("$Cube", "name,physics", "-")]),
    // physics.constraint.add: objects = id-or-name, else the selection (the constraint's owner; merges into target.physics.rigidConstraint); object1/object2 = other objects by id or name, any kind, existence only (they are not checked for a rigid body); position-free
    // was: {$Ball */*, $Cube */*, $Rig */*}
    r(
        "physics.constraint.add",
        r##"{"objects": ["$Rig"], "type": "hinge", "object1": "$Cube", "object2": "$Ball"}"##,
        "",
        &[("$Rig", "physics", "-"), ("$Cube", "-", "-"), ("$Ball", "-", "-")],
    ),
    // was: {$Ball */*, $Child */*}
    r(
        "physics.constraint.add",
        r##"{"type": "hinge", "object1": "$Ball", "object2": "$Child"}"##,
        "selection",
        &[("$Cube", "physics", "-"), ("$Ball", "-", "-"), ("$Child", "-", "-")],
    ),
    // was: {$Ball */*, $Cube */*, $Rig */*}
    r(
        "physics.constraint.add",
        r##"{"objects": ["Rig"], "type": "hinge", "object1": "Cube", "object2": "Ball"}"##,
        "",
        &[("$Rig", "name,physics", "-"), ("$Cube", "name", "-"), ("$Ball", "name", "-")],
    ),
    // was: {$Rig */*}
    r("physics.constraint.add", r##"{"objects": ["$Rig"], "type": "hinge"}"##, "", &[("$Rig", "physics", "-")]),
    // physics.constraint.set: objects = id-or-name, else the selection (the constraint's owner; merges into target.physics.rigidConstraint); object1/object2 = other objects by id or name, any kind, existence only (they are not checked for a rigid body); position-free
    // was: {$Ball */*, $Cube */*, $Rig */*}
    r(
        "physics.constraint.set",
        r##"{"objects": ["$Rig"], "object1": "$Cube", "object2": "$Ball"}"##,
        "",
        &[("$Rig", "physics", "-"), ("$Cube", "-", "-"), ("$Ball", "-", "-")],
    ),
    // was: {$Ball */*, $Child */*}
    r(
        "physics.constraint.set",
        r##"{"object1": "$Ball", "object2": "$Child"}"##,
        "selection",
        &[("$Cube", "physics", "-"), ("$Ball", "-", "-"), ("$Child", "-", "-")],
    ),
    // was: {$Ball */*, $Cube */*, $Rig */*}
    r(
        "physics.constraint.set",
        r##"{"objects": ["Rig"], "object1": "Cube", "object2": "Ball"}"##,
        "",
        &[("$Rig", "name,physics", "-"), ("$Cube", "name", "-"), ("$Ball", "name", "-")],
    ),
    // was: {$Rig */*}
    r("physics.constraint.set", r##"{"objects": ["$Rig"], "enabled": false}"##, "", &[("$Rig", "physics", "-")]),
    // physics.field.new: appends a new Empty with a field; location defaults to the origin (not the 3D cursor); the name is made unique (not held); reads no stored fact; sets the selection (write, not read)
    r("physics.field.new", r##"{"type": "wind", "strength": 2, "location": [0, 0, 1], "name": "Wind"}"##, "", &[]),
    r("physics.field.new", r##"{}"##, "", &[]),
    // physics.constraint.connect: objects = id-or-name list in order, else the selection in stored order; keeps those with physics.rigidBody (silently drops others); each new Empty sits at the midpoint of the two bodies' WORLD origins (world_matrix: ancestors and constraints; with a current bake, the baked pose at the current frame - no bake in the fixture, so no `time`)
    // was: {$Ball */*, $Cube */*}
    r(
        "physics.constraint.connect",
        r##"{"objects": ["$Cube", "$Ball"], "type": "fixed"}"##,
        "",
        &[("$Cube", "constraints,physics", "*"), ("$Ball", "constraints,physics", "*")],
    ),
    // was: {}
    r("physics.constraint.connect", r##"{}"##, "selection", &[("$Cube", "constraints,physics", "*")]),
    // was: {$Ball */*, $Cube */*}
    r(
        "physics.constraint.connect",
        r##"{"objects": ["Cube", "Ball"]}"##,
        "",
        &[("$Cube", "constraints,name,physics", "*"), ("$Ball", "constraints,name,physics", "*")],
    ),
    // was: {$Child */*, $Cube */*}
    r(
        "physics.constraint.connect",
        r##"{"objects": ["$Cube", "$Child"]}"##,
        "",
        &[("$Cube", "constraints,physics", "*"), ("$Child", "constraints,physics", "*"), ("$Rig", "constraints", "*")],
    ),
];
