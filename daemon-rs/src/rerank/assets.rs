// SPDX-License-Identifier: MIT
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug)]
pub(crate) struct RerankerAsset {
    pub(crate) file: &'static str,
    pub(crate) url: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct RerankerSelection {
    pub key: &'static str,
    pub display_name: &'static str,
    pub model_size_mb: u64,
    pub max_input_tokens: usize,
    pub model_file: &'static str,
    pub tokenizer_file: &'static str,
}

pub(crate) struct RerankerProfile {
    pub(crate) key: &'static str,
    pub(crate) display_name: &'static str,
    pub(crate) model_size_mb: u64,
    pub(crate) max_input_tokens: usize,
    pub(crate) model_file: &'static str,
    pub(crate) tokenizer_file: &'static str,
    pub(crate) assets: &'static [RerankerAsset],
}

impl RerankerProfile {
    fn selection(&self) -> RerankerSelection {
        RerankerSelection {
            key: self.key,
            display_name: self.display_name,
            model_size_mb: self.model_size_mb,
            max_input_tokens: self.max_input_tokens,
            model_file: self.model_file,
            tokenizer_file: self.tokenizer_file,
        }
    }

    fn assets_exist(&self, models_dir: &Path) -> bool {
        self.missing_assets(models_dir).is_empty()
    }

    pub(crate) fn missing_assets(&self, models_dir: &Path) -> Vec<RerankerAsset> {
        self.assets
            .iter()
            .copied()
            .filter(|asset| !models_dir.join(asset.file).exists())
            .collect()
    }
}

const MINILM_RERANKER_ASSETS: &[RerankerAsset] = &[
    RerankerAsset {
        file: "rerank/ms-marco-MiniLM-L-6-v2/model_int8.onnx",
        url: "https://huggingface.co/Xenova/ms-marco-MiniLM-L-6-v2/resolve/main/onnx/model_int8.onnx",
    },
    RerankerAsset {
        file: "rerank/ms-marco-MiniLM-L-6-v2/tokenizer.json",
        url: "https://huggingface.co/Xenova/ms-marco-MiniLM-L-6-v2/resolve/main/tokenizer.json",
    },
    RerankerAsset {
        file: "rerank/ms-marco-MiniLM-L-6-v2/config.json",
        url: "https://huggingface.co/Xenova/ms-marco-MiniLM-L-6-v2/resolve/main/config.json",
    },
    RerankerAsset {
        file: "rerank/ms-marco-MiniLM-L-6-v2/tokenizer_config.json",
        url: "https://huggingface.co/Xenova/ms-marco-MiniLM-L-6-v2/resolve/main/tokenizer_config.json",
    },
    RerankerAsset {
        file: "rerank/ms-marco-MiniLM-L-6-v2/special_tokens_map.json",
        url: "https://huggingface.co/Xenova/ms-marco-MiniLM-L-6-v2/resolve/main/special_tokens_map.json",
    },
];

const MINILM_RERANKER: RerankerProfile = RerankerProfile {
    key: "ms-marco-MiniLM-L-6-v2",
    display_name: "ms-marco-MiniLM-L-6-v2 int8",
    model_size_mb: 23,
    max_input_tokens: 512,
    model_file: "rerank/ms-marco-MiniLM-L-6-v2/model_int8.onnx",
    tokenizer_file: "rerank/ms-marco-MiniLM-L-6-v2/tokenizer.json",
    assets: MINILM_RERANKER_ASSETS,
};

pub(crate) fn selected_profile() -> &'static RerankerProfile {
    &MINILM_RERANKER
}

pub fn selected_reranker_selection() -> RerankerSelection {
    selected_profile().selection()
}

pub fn selected_reranker_assets_exist(models_dir: &Path) -> bool {
    selected_profile().assets_exist(models_dir)
}

pub async fn ensure_reranker_downloaded() -> Option<PathBuf> {
    let models_dir = dirs::home_dir()?.join(".cortex").join("models");
    ensure_reranker_downloaded_in(&models_dir).await
}

pub async fn ensure_reranker_downloaded_in(models_dir: &Path) -> Option<PathBuf> {
    let profile = selected_profile();
    std::fs::create_dir_all(models_dir).ok()?;
    if profile.assets_exist(models_dir) {
        return Some(models_dir.to_path_buf());
    }

    eprintln!(
        "[rerank] Downloading reranker '{}' (first run)...",
        profile.display_name
    );
    for asset in profile.missing_assets(models_dir) {
        let asset_path = models_dir.join(asset.file);
        match download_file(asset.url, &asset_path).await {
            Ok(()) => eprintln!("[rerank] Asset downloaded: {}", asset_path.display()),
            Err(error) => {
                eprintln!("[rerank] Asset download failed for {}: {error}", asset.file);
                return None;
            }
        }
    }
    Some(models_dir.to_path_buf())
}

async fn download_file(url: &str, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|error| error.to_string())?;
    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let tmp_dest = dest.with_file_name(format!(
        "{}.tmp",
        dest.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("download")
    ));
    let mut file = std::fs::File::create(&tmp_dest).map_err(|error| error.to_string())?;
    while let Some(chunk) = resp.chunk().await.map_err(|error| error.to_string())? {
        file.write_all(&chunk).map_err(|error| error.to_string())?;
    }
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);
    std::fs::rename(&tmp_dest, dest).map_err(|error| error.to_string())?;
    Ok(())
}
