//! Multi-agent replication primitives: per-origin causal order, pending
//! parents, origin-lineage support, and fencing tokens for exclusive
//! effects. Convergence is not consensus: replicas converge on the *evidence*
//! while contradictory heads stay contradictory.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub const DDL: &str = r#"
CREATE TABLE IF NOT EXISTS pending_commits (
  origin_id TEXT NOT NULL, origin_counter INTEGER NOT NULL,
  payload_json TEXT NOT NULL CHECK(json_valid(payload_json)),
  received_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY(origin_id, origin_counter)
);
CREATE TABLE IF NOT EXISTS fences (
  resource TEXT PRIMARY KEY, holder TEXT NOT NULL, token INTEGER NOT NULL,
  expires_at TEXT, issued_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
"#;

pub fn ensure(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(DDL)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CausalRef {
    pub origin_id: String,
    pub origin_counter: i64,
}

/// One replicated commit as received from a peer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicatedCommit {
    pub origin_id: String,
    pub origin_counter: i64,
    pub parents: Vec<CausalRef>,
    pub principal: String,
    pub ack_profile: String,
    /// Record payload: record_id, kind, body, epistemic_status, parent revisions.
    pub record_id: String,
    pub kind: String,
    pub body: Value,
    #[serde(default)]
    pub parent_revisions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Ingest {
    Applied {
        sequence: i64,
        revision_id: String,
        drained: usize,
    },
    Pending {
        missing: Vec<CausalRef>,
    },
    Duplicate,
}

fn have(conn: &Connection, r: &CausalRef) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT COUNT(*) FROM commits WHERE origin_id = ?1 AND origin_counter = ?2",
        params![r.origin_id, r.origin_counter],
        |row| row.get::<_, i64>(0),
    )
    .map(|n| n > 0)
}

fn last_counter(conn: &Connection, origin: &str) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT MAX(origin_counter) FROM commits WHERE origin_id = ?1",
        [origin],
        |r| r.get::<_, Option<i64>>(0),
    )
}

