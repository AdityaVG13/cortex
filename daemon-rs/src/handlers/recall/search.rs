// SPDX-License-Identifier: MIT
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::{TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;
use std::time::Instant;

use crate::handlers::{ensure_auth_with_caller_rated_for_class, ensure_endpoint_budget};
use crate::handlers::{
    estimate_tokens, json_response, now_iso, parse_timestamp_ms, resolve_source_identity,
    truncate_chars,
};

use super::*;
use crate::budgets::BudgetEndpoint;
use crate::co_occurrence;
use crate::db::checkpoint_wal_best_effort;
use crate::rate_limit::RequestClass;
use crate::rerank::{RerankCandidate, RerankedScore};
use crate::state::{
    PreCacheEntry, RecallHistoryEntry, RuntimeState, SqliteVecCanaryConfig, SqliteVecRouteMode,
};

// ─── Search helpers ──────────────────────────────────────────────────────────

pub(crate) fn search_memories(
    conn: &Connection,
    query_text: &str,
    limit: usize,
    source_prefix: Option<&str>,
) -> Result<Vec<SearchCandidate>, String> {
    let term_groups = build_search_term_groups(query_text);
    let excerpt_focus_terms = query_focus_terms_for_excerpt(query_text);
    let source_like = source_prefix.map(|prefix| format!("{prefix}%"));

    if term_groups.is_empty() {
        let mut stmt = conn
            .prepare(
                "SELECT id, text, source, tags, score, trust_score, retrievals, last_accessed, created_at, compressed_text, age_tier \
                 FROM memories WHERE status = 'active' \
                 AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now')) \
                 AND (?2 IS NULL OR COALESCE(source, 'memory::' || id) LIKE ?2) \
                 ORDER BY COALESCE(last_accessed, created_at) DESC LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(params![limit as i64, source_like.as_deref()], |row| {
                let text: String = row.get(1)?;
                let compressed: Option<String> = row.get(9)?;
                let age_tier: String = row
                    .get::<_, Option<String>>(10)?
                    .unwrap_or_else(|| "fresh".to_string());
                let display = crate::aging::get_display_text(&text, &compressed, &age_tier);
                let effective_score =
                    blend_importance(row.get::<_, Option<f64>>(4)?, row.get::<_, Option<f64>>(5)?);
                Ok(SearchCandidate {
                    source: row.get::<_, Option<String>>(2)?.unwrap_or_else(|| {
                        format!("memory::{}", row.get::<_, i64>(0).unwrap_or(0))
                    }),
                    excerpt: query_focused_excerpt_with_terms(&display, &excerpt_focus_terms, 220),
                    alignment: (0, 0),
                    relevance: round4(0.5 * effective_score),
                    matched_keywords: 0,
                    score: effective_score,
                    ts: parse_timestamp_ms(
                        &row.get::<_, Option<String>>(7)?
                            .or(row.get::<_, Option<String>>(8)?)
                            .unwrap_or_default(),
                    ),
                    owner_id: None,
                    visibility: None,
                })
            })
            .map_err(|e| e.to_string())?;

        return Ok(rows
            .flatten()
            .filter(|row| source_matches_prefix(&row.source, source_prefix))
            .collect());
    }

    let fts_query = build_fts_query(&term_groups);
    let bm25 = bm25_weights();

    let fts_result: Result<Vec<SearchCandidate>, String> = (|| {
        // Field-boosted BM25: memories_fts columns are (text, source, tags).
        // Weight tuning favors rich content matches while preserving useful source/tag
        // signal for code paths and metadata lookups.
        // bm25() returns negative values (more negative = better match), so ORDER BY ASC.
        let mut stmt = conn
            .prepare(
                "SELECT m.id, m.text, m.source, m.tags, m.score, m.trust_score, m.retrievals, m.last_accessed, m.created_at, m.compressed_text, m.age_tier, m.owner_id, m.visibility \
                 FROM memories_fts fts \
                 JOIN memories m ON m.id = fts.rowid \
                 WHERE memories_fts MATCH ?1 AND m.status = 'active' \
                 AND (m.expires_at IS NULL OR m.expires_at > datetime('now')) \
         AND (m.valid_from IS NULL OR m.valid_from <= datetime('now')) \
         AND (m.valid_until IS NULL OR m.valid_until > datetime('now')) \
                 AND (?6 IS NULL OR COALESCE(m.source, 'memory::' || m.id) LIKE ?6) \
                 ORDER BY bm25(memories_fts, ?3, ?4, ?5) \
                 LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(
                params![
                    &fts_query,
                    limit as i64,
                    bm25.memories_text,
                    bm25.memories_source,
                    bm25.memories_tags,
                    source_like.as_deref()
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<f64>>(4)?,
                        row.get::<_, Option<f64>>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, Option<String>>(10)?,
                        row.get::<_, Option<i64>>(11)?,
                        row.get::<_, Option<String>>(12)?,
                    ))
                },
            )
            .map_err(|e| e.to_string())?;

        let mut ranked = Vec::new();
        for row in rows.flatten() {
            let (
                id,
                text,
                source,
                tags,
                score,
                trust_score,
                retrievals,
                last_accessed,
                created_at,
                compressed_text,
                age_tier,
                row_owner_id,
                row_visibility,
            ) = row;
            let source_key = source
                .as_deref()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("memory::{id}"));
            if !source_matches_prefix(&source_key, source_prefix) {
                continue;
            }
            let effective_score = blend_importance(score, trust_score);
            let ts = parse_timestamp_ms(
                last_accessed
                    .as_deref()
                    .or(created_at.as_deref())
                    .unwrap_or(""),
            );
            let display = crate::aging::get_display_text(
                &text,
                &compressed_text,
                age_tier.as_deref().unwrap_or("fresh"),
            );

            let haystacks = [
                text.to_lowercase(),
                source.as_deref().unwrap_or("").to_lowercase(),
                tags.as_deref().unwrap_or("").to_lowercase(),
            ];
            let matched = count_matching_term_groups(&haystacks, &term_groups);
            let recency_d = recency_days(last_accessed.as_deref().or(created_at.as_deref()));
            let ranking = fallback_ranking_score(
                query_text,
                term_groups.len(),
                matched,
                effective_score,
                recency_d,
                retrievals,
            );

            ranked.push(SearchCandidate {
                source: source_key,
                excerpt: query_focused_excerpt_with_terms(&display, &excerpt_focus_terms, 280),
                alignment: (0, 0),
                relevance: round4(ranking),
                matched_keywords: matched,
                score: effective_score,
                ts,
                owner_id: row_owner_id,
                visibility: row_visibility,
            });
        }

        ranked.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.matched_keywords.cmp(&a.matched_keywords))
                .then(
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(b.ts.cmp(&a.ts))
                .then_with(|| a.source.cmp(&b.source))
        });

        ranked.truncate(limit);
        Ok(ranked)
    })();

    match fts_result {
        Ok(results) if !results.is_empty() => Ok(results),
        _ => search_memories_fallback(conn, query_text, limit, source_prefix),
    }
}

