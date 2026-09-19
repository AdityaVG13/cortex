#![forbid(unsafe_code)]

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
    // Only accept a binary in a Cargo profile dir: the per-package layout
    // (1.100+) keeps no top-level `deps`/`.fingerprint`, so the marker is a
    // `build/` dir plus the binary itself. Walking into `$HOME` would pick up
    // an unrelated `cortex` and hide daemon defects.
    exe.ancestors()
        .skip(1)
        .find(|dir| dir.join("build").is_dir() && dir.join(name).is_file())
        .map(|dir| dir.join(name))
        .unwrap_or_else(|| {
            panic!(
                "cortex binary not found next to the Cargo profile of {}; run \
                 `cargo build -p cortex-daemon --bin cortex` first",
                exe.display()
            )
        })
}

pub use env::{in_subprocess, lock, lock_async};
pub use support::{runtime_state, solo_state, team_state, test_conn};
