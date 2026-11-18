//! Regression: rendering to the end (`finish`, what
//! `view.renderedStep {wait: true}` and final renders run) uses every core the machine has, and
//! gives exactly the picture stepping row by row gives.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::expect_used)]

use std::time::Instant;

use kition_scene_model::Scene;
use kition_scene_trace::{TraceOptions, Tracer};

fn tracer(samples: u32) -> Tracer {
    let mut scene = Scene::starter();
    scene.render.width = 320;
    scene.render.height = 180;
    Tracer::new(&scene, TraceOptions { samples, ..TraceOptions::default() }).unwrap()
}

#[test]
fn finishing_is_parallel_and_the_same_as_stepping() {
    let mut stepped = tracer(8);
    let started = Instant::now();
    while !stepped.step_rows(1).done {}
    let sequential = started.elapsed();
    let mut finished = tracer(8);
    let started = Instant::now();
    assert!(finished.finish().done);
    let parallel = started.elapsed();
    assert_eq!(finished.linear(), stepped.linear(), "the same picture, bit for bit");
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    if cores >= 4 {
        assert!(parallel.as_secs_f64() < sequential.as_secs_f64() * 0.6, "{cores} cores: finishing took {parallel:?}, stepping {sequential:?}");
    }
}
