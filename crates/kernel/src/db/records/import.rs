use super::{NewRevision, append_commit, append_revision, ensure_authoritative_schema};
use rusqlite::{Connection, params};
use serde_json::json;

/// Import every legacy row that has no record yet as a baseline revision.
/// Timestamps become observed metadata; provenance gaps are labeled, never
/// manufactured. Returns the number of records created.
pub fn import_legacy(conn: &Connection) -> rusqlite::Result<usize> {
    ensure_authoritative_schema(conn)?;
    let mut created = 0usize;
    let mut sequence: Option<i64> = None;
    for (table, kind, text_col) in [
        ("decisions", "decision", "decision"),
        ("memories", "memory", "text"),
    ] {
        // Column-tolerant: this migration may run on a database whose later
        // legacy columns (retention_class, trust_score, version_id) do not
        // exist yet; missing columns are reported as absent, not invented.
        let has = |col: &str| crate::db::table_has_column(conn, table, col);
        let retention_expr = if has("retention_class") {
            "COALESCE(t.retention_class, 'operational')"
        } else {
            "'operational'"
        };
        let trust_expr = if has("trust_score") {
            "COALESCE(t.trust_score, 0.8)"
        } else {
            "0.8"
        };
        let version_expr = if has("version_id") {
            "t.version_id"
        } else {
            "NULL"
        };
        let sql = format!(
            "SELECT t.id, t.{text_col}, t.status, t.source_agent, t.created_at, {retention_expr}, {trust_expr}, {version_expr} FROM {table} t WHERE NOT EXISTS (SELECT 1 FROM addresses a WHERE a.scheme = 'legacy' AND a.namespace = ?1 AND a.address = CAST(t.id AS TEXT)) ORDER BY t.id"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows: Vec<(
            i64,
            String,
            String,
            String,
            String,
            String,
            f64,
            Option<i64>,
        )> = stmt
            .query_map(params![kind], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            })?
            .collect::<Result<_, _>>()?;
        for (id, text, status, agent, created_at, retention, trust, version_id) in rows {
            let seq = match sequence {
                Some(s) => s,
                None => {
                    let s = append_commit(conn, "system:legacy-import", None, "process_crash")?;
                    sequence = Some(s);
                    s
                }
            };
            let record_id = format!("{kind}:{id}");
            let retention =
                if ["durable", "operational", "audit", "ephemeral"].contains(&retention.as_str()) {
                    retention
                } else {
                    "operational".to_string()
                };
            let body = json!({"text":text,"legacy":{"table":table,"id":id,"status":status,"agent":agent,"observed_created_at":created_at,"trust_score":trust,"trust_basis":"legacy_model_weight","version_id":version_id},"provenance_gap":"imported from a pre-revision schema: no source lineage, validity or verification was recorded"});
            append_revision(
                conn,
                seq,
                NewRevision {
                    record_id: &record_id,
                    kind,
                    retention: &retention,
                    body,
                    epistemic_status: "asserted",
                    parents: &[],
                    replace_parents: false,
                    representation_version: "legacy-import/1",
                },
            )?;
            conn.execute("INSERT OR IGNORE INTO addresses (scheme, namespace, address, record_id) VALUES ('legacy', ?1, ?2, ?3)", params![kind, id.to_string(), record_id])?;
            created += 1;
        }
    }
    Ok(created)
}