/// Ingest a peer commit. Per-origin order is total: counter n needs n-1 (or
/// is the first from that origin); causal parents must be present. Anything
/// else is stored pending and replayed when its parents arrive. Presentation
/// order (local sequence) is never causal order.
pub fn ingest(conn: &Connection, commit: &ReplicatedCommit) -> Result<Ingest, String> {
    ensure(conn).map_err(|e| e.to_string())?;
    super::records::ensure_authoritative_schema(conn).map_err(|e| e.to_string())?;
    let me = CausalRef {
        origin_id: commit.origin_id.clone(),
        origin_counter: commit.origin_counter,
    };
    if have(conn, &me).map_err(|e| e.to_string())? {
        return Ok(Ingest::Duplicate);
    }
    let mut missing: Vec<CausalRef> = Vec::new();
    for parent in &commit.parents {
        if !have(conn, parent).map_err(|e| e.to_string())? {
            missing.push(parent.clone());
        }
    }
    let last = last_counter(conn, &commit.origin_id).map_err(|e| e.to_string())?;
    if let Some(counter) = last {
        if commit.origin_counter != counter + 1 {
            missing.push(CausalRef {
                origin_id: commit.origin_id.clone(),
                origin_counter: counter + 1,
            });
        }
    }
    if !missing.is_empty() {
        conn.execute(
            "INSERT OR REPLACE INTO pending_commits (origin_id, origin_counter, payload_json) VALUES (?1, ?2, ?3)",
            params![commit.origin_id, commit.origin_counter, serde_json::to_string(commit).map_err(|e| e.to_string())?],
        )
        .map_err(|e| e.to_string())?;
        missing.sort_by(|a, b| {
            (&a.origin_id, a.origin_counter).cmp(&(&b.origin_id, b.origin_counter))
        });
        missing.dedup();
        return Ok(Ingest::Pending { missing });
    }
    let sequence = apply(conn, commit)?;
    let revision_id = super::records::heads(conn, &commit.record_id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .last()
        .unwrap_or_default();
    let drained = drain_pending(conn)?;
    Ok(Ingest::Applied {
        sequence,
        revision_id,
        drained,
    })
}

fn apply(conn: &Connection, commit: &ReplicatedCommit) -> Result<i64, String> {
    let commit_id = format!("{}:{}", commit.origin_id, commit.origin_counter);
    conn.execute(
        "INSERT INTO commits (commit_id, principal_id, idempotency_key, ack_profile, origin_id, origin_counter) VALUES (?1, ?2, NULL, ?3, ?4, ?5)",
        params![
            commit_id,
            commit.principal,
            if commit.ack_profile == "power_loss_assumed" { "power_loss_assumed" } else { "process_crash" },
            commit.origin_id,
            commit.origin_counter
        ],
    )
    .map_err(|e| e.to_string())?;
    let sequence = conn.last_insert_rowid();
    // Concurrent heads stay concurrent: a peer revision never replaces a
    // local head it did not descend from.
    super::records::append_revision(
        conn,
        sequence,
        super::records::NewRevision {
            record_id: &commit.record_id,
            kind: &commit.kind,
            retention: "durable",
            body: commit.body.clone(),
            epistemic_status: "asserted",
            parents: &commit.parent_revisions,
            replace_parents: true,
            representation_version: "replicated/1",
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(sequence)
}

/// Replay pending commits whose parents have since arrived. Loops until a
/// pass applies nothing.
pub fn drain_pending(conn: &Connection) -> Result<usize, String> {
    let mut total = 0usize;
    loop {
        let mut stmt = conn
            .prepare("SELECT origin_id, origin_counter, payload_json FROM pending_commits ORDER BY origin_id, origin_counter")
            .map_err(|e| e.to_string())?;
        let rows: Vec<(String, i64, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let mut applied = 0usize;
        for (origin, counter, payload) in rows {
            let Ok(commit) = serde_json::from_str::<ReplicatedCommit>(&payload) else {
                continue;
            };
            let mut parents_ok = true;
            for parent in &commit.parents {
                if !have(conn, parent).map_err(|e| e.to_string())? {
                    parents_ok = false;
                    break;
                }
            }
            let order_ok = match last_counter(conn, &origin).map_err(|e| e.to_string())? {
                Some(c) => c + 1 == counter,
                None => true,
            };
            if parents_ok && order_ok {
                apply(conn, &commit)?;
                conn.execute(
                    "DELETE FROM pending_commits WHERE origin_id = ?1 AND origin_counter = ?2",
                    params![origin, counter],
                )
                .map_err(|e| e.to_string())?;
                applied += 1;
            }
        }
        total += applied;
        if applied == 0 {
            return Ok(total);
        }
    }
}

pub fn pending_count(conn: &Connection) -> i64 {
    ensure(conn).ok();
    conn.query_row("SELECT COUNT(*) FROM pending_commits", [], |r| r.get(0))
        .unwrap_or(0)
}

/// Origin-lineage support: copies of a claim collapse to their origin, agent
/// identity is not lineage, and unknown lineage earns no bonus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportWitness {
    pub origin_id: Option<String>,
    pub agent: String,
    /// Copied from another witness (a re-statement, a forward, a paste).
    pub copied_from: Option<String>,
}

pub fn independent_support(witnesses: &[SupportWitness]) -> usize {
    let mut origins: BTreeSet<String> = BTreeSet::new();
    for w in witnesses {
        let Some(origin) = w.origin_id.as_deref() else {
            continue;
        };
        let root = w.copied_from.as_deref().unwrap_or(origin);
        origins.insert(root.to_string());
    }
    origins.len()
}

/// Fencing token for an exclusive effect. A lease is advisory; the effect
/// is authorized only when the resource owner checks the token it holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Fence {
    pub resource: String,
    pub holder: String,
    pub token: i64,
}

pub fn acquire_fence(
    conn: &Connection,
    resource: &str,
    holder: &str,
    ttl_seconds: i64,
) -> Result<Fence, String> {
    ensure(conn).map_err(|e| e.to_string())?;
    let now = chrono::Utc::now();
    let current: Option<(String, i64, Option<String>)> = conn
        .query_row(
            "SELECT holder, token, expires_at FROM fences WHERE resource = ?1",
            [resource],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some((h, token, expires)) = &current {
        let live = fence_unexpired(expires.as_deref(), now);
        if live && h != holder {
            return Err(format!("resource {resource} fenced by {h} (token {token})"));
        }
    }
    let token = current.map(|(_, t, _)| t + 1).unwrap_or(1);
    let expires = (now + chrono::Duration::seconds(ttl_seconds.max(1))).to_rfc3339();
    conn.execute(
        "INSERT INTO fences (resource, holder, token, expires_at) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(resource) DO UPDATE SET holder = excluded.holder, token = excluded.token, expires_at = excluded.expires_at, issued_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')",
        params![resource, holder, token, expires],
    )
    .map_err(|e| e.to_string())?;
    Ok(Fence {
        resource: resource.to_string(),
        holder: holder.to_string(),
        token,
    })
}

/// NULL expiry is a permanent lease. An unparseable timestamp is not live:
/// a garbage `expires_at` must not lock the resource forever or authorize
/// exclusive effects.
fn fence_unexpired(expires_at: Option<&str>, now: chrono::DateTime<chrono::Utc>) -> bool {
    match expires_at {
        None => true,
        Some(raw) => chrono::DateTime::parse_from_rfc3339(raw)
            .map(|expires| expires > now)
            .unwrap_or(false),
    }
}

/// The resource owner's check before an exclusive effect. A stale token
/// (a newer fence was issued), an expired lease, or an unknown resource is
/// refused; memory never self-authorizes execution.
pub fn check_fence(conn: &Connection, resource: &str, token: i64) -> Result<(), Value> {
    if let Err(err) = ensure(conn) {
        return Err(
            json!({"status": "denied", "error": format!("fence table unavailable: {err}"), "resource": resource}),
        );
    }
    let current: Option<(i64, Option<String>)> = conn
        .query_row(
            "SELECT token, expires_at FROM fences WHERE resource = ?1",
            [resource],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|err| {
            json!({"status": "denied", "error": format!("fence lookup failed: {err}"), "resource": resource})
        })?;
    match current {
        Some((t, expires)) if t == token => {
            if fence_unexpired(expires.as_deref(), chrono::Utc::now()) {
                Ok(())
            } else {
                Err(
                    json!({"status": "denied", "error": "expired fencing token", "resource": resource, "presented": token}),
                )
            }
        }
        Some((t, _)) => Err(
            json!({"status": "denied", "error": "stale fencing token", "resource": resource, "current_token": t, "presented": token}),
        ),
        None => Err(
            json!({"status": "denied", "error": "no fence for resource; leases are advisory and never authorize exclusive effects", "resource": resource}),
        ),
    }
}
