use crate::handlers::{estimate_tokens, estimate_tokens_from_chars};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::Path;
const CURRENT_UPDATED_WINDOW: &str = "status = 'active' \
 AND updated_at IS NOT NULL AND TRIM(updated_at) != '' \
 AND (expires_at IS NULL OR TRIM(expires_at) = '' OR julianday(expires_at) > julianday('now')) \
 AND (valid_from IS NULL OR TRIM(valid_from) = '' OR julianday(valid_from) <= julianday('now')) \
 AND (valid_until IS NULL OR TRIM(valid_until) = '' OR julianday(valid_until) > julianday('now')) \
 AND (version_id IS NULL OR version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned'))";

fn latest_updated_at(conn: &Connection, table: &str) -> Option<String> {
    conn.query_row(
        &format!(
            "SELECT updated_at FROM {table} WHERE {CURRENT_UPDATED_WINDOW} \
             ORDER BY julianday(updated_at) DESC, id DESC LIMIT 1"
        ),
        [],
        |r| r.get(0),
    )
    .optional()
    .ok()
    .flatten()
}

fn later_timestamp(left: &str, right: &str) -> bool {
    match (
        super::parse_timestamp(Some(left)),
        super::parse_timestamp(Some(right)),
    ) {
        (Some(a), Some(b)) => b > a,
        (None, Some(_)) => true,
        _ => false,
    }
}

