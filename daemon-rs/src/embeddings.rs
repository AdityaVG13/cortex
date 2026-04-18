// SPDX-License-Identifier: MIT
//! In-process ONNX embedding engine.
//!
//! Uses a selectable embedding profile (default: all-MiniLM-L6-v2, 23MB, 384-dim)
//! downloaded on first run.
//! Embeddings work the moment Cortex starts.

use ort::session::Session;
use ort::value::Tensor;
use std::path::{Path, PathBuf};
use tokenizers::Tokenizer;

const MAX_INPUT_TOKENS: usize = 256;

const MODEL_ENV_KEY: &str = "CORTEX_EMBEDDING_MODEL";
const POOL_ENV_KEY: &str = "CORTEX_EMBED_SESSION_POOL_SIZE";

struct EmbeddingModelProfile {
    key: &'static str,
    display_name: &'static str,
    dimension: usize,
    model_file: &'static str,
    tokenizer_file: &'static str,
    model_url: &'static str,
    tokenizer_url: &'static str,
}

const ALL_MINILM_L6_V2: EmbeddingModelProfile = EmbeddingModelProfile {
    key: "all-minilm-l6-v2",
    display_name: "all-MiniLM-L6-v2",
    dimension: 384,
    model_file: "all-MiniLM-L6-v2.onnx",
    tokenizer_file: "tokenizer.json",
    model_url:
        "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/onnx/model.onnx",
    tokenizer_url:
        "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/tokenizer.json",
};

const ALL_MINILM_L12_V2: EmbeddingModelProfile = EmbeddingModelProfile {
    key: "all-minilm-l12-v2",
    display_name: "all-MiniLM-L12-v2",
    dimension: 384,
    model_file: "all-MiniLM-L12-v2.onnx",
    tokenizer_file: "all-MiniLM-L12-v2-tokenizer.json",
    model_url:
        "https://huggingface.co/sentence-transformers/all-MiniLM-L12-v2/resolve/main/onnx/model.onnx",
    tokenizer_url:
        "https://huggingface.co/sentence-transformers/all-MiniLM-L12-v2/resolve/main/tokenizer.json",
};

#[derive(Clone, Copy, Debug)]
pub struct EmbeddingModelSelection {
    pub key: &'static str,
    pub display_name: &'static str,
    pub dimension: usize,
    pub model_file: &'static str,
    pub tokenizer_file: &'static str,
}

fn normalize_model_key(raw: &str) -> String {
    raw.trim().to_ascii_lowercase().replace('_', "-")
}

fn resolve_profile() -> &'static EmbeddingModelProfile {
    match std::env::var(MODEL_ENV_KEY) {
        Ok(raw) => match normalize_model_key(&raw).as_str() {
            "all-minilm-l6-v2" | "all-minilm-l6v2" | "minilm" => &ALL_MINILM_L6_V2,
            "all-minilm-l12-v2" | "all-minilm-l12v2" | "minilm-l12" | "minilm-modern" => {
                &ALL_MINILM_L12_V2
            }
            unknown => {
                eprintln!(
                    "[embeddings] Unknown {MODEL_ENV_KEY}='{unknown}', falling back to {}",
                    ALL_MINILM_L6_V2.key
                );
                &ALL_MINILM_L6_V2
            }
        },
        Err(_) => &ALL_MINILM_L6_V2,
    }
}

pub fn selected_model_selection() -> EmbeddingModelSelection {
    let profile = resolve_profile();
    EmbeddingModelSelection {
        key: profile.key,
        display_name: profile.display_name,
        dimension: profile.dimension,
        model_file: profile.model_file,
        tokenizer_file: profile.tokenizer_file,
    }
}

pub fn selected_model_key() -> &'static str {
    selected_model_selection().key
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

const DEFAULT_POOL_SIZE: usize = 2;
const MAX_POOL_SIZE: usize = 8;

fn resolved_pool_size() -> usize {
    match std::env::var(POOL_ENV_KEY) {
        Ok(raw) => match raw.trim().parse::<usize>() {
            Ok(parsed) => parsed.clamp(1, MAX_POOL_SIZE),
            Err(_) => {
                eprintln!(
                    "[embeddings] Invalid {POOL_ENV_KEY}='{}'; using default {}",
                    raw, DEFAULT_POOL_SIZE
                );
                DEFAULT_POOL_SIZE
            }
        },
        Err(_) => DEFAULT_POOL_SIZE,
    }
}

