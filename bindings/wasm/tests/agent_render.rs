//! `file.render`, the kit's agent preview (`agent.render` in kit.json, KHP A24, A39): the scene
//! seen from an explicit view (the scene's camera, another camera, or a named view framed to fit)
//! at an explicit time, in an explicit pass, or a labelled sheet of views or of times, as a pure
//! function of the scene. It never reads or moves the editor's viewport, the current frame, the
//! mode or the selection, never makes the document dirty, and answers the same bytes for the same
//! call.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;
use serde_json::{Value, json};

fn run(doc: &mut SceneDocument, command: &str, params: Value) -> Value {
    doc.execute(command, params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

/// The starter scene (a 2 m cube, a light, and the active camera "Camera") rendered at 320 x 180,
/// with the cube keyed to move from x = -2 at frame 1 to x = 2 at frame 25 (one second at the
/// default 24 fps), and a second camera, "Side", looking at it from the front. Saved, so it is
/// clean.
fn scene() -> SceneDocument {
    let mut doc = SceneDocument::create("Preview.kscene", 640, 360).unwrap();
    run(&mut doc, "render.set", json!({ "width": 320, "height": 180 }));
    run(&mut doc, "object.transform", json!({ "objects": "Cube", "location": [-2, 0, 0] }));
    run(&mut doc, "anim.insertKey", json!({ "objects": "Cube", "properties": ["location"], "frame": 1 }));
    run(&mut doc, "frame.set", json!({ "frame": 25 }));
    run(&mut doc, "object.transform", json!({ "objects": "Cube", "location": [2, 0, 0] }));
    run(&mut doc, "anim.insertKey", json!({ "objects": "Cube", "properties": ["location"], "frame": 25 }));
    run(&mut doc, "frame.set", json!({ "frame": 1 }));
    run(&mut doc, "add.camera", json!({ "name": "Side", "location": [0, -12, 1] }));
    run(&mut doc, "object.lookAt", json!({ "object": "Side", "target": [0, 0, 0] }));
    run(&mut doc, "select.none", json!({}));
    doc.mark_saved();
    doc
}

fn id_of(doc: &SceneDocument, name: &str) -> u64 {
    let inspected = doc.inspect().unwrap();
    inspected["objects"].as_array().unwrap().iter().find(|object| object["name"] == name).unwrap()["id"].as_u64().unwrap()
}

fn render(doc: &mut SceneDocument, params: Value) -> Value {
    run(doc, "file.render", params)
}

/// The refusal of `params`: its message and the JSON pointer of its first error.
fn refused(doc: &mut SceneDocument, params: Value) -> (String, String) {
    match doc.execute("file.render", params.clone()) {
        Ok(answer) => panic!("{params} rendered: {}", answer["frames"]),
        Err(error) => {
            assert!(error.starts_with("invalid-params: "), "not an invalid-params error: {error}");
            let (message, errors) = kition_kit::schema::parse_refusal(&error).unwrap_or_else(|| panic!("not a typed refusal: {error}"));
            (message.to_owned(), errors[0].path.clone())
        }
    }
}

/// Standard base64, padded.
fn base64_decode(text: &str) -> Vec<u8> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => panic!("not base64: {c}"),
    };
    let mut out = Vec::new();
    for chunk in text.as_bytes().chunks(4) {
        let digits: Vec<u8> = chunk.iter().copied().filter(|c| *c != b'=').map(value).collect();
        let n = digits.iter().enumerate().fold(0u32, |n, (i, d)| n | (u32::from(*d) << (18 - 6 * i)));
        out.extend((0..digits.len().saturating_sub(1)).map(|i| (n >> (16 - 8 * i)) as u8));
    }
    out
}

/// The answer's picture, decoded.
fn picture(answer: &Value) -> image::RgbaImage {
    let png = base64_decode(answer["png"].as_str().unwrap());
    image::load_from_memory(&png).unwrap().to_rgba8()
}

