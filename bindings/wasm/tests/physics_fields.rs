//! Force fields, rigid body constraints, deactivation, cloth self collision, and particle
//! density groups through commands: settings (undoable, saved, inspectable), what they do to a
//! bake and to particles, and constraints letting go of deleted bodies.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn empty() -> SceneDocument {
    let mut doc = SceneDocument::create("Untitled.kscene", 640, 360).unwrap();
    run(&mut doc, "select.all", json!({}));
    run(&mut doc, "edit.delete", json!({}));
    doc
}

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn location(doc: &SceneDocument, name: &str) -> [f64; 3] {
    let inspected = doc.inspect().unwrap();
    let object = inspected["objects"].as_array().unwrap().iter().find(|object| object["name"] == name).unwrap().clone();
    let at = |axis: usize| object["worldLocation"][axis].as_f64().unwrap();
    [at(0), at(1), at(2)]
}

fn physics_of(doc: &SceneDocument, name: &str) -> Value {
    let inspected = doc.inspect().unwrap();
    let id = inspected["objects"].as_array().unwrap().iter().find(|object| object["name"] == name).unwrap()["id"].clone();
    inspected["physics"]["objects"].as_array().unwrap().iter().find(|object| object["id"] == id).cloned().unwrap_or(Value::Null)
}

/// A passive anchor at the origin and a box 2 m along X joined by a hinge about Y.
fn pendulum() -> SceneDocument {
    let mut doc = empty();
    run(&mut doc, "add.cube", json!({ "size": 0.2, "name": "Anchor" }));
    run(&mut doc, "physics.rigidBody.add", json!({ "type": "passive" }));
    run(&mut doc, "add.cube", json!({ "size": 0.5, "location": [2, 0, 0], "name": "Bob" }));
    run(&mut doc, "physics.rigidBody.add", json!({}));
    run(&mut doc, "add.empty", json!({ "name": "Hinge", "rotation": [90, 0, 0] }));
    run(&mut doc, "physics.constraint.add", json!({ "type": "hinge", "object1": "Anchor", "object2": "Bob" }));
    run(&mut doc, "physics.world.set", json!({ "frameEnd": 40 }));
    doc
}

#[test]
fn a_hinge_swings_the_bob_and_lets_go_of_deleted_bodies() {
    let mut doc = pendulum();
    let hinge = physics_of(&doc, "Hinge");
    assert_eq!(hinge["rigidConstraint"]["kind"], "hinge");
    assert!(doc.execute("physics.constraint.set", json!({ "objects": "Hinge", "object1": "Hinge" })).is_err(), "not its own object");
    assert!(doc.execute("physics.constraint.set", json!({ "objects": "Hinge", "angleLimit": [30, -30] })).is_err(), "lower above upper");
    run(&mut doc, "physics.bake", json!({}));
    run(&mut doc, "frame.set", json!({ "frame": 20 }));
    let bob = location(&doc, "Bob");
    // The reference has the bob at (0.33, 0, -1.97) at frame 20.
    assert!((bob[0] - 0.33).abs() < 0.1 && (bob[2] + 1.97).abs() < 0.1 && bob[1].abs() < 1e-3, "{bob:?}");
    // Limits in degrees, kept in the file.
    run(&mut doc, "physics.constraint.set", json!({ "objects": "Hinge", "angleLimit": [-30, 30] }));
    let reopened = SceneDocument::open("t.kscene", &doc.save_file(None).unwrap()).unwrap();
    let limit = physics_of(&reopened, "Hinge")["rigidConstraint"]["angleLimit"].clone();
    assert!((limit[0].as_f64().unwrap() + 30f64.to_radians()).abs() < 1e-5, "{limit}");
    // Deleting a body lets go of it; undo brings it back.
    run(&mut doc, "select.objects", json!({ "objects": ["Bob"] }));
    run(&mut doc, "edit.delete", json!({}));
    assert!(physics_of(&doc, "Hinge")["rigidConstraint"].get("object2").is_none());
    run(&mut doc, "edit.undo", json!({}));
    assert!(physics_of(&doc, "Hinge")["rigidConstraint"]["object2"].is_number());
}

#[test]
fn connect_joins_bodies_one_to_the_next() {
    let mut doc = empty();
    for (index, name) in ["A", "B", "C"].iter().enumerate() {
        run(&mut doc, "add.cube", json!({ "size": 1, "location": [index as f64 * 1.0, 0, 3], "name": name }));
        run(&mut doc, "physics.rigidBody.add", json!({}));
    }
    let made = run(&mut doc, "physics.constraint.connect", json!({ "objects": ["A", "B", "C"], "type": "fixed" }));
    let constraints = made["objects"].as_array().unwrap();
    assert_eq!(constraints.len(), 2);
    assert_eq!(location(&doc, "Constraint"), [0.5, 0.0, 3.0]);
    // Glued in a row, the three fall as one.
    run(&mut doc, "physics.world.set", json!({ "frameEnd": 20 }));
    run(&mut doc, "physics.bake", json!({}));
    run(&mut doc, "frame.set", json!({ "frame": 20 }));
    let (a, c) = (location(&doc, "A"), location(&doc, "C"));
    assert!(a[2] < 1.0 && (c[0] - a[0] - 2.0).abs() < 0.02 && (c[2] - a[2]).abs() < 0.02, "{a:?} {c:?}");
}

