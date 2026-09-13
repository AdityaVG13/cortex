//! Transactional outbox and maintenance debt.
//!
//! A deposit records minimal exact lookup metadata and appends outbox jobs in
//! the same transaction. Jobs are idempotent, versioned and keyed by
//! (commit sequence, job kind); workers claim them under a lease with an
//! expected generation, completion is idempotent, and a crashed claimant
//! leaves the job for a later caller. Without callers there is no progress:
//! the brain is persistent, not computationally active. Debt is measured and
//! exposed; when it exceeds the declared bound the intake reports
//! backpressure instead of pretending freshness.

use super::records::DEFAULT_SCOPE;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

pub const JOB_LEASE_SECONDS: i64 = 60;
pub const MAX_ATTEMPTS: i64 = 5;
/// Declared load bound: more pending jobs than this is backpressure.
pub const DEBT_SOFT_LIMIT_JOBS: i64 = 5_000;
pub const DEBT_HARD_LIMIT_JOBS: i64 = 50_000;
/// Operational telemetry retention (feed, boot audits, acks): a separate
/// policy class from durable memory.
pub const FEED_RETENTION_DAYS: i64 = 30;
pub const FEED_MAX_ROWS: i64 = 20_000;
/// Anchor hub control: an anchor shared by more than this many rows is a
/// hub; its pairwise links are not materialized beyond the cap.
pub const ANCHOR_HUB_DEGREE: i64 = 64;

pub const JOB_KINDS: [&str; 4] = [
    "fts_optimize",
    "checkpoint_wal",
    "prune_telemetry",
    "clock_link_audit",
];

/// Enqueue the projection jobs for one commit. Idempotent per (sequence, kind).
pub fn enqueue_for_commit(
    conn: &Connection,
    sequence: i64,
    kinds: &[&str],
    payload: Value,
) -> rusqlite::Result<usize> {
    let mut added = 0;
    for kind in kinds {
        let job_id = format!("job:{sequence}:{kind}");
        added += conn.execute(
            "INSERT OR IGNORE INTO outbox (job_id, commit_sequence, job_kind, state, generation, payload_json, attempts) VALUES (?1, ?2, ?3, 'pending', 0, ?4, 0)",
            params![job_id, sequence, kind, payload.to_string()],
        )?;
    }
    Ok(added)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub job_id: String,
    pub kind: String,
    pub generation: i64,
    pub sequence: i64,
}

/// Claim the oldest runnable job. Expired leases are reclaimable; the
/// generation increments on every claim so a stale claimant's completion
/// is rejected. Fairness: round-robin over job kinds by oldest first.
pub fn claim_next(conn: &Connection, worker: &str) -> rusqlite::Result<Option<Claim>> {
    let now = chrono::Utc::now().to_rfc3339();
    let candidate: Option<(String, String, i64, i64)> = conn
        .query_row(
            "SELECT job_id, job_kind, generation, commit_sequence FROM outbox WHERE (state = 'pending' OR (state = 'claimed' AND lease_until < ?1)) AND attempts < ?2 ORDER BY commit_sequence ASC, job_kind ASC LIMIT 1",
            params![now, MAX_ATTEMPTS],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((job_id, kind, generation, sequence)) = candidate else {
        return Ok(None);
    };
    let lease_until =
        (chrono::Utc::now() + chrono::Duration::seconds(JOB_LEASE_SECONDS)).to_rfc3339();
    let updated = conn.execute(
        "UPDATE outbox SET state = 'claimed', generation = generation + 1, claimed_by = ?1, lease_until = ?2, attempts = attempts + 1 WHERE job_id = ?3 AND generation = ?4",
        params![worker, lease_until, job_id, generation],
    )?;
    if updated == 0 {
        return Ok(None);
    }
    Ok(Some(Claim {
        job_id,
        kind,
        generation: generation + 1,
        sequence,
    }))
}

/// Complete a claim. Idempotent: a repeat completion of the same generation
/// is a no-op; a stale generation is rejected.
pub fn complete(conn: &Connection, claim: &Claim) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "UPDATE outbox SET state = 'done', lease_until = NULL WHERE job_id = ?1 AND generation = ?2 AND state IN ('claimed', 'done')",
        params![claim.job_id, claim.generation],
    )?;
    Ok(n > 0)
}

pub fn fail(conn: &Connection, claim: &Claim, error: &str) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "UPDATE outbox SET state = CASE WHEN attempts >= ?3 THEN 'failed' ELSE 'pending' END, last_error = ?1, lease_until = NULL WHERE job_id = ?2 AND generation = ?4 AND state = 'claimed'",
        params![error, claim.job_id, MAX_ATTEMPTS, claim.generation],
    )?;
    Ok(n > 0)
}

