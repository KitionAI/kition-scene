//! Rendered viewport state, invalidation, software sampling, picking, and overlays.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};
fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}
fn starter() -> SceneDocument {
    let doc = SceneDocument::create("View.kscene", 96, 64).unwrap();
    let mut doc = SceneDocument::open("View.kscene", &doc.save(None).unwrap()).unwrap();
    run(&mut doc, "view.resize", json!({"width":96,"height":64}));
    run(&mut doc, "view.grid", json!({"on":false}));
    run(&mut doc, "select.none", json!({}));
    doc
}
fn complete(doc: &mut SceneDocument, samples: u32) -> Value {
    run(doc, "view.renderedStep", json!({"samples":samples,"wait":true}))
}
fn rgba(doc: &SceneDocument) -> Vec<u8> {
    doc.render(0.0, 0.0, 1.0, doc.width(), doc.height()).unwrap()
}
#[test]
fn sampling_keeps_one_compiled_scene_without_dirtying_or_undo() {
    let mut doc = starter();
    run(&mut doc, "view.rendered", json!({}));
    let first = complete(&mut doc, 2);
    let initial = rgba(&doc);
    assert_eq!(first["samples"], 2);
    assert_eq!(first["generation"], 1);
    let more = complete(&mut doc, 4);
    assert_eq!(more["generation"], first["generation"]);
    assert_eq!(more["samples"], 4);
    assert_eq!(complete(&mut doc, 2)["generation"], first["generation"]);
    assert_eq!(rgba(&doc), initial, "resampling starts with the same deterministic seed");
    assert!(!doc.dirty());
    assert_eq!(doc.inspect().unwrap()["canUndo"], false);
    assert_eq!(doc.gl_scene()["supported"], false);
    let reopened = SceneDocument::open("View.kscene", &doc.save(None).unwrap()).unwrap();
    assert_eq!(reopened.inspect().unwrap()["view"]["shading"], "rendered");
    assert_eq!(reopened.inspect().unwrap()["view"]["rendered"]["samples"], 0);
    assert_eq!(doc.render_area(0, 0, 96, 64, 48, 32).unwrap(), doc.render(0.0, 0.0, 0.5, 48, 32).unwrap());
    for bad in [json!({"samples":0}), json!({"samples":257}), json!({"budgetMs":"bad"}), json!({"wait":"bad"})] {
        assert!(doc.execute("view.renderedStep", bad).is_err());
    }
    assert_eq!(rgba(&doc), initial);
}
#[test]
fn navigation_edits_frames_undo_and_mode_switch_discard_old_samples() {
    let mut doc = starter();
    run(&mut doc, "view.rendered", json!({}));
    complete(&mut doc, 1);
    let mut generation = 1;
    let mut picture = rgba(&doc);
    for (command, params) in [
        ("view.orbit", json!({"turn":20})),
        ("view.perspective", json!({})),
        ("view.camera", json!({})),
        ("object.transform", json!({"objects":"Cube","location":[1,0,0]})),
        ("edit.undo", json!({})),
        ("edit.redo", json!({})),
        ("light.set", json!({"object":"Light","power":0})),
    ] {
        run(&mut doc, command, params);
        assert_eq!(doc.inspect().unwrap()["view"]["rendered"]["samples"], 0, "{command}");
        generation += 1;
        assert_eq!(complete(&mut doc, 1)["generation"], generation, "{command}");
        let next = rgba(&doc);
        assert_ne!(next, picture, "{command}");
        picture = next;
    }
    run(&mut doc, "frame.set", json!({"frame":2}));
    assert_eq!(complete(&mut doc, 1)["generation"], generation + 1);
    run(&mut doc, "view.solid", json!({}));
    let solid = rgba(&doc);
    assert!(doc.execute("view.renderedStep", json!({})).is_err());
    assert_eq!(rgba(&doc), solid, "an obsolete queued step cannot paint rendered pixels in solid mode");
}
#[test]
fn large_view_picking_and_overlays_keep_full_resolution_and_camera_margins() {
    let mut doc = starter();
    run(&mut doc, "view.resize", json!({"width":770,"height":514}));
    let before = run(&mut doc, "presence.local", json!({"pointer":[385,257]}));
    run(&mut doc, "view.rendered", json!({}));
    let status = complete(&mut doc, 1);
    assert_eq!((status["width"].as_u64(), status["height"].as_u64()), (Some(385), Some(257)));
    let after = run(&mut doc, "presence.local", json!({"pointer":[385,257]}));
    assert_eq!(before["viewport"]["pointer"], after["viewport"]["pointer"]);
    let unselected = rgba(&doc);
    run(&mut doc, "select.objects", json!({"objects":"Cube"}));
    complete(&mut doc, 1);
    let selected = rgba(&doc);
    assert_ne!(unselected, selected);
    assert!(selected.as_chunks::<4>().0.iter().filter(|pixel| pixel[0] > 240 && pixel[1] > 120 && pixel[1] < 230 && pixel[2] < 160).count() > 80);
    run(&mut doc, "view.camera", json!({}));
    complete(&mut doc, 1);
    let through = rgba(&doc);
    assert_ne!(selected, through);
    assert_eq!(through.len(), 770 * 514 * 4);
    assert!(doc.gl_scene()["camera"]["frame"].is_array());
}

