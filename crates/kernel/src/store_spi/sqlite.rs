//! Reference `BrainStore` over the existing SQLite schema.
//!
//! Frontier = (`versions.id` high-water mark) inside the current restore epoch
//! (`brain_meta.restore_epoch`, "0" until the backup/restore bead writes it).
//! Reads run inside one deferred transaction so every query in a snapshot sees
//! the same WAL state; writes run inside one immediate transaction so a
//! failure returns no partial success.

use super::*;
use crate::protocol::{
    AckProfile, DurabilityVector, Frontier, LogicalId, PayloadAvailability, Receipt,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const IDEMPOTENCY_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS operation_ledger (
  principal TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  request_id TEXT NOT NULL,
  canonical_hash TEXT NOT NULL,
  receipt_json TEXT NOT NULL,
  entry_json TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY (principal, idempotency_key)
);
"#;

pub fn restore_epoch(conn: &Connection) -> String {
    conn.query_row(
        "SELECT restore_epoch FROM brain_meta WHERE singleton = 1",
        [],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
    .unwrap_or_else(|| "0".to_string())
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

fn row_from_decision(conn: &Connection, id: i64) -> Result<Option<Row>, StoreSpiError> {
    conn.query_row(
        "SELECT id, decision, context, status, source_agent, created_at, COALESCE(version_id, 0) FROM decisions WHERE id = ?1",
        params![id],
        |r| {
            Ok(Row {
                id: LogicalId::from_legacy("decision", r.get(0)?),
                revision: LogicalId::from_legacy("version", r.get::<_, i64>(6)?),
                kind: "decision".into(),
                body: json!({"text": r.get::<_, String>(1)?, "context": r.get::<_, Option<String>>(2)?, "status": r.get::<_, String>(3)?, "agent": r.get::<_, String>(4)?, "created_at": r.get::<_, String>(5)?}),
            })
        },
    )
    .optional()
    .map_err(|e| StoreSpiError::Unavailable(e.to_string()))
}

fn row_from_memory(conn: &Connection, id: i64) -> Result<Option<Row>, StoreSpiError> {
    conn.query_row(
        "SELECT id, text, type, status, source_agent, created_at, COALESCE(version_id, 0) FROM memories WHERE id = ?1",
        params![id],
        |r| {
            Ok(Row {
                id: LogicalId::from_legacy("memory", r.get(0)?),
                revision: LogicalId::from_legacy("version", r.get::<_, i64>(6)?),
                kind: r.get::<_, String>(2)?,
                body: json!({"text": r.get::<_, String>(1)?, "status": r.get::<_, String>(3)?, "agent": r.get::<_, String>(4)?, "created_at": r.get::<_, String>(5)?}),
            })
        },
    )
    .optional()
    .map_err(|e| StoreSpiError::Unavailable(e.to_string()))
}

impl ReadSnapshot for SqliteSnapshot<'_> {
    fn frontier(&self) -> &Frontier {
        &self.frontier
    }
    fn get(&self, refs: &[LogicalId]) -> Result<Vec<Row>, StoreSpiError> {
        let mut out = Vec::new();
        for reference in refs {
            let Ok(id) = reference.value.parse::<i64>() else {
                continue;
            };
            let row = match reference.namespace.as_str() {
                "decision" => row_from_decision(self.conn, id)?,
                "memory" => row_from_memory(self.conn, id)?,
                _ => None,
            };
            out.extend(row);
        }
        Ok(out)
    }
    fn scan(
        &self,
        predicate: &Predicate,
        continuation: Option<&str>,
        limits: ScanLimits,
    ) -> Result<Page, StoreSpiError> {
        scan_rows(self.conn, &self.frontier, predicate, continuation, limits)
    }
    fn candidates(
        &self,
        profile: &CandidateProfile,
        keys: &[String],
        limits: ScanLimits,
    ) -> Result<Page, StoreSpiError> {
        let CandidateProfile::ExactLexical = profile else {
            return Err(StoreSpiError::Unavailable(format!(
                "candidate profile {profile:?} not provided by the sqlite reference store"
            )));
        };
        let needles: Vec<String> = keys
            .iter()
            .map(|k| k.to_ascii_lowercase())
            .filter(|k| !k.is_empty())
            .collect();
        if needles.is_empty() {
            return Ok(Page {
                rows: Vec::new(),
                coverage: Coverage {
                    frontier: self.frontier.clone(),
                    exhausted: true,
                    rows_examined: 0,
                    continuation: None,
                },
            });
        }
        // Exact profile: full scan with deterministic ordering; completeness is
        // never inferred from an index cutoff.
        let mut rows = Vec::new();
        let mut examined = 0u64;
        for table in ["decisions", "memories"] {
            let (kind, col) = if table == "decisions" {
                ("decision", "decision")
            } else {
                ("memory", "text")
            };
            let mut stmt = self
                .conn
                .prepare(&format!(
                    "SELECT id, {col}, status, source_agent, created_at, COALESCE(version_id,0) FROM {table} WHERE status = 'active' ORDER BY id"
                ))
                .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
            let iter = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, i64>(5)?,
                    ))
                })
                .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
            for item in iter {
                let (id, text, status, agent, created_at, version) =
                    item.map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
                examined += 1;
                let lower = text.to_ascii_lowercase();
                if needles.iter().all(|n| lower.contains(n.as_str())) {
                    rows.push(Row {
                        id: LogicalId::from_legacy(kind, id),
                        revision: LogicalId::from_legacy("version", version),
                        kind: kind.into(),
                        body: json!({"text": text, "status": status, "agent": agent, "created_at": created_at}),
                    });
                }
            }
        }
        rows.sort_by(|a, b| a.id.canonical().cmp(&b.id.canonical()));
        let exhausted = rows.len() as u32 <= limits.rows;
        rows.truncate(limits.rows as usize);
        Ok(Page {
            rows,
            coverage: Coverage {
                frontier: self.frontier.clone(),
                exhausted,
                rows_examined: examined,
                continuation: None,
            },
        })
    }
}

