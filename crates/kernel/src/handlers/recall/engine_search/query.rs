use super::*;
use crate::db::{ACTIVE_TEMPORAL_SQL, LAST_ACCESSED_CREATED_STAMP_SQL, optional_coalesce_like_sql};
use crate::handlers::parse_timestamp_ms;
use rusqlite::{Connection, params};

fn memories_recency_sql() -> String {
    format!(
        "SELECT id, text, source, tags, score, trust_score, retrievals, last_accessed, created_at, compressed_text, age_tier, owner_id, visibility FROM memories WHERE {ACTIVE_TEMPORAL_SQL} AND {} ORDER BY julianday({LAST_ACCESSED_CREATED_STAMP_SQL}) DESC, id DESC LIMIT ?1",
        optional_coalesce_like_sql("source", "memory::", "id", 2)
    )
}
fn decisions_recency_sql() -> String {
    format!(
        "SELECT id, decision, context, score, trust_score, retrievals, last_accessed, created_at, owner_id, visibility FROM decisions WHERE {ACTIVE_TEMPORAL_SQL} AND {} ORDER BY julianday({LAST_ACCESSED_CREATED_STAMP_SQL}) DESC, id DESC LIMIT ?1",
        optional_coalesce_like_sql("context", "decision::", "id", 2)
    )
}

pub(super) fn search_haystacks(
    kind: SearchTableKind,
    primary: &str,
    alt: Option<&str>,
    tags: Option<&str>,
) -> Vec<String> {
    let mut hay = vec![primary.to_lowercase(), alt.unwrap_or("").to_lowercase()];
    if matches!(kind, SearchTableKind::Memories) {
        hay.push(tags.unwrap_or("").to_lowercase());
    }
    hay
}

struct RecencyCols {
    score: usize,
    trust: usize,
    last_accessed: usize,
    created_at: usize,
    owner: usize,
    visibility: usize,
    aging: bool,
}

fn recency_cols(kind: SearchTableKind) -> RecencyCols {
    match kind {
        SearchTableKind::Memories => RecencyCols {
            score: 4,
            trust: 5,
            last_accessed: 7,
            created_at: 8,
            owner: 11,
            visibility: 12,
            aging: true,
        },
        SearchTableKind::Decisions => RecencyCols {
            score: 3,
            trust: 4,
            last_accessed: 6,
            created_at: 7,
            owner: 8,
            visibility: 9,
            aging: false,
        },
    }
}

pub(super) fn search_table_recency(
    conn: &Connection,
    limit: usize,
    source_prefix: Option<&str>,
    source_like: Option<&str>,
    kind: SearchTableKind,
    excerpt_focus_terms: &[String],
) -> Result<Vec<SearchCandidate>, String> {
    let sql = match kind {
        SearchTableKind::Memories => memories_recency_sql(),
        SearchTableKind::Decisions => decisions_recency_sql(),
    };
    let cols = recency_cols(kind);
    let mut stmt = conn.prepare_cached(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![limit as i64, source_like], |row| {
            let effective_score = blend_importance(
                row.get::<_, Option<f64>>(cols.score)?,
                row.get::<_, Option<f64>>(cols.trust)?,
            );
            let text: String = row.get(1)?;
            let display = if cols.aging {
                let compressed: Option<String> = row.get(9)?;
                let age_tier: String = row
                    .get::<_, Option<String>>(10)?
                    .unwrap_or_else(|| "fresh".to_string());
                crate::aging::get_display_text(&text, &compressed, &age_tier)
            } else {
                text
            };
            let id: i64 = row.get(0)?;
            let alt: Option<String> = row.get(2)?;
            Ok(SearchCandidate {
                source: search_source_key(kind, id, alt.as_deref()),
                excerpt: query_focused_excerpt_with_terms(&display, excerpt_focus_terms, 220),
                alignment: (0, 0),
                relevance: round4(0.5 * effective_score),
                matched_keywords: 0,
                score: effective_score,
                ts: parse_timestamp_ms(
                    &row.get::<_, Option<String>>(cols.last_accessed)?
                        .or(row.get::<_, Option<String>>(cols.created_at)?)
                        .unwrap_or_default(),
                ),
                owner_id: row.get(cols.owner)?,
                visibility: row.get(cols.visibility)?,
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(rows
        .flatten()
        .filter(|row| source_matches_prefix(&row.source, source_prefix))
        .collect())
}
#[path = "query/fts.rs"]
mod fts;
#[path = "query/scan.rs"]
mod scan;
pub(super) use fts::search_table_fts;
pub(super) use scan::search_table_scan_fallback;
