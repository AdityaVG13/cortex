//! Authorized erasure: retained content is removed across sources,
//! revisions, legacy rows, projections, cached Views and aliases; a tombstone
//! keeps the minimum metadata; the erasure is appended to a home-local
//! ledger so a restore reconciles the erasure floor *first*. What cannot be
//! retracted externally (delivered exports, disconnected replicas) is
//! disclosed, never hidden.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const LEDGER_FILE: &str = "erasures.ledger.jsonl";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErasureRecord {
    pub erasure_id: String,
    /// The brain the erasure was issued on; a ledger never applies to another brain.
    #[serde(default)]
    pub brain_id: String,
    pub record_id: String,
    pub authority: String,
    pub reason: String,
    pub sequence: i64,
    pub erased_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErasureReport {
    pub erasure: ErasureRecord,
    pub revisions_tombstoned: usize,
    pub sources_erased: usize,
    pub legacy_rows_erased: usize,
    pub projections_dropped: usize,
    pub views_revoked: usize,
    pub aliases_revoked: usize,
    pub erasure_floor: i64,
    /// Places the erasure cannot reach from here; disclosed to the caller.
    pub not_retractable: Vec<String>,
}

pub fn ledger_path(home: &Path) -> PathBuf {
    home.join(LEDGER_FILE)
}

pub fn read_ledger(home: &Path) -> Vec<ErasureRecord> {
    std::fs::read_to_string(ledger_path(home))
        .map(|s| {
            s.lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn append_ledger(home: &Path, record: &ErasureRecord) -> Result<(), String> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(ledger_path(home))
        .map_err(|e| e.to_string())?;
    writeln!(
        f,
        "{}",
        serde_json::to_string(record).map_err(|e| e.to_string())?
    )
    .map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())
}

pub fn erasure_floor(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT erasure_floor FROM brain_meta WHERE singleton = 1",
        [],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
    .and_then(|s| s.parse().ok())
    .unwrap_or(0)
}

fn legacy_target(conn: &Connection, record_id: &str) -> Option<(String, i64)> {
    conn.query_row("SELECT namespace, address FROM addresses WHERE record_id = ?1 AND scheme = 'legacy' LIMIT 1", [record_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })
    .optional()
    .ok()
    .flatten()
    .and_then(|(ns, addr)| addr.parse::<i64>().ok().map(|id| (ns, id)))
}

/// Apply the erasure to every derived and retained representation. Idempotent.
fn apply(
    conn: &Connection,
    record_id: &str,
    erasure_id: &str,
) -> Result<(usize, usize, usize, usize, usize, usize), String> {
    let tombstone = json!({"erased": true, "erasure_id": erasure_id}).to_string();
    let revisions = conn
        .execute(
            "UPDATE revisions SET body_json = ?1, epistemic_status = 'retracted' WHERE record_id = ?2 AND json_extract(body_json, '$.erased') IS NOT 1",
            params![tombstone, record_id],
        )
        .map_err(|e| e.to_string())?;
    let sources = conn
        .execute(
            "UPDATE sources SET inline_payload = NULL, provider_locator = NULL, availability = 'erased' WHERE availability != 'erased' AND source_id IN (SELECT rs.source_id FROM revision_sources rs JOIN revisions r ON r.revision_id = rs.revision_id WHERE r.record_id = ?1)",
            params![record_id],
        )
        .map_err(|e| e.to_string())?;
    let mut legacy_rows = 0usize;
    let mut projections = 0usize;
    if let Some((namespace, id)) = legacy_target(conn, record_id) {
        let (table, col) = match namespace.as_str() {
            "decision" => ("decisions", "decision"),
            "memory" => ("memories", "text"),
            _ => ("decisions", "decision"),
        };
        legacy_rows += conn
            .execute(
                &format!(
                    "UPDATE {table} SET {col} = '[erased]', context = NULL, status = 'erased', compressed_text = NULL WHERE id = ?1 AND status != 'erased'"
                ),
                params![id],
            )
            .map_err(|e| e.to_string())?;
        projections += conn
            .execute(
                "DELETE FROM clock_anchor_evidence WHERE target_type = ?1 AND target_id = ?2",
                params![namespace, id],
            )
            .unwrap_or(0);
        projections += conn
            .execute("DELETE FROM clock_links WHERE (src_type = ?1 AND src_id = ?2) OR (dst_type = ?1 AND dst_id = ?2)", params![namespace, id])
            .unwrap_or(0);
        projections += conn
            .execute(
                "DELETE FROM entity_mentions WHERE target_type = ?1 AND target_id = ?2",
                params![namespace, id],
            )
            .unwrap_or(0);
        if table == "decisions" {
            let _ = conn.execute(
                "INSERT INTO decisions_fts(decisions_fts, rowid, decision, context) SELECT 'delete', id, '[erased]', NULL FROM decisions WHERE id = ?1",
                params![id],
            );
        }
    }
    // persist_receipt stores `{profile, cards: N}`, not the record id. LIKE on
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
        views += conn
            .execute(
                "DELETE FROM view_receipts WHERE receipt_id = ?1 AND NOT EXISTS (SELECT 1 FROM view_aliases WHERE receipt_id = ?1)",
                params![receipt_id],
            )
            .map_err(|e| e.to_string())?;
    }
    let _ = conn.execute("DELETE FROM compiled_reads WHERE 1 = 1 AND EXISTS (SELECT 1 FROM records WHERE record_id = ?1)", params![record_id]);
    super::compiled::bump_guard(conn, super::records::DEFAULT_SCOPE, "*")
        .map_err(|e| e.to_string())?;
    Ok((revisions, sources, legacy_rows, projections, views, aliases))
}