pub fn stored_max_timestamp(conn: &Connection) -> Option<String> {
    // `updated_at` mixes RFC3339 (`now_iso`) and SQLite `datetime('now')`.
    // `MAX(text)` is lexicographic (`T` > space), so a later space-format
    // aging/mutate stamp loses to an earlier same-day RFC3339 value.
    let mem_max = latest_updated_at(conn, "memories");
    let dec_max = latest_updated_at(conn, "decisions");
    match (mem_max, dec_max) {
        (Some(a), Some(b)) => Some(if later_timestamp(&a, &b) { b } else { a }),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// Boot identity matches Control Center `sessionMatchesAgent`: trim, drop a
/// trailing ` (model)` suffix, then ASCII-lowercase. Exact SQL `!=` treated
/// `claude-code` and `Claude-Code` / `claude-code (opus)` as other agents.
fn strip_trailing_model_suffix(raw: &str) -> &str {
    let s = raw.trim();
    if !s.ends_with(')') {
        return s;
    }
    let Some(open) = s.rfind('(') else {
        return s;
    };
    if open == 0 {
        return s;
    }
    let inner = &s[open + 1..s.len() - 1];
    if inner.is_empty() || inner.contains(')') {
        return s;
    }
    s[..open].trim()
}

fn agent_identity(raw: &str) -> String {
    strip_trailing_model_suffix(raw).to_ascii_lowercase()
}

fn same_boot_agent(left: &str, right: &str) -> bool {
    let a = left.trim();
    let b = right.trim();
    !a.is_empty() && !b.is_empty() && agent_identity(a) == agent_identity(b)
}

fn like_literal(raw: &str) -> String {
    raw.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Exact identity plus trailing ` (model)` so SQL can match without a
/// full-table scan. `LIKE` wildcards in the agent name are escaped.
pub(crate) fn boot_agent_match_params(agent: &str) -> Option<(String, String)> {
    let ident = agent_identity(agent);
    if ident.is_empty() {
        return None;
    }
    Some((ident.clone(), format!("{} (%", like_literal(&ident))))
}

pub fn get_last_boot_time(conn: &Connection, agent: &str) -> Option<String> {
    // `lower(trim(source_agent)) =` treated `claude-code (opus)` as a
    // different booter than `claude-code`, so the delta capsule missed the
    // last boot and replayed "new since boot" on every SessionStart.
    let (ident, like) = boot_agent_match_params(agent)?;
    conn.query_row(
        "SELECT created_at FROM events WHERE type = 'agent_boot' \
         AND (lower(trim(source_agent)) = ?1 OR lower(trim(source_agent)) LIKE ?2 ESCAPE '\\') \
         ORDER BY id DESC LIMIT 1",
        params![ident, like],
        |r| r.get::<_, String>(0),
    )
    .ok()
}
/// `AND owner_id = N` when the table carries owner scoping and a caller is
/// known; empty otherwise. Team-mode boot must not leak another owner's
/// messages, tasks, locks, feed or decisions into a caller's capsule.
pub fn owner_clause(conn: &Connection, table: &str, owner: Option<i64>) -> String {
    crate::db::owner_and_clause(conn, table, owner)
}
thread_local! {
    static BOOT_OWNER: std::cell::Cell<Option<i64>> = const { std::cell::Cell::new(None) };
    static BOOT_PATHS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}
pub fn set_boot_owner(owner: Option<i64>) {
    BOOT_OWNER.with(|cell| cell.set(owner));
}
pub fn boot_owner() -> Option<i64> {
    BOOT_OWNER.with(|cell| cell.get())
}
pub fn set_boot_paths(paths: &[String]) {
    let normalized = crate::handlers::recall::normalize_query_paths(paths);
    BOOT_PATHS.with(|cell| *cell.borrow_mut() = normalized);
}
pub fn with_boot_paths<R>(f: impl FnOnce(&[String]) -> R) -> R {
    BOOT_PATHS.with(|cell| f(&cell.borrow()))
}
/// `Ok(None)` means the boot is unscoped and every id stays eligible.
/// `Ok(Some(empty))` means the boot is path-scoped and no id survived.
/// `Err` is a failed path lookup: not "no paths" (read law would treat that
/// as visible and leak every foreign project into constraints / recent / identity).
pub(crate) fn boot_scope_allowlist(
    conn: &Connection,
    target_type: &str,
    ids: &[i64],
) -> Result<Option<HashSet<i64>>, String> {
    with_boot_paths(|paths| {
        if paths.is_empty() {
            return Ok(None);
        }
        let path_map =
            crate::handlers::recall::explicit_paths_by_target(conn, target_type, ids)?;
        Ok(Some(
            ids.iter()
                .copied()
                .filter(|id| {
                    crate::handlers::recall::read_path_sets(
                        paths,
                        path_map.get(id).map(Vec::as_slice).unwrap_or(&[]),
                    )
                })
                .collect(),
        ))
    })
}
pub(crate) fn keep_boot_id(allow: &Option<HashSet<i64>>, id: i64) -> bool {
    allow.as_ref().map(|ids| ids.contains(&id)).unwrap_or(true)
}
pub fn fetch_messages_for_agent(conn: &Connection, agent: &str) -> Vec<Value> {
    let mut out = Vec::new();
    // The messages table has no runtime writer or pruner (auto_repair salvage
    // repopulates it whole), so an unbounded SELECT rendered one capsule line
    // per message ever salvaged on every boot. Bound the capsule to the newest
    // messages, rendered oldest-first as before. Identity matching must stay in
    // SQL: a global newest-N scan then Rust filter drops this agent's mail when
    // other recipients are more recent.
    let Some((ident, like)) = boot_agent_match_params(agent) else {
        return out;
    };
    let scope = owner_clause(conn, "messages", boot_owner());
    if let Ok(mut stmt) = conn.prepare_cached(&format!(
        "SELECT sender, message FROM ( \
             SELECT sender, message, timestamp, id FROM messages \
             WHERE (lower(trim(recipient)) = ?1 OR lower(trim(recipient)) LIKE ?2 ESCAPE '\\'){scope} \
             ORDER BY julianday(timestamp) DESC, id DESC LIMIT 10 \
         ) ORDER BY julianday(timestamp) ASC, id ASC"
    )) {
        if let Ok(rows) = stmt.query_map(params![ident, like], |r| {
            Ok(json!({"from":r.get::<_,String>(0)?,"message":r.get::<_,String>(1)?}))
        }) {
            for row in rows.flatten() {
                out.push(row);
            }
        }
    }
    out
}
pub fn fetch_sessions(conn: &Connection) -> Vec<Value> {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare_cached(&format!(
        "SELECT agent, project, description, files_json FROM sessions WHERE expires_at IS NOT NULL AND TRIM(expires_at) != '' AND julianday(expires_at) > julianday('now'){} ORDER BY agent ASC, rowid ASC",
        owner_clause(conn, "sessions", boot_owner())
    )) {
        if let Ok(rows) = stmt.query_map([], |r| {
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
pub fn fetch_locks(conn: &Connection) -> Vec<Value> {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare_cached(&format!(
        "SELECT path, agent, expires_at FROM locks WHERE expires_at IS NOT NULL AND TRIM(expires_at) != '' AND julianday(expires_at) > julianday('now'){} ORDER BY path ASC, rowid ASC",
        owner_clause(conn, "locks", boot_owner())
    )) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok(json!({"path":r.get::<_,String>(0)?,"agent":r.get::<_,String>(1)?,"expiresAt":r.get::<_,String
>(2)?}))
        }) {
            for row in rows.flatten() {
                out.push(row);
            }
        }
    }
    with_boot_paths(|paths| {
        if paths.is_empty() {
            return;
        }
        out.retain(|lock| {
            let path = lock.get("path").and_then(|v| v.as_str()).unwrap_or("");
            crate::handlers::recall::read_path_sets(paths, &[path.to_string()])
        });
    });
    out
}
pub fn fetch_unread_feed(conn: &Connection, agent: &str) -> Vec<Value> {
    // The feed table has no retention pruner, so the previous whole-table scan
    // made every boot O(feed rows ever posted) in time and memory even though
    // the capsule renders at most the last 10 unread entries. Bound the scan
    // to the newest entries. Instant order (julianday) plus rowid matches
    // insertion among equal stamps; an ack whose feed row is gone still
    // yields no unread entries (it marked a position, not a filter).
    const FEED_CAPSULE_LINES: i64 = 10;
    // SQL error is not "never acked": swallowing it showed the newest 10 as
    // unread. Missing ack still means unread-from-start. Exact `lower(trim)`
    // missed an ack stored as `claude-code (opus)` when boot is `claude-code`.
    let Some((ident, like)) = boot_agent_match_params(agent) else {
        return Vec::new();
    };
    let ack = match feed_ack_last_seen(conn, agent) {
        Ok(ack) => ack,
        Err(_) => return Vec::new(),
    };
    if let Some(ack_id) = &ack {
        let anchor = match conn
            .query_row(
                &format!(
                    "SELECT timestamp FROM feed WHERE id = ?1{}",
                    owner_clause(conn, "feed", boot_owner())
                ),
                params![ack_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
        {
            Ok(anchor) => anchor,
            Err(_) => return Vec::new(),
        };
        let Some(anchor_ts) = anchor else {
            return Vec::new();
        };
        // `timestamp` mixes RFC3339 (`T`) and SQLite `datetime('now')` (space).
        // Lexicographic `>` skips a later space-format row after an RFC3339 ack.
        if let Ok(mut stmt) = conn.prepare_cached(&format!(
            "SELECT agent, kind, summary FROM feed \
             WHERE NOT (lower(trim(agent)) = ?1 OR lower(trim(agent)) LIKE ?2 ESCAPE '\\') \
               AND (julianday(timestamp) > julianday(?3) OR (julianday(timestamp) = julianday(?3) AND rowid > (SELECT rowid FROM feed WHERE id = ?4))){} \
             ORDER BY julianday(timestamp) DESC, rowid DESC LIMIT ?5",
            owner_clause(conn, "feed", boot_owner())
        )) {
            if let Ok(rows) = stmt.query_map(params![ident, like, anchor_ts, ack_id, FEED_CAPSULE_LINES], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
            }) {
                let mut newest: Vec<(String, String, String)> = rows.flatten().collect();
                newest.reverse();
                return newest
                    .into_iter()
                    .filter(|(entry_agent, ..)| !same_boot_agent(entry_agent, agent))
                    .map(|(entry_agent, kind, summary)| json!({"kind":kind,"agent":entry_agent,"summary":summary}))
                    .collect();
            }
        }
        return Vec::new();
    }
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare_cached(&format!(
        "SELECT agent, kind, summary FROM feed WHERE NOT (lower(trim(agent)) = ?1 OR lower(trim(agent)) LIKE ?2 ESCAPE '\\'){} ORDER BY julianday(timestamp) DESC, rowid DESC LIMIT ?3",
        owner_clause(conn, "feed", boot_owner())
    )) {
        if let Ok(rows) = stmt.query_map(params![ident, like, FEED_CAPSULE_LINES], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))) {
            let mut newest: Vec<(String, String, String)> = rows.flatten().collect();
            newest.reverse();
            for (entry_agent, kind, summary) in newest {
                if same_boot_agent(&entry_agent, agent) {
                    continue;
                }
                out.push(json!({"kind":kind,"agent":entry_agent,"summary":summary}));
            }
        }
    }
    out
}
pub fn fetch_pending_tasks(conn: &Connection) -> Vec<Value> {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare_cached(&format!(
        "SELECT task_id, title, priority, project, files_json FROM tasks WHERE status = 'pending'{} ORDER BY julianday(created_at) ASC, task_id ASC LIMIT 5",
        owner_clause(conn, "tasks", boot_owner())
    )) {
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
fn feed_ack_last_seen(conn: &Connection, agent: &str) -> rusqlite::Result<Option<String>> {
    let Some((ident, like)) = boot_agent_match_params(agent) else {
        return Ok(None);
    };
    conn.query_row(
        &format!(
            "SELECT last_seen_id FROM feed_acks \
             WHERE (lower(trim(agent)) = ?1 OR lower(trim(agent)) LIKE ?2 ESCAPE '\\'){} \
             ORDER BY rowid DESC LIMIT 1",
            owner_clause(conn, "feed_acks", boot_owner())
        ),
        params![ident, like],
        |row| row.get::<_, String>(0),
    )
    .optional()
}
pub fn fetch_claimed_tasks_for_agent(conn: &Connection, agent: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let Some((ident, like)) = boot_agent_match_params(agent) else {
        return out;
    };
    if let Ok(mut stmt) = conn.prepare_cached(&format!(
        "SELECT task_id, title, priority, claimed_at FROM tasks \
         WHERE status = 'claimed' AND (lower(trim(claimed_by)) = ?1 OR lower(trim(claimed_by)) LIKE ?2 ESCAPE '\\'){} \
         ORDER BY julianday(claimed_at) ASC, task_id ASC",
        owner_clause(conn, "tasks", boot_owner())
    )) {
        if let Ok(rows) = stmt.query_map(params![ident, like], |r| {
            Ok(json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,
"priority":r.get::<_,String>(2)?,"claimedAt":r.get::<_,Option<String>>(3)?}))
        }) {
            for row in rows.flatten() {
                out.push(row);
            }
        }
    }
    out
}
pub fn build_delta_capsule(conn: &Connection, agent: &str) -> (String, usize, String) {
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
    let other_sessions: Vec<&Value> = sessions
        .iter()
        .filter(|s| {
            !same_boot_agent(
                s.get("agent").and_then(|v| v.as_str()).unwrap_or(""),
                agent,
            )
        })
        .collect();
    if !other_sessions.is_empty() {
        let lines: Vec<String> = other_sessions
            .iter()
            .map(|s| {
                let ag = s.get("agent").and_then(|v| v.as_str()).unwrap_or("?");
                let proj = s
                    .get("project")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let desc = s
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("no description");
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
    if let Ok(mut stmt) = conn.prepare_cached(&format!(
        "SELECT d.id, d.decision, d.disputes_id, d.confirmed_by,
                COALESCE(NULLIF(TRIM(d.valid_from), ''), NULLIF(TRIM(d.observed_at), ''), d.created_at), d.valid_until
         FROM decisions d
         WHERE d.status = 'disputed'{}
           AND (d.valid_from IS NULL OR TRIM(d.valid_from) = '' OR julianday(d.valid_from) <= julianday('now'))
           AND (d.valid_until IS NULL OR TRIM(d.valid_until) = '' OR julianday(d.valid_until) > julianday('now'))
           AND EXISTS (SELECT 1 FROM decision_conflicts c WHERE c.source_decision_id = d.id AND c.status = 'open')
         ORDER BY julianday(d.created_at) DESC, d.id DESC LIMIT 6",
        owner_clause(conn, "decisions", boot_owner()),
    )) {
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
                        &format!(
                            "SELECT decision, status, confirmed_by, COALESCE(NULLIF(TRIM(valid_from), ''), NULLIF(TRIM(observed_at), ''), created_at), valid_until FROM decisions WHERE id = ?1{}",
                            owner_clause(conn, "decisions", boot_owner())
                        ),
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
    if let Some(focus) = crate::focus::focus_current(conn, agent, boot_owner()) {
        let label = focus.get("label").and_then(|v| v.as_str()).unwrap_or("?");
        let entries = focus.get("entries").and_then(|v| v.as_u64()).unwrap_or(0);
        parts.push(format!("## Active Focus\n- {label} ({entries} entries)"));
    }
    if let Some(ref lb) = last_boot {
        if let Ok(mut stmt) =
            conn.prepare_cached(&format!("SELECT id, decision, context, source_agent FROM decisions WHERE status = 'active'{} AND julianday(COALESCE(NULLIF(TRIM(created_at), ''), NULLIF(TRIM(updated_at), ''))) >= julianday(?1) AND (expires_at IS NULL OR TRIM(expires_at) = '' OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR TRIM(valid_from) = '' OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR TRIM(valid_until) = '' OR julianday(valid_until) > julianday('now')) AND (version_id IS NULL OR version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned')) ORDER BY julianday(COALESCE(NULLIF(TRIM(created_at), ''), NULLIF(TRIM(updated_at), ''))) DESC, rowid DESC LIMIT 20", owner_clause(conn, "decisions", boot_owner())))
        {
            if let Ok(rows) = stmt.query_map(params![lb], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, String>(3)?))) {
                let collected: Vec<(i64, String, Option<String>, String)> = rows.flatten().collect();
                let ids: Vec<i64> = collected.iter().map(|row| row.0).collect();
                if let Ok(allow) = boot_scope_allowlist(conn, "decision", &ids) {
                let lines: Vec<String> = collected
                    .into_iter()
                    .filter(|(id, ..)| keep_boot_id(&allow, *id))
                    .take(5)
                    .map(|(_, dec, ctx, ag)| {
                        let c = ctx.map(|c| format!(" ({c})")).unwrap_or_default();
                        format!("- [{ag}] {dec}{c}")
                    })
                    .collect();
                if !lines.is_empty() {
                    parts.push(format!("New decisions:\n{}", lines.join("\n")));
                }
                }
            }
        }
        if let Ok(mut stmt) =
            conn.prepare_cached(&format!("SELECT id, text, type FROM memories WHERE status = 'active'{} AND julianday(COALESCE(NULLIF(TRIM(updated_at), ''), NULLIF(TRIM(created_at), ''))) >= julianday(?1) AND type != 'state' AND (expires_at IS NULL OR TRIM(expires_at) = '' OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR TRIM(valid_from) = '' OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR TRIM(valid_until) = '' OR julianday(valid_until) > julianday('now')) AND (version_id IS NULL OR version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned')) ORDER BY julianday(COALESCE(NULLIF(TRIM(updated_at), ''), NULLIF(TRIM(created_at), ''))) DESC, id DESC LIMIT 20", owner_clause(conn, "memories", boot_owner())))
        {
            if let Ok(rows) = stmt.query_map(params![lb], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))) {
                let collected: Vec<(i64, String, String)> = rows.flatten().collect();
                let ids: Vec<i64> = collected.iter().map(|row| row.0).collect();
                if let Ok(allow) = boot_scope_allowlist(conn, "memory", &ids) {
                let lines: Vec<String> = collected
                    .into_iter()
                    .filter(|(id, ..)| keep_boot_id(&allow, *id))
                    .take(3)
                    .map(|(_, text, mtype)| {
                        let truncated: String = text.chars().take(100).collect();
                        format!("- [{mtype}] {truncated}")
                    })
                    .collect();
                if !lines.is_empty() {
                    parts.push(format!("New knowledge:\n{}", lines.join("\n")));
                }
                }
            }
        }
        if let Ok(mut stmt) = conn
            .prepare("SELECT type, COUNT(*) as cnt FROM events WHERE julianday(created_at) > julianday(?1) AND type NOT IN ('brain_init', 'index_all', 'agent_boot') GROUP BY type ORDER BY type ASC")
        {
            if let Ok(rows) = stmt.query_map(params![lb], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))) {
                let entries: Vec<String> = rows.flatten().map(|(etype, cnt)| format!("{cnt} {}", etype.replace('_', " "))).collect();
                if !entries.is_empty() {
                    parts.push(format!("Activity since last boot: {}", entries.join(", ")));
                }
            }
        }
    }
    let has_new_section = parts.iter().any(|p| {
        p.starts_with("New decisions:")
            || p.starts_with("New knowledge:")
            || p.starts_with("Activity since last boot:")
    });
    if !has_new_section {
        let already_has_recent = parts.iter().any(|p| p.starts_with("Recent decisions:"));
        if !already_has_recent {
            if let Ok(mut stmt) = conn.prepare_cached(&format!("SELECT id, decision, context FROM decisions WHERE status = 'active'{} AND (expires_at IS NULL OR TRIM(expires_at) = '' OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR TRIM(valid_from) = '' OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR TRIM(valid_until) = '' OR julianday(valid_until) > julianday('now')) AND (version_id IS NULL OR version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned')) ORDER BY julianday(created_at) DESC, id DESC LIMIT 20", owner_clause(conn, "decisions", boot_owner()))) {
            if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?))) {
                let collected: Vec<(i64, String, Option<String>)> = rows.flatten().collect();
                let ids: Vec<i64> = collected.iter().map(|row| row.0).collect();
                if let Ok(allow) = boot_scope_allowlist(conn, "decision", &ids) {
                let lines: Vec<String> = collected
                    .into_iter()
                    .filter(|(id, ..)| keep_boot_id(&allow, *id))
                    .take(5)
                    .map(|(_, dec, ctx)| {
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
pub fn estimate_raw_baseline(conn: &Connection, _home: &Path) -> usize {
    let mut total_chars: usize = 0;
    let mem_scope = owner_clause(conn, "memories", boot_owner());
    let dec_scope = owner_clause(conn, "decisions", boot_owner());
    let mem_chars: i64 = conn
        .query_row(&format!("SELECT COALESCE(SUM(LENGTH(text)), 0) FROM memories WHERE status = 'active' AND (expires_at IS NULL OR TRIM(expires_at) = '' OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR TRIM(valid_from) = '' OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR TRIM(valid_until) = '' OR julianday(valid_until) > julianday('now')){mem_scope}"), [], |r| r.get(0))
        .unwrap_or(0);
    total_chars += mem_chars as usize;
    let dec_chars: i64 = conn
        .query_row(&format!("SELECT COALESCE(SUM(LENGTH(decision)), 0) FROM decisions WHERE status = 'active' AND (expires_at IS NULL OR TRIM(expires_at) = '' OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR TRIM(valid_from) = '' OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR TRIM(valid_until) = '' OR julianday(valid_until) > julianday('now')){dec_scope}"), [], |r| r.get(0))
        .unwrap_or(0);
    total_chars += dec_chars as usize;
    estimate_tokens_from_chars(total_chars)
}
pub fn record_boot(conn: &Connection, agent: &str) {
    let now = stored_max_timestamp(conn)
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
    let _ = crate::handlers::log_event(
        conn,
        "agent_boot",
        json!({"timestamp": &now, "agent": agent}),
        agent,
    );
}
