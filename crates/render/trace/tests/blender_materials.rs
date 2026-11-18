//! Glass, depth of field, and a world image against the reference application's path tracer
//! (Cycles, Standard view transform, no denoising), built the same way on both sides. Glass is
//! compared with Cycles' Blur Glossy off: that default blurs glossy and glass lobes after a
//! diffuse bounce (a biased noise filter that spreads caustics into shadows), which this tracer
//! does not do. Runs when `KITION_BLENDER` names a Blender executable; otherwise it is skipped.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_scene_model::glam::Vec3;
use kition_scene_model::hdr;
use kition_scene_model::{Image, Light, LightKind, ObjectData, Primitive, Scene};
use kition_scene_trace::{TraceOptions, Tracer};

const WIDTH: u32 = 240;
const HEIGHT: u32 = 135;

const SCRIPT: &str = r#"
import bpy, sys, math
args = sys.argv[sys.argv.index('--') + 1:]
out, which, sky = args[0], args[1], args[2]
bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
world = bpy.data.worlds.new('W'); scene.world = world; world.use_nodes = True
background = world.node_tree.nodes['Background']
background.inputs[0].default_value = (0.05, 0.05, 0.05, 1)
def material(name, colour, roughness, transmission=0.0, ior=1.5):
    m = bpy.data.materials.new(name); m.use_nodes = True
    b = m.node_tree.nodes['Principled BSDF']
    b.inputs['Base Color'].default_value = colour
    b.inputs['Roughness'].default_value = roughness
    b.inputs['Transmission Weight'].default_value = transmission
    b.inputs['IOR'].default_value = ior
    return m
def camera(location, rotation):
    bpy.ops.object.camera_add(location=location, rotation=rotation)
    scene.camera = bpy.context.active_object
    return bpy.context.active_object
if which == 'glass':
    bpy.ops.mesh.primitive_plane_add(size=10)
    bpy.context.active_object.data.materials.append(material('Floor', (0.8, 0.8, 0.8, 1), 0.5))
    bpy.ops.mesh.primitive_uv_sphere_add(radius=1, location=(0, 0, 1))
    bpy.ops.object.shade_smooth()
    bpy.context.active_object.data.materials.append(material('Glass', (1, 1, 1, 1), 0.1, 1.0, 1.45))
    bpy.ops.object.light_add(type='AREA', location=(2, -1, 5)); light = bpy.context.active_object
    light.data.energy = 300; light.data.size = 2
    scene.cycles.blur_glossy = float(args[3]) if len(args) > 3 else 0.0
    camera((6, -6, 4), (math.radians(68), 0, math.radians(45)))
elif which == 'dof':
    bpy.ops.mesh.primitive_plane_add(size=20)
    bpy.context.active_object.data.materials.append(material('Floor', (0.8, 0.8, 0.8, 1), 0.5))
    for i, x in enumerate((-4, 0, 4)):
        bpy.ops.mesh.primitive_cube_add(size=1.5, location=(x, x * 1.5, 0.75))
        bpy.context.active_object.data.materials.append(material('C%d' % i, ((0.8, 0.1, 0.1, 1), (0.1, 0.8, 0.1, 1), (0.1, 0.1, 0.8, 1))[i], 0.5))
    bpy.ops.object.light_add(type='SUN', rotation=(math.radians(40), 0, math.radians(30))); bpy.context.active_object.data.energy = 3
    cam = camera((9, -9, 4), (math.radians(75), 0, math.radians(45)))
    cam.data.dof.use_dof = True; cam.data.dof.aperture_fstop = 0.4; cam.data.dof.focus_distance = 12.0
else:
    bpy.ops.mesh.primitive_uv_sphere_add(radius=1, location=(0, 0, 0))
    bpy.ops.object.shade_smooth()
    bpy.context.active_object.data.materials.append(material('White', (0.8, 0.8, 0.8, 1), 0.5))
    bpy.ops.mesh.primitive_cube_add(size=0.8, location=(1.6, 0.6, -0.6))
    bpy.context.active_object.data.materials.append(material('Mirror', (0.9, 0.9, 0.9, 1), 0.1))
    bpy.data.materials['Mirror'].node_tree.nodes['Principled BSDF'].inputs['Metallic'].default_value = 1.0
    environment = world.node_tree.nodes.new('ShaderNodeTexEnvironment')
    environment.image = bpy.data.images.load(sky)
    if which == 'hdri90':
        coordinates = world.node_tree.nodes.new('ShaderNodeTexCoord')
        mapping = world.node_tree.nodes.new('ShaderNodeMapping')
        mapping.inputs['Rotation'].default_value = (0, 0, math.radians(90))
        world.node_tree.links.new(coordinates.outputs['Generated'], mapping.inputs['Vector'])
        world.node_tree.links.new(mapping.outputs['Vector'], environment.inputs['Vector'])
    world.node_tree.links.new(environment.outputs['Color'], background.inputs['Color'])
    camera((4, -4, 1.5), (math.radians(80), 0, math.radians(45)))
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