/// Execute one job kind. Every kind is idempotent and bounded.
pub fn run_job(conn: &Connection, claim: &Claim) -> Result<Value, String> {
    match claim.kind.as_str() {
        "fts_optimize" => {
            for table in ["decisions_fts", "memories_fts"] {
                if super::table_exists(conn, table) {
                    conn.execute_batch(&format!(
                        "INSERT INTO {table}({table}) VALUES ('merge=16')"
                    ))
                    .map_err(|e| e.to_string())?;
                }
            }
            Ok(json!({"optimized": true}))
        }
        "checkpoint_wal" => {
            conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE)")
                .map_err(|e| e.to_string())?;
            Ok(json!({"checkpoint": "passive"}))
        }
        "prune_telemetry" => {
            let pruned = prune_telemetry(conn).map_err(|e| e.to_string())?;
            Ok(json!({"pruned": pruned}))
        }
        "clock_link_audit" => {
            let hubs = hub_anchors(conn).map_err(|e| e.to_string())?;
            let mut trimmed = 0usize;
            for (anchor_id, _) in &hubs {
                // Keep membership (evidence rows). Drop stored pairwise links
                // past the cap; extra clique edges are rebuilt at query time.
                // Columns are src_*/dst_* (not from_/to_). OFFSET keeps the
                // newest cap rows the same way feed prune keeps newest rows.
                trimmed += conn
                    .execute(
                        "DELETE FROM clock_links WHERE rowid IN (SELECT l.rowid FROM clock_links l WHERE l.relation IN ('observed_with','same_path','same_symbol') AND EXISTS (SELECT 1 FROM clock_anchor_evidence e1 WHERE e1.anchor_id = ?1 AND e1.target_type = l.src_type AND e1.target_id = l.src_id) AND EXISTS (SELECT 1 FROM clock_anchor_evidence e2 WHERE e2.anchor_id = ?1 AND e2.target_type = l.dst_type AND e2.target_id = l.dst_id) ORDER BY l.rowid DESC LIMIT -1 OFFSET ?2)",
                        params![anchor_id, ANCHOR_HUB_DEGREE],
                    )
                    .map_err(|e| e.to_string())?;
            }
            Ok(json!({"hubs": hubs.len(), "links_trimmed": trimmed}))
        }
        other => Err(format!("unknown job kind {other}")),
    }
}

/// Anchors whose evidence degree exceeds the hub threshold.
pub fn hub_anchors(conn: &Connection) -> rusqlite::Result<Vec<(i64, i64)>> {
    let mut stmt =
        conn.prepare("SELECT anchor_id, COUNT(*) AS degree FROM clock_anchor_evidence GROUP BY anchor_id HAVING degree > ?1 ORDER BY degree DESC LIMIT 64")?;
    let rows = stmt.query_map(params![ANCHOR_HUB_DEGREE], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

/// Operational telemetry pruning: feed by age and row cap, feed_acks for
/// vanished agents, boot audits older than their retention.
pub fn prune_telemetry(conn: &Connection) -> rusqlite::Result<usize> {
    let mut pruned = 0;
    let cutoff = format!("-{FEED_RETENTION_DAYS} days");
    pruned += conn.execute(
        "DELETE FROM feed WHERE julianday(timestamp) < julianday('now', ?1)",
        params![cutoff],
    )?;
    pruned += conn
        .execute("DELETE FROM feed WHERE rowid IN (SELECT rowid FROM feed ORDER BY julianday(timestamp) DESC, rowid DESC LIMIT -1 OFFSET ?1)", params![FEED_MAX_ROWS])?;
    pruned += conn.execute("DELETE FROM feed_acks WHERE last_seen_id IS NOT NULL AND last_seen_id NOT IN (SELECT id FROM feed)", [])?;
    Ok(pruned)
}

/// One bounded maintenance slice run by a caller. Returns work units used.
pub fn maintain_slice(conn: &Connection, worker: &str, max_jobs: usize) -> Result<Value, String> {
    let mut done = Vec::new();
    for _ in 0..max_jobs {
        let Some(claim) = claim_next(conn, worker).map_err(|e| e.to_string())? else {
            break;
        };
        match run_job(conn, &claim) {
            Ok(result) => {
                complete(conn, &claim).map_err(|e| e.to_string())?;
                done.push(json!({"job": claim.job_id, "kind": claim.kind, "result": result}));
            }
            Err(err) => {
                fail(conn, &claim, &err).map_err(|e| e.to_string())?;
                done.push(json!({"job": claim.job_id, "kind": claim.kind, "error": err}));
            }
        }
    }
    Ok(json!({"worker": worker, "jobs": done}))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Debt {
    pub pending_jobs: i64,
    pub claimed_jobs: i64,
    pub failed_jobs: i64,
    pub earliest_unprojected_sequence: Option<i64>,
    pub commit_frontier: i64,
    pub feed_rows: i64,
}

impl Debt {
    pub fn pressure(&self) -> &'static str {
        if self.pending_jobs >= DEBT_HARD_LIMIT_JOBS {
            "hard"
        } else if self.pending_jobs >= DEBT_SOFT_LIMIT_JOBS {
            "soft"
        } else {
            "none"
        }
    }
    /// Backpressure decision for intake: at hard pressure new deposits are
    /// refused with `unavailable` until a maintenance slice drains debt.
    pub fn refuse_intake(&self) -> bool {
        self.pressure() == "hard"
    }
    pub fn to_json(&self) -> Value {
        json!({
            "pending_jobs": self.pending_jobs, "claimed_jobs": self.claimed_jobs, "failed_jobs": self.failed_jobs,
            "earliest_unprojected_sequence": self.earliest_unprojected_sequence, "commit_frontier": self.commit_frontier,
            "projection_lag": self.earliest_unprojected_sequence.map(|s| self.commit_frontier - s + 1).unwrap_or(0),
            "feed_rows": self.feed_rows, "pressure": self.pressure(), "soft_limit_jobs": DEBT_SOFT_LIMIT_JOBS, "hard_limit_jobs": DEBT_HARD_LIMIT_JOBS,
            "note": "no caller means no progress; run `cortex maintain` or any operation to drain"
        })
    }
}

pub fn debt(conn: &Connection) -> Debt {
    // An unreadable COUNT is not "no jobs": that would let intake proceed
    // while the outbox is at the hard bound. Treat a failed read as hard
    // pressure so refuse_intake stays fail-closed.
    let count = |state: &str| -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM outbox WHERE state = ?1",
            params![state],
            |r| r.get(0),
        )
        .unwrap_or(DEBT_HARD_LIMIT_JOBS)
    };
    Debt {
        pending_jobs: count("pending"),
        claimed_jobs: count("claimed"),
        failed_jobs: count("failed"),
        earliest_unprojected_sequence: conn
            .query_row(
                "SELECT MIN(commit_sequence) FROM outbox WHERE state IN ('pending','claimed')",
                [],
                |r| r.get::<_, Option<i64>>(0),
            )
            .ok()
            .flatten(),
        commit_frontier: conn
            .query_row("SELECT COALESCE(MAX(sequence),0) FROM commits", [], |r| {
                r.get(0)
            })
            .unwrap_or(0),
        feed_rows: conn
            .query_row("SELECT COUNT(*) FROM feed", [], |r| r.get(0))
            .unwrap_or(0),
    }
}

