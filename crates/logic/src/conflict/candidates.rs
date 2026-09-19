use super::RecentDecisionCandidate;
use crate::protocol::ACTIVE_TEMPORAL_SQL;
use rusqlite::Connection;

fn decision_gates() -> String {
    format!(
        "{ACTIVE_TEMPORAL_SQL} AND lower(trim(COALESCE(type, 'decision'))) NOT IN ('case','counterexample','attempt','failure','outcome','procedure','playbook','runbook','exception','constraint','policy','rule','convention','contract','obligation','checkpoint','preference','lesson','verified_result')"
    )
}

fn owner_prefix(owner: bool) -> &'static str {
    if owner { "owner_id = ?1 AND " } else { "" }
}

fn inner_decision_select(owner: bool) -> String {
    let gates = decision_gates();
    format!(
        "SELECT id, decision, source_agent, COALESCE(trust_score, confidence, 0.8) AS trust_score FROM decisions WHERE {}{gates}",
        owner_prefix(owner)
    )
}

fn recent_candidates_sql(owner: bool) -> String {
    let inner = inner_decision_select(owner);
    format!(
        "SELECT id, decision, source_agent, trust_score, MAX(in_conflict_window) AS in_conflict_window FROM ( SELECT id, decision, source_agent, trust_score, 1 AS in_conflict_window FROM ( {inner} ORDER BY id DESC LIMIT 50 ) UNION ALL SELECT id, decision, source_agent, trust_score, 0 AS in_conflict_window FROM ( {inner} ORDER BY julianday(created_at) DESC LIMIT 50 ) ) GROUP BY id, decision, source_agent, trust_score ORDER BY in_conflict_window DESC, id DESC"
    )
}

pub(super) fn detect_sql(owner: bool) -> String {
    let gates = decision_gates();
    format!(
        "SELECT id, decision, source_agent, COALESCE(trust_score, confidence, 0.8) FROM decisions WHERE {}{gates} ORDER BY id DESC LIMIT 50",
        owner_prefix(owner)
    )
}

pub fn fetch_recent_decision_candidates(
    conn: &Connection,
    owner_id: Option<i64>,
) -> Result<Vec<RecentDecisionCandidate>, String> {
    let sql = recent_candidates_sql(owner_id.is_some());
    let mut stmt = conn
        .prepare_cached(&sql)
        .map_err(|error| format!("Failed to prepare recent decision query: {error}"))?;
    let map_candidate = |row: &rusqlite::Row<'_>| {
        let in_conflict_window: i64 = row.get(4)?;
        Ok(RecentDecisionCandidate {
            id: row.get(0)?,
            decision: row.get(1)?,
            source_agent: row.get(2)?,
            trust_score: row.get(3)?,
            in_conflict_window: in_conflict_window != 0,
        })
    };
    super::query_with_optional_i64(&mut stmt, owner_id, map_candidate)
        .map_err(|error| format!("Failed to query recent decisions: {error}"))
}
