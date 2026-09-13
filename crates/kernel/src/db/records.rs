//! Authoritative record/revision/head tables and the legacy import.
//!
//! Records name continuing logical objects; revisions are immutable; heads are
//! a set (composite FK so a head belongs to its record). Legacy `memories` /
//! `decisions` rows are imported as baseline revisions with an explicit
//! provenance gap and stay addressable through `addresses`. Nothing here
//! rewrites legacy rows; the new tables are additive.

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

pub const AUTHORITATIVE_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS brain_meta (
  singleton INTEGER PRIMARY KEY CHECK(singleton=1), brain_id TEXT NOT NULL,
  restore_epoch TEXT NOT NULL, policy_epoch TEXT NOT NULL,
  schema_version TEXT NOT NULL, erasure_floor TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS scopes (
  scope_id TEXT PRIMARY KEY, parent_scope TEXT REFERENCES scopes(scope_id),
  owner_id TEXT NOT NULL, kind TEXT NOT NULL, descriptor TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS commits (
  sequence INTEGER PRIMARY KEY AUTOINCREMENT, commit_id TEXT NOT NULL UNIQUE,
  principal_id TEXT NOT NULL, idempotency_key TEXT,
  canonical_request BLOB, receipt_json TEXT,
  ack_profile TEXT NOT NULL CHECK(ack_profile IN ('process_crash','power_loss_assumed')),
  origin_id TEXT NOT NULL, origin_counter INTEGER NOT NULL CHECK(origin_counter>=0),
  recorded_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE(origin_id,origin_counter)
);
CREATE TABLE IF NOT EXISTS sources (
  source_id TEXT PRIMARY KEY, scope_id TEXT NOT NULL REFERENCES scopes(scope_id),
  origin_id TEXT NOT NULL, media_type TEXT NOT NULL,
  availability TEXT NOT NULL CHECK(availability IN ('owned_inline','owned_external','external_only','erased','unavailable')),
  inline_payload BLOB, provider_locator TEXT, byte_length INTEGER NOT NULL CHECK(byte_length>=0),
  capture_sequence INTEGER NOT NULL REFERENCES commits(sequence),
  CHECK ((availability='owned_inline' AND inline_payload IS NOT NULL) OR availability!='owned_inline'),
  CHECK ((availability IN ('owned_external','external_only') AND provider_locator IS NOT NULL)
    OR availability NOT IN ('owned_external','external_only'))
);
CREATE TABLE IF NOT EXISTS records (
  record_id TEXT PRIMARY KEY, scope_id TEXT NOT NULL REFERENCES scopes(scope_id),
  kind TEXT NOT NULL, retention TEXT NOT NULL CHECK(retention IN ('durable','operational','audit','ephemeral')),
  created_sequence INTEGER NOT NULL REFERENCES commits(sequence)
);
CREATE TABLE IF NOT EXISTS revisions (
  revision_id TEXT PRIMARY KEY, record_id TEXT NOT NULL REFERENCES records(record_id),
  body_json TEXT NOT NULL CHECK(json_valid(body_json)),
  epistemic_status TEXT NOT NULL CHECK(epistemic_status IN ('asserted','supported','checker_verified','contested','hypothesis','retracted','unknown')),
  valid_from INTEGER, valid_until INTEGER, recorded_sequence INTEGER NOT NULL REFERENCES commits(sequence),
  representation_version TEXT NOT NULL, UNIQUE(record_id,revision_id),
  CHECK(valid_from IS NULL OR valid_until IS NULL OR valid_from<valid_until)
);
CREATE TABLE IF NOT EXISTS revision_parents (
  record_id TEXT NOT NULL, revision_id TEXT NOT NULL, parent_revision TEXT NOT NULL,
  PRIMARY KEY(record_id,revision_id,parent_revision), CHECK(revision_id!=parent_revision),
  FOREIGN KEY(record_id,revision_id) REFERENCES revisions(record_id,revision_id),
  FOREIGN KEY(record_id,parent_revision) REFERENCES revisions(record_id,revision_id)
);
CREATE TABLE IF NOT EXISTS record_heads (
  record_id TEXT NOT NULL, revision_id TEXT NOT NULL,
  PRIMARY KEY(record_id,revision_id),
  FOREIGN KEY(record_id,revision_id) REFERENCES revisions(record_id,revision_id)
);
CREATE TABLE IF NOT EXISTS revision_sources (
  revision_id TEXT NOT NULL REFERENCES revisions(revision_id), source_id TEXT NOT NULL REFERENCES sources(source_id),
  role TEXT NOT NULL, extent_json TEXT NOT NULL CHECK(json_valid(extent_json)),
  PRIMARY KEY(revision_id,source_id,role)
);
CREATE TABLE IF NOT EXISTS relations (
  relation_id TEXT PRIMARY KEY, from_revision TEXT NOT NULL REFERENCES revisions(revision_id),
  to_revision TEXT NOT NULL REFERENCES revisions(revision_id), scope_id TEXT NOT NULL REFERENCES scopes(scope_id),
  relation_kind TEXT NOT NULL, rule_version TEXT, recorded_sequence INTEGER NOT NULL REFERENCES commits(sequence)
);
CREATE TABLE IF NOT EXISTS digests (
  source_id TEXT NOT NULL REFERENCES sources(source_id), algorithm TEXT NOT NULL,
  domain TEXT NOT NULL, canonicalization TEXT NOT NULL DEFAULT '', digest BLOB NOT NULL,
  PRIMARY KEY(source_id,algorithm,domain,canonicalization)
);
CREATE TABLE IF NOT EXISTS addresses (
  scheme TEXT NOT NULL, namespace TEXT NOT NULL, address TEXT NOT NULL,
  record_id TEXT NOT NULL REFERENCES records(record_id), PRIMARY KEY(scheme,namespace,address)
);
CREATE TABLE IF NOT EXISTS threads (
  thread_id TEXT PRIMARY KEY, scope_id TEXT NOT NULL REFERENCES scopes(scope_id),
  title TEXT NOT NULL, created_sequence INTEGER NOT NULL REFERENCES commits(sequence)
);
CREATE TABLE IF NOT EXISTS thread_members (
  thread_id TEXT NOT NULL REFERENCES threads(thread_id), record_id TEXT NOT NULL REFERENCES records(record_id),
  role TEXT NOT NULL, PRIMARY KEY(thread_id,record_id,role)
);
CREATE TABLE IF NOT EXISTS obligations (
  record_id TEXT PRIMARY KEY REFERENCES records(record_id), thread_id TEXT NOT NULL REFERENCES threads(thread_id),
  state TEXT NOT NULL CHECK(state IN ('proposed','ready','in_progress','blocked','observed_complete','verified_complete','reopened','cancelled')),
  predicate_json TEXT NOT NULL CHECK(json_valid(predicate_json)),
  verification_revision TEXT REFERENCES revisions(revision_id)
);
CREATE TABLE IF NOT EXISTS change_items (
  sequence INTEGER NOT NULL REFERENCES commits(sequence), ordinal INTEGER NOT NULL CHECK(ordinal>=0),
  scope_id TEXT NOT NULL REFERENCES scopes(scope_id), record_id TEXT REFERENCES records(record_id),
  change_kind TEXT NOT NULL, PRIMARY KEY(sequence,ordinal)
);
CREATE TABLE IF NOT EXISTS projection_state (
  projection_name TEXT NOT NULL, scope_id TEXT NOT NULL REFERENCES scopes(scope_id),
  version TEXT NOT NULL, through_sequence INTEGER NOT NULL CHECK(through_sequence>=0),
  state TEXT NOT NULL CHECK(state IN ('ready','pending','failed')),
  PRIMARY KEY(projection_name,scope_id)
);
CREATE TABLE IF NOT EXISTS outbox (
  job_id TEXT PRIMARY KEY, commit_sequence INTEGER NOT NULL REFERENCES commits(sequence),
  job_kind TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('pending','claimed','done','failed')),
  generation INTEGER NOT NULL CHECK(generation>=0), payload_json TEXT NOT NULL CHECK(json_valid(payload_json)),
  claimed_by TEXT, lease_until TEXT, attempts INTEGER NOT NULL DEFAULT 0, last_error TEXT,
  UNIQUE(commit_sequence,job_kind)
);
CREATE TABLE IF NOT EXISTS guard_epochs (
  scope_id TEXT NOT NULL REFERENCES scopes(scope_id), guard_key TEXT NOT NULL,
  generation INTEGER NOT NULL CHECK(generation>=0), PRIMARY KEY(scope_id,guard_key)
);
CREATE TABLE IF NOT EXISTS compiled_reads (
  compiled_id TEXT PRIMARY KEY, recipe_id TEXT NOT NULL, operator_versions TEXT NOT NULL,
  scope_id TEXT NOT NULL REFERENCES scopes(scope_id), principal_id TEXT NOT NULL,
  brain_epoch TEXT NOT NULL, policy_epoch TEXT NOT NULL,
  parameters_json TEXT NOT NULL CHECK(json_valid(parameters_json)),
  environment_ref TEXT NOT NULL, through_sequence INTEGER NOT NULL CHECK(through_sequence>=0),
  result_json TEXT NOT NULL CHECK(json_valid(result_json))
);
CREATE TABLE IF NOT EXISTS compiled_guards (
  compiled_id TEXT NOT NULL REFERENCES compiled_reads(compiled_id) ON DELETE CASCADE,
  scope_id TEXT NOT NULL REFERENCES scopes(scope_id), guard_key TEXT NOT NULL,
  expected_generation INTEGER NOT NULL CHECK(expected_generation>=0),
  kind TEXT NOT NULL CHECK(kind IN ('positive','negative','policy','environment')),
  PRIMARY KEY(compiled_id,scope_id,guard_key)
);
CREATE TABLE IF NOT EXISTS view_receipts (
  receipt_id TEXT PRIMARY KEY, principal_id TEXT NOT NULL, brain_epoch TEXT NOT NULL,
  through_sequence INTEGER NOT NULL CHECK(through_sequence>=0),
  receipt_json TEXT NOT NULL CHECK(json_valid(receipt_json))
);
CREATE TABLE IF NOT EXISTS view_aliases (
  receipt_id TEXT NOT NULL REFERENCES view_receipts(receipt_id), alias TEXT NOT NULL,
  record_id TEXT NOT NULL, revision_id TEXT NOT NULL, representation_version TEXT NOT NULL,
  PRIMARY KEY(receipt_id,alias),
  FOREIGN KEY(record_id,revision_id) REFERENCES revisions(record_id,revision_id)
);
CREATE TABLE IF NOT EXISTS erasures (
  erasure_id TEXT PRIMARY KEY, scope_id TEXT NOT NULL REFERENCES scopes(scope_id),
  target_descriptor TEXT NOT NULL, sequence INTEGER NOT NULL REFERENCES commits(sequence),
  propagation_state TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS revisions_record_seq ON revisions(record_id,recorded_sequence);
CREATE INDEX IF NOT EXISTS records_scope_kind ON records(scope_id,kind);
CREATE INDEX IF NOT EXISTS changes_scope_seq ON change_items(scope_id,sequence);
CREATE INDEX IF NOT EXISTS relations_from ON relations(from_revision,relation_kind);
CREATE INDEX IF NOT EXISTS relations_to ON relations(to_revision,relation_kind);
CREATE INDEX IF NOT EXISTS outbox_state ON outbox(state,commit_sequence);
CREATE INDEX IF NOT EXISTS sources_capture ON sources(scope_id,capture_sequence);
CREATE INDEX IF NOT EXISTS guards_reverse ON compiled_guards(scope_id,guard_key);
"#;

pub const DEFAULT_SCOPE: &str = "default";
pub const SCHEMA_LABEL: &str = "authoritative-records";

/// Create tables, the brain singleton and the default scope. Idempotent.
pub fn ensure_authoritative_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(AUTHORITATIVE_DDL)?;
    conn.execute_batch(crate::store_spi::sqlite::IDEMPOTENCY_DDL)?;
    conn.execute_batch(super::views::AGENT_VIEWS_DDL)?;
    let exists: bool = conn.query_row("SELECT COUNT(*) FROM brain_meta", [], |r| {
        r.get::<_, i64>(0)
    })? > 0;
    if !exists {
        let brain_id = format!("brain-{}", uuid_like(conn));
        conn.execute(
            "INSERT INTO brain_meta (singleton, brain_id, restore_epoch, policy_epoch, schema_version, erasure_floor) VALUES (1, ?1, '0', '0', ?2, '0')",
            params![brain_id, SCHEMA_LABEL],
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO scopes (scope_id, parent_scope, owner_id, kind, descriptor) VALUES (?1, NULL, 'local', 'brain', '{}')",
        params![DEFAULT_SCOPE],
    )?;
    Ok(())
}

fn table_has_column(conn: &Connection, table: &str, column: &str) -> bool {
    conn.prepare(&format!("PRAGMA table_info({table})"))
        .and_then(|mut stmt| {
            stmt.query_map([], |r| r.get::<_, String>(1))
                .map(|rows| rows.filter_map(Result::ok).any(|c| c == column))
        })
        .unwrap_or(false)
}

fn uuid_like(conn: &Connection) -> String {
    conn.query_row("SELECT lower(hex(randomblob(8)))", [], |r| {
        r.get::<_, String>(0)
    })
    .unwrap_or_else(|_| "local".into())
}

pub fn brain_epochs(conn: &Connection) -> (String, String, String) {
    conn.query_row(
        "SELECT brain_id, restore_epoch, policy_epoch FROM brain_meta WHERE singleton = 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .unwrap_or_else(|_| ("brain-unknown".into(), "0".into(), "0".into()))
}

/// Append one commit row and return its sequence. The origin is this brain
/// unless a replicated origin is supplied.
pub fn append_commit(
    conn: &Connection,
    principal: &str,
    idempotency_key: Option<&str>,
    ack_profile: &str,
) -> rusqlite::Result<i64> {
    let (brain_id, _, _) = brain_epochs(conn);
    let counter: i64 = conn.query_row(
        "SELECT COALESCE(MAX(origin_counter), -1) + 1 FROM commits WHERE origin_id = ?1",
        params![brain_id],
        |r| r.get(0),
    )?;
    let commit_id = format!("{brain_id}:{counter}");
    conn.execute(
        "INSERT INTO commits (commit_id, principal_id, idempotency_key, ack_profile, origin_id, origin_counter) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![commit_id, principal, idempotency_key, ack_profile, brain_id, counter],
    )?;
    Ok(conn.last_insert_rowid())
}

pub struct NewRevision<'a> {
    pub record_id: &'a str,
    pub kind: &'a str,
    pub retention: &'a str,
    pub body: Value,
    pub epistemic_status: &'a str,
    /// Parent revisions this revision descends from. Empty = baseline.
    pub parents: &'a [String],
    /// When true the parents are removed from the head set (a normal
    /// successor). When false the new revision is a concurrent head.
    pub replace_parents: bool,
    pub representation_version: &'a str,
}

/// Insert a record (if new) and an immutable revision, maintain the head set,
/// and append a change item. Never deletes a revision.
pub fn append_revision(
    conn: &Connection,
    sequence: i64,
    rev: NewRevision<'_>,
) -> rusqlite::Result<String> {
    conn.execute(
        "INSERT OR IGNORE INTO records (record_id, scope_id, kind, retention, created_sequence) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![rev.record_id, DEFAULT_SCOPE, rev.kind, rev.retention, sequence],
    )?;
    let ordinal: i64 = conn.query_row(
        "SELECT COUNT(*) FROM revisions WHERE record_id = ?1",
        params![rev.record_id],
        |r| r.get(0),
    )?;
    let revision_id = format!("{}@{}", rev.record_id, ordinal + 1);
    conn.execute(
        "INSERT INTO revisions (revision_id, record_id, body_json, epistemic_status, recorded_sequence, representation_version) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![revision_id, rev.record_id, rev.body.to_string(), rev.epistemic_status, sequence, rev.representation_version],
    )?;
    for parent in rev.parents {
        conn.execute(
            "INSERT INTO revision_parents (record_id, revision_id, parent_revision) VALUES (?1, ?2, ?3)",
            params![rev.record_id, revision_id, parent],
        )?;
        if rev.replace_parents {
            conn.execute(
                "DELETE FROM record_heads WHERE record_id = ?1 AND revision_id = ?2",
                params![rev.record_id, parent],
            )?;
        }
    }
    conn.execute(
        "INSERT INTO record_heads (record_id, revision_id) VALUES (?1, ?2)",
        params![rev.record_id, revision_id],
    )?;
    // Negative-dependency guard: any cached read that searched this
    // record kind (or the scope at large) is invalidated by this insert.
    super::compiled::bump_guard(conn, DEFAULT_SCOPE, rev.kind)?;
    let change_ordinal: i64 = conn.query_row(
        "SELECT COUNT(*) FROM change_items WHERE sequence = ?1",
        params![sequence],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO change_items (sequence, ordinal, scope_id, record_id, change_kind) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![sequence, change_ordinal, DEFAULT_SCOPE, rev.record_id, if rev.parents.is_empty() { "created" } else { "revised" }],
    )?;
    Ok(revision_id)
}

pub fn heads(conn: &Connection, record_id: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT revision_id FROM record_heads WHERE record_id = ?1 ORDER BY revision_id",
    )?;
    let rows = stmt.query_map(params![record_id], |r| r.get::<_, String>(0))?;
    rows.collect()
}

pub fn revision_body(conn: &Connection, revision_id: &str) -> rusqlite::Result<Option<Value>> {
    let raw = conn
        .query_row(
            "SELECT body_json FROM revisions WHERE revision_id = ?1",
            params![revision_id],
            |r| r.get::<_, String>(0),
        )
        .optional()?;
    match raw {
        None => Ok(None),
        Some(s) => serde_json::from_str(&s).map(Some).map_err(|err| {
            rusqlite::Error::InvalidParameterName(format!(
                "revision `{revision_id}` body is not valid JSON: {err}"
            ))
        }),
    }
}

/// A resolution names the heads it considered, its authority and rationale;
/// it becomes the single head. Heads not listed stay unresolved.
pub fn resolve_heads(
    conn: &Connection,
    sequence: i64,
    record_id: &str,
    considered: &[String],
    authority: &str,
    rationale: &str,
    body: Value,
) -> rusqlite::Result<String> {
    let current = heads(conn, record_id)?;
    for head in considered {
        if !current.contains(head) {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "{head} is not a current head of {record_id}"
            )));
        }
    }
    let (kind, retention): (String, String) = conn.query_row(
        "SELECT kind, retention FROM records WHERE record_id = ?1",
        params![record_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let resolution_body = json!({"resolution": {"considered": considered, "authority": authority, "rationale": rationale}, "body": body});
    append_revision(
        conn,
        sequence,
        NewRevision {
            record_id,
            kind: &kind,
            retention: &retention,
            body: resolution_body,
            epistemic_status: "supported",
            parents: considered,
            replace_parents: true,
            representation_version: "resolution/1",
        },
    )
}

/// Legacy address of a `memories`/`decisions` row → record id, if imported.
pub fn record_for_legacy(
    conn: &Connection,
    table_kind: &str,
    id: i64,
) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT record_id FROM addresses WHERE scheme = 'legacy' AND namespace = ?1 AND address = ?2", params![table_kind, id.to_string()], |r| {
        r.get(0)
    })
    .optional()
}

