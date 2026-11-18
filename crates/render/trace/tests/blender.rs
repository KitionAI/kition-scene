//! Path-traced renders against the reference application's path tracer (Cycles, Standard view
//! transform, no denoising) on the same scenes. Runs when `KITION_BLENDER` names a Blender
//! executable; otherwise it is skipped.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_scene_model::glam::Vec3;
use kition_scene_model::{Light, LightKind, ObjectData, Primitive, Scene};
use kition_scene_trace::{TraceOptions, Tracer};

const WIDTH: u32 = 240;
const HEIGHT: u32 = 135;

/// Two scenes the reference builds itself: its default scene, and a white cube on a red floor
/// under an area light (light bounces red onto the cube).
const SCRIPT: &str = r#"
import bpy, sys, math
out, which = sys.argv[sys.argv.index('--') + 1], sys.argv[sys.argv.index('--') + 2]
if which == 'bleed':
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    w = bpy.data.worlds.new('W'); scene.world = w; w.use_nodes = True
    w.node_tree.nodes['Background'].inputs[0].default_value = (0, 0, 0, 1)
    def material(name, colour, roughness):
        m = bpy.data.materials.new(name); m.use_nodes = True
        b = m.node_tree.nodes['Principled BSDF']
        b.inputs['Base Color'].default_value = colour; b.inputs['Roughness'].default_value = roughness
        return m
    bpy.ops.mesh.primitive_plane_add(size=10)
    bpy.context.active_object.data.materials.append(material('Red', (0.8, 0.05, 0.05, 1), 1.0))
    bpy.ops.mesh.primitive_cube_add(size=2, location=(0, 0, 1))
    bpy.context.active_object.data.materials.append(material('White', (0.8, 0.8, 0.8, 1), 0.5))
    bpy.ops.object.light_add(type='AREA', location=(0, 0, 4)); light = bpy.context.active_object
    light.data.energy = 500; light.data.size = 2
    bpy.ops.object.camera_add(location=(7.4, -7.4, 6), rotation=(math.radians(60), 0, math.radians(45)))
    scene.camera = bpy.context.active_object
scene = bpy.context.scene
scene.render.engine = 'CYCLES'
scene.cycles.device = 'CPU'
scene.cycles.samples = 1024
scene.cycles.use_denoising = False
scene.cycles.use_adaptive_sampling = False
scene.render.resolution_x, scene.render.resolution_y, scene.render.resolution_percentage = 240, 135, 100
scene.view_settings.view_transform = 'Standard'
scene.render.image_settings.file_format = 'PNG'
scene.render.image_settings.color_mode = 'RGB'
scene.render.filepath = out
bpy.ops.render.render(write_still=True)
"#;

fn bleed() -> Scene {
    let mut scene = Scene::empty();
    let red = scene.add_material("Red");
    let white = scene.add_material("White");
    {
        let m = scene.material_mut(red).unwrap();
        m.base_color = [0.8, 0.05, 0.05, 1.0];
        m.roughness = 1.0;
    }
    scene.material_mut(white).unwrap().base_color = [0.8, 0.8, 0.8, 1.0];
    let floor = scene.add_object("Plane", ObjectData::Mesh(Primitive::Plane { size: 10.0 }.build().unwrap()));
    scene.object_mut(floor).unwrap().material = Some(red);
    let cube = scene.add_object("Cube", ObjectData::Mesh(Primitive::Cube { size: 2.0 }.build().unwrap()));
    scene.object_mut(cube).unwrap().location = Vec3::new(0.0, 0.0, 1.0);
    scene.object_mut(cube).unwrap().material = Some(white);
    let mut light = Light::new(LightKind::Area);
    light.power = 500.0;
    light.radius = 2.0;
    let light = scene.add_object("Light", ObjectData::Light(light));
    scene.object_mut(light).unwrap().location = Vec3::new(0.0, 0.0, 4.0);
    let camera = scene.add_object("Camera", ObjectData::Camera(Default::default()));
    scene.object_mut(camera).unwrap().location = Vec3::new(7.4, -7.4, 6.0);
    scene.object_mut(camera).unwrap().rotation = Vec3::new(60f32.to_radians(), 0.0, 45f32.to_radians());
    scene.camera = Some(camera);
    scene.world.color = Vec3::ZERO;
    scene
}

