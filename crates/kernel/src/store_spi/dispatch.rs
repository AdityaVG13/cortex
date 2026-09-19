//! Typed dispatch shim over the providers compiled into this build. A closed
//! enum, not a dynamic ABI: semantics must stabilize before anything crosses
//! a language boundary.

use super::conformance::{ConformanceLevel, DigestDescriptor, ProviderManifest};
use super::memory::{MemorySnapshot, MemoryStore, MemoryTx};
use super::sqlite::{SqliteSnapshot, SqliteStore, SqliteTx};
use super::*;
use crate::protocol::{Frontier, LogicalId, Receipt};

macro_rules! each_provider {
    ($this:expr, $method:ident $(, $arg:expr)* $(,)?) => {
        match $this {
            Self::Sqlite(inner) => inner.$method($($arg),*),
            Self::Memory(inner) => inner.$method($($arg),*),
        }
    };
}

macro_rules! each_provider_map {
    ($this:expr, $wrap:ident, $method:ident $(, $arg:expr)* $(,)?) => {
        match $this {
            Self::Sqlite(inner) => inner.$method($($arg),*).map($wrap::Sqlite),
            Self::Memory(inner) => inner.$method($($arg),*).map($wrap::Memory),
        }
    };
}

pub enum StoreHandle {
    Sqlite(SqliteStore),
    Memory(MemoryStore),
}

pub enum SnapshotHandle<'a> {
    Sqlite(SqliteSnapshot<'a>),
    Memory(MemorySnapshot),
}

pub enum TxHandle<'a> {
    Sqlite(SqliteTx<'a>),
    Memory(MemoryTx<'a>),
}

impl StoreHandle {
    pub fn manifest(&self) -> ProviderManifest {
        match self {
            Self::Sqlite(_) => ProviderManifest {
                name: "sqlite".into(),
                claims: [
                    ConformanceLevel::Core,
                    ConformanceLevel::DurableLocal,
                    ConformanceLevel::ConcurrentLocal,
                    ConformanceLevel::TamperEvident,
                ]
                .into_iter()
                .collect(),
                integrity: vec![DigestDescriptor::sha256("cortex/record")],
                optional_indexes: ["fts".to_string(), "clock_anchor".to_string()]
                    .into_iter()
                    .collect(),
                security_schemes: ["local-token".to_string()].into_iter().collect(),
            },
            Self::Memory(_) => ProviderManifest {
                name: "memory-oracle".into(),
                claims: [ConformanceLevel::Core].into_iter().collect(),
                integrity: Vec::new(),
                optional_indexes: Default::default(),
                security_schemes: ["local-token".to_string()].into_iter().collect(),
            },
        }
    }
}

impl ReadSnapshot for SnapshotHandle<'_> {
    fn frontier(&self) -> &Frontier {
        each_provider!(self, frontier)
    }
    fn get(&self, refs: &[LogicalId]) -> Result<Vec<Row>, StoreSpiError> {
        each_provider!(self, get, refs)
    }
    fn scan(
        &self,
        predicate: &Predicate,
        continuation: Option<&str>,
        limits: ScanLimits,
    ) -> Result<Page, StoreSpiError> {
        each_provider!(self, scan, predicate, continuation, limits)
    }
    fn candidates(
        &self,
        profile: &CandidateProfile,
        keys: &[String],
        limits: ScanLimits,
    ) -> Result<Page, StoreSpiError> {
        each_provider!(self, candidates, profile, keys, limits)
    }
}

impl WriteTransaction for TxHandle<'_> {
    fn apply(&mut self, op: Op) -> Result<(), StoreSpiError> {
        each_provider!(self, apply, op)
    }
    fn commit(self, durability: Durability) -> Result<Receipt, StoreSpiError> {
        each_provider!(self, commit, durability)
    }
    fn abort(self) {
        each_provider!(self, abort)
    }
}

impl BrainStore for StoreHandle {
    type Snapshot<'a> = SnapshotHandle<'a>;
    type Tx<'a> = TxHandle<'a>;
    fn read_snapshot(&self) -> Result<SnapshotHandle<'_>, StoreSpiError> {
        each_provider_map!(self, SnapshotHandle, read_snapshot)
    }
    fn begin_write(&mut self, intent: WriteIntent) -> Result<TxHandle<'_>, StoreSpiError> {
        each_provider_map!(self, TxHandle, begin_write, intent)
    }
    fn read_changes(
        &self,
        after: &Frontier,
        limit: u32,
    ) -> Result<(Vec<Change>, Frontier), StoreSpiError> {
        each_provider!(self, read_changes, after, limit)
    }
    fn export_snapshot(&self, destination: &std::path::Path) -> Result<(), StoreSpiError> {
        each_provider!(self, export_snapshot, destination)
    }
    fn diagnose(&self) -> Result<Diagnosis, StoreSpiError> {
        each_provider!(self, diagnose)
    }
    fn maintain_slice(&mut self, max_work_units: u64) -> Result<u64, StoreSpiError> {
        each_provider!(self, maintain_slice, max_work_units)
    }
    fn replay(
        &self,
        principal: &str,
        key: &str,
        canonical_ops: &[Op],
    ) -> Result<Option<Receipt>, StoreSpiError> {
        each_provider!(self, replay, principal, key, canonical_ops)
    }
}

/// Shadow one provider's candidate collector against another's exact scan.
/// Agreement is measured on canonical ids; the report never claims
/// equivalence, it counts disagreements.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ShadowReport {
    pub queries: usize,
    pub agreed: usize,
    pub only_in_primary: Vec<(String, String)>,
    pub only_in_shadow: Vec<(String, String)>,
    pub primary_rows_examined: u64,
    pub shadow_rows_examined: u64,
}

pub fn shadow_compare<P: BrainStore, S: BrainStore>(
    primary: &P,
    shadow: &S,
    queries: &[Vec<String>],
) -> Result<ShadowReport, StoreSpiError> {
    let mut report = ShadowReport {
        queries: queries.len(),
        agreed: 0,
        only_in_primary: Vec::new(),
        only_in_shadow: Vec::new(),
        primary_rows_examined: 0,
        shadow_rows_examined: 0,
    };
    for keys in queries {
        let label = keys.join(" ");
        let p = primary.read_snapshot()?.candidates(
            &CandidateProfile::ExactLexical,
            keys,
            ScanLimits::default(),
        )?;
        let s = shadow.read_snapshot()?.candidates(
            &CandidateProfile::ExactLexical,
            keys,
            ScanLimits::default(),
        )?;
        report.primary_rows_examined += p.coverage.rows_examined;
        report.shadow_rows_examined += s.coverage.rows_examined;
        let pt: Vec<String> = p.rows.iter().map(|r| r.id.canonical()).collect();
        let st: Vec<String> = s.rows.iter().map(|r| r.id.canonical()).collect();
        let mut same = true;
        for t in &pt {
            if !st.contains(t) {
                same = false;
                report.only_in_primary.push((label.clone(), t.clone()));
            }
        }
        for t in &st {
            if !pt.contains(t) {
                same = false;
                report.only_in_shadow.push((label.clone(), t.clone()));
            }
        }
        if same {
            report.agreed += 1;
        }
    }
    Ok(report)
}