fn size(answer: &Value) -> (u64, u64) {
    (answer["width"].as_u64().unwrap(), answer["height"].as_u64().unwrap())
}

/// What the editor holds besides the scene, and the document's state: the viewport, the current
/// frame, the mode, the selection, history, dirty state, revision, and the file's bytes.
fn editor_state(doc: &SceneDocument) -> (Value, Value, Value, Value, Value, bool, u64, Vec<u8>) {
    let inspected = doc.inspect().unwrap();
    (
        inspected["view"].clone(),
        inspected["animation"]["frame"].clone(),
        inspected["mode"].clone(),
        inspected["selection"].clone(),
        json!([inspected["canUndo"], inspected["canRedo"]]),
        doc.dirty(),
        doc.revision(),
        doc.save(None).unwrap(),
    )
}

#[test]
fn rendering_reads_no_viewport_frame_mode_or_selection_and_changes_nothing() {
    let mut doc = scene();
    let calls = [
        json!({ "target": { "view": "front", "time": 0.5 } }),
        json!({ "target": { "view": "camera", "time": 0.5 } }),
        json!({ "target": { "views": ["persp", "top"], "time": 0.25, "pass": "id" } }),
        json!({ "target": { "range": [0, 1], "frames": 3, "view": "persp" } }),
    ];
    let before = editor_state(&doc);
    let first: Vec<Value> = calls.iter().map(|call| render(&mut doc, call.clone())).collect();
    assert_eq!(editor_state(&doc), before, "rendering moved the viewport, the frame, the mode or the selection, or dirtied the document");

    // Orbit the viewport, look through the camera, go to another frame, select, and edit the cube.
    run(&mut doc, "view.orbit", json!({ "turn": 70, "tilt": -20 }));
    run(&mut doc, "view.zoom", json!({ "factor": 0.4 }));
    run(&mut doc, "view.wireframe", json!({}));
    run(&mut doc, "frame.set", json!({ "frame": 40 }));
    run(&mut doc, "select.objects", json!({ "objects": "Cube" }));
    run(&mut doc, "mode.set", json!({ "mode": "edit", "object": "Cube" }));
    run(&mut doc, "mesh.select", json!({ "vertices": [0, 1, 2] }));
    let state = doc.inspect().unwrap();
    assert_eq!(state["mode"]["mode"], json!("edit"), "{}", state["mode"]);
    assert_eq!(state["animation"]["frame"], json!(40));
    let before = editor_state(&doc);
    for (call, first) in calls.iter().zip(&first) {
        let again = render(&mut doc, call.clone());
        assert_eq!(editor_state(&doc), before, "rendering moved the viewport, the frame, the mode or the selection, or dirtied the document");
        assert_eq!(again["png"], first["png"], "{call}: the same view at the same time is the same picture whatever the editor shows");
        assert_eq!(again["frames"], first["frames"], "{call}");
        assert_eq!(again["legend"], first["legend"], "{call}");
    }
    assert!(!doc.dirty(), "rendering leaves the document clean");
}

#[test]
fn named_views_are_different_pictures() {
    let mut doc = scene();
    let front = render(&mut doc, json!({ "target": { "view": "front" } }));
    let top = render(&mut doc, json!({ "target": { "view": "top" } }));
    let persp = render(&mut doc, json!({ "target": { "view": "persp" } }));
    let camera = render(&mut doc, json!({ "target": { "view": "camera" } }));
    for (a, b) in [(&front, &top), (&front, &persp), (&top, &persp), (&persp, &camera)] {
        assert_ne!(a["png"], b["png"], "{} and {} look the same", a["frames"][0]["view"], b["frames"][0]["view"]);
    }
    assert_eq!(front["frames"][0]["view"], json!("front"));
    assert_eq!(camera["frames"][0]["view"], json!("camera"));
    assert_eq!(camera["frames"][0]["camera"], json!(id_of(&doc, "Camera")));
    assert_eq!(size(&front), (320, 180), "one picture at the render size");
    assert_eq!((front["columns"].as_u64(), front["rows"].as_u64()), (Some(1), Some(1)));
    assert_eq!(front["mimeType"], json!("image/png"));
    assert_eq!(front["scale"], json!(1.0));
    // Without a time, the first frame: the cube at x = -2.
    assert_eq!((front["frames"][0]["time"].clone(), front["frames"][0]["frame"].clone()), (json!(0.0), json!(1)));
}