#[test]
fn peer_edits_invalidate_samples_in_both_orders_with_fixed_actors() {
    use kition_kit::{
        kdm::{ActorId, Replica},
        session::{KitSession, KitSessionDocument},
    };
    use kition_scene_kdm::{SceneModel, blobs::SharedBlobs};
    fn fixed(bytes: &[u8], actor: u64) -> SceneDocument {
        let file = kition_scene_kscene::read(bytes).unwrap();
        let blobs = SharedBlobs::default();
        for (hash, bytes) in file.blobs {
            blobs.insert(&hash, &bytes).unwrap();
        }
        let replica = Replica::load(kition_scene_kdm::schema(), &file.content, ActorId::from_u64(actor).unwrap()).unwrap();
        let mut doc = SceneDocument::open("Peer.kscene", bytes).unwrap();
        *KitSessionDocument::session_mut(&mut doc) = KitSession::<SceneModel>::from_replica(replica, Box::new(blobs)).unwrap();
        run(&mut doc, "view.resize", json!({"width":32,"height":24}));
        run(&mut doc, "view.rendered", json!({}));
        doc
    }
    let bytes = starter().save(None).unwrap();
    for _ in 0..20 {
        let mut left = fixed(&bytes, 101);
        let mut right = fixed(&bytes, 202);
        run(&mut left, "object.transform", json!({"objects":"Cube","location":[1,0,0]}));
        run(&mut right, "light.set", json!({"object":"Light","power":100}));
        let changes = [kition_kit::session::__snapshot(&left).unwrap(), kition_kit::session::__snapshot(&right).unwrap()];
        let mut pictures = Vec::new();
        for order in [[0, 1], [1, 0]] {
            let mut viewer = fixed(&bytes, 303);
            complete(&mut viewer, 1);
            let original = rgba(&viewer);
            for index in order {
                viewer.import_changes(&changes[index]).unwrap();
                assert_eq!(viewer.inspect().unwrap()["view"]["rendered"]["samples"], 0);
                complete(&mut viewer, 1);
            }
            let merged = rgba(&viewer);
            assert_ne!(merged, original);
            assert_eq!(viewer.inspect().unwrap()["view"]["rendered"]["generation"], 3);
            pictures.push(merged);
        }
        assert_eq!(pictures[0], pictures[1]);
    }
}

#[test]
fn rendered_mode_keeps_tool_picking_and_sculpt_working_surfaces() {
    use kition_kit::tools::{KitTools, PointerAction, ToolMods, ToolPointer};
    let mut doc = starter();
    run(&mut doc, "view.rendered", json!({}));
    complete(&mut doc, 1);
    for action in [PointerAction::Down, PointerAction::Up] {
        doc.pointer(&ToolPointer { action, x: 48.0, y: 32.0, mods: ToolMods::default(), pressure: 1.0, zoom: 1.0 }).unwrap();
    }
    let selected = doc.inspect().unwrap()["selection"]["objects"].clone();
    assert_eq!(selected.as_array().unwrap().len(), 1);
    assert_eq!(selected[0], doc.scene().object_named("Cube").unwrap().id);
    run(&mut doc, "mode.sculpt", json!({}));
    run(&mut doc, "view.top", json!({}));
    run(&mut doc, "view.frameSelected", json!({}));
    run(&mut doc, "sculpt.maskFill", json!({}));
    complete(&mut doc, 1);
    let rendered = rgba(&doc);
    run(&mut doc, "view.solid", json!({}));
    let solid = rgba(&doc);
    // Probe the working surface away from the selected object's origin overlay.
    let surface = (32 * 96 + 60) * 4;
    assert_eq!(&rendered[surface..surface + 4], &solid[surface..surface + 4]);
    run(&mut doc, "sculpt.gesture", json!({"action":"hide", "points":[[0,0],[96,64]]}));
    run(&mut doc, "view.rendered", json!({}));
    complete(&mut doc, 1);
    let hidden = rgba(&doc);
    assert_ne!(&rendered[surface..surface + 4], &hidden[surface..surface + 4]);
}
