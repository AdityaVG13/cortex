// SPDX-License-Identifier: MIT
//! Cross-encoder reranker support for recall.
//!
//! Default mode is off. When enabled, the model is loaded only if all assets are
//! already present under `~/.cortex/models/rerank/ms-marco-MiniLM-L-6-v2/`.
//! `cortex setup` can download those assets.

use ort::session::Session;
use ort::value::Tensor;
use std::cmp::Ordering;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tokenizers::{EncodeInput, Tokenizer};

const RERANK_MODE_ENV: &str = "CORTEX_RERANK_MODE";
const RERANK_ENABLED_ENV: &str = "CORTEX_RERANK_ENABLED";
const RERANK_TOP_N_ENV: &str = "CORTEX_RERANK_TOP_N";
const RERANK_FUSION_ALPHA_ENV: &str = "CORTEX_RERANK_FUSION_ALPHA";
const DEFAULT_TOP_N: usize = 24;
const MAX_TOP_N: usize = 64;
const DEFAULT_FUSION_ALPHA: f64 = 0.65;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RerankMode {
    Off,
    Shadow,
    Primary,
}

impl RerankMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RerankMode::Off => "off",
            RerankMode::Shadow => "shadow",
            RerankMode::Primary => "primary",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RerankConfig {
    pub mode: RerankMode,
    pub top_n: usize,
    pub fusion_alpha: f64,
}

impl RerankConfig {
    pub fn from_env() -> Self {
        let mode = parse_mode_from_env();
        let top_n = std::env::var(RERANK_TOP_N_ENV)
            .ok()
            .and_then(|raw| raw.trim().parse::<usize>().ok())
            .unwrap_or(DEFAULT_TOP_N)
            .clamp(1, MAX_TOP_N);
        let fusion_alpha = std::env::var(RERANK_FUSION_ALPHA_ENV)
            .ok()
            .and_then(|raw| raw.trim().parse::<f64>().ok())
            .filter(|value| value.is_finite())
            .unwrap_or(DEFAULT_FUSION_ALPHA)
            .clamp(0.0, 1.0);
        Self {
            mode,
            top_n,
            fusion_alpha,
        }
    }

    #[cfg(test)]
    pub fn off() -> Self {
        Self {
            mode: RerankMode::Off,
            top_n: DEFAULT_TOP_N,
            fusion_alpha: DEFAULT_FUSION_ALPHA,
        }
    }

    pub fn is_active(&self) -> bool {
        !matches!(self.mode, RerankMode::Off)
    }

    pub fn is_primary(&self) -> bool {
        matches!(self.mode, RerankMode::Primary)
    }
}

fn parse_mode_from_env() -> RerankMode {
    if let Ok(raw) = std::env::var(RERANK_MODE_ENV) {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" | "0" | "false" | "disabled" => return RerankMode::Off,
            "shadow" | "trial" | "observe" => return RerankMode::Shadow,
            "primary" | "on" | "1" | "true" | "enabled" => return RerankMode::Primary,
            unknown => {
                eprintln!("[rerank] Unknown {RERANK_MODE_ENV}={unknown:?}; using off");
                return RerankMode::Off;
            }
        }
    }

    match std::env::var(RERANK_ENABLED_ENV) {
        Ok(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" | "primary" => RerankMode::Primary,
            "shadow" | "trial" => RerankMode::Shadow,
            _ => RerankMode::Off,
        },
        Err(_) => RerankMode::Off,
    }
}

#[derive(Clone, Copy, Debug)]
struct RerankerAsset {
    file: &'static str,
    url: &'static str,
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

struct RerankerProfile {
    key: &'static str,
    display_name: &'static str,
    model_size_mb: u64,
    max_input_tokens: usize,
    model_file: &'static str,
    tokenizer_file: &'static str,
    assets: &'static [RerankerAsset],
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

    fn missing_assets(&self, models_dir: &Path) -> Vec<RerankerAsset> {
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

fn selected_profile() -> &'static RerankerProfile {
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

#[derive(Clone, Debug)]
pub struct RerankCandidate {
    pub id: String,
    pub text: String,
    pub base_score: f64,
}

#[derive(Clone, Debug)]
pub struct RerankedScore {
    pub id: String,
    pub base_score: f64,
    pub rerank_score: f64,
    pub fused_score: f64,
}

pub trait Reranker: Send + Sync {
    fn name(&self) -> &'static str;
    fn model_size_mb(&self) -> u64;
    fn rerank(
        &self,
        query: &str,
        candidates: &[RerankCandidate],
        fusion_alpha: f64,
    ) -> Result<Vec<RerankedScore>, String>;
}

#[cfg(test)]
pub struct NoopReranker;

#[cfg(test)]
impl Reranker for NoopReranker {
    fn name(&self) -> &'static str {
        "noop_baseline"
    }

    fn model_size_mb(&self) -> u64 {
        0
    }

    fn rerank(
        &self,
        _query: &str,
        candidates: &[RerankCandidate],
        fusion_alpha: f64,
    ) -> Result<Vec<RerankedScore>, String> {
        let scores = candidates
            .iter()
            .map(|candidate| (candidate.id.clone(), candidate.base_score as f32))
            .collect::<Vec<_>>();
        Ok(fuse_scores(candidates, &scores, fusion_alpha))
    }
}

pub struct MiniLmReranker {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
    max_input_tokens: usize,
}

impl MiniLmReranker {
    pub fn load(models_dir: &Path) -> Option<Self> {
        match Self::try_load(models_dir) {
            Ok(reranker) => Some(reranker),
            Err(error) => {
                eprintln!("[rerank] Engine load failed: {error}");
                None
            }
        }
    }