/// Shared embedding engine with a pool of ONNX sessions for concurrent access.
/// Each `embed()` call acquires an available session from the pool, runs
/// inference, then returns it. This prevents contention when multiple agents
/// or background tasks embed simultaneously.
pub struct EmbeddingEngine {
    sessions: Vec<std::sync::Mutex<Session>>,
    next: std::sync::atomic::AtomicUsize,
    tokenizer: Tokenizer,
    dimension: usize,
    model_key: &'static str,
}

impl EmbeddingEngine {
    /// Try to load from cached model files.  Returns `None` when files are
    /// missing or corrupt.  Opens `POOL_SIZE` independent ONNX sessions.
    pub fn load(models_dir: &Path) -> Option<Self> {
        match Self::try_load(models_dir) {
            Ok(engine) => Some(engine),
            Err(error) => {
                eprintln!("[embeddings] Engine load failed: {error}");
                None
            }
        }
    }

    fn try_load(models_dir: &Path) -> Result<Self, String> {
        let profile = resolve_profile();
        let pool_size = resolved_pool_size();
        let model_path = models_dir.join(profile.model_file);
        let tok_path = models_dir.join(profile.tokenizer_file);

        if !model_path.exists() || !tok_path.exists() {
            return Err(format!(
                "model assets missing (model_exists={}, tokenizer_exists={}) at {}",
                model_path.exists(),
                tok_path.exists(),
                models_dir.display()
            ));
        }

        let tokenizer = Tokenizer::from_file(&tok_path)
            .map_err(|error| format!("failed to load tokenizer {}: {error}", tok_path.display()))?;

        let mut sessions = Vec::with_capacity(pool_size);
        for index in 0..pool_size {
            let session = Self::build_session(&model_path)
                .map_err(|error| format!("session {} failed: {error}", index + 1))?;
            sessions.push(std::sync::Mutex::new(session));
        }

        eprintln!(
            "[embeddings] Session pool: {pool_size} sessions loaded for {}",
            profile.display_name,
        );
        Ok(Self {
            sessions,
            next: std::sync::atomic::AtomicUsize::new(0),
            tokenizer,
            dimension: profile.dimension,
            model_key: profile.key,
        })
    }

    fn build_session(model_path: &Path) -> Result<Session, String> {
        let tuned = Session::builder()
            .map_err(|error| format!("session builder init failed: {error}"))
            .and_then(|builder| {
                builder
                    .with_intra_threads(2)
                    .map_err(|error| format!("with_intra_threads(2) failed: {error}"))
            })
            .and_then(|mut builder| {
                builder.commit_from_file(model_path).map_err(|error| {
                    format!(
                        "commit_from_file (tuned threads) failed for {}: {error}",
                        model_path.display()
                    )
                })
            });

        match tuned {
            Ok(session) => Ok(session),
            Err(tuned_error) => {
                let fallback = Session::builder()
                    .map_err(|error| format!("session builder fallback init failed: {error}"))?
                    .commit_from_file(model_path)
                    .map_err(|error| {
                        format!(
                            "commit_from_file (fallback threads) failed for {}: {error}",
                            model_path.display()
                        )
                    })?;
                eprintln!(
                    "[embeddings] Falling back to default ORT session threading after tuned setup failed: {tuned_error}"
                );
                Ok(fallback)
            }
        }
    }

    fn truncate_to_char_boundary(text: &str, max_bytes: usize) -> &str {
        if text.len() <= max_bytes {
            return text;
        }
        let mut end = max_bytes;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        &text[..end]
    }

