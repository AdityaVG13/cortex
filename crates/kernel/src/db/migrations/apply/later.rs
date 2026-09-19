use super::super::{
    add_twin_columns, ensure_temporal_columns, fill_twins, migration_error, require_columns,
    require_tables,
};
use crate::db::rebuild_fts;
use crate::db::schema::{
    AGENT_FEEDBACK_DDL, CLIENT_PERMISSIONS_DDL, DECISION_CONFLICTS_DDL,
    IDX_ACTIVE_SOURCE_RECENT_SQL, IDX_EMBEDDINGS_MODEL_NORM_SQL, fts_rebuild_sql,
};
use rusqlite::Connection;

fn exec_batch(conn: &Connection, sql: &str) -> rusqlite::Result<()> {
    conn.execute_batch(sql)?;
    Ok(())
}

fn exec_twins(conn: &Connection, sql: &str) -> rusqlite::Result<()> {
    for table in ["memories", "decisions"] {
        exec_batch(conn, &sql.replace("{table}", table))?;
    }
    Ok(())
}

fn twin_index(conn: &Connection, suffix: &str, columns: &str) -> rusqlite::Result<()> {
    exec_twins(
        conn,
        &format!("CREATE INDEX IF NOT EXISTS idx_{{table}}_{suffix} ON {{table}}{columns}"),
    )
}

pub(super) fn apply_later(
    conn: &Connection,
    version: &str,
    log_success: bool,
) -> rusqlite::Result<()> {
    match version {
        "008" => exec_batch(conn, CLIENT_PERMISSIONS_DDL),
        "009" => {
            add_twin_columns(
                conn,
                &[
                    ("source_client", "TEXT DEFAULT 'unknown'"),
                    ("source_model", "TEXT"),
                    ("reasoning_depth", "TEXT DEFAULT 'single-shot'"),
                    ("trust_score", "REAL DEFAULT 0.8"),
                ],
            )?;
            fill_twins(
                conn,
                &[
                    "UPDATE {table} SET source_client = COALESCE(NULLIF(lower(source_agent), ''), 'unknown') WHERE source_client IS NULL OR source_client = ''",
                    "UPDATE {table} SET trust_score = COALESCE(confidence, 0.8) WHERE trust_score IS NULL",
                ],
            );
            Ok(())
        }
        "010" => exec_batch(conn, DECISION_CONFLICTS_DDL),
        "011" => exec_batch(conn, AGENT_FEEDBACK_DDL),
        "012" => {
            exec_batch(conn, &fts_rebuild_sql())?;
            rebuild_fts(conn)?;
            Ok(())
        }
        "013" => exec_batch(conn, IDX_EMBEDDINGS_MODEL_NORM_SQL),
        "015" => exec_batch(
            conn,
            "CREATE TABLE IF NOT EXISTS boot_audits (id INTEGER PRIMARY KEY AUTOINCREMENT, agent TEXT NOT NULL, profile TEXT NOT NULL, budget_tokens INTEGER NOT NULL, token_estimate INTEGER NOT NULL, token_savings INTEGER NOT NULL DEFAULT 0, capsules_count INTEGER NOT NULL DEFAULT 0, capsules_json TEXT, latency_ms INTEGER, created_at TEXT NOT NULL DEFAULT (datetime('now'))); CREATE INDEX IF NOT EXISTS idx_boot_audits_created_at ON boot_audits(created_at); CREATE INDEX IF NOT EXISTS idx_boot_audits_agent_created ON boot_audits(agent, created_at);",
        ),
        "014" => ensure_temporal_columns(conn),
        "016" => {
            add_twin_columns(
                conn,
                &[("retention_class", "TEXT NOT NULL DEFAULT 'operational'")],
            )?;
            fill_twins(
                conn,
                &[
                    "UPDATE {table} SET retention_class = 'operational' WHERE retention_class IS NULL OR retention_class = '' OR retention_class NOT IN ('durable', 'operational', 'audit', 'ephemeral')",
                ],
            );
            twin_index(conn, "retention_class", "(retention_class)")
        }
        "017" => exec_batch(conn, IDX_ACTIVE_SOURCE_RECENT_SQL),
        "018" => {
            add_twin_columns(conn, &[("owner_id", "INTEGER"), ("visibility", "TEXT")])?;
            twin_index(conn, "owner", "(owner_id) WHERE owner_id IS NOT NULL")
        }
        "019" => {
            crate::traces::migrate_history_tables(conn)?;
            require_tables(
                conn,
                &["traces", "versions"],
                "history migration did not create traces/versions/version_id",
            )?;
            require_columns(
                conn,
                &[("decisions", "version_id")],
                "history migration did not create traces/versions/version_id",
            )
        }
        "020" => {
            crate::graph::migrate_entity_tables(conn)?;
            require_tables(
                conn,
                &["entities", "entity_aliases"],
                "entity migration did not create entities/entity_aliases",
            )
        }
        "021" => {
            ensure_temporal_columns(conn)?;
            let now = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";
            exec_twins(
                conn,
                &format!(
                    "UPDATE {{table}} SET observed_at = COALESCE(observed_at, created_at, {now}), valid_from = COALESCE(valid_from, observed_at, created_at, {now}) WHERE observed_at IS NULL OR valid_from IS NULL"
                ),
            )?;
            exec_batch(
                conn,
                "UPDATE memories SET valid_until = COALESCE(updated_at, observed_at, valid_from) WHERE status = 'superseded' AND valid_until IS NULL; UPDATE decisions SET valid_until = COALESCE((SELECT MIN(COALESCE(newer.valid_from, newer.observed_at, newer.created_at)) FROM decisions newer WHERE newer.supersedes_id = decisions.id), updated_at, observed_at, valid_from) WHERE status = 'superseded' AND valid_until IS NULL;",
            )?;
            Ok(())
        }
        "022_clock_anchors" => {
            crate::clockwork::migrate_clock_tables(conn)?;
            crate::db::ensure_column(
                conn,
                "recall_feedback",
                "ALTER TABLE recall_feedback ADD COLUMN query_signature TEXT",
            )?;
            Ok(())
        }
        "023_authoritative_records" => {
            // Additive: records/revisions/heads + legacy import as baseline
            // revisions with labeled provenance gaps. No legacy row is rewritten.
            crate::db::records::ensure_authoritative_schema(conn)?;
            add_twin_columns(
                conn,
                &[("trust_basis", "TEXT DEFAULT 'legacy_model_weight'")],
            )?;
            let imported = crate::db::records::import_legacy(conn)?;
            if log_success && imported > 0 {
                eprintln!(
                    "[db] Migration applied: imported {imported} legacy rows as baseline revisions"
                );
            }
            Ok(())
        }
        "024_boot_capsule_indexes" => {
            // The constraints capsule keys its cache on MAX(updated_at) and
            // scans durable active decisions oldest-first; both need indexes
            // to stay off the boot hot path.
            exec_batch(
                conn,
                "CREATE INDEX IF NOT EXISTS idx_decisions_updated_at ON decisions(updated_at); CREATE INDEX IF NOT EXISTS idx_decisions_status_retention_created ON decisions(status, retention_class, created_at, id);",
            )
        }
        other => Err(migration_error(format!(
            "unknown schema migration: {other}"
        ))),
    }
}