/// Erase one record with authority. Appends the erasure to the ledger before
/// touching data, so a crash between the two leaves a replayable intent.
pub fn erase(
    conn: &Connection,
    home: &Path,
    record_id: &str,
    authority: &str,
    reason: &str,
) -> Result<ErasureReport, String> {
    if authority.trim().is_empty() {
        return Err("erasure requires an authority".into());
    }
    super::records::ensure_authoritative_schema(conn).map_err(|e| e.to_string())?;
    let exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM records WHERE record_id = ?1",
            [record_id],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .unwrap_or(false);
    if !exists {
        return Err(format!("unknown record {record_id}"));
    }
    let ack = crate::runtime::ack_profile_label_pub(&crate::store_spi::sqlite::ack_profile(conn));
    let sp = crate::db::SqliteSavepoint::enter(conn, "erase").map_err(|e| e.to_string())?;
    let result = (|| {
        let sequence =
            super::records::append_commit(conn, authority, None, ack).map_err(|e| e.to_string())?;
        let erasure_id = format!("erasure:{record_id}@{sequence}");
        let record = ErasureRecord {
            erasure_id: erasure_id.clone(),
            brain_id: super::records::brain_epochs(conn).0,
            record_id: record_id.to_string(),
            authority: authority.to_string(),
            reason: reason.to_string(),
            sequence,
            erased_at: chrono::Utc::now().to_rfc3339(),
        };
        append_ledger(home, &record)?;
        conn.execute(
            "INSERT INTO erasures (erasure_id, scope_id, target_descriptor, sequence, propagation_state) VALUES (?1, ?2, ?3, ?4, 'local')",
            params![
                erasure_id,
                super::records::DEFAULT_SCOPE,
                json!({"record_id": record_id, "authority": authority, "reason": reason}).to_string(),
                sequence
            ],
        )
        .map_err(|e| e.to_string())?;
        let (revisions, sources, legacy_rows, projections, views, aliases) =
            apply(conn, record_id, &erasure_id)?;
        conn.execute(
            "UPDATE brain_meta SET erasure_floor = ?1 WHERE singleton = 1",
            params![sequence.to_string()],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO change_items (sequence, ordinal, scope_id, record_id, change_kind) VALUES (?1, 0, ?2, ?3, 'erased')",
            params![sequence, super::records::DEFAULT_SCOPE, record_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(ErasureReport {
            erasure: record,
            revisions_tombstoned: revisions,
            sources_erased: sources,
            legacy_rows_erased: legacy_rows,
            projections_dropped: projections,
            views_revoked: views,
            aliases_revoked: aliases,
            erasure_floor: sequence,
            not_retractable: vec![
                "exports already delivered to callers".into(),
                "replicas not connected at erasure time (propagation_state=local)".into(),
                "host contexts that already received a View".into(),
            ],
        })
    })();
    match result {
        Ok(report) => {
            sp.release().map_err(|e| e.to_string())?;
            Ok(report)
        }
        Err(err) => Err(err),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reconciliation {
    pub ledger_entries: usize,
    pub reapplied: usize,
    pub already_erased: usize,
    pub floor_before: i64,
    pub floor_after: i64,
    /// Ledger entries issued on a different brain: never applied here.
    pub foreign: usize,
    /// True when the home ledger has entries this restored database cannot
    /// account for (foreign brain, or no erasure rows and nothing to
    /// re-apply): the restore is quarantined until an operator reconciles.
    pub quarantined: bool,
}

/// Restore reconciliation: the erasure floor is re-established from the
/// home ledger before anything else is trusted. A restored backup can never
/// resurrect an erased record.
pub fn reconcile_after_restore(conn: &Connection, home: &Path) -> Result<Reconciliation, String> {
    super::records::ensure_authoritative_schema(conn).map_err(|e| e.to_string())?;
    let ledger = read_ledger(home);
    let floor_before = erasure_floor(conn);
    let db_has_ledger: i64 = conn
        .query_row("SELECT COUNT(*) FROM erasures", [], |r| r.get(0))
        .unwrap_or(0);
    let quarantined = db_has_ledger == 0 && !ledger.is_empty();
    let brain_id = super::records::brain_epochs(conn).0;
    let mut reapplied = 0usize;
    let mut already = 0usize;
    let mut foreign = 0usize;
    let mut floor_after = floor_before;
    for entry in &ledger {
        if !entry.brain_id.is_empty() && entry.brain_id != brain_id {
            foreign += 1;
            continue;
        }
        let present: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM erasures WHERE erasure_id = ?1",
                [&entry.erasure_id],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .unwrap_or(false);
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM records WHERE record_id = ?1",
                [&entry.record_id],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .unwrap_or(false);
        if present {
            already += 1;
        } else if exists {
            let ack =
                crate::runtime::ack_profile_label_pub(&crate::store_spi::sqlite::ack_profile(conn));
            let sequence = super::records::append_commit(
                conn,
                &format!("reconcile:{}", entry.authority),
                None,
                ack,
            )
            .map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT OR IGNORE INTO erasures (erasure_id, scope_id, target_descriptor, sequence, propagation_state) VALUES (?1, ?2, ?3, ?4, 'reconciled')",
                params![
                    entry.erasure_id,
                    super::records::DEFAULT_SCOPE,
                    json!({"record_id": entry.record_id, "authority": entry.authority, "reason": entry.reason, "reconciled": true}).to_string(),
                    sequence
                ],
            )
            .map_err(|e| e.to_string())?;
            apply(conn, &entry.record_id, &entry.erasure_id)?;
            reapplied += 1;
        }
        floor_after = floor_after.max(entry.sequence);
    }
    if floor_after != floor_before {
        conn.execute(
            "UPDATE brain_meta SET erasure_floor = ?1 WHERE singleton = 1",
            params![floor_after.to_string()],
        )
        .map_err(|e| e.to_string())?;
    }
    let quarantined = foreign > 0 || (quarantined && reapplied == 0 && already == 0);
    Ok(Reconciliation {
        ledger_entries: ledger.len(),
        reapplied,
        already_erased: already,
        floor_before,
        floor_after,
        foreign,
        quarantined,
    })
}

/// Revocation fence: a cached View or alias minted at `through_sequence` is
/// only deliverable when no erasure happened after it. Stronger deployments
/// serialize this check; disconnected replicas get no instant-revocation claim.
pub fn fence_check(conn: &Connection, through_sequence: i64) -> Result<(), Value> {
    let floor = erasure_floor(conn);
    if through_sequence < floor {
        Err(
            json!({"status": "resnapshot_required", "error": "revocation fence: an erasure happened after this View was minted", "erasure_floor": floor, "through_sequence": through_sequence}),
        )
    } else {
        Ok(())
    }
}

pub fn is_erased(conn: &Connection, record_id: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM erasures WHERE json_extract(target_descriptor, '$.record_id') = ?1",
        [record_id],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .unwrap_or(false)
}
