// SPDX-License-Identifier: MIT
use crate::handlers::recall::*;
use crate::handlers::store::{persist_decision_embedding, store_decision_with_input_embedding};
use crate::state::RuntimeState;
use rusqlite::params;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};

static SHARED_TEST_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

struct StaticReranker;

impl crate::rerank::Reranker for StaticReranker {
    fn name(&self) -> &'static str {
        "static_test_reranker"
    }

    fn model_size_mb(&self) -> u64 {
        1
    }

    fn rerank(
        &self,
        _query: &str,
        candidates: &[crate::rerank::RerankCandidate],
        fusion_alpha: f64,
    ) -> Result<Vec<crate::rerank::RerankedScore>, String> {
        let scores = candidates
            .iter()
            .map(|candidate| {
                let score = if candidate.id == "memory::winner" {
                    10.0
                } else {
                    -10.0
                };
                (candidate.id.clone(), score)
            })
            .collect::<Vec<_>>();
        Ok(crate::rerank::fuse_scores(
            candidates,
            &scores,
            fusion_alpha,
        ))
    }
}

// ── is_visible tests ───────────────────────────────────────────

pub(crate) fn solo_ctx() -> RecallContext {
    RecallContext {
        caller_id: None,
        team_mode: false,
    }
}
pub(crate) fn team_ctx(caller: i64) -> RecallContext {
    RecallContext {
        caller_id: Some(caller),
        team_mode: true,
    }
}
fn team_ctx_no_caller() -> RecallContext {
    RecallContext {
        caller_id: None,
        team_mode: true,
    }
}

pub(crate) fn test_conn() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::configure(&conn).unwrap();
    crate::db::initialize_schema(&conn).unwrap();
    crate::db::run_pending_migrations(&conn);
    conn
}

