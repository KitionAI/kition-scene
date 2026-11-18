//! Camera framing and boid simulation share a scene saved by the released 0.16.0 engine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use kition_scene_model::{Camera, ObjectData, particles};
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"));
}

fn camera(doc: &SceneDocument) -> Camera {
    let ObjectData::Camera(camera) = &doc.scene().object_named("Camera").unwrap().data else { panic!("camera") };
    camera.clone()
}

fn system(doc: &SceneDocument) -> particles::ParticleSystem {
    doc.scene().object_named("Cube").unwrap().particles[0].clone()
}

fn flight(doc: &SceneDocument, cache: &mut particles::Cache) -> Vec<particles::Particle> {
    let id = doc.scene().object_named("Cube").unwrap().id;
    particles::particles(doc.scene(), id, 0, cache).as_ref().clone()
}

#[test]
fn framing_a_released_emitter_preserves_boid_motion_and_independent_undo() {
    let mut doc = SceneDocument::open("Released.kscene", include_bytes!("fixtures/released-0.16.0.kscene")).unwrap();
    let old_camera = camera(&doc);
    let old_system = system(&doc);
    assert_eq!(old_system.name, "Flight");
    assert_eq!(old_system.count, 6);
    assert_eq!(serde_json::to_value(&old_system).unwrap()["physics"], "newtonian");
    run(&mut doc, "view.camera", json!({}));
    let projection = doc.gl_scene()["camera"]["projection"].clone();
    assert!(projection.is_array());
    let mut cache = particles::Cache::default();
    let newtonian = flight(&doc, &mut cache);
    assert_eq!(newtonian.len(), 6);

    run(
        &mut doc,
        "particles.set",
        json!({"object":"Cube", "system":"Flight", "physics":"boids",
            "boids":{"rules":[{"kind":"goal", "target":"Target"}]}}),
    );
    let boids = system(&doc);
    assert_eq!(serde_json::to_value(&boids).unwrap()["physics"], "boids");
    assert_eq!(camera(&doc), old_camera);
    let flying = flight(&doc, &mut cache);
    assert_eq!(flying.len(), 6);
    assert_ne!(flying, newtonian, "switching physics must replace the cached Newtonian flight");
    run(&mut doc, "frame.set", json!({"frame":1}));
    let born = flight(&doc, &mut cache);
    assert_eq!(born.len(), flying.len());
    assert!(flying.iter().zip(&born).all(|(later, first)| later.position.distance(first.position) > 0.01));
    run(&mut doc, "frame.set", json!({"frame":12}));
    assert_eq!(flight(&doc, &mut cache), flying, "seeking back and forward reconstructs the same flock");

    run(&mut doc, "camera.set", json!({"object":"Camera", "sensorFit":"vertical", "sensorHeight":19}));
    let fitted = camera(&doc);
    assert_ne!(fitted, old_camera);
    assert_ne!(doc.gl_scene()["camera"]["projection"], projection);
    assert_eq!(system(&doc), boids);
    assert_eq!(flight(&doc, &mut cache), flying);
    assert_eq!(flight(&doc, &mut particles::Cache::default()), flying, "a fresh simulation also ignores camera framing");

    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(camera(&doc), old_camera);
    assert_eq!(system(&doc), boids, "undoing camera framing keeps the boid edit");
    assert_eq!(flight(&doc, &mut cache), flying);
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(system(&doc), old_system, "a separate undo restores released Newtonian settings");
    assert_eq!(camera(&doc), old_camera);
    assert_eq!(flight(&doc, &mut cache), newtonian, "undo also restores the original flight in a reused cache");
    assert_eq!(flight(&doc, &mut particles::Cache::default()), newtonian);
    run(&mut doc, "edit.redo", json!({}));
    assert_eq!(system(&doc), boids);
    assert_eq!(camera(&doc), old_camera);
    run(&mut doc, "edit.redo", json!({}));
    assert_eq!(camera(&doc), fitted);
    assert_eq!(system(&doc), boids);
    assert_eq!(flight(&doc, &mut cache), flying);

    let reopened = SceneDocument::open("Saved.kscene", &doc.save(None).unwrap()).unwrap();
    assert_eq!(camera(&reopened), fitted);
    assert_eq!(system(&reopened), boids);
    assert_eq!(flight(&reopened, &mut particles::Cache::default()), flying);
    assert_eq!(reopened.gl_scene()["camera"]["projection"], doc.gl_scene()["camera"]["projection"]);
}