#[test]
fn a_fitted_view_shows_the_whole_scene_and_an_object_fills_it() {
    let mut doc = scene();
    run(&mut doc, "add.cylinder", json!({ "name": "Post", "radius": 0.3, "depth": 3, "location": [0, 2, 1.5] }));
    // Front, the scene's meshes fill the picture with a margin: the edges are background.
    let front = picture(&render(&mut doc, json!({ "target": { "view": "front", "pass": "id" } })));
    let (w, h) = front.dimensions();
    for x in 0..w {
        for y in [0, h - 1] {
            assert_eq!(front.get_pixel(x, y).0, [0, 0, 0, 255], "the fitted scene touches the edge at {x}, {y}");
        }
    }
    let covered = |img: &image::RgbaImage| img.pixels().filter(|p| p.0 != [0, 0, 0, 255]).count();
    let scene_cover = covered(&front);
    assert!(scene_cover > 1000, "the scene is in view: {scene_cover} pixels");
    // Framing the cube alone fills more of the picture; `objectId` takes an id or a diagnostic's
    // string id, `path` an object path.
    let cube = id_of(&doc, "Cube");
    let fitted = render(&mut doc, json!({ "target": { "view": "front", "pass": "id", "objectId": cube } }));
    assert!(covered(&picture(&fitted)) > scene_cover, "the cube fills more of the picture when it is framed alone");
    let by_string = render(&mut doc, json!({ "target": { "view": "front", "pass": "id", "objectId": cube.to_string() } }));
    let by_path = render(&mut doc, json!({ "target": { "view": "front", "pass": "id", "path": format!("object:{cube}") } }));
    assert_eq!(by_string["png"], fitted["png"]);
    assert_eq!(by_path["png"], fitted["png"]);
}

#[test]
fn two_times_are_two_pictures() {
    let mut doc = scene();
    let early = render(&mut doc, json!({ "target": { "view": "top", "time": 0 } }));
    let late = render(&mut doc, json!({ "target": { "view": "top", "time": 1 } }));
    assert_ne!(early["png"], late["png"], "the cube moved");
    assert_eq!(late["frames"][0]["frame"], json!(25), "1 s after frame 1 at 24 fps: {late}");
    assert_eq!(late["frames"][0]["time"], json!(1.0));
    // A time inside a frame shows the frame that started at or before it.
    let inside = render(&mut doc, json!({ "target": { "view": "top", "time": 1.02 } }));
    assert_eq!(inside["frames"][0]["frame"], json!(25));
    assert_eq!(inside["png"], late["png"]);
    // The camera sees the animation too, and the current frame stays where it was.
    let camera_early = render(&mut doc, json!({ "target": { "view": "camera", "time": 0 } }));
    let camera_late = render(&mut doc, json!({ "target": { "view": "camera", "time": 1 } }));
    assert_ne!(camera_early["png"], camera_late["png"]);
    assert_eq!(doc.inspect().unwrap()["animation"]["frame"], json!(1));
}

