//! The Scene kit against the convergence suite (L6): two to five replicas of a scene, edited
//! through the engine's own commands.

mod common;

use std::path::Path;

use common::{NAME, SceneKit};
use kition_convergence::wasm::{self, WasmEngine};
use kition_convergence::{Options, Outcome};

#[test]
fn converges() {
    let report = kition_convergence::run(&SceneKit, &Options::from_env());
    println!("{}", report.render());
    report.save_if_requested().unwrap();
    assert!(report.level6(), "{}", report.render());
    // A scene has no text the suite can type into.
    assert!(matches!(report.outcome("text-position"), Some(Outcome::Skipped(_))));
}

/// Native and WebAssembly engines reach one state from one change log. Set `KITION_WASM_ENGINE`
/// to a folder with `kition_scene_kit.js` and `kition_scene_kit_bg.wasm`.
#[test]
fn wasm_parity() {
    let Some(engine) = WasmEngine::from_env("kition_scene_kit", "SceneDocument", NAME) else {
        println!("SKIP wasm_parity: KITION_WASM_ENGINE is not set");
        return;
    };
    wasm::check_parity(&SceneKit, &engine, Path::new(env!("CARGO_TARGET_TMPDIR")), 1).unwrap();
}