    /// Generate an embedding for `text` using the selected profile dimension.
    pub fn embed(&self, text: &str) -> Option<Vec<f32>> {
        let truncated = Self::truncate_to_char_boundary(text, 2000);

        let encoding = self.tokenizer.encode(truncated, true).ok()?;

        let ids = encoding.get_ids();
        let attention = encoding.get_attention_mask();
        let type_ids = encoding.get_type_ids();

        let len = ids.len().min(MAX_INPUT_TOKENS);
        let ids = &ids[..len];
        let attention = &attention[..len];
        let type_ids = &type_ids[..len];

        let shape = vec![1i64, len as i64];
        let ids_vec: Vec<i64> = ids.iter().map(|&x| x as i64).collect();
        let mask_vec: Vec<i64> = attention.iter().map(|&x| x as i64).collect();
        let type_vec: Vec<i64> = type_ids.iter().map(|&x| x as i64).collect();

        let ids_tensor = Tensor::from_array((shape.clone(), ids_vec)).ok()?;
        let mask_tensor = Tensor::from_array((shape.clone(), mask_vec)).ok()?;
        let type_tensor = Tensor::from_array((shape, type_vec)).ok()?;

        // Round-robin session selection across the configured session pool.
        let idx =
            self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % self.sessions.len();
        let mut session = self.sessions[idx].lock().ok()?;
        let outputs = session
            .run(ort::inputs![
                "input_ids" => ids_tensor,
                "attention_mask" => mask_tensor,
                "token_type_ids" => type_tensor,
            ])
            .ok()?;

        let (shape, data) = outputs[0].try_extract_tensor::<f32>().ok()?;
        let dims: Vec<i64> = shape.iter().copied().collect();

        if dims.len() != 3 || dims[2] as usize != self.dimension {
            eprintln!("[embeddings] Unexpected output shape: {dims:?}");
            return None;
        }

        let seq_len_out = dims[1] as usize;
        let mut pooled = vec![0.0f32; self.dimension];
        let mut mask_sum = 0.0f32;

        for seq_idx in 0..seq_len_out {
            let mask_val = attention[seq_idx.min(len - 1)] as f32;
            mask_sum += mask_val;
            let offset = seq_idx * self.dimension;
            for dim in 0..self.dimension {
                pooled[dim] += data[offset + dim] * mask_val;
            }
        }

        if mask_sum > 0.0 {
            for v in &mut pooled {
                *v /= mask_sum;
            }
        }

        let norm: f32 = pooled.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut pooled {
                *v /= norm;
            }
        }

        Some(pooled)
    }

    pub fn dimension(&self) -> usize {
        self.dimension
    }

    pub fn model_key(&self) -> &'static str {
        self.model_key
    }
}

// ---------------------------------------------------------------------------
// Vector utilities
// ---------------------------------------------------------------------------

/// Cosine similarity between two f32 slices (assumed L2-normalised, but this
/// implementation handles the general case too).
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;

    for i in 0..a.len() {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }

    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 {
        return 0.0;
    }

    (dot / denom).clamp(0.0, 1.0)
}