#[test]
fn another_camera_is_another_picture() {
    let mut doc = scene();
    let active = render(&mut doc, json!({ "target": { "view": "camera" } }));
    let side = render(&mut doc, json!({ "target": { "camera": "Side" } }));
    let side_id = id_of(&doc, "Side");
    let by_id = render(&mut doc, json!({ "target": { "camera": side_id } }));
    assert_ne!(active["png"], side["png"]);
    assert_eq!(side["png"], by_id["png"], "a camera by name or by id");
    assert_eq!(side["frames"][0]["view"], json!("camera"));
    assert_eq!(side["frames"][0]["camera"], json!(id_of(&doc, "Side")));
    assert_eq!(side["frames"][0]["label"].as_str().map(|label| label.starts_with("Side")), Some(true), "{side}");
    // Naming the active camera is the camera view.
    let named = render(&mut doc, json!({ "target": { "camera": "Camera" } }));
    assert_eq!(named["png"], active["png"]);
}

#[test]
fn the_same_call_answers_the_same_bytes() {
    let mut doc = scene();
    for params in [
        json!({}),
        json!({ "target": { "view": "persp", "time": 0.75 } }),
        json!({ "target": { "range": [0, 2], "frames": 5 }, "scale": 0.5 }),
        json!({ "target": { "views": ["front", { "camera": "Side" }], "pass": "depth" } }),
        json!({ "target": { "view": "top", "pass": "wireframe" } }),
    ] {
        let once = render(&mut doc, params.clone());
        let again = render(&mut doc, params.clone());
        assert_eq!(once, again, "{params}");
    }
}

#[test]
fn scale_and_max_size_size_the_picture() {
    let mut doc = scene();
    assert_eq!(size(&render(&mut doc, json!({ "target": { "view": "front" }, "scale": 0.5 }))), (160, 90));
    let double = render(&mut doc, json!({ "target": { "view": "front" }, "scale": 2 }));
    assert_eq!(size(&double), (640, 360));
    assert_eq!(double["scale"], json!(2.0));
    let capped = render(&mut doc, json!({ "target": { "view": "front" }, "scale": 2, "maxSize": 400 }));
    assert_eq!(size(&capped), (400, 225), "maxSize caps the longer side");
    assert!((capped["scale"].as_f64().unwrap() - 1.25).abs() < 1e-9, "{}", capped["scale"]);
    let sheet = render(&mut doc, json!({ "target": { "views": ["front", "top", "right", "persp"] }, "maxSize": 200 }));
    let (w, h) = size(&sheet);
    assert!(w <= 200 && h <= 200, "{w} x {h}");
    for (params, path) in [
        (json!({ "scale": 0 }), "/scale"),
        (json!({ "scale": 2.5 }), "/scale"),
        (json!({ "scale": "big" }), "/scale"),
        (json!({ "maxSize": 15 }), "/maxSize"),
        (json!({ "maxSize": 4097 }), "/maxSize"),
    ] {
        let (message, pointer) = refused(&mut doc, params.clone());
        assert_eq!(pointer, path, "{params}: {message}");
    }
}

/// White text on a dark box in the bottom-left quarter of a cell (its burned-in label): how many
/// white pixels, when there is a dark box as well.
fn label_pixels(img: &image::RgbaImage, x0: u32, y0: u32, w: u32, h: u32) -> usize {
    let corner: Vec<[u8; 4]> = (x0..x0 + w / 2).flat_map(|x| (y0 + h / 2..y0 + h).map(move |y| (x, y))).map(|(x, y)| img.get_pixel(x, y).0).collect();
    let dark = corner.iter().filter(|p| p[..3].iter().all(|c| *c < 90)).count();
    if dark < 50 { 0 } else { corner.iter().filter(|p| p[..3].iter().all(|c| *c > 250)).count() }
}

