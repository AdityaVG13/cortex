use crate::protocol::{Frontier, LogicalId};
use crate::store_spi::{Page, Predicate, Row, ScanLimits, StoreSpiError};
use rusqlite::{Connection, params};
use serde_json::{Value, json};

pub(super) struct LegacyTable {
    pub ns: &'static str,
    pub table: &'static str,
    pub text_col: &'static str,
}

pub(super) const LEGACY_TABLES: &[LegacyTable] = &[
    LegacyTable {
        ns: "decision",
        table: "decisions",
        text_col: "decision",
    },
    LegacyTable {
        ns: "memory",
        table: "memories",
        text_col: "text",
    },
];

pub(super) fn legacy_table(ns: &str) -> Option<&'static LegacyTable> {
    LEGACY_TABLES.iter().find(|table| table.ns == ns)
}

pub(super) struct ScanTarget {
    ns: &'static str,
    table: &'static str,
    text_col: &'static str,
    type_eq: Option<String>,
}

fn scan_target(table: &LegacyTable, type_eq: Option<String>) -> ScanTarget {
    ScanTarget {
        ns: table.ns,
        table: table.table,
        text_col: table.text_col,
        type_eq,
    }
}

pub(super) fn scan_targets(predicate: &Predicate) -> Vec<ScanTarget> {
    let kind = predicate_kind(predicate);
    let (tables, type_eq): (&[LegacyTable], Option<String>) = match kind {
        Some("decision") => (&LEGACY_TABLES[..1], None),
        Some("memory") => (&LEGACY_TABLES[1..], None),
        Some(other) => (LEGACY_TABLES, Some(other.to_string())),
        None => (LEGACY_TABLES, None),
    };
    tables
        .iter()
        .map(|table| scan_target(table, type_eq.clone()))
        .collect()
}

pub(super) fn parse_scan_cursor(
    continuation: Option<&str>,
    targets: &[ScanTarget],
) -> (usize, i64) {
    let Some(raw) = continuation.filter(|s| !s.is_empty()) else {
        return (0, 0);
    };
    for ns in ["memory", "decision"] {
        if let Some(rest) = raw.strip_prefix(ns).and_then(|s| s.strip_prefix(':')) {
            return targets
                .iter()
                .position(|t| t.ns == ns)
                .map(|idx| (idx, rest.parse().unwrap_or(0)))
                .unwrap_or((0, 0));
        }
    }
    (0, raw.parse().unwrap_or(0))
}

pub(super) fn cursor_token(multi: bool, ns: &str, id: i64) -> String {
    if multi {
        format!("{ns}:{id}")
    } else {
        id.to_string()
    }
}

pub(super) fn cursor_from_row(multi: bool, row: &Row) -> String {
    cursor_token(multi, &row.id.namespace, row.id.value.parse().unwrap_or(0))
}

pub(super) fn map_scan_row(
    r: &rusqlite::Row<'_>,
) -> rusqlite::Result<(i64, String, String, String, String, i64, String)> {
    Ok((
        r.get::<_, i64>(0)?,
        r.get::<_, String>(1)?,
        r.get::<_, String>(2)?,
        r.get::<_, String>(3)?,
        r.get::<_, String>(4)?,
        r.get::<_, i64>(5)?,
        r.get::<_, String>(6)?,
    ))
}

pub(super) fn scan_rows(
    conn: &Connection,
    frontier: &Frontier,
    predicate: &Predicate,
    continuation: Option<&str>,
    limits: ScanLimits,
) -> Result<Page, StoreSpiError> {
    let targets = scan_targets(predicate);
    let multi = targets.len() > 1;
    let (start_idx, mut after) = parse_scan_cursor(continuation, &targets);
    let mut rows = Vec::new();
    let mut bytes = 0u64;
    let mut examined = 0u64;
    let mut truncated = false;
    let mut continuation_out = None;
    let fetch = (limits.rows as i64).saturating_add(1).max(32);
    'tables: for (idx, target) in targets.iter().enumerate() {
        if idx < start_idx {
            continue;
        }
        if idx > start_idx {
            after = 0;
        }
        loop {
            let type_clause = if target.type_eq.is_some() {
                " AND type = ?3"
            } else {
                ""
            };
            let sql = format!(
                "SELECT id, {text_col}, status, source_agent, created_at, COALESCE(version_id,0), COALESCE(type, '{ns}') FROM {table} WHERE id > ?1{type_clause} ORDER BY id LIMIT ?2",
                text_col = target.text_col,
                ns = target.ns,
                table = target.table
            );
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
            let batch: Vec<(i64, String, String, String, String, i64, String)> =
                if let Some(typ) = &target.type_eq {
                    stmt.query_map(params![after, fetch, typ], map_scan_row)
                        .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?
                } else {
                    stmt.query_map(params![after, fetch], map_scan_row)
                        .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?
                };
            if batch.is_empty() {
                break;
            }
            let batch_len = batch.len();
            for (id, text, status, agent, created_at, version, row_type) in batch {
                examined += 1;
                after = id;
                let body = json!({"text": text, "status": status, "agent": agent, "created_at": created_at});
                if !predicate_matches(predicate, &body) {
                    continue;
                }
                if rows.len() as u32 >= limits.rows {
                    truncated = true;
                    continuation_out = rows
                        .last()
                        .map(|r| cursor_from_row(multi, r))
                        .or_else(|| Some(cursor_token(multi, target.ns, id)));
                    break 'tables;
                }
                let text_len = text.len() as u64;
                if bytes.saturating_add(text_len) > limits.bytes && limits.bytes > 0 {
                    if rows.is_empty() {
                        // A single row larger than the byte budget must not
                        // pin the cursor: skip it so the next page can move.
                        continue;
                    }
                    truncated = true;
                    continuation_out = rows.last().map(|r| cursor_from_row(multi, r));
                    break 'tables;
                }
                bytes = bytes.saturating_add(text_len);
                let kind = if target.ns == "decision" {
                    "decision".to_string()
                } else {
                    row_type
                };
                rows.push(Row {
                    id: LogicalId::from_legacy(target.ns, id),
                    revision: LogicalId::from_legacy("version", version),
                    kind,
                    body,
                });
            }
            if batch_len < fetch as usize {
                break;
            }
        }
    }
    Ok(Page::covered(
        rows,
        frontier.clone(),
        !truncated,
        examined,
        continuation_out,
    ))
}

pub(super) fn predicate_kind(p: &Predicate) -> Option<&str> {
    match p {
        Predicate::Kind(k) => Some(k.as_str()),
        Predicate::And(items) => items.iter().find_map(predicate_kind),
        _ => None,
    }
}

pub(super) fn predicate_matches(p: &Predicate, body: &Value) -> bool {
    match p {
        Predicate::All | Predicate::Kind(_) => true,
        Predicate::Eq { field, value } => body.get(field) == Some(value),
        Predicate::And(items) => items.iter().all(|i| predicate_matches(i, body)),
    }
}
