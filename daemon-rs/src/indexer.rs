//! Knowledge indexer: reads filesystem sources and upserts into memories table.
//! Full implementation in Task 4 of the port plan.

use rusqlite::Connection;
use std::path::Path;

/// Run all indexers. Returns total entries indexed.
pub fn index_all(_conn: &Connection, _home: &Path) -> usize {
    // TODO: Task 4 -- port brain.js indexAll()
    0
}

/// Score decay pass: apply 0.95^days to all entries.
pub fn decay_pass(conn: &Connection) -> usize {
    let result = conn.execute(
        "UPDATE memories SET score = MAX(0.1, score * POWER(0.95,
            CAST((julianday('now') - julianday(COALESCE(updated_at, created_at))) AS REAL)))
         WHERE status = 'active' AND score > 0.1
           AND (julianday('now') - julianday(COALESCE(updated_at, created_at))) > 1",
        [],
    );
    result.unwrap_or(0)
}
