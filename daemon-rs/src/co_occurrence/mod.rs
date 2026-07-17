use rusqlite::{params, params_from_iter, Connection};
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
            let (a, b) =
                if unique[i] <= unique[j] { (unique[i].clone(), unique[j].clone()) } else { (unique[j].clone(), unique[i].clone()) };
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
    if recalled_sources.is_empty() || limit == 0 {
        return Ok(vec![]);
    }

    let already_have = recalled_sources.iter().filter(|s| !s.trim().is_empty()).cloned().collect::<HashSet<_>>();
    if already_have.is_empty() {
        return Ok(vec![]);
    }

    let sources = already_have.iter().map(String::as_str).collect::<Vec<_>>();
    let placeholders = std::iter::repeat("?").take(sources.len()).collect::<Vec<_>>().join(",");
    let scan_limit = sources.len().saturating_mul(10).max(limit).min(1000);
    let sql = format!(
        "SELECT
           CASE WHEN source_a IN ({placeholders}) THEN source_b ELSE source_a END AS partner,
           count
         FROM co_occurrence
         WHERE source_a IN ({placeholders}) OR source_b IN ({placeholders})
         ORDER BY count DESC
         LIMIT {scan_limit}"
    );

    let mut bind_values = Vec::with_capacity(sources.len() * 3);
    for _ in 0..3 {
        bind_values.extend(sources.iter().copied());
    }

    let mut candidates: HashMap<String, i64> = HashMap::new();
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params_from_iter(bind_values), |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
        .map_err(|e| e.to_string())?;
    for row in rows.flatten() {
        let (partner, count) = row;
        if already_have.contains(&partner) {
            continue;
        }
        let existing = candidates.get(&partner).copied().unwrap_or(0);
        candidates.insert(partner, existing + count);
    }

    let mut ranked = candidates.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));
    ranked.truncate(limit);
    Ok(ranked.into_iter().map(|(source, score)| json!({"source": source, "coScore": score})).collect())
}

pub fn reset(conn: &Connection) -> Result<(), String> {
    conn.execute("DELETE FROM co_occurrence", []).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests;