    fn try_load(models_dir: &Path) -> Result<Self, String> {
        let profile = selected_profile();
        let missing = profile.missing_assets(models_dir);
        if !missing.is_empty() {
            let missing = missing
                .iter()
                .map(|asset| asset.file)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "model assets missing ({missing}) at {}",
                models_dir.display()
            ));
        }

        let model_path = models_dir.join(profile.model_file);
        let tokenizer_path = models_dir.join(profile.tokenizer_file);
        let tokenizer = Tokenizer::from_file(&tokenizer_path).map_err(|error| {
            format!(
                "failed to load tokenizer {}: {error}",
                tokenizer_path.display()
            )
        })?;
        let session = build_session(&model_path)?;
        Ok(Self {
            session: Mutex::new(session),
            tokenizer,
            max_input_tokens: profile.max_input_tokens,
        })
    }

    fn score_pair(&self, query: &str, document: &str) -> Result<f32, String> {
        let encoding = self
            .tokenizer
            .encode(EncodeInput::Dual(query.into(), document.into()), true)
            .map_err(|error| format!("tokenize failed: {error}"))?;
        let ids = encoding.get_ids();
        let attention = encoding.get_attention_mask();
        let type_ids = encoding.get_type_ids();
        let len = ids.len().min(self.max_input_tokens);
        if len == 0 {
            return Err("empty tokenized pair".to_string());
        }

        let shape = vec![1i64, len as i64];
        let ids_tensor = Tensor::from_array((
            shape.clone(),
            ids[..len]
                .iter()
                .map(|value| *value as i64)
                .collect::<Vec<_>>(),
        ))
        .map_err(|error| format!("input_ids tensor failed: {error}"))?;
        let mask_tensor = Tensor::from_array((
            shape.clone(),
            attention[..len]
                .iter()
                .map(|value| *value as i64)
                .collect::<Vec<_>>(),
        ))
        .map_err(|error| format!("attention_mask tensor failed: {error}"))?;
        let type_tensor = Tensor::from_array((
            shape,
            type_ids[..len]
                .iter()
                .map(|value| *value as i64)
                .collect::<Vec<_>>(),
        ))
        .map_err(|error| format!("token_type_ids tensor failed: {error}"))?;

        let mut session = self
            .session
            .lock()
            .map_err(|_| "reranker session lock poisoned".to_string())?;
        let outputs = session
            .run(ort::inputs![
                "input_ids" => ids_tensor,
                "attention_mask" => mask_tensor,
                "token_type_ids" => type_tensor,
            ])
            .map_err(|error| format!("reranker inference failed: {error}"))?;
        let (_shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|error| format!("reranker output extraction failed: {error}"))?;
        data.first()
            .copied()
            .filter(|score| score.is_finite())
            .ok_or_else(|| "reranker output missing finite score".to_string())
    }
}

impl Reranker for MiniLmReranker {
    fn name(&self) -> &'static str {
        "cross_encoder_minilm_l6_v2"
    }

    fn model_size_mb(&self) -> u64 {
        selected_profile().model_size_mb
    }

    fn rerank(
        &self,
        query: &str,
        candidates: &[RerankCandidate],
        fusion_alpha: f64,
    ) -> Result<Vec<RerankedScore>, String> {
        let mut raw_scores = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let score = self.score_pair(query, &candidate.text)?;
            raw_scores.push((candidate.id.clone(), score));
        }
        Ok(fuse_scores(candidates, &raw_scores, fusion_alpha))
    }
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
                "[rerank] Falling back to default ORT session threading after tuned setup failed: {tuned_error}"
            );
            Ok(fallback)
        }
    }
}