/// Encode a `Vec<f32>` as little-endian bytes for SQLite BLOB storage.
pub fn vector_to_blob(vec: &[f32]) -> Vec<u8> {
    vec.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Decode a SQLite BLOB (little-endian f32s) back to `Vec<f32>`.
pub fn blob_to_vector(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

// ---------------------------------------------------------------------------
// Model management
// ---------------------------------------------------------------------------

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

    let model_path = models_dir.join(profile.model_file);
    let tok_path = models_dir.join(profile.tokenizer_file);

    if model_path.exists() && tok_path.exists() {
        return Some(models_dir.to_path_buf());
    }

    eprintln!(
        "[embeddings] Downloading embedding model '{}' (first run)...",
        profile.display_name
    );

    if !model_path.exists() {
        match download_file(profile.model_url, &model_path).await {
            Ok(()) => eprintln!("[embeddings] Model downloaded: {}", model_path.display()),
            Err(e) => {
                eprintln!("[embeddings] Model download failed: {e}");
                return None;
            }
        }
    }

    if !tok_path.exists() {
        match download_file(profile.tokenizer_url, &tok_path).await {
            Ok(()) => eprintln!("[embeddings] Tokenizer downloaded: {}", tok_path.display()),
            Err(e) => {
                eprintln!("[embeddings] Tokenizer download failed: {e}");
                return None;
            }
        }
    }

    Some(models_dir.to_path_buf())
}

async fn download_file(url: &str, dest: &Path) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client.get(url).send().await.map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    std::fs::write(dest, &bytes).map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct ModelEnvRestore(Option<String>);

    impl Drop for ModelEnvRestore {
        fn drop(&mut self) {
            if let Some(previous) = self.0.as_ref() {
                std::env::set_var(MODEL_ENV_KEY, previous);
            } else {
                std::env::remove_var(MODEL_ENV_KEY);
            }
        }
    }

    fn set_model_env_for_test(value: Option<&str>) -> ModelEnvRestore {
        let previous = std::env::var(MODEL_ENV_KEY).ok();
        if let Some(value) = value {
            std::env::set_var(MODEL_ENV_KEY, value);
        } else {
            std::env::remove_var(MODEL_ENV_KEY);
        }
        ModelEnvRestore(previous)
    }

    fn set_pool_env_for_test(value: Option<&str>) -> ModelEnvRestore {
        let previous = std::env::var(POOL_ENV_KEY).ok();
        if let Some(value) = value {
            std::env::set_var(POOL_ENV_KEY, value);
        } else {
            std::env::remove_var(POOL_ENV_KEY);
        }
        ModelEnvRestore(previous)
    }

    #[test]
    fn selected_model_defaults_to_minilm() {
        let _env_lock = ENV_LOCK.lock().unwrap();
        let _restore = set_model_env_for_test(None);
        let selected = selected_model_selection();
        assert_eq!(selected.key, "all-minilm-l6-v2");
        assert_eq!(selected.display_name, "all-MiniLM-L6-v2");
        assert_eq!(selected.dimension, 384);
        assert_eq!(selected.model_file, "all-MiniLM-L6-v2.onnx");
        assert_eq!(selected.tokenizer_file, "tokenizer.json");
    }

    #[test]
    fn selected_model_accepts_minilm_aliases() {
        let _env_lock = ENV_LOCK.lock().unwrap();
        let _restore = set_model_env_for_test(None);
        std::env::set_var(MODEL_ENV_KEY, "MiniLM");
        assert_eq!(selected_model_key(), "all-minilm-l6-v2");
        std::env::set_var(MODEL_ENV_KEY, "all_miniLM_l6_v2");
        assert_eq!(selected_model_key(), "all-minilm-l6-v2");
    }

    #[test]
    fn unknown_model_falls_back_to_default() {
        let _env_lock = ENV_LOCK.lock().unwrap();
        let _restore = set_model_env_for_test(Some("unknown-model-key"));
        assert_eq!(selected_model_key(), "all-minilm-l6-v2");
    }

    #[test]
    fn selected_model_accepts_l12_aliases() {
        let _env_lock = ENV_LOCK.lock().unwrap();
        let _restore = set_model_env_for_test(None);
        std::env::set_var(MODEL_ENV_KEY, "all-minilm-l12-v2");
        assert_eq!(selected_model_key(), "all-minilm-l12-v2");
        std::env::set_var(MODEL_ENV_KEY, "minilm-modern");
        assert_eq!(selected_model_key(), "all-minilm-l12-v2");
    }

    #[test]
    fn session_pool_defaults_to_two() {
        let _env_lock = ENV_LOCK.lock().unwrap();
        let _restore = set_pool_env_for_test(None);
        assert_eq!(resolved_pool_size(), DEFAULT_POOL_SIZE);
    }

    #[test]
    fn session_pool_parses_and_clamps_env_values() {
        let _env_lock = ENV_LOCK.lock().unwrap();
        let _restore = set_pool_env_for_test(None);

        std::env::set_var(POOL_ENV_KEY, "3");
        assert_eq!(resolved_pool_size(), 3);

        std::env::set_var(POOL_ENV_KEY, "99");
        assert_eq!(resolved_pool_size(), MAX_POOL_SIZE);

        std::env::set_var(POOL_ENV_KEY, "0");
        assert_eq!(resolved_pool_size(), 1);

        std::env::set_var(POOL_ENV_KEY, "invalid");
        assert_eq!(resolved_pool_size(), DEFAULT_POOL_SIZE);
    }

    #[test]
    fn truncate_to_char_boundary_is_utf8_safe() {
        let text = "a🧠b";
        assert_eq!(EmbeddingEngine::truncate_to_char_boundary(text, 6), text);
        assert_eq!(EmbeddingEngine::truncate_to_char_boundary(text, 5), "a🧠");
        assert_eq!(EmbeddingEngine::truncate_to_char_boundary(text, 4), "a");
        assert_eq!(EmbeddingEngine::truncate_to_char_boundary(text, 1), "a");
    }
}
