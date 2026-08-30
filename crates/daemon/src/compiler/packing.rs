use super::*;
use crate::handlers::estimate_tokens;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::env;
pub(crate) fn read_usize_env(name: &str, default: usize) -> usize {
    env::var(name).ok().and_then(|value| value.trim().parse::<usize>().ok()).filter(|value| *value > 0).unwrap_or(default)
}
pub(crate) fn boot_source_token_bounds() -> SourceTokenBounds {
    SourceTokenBounds::new(
        read_usize_env("CORTEX_BOOT_MIN_SOURCE_TOKENS", DEFAULT_BOOT_MIN_SOURCE_TOKENS),
        read_usize_env("CORTEX_BOOT_MAX_SOURCE_TOKENS", DEFAULT_BOOT_MAX_SOURCE_TOKENS),
    )
}
pub(crate) fn boot_rank_top_n() -> usize {
    read_usize_env("CORTEX_BOOT_RANK_TOP_N", DEFAULT_BOOT_RANK_TOP_N).min(20)
}
pub(crate) fn empty_rank_components() -> RankComponents {
    RankComponents { class_score: 0.0, recency_score: 0.0, relevance_score: 0.0, activity_score: 0.0, total_score: 0.0 }
}
pub(crate) fn fetch_rank_candidates(conn: &Connection) -> Vec<RankedCandidate> {
    let mut candidates = Vec::new();
    if let Ok(mut stmt) = conn.prepare_cached(
        "SELECT id, text, retention_class, score, retrievals, last_accessed, updated_at, created_at, status, confirmed_by,
                COALESCE(valid_from, observed_at, created_at), valid_until
         FROM memories
         WHERE status = 'active' AND type != 'state'
           AND (expires_at IS NULL OR julianday(expires_at) > julianday('now'))
           AND (valid_from IS NULL OR julianday(valid_from) <= julianday('now'))
           AND (valid_until IS NULL OR julianday(valid_until) > julianday('now'))
           AND (version_id IS NULL OR version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned'))
         ORDER BY updated_at DESC, id DESC
         LIMIT 80",
    ) {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok(RankedCandidate {
                source_kind: "memory",
                source_id: row.get::<_, i64>(0)?,
                body: row.get::<_, String>(1)?,
                retention_class: row.get::<_, Option<String>>(2)?.unwrap_or_else(|| "operational".to_string()),
                relevance: row.get::<_, Option<f64>>(3)?.unwrap_or(0.5),
                retrievals: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
                last_accessed: row.get::<_, Option<String>>(5)?,
                updated_at: row.get::<_, Option<String>>(6)?,
                created_at: row.get::<_, Option<String>>(7)?,
                status: row.get::<_, Option<String>>(8)?.unwrap_or_else(|| "active".to_string()),
                confirmed_by: row.get(9)?,
                valid_from: row.get(10)?,
                valid_until: row.get(11)?,
                components: empty_rank_components(),
            })
        }) {
            candidates.extend(rows.flatten());
        }
    }
    if let Ok(mut stmt) = conn.prepare_cached(
        "SELECT id, decision, context, retention_class, score, retrievals, last_accessed, updated_at, created_at, status, confirmed_by,
                COALESCE(valid_from, observed_at, created_at), valid_until
         FROM decisions
         WHERE status = 'active'
           AND (expires_at IS NULL OR julianday(expires_at) > julianday('now'))
           AND (valid_from IS NULL OR julianday(valid_from) <= julianday('now'))
           AND (valid_until IS NULL OR julianday(valid_until) > julianday('now'))
           AND (version_id IS NULL OR version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned'))
         ORDER BY updated_at DESC, id DESC
         LIMIT 80",
    ) {
        if let Ok(rows) = stmt.query_map([], |row| {
            let decision: String = row.get(1)?;
            let context: Option<String> = row.get(2)?;
            let body = match context {
                Some(context) if !context.trim().is_empty() => format!("{decision} ({context})"),
                _ => decision,
            };
            Ok(RankedCandidate {
                source_kind: "decision",
                source_id: row.get::<_, i64>(0)?,
                body,
                retention_class: row.get::<_, Option<String>>(3)?.unwrap_or_else(|| "operational".to_string()),
                relevance: row.get::<_, Option<f64>>(4)?.unwrap_or(0.5),
                retrievals: row.get::<_, Option<i64>>(5)?.unwrap_or(0),
                last_accessed: row.get::<_, Option<String>>(6)?,
                updated_at: row.get::<_, Option<String>>(7)?,
                created_at: row.get::<_, Option<String>>(8)?,
                status: row.get::<_, Option<String>>(9)?.unwrap_or_else(|| "active".to_string()),
                confirmed_by: row.get(10)?,
                valid_from: row.get(11)?,
                valid_until: row.get(12)?,
                components: empty_rank_components(),
            })
        }) {
            candidates.extend(rows.flatten());
        }
    }
    candidates
}
pub(crate) fn score_signal_is_flat(items: &[ContextItem]) -> bool {
    let mut count = 0usize;
    let mut sum = 0.0;
    for item in items.iter().filter(|item| !item.text.is_empty()) {
        count += 1;
        sum += item.priority;
    }
    if count <= 1 {
        return true;
    }
    let mean = sum / count as f64;
    let variance = items
        .iter()
        .filter(|item| !item.text.is_empty())
        .map(|item| {
            let delta = item.priority - mean;
            delta * delta
        })
        .sum::<f64>()
        / count as f64;
    variance < SCORE_VARIANCE_FLAT_THRESHOLD
}
pub(crate) fn truncate_to_token_budget(text: &str, token_budget: usize) -> (String, usize) {
    if token_budget == 0 {
        return (String::new(), 0);
    }
    let total_tokens = estimate_tokens(text);
    if total_tokens <= token_budget {
        return (text.to_string(), total_tokens);
    }
    let total_chars = text.chars().count();
    let mut lo = 1usize;
    let mut hi = total_chars;
    let mut best = (String::from("..."), estimate_tokens("..."));
    while lo <= hi {
        let mid = (lo + hi) / 2;
        let prefix: String = text.chars().take(mid).collect();
        let candidate = format!("{prefix}...");
        let tokens = estimate_tokens(&candidate);
        if tokens <= token_budget {
            best = (candidate, tokens);
            lo = mid.saturating_add(1);
        } else if mid <= 1 {
            break;
        } else {
            hi = mid - 1;
        }
    }
    best
}
pub(crate) fn pack_context_items_greedy(items: &[ContextItem], max_tokens: usize) -> PackedContext {
    let mut budget_remaining = max_tokens;
    let mut admitted: Vec<Value> = Vec::new();
    let mut rejected: Vec<Value> = Vec::new();
    let mut assembled_parts: Vec<String> = Vec::new();
    for item in items {
        if item.tokens <= budget_remaining && !item.text.is_empty() {
            assembled_parts.push(item.text.clone());
            budget_remaining -= item.tokens;
            admitted.push(attach_rank_audit(
                json!({"name":item.name,"tokens":item.tokens,"priority":
item.priority,"utility":(item.utility*10000.0).round()/10000.0}),
                item,
            ));
        } else if !item.text.is_empty() {
            if item.priority >= 0.7 && budget_remaining > 30 {
                let trunc_chars = (budget_remaining as f64 * 3.5) as usize;
                let truncated: String = item.text.chars().take(trunc_chars).collect();
                let trunc_tokens = estimate_tokens(&truncated);
                assembled_parts.push(format!("{truncated}..."));
                budget_remaining = budget_remaining.saturating_sub(trunc_tokens);
                admitted.push(attach_rank_audit(
                    json!({"name":item.name,"tokens":trunc_tokens,
"priority":item.priority,"truncated":true}),
                    item,
                ));
            } else {
                rejected.push(attach_rank_audit(
                    json!({"name":item.name,"tokens":item.
tokens,"priority":item.priority,"reason":"budget_exceeded"}),
                    item,
                ));
            }
        }
    }
    PackedContext { assembled_parts, admitted, rejected }
}
pub(crate) fn score_adaptive_allocations(items: &[ContextItem], max_tokens: usize, bounds: SourceTokenBounds) -> Vec<usize> {
    let mut order: Vec<usize> = items.iter().enumerate().filter(|(_, item)| !item.text.is_empty()).map(|(idx, _)| idx).collect();
    order.sort_by(|left, right| {
        items[*right]
            .priority
            .partial_cmp(&items[*left].priority)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| items[*right].utility.partial_cmp(&items[*left].utility).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| items[*left].name.cmp(&items[*right].name))
    });
    let mut allocations = vec![0usize; items.len()];
    let mut floor_spent = 0usize;
    for idx in order {
        let item = &items[idx];
        let floor = item.tokens.min(bounds.min).min(bounds.max);
        if floor == 0 {
            continue;
        }
        if floor_spent.saturating_add(floor) <= max_tokens {
            allocations[idx] = floor;
            floor_spent += floor;
        } else if allocations.iter().all(|allocation| *allocation == 0) && max_tokens > 0 {
            allocations[idx] = item.tokens.min(bounds.max).min(max_tokens);
            floor_spent += allocations[idx];
        }
    }
    let mut remaining = max_tokens.saturating_sub(floor_spent);
    while remaining > 0 {
        let eligible: Vec<usize> = allocations
            .iter()
            .enumerate()
            .filter(|(idx, allocation)| **allocation > 0 && **allocation < items[*idx].tokens.min(bounds.max))
            .map(|(idx, _)| idx)
            .collect();
        if eligible.is_empty() {
            break;
        }
        let total_score = eligible.iter().map(|idx| items[*idx].priority.max(0.01)).sum::<f64>();
        let mut allocated_any = false;
        for idx in eligible {
            if remaining == 0 {
                break;
            }
            let cap = items[idx].tokens.min(bounds.max);
            let room = cap.saturating_sub(allocations[idx]);
            if room == 0 {
                continue;
            }
            let share = ((remaining as f64) * (items[idx].priority.max(0.01) / total_score)).ceil() as usize;
            let delta = share.max(1).min(room).min(remaining);
            allocations[idx] += delta;
            remaining -= delta;
            allocated_any = true;
        }
        if !allocated_any {
            break;
        }
    }
    allocations
}
pub(crate) fn pack_context_items_score_adaptive(items: &[ContextItem], max_tokens: usize, bounds: SourceTokenBounds) -> PackedContext {
    let allocations = score_adaptive_allocations(items, max_tokens, bounds);
    let mut admitted: Vec<Value> = Vec::new();
    let mut rejected: Vec<Value> = Vec::new();
    let mut assembled_parts: Vec<String> = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        if item.text.is_empty() {
            continue;
        }
        let allocation = allocations[idx];
        if allocation == 0 {
            rejected.push(attach_rank_audit(
                json!({"name":item.name,"tokens":item.tokens,"priority":item.priority,"reason":"score_adaptive_budget_exceeded"}),
                item,
            ));
            continue;
        }
        if item.tokens <= allocation {
            assembled_parts.push(item.text.clone());
            admitted.push(attach_rank_audit(
                json!({"name":item.name,
"tokens":item.tokens,"allocatedTokens":allocation,"priority":item.priority,"utility":(item.utility*10000.0).round()/10000.0,
"packing":"score_adaptive"}),
                item,
            ));
        } else {
            let (truncated, trunc_tokens) = truncate_to_token_budget(&item.text, allocation);
            assembled_parts.push(truncated);
            admitted.push(attach_rank_audit(
                json!({"name":item.name,"tokens":trunc_tokens,"allocatedTokens":
allocation,"priority":item.priority,"truncated":true,"packing":"score_adaptive"}),
                item,
            ));
        }
    }
    PackedContext { assembled_parts, admitted, rejected }
}
pub(crate) fn pack_context_items(items: &[ContextItem], max_tokens: usize, bounds: SourceTokenBounds) -> PackedContext {
    pack_context_items_with_mode(items, max_tokens, bounds, boot_packing_mode())
}
pub(crate) fn pack_context_items_with_mode(items: &[ContextItem], max_tokens: usize, bounds: SourceTokenBounds, mode: BootPackingMode) -> PackedContext {
    match mode {
        BootPackingMode::LegacyGreedy => pack_context_items_greedy(items, max_tokens),
        BootPackingMode::ScoreAdaptive => pack_context_items_score_adaptive(items, max_tokens, bounds),
        BootPackingMode::Auto => {
            if score_signal_is_flat(items) {
                pack_context_items_greedy(items, max_tokens)
            } else {
                pack_context_items_score_adaptive(items, max_tokens, bounds)
            }
        }
    }
}
