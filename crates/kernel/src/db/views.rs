//! Trusted-local inspection views over the authoritative tables. Ergonomic
//! for a human or a tool that already holds the database file; NOT an access
//! control boundary and NOT a Lens evaluator (no valid-at / as-known /
//! closure semantics). Untrusted clients get a principal-filtered export or
//! the runtime, never this file.

use rusqlite::Connection;

pub const AGENT_VIEWS_DDL: &str = "CREATE VIEW IF NOT EXISTS agent_memory_cards AS SELECT r.record_id, r.scope_id, r.kind, r.retention, v.revision_id, v.epistemic_status, v.valid_from, v.valid_until, v.body_json, v.recorded_sequence FROM records r JOIN record_heads h ON r.record_id = h.record_id JOIN revisions v ON h.record_id = v.record_id AND h.revision_id = v.revision_id; CREATE VIEW IF NOT EXISTS agent_constraints AS SELECT * FROM agent_memory_cards WHERE kind IN ('constraint','policy','rule','convention','contract','preference'); CREATE VIEW IF NOT EXISTS agent_competing_heads AS SELECT record_id, COUNT(*) AS head_count FROM record_heads GROUP BY record_id HAVING COUNT(*) > 1; CREATE VIEW IF NOT EXISTS agent_open_work AS SELECT o.record_id, o.thread_id, o.state, o.predicate_json FROM obligations o WHERE state NOT IN ('verified_complete','cancelled'); CREATE VIEW IF NOT EXISTS agent_attempts AS SELECT * FROM agent_memory_cards WHERE kind IN ('attempt','failure','outcome'); CREATE VIEW IF NOT EXISTS agent_recent_changes AS SELECT c.sequence, c.recorded_at, c.principal_id, i.scope_id, i.record_id, i.change_kind FROM commits c JOIN change_items i ON c.sequence = i.sequence; CREATE VIEW IF NOT EXISTS agent_threads AS SELECT t.thread_id, t.title, t.scope_id, m.record_id, m.role FROM threads t LEFT JOIN thread_members m ON m.thread_id = t.thread_id; CREATE VIEW IF NOT EXISTS agent_sources AS SELECT source_id, scope_id, origin_id, media_type, availability, byte_length, capture_sequence FROM sources; CREATE VIEW IF NOT EXISTS agent_receipts AS SELECT commit_id, sequence, principal_id, ack_profile, receipt_json FROM commits; CREATE VIEW IF NOT EXISTS agent_projection_health AS SELECT * FROM projection_state; CREATE VIEW IF NOT EXISTS agent_help AS SELECT 'write' AS topic, 'Use the runtime (cortex op commit / MCP cortex_commit); direct SQL bypasses redaction, provenance, heads and receipts.' AS guidance UNION ALL SELECT 'current', 'Heads are not a valid-at / as-known / conflict-resolution evaluation; use cortex_query.' UNION ALL SELECT 'security', 'Possession of this file grants access to its data; views do not enforce row-level ACLs.';";

pub const AGENT_VIEW_NAMES: [&str; 11] = [
    "agent_memory_cards",
    "agent_constraints",
    "agent_competing_heads",
    "agent_open_work",
    "agent_attempts",
    "agent_recent_changes",
    "agent_threads",
    "agent_sources",
    "agent_receipts",
    "agent_projection_health",
    "agent_help",
];

pub fn ensure_agent_views(conn: &Connection) -> rusqlite::Result<()> {
    super::records::ensure_authoritative_schema(conn)?;
    conn.execute_batch(AGENT_VIEWS_DDL)
}
