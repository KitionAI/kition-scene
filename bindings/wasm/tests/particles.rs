//! Particle systems through commands, as the agent and the Particles panel drive them: emitters
//! and hair added and set, particles worked out at any frame from the seed (the same after a
//! save), Particle Edit grooming as one undo step that the file keeps, instances and hair in
//! pictures and exports, and a click on a particle selecting its emitter.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_kit::tools::{KitTools, PointerAction, ToolMods, ToolPointer};
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn reopen(doc: &mut SceneDocument) -> SceneDocument {
    SceneDocument::open("t.kscene", &doc.save_file(None).unwrap()).unwrap()
}

/// The starter scene without its cube, and the reference's default plane.
fn plane() -> SceneDocument {
    let mut doc = SceneDocument::create("Particles.kscene", 640, 480).unwrap();
    run(&mut doc, "edit.delete", json!({ "objects": "Cube" }));
    run(&mut doc, "object.addPlane", json!({}));
    doc
}

fn decode(base64: &str) -> Vec<u8> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let clean: Vec<u8> = base64.bytes().filter(|byte| *byte != b'=').collect();
    for chunk in clean.chunks(4) {
        let mut value = 0u32;
        for (index, byte) in chunk.iter().enumerate() {
            value |= (TABLE.iter().position(|item| item == byte).unwrap() as u32) << (18 - 6 * index);
        }
        out.extend(value.to_be_bytes().iter().skip(1).take(chunk.len() - 1));
    }
    out
}

/// The objects (`o` lines) and vertices (`v` lines) of an OBJ export.
fn obj(doc: &mut SceneDocument, selected_only: bool) -> (usize, usize) {
    let exported = run(doc, "file.export", json!({ "format": "obj", "selectedOnly": selected_only }));
    let text = String::from_utf8(decode(exported["base64"].as_str().unwrap())).unwrap();
    (text.lines().filter(|line| line.starts_with("o ")).count(), text.lines().filter(|line| line.starts_with("v ")).count())
}

#[test]
fn an_emitter_matches_the_reference_counts_at_any_frame_and_after_a_save() {
    let mut doc = plane();
    let added = run(&mut doc, "particle.add", json!({}));
    assert_eq!(added["name"], "ParticleSystem");
    assert_eq!(doc.inspect().unwrap()["particles"][0]["count"], 1000);
    // Blender 5.2.3 on the same plane: 46 alive at frame 10, 247 at frame 50.
    let at_10 = run(&mut doc, "particle.stats", json!({ "frame": 10 }));
    let at_50 = run(&mut doc, "particle.stats", json!({ "frame": 50 }));
    assert_eq!((at_10["alive"].as_u64(), at_50["alive"].as_u64()), (Some(46), Some(247)));
    assert!((at_50["meanPosition"][2].as_f64().unwrap() + 5.3187).abs() < 0.01, "{at_50}");
    // Particles are derived: none in the file, the same after opening it.
    let mut again = reopen(&mut doc);
    run(&mut again, "select.objects", json!({ "objects": "Plane" }));
    assert_eq!(run(&mut again, "particle.stats", json!({ "frame": 50 })), at_50);
    // Any frame at once, and a different seed differs.
    run(&mut doc, "frame.set", json!({ "frame": 120 }));
    let jumped = run(&mut doc, "particle.stats", json!({}));
    assert_eq!(jumped, run(&mut doc, "particle.stats", json!({ "frame": 120 })));
    run(&mut doc, "particle.set", json!({ "seed": 3 }));
    assert_ne!(run(&mut doc, "particle.stats", json!({}))["meanPosition"], jumped["meanPosition"]);
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(run(&mut doc, "particle.stats", json!({})), jumped);
    // Settings are checked.
    assert!(doc.execute("particle.set", json!({ "lifetimeRandom": 2 })).is_err());
    assert!(doc.execute("particle.set", json!({ "emitFrom": "edges" })).is_err());
    assert!(doc.execute("particle.add", json!({ "object": "Light" })).is_err(), "only meshes emit");
    run(&mut doc, "particle.remove", json!({}));
    assert_eq!(doc.inspect().unwrap()["particles"], json!([]));
}

