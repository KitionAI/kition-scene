//! Colour management through commands: the view transform, look, and exposure are render
//! settings (refused when out of range, undone, saved), the default is plain clipping at white,
//! and fast renders, path-traced renders, and composited renders all show them.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn starter() -> SceneDocument {
    let made = SceneDocument::create("Untitled.kscene", 1280, 720).unwrap();
    let mut doc = SceneDocument::open("t.kscene", &made.save(None).unwrap()).unwrap();
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

/// Pixels with a channel at full white (clipped highlights).
fn clipped(rgba: &[u8]) -> usize {
    rgba.chunks(4).filter(|pixel| pixel[..3].contains(&255)).count()
}

fn mean(rgba: &[u8]) -> f64 {
    let sum: u64 = rgba.chunks(4).map(|pixel| u64::from(pixel[0]) + u64::from(pixel[1]) + u64::from(pixel[2])).sum();
    sum as f64 / (rgba.len() / 4 * 3) as f64
}

/// The starter scene with its light so bright that the cube's lit faces clip in Standard.
fn overexposed() -> SceneDocument {
    let mut doc = starter();
    run(&mut doc, "light.set", json!({ "object": "Light", "power": 20000 }));
    doc
}

#[test]
fn colour_management_is_a_render_setting() {
    let mut doc = starter();
    let render = doc.inspect().unwrap()["render"].clone();
    assert_eq!((render["view"].as_str(), render["look"].as_str(), render["exposure"].as_f64()), (Some("standard"), Some("none"), Some(0.0)));
    run(&mut doc, "render.set", json!({ "view": "agx", "look": "punchy", "exposure": 1.5 }));
    let render = doc.inspect().unwrap()["render"].clone();
    assert_eq!((render["view"].as_str(), render["look"].as_str(), render["exposure"].as_f64()), (Some("agx"), Some("punchy"), Some(1.5)));
    for refused in
        [json!({ "view": "filmic-log" }), json!({ "look": "golden" }), json!({ "exposure": 40 }), json!({ "view": "standard", "exposure": -33 })]
    {
        assert!(doc.execute("render.set", refused.clone()).is_err(), "{refused}");
    }
    assert_eq!(doc.inspect().unwrap()["render"]["view"], "agx", "a refused change changes nothing");
    let reopened = SceneDocument::open("t.kscene", &doc.save(None).unwrap()).unwrap();
    let render = reopened.inspect().unwrap()["render"].clone();
    assert_eq!((render["view"].as_str(), render["look"].as_str(), render["exposure"].as_f64()), (Some("agx"), Some("punchy"), Some(1.5)));
    run(&mut doc, "edit.undo", json!({}));
    assert_eq!(doc.inspect().unwrap()["render"]["view"], "standard");
}

#[test]
fn standard_without_exposure_renders_as_before() {
    let mut doc = overexposed();
    let before = render(&mut doc);
    run(&mut doc, "render.set", json!({ "view": "agx" }));
    run(&mut doc, "render.set", json!({ "view": "standard", "look": "none", "exposure": 0 }));
    assert_eq!(render(&mut doc), before);
}

#[test]
fn agx_rolls_off_the_highlights_standard_clips() {
    let mut doc = overexposed();
    let standard = render(&mut doc);
    run(&mut doc, "render.set", json!({ "view": "agx" }));
    let agx = render(&mut doc);
    assert!(clipped(&standard) > 50, "the bright cube clips in Standard: {}", clipped(&standard));
    assert!(clipped(&agx) * 10 < clipped(&standard), "AgX keeps highlights below white: {} against {}", clipped(&agx), clipped(&standard));
    run(&mut doc, "render.set", json!({ "view": "khronos-pbr-neutral" }));
    let neutral = render(&mut doc);
    assert!(clipped(&neutral) < clipped(&standard), "{} against {}", clipped(&neutral), clipped(&standard));
    // Exposure is in stops: one stop down is darker, in any view.
    run(&mut doc, "render.set", json!({ "view": "standard", "exposure": -1 }));
    assert!(mean(&render(&mut doc)) < mean(&standard) - 5.0);
}

#[test]
fn path_traced_and_composited_renders_show_the_view_transform() {
    let mut doc = overexposed();
    let trace = |doc: &mut SceneDocument| pixels(&run(doc, "render.trace.start", json!({ "samples": 4, "wait": true })));
    let standard = trace(&mut doc);
    run(&mut doc, "render.set", json!({ "view": "agx" }));
    let agx = trace(&mut doc);
    assert!(clipped(&agx) < clipped(&standard), "path traced: {} against {}", clipped(&agx), clipped(&standard));
    // A compositing group passes the light through; the view transform comes after it.
    run(&mut doc, "compositor.new", json!({}));
    let composited = render(&mut doc);
    run(&mut doc, "render.set", json!({ "view": "standard" }));
    let composited_standard = render(&mut doc);
    assert!(clipped(&composited) < clipped(&composited_standard), "composited: {} against {}", clipped(&composited), clipped(&composited_standard));
}
