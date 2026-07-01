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

// ─── GET /unfold ────────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
pub struct UnfoldQuery {
    pub sources: Option<String>,
}

pub(crate) const MAX_UNFOLD_SOURCES: usize = 50;

/// Unfold specific items by source string. Returns full text for each requested
/// source without re-running search. Designed for progressive disclosure:
/// peek (headlines) → unfold (full text of selected items).
pub async fn handle_unfold(
    State(state): State<RuntimeState>,
    Query(query): Query<UnfoldQuery>,
    headers: HeaderMap,
) -> Response {
    let caller_id =
        match ensure_auth_with_caller_rated_for_class(&headers, &state, RequestClass::Recall).await
        {
            Ok(id) => id,
            Err(resp) => return resp,
        };
    let caller_id = match require_team_caller(&state, caller_id) {
        Ok(caller_id) => caller_id,
        Err(resp) => return resp,
    };
    let ctx = RecallContext::from_caller(caller_id, &state);
    let sources_str = match &query.sources {
        Some(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "Missing query parameter: sources (comma-separated)"}),
            );
        }
    };

    let requested: Vec<&str> = sources_str
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if requested.is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": "No valid sources provided"}),
        );
    }
    if requested.len() > MAX_UNFOLD_SOURCES {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": format!("Too many sources (max {MAX_UNFOLD_SOURCES})")}),
        );
    }

    let agent = resolve_source_identity(&headers, "http").agent;
    if let Err(resp) =
        ensure_endpoint_budget(&headers, &state, BudgetEndpoint::Recall, &agent).await
    {
        return resp;
    }

    let conn = state.db_read.lock().await;
    let mut results: Vec<Value> = Vec::new();
    let mut total_tokens = 0usize;

    for source in &requested {
        if let Some(mut item) = unfold_source(&conn, source, &ctx) {
            let tokens = estimate_tokens(item["text"].as_str().unwrap_or(""));
            total_tokens += tokens;
            if let Value::Object(ref mut map) = item {
                if !map.contains_key("source") {
                    map.insert("source".to_string(), Value::String(source.to_string()));
                }
                map.insert("tokens".to_string(), Value::Number((tokens as u64).into()));
            }
            results.push(item);
        } else {
            results.push(json!({
                "source": source,
                "text": null,
                "type": "not_found",
                "tokens": 0,
            }));
        }
    }

    json_response(
        StatusCode::OK,
        json!({
            "results": results,
            "totalTokens": total_tokens,
            "count": results.iter().filter(|r| r["type"] != "not_found").count(),
        }),
    )
}

