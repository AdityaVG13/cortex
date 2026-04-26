// SPDX-License-Identifier: MIT
//! In-process ONNX embedding engine.
//!
//! Uses a selectable embedding profile (default: bge-base-en-v1.5, 438MB,
//! 768-dim) downloaded on first run.
//! Embeddings work the moment Cortex starts.

use ort::session::Session;
use ort::value::Tensor;
use std::borrow::Cow;
use std::io::Write;
use std::path::{Path, PathBuf};
use tokenizers::Tokenizer;

const MODEL_ENV_KEY: &str = "CORTEX_EMBEDDING_MODEL";
const POOL_ENV_KEY: &str = "CORTEX_EMBED_SESSION_POOL_SIZE";
const TEXT_TRUNCATE_BYTES: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PoolingStrategy {
    Mean,
    Cls,
    LastToken,
}

impl PoolingStrategy {
    fn as_str(self) -> &'static str {
        match self {
            PoolingStrategy::Mean => "mean",
            PoolingStrategy::Cls => "cls",
            PoolingStrategy::LastToken => "last_token",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum EmbeddingInputKind {
    Query,
    Passage,
}

#[derive(Clone, Copy, Debug)]
struct EmbeddingModelAsset {
    file: &'static str,
    url: &'static str,
}

struct EmbeddingModelProfile {
    key: &'static str,
    display_name: &'static str,
    dimension: usize,
    max_input_tokens: usize,
    model_file: &'static str,
    tokenizer_file: &'static str,
    model_url: &'static str,
    tokenizer_url: &'static str,
    auxiliary_files: &'static [EmbeddingModelAsset],
    query_prefix: &'static str,
    passage_prefix: &'static str,
    pooling: PoolingStrategy,
    normalize: bool,
    include_token_type_ids: bool,
}

impl EmbeddingModelProfile {
    fn primary_assets(&self) -> [EmbeddingModelAsset; 2] {
        [
            EmbeddingModelAsset {
                file: self.model_file,
                url: self.model_url,
            },
            EmbeddingModelAsset {
                file: self.tokenizer_file,
                url: self.tokenizer_url,
            },
        ]
    }

    fn missing_assets(&self, models_dir: &Path) -> Vec<EmbeddingModelAsset> {
        let primary = self.primary_assets();
        primary
            .iter()
            .chain(self.auxiliary_files.iter())
            .copied()
            .filter(|asset| !models_dir.join(asset.file).exists())
            .collect()
    }

    fn assets_exist(&self, models_dir: &Path) -> bool {
        self.missing_assets(models_dir).is_empty()
    }
}

const ALL_MINILM_L6_V2: EmbeddingModelProfile = EmbeddingModelProfile {
    key: "all-minilm-l6-v2",
    display_name: "all-MiniLM-L6-v2",
    dimension: 384,
    max_input_tokens: 256,
    model_file: "all-MiniLM-L6-v2.onnx",
    tokenizer_file: "tokenizer.json",
    model_url:
        "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/onnx/model.onnx",
    tokenizer_url:
        "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/tokenizer.json",
    auxiliary_files: &[],
    query_prefix: "",
    passage_prefix: "",
    pooling: PoolingStrategy::Mean,
    normalize: true,
    include_token_type_ids: true,
};

const ALL_MINILM_L12_V2: EmbeddingModelProfile = EmbeddingModelProfile {
    key: "all-minilm-l12-v2",
    display_name: "all-MiniLM-L12-v2",
    dimension: 384,
    max_input_tokens: 256,
    model_file: "all-MiniLM-L12-v2.onnx",
    tokenizer_file: "all-MiniLM-L12-v2-tokenizer.json",
    model_url:
        "https://huggingface.co/sentence-transformers/all-MiniLM-L12-v2/resolve/main/onnx/model.onnx",
    tokenizer_url:
        "https://huggingface.co/sentence-transformers/all-MiniLM-L12-v2/resolve/main/tokenizer.json",
    auxiliary_files: &[],
    query_prefix: "",
    passage_prefix: "",
    pooling: PoolingStrategy::Mean,
    normalize: true,
    include_token_type_ids: true,
};

const BGE_BASE_EN_V1_5: EmbeddingModelProfile = EmbeddingModelProfile {
    key: "bge-base-en-v1.5",
    display_name: "bge-base-en-v1.5",
    dimension: 768,
    max_input_tokens: 512,
    model_file: "bge-base-en-v1.5.onnx",
    tokenizer_file: "bge-base-en-v1.5-tokenizer.json",
    model_url: "https://huggingface.co/BAAI/bge-base-en-v1.5/resolve/main/onnx/model.onnx",
    tokenizer_url: "https://huggingface.co/BAAI/bge-base-en-v1.5/resolve/main/tokenizer.json",
    auxiliary_files: &[],
    query_prefix: "Represent this sentence for searching relevant passages: ",
    passage_prefix: "",
    pooling: PoolingStrategy::Cls,
    normalize: true,
    include_token_type_ids: true,
};

const QWEN3_EMBEDDING_0_6B: EmbeddingModelProfile = EmbeddingModelProfile {
    key: "qwen3-embedding-0.6b",
    display_name: "Qwen3-Embedding-0.6B",
    dimension: 1024,
    max_input_tokens: 512,
    model_file: "qwen3-embedding-0.6b/model_uint8.onnx",
    tokenizer_file: "qwen3-embedding-0.6b/tokenizer.json",
    model_url:
        "https://huggingface.co/onnx-community/Qwen3-Embedding-0.6B-ONNX/resolve/main/onnx/model_uint8.onnx",
    tokenizer_url:
        "https://huggingface.co/onnx-community/Qwen3-Embedding-0.6B-ONNX/resolve/main/tokenizer.json",
    auxiliary_files: &[],
    query_prefix:
        "Instruct: Given a web search query, retrieve relevant passages that answer the query\nQuery:",
    passage_prefix: "",
    pooling: PoolingStrategy::LastToken,
    normalize: true,
    include_token_type_ids: false,
};

const DEFAULT_PROFILE: &EmbeddingModelProfile = &BGE_BASE_EN_V1_5;

#[derive(Clone, Copy, Debug)]
pub struct EmbeddingModelSelection {
    pub key: &'static str,
    pub display_name: &'static str,
    pub dimension: usize,
    pub max_input_tokens: usize,
    pub model_file: &'static str,
    pub tokenizer_file: &'static str,
    pub pooling: &'static str,
}

fn normalize_model_key(raw: &str) -> String {
    raw.trim().to_ascii_lowercase().replace('_', "-")
}

fn resolve_profile() -> &'static EmbeddingModelProfile {
    match std::env::var(MODEL_ENV_KEY) {
        Ok(raw) => match normalize_model_key(&raw).as_str() {
            "all-minilm-l6-v2" | "all-minilm-l6v2" | "minilm-l6" | "minilm-legacy" => {
                &ALL_MINILM_L6_V2
            }
            "all-minilm-l12-v2" | "all-minilm-l12v2" | "minilm-l12" | "minilm-modern"
            | "minilm" => &ALL_MINILM_L12_V2,
            "bge-base-en-v1.5" | "bge-base-en-v15" | "bge-base" | "bge" => &BGE_BASE_EN_V1_5,
            "qwen3-embedding-0.6b" | "qwen3-embedding-06b" | "qwen3-embedding" | "qwen3" => {
                &QWEN3_EMBEDDING_0_6B
            }
            unknown => {
                eprintln!(
                    "[embeddings] Unknown {MODEL_ENV_KEY}='{unknown}', falling back to {}",
                    DEFAULT_PROFILE.key
                );
                DEFAULT_PROFILE
            }
        },
        Err(_) => DEFAULT_PROFILE,
    }
}

pub fn selected_model_selection() -> EmbeddingModelSelection {
    let profile = resolve_profile();
    EmbeddingModelSelection {
        key: profile.key,
        display_name: profile.display_name,
        dimension: profile.dimension,
        max_input_tokens: profile.max_input_tokens,
        model_file: profile.model_file,
        tokenizer_file: profile.tokenizer_file,
        pooling: profile.pooling.as_str(),
    }
}

pub fn selected_model_key() -> &'static str {
    selected_model_selection().key
}