#[test]
fn colliders_stop_the_fall_and_let_go_when_deleted() {
    let mut doc = plane();
    run(&mut doc, "object.addPlane", json!({ "name": "Floor", "size": 20, "location": [0, 0, -1] }));
    run(&mut doc, "select.objects", json!({ "objects": "Plane" }));
    run(&mut doc, "particle.add", json!({ "colliders": ["Floor"] }));
    let stats = run(&mut doc, "particle.stats", json!({ "frame": 60 }));
    assert!(stats["meanPosition"][2].as_f64().unwrap() > -1.0, "{stats}");
    run(&mut doc, "particle.set", json!({ "dieOnCollision": true }));
    let dying = run(&mut doc, "particle.stats", json!({ "frame": 60 }));
    assert!(dying["alive"].as_u64() < stats["alive"].as_u64(), "{dying} {stats}");
    run(&mut doc, "edit.delete", json!({ "objects": "Floor" }));
    assert_eq!(doc.inspect().unwrap()["particles"][0]["colliders"], Value::Null);
    let free = run(&mut doc, "particle.stats", json!({ "object": "Plane", "frame": 60 }));
    assert!(free["meanPosition"][2].as_f64().unwrap() < -1.0, "{free}");
}

#[test]
fn hair_is_groomed_in_one_undo_step_and_the_file_keeps_it() {
    let mut doc = SceneDocument::create("Hair.kscene", 640, 480).unwrap();
    run(&mut doc, "edit.delete", json!({ "objects": "Cube" }));
    run(&mut doc, "object.addUvSphere", json!({}));
    run(&mut doc, "particle.add", json!({ "kind": "hair", "hairLength": 1, "count": 400 }));
    let grown = run(&mut doc, "particle.stats", json!({}));
    assert_eq!((grown["strands"].as_u64(), grown["meanLength"].as_f64()), (Some(400), Some(1.0)));
    run(&mut doc, "view.front", json!({}));
    run(&mut doc, "view.frameSelected", json!({}));
    run(&mut doc, "mode.particleEdit", json!({}));
    assert_eq!(doc.inspect().unwrap()["mode"]["mode"], "particleEdit");
    let (x, y) = (f64::from(doc.width()) / 2.0, f64::from(doc.height()) / 2.0);
    // A comb stroke across the sphere keeps lengths; cut shortens.
    let combed = run(&mut doc, "particleEdit.stroke", json!({ "kind": "comb", "radius": 150, "points": [[x - 150.0, y], [x, y], [x + 150.0, y]] }));
    assert!(combed["touched"].as_u64().unwrap() > 10, "{combed}");
    let after_comb = run(&mut doc, "particle.stats", json!({}));
    assert!(after_comb["groomed"].as_bool().unwrap());
    assert!((after_comb["meanLength"].as_f64().unwrap() - 1.0).abs() < 1e-3, "{after_comb}");
    run(&mut doc, "particleEdit.stroke", json!({ "kind": "cut", "radius": 2000, "strength": 1, "points": [[x, y]] }));
    let cut = run(&mut doc, "particle.stats", json!({}));
    assert!(cut["meanLength"].as_f64().unwrap() < 0.95, "{cut}");
    // The file keeps the groom; undo takes back one stroke.
    let reopened = run(&mut reopen(&mut doc), "particle.stats", json!({ "object": "Sphere" }));
    assert_eq!(reopened, cut);
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(run(&mut doc, "particle.stats", json!({})), after_comb);
    // The pointer grooms too: a drag is one step.
    let event = |action, x: f64, y: f64| ToolPointer { action, x, y, mods: ToolMods::default(), pressure: 1.0, zoom: 1.0 };
    run(&mut doc, "particleEdit.brush", json!({ "kind": "length", "radius": 2000, "strength": 1 }));
    doc.pointer(&event(PointerAction::Down, x, y)).unwrap();
    doc.pointer(&event(PointerAction::Drag, x + 10.0, y)).unwrap();
    doc.pointer(&event(PointerAction::Drag, x + 20.0, y)).unwrap();
    doc.pointer(&event(PointerAction::Up, x + 20.0, y)).unwrap();
    let longer = run(&mut doc, "particle.stats", json!({}));
    assert!(longer["meanLength"].as_f64().unwrap() > 1.0, "{longer}");
    assert_eq!(doc.inspect().unwrap()["undo"], "Groom");
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(run(&mut doc, "particle.stats", json!({})), after_comb);
    // Changing the count regrows the hair: the groom goes.
    run(&mut doc, "mode.particleEdit", json!({ "on": false }));
    let set = run(&mut doc, "particle.set", json!({ "count": 500 }));
    assert_eq!(set["groomDropped"], true);
    assert_eq!(run(&mut doc, "particle.stats", json!({}))["groomed"], false);
    // Children multiply the strands.
    run(&mut doc, "particle.set", json!({ "children": "simple", "childCount": 4 }));
    assert_eq!(run(&mut doc, "particle.stats", json!({}))["strands"], 2500);
}