fn shared_test_state() -> RuntimeState {
    let unique_id = SHARED_TEST_DB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let db_path = std::env::temp_dir().join(format!(
        "cortex-recall-shared-{}-{}-{}.db",
        std::process::id(),
        unique_id,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let write_conn = rusqlite::Connection::open(&db_path).unwrap();
    crate::db::configure(&write_conn).unwrap();
    crate::db::initialize_schema(&write_conn).unwrap();
    crate::db::run_pending_migrations(&write_conn);

    let read_conn = rusqlite::Connection::open(&db_path).unwrap();
    crate::db::configure(&read_conn).unwrap();
    crate::db::initialize_schema(&read_conn).unwrap();
    crate::db::run_pending_migrations(&read_conn);

    let (events, _) = broadcast::channel(8);
    let (brain_firing, _) = broadcast::channel(8);
    RuntimeState {
        db: Arc::new(Mutex::new(write_conn)),
        db_read: Arc::new(Mutex::new(read_conn)),
        token: Arc::new("test-token".to_string()),
        events,
        brain_firing,
        mcp_calls: Arc::new(AtomicU64::new(0)),
        mcp_sessions: Arc::new(Mutex::new(HashMap::new())),
        recall_history: Arc::new(Mutex::new(HashMap::new())),
        pre_cache: Arc::new(Mutex::new(HashMap::new())),
        served_content: Arc::new(Mutex::new(HashMap::new())),
        shutdown_tx: Arc::new(Mutex::new(None)),
        home: PathBuf::from("."),
        db_path: db_path.clone(),
        token_path: PathBuf::from("cortex.token"),
        pid_path: PathBuf::from("cortex.pid"),
        port: 7437,
        embedding_engine: None,
        rate_limiter: crate::rate_limit::RateLimiter::new(),
        team_mode: false,
        default_owner_id: None,
        team_api_key_hashes: Arc::new(std::sync::RwLock::new(Vec::new())),
        degraded_mode: Arc::new(AtomicBool::new(false)),
        db_corrupted: Arc::new(AtomicBool::new(false)),
        readiness: Arc::new(AtomicBool::new(true)),
        last_activity_unix_secs: Arc::new(AtomicU64::new(0)),
        write_buffer_path: PathBuf::from("write_buffer.jsonl"),
        sqlite_vec_canary: crate::state::SqliteVecCanaryConfig {
            trial_percent: 0,
            force_off: false,
            route_mode: crate::state::SqliteVecRouteMode::Trial,
        },
        rerank_config: crate::rerank::RerankConfig::off(),
        reranker: None,
    }
}

fn latest_recall_query_event(conn: &rusqlite::Connection) -> Value {
    let raw: String = conn
        .query_row(
            "SELECT data FROM events WHERE type = 'recall_query' ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("latest recall_query event should exist");
    serde_json::from_str(&raw).expect("recall_query event should be valid json")
}

fn recall_item_for_rerank(source: &str, relevance: f64) -> RecallItem {
    RecallItem {
        source: source.to_string(),
        relevance,
        excerpt: format!("rerank fixture for {source}"),
        method: "hybrid".to_string(),
        tokens: Some(10),
        entropy: Some(0.5),
        family_members: Vec::new(),
        collapsed_sources: Vec::new(),
        collapsed_source_scores: Vec::new(),
    }
}

#[test]
fn primary_rerank_reorders_top_window_and_marks_method() {
    let mut state = shared_test_state();
    state.rerank_config = crate::rerank::RerankConfig {
        mode: crate::rerank::RerankMode::Primary,
        top_n: 2,
        fusion_alpha: 0.90,
    };
    state.reranker = Some(Arc::new(StaticReranker));
    let results = vec![
        recall_item_for_rerank("memory::baseline", 0.95),
        recall_item_for_rerank("memory::winner", 0.70),
        recall_item_for_rerank("memory::outside", 0.60),
    ];

    let (reranked, route) = maybe_apply_rerank(&state, "winner query", results, 240);

    assert_eq!(route["status"], "ok");
    assert_eq!(route["mode"], "primary");
    assert_eq!(route["applied"], true);
    assert_eq!(route["baselineTopSources"][0], "memory::baseline");
    assert_eq!(route["rerankedTopSources"][0], "memory::winner");
    assert_eq!(reranked[0].source, "memory::winner");
    assert_eq!(reranked[2].source, "memory::outside");
    assert!(reranked[0].method.ends_with("+rerank"));
}

#[test]
fn shadow_rerank_reports_route_without_reordering() {
    let mut state = shared_test_state();
    state.rerank_config = crate::rerank::RerankConfig {
        mode: crate::rerank::RerankMode::Shadow,
        top_n: 2,
        fusion_alpha: 0.90,
    };
    state.reranker = Some(Arc::new(StaticReranker));
    let results = vec![
        recall_item_for_rerank("memory::baseline", 0.95),
        recall_item_for_rerank("memory::winner", 0.70),
        recall_item_for_rerank("memory::outside", 0.60),
    ];

    let (reranked, route) = maybe_apply_rerank(&state, "winner query", results, 240);

    assert_eq!(route["status"], "ok");
    assert_eq!(route["mode"], "shadow");
    assert_eq!(route["applied"], false);
    assert_eq!(route["baselineTopSources"][0], "memory::baseline");
    assert_eq!(route["rerankedTopSources"][0], "memory::winner");
    assert_eq!(reranked[0].source, "memory::baseline");
    assert!(!reranked[0].method.ends_with("+rerank"));
}

fn insert_memory_with_embedding(
    conn: &rusqlite::Connection,
    text: &str,
    source: &str,
    vector: &[f32],
) -> i64 {
    let model_key = crate::embeddings::selected_model_key();
    conn.execute(
        "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
         VALUES (?1, ?2, 'note', 'active', 1.0, datetime('now'), datetime('now'))",
        params![text, source],
    )
    .unwrap();
    let id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO embeddings (target_type, target_id, vector, model)
         VALUES ('memory', ?1, ?2, ?3)",
        params![id, crate::embeddings::vector_to_blob(vector), model_key],
    )
    .unwrap();
    id
}

fn insert_memory_with_optional_source_and_embedding(
    conn: &rusqlite::Connection,
    text: &str,
    source: Option<&str>,
    vector: &[f32],
) -> i64 {
    let model_key = crate::embeddings::selected_model_key();
    conn.execute(
        "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
         VALUES (?1, ?2, 'note', 'active', 1.0, datetime('now'), datetime('now'))",
        params![text, source],
    )
    .unwrap();
    let id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO embeddings (target_type, target_id, vector, model)
         VALUES ('memory', ?1, ?2, ?3)",
        params![id, crate::embeddings::vector_to_blob(vector), model_key],
    )
    .unwrap();
    id
}

pub(crate) fn store_decision_with_embedding(
    conn: &mut rusqlite::Connection,
    decision: &str,
    context: &str,
    vector: &[f32],
) {
    let (_, new_id) = store_decision_with_input_embedding(
        conn,
        decision,
        Some(context.to_string()),
        None,
        "tester".to_string(),
        None,
        None,
        Some(vector),
        None,
    )
    .unwrap();

    if let Some(id) = new_id {
        persist_decision_embedding(conn, id, vector, crate::embeddings::selected_model_key())
            .unwrap();
    }
}

fn insert_crystal_with_memory_members(
    conn: &rusqlite::Connection,
    label: &str,
    consolidated_text: &str,
    crystal_vector: &[f32],
    members: &[(&str, &str, &[f32])],
) -> (i64, String, Vec<String>) {
    let mut member_sources = Vec::with_capacity(members.len());
    let mut member_ids = Vec::with_capacity(members.len());
    for (text, source, vector) in members {
        let id = insert_memory_with_embedding(conn, text, source, vector);
        member_ids.push(id);
        member_sources.push((*source).to_string());
    }

    if conn
        .execute(
            "INSERT INTO memory_clusters (
                label,
                centroid,
                consolidated_text,
                member_count,
                owner_id,
                visibility,
                created_at,
                updated_at
             ) VALUES (?1, NULL, ?2, ?3, 1, 'shared', datetime('now'), datetime('now'))",
            params![label, consolidated_text, members.len() as i64],
        )
        .is_err()
    {
        conn.execute(
            "INSERT INTO memory_clusters (
                label,
                centroid,
                consolidated_text,
                member_count,
                created_at,
                updated_at
             ) VALUES (?1, NULL, ?2, ?3, datetime('now'), datetime('now'))",
            params![label, consolidated_text, members.len() as i64],
        )
        .unwrap();
    }
    let crystal_id = conn.last_insert_rowid();

    for member_id in member_ids {
        conn.execute(
            "INSERT INTO cluster_members (cluster_id, target_type, target_id, similarity)
             VALUES (?1, 'memory', ?2, 1.0)",
            params![crystal_id, member_id],
        )
        .unwrap();
    }

    conn.execute(
        "INSERT INTO embeddings (target_type, target_id, vector, model)
         VALUES ('crystal', ?1, ?2, ?3)",
        params![
            crystal_id,
            crate::embeddings::vector_to_blob(crystal_vector),
            crate::embeddings::selected_model_key()
        ],
    )
    .unwrap();

    (
        crystal_id,
        crystal_source(crystal_id, label),
        member_sources,
    )
}