#[test]
fn a_sheet_of_views_is_one_labelled_picture() {
    let mut doc = scene();
    let sheet = render(&mut doc, json!({ "target": { "views": ["front", "top", "right", { "camera": "Side" }], "time": 0.5 } }));
    assert_eq!((sheet["columns"].as_u64(), sheet["rows"].as_u64()), (Some(2), Some(2)));
    assert_eq!(size(&sheet), (642, 362), "two 320 x 180 cells across and down, 2 px apart");
    let frames = sheet["frames"].as_array().unwrap();
    let views: Vec<&str> = frames.iter().map(|frame| frame["view"].as_str().unwrap()).collect();
    assert_eq!(views, ["front", "top", "right", "camera"]);
    let labels: Vec<&str> = frames.iter().map(|frame| frame["label"].as_str().unwrap()).collect();
    assert_eq!(labels, ["front", "top", "right", "Side"]);
    for frame in frames {
        assert_eq!((frame["time"].clone(), frame["frame"].clone()), (json!(0.5), json!(13)), "{frame}");
    }
    let shown = picture(&sheet);
    for (x0, y0) in [(0, 0), (322, 0), (0, 182), (322, 182)] {
        assert!(label_pixels(&shown, x0, y0, 320, 180) > 10, "no label in the cell at {x0}, {y0}");
    }
    // The cells are the single views, labels aside: the top right quarter of each is untouched.
    let top = picture(&render(&mut doc, json!({ "target": { "view": "top", "time": 0.5 } })));
    for x in 200..320 {
        for y in 0..60 {
            assert_eq!(shown.get_pixel(322 + x, y), top.get_pixel(x, y), "{x}, {y}");
        }
    }
    let six = render(&mut doc, json!({ "target": { "views": ["front", "back", "left", "right", "top", "bottom"] } }));
    assert_eq!((six["columns"].as_u64(), six["rows"].as_u64()), (Some(3), Some(2)));
}

#[test]
fn a_sheet_over_time_is_one_labelled_picture() {
    let mut doc = scene();
    let sheet = render(&mut doc, json!({ "target": { "view": "top", "range": [0, 1], "frames": 4 } }));
    let frames = sheet["frames"].as_array().unwrap();
    assert_eq!(frames.len(), 4);
    let numbers: Vec<i64> = frames.iter().map(|frame| frame["frame"].as_i64().unwrap()).collect();
    assert_eq!(numbers, [1, 9, 17, 25], "spread evenly from 0 s to 1 s, both included");
    assert_eq!(frames[0]["label"], json!("0 s, frame 1"));
    assert_eq!(frames[3]["label"], json!("1 s, frame 25"));
    assert_eq!(frames[1]["time"], json!(0.333333));
    for frame in frames {
        assert_eq!(frame["view"], json!("top"), "{frame}");
    }
    assert_eq!((sheet["columns"].as_u64(), sheet["rows"].as_u64()), (Some(2), Some(2)));
    assert_eq!(size(&sheet), (642, 362));
    let shown = picture(&sheet);
    for (x0, y0) in [(0, 0), (322, 0), (0, 182), (322, 182)] {
        assert!(label_pixels(&shown, x0, y0, 320, 180) > 10, "no label in the cell at {x0}, {y0}");
    }
    // One framing for the whole sheet, so the motion shows: the first and last cells differ.
    let first: Vec<_> = (160..320).flat_map(|x| (0..90).map(move |y| (x, y))).map(|(x, y)| *shown.get_pixel(x, y)).collect();
    let last: Vec<_> = (160..320).flat_map(|x| (0..90).map(move |y| (x, y))).map(|(x, y)| *shown.get_pixel(322 + x, 182 + y)).collect();
    assert_ne!(first, last);
    // Without a view, a sheet over time looks through the scene's camera; the range's end is the
    // last frame.
    let whole = render(&mut doc, json!({ "target": { "range": [0, 250.0 / 24.0] } }));
    let frames = whole["frames"].as_array().unwrap();
    assert_eq!(frames.len(), 6, "six frames by default");
    assert_eq!(frames[5]["frame"], json!(250));
    assert_eq!(frames[0]["view"], json!("camera"));
    let twelve = render(&mut doc, json!({ "target": { "range": [0, 1], "frames": 12, "view": "front" } }));
    assert_eq!((twelve["columns"].as_u64(), twelve["rows"].as_u64()), (Some(4), Some(3)));
}

