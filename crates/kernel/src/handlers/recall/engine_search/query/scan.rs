use super::*;
use crate::db::{ACTIVE_TEMPORAL_SQL, like_prefix, optional_coalesce_like_sql};
use rusqlite::{Connection, params};

pub(crate) fn search_table_scan_fallback(
    conn: &Connection,
    query_text: &str,
    limit: usize,
    source_prefix: Option<&str>,
    kind: SearchTableKind,
    term_groups: &[Vec<String>],
    excerpt_focus_terms: &[String],
    alignment_profile: &QueryAlignmentProfile,
) -> Result<Vec<SearchCandidate>, String> {
    let source_like = source_prefix.map(like_prefix);
    let mut ranked = Vec::new();
    let sql = match kind {
        SearchTableKind::Memories => format!(
            "SELECT id, text, source, tags, score, trust_score, retrievals, last_accessed, created_at FROM memories WHERE {ACTIVE_TEMPORAL_SQL} AND {}",
            optional_coalesce_like_sql("source", "memory::", "id", 1)
        ),
        SearchTableKind::Decisions => format!(
            "SELECT id, decision, context, score, trust_score, retrievals, last_accessed, created_at FROM decisions WHERE {ACTIVE_TEMPORAL_SQL} AND {}",
            optional_coalesce_like_sql("context", "decision::", "id", 1)
        ),
    };
    let mut stmt = conn.prepare_cached(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![source_like.as_deref()], |row| {
            Ok(match kind {
                SearchTableKind::Memories => (
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<f64>>(4)?,
                    row.get::<_, Option<f64>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ),
                SearchTableKind::Decisions => (
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    None,
                    row.get::<_, Option<f64>>(3)?,
                    row.get::<_, Option<f64>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ),
            })
        })
        .map_err(|e| e.to_string())?;
    for row in rows.flatten() {
        let (id, primary, alt, tags, score, trust_score, retrievals, last_accessed, created_at) =
            row;
        let source_key = search_source_key(kind, id, alt.as_deref());
        if !source_matches_prefix(&source_key, source_prefix) {
            continue;
        }
        let effective_score = blend_importance(score, trust_score);
        let stamp = last_accessed.as_deref().or(created_at.as_deref());
        let ts = parse_timestamp_ms(stamp.unwrap_or(""));
        let (relevance, matched_keywords, excerpt_chars) = if term_groups.is_empty() {
            (round4(0.5 * effective_score), 0, 220)
        } else {
            let haystacks = search_haystacks(kind, &primary, alt.as_deref(), tags.as_deref());
            let matched = count_matching_term_groups(&haystacks, term_groups);
            if matched == 0 {
                continue;
            }
            let ranking = fallback_ranking_score(
                query_text,
                term_groups.len(),
                matched,
                effective_score,
                recency_days(stamp),
                retrievals,
            );
            (round4(ranking), matched, 260)
        };
        let excerpt =
            query_focused_excerpt_with_terms(&primary, excerpt_focus_terms, excerpt_chars);
        ranked.push(SearchCandidate {
            source: source_key,
            alignment: alignment_profile.alignment_score(&excerpt),
            excerpt,
            relevance,
            matched_keywords,
            score: effective_score,
            ts,
            owner_id: None,
            visibility: None,
        });
    }
    sort_search_candidates(&mut ranked, !term_groups.is_empty());
    ranked.truncate(limit);
    Ok(ranked)
}
