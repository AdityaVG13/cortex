//! Authoritative record/revision/head tables and the legacy import.
//!
//! Records name continuing logical objects; revisions are immutable; heads are
//! a set (composite FK so a head belongs to its record). Legacy `memories` /
//! `decisions` rows are imported as baseline revisions with an explicit
//! provenance gap and stay addressable through `addresses`. Nothing here
//! rewrites legacy rows; the new tables are additive.

use rusqlite::{Connection, params};

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

mod import;
pub use import::import_legacy;

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
        conn.execute("INSERT INTO brain_meta (singleton, brain_id, restore_epoch, policy_epoch, schema_version, erasure_floor) VALUES (1, ?1, '0', '0', ?2, '0')", params![brain_id, SCHEMA_LABEL])?;
    }
    conn.execute("INSERT OR IGNORE INTO scopes (scope_id, parent_scope, owner_id, kind, descriptor) VALUES (?1, NULL, 'local', 'brain', '{}')", params![DEFAULT_SCOPE])?;
    Ok(())
}

fn uuid_like(conn: &Connection) -> String {
    conn.query_row("SELECT lower(hex(randomblob(8)))", [], |r| {
        r.get::<_, String>(0)
    })
    .unwrap_or_else(|_| "local".into())
}

pub fn try_brain_epochs(conn: &Connection) -> rusqlite::Result<(String, String, String)> {
    conn.query_row(
        "SELECT brain_id, restore_epoch, policy_epoch FROM brain_meta WHERE singleton = 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
}

pub const POLICY_EPOCH_SELECT: &str = "SELECT policy_epoch FROM brain_meta WHERE singleton=1";

/// Bootstrap/display helper. Missing `brain_meta` (not yet inserted, or
/// schema not yet applied) is the seed epoch `"0"`. A locked/corrupt read
/// is not a live epoch: `"0"` would pass never-restored equality checks, so
/// the sentinel cannot match a minted receipt, compiled read, or origin id.
pub fn brain_epochs(conn: &Connection) -> (String, String, String) {
    match try_brain_epochs(conn) {
        Ok(v) => v,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            ("brain-unknown".into(), "0".into(), "0".into())
        }
        Err(err) if err.to_string().contains("no such table") => {
            ("brain-unknown".into(), "0".into(), "0".into())
        }
        Err(_) => (
            "brain-unreadable".into(),
            "unreadable".into(),
            "unreadable".into(),
        ),
    }
}

mod mutate;
pub use mutate::*;
