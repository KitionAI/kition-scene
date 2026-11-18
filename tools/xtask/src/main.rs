//! Workspace tooling: `cargo xtask <command>`.
//!
//! Pure Rust (std + serde_json). External tools (`cargo`, `node`) are invoked through
//! `std::process::Command`.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci        fmt --check, clippy -D warnings, test, wasm (stops at the first failure)
  wasm      cargo check --target wasm32-unknown-unknown for the engine crates
  manifest  kition-sdk's tools/check-manifest.mjs on kit.json (needs node and a kition-sdk
            checkout: KITION_SDK_DIR, or ../kition-sdk beside this repository)
  eval      the layer: eval cases (kition-sdk's tools/eval-cases.mjs; real models). Only a
            maintainer starts it (docs/quality.md, F-QA-16); agents never do. `eval --dry-run`
            calls no model
";

/// The crates the page's WebAssembly engine is built from.
const WASM_CRATES: &[&str] = &[
    "kition-scene-model",
    "kition-scene-physics",
    "kition-scene-particles",
    "kition-scene-nodes",
    "kition-scene-raster",
    "kition-scene-kscene",
    "kition-scene-kdm",
    "kition-scene-exchange",
    "kition-scene-blend",
    "kition-scene-fbx",
    "kition-scene-usd",
    "kition-scene-trace",
    "kition-scene-kit",
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("ci") => ci(),
        Some("wasm") => wasm(),
        Some("manifest") => manifest(),
        Some("eval") => eval(&args[1..]),
        Some("-h" | "--help" | "help") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The workspace root (two levels above the xtask crate in `tools/xtask`).
fn root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.ancestors().nth(2).map_or_else(|| manifest_dir.to_path_buf(), Path::to_path_buf)
}

fn cargo() -> Command {
    let mut command = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    command.current_dir(root());
    command
}

fn run(mut command: Command, what: &str) -> Result<(), String> {
    eprintln!("$ {what}");
    let status = command.status().map_err(|error| format!("{what}: failed to spawn: {error}"))?;
    if status.success() { Ok(()) } else { Err(format!("{what}: exited with {status}")) }
}

fn cargo_step(args: &[&str]) -> Result<(), String> {
    let mut command = cargo();
    command.args(args);
    run(command, &format!("cargo {}", args.join(" ")))
}

fn wasm() -> Result<(), String> {
    let mut args = vec!["check", "--target", "wasm32-unknown-unknown"];
    for name in WASM_CRATES {
        args.extend(["-p", name]);
    }
    cargo_step(&args)
}

fn manifest() -> Result<(), String> {
    let sdk = std::env::var_os("KITION_SDK_DIR").map(PathBuf::from).unwrap_or_else(|| root().join("..").join("kition-sdk"));
    let tool = sdk.join("tools").join("check-manifest.mjs");
    if !tool.is_file() {
        return Err(format!("no {} (set KITION_SDK_DIR to a kition-sdk checkout)", tool.display()));
    }
    let mut command = Command::new("node");
    command.current_dir(root()).arg(tool).arg("kit.json");
    run(command, "node <kition-sdk>/tools/check-manifest.mjs kit.json")
}

fn ci() -> Result<(), String> {
    type Step = (&'static str, fn() -> Result<(), String>);
    let steps: [Step; 4] = [
        ("fmt", || cargo_step(&["fmt", "--all", "--", "--check"])),
        ("clippy", || cargo_step(&["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])),
        ("test", || cargo_step(&["test", "--workspace"])),
        ("wasm", wasm),
    ];
    let mut done = Vec::new();
    for (name, step) in steps {
        eprintln!("\n=== ci: {name} ===");
        if let Err(error) = step() {
            println!("\nCI summary:");
            for passed in &done {
                println!("  ok    {passed}");
            }
            println!("  FAIL  {name}: {error}");
            return Err(format!("ci failed at `{name}`"));
        }
        done.push(name);
    }
    println!("\nCI summary: all {} steps passed ({})", done.len(), done.join(", "));
    Ok(())
}

/// The `layer: eval` cases through kition-sdk's runner (`tools/eval-cases.mjs`); its guard asks
/// the person at the terminal before any model call.
fn eval(args: &[String]) -> Result<(), String> {
    let root = root();
    let sdk = std::env::var_os("KITION_SDK_DIR").map(PathBuf::from).unwrap_or_else(|| root.join("..").join("kition-sdk"));
    let status = Command::new("node")
        .arg(sdk.join("tools").join("eval-cases.mjs"))
        .arg("--repo")
        .arg(&root)
        .args(args)
        .status()
        .map_err(|e| format!("could not start node: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("eval cases: {status}")) }
}