pub fn selected_model_assets_exist(models_dir: &Path) -> bool {
    resolve_profile().assets_exist(models_dir)
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
    max_input_tokens: usize,
    model_key: &'static str,
    query_prefix: &'static str,
    passage_prefix: &'static str,
    pooling: PoolingStrategy,
    normalize: bool,
    include_token_type_ids: bool,
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

        let missing_assets = profile.missing_assets(models_dir);
        if !missing_assets.is_empty() {
            let missing = missing_assets
                .iter()
                .map(|asset| asset.file)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "model assets missing ({missing}) at {}",
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
            max_input_tokens: profile.max_input_tokens,
            model_key: profile.key,
            query_prefix: profile.query_prefix,
            passage_prefix: profile.passage_prefix,
            pooling: profile.pooling,
            normalize: profile.normalize,
            include_token_type_ids: profile.include_token_type_ids,
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

    fn input_text<'a>(&self, text: &'a str, kind: EmbeddingInputKind) -> Cow<'a, str> {
        let prefix = match kind {
            EmbeddingInputKind::Query => self.query_prefix,
            EmbeddingInputKind::Passage => self.passage_prefix,
        };
        if prefix.is_empty() {
            Cow::Borrowed(text)
        } else {
            Cow::Owned(format!("{prefix}{text}"))
        }
    }

    fn embed_with_kind(&self, text: &str, kind: EmbeddingInputKind) -> Option<Vec<f32>> {
        let input = self.input_text(text, kind);
        let truncated = Self::truncate_to_char_boundary(input.as_ref(), TEXT_TRUNCATE_BYTES);
        let encoding = self.tokenizer.encode(truncated, true).ok()?;

        let ids = encoding.get_ids();
        let attention = encoding.get_attention_mask();
        let type_ids = encoding.get_type_ids();

        let len = ids.len().min(self.max_input_tokens);
        if len == 0 {
            return None;
        }
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
        let outputs = if self.include_token_type_ids {
            session.run(ort::inputs![
                "input_ids" => ids_tensor,
                "attention_mask" => mask_tensor,
                "token_type_ids" => type_tensor,
            ])
        } else {
            session.run(ort::inputs![
                "input_ids" => ids_tensor,
                "attention_mask" => mask_tensor,
            ])
        }
        .ok()?;

        let (shape, data) = outputs[0].try_extract_tensor::<f32>().ok()?;
        let dims: Vec<i64> = shape.iter().copied().collect();

        if dims.len() != 3 || dims[2] as usize != self.dimension {
            eprintln!("[embeddings] Unexpected output shape: {dims:?}");
            return None;
        }

        let seq_len_out = dims[1] as usize;
        Self::pool_output(
            data,
            self.dimension,
            seq_len_out,
            attention,
            self.pooling,
            self.normalize,
        )
    }

    fn pool_output(
        data: &[f32],
        dimension: usize,
        seq_len_out: usize,
        attention: &[u32],
        pooling: PoolingStrategy,
        normalize: bool,
    ) -> Option<Vec<f32>> {
        if dimension == 0 || seq_len_out == 0 || data.len() < seq_len_out * dimension {
            return None;
        }

        let mut pooled = vec![0.0f32; dimension];
        match pooling {
            PoolingStrategy::Mean => {
                let mut mask_sum = 0.0f32;
                let attention_fallback_index = attention.len().saturating_sub(1);
                for seq_idx in 0..seq_len_out {
                    let mask_val = attention
                        .get(seq_idx)
                        .or_else(|| attention.get(attention_fallback_index))
                        .copied()
                        .unwrap_or(1) as f32;
                    mask_sum += mask_val;
                    let offset = seq_idx * dimension;
                    for dim in 0..dimension {
                        pooled[dim] += data[offset + dim] * mask_val;
                    }
                }

                if mask_sum > 0.0 {
                    for v in &mut pooled {
                        *v /= mask_sum;
                    }
                }
            }
            PoolingStrategy::Cls => {
                pooled.copy_from_slice(data.get(0..dimension)?);
            }
            PoolingStrategy::LastToken => {
                let attention_limit = seq_len_out.min(attention.len());
                let last_idx = attention
                    .iter()
                    .take(attention_limit)
                    .rposition(|mask| *mask != 0)
                    .unwrap_or(seq_len_out - 1);
                let offset = last_idx * dimension;
                pooled.copy_from_slice(data.get(offset..offset + dimension)?);
            }
        }

        if normalize {
            let norm: f32 = pooled.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > 0.0 {
                for v in &mut pooled {
                    *v /= norm;
                }
            }
        }

        Some(pooled)
    }

    /// Generate a passage embedding for `text` using the selected profile.
    pub fn embed(&self, text: &str) -> Option<Vec<f32>> {
        self.embed_with_kind(text, EmbeddingInputKind::Passage)
    }

    /// Generate a query embedding for retrieval. Profiles such as BGE apply a
    /// query instruction prefix here while stored passages remain unprefixed.
    pub fn embed_query(&self, text: &str) -> Option<Vec<f32>> {
        self.embed_with_kind(text, EmbeddingInputKind::Query)
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
    fn selected_model_defaults_to_bge_base() {
        let _env_lock = ENV_LOCK.lock().unwrap();
        let _restore = set_model_env_for_test(None);
        let selected = selected_model_selection();
        assert_eq!(selected.key, "bge-base-en-v1.5");
        assert_eq!(selected.display_name, "bge-base-en-v1.5");
        assert_eq!(selected.dimension, 768);
        assert_eq!(selected.max_input_tokens, 512);
        assert_eq!(selected.model_file, "bge-base-en-v1.5.onnx");
        assert_eq!(selected.tokenizer_file, "bge-base-en-v1.5-tokenizer.json");
        assert_eq!(selected.pooling, "cls");
    }

    #[test]
    fn selected_model_accepts_bge_aliases() {
        let _env_lock = ENV_LOCK.lock().unwrap();
        let _restore = set_model_env_for_test(None);
        std::env::set_var(MODEL_ENV_KEY, "bge");
        assert_eq!(selected_model_key(), "bge-base-en-v1.5");
        std::env::set_var(MODEL_ENV_KEY, "bge-base");
        assert_eq!(selected_model_key(), "bge-base-en-v1.5");
    }

    #[test]
    fn selected_model_accepts_legacy_l6_aliases() {
        let _env_lock = ENV_LOCK.lock().unwrap();
        let _restore = set_model_env_for_test(None);
        std::env::set_var(MODEL_ENV_KEY, "minilm-l6");
        assert_eq!(selected_model_key(), "all-minilm-l6-v2");
        std::env::set_var(MODEL_ENV_KEY, "minilm-legacy");
        assert_eq!(selected_model_key(), "all-minilm-l6-v2");
    }

    #[test]
    fn unknown_model_falls_back_to_default() {
        let _env_lock = ENV_LOCK.lock().unwrap();
        let _restore = set_model_env_for_test(Some("unknown-model-key"));
        assert_eq!(selected_model_key(), "bge-base-en-v1.5");
    }

    #[test]
    fn selected_model_accepts_l12_aliases() {
        let _env_lock = ENV_LOCK.lock().unwrap();
        let _restore = set_model_env_for_test(None);
        std::env::set_var(MODEL_ENV_KEY, "all-minilm-l12-v2");
        assert_eq!(selected_model_key(), "all-minilm-l12-v2");
        std::env::set_var(MODEL_ENV_KEY, "MiniLM");
        assert_eq!(selected_model_key(), "all-minilm-l12-v2");
        std::env::set_var(MODEL_ENV_KEY, "minilm-modern");
        assert_eq!(selected_model_key(), "all-minilm-l12-v2");
    }

    #[test]
    fn selected_model_accepts_qwen3_aliases() {
        let _env_lock = ENV_LOCK.lock().unwrap();
        let _restore = set_model_env_for_test(None);
        std::env::set_var(MODEL_ENV_KEY, "qwen3");
        let selected = selected_model_selection();
        assert_eq!(selected.key, "qwen3-embedding-0.6b");
        assert_eq!(selected.dimension, 1024);
        assert_eq!(selected.max_input_tokens, 512);
        assert_eq!(selected.model_file, "qwen3-embedding-0.6b/model_uint8.onnx");
        assert_eq!(
            selected.tokenizer_file,
            "qwen3-embedding-0.6b/tokenizer.json"
        );
        assert_eq!(selected.pooling, "last_token");
    }

    #[test]
    fn qwen3_profile_uses_single_quantized_onnx_asset() {
        let missing = QWEN3_EMBEDDING_0_6B.missing_assets(Path::new("missing-models-dir"));
        let files = missing.iter().map(|asset| asset.file).collect::<Vec<_>>();
        assert!(files.contains(&"qwen3-embedding-0.6b/model_uint8.onnx"));
        assert!(files.contains(&"qwen3-embedding-0.6b/tokenizer.json"));
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn pooling_strategies_select_expected_token() {
        let data = [
            1.0, 0.0, // token 0
            0.0, 2.0, // token 1
            3.0, 0.0, // token 2
        ];
        let attention = [1, 1, 0];

        let mean =
            EmbeddingEngine::pool_output(&data, 2, 3, &attention, PoolingStrategy::Mean, false)
                .unwrap();
        assert_eq!(mean, vec![0.5, 1.0]);

        let cls =
            EmbeddingEngine::pool_output(&data, 2, 3, &attention, PoolingStrategy::Cls, false)
                .unwrap();
        assert_eq!(cls, vec![1.0, 0.0]);

        let last = EmbeddingEngine::pool_output(
            &data,
            2,
            3,
            &attention,
            PoolingStrategy::LastToken,
            false,
        )
        .unwrap();
        assert_eq!(last, vec![0.0, 2.0]);
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
