//! Reads an FBX file and writes it back: `cargo run -p kition-scene-fbx --example roundtrip -- <in> <out>`.
#![allow(clippy::unwrap_used)]

fn main() {
    let input = std::env::args().nth(1).unwrap();
    let output = std::env::args().nth(2).unwrap();
    let imported = kition_scene_fbx::read(&std::fs::read(input).unwrap()).unwrap();
    let mut scene = kition_scene_model::Scene::empty();
    kition_scene_exchange::place(&mut scene, imported).unwrap();
    std::fs::write(output, kition_scene_fbx::write(&scene, None).unwrap()).unwrap();
}
