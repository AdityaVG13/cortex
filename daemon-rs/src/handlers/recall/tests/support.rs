// SPDX-License-Identifier: MIT
use crate::handlers::recall::*;
use crate::handlers::store::{persist_decision_embedding, store_decision_with_input_embedding};

pub(crate) fn solo_ctx() -> RecallContext {
    RecallContext { caller_id: None, team_mode: false }
}

pub(crate) fn team_ctx(caller: i64) -> RecallContext {
    RecallContext { caller_id: Some(caller), team_mode: true }
}

pub(crate) fn test_conn() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::configure(&conn).unwrap();
    crate::db::initialize_schema(&conn).unwrap();
    crate::db::run_pending_migrations(&conn);
    conn
}

pub(crate) fn store_decision_with_embedding(conn: &mut rusqlite::Connection, decision: &str, context: &str, vector: &[f32]) {
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
        persist_decision_embedding(conn, id, vector, crate::embeddings::selected_model_key()).unwrap();
    }
}
