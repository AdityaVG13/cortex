use super::scan::{LEGACY_TABLES, scan_rows};
use super::*;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

fn row_from_decision(conn: &Connection, id: i64) -> Result<Option<Row>, StoreSpiError> {
    conn.query_row("SELECT id, decision, context, status, source_agent, created_at, COALESCE(version_id, 0) FROM decisions WHERE id = ?1", params![id], |r| Ok(Row { id: LogicalId::from_legacy("decision", r.get(0)?), revision: LogicalId::from_legacy("version", r.get::<_, i64>(6)?), kind: "decision".into(), body: json!({"text": r.get::<_, String>(1)?, "context": r.get::<_, Option<String>>(2)?, "status": r.get::<_, String>(3)?, "agent": r.get::<_, String>(4)?, "created_at": r.get::<_, String>(5)?}) })).optional().map_err(|e| StoreSpiError::Unavailable(e.to_string()))
}

fn row_from_memory(conn: &Connection, id: i64) -> Result<Option<Row>, StoreSpiError> {
    conn.query_row("SELECT id, text, type, status, source_agent, created_at, COALESCE(version_id, 0) FROM memories WHERE id = ?1", params![id], |r| Ok(Row { id: LogicalId::from_legacy("memory", r.get(0)?), revision: LogicalId::from_legacy("version", r.get::<_, i64>(6)?), kind: r.get::<_, String>(2)?, body: json!({"text": r.get::<_, String>(1)?, "status": r.get::<_, String>(3)?, "agent": r.get::<_, String>(4)?, "created_at": r.get::<_, String>(5)?}) })).optional().map_err(|e| StoreSpiError::Unavailable(e.to_string()))
}

impl ReadSnapshot for SqliteSnapshot<'_> {
    fn frontier(&self) -> &Frontier {
        &self.frontier
    }
    fn get(&self, refs: &[LogicalId]) -> Result<Vec<Row>, StoreSpiError> {
        let mut out = Vec::new();
        for reference in refs {
            let Ok(id) = reference.value.parse::<i64>() else {
                continue;
            };
            let row = match reference.namespace.as_str() {
                "decision" => row_from_decision(self.conn, id)?,
                "memory" => row_from_memory(self.conn, id)?,
                _ => None,
            };
            out.extend(row);
        }
        Ok(out)
    }
    fn scan(
        &self,
        predicate: &Predicate,
        continuation: Option<&str>,
        limits: ScanLimits,
    ) -> Result<Page, StoreSpiError> {
        scan_rows(self.conn, &self.frontier, predicate, continuation, limits)
    }
    fn candidates(
        &self,
        profile: &CandidateProfile,
        keys: &[String],
        limits: ScanLimits,
    ) -> Result<Page, StoreSpiError> {
        let CandidateProfile::ExactLexical = profile else {
            return Err(StoreSpiError::Unavailable(format!(
                "candidate profile {profile:?} not provided by the sqlite reference store"
            )));
        };
        let needles: Vec<String> = keys
            .iter()
            .map(|k| k.to_ascii_lowercase())
            .filter(|k| !k.is_empty())
            .collect();
        if needles.is_empty() {
            return Ok(Page::covered(
                Vec::new(),
                self.frontier.clone(),
                true,
                0,
                None,
            ));
        }
        // Exact profile: full scan with deterministic ordering; completeness is
        // never inferred from an index cutoff.
        let mut rows = Vec::new();
        let mut examined = 0u64;
        for target in LEGACY_TABLES {
            let mut stmt = self.conn.prepare(&format!("SELECT id, {col}, status, source_agent, created_at, COALESCE(version_id,0) FROM {table} WHERE status = 'active' ORDER BY id", col = target.text_col, table = target.table)).map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
            let iter = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, i64>(5)?,
                    ))
                })
                .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
            for item in iter {
                let (id, text, status, agent, created_at, version) =
                    item.map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
                examined += 1;
                let lower = text.to_ascii_lowercase();
                if needles.iter().all(|n| lower.contains(n.as_str())) {
                    rows.push(Row { id: LogicalId::from_legacy(target.ns, id), revision: LogicalId::from_legacy("version", version), kind: target.ns.into(), body: json!({"text": text, "status": status, "agent": agent, "created_at": created_at}) });
                }
            }
        }
        rows.sort_by(|a, b| a.id.canonical().cmp(&b.id.canonical()));
        let exhausted = rows.len() as u32 <= limits.rows;
        rows.truncate(limits.rows as usize);
        Ok(Page::covered(
            rows,
            self.frontier.clone(),
            exhausted,
            examined,
            None,
        ))
    }
}
