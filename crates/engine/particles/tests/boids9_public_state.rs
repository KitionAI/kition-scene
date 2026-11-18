//! Public Blender motion state does not include the random state used for some turns.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use serde_json::Value;

#[test]
fn recorded_seed_dependent_turns_begin_from_identical_public_motion_state() {
    let simple: Value = serde_json::from_str(include_str!("fixtures/boids9-public-state.json")).unwrap();
    assert_eq!(simple["seed0"][1], simple["seed1MatchingPublicState"][1]);
    assert_ne!(simple["seed0"][2], simple["seed1MatchingPublicState"][2]);
    assert_ne!(simple["seed0"][2], simple["velocityOverrideControl"][2]);
    let mixed: Value = serde_json::from_str(include_str!("fixtures/boids9-mixed-public-state.json")).unwrap();
    assert_eq!(mixed["observations"].as_array().unwrap().len(), 2);
    for observation in mixed["observations"].as_array().unwrap() {
        assert_eq!(observation["seed0"]["samples"][0], observation["seed1MatchingPublicState"]["samples"][0]);
        assert_ne!(observation["seed0"]["samples"][1], observation["seed1MatchingPublicState"]["samples"][1]);
    }
}

#[test]
fn the_optional_blender_application_rechecks_public_state_turn_probes() {
    let Some(blender) = std::env::var_os("KITION_BLENDER") else { return };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for (script, fixture) in [("boids9_state.py", "boids9-public-state.json"), ("boids9_mixed_state.py", "boids9-mixed-public-state.json")] {
        let output = std::env::temp_dir().join(format!("scene-{}-{fixture}", std::process::id()));
        let result = std::process::Command::new(&blender)
            .args(["--background", "--factory-startup", "--python"])
            .arg(root.join("tests/blender").join(script))
            .arg("--")
            .arg(&output)
            .output()
            .unwrap();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        let live: Value = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        let recorded: Value = serde_json::from_slice(&std::fs::read(root.join("tests/fixtures").join(fixture)).unwrap()).unwrap();
        assert_eq!(live, recorded);
        std::fs::remove_file(output).unwrap();
    }
}
