//! Times the starter scene at 480 × 270: `cargo run --release -p kition-scene-trace --example speed`.

use kition_scene_model::Scene;
use kition_scene_trace::{TraceOptions, Tracer};

fn main() -> Result<(), String> {
    let mut scene = Scene::starter();
    scene.render.width = 480;
    scene.render.height = 270;
    let samples = 32;
    let start = std::time::Instant::now();
    let mut tracer = Tracer::new(&scene, TraceOptions { samples, ..Default::default() })?;
    let built = start.elapsed().as_secs_f64();
    tracer.finish();
    let seconds = start.elapsed().as_secs_f64() - built;
    let paths = f64::from(samples) * 480.0 * 270.0;
    println!(
        "setup {:.1} ms; {samples} samples in {seconds:.2} s: {:.1} samples per pixel per second, {:.2} M paths/s",
        built * 1000.0,
        f64::from(samples) / seconds,
        paths / seconds / 1e6
    );
    Ok(())
}
