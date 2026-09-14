use crate::handlers::estimate_tokens;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

/// Pass 45 recovered a corrupt `raw_entries` blob on append so later
/// deposits still land. `focus_current` used to fail the JSON parse and
/// return None, so boot omitted ## Active Focus while `focus_start` still
/// reported already_open. Treat unreadable JSON the same way append does.
fn parse_focus_entries(raw_json: &str) -> Vec<String> {
    match serde_json::from_str::<Vec<String>>(raw_json) {
        Ok(entries) => entries,
        Err(_) => {
            let trimmed = raw_json.trim();
            if trimmed.is_empty() || trimmed == "[]" || trimmed == "null" {
                Vec::new()
            } else {
                vec![raw_json.to_string()]
            }
        }
    }
}

pub fn focus_start(conn: &Connection, label: &str, agent: &str) -> Result<Value, String> {
    let existing: Option<(i64, String)> = conn
        .query_row(
            "SELECT id, label FROM focus_sessions WHERE lower(trim(agent)) = lower(trim(?1)) AND status = 'open' ORDER BY julianday(started_at) DESC, id DESC LIMIT 1",
            params![agent],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| format!("Failed to look up focus: {e}"))?;
    if let Some((id, open_label)) = existing {
        return Ok(
            json!({"id":id,"label":open_label,"status":"already_open","message":
format!("Focus session already open with label '{open_label}'")}),
        );
    }
    conn.execute("INSERT INTO focus_sessions (label, agent, status, raw_entries) VALUES (?1, ?2, 'open', '[]')", params![label, agent])
        .map_err(|e| format!("Failed to start focus: {e}"))?;
    let id = conn.last_insert_rowid();
    Ok(json!({"id":id,"label":label,"status":"opened",
"message":format!("Focus started: '{label}'. Store decisions normally — they'll be tracked. Call focus_end when done.")}))
}
pub fn focus_append(conn: &Connection, agent: &str, entry: &str) -> bool {
    let result = conn.query_row(
        "SELECT id, raw_entries FROM focus_sessions WHERE lower(trim(agent)) = lower(trim(?1)) AND status = 'open' ORDER BY julianday(started_at) DESC, id DESC LIMIT 1",
        params![agent],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
    );
    let (id, raw_json) = match result {
        Ok(row) => row,
        Err(rusqlite::Error::QueryReturnedNoRows) => return false,
        Err(_) => return false,
    };
    // Corrupt raw_entries used to make every later append no-op while the
    // session stayed "open". Keep the unreadable blob as one entry so the
    // session can still capture new deposits until focus_end.
    let mut entries = parse_focus_entries(&raw_json);
    entries.push(entry.to_string());
    let Ok(updated) = serde_json::to_string(&entries) else {
        return false;
    };
    conn.execute(
        "UPDATE focus_sessions SET raw_entries = ?1 WHERE id = ?2 AND status = 'open'",
        params![updated, id],
    )
    .ok()
    .is_some_and(|n| n > 0)
}
pub fn focus_end(
    conn: &mut Connection,
    label: &str,
    agent: &str,
    owner_id: Option<i64>,
) -> Result<Value, String> {
    let session: Option<(i64, String)> = conn
        .query_row("SELECT id, raw_entries FROM focus_sessions WHERE label = ?1 AND lower(trim(agent)) = lower(trim(?2)) AND status = 'open'", params![label, agent], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .optional()
        .map_err(|e| format!("Failed to look up focus: {e}"))?;
    let (id, raw_json) =
        session.ok_or_else(|| format!("No open focus session with label '{label}'"))?;
    let entries: Vec<String> = match serde_json::from_str(&raw_json) {
        Ok(entries) => entries,
        Err(e) => {
            conn.execute(
                "UPDATE focus_sessions SET status = 'closed', ended_at = datetime('now') WHERE id = ?1",
                params![id],
            )
            .map_err(|err| err.to_string())?;
            return Err(format!(
                "focus session '{label}' raw_entries is not valid JSON: {e}; session closed without a summary"
            ));
        }
    };
    if entries.is_empty() {
        conn.execute(
            "UPDATE focus_sessions SET status = 'closed', ended_at = datetime('now') WHERE id = ?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        return Ok(
            json!({"id":id,"label":label,"status":"closed","entries":0,"summary":null,"message":
"Focus closed (no entries captured)"}),
        );
    }
    let tokens_before = entries.iter().map(|e| estimate_tokens(e)).sum::<usize>();
    let summary = summarize_entries(&entries);
    let tokens_after = estimate_tokens(&summary);
    // The summary memory and the session close must commit together: if the
    // memory INSERT committed but the UPDATE failed, a client retry of
    // focus_end would store a duplicate summary (memories.source has no
    // uniqueness constraint).
    let tx = conn
        .transaction()
        .map_err(|e| format!("Failed to start focus close transaction: {e}"))?;
    let stored_summary = if let Some(oid) = owner_id {
        tx.execute(
            "INSERT INTO memories (text, source, type, source_agent, confidence, owner_id, observed_at, valid_from) \
             VALUES (?1, ?2, 'focus_summary', ?3, 0.9, ?4, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![summary, format!("focus::{label}"), agent, oid],
        )
    } else {
        tx.execute(
            "INSERT INTO memories (text, source, type, source_agent, confidence, observed_at, valid_from) \
             VALUES (?1, ?2, 'focus_summary', ?3, 0.9, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![summary, format!("focus::{label}"), agent],
        )
    };
    stored_summary.map_err(|e| format!("Failed to store focus summary: {e}"))?;
    tx.execute(
        "UPDATE focus_sessions SET status = 'closed', summary = ?1, ended_at = datetime('now'), tokens_before = ?2, tokens_after = ?3 WHERE id = ?4",
        params![summary, tokens_before as i64, tokens_after as i64, id],
    )
    .map_err(|e| e.to_string())?;
    tx.commit()
        .map_err(|e| format!("Failed to commit focus close: {e}"))?;
    let savings = if tokens_before > 0 {
        ((1.0 - (tokens_after as f64 / tokens_before as f64)) * 100.0).round() as i64
    } else {
        0
    };
    Ok(json!({"id":id,"label":label,"status":"closed",
"entries":entries.len(),"tokensBefore":tokens_before,"tokensAfter":tokens_after,"savings":format!("{savings}%"),"summary":summary,
"message":format!("Focus '{label}' consolidated: {} entries → {} tokens ({}% reduction)",entries.len(),tokens_after,savings)}))
}
pub fn focus_current(conn: &Connection, agent: &str, owner: Option<i64>) -> Option<Value> {
    let scope = crate::db::owner_and_clause(conn, "focus_sessions", owner);
    conn.query_row(
        &format!("SELECT id, label, raw_entries, started_at FROM focus_sessions WHERE lower(trim(agent)) = lower(trim(?1)) AND status = 'open'{scope} ORDER BY julianday(started_at) DESC, id DESC LIMIT 1"),
        params![agent],
        |row| {
            let raw: String = row.get(2)?;
            let entries = parse_focus_entries(&raw);
            Ok(json!({
"id":row.get::<_,i64>(0)?,"label":row.get::<_,String>(1)?,"entries":entries.len(),"startedAt":row.get::<_,String>(3)?,}))
        },
    )
    .ok()
}
fn summarize_entries(entries: &[String]) -> String {
    if entries.len() <= 3 {
        return entries.join(" | ");
    }
    let high_signal = [
        "decision",
        "fixed",
        "built",
        "created",
        "removed",
        "changed",
        "bug",
        "error",
        "confirmed",
        "architecture",
        "migration",
        "breaking",
        "security",
        "important",
        "must",
        "never",
    ];
    let mut kept: Vec<&str> = Vec::new();
    for entry in entries {
        let lower = entry.to_lowercase();
        if high_signal.iter().any(|kw| lower.contains(kw)) {
            kept.push(entry);
        }
    }
    if kept.is_empty() {
        kept.push(&entries[0]);
        if entries.len() > 1 {
            kept.push(&entries[entries.len() - 1]);
        }
    }
    if kept.len() > 5 {
        kept.truncate(5);
    }
    let result = kept.join(" | ");
    if result.len() > 500 {
        result.chars().take(500).collect::<String>() + "..."
    } else {
        result
    }
}
