// SPDX-License-Identifier: MIT
//! Administrative operations that do not belong on the hot-path MCP/HTTP
//! surface. Kept deliberately thin: each function takes a live `&Connection`
//! and is synchronous. Callers handle process-level concerns (CLI output,
//! JSON formatting, SSE emission).

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

/// Marker written to `status` for memories and decisions that were part of a
/// rolled-back session. Existing recall paths already filter by
/// `status = 'active'`, so flipping this value transparently hides the rows
/// from recall without touching the hot path.
pub const ROLLED_BACK_STATUS: &str = "rolled_back";

/// Counts returned by a dry-run or applied rollback so the CLI + SSE event
/// payload share a single shape.
#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub struct RollbackStats {
    /// Session ID the rollback targeted (echoes the CLI input).
    pub session_id: String,
    /// Agent identifier (e.g. `claude-code`) resolved from the `sessions` row.
    /// Empty when no matching session row exists.
    pub agent: String,
    /// ISO timestamp `sessions.started_at`. Empty when session not found.
    pub session_started_at: String,
    /// How many `memories` rows were (or would be) flipped to `rolled_back`.
    pub memories_affected: i64,
    /// How many `decisions` rows were (or would be) flipped to `rolled_back`.
    pub decisions_affected: i64,
    /// True when `--apply` was passed; false for dry-run.
    pub applied: bool,
    /// True when the same rollback was previously applied, in which case
    /// the current call is a no-op. Helps callers reason about idempotency.
    pub already_rolled_back: bool,
}

