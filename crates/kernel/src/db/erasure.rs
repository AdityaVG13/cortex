//! Authorized erasure: retained content is removed across sources,
//! revisions, legacy rows, projections, cached Views and aliases; a tombstone
//! keeps the minimum metadata; the erasure is appended to a home-local
//! ledger so a restore reconciles the erasure floor *first*. What cannot be
//! retracted externally (delivered exports, disconnected replicas) is
//! disclosed, never hidden.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub const LEDGER_FILE: &str = "erasures.ledger.jsonl";
/// Home-local jsonl. Restore must ingest it, but a planted giant file
/// must not OOM the reconciliation path; fail closed instead of skipping.
pub const MAX_ERASURE_LEDGER_BYTES: u64 = 16 * 1024 * 1024;

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

fn id_present(conn: &Connection, sql: &str, id: &str, err: &str) -> Result<bool, String> {
    conn.query_row(sql, [id], |r| r.get::<_, i64>(0))
        .map(|n| n > 0)
        .map_err(|e| format!("{err}: {e}"))
}

mod ledger;
pub use ledger::read_ledger;
use ledger::{append_ledger, parse_or_fail};

fn floor_raw(conn: &Connection) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT erasure_floor FROM brain_meta WHERE singleton = 1",
        [],
        |r| r.get::<_, String>(0),
    )
    .optional()
}

pub fn erasure_floor(conn: &Connection) -> i64 {
    floor_raw(conn)
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn legacy_target(conn: &Connection, record_id: &str) -> Result<Option<(String, i64)>, String> {
    // SQL error is not "no legacy row": that would tombstone the record while
    // leaving memories/decisions plaintext in place. Unreadable or
    // non-integer locators fail closed so erase cannot report success.
    let row = conn.query_row("SELECT namespace, address FROM addresses WHERE record_id = ?1 AND scheme = 'legacy' LIMIT 1", [record_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).optional().map_err(|e| format!("legacy address lookup failed: {e}"))?;
    row.map(|(namespace, addr)| {
        addr.parse::<i64>()
            .map(|id| (namespace, id))
            .map_err(|_| format!("legacy address `{addr}` for {record_id} is not an integer id"))
    })
    .transpose()
}

mod apply;
use apply::apply;
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
    if !id_present(
        conn,
        "SELECT COUNT(*) FROM records WHERE record_id = ?1",
        record_id,
        "record lookup failed",
    )? {
        return Err(format!("unknown record {record_id}"));
    }
    let sp = crate::db::SqliteSavepoint::enter(conn, "erase").map_err(|e| e.to_string())?;
    let result = (|| {
        let sequence = super::records::append_ack_commit(conn, authority)?;
        let erasure_id = format!("erasure:{record_id}@{sequence}");
        let record = ErasureRecord {
            erasure_id: erasure_id.clone(),
            brain_id: super::records::try_brain_epochs(conn)
                .map_err(|e| e.to_string())?
                .0,
            record_id: record_id.to_string(),
            authority: authority.to_string(),
            reason: reason.to_string(),
            sequence,
            erased_at: chrono::Utc::now().to_rfc3339(),
        };
        append_ledger(home, &record)?;
        conn.execute("INSERT INTO erasures (erasure_id, scope_id, target_descriptor, sequence, propagation_state) VALUES (?1, ?2, ?3, ?4, 'local')", params![erasure_id, super::records::DEFAULT_SCOPE, json!({"record_id": record_id, "authority": authority, "reason": reason}).to_string(), sequence]).map_err(|e| e.to_string())?;
        let (revisions, sources, legacy_rows, projections, views, aliases) =
            apply(conn, record_id, &erasure_id)?;
        conn.execute(
            "UPDATE brain_meta SET erasure_floor = ?1 WHERE singleton = 1",
            params![sequence.to_string()],
        )
        .map_err(|e| e.to_string())?;
        conn.execute("INSERT INTO change_items (sequence, ordinal, scope_id, record_id, change_kind) VALUES (?1, 0, ?2, ?3, 'erased')", params![sequence, super::records::DEFAULT_SCOPE, record_id]).map_err(|e| e.to_string())?;
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
    let ledger = parse_or_fail(home)?;
    let floor_before = match floor_raw(conn) {
        Ok(Some(raw)) => raw
            .parse::<i64>()
            .map_err(|_| "erasure floor is not an integer".to_string())?,
        Ok(None) => 0,
        Err(err) => return Err(format!("erasure floor unreadable: {err}")),
    };
    let db_has_ledger: i64 = conn
        .query_row("SELECT COUNT(*) FROM erasures", [], |r| r.get(0))
        .map_err(|e| format!("erasures table unreadable: {e}"))?;
    let quarantined = db_has_ledger == 0 && !ledger.is_empty();
    let brain_id = super::records::try_brain_epochs(conn)
        .map_err(|e| format!("brain_id unreadable: {e}"))?
        .0;
    let mut reapplied = 0usize;
    let mut already = 0usize;
    let mut foreign = 0usize;
    let mut floor_after = floor_before;
    for entry in &ledger {
        if !entry.brain_id.is_empty() && entry.brain_id != brain_id {
            foreign += 1;
            continue;
        }
        let present = id_present(
            conn,
            "SELECT COUNT(*) FROM erasures WHERE erasure_id = ?1",
            &entry.erasure_id,
            "erasure presence unreadable",
        )?;
        let exists = id_present(
            conn,
            "SELECT COUNT(*) FROM records WHERE record_id = ?1",
            &entry.record_id,
            "record presence unreadable",
        )?;
        if present {
            already += 1;
        } else if exists {
            let sequence =
                super::records::append_ack_commit(conn, &format!("reconcile:{}", entry.authority))?;
            conn.execute("INSERT OR IGNORE INTO erasures (erasure_id, scope_id, target_descriptor, sequence, propagation_state) VALUES (?1, ?2, ?3, ?4, 'reconciled')", params![entry.erasure_id, super::records::DEFAULT_SCOPE, json!({"record_id": entry.record_id, "authority": entry.authority, "reason": entry.reason, "reconciled": true}).to_string(), sequence]).map_err(|e| e.to_string())?;
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
    let floor = match floor_raw(conn) {
        Ok(Some(raw)) => raw.parse::<i64>().map_err(
            |_| json!({"status": "unavailable", "error": "erasure floor is not an integer"}),
        )?,
        Ok(None) => 0,
        Err(_) => {
            return Err(json!({"status": "unavailable", "error": "erasure floor unreadable"}));
        }
    };
    (through_sequence >= floor).then_some(()).ok_or_else(|| {
        json!({"status": "resnapshot_required", "error": "revocation fence: an erasure happened after this View was minted", "erasure_floor": floor, "through_sequence": through_sequence})
    })
}

pub fn is_erased(conn: &Connection, record_id: &str) -> bool {
    // Unreadable COUNT is fail-closed: alias expand must not look like "not erased".
    conn.query_row("SELECT COUNT(*) FROM erasures WHERE COALESCE(json_extract(target_descriptor, '$.record_id'), target_descriptor) = ?1", [record_id], |r| r.get::<_, i64>(0)).map(|n| n > 0).unwrap_or(true)
}
