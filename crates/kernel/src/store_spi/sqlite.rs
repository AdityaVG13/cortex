//! Reference `BrainStore` over the existing SQLite schema.
//!
//! Frontier = (`versions.id` high-water mark) inside the current restore epoch
//! (`brain_meta.restore_epoch`, "0" until the backup/restore bead writes it).
//! Reads run inside one deferred transaction so every query in a snapshot sees
//! the same WAL state; writes run inside one immediate transaction so a
//! failure returns no partial success.

use super::*;
use crate::protocol::{AckProfile, Frontier, LogicalId, Receipt};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeMap;
mod read;
mod scan;
mod write;

pub const IDEMPOTENCY_DDL: &str = "CREATE TABLE IF NOT EXISTS operation_ledger (principal TEXT NOT NULL, idempotency_key TEXT NOT NULL, request_id TEXT NOT NULL, canonical_hash TEXT NOT NULL, receipt_json TEXT NOT NULL, entry_json TEXT, created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), PRIMARY KEY (principal, idempotency_key));";

pub fn restore_epoch(conn: &Connection) -> String {
    // Same sentinels as `records::brain_epochs`: missing schema is `"0"`,
    // a locked/corrupt read is `"unreadable"`. Treating SQL failure as `"0"`
    // would let a pre-restore cursor pass equality against a restored brain.
    crate::db::records::brain_epochs(conn).1
}

pub fn current_frontier(conn: &Connection) -> Frontier {
    let seq: i64 = conn
        .query_row("SELECT COALESCE(MAX(id), 0) FROM versions", [], |r| {
            r.get(0)
        })
        .unwrap_or(0);
    Frontier {
        provider: "sqlite".into(),
        restore_epoch: restore_epoch(conn),
        opaque: seq.to_be_bytes().to_vec(),
    }
}

pub fn frontier_sequence(frontier: &Frontier) -> i64 {
    let mut buf = [0u8; 8];
    let n = frontier.opaque.len().min(8);
    buf[8 - n..].copy_from_slice(&frontier.opaque[frontier.opaque.len() - n..]);
    i64::from_be_bytes(buf)
}

/// The acknowledgement profile follows the connection's actual
/// `synchronous` pragma, never a config file's intent.
pub fn ack_profile(conn: &Connection) -> AckProfile {
    let sync: i64 = conn
        .query_row("PRAGMA synchronous", [], |r| r.get(0))
        .unwrap_or(1);
    if sync >= 2 {
        AckProfile::PowerLossAssumed {
            platform_profile: format!("sqlite-wal-synchronous-{sync}"),
        }
    } else {
        AckProfile::ProcessCrash
    }
}

pub struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    pub fn new(conn: Connection) -> Result<Self, StoreSpiError> {
        conn.execute_batch(IDEMPOTENCY_DDL)
            .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
        Ok(Self { conn })
    }
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
    pub fn into_connection(self) -> Connection {
        self.conn
    }
}

pub struct SqliteSnapshot<'a> {
    conn: &'a Connection,
    frontier: Frontier,
}

impl Drop for SqliteSnapshot<'_> {
    fn drop(&mut self) {
        // Abort polarity. `BEGIN DEFERRED` is a read snapshot; COMMIT here
        // would persist any writes that joined the txn through rusqlite's
        // `&Connection` API (`SqliteStore::connection()` can alias this
        // borrow). BrainStore `begin_write` needs `&mut self`, so `SqliteTx`
        // cannot overlap, but that seam still can -- including on panic unwind.
        // ROLLBACK matches `ImmediateWrite`, `SqliteSavepoint`, `SqliteTx`,
        // and rusqlite::Transaction. For a true read-only txn it is equivalent
        // to COMMIT except it also closes an aborted txn COMMIT would leave
        // open.
        let _ = self.conn.execute_batch("ROLLBACK");
    }
}

pub struct SqliteTx<'a> {
    conn: &'a mut Connection,
    intent: WriteIntent,
    canonical: Vec<String>,
    entries: BTreeMap<String, LogicalId>,
    finished: bool,
}

impl Drop for SqliteTx<'_> {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
    }
}

impl BrainStore for SqliteStore {
    type Snapshot<'a> = SqliteSnapshot<'a>;
    type Tx<'a> = SqliteTx<'a>;

