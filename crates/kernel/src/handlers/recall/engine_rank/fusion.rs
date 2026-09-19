use super::*;
use rustc_hash::FxHashMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FusionWeights {
    pub keyword: f64,
    pub semantic: f64,
}
pub fn adaptive_rrf_weights(
    query_text: &str,
    source_prefix: Option<&str>,
    semantic_available: bool,
) -> FusionWeights {
    if !semantic_available {
        return FusionWeights {
            keyword: 1.0,
            semantic: 0.0,
        };
    }
    let profile = query_shape_profile(query_text, source_prefix);
    let mut keyword = 1.0_f64;
    let mut semantic = 1.0_f64;
    if profile.exactish {
        keyword += 0.35;
        semantic -= 0.15;
    }
    if profile.naturalish {
        semantic += 0.35;
        keyword -= 0.15;
    }
    FusionWeights {
        keyword: keyword.clamp(0.35, 1.75),
        semantic: semantic.clamp(0.35, 1.75),
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FallbackRankingWeights {
    pub keyword: f64,
    pub score: f64,
    pub recency: f64,
    pub retrieval: f64,
}
pub fn adaptive_fallback_ranking_weights(
    query_text: &str,
    term_group_count: usize,
) -> FallbackRankingWeights {
    let profile = query_shape_profile(query_text, None);
    let mut keyword = 0.40_f64;
    let mut score = 0.25_f64;
    let mut recency = 0.20_f64;
    let mut retrieval = 0.15_f64;
    if profile.exactish && !profile.naturalish {
        keyword += 0.12;
        score -= 0.03;
        recency -= 0.05;
        retrieval -= 0.04;
    } else if profile.naturalish && !profile.exactish {
        keyword -= 0.08;
        score += 0.05;
        recency += 0.02;
        retrieval += 0.01;
    }
    if term_group_count <= 1 {
        keyword += 0.05;
        score += 0.01;
        recency -= 0.03;
        retrieval -= 0.03;
    } else if term_group_count >= 5 {
        keyword -= 0.04;
        score += 0.02;
        recency += 0.01;
        retrieval += 0.01;
    }
    keyword = keyword.max(0.05);
    score = score.max(0.05);
    recency = recency.max(0.05);
    retrieval = retrieval.max(0.05);
    let total = keyword + score + recency + retrieval;
    FallbackRankingWeights {
        keyword: keyword / total,
        score: score / total,
        recency: recency / total,
        retrieval: retrieval / total,
    }
}
pub fn fallback_ranking_score(
    query_text: &str,
    term_group_count: usize,
    matched: i64,
    effective_score: f64,
    recency_days: i64,
    retrievals: Option<i64>,
) -> f64 {
    let keyword_weight = if term_group_count == 0 {
        0.0
    } else {
        matched as f64 / term_group_count as f64
    };
    let recency_weight = 1.0 / (1.0 + recency_days.max(0) as f64 / 7.0);
    let retrieval_weight = (retrievals.unwrap_or(0).clamp(0, 20) as f64) / 20.0;
    let score_weight = effective_score.clamp(0.0, 1.0);
    let weights = adaptive_fallback_ranking_weights(query_text, term_group_count);
    (keyword_weight * weights.keyword)
        + (score_weight * weights.score)
        + (recency_weight * weights.recency)
        + (retrieval_weight * weights.retrieval)
}
pub fn rrf_fuse_weighted(lists: &[Vec<(i64, f64)>], weights: &[f64], k: f64) -> Vec<(i64, f64)> {
    let smooth_k = if k.is_finite() && k >= 0.0 { k } else { 60.0 };
    let fused_cap = lists.iter().map(|list| list.len()).sum();
    let mut fused: FxHashMap<i64, f64> =
        FxHashMap::with_capacity_and_hasher(fused_cap, Default::default());
    for (list_index, list) in lists.iter().enumerate() {
        let weight = weights
            .get(list_index)
            .copied()
            .map(|value| {
                if value.is_finite() {
                    value.max(0.0)
                } else {
                    0.0
                }
            })
            .unwrap_or(1.0);
        if weight == 0.0 {
            continue;
        }
        for (rank, &(id, _score)) in list.iter().enumerate() {
            *fused.entry(id).or_insert(0.0) += weight / (smooth_k + rank as f64 + 1.0);
        }
    }
    let mut result: Vec<(i64, f64)> = fused.into_iter().collect();
    result.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    result
}
pub fn days_since(created_at: &str) -> f64 {
    chrono::DateTime::parse_from_rfc3339(created_at)
        .map(|dt| {
            let duration = chrono::Utc::now().signed_duration_since(dt);
            duration.num_days() as f64 + (duration.num_seconds() as f64 % 86400.0) / 86400.0
        })
        .unwrap_or(f64::MAX)
}
pub fn normalize(importance: f64) -> f64 {
    if !importance.is_finite() {
        return 0.0;
    }
    let clamped = importance.clamp(0.0, 100.0);
    if clamped <= 1.0 {
        clamped
    } else {
        clamped / 100.0
    }
}
pub fn compound_score(rrf: f64, importance: f64, created_at: &str) -> f64 {
    let days = days_since(created_at);
    let recency = (-days / 30.0).exp();
    let importance_normalized = normalize(importance);
    rrf * 0.6 + importance_normalized * 0.2 + recency * 0.2
}