struct ScanTarget {
    ns: &'static str,
    table: &'static str,
    text_col: &'static str,
    type_eq: Option<String>,
}

fn scan_targets(predicate: &Predicate) -> Vec<ScanTarget> {
    match predicate_kind(predicate) {
        Some("decision") => vec![ScanTarget {
            ns: "decision",
            table: "decisions",
            text_col: "decision",
            type_eq: None,
        }],
        Some("memory") => vec![ScanTarget {
            ns: "memory",
            table: "memories",
            text_col: "text",
            type_eq: None,
        }],
        Some(other) => vec![
            ScanTarget {
                ns: "decision",
                table: "decisions",
                text_col: "decision",
                type_eq: Some(other.to_string()),
            },
            ScanTarget {
                ns: "memory",
                table: "memories",
                text_col: "text",
                type_eq: Some(other.to_string()),
            },
        ],
        None => vec![
            ScanTarget {
                ns: "decision",
                table: "decisions",
                text_col: "decision",
                type_eq: None,
            },
            ScanTarget {
                ns: "memory",
                table: "memories",
                text_col: "text",
                type_eq: None,
            },
        ],
    }
}

fn parse_scan_cursor(continuation: Option<&str>, targets: &[ScanTarget]) -> (usize, i64) {
    let Some(raw) = continuation.filter(|s| !s.is_empty()) else {
        return (0, 0);
    };
    if let Some(rest) = raw.strip_prefix("memory:") {
        return match targets.iter().position(|t| t.ns == "memory") {
            Some(idx) => (idx, rest.parse().unwrap_or(0)),
            None => (0, 0),
        };
    }
    if let Some(rest) = raw.strip_prefix("decision:") {
        return match targets.iter().position(|t| t.ns == "decision") {
            Some(idx) => (idx, rest.parse().unwrap_or(0)),
            None => (0, 0),
        };
    }
    (0, raw.parse().unwrap_or(0))
}

