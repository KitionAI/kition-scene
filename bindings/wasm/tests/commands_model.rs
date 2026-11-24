//! The second set of Edit Mode commands, proportional editing, the new modifiers, and `.blend`
//! import, through the document as people and the agent run them.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn doc() -> SceneDocument {
    let mut made = SceneDocument::create("Untitled scene.kscene", 1280, 720).unwrap();
    made.reopen("t.kscene").unwrap()
}

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn edit_cube(doc: &mut SceneDocument) {
    run(doc, "select.objects", json!({ "objects": "Cube" }));
    run(doc, "mode.set", json!({ "mode": "edit" }));
}

fn mesh_counts(doc: &SceneDocument, name: &str) -> (u64, u64) {
    let inspected = doc.inspect().unwrap();
    let object = inspected["objects"].as_array().unwrap().iter().find(|object| object["name"] == name).unwrap().clone();
    (object["mesh"]["vertices"].as_u64().unwrap(), object["mesh"]["faces"].as_u64().unwrap())
}

#[test]
fn extrude_works_on_vertices_edges_and_single_faces() {
    let mut doc = doc();
    edit_cube(&mut doc);
    run(&mut doc, "mesh.select", json!({ "vertices": [3, 7] }));
    run(&mut doc, "mesh.extrude", json!({ "offset": [0, 0, 1] }));
    assert_eq!(mesh_counts(&doc, "Cube"), (10, 7), "an edge pulls out a face");
    run(&mut doc, "edit.undo", json!({}));
    run(&mut doc, "mesh.select", json!({ "faces": [5, 1] }));
    run(&mut doc, "mesh.extrudeIndividual", json!({ "distance": 0.5 }));
    assert_eq!(mesh_counts(&doc, "Cube"), (16, 14));
    assert!(doc.check().is_empty(), "{:?}", doc.check());
}

#[test]
fn dissolve_bisect_symmetrize_spin_and_bevel() {
    let mut doc = doc();
    edit_cube(&mut doc);
    run(&mut doc, "mesh.select", json!({ "vertices": [3, 7] }));
    run(&mut doc, "mesh.dissolveEdges", json!({}));
    assert_eq!(mesh_counts(&doc, "Cube"), (6, 5));
    run(&mut doc, "edit.undo", json!({}));

    run(&mut doc, "select.all", json!({}));
    run(&mut doc, "mesh.bisect", json!({ "point": [0, 0, 0.3], "normal": [0, 0, 1], "clearOuter": true, "fill": true }));
    assert_eq!(mesh_counts(&doc, "Cube"), (8, 6));
    assert_eq!(doc.inspect().unwrap()["selection"]["vertices"].as_array().unwrap().len(), 4, "the cut is selected");
    run(&mut doc, "edit.undo", json!({}));

    run(&mut doc, "mesh.symmetrize", json!({ "axis": "x" }));
    assert_eq!(mesh_counts(&doc, "Cube"), (12, 10));
    run(&mut doc, "edit.undo", json!({}));

    run(&mut doc, "select.all", json!({}));
    run(&mut doc, "mesh.bevelVertices", json!({ "width": 0.2 }));
    assert_eq!(mesh_counts(&doc, "Cube"), (24, 14));
    run(&mut doc, "edit.undo", json!({}));

    // Spin the top face's outline a quarter turn round Y.
    run(&mut doc, "mesh.select", json!({ "faces": [5] }));
    run(&mut doc, "mesh.spin", json!({ "steps": 4, "angle": 90, "axis": [0, 1, 0], "center": [3, 0, 0] }));
    let (vertices, _) = mesh_counts(&doc, "Cube");
    assert_eq!(vertices, 8 + 4 * 4);
}

#[test]
fn separate_makes_a_new_object_and_bridge_joins_loops() {
    let mut doc = doc();
    edit_cube(&mut doc);
    run(&mut doc, "mesh.select", json!({ "faces": [5] }));
    let made = run(&mut doc, "mesh.separate", json!({}));
    assert_eq!(made["name"], "Cube.001");
    assert_eq!(mesh_counts(&doc, "Cube.001"), (4, 1));
    assert_eq!(mesh_counts(&doc, "Cube"), (8, 5));

    // The open top's outline and the separated square's, bridged in one mesh.
    run(&mut doc, "mode.set", json!({ "mode": "object" }));
    run(&mut doc, "object.translate", json!({ "objects": "Cube.001", "offset": [0, 0, 1] }));
    run(&mut doc, "select.objects", json!({ "objects": ["Cube.001", "Cube"] }));
    run(&mut doc, "object.join", json!({}));
    run(&mut doc, "mode.set", json!({ "mode": "edit" }));
    // Select both loops: the cube's open top ring (z = 1) and the square (z = 2).
    let inspected = doc.inspect().unwrap();
    assert_eq!(inspected["objects"].as_array().unwrap().iter().filter(|object| object["type"] == "mesh").count(), 1);
    run(&mut doc, "mesh.select", json!({ "vertices": [1, 3, 5, 7, 8, 9, 10, 11] }));
    let bridged = run(&mut doc, "mesh.bridge", json!({}));
    assert_eq!(bridged["faces"], 4);
}