#[test]
fn without_a_target_it_shows_four_views() {
    let mut doc = scene();
    let whole = render(&mut doc, json!({}));
    let views: Vec<&str> = whole["frames"].as_array().unwrap().iter().map(|frame| frame["view"].as_str().unwrap()).collect();
    assert_eq!(views, ["camera", "front", "right", "top"]);
    assert_eq!((whole["columns"].as_u64(), whole["rows"].as_u64()), (Some(2), Some(2)));
    assert_eq!(whole["frames"][0]["frame"], json!(1), "the first frame");
    // Without a camera, a three-quarter view instead.
    run(&mut doc, "edit.delete", json!({ "objects": ["Camera", "Side"] }));
    let whole = render(&mut doc, json!({}));
    let views: Vec<&str> = whole["frames"].as_array().unwrap().iter().map(|frame| frame["view"].as_str().unwrap()).collect();
    assert_eq!(views, ["persp", "front", "right", "top"]);
}

#[test]
fn the_id_pass_colours_each_object_and_names_the_colours() {
    let mut doc = scene();
    run(&mut doc, "add.cylinder", json!({ "name": "Post", "radius": 0.3, "depth": 3, "location": [0, 2, 1.5] }));
    let answer = render(&mut doc, json!({ "target": { "view": "front", "pass": "id" } }));
    let legend = answer["legend"].as_array().unwrap_or_else(|| panic!("an id pass answers its legend: {answer}"));
    let names: Vec<&str> = legend.iter().map(|entry| entry["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Cube", "Post"], "the scene's meshes, in scene order");
    assert_eq!(legend[0]["id"], json!(id_of(&doc, "Cube")));
    assert_eq!(legend[1]["id"], json!(id_of(&doc, "Post")));
    let colours: Vec<[u8; 3]> = legend
        .iter()
        .map(|entry| {
            let hex = entry["color"].as_str().unwrap().trim_start_matches('#');
            let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap();
            [channel(0), channel(2), channel(4)]
        })
        .collect();
    assert_ne!(colours[0], colours[1], "distinct colours");
    let shown = picture(&answer);
    let mut seen = std::collections::BTreeMap::new();
    for pixel in shown.pixels() {
        let rgb = [pixel.0[0], pixel.0[1], pixel.0[2]];
        assert!(rgb == [0, 0, 0] || colours.contains(&rgb), "a pixel of no legend colour: {rgb:?}");
        *seen.entry(rgb).or_insert(0) += 1;
    }
    for colour in &colours {
        assert!(seen.get(colour).copied().unwrap_or(0) > 100, "{colour:?} is drawn");
    }
    // The cube is at x = -2 at the first frame: left of the post, from the front.
    let column = |rgb: [u8; 3]| {
        let xs: Vec<u32> = shown.enumerate_pixels().filter(|(_, _, p)| [p.0[0], p.0[1], p.0[2]] == rgb).map(|(x, _, _)| x).collect();
        xs.iter().sum::<u32>() / xs.len() as u32
    };
    assert!(column(colours[0]) < column(colours[1]), "the cube is left of the post");
    // The colours stay with the objects from call to call and from view to view.
    let top = render(&mut doc, json!({ "target": { "view": "top", "pass": "id" } }));
    assert_eq!(top["legend"], answer["legend"]);
    // Other passes have no legend.
    assert!(render(&mut doc, json!({ "target": { "view": "front" } })).get("legend").is_none());
}

#[test]
fn depth_and_wireframe_are_their_own_pictures() {
    let mut doc = scene();
    let color = render(&mut doc, json!({ "target": { "view": "persp" } }));
    let depth = render(&mut doc, json!({ "target": { "view": "persp", "pass": "depth" } }));
    let wire = render(&mut doc, json!({ "target": { "view": "persp", "pass": "wireframe" } }));
    assert_ne!(color["png"], depth["png"]);
    assert_ne!(color["png"], wire["png"]);
    assert_ne!(depth["png"], wire["png"]);
    // Depth is grey: nearer is lighter, nothing is black.
    let shown = picture(&depth);
    assert!(shown.pixels().all(|p| p.0[0] == p.0[1] && p.0[1] == p.0[2]), "a grey picture");
    assert_eq!(shown.get_pixel(0, 0).0, [0, 0, 0, 255], "nothing there is black");
    assert!(shown.pixels().any(|p| p.0[0] > 200), "the nearest surface is light");
}