fn cursor_token(multi: bool, ns: &str, id: i64) -> String {
    if multi {
        format!("{ns}:{id}")
    } else {
        id.to_string()
    }
}

fn cursor_from_row(multi: bool, row: &Row) -> String {
    cursor_token(multi, &row.id.namespace, row.id.value.parse().unwrap_or(0))
}

fn map_scan_row(
    r: &rusqlite::Row<'_>,
) -> rusqlite::Result<(i64, String, String, String, String, i64, String)> {
    Ok((
        r.get::<_, i64>(0)?,
        r.get::<_, String>(1)?,
        r.get::<_, String>(2)?,
        r.get::<_, String>(3)?,
        r.get::<_, String>(4)?,
        r.get::<_, i64>(5)?,
        r.get::<_, String>(6)?,
    ))
}

fn scan_rows(
    conn: &Connection,
    frontier: &Frontier,
    predicate: &Predicate,
    continuation: Option<&str>,
    limits: ScanLimits,
) -> Result<Page, StoreSpiError> {
    let targets = scan_targets(predicate);
    let multi = targets.len() > 1;
    let (start_idx, mut after) = parse_scan_cursor(continuation, &targets);
    let mut rows = Vec::new();
    let mut bytes = 0u64;
    let mut examined = 0u64;
    let mut truncated = false;
    let mut continuation_out = None;
    let fetch = (limits.rows as i64).saturating_add(1).max(32);
    'tables: for (idx, target) in targets.iter().enumerate() {
        if idx < start_idx {
            continue;
        }
        if idx > start_idx {
            after = 0;
        }
        loop {
            let type_clause = if target.type_eq.is_some() {
                " AND type = ?3"
            } else {
                ""
            };
            let sql = format!(
                "SELECT id, {text_col}, status, source_agent, created_at, COALESCE(version_id,0), COALESCE(type, '{ns}') \
                 FROM {table} WHERE id > ?1{type_clause} ORDER BY id LIMIT ?2",
                text_col = target.text_col,
                ns = target.ns,
                table = target.table,
            );
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
            let batch: Vec<(i64, String, String, String, String, i64, String)> =
                if let Some(typ) = &target.type_eq {
                    stmt.query_map(params![after, fetch, typ], map_scan_row)
                        .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?
                } else {
                    stmt.query_map(params![after, fetch], map_scan_row)
                        .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?
                };
            if batch.is_empty() {
                break;
            }
            let batch_len = batch.len();
            for (id, text, status, agent, created_at, version, row_type) in batch {
                examined += 1;
                after = id;
                let body =
                    json!({"text": text, "status": status, "agent": agent, "created_at": created_at});
                if !predicate_matches(predicate, &body) {
                    continue;
                }
                if rows.len() as u32 >= limits.rows {
                    truncated = true;
                    continuation_out = rows
                        .last()
                        .map(|r| cursor_from_row(multi, r))
                        .or_else(|| Some(cursor_token(multi, target.ns, id)));
                    break 'tables;
                }
                let text_len = text.len() as u64;
                if bytes.saturating_add(text_len) > limits.bytes && limits.bytes > 0 {
                    if rows.is_empty() {
                        // A single row larger than the byte budget must not
                        // pin the cursor: skip it so the next page can move.
                        continue;
                    }
                    truncated = true;
                    continuation_out = rows.last().map(|r| cursor_from_row(multi, r));
                    break 'tables;
                }
                bytes = bytes.saturating_add(text_len);
                let kind = if target.ns == "decision" {
                    "decision".to_string()
                } else {
                    row_type
                };
                rows.push(Row {
                    id: LogicalId::from_legacy(target.ns, id),
                    revision: LogicalId::from_legacy("version", version),
                    kind,
                    body,
                });
            }
            if batch_len < fetch as usize {
                break;
            }
        }
    }
    Ok(Page {
        rows,
        coverage: Coverage {
            frontier: frontier.clone(),
            exhausted: !truncated,
            rows_examined: examined,
            continuation: continuation_out,
        },
    })
}

