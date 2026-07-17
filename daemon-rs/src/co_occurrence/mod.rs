// SPDX-License-Identifier: MIT
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
pub fn record(conn: &Connection, sources: &[String]) -> Result<(), String> {
    if sources.len() < 2 {
        return Ok(());
    }
    let unique = sources
        .iter()
        .filter(|s| !s.trim().is_empty())
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .take(10)
        .collect::<Vec<_>>();
    if unique.len() < 2 {
        return Ok(());
    }
    for i in 0..unique.len() {
        for j in (i + 1)..unique.len() {
            let (a, b) = if unique[i] <= unique[j] {
                (unique[i].clone(), unique[j].clone())
            } else {
                (unique[j].clone(), unique[i].clone())
            };
            conn.execute(
                "INSERT INTO co_occurrence (source_a, source_b, count, last_seen)
                 VALUES (?1, ?2, 1, datetime('now'))
                 ON CONFLICT(source_a, source_b) DO UPDATE SET
                   count = count + 1,
                   last_seen = datetime('now')",
                params![a, b],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
pub fn predict(conn: &Connection, recalled_sources: &[String], limit: usize) -> Result<Vec<Value>, String> {
    if recalled_sources.is_empty() {
        return Ok(vec![]);
    }
    let already_have = recalled_sources.iter().filter(|s| !s.trim().is_empty()).cloned().collect::<HashSet<_>>();
    let mut candidates: HashMap<String, i64> = HashMap::new();
    for source in &already_have {
        let mut stmt = conn
            .prepare(
                "SELECT
                   CASE WHEN source_a = ?1 THEN source_b ELSE source_a END AS partner,
                   count
                 FROM co_occurrence
                 WHERE source_a = ?1 OR source_b = ?1
                 ORDER BY count DESC
                 LIMIT 10",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![source], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
            .map_err(|e| e.to_string())?;
        for row in rows.flatten() {
            let (partner, count) = row;
            if already_have.contains(&partner) {
                continue;
            }
            let existing = candidates.get(&partner).copied().unwrap_or(0);
            candidates.insert(partner, existing + count);
        }
    }
    let mut ranked = candidates.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));
    ranked.truncate(limit);
    Ok(ranked.into_iter().map(|(source, score)| json!({ "source": source, "coScore": score })).collect())
}
pub fn reset(conn: &Connection) -> Result<(), String> {
    conn.execute("DELETE FROM co_occurrence", []).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests;
