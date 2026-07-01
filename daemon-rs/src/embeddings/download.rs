// SPDX-License-Identifier: MIT
use std::io::Write;
use std::path::{Path, PathBuf};

use super::profiles::resolve_profile;

/// Return the models directory, downloading missing files from HuggingFace if
/// necessary.  Returns `None` on download failure (keyword-only search will be
/// used as a fallback).
pub async fn ensure_model_downloaded() -> Option<PathBuf> {
    let models_dir = dirs::home_dir()?.join(".cortex").join("models");
    ensure_model_downloaded_in(&models_dir).await
}

/// Ensure embedding assets exist in a specific models directory.
pub async fn ensure_model_downloaded_in(models_dir: &Path) -> Option<PathBuf> {
    let profile = resolve_profile();
    std::fs::create_dir_all(models_dir).ok()?;

    if profile.assets_exist(models_dir) {
        return Some(models_dir.to_path_buf());
    }

    eprintln!(
        "[embeddings] Downloading embedding model '{}' (first run)...",
        profile.display_name
    );

    for asset in profile.missing_assets(models_dir) {
        let asset_path = models_dir.join(asset.file);
        match download_file(asset.url, &asset_path).await {
            Ok(()) => eprintln!("[embeddings] Asset downloaded: {}", asset_path.display()),
            Err(e) => {
                eprintln!("[embeddings] Asset download failed for {}: {e}", asset.file);
                return None;
            }
        }
    }

    Some(models_dir.to_path_buf())
}

async fn download_file(url: &str, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| e.to_string())?;

    let mut resp = client.get(url).send().await.map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let tmp_dest = dest.with_file_name(format!(
        "{}.tmp",
        dest.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("download")
    ));
    let mut file = std::fs::File::create(&tmp_dest).map_err(|e| e.to_string())?;
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        file.write_all(&chunk).map_err(|e| e.to_string())?;
    }
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);

    std::fs::rename(&tmp_dest, dest).map_err(|e| e.to_string())?;

    Ok(())
}