/// Roll back a session by id. Resolves `agent` + `started_at` from the
/// `sessions` table, then soft-deletes every memory and decision written by
/// that agent since session start.
///
/// **Dry-run by default.** Pass `apply=true` to actually write `status = 'rolled_back'`.
///
/// **Idempotent.** Re-running after a successful rollback returns counts
/// equal to zero and sets `already_rolled_back=true` when the session's
/// prior memories are all already flipped.
///
/// Errors on DB failure. Returns a stats struct even when the session id
/// is unknown (counts all zero, `agent`/`session_started_at` empty) — CLI
/// surface decides whether that is an error exit.
pub fn rollback_session_by_id(
    conn: &Connection,
    session_id: &str,
    apply: bool,
) -> rusqlite::Result<RollbackStats> {
    let mut stats = RollbackStats {
        session_id: session_id.to_string(),
        ..Default::default()
    };

    // Look up the active session row. The `sessions` table is keyed by agent;
    // session_id is a column. A session id that rotates across agents would
    // match multiple rows — we take the most recent by `started_at` to be
    // deterministic.
    let session_row: Option<(String, String)> = conn
        .query_row(
            "SELECT agent, started_at FROM sessions
             WHERE session_id = ?1
             ORDER BY started_at DESC
             LIMIT 1",
            params![session_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?;

    let (agent, started_at) = match session_row {
        Some(row) => row,
        None => return Ok(stats),
    };
    stats.agent = agent.clone();
    stats.session_started_at = started_at.clone();

    // Count candidate rows created by this agent from session start onward.
    // Active ones are the rollback target; already-rolled-back ones tell us
    // whether this is a repeat invocation.
    let active_memories: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories
         WHERE source_agent = ?1
           AND created_at >= ?2
           AND status = 'active'",
        params![agent, started_at],
        |r| r.get(0),
    )?;
    let active_decisions: i64 = conn.query_row(
        "SELECT COUNT(*) FROM decisions
         WHERE source_agent = ?1
           AND created_at >= ?2
           AND status = 'active'",
        params![agent, started_at],
        |r| r.get(0),
    )?;
    let prior_rolled_memories: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories
         WHERE source_agent = ?1
           AND created_at >= ?2
           AND status = ?3",
        params![agent, started_at, ROLLED_BACK_STATUS],
        |r| r.get(0),
    )?;
    let prior_rolled_decisions: i64 = conn.query_row(
        "SELECT COUNT(*) FROM decisions
         WHERE source_agent = ?1
           AND created_at >= ?2
           AND status = ?3",
        params![agent, started_at, ROLLED_BACK_STATUS],
        |r| r.get(0),
    )?;

    stats.memories_affected = active_memories;
    stats.decisions_affected = active_decisions;
    stats.already_rolled_back = active_memories == 0
        && active_decisions == 0
        && (prior_rolled_memories > 0 || prior_rolled_decisions > 0);

    if !apply {
        return Ok(stats);
    }

    // Apply inside a single transaction. Both updates are idempotent under
    // the `status = 'active'` guard, so a rerun after a partial failure is
    // safe.
    let tx = conn.unchecked_transaction()?;
    let updated_memories = tx.execute(
        "UPDATE memories
            SET status = ?3,
                updated_at = datetime('now')
          WHERE source_agent = ?1
            AND created_at >= ?2
            AND status = 'active'",
        params![agent, started_at, ROLLED_BACK_STATUS],
    )? as i64;
    let updated_decisions = tx.execute(
        "UPDATE decisions
            SET status = ?3,
                updated_at = datetime('now')
          WHERE source_agent = ?1
            AND created_at >= ?2
            AND status = 'active'",
        params![agent, started_at, ROLLED_BACK_STATUS],
    )? as i64;
    tx.commit()?;

    // Report the actually-updated counts (should match the pre-count).
    stats.memories_affected = updated_memories;
    stats.decisions_affected = updated_decisions;
    stats.applied = true;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        // Minimal schema mirroring the two tables + session row the function needs.
        conn.execute_batch(
            r#"
            CREATE TABLE memories (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              text TEXT,
              source TEXT,
              source_agent TEXT,
              status TEXT DEFAULT 'active',
              created_at TEXT DEFAULT (datetime('now')),
              updated_at TEXT DEFAULT (datetime('now'))
            );
            CREATE TABLE decisions (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              decision TEXT,
              source_agent TEXT,
              status TEXT DEFAULT 'active',
              created_at TEXT DEFAULT (datetime('now')),
              updated_at TEXT DEFAULT (datetime('now'))
            );
            CREATE TABLE sessions (
              agent TEXT PRIMARY KEY,
              session_id TEXT NOT NULL,
              started_at TEXT NOT NULL,
              last_heartbeat TEXT NOT NULL,
              expires_at TEXT NOT NULL
            );
            "#,
        )
        .unwrap();
        conn
    }

    fn seed_session(conn: &Connection, agent: &str, session_id: &str, started_at: &str) {
        conn.execute(
            "INSERT INTO sessions(agent, session_id, started_at, last_heartbeat, expires_at)
             VALUES (?1, ?2, ?3, ?3, ?3)",
            params![agent, session_id, started_at],
        )
        .unwrap();
    }

    fn seed_memory(conn: &Connection, agent: &str, text: &str, created_at: &str, status: &str) {
        conn.execute(
            "INSERT INTO memories(text, source_agent, status, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![text, agent, status, created_at],
        )
        .unwrap();
    }

    fn seed_decision(conn: &Connection, agent: &str, decision: &str, created_at: &str) {
        conn.execute(
            "INSERT INTO decisions(decision, source_agent, status, created_at)
             VALUES (?1, ?2, 'active', ?3)",
            params![decision, agent, created_at],
        )
        .unwrap();
    }

    #[test]
    fn unknown_session_returns_zero_stats() {
        let conn = setup();
        let stats = rollback_session_by_id(&conn, "nonexistent", false).unwrap();
        assert_eq!(stats.memories_affected, 0);
        assert_eq!(stats.decisions_affected, 0);
        assert_eq!(stats.agent, "");
        assert!(!stats.applied);
        assert!(!stats.already_rolled_back);
    }

    #[test]
    fn dry_run_counts_without_writing() {
        let conn = setup();
        seed_session(&conn, "claude", "sess-1", "2026-04-24T00:00:00Z");
        seed_memory(&conn, "claude", "m1", "2026-04-24T00:05:00Z", "active");
        seed_memory(&conn, "claude", "m2", "2026-04-24T00:10:00Z", "active");
        seed_decision(&conn, "claude", "d1", "2026-04-24T00:06:00Z");

        let stats = rollback_session_by_id(&conn, "sess-1", false).unwrap();
        assert_eq!(stats.agent, "claude");
        assert_eq!(stats.memories_affected, 2);
        assert_eq!(stats.decisions_affected, 1);
        assert!(!stats.applied);

        // Confirm nothing was actually written
        let active: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE status = 'active'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(active, 2);
    }

    #[test]
    fn apply_flips_statuses_and_excludes_older_rows() {
        let conn = setup();
        seed_session(&conn, "claude", "sess-1", "2026-04-24T00:00:00Z");
        // Older memory predating the session — must not be touched.
        seed_memory(&conn, "claude", "old", "2026-04-23T23:59:00Z", "active");
        // Another agent's memory in the same time window — must not be touched.
        seed_memory(&conn, "codex", "other-agent", "2026-04-24T00:05:00Z", "active");
        // In-session memories + one already-rolled-back row to confirm we only
        // touch active rows.
        seed_memory(&conn, "claude", "m1", "2026-04-24T00:05:00Z", "active");
        seed_memory(&conn, "claude", "m2", "2026-04-24T00:10:00Z", "active");
        seed_memory(&conn, "claude", "pre-rolled", "2026-04-24T00:07:00Z", ROLLED_BACK_STATUS);
        seed_decision(&conn, "claude", "d1", "2026-04-24T00:06:00Z");

        let stats = rollback_session_by_id(&conn, "sess-1", true).unwrap();
        assert_eq!(stats.memories_affected, 2, "only the 2 active in-session memories");
        assert_eq!(stats.decisions_affected, 1);
        assert!(stats.applied);

        // Pre-existing rows preserved
        let old_status: String = conn
            .query_row(
                "SELECT status FROM memories WHERE text = 'old'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(old_status, "active");
        let other_status: String = conn
            .query_row(
                "SELECT status FROM memories WHERE text = 'other-agent'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(other_status, "active");

        // Our two in-session memories flipped
        let rolled: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE source_agent = 'claude' AND status = ?1",
                params![ROLLED_BACK_STATUS],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rolled, 3); // pre-rolled + 2 freshly rolled
    }

    #[test]
    fn idempotent_second_apply() {
        let conn = setup();
        seed_session(&conn, "claude", "sess-1", "2026-04-24T00:00:00Z");
        seed_memory(&conn, "claude", "m1", "2026-04-24T00:05:00Z", "active");

        // First apply rolls back
        let first = rollback_session_by_id(&conn, "sess-1", true).unwrap();
        assert_eq!(first.memories_affected, 1);
        assert!(first.applied);

        // Second apply is a no-op — zero rows left to flip.
        let second = rollback_session_by_id(&conn, "sess-1", true).unwrap();
        assert_eq!(second.memories_affected, 0);
        assert_eq!(second.decisions_affected, 0);
        assert!(second.applied);
        assert!(second.already_rolled_back);
    }

    #[test]
    fn multi_row_session_chooses_most_recent() {
        // Two sessions with the same id but different agents (edge case —
        // we take the most-recent started_at). This is odd in practice
        // but guards against cross-agent collisions.
        let conn = setup();
        conn.execute(
            "INSERT INTO sessions(agent, session_id, started_at, last_heartbeat, expires_at)
             VALUES ('old-agent', 'sess-1', '2026-04-24T00:00:00Z',
                     '2026-04-24T00:00:00Z', '2026-04-25T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions(agent, session_id, started_at, last_heartbeat, expires_at)
             VALUES ('new-agent', 'sess-1', '2026-04-24T01:00:00Z',
                     '2026-04-24T01:00:00Z', '2026-04-25T01:00:00Z')",
            [],
        )
        .unwrap();
        seed_memory(&conn, "new-agent", "m1", "2026-04-24T01:30:00Z", "active");
        seed_memory(&conn, "old-agent", "o1", "2026-04-24T00:30:00Z", "active");

        let stats = rollback_session_by_id(&conn, "sess-1", false).unwrap();
        assert_eq!(stats.agent, "new-agent");
        assert_eq!(stats.memories_affected, 1);
    }
}
