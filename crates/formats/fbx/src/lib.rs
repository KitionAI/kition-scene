//! FBX for Kition scenes: binary (7.x) and ASCII files read, binary 7400 written.
#![forbid(unsafe_code)]

mod export;
mod import;
pub mod node;

use kition_scene_exchange::Imported;
use kition_scene_model::SceneError;

/// Reads an FBX file (binary or ASCII).
pub fn read(bytes: &[u8]) -> Result<Imported, SceneError> {
    let (nodes, _) = node::parse(bytes)?;
    import::import(&nodes)
}

/// Writes the scene's visible objects (or `only` and their ancestors) as binary FBX 7400.
pub fn write(scene: &kition_scene_model::Scene, only: Option<&[kition_scene_model::Id]>) -> Result<Vec<u8>, SceneError> {
    export::export(scene, only)
}