fn predicate_kind(p: &Predicate) -> Option<&str> {
    match p {
        Predicate::Kind(k) => Some(k.as_str()),
        Predicate::And(items) => items.iter().find_map(predicate_kind),
        _ => None,
    }
}

fn predicate_matches(p: &Predicate, body: &Value) -> bool {
    match p {
        Predicate::All | Predicate::Kind(_) => true,
        Predicate::Eq { field, value } => body.get(field) == Some(value),
        Predicate::And(items) => items.iter().all(|i| predicate_matches(i, body)),
    }
}

pub struct SqliteTx<'a> {
    conn: &'a mut Connection,
    intent: WriteIntent,
    canonical: Vec<String>,
    entries: BTreeMap<String, LogicalId>,
    finished: bool,
}

impl SqliteTx<'_> {
    fn head_of(conn: &Connection, record: &LogicalId) -> Result<Option<LogicalId>, StoreSpiError> {
        let Ok(id) = record.value.parse::<i64>() else {
            return Ok(None);
        };
        let table = match record.namespace.as_str() {
            "decision" => "decisions",
            "memory" => "memories",
            _ => return Ok(None),
        };
        conn.query_row(
            &format!("SELECT version_id FROM {table} WHERE id = ?1"),
            params![id],
            |r| r.get::<_, Option<i64>>(0),
        )
        .optional()
        .map_err(|e| StoreSpiError::Unavailable(e.to_string()))
        .map(|v| v.flatten().map(|v| LogicalId::from_legacy("version", v)))
    }
}