#[test]
fn particles_show_in_pictures_and_exports_and_select_their_emitter() {
    let mut doc = plane();
    let (objects, vertices) = obj(&mut doc, false);
    // One particle that drifts along +X for 5 m and never falls, 2 m wide.
    run(
        &mut doc,
        "particle.add",
        json!({ "count": 1, "emitFrom": "verts", "frameStart": 1, "frameEnd": 1, "lifetime": 500, "normalVelocity": 0, "gravity": 0, "objectVelocity": [1.25, 0, 0], "size": 2 }),
    );
    run(&mut doc, "frame.set", json!({ "frame": 101 }));
    // Exports bake the halo; selected-only takes the emitter's particles with it.
    assert_eq!(obj(&mut doc, false), (objects + 1, vertices + 6));
    run(&mut doc, "select.objects", json!({ "objects": "Plane" }));
    assert_eq!(obj(&mut doc, true).0, 2);
    // The viewport draws it where the particle is; a click there selects the plane.
    run(&mut doc, "view.top", json!({}));
    run(&mut doc, "edit.deselectAll", json!({}));
    let projector = doc.session().editor.view.projector(doc.scene(), doc.width(), doc.height());
    let screen = projector.to_screen(projector.view_projection() * kition_scene_model::glam::Vec4::new(4.0, -1.0, 0.0, 1.0));
    doc.select_tool("select", 1.0).unwrap();
    let event = |action| ToolPointer { action, x: f64::from(screen.x), y: f64::from(screen.y), mods: ToolMods::default(), pressure: 1.0, zoom: 1.0 };
    doc.pointer(&event(PointerAction::Down)).unwrap();
    doc.pointer(&event(PointerAction::Up)).unwrap();
    let plane_id = doc.scene().object_named("Plane").unwrap().id;
    assert_eq!(doc.inspect().unwrap()["selection"]["active"], plane_id);
    // Instances: a copy of a cone at each particle; Convert makes them a real mesh.
    run(&mut doc, "object.addCone", json!({ "name": "Spike", "location": [0, 0, 5] }));
    run(&mut doc, "select.objects", json!({ "objects": "Plane" }));
    run(
        &mut doc,
        "particle.set",
        json!({ "count": 20, "frameEnd": 20, "renderAs": "object", "instanceObject": "Spike", "size": 0.3, "rotationRandom": 1 }),
    );
    let converted = run(&mut doc, "particle.convert", json!({}));
    let cone = doc.scene().object_named("Spike").unwrap().mesh().unwrap().faces.len();
    assert_eq!(converted["faces"].as_u64().unwrap() as usize, cone * 20);
    // A camera render shows them.
    let png = doc.render_png(1).unwrap();
    assert!(png.len() > 100);
}
