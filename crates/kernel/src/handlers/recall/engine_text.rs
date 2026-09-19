use super::{RecallContext, RecallItem};
use std::collections::{HashMap, HashSet};

pub fn is_visible(owner_id: Option<i64>, visibility: Option<&str>, ctx: &RecallContext) -> bool {
    if !ctx.team_mode {
        return true;
    }
    let (Some(caller), Some(owner)) = (ctx.caller_id, owner_id) else {
        return false;
    };
    owner == caller || matches!(visibility, Some("shared") | Some("team"))
}

pub fn source_matches_prefix(source: &str, source_prefix: Option<&str>) -> bool {
    source_prefix.is_none_or(|prefix| source.starts_with(prefix))
}

/// FTS5 MATCH is a second query language on bind `?1` (`*`, `^`, `NEAR`,
/// `col:`, quotes). Doubling `"` is not enough: a quoted phrase still treats
/// trailing `*` as prefix and a leading `^` as initial-token.
///
/// Replace those operators with whitespace instead of deleting them:
/// deleting `^` from `2^n` concatenates to `2n` and misses the indexed
/// tokens `2` / `n`. Quoted wrapping still blocks unquoted operators.
pub(crate) fn strip_fts_operators(term: &str) -> Option<String> {
    let mut cleaned = String::with_capacity(term.len());
    for ch in term.chars() {
        cleaned.push(if ch == '*' || ch == '^' { ' ' } else { ch });
    }
    let t = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    (!t.is_empty() && !t.replace('"', "").is_empty()).then_some(t)
}

pub(crate) fn quote_fts_match_term(term: &str) -> Option<String> {
    strip_fts_operators(term).map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
}

pub fn dedup_preserve_order(values: &mut Vec<String>) {
    let mut seen = HashSet::with_capacity(values.len());
    values.retain(|value| seen.insert(value.clone()));
}

pub fn normalize_collapsed_source_rank(item: &mut RecallItem) {
    let mut best_scores: HashMap<String, (f64, usize)> =
        HashMap::with_capacity(item.collapsed_sources.len() + item.collapsed_source_scores.len());
    for (order, source) in item.collapsed_sources.iter().enumerate() {
        best_scores.entry(source.clone()).or_insert((0.0, order));
    }
    for (order, (source, score)) in item.collapsed_source_scores.iter().enumerate() {
        best_scores
            .entry(source.clone())
            .and_modify(|entry| {
                entry.0 = entry.0.max(*score);
                entry.1 = entry.1.min(order);
            })
            .or_insert((*score, order));
    }
    let mut ranked: Vec<(String, f64, usize)> = best_scores
        .into_iter()
        .map(|(source, (score, order))| (source, score, order))
        .collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.2.cmp(&b.2))
    });
    item.collapsed_source_scores = ranked
        .iter()
        .map(|(source, score, _)| (source.clone(), *score))
        .collect();
    item.collapsed_sources = item
        .collapsed_source_scores
        .iter()
        .map(|(source, _)| source.clone())
        .collect();
}

pub fn is_missing_team_visibility_columns(err: &rusqlite::Error) -> bool {
    let normalized = err.to_string().to_ascii_lowercase();
    normalized.contains("no such column")
        && (normalized.contains("owner_id") || normalized.contains("visibility"))
}
