use super::*;
use crate::db::optional_coalesce_like_sql;
use rusqlite::{Connection, Row, Statement, params};

fn memories_fts_sql() -> String {
    format!(
        "SELECT m.id, m.text, m.source, m.tags, m.score, m.trust_score, m.retrievals, m.last_accessed, m.created_at, m.compressed_text, m.age_tier, m.owner_id, m.visibility FROM memories_fts fts JOIN memories m ON m.id = fts.rowid WHERE memories_fts MATCH ?1 AND {} AND {} ORDER BY bm25(memories_fts, ?3, ?4, ?5) LIMIT ?2",
        crate::db::active_temporal_sql("m."),
        optional_coalesce_like_sql("m.source", "memory::", "m.id", 6)
    )
}
fn decisions_fts_sql() -> String {
    format!(
        "SELECT d.id, d.decision, d.context, d.score, d.trust_score, d.retrievals, d.last_accessed, d.created_at, d.compressed_text, d.age_tier, d.owner_id, d.visibility FROM decisions_fts fts JOIN decisions d ON d.id = fts.rowid WHERE decisions_fts MATCH ?1 AND {} AND {} ORDER BY bm25(decisions_fts, ?3, ?4) LIMIT ?2",
        crate::db::active_temporal_sql("d."),
        optional_coalesce_like_sql("d.context", "decision::", "d.id", 5)
    )
}

struct FtsHit {
    id: i64,
    primary: String,
    alt: Option<String>,
    tags: Option<String>,
    score: Option<f64>,
    trust_score: Option<f64>,
    retrievals: Option<i64>,
    last_accessed: Option<String>,
    created_at: Option<String>,
    compressed_text: Option<String>,
    age_tier: Option<String>,
    owner_id: Option<i64>,
    visibility: Option<String>,
}

fn fts_hit(row: &Row<'_>, has_tags: bool) -> rusqlite::Result<FtsHit> {
    let tags = if has_tags { row.get(3)? } else { None };
    let b = if has_tags { 4 } else { 3 };
    Ok(FtsHit {
        id: row.get(0)?,
        primary: row.get(1)?,
        alt: row.get(2)?,
        tags,
        score: row.get(b)?,
        trust_score: row.get(b + 1)?,
        retrievals: row.get(b + 2)?,
        last_accessed: row.get(b + 3)?,
        created_at: row.get(b + 4)?,
        compressed_text: row.get(b + 5)?,
        age_tier: row.get(b + 6)?,
        owner_id: row.get(b + 7)?,
        visibility: row.get(b + 8)?,
    })
}

fn drain_fts_hits(
    stmt: &mut Statement<'_>,
    params: impl rusqlite::Params,
    has_tags: bool,
    mut push: impl FnMut(FtsHit),
) -> Result<(), String> {
    let rows = stmt
        .query_map(params, |row| fts_hit(row, has_tags))
        .map_err(|e| e.to_string())?;
    for hit in rows.flatten() {
        push(hit);
    }
    Ok(())
}

pub(crate) fn search_table_fts(
    conn: &Connection,
    fts_query: &str,
    limit: usize,
    source_like: Option<&str>,
    source_prefix: Option<&str>,
    kind: SearchTableKind,
    term_groups: &[Vec<String>],
    excerpt_focus_terms: &[String],
    query_text: &str,
    bm25: &Bm25Weights,
) -> Result<Vec<SearchCandidate>, String> {
    let sql = match kind {
        SearchTableKind::Memories => memories_fts_sql(),
        SearchTableKind::Decisions => decisions_fts_sql(),
    };
    let mut stmt = conn.prepare_cached(&sql).map_err(|e| e.to_string())?;
    let mut ranked = Vec::new();
    let mut push_fts_row = |hit: FtsHit| {
        let source_key = search_source_key(kind, hit.id, hit.alt.as_deref());
        if !source_matches_prefix(&source_key, source_prefix) {
            return;
        }
        let effective_score = blend_importance(hit.score, hit.trust_score);
        let ts = parse_timestamp_ms(
            hit.last_accessed
                .as_deref()
                .or(hit.created_at.as_deref())
                .unwrap_or(""),
        );
        let display = crate::aging::get_display_text(
            &hit.primary,
            &hit.compressed_text,
            hit.age_tier.as_deref().unwrap_or("fresh"),
        );
        let haystacks =
            search_haystacks(kind, &hit.primary, hit.alt.as_deref(), hit.tags.as_deref());
        let matched = count_matching_term_groups(&haystacks, term_groups);
        let recency_d = recency_days(hit.last_accessed.as_deref().or(hit.created_at.as_deref()));
        let ranking = fallback_ranking_score(
            query_text,
            term_groups.len(),
            matched,
            effective_score,
            recency_d,
            hit.retrievals,
        );
        ranked.push(SearchCandidate {
            source: source_key,
            excerpt: query_focused_excerpt_with_terms(&display, excerpt_focus_terms, 280),
            alignment: (0, 0),
            relevance: round4(ranking),
            matched_keywords: matched,
            score: effective_score,
            ts,
            owner_id: hit.owner_id,
            visibility: hit.visibility,
        });
    };
    match kind {
        SearchTableKind::Memories => drain_fts_hits(
            &mut stmt,
            params![
                fts_query,
                limit as i64,
                bm25.memories_text,
                bm25.memories_source,
                bm25.memories_tags,
                source_like
            ],
            true,
            &mut push_fts_row,
        )?,
        SearchTableKind::Decisions => drain_fts_hits(
            &mut stmt,
            params![
                fts_query,
                limit as i64,
                bm25.decisions_text,
                bm25.decisions_context,
                source_like
            ],
            false,
            &mut push_fts_row,
        )?,
    }
    sort_search_candidates(&mut ranked, true);
    ranked.truncate(limit);
    Ok(ranked)
}