/// Look up the full text of a single source string (team visibility applied when `ctx.team_mode`).
pub fn unfold_source(conn: &Connection, source: &str, ctx: &RecallContext) -> Option<Value> {
    if let Some(crystal_id) = parse_crystal_source_id(source) {
        if let Some((label, consolidated_text, member_count, owner_id, visibility)) =
            query_crystal_for_unfold(conn, crystal_id)
        {
            if is_visible(owner_id, visibility.as_deref(), ctx) {
                let members = crystal_member_sources(conn, crystal_id, ctx);
                let mut full_text = consolidated_text.clone();
                if !members.is_empty() {
                    full_text.push_str("\n\nFamily members:\n");
                    for member in members.iter().take(16) {
                        full_text.push_str("- ");
                        full_text.push_str(member);
                        full_text.push('\n');
                    }
                    if member_count as usize > members.len() {
                        full_text.push_str(&format!(
                            "... plus {} more hidden or archived member(s)",
                            (member_count as usize).saturating_sub(members.len())
                        ));
                    }
                }
                return Some(json!({
                    "source": crystal_source(crystal_id, &label),
                    "text": full_text.trim_end().to_string(),
                    "type": "crystal",
                    "label": label,
                    "clusterId": crystal_id,
                    "members": members,
                    "memberCount": member_count,
                }));
            }
        }
    }

    if let Some((text, ty, owner_id, visibility)) = query_memory_for_unfold(conn, source) {
        if is_visible(owner_id, visibility.as_deref(), ctx) {
            return Some(json!({"text": text, "type": ty}));
        }
    }

    if let Some(id_str) = source.strip_prefix("decision::") {
        if let Ok(id) = id_str.parse::<i64>() {
            if let Some((decision, context, owner_id, visibility)) =
                query_decision_by_id_for_unfold(conn, id)
            {
                if is_visible(owner_id, visibility.as_deref(), ctx) {
                    let full = match context {
                        Some(c) => format!("{decision}\n\nContext: {c}"),
                        None => decision,
                    };
                    return Some(json!({"text": full, "type": "decision"}));
                }
            }
        }
    }

    if let Some((decision, context, owner_id, visibility)) =
        query_decision_by_context_for_unfold(conn, source)
    {
        if is_visible(owner_id, visibility.as_deref(), ctx) {
            let full = match context {
                Some(c) => format!("{decision}\n\nContext: {c}"),
                None => decision,
            };
            return Some(json!({"text": full, "type": "decision"}));
        }
    }

    let stripped = source.strip_prefix("memory::").unwrap_or(source);
    if stripped != source {
        if let Some((text, ty, owner_id, visibility)) = query_memory_for_unfold(conn, stripped) {
            if is_visible(owner_id, visibility.as_deref(), ctx) {
                return Some(json!({"text": text, "type": ty}));
            }
        }
    }

    None
}

pub(crate) type MemoryUnfoldRow = (String, String, Option<i64>, Option<String>);
pub(crate) type DecisionUnfoldRow = (String, Option<String>, Option<i64>, Option<String>);

pub(crate) fn query_memory_for_unfold(conn: &Connection, source: &str) -> Option<MemoryUnfoldRow> {
    let sql_with_visibility =
        "SELECT text, type, owner_id, visibility FROM memories WHERE source = ?1 \
         AND status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now')) \
         ORDER BY score DESC LIMIT 1";
    match conn.query_row(sql_with_visibility, params![source], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    }) {
        Ok(row) => Some(row),
        Err(err) if is_missing_team_visibility_columns(&err) => conn
            .query_row(
                "SELECT text, type FROM memories WHERE source = ?1 \
                 AND status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now')) \
                 ORDER BY score DESC LIMIT 1",
                params![source],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        None,
                        None,
                    ))
                },
            )
            .ok(),
        Err(_) => None,
    }
}

pub(crate) fn query_decision_by_id_for_unfold(conn: &Connection, id: i64) -> Option<DecisionUnfoldRow> {
    let sql_with_visibility =
        "SELECT decision, context, owner_id, visibility FROM decisions WHERE id = ?1 \
         AND status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now'))";
    match conn.query_row(sql_with_visibility, params![id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    }) {
        Ok(row) => Some(row),
        Err(err) if is_missing_team_visibility_columns(&err) => conn
            .query_row(
                "SELECT decision, context FROM decisions WHERE id = ?1 \
                 AND status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now'))",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        None,
                        None,
                    ))
                },
            )
            .ok(),
        Err(_) => None,
    }
}

pub(crate) fn query_decision_by_context_for_unfold(
    conn: &Connection,
    source: &str,
) -> Option<DecisionUnfoldRow> {
    let sql_with_visibility =
        "SELECT decision, context, owner_id, visibility FROM decisions WHERE context = ?1 \
         AND status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now')) \
         ORDER BY score DESC LIMIT 1";
    match conn.query_row(sql_with_visibility, params![source], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    }) {
        Ok(row) => Some(row),
        Err(err) if is_missing_team_visibility_columns(&err) => conn
            .query_row(
                "SELECT decision, context FROM decisions WHERE context = ?1 \
                 AND status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now')) \
                 ORDER BY score DESC LIMIT 1",
                params![source],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        None,
                        None,
                    ))
                },
            )
            .ok(),
        Err(_) => None,
    }
}

