//! Reflex snapshot: a warm, derived, discardable Level-0 decision surface.
//!
//! The snapshot is *derived* from the brain: versioned header (frontier,
//! policy epoch, projection versions), an anchor dictionary, a trigger
//! automaton with positive/negative guards, Thread applicability tables and
//! pre-rendered orientation lines. It is published atomically (write-then-
//! rename) and readers keep the generation they loaded; a reader never
//! mutates the bytes it mapped. Deleting the file loses nothing: the next
//! publish rebuilds it. A stale snapshot is `Expired`, never "no memory".

use crate::adapter::SnapshotState;
use crate::clockwork::{extract_anchors, Anchor, AnchorKind};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const REFLEX_FORMAT_VERSION: u32 = 1;
pub const DEFAULT_MAX_RECORDS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReflexHeader {
    pub format_version: u32,
    pub generation: u64,
    pub brain_id: String,
    pub restore_epoch: String,
    pub policy_epoch: String,
    /// versions.id high-water mark the snapshot was cut at.
    pub frontier_sequence: i64,
    pub projection_versions: BTreeMap<String, String>,
    pub built_at: String,
    pub records: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreRendered {
    pub source: String,
    pub line: String,
    pub kind: String,
}

/// Trigger automaton: exact anchor value → record set, with guards. A
/// negative guard suppresses a trigger when the query names a superseding
/// anchor (e.g. the query mentions a newer ticket that superseded the record).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Automaton {
    pub positive: BTreeMap<String, Vec<usize>>,
    pub negative: BTreeMap<String, Vec<usize>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReflexSnapshot {
    pub header: ReflexHeader,
    /// anchor value → anchor kind, the dictionary every trigger is keyed by.
    pub anchor_dictionary: BTreeMap<String, String>,
    pub automaton: Automaton,
    /// thread id → record indexes applicable to that Thread.
    pub thread_applicability: BTreeMap<String, Vec<usize>>,
    pub views: Vec<PreRendered>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Level0 {
    pub hits: Vec<PreRendered>,
    pub suppressed: usize,
    pub anchors_consulted: usize,
    pub micros: u128,
    /// True when Level-0 could not answer and the caller must fall back.
    pub fallback: bool,
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn frontier_sequence(conn: &Connection) -> i64 {
    conn.query_row("SELECT COALESCE(MAX(id), 0) FROM versions", [], |r| {
        r.get(0)
    })
    .unwrap_or(0)
}

/// Cut a snapshot from the current brain state.
pub fn build(
    conn: &Connection,
    generation: u64,
    max_records: usize,
) -> Result<ReflexSnapshot, String> {
    let (brain_id, restore_epoch, policy_epoch) = crate::db::records::brain_epochs(conn);
    let frontier = frontier_sequence(conn);
    let mut views = Vec::new();
    let mut dictionary = BTreeMap::new();
    let mut automaton = Automaton::default();
    let mut threads: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut stmt = conn
        .prepare("SELECT id, decision, status FROM decisions WHERE status = 'active' ORDER BY id DESC LIMIT ?1")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([max_records as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    for row in rows.flatten() {
        let (id, text, status) = row;
        let index = views.len();
        views.push(PreRendered {
            source: format!("decision::{id}"),
            line: format!("- {} ({status})", text.trim()),
            kind: "decision".into(),
        });
        let anchors: Vec<Anchor> = extract_anchors(&text, &[], 24);
        for anchor in anchors.iter().filter(|a| a.specificity >= 2) {
            dictionary.insert(anchor.value.clone(), anchor.kind.as_str().to_string());
            automaton
                .positive
                .entry(anchor.value.clone())
                .or_default()
                .push(index);
        }
        if let Some(sup) = superseder_anchor(conn, id) {
            automaton.negative.entry(sup).or_default().push(index);
        }
    }
    let mut tstmt = conn.prepare("SELECT tm.thread_id, a.namespace, a.address FROM thread_members tm JOIN addresses a ON a.record_id = tm.record_id AND a.scheme = 'legacy' ORDER BY tm.thread_id").map_err(|e| e.to_string())?;
    let trows = tstmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    for (thread, namespace, address) in trows.flatten() {
        let source = format!("{namespace}::{address}");
        if let Some(index) = views.iter().position(|v| v.source == source) {
            threads.entry(thread).or_default().push(index);
        }
    }
    let projection_versions = [
        ("exact".to_string(), frontier.to_string()),
        (
            "clock".to_string(),
            crate::clockwork::current_generation(conn)
                .map(|g| g.to_string())
                .unwrap_or_else(|_| "0".into()),
        ),
    ]
    .into_iter()
    .collect();
    Ok(ReflexSnapshot {
        header: ReflexHeader {
            format_version: REFLEX_FORMAT_VERSION,
            generation,
            brain_id,
            restore_epoch,
            policy_epoch,
            frontier_sequence: frontier,
            projection_versions,
            built_at: now_iso(),
            records: views.len(),
        },
        anchor_dictionary: dictionary,
        automaton,
        thread_applicability: threads,
        views,
    })
}

/// A ticket anchor of the decision that superseded this one, if any.
fn superseder_anchor(conn: &Connection, decision_id: i64) -> Option<String> {
    let text: String = conn
        .query_row("SELECT d2.decision FROM decisions d2 WHERE d2.supersedes_id = ?1 ORDER BY d2.id DESC LIMIT 1", [decision_id], |r| r.get(0))
        .ok()?;
    extract_anchors(&text, &[], 8)
        .into_iter()
        .find(|a| a.kind == AnchorKind::Ticket)
        .map(|a| a.value)
}

pub fn snapshot_path(home: &Path) -> PathBuf {
    home.join("reflex").join("snapshot.json")
}

/// Atomic publish: write to a temp file beside the target, fsync, rename.
/// Readers holding the previous generation keep reading their bytes.
pub fn publish(snapshot: &ReflexSnapshot, path: &Path) -> Result<(), String> {
    let dir = path.parent().ok_or("snapshot path has no parent")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!(".snapshot.{}.tmp", snapshot.header.generation));
    let body = serde_json::to_vec(snapshot).map_err(|e| e.to_string())?;
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        f.write_all(&body).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub fn load(path: &Path) -> Option<ReflexSnapshot> {
    let bytes = std::fs::read(path).ok()?;
    let snapshot: ReflexSnapshot = serde_json::from_slice(&bytes).ok()?;
    (snapshot.header.format_version == REFLEX_FORMAT_VERSION).then_some(snapshot)
}

/// Validate a loaded snapshot against the live brain: frontier, restore
/// epoch and policy epoch must all match for it to be `Fresh`.
pub fn state_for(snapshot: Option<&ReflexSnapshot>, conn: &Connection) -> SnapshotState {
    let Some(s) = snapshot else {
        return SnapshotState::Unavailable;
    };
    let (brain_id, restore_epoch, policy_epoch) = crate::db::records::brain_epochs(conn);
    if s.header.brain_id != brain_id
        || s.header.restore_epoch != restore_epoch
        || s.header.policy_epoch != policy_epoch
    {
        return SnapshotState::Expired;
    }
    if s.header.frontier_sequence != frontier_sequence(conn) {
        return SnapshotState::Expired;
    }
    SnapshotState::Fresh
}

/// Warm Level-0 decision: exact anchor triggers over the dictionary with
/// negative guards and optional Thread applicability. No I/O, no tokenizer.
pub fn level0(
    snapshot: &ReflexSnapshot,
    query: &str,
    thread: Option<&str>,
    limit: usize,
) -> Level0 {
    let start = Instant::now();
    let anchors = extract_anchors(query, &[], 16);
    let mut hits: BTreeSet<usize> = BTreeSet::new();
    let mut suppressed = BTreeSet::new();
    for anchor in anchors.iter().filter(|a| a.specificity >= 2) {
        if let Some(indexes) = snapshot.automaton.positive.get(&anchor.value) {
            hits.extend(indexes.iter().copied());
        }
    }
    for anchor in &anchors {
        if let Some(indexes) = snapshot.automaton.negative.get(&anchor.value) {
            for i in indexes {
                if hits.remove(i) {
                    suppressed.insert(*i);
                }
            }
        }
    }
    if let Some(thread) = thread.and_then(|t| snapshot.thread_applicability.get(t)) {
        let applicable: BTreeSet<usize> = thread.iter().copied().collect();
        let narrowed: BTreeSet<usize> = hits.intersection(&applicable).copied().collect();
        if !narrowed.is_empty() {
            hits = narrowed;
        }
    }
    let rendered: Vec<PreRendered> = hits
        .iter()
        .take(limit)
        .filter_map(|i| snapshot.views.get(*i).cloned())
        .collect();
    Level0 {
        fallback: rendered.is_empty(),
        hits: rendered,
        suppressed: suppressed.len(),
        anchors_consulted: anchors.len(),
        micros: start.elapsed().as_micros(),
    }
}

/// Percentile helper for the benchmark harness (nearest-rank).
pub fn percentile(sorted_micros: &[u128], p: f64) -> u128 {
    if sorted_micros.is_empty() {
        return 0;
    }
    let rank = ((p / 100.0) * sorted_micros.len() as f64).ceil().max(1.0) as usize;
    sorted_micros[rank.min(sorted_micros.len()) - 1]
}