fn material(scene: &mut Scene, name: &str, colour: [f32; 3], roughness: f32) -> u32 {
    let id = scene.add_material(name);
    let m = scene.material_mut(id).unwrap();
    m.base_color = [colour[0], colour[1], colour[2], 1.0];
    m.roughness = roughness;
    id
}

fn mesh(scene: &mut Scene, name: &str, primitive: Primitive, location: Vec3, material: u32, smooth: bool) {
    let mut built = primitive.build().unwrap();
    built.smooth = smooth;
    let id = scene.add_object(name, ObjectData::Mesh(built));
    let object = scene.object_mut(id).unwrap();
    object.location = location;
    object.material = Some(material);
}

fn camera(scene: &mut Scene, location: Vec3, rotation_degrees: Vec3) -> u32 {
    let id = scene.add_object("Camera", ObjectData::Camera(Default::default()));
    let object = scene.object_mut(id).unwrap();
    object.location = location;
    object.rotation = Vec3::new(rotation_degrees.x.to_radians(), rotation_degrees.y.to_radians(), rotation_degrees.z.to_radians());
    scene.camera = Some(id);
    id
}

fn glass() -> Scene {
    let mut scene = Scene::empty();
    scene.world.color = Vec3::splat(0.05);
    let floor = material(&mut scene, "Floor", [0.8; 3], 0.5);
    mesh(&mut scene, "Floor", Primitive::Plane { size: 10.0 }, Vec3::ZERO, floor, false);
    let glass = material(&mut scene, "Glass", [1.0; 3], 0.1);
    scene.material_mut(glass).unwrap().transmission = 1.0;
    scene.material_mut(glass).unwrap().ior = 1.45;
    mesh(&mut scene, "Sphere", Primitive::UvSphere { segments: 32, rings: 16, radius: 1.0 }, Vec3::new(0.0, 0.0, 1.0), glass, true);
    let mut light = Light::new(LightKind::Area);
    light.power = 300.0;
    light.radius = 2.0;
    let light = scene.add_object("Light", ObjectData::Light(light));
    scene.object_mut(light).unwrap().location = Vec3::new(2.0, -1.0, 5.0);
    camera(&mut scene, Vec3::new(6.0, -6.0, 4.0), Vec3::new(68.0, 0.0, 45.0));
    scene
}

fn depth_of_field() -> Scene {
    let mut scene = Scene::empty();
    scene.world.color = Vec3::splat(0.05);
    let floor = material(&mut scene, "Floor", [0.8; 3], 0.5);
    mesh(&mut scene, "Floor", Primitive::Plane { size: 20.0 }, Vec3::ZERO, floor, false);
    for (index, (x, colour)) in [(-4.0f32, [0.8, 0.1, 0.1]), (0.0, [0.1, 0.8, 0.1]), (4.0, [0.1, 0.1, 0.8])].into_iter().enumerate() {
        let id = material(&mut scene, &format!("C{index}"), colour, 0.5);
        mesh(&mut scene, &format!("Cube{index}"), Primitive::Cube { size: 1.5 }, Vec3::new(x, x * 1.5, 0.75), id, false);
    }
    let mut sun = Light::new(LightKind::Sun);
    sun.power = 3.0;
    let sun = scene.add_object("Sun", ObjectData::Light(sun));
    scene.object_mut(sun).unwrap().rotation = Vec3::new(40f32.to_radians(), 0.0, 30f32.to_radians());
    let id = camera(&mut scene, Vec3::new(9.0, -9.0, 4.0), Vec3::new(75.0, 0.0, 45.0));
    if let ObjectData::Camera(camera) = &mut scene.object_mut(id).unwrap().data {
        camera.dof.enabled = true;
        camera.dof.f_stop = 0.4;
        camera.dof.focus_distance = 12.0;
    }
    scene
}

