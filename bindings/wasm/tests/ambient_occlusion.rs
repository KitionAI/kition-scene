//! Ambient occlusion through commands: on or off, a distance (or the automatic one from the
//! scene's size), and a strength, as render settings (refused when out of range, undone, saved),
//! the defaults rendering as before.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn starter() -> SceneDocument {
    let mut made = SceneDocument::create("Untitled.kscene", 1280, 720).unwrap();
    let mut doc = SceneDocument::open("t.kscene", &made.save_file(None).unwrap()).unwrap();
    run(&mut doc, "render.set", json!({ "width": 64, "height": 36 }));
    doc
}

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

fn unbase64(text: &str) -> Vec<u8> {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let (mut bits, mut count, mut out) = (0u32, 0, Vec::new());
    for byte in text.bytes().filter(|byte| *byte != b'=') {
        bits = (bits << 6) | TABLE.iter().position(|item| *item == byte).unwrap() as u32;
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
        }
    }
    out
}

/// The RGBA8 of a command's PNG answer.
fn pixels(answer: &Value) -> Vec<u8> {
    let png = unbase64(answer["base64"].as_str().unwrap());
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut buffer).unwrap();
    buffer.truncate(frame.buffer_size());
    buffer
}

fn render(doc: &mut SceneDocument) -> Vec<u8> {
    pixels(&run(doc, "render.image", json!({ "samples": 2 })))
}

fn mean(rgba: &[u8]) -> f64 {
    let sum: u64 = rgba.chunks(4).map(|pixel| u64::from(pixel[0]) + u64::from(pixel[1]) + u64::from(pixel[2])).sum();
    sum as f64 / (rgba.len() / 4 * 3) as f64
}

/// The cube standing on a floor 400 m wide under a white world, so the world's light (which
/// occlusion takes away) shows.
fn on_a_big_floor() -> SceneDocument {
    let mut doc = starter();
    run(&mut doc, "world.set", json!({ "color": "#ffffff", "strength": 1 }));
    run(&mut doc, "object.addPlane", json!({ "name": "Floor", "size": 400, "location": [0, 0, -1] }));
    doc
}

#[test]
fn ambient_occlusion_is_a_render_setting() {
    let mut doc = starter();
    let render = doc.inspect().unwrap()["render"].clone();
    assert_eq!((render["ao"].as_bool(), render["aoDistance"].clone(), render["aoStrength"].as_f64()), (Some(true), Value::Null, Some(1.0)));
    run(&mut doc, "render.set", json!({ "ao": false, "aoDistance": 0.25, "aoStrength": 1.5 }));
    let render = doc.inspect().unwrap()["render"].clone();
    assert_eq!((render["ao"].as_bool(), render["aoDistance"].as_f64(), render["aoStrength"].as_f64()), (Some(false), Some(0.25), Some(1.5)));
    for refused in [
        json!({ "aoDistance": 0 }),
        json!({ "aoDistance": -1 }),
        json!({ "aoDistance": 2000 }),
        json!({ "aoStrength": -0.5 }),
        json!({ "aoStrength": 5 }),
        json!({ "ao": "yes" }),
    ] {
        assert!(doc.execute("render.set", refused.clone()).is_err(), "{refused}");
    }
    assert_eq!(doc.inspect().unwrap()["render"]["aoDistance"].as_f64(), Some(0.25), "a refused change changes nothing");
    let reopened = SceneDocument::open("t.kscene", &doc.save_file(None).unwrap()).unwrap();
    let render = reopened.inspect().unwrap()["render"].clone();
    assert_eq!((render["ao"].as_bool(), render["aoDistance"].as_f64(), render["aoStrength"].as_f64()), (Some(false), Some(0.25), Some(1.5)));
    // null goes back to the automatic distance.
    run(&mut doc, "render.set", json!({ "aoDistance": null }));
    assert_eq!(doc.inspect().unwrap()["render"]["aoDistance"], Value::Null);
    run(&mut doc, "edit.undo", json!({}));
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(doc.inspect().unwrap()["render"]["ao"], true);
}

#[test]
fn the_defaults_render_as_before_and_strength_scales_the_darkening() {
    let mut doc = on_a_big_floor();
    let automatic = render(&mut doc);
    run(&mut doc, "render.set", json!({ "aoDistance": 0.3 }));
    run(&mut doc, "render.set", json!({ "ao": true, "aoDistance": null, "aoStrength": 1 }));
    assert_eq!(render(&mut doc), automatic, "the defaults are the automatic distance at full strength");
    run(&mut doc, "render.set", json!({ "ao": false }));
    let off = render(&mut doc);
    run(&mut doc, "render.set", json!({ "ao": true, "aoStrength": 0 }));
    assert_eq!(render(&mut doc), off, "no strength is no occlusion");
    run(&mut doc, "render.set", json!({ "aoStrength": 1, "aoDistance": 0.5 }));
    let one = render(&mut doc);
    run(&mut doc, "render.set", json!({ "aoStrength": 3 }));
    let three = render(&mut doc);
    assert!(mean(&one) < mean(&off), "occlusion darkens: {} against {}", mean(&one), mean(&off));
    assert!(mean(&three) < mean(&one) - 0.2, "more strength darkens more: {} against {}", mean(&three), mean(&one));
}

#[test]
fn a_chosen_distance_keeps_contact_shadows_on_a_big_floor() {
    let mut doc = on_a_big_floor();
    run(&mut doc, "render.set", json!({ "ao": false }));
    let off = render(&mut doc);
    run(&mut doc, "render.set", json!({ "ao": true, "aoDistance": 0.4 }));
    let chosen = render(&mut doc);
    // Where the cube meets the floor the occlusion darkens: count the pixels it darkens clearly.
    let darker = off.chunks(4).zip(chosen.chunks(4)).filter(|(a, b)| i32::from(a[1]) - i32::from(b[1]) > 8).count();
    assert!(darker > 20, "{darker} pixels darkened");
}
