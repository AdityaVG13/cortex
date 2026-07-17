use crate::embeddings::EmbeddingEngine;
use crate::state::{BrainFiringEvent, BrainKind};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use tokio::sync::broadcast;

pub type BrainFiringSender = Option<broadcast::Sender<BrainFiringEvent>>;
pub const CRYSTAL_RELEVANCE_BOOST: f64 = 1.15;

#[derive(Debug)]
pub struct CrystallizeResult {
    pub clusters_found: usize,
    pub crystals_created: usize,
    pub crystals_updated: usize,
    pub entries_consolidated: usize,
}

pub fn run_crystallize_pass_with_brain(
    _conn: &Connection, _engine: Option<&EmbeddingEngine>, owner_id: Option<i64>, brain: &BrainFiringSender,
) -> CrystallizeResult {
    if let Some(tx) = brain {
        let _ = tx.send(BrainFiringEvent { kind: BrainKind::ConsolidationStarted, payload: json!({}), owner_id });
    }
    CrystallizeResult { clusters_found: 0, crystals_created: 0, crystals_updated: 0, entries_consolidated: 0 }
}

pub fn search_crystals_filtered(
    conn: &Connection, query_vector: &[f32], limit: usize, owner_id: Option<i64>, team_mode: bool,
) -> Vec<(i64, String, String, f64)> {
    if query_vector.is_empty() {
        return Vec::new();
    }
    let mut stmt = match conn.prepare(
        "SELECT id, label, consolidated_text, centroid, owner_id, visibility
         FROM memory_clusters
         ORDER BY updated_at DESC
         LIMIT ?1",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Vec<u8>>(3)?,
            row.get::<_, Option<i64>>(4).ok().flatten(),
            row.get::<_, Option<String>>(5).ok().flatten(),
        ))
    });
    rows.into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|(_, _, _, _, row_owner, visibility)| !team_mode || row_owner == &owner_id || matches!(visibility.as_deref(), Some("shared" | "team")))
        .map(|(id, label, text, blob, _, _)| {
            let sim = crate::embeddings::cosine_similarity(query_vector, &crate::embeddings::blob_to_vector(&blob)) as f64;
            (id, label, text, sim * CRYSTAL_RELEVANCE_BOOST)
        })
        .collect()
}

pub fn unfold_crystal(conn: &Connection, crystal_id: i64) -> Vec<String> {
    let mut stmt = match conn.prepare("SELECT source FROM cluster_members WHERE cluster_id = ?1 ORDER BY id ASC") {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };
    stmt.query_map(params![crystal_id], |row| row.get::<_, String>(0))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

pub fn list_crystals(conn: &Connection) -> Vec<Value> {
    let mut stmt = match conn.prepare("SELECT id, label, consolidated_text, member_count, updated_at FROM memory_clusters ORDER BY updated_at DESC") {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };
    stmt.query_map([], |row| {
        Ok(json!({"id":row.get::<_,i64>(0)?,"label":row.get::<_,String>(1)?,"text":row.get::<_,String>(2)?,
            "members":row.get::<_,i64>(3)?,"updatedAt":row.get::<_,String>(4)?}))
    })
    .map(|rows| rows.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

pub fn migrate_crystal_tables(conn: &Connection) {
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory_clusters (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            label TEXT NOT NULL,
            centroid BLOB NOT NULL DEFAULT X'',
            consolidated_text TEXT NOT NULL,
            member_count INTEGER NOT NULL DEFAULT 0,
            owner_id INTEGER,
            visibility TEXT NOT NULL DEFAULT 'private',
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS cluster_members (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            cluster_id INTEGER NOT NULL,
            source TEXT NOT NULL,
            target_type TEXT,
            target_id INTEGER,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_cluster_members_cluster ON cluster_members(cluster_id);
        CREATE INDEX IF NOT EXISTS idx_memory_clusters_updated ON memory_clusters(updated_at);",
    );
}
