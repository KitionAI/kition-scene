//! Writes the starter scene a new file gets: `cargo run -p kition-scene-kit --example starter -- <path>`.

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;

fn main() -> Result<(), String> {
    let path = std::env::args().nth(1).ok_or("usage: starter <path.kscene>")?;
    let mut doc = SceneDocument::create("Untitled scene.kscene", 1280, 720)?;
    std::fs::write(&path, doc.save_file(None)?).map_err(|error| error.to_string())
}
