use super::legacy_target;
use rusqlite::{Connection, params};
use serde_json::json;

/// Apply the erasure to every derived and retained representation. Idempotent.
pub(super) fn apply(
    conn: &Connection,
    record_id: &str,
    erasure_id: &str,
) -> Result<(usize, usize, usize, usize, usize, usize), String> {
    let tombstone = json!({"erased": true, "erasure_id": erasure_id}).to_string();
    let revisions = conn.execute("UPDATE revisions SET body_json = ?1, epistemic_status = 'retracted' WHERE record_id = ?2 AND json_extract(body_json, '$.erased') IS NOT 1", params![tombstone, record_id]).map_err(|e| e.to_string())?;
    let sources = conn.execute("UPDATE sources SET inline_payload = NULL, provider_locator = NULL, availability = 'erased' WHERE availability != 'erased' AND source_id IN (SELECT rs.source_id FROM revision_sources rs JOIN revisions r ON r.revision_id = rs.revision_id WHERE r.record_id = ?1)", params![record_id]).map_err(|e| e.to_string())?;
    let mut legacy_rows = 0usize;
    let mut projections = 0usize;
    if let Some((namespace, id)) = legacy_target(conn, record_id)? {
        let (table, col) = match namespace.as_str() {
            "decision" => ("decisions", "decision"),
            "memory" => ("memories", "text"),
            other => {
                return Err(format!(
                    "legacy address namespace {other} is not a memories/decisions table"
                ));
            }
        };
        let extra = if crate::db::table_has_column(conn, table, "compressed_text") {
            ", compressed_text = NULL"
        } else {
            ""
        };
        let side = if namespace == "decision" {
            ", context = NULL"
        } else {
            ", tags = NULL"
        };
        legacy_rows += conn.execute(&format!("UPDATE {table} SET {col} = '[erased]'{side}{extra}, status = 'erased' WHERE id = ?1 AND status != 'erased'"), params![id]).map_err(|e| e.to_string())?;
        projections += delete_if_present(
            conn,
            "clock_anchor_evidence",
            "DELETE FROM clock_anchor_evidence WHERE target_type = ?1 AND target_id = ?2",
            params![namespace, id],
        )?;
        projections += delete_if_present(
            conn,
            "clock_links",
            "DELETE FROM clock_links WHERE (src_type = ?1 AND src_id = ?2) OR (dst_type = ?1 AND dst_id = ?2)",
            params![namespace, id],
        )?;
        projections += delete_if_present(
            conn,
            "entity_mentions",
            "DELETE FROM entity_mentions WHERE target_type = ?1 AND target_id = ?2",
            params![namespace, id],
        )?;
        if table == "decisions" {
            fts_delete_if_present(
                conn,
                "decisions_fts",
                "INSERT INTO decisions_fts(decisions_fts, rowid, decision, context) SELECT 'delete', id, '[erased]', NULL FROM decisions WHERE id = ?1",
                params![id],
            )?;
        } else {
            fts_delete_if_present(
                conn,
                "memories_fts",
                "INSERT INTO memories_fts(memories_fts, rowid, text, source, tags) SELECT 'delete', id, '[erased]', NULL, NULL FROM memories WHERE id = ?1",
                params![id],
            )?;
        }
    }
    // persist_receipt stores `{profile, cards: N, need}`, not the record id. LIKE on
    // receipt_json therefore leaves production receipts in place and, on the
    // test shape, also matches unrelated JSON keys / prefix ids.
    let receipt_ids: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT DISTINCT receipt_id FROM view_aliases WHERE record_id = ?1")
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![record_id], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };
    let aliases = conn
        .execute(
            "DELETE FROM view_aliases WHERE record_id = ?1",
            params![record_id],
        )
        .map_err(|e| e.to_string())?;
    let mut views = 0usize;
    for receipt_id in receipt_ids {
        views += conn.execute("DELETE FROM view_receipts WHERE receipt_id = ?1 AND NOT EXISTS (SELECT 1 FROM view_aliases WHERE receipt_id = ?1)", params![receipt_id]).map_err(|e| e.to_string())?;
    }
    delete_if_present(
        conn,
        "compiled_reads",
        "DELETE FROM compiled_reads WHERE 1 = 1 AND EXISTS (SELECT 1 FROM records WHERE record_id = ?1)",
        params![record_id],
    )?;
    crate::db::compiled::bump_guard(conn, crate::db::records::DEFAULT_SCOPE, "*")
        .map_err(|e| e.to_string())?;
    Ok((revisions, sources, legacy_rows, projections, views, aliases))
}

fn delete_if_present(
    conn: &Connection,
    table: &str,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<usize, String> {
    if !crate::db::table_exists(conn, table) {
        return Ok(0);
    }
    conn.execute(sql, params).map_err(|e| e.to_string())
}

fn fts_delete_if_present(
    conn: &Connection,
    table: &str,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<(), String> {
    delete_if_present(conn, table, sql, params).map(|_| ())
}
