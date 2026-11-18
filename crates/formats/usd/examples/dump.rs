//! Prints a USD file as the reader sees it: `cargo run -p kition-scene-usd --example dump -- <file>`.
#![allow(clippy::unwrap_used)]

fn main() {
    let path = std::env::args().nth(1).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let layer = if bytes.starts_with(kition_scene_usd::usdc::MAGIC) {
        kition_scene_usd::usdc::parse(&bytes).unwrap()
    } else {
        kition_scene_usd::usda::parse(std::str::from_utf8(&bytes).unwrap()).unwrap()
    };
    println!("{layer:#?}");
}
