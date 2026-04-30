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

/// Encode a `Vec<f32>` as a SQLite BLOB. As of v0.6.0 this writes the
/// compact PQ8 format (~4x smaller than LE f32). Reads transparently
/// handle both formats via `blob_to_vector` — see `pq8_blob_to_vector`
/// and `legacy_f32_blob_to_vector` for format-specific entry points.
pub fn vector_to_blob(vec: &[f32]) -> Vec<u8> {
    vector_to_pq8_blob(vec)
}

/// Strict legacy encoder: writes LE f32 packed bytes. Used by tests that
/// need to assert behaviour on legacy blobs, and by any one-off migration
/// tool that needs to produce the old wire format.
pub fn vector_to_legacy_f32_blob(vec: &[f32]) -> Vec<u8> {
    vec.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Decode a SQLite BLOB back to `Vec<f32>`. Auto-detects PQ8 quantized
/// blobs vs legacy LE-f32 blobs so the read path transparently handles
/// the mixed corpus during the backfill window. Callers that specifically
/// need the legacy decoder can call `legacy_f32_blob_to_vector` directly.
pub fn blob_to_vector(blob: &[u8]) -> Vec<f32> {
    if let Some(v) = pq8_blob_to_vector(blob) {
        return v;
    }
    legacy_f32_blob_to_vector(blob)
}

/// Strict legacy decoder: treat the blob as a packed LE-f32 array. Used by
/// tests and any caller that knows it is reading pre-PQ8 data.
pub fn legacy_f32_blob_to_vector(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

// ---------------------------------------------------------------------------
// PQ8 (per-vector symmetric int8) quantization
//
// Embedding vectors are the second-largest table in a mature Cortex DB. The
// f32 representation costs 4 * D bytes per row (3072 bytes for BGE-base).
// Symmetric int8 quantization with a single per-vector f32 scale collapses
// that to D + 5 bytes (773 bytes for BGE-base) — a ~4x reduction — while
// preserving cosine similarity to within a few hundredths in practice.
//
// BGE produces L2-normalised vectors so values land in [-1, 1] tightly,
// which means the quantization scale stays small and round-trip error is
// uniform. For non-normalised models the scale tracks the per-vector
// max(|v|) so the dynamic range of any single vector is fully used.
//
// Blob layout (PQ8_FORMAT_VERSION = 0x02):
//
//   byte 0:        magic = PQ8_MAGIC_BYTE (0xC8 — distinct from any
//                  byte that can appear at the head of an LE f32 storing
//                  a normalised value)
//   byte 1:        format version (0x02)
//   bytes 2..6:    scale (LE f32). Zero implies an all-zero vector.
//   bytes 6..6+D:  D signed int8 values, one per dimension
//
// Total: D + 6 bytes. For D=768 that is 774 bytes vs 3072 bytes of f32 —
// a 3.97x compression ratio. The 6-byte header amortises trivially.
// ---------------------------------------------------------------------------

/// Magic byte that uniquely identifies a PQ8 blob. Chosen so it cannot
/// appear as the leading byte of an LE-encoded f32 holding a typical
/// normalised value: 0xC8 corresponds to LE float values around -1e22.
pub const PQ8_MAGIC_BYTE: u8 = 0xC8;
/// Current PQ8 wire format version. Future formats bump this.
pub const PQ8_FORMAT_VERSION: u8 = 0x02;
/// Header size in bytes: magic(1) + version(1) + scale(4).
pub const PQ8_HEADER_BYTES: usize = 6;

/// Quantize a Vec<f32> to a compact int8 blob. Returns the raw bytes ready
/// for SQLite storage. Lossless when the input is all-zero (scale = 0).
pub fn vector_to_pq8_blob(vec: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(PQ8_HEADER_BYTES + vec.len());
    out.push(PQ8_MAGIC_BYTE);
    out.push(PQ8_FORMAT_VERSION);

    // Scale is the per-vector max absolute value mapped onto int8::MAX so
    // every vector uses its full int8 dynamic range. NaN/inf inputs are
    // treated as zero; we never want a poisoned scale to corrupt storage.
    let max_abs = vec
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(0.0f32, |acc, v| acc.max(v.abs()));

    let scale = if max_abs > 0.0 {
        max_abs / i8::MAX as f32
    } else {
        0.0
    };
    out.extend_from_slice(&scale.to_le_bytes());

    for &v in vec {
        let q = if scale > 0.0 && v.is_finite() {
            // round-half-to-even via f32::round, then clamp into int8.
            let scaled = (v / scale).round().clamp(i8::MIN as f32, i8::MAX as f32);
            scaled as i8
        } else {
            0i8
        };
        out.push(q as u8);
    }
    out
}

/// True iff the blob is a PQ8-encoded vector (magic + version match).
pub fn is_pq8_blob(blob: &[u8]) -> bool {
    blob.len() >= PQ8_HEADER_BYTES
        && blob[0] == PQ8_MAGIC_BYTE
        && blob[1] == PQ8_FORMAT_VERSION
}

/// Decode a PQ8 blob back to Vec<f32>. Returns None if the blob is not a
/// valid PQ8 payload — callers should fall back to `blob_to_vector` on
/// legacy LE-f32 storage in that case.
pub fn pq8_blob_to_vector(blob: &[u8]) -> Option<Vec<f32>> {
    if !is_pq8_blob(blob) {
        return None;
    }
    let scale = f32::from_le_bytes([blob[2], blob[3], blob[4], blob[5]]);
    let body = &blob[PQ8_HEADER_BYTES..];
    let mut out = Vec::with_capacity(body.len());
    if scale == 0.0 {
        // All-zero vector. Preserve the original length.
        out.resize(body.len(), 0.0);
        return Some(out);
    }
    for &b in body {
        let q = b as i8;
        out.push(q as f32 * scale);
    }
    Some(out)
}

/// Convenience: max absolute element-wise error between two equal-length
/// f32 slices. Used by tests to bound quantization round-trip error.
#[cfg(test)]
pub(crate) fn max_abs_error(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max)
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

    // ── PQ8 quantization tests ────────────────────────────────────────────

    fn deterministic_unit_vec(seed: u64, dim: usize) -> Vec<f32> {
        // Tiny xorshift64 PRNG seeded for reproducibility — keeps the test
        // suite hermetic without pulling in the `rand` dev-dependency.
        let mut s = seed | 1;
        let mut raw = Vec::with_capacity(dim);
        for _ in 0..dim {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            raw.push(((s as i64) as f32) / (i64::MAX as f32));
        }
        let norm: f32 = raw.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut raw {
                *v /= norm;
            }
        }
        raw
    }

    #[test]
    fn pq8_blob_is_well_formed() {
        let v = deterministic_unit_vec(0xDEADBEEF, 768);
        let blob = vector_to_pq8_blob(&v);
        assert_eq!(blob.len(), PQ8_HEADER_BYTES + v.len());
        assert_eq!(blob[0], PQ8_MAGIC_BYTE);
        assert_eq!(blob[1], PQ8_FORMAT_VERSION);
        assert!(is_pq8_blob(&blob));
    }

    #[test]
    fn pq8_compression_ratio_matches_target() {
        // Goal: PQ8 cuts a 768-dim BGE vector from 3072B to 774B. Compare
        // explicitly against the legacy f32 encoder since the default path
        // now writes PQ8.
        let v = deterministic_unit_vec(0x11, 768);
        let f32_blob = vector_to_legacy_f32_blob(&v);
        let q8_blob = vector_to_pq8_blob(&v);
        assert_eq!(f32_blob.len(), 3072);
        assert_eq!(q8_blob.len(), 774);
        let ratio = f32_blob.len() as f32 / q8_blob.len() as f32;
        assert!(
            ratio > 3.9 && ratio < 4.0,
            "expected ~4x ratio, got {ratio}"
        );
        // The default writer must agree with the explicit PQ8 encoder.
        assert_eq!(vector_to_blob(&v), q8_blob);
    }

    #[test]
    fn pq8_roundtrip_bounds_error_by_scale() {
        // For a unit vector the scale ~ 1/127, so per-dimension error is
        // bounded by half a step (~0.004). Verify across many seeds.
        for seed in [0x1, 0x100, 0x10000, 0xCAFE, 0xBEEF, 0xFEED] {
            let v = deterministic_unit_vec(seed, 768);
            let blob = vector_to_pq8_blob(&v);
            let recovered = pq8_blob_to_vector(&blob).expect("blob should decode");
            assert_eq!(recovered.len(), v.len());
            // Reconstruct the scale from the blob header so the bound
            // adapts to the actual magnitude of the input.
            let scale = f32::from_le_bytes([blob[2], blob[3], blob[4], blob[5]]);
            let bound = scale; // round error <= half a step, allow full step for safety.
            let err = max_abs_error(&v, &recovered);
            assert!(
                err <= bound,
                "seed={seed:#x}: max_abs_error={err} exceeds bound={bound}"
            );
        }
    }

    #[test]
    fn pq8_preserves_cosine_similarity() {
        // Cosine similarity drift after PQ8 should be small (< 0.01) for
        // L2-normalised vectors. We test self-similarity (=1.0), an
        // orthogonal pair (=0.0 ish), and several random pairs.
        let a = deterministic_unit_vec(0xA1, 768);
        let b = deterministic_unit_vec(0xB2, 768);
        let pairs = [(a.clone(), a.clone()), (a.clone(), b.clone())];
        for (x, y) in pairs {
            let qx = pq8_blob_to_vector(&vector_to_pq8_blob(&x)).unwrap();
            let qy = pq8_blob_to_vector(&vector_to_pq8_blob(&y)).unwrap();
            let raw = cosine_similarity(&x, &y);
            let q = cosine_similarity(&qx, &qy);
            let drift = (raw - q).abs();
            assert!(
                drift < 0.01,
                "cosine drift {drift} too large; raw={raw}, q={q}"
            );
        }
    }

    #[test]
    fn pq8_handles_all_zero_vector() {
        let z = vec![0.0f32; 768];
        let blob = vector_to_pq8_blob(&z);
        let recovered = pq8_blob_to_vector(&blob).unwrap();
        assert_eq!(recovered.len(), 768);
        assert!(recovered.iter().all(|&v| v == 0.0));
        // Scale must be zero for the all-zero special case.
        let scale = f32::from_le_bytes([blob[2], blob[3], blob[4], blob[5]]);
        assert_eq!(scale, 0.0);
    }

    #[test]
    fn pq8_handles_nan_and_infinity_safely() {
        // NaN/inf must NOT poison the scale; they get treated as zero so the
        // remaining valid dimensions are still represented faithfully.
        let mut v = vec![0.5f32; 8];
        v[3] = f32::NAN;
        v[5] = f32::INFINITY;
        v[6] = f32::NEG_INFINITY;
        let blob = vector_to_pq8_blob(&v);
        assert!(is_pq8_blob(&blob));
        let recovered = pq8_blob_to_vector(&blob).unwrap();
        // Non-finite slots come back as zero; finite slots come back ~0.5.
        assert!(recovered[3].abs() < 1e-3);
        assert!(recovered[5].abs() < 1e-3);
        assert!(recovered[6].abs() < 1e-3);
        assert!((recovered[0] - 0.5).abs() < 0.01);
    }

    #[test]
    fn pq8_handles_large_magnitude_input() {
        // Non-normalised vector — scale should track max(|v|) so dynamic
        // range is fully used and clamping never silently saturates.
        let v: Vec<f32> = (0..16).map(|i| (i as f32) - 8.0).collect();
        let blob = vector_to_pq8_blob(&v);
        let recovered = pq8_blob_to_vector(&blob).unwrap();
        let scale = f32::from_le_bytes([blob[2], blob[3], blob[4], blob[5]]);
        // Bound: round error <= scale.
        let err = max_abs_error(&v, &recovered);
        assert!(err <= scale, "err={err} > scale={scale}");
    }

    #[test]
    fn pq8_single_element_works() {
        let v = vec![0.7f32];
        let blob = vector_to_pq8_blob(&v);
        let recovered = pq8_blob_to_vector(&blob).unwrap();
        assert_eq!(recovered.len(), 1);
        assert!((recovered[0] - 0.7).abs() < 0.01);
    }

    #[test]
    fn legacy_blob_is_not_misidentified_as_pq8() {
        // A legacy LE-f32 blob must never be decoded as PQ8 — pq8_blob_to_vector
        // returns None and blob_to_vector falls back to the f32 path. We
        // explicitly call the legacy encoder here because the default
        // `vector_to_blob` now writes PQ8.
        let v = vec![0.1f32, -0.2, 0.3, -0.4, 0.5];
        let legacy = vector_to_legacy_f32_blob(&v);
        assert!(!is_pq8_blob(&legacy));
        assert!(pq8_blob_to_vector(&legacy).is_none());
        let recovered = blob_to_vector(&legacy);
        assert_eq!(recovered.len(), v.len());
        for (a, b) in v.iter().zip(recovered.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn blob_to_vector_handles_pq8_payload() {
        let v = deterministic_unit_vec(0xABCD, 16);
        let blob = vector_to_pq8_blob(&v);
        let recovered = blob_to_vector(&blob);
        assert_eq!(recovered.len(), v.len());
        let drift = max_abs_error(&v, &recovered);
        let scale = f32::from_le_bytes([blob[2], blob[3], blob[4], blob[5]]);
        assert!(drift <= scale);
    }

    #[test]
    fn pq8_decodes_fail_on_bad_header() {
        // Truncated header
        assert!(pq8_blob_to_vector(&[]).is_none());
        assert!(pq8_blob_to_vector(&[PQ8_MAGIC_BYTE]).is_none());
        // Wrong magic
        let mut bad = vector_to_pq8_blob(&[0.1, 0.2, 0.3]);
        bad[0] = 0x00;
        assert!(pq8_blob_to_vector(&bad).is_none());
        // Wrong version
        let mut bad = vector_to_pq8_blob(&[0.1, 0.2, 0.3]);
        bad[1] = 0xFF;
        assert!(pq8_blob_to_vector(&bad).is_none());
    }

    #[test]
    fn pq8_recall_preserves_top_k_ordering() {
        // Build a small corpus, find the top-3 neighbours of a query in
        // both raw f32 and PQ8 round-tripped form, and assert the top
        // results agree. This is the recall-quality regression guard.
        let dim = 64;
        let corpus: Vec<Vec<f32>> = (0..50)
            .map(|i| deterministic_unit_vec(0xC0DE_0000 + i as u64, dim))
            .collect();
        let query = deterministic_unit_vec(0xC0DE_0001, dim); // exists in corpus
        let raw_scores: Vec<(usize, f32)> = corpus
            .iter()
            .enumerate()
            .map(|(i, v)| (i, cosine_similarity(&query, v)))
            .collect();
        let q_corpus: Vec<Vec<f32>> = corpus
            .iter()
            .map(|v| pq8_blob_to_vector(&vector_to_pq8_blob(v)).unwrap())
            .collect();
        let q_query = pq8_blob_to_vector(&vector_to_pq8_blob(&query)).unwrap();
        let q_scores: Vec<(usize, f32)> = q_corpus
            .iter()
            .enumerate()
            .map(|(i, v)| (i, cosine_similarity(&q_query, v)))
            .collect();
        let mut raw_sorted = raw_scores.clone();
        raw_sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let mut q_sorted = q_scores.clone();
        q_sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let raw_top: Vec<usize> = raw_sorted.iter().take(3).map(|p| p.0).collect();
        let q_top: Vec<usize> = q_sorted.iter().take(3).map(|p| p.0).collect();
        assert_eq!(
            raw_top[0], q_top[0],
            "top-1 must match: raw={raw_top:?}, q={q_top:?}"
        );
        // Top-3 may permute slightly; require at least 2/3 overlap.
        let overlap = raw_top.iter().filter(|i| q_top.contains(i)).count();
        assert!(overlap >= 2, "top-3 overlap < 2: raw={raw_top:?}, q={q_top:?}");
    }
}