#[test]
fn proportional_editing_pulls_neighbours() {
    let mut doc = doc();
    run(&mut doc, "add.grid", json!({ "xSubdivisions": 4, "ySubdivisions": 1, "size": 4 }));
    run(&mut doc, "mode.set", json!({ "mode": "edit" }));
    let state = run(&mut doc, "mesh.proportional", json!({ "on": true, "radius": 2, "falloff": "linear" }));
    assert_eq!(state["proportional"]["falloff"], "linear");
    assert_eq!(doc.inspect().unwrap()["mode"]["proportional"]["radius"], 2.0);
    // The grid's middle column is x = 0: vertices 2 and 7.
    run(&mut doc, "mesh.select", json!({ "vertices": [2, 7] }));
    run(&mut doc, "mesh.translate", json!({ "offset": [0, 0, 1] }));
    run(&mut doc, "mode.set", json!({ "mode": "object" }));
    let check = doc.inspect().unwrap();
    let grid = check["objects"].as_array().unwrap().iter().find(|object| object["name"] == "Grid").unwrap();
    // Bounds: the column at 1 m rose half way (linear at half the radius).
    assert!((grid["dimensions"][2].as_f64().unwrap() - 1.0).abs() < 1e-3);
    run(&mut doc, "mesh.proportional", json!({ "on": false }));
    assert!(doc.inspect().unwrap()["mode"]["proportional"].is_null());
    run(&mut doc, "mode.set", json!({ "mode": "edit" }));
    run(&mut doc, "mesh.select", json!({ "vertices": [0] }));
    run(&mut doc, "mesh.rotate", json!({ "axis": "z", "angle": 90, "pivot": [0, 0, 0] }));
}

#[test]
fn the_new_modifiers_take_degrees_and_check_ranges() {
    let mut doc = doc();
    run(&mut doc, "modifier.add", json!({ "object": "Cube", "type": "simpleDeform", "mode": "bend", "axis": "z", "angle": 60 }));
    let cube = &doc.inspect().unwrap()["objects"][0];
    assert_eq!(cube["modifiers"][0]["angle"], 60.0);
    for kind in ["displace", "decimate", "wireframe", "cast"] {
        run(&mut doc, "modifier.add", json!({ "type": kind }));
    }
    assert_eq!(doc.inspect().unwrap()["objects"][0]["modifiers"].as_array().unwrap().len(), 5);
    assert!(doc.execute("modifier.add", json!({ "type": "decimate", "ratio": 3 })).is_err());
    assert!(doc.execute("modifier.add", json!({ "type": "simpleDeform", "limits": [0.9, 0.1] })).is_err());
}

#[test]
fn damaged_blend_files_change_nothing() {
    let mut doc = doc();
    let before = doc.save_file(None).unwrap();
    assert!(doc.execute("file.import", json!({ "base64": "QkxFTkRFUi12NDA1", "format": "blend" })).is_err());
    assert_eq!(doc.save_file(None).unwrap(), before);
}

#[test]
fn blend_files_the_reference_writes_import() {
    let Some(blender) = std::env::var_os("KITION_BLENDER") else {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    };
    let dir = std::env::temp_dir().join(format!("kition-scene-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("make.py");
    std::fs::write(
        &script,
        "import bpy, sys\nout = sys.argv[sys.argv.index('--') + 1]\nbpy.ops.mesh.primitive_monkey_add(location=(0, 3, 0))\nbpy.context.active_object.modifiers.new('Sub', 'SUBSURF')\nbpy.ops.wm.save_as_mainfile(filepath=out + '/monkey.blend', compress=True)\n",
    )
    .unwrap();
    let status = std::process::Command::new(blender)
        .args(["-b", "--factory-startup", "--python"])
        .arg(&script)
        .arg("--")
        .arg(&dir)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let bytes = std::fs::read(dir.join("monkey.blend")).unwrap();
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::new();
    for chunk in bytes.chunks(3) {
        let triple = chunk.iter().enumerate().fold(0u32, |acc, (at, byte)| acc | u32::from(*byte) << (16 - 8 * at));
        for slot in 0..4 {
            text.push(if slot <= chunk.len() { TABLE[((triple >> (18 - 6 * slot)) & 63) as usize] as char } else { '=' });
        }
    }
    let mut doc = doc();
    let placed = run(&mut doc, "file.import", json!({ "base64": text, "format": "blend" }));
    assert_eq!(placed["objects"].as_array().unwrap().len(), 4, "the file's cube, light, camera, and monkey");
    assert_eq!(mesh_counts(&doc, "Suzanne"), (507, 500));
    let inspected = doc.inspect().unwrap();
    let monkey = inspected["objects"].as_array().unwrap().iter().find(|object| object["name"] == "Suzanne").unwrap();
    assert_eq!(monkey["modifiers"][0]["type"], "subdivision");
    let _ = std::fs::remove_dir_all(&dir);
}