#[test]
fn every_wrong_target_is_a_typed_refusal_that_says_what_to_do() {
    let mut doc = scene();
    let cases = [
        (json!({ "view": "isometric" }), "/target/view", "front"),
        (json!({ "camera": "Nope" }), "/target/camera", "\"Side\""),
        (json!({ "camera": "Cube" }), "/target/camera", "\"Camera\""),
        (json!({ "view": "front", "camera": "Side" }), "/target/camera", "not both"),
        (json!({ "views": ["front", "top"], "range": [0, 1] }), "/target/range", "one axis"),
        (json!({ "views": ["front", "top"], "view": "top" }), "/target/views", "not both"),
        (json!({ "views": ["front"] }), "/target/views", "2 to 6"),
        (json!({ "views": ["front", "top", "left", "right", "back", "bottom", "persp"] }), "/target/views", "2 to 6"),
        (json!({ "views": ["front", "sideways"] }), "/target/views/1", "persp"),
        (json!({ "views": ["front", { "camera": "Nope" }] }), "/target/views/1/camera", "\"Side\""),
        (json!({ "time": 11 }), "/target/time", "10.417 s"),
        (json!({ "time": -1 }), "/target/time", "10.417 s"),
        (json!({ "time": "soon" }), "/target/time", "seconds"),
        (json!({ "time": 1, "range": [0, 1] }), "/target/time", "not both"),
        (json!({ "range": [0, 1], "frames": 13 }), "/target/frames", "2 to 12"),
        (json!({ "range": [0, 1], "frames": 1 }), "/target/frames", "2 to 12"),
        (json!({ "frames": 3 }), "/target/frames", "range"),
        (json!({ "range": [1, 0.5] }), "/target/range", "10.417"),
        (json!({ "range": [0, 20] }), "/target/range", "10.417"),
        (json!({ "range": [0] }), "/target/range", "[from, to]"),
        (json!({ "view": "camera", "objectId": 2 }), "/target/objectId", "named view"),
        (json!({ "camera": "Side", "objectId": 2 }), "/target/objectId", "named view"),
        (json!({ "views": ["front", "camera"], "objectId": 2 }), "/target/objectId", "named view"),
        (json!({ "objectId": 999 }), "/target/objectId", "\"Cube\""),
        (json!({ "path": "layer:3" }), "/target/path", "object:"),
        (json!({ "pass": "normal" }), "/target/pass", "wireframe"),
        (json!({ "sequence": 1 }), "/target/sequence", "view"),
    ];
    for (target, path, hint) in cases {
        let (message, pointer) = refused(&mut doc, json!({ "target": target }));
        assert_eq!(pointer, path, "{target}: {message}");
        assert!(message.contains(hint), "{target}: the message does not say {hint:?}: {message}");
    }
    let (message, pointer) = refused(&mut doc, json!({ "target": "front" }));
    assert_eq!(pointer, "/target", "{message}");
    // Deleting the active camera makes another one active; with none left, the camera's view and
    // a named camera say there is none and what to do instead.
    run(&mut doc, "edit.delete", json!({ "objects": ["Camera", "Side"] }));
    let (message, pointer) = refused(&mut doc, json!({ "target": { "view": "camera" } }));
    assert_eq!(pointer, "/target/view", "{message}");
    assert!(message.contains("no active camera") && message.contains("add.camera"), "{message}");
    let (message, _) = refused(&mut doc, json!({ "target": { "camera": "Side" } }));
    assert!(message.contains("no cameras"), "{message}");
}
