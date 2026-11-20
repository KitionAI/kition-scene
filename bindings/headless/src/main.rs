//! `kition-scene`: the Scene kit for Kition's agent. The same document the in-page editor runs
//! (`kition-scene-kit`), served headless over the Kit Protocol by `kition-headless`, as the kit's
//! `kit.json` declares it, with the object model (`model.describe`, `model.batch`) and the replica
//! methods over the document's change exchange (engine contract 2), and the identity a file
//! carries (`document.identity`, `document.fork`: a `.kscene` embeds its replica,
//! `kition_scene::SceneFiles`).

use std::sync::Arc;

use kition_headless::{FileIdentity, Headless, KitInfo};

/// `inspect()` keys about the editing session rather than the file.
const VOLATILE: &[&str] = kition_scene_kit::EDITOR_STATE;

fn main() -> std::process::ExitCode {
    let kit = match KitInfo::from_manifest(include_str!("../../../kit.json"), kition_scene_kit::SANDBOXED, VOLATILE) {
        Ok(kit) => kit,
        Err(error) => {
            eprintln!("kit: {error}");
            return std::process::ExitCode::from(2);
        }
    };
    let backend = Headless::<kition_scene_kit::SceneDocument>::with_replica_port(kit.clone())
        .with_file_identity(FileIdentity::Embedded(Arc::new(kition_scene::SceneFiles)));
    kition_headless::serve_backend(&kit, backend)
}