#[test]
fn a_force_field_pushes_particles_and_bodies() {
    let mut doc = empty();
    run(&mut doc, "add.plane", json!({ "size": 2, "name": "Emitter" }));
    run(&mut doc, "particles.add", json!({ "count": 100, "frameEnd": 1, "lifetime": 100, "normalVelocity": 0, "gravity": 0 }));
    let still = run(&mut doc, "particles.stats", json!({ "frame": 30 }))["meanPosition"].clone();
    let field =
        run(&mut doc, "physics.field.new", json!({ "type": "wind", "location": [-3, 0, 0], "rotation": [0, 90, 0], "strength": 5, "flow": 0 }));
    assert_eq!(field["objects"][0]["field"]["kind"], "wind");
    let blown = run(&mut doc, "particles.stats", json!({ "object": "Emitter", "frame": 30 }))["meanPosition"].clone();
    // 5 m/s² for 1.16 s: 3.36 m along X, as the reference.
    let moved = blown[0].as_f64().unwrap() - still[0].as_f64().unwrap();
    assert!((moved - 3.364).abs() < 0.02, "{moved}");
    // Undo takes the field back; the particles are as before.
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(run(&mut doc, "particles.stats", json!({ "object": "Emitter", "frame": 30 }))["meanPosition"], still);
    assert!(doc.execute("physics.field.set", json!({ "objects": "Emitter", "strength": 1 })).is_err(), "no field to set");
    run(&mut doc, "physics.field.add", json!({ "objects": "Emitter", "type": "force" }));
    assert!(doc.execute("physics.field.set", json!({ "objects": "Emitter", "falloffPower": -1 })).is_err(), "out of range");
    // The emitter's own field leaves its own particles alone.
    assert_eq!(run(&mut doc, "particles.stats", json!({ "object": "Emitter", "frame": 30 }))["meanPosition"], still);
}

#[test]
fn a_body_that_starts_asleep_hangs_until_it_is_hit() {
    let mut doc = empty();
    run(&mut doc, "add.plane", json!({ "size": 20, "name": "Ground" }));
    run(&mut doc, "physics.rigidBody.add", json!({ "type": "passive" }));
    run(&mut doc, "add.cube", json!({ "size": 1, "location": [0, 0, 3], "name": "Sleeper" }));
    run(&mut doc, "physics.rigidBody.add", json!({ "deactivation": true, "startDeactivated": true }));
    run(&mut doc, "physics.world.set", json!({ "frameEnd": 60 }));
    run(&mut doc, "physics.bake", json!({}));
    run(&mut doc, "frame.set", json!({ "frame": 60 }));
    assert_eq!(location(&doc, "Sleeper")[2], 3.0, "it hangs");
    run(&mut doc, "frame.set", json!({ "frame": 1 }));
    run(&mut doc, "add.cube", json!({ "size": 1, "location": [0.2, 0, 6], "name": "Knock" }));
    run(&mut doc, "physics.rigidBody.add", json!({}));
    run(&mut doc, "physics.bake", json!({}));
    run(&mut doc, "frame.set", json!({ "frame": 60 }));
    assert!((location(&doc, "Sleeper")[2] - 0.5).abs() < 0.05, "knocked awake, it falls");
}

#[test]
fn cloth_self_collision_and_density_groups_are_settings() {
    let mut doc = empty();
    run(&mut doc, "add.grid", json!({ "xSubdivisions": 4, "ySubdivisions": 4, "size": 2, "name": "Sheet" }));
    run(&mut doc, "physics.cloth.add", json!({ "selfCollision": true, "selfDistance": 0.02 }));
    let cloth = physics_of(&doc, "Sheet")["cloth"].clone();
    assert_eq!((cloth["selfCollision"].as_bool(), cloth["selfDistance"].as_f64().map(|value| (value * 1000.0).round())), (Some(true), Some(20.0)));
    assert!(doc.execute("physics.cloth.set", json!({ "selfDistance": 0 })).is_err());
    // Only the right half weighs: every particle starts there.
    let inspected = doc.inspect().unwrap();
    let sheet = inspected["objects"].as_array().unwrap().iter().find(|object| object["name"] == "Sheet").unwrap().clone();
    let right: Vec<u32> = (0..sheet["mesh"]["vertices"].as_u64().unwrap() as u32).filter(|index| index % 5 >= 3).collect();
    run(&mut doc, "vertexGroup.assign", json!({ "name": "Dense", "vertices": right, "weight": 1 }));
    run(&mut doc, "particles.add", json!({ "count": 200, "frameEnd": 1, "normalVelocity": 0, "gravity": 0, "densityGroup": "Dense" }));
    let stats = run(&mut doc, "particles.stats", json!({ "frame": 2 }));
    assert_eq!(stats["alive"], 200);
    assert!(stats["meanPosition"][0].as_f64().unwrap() > 0.4, "{stats}");
    let reopened = SceneDocument::open("t.kscene", &doc.save_file(None).unwrap()).unwrap();
    assert_eq!(reopened.inspect().unwrap()["particles"][0]["densityGroup"], "Dense");
}
