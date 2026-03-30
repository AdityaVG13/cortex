//! Boot prompt compiler — capsule-based system.
//!
//! Ported from `src/compiler.js` (707 lines).  Builds an identity capsule
//! (stable, ~200 tokens) and a delta capsule (what changed since last boot,
//! ~300 tokens), then assembles them within a token budget.
//!
//! The compiler also reads state.md, memory files, lessons, conductor state
//! (sessions, locks, feed, tasks, messages), and recent decisions/memories
//! to produce a compact, high-signal boot prompt.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

// ─── Public types ───────────────────────────────────────────────────────────

/// The assembled boot prompt and its metadata.
pub struct BootResult {
    pub boot_prompt: String,
    pub token_estimate: usize,
    pub savings: Value,
    pub capsules: Vec<Value>,
}

// ─── Token estimation ───────────────────────────────────────────────────────

/// Estimate tokens from character length (~3.8 chars/token, matching Node.js).
fn estimate_tokens(text: &str) -> usize {
    (text.len() as f64 / 3.8).ceil() as usize
}

// ─── State.md helpers ───────────────────────────────────────────────────────

fn read_state_md(home: &Path) -> String {
    let path = home.join(".claude").join("state.md");
    std::fs::read_to_string(path).unwrap_or_default()
}

/// Extract the content under a `## heading` from a Markdown document,
/// capturing at most `max_lines` lines until the next `## `.
fn extract_section(content: &str, heading: &str, max_lines: usize) -> String {
    let mut capturing = false;
    let mut captured: Vec<&str> = Vec::new();

    for line in content.lines() {
        if capturing {
            if line.starts_with("## ") {
                break;
            }
            captured.push(line);
            if captured.len() >= max_lines {
                break;
            }
        } else if line.trim_start_matches("## ").trim() == heading {
            capturing = true;
        }
    }

    captured.join("\n").trim().to_string()
}

// ─── Identity capsule ───────────────────────────────────────────────────────

/// Build the identity capsule — stable across sessions, ~200 tokens.
/// Contains core user identity, hard constraints, and platform sharp edges.
fn build_identity_capsule(conn: &Connection) -> (String, usize) {
    let mut parts = vec![
        "User: Aditya. Platform: Windows 10. Shell: bash. Python: uv only. Git: conventional commits."
            .to_string(),
    ];

    // Hard constraints (never/always/must rules)
    if let Ok(constraint_re) =
        Regex::new(r"(?i)\b(never|always|must|do not|don't|required|mandatory)\b")
    {
        if let Ok(mut stmt) = conn.prepare(
            "SELECT text FROM memories WHERE type = 'feedback' AND status = 'active' ORDER BY score DESC LIMIT 20",
        ) {
            if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
                let constraints: Vec<String> = rows
                    .filter_map(|r| r.ok())
                    .filter(|t| constraint_re.is_match(t))
                    .take(5)
                    .map(|t| t.chars().take(120).collect::<String>())
                    .collect();
                if !constraints.is_empty() {
                    parts.push(format!("Rules: {}", constraints.join(" | ")));
                }
            }
        }
    }

    // Platform sharp edges (Windows-specific gotchas)
    if let Ok(edge_re) =
        Regex::new(r"(?i)\b(windows|win32|encoding|cp1252|bash\.exe|CRLF)\b")
    {
        if let Ok(mut stmt) = conn.prepare(
            "SELECT text FROM memories WHERE type = 'feedback' AND status = 'active' ORDER BY score DESC LIMIT 20",
        ) {
            if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
                let edges: Vec<String> = rows
                    .filter_map(|r| r.ok())
                    .filter(|t| edge_re.is_match(t))
                    .take(3)
                    .map(|t| t.chars().take(100).collect::<String>())
                    .collect();
                if !edges.is_empty() {
                    parts.push(format!("Sharp edges: {}", edges.join(" | ")));
                }
            }
        }
    }

    let text = parts.join("\n");
    let tokens = estimate_tokens(&text);
    (text, tokens)
}

