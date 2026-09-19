use super::fetch::cached_rows;
use super::{
    boot_owner, boot_scope_allowlist, bounds_and_unorphaned, fetch_claimed_tasks_for_agent,
    fetch_locks, fetch_messages_for_agent, fetch_pending_tasks, fetch_sessions, fetch_unread_feed,
    get_last_boot_time, keep_boot_id, owner_clause,
};
use crate::db::{
    CREATED_UPDATED_STAMP_SQL, UPDATED_CREATED_STAMP_SQL, VALIDITY_START_SQL, validity_start_sql,
};
use crate::handlers::{estimate_tokens, same_agent};
use crate::protocol::json_str;
use rusqlite::{Connection, params};
use serde_json::Value;
use std::collections::HashSet;

fn push_block(parts: &mut Vec<String>, title: &str, lines: Vec<String>) {
    if !lines.is_empty() {
        parts.push(format!("{title}\n{}", lines.join("\n")));
    }
}

fn task_lines(rows: &[Value]) -> Vec<String> {
    rows.iter()
        .map(|t| {
            format!(
                "- [{}] {}",
                json_str(t, "priority", "?"),
                json_str(t, "title", "?")
            )
        })
        .collect()
}

fn boot_scoped_lines<R>(
    conn: &Connection,
    kind: &str,
    rows: Vec<R>,
    take: usize,
    id_of: impl Fn(&R) -> i64,
    line: impl Fn(R) -> String,
) -> Vec<String> {
    let ids: Vec<i64> = rows.iter().map(&id_of).collect();
    let Ok(allow) = boot_scope_allowlist(conn, kind, &ids) else {
        return Vec::new();
    };
    rows.into_iter()
        .filter(|row| keep_boot_id(&allow, id_of(row)))
        .take(take)
        .map(line)
        .collect()
}