/// A small sky: blue above with a bright warm sun to one side, grey-brown ground below.
fn sky() -> Vec<u8> {
    let (width, height) = (64usize, 32usize);
    let mut pixels = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let up = 1.0 - (y as f32 + 0.5) / height as f32 * 2.0;
            let pixel = if up > 0.0 { [0.15 + 0.2 * (1.0 - up), 0.3 + 0.2 * (1.0 - up), 0.9] } else { [0.25, 0.2, 0.15] };
            let sun = (x as i32 - 40).abs() <= 2 && (y as i32 - 10).abs() <= 2;
            pixels.push(if sun { [60.0, 50.0, 35.0] } else { pixel });
        }
    }
    hdr::encode(width, height, &pixels)
}

fn world_image(rotation_degrees: f32) -> Scene {
    let mut scene = Scene::empty();
    let white = material(&mut scene, "White", [0.8; 3], 0.5);
    mesh(&mut scene, "Sphere", Primitive::UvSphere { segments: 32, rings: 16, radius: 1.0 }, Vec3::ZERO, white, true);
    let mirror = material(&mut scene, "Mirror", [0.9; 3], 0.1);
    scene.material_mut(mirror).unwrap().metallic = 1.0;
    mesh(&mut scene, "Mirror", Primitive::Cube { size: 0.8 }, Vec3::new(1.6, 0.6, -0.6), mirror, false);
    let hash = scene.add_image(Image::new("sky.hdr", sky()).unwrap());
    scene.world.image = Some(hash);
    scene.world.rotation = rotation_degrees.to_radians();
    camera(&mut scene, Vec3::new(4.0, -4.0, 1.5), Vec3::new(80.0, 0.0, 45.0));
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

/// Mean absolute difference in levels after averaging 3 × 3 blocks, and the share of blocks more
/// than 16 levels apart.
fn compare(ours: &[u8], reference: &[u8]) -> (f64, f64) {
    let block = 3usize;
    let (w, h) = (WIDTH as usize / block, HEIGHT as usize / block);
    let (mut total, mut off) = (0.0, 0usize);
    for by in 0..h {
        for bx in 0..w {
            for channel in 0..3 {
                let mean = |image: &[u8]| {
                    let mut sum = 0.0;
                    for y in 0..block {
                        for x in 0..block {
                            sum += f64::from(image[((by * block + y) * WIDTH as usize + bx * block + x) * 3 + channel]);
                        }
                    }
                    sum / (block * block) as f64
                };
                let difference = (mean(ours) - mean(reference)).abs();
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
fn glass_depth_of_field_and_world_images_match_the_reference_path_tracer() {
    let Some(blender) = std::env::var_os("KITION_BLENDER") else {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    };
    let dir = std::env::temp_dir().join(format!("kition-scene-trace-materials-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("render.py"), SCRIPT).unwrap();
    let sky_path = dir.join("sky.hdr");
    std::fs::write(&sky_path, sky()).unwrap();
    // Name, scene, and bounds on the mean difference and the share of blocks off.
    let cases: [(&str, Scene, f64, f64); 4] = [
        ("glass", glass(), 2.0, 0.01),
        ("dof", depth_of_field(), 2.0, 0.01),
        ("hdri", world_image(0.0), 1.0, 0.005),
        ("hdri90", world_image(90.0), 1.2, 0.005),
    ];
    let mut report = Vec::new();
    for (name, mut scene, mean_limit, share_limit) in cases {
        let output = dir.join(format!("{name}.png"));
        let status = std::process::Command::new(&blender)
            .args(["-b", "--factory-startup", "--python"])
            .arg(dir.join("render.py"))
            .arg("--")
            .arg(&output)
            .arg(name)
            .arg(&sky_path)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "{name}: the reference failed");
        let reference = read_png(&output);
        scene.render.width = WIDTH;
        scene.render.height = HEIGHT;
        let mut tracer = Tracer::new(&scene, TraceOptions { samples: 512, bounces: 12, ..Default::default() }).unwrap();
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
        report.push((name, mean, share, mean_limit, share_limit));
    }
    for (name, mean, share, mean_limit, share_limit) in report {
        assert!(mean < mean_limit, "{name}: mean difference {mean:.2}");
        assert!(share < share_limit, "{name}: {:.2}% of blocks differ", share * 100.0);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