pub(crate) fn search_memories_fallback(
    conn: &Connection,
    query_text: &str,
    limit: usize,
    source_prefix: Option<&str>,
) -> Result<Vec<SearchCandidate>, String> {
    let source_like = source_prefix.map(|prefix| format!("{prefix}%"));
    let mut stmt = conn
        .prepare(
            "SELECT id, text, source, tags, score, trust_score, retrievals, last_accessed, created_at \
             FROM memories WHERE status = 'active' \
             AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now')) \
             AND (?1 IS NULL OR COALESCE(source, 'memory::' || id) LIKE ?1)",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(params![source_like.as_deref()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<f64>>(4)?,
                row.get::<_, Option<f64>>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let term_groups = build_search_term_groups(query_text);
    let excerpt_focus_terms = query_focus_terms_for_excerpt(query_text);
    let alignment_profile = QueryAlignmentProfile::from_query(query_text);
    let mut ranked = Vec::new();

    for row in rows.flatten() {
        let (id, text, source, tags, score, trust_score, retrievals, last_accessed, created_at) =
            row;
        let source_key = source
            .as_deref()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("memory::{id}"));
        if !source_matches_prefix(&source_key, source_prefix) {
            continue;
        }
        let effective_score = blend_importance(score, trust_score);
        let ts = parse_timestamp_ms(
            last_accessed
                .as_deref()
                .or(created_at.as_deref())
                .unwrap_or(""),
        );

        if term_groups.is_empty() {
            let excerpt = query_focused_excerpt_with_terms(&text, &excerpt_focus_terms, 220);
            ranked.push(SearchCandidate {
                source: source_key,
                alignment: alignment_profile.alignment_score(&excerpt),
                excerpt,
                relevance: round4(0.5 * effective_score),
                matched_keywords: 0,
                score: effective_score,
                ts,
                owner_id: None,
                visibility: None,
            });
            continue;
        }

        let haystacks = [
            text.to_lowercase(),
            source.as_deref().unwrap_or("").to_lowercase(),
            tags.as_deref().unwrap_or("").to_lowercase(),
        ];

        let matched = count_matching_term_groups(&haystacks, &term_groups);
        if matched == 0 {
            continue;
        }

        let recency_d = recency_days(last_accessed.as_deref().or(created_at.as_deref()));
        let ranking = fallback_ranking_score(
            query_text,
            term_groups.len(),
            matched,
            effective_score,
            recency_d,
            retrievals,
        );

        let excerpt = query_focused_excerpt_with_terms(&text, &excerpt_focus_terms, 260);
        ranked.push(SearchCandidate {
            source: source_key,
            alignment: alignment_profile.alignment_score(&excerpt),
            excerpt,
            relevance: round4(ranking),
            matched_keywords: matched,
            score: effective_score,
            ts,
            owner_id: None,
            visibility: None,
        });
    }

    if term_groups.is_empty() {
        ranked.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(b.ts.cmp(&a.ts))
                .then(b.alignment.cmp(&a.alignment))
                .then_with(|| a.source.cmp(&b.source))
        });
    } else {
        ranked.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.matched_keywords.cmp(&a.matched_keywords))
                .then(
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(b.ts.cmp(&a.ts))
                .then(b.alignment.cmp(&a.alignment))
                .then_with(|| a.source.cmp(&b.source))
        });
    }

    ranked.truncate(limit);
    Ok(ranked)
}

