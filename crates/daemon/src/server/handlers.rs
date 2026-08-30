use crate::handlers;
use crate::state::RuntimeState;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
pub(crate) async fn handle_compact(State(state): State<RuntimeState>, headers: HeaderMap) -> axum::response::Response {
    if let Err(resp) = handlers::ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db.lock().await;
    let result = crate::compaction::run_compaction(&conn);
    handlers::json_response(
        axum::http::StatusCode::OK,
        serde_json::json!({"eventsPruned":result.events_pruned,"benchmarkPruned"
:result.benchmark_pruned,"archivedTextStripped":result.archived_text_stripped,"expiredPruned":result.expired_pruned,
"crystalEmbeddingsPruned":result.crystal_embeddings_pruned,"clusterMembersPruned":result.cluster_members_pruned,
"feedbackAggregated":result.feedback_aggregated,"staleEmbeddingsPruned":result.stale_embeddings_pruned,"coOccurrencePruned":result
.co_occurrence_pruned,"legacyEmbeddingsMigrated":result.legacy_embeddings_migrated,"ftsOptimized":result.fts_optimized,
"bytesBefore":result.bytes_before,"bytesAfter":result.bytes_after,"savedKB":(result.bytes_before-result.bytes_after)/1024,}),
    )
}
pub(crate) async fn handle_compact_benchmark(State(state): State<RuntimeState>, headers: HeaderMap) -> axum::response::Response {
    if let Err(resp) = handlers::ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db.lock().await;
    let result = crate::compaction::purge_benchmark_artifacts(&conn);
    handlers::json_response(
        axum::http::StatusCode::OK,
        serde_json::json!({"decisionsDeleted":result
.decisions_deleted,"embeddingsDeleted":result.embeddings_deleted,"clusterMembersDeleted":result.cluster_members_deleted,
"decisionConflictsDeleted":result.decision_conflicts_deleted,"recallFeedbackDeleted":result.recall_feedback_deleted,
"coOccurrenceDeleted":result.co_occurrence_deleted,"eventsDeleted":result.events_deleted,"bytesBefore":result.bytes_before,
"bytesAfter":result.bytes_after,"savedKB":(result.bytes_before-result.bytes_after)/1024,}),
    )
}
pub(crate) async fn handle_storage(State(state): State<RuntimeState>, headers: HeaderMap) -> axum::response::Response {
    if let Err(resp) = handlers::ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db_read.lock().await;
    let breakdown = crate::compaction::storage_breakdown(&conn);
    let total_bytes: i64 = conn.query_row("PRAGMA page_count", [], |r| r.get::<_, i64>(0)).unwrap_or(0)
        * conn.query_row("PRAGMA page_size", [], |r| r.get::<_, i64>(0)).unwrap_or(4096);
    let tables: Vec<serde_json::Value> = breakdown
        .iter()
        .map(|(name, count)| {
            serde_json::json!({
"table":name,"rows":count})
        })
        .collect();
    handlers::json_response(
        axum::http::StatusCode::OK,
        serde_json::json!({"totalBytes":
total_bytes,"totalMB":format!("{:.1}",total_bytes as f64/1_048_576.0),"tables":tables,}),
    )
}
pub(crate) async fn handle_crystals(State(state): State<RuntimeState>, headers: HeaderMap) -> axum::response::Response {
    if let Err(resp) = handlers::ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db_read.lock().await;
    let crystals = crate::crystallize::list_crystals(&conn);
    handlers::json_response(axum::http::StatusCode::OK, serde_json::json!({"crystals":crystals,"count":crystals.len()}))
}
pub(crate) async fn handle_crystallize(State(state): State<RuntimeState>, headers: HeaderMap) -> axum::response::Response {
    if let Err(resp) = handlers::ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db.lock().await;
    let brain_sender = Some(state.brain_firing.clone());
    let result = crate::crystallize::run_crystallize_pass_with_brain(&conn, state.default_owner_id, &brain_sender);
    handlers::json_response(
        axum::http::StatusCode::OK,
        serde_json::json!({"clusters":result.
clusters_found,"created":result.crystals_created,"updated":result.crystals_updated,"consolidated":result.entries_consolidated,}),
    )
}
#[derive(serde::Deserialize)]
pub(crate) struct RollbackRequest {
    to: i64,
}
pub(crate) async fn handle_versions(State(state): State<RuntimeState>, headers: HeaderMap) -> axum::response::Response {
    if let Err(resp) = handlers::ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db_read.lock().await;
    let versions = crate::traces::list_versions(&conn, 100);
    let head = crate::traces::current_head(&conn);
    handlers::json_response(axum::http::StatusCode::OK, serde_json::json!({"versions":versions,"count":versions.len(),"head":head}))
}
pub(crate) async fn handle_rollback(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<RollbackRequest>) -> axum::response::Response {
    if let Err(resp) = handlers::ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db.lock().await;
    match crate::traces::rollback_to(&conn, body.to) {
        Ok((orphaned, head)) => handlers::json_response(axum::http::StatusCode::OK, serde_json::json!({"rolledBack":true,"head":head,"orphaned":orphaned})),
        Err(err) => handlers::json_response(axum::http::StatusCode::BAD_REQUEST, serde_json::json!({"error":err})),
    }
}
#[derive(serde::Deserialize)]
pub(crate) struct EntitiesQuery {
    q: Option<String>,
}
pub(crate) async fn handle_entities(
    State(state): State<RuntimeState>, headers: HeaderMap, axum::extract::Query(query): axum::extract::Query<EntitiesQuery>,
) -> axum::response::Response {
    if let Err(resp) = handlers::ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db_read.lock().await;
    let ids = query.q.as_deref().map(|q| crate::graph::resolve_query(&conn, q)).unwrap_or_default();
    let mut entities = Vec::new();
    for id in &ids {
        if let Ok(row) = conn.query_row(
            "SELECT id, canonical_name, qualifier, kind FROM entities WHERE id = ?1",
            rusqlite::params![id],
            |r| {
                Ok(serde_json::json!({"id":r.get::<_,i64>(0)?,"canonicalName":r.get::<_,String>(1)?,"qualifier":r.get::<_,String>(2)?,"kind":r.get::<_,String>(3)?}))
            },
        ) {
            entities.push(row);
        }
    }
    handlers::json_response(axum::http::StatusCode::OK, serde_json::json!({"entities":entities,"count":entities.len()}))
}
#[derive(serde::Deserialize)]
pub(crate) struct FocusRequest {
    label: Option<String>,
    agent: Option<String>,
}
pub(crate) async fn handle_focus_start(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<FocusRequest>) -> axum::response::Response {
    if let Err(resp) = handlers::ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let label = match &body.label {
        Some(l) if !l.is_empty() => l.as_str(),
        _ => {
            return handlers::json_error(axum::http::StatusCode::BAD_REQUEST, "Missing field: label");
        }
    };
    let agent = body.agent.as_deref().unwrap_or("http");
    let conn = state.db.lock().await;
    match crate::focus::focus_start(&conn, label, agent) {
        Ok(v) => handlers::json_response(axum::http::StatusCode::OK, v),
        Err(e) => handlers::json_error(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}
pub(crate) async fn handle_focus_end(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<FocusRequest>) -> axum::response::Response {
    if let Err(resp) = handlers::ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let label = match &body.label {
        Some(l) if !l.is_empty() => l.as_str(),
        _ => {
            return handlers::json_error(axum::http::StatusCode::BAD_REQUEST, "Missing field: label");
        }
    };
    let agent = body.agent.as_deref().unwrap_or("http");
    let conn = state.db.lock().await;
    match crate::focus::focus_end(&conn, label, agent, state.default_owner_id) {
        Ok(v) => handlers::json_response(axum::http::StatusCode::OK, v),
        Err(e) => handlers::json_error(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}