pub fn build_delta_capsule(conn: &Connection, agent: &str) -> (String, usize, String) {
    let last_boot = get_last_boot_time(conn, agent);
    let mut parts: Vec<String> = Vec::new();
    let messages = fetch_messages_for_agent(conn, agent);
    push_block(
        &mut parts,
        "## Pending Messages",
        messages
            .iter()
            .map(|m| {
                let truncated: String = json_str(m, "message", "").chars().take(200).collect();
                format!("- From {}: \"{truncated}\"", json_str(m, "from", "?"))
            })
            .collect(),
    );
    let sessions = fetch_sessions(conn);
    let other_sessions: Vec<&Value> = sessions
        .iter()
        .filter(|s| !same_agent(json_str(s, "agent", ""), agent))
        .collect();
    push_block(
        &mut parts,
        "## Active Agents",
        other_sessions
            .iter()
            .map(|s| {
                format!(
                    "- {} working on {}: \"{}\"",
                    json_str(s, "agent", "?"),
                    json_str(s, "project", "unknown"),
                    json_str(s, "description", "no description")
                )
            })
            .collect(),
    );
    let locks = fetch_locks(conn);
    push_block(
        &mut parts,
        "## Active Locks",
        locks
            .iter()
            .map(|l| {
                format!(
                    "- {} locked by {}",
                    json_str(l, "path", "?"),
                    json_str(l, "agent", "?")
                )
            })
            .collect(),
    );
    let mut feed = fetch_unread_feed(conn, agent);
    if feed.len() > 10 {
        feed = feed.split_off(feed.len() - 10);
    }
    push_block(
        &mut parts,
        "## Feed",
        feed.iter()
            .map(|e| {
                format!(
                    "- [{}] {}: {}",
                    json_str(e, "kind", "?"),
                    json_str(e, "agent", "?"),
                    json_str(e, "summary", "")
                )
            })
            .collect(),
    );
    push_block(
        &mut parts,
        "## Pending Tasks",
        task_lines(&fetch_pending_tasks(conn)),
    );
    push_block(
        &mut parts,
        "## Your Active Tasks",
        task_lines(&fetch_claimed_tasks_for_agent(conn, agent)),
    );
    let disputed_start = validity_start_sql("d");
    let disputed_rows = cached_rows(
        conn,
        &format!(
            "SELECT d.id, d.decision, d.disputes_id, d.confirmed_by, {disputed_start}, d.valid_until FROM decisions d WHERE d.status = 'disputed'{} AND (d.valid_from IS NULL OR TRIM(d.valid_from) = '' OR julianday(d.valid_from) <= julianday('now')) AND (d.valid_until IS NULL OR TRIM(d.valid_until) = '' OR julianday(d.valid_until) > julianday('now')) AND EXISTS (SELECT 1 FROM decision_conflicts c WHERE c.source_decision_id = d.id AND c.status = 'open') ORDER BY julianday(d.created_at) DESC, d.id DESC LIMIT 6",
            owner_clause(conn, "decisions", boot_owner())
        ),
        [],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        },
    );
    let mut seen = HashSet::new();
    let mut lines: Vec<String> = Vec::new();
    for (id, decision, disputes_id, confirmed_by, valid_from, valid_until) in disputed_rows {
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
            if let Ok((partner_decision, partner_status, partner_confirmed_by, partner_valid_from, partner_valid_until)) = conn.query_row(&format!("SELECT decision, status, confirmed_by, {VALIDITY_START_SQL}, valid_until FROM decisions WHERE id = ?1{}", owner_clause(conn, "decisions", boot_owner())), params![disputed_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, Option<String>>(4)?))) {
                line.push_str(" vs ");
                line.push_str(&crate::compiler::format_fact_line("decision", disputed_id, &partner_decision, &partner_status, partner_confirmed_by.as_deref(), partner_valid_from.as_deref(), partner_valid_until.as_deref()));
            }
        }
        lines.push(line);
    }
    if !lines.is_empty() {
        parts.push(format!(
            "CONFLICTS:\n{}",
            lines
                .iter()
                .map(|line| format!("- {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    if let Some(focus) = crate::focus::focus_current(conn, agent, boot_owner()) {
        let label = json_str(&focus, "label", "?");
        let entries = focus.get("entries").and_then(|v| v.as_u64()).unwrap_or(0);
        parts.push(format!("## Active Focus\n- {label} ({entries} entries)"));
    }
    if let Some(ref lb) = last_boot {
        let collected = cached_rows(
            conn,
            &format!(
                "SELECT id, decision, context, source_agent FROM decisions WHERE status = 'active'{} AND julianday({CREATED_UPDATED_STAMP_SQL}) >= julianday(?1) AND {} ORDER BY julianday({CREATED_UPDATED_STAMP_SQL}) DESC, rowid DESC LIMIT 20",
                owner_clause(conn, "decisions", boot_owner()),
                bounds_and_unorphaned()
            ),
            params![lb],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        );
        let lines = boot_scoped_lines(
            conn,
            "decision",
            collected,
            5,
            |row| row.0,
            |(_, dec, ctx, ag)| {
                let c = ctx.map(|c| format!(" ({c})")).unwrap_or_default();
                format!("- [{ag}] {dec}{c}")
            },
        );
        push_block(&mut parts, "New decisions:", lines);
        let collected = cached_rows(
            conn,
            &format!(
                "SELECT id, text, type FROM memories WHERE status = 'active'{} AND julianday({UPDATED_CREATED_STAMP_SQL}) >= julianday(?1) AND type != 'state' AND {} ORDER BY julianday({UPDATED_CREATED_STAMP_SQL}) DESC, id DESC LIMIT 20",
                owner_clause(conn, "memories", boot_owner()),
                bounds_and_unorphaned()
            ),
            params![lb],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        );
        let lines = boot_scoped_lines(
            conn,
            "memory",
            collected,
            3,
            |row| row.0,
            |(_, text, mtype)| {
                let truncated: String = text.chars().take(100).collect();
                format!("- [{mtype}] {truncated}")
            },
        );
        push_block(&mut parts, "New knowledge:", lines);
        let entries: Vec<String> = cached_rows(
            conn,
            "SELECT type, COUNT(*) as cnt FROM events WHERE julianday(created_at) > julianday(?1) AND type NOT IN ('brain_init', 'index_all', 'agent_boot') GROUP BY type ORDER BY type ASC",
            params![lb],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        )
        .into_iter().map(|(etype, cnt)| format!("{cnt} {}", etype.replace('_', " "))).collect();
        if !entries.is_empty() {
            parts.push(format!("Activity since last boot: {}", entries.join(", ")));
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
            let collected = cached_rows(
                conn,
                &format!(
                    "SELECT id, decision, context FROM decisions WHERE status = 'active'{} AND {} ORDER BY julianday(created_at) DESC, id DESC LIMIT 20",
                    owner_clause(conn, "decisions", boot_owner()),
                    bounds_and_unorphaned()
                ),
                [],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                },
            );
            let lines = boot_scoped_lines(
                conn,
                "decision",
                collected,
                5,
                |row| row.0,
                |(_, dec, ctx)| {
                    let c = ctx.map(|c| format!(" — {c}")).unwrap_or_default();
                    format!("- {dec}{c}")
                },
            );
            push_block(&mut parts, "Recent decisions:", lines);
        }
    }
    let text = parts.join("\n\n");
    let tokens = estimate_tokens(&text);
    let freshness = last_boot
        .as_ref()
        .map(|lb| format!("since {}", lb.chars().take(16).collect::<String>()))
        .unwrap_or_else(|| "first boot".to_string());
    (text, tokens, freshness)
}