pub fn fuse_scores(
    candidates: &[RerankCandidate],
    raw_scores: &[(String, f32)],
    fusion_alpha: f64,
) -> Vec<RerankedScore> {
    let alpha = fusion_alpha.clamp(0.0, 1.0);
    let raw_by_id = raw_scores
        .iter()
        .map(|(id, score)| (id.as_str(), *score as f64))
        .collect::<std::collections::HashMap<_, _>>();
    let base_values = candidates
        .iter()
        .map(|candidate| candidate.base_score)
        .collect::<Vec<_>>();
    let rerank_values = candidates
        .iter()
        .map(|candidate| raw_by_id.get(candidate.id.as_str()).copied().unwrap_or(0.0))
        .collect::<Vec<_>>();
    let (base_min, base_max) = min_max(&base_values);
    let (rerank_min, rerank_max) = min_max(&rerank_values);

    let mut fused = candidates
        .iter()
        .enumerate()
        .map(|(idx, candidate)| {
            let rerank_score = raw_by_id.get(candidate.id.as_str()).copied().unwrap_or(0.0);
            let base_norm = normalize(candidate.base_score, base_min, base_max);
            let rerank_norm = normalize(rerank_score, rerank_min, rerank_max);
            let fused_score = ((1.0 - alpha) * base_norm) + (alpha * rerank_norm);
            (
                idx,
                RerankedScore {
                    id: candidate.id.clone(),
                    base_score: candidate.base_score,
                    rerank_score,
                    fused_score,
                },
            )
        })
        .collect::<Vec<_>>();
    fused.sort_by(|(left_idx, left), (right_idx, right)| {
        right
            .fused_score
            .partial_cmp(&left.fused_score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left_idx.cmp(right_idx))
    });
    fused.into_iter().map(|(_, score)| score).collect()
}

fn min_max(values: &[f64]) -> (f64, f64) {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for value in values.iter().copied().filter(|value| value.is_finite()) {
        min = min.min(value);
        max = max.max(value);
    }
    if min.is_finite() && max.is_finite() {
        (min, max)
    } else {
        (0.0, 0.0)
    }
}

fn normalize(value: f64, min: f64, max: f64) -> f64 {
    let span = max - min;
    if span.abs() < f64::EPSILON {
        1.0
    } else {
        ((value - min) / span).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_off_and_bounded() {
        let config = RerankConfig::off();
        assert_eq!(config.mode, RerankMode::Off);
        assert!(!config.is_active());
        assert_eq!(config.top_n, DEFAULT_TOP_N);
    }

    #[test]
    fn fuse_scores_can_promote_cross_encoder_winner() {
        let candidates = vec![
            RerankCandidate {
                id: "a".to_string(),
                text: "weak".to_string(),
                base_score: 0.95,
            },
            RerankCandidate {
                id: "b".to_string(),
                text: "strong".to_string(),
                base_score: 0.70,
            },
        ];
        let fused = fuse_scores(
            &candidates,
            &[("a".to_string(), -4.0), ("b".to_string(), 8.0)],
            0.80,
        );
        assert_eq!(fused[0].id, "b");
        assert!(fused[0].fused_score > fused[1].fused_score);
    }

    #[test]
    fn noop_preserves_base_order() {
        let candidates = vec![
            RerankCandidate {
                id: "a".to_string(),
                text: "first".to_string(),
                base_score: 0.9,
            },
            RerankCandidate {
                id: "b".to_string(),
                text: "second".to_string(),
                base_score: 0.7,
            },
        ];
        let reranked = NoopReranker
            .rerank("query", &candidates, DEFAULT_FUSION_ALPHA)
            .unwrap();
        assert_eq!(reranked[0].id, "a");
        assert_eq!(reranked[1].id, "b");
    }

    #[test]
    fn real_minilm_model_loads_and_scores_when_enabled() {
        if std::env::var("CORTEX_RERANK_REAL_MODEL_SMOKE")
            .ok()
            .as_deref()
            != Some("1")
        {
            eprintln!("skipping real reranker smoke; set CORTEX_RERANK_REAL_MODEL_SMOKE=1");
            return;
        }
        let models_dir = std::env::var("CORTEX_RERANK_REAL_MODEL_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                dirs::home_dir()
                    .expect("home dir should resolve")
                    .join(".cortex")
                    .join("models")
            });
        let reranker = MiniLmReranker::load(&models_dir).expect("real reranker assets should load");
        let candidates = vec![
            RerankCandidate {
                id: "relevant".to_string(),
                text: "Paris is the capital city of France.".to_string(),
                base_score: 0.5,
            },
            RerankCandidate {
                id: "irrelevant".to_string(),
                text: "A banana ripens from green to yellow.".to_string(),
                base_score: 0.5,
            },
        ];
        let scored = reranker
            .rerank("What is the capital of France?", &candidates, 1.0)
            .expect("real reranker inference should score candidates");
        assert_eq!(scored[0].id, "relevant");
        assert!(
            scored[0].rerank_score > scored[1].rerank_score,
            "relevant candidate should score higher: {scored:?}"
        );
    }
}
