//! `kition-scene`: the Scene kit for Kition's agent. The same document the in-page editor runs
//! (`kition-scene-kit`), served headless over the Kit Protocol by `kition-headless`, as the kit's
//! `kit.json` declares it, with the object model (`model.describe`, `model.batch`) and the replica
//! methods over the document's change exchange (engine contract 2).

/// `inspect()` keys about the editing session rather than the file.
const VOLATILE: &[&str] = kition_scene_kit::EDITOR_STATE;

fn main() -> std::process::ExitCode {
    kition_headless::run_manifest_model::<kition_scene_kit::SceneDocument>(include_str!("../../../kit.json"), kition_scene_kit::SANDBOXED, VOLATILE)
}
