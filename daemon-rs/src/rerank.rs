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
    if !value.is_finite() {
        return 0.0;
    }
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
    use proptest::prelude::*;
    use std::collections::HashMap;

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

    #[derive(Clone, Debug)]
    struct FusionCase {
        candidates: Vec<RerankCandidate>,
        raw_scores: Vec<(String, f32)>,
        alpha: f64,
    }

    struct Lcg(u64);

    impl Lcg {
        fn new(seed: u64) -> Self {
            Self(seed)
        }

        fn next_u32(&mut self) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 32) as u32
        }

        fn next_unit(&mut self) -> f64 {
            self.next_u32() as f64 / u32::MAX as f64
        }

        fn next_score(&mut self) -> f64 {
            ((self.next_unit() * 200.0) - 100.0) + (self.next_unit() * 0.0001)
        }
    }

    fn generated_fusion_cases() -> Vec<FusionCase> {
        let mut rng = Lcg::new(0xC057_A7E5);
        (0..96)
            .map(|case_idx| {
                let len = 3 + (rng.next_u32() as usize % 8);
                let mut candidates = Vec::with_capacity(len);
                let mut raw_scores = Vec::with_capacity(len);
                for idx in 0..len {
                    let id = format!("c{case_idx}_{idx}");
                    let base_score = rng.next_score() + idx as f64 * 0.01;
                    let rerank_score = (rng.next_score() + idx as f64 * 0.01) as f32;
                    candidates.push(RerankCandidate {
                        id: id.clone(),
                        text: format!("candidate {case_idx} {idx}"),
                        base_score,
                    });
                    raw_scores.push((id, rerank_score));
                }
                let alpha = 0.05 + (rng.next_unit() * 0.90);
                FusionCase {
                    candidates,
                    raw_scores,
                    alpha,
                }
            })
            .collect()
    }

    fn fusion_case_strategy() -> impl Strategy<Value = FusionCase> {
        (
            proptest::collection::vec(
                (
                    -1_000_000.0f64..1_000_000.0f64,
                    -1_000_000.0f32..1_000_000.0f32,
                ),
                3..10,
            ),
            0.05f64..0.95f64,
        )
            .prop_map(|(scores, alpha)| {
                let mut candidates = Vec::with_capacity(scores.len());
                let mut raw_scores = Vec::with_capacity(scores.len());
                for (idx, (base_score, rerank_score)) in scores.into_iter().enumerate() {
                    let id = format!("p{idx}");
                    candidates.push(RerankCandidate {
                        id: id.clone(),
                        text: format!("property candidate {idx}"),
                        base_score: base_score + idx as f64 * 0.0001,
                    });
                    raw_scores.push((id, rerank_score + idx as f32 * 0.0001));
                }
                FusionCase {
                    candidates,
                    raw_scores,
                    alpha,
                }
            })
    }

    fn ids(scores: &[RerankedScore]) -> Vec<String> {
        scores.iter().map(|score| score.id.clone()).collect()
    }

    fn fused_by_id(scores: &[RerankedScore]) -> HashMap<String, f64> {
        scores
            .iter()
            .map(|score| (score.id.clone(), score.fused_score))
            .collect()
    }

    fn prop_assert_same_fused_scores(
        left: &[RerankedScore],
        right: &[RerankedScore],
    ) -> Result<(), TestCaseError> {
        prop_assert_eq!(left.len(), right.len());
        let right_by_id = fused_by_id(right);
        for left_score in left {
            let right_score = right_by_id
                .get(&left_score.id)
                .unwrap_or_else(|| panic!("missing fused score for {}", left_score.id));
            prop_assert!(
                (left_score.fused_score - right_score).abs() <= 1e-6,
                "fused score changed for {}: {} vs {}",
                left_score.id,
                left_score.fused_score,
                right_score
            );
        }
        Ok(())
    }

    fn prop_assert_unique_rank_scores(scores: &[RerankedScore]) -> Result<(), TestCaseError> {
        for left in 0..scores.len() {
            for right in (left + 1)..scores.len() {
                prop_assert!(
                    (scores[left].fused_score - scores[right].fused_score).abs() > 1e-10,
                    "generated case produced tied fused scores: {scores:?}"
                );
            }
        }
        Ok(())
    }

    fn shifted_case(case: &FusionCase, base_delta: f64, rerank_delta: f32) -> FusionCase {
        let candidates = case
            .candidates
            .iter()
            .map(|candidate| RerankCandidate {
                id: candidate.id.clone(),
                text: candidate.text.clone(),
                base_score: candidate.base_score + base_delta,
            })
            .collect();
        let raw_scores = case
            .raw_scores
            .iter()
            .map(|(id, score)| (id.clone(), *score + rerank_delta))
            .collect();
        FusionCase {
            candidates,
            raw_scores,
            alpha: case.alpha,
        }
    }

    fn scaled_case(case: &FusionCase, base_factor: f64, rerank_factor: f32) -> FusionCase {
        let candidates = case
            .candidates
            .iter()
            .map(|candidate| RerankCandidate {
                id: candidate.id.clone(),
                text: candidate.text.clone(),
                base_score: candidate.base_score * base_factor,
            })
            .collect();
        let raw_scores = case
            .raw_scores
            .iter()
            .map(|(id, score)| (id.clone(), *score * rerank_factor))
            .collect();
        FusionCase {
            candidates,
            raw_scores,
            alpha: case.alpha,
        }
    }

    fn swapped_score_sources(case: &FusionCase) -> FusionCase {
        let raw_by_id = case
            .raw_scores
            .iter()
            .map(|(id, score)| (id.as_str(), *score))
            .collect::<HashMap<_, _>>();
        let candidates = case
            .candidates
            .iter()
            .map(|candidate| RerankCandidate {
                id: candidate.id.clone(),
                text: candidate.text.clone(),
                base_score: raw_by_id[&candidate.id.as_str()] as f64,
            })
            .collect::<Vec<_>>();
        let raw_scores = case
            .candidates
            .iter()
            .map(|candidate| (candidate.id.clone(), candidate.base_score as f32))
            .collect();
        FusionCase {
            candidates,
            raw_scores,
            alpha: 1.0 - case.alpha,
        }
    }

    fn reverse_candidates(case: &FusionCase) -> FusionCase {
        let mut candidates = case.candidates.clone();
        candidates.reverse();
        FusionCase {
            candidates,
            raw_scores: case.raw_scores.clone(),
            alpha: case.alpha,
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 128,
            failure_persistence: None,
            .. ProptestConfig::default()
        })]

        #[test]
        fn mr_fuse_scores_additive_shift_invariance(case in fusion_case_strategy()) {
            let original = fuse_scores(&case.candidates, &case.raw_scores, case.alpha);
            let shifted = shifted_case(&case, 317.25, -49.5);
            let shifted_scores =
                fuse_scores(&shifted.candidates, &shifted.raw_scores, shifted.alpha);
            prop_assert_eq!(ids(&original), ids(&shifted_scores));
            prop_assert_same_fused_scores(&original, &shifted_scores)?;
        }

        #[test]
        fn mr_fuse_scores_positive_scale_invariance(case in fusion_case_strategy()) {
            let original = fuse_scores(&case.candidates, &case.raw_scores, case.alpha);
            let scaled = scaled_case(&case, 11.0, 0.25);
            let scaled_scores = fuse_scores(&scaled.candidates, &scaled.raw_scores, scaled.alpha);
            prop_assert_eq!(ids(&original), ids(&scaled_scores));
            prop_assert_same_fused_scores(&original, &scaled_scores)?;
        }

        #[test]
        fn mr_fuse_scores_candidate_permutation_preserves_unique_rankings(case in fusion_case_strategy()) {
            let original = fuse_scores(&case.candidates, &case.raw_scores, case.alpha);
            prop_assert_unique_rank_scores(&original)?;
            let reversed = reverse_candidates(&case);
            let reversed_scores =
                fuse_scores(&reversed.candidates, &reversed.raw_scores, reversed.alpha);
            prop_assert_eq!(ids(&original), ids(&reversed_scores));
            prop_assert_same_fused_scores(&original, &reversed_scores)?;
        }

        #[test]
        fn mr_fuse_scores_alpha_endpoints_exclude_irrelevant_signal(case in fusion_case_strategy()) {
            let mut base_only = case.clone();
            base_only.alpha = 0.0;
            let base_original = fuse_scores(&base_only.candidates, &base_only.raw_scores, 0.0);
            let raw_shifted = shifted_case(&base_only, 0.0, 500.0);
            let base_after_raw_shift =
                fuse_scores(&raw_shifted.candidates, &raw_shifted.raw_scores, 0.0);
            prop_assert_same_fused_scores(&base_original, &base_after_raw_shift)?;

            let mut rerank_only = case.clone();
            rerank_only.alpha = 1.0;
            let rerank_original =
                fuse_scores(&rerank_only.candidates, &rerank_only.raw_scores, 1.0);
            let base_shifted = shifted_case(&rerank_only, -500.0, 0.0);
            let rerank_after_base_shift =
                fuse_scores(&base_shifted.candidates, &base_shifted.raw_scores, 1.0);
            prop_assert_same_fused_scores(&rerank_original, &rerank_after_base_shift)?;
        }

        #[test]
        fn mr_fuse_scores_base_rerank_swap_alpha_complement(case in fusion_case_strategy()) {
            let original = fuse_scores(&case.candidates, &case.raw_scores, case.alpha);
            let swapped = swapped_score_sources(&case);
            let swapped_scores =
                fuse_scores(&swapped.candidates, &swapped.raw_scores, swapped.alpha);
            prop_assert_eq!(ids(&original), ids(&swapped_scores));
            prop_assert_same_fused_scores(&original, &swapped_scores)?;
        }

        #[test]
        fn mr_fuse_scores_composes_permutation_shift_and_scale(case in fusion_case_strategy()) {
            let original = fuse_scores(&case.candidates, &case.raw_scores, case.alpha);
            let composed = scaled_case(
                &shifted_case(&reverse_candidates(&case), 42.0, -9.0),
                3.0,
                2.0,
            );
            let composed_scores =
                fuse_scores(&composed.candidates, &composed.raw_scores, composed.alpha);
            prop_assert_eq!(ids(&original), ids(&composed_scores));
            prop_assert_same_fused_scores(&original, &composed_scores)?;
        }
    }

    #[test]
    fn fuse_scores_sanitizes_non_finite_inputs() {
        let candidates = vec![
            RerankCandidate {
                id: "finite".to_string(),
                text: "finite".to_string(),
                base_score: 0.5,
            },
            RerankCandidate {
                id: "nan_base".to_string(),
                text: "nan base".to_string(),
                base_score: f64::NAN,
            },
        ];
        let scores = fuse_scores(
            &candidates,
            &[
                ("finite".to_string(), 1.0),
                ("nan_base".to_string(), f32::NAN),
            ],
            0.5,
        );
        assert!(
            scores.iter().all(|score| score.fused_score.is_finite()),
            "non-finite inputs must not produce non-finite fused scores: {scores:?}"
        );
    }

    type FusionFn = fn(&[RerankCandidate], &[(String, f32)], f64) -> Vec<RerankedScore>;

    fn mutant_base_shift_sensitive(
        candidates: &[RerankCandidate],
        raw_scores: &[(String, f32)],
        fusion_alpha: f64,
    ) -> Vec<RerankedScore> {
        mutant_fuse_scores(
            candidates,
            raw_scores,
            fusion_alpha,
            true,
            false,
            false,
            false,
        )
    }

    fn mutant_rerank_scale_sensitive(
        candidates: &[RerankCandidate],
        raw_scores: &[(String, f32)],
        fusion_alpha: f64,
    ) -> Vec<RerankedScore> {
        mutant_fuse_scores(
            candidates,
            raw_scores,
            fusion_alpha,
            false,
            true,
            false,
            false,
        )
    }

    fn mutant_input_order_biased(
        candidates: &[RerankCandidate],
        raw_scores: &[(String, f32)],
        fusion_alpha: f64,
    ) -> Vec<RerankedScore> {
        mutant_fuse_scores(
            candidates,
            raw_scores,
            fusion_alpha,
            false,
            false,
            true,
            false,
        )
    }

    fn mutant_ignores_alpha(
        candidates: &[RerankCandidate],
        raw_scores: &[(String, f32)],
        _fusion_alpha: f64,
    ) -> Vec<RerankedScore> {
        mutant_fuse_scores(
            candidates,
            raw_scores,
            DEFAULT_FUSION_ALPHA,
            false,
            false,
            false,
            false,
        )
    }

    fn mutant_non_complementary_alpha_weights(
        candidates: &[RerankCandidate],
        raw_scores: &[(String, f32)],
        fusion_alpha: f64,
    ) -> Vec<RerankedScore> {
        mutant_fuse_scores(
            candidates,
            raw_scores,
            fusion_alpha,
            false,
            false,
            false,
            true,
        )
    }

    fn mutant_fuse_scores(
        candidates: &[RerankCandidate],
        raw_scores: &[(String, f32)],
        fusion_alpha: f64,
        use_raw_base: bool,
        use_raw_rerank: bool,
        use_order_bias: bool,
        use_alpha_for_both_weights: bool,
    ) -> Vec<RerankedScore> {
        let alpha = fusion_alpha.clamp(0.0, 1.0);
        let raw_by_id = raw_scores
            .iter()
            .map(|(id, score)| (id.as_str(), *score as f64))
            .collect::<HashMap<_, _>>();
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
                let base_component = if use_raw_base {
                    candidate.base_score
                } else {
                    normalize(candidate.base_score, base_min, base_max)
                };
                let rerank_component = if use_raw_rerank {
                    rerank_score
                } else {
                    normalize(rerank_score, rerank_min, rerank_max)
                };
                let order_bias = if use_order_bias {
                    idx as f64 * 0.01
                } else {
                    0.0
                };
                let base_weight = if use_alpha_for_both_weights {
                    alpha
                } else {
                    1.0 - alpha
                };
                let fused_score =
                    (base_weight * base_component) + (alpha * rerank_component) + order_bias;
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

    fn additive_shift_mr_detects(fuse: FusionFn) -> bool {
        generated_fusion_cases().into_iter().any(|case| {
            let original = fuse(&case.candidates, &case.raw_scores, case.alpha);
            let shifted = shifted_case(&case, 317.25, -49.5);
            let shifted_scores = fuse(&shifted.candidates, &shifted.raw_scores, shifted.alpha);
            ids(&original) != ids(&shifted_scores)
                || fused_by_id(&original) != fused_by_id(&shifted_scores)
        })
    }

    fn positive_scale_mr_detects(fuse: FusionFn) -> bool {
        generated_fusion_cases().into_iter().any(|case| {
            let original = fuse(&case.candidates, &case.raw_scores, case.alpha);
            let scaled = scaled_case(&case, 11.0, 0.25);
            let scaled_scores = fuse(&scaled.candidates, &scaled.raw_scores, scaled.alpha);
            ids(&original) != ids(&scaled_scores)
                || fused_by_id(&original) != fused_by_id(&scaled_scores)
        })
    }

    fn permutation_mr_detects(fuse: FusionFn) -> bool {
        generated_fusion_cases().into_iter().any(|case| {
            let original = fuse(&case.candidates, &case.raw_scores, case.alpha);
            let reversed = reverse_candidates(&case);
            let reversed_scores = fuse(&reversed.candidates, &reversed.raw_scores, reversed.alpha);
            ids(&original) != ids(&reversed_scores)
                || fused_by_id(&original) != fused_by_id(&reversed_scores)
        })
    }

    fn alpha_endpoint_mr_detects(fuse: FusionFn) -> bool {
        generated_fusion_cases().into_iter().any(|case| {
            let mut base_only = case.clone();
            base_only.alpha = 0.0;
            let base_original = fuse(&base_only.candidates, &base_only.raw_scores, 0.0);
            let raw_shifted = shifted_case(&base_only, 0.0, 500.0);
            let base_after_raw_shift = fuse(&raw_shifted.candidates, &raw_shifted.raw_scores, 0.0);
            if fused_by_id(&base_original) != fused_by_id(&base_after_raw_shift) {
                return true;
            }

            let mut rerank_only = case.clone();
            rerank_only.alpha = 1.0;
            let rerank_original = fuse(&rerank_only.candidates, &rerank_only.raw_scores, 1.0);
            let base_shifted = shifted_case(&rerank_only, -500.0, 0.0);
            let rerank_after_base_shift =
                fuse(&base_shifted.candidates, &base_shifted.raw_scores, 1.0);
            fused_by_id(&rerank_original) != fused_by_id(&rerank_after_base_shift)
        })
    }

    fn swap_alpha_complement_mr_detects(fuse: FusionFn) -> bool {
        generated_fusion_cases().into_iter().any(|case| {
            let original = fuse(&case.candidates, &case.raw_scores, case.alpha);
            let swapped = swapped_score_sources(&case);
            let swapped_scores = fuse(&swapped.candidates, &swapped.raw_scores, swapped.alpha);
            ids(&original) != ids(&swapped_scores)
                || fused_by_id(&original) != fused_by_id(&swapped_scores)
        })
    }

    fn composite_mr_detects(fuse: FusionFn) -> bool {
        generated_fusion_cases().into_iter().any(|case| {
            let original = fuse(&case.candidates, &case.raw_scores, case.alpha);
            let composed = scaled_case(
                &shifted_case(&reverse_candidates(&case), 42.0, -9.0),
                3.0,
                2.0,
            );
            let composed_scores = fuse(&composed.candidates, &composed.raw_scores, composed.alpha);
            ids(&original) != ids(&composed_scores)
                || fused_by_id(&original) != fused_by_id(&composed_scores)
        })
    }

    #[test]
    fn validate_mr_suite_catches_planted_fusion_mutations() {
        assert!(additive_shift_mr_detects(mutant_base_shift_sensitive));
        assert!(positive_scale_mr_detects(mutant_rerank_scale_sensitive));
        assert!(permutation_mr_detects(mutant_input_order_biased));
        assert!(alpha_endpoint_mr_detects(mutant_ignores_alpha));
        assert!(swap_alpha_complement_mr_detects(
            mutant_non_complementary_alpha_weights
        ));
        assert!(composite_mr_detects(mutant_base_shift_sensitive));
        assert!(composite_mr_detects(mutant_rerank_scale_sensitive));
        assert!(composite_mr_detects(mutant_input_order_biased));
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
