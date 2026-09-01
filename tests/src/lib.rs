#![deny(unsafe_code)]

pub mod env;
pub mod support;

/// Resolves the daemon binary beside the active Cargo profile directory.
/// Build `cortex-daemon` before running integration contracts.
///
/// Cargo 1.100+ places test executables under the package build dir
/// (`build/<pkg>/<hash>/deps/`) instead of the profile `deps/`, so the
/// profile directory is an ancestor at an unknown depth: walk up until the
/// binary turns up rather than assuming a fixed parent count.
pub fn cortex_bin() -> std::path::PathBuf {
    let name = if cfg!(target_os = "windows") {
        "cortex.exe"
    } else {
        "cortex"
    };
    let exe = std::env::current_exe().expect("cortex-tests: current exe");
    exe.ancestors()
        .skip(1)
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
        .unwrap_or_else(|| {
            panic!(
                "cortex binary not found in any ancestor of {}; run \
                 `cargo build -p cortex-daemon --bin cortex` first",
                exe.display()
            )
        })
}

pub use env::{lock, lock_async, ScopedEnvVar};
pub use support::{runtime_state, solo_state, team_state, test_conn};
