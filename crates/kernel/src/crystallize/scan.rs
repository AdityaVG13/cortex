use super::Candidate;
use crate::conflict::{
    fold_jaccard_token, jaccard_similarity_token_sets, jaccard_token_set as conflict_token_set,
};
use crate::db::LAST_ACCESSED_CREATED_STAMP_SQL;
use rusqlite::{Connection, params};
use rustc_hash::FxHashSet;
use std::collections::HashMap;

const MAX_SCAN_ROWS: i64 = 500;
const JACCARD_THRESHOLD: f64 = 0.30;

const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "been", "but", "by", "can", "could", "did", "do",
    "does", "for", "from", "had", "has", "have", "he", "how", "i", "if", "in", "is", "it", "its",
    "me", "my", "no", "not", "of", "on", "or", "our", "she", "should", "so", "than", "that", "the",
    "their", "them", "then", "these", "they", "this", "those", "to", "us", "was", "we", "were",
    "what", "when", "which", "who", "why", "will", "with", "would", "yes", "you", "your",
];

fn is_crystal_stop(token: &str) -> bool {
    STOPWORDS.binary_search(&token).is_ok()
}

fn crystal_token_set(text: &str) -> FxHashSet<String> {
    let mut tokens = conflict_token_set(text);
    tokens.retain(|token| !is_crystal_stop(token));
    tokens
}

pub(super) fn label_for_members(texts: &[String]) -> String {
    let mut freq: HashMap<String, usize> = HashMap::new();
    for t in texts {
        for word in t.split_whitespace().filter(|w| w.len() > 1) {
            let lower = fold_jaccard_token(word);
            let cleaned: String = lower.chars().filter(|c| c.is_alphanumeric()).collect();
            if cleaned.len() <= 1 {
                continue;
            }
            if is_crystal_stop(&cleaned) {
                continue;
            }
            *freq.entry(cleaned).or_insert(0) += 1;
        }
    }
    let mut items: Vec<(String, usize)> = freq.into_iter().collect();
    items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    items.truncate(4);
    if items.is_empty() {
        let first = texts.first().map(|s| s.as_str()).unwrap_or("");
        let tokens: Vec<String> = first
            .split_whitespace()
            .filter(|w| w.len() > 1)
            .take(4)
            .map(fold_jaccard_token)
            .collect();
        if tokens.is_empty() {
            return "cluster".to_string();
        }
        return tokens.join(" ");
    }
    items
        .into_iter()
        .map(|(w, _)| w)
        .collect::<Vec<_>>()
        .join(" ")
}

fn candidate_from_row(row: &rusqlite::Row<'_>, target_type: &str) -> rusqlite::Result<Candidate> {
    Ok(Candidate {
        id: row.get(0)?,
        text: row.get(1)?,
        score: row.get(3)?,
        target_type: target_type.to_string(),
        recency: row.get(4)?,
    })
}

fn cap_candidates_by_recency(out: &mut Vec<Candidate>) {
    if out.len() as i64 <= MAX_SCAN_ROWS {
        return;
    }
    out.sort_by(|a, b| {
        b.recency
            .unwrap_or(f64::NEG_INFINITY)
            .total_cmp(&a.recency.unwrap_or(f64::NEG_INFINITY))
            .then_with(|| b.id.cmp(&a.id))
    });
    out.truncate(MAX_SCAN_ROWS as usize);
}

pub(super) fn scan_candidates(
    conn: &Connection,
    owner_id: Option<i64>,
) -> Result<Vec<Candidate>, String> {
    const KINDS: &[(&str, &str, &str, &str)] = &[
        ("memories", "text", "source", "memory"),
        ("decisions", "decision", "context", "decision"),
    ];
    let mut out: Vec<Candidate> = Vec::new();
    for &(table, text_col, extra_col, kind) in KINDS {
        let sql = format!(
            "SELECT id, {text_col}, COALESCE({extra_col},''), COALESCE(score,1.0), julianday({LAST_ACCESSED_CREATED_STAMP_SQL}) FROM {table} WHERE status NOT IN ('superseded','archived') AND id NOT IN (SELECT target_id FROM cluster_members WHERE target_type='{kind}') AND (?1 IS NULL OR owner_id = ?1 OR owner_id IS NULL) ORDER BY julianday({LAST_ACCESSED_CREATED_STAMP_SQL}) DESC, id DESC LIMIT ?2"
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows: Vec<rusqlite::Result<Candidate>> = stmt
            .query_map(params![owner_id, MAX_SCAN_ROWS], |row| {
                candidate_from_row(row, kind)
            })
            .map_err(|e| e.to_string())?
            .collect();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
    }
    cap_candidates_by_recency(&mut out);
    Ok(out)
}

pub(super) fn cluster_by_jaccard(candidates: &[Candidate]) -> Vec<Vec<usize>> {
    let token_sets: Vec<FxHashSet<String>> = candidates
        .iter()
        .map(|c| crystal_token_set(&c.text))
        .collect();
    let mut clusters: Vec<Vec<usize>> = Vec::new();
    for (idx, set) in token_sets.iter().enumerate() {
        let mut assigned = false;
        for cluster in &mut clusters {
            let rep_idx = cluster[0];
            let rep_set = &token_sets[rep_idx];
            let sim = jaccard_similarity_token_sets(set, rep_set);
            if sim >= JACCARD_THRESHOLD {
                cluster.push(idx);
                assigned = true;
                break;
            }
        }
        if !assigned {
            clusters.push(vec![idx]);
        }
    }
    clusters
}
