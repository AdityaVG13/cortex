//! Shared suite library for the extracted Cortex daemon tests.
//!
//! This crate re-homes the test-only helpers that previously lived in
//! `cortex-daemon/src/test_support.rs` and `cortex-daemon/src/test_env.rs`,
//! promoting them to a standalone test-support crate so the daemon itself
//! ships no `#[cfg(test)]` code (Phase B of the test extraction).
#![deny(unsafe_code)]

pub mod env;
pub mod support;

/// Resolve the path to the `cortex` daemon binary at runtime.
///
/// The binary is built by the `cortex-daemon` crate; integration tests run
/// from `target/<profile>/deps/<test>-<hash>`, so the binary lives one level
/// up at `target/<profile>/cortex`.
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
    bin_dir.join(name)
}

pub use env::{lock, lock_async, ScopedEnvVar};
pub use support::{runtime_state, solo_state, team_state, test_conn};
