//! Memory-only oracle provider: exact semantics, no durability. It exists so
//! the conformance suite has a second provider that passes `core` and
//! nothing else, and so a candidate collector can be shadowed against an
//! exhaustive in-memory scan.

use super::*;
use crate::protocol::{Frontier, LogicalId, Receipt};
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

mod read;
mod write;
pub use write::MemoryTx;