// ─── Last boot time ─────────────────────────────────────────────────────────

fn get_last_boot_time(conn: &Connection, agent: &str) -> Option<String> {
    conn.query_row(
        "SELECT data FROM events WHERE type = 'agent_boot' AND source_agent = ?1 ORDER BY created_at DESC LIMIT 1",
        params![agent],
        |r| r.get::<_, String>(0),
    )
    .ok()
    .and_then(|data| {
        serde_json::from_str::<Value>(&data)
            .ok()?
            .get("timestamp")?
            .as_str()
            .map(|s| s.to_string())
    })
}

// ─── Conductor state helpers ────────────────────────────────────────────────

fn fetch_messages_for_agent(conn: &Connection, agent: &str) -> Vec<Value> {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT sender, message FROM messages WHERE recipient = ?1 ORDER BY timestamp ASC",
    ) {
        if let Ok(rows) = stmt.query_map(params![agent], |r| {
            Ok(json!({
                "from": r.get::<_, String>(0)?,
                "message": r.get::<_, String>(1)?
            }))
        }) {
            for row in rows.flatten() {
                out.push(row);
            }
        }
    }
    out
}

fn fetch_sessions(conn: &Connection) -> Vec<Value> {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT agent, project, description, files_json FROM sessions WHERE expires_at > ?1",
    ) {
        let now = chrono::Utc::now()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        if let Ok(rows) = stmt.query_map(params![now], |r| {
            let files_json: String = r.get(3)?;
            Ok(json!({
                "agent": r.get::<_, String>(0)?,
                "project": r.get::<_, Option<String>>(1)?,
                "description": r.get::<_, Option<String>>(2)?,
                "files": serde_json::from_str::<Value>(&files_json).unwrap_or(json!([]))
            }))
        }) {
            for row in rows.flatten() {
                out.push(row);
            }
        }
    }
    out
}

fn fetch_locks(conn: &Connection) -> Vec<Value> {
    let mut out = Vec::new();
    let now = chrono::Utc::now()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    if let Ok(mut stmt) = conn.prepare(
        "SELECT path, agent, expires_at FROM locks WHERE expires_at > ?1",
    ) {
        if let Ok(rows) = stmt.query_map(params![now], |r| {
            Ok(json!({
                "path": r.get::<_, String>(0)?,
                "agent": r.get::<_, String>(1)?,
                "expiresAt": r.get::<_, String>(2)?
            }))
        }) {
            for row in rows.flatten() {
                out.push(row);
            }
        }
    }
    out
}

fn fetch_unread_feed(conn: &Connection, agent: &str) -> Vec<Value> {
    let ack: Option<String> = conn
        .query_row(
            "SELECT last_seen_id FROM feed_acks WHERE agent = ?1",
            params![agent],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten();

    let mut all: Vec<(String, String, String, String)> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT id, agent, kind, summary FROM feed ORDER BY timestamp ASC",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        }) {
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
                unread.push(json!({
                    "kind": kind,
                    "agent": entry_agent,
                    "summary": summary
                }));
            }
        }
        unread
    } else {
        // No ack — all entries from other agents are unread
        all.into_iter()
            .filter(|(_, entry_agent, _, _)| entry_agent != agent)
            .map(|(_, entry_agent, kind, summary)| {
                json!({
                    "kind": kind,
                    "agent": entry_agent,
                    "summary": summary
                })
            })
            .collect()
    }
}

fn fetch_pending_tasks(conn: &Connection) -> Vec<Value> {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT task_id, title, priority, project, files_json FROM tasks WHERE status = 'pending' ORDER BY created_at ASC LIMIT 5",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            let files_json: String = r.get(4)?;
            Ok(json!({
                "id": r.get::<_, String>(0)?,
                "title": r.get::<_, String>(1)?,
                "priority": r.get::<_, String>(2)?,
                "project": r.get::<_, Option<String>>(3)?,
                "files": serde_json::from_str::<Value>(&files_json).unwrap_or(json!([]))
            }))
        }) {
            for row in rows.flatten() {
                out.push(row);
            }
        }
    }
    out
}

