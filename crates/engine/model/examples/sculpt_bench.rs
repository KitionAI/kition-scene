//! Times sculpt dabs on a 200k-vertex grid: `cargo run --release -p kition-scene-model --example sculpt_bench`.
#![allow(clippy::unwrap_used)]

use kition_scene_model::Primitive;
use kition_scene_model::glam::Vec3;
use kition_scene_model::sculpt::{Brush, BrushKind, Dab, Stroke};

fn main() {
    let fresh = Primitive::Grid { x_subdivisions: 446, y_subdivisions: 446, size: 2.0 }.build().unwrap();
    let mut mesh = fresh.clone();
    println!("{} vertices, {} faces", mesh.positions.len(), mesh.faces.len());
    let start = std::time::Instant::now();
    let _grid = kition_scene_model::sculpt::Grid::build(&mesh.positions, 0.1);
    println!("grid alone: {:.1} ms", start.elapsed().as_secs_f64() * 1000.0);
    for (kind, radius) in
        [(BrushKind::Draw, 0.1), (BrushKind::Draw, 0.3), (BrushKind::Smooth, 0.1), (BrushKind::ClayStrips, 0.1), (BrushKind::Grab, 0.3)]
    {
        mesh = fresh.clone();
        let start = std::time::Instant::now();
        let mut stroke = Stroke::new(&mesh, Brush { symmetry: [true, false, false], ..Brush::of(kind) }, radius);
        let setup = start.elapsed();
        let dabs = 100;
        let start = std::time::Instant::now();
        for step in 0..dabs {
            let center = Vec3::new(-0.5 + step as f32 * 0.01, 0.2, 0.0);
            stroke.dab(&mut mesh, &Dab { center, radius, strength: 0.5, toward_eye: Vec3::Z, drag: Vec3::new(0.0, 0.0, step as f32 * 0.001) });
        }
        println!(
            "{kind:?} r={radius}: stroke setup {:.1} ms, {:.3} ms per dab, {} vertices touched",
            setup.as_secs_f64() * 1000.0,
            start.elapsed().as_secs_f64() * 1000.0 / dabs as f64,
            stroke.touched.len()
        );
    }
}