/// Import every legacy row that has no record yet as a baseline revision.
/// Timestamps become observed metadata; provenance gaps are labeled, never
/// manufactured. Returns the number of records created.
pub fn import_legacy(conn: &Connection) -> rusqlite::Result<usize> {
    ensure_authoritative_schema(conn)?;
    let mut created = 0usize;
    let mut sequence: Option<i64> = None;
    for (table, kind, text_col) in [
        ("decisions", "decision", "decision"),
        ("memories", "memory", "text"),
    ] {
        // Column-tolerant: this migration may run on a database whose later
        // legacy columns (retention_class, trust_score, version_id) do not
        // exist yet; missing columns are reported as absent, not invented.
        let has = |col: &str| table_has_column(conn, table, col);
        let retention_expr = if has("retention_class") {
            "COALESCE(t.retention_class, 'operational')"
        } else {
            "'operational'"
        };
        let trust_expr = if has("trust_score") {
            "COALESCE(t.trust_score, 0.8)"
        } else {
            "0.8"
        };
        let version_expr = if has("version_id") {
            "t.version_id"
        } else {
            "NULL"
        };
        let sql = format!(
            "SELECT t.id, t.{text_col}, t.status, t.source_agent, t.created_at, {retention_expr}, {trust_expr}, {version_expr} \
             FROM {table} t WHERE NOT EXISTS (SELECT 1 FROM addresses a WHERE a.scheme = 'legacy' AND a.namespace = ?1 AND a.address = CAST(t.id AS TEXT)) ORDER BY t.id"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows: Vec<(
            i64,
            String,
            String,
            String,
            String,
            String,
            f64,
            Option<i64>,
        )> = stmt
            .query_map(params![kind], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            })?
            .collect::<Result<_, _>>()?;
        for (id, text, status, agent, created_at, retention, trust, version_id) in rows {
            let seq = match sequence {
                Some(s) => s,
                None => {
                    let s = append_commit(conn, "system:legacy-import", None, "process_crash")?;
                    sequence = Some(s);
                    s
                }
            };
            let record_id = format!("{kind}:{id}");
            let retention =
                if ["durable", "operational", "audit", "ephemeral"].contains(&retention.as_str()) {
                    retention
                } else {
                    "operational".to_string()
                };
            let body = json!({
                "text": text,
                "legacy": {"table": table, "id": id, "status": status, "agent": agent, "observed_created_at": created_at, "trust_score": trust, "trust_basis": "legacy_model_weight", "version_id": version_id},
                "provenance_gap": "imported from a pre-revision schema: no source lineage, validity or verification was recorded"
            });
            append_revision(
                conn,
                seq,
                NewRevision {
                    record_id: &record_id,
                    kind,
                    retention: &retention,
                    body,
                    epistemic_status: "asserted",
                    parents: &[],
                    replace_parents: false,
                    representation_version: "legacy-import/1",
                },
            )?;
            conn.execute(
                "INSERT OR IGNORE INTO addresses (scheme, namespace, address, record_id) VALUES ('legacy', ?1, ?2, ?3)",
                params![kind, id.to_string(), record_id],
            )?;
            created += 1;
        }
    }
    Ok(created)
}
