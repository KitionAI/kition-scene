//! Reads a USD file and prints its objects: `cargo run -p kition-scene-usd --example summary -- <file> [out.usda]`.
#![allow(clippy::unwrap_used, clippy::print_stdout)]

fn main() {
    let input = std::env::args().nth(1).unwrap();
    let imported = kition_scene_usd::read(&std::fs::read(input).unwrap()).unwrap();
    let channels = imported.animation.len();
    let mut scene = kition_scene_model::Scene::empty();
    kition_scene_exchange::place(&mut scene, imported).unwrap();
    for object in &scene.objects {
        let world = scene.world_matrix(object.id).unwrap();
        println!(
            "{} {} parent={:?} at {:?}",
            object.name,
            object.data.kind(),
            object.parent.and_then(|id| scene.object(id).ok()).map(|parent| parent.name.clone()),
            world.w_axis.truncate()
        );
    }
    println!("{channels} channels");
    if let Some(output) = std::env::args().nth(2) {
        std::fs::write(output, kition_scene_usd::write(&scene, None).unwrap()).unwrap();
    }
}