pub(crate) fn search_decisions(
    conn: &Connection,
    query_text: &str,
    limit: usize,
    source_prefix: Option<&str>,
) -> Result<Vec<SearchCandidate>, String> {
    let term_groups = build_search_term_groups(query_text);
    let excerpt_focus_terms = query_focus_terms_for_excerpt(query_text);
    let source_like = source_prefix.map(|prefix| format!("{prefix}%"));

    if term_groups.is_empty() {
        let mut stmt = conn
            .prepare(
                "SELECT id, decision, context, score, trust_score, retrievals, last_accessed, created_at \
                 FROM decisions WHERE status = 'active' \
                 AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now')) \
                 AND (?2 IS NULL OR COALESCE(context, 'decision::' || id) LIKE ?2) \
                 ORDER BY COALESCE(last_accessed, created_at) DESC LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(params![limit as i64, source_like.as_deref()], |row| {
                let effective_score =
                    blend_importance(row.get::<_, Option<f64>>(3)?, row.get::<_, Option<f64>>(4)?);
                Ok(SearchCandidate {
                    source: row.get::<_, Option<String>>(2)?.unwrap_or_else(|| {
                        format!("decision::{}", row.get::<_, i64>(0).unwrap_or(0))
                    }),
                    excerpt: query_focused_excerpt_with_terms(
                        &row.get::<_, String>(1)?,
                        &excerpt_focus_terms,
                        220,
                    ),
                    alignment: (0, 0),
                    relevance: round4(0.5 * effective_score),
                    matched_keywords: 0,
                    score: effective_score,
                    ts: parse_timestamp_ms(
                        &row.get::<_, Option<String>>(6)?
                            .or(row.get::<_, Option<String>>(7)?)
                            .unwrap_or_default(),
                    ),
                    owner_id: None,
                    visibility: None,
                })
            })
            .map_err(|e| e.to_string())?;

        return Ok(rows
            .flatten()
            .filter(|row| source_matches_prefix(&row.source, source_prefix))
            .collect());
    }

    let fts_query = build_fts_query(&term_groups);
    let bm25 = bm25_weights();

    let fts_result: Result<Vec<SearchCandidate>, String> = (|| {
        // Field-boosted BM25: decisions_fts columns are (decision, context).
        // Decision text carries most signal; context acts as a secondary anchor.
        let mut stmt = conn
            .prepare(
                "SELECT d.id, d.decision, d.context, d.score, d.trust_score, d.retrievals, d.last_accessed, d.created_at, d.compressed_text, d.age_tier, d.owner_id, d.visibility \
                 FROM decisions_fts fts \
                 JOIN decisions d ON d.id = fts.rowid \
                 WHERE decisions_fts MATCH ?1 AND d.status = 'active' \
                 AND (d.expires_at IS NULL OR d.expires_at > datetime('now')) \
         AND (d.valid_from IS NULL OR d.valid_from <= datetime('now')) \
         AND (d.valid_until IS NULL OR d.valid_until > datetime('now')) \
                 AND (?5 IS NULL OR COALESCE(d.context, 'decision::' || d.id) LIKE ?5) \
                 ORDER BY bm25(decisions_fts, ?3, ?4) \
                 LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(
                params![
                    &fts_query,
                    limit as i64,
                    bm25.decisions_text,
                    bm25.decisions_context,
                    source_like.as_deref()
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<f64>>(3)?,
                        row.get::<_, Option<f64>>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, Option<i64>>(10)?,
                        row.get::<_, Option<String>>(11)?,
                    ))
                },
            )
            .map_err(|e| e.to_string())?;

        let mut ranked = Vec::new();
        for row in rows.flatten() {
            let (
                id,
                decision,
                context,
                score,
                trust_score,
                retrievals,
                last_accessed,
                created_at,
                compressed_text,
                age_tier,
                row_owner_id,
                row_visibility,
            ) = row;
            let source_key = context
                .as_deref()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("decision::{id}"));
            if !source_matches_prefix(&source_key, source_prefix) {
                continue;
            }
            let effective_score = blend_importance(score, trust_score);
            let ts = parse_timestamp_ms(
                last_accessed
                    .as_deref()
                    .or(created_at.as_deref())
                    .unwrap_or(""),
            );
            let display = crate::aging::get_display_text(
                &decision,
                &compressed_text,
                age_tier.as_deref().unwrap_or("fresh"),
            );

            let haystacks = [
                decision.to_lowercase(),
                context.as_deref().unwrap_or("").to_lowercase(),
            ];
            let matched = count_matching_term_groups(&haystacks, &term_groups);
            let recency_d = recency_days(last_accessed.as_deref().or(created_at.as_deref()));
            let ranking = fallback_ranking_score(
                query_text,
                term_groups.len(),
                matched,
                effective_score,
                recency_d,
                retrievals,
            );

            ranked.push(SearchCandidate {
                source: source_key,
                excerpt: query_focused_excerpt_with_terms(&display, &excerpt_focus_terms, 280),
                alignment: (0, 0),
                relevance: round4(ranking),
                matched_keywords: matched,
                score: effective_score,
                ts,
                owner_id: row_owner_id,
                visibility: row_visibility,
            });
        }

        ranked.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.matched_keywords.cmp(&a.matched_keywords))
                .then(
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(b.ts.cmp(&a.ts))
                .then_with(|| a.source.cmp(&b.source))
        });

        ranked.truncate(limit);
        Ok(ranked)
    })();

    match fts_result {
        Ok(results) if !results.is_empty() => Ok(results),
        _ => search_decisions_fallback(conn, query_text, limit, source_prefix),
    }
}

