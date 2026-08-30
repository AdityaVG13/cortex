use crate::handlers::{estimate_tokens, estimate_tokens_from_chars};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;
pub(crate) fn stored_max_timestamp(conn: &Connection) -> Option<String> {
    let mem_max: Option<String> = conn.query_row("SELECT MAX(updated_at) FROM memories WHERE status = 'active' AND (expires_at IS NULL OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR julianday(valid_until) > julianday('now')) AND (version_id IS NULL OR version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned'))", [], |r| r.get(0)).ok().flatten();
    let dec_max: Option<String> = conn.query_row("SELECT MAX(updated_at) FROM decisions WHERE status = 'active' AND (expires_at IS NULL OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR julianday(valid_until) > julianday('now')) AND (version_id IS NULL OR version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned'))", [], |r| r.get(0)).ok().flatten();
    match (mem_max, dec_max) {
        (Some(a), Some(b)) => Some(if a > b { a } else { b }),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

pub(crate) fn get_last_boot_time(conn: &Connection, agent: &str) -> Option<String> {
    if let Some(stored) = stored_max_timestamp(conn) {
        return Some(stored);
    }
    conn.query_row("SELECT data FROM events WHERE type = 'agent_boot' AND source_agent = ?1 ORDER BY created_at DESC, id DESC LIMIT 1", params![agent], |r| {
        r.get::<_, String>(0)
    })
    .ok()
    .and_then(|data| serde_json::from_str::<Value>(&data).ok()?.get("timestamp")?.as_str().map(|s| s.to_string()))
}
pub(crate) fn fetch_messages_for_agent(conn: &Connection, agent: &str) -> Vec<Value> {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare_cached("SELECT sender, message FROM messages WHERE recipient = ?1 ORDER BY timestamp ASC, id ASC") {
        if let Ok(rows) = stmt.query_map(params![agent], |r| Ok(json!({"from":r.get::<_,String>(0)?,"message":r.get::<_,String>(1)?}))) {
            for row in rows.flatten() {
                out.push(row);
            }
        }
    }
    out
}
pub(crate) fn fetch_sessions(conn: &Connection) -> Vec<Value> {
    let mut out = Vec::new();
    if let Ok(mut stmt) =
        conn.prepare_cached("SELECT agent, project, description, files_json FROM sessions WHERE expires_at > ?1 ORDER BY agent ASC, rowid ASC")
    {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        if let Ok(rows) = stmt.query_map(params![now], |r| {
            let files_json: String = r.get(3)?;
            Ok(json!({
"agent":r.get::<_,String>(0)?,"project":r.get::<_,Option<String>>(1)?,"description":r.get::<_,Option<String>>(2)?,"files":
serde_json::from_str::<Value>(&files_json).unwrap_or(json!([]))}))
        }) {
            for row in rows.flatten() {
                out.push(row);
            }
        }
    }
    out
}
pub(crate) fn fetch_locks(conn: &Connection) -> Vec<Value> {
    let mut out = Vec::new();
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    if let Ok(mut stmt) = conn.prepare_cached("SELECT path, agent, expires_at FROM locks WHERE expires_at > ?1 ORDER BY path ASC, rowid ASC") {
        if let Ok(rows) = stmt.query_map(params![now], |r| {
            Ok(json!({"path":r.get::<_,String>(0)?,"agent":r.get::<_,String>(1)?,"expiresAt":r.get::<_,String
>(2)?}))
        }) {
            for row in rows.flatten() {
                out.push(row);
            }
        }
    }
    out
}
pub(crate) fn fetch_unread_feed(conn: &Connection, agent: &str) -> Vec<Value> {
    let ack: Option<String> = conn
        .query_row("SELECT last_seen_id FROM feed_acks WHERE agent = ?1", params![agent], |row| row.get(0))
        .optional()
        .ok()
        .flatten();
    let mut all: Vec<(String, String, String, String)> = Vec::new();
    if let Ok(mut stmt) = conn.prepare_cached("SELECT id, agent, kind, summary FROM feed ORDER BY timestamp ASC, id ASC") {
        if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))) {
            for row in rows.flatten() {
                all.push(row);
            }
        }
    }
    if let Some(ack_id) = ack {
        let mut past_ack = false;
        let mut unread = Vec::new();
        for (id, entry_agent, kind, summary) in all {
            if id == ack_id {
                past_ack = true;
                continue;
            }
            if past_ack && entry_agent != agent {
                unread.push(json!({"kind":kind,"agent":entry_agent,"summary":summary}));
            }
        }
        unread
    } else {
        all.into_iter()
            .filter(|(_, entry_agent, _, _)| entry_agent != agent)
            .map(|(_, entry_agent, kind, summary)| {
                json!({"kind":kind,"agent":
entry_agent,"summary":summary})
            })
            .collect()
    }
}
pub(crate) fn fetch_pending_tasks(conn: &Connection) -> Vec<Value> {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn
        .prepare_cached("SELECT task_id, title, priority, project, files_json FROM tasks WHERE status = 'pending' ORDER BY created_at ASC, task_id ASC LIMIT 5")
    {
        if let Ok(rows) = stmt.query_map([], |r| {
            let files_json: String = r.get(4)?;
            Ok(json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,
"priority":r.get::<_,String>(2)?,"project":r.get::<_,Option<String>>(3)?,"files":serde_json::from_str::<Value>(&files_json).
unwrap_or(json!([]))}))
        }) {
            for row in rows.flatten() {
                out.push(row);
            }
        }
    }
    out
}
pub(crate) fn fetch_claimed_tasks_for_agent(conn: &Connection, agent: &str) -> Vec<Value> {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare_cached(
        "SELECT task_id, title, priority, claimed_at FROM tasks WHERE status = 'claimed' AND claimed_by = ?1 ORDER BY claimed_at ASC, task_id ASC",
    ) {
        if let Ok(rows) = stmt.query_map(params![agent], |r| {
            Ok(json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,"priority":r.get
::<_,String>(2)?,"claimedAt":r.get::<_,Option<String>>(3)?}))
        }) {
            for row in rows.flatten() {
                out.push(row);
            }
        }
    }
    out
}
pub(crate) fn build_delta_capsule(conn: &Connection, agent: &str) -> (String, usize, String) {
    let last_boot = get_last_boot_time(conn, agent);
    let mut parts: Vec<String> = Vec::new();
    let messages = fetch_messages_for_agent(conn, agent);
    if !messages.is_empty() {
        let lines: Vec<String> = messages
            .iter()
            .map(|m| {
                let from = m.get("from").and_then(|v| v.as_str()).unwrap_or("?");
                let msg = m.get("message").and_then(|v| v.as_str()).unwrap_or("");
                let truncated: String = msg.chars().take(200).collect();
                format!("- From {from}: \"{truncated}\"")
            })
            .collect();
        parts.push(format!("## Pending Messages\n{}", lines.join("\n")));
    }
    let sessions = fetch_sessions(conn);
    let other_sessions: Vec<&Value> = sessions.iter().filter(|s| s.get("agent").and_then(|v| v.as_str()) != Some(agent)).collect();
    if !other_sessions.is_empty() {
        let lines: Vec<String> = other_sessions
            .iter()
            .map(|s| {
                let ag = s.get("agent").and_then(|v| v.as_str()).unwrap_or("?");
                let proj = s.get("project").and_then(|v| v.as_str()).unwrap_or("unknown");
                let desc = s.get("description").and_then(|v| v.as_str()).unwrap_or("no description");
                format!("- {ag} working on {proj}: \"{desc}\"")
            })
            .collect();
        parts.push(format!("## Active Agents\n{}", lines.join("\n")));
    }
    let locks = fetch_locks(conn);
    if !locks.is_empty() {
        let lines: Vec<String> = locks
            .iter()
            .map(|l| {
                let path = l.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                let ag = l.get("agent").and_then(|v| v.as_str()).unwrap_or("?");
                format!("- {path} locked by {ag}")
            })
            .collect();
        parts.push(format!("## Active Locks\n{}", lines.join("\n")));
    }
    let mut feed = fetch_unread_feed(conn, agent);
    if feed.len() > 10 {
        feed = feed.split_off(feed.len() - 10);
    }
    if !feed.is_empty() {
        let lines: Vec<String> = feed
            .iter()
            .map(|e| {
                let kind = e.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
                let ag = e.get("agent").and_then(|v| v.as_str()).unwrap_or("?");
                let summary = e.get("summary").and_then(|v| v.as_str()).unwrap_or("");
                format!("- [{kind}] {ag}: {summary}")
            })
            .collect();
        parts.push(format!("## Feed\n{}", lines.join("\n")));
    }
    let pending_tasks = fetch_pending_tasks(conn);
    if !pending_tasks.is_empty() {
        let lines: Vec<String> = pending_tasks
            .iter()
            .map(|t| {
                let pri = t.get("priority").and_then(|v| v.as_str()).unwrap_or("?");
                let title = t.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                format!("- [{pri}] {title}")
            })
            .collect();
        parts.push(format!("## Pending Tasks\n{}", lines.join("\n")));
    }
    let my_tasks = fetch_claimed_tasks_for_agent(conn, agent);
    if !my_tasks.is_empty() {
        let lines: Vec<String> = my_tasks
            .iter()
            .map(|t| {
                let pri = t.get("priority").and_then(|v| v.as_str()).unwrap_or("?");
                let title = t.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                format!("- [{pri}] {title}")
            })
            .collect();
        parts.push(format!("## Your Active Tasks\n{}", lines.join("\n")));
    }
    if let Ok(mut stmt) = conn.prepare_cached(
        "SELECT d.id, d.decision, d.disputes_id, d.confirmed_by,
                COALESCE(d.valid_from, d.observed_at, d.created_at), d.valid_until
         FROM decisions d
         WHERE d.status = 'disputed'
           AND (d.valid_from IS NULL OR julianday(d.valid_from) <= julianday('now'))
           AND (d.valid_until IS NULL OR julianday(d.valid_until) > julianday('now'))
           AND EXISTS (SELECT 1 FROM decision_conflicts c WHERE c.source_decision_id = d.id AND c.status = 'open')
         ORDER BY d.created_at DESC, d.id DESC LIMIT 6",
    ) {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        }) {
            let mut seen = HashSet::new();
            let mut lines: Vec<String> = Vec::new();
            for (id, decision, disputes_id, confirmed_by, valid_from, valid_until) in rows.flatten() {
                if !seen.insert(id) {
                    continue;
                }
                if let Some(disputed_id) = disputes_id {
                    seen.insert(disputed_id);
                }
                let mut line = crate::compiler::format_fact_line(
                    "decision",
                    id,
                    &decision,
                    "disputed",
                    confirmed_by.as_deref(),
                    valid_from.as_deref(),
                    valid_until.as_deref(),
                );
                if let Some(disputed_id) = disputes_id {
                    if let Ok((partner_decision, partner_status, partner_confirmed_by, partner_valid_from, partner_valid_until)) = conn.query_row(
                        "SELECT decision, status, confirmed_by, COALESCE(valid_from, observed_at, created_at), valid_until FROM decisions WHERE id = ?1",
                        params![disputed_id],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, Option<String>>(2)?,
                                row.get::<_, Option<String>>(3)?,
                                row.get::<_, Option<String>>(4)?,
                            ))
                        },
                    ) {
                        line.push_str(" vs ");
                        line.push_str(&crate::compiler::format_fact_line(
                            "decision",
                            disputed_id,
                            &partner_decision,
                            &partner_status,
                            partner_confirmed_by.as_deref(),
                            partner_valid_from.as_deref(),
                            partner_valid_until.as_deref(),
                        ));
                    }
                }
                lines.push(line);
            }
            if !lines.is_empty() {
                parts.push(format!("CONFLICTS:\n{}", lines.iter().map(|line| format!("- {line}")).collect::<Vec<_>>().join("\n")));
            }
        }
    }
    if let Some(focus) = crate::focus::focus_current(conn, agent) {
        let label = focus.get("label").and_then(|v| v.as_str()).unwrap_or("?");
        let entries = focus.get("entries").and_then(|v| v.as_u64()).unwrap_or(0);
        parts.push(format!("## Active Focus\n- {label} ({entries} entries)"));
    }
    if let Some(ref lb) = last_boot {
        if let Ok(mut stmt) =
            conn.prepare_cached("SELECT decision, context, source_agent FROM decisions WHERE status = 'active' AND created_at >= ?1 AND (expires_at IS NULL OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR julianday(valid_until) > julianday('now')) AND (version_id IS NULL OR version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned')) ORDER BY created_at DESC, rowid DESC LIMIT 5")
        {
            if let Ok(rows) = stmt.query_map(params![lb], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?))) {
                let lines: Vec<String> = rows
                    .flatten()
                    .map(|(dec, ctx, ag)| {
                        let c = ctx.map(|c| format!(" ({c})")).unwrap_or_default();
                        format!("- [{ag}] {dec}{c}")
                    })
                    .collect();
                if !lines.is_empty() {
                    parts.push(format!("New decisions:\n{}", lines.join("\n")));
                }
            }
        }
        if let Ok(mut stmt) =
            conn.prepare_cached("SELECT text, type FROM memories WHERE status = 'active' AND updated_at >= ?1 AND type != 'state' AND (expires_at IS NULL OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR julianday(valid_until) > julianday('now')) AND (version_id IS NULL OR version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned')) ORDER BY updated_at DESC, id DESC LIMIT 3")
        {
            if let Ok(rows) = stmt.query_map(params![lb], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) {
                let lines: Vec<String> = rows
                    .flatten()
                    .map(|(text, mtype)| {
                        let truncated: String = text.chars().take(100).collect();
                        format!("- [{mtype}] {truncated}")
                    })
                    .collect();
                if !lines.is_empty() {
                    parts.push(format!("New knowledge:\n{}", lines.join("\n")));
                }
            }
        }
        if let Ok(mut stmt) = conn
            .prepare("SELECT type, COUNT(*) as cnt FROM events WHERE created_at > ?1 AND type NOT IN ('brain_init', 'index_all', 'agent_boot') GROUP BY type ORDER BY type ASC")
        {
            if let Ok(rows) = stmt.query_map(params![lb], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))) {
                let entries: Vec<String> = rows.flatten().map(|(etype, cnt)| format!("{cnt} {}", etype.replace('_', " "))).collect();
                if !entries.is_empty() {
                    parts.push(format!("Activity since last boot: {}", entries.join(", ")));
                }
            }
        }
    }
    let has_new_section = parts
        .iter()
        .any(|p| p.starts_with("New decisions:") || p.starts_with("New knowledge:") || p.starts_with("Activity since last boot:"));
    if !has_new_section {
        let already_has_recent = parts.iter().any(|p| p.starts_with("Recent decisions:"));
        if !already_has_recent {
            if let Ok(mut stmt) = conn.prepare_cached("SELECT decision, context FROM decisions WHERE status = 'active' AND (expires_at IS NULL OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR julianday(valid_until) > julianday('now')) AND (version_id IS NULL OR version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned')) ORDER BY created_at DESC, id DESC LIMIT 5") {
            if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))) {
                let lines: Vec<String> = rows
                    .flatten()
                    .map(|(dec, ctx)| {
                        let c = ctx.map(|c| format!(" — {c}")).unwrap_or_default();
                        format!("- {dec}{c}")
                    })
                    .collect();
                if !lines.is_empty() {
                    parts.push(format!("Recent decisions:\n{}", lines.join("\n")));
                }
            }
        }
        }
    }
    let text = parts.join("\n\n");
    let tokens = estimate_tokens(&text);
    let freshness = last_boot
        .as_ref()
        .map(|lb| {
            let prefix: String = lb.chars().take(16).collect();
            format!("since {prefix}")
        })
        .unwrap_or_else(|| "first boot".to_string());
    (text, tokens, freshness)
}
pub(crate) fn estimate_raw_baseline(conn: &Connection, _home: &Path) -> usize {
    let mut total_chars: usize = 0;
    let mem_chars: i64 = conn
        .query_row("SELECT COALESCE(SUM(LENGTH(text)), 0) FROM memories WHERE status = 'active' AND (expires_at IS NULL OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR julianday(valid_until) > julianday('now'))", [], |r| r.get(0))
        .unwrap_or(0);
    total_chars += mem_chars as usize;
    let dec_chars: i64 = conn
        .query_row("SELECT COALESCE(SUM(LENGTH(decision)), 0) FROM decisions WHERE status = 'active' AND (expires_at IS NULL OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR julianday(valid_until) > julianday('now'))", [], |r| r.get(0))
        .unwrap_or(0);
    total_chars += dec_chars as usize;
    estimate_tokens_from_chars(total_chars)
}
pub(crate) fn record_boot(conn: &Connection, agent: &str) {
    let now = stored_max_timestamp(conn).unwrap_or_else(|| chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
    let _ = conn.execute(
        "INSERT INTO events (type, data, source_agent) VALUES (?1, ?2, ?3)",
        params![
            "agent_boot",
            serde_json::to_string(&json!({"timestamp"
:&now,"agent":agent}))
            .unwrap_or_default(),
            agent
        ],
    );
}
