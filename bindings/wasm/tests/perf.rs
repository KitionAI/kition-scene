//! The convergence suite's performance mode for the Scene kit (Kition `docs/kits.md` section
//! 7): `cargo test --release -p kition-scene-kit --test perf -- --ignored --nocapture`.

mod common;

use common::SceneKit;
use kition_convergence::CountingAlloc;
use kition_convergence::perf::{self, PerfOptions};

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc::new();

#[test]
#[ignore = "a measurement, run in release builds"]
fn budgets() {
    let report = perf::measure(&SceneKit, &PerfOptions { allocator: Some(&ALLOC), ..PerfOptions::default() }).unwrap();
    println!("{}", report.render());
}