impl WriteTransaction for SqliteTx<'_> {
    fn apply(&mut self, op: Op) -> Result<(), StoreSpiError> {
        self.canonical.push(format!("{op:?}"));
        match op {
            Op::InsertDecision {
                local_name,
                text,
                context,
                agent,
                owner_id,
            } => {
                self.conn
                    .execute(
                        "INSERT INTO decisions (decision, context, type, source_agent, status, owner_id) VALUES (?1, ?2, 'decision', ?3, 'active', ?4)",
                        params![text, context, agent, owner_id],
                    )
                    .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
                let id = self.conn.last_insert_rowid();
                let version = crate::traces::record_store_write(
                    self.conn,
                    &agent,
                    &text,
                    "stored",
                    "decision",
                    Some(id),
                    owner_id,
                );
                if let Some(v) = version {
                    let stamped = self
                        .conn
                        .execute(
                            "UPDATE decisions SET version_id = ?1 WHERE id = ?2",
                            params![v, id],
                        )
                        .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
                    if stamped == 0 {
                        return Err(StoreSpiError::Unavailable(format!(
                            "decision {id} missing after insert"
                        )));
                    }
                }
                self.entries
                    .insert(local_name, LogicalId::from_legacy("decision", id));
            }
            Op::InsertMemory {
                local_name,
                text,
                kind,
                agent,
                owner_id,
            } => {
                self.conn
                    .execute(
                        "INSERT INTO memories (text, type, source, source_agent, status, owner_id) VALUES (?1, ?2, 'spi', ?3, 'active', ?4)",
                        params![text, kind, agent, owner_id],
                    )
                    .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
                let id = self.conn.last_insert_rowid();
                let version = crate::traces::record_store_write(
                    self.conn,
                    &agent,
                    &text,
                    "stored",
                    "memory",
                    Some(id),
                    owner_id,
                );
                if let Some(v) = version {
                    let stamped = self
                        .conn
                        .execute(
                            "UPDATE memories SET version_id = ?1 WHERE id = ?2",
                            params![v, id],
                        )
                        .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
                    if stamped == 0 {
                        return Err(StoreSpiError::Unavailable(format!(
                            "memory {id} missing after insert"
                        )));
                    }
                }
                self.entries
                    .insert(local_name, LogicalId::from_legacy("memory", id));
            }
            Op::SetStatus { target, status } => {
                let (table, target_type) = match target.namespace.as_str() {
                    "decision" => ("decisions", "decision"),
                    "memory" => ("memories", "memory"),
                    other => {
                        return Err(StoreSpiError::LimitExceeded(format!(
                            "unknown record namespace {other}"
                        )))
                    }
                };
                let id: i64 = target
                    .value
                    .parse()
                    .map_err(|_| StoreSpiError::LimitExceeded("non-numeric legacy id".into()))?;
                let updated = self
                    .conn
                    .execute(
                        &format!(
                            "UPDATE {table} SET status = ?1, updated_at = datetime('now') WHERE id = ?2"
                        ),
                        params![status, id],
                    )
                    .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
                if updated == 0 {
                    return Err(StoreSpiError::LimitExceeded(format!(
                        "unknown record {}",
                        target.canonical()
                    )));
                }
                let _ = crate::traces::record_store_write(
                    self.conn,
                    &self.intent.principal,
                    &status,
                    "status",
                    target_type,
                    Some(id),
                    None,
                );
            }
        }
        Ok(())
    }

    fn commit(mut self, durability: Durability) -> Result<Receipt, StoreSpiError> {
        let frontier = current_frontier(self.conn);
        let receipt = Receipt {
            receipt_id: LogicalId::new("receipt", frontier_sequence(&frontier).to_string()),
            request_id: self.intent.request_id.clone(),
            durability: DurabilityVector {
                accepted: true,
                local_commit: Some(frontier.clone()),
                projected_through: [("exact".to_string(), frontier.clone())]
                    .into_iter()
                    .collect(),
                replicated_through: BTreeMap::new(),
                payload_availability: PayloadAvailability::Retained,
                ack_profile: match durability {
                    Durability::PowerLossAssumed => AckProfile::PowerLossAssumed {
                        platform_profile: "sqlite-wal-synchronous-full".into(),
                    },
                    Durability::ProcessCrash => AckProfile::ProcessCrash,
                },
            },
            entries: std::mem::take(&mut self.entries),
            aliases: BTreeMap::new(),
            omissions: Vec::new(),
            unresolved_needs: Vec::new(),
        };
        if let Some(key) = &self.intent.idempotency_key {
            let hash = cortex_logic::traces::content_hash(&self.canonical.join("\n"));
            self.conn
                .execute(
                    "INSERT INTO operation_ledger (principal, idempotency_key, request_id, canonical_hash, receipt_json) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![self.intent.principal, key, self.intent.request_id, hash, serde_json::to_string(&receipt).unwrap_or_default()],
                )
                .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
        }
        if matches!(durability, Durability::PowerLossAssumed) {
            let _ = self.conn.execute_batch("PRAGMA synchronous = FULL");
        }
        self.conn
            .execute_batch("COMMIT")
            .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
        self.finished = true;
        Ok(receipt)
    }

    fn abort(mut self) {
        let _ = self.conn.execute_batch("ROLLBACK");
        // Only mark finished when the connection is actually autocommit.
        // A failed ROLLBACK with a live statement would otherwise skip Drop's
        // retry and leave the next lock holder inside the write transaction.
        self.finished = self.conn.is_autocommit();
    }
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
        let mut stmt = self
            .conn
            .prepare("SELECT v.id, v.target_type, v.target_id, v.op, COALESCE(t.agent, 'unknown') FROM versions v LEFT JOIN traces t ON t.id = v.trace_id WHERE v.id > ?1 ORDER BY v.id LIMIT ?2")
            .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
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
        let canonical: Vec<String> = canonical_ops.iter().map(|op| format!("{op:?}")).collect();
        let hash = cortex_logic::traces::content_hash(&canonical.join("\n"));
        let found = self
            .conn
            .query_row(
                "SELECT canonical_hash, receipt_json FROM operation_ledger WHERE principal = ?1 AND idempotency_key = ?2",
                params![principal, key],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
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
