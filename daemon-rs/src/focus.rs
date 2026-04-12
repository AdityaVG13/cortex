// SPDX-License-Identifier: MIT
//! Focus tools — context checkpointing for the sawtooth pattern.
//!
//! `focus_start(label)` opens a focus session. Entries stored/recalled during
//! the session are tracked as raw traces.
//!
//! `focus_end(label)` summarizes the raw traces into a compact summary,
//! stores the summary as a memory, and marks the session complete.
//!
//! The net effect: context grows during work (exploration), then collapses
//! at checkpoints (consolidation). Research shows 22.7% token reduction
//! with no loss in task accuracy (Focus Architecture, SWE-bench).

use rusqlite::{Connection, params};
use serde_json::{Value, json};

/// Start a new focus session. Returns the session ID.
pub fn focus_start(conn: &Connection, label: &str, agent: &str) -> Result<Value, String> {
    // Check for existing open session with same label
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM focus_sessions WHERE label = ?1 AND agent = ?2 AND status = 'open'",
            params![label, agent],
            |row| row.get(0),
        )
        .ok();

    if let Some(id) = existing {
        return Ok(json!({
            "id": id,
            "label": label,
            "status": "already_open",
            "message": "Focus session already open with this label"
        }));
    }

    conn.execute(
        "INSERT INTO focus_sessions (label, agent, status, raw_entries) VALUES (?1, ?2, 'open', '[]')",
        params![label, agent],
    )
    .map_err(|e| format!("Failed to start focus: {e}"))?;

    let id = conn.last_insert_rowid();

    Ok(json!({
        "id": id,
        "label": label,
        "status": "opened",
        "message": format!("Focus started: '{label}'. Store decisions normally — they'll be tracked. Call focus_end when done.")
    }))
}

/// Add an entry to the active focus session's raw trace.
/// Called automatically when cortex_store happens during an open focus.
pub fn focus_append(conn: &Connection, agent: &str, entry: &str) -> bool {
    let result: Option<(i64, String)> = conn
        .query_row(
            "SELECT id, raw_entries FROM focus_sessions WHERE agent = ?1 AND status = 'open' ORDER BY started_at DESC LIMIT 1",
            params![agent],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();

    if let Some((id, raw_json)) = result {
        let mut entries: Vec<String> = serde_json::from_str(&raw_json).unwrap_or_default();
        entries.push(entry.to_string());
        let updated = serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string());
        let _ = conn.execute(
            "UPDATE focus_sessions SET raw_entries = ?1 WHERE id = ?2",
            params![updated, id],
        );
        true
    } else {
        false
    }
}

/// End a focus session. Summarizes raw traces, stores the summary, returns stats.
pub fn focus_end(
    conn: &Connection,
    label: &str,
    agent: &str,
    owner_id: Option<i64>,
) -> Result<Value, String> {
    let session: Option<(i64, String)> = conn
        .query_row(
            "SELECT id, raw_entries FROM focus_sessions WHERE label = ?1 AND agent = ?2 AND status = 'open'",
            params![label, agent],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();

    let (id, raw_json) =
        session.ok_or_else(|| format!("No open focus session with label '{label}'"))?;
    let entries: Vec<String> = serde_json::from_str(&raw_json).unwrap_or_default();

    if entries.is_empty() {
        // Close without summary
        conn.execute(
            "UPDATE focus_sessions SET status = 'closed', ended_at = datetime('now') WHERE id = ?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;

        return Ok(json!({
            "id": id,
            "label": label,
            "status": "closed",
            "entries": 0,
            "summary": null,
            "message": "Focus closed (no entries captured)"
        }));
    }

    // Summarize the raw entries
    let tokens_before = entries.iter().map(|e| estimate_tokens(e)).sum::<usize>();
    let summary = summarize_entries(&entries);
    let tokens_after = estimate_tokens(&summary);

    // Store the summary as a memory
    if let Some(oid) = owner_id {
        conn.execute(
            "INSERT INTO memories (text, source, type, source_agent, confidence, owner_id) VALUES (?1, ?2, 'focus_summary', ?3, 0.9, ?4)",
            params![summary, format!("focus::{label}"), agent, oid],
        )
    } else {
        conn.execute(
            "INSERT INTO memories (text, source, type, source_agent, confidence) VALUES (?1, ?2, 'focus_summary', ?3, 0.9)",
            params![summary, format!("focus::{label}"), agent],
        )
    }
    .map_err(|e| format!("Failed to store focus summary: {e}"))?;

    // Close the session
    conn.execute(
        "UPDATE focus_sessions SET status = 'closed', summary = ?1, ended_at = datetime('now'), tokens_before = ?2, tokens_after = ?3 WHERE id = ?4",
        params![summary, tokens_before as i64, tokens_after as i64, id],
    )
    .map_err(|e| e.to_string())?;

    let savings = if tokens_before > 0 {
        ((1.0 - (tokens_after as f64 / tokens_before as f64)) * 100.0).round() as i64
    } else {
        0
    };

    Ok(json!({
        "id": id,
        "label": label,
        "status": "closed",
        "entries": entries.len(),
        "tokensBefore": tokens_before,
        "tokensAfter": tokens_after,
        "savings": format!("{savings}%"),
        "summary": summary,
        "message": format!("Focus '{label}' consolidated: {} entries → {} tokens ({}% reduction)", entries.len(), tokens_after, savings)
    }))
}

/// Get the currently open focus session for an agent.
pub fn focus_current(conn: &Connection, agent: &str) -> Option<Value> {
    conn.query_row(
        "SELECT id, label, raw_entries, started_at FROM focus_sessions WHERE agent = ?1 AND status = 'open' ORDER BY started_at DESC LIMIT 1",
        params![agent],
        |row| {
            let raw: String = row.get(2)?;
            let entries: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
            Ok(json!({
                "id": row.get::<_, i64>(0)?,
                "label": row.get::<_, String>(1)?,
                "entries": entries.len(),
                "startedAt": row.get::<_, String>(3)?,
            }))
        },
    )
    .ok()
}

// ─── Summarization ──────────────────────────────────────────────────────────

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

    // If nothing matched high-signal, keep first and last
    if kept.is_empty() {
        kept.push(&entries[0]);
        if entries.len() > 1 {
            kept.push(&entries[entries.len() - 1]);
        }
    }

    // Cap at 5 entries
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

fn estimate_tokens(text: &str) -> usize {
    (text.len() as f64 / 3.8).ceil() as usize
}
