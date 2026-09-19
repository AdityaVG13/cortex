//! BrainStore storage SPI.
//!
//! The runtime validates semantics; a store enforces atomicity, uniqueness,
//! parent relationships, stable references and declared durability. This is
//! a synchronous local facade: a blocking filesystem call is not made async
//! by wrapping it. Backends implement these traits over shared request/result
//! types; dispatch is by concrete type or an object-safe shim, never a
//! cross-language ABI.
//!
//! Conformance levels (see docs/contracts/protocol/README.md): `core` is what
//! `tests/contracts/store_spi.rs` exercises against the SQLite reference.

pub mod conformance;
pub mod dispatch;
pub mod memory;
pub mod sqlite;

use crate::protocol::{AckProfile, Frontier, LogicalId, Receipt};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum StoreSpiError {
    /// Expected head did not match the current head for a record.
    HeadConflict {
        record: LogicalId,
        expected: Option<LogicalId>,
        actual: Option<LogicalId>,
    },
    /// Same idempotency key seen with a different canonical payload.
    IdempotencyConflict {
        key: String,
    },
    /// Declared limits exceeded.
    LimitExceeded(String),
    Unavailable(String),
}

impl std::fmt::Display for StoreSpiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HeadConflict {
                record,
                expected,
                actual,
            } => write!(
                f,
                "head conflict on {}: expected {:?}, actual {:?}",
                record.canonical(),
                expected.as_ref().map(LogicalId::canonical),
                actual.as_ref().map(LogicalId::canonical)
            ),
            Self::IdempotencyConflict { key } => {
                write!(f, "idempotency key {key:?} reused with a different payload")
            }
            Self::LimitExceeded(what) => write!(f, "limit exceeded: {what}"),
            Self::Unavailable(msg) => write!(f, "store unavailable: {msg}"),
        }
    }
}

/// Portable predicate for `scan`. Deliberately tiny; anything richer is a
/// candidate profile, not an exact scan.
#[derive(Debug, Clone, PartialEq)]
pub enum Predicate {
    All,
    Kind(String),
    Eq { field: String, value: Value },
    And(Vec<Predicate>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanLimits {
    pub rows: u32,
    pub bytes: u64,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            rows: 256,
            bytes: 1 << 20,
        }
    }
}

/// Which part of the domain a result actually covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    pub frontier: Frontier,
    pub exhausted: bool,
    pub rows_examined: u64,
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: LogicalId,
    pub revision: LogicalId,
    pub kind: String,
    pub body: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub rows: Vec<Row>,
    pub coverage: Coverage,
}

impl Page {
    pub fn covered(
        rows: Vec<Row>,
        frontier: Frontier,
        exhausted: bool,
        rows_examined: u64,
        continuation: Option<String>,
    ) -> Self {
        Self {
            rows,
            coverage: Coverage {
                frontier,
                exhausted,
                rows_examined,
                continuation,
            },
        }
    }
}

/// Exact-semantic vs accelerated candidate discovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateProfile {
    ExactLexical,
    Accelerated(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Durability {
    ProcessCrash,
    PowerLossAssumed,
}

/// One validated semantic operation inside a write batch.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    InsertDecision {
        local_name: String,
        text: String,
        context: Option<String>,
        agent: String,
        owner_id: Option<i64>,
    },
    InsertMemory {
        local_name: String,
        text: String,
        kind: String,
        agent: String,
        owner_id: Option<i64>,
    },
    SetStatus {
        target: LogicalId,
        status: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct WriteIntent {
    pub request_id: String,
    pub idempotency_key: Option<String>,
    pub principal: String,
    pub expected_heads: Vec<(LogicalId, Option<LogicalId>)>,
}

/// A coherent read at one frontier.
pub trait ReadSnapshot {
    fn frontier(&self) -> &Frontier;
    fn get(&self, refs: &[LogicalId]) -> Result<Vec<Row>, StoreSpiError>;
    fn scan(
        &self,
        predicate: &Predicate,
        continuation: Option<&str>,
        limits: ScanLimits,
    ) -> Result<Page, StoreSpiError>;
    fn candidates(
        &self,
        profile: &CandidateProfile,
        keys: &[String],
        limits: ScanLimits,
    ) -> Result<Page, StoreSpiError>;
}

/// An owned write transaction; `commit` consumes it.
pub trait WriteTransaction: Sized {
    fn apply(&mut self, op: Op) -> Result<(), StoreSpiError>;
    fn commit(self, durability: Durability) -> Result<Receipt, StoreSpiError>;
    fn abort(self);
}

#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub sequence: i64,
    pub target: LogicalId,
    pub action: String,
    pub agent: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnosis {
    pub sqlite_version: String,
    pub ack_profile: AckProfile,
    pub integrity_ok: bool,
    pub frontier: Frontier,
}

pub trait BrainStore {
    type Snapshot<'a>: ReadSnapshot
    where
        Self: 'a;
    type Tx<'a>: WriteTransaction
    where
        Self: 'a;
    fn read_snapshot(&self) -> Result<Self::Snapshot<'_>, StoreSpiError>;
    fn begin_write(&mut self, intent: WriteIntent) -> Result<Self::Tx<'_>, StoreSpiError>;
    fn read_changes(
        &self,
        after: &Frontier,
        limit: u32,
    ) -> Result<(Vec<Change>, Frontier), StoreSpiError>;
    fn export_snapshot(&self, destination: &std::path::Path) -> Result<(), StoreSpiError>;
    fn diagnose(&self) -> Result<Diagnosis, StoreSpiError>;
    /// Perform a bounded slice of maintenance; returns work units consumed.
    fn maintain_slice(&mut self, max_work_units: u64) -> Result<u64, StoreSpiError>;
    /// Idempotent replay: the original receipt for a known `(principal,
    /// key)` when the canonical payload matches, a conflict when it differs,
    /// `None` when the key is new. Core-level providers must implement it.
    fn replay(
        &self,
        principal: &str,
        key: &str,
        canonical_ops: &[Op],
    ) -> Result<Option<Receipt>, StoreSpiError>;
}