pub(crate) fn search_decisions_fallback(
    conn: &Connection,
    query_text: &str,
    limit: usize,
    source_prefix: Option<&str>,
) -> Result<Vec<SearchCandidate>, String> {
    let source_like = source_prefix.map(|prefix| format!("{prefix}%"));
    let mut stmt = conn
        .prepare(
            "SELECT id, decision, context, score, trust_score, retrievals, last_accessed, created_at \
             FROM decisions WHERE status = 'active' \
             AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now')) \
             AND (?1 IS NULL OR COALESCE(context, 'decision::' || id) LIKE ?1)",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(params![source_like.as_deref()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<f64>>(3)?,
                row.get::<_, Option<f64>>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let term_groups = build_search_term_groups(query_text);
    let excerpt_focus_terms = query_focus_terms_for_excerpt(query_text);
    let alignment_profile = QueryAlignmentProfile::from_query(query_text);
    let mut ranked = Vec::new();

    for row in rows.flatten() {
        let (id, decision, context, score, trust_score, retrievals, last_accessed, created_at) =
            row;
        let source_key = context
            .as_deref()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("decision::{id}"));
        if !source_matches_prefix(&source_key, source_prefix) {
            continue;
        }
        let effective_score = blend_importance(score, trust_score);
        let ts = parse_timestamp_ms(
            last_accessed
                .as_deref()
                .or(created_at.as_deref())
                .unwrap_or(""),
        );

        if term_groups.is_empty() {
            let excerpt = query_focused_excerpt_with_terms(&decision, &excerpt_focus_terms, 220);
            ranked.push(SearchCandidate {
                source: source_key,
                alignment: alignment_profile.alignment_score(&excerpt),
                excerpt,
                relevance: round4(0.5 * effective_score),
                matched_keywords: 0,
                score: effective_score,
                ts,
                owner_id: None,
                visibility: None,
            });
            continue;
        }

        let haystacks = [
            decision.to_lowercase(),
            context.as_deref().unwrap_or("").to_lowercase(),
        ];
        let matched = count_matching_term_groups(&haystacks, &term_groups);
        if matched == 0 {
            continue;
        }

        let recency_d = recency_days(last_accessed.as_deref().or(created_at.as_deref()));
        let ranking = fallback_ranking_score(
            query_text,
            term_groups.len(),
            matched,
            effective_score,
            recency_d,
            retrievals,
        );

        let excerpt = query_focused_excerpt_with_terms(&decision, &excerpt_focus_terms, 260);
        ranked.push(SearchCandidate {
            source: source_key,
            alignment: alignment_profile.alignment_score(&excerpt),
            excerpt,
            relevance: round4(ranking),
            matched_keywords: matched,
            score: effective_score,
            ts,
            owner_id: None,
            visibility: None,
        });
    }

    if term_groups.is_empty() {
        ranked.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(b.ts.cmp(&a.ts))
                .then(b.alignment.cmp(&a.alignment))
                .then_with(|| a.source.cmp(&b.source))
        });
    } else {
        ranked.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.matched_keywords.cmp(&a.matched_keywords))
                .then(
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(b.ts.cmp(&a.ts))
                .then(b.alignment.cmp(&a.alignment))
                .then_with(|| a.source.cmp(&b.source))
        });
    }

    ranked.truncate(limit);
    Ok(ranked)
}

