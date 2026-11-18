//! The viewport and camera renders of the starter scene: what is drawn where.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::collections::BTreeSet;

use kition_scene_model::Scene;
use kition_scene_raster::{Overlays, Shading, ViewState, hit, render_camera, render_viewport};

fn save(name: &str, canvas: &kition_scene_raster::Canvas) {
    // KITION_SCENE_SNAPSHOTS=<dir> writes the pictures for a person to look at.
    let Some(dir) = std::env::var_os("KITION_SCENE_SNAPSHOTS") else { return };
    let file = std::fs::File::create(std::path::Path::new(&dir).join(format!("{name}.png"))).unwrap();
    let mut encoder = png::Encoder::new(file, canvas.width, canvas.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.write_header().unwrap().write_image_data(&canvas.to_rgba8()).unwrap();
}

#[test]
fn the_cube_fills_the_middle_of_the_default_view() {
    let scene = Scene::starter();
    let cube = scene.object_named("Cube").unwrap().id;
    let overlays = Overlays {
        grid: true,
        selected: BTreeSet::from([cube]),
        active: Some(cube),
        edit: None,
        bones: None,
        weights: None,
        curve: None,
        caret: None,
        peers: Vec::new(),
        sculpt: None,
        hide_extras: false,
    };
    let canvas = render_viewport(&scene, &ViewState::default(), 640, 400, Shading::Solid, &overlays);
    save("viewport-solid", &canvas);
    assert_eq!(hit(&canvas, 320, 200).map(|found| found.object), Some(cube));
    assert_eq!(hit(&canvas, 5, 5), None);
    let wire = render_viewport(&scene, &ViewState::default(), 640, 400, Shading::Wireframe, &overlays);
    save("viewport-wire", &wire);
    let material = render_viewport(&scene, &ViewState::default(), 640, 400, Shading::Material, &overlays);
    save("viewport-material", &material);
}

#[test]
fn the_camera_render_sees_a_lit_cube() {
    let mut scene = Scene::starter();
    scene.render.width = 480;
    scene.render.height = 270;
    let canvas = render_camera(&scene, 2).unwrap();
    save("render", &canvas);
    assert_eq!((canvas.width, canvas.height), (480, 270));
    let centre = canvas.pixel(240, 135).unwrap();
    let corner = canvas.pixel(2, 2).unwrap();
    assert!(centre[0] > corner[0] + 0.05, "the cube is brighter than the world: {centre:?} {corner:?}");
}

#[test]
fn sculpt_mode_tints_face_sets_and_leaves_hidden_faces_out() {
    use kition_scene_model::{ObjectData, Primitive};
    let mut scene = Scene::empty();
    let mesh = Primitive::Grid { x_subdivisions: 20, y_subdivisions: 20, size: 4.0 }.build().unwrap();
    let centres: Vec<f32> = mesh.faces.iter().map(|face| mesh.face_center(face).x).collect();
    let clay = scene.add_object("Clay", ObjectData::Mesh(mesh));
    let sculpt = kition_scene_raster::SculptOverlay {
        object: clay,
        face_sets: centres.iter().map(|x| u32::from(*x > 0.0)).collect(),
        hidden: centres.iter().map(|x| *x < -1.0).collect(),
        ..Default::default()
    };
    let overlays = Overlays { sculpt: Some(sculpt), ..Default::default() };
    let canvas = render_viewport(&scene, &ViewState::default(), 640, 400, Shading::Solid, &overlays);
    save("viewport-sculpt-face-sets", &canvas);
    let (mut tinted, mut plain, mut hidden) = (Vec::new(), Vec::new(), 0);
    for y in (0..400).step_by(4) {
        for x in (0..640).step_by(4) {
            let Some(found) = hit(&canvas, x, y) else { continue };
            let centre = centres[found.face as usize];
            let [r, g, b, _] = canvas.pixel(x, y).unwrap();
            if centre < -1.0 {
                hidden += 1;
            } else if centre > 0.0 {
                tinted.push((r, g, b));
            } else {
                plain.push((r, g, b));
            }
        }
    }
    assert_eq!(hidden, 0, "hidden faces are not drawn");
    assert!(!tinted.is_empty() && !plain.is_empty());
    // Untinted clay is grey; the set's tint moves the channels apart.
    let spread = |list: &[(f32, f32, f32)]| list.iter().map(|(r, g, b)| r.max(*g).max(*b) - r.min(*g).min(*b)).sum::<f32>() / list.len() as f32;
    assert!(spread(&tinted) > spread(&plain) + 0.03, "{} vs {}", spread(&tinted), spread(&plain));
}

#[test]
fn a_custom_shape_draws_a_bone_with_its_faces_or_as_wire() {
    use kition_scene_model::glam::Vec3;
    use kition_scene_model::{Armature, Bone, BoneShape, ObjectData, Primitive};
    let mut scene = Scene::empty();
    let shape = scene.add_object("Shape", ObjectData::Mesh(Primitive::Plane { size: 1.0 }.build().unwrap()));
    let object = scene.object_mut(shape).unwrap();
    object.location = Vec3::new(30.0, 30.0, 30.0);
    object.hidden = true;
    let mut bone = Bone::new("Bone", Vec3::ZERO, Vec3::Z);
    // The plane stands in the bone's X and Y axes (world X and Z), lifted half a metre up the bone.
    bone.shape = Some(BoneShape { translation: Vec3::new(0.0, 0.5, 0.0), ..BoneShape::new(shape) });
    let rig = scene.add_object("Armature", ObjectData::Armature(Armature { bones: vec![bone] }));
    let view = ViewState::default();
    let projector = view.projector(&scene, 320, 240);
    let centre = projector.to_screen(projector.view_projection() * Vec3::new(0.0, 0.0, 0.5).extend(1.0));
    let edge = projector.to_screen(projector.view_projection() * Vec3::new(0.5, 0.0, 0.5).extend(1.0));
    let overlays = Overlays::default();
    let filled = render_viewport(&scene, &view, 320, 240, Shading::Solid, &overlays);
    save("bone-shape", &filled);
    let background = render_viewport(&Scene::empty(), &view, 320, 240, Shading::Solid, &overlays);
    let (x, y) = (centre.x as u32, centre.y as u32);
    let fill = kition_scene_raster::shape_fill(kition_scene_raster::draw::hex(0x1a_1a_1a));
    let got = filled.pixel(x, y).unwrap();
    assert!((0..3).all(|channel| (got[channel] - fill[channel]).abs() < 1e-3), "the shape's face is filled: {got:?} against {fill:?}");
    assert_eq!(hit(&filled, x, y), None, "a shape's faces are not picked");
    let on_edge = filled.pixel(edge.x as u32, edge.y as u32).unwrap();
    assert!(on_edge[0] < 0.1, "the shape's edge is drawn: {on_edge:?}");
    let ObjectData::Armature(armature) = &mut scene.object_mut(rig).unwrap().data else { panic!() };
    armature.bones[0].shape.as_mut().unwrap().wire = true;
    let wire = render_viewport(&scene, &view, 320, 240, Shading::Solid, &overlays);
    assert_eq!(wire.pixel(x, y), background.pixel(x, y), "a wire shape leaves its faces out");
    assert!(wire.pixel(edge.x as u32, edge.y as u32).unwrap()[0] < 0.1, "and draws its edges");
}