    fn read_snapshot(&self) -> Result<SqliteSnapshot<'_>, StoreSpiError> {
        self.conn
            .execute_batch("BEGIN DEFERRED")
            .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
        let mut snapshot = SqliteSnapshot {
            conn: &self.conn,
            frontier: Frontier {
                provider: "sqlite".into(),
                restore_epoch: String::new(),
                opaque: Vec::new(),
            },
        };
        snapshot.frontier = current_frontier(snapshot.conn);
        Ok(snapshot)
    }

    fn begin_write(&mut self, intent: WriteIntent) -> Result<SqliteTx<'_>, StoreSpiError> {
        // Idempotency is resolved before any lock: same key + same payload
        // returns the original receipt at commit; a different payload is a
        // conflict surfaced by `replay`.
        self.conn
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
        let tx = SqliteTx {
            conn: &mut self.conn,
            intent,
            canonical: Vec::new(),
            entries: BTreeMap::new(),
            finished: false,
        };
        {
            let SqliteTx { conn, intent, .. } = &tx;
            for (record, expected) in &intent.expected_heads {
                let actual = SqliteTx::head_of(conn, record)?;
                if &actual != expected {
                    return Err(StoreSpiError::HeadConflict {
                        record: record.clone(),
                        expected: expected.clone(),
                        actual,
                    });
                }
            }
        }
        Ok(tx)
    }

    fn read_changes(
        &self,
        after: &Frontier,
        limit: u32,
    ) -> Result<(Vec<Change>, Frontier), StoreSpiError> {
        if after.restore_epoch != restore_epoch(&self.conn) {
            return Err(StoreSpiError::Unavailable(
                "resnapshot_required: cursor from a different restore epoch".into(),
            ));
        }
        let after_seq = frontier_sequence(after);
        let mut stmt = self.conn.prepare("SELECT v.id, v.target_type, v.target_id, v.op, COALESCE(t.agent, 'unknown') FROM versions v LEFT JOIN traces t ON t.id = v.trace_id WHERE v.id > ?1 ORDER BY v.id LIMIT ?2").map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
        let rows = stmt
            .query_map(params![after_seq, limit as i64], |r| {
                Ok(Change {
                    sequence: r.get(0)?,
                    target: LogicalId::from_legacy(
                        &r.get::<_, Option<String>>(1)?
                            .unwrap_or_else(|| "unknown".into()),
                        r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                    ),
                    action: r.get(3)?,
                    agent: r.get(4)?,
                })
            })
            .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
        Ok((rows, current_frontier(&self.conn)))
    }

    fn export_snapshot(&self, destination: &std::path::Path) -> Result<(), StoreSpiError> {
        let mut dest =
            Connection::open(destination).map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
        let backup = rusqlite::backup::Backup::new(&self.conn, &mut dest)
            .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
        backup
            .run_to_completion(64, std::time::Duration::from_millis(5), None)
            .map_err(|e| StoreSpiError::Unavailable(e.to_string()))
    }

    fn diagnose(&self) -> Result<Diagnosis, StoreSpiError> {
        let integrity_ok = crate::db::quick_check(&self.conn);
        Ok(Diagnosis {
            sqlite_version: crate::db::sqlite_version(),
            ack_profile: ack_profile(&self.conn),
            integrity_ok,
            frontier: current_frontier(&self.conn),
        })
    }

    fn maintain_slice(&mut self, max_work_units: u64) -> Result<u64, StoreSpiError> {
        if max_work_units == 0 {
            return Ok(0);
        }
        crate::db::records::ensure_authoritative_schema(&self.conn)
            .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
        let result = crate::db::outbox::maintain_slice(&self.conn, "spi", max_work_units as usize)
            .map_err(StoreSpiError::Unavailable)?;
        Ok(result["jobs"]
            .as_array()
            .map(|j| j.len() as u64)
            .unwrap_or(0))
    }

    fn replay(
        &self,
        principal: &str,
        key: &str,
        canonical_ops: &[Op],
    ) -> Result<Option<Receipt>, StoreSpiError> {
        let hash = cortex_logic::traces::content_hash(
            &canonical_ops
                .iter()
                .map(|op| format!("{op:?}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let found = self.conn.query_row("SELECT canonical_hash, receipt_json FROM operation_ledger WHERE principal = ?1 AND idempotency_key = ?2", params![principal, key], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).optional().map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
        match found {
            None => Ok(None),
            Some((stored_hash, receipt_json)) if stored_hash == hash => {
                serde_json::from_str(&receipt_json)
                    .map(Some)
                    .map_err(|e| StoreSpiError::Unavailable(e.to_string()))
            }
            Some(_) => Err(StoreSpiError::IdempotencyConflict {
                key: key.to_string(),
            }),
        }
    }
}
