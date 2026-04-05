use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
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

  if dest.exists() {
    return;
  }

  let home = env::var_os("USERPROFILE")
    .or_else(|| env::var_os("HOME"))
    .map(PathBuf::from);

  if let Some(home) = home {
    let src = home
      .join("cortex")
      .join("daemon-rs")
      .join("target")
      .join("release")
      .join(format!("cortex{ext}"));
    if src.exists() {
      let _ = fs::copy(&src, &dest);
      println!(
        "cargo:warning=Copied sidecar binary from {}",
        src.display()
      );
    } else {
      println!(
        "cargo:warning=Cortex daemon binary not found at {}. Build daemon-rs first.",
        src.display()
      );
    }
  }
}