fn fetch_claimed_tasks_for_agent(conn: &Connection, agent: &str) -> Vec<Value> {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT task_id, title, priority, claimed_at FROM tasks WHERE status = 'claimed' AND claimed_by = ?1 ORDER BY claimed_at ASC",
    ) {
        if let Ok(rows) = stmt.query_map(params![agent], |r| {
            Ok(json!({
                "id": r.get::<_, String>(0)?,
                "title": r.get::<_, String>(1)?,
                "priority": r.get::<_, String>(2)?,
                "claimedAt": r.get::<_, Option<String>>(3)?
            }))
        }) {
            for row in rows.flatten() {
                out.push(row);
            }
        }
    }
    out
}

// ─── Delta capsule ──────────────────────────────────────────────────────────

/// Build the delta capsule — what changed since the agent's last boot.
/// High relevance, changes every session.  Target: ~300 tokens.
fn build_delta_capsule(conn: &Connection, home: &Path, agent: &str) -> (String, usize, String) {
    let last_boot = get_last_boot_time(conn, agent);
    let mut parts: Vec<String> = Vec::new();

    // 0. Pending messages (highest priority)
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

    // 0b. Active agents (session bus — who else is online)
    let sessions = fetch_sessions(conn);
    let other_sessions: Vec<&Value> = sessions
        .iter()
        .filter(|s| s.get("agent").and_then(|v| v.as_str()) != Some(agent))
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

    // 0c. Active locks
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

    // 0d. Shared feed (unread entries from other agents)
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

    // 0e. Task board (pending tasks + agent's claimed tasks)
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

    // 1. Open conflicts (always include — highest priority)
    if let Ok(mut stmt) = conn.prepare(
        "SELECT id, decision, source_agent, disputes_id FROM decisions WHERE status = 'disputed' ORDER BY created_at DESC LIMIT 6",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<i64>>(3)?,
            ))
        }) {
            let mut seen = HashSet::new();
            let mut lines: Vec<String> = Vec::new();
            for (id, decision, source_agent, disputes_id) in rows.flatten() {
                if seen.contains(&id) {
                    continue;
                }
                seen.insert(id);
                if let Some(did) = disputes_id {
                    seen.insert(did);
                }

                let mut line = format!("#{id} ({source_agent}): {decision}");
                if let Some(did) = disputes_id {
                    if let Ok((partner_dec, partner_agent)) = conn.query_row(
                        "SELECT decision, source_agent FROM decisions WHERE id = ?1",
                        params![did],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                    ) {
                        line.push_str(&format!(
                            " vs #{did} ({partner_agent}): {partner_dec}"
                        ));
                    }
                }
                lines.push(line);
            }
            if !lines.is_empty() {
                parts.push(format!(
                    "CONFLICTS:\n{}",
                    lines.iter().map(|l| format!("- {l}")).collect::<Vec<_>>().join("\n")
                ));
            }
        }
    }

    // 2. State.md: next session + pending + known issues (always fresh)
    let state = read_state_md(home);
    if !state.is_empty() {
        let next = extract_section(&state, "Next Session", 5);
        if !next.is_empty() {
            parts.push(format!("Next: {}", next.replace('\n', " | ")));
        }
        let pending = extract_section(&state, "Pending", 3);
        if !pending.is_empty() {
            parts.push(format!("Pending: {}", pending.replace('\n', " | ")));
        }
        let issues = extract_section(&state, "Known Issues", 3);
        if !issues.is_empty() {
            parts.push(format!("Issues: {}", issues.replace('\n', " | ")));
        }
    }

    // 3. New decisions since last boot
    if let Some(ref lb) = last_boot {
        if let Ok(mut stmt) = conn.prepare(
            "SELECT decision, context, source_agent FROM decisions WHERE status = 'active' AND created_at >= ?1 ORDER BY created_at DESC LIMIT 5",
        ) {
            if let Ok(rows) = stmt.query_map(params![lb], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                ))
            }) {
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

        // 4. New memories since last boot
        if let Ok(mut stmt) = conn.prepare(
            "SELECT text, type FROM memories WHERE status = 'active' AND updated_at >= ?1 AND type != 'state' ORDER BY updated_at DESC LIMIT 3",
        ) {
            if let Ok(rows) = stmt.query_map(params![lb], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            }) {
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

        // 5. Events since last boot (summarized)
        if let Ok(mut stmt) = conn.prepare(
            "SELECT type, COUNT(*) as cnt FROM events WHERE created_at > ?1 AND type NOT IN ('brain_init', 'index_all', 'agent_boot') GROUP BY type",
        ) {
            if let Ok(rows) = stmt.query_map(params![lb], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            }) {
                let entries: Vec<String> = rows
                    .flatten()
                    .map(|(etype, cnt)| {
                        format!("{cnt} {}", etype.replace('_', " "))
                    })
                    .collect();
                if !entries.is_empty() {
                    parts.push(format!("Activity since last boot: {}", entries.join(", ")));
                }
            }
        }
    } else {
        // First boot for this agent — include recent decisions as orientation
        if let Ok(mut stmt) = conn.prepare(
            "SELECT decision, context FROM decisions WHERE status = 'active' ORDER BY created_at DESC LIMIT 5",
        ) {
            if let Ok(rows) = stmt.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            }) {
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

// ─── Raw baseline estimation ────────────────────────────────────────────────

/// Estimate what raw file reads would cost — the baseline Cortex replaces.
/// Counts chars in state.md + all memory files + lessons.
fn estimate_raw_baseline(home: &Path) -> usize {
    let mut total_chars: usize = 0;

    // state.md
    let state_path = home.join(".claude").join("state.md");
    if let Ok(meta) = std::fs::metadata(&state_path) {
        total_chars += meta.len() as usize;
    }

    // Memory files
    let mem_dir = home
        .join(".claude")
        .join("projects")
        .join("C--Users-aditya")
        .join("memory");
    if let Ok(entries) = std::fs::read_dir(&mem_dir) {
        for entry in entries.flatten() {
            if entry
                .path()
                .extension()
                .map(|x| x == "md")
                .unwrap_or(false)
            {
                if let Ok(meta) = entry.metadata() {
                    total_chars += meta.len() as usize;
                }
            }
        }
    }

    // Lessons directory
    let lessons_dir = home.join("self-improvement-engine").join("lessons");
    if let Ok(entries) = std::fs::read_dir(&lessons_dir) {
        for entry in entries.flatten() {
            if entry
                .path()
                .extension()
                .map(|x| x == "md" || x == "json")
                .unwrap_or(false)
            {
                if let Ok(meta) = entry.metadata() {
                    total_chars += meta.len() as usize;
                }
            }
        }
    }

    estimate_tokens(&"x".repeat(total_chars))
}

// ─── Record boot ────────────────────────────────────────────────────────────

/// Record this boot so the next session's delta knows when we last connected.
fn record_boot(conn: &Connection, agent: &str) {
    let now = chrono::Utc::now()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let _ = conn.execute(
        "INSERT INTO events (type, data, source_agent) VALUES (?1, ?2, ?3)",
        params![
            "agent_boot",
            serde_json::to_string(&json!({ "timestamp": &now, "agent": agent }))
                .unwrap_or_default(),
            agent
        ],
    );
}

// ─── Public API ─────────────────────────────────────────────────────────────

/// Compile the boot prompt for an agent within a token budget.
///
/// Pipeline:
///  1. Build identity capsule (stable, ~200 tokens)
///  2. Build delta capsule (what's changed, ~300 tokens)
///  3. Record this boot for next session's delta
///  4. Assemble within budget, truncating delta if needed
///  5. Return prompt with capsule metadata and savings
pub fn compile(conn: &Connection, home: &Path, agent: &str, max_tokens: usize) -> BootResult {
    // 1. Build capsules
    let (identity_text, identity_tokens) = build_identity_capsule(conn);
    let (delta_text, delta_tokens, delta_freshness) =
        build_delta_capsule(conn, home, agent);

    // 2. Record this boot for next session
    record_boot(conn, agent);

    // 3. Assemble: identity first, then delta
    let identity_section = format!("## Identity\n{identity_text}");
    let delta_section = format!("## Delta\n{delta_text}");

    let mut assembled = identity_section;
    let mut capsules = vec![json!({
        "name": "identity",
        "tokens": identity_tokens,
        "freshness": "stable",
        "truncated": false
    })];

    let combined = format!("{assembled}\n\n{delta_section}");
    if estimate_tokens(&combined) <= max_tokens {
        // Both fit within budget
        assembled = combined;
        capsules.push(json!({
            "name": "delta",
            "tokens": delta_tokens,
            "freshness": delta_freshness,
            "truncated": false
        }));
    } else {
        // Truncate delta to fit budget
        let remaining = max_tokens
            .saturating_sub(estimate_tokens(&assembled))
            .saturating_sub(10); // 10 tokens for header overhead
        if remaining > 50 && !delta_text.is_empty() {
            let trunc_chars = (remaining as f64 * 3.8) as usize;
            let truncated: String = delta_text.chars().take(trunc_chars).collect();
            assembled = format!("{assembled}\n\n## Delta\n{truncated}...");
            capsules.push(json!({
                "name": "delta",
                "tokens": estimate_tokens(&truncated),
                "freshness": delta_freshness,
                "truncated": true
            }));
        }
    }

    // 4. Savings
    let token_estimate = estimate_tokens(&assembled);
    let raw_baseline = estimate_raw_baseline(home);
    let saved = raw_baseline.saturating_sub(token_estimate);
    let percent = if raw_baseline > 0 {
        (saved * 100) / raw_baseline
    } else {
        0
    };

    // Log savings event
    let _ = conn.execute(
        "INSERT INTO events (type, data, source_agent) VALUES (?1, ?2, ?3)",
        params![
            "boot_savings",
            serde_json::to_string(&json!({
                "agent": agent,
                "served": token_estimate,
                "baseline": raw_baseline,
                "saved": saved,
                "percent": percent
            }))
            .unwrap_or_default(),
            "rust-daemon"
        ],
    );

    BootResult {
        boot_prompt: assembled,
        token_estimate,
        savings: json!({
            "rawBaseline": raw_baseline,
            "served": token_estimate,
            "saved": saved,
            "percent": percent
        }),
        capsules,
    }
}

// ─── Memory directory discovery (used by raw baseline) ──────────────────────

/// Find the memory directory.  Looks for `~/.claude/projects/*/memory/` dirs.
/// Returns the first found, preferring `C--Users-aditya` on this machine.
pub fn find_memory_dir(home: &Path) -> Option<PathBuf> {
    let projects_dir = home.join(".claude").join("projects");

    // Prefer the known path on this machine
    let preferred = projects_dir.join("C--Users-aditya").join("memory");
    if preferred.is_dir() {
        return Some(preferred);
    }

    // Fallback: scan for any project with a memory dir
    if let Ok(entries) = std::fs::read_dir(&projects_dir) {
        for entry in entries.flatten() {
            let mem = entry.path().join("memory");
            if mem.is_dir() {
                return Some(mem);
            }
        }
    }

    None
}

/// Read all .md files from a memory directory, returning (filename, content) pairs.
pub fn read_memory_files(dir: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|x| x == "md").unwrap_or(false) {
                let name = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                if let Ok(content) = std::fs::read_to_string(&path) {
                    out.push((name, content));
                }
            }
        }
    }
    out
}

/// Read lesson files from the self-improvement-engine/lessons directory.
pub fn read_lessons(dir: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let ext_ok = path
                .extension()
                .map(|x| x == "md" || x == "json")
                .unwrap_or(false);
            if ext_ok {
                let name = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                if let Ok(content) = std::fs::read_to_string(&path) {
                    out.push((name, content));
                }
            }
        }
    }
    out
}
