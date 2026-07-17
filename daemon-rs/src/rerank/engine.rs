use super::assets::selected_profile;
use ort::session::Session;
use ort::value::Tensor;
use std::cmp::Ordering;
use std::path::Path;
use std::sync::Mutex;
use tokenizers::{EncodeInput, Tokenizer};
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
    fn rerank(&self, query: &str, candidates: &[RerankCandidate], fusion_alpha: f64) -> Result<Vec<RerankedScore>, String>;
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
    fn rerank(&self, _query: &str, candidates: &[RerankCandidate], fusion_alpha: f64) -> Result<Vec<RerankedScore>, String> {
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
            let missing = missing.iter().map(|asset| asset.file).collect::<Vec<_>>().join(", ");
            return Err(format!("model assets missing ({missing}) at {}", models_dir.display()));
        }
        let model_path = models_dir.join(profile.model_file);
        let tokenizer_path = models_dir.join(profile.tokenizer_file);
        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|error| format!("failed to load tokenizer {}: {error}", tokenizer_path.display()))?;
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
        let ids_tensor = Tensor::from_array((shape.clone(), ids[..len].iter().map(|value| *value as i64).collect::<Vec<_>>()))
            .map_err(|error| format!("input_ids tensor failed: {error}"))?;
        let mask_tensor = Tensor::from_array((shape.clone(), attention[..len].iter().map(|value| *value as i64).collect::<Vec<_>>()))
            .map_err(|error| format!("attention_mask tensor failed: {error}"))?;
        let type_tensor = Tensor::from_array((shape, type_ids[..len].iter().map(|value| *value as i64).collect::<Vec<_>>()))
            .map_err(|error| format!("token_type_ids tensor failed: {error}"))?;
        let mut session = self.session.lock().map_err(|_| "reranker session lock poisoned".to_string())?;
        let outputs = session
            .run(ort::inputs!["input_ids"=>
ids_tensor,"attention_mask"=>mask_tensor,"token_type_ids"=>type_tensor,])
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
    fn rerank(&self, query: &str, candidates: &[RerankCandidate], fusion_alpha: f64) -> Result<Vec<RerankedScore>, String> {
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
        .and_then(|builder| builder.with_intra_threads(2).map_err(|error| format!("with_intra_threads(2) failed: {error}")))
        .and_then(|mut builder| {
            builder
                .commit_from_file(model_path)
                .map_err(|error| format!("commit_from_file (tuned threads) failed for {}: {error}", model_path.display()))
        });
    match tuned {
        Ok(session) => Ok(session),
        Err(tuned_error) => {
            let fallback = Session::builder()
                .map_err(|error| format!("session builder fallback init failed: {error}"))?
                .commit_from_file(model_path)
                .map_err(|error| format!("commit_from_file (fallback threads) failed for {}: {error}", model_path.display()))?;
            eprintln!("[rerank] Falling back to default ORT session threading after tuned setup failed: {tuned_error}");
            Ok(fallback)
        }
    }
}
pub fn fuse_scores(candidates: &[RerankCandidate], raw_scores: &[(String, f32)], fusion_alpha: f64) -> Vec<RerankedScore> {
    let alpha = fusion_alpha.clamp(0.0, 1.0);
    let raw_by_id = raw_scores
        .iter()
        .map(|(id, score)| (id.as_str(), *score as f64))
        .collect::<std::collections::HashMap<_, _>>();
    let base_values = candidates.iter().map(|candidate| candidate.base_score).collect::<Vec<_>>();
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