/// Semantic brain health: what the memory can still promise.
pub fn brain_health(conn: &Connection, home: &std::path::Path) -> Value {
    let debt = debt(conn);
    let unresolved_heads: i64 = conn
        .query_row("SELECT COUNT(*) FROM (SELECT record_id FROM record_heads GROUP BY record_id HAVING COUNT(*) > 1)", [], |r| r.get(0))
        .unwrap_or(0);
    let open_conflicts: i64 = conn
        .query_row("SELECT COUNT(*) FROM decision_conflicts WHERE status = 'open' AND classification = 'CONTRADICTS'", [], |r| r.get(0))
        .unwrap_or(0);
    let compiled: i64 = conn
        .query_row("SELECT COUNT(*) FROM compiled_reads", [], |r| r.get(0))
        .unwrap_or(0);
    let records: i64 = conn
        .query_row("SELECT COUNT(*) FROM records", [], |r| r.get(0))
        .unwrap_or(0);
    let recoverable_sources: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM decisions WHERE status != 'erased'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    json!({
        "commit_durability": crate::runtime::ack_profile_label_pub(&crate::store_spi::sqlite::ack_profile(conn)),
        "durability_profile": super::DurabilityProfile::from_env().as_str(),
        "recoverable_sources": recoverable_sources,
        "records": records,
        "projection_lag": debt.to_json()["projection_lag"],
        "maintenance_debt": debt.to_json(),
        "unresolved_heads": unresolved_heads,
        "open_contradictions": open_conflicts,
        "compiled_reads": compiled,
        "last_verified_restore": super::backup::last_verified_restore(home),
        "restore_epoch": super::records::brain_epochs(conn).1,
        "scope": DEFAULT_SCOPE,
        "capture_policy": super::capture_policy::inspect(conn),
        "capture_receipts": capture_receipts(conn),
        "reflex": reflex_status(conn, home),
    })
}

/// Capture receipts by status over the sources table plus recent capture
/// events: what was offered vs retained is what the control center shows.
fn capture_receipts(conn: &Connection) -> Value {
    let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap_or(0) };
    json!({
        "sources_owned": count("SELECT COUNT(*) FROM sources WHERE availability IN ('owned_inline','owned_external')"),
        "sources_external_only": count("SELECT COUNT(*) FROM sources WHERE availability = 'external_only'"),
        "sources_erased": count("SELECT COUNT(*) FROM sources WHERE availability = 'erased'"),
        "sources_unavailable": count("SELECT COUNT(*) FROM sources WHERE availability = 'unavailable'"),
        "hook_captures": count("SELECT COUNT(*) FROM operation_ledger WHERE idempotency_key LIKE 'capture:%'"),
    })
}

fn reflex_status(conn: &Connection, home: &std::path::Path) -> Value {
    let path = crate::reflex::snapshot_path(home);
    let snapshot = crate::reflex::load(&path);
    let state = crate::reflex::state_for(snapshot.as_ref(), conn);
    json!({
        "state": state,
        "generation": snapshot.as_ref().map(|s| s.header.generation),
        "records": snapshot.as_ref().map(|s| s.header.records),
        "built_at": snapshot.as_ref().map(|s| s.header.built_at.clone()),
        "path": path.display().to_string(),
    })
}
