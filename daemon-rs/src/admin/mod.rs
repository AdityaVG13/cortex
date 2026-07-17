use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
pub const ROLLED_BACK_STATUS: &str = "rolled_back";
#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub struct RollbackStats {
    pub session_id: String,
    pub agent: String,
    pub session_started_at: String,
    pub memories_affected: i64,
    pub decisions_affected: i64,
    pub applied: bool,
    pub already_rolled_back: bool,
}
pub fn rollback_session_by_id(conn: &Connection, session_id: &str, apply: bool) -> rusqlite::Result<RollbackStats> {
    let mut stats = RollbackStats { session_id: session_id.to_string(), ..Default::default() };
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
    stats.already_rolled_back = active_memories == 0 && active_decisions == 0 && (prior_rolled_memories > 0 || prior_rolled_decisions > 0);
    if !apply {
        return Ok(stats);
    }
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
    stats.memories_affected = updated_memories;
    stats.decisions_affected = updated_decisions;
    stats.applied = true;
    Ok(stats)
}
#[cfg(test)]
mod tests;
