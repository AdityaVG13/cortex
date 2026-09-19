mod lookup;
mod mentions;

pub use lookup::{entity_arm_candidates, ingest_for_target, resolve_mention, resolve_query};
pub use mentions::{Mention, extract_mentions, lexical_cluster_mates};
pub(crate) use mentions::{is_ticket, looks_like_http_url};
use rusqlite::Connection;

pub fn migrate_entity_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS entities (id INTEGER PRIMARY KEY AUTOINCREMENT, canonical_name TEXT NOT NULL, qualifier TEXT NOT NULL, kind TEXT NOT NULL DEFAULT '', owner_id INTEGER, created_at TEXT NOT NULL DEFAULT (datetime('now'))); CREATE INDEX IF NOT EXISTS idx_entities_qualifier_kind ON entities(qualifier, kind); CREATE TABLE IF NOT EXISTS entity_aliases (id INTEGER PRIMARY KEY AUTOINCREMENT, entity_id INTEGER NOT NULL, alias TEXT NOT NULL, qualifier TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'confirmed' CHECK (status IN ('confirmed', 'candidate')), source_trace_id INTEGER, created_at TEXT NOT NULL DEFAULT (datetime('now'))); CREATE UNIQUE INDEX IF NOT EXISTS idx_entity_aliases_alias ON entity_aliases(alias, entity_id); CREATE INDEX IF NOT EXISTS idx_entity_aliases_entity ON entity_aliases(entity_id); CREATE TABLE IF NOT EXISTS entity_mentions (id INTEGER PRIMARY KEY AUTOINCREMENT, entity_id INTEGER NOT NULL, target_type TEXT NOT NULL, target_id INTEGER NOT NULL, created_at TEXT NOT NULL DEFAULT (datetime('now'))); CREATE UNIQUE INDEX IF NOT EXISTS idx_entity_mentions_unique ON entity_mentions(entity_id, target_type, target_id); CREATE INDEX IF NOT EXISTS idx_entity_mentions_target ON entity_mentions(target_type, target_id);")
}
