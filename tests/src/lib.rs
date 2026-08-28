//! Shared suite library for Cortex daemon tests.
//!
//! Helpers live here so `crates/cortex-daemon` ships no `#[cfg(test)]` code.
//! Integration contracts live in `tests/tests/<domain>.rs`.
#![deny(unsafe_code)]

pub mod env;
pub mod support;

/// Resolve the path to the `cortex` daemon binary at runtime.
///
/// `cargo test -p cortex-tests` does not rebuild this binary. Build it first
/// with `cargo build -p cortex-daemon --bin cortex` (npm test does that).
/// Integration tests run from `target/<profile>/deps/<test>-<hash>`, so the
/// binary lives one level up at `target/<profile>/cortex`.
pub fn cortex_bin() -> std::path::PathBuf {
    let exe = std::env::current_exe().expect("cortex-tests: current exe");
    let bin_dir = exe
        .parent()
        .and_then(|p| p.parent())
        .expect("cortex-tests: bin dir");
    let name = if cfg!(target_os = "windows") {
        "cortex.exe"
    } else {
        "cortex"
    };
    let path = bin_dir.join(name);
    assert!(
        path.is_file(),
        "cortex binary missing at {}; run `cargo build -p cortex-daemon --bin cortex` first",
        path.display()
    );
    path
}

pub use env::{lock, lock_async, ScopedEnvVar};
pub use support::{runtime_state, solo_state, team_state, test_conn};
