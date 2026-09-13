//! Memory-only oracle provider: exact semantics, no durability. It exists so
//! the conformance suite has a second provider that passes `core` and
//! nothing else, and so a candidate collector can be shadowed against an
//! exhaustive in-memory scan.

use super::*;
use crate::protocol::{
    AckProfile, DurabilityVector, Frontier, LogicalId, PayloadAvailability, Receipt,
};
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
struct Stored {
    kind: String,
    text: String,
    status: String,
    agent: String,
    version: i64,
}

#[derive(Default)]
pub struct MemoryStore {
    rows: BTreeMap<String, Stored>,
    next_id: BTreeMap<String, i64>,
    log: Vec<Change>,
    ledger: BTreeMap<(String, String), (String, Receipt)>,
    epoch: String,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self {
            epoch: "0".into(),
            ..Default::default()
        }
    }
    fn frontier(&self) -> Frontier {
        Frontier {
            provider: "memory".into(),
            restore_epoch: self.epoch.clone(),
            opaque: (self.log.len() as i64).to_be_bytes().to_vec(),
        }
    }
    fn row(&self, key: &str) -> Option<Row> {
        let (namespace, value) = key.split_once(':')?;
        let s = self.rows.get(key)?;
        Some(Row {
            id: LogicalId::new(namespace, value),
            revision: LogicalId::from_legacy("version", s.version),
            kind: s.kind.clone(),
            body: json!({"text": s.text, "status": s.status, "agent": s.agent}),
        })
    }
}

pub struct MemorySnapshot {
    rows: Vec<(String, Row)>,
    frontier: Frontier,
}

impl ReadSnapshot for MemorySnapshot {
    fn frontier(&self) -> &Frontier {
        &self.frontier
    }
    fn get(&self, refs: &[LogicalId]) -> Result<Vec<Row>, StoreSpiError> {
        Ok(refs
            .iter()
            .filter_map(|r| {
                self.rows
                    .iter()
                    .find(|(k, _)| k == &r.canonical())
                    .map(|(_, row)| row.clone())
            })
            .collect())
    }
    fn scan(
        &self,
        predicate: &Predicate,
        continuation: Option<&str>,
        limits: ScanLimits,
    ) -> Result<Page, StoreSpiError> {
        let after = continuation.unwrap_or("");
        let mut rows = Vec::new();
        let mut examined = 0u64;
        let mut bytes = 0u64;
        let mut truncated = false;
        if limits.rows == 0 {
            return Ok(Page {
                rows,
                coverage: Coverage {
                    frontier: self.frontier.clone(),
                    exhausted: true,
                    rows_examined: 0,
                    continuation: None,
                },
            });
        }
        for (_key, row) in self.rows.iter().filter(|(k, _)| k.as_str() > after) {
            examined += 1;
            if !predicate_ok(predicate, row) {
                continue;
            }
            if rows.len() as u32 >= limits.rows {
                truncated = true;
                break;
            }
            let text_len = row.body["text"].as_str().map(|t| t.len() as u64).unwrap_or(0);
            if bytes.saturating_add(text_len) > limits.bytes && limits.bytes > 0 {
                if rows.is_empty() {
                    continue;
                }
                truncated = true;
                break;
            }
            bytes = bytes.saturating_add(text_len);
            rows.push(row.clone());
        }
        let continuation = if truncated {
            rows.last().map(|r| r.id.canonical())
        } else {
            None
        };
        Ok(Page {
            rows,
            coverage: Coverage {
                frontier: self.frontier.clone(),
                exhausted: !truncated,
                rows_examined: examined,
                continuation,
            },
        })
    }
    fn candidates(
        &self,
        profile: &CandidateProfile,
        keys: &[String],
        limits: ScanLimits,
    ) -> Result<Page, StoreSpiError> {
        if *profile != CandidateProfile::ExactLexical {
            return Err(StoreSpiError::Unavailable(format!(
                "memory oracle provides only the exact profile, not {profile:?}"
            )));
        }
        let needles: Vec<String> = keys
            .iter()
            .map(|k| k.to_ascii_lowercase())
            .filter(|k| !k.is_empty())
            .collect();
        let mut rows: Vec<Row> = self
            .rows
            .iter()
            .filter(|(_, r)| r.body["status"] == "active")
            .filter(|(_, r)| {
                !needles.is_empty()
                    && needles.iter().all(|n| {
                        r.body["text"]
                            .as_str()
                            .unwrap_or("")
                            .to_ascii_lowercase()
                            .contains(n.as_str())
                    })
            })
            .map(|(_, r)| r.clone())
            .collect();
        rows.sort_by(|a, b| a.id.canonical().cmp(&b.id.canonical()));
        let exhausted = rows.len() as u32 <= limits.rows;
        rows.truncate(limits.rows as usize);
        Ok(Page {
            rows,
            coverage: Coverage {
                frontier: self.frontier.clone(),
                exhausted,
                rows_examined: self.rows.len() as u64,
                continuation: None,
            },
        })
    }
}

