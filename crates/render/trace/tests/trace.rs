//! The tracer's pieces and whole renders.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::expect_used)]

use kition_scene_model::glam::Vec3;
use kition_scene_model::{Light, LightKind, ObjectData, Primitive, Scene};
use kition_scene_trace::bvh::{Bvh, Triangle};
use kition_scene_trace::sampling::Rng;
use kition_scene_trace::{TraceOptions, Tracer};

#[test]
fn the_hierarchy_finds_what_testing_every_triangle_finds() {
    let mut rng = Rng::for_sample(1, 2, 3);
    let mut point = || Vec3::new(rng.uniform() * 10.0 - 5.0, rng.uniform() * 10.0 - 5.0, rng.uniform() * 10.0 - 5.0);
    let triangles: Vec<Triangle> = (0..2000)
        .map(|_| {
            let a = point();
            let offset = Vec3::new(0.4, 0.1, -0.3);
            Triangle::new(a, a + offset, a + offset.cross(Vec3::Z) + Vec3::Z * 0.2)
        })
        .collect();
    let bvh = Bvh::build(triangles.clone());
    let mut rng = Rng::for_sample(9, 9, 9);
    for _ in 0..2000 {
        let origin = Vec3::new(rng.uniform() * 12.0 - 6.0, rng.uniform() * 12.0 - 6.0, rng.uniform() * 12.0 - 6.0);
        let direction = Vec3::new(rng.uniform() - 0.5, rng.uniform() - 0.5, rng.uniform() - 0.5).normalize();
        let brute = triangles
            .iter()
            .enumerate()
            .filter_map(|(index, triangle)| triangle.intersect(origin, direction, 0.0, f32::INFINITY).map(|(t, _, _)| (t, index)))
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let found = bvh.intersect(origin, direction, 0.0, f32::INFINITY, false).map(|hit| (hit.distance, hit.triangle as usize));
        match (brute, found) {
            (None, None) => {}
            (Some((t, index)), Some((u, found))) => {
                assert!((t - u).abs() < 1e-5, "{t} vs {u}");
                assert!(index == found || (t - u).abs() < 1e-6);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(brute.is_some(), bvh.intersect(origin, direction, 0.0, f32::INFINITY, true).is_some());
    }
}

/// A white sphere inside a uniformly lit world reflects the world's light (white furnace).
#[test]
fn a_white_furnace_stays_uniform() {
    let mut scene = Scene::empty();
    let sphere = scene.add_object("Sphere", ObjectData::Mesh(Primitive::UvSphere { segments: 32, rings: 16, radius: 1.0 }.build().unwrap()));
    let material = scene.add_material("White");
    {
        let m = scene.material_mut(material).unwrap();
        m.base_color = [1.0, 1.0, 1.0, 1.0];
        m.roughness = 1.0;
    }
    scene.object_mut(sphere).unwrap().material = Some(material);
    let camera = scene.add_object("Camera", ObjectData::Camera(Default::default()));
    scene.object_mut(camera).unwrap().location = Vec3::new(0.0, -6.0, 0.0);
    scene.object_mut(camera).unwrap().rotation = Vec3::new(std::f32::consts::FRAC_PI_2, 0.0, 0.0);
    scene.camera = Some(camera);
    scene.world.color = Vec3::splat(0.5);
    scene.world.strength = 1.0;
    scene.render.width = 48;
    scene.render.height = 48;
    let mut tracer = Tracer::new(&scene, TraceOptions { samples: 256, bounces: 32, clamp_indirect: 0.0, ..Default::default() }).unwrap();
    tracer.finish();
    let picture = tracer.linear();
    // The sphere in the middle reflects most of the world's 0.5 (the specular layer loses a little).
    let centre = picture[24 * 48 + 24];
    assert!(centre.x > 0.42 && centre.x <= 0.52, "{centre}");
}

fn starter(width: u32, height: u32) -> Scene {
    let mut scene = Scene::starter();
    scene.render.width = width;
    scene.render.height = height;
    scene
}

#[test]
fn renders_are_reproducible_however_the_work_is_split() {
    let scene = starter(64, 36);
    let options = TraceOptions { samples: 4, ..Default::default() };
    let mut whole = Tracer::new(&scene, options).unwrap();
    whole.finish();
    let mut pieces = Tracer::new(&scene, options).unwrap();
    while !pieces.step_rows(7).done {}
    assert_eq!(whole.image(), pieces.image());
    let mut other = Tracer::new(&scene, TraceOptions { seed: 1, ..options }).unwrap();
    other.finish();
    assert_ne!(whole.image(), other.image());
}

#[test]
fn shards_merged_make_the_single_tracers_picture_exactly() {
    let scene = starter(48, 27);
    for denoise in [false, true] {
        let options = TraceOptions { samples: 5, denoise, ..Default::default() };
        let mut whole = Tracer::new(&scene, options).unwrap();
        whole.finish();
        for count in [1, 3, 8] {
            let mut collector = Tracer::collector(&scene, options).unwrap();
            assert!(!collector.progress().done);
            let mut shards: Vec<Tracer> = (0..count).map(|index| Tracer::shard(&scene, options, index, count).unwrap()).collect();
            // Partial sums first (as previews arrive), then the finished ones replace them.
            for shard in &mut shards {
                shard.step_rows(5);
                collector.merge(&shard.sums()).unwrap();
            }
            assert!(!collector.progress().done);
            for shard in &mut shards {
                assert!(shard.finish().done);
                let progress = collector.merge(&shard.sums()).unwrap();
                assert!(progress.samples <= 5);
            }
            let progress = collector.progress();
            assert!(progress.done && progress.samples == 5 && (progress.fraction - 1.0).abs() < 1e-6);
            assert_eq!(collector.image(), whole.image(), "{count} shards, denoise {denoise}");
        }
    }
    // Sums of another render, or damaged, are refused.
    let options = TraceOptions { samples: 5, ..Default::default() };
    let mut collector = Tracer::collector(&scene, options).unwrap();
    let mut shard = Tracer::shard(&scene, options, 0, 2).unwrap();
    shard.step_rows(3);
    let sums = shard.sums();
    for end in 0..sums.len().min(200) {
        assert!(collector.merge(&sums[..end]).is_err());
    }
    let other = Tracer::shard(&starter(40, 27), options, 0, 2).unwrap();
    assert!(collector.merge(&other.sums()).is_err());
    assert!(Tracer::shard(&scene, options, 2, 2).is_err());
}

#[test]
fn the_starter_cube_is_lit_and_the_world_is_dim() {
    let scene = starter(96, 54);
    let mut tracer = Tracer::new(&scene, TraceOptions { samples: 32, ..Default::default() }).unwrap();
    let progress = tracer.finish();
    assert!(progress.done && progress.samples == 32);
    let image = tracer.image();
    let at = |x: usize, y: usize| image[(y * 96 + x) * 4];
    assert!(at(48, 27) > 100, "the cube's lit faces");
    assert!(at(2, 2) < 80, "the dark world");
}

#[test]
fn an_area_light_and_a_sun_both_light_a_floor() {
    for kind in [LightKind::Area, LightKind::Sun, LightKind::Spot] {
        let mut scene = Scene::empty();
        let floor = scene.add_object("Floor", ObjectData::Mesh(Primitive::Plane { size: 20.0 }.build().unwrap()));
        let _ = floor;
        let mut light = Light::new(kind);
        light.radius = 1.0;
        let id = scene.add_object("Light", ObjectData::Light(light));
        scene.object_mut(id).unwrap().location = Vec3::new(0.0, 0.0, 2.0);
        let camera = scene.add_object("Camera", ObjectData::Camera(Default::default()));
        scene.object_mut(camera).unwrap().location = Vec3::new(0.0, 0.0, 5.0);
        scene.camera = Some(camera);
        scene.world.strength = 0.0;
        scene.render.width = 16;
        scene.render.height = 16;
        let mut tracer = Tracer::new(&scene, TraceOptions { samples: 16, ..Default::default() }).unwrap();
        tracer.finish();
        let centre = tracer.linear()[8 * 16 + 8];
        assert!(centre.x > 0.01, "{kind:?}: {centre}");
    }
}

#[test]
fn a_scene_without_a_camera_is_refused() {
    let mut scene = Scene::starter();
    scene.camera = None;
    assert!(Tracer::new(&scene, TraceOptions::default()).is_err());
}
