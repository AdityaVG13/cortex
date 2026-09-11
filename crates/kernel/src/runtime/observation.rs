//! Exact observation intake. Source registration is an operator/library action,
//! never inferred from event text. A source cursor and all its accepted
//! occurrences commit together. This module does not interpret prose or learn.
use super::CortexRuntime;
use crate::clockwork::AnchorKind;
use crate::db::{compiled, outbox, records};
use asupersync::Cx;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;

pub const MAX_CAPTURE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_BATCH_EVENTS: usize = 128;
const DDL: &str = r#"
CREATE TABLE IF NOT EXISTS observation_sources (
 principal TEXT NOT NULL, source_key TEXT NOT NULL,
 scope_id TEXT NOT NULL REFERENCES scopes(scope_id), scope_label TEXT NOT NULL,
 role TEXT NOT NULL, max_bytes INTEGER NOT NULL CHECK(max_bytes>0),
 enabled INTEGER NOT NULL CHECK(enabled IN (0,1)), policy_epoch TEXT NOT NULL,
 PRIMARY KEY(principal,source_key)
);
CREATE TABLE IF NOT EXISTS observation_events (
 principal TEXT NOT NULL, source_key TEXT NOT NULL, generation TEXT NOT NULL, event_key TEXT NOT NULL,
 source_id TEXT NOT NULL UNIQUE REFERENCES sources(source_id),
 revision_id TEXT NOT NULL REFERENCES revisions(revision_id),
 receipt_json TEXT NOT NULL CHECK(json_valid(receipt_json)),
 PRIMARY KEY(principal,source_key,generation,event_key),
 FOREIGN KEY(principal,source_key) REFERENCES observation_sources(principal,source_key)
);
CREATE TABLE IF NOT EXISTS observation_cursors (
 principal TEXT NOT NULL, source_key TEXT NOT NULL, generation TEXT NOT NULL,
 byte_offset INTEGER NOT NULL CHECK(byte_offset>=0),
 PRIMARY KEY(principal,source_key,generation),
 FOREIGN KEY(principal,source_key) REFERENCES observation_sources(principal,source_key)
);
"#;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObservationRole {
    Document,
    UserStatement,
    AgentAssertion,
    ToolReport,
    DeliveryOnly,
}
impl ObservationRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::UserStatement => "user_statement",
            Self::AgentAssertion => "agent_assertion",
            Self::ToolReport => "tool_report",
            Self::DeliveryOnly => "delivery_only",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SourceSpec {
    pub key: String,
    pub scope: String,
    pub role: ObservationRole,
    pub max_bytes: usize,
}
impl SourceSpec {
    pub fn document(key: impl Into<String>, scope: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            scope: scope.into(),
            role: ObservationRole::Document,
            max_bytes: 64 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ObservationEvent {
    pub event_key: String,
    pub text: String,
    pub observed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationReceipt {
    pub source_id: String,
    pub record_id: String,
    pub revision_id: String,
    pub sequence: i64,
    pub restore_epoch: String,
    pub ack_profile: crate::protocol::AckProfile,
    pub retained_bytes: usize,
    pub duplicate: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapturedObservation {
    pub source_id: String,
    pub source_key: String,
    pub scope_id: String,
    pub generation: String,
    pub role: String,
    pub event_key: String,
    pub text: String,
    pub observed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TailReceipt {
    pub accepted: Vec<ObservationReceipt>,
    pub next_offset: u64,
    pub uncommitted_tail_bytes: usize,
}

pub(super) struct GrantedSource {
    scope_id: String,
    pub(super) role: String,
    max_bytes: usize,
}

fn check_label(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > 1024 {
        return Err("invalid_source_identity".into());
    }
    Ok(())
}

pub(crate) fn scope_is_path(value: &str) -> bool {
    value.contains('/') || value.contains('\\')
}

/// Canonical observation scope. Path-like values use the same path
/// identity as CQR roots; coarse labels (`project`, `repo`) stay as given.
pub fn normalize_scope(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if !scope_is_path(trimmed) {
        return trimmed.to_string();
    }
    let mut value = crate::clockwork::normalize_anchor_value(AnchorKind::Path, trimmed);
    loop {
        if let Some(stripped) = value.strip_suffix("/**") {
            value = stripped.to_string();
            continue;
        }
        if let Some(stripped) = value.strip_suffix("/*") {
            value = stripped.to_string();
            continue;
        }
        if let Some(stripped) = value.strip_suffix('*') {
            value = stripped.to_string();
            continue;
        }
        break;
    }
    value.trim_end_matches('/').to_string()
}

pub(crate) fn scopes_compatible(candidate: &str, query: &str) -> bool {
    if candidate == query {
        return true;
    }
    candidate.starts_with(&(query.to_string() + "/"))
        || query.starts_with(&(candidate.to_string() + "/"))
}

/// Scopes to pull for a caller. Empty paths keep the exact extra label
/// (default `project`). Named roots include compatible path-scoped sources
/// plus the extra/unscoped bucket (`project` when extra is omitted).
pub(crate) fn resolve_query_scopes(
    conn: &Connection,
    principal: &str,
    paths: &[String],
    extra_scope: Option<&str>,
) -> Result<Vec<String>, String> {
    let query_paths: Vec<String> = paths
        .iter()
        .map(|p| normalize_scope(p))
        .filter(|p| !p.is_empty())
        .collect();
    let extra = extra_scope
        .map(normalize_scope)
        .filter(|s| !s.is_empty());
    if query_paths.is_empty() {
        return Ok(vec![extra.unwrap_or_else(|| "project".into())]);
    }
    let mut wanted: BTreeSet<String> = BTreeSet::new();
    wanted.insert(extra.unwrap_or_else(|| "project".into()));
    for path in &query_paths {
        wanted.insert(path.clone());
    }
    let mut stmt = conn
        .prepare("SELECT DISTINCT scope_label FROM observation_sources WHERE principal=?1")
        .map_err(|e| e.to_string())?;
    let stored = stmt
        .query_map(params![principal], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    for label in stored {
        let normalized = normalize_scope(&label);
        if !scope_is_path(&normalized) {
            continue;
        }
        if query_paths
            .iter()
            .any(|query| scopes_compatible(&normalized, query))
        {
            wanted.insert(label);
        }
    }
    Ok(wanted.into_iter().collect())
}

pub(super) fn ensure(conn: &Connection) -> Result<(), String> {
    records::ensure_authoritative_schema(conn).map_err(|err| err.to_string())?;
    crate::db::capture_policy::ensure(conn).map_err(|err| err.to_string())?;
    conn.execute_batch(DDL).map_err(|err| err.to_string())
}

pub(super) fn granted(
    conn: &Connection,
    principal: &str,
    key: &str,
    writing: bool,
) -> Result<GrantedSource, String> {
    let row = conn.query_row(
    "SELECT scope_id,scope_label,role,max_bytes,enabled,policy_epoch FROM observation_sources WHERE principal=?1 AND source_key=?2",
    params![principal,key], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?,r.get::<_,bool>(4)?,r.get::<_,String>(5)?)),
).optional().map_err(|err| err.to_string())?.ok_or("source_not_authorized")?;
    let policy: String = conn
        .query_row(
            "SELECT policy_epoch FROM brain_meta WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(|err| err.to_string())?;
    if !row.4 || row.5 != policy {
        return Err("source_disabled_or_policy_stale".into());
    }
    let state: Option<String> = conn.query_row(
    "SELECT state FROM capture_policy WHERE scope IN (?1,'*') ORDER BY CASE WHEN scope=?1 THEN 0 ELSE 1 END LIMIT 1",
    params![row.1], |r| r.get(0),
).optional().map_err(|err| err.to_string())?;
    if state
        .as_deref()
        .is_some_and(|s| s == "stopped" || (writing && s != "active"))
    {
        return Err("capture_disabled".into());
    }
    Ok(GrantedSource {
        scope_id: row.0,
        role: row.2,
        max_bytes: usize::try_from(row.3).map_err(|_| "invalid_capture_limit")?,
    })
}

pub(super) fn offset(
    conn: &Connection,
    principal: &str,
    key: &str,
    generation: &str,
) -> Result<u64, String> {
    let value: i64 = conn.query_row("SELECT byte_offset FROM observation_cursors WHERE principal=?1 AND source_key=?2 AND generation=?3", params![principal,key,generation], |r| r.get(0)).optional().map_err(|err| err.to_string())?.unwrap_or(0);
    u64::try_from(value).map_err(|_| "invalid_source_cursor".into())
}

pub(super) fn ensure_source(
    conn: &Connection,
    principal: &str,
    spec: &SourceSpec,
) -> Result<(), String> {
    check_label(&spec.key)?;
    let scope_label = normalize_scope(&spec.scope);
    check_label(&scope_label)?;
    if spec.max_bytes == 0 || spec.max_bytes > MAX_CAPTURE_BYTES {
        return Err("invalid_capture_limit".into());
    }
    ensure(conn)?;
    let scope = format!("observation-scope:{}", json!([principal, scope_label]));
    let existing: Option<(String, String, i64)> = conn
        .query_row(
            "SELECT scope_id,role,max_bytes FROM observation_sources WHERE principal=?1 AND source_key=?2",
            params![principal, spec.key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|err| err.to_string())?;
    if let Some((old_scope, role, limit)) = existing {
        if old_scope != scope || role != spec.role.as_str() || limit != spec.max_bytes as i64 {
            return Err("source_registration_conflict".into());
        }
        return Ok(());
    }
    conn.execute(
        "INSERT OR IGNORE INTO scopes(scope_id,owner_id,kind,descriptor) VALUES(?1,?2,'observation',?3)",
        params![scope, principal, json!({"label": scope_label}).to_string()],
    )
    .map_err(|err| err.to_string())?;
    let policy: String = conn
        .query_row(
            "SELECT policy_epoch FROM brain_meta WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(|err| err.to_string())?;
    conn.execute(
        "INSERT INTO observation_sources VALUES(?1,?2,?3,?4,?5,?6,1,?7)",
        params![
            principal,
            spec.key,
            scope,
            scope_label,
            spec.role.as_str(),
            spec.max_bytes as i64,
            policy
        ],
    )
    .map_err(|err| err.to_string())?;
    Ok(())
}

pub(crate) fn capture_registered(
    conn: &Connection,
    principal: &str,
    source: &str,
    generation: &str,
    event: ObservationEvent,
) -> Result<ObservationReceipt, String> {
    check_label(source)?;
    check_label(generation)?;
    ensure(conn)?;
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .map_err(|err| err.to_string())?;
    let grant = granted(&tx, principal, source, true)?;
    let receipt = capture(&tx, principal, source, generation, &grant, event)?;
    tx.commit().map_err(|err| err.to_string())?;
    Ok(receipt)
}

pub(crate) fn source_capture_limit(
    conn: &Connection,
    principal: &str,
    source: &str,
) -> Result<usize, String> {
    ensure(conn)?;
    Ok(granted(conn, principal, source, true)?.max_bytes)
}
pub(super) fn capture(
    conn: &Connection,
    principal: &str,
    key: &str,
    generation: &str,
    grant: &GrantedSource,
    event: ObservationEvent,
) -> Result<ObservationReceipt, String> {
    check_label(&event.event_key)?;
    if event.text.len() > grant.max_bytes {
        return Err("capture_byte_limit".into());
    }
    if event
        .observed_at
        .as_ref()
        .is_some_and(|time| chrono::DateTime::parse_from_rfc3339(time).is_err())
    {
        return Err("invalid_observation_time".into());
    }
    if crate::handlers::redact_secrets(&event.text) != event.text {
        return Err("capture_secret_rejected".into());
    }
    let body = json!({"observation":event,"role":grant.role});
    let old: Option<(String,String)> = conn.query_row(
    "SELECT r.body_json,e.receipt_json FROM observation_events e JOIN revisions r ON r.revision_id=e.revision_id WHERE e.principal=?1 AND e.source_key=?2 AND e.generation=?3 AND e.event_key=?4",
    params![principal,key,generation,event.event_key], |r| Ok((r.get(0)?,r.get(1)?)),
).optional().map_err(|err| err.to_string())?;
    if let Some((previous, receipt)) = old {
        if serde_json::from_str::<serde_json::Value>(&previous).map_err(|err| err.to_string())?
            != body
        {
            return Err("event_identity_conflict".into());
        }
        let mut receipt: ObservationReceipt =
            serde_json::from_str(&receipt).map_err(|err| err.to_string())?;
        let availability: String = conn
            .query_row(
                "SELECT availability FROM sources WHERE source_id=?1",
                params![receipt.source_id],
                |r| r.get(0),
            )
            .map_err(|err| err.to_string())?;
        if availability != "owned_inline" {
            return Err("source_unavailable".into());
        }
        receipt.duplicate = true;
        return Ok(receipt);
    }
    if outbox::debt(conn).refuse_intake() {
        return Err(
            "maintenance_debt_hard_limit: run cortex maintain before capturing more".into(),
        );
    }
    let ack = crate::store_spi::sqlite::ack_profile(conn);
    let sequence =
        records::append_commit(conn, principal, None, super::ack_profile_label_pub(&ack))
            .map_err(|err| err.to_string())?;
    let source_id = format!("source:{}", uuid::Uuid::new_v4());
    let record_id = format!("observation:{}", uuid::Uuid::new_v4());
    let revision_id = format!("{record_id}@1");
    conn.execute("INSERT INTO sources(source_id,scope_id,origin_id,media_type,availability,inline_payload,byte_length,capture_sequence) VALUES(?1,?2,?3,'text/plain;charset=utf-8','owned_inline',?4,?5,?6)", params![source_id,grant.scope_id,key,event.text.as_bytes(),event.text.len() as i64,sequence]).map_err(|err| err.to_string())?;
    conn.execute("INSERT INTO records(record_id,scope_id,kind,retention,created_sequence) VALUES(?1,?2,'observation','operational',?3)", params![record_id,grant.scope_id,sequence]).map_err(|err| err.to_string())?;
    conn.execute("INSERT INTO revisions(revision_id,record_id,body_json,epistemic_status,recorded_sequence,representation_version) VALUES(?1,?2,?3,'asserted',?4,'observation/1')", params![revision_id,record_id,body.to_string(),sequence]).map_err(|err| err.to_string())?;
    conn.execute(
        "INSERT INTO record_heads(record_id,revision_id) VALUES(?1,?2)",
        params![record_id, revision_id],
    )
    .map_err(|err| err.to_string())?;
    conn.execute(
        "INSERT INTO revision_sources(revision_id,source_id,role,extent_json) VALUES(?1,?2,?3,?4)",
        params![
            revision_id,
            source_id,
            grant.role,
            json!({"start":0,"end":event.text.len()}).to_string()
        ],
    )
    .map_err(|err| err.to_string())?;
    conn.execute("INSERT INTO change_items(sequence,ordinal,scope_id,record_id,change_kind) VALUES(?1,0,?2,?3,'captured')", params![sequence,grant.scope_id,record_id]).map_err(|err| err.to_string())?;
    compiled::bump_guard(conn, &grant.scope_id, "observation").map_err(|err| err.to_string())?;
    outbox::enqueue_for_commit(
        conn,
        sequence,
        &["checkpoint_wal"],
        json!({"source_id":source_id}),
    )
    .map_err(|err| err.to_string())?;
    let receipt = ObservationReceipt {
        source_id,
        record_id,
        revision_id,
        sequence,
        restore_epoch: records::brain_epochs(conn).1,
        ack_profile: ack,
        retained_bytes: event.text.len(),
        duplicate: false,
    };
    conn.execute("INSERT INTO observation_events(principal,source_key,generation,event_key,source_id,revision_id,receipt_json) VALUES(?1,?2,?3,?4,?5,?6,?7)", params![principal,key,generation,event.event_key,receipt.source_id,receipt.revision_id,serde_json::to_string(&receipt).map_err(|err| err.to_string())?]).map_err(|err| err.to_string())?;
    super::cycle::on_capture(conn, principal, key)?;

    Ok(receipt)
}

impl CortexRuntime {
    pub fn observation_principal(&self) -> Result<String, String> {
        match (self.state().team_mode, self.state().default_owner_id) {
            (true, Some(id)) => Ok(format!("user:{id}")),
            (true, None) => Err("local_owner_required".into()),
            (false, _) => Ok("local".into()),
        }
    }

    /// Explicit local operator registration. Observation bodies cannot create or widen it.
    pub async fn register_source(&self, cx: &Cx, spec: SourceSpec) -> Result<(), String> {
        let principal = self.observation_principal()?;
        let mut conn = self
            .state()
            .db
            .lock(cx)
            .await
            .map_err(|err| err.to_string())?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|err| err.to_string())?;
        ensure_source(&tx, &principal, &spec)?;
        tx.commit().map_err(|err| err.to_string())
    }

    pub async fn set_source_enabled(
        &self,
        cx: &Cx,
        source: &str,
        enabled: bool,
    ) -> Result<(), String> {
        let principal = self.observation_principal()?;
        let conn = self
            .state()
            .db
            .lock(cx)
            .await
            .map_err(|err| err.to_string())?;
        ensure(&conn)?;
        let changed = conn
            .execute(
                "UPDATE observation_sources SET enabled=?3, policy_epoch=CASE WHEN ?3=1 THEN (SELECT policy_epoch FROM brain_meta WHERE singleton=1) ELSE policy_epoch END WHERE principal=?1 AND source_key=?2",
                params![principal, source, enabled],
            )
            .map_err(|err| err.to_string())?;
        if changed == 0 {
            return Err("source_not_authorized".into());
        }
        Ok(())
    }

    /// Import one explicitly registered file; file content cannot supply a principal or grant.
    pub async fn observe_file(
        &self,
        cx: &Cx,
        path: &std::path::Path,
    ) -> Result<ObservationReceipt, String> {
        self.observation_principal()?;
        let owner = if self.state().team_mode {
            self.state().default_owner_id
        } else {
            None
        };
        let conn = self
            .state()
            .db
            .lock(cx)
            .await
            .map_err(|err| err.to_string())?;
        crate::indexer::index_file(&conn, path, owner)
    }
    pub async fn observe(
        &self,
        cx: &Cx,
        source: &str,
        generation: &str,
        event: ObservationEvent,
    ) -> Result<ObservationReceipt, String> {
        let principal = self.observation_principal()?;
        let conn = self
            .state()
            .db
            .lock(cx)
            .await
            .map_err(|err| err.to_string())?;
        capture_registered(&conn, &principal, source, generation, event)
    }

    pub async fn source_offset(
        &self,
        cx: &Cx,
        source: &str,
        generation: &str,
    ) -> Result<u64, String> {
        let principal = self.observation_principal()?;
        let conn = self
            .state()
            .db
            .lock(cx)
            .await
            .map_err(|err| err.to_string())?;
        ensure(&conn)?;
        granted(&conn, &principal, source, false)?;
        offset(&conn, &principal, source, generation)
    }

    /// Normalized JSONL only. A trailing partial record remains at its source.
    pub async fn tail_observations(
        &self,
        cx: &Cx,
        source: &str,
        generation: &str,
        start: u64,
        chunk: &[u8],
    ) -> Result<TailReceipt, String> {
        check_label(source)?;
        check_label(generation)?;
        if chunk.len() > MAX_CAPTURE_BYTES {
            return Err("capture_batch_byte_limit".into());
        }
        let cutoff = chunk
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |i| i + 1);
        let mut events = Vec::new();
        for line in chunk[..cutoff].split(|byte| *byte == b'\n') {
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            if events.len() == MAX_BATCH_EVENTS {
                return Err("capture_batch_event_limit".into());
            }
            events.push(
                serde_json::from_slice::<ObservationEvent>(line)
                    .map_err(|err| format!("malformed_complete_record: {err}"))?,
            );
        }
        let next = start
            .checked_add(cutoff as u64)
            .filter(|n| *n <= i64::MAX as u64)
            .ok_or("invalid_source_cursor")?;
        let principal = self.observation_principal()?;
        let mut conn = self
            .state()
            .db
            .lock(cx)
            .await
            .map_err(|err| err.to_string())?;
        ensure(&conn)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|err| err.to_string())?;
        let grant = granted(&tx, &principal, source, true)?;
        if offset(&tx, &principal, source, generation)? != start {
            return Err("cursor_conflict".into());
        }
        let mut accepted = Vec::new();
        for event in events {
            cx.checkpoint().map_err(|err| err.to_string())?;
            accepted.push(capture(&tx, &principal, source, generation, &grant, event)?);
        }
        tx.execute("INSERT INTO observation_cursors VALUES(?1,?2,?3,?4) ON CONFLICT(principal,source_key,generation) DO UPDATE SET byte_offset=excluded.byte_offset", params![principal,source,generation,next as i64]).map_err(|err| err.to_string())?;
        tx.commit().map_err(|err| err.to_string())?;
        Ok(TailReceipt {
            accepted,
            next_offset: next,
            uncommitted_tail_bytes: chunk.len() - cutoff,
        })
    }

    pub async fn read_observation(
        &self,
        cx: &Cx,
        source_id: &str,
    ) -> Result<CapturedObservation, String> {
        let principal = self.observation_principal()?;
        let mut conn = self
            .state()
            .db
            .lock(cx)
            .await
            .map_err(|err| err.to_string())?;
        ensure(&conn)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|err| err.to_string())?;
        let (key,generation,body,available): (String,String,String,String) = tx.query_row(
        "SELECT e.source_key,e.generation,r.body_json,s.availability FROM observation_events e JOIN revisions r ON r.revision_id=e.revision_id JOIN sources s ON s.source_id=e.source_id WHERE e.principal=?1 AND e.source_id=?2",
        params![principal,source_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).optional().map_err(|err| err.to_string())?.ok_or("source_not_authorized")?;
        let grant = granted(&tx, &principal, &key, false)?;
        if available != "owned_inline" {
            return Err("source_unavailable".into());
        }
        let body: serde_json::Value = serde_json::from_str(&body).map_err(|err| err.to_string())?;
        let event: ObservationEvent =
            serde_json::from_value(body["observation"].clone()).map_err(|err| err.to_string())?;
        let result = CapturedObservation {
            source_id: source_id.into(),
            source_key: key,
            scope_id: grant.scope_id,
            generation,
            role: grant.role,
            event_key: event.event_key,
            text: event.text,
            observed_at: event.observed_at,
        };
        tx.commit().map_err(|err| err.to_string())?;
        Ok(result)
    }
}
