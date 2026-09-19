use super::*;
use crate::db::like_prefix;
use crate::protocol::nonempty_opt;
use rusqlite::Connection;

fn sort_search_candidates(ranked: &mut [SearchCandidate], by_keywords: bool) {
    ranked.sort_by(|a, b| {
        let ord = b
            .relevance
            .partial_cmp(&a.relevance)
            .unwrap_or(std::cmp::Ordering::Equal);
        let ord = if by_keywords {
            ord.then(b.matched_keywords.cmp(&a.matched_keywords))
        } else {
            ord
        };
        ord.then(
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
        .then(b.ts.cmp(&a.ts))
        .then(b.alignment.cmp(&a.alignment))
        .then_with(|| a.source.cmp(&b.source))
    });
}
#[derive(Clone, Copy)]
pub(crate) enum SearchTableKind {
    Memories,
    Decisions,
}
pub(crate) fn search_source_key(kind: SearchTableKind, id: i64, alt: Option<&str>) -> String {
    // Blank `source`/`context` is stored as '' not NULL; treat it as missing
    // so ranking, feedback, and retrieval bumps use the identity key.
    nonempty_opt(alt).map(str::to_string).unwrap_or_else(|| {
        let kind = match kind {
            SearchTableKind::Memories => "memory",
            SearchTableKind::Decisions => "decision",
        };
        format!("{kind}::{id}")
    })
}
#[path = "engine_search/query.rs"]
mod query;
use query::{search_table_fts, search_table_recency, search_table_scan_fallback};
fn search_table(
    conn: &Connection,
    query_text: &str,
    limit: usize,
    source_prefix: Option<&str>,
    kind: SearchTableKind,
) -> Result<Vec<SearchCandidate>, String> {
    let term_groups = build_search_term_groups(query_text);
    let excerpt_focus_terms = query_focus_terms_for_excerpt(query_text);
    let source_like = source_prefix.map(like_prefix);
    if term_groups.is_empty() {
        return search_table_recency(
            conn,
            limit,
            source_prefix,
            source_like.as_deref(),
            kind,
            &excerpt_focus_terms,
        );
    }
    let fts_query = build_fts_query(&term_groups);
    if fts_query.is_empty() {
        return search_table_recency(
            conn,
            limit,
            source_prefix,
            source_like.as_deref(),
            kind,
            &excerpt_focus_terms,
        );
    }
    let fts_result = search_table_fts(
        conn,
        &fts_query,
        limit,
        source_like.as_deref(),
        source_prefix,
        kind,
        &term_groups,
        &excerpt_focus_terms,
        query_text,
        bm25_weights(),
    );
    match fts_result {
        Ok(results) if !results.is_empty() => Ok(results),
        _ => search_table_scan_fallback(
            conn,
            query_text,
            limit,
            source_prefix,
            kind,
            &term_groups,
            &excerpt_focus_terms,
            &QueryAlignmentProfile::from_query(query_text),
        ),
    }
}
pub fn search_memories(
    conn: &Connection,
    query_text: &str,
    limit: usize,
    source_prefix: Option<&str>,
) -> Result<Vec<SearchCandidate>, String> {
    search_table(
        conn,
        query_text,
        limit,
        source_prefix,
        SearchTableKind::Memories,
    )
}
pub fn search_decisions(
    conn: &Connection,
    query_text: &str,
    limit: usize,
    source_prefix: Option<&str>,
) -> Result<Vec<SearchCandidate>, String> {
    search_table(
        conn,
        query_text,
        limit,
        source_prefix,
        SearchTableKind::Decisions,
    )
}
