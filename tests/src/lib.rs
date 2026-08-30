#![deny(unsafe_code)]

pub mod env;
pub mod support;

/// Resolves the daemon binary beside the active Cargo profile directory.
/// Build `cortex-daemon` before running integration contracts.
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