fn predicate_ok(p: &Predicate, row: &Row) -> bool {
    match p {
        Predicate::All => true,
        Predicate::Kind(k) => match k.as_str() {
            "decision" => row.id.namespace == "decision",
            "memory" => row.id.namespace == "memory",
            other => row.kind == other,
        },
        Predicate::Eq { field, value } => row.body.get(field) == Some(value),
        Predicate::And(items) => items.iter().all(|i| predicate_ok(i, row)),
    }
}

pub struct MemoryTx<'a> {
    store: &'a mut MemoryStore,
    intent: WriteIntent,
    staged: Vec<Op>,
    canonical: Vec<String>,
}

impl WriteTransaction for MemoryTx<'_> {
    fn apply(&mut self, op: Op) -> Result<(), StoreSpiError> {
        self.canonical.push(format!("{op:?}"));
        self.staged.push(op);
        Ok(())
    }
    fn commit(self, _durability: Durability) -> Result<Receipt, StoreSpiError> {
        let store = self.store;
        let mut rows = store.rows.clone();
        let mut next_id = store.next_id.clone();
        let mut log = store.log.clone();
        let mut entries = BTreeMap::new();
        for op in self.staged {
            match op {
                Op::InsertDecision {
                    local_name,
                    text,
                    agent,
                    ..
                } => {
                    insert_memory_row(
                        &mut rows,
                        &mut next_id,
                        &mut log,
                        &mut entries,
                        "decision",
                        "decision",
                        local_name,
                        text,
                        agent,
                    );
                }
                Op::InsertMemory {
                    local_name,
                    text,
                    kind,
                    agent,
                    ..
                } => {
                    let kind = if kind.is_empty() {
                        "memory".to_string()
                    } else {
                        kind
                    };
                    insert_memory_row(
                        &mut rows,
                        &mut next_id,
                        &mut log,
                        &mut entries,
                        "memory",
                        &kind,
                        local_name,
                        text,
                        agent,
                    );
                }
                Op::SetStatus { target, status } => {
                    let Some(row) = rows.get_mut(&target.canonical()) else {
                        return Err(StoreSpiError::LimitExceeded(format!(
                            "unknown record {}",
                            target.canonical()
                        )));
                    };
                    row.status = status;
                    let version = log.len() as i64 + 1;
                    row.version = version;
                    log.push(Change {
                        sequence: version,
                        target,
                        action: "status".into(),
                        agent: self.intent.principal.clone(),
                    });
                }
            }
        }
        store.rows = rows;
        store.next_id = next_id;
        store.log = log;
        let frontier = store.frontier();
        let receipt = Receipt {
            receipt_id: LogicalId::new("receipt", store.log.len().to_string()),
            request_id: self.intent.request_id.clone(),
            durability: DurabilityVector {
                accepted: true,
                local_commit: Some(frontier.clone()),
                projected_through: [("exact".to_string(), frontier)].into_iter().collect(),
                replicated_through: BTreeMap::new(),
                payload_availability: PayloadAvailability::Retained,
                ack_profile: AckProfile::ProcessCrash,
            },
            entries,
            aliases: BTreeMap::new(),
            omissions: vec!["memory oracle: no durability beyond the process".into()],
            unresolved_needs: Vec::new(),
        };
        if let Some(key) = self.intent.idempotency_key {
            let hash = cortex_logic::traces::content_hash(&self.canonical.join("\n"));
            store
                .ledger
                .insert((self.intent.principal, key), (hash, receipt.clone()));
        }
        Ok(receipt)
    }
    fn abort(self) {}
}

