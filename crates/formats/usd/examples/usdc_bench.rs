//! Reproducible writer workload; run with `cargo run --release -p kition-scene-usd --example usdc_bench`.
//! Reports seven timed batches after warmup. Layer construction and file I/O are not timed.

use std::{error::Error, hint::black_box, time::Instant};

use kition_scene_model::Scene;
use kition_scene_usd::{usda, usdc};

fn measure(name: &str, iterations: usize, mut write: impl FnMut() -> Result<Vec<u8>, Box<dyn Error>>) -> Result<(), Box<dyn Error>> {
    let bytes = write()?.len();
    for _ in 0..3 {
        black_box(write()?);
    }
    for batch in 0..7 {
        let start = Instant::now();
        for _ in 0..iterations {
            black_box(write()?);
        }
        println!("{name},{bytes},{batch},{iterations},{:.6}", start.elapsed().as_secs_f64() * 1000.0 / iterations as f64);
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let scene = Scene::starter();
    let template = usda::parse(
        r#"#usda 1.0
def Xform "Object" {
    double3 xformOp:translate.timeSamples = {1: (0, 0, 0), 11: (1, 2, 3), 21: (3, 2, 1)}
    uniform token[] xformOpOrder = ["xformOp:translate"]
    custom string label = "Repeated animated object"
    rel material:binding = </Material>
    def Mesh "Mesh" {
        point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0, 1, 2]
        uniform token subdivisionScheme = "none"
    }
}
"#,
    )?;
    let object = template.prims.first().ok_or("missing benchmark prim")?;
    let mut layer = usda::parse("#usda 1.0\ndef Material \"Material\" {}")?;
    for index in 0..2_000 {
        let mut prim = object.clone();
        prim.name = format!("Object_{index:04}");
        layer.prims.push(prim);
    }
    println!("workload,bytes,batch,iterations,ms_per_export");
    measure("starter_scene", 100, || Ok(kition_scene_usd::write_binary(&scene, None)?))?;
    measure("animated_layer_2000", 3, || Ok(usdc::write(&layer)?))?;
    Ok(())
}
