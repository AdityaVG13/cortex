// SPDX-License-Identifier: MIT
use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection};
use serde_json::{json, Value};

/// Record pairwise co-occurrences for every unique pair in `sources`.
/// Sources that are blank or appear only once are ignored.
/// At most 10 unique sources are considered per call.
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

/// Return up to `limit` sources that frequently co-occur with
/// `recalled_sources` but are not already in that set.
/// Each result is a JSON object `{ "source": "...", "coScore": <i64> }`.
#[allow(dead_code)]
pub fn predict(
    conn: &Connection,
    recalled_sources: &[String],
    limit: usize,
) -> Result<Vec<Value>, String> {
    if recalled_sources.is_empty() {
        return Ok(vec![]);
    }

    let already_have = recalled_sources
        .iter()
        .filter(|s| !s.trim().is_empty())
        .cloned()
        .collect::<HashSet<_>>();

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
            .query_map(params![source], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
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

    Ok(ranked
        .into_iter()
        .map(|(source, score)| json!({ "source": source, "coScore": score }))
        .collect())
}

/// Delete all rows from the `co_occurrence` table.
pub fn reset(conn: &Connection) -> Result<(), String> {
    conn.execute("DELETE FROM co_occurrence", [])
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::configure(&conn).unwrap();
        crate::db::initialize_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_record_and_predict() {
        let conn = setup();

        let sources = vec![
            "source_a".to_string(),
            "source_b".to_string(),
            "source_c".to_string(),
        ];
        record(&conn, &sources).unwrap();
        record(&conn, &sources).unwrap(); // Second call increases counts

        let predictions = predict(&conn, &["source_a".to_string()], 5).unwrap();
        assert!(!predictions.is_empty());

        // Every prediction should have a coScore > 0
        for p in &predictions {
            assert!(p["coScore"].as_i64().unwrap() > 0);
        }
    }

    #[test]
    fn test_predict_excludes_known_sources() {
        let conn = setup();

        let sources = vec!["source_a".to_string(), "source_b".to_string()];
        record(&conn, &sources).unwrap();

        // Predicting with both sources — neither should appear in results
        let predictions = predict(&conn, &sources, 5).unwrap();
        for p in &predictions {
            let s = p["source"].as_str().unwrap();
            assert_ne!(s, "source_a");
            assert_ne!(s, "source_b");
        }
    }

    #[test]
    fn test_reset() {
        let conn = setup();

        let sources = vec!["source_a".to_string(), "source_b".to_string()];
        record(&conn, &sources).unwrap();

        reset(&conn).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM co_occurrence", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_record_fewer_than_two_sources_is_noop() {
        let conn = setup();
        record(&conn, &["only_one".to_string()]).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM co_occurrence", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_predict_empty_recalled_sources() {
        let conn = setup();
        let results = predict(&conn, &[], 5).unwrap();
        assert!(results.is_empty());
    }
}