fn insert_memory_row(
    rows: &mut BTreeMap<String, Stored>,
    next_id: &mut BTreeMap<String, i64>,
    log: &mut Vec<Change>,
    entries: &mut BTreeMap<String, LogicalId>,
    ns: &str,
    kind: &str,
    local_name: String,
    text: String,
    agent: String,
) {
    let id = next_id.entry(ns.into()).or_insert(0);
    *id += 1;
    let logical = LogicalId::from_legacy(ns, *id);
    let version = log.len() as i64 + 1;
    rows.insert(
        logical.canonical(),
        Stored {
            kind: kind.into(),
            text,
            status: "active".into(),
            agent: agent.clone(),
            version,
        },
    );
    log.push(Change {
        sequence: version,
        target: logical.clone(),
        action: "stored".into(),
        agent,
    });
    entries.insert(local_name, logical);
}

impl BrainStore for MemoryStore {
    type Snapshot<'a> = MemorySnapshot;
    type Tx<'a> = MemoryTx<'a>;
    fn read_snapshot(&self) -> Result<MemorySnapshot, StoreSpiError> {
        let rows = self
            .rows
            .keys()
            .filter_map(|k| self.row(k).map(|r| (k.clone(), r)))
            .collect();
        Ok(MemorySnapshot {
            rows,
            frontier: self.frontier(),
        })
    }
    fn begin_write(&mut self, intent: WriteIntent) -> Result<MemoryTx<'_>, StoreSpiError> {
        for (record, expected) in &intent.expected_heads {
            let actual = self
                .rows
                .get(&record.canonical())
                .map(|s| LogicalId::from_legacy("version", s.version));
            if &actual != expected {
                return Err(StoreSpiError::HeadConflict {
                    record: record.clone(),
                    expected: expected.clone(),
                    actual,
                });
            }
        }
        Ok(MemoryTx {
            store: self,
            intent,
            staged: Vec::new(),
            canonical: Vec::new(),
        })
    }
    fn read_changes(
        &self,
        after: &Frontier,
        limit: u32,
    ) -> Result<(Vec<Change>, Frontier), StoreSpiError> {
        if after.restore_epoch != self.epoch {
            return Err(StoreSpiError::Unavailable("resnapshot_required".into()));
        }
        let seq = super::sqlite::frontier_sequence(after);
        Ok((
            self.log
                .iter()
                .filter(|c| c.sequence > seq)
                .take(limit as usize)
                .cloned()
                .collect(),
            self.frontier(),
        ))
    }
    fn export_snapshot(&self, destination: &std::path::Path) -> Result<(), StoreSpiError> {
        let rows: Vec<Row> = self.rows.keys().filter_map(|k| self.row(k)).collect();
        let body = serde_json::to_string(
            &rows
                .iter()
                .map(|r| json!({"id": r.id.canonical(), "kind": r.kind, "body": r.body}))
                .collect::<Vec<_>>(),
        )
        .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
        std::fs::write(destination, body).map_err(|e| StoreSpiError::Unavailable(e.to_string()))
    }
    fn diagnose(&self) -> Result<Diagnosis, StoreSpiError> {
        Ok(Diagnosis {
            sqlite_version: "n/a".into(),
            ack_profile: AckProfile::ProcessCrash,
            integrity_ok: true,
            frontier: self.frontier(),
        })
    }
    fn maintain_slice(&mut self, _max_work_units: u64) -> Result<u64, StoreSpiError> {
        Ok(0)
    }
    fn replay(
        &self,
        principal: &str,
        key: &str,
        canonical_ops: &[Op],
    ) -> Result<Option<Receipt>, StoreSpiError> {
        let canonical: Vec<String> = canonical_ops.iter().map(|op| format!("{op:?}")).collect();
        let hash = cortex_logic::traces::content_hash(&canonical.join("\n"));
        match self.ledger.get(&(principal.to_string(), key.to_string())) {
            None => Ok(None),
            Some((stored, receipt)) if stored == &hash => Ok(Some(receipt.clone())),
            Some(_) => Err(StoreSpiError::IdempotencyConflict {
                key: key.to_string(),
            }),
        }
    }
}