fn read_png(path: &std::path::Path) -> Vec<u8> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).unwrap();
    buffer.truncate(info.buffer_size());
    match info.color_type {
        png::ColorType::Rgb => buffer,
        png::ColorType::Rgba => buffer.chunks(4).flat_map(|px| [px[0], px[1], px[2]]).collect(),
        other => panic!("{other:?}"),
    }
}

/// Mean absolute difference in levels, after averaging 3 × 3 blocks (so sampling noise on both
/// sides mostly cancels), and the share of blocks more than 16 levels apart.
fn compare(ours: &[u8], reference: &[u8]) -> (f64, f64) {
    let block = 3usize;
    let (w, h) = (WIDTH as usize / block, HEIGHT as usize / block);
    let (mut total, mut off) = (0.0, 0usize);
    for by in 0..h {
        for bx in 0..w {
            for channel in 0..3 {
                let mean = |image: &dyn Fn(usize) -> u8| {
                    let mut sum = 0.0;
                    for y in 0..block {
                        for x in 0..block {
                            sum += f64::from(image(((by * block + y) * WIDTH as usize + bx * block + x) * 3 + channel));
                        }
                    }
                    sum / (block * block) as f64
                };
                let difference = (mean(&|i| ours[i]) - mean(&|i| reference[i])).abs();
                total += difference;
                if difference > 16.0 {
                    off += 1;
                }
            }
        }
    }
    let count = (w * h * 3) as f64;
    (total / count, off as f64 / count)
}

#[test]
fn path_traced_renders_match_the_reference_path_tracer() {
    let Some(blender) = std::env::var_os("KITION_BLENDER") else {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    };
    let dir = std::env::temp_dir().join(format!("kition-scene-trace-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("render.py"), SCRIPT).unwrap();
    for (name, mut scene, mean_limit) in [("starter", Scene::starter(), 1.0), ("bleed", bleed(), 2.0)] {
        let output = dir.join(format!("{name}.png"));
        let status = std::process::Command::new(&blender)
            .args(["-b", "--factory-startup", "--python"])
            .arg(dir.join("render.py"))
            .arg("--")
            .arg(&output)
            .arg(name)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        let reference = read_png(&output);
        scene.render.width = WIDTH;
        scene.render.height = HEIGHT;
        let mut tracer = Tracer::new(&scene, TraceOptions { samples: 512, bounces: 8, ..Default::default() }).unwrap();
        tracer.finish();
        let ours: Vec<u8> = tracer.image().chunks(4).flat_map(|px| [px[0], px[1], px[2]]).collect();
        if let Some(snapshots) = std::env::var_os("KITION_SCENE_SNAPSHOTS") {
            let file = std::fs::File::create(std::path::Path::new(&snapshots).join(format!("trace-{name}.png"))).unwrap();
            let mut encoder = png::Encoder::new(file, WIDTH, HEIGHT);
            encoder.set_color(png::ColorType::Rgb);
            encoder.write_header().unwrap().write_image_data(&ours).unwrap();
            std::fs::copy(&output, std::path::Path::new(&snapshots).join(format!("cycles-{name}.png"))).unwrap();
        }
        let (mean, share) = compare(&ours, &reference);
        eprintln!("{name}: mean difference {mean:.2} levels; {:.2}% of 3x3 blocks differ by more than 16", share * 100.0);
        assert!(mean < mean_limit, "{name}: mean difference {mean:.2}");
        assert!(share < 0.03, "{name}: {:.2}% of blocks differ", share * 100.0);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
