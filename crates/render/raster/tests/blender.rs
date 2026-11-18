//! The camera render of the starter scene against the reference application's render of its own
//! default scene (EEVEE, Standard view transform). Runs when `KITION_BLENDER` names a Blender
//! executable; otherwise it is skipped.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_scene_model::Scene;
use kition_scene_raster::{RenderOptions, render_camera_with};

const SCRIPT: &str = r#"
import bpy, sys
scene = bpy.context.scene
scene.render.resolution_x, scene.render.resolution_y, scene.render.resolution_percentage = 480, 270, 100
engines = [item.identifier for item in bpy.types.RenderSettings.bl_rna.properties['engine'].enum_items]
scene.render.engine = 'BLENDER_EEVEE' if 'BLENDER_EEVEE' in engines else 'BLENDER_EEVEE_NEXT'
scene.view_settings.view_transform = 'Standard'
scene.render.image_settings.file_format = 'PNG'
scene.render.image_settings.color_mode = 'RGBA'
scene.render.filepath = sys.argv[sys.argv.index('--') + 1]
bpy.ops.render.render(write_still=True)
"#;

fn read_png(path: &std::path::Path) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).unwrap();
    buffer.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buffer,
        png::ColorType::Rgb => buffer.chunks(3).flat_map(|px| [px[0], px[1], px[2], 255]).collect(),
        other => panic!("unexpected PNG colour type {other:?}"),
    };
    (info.width, info.height, rgba)
}

#[test]
fn the_starter_scene_renders_like_the_reference() {
    let Some(blender) = std::env::var_os("KITION_BLENDER") else {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    };
    let dir = std::env::temp_dir().join(format!("kition-scene-reference-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("render.py");
    std::fs::write(&script, SCRIPT).unwrap();
    let output = dir.join("reference.png");
    let status = std::process::Command::new(blender)
        .args(["-b", "--factory-startup", "--python"])
        .arg(&script)
        .arg("--")
        .arg(&output)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "Blender failed");
    let (width, height, reference) = read_png(&output);

    let mut scene = Scene::starter();
    scene.render.width = width;
    scene.render.height = height;
    // Mean absolute difference per channel, and the share of pixels that differ by more than 24
    // levels in any channel (edges and the reference's soft shading).
    let compare = |options: RenderOptions| -> (f64, f64) {
        let ours = render_camera_with(&scene, &options).unwrap().to_rgba8();
        assert_eq!(ours.len(), reference.len());
        let mut total = 0u64;
        let mut off = 0usize;
        for (a, b) in ours.chunks(4).zip(reference.chunks(4)) {
            let worst = (0..3).map(|channel| a[channel].abs_diff(b[channel])).max().unwrap_or(0);
            total += (0..3).map(|channel| u64::from(a[channel].abs_diff(b[channel]))).sum::<u64>();
            if worst > 24 {
                off += 1;
            }
        }
        let pixels = (width * height) as usize;
        (total as f64 / (pixels * 3) as f64, off as f64 / pixels as f64)
    };
    let (plain_mean, plain_share) = compare(RenderOptions { shadows: false, ambient_occlusion: false, samples: 2 });
    let (mean, share) = compare(RenderOptions::default());
    eprintln!("without shadows and AO: mean difference {plain_mean:.2} levels, {:.2}% of pixels differ by more than 24", plain_share * 100.0);
    eprintln!("with shadows and AO:    mean difference {mean:.2} levels, {:.2}% of pixels differ by more than 24", share * 100.0);
    assert!(mean < 1.5, "mean difference {mean:.2}");
    assert!(share < 0.005, "{:.2}% of pixels differ", share * 100.0);
    assert!(mean <= plain_mean + 0.25, "shadows and AO took the render further from the reference");
    let _ = std::fs::remove_dir_all(&dir);
}
