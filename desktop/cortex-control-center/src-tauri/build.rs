use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

const DEV_DAEMON_TARGET_DIR: &str = "target-control-center-dev";
const RELEASE_DAEMON_TARGET_DIR: &str = "target-control-center-release";

fn main() {
    println!("cargo:rerun-if-env-changed=CORTEX_SIDECAR_BIN");
    copy_sidecar_binary();
    tauri_build::build()
}

fn copy_sidecar_binary() {
    let target_triple = env::var("TARGET").unwrap_or_default();
    if target_triple.is_empty() {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let binaries_dir = manifest_dir.join("binaries");
    let _ = fs::create_dir_all(&binaries_dir);

    let ext = if target_triple.contains("windows") {
        ".exe"
    } else {
        ""
    };
    let dest = binaries_dir.join(format!("cortex-{target_triple}{ext}"));

    let profile = env::var("PROFILE").unwrap_or_default();

    // Always copy the latest binary (daemon is built by the desktop npm scripts).
    let mut candidates = Vec::new();
    if let Some(sidecar_override) = env::var_os("CORTEX_SIDECAR_BIN") {
        candidates.push(PathBuf::from(sidecar_override));
    }

    if let Some(repo_root) = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
    {
        let daemon_root = repo_root.join("daemon-rs");
        if profile != "release" {
            candidates.push(
                daemon_root
                    .join(DEV_DAEMON_TARGET_DIR)
                    .join("debug")
                    .join(format!("cortex{ext}")),
            );
            candidates.push(
                daemon_root
                    .join("target")
                    .join("debug")
                    .join(format!("cortex{ext}")),
            );
            candidates.push(
                daemon_root
                    .join(RELEASE_DAEMON_TARGET_DIR)
                    .join("release")
                    .join(format!("cortex{ext}")),
            );
        } else {
            candidates.push(
                daemon_root
                    .join(RELEASE_DAEMON_TARGET_DIR)
                    .join("release")
                    .join(format!("cortex{ext}")),
            );
        }
        candidates.push(
            daemon_root
                .join("target")
                .join("release")
                .join(format!("cortex{ext}")),
        );
    }

    let home = env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from);

    if let Some(home) = home {
        candidates.push(
            home.join(".cortex")
                .join("bin")
                .join(format!("cortex{ext}")),
        );
        candidates.push(
            home.join("cortex")
                .join("daemon-rs")
                .join("target")
                .join("release")
                .join(format!("cortex{ext}")),
        );
    }

    for src in candidates {
        if src.exists() {
            if let Err(err) = copy_if_changed(&src, &dest) {
                println!(
                    "cargo:warning=Failed to copy Cortex sidecar from {} to {}: {}",
                    src.display(),
                    dest.display(),
                    err
                );
            }
            return;
        }
    }

    println!(
    "cargo:warning=Cortex sidecar binary not found. Expected one of: CORTEX_SIDECAR_BIN, <repo>/daemon-rs/{DEV_DAEMON_TARGET_DIR}/debug/cortex{ext}, <repo>/daemon-rs/{RELEASE_DAEMON_TARGET_DIR}/release/cortex{ext}, <repo>/daemon-rs/target/release/cortex{ext}, ~/.cortex/bin/cortex{ext}, ~/cortex/daemon-rs/target/release/cortex{ext}"
  );
}

fn copy_if_changed(src: &PathBuf, dest: &PathBuf) -> io::Result<()> {
    let needs_copy = match fs::read(dest) {
        Ok(existing) => existing != fs::read(src)?,
        Err(err) if err.kind() == io::ErrorKind::NotFound => true,
        Err(err) => return Err(err),
    };

    if needs_copy {
        fs::copy(src, dest)?;
    }

    Ok(())
}
