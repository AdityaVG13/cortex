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
use std::path::Path;

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

// ─── Content-addressed cache ────────────────────────────────────────────────

/// Compute a fast content hash for cache invalidation.
/// Uses FNV-1a for speed (not crypto-secure, just change detection).
fn content_hash(data: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in data.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// Check the context cache for a cached result.
fn cache_get(conn: &Connection, key: &str, expected_hash: &str) -> Option<(String, usize)> {
    conn.query_row(
        "SELECT compressed, tokens, content_hash FROM context_cache WHERE cache_key = ?1",
        params![key],
        |row| {
            let compressed: String = row.get(0)?;
            let tokens: usize = row.get::<_, i64>(1)? as usize;
            let stored_hash: String = row.get(2)?;
            Ok((compressed, tokens, stored_hash))
        },
    )
    .ok()
    .and_then(|(compressed, tokens, stored_hash)| {
        if stored_hash == expected_hash {
            // Cache hit -- bump hit count
            let _ = conn.execute(
                "UPDATE context_cache SET hits = hits + 1 WHERE cache_key = ?1",
                params![key],
            );
            Some((compressed, tokens))
        } else {
            None // Hash mismatch -- content changed
        }
    })
}

/// Store a compiled result in the cache.
fn cache_set(conn: &Connection, key: &str, hash: &str, compressed: &str, tokens: usize) {
    let _ = conn.execute(
        "INSERT OR REPLACE INTO context_cache (cache_key, content_hash, compressed, tokens) \
         VALUES (?1, ?2, ?3, ?4)",
        params![key, hash, compressed, tokens as i64],
    );
}

// State.md helpers removed — session-auto-restore.js handles state.md injection.

// ─── Identity capsule ───────────────────────────────────────────────────────

/// Build the identity capsule — stable across sessions, ~200 tokens.
/// Contains core user identity, hard constraints, and platform sharp edges.
/// Uses content-addressed cache: if feedback memories haven't changed, reuse.
fn build_identity_capsule(conn: &Connection) -> (String, usize) {
    // Compute hash of the feedback memories that feed this capsule
    let feedback_hash = {
        let mut all_feedback = String::new();
        if let Ok(mut stmt) = conn.prepare(
            "SELECT text FROM memories WHERE type = 'feedback' AND status = 'active' ORDER BY id",
        ) {
            if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
                for text in rows.flatten() {
                    all_feedback.push_str(&text);
                    all_feedback.push('\n');
                }
            }
        }
        content_hash(&all_feedback)
    };

    // Check cache
    if let Some((cached, tokens)) = cache_get(conn, "identity_capsule", &feedback_hash) {
        return (cached, tokens);
    }
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

    // Cache the result for next boot
    cache_set(conn, "identity_capsule", &feedback_hash, &text, tokens);

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
fn build_delta_capsule(conn: &Connection, agent: &str) -> (String, usize, String) {
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

    // 2. Active focus session (sawtooth pattern indicator)
    if let Some(focus) = crate::focus::focus_current(conn, agent) {
        let label = focus.get("label").and_then(|v| v.as_str()).unwrap_or("?");
        let entries = focus.get("entries").and_then(|v| v.as_u64()).unwrap_or(0);
        parts.push(format!("## Active Focus\n- {label} ({entries} entries)"));
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

// ─── Context Item for ranked compilation ───────────────────────────────────

struct ContextItem {
    name: String,
    text: String,
    tokens: usize,
    /// Base priority: 1.0 = must-have, 0.5 = important, 0.2 = nice-to-have
    priority: f64,
    /// Utility score: priority / token_cost (higher = more efficient)
    utility: f64,
}

impl ContextItem {
    fn new(name: &str, text: String, priority: f64) -> Self {
        let tokens = estimate_tokens(&text);
        let utility = if tokens > 0 {
            priority / (tokens as f64)
        } else {
            0.0
        };
        Self {
            name: name.to_string(),
            text,
            tokens,
            priority,
            utility,
        }
    }
}

/// Compile the boot prompt for an agent within a token budget.
///
/// Prompt Compiler Pipeline (v2 -- ranked context packing):
///  1. Gather all context items with priority scores
///  2. Sort by utility (priority / token_cost) -- best bang-per-token first
///  3. Greedily pack within budget
///  4. Record admitted vs rejected for observability
///  5. Return prompt with compilation metadata and savings
pub fn compile(conn: &Connection, home: &Path, agent: &str, max_tokens: usize) -> BootResult {
    // ── 1. Gather context items with priorities ─────────────────────────────

    let mut items: Vec<ContextItem> = Vec::new();

    // Identity capsule: must-have (priority 1.0)
    let (identity_text, _) = build_identity_capsule(conn);
    if !identity_text.is_empty() {
        items.push(ContextItem::new("identity", format!("## Identity\n{identity_text}"), 1.0));
    }

    // Delta capsule: broken into sub-items with individual priorities
    let (delta_text, _, _delta_freshness) = build_delta_capsule(conn, agent);
    if !delta_text.is_empty() {
        // Split delta into sections, each scored independently
        let sections: Vec<(&str, f64)> = vec![
            ("## Pending Messages", 0.95),   // Messages from other agents = urgent
            ("## Active Agents", 0.60),       // Who's online = coordination
            ("## Active Locks", 0.70),        // Locks = collision prevention
            ("## Feed", 0.40),                // Feed = nice context
            ("## Pending Tasks", 0.75),       // Task board = actionable
            ("## Your Active Tasks", 0.80),   // Your tasks = high priority
            ("CONFLICTS:", 0.90),             // Conflicts = must resolve
            ("## Active Focus", 0.85),        // Focus scope = context boundary
            ("New decisions:", 0.55),         // Recent decisions = orientation
            ("New knowledge:", 0.45),         // New memories
            ("Activity since last boot:", 0.30), // Activity summary = low value
            ("Recent decisions:", 0.50),      // First-boot orientation
        ];

        // Try to split delta into scored sub-sections
        let remaining_delta = delta_text.as_str();
        let mut matched_any = false;

        for (header, priority) in &sections {
            if let Some(start) = remaining_delta.find(header) {
                // Find end: next section header or end of string
                let content_start = start;
                let after_header = start + header.len();
                let end = remaining_delta[after_header..]
                    .find("\n\n")
                    .map(|p| after_header + p)
                    .unwrap_or(remaining_delta.len());

                let section_text = remaining_delta[content_start..end].trim().to_string();
                if !section_text.is_empty() {
                    items.push(ContextItem::new(header, section_text, *priority));
                    matched_any = true;
                }
            }
        }

        // Fallback: if no sections matched, treat delta as one block
        if !matched_any {
            items.push(ContextItem::new("delta", format!("## Delta\n{delta_text}"), 0.70));
        }
    }

    // ── 2. Record boot ──────────────────────────────────────────────────────
    record_boot(conn, agent);

    // ── 3. Sort by utility (priority / token_cost) descending ───────────────
    items.sort_by(|a, b| b.utility.partial_cmp(&a.utility).unwrap_or(std::cmp::Ordering::Equal));

    // ── 4. Greedy budget packing ────────────────────────────────────────────
    let mut budget_remaining = max_tokens;
    let mut admitted: Vec<Value> = Vec::new();
    let mut rejected: Vec<Value> = Vec::new();
    let mut assembled_parts: Vec<String> = Vec::new();

    for item in &items {
        if item.tokens <= budget_remaining && !item.text.is_empty() {
            assembled_parts.push(item.text.clone());
            budget_remaining -= item.tokens;
            admitted.push(json!({
                "name": item.name,
                "tokens": item.tokens,
                "priority": item.priority,
                "utility": (item.utility * 10000.0).round() / 10000.0
            }));
        } else if !item.text.is_empty() {
            // Try truncation for high-priority items
            if item.priority >= 0.7 && budget_remaining > 30 {
                let trunc_chars = (budget_remaining as f64 * 3.5) as usize;
                let truncated: String = item.text.chars().take(trunc_chars).collect();
                let trunc_tokens = estimate_tokens(&truncated);
                assembled_parts.push(format!("{truncated}..."));
                budget_remaining = budget_remaining.saturating_sub(trunc_tokens);
                admitted.push(json!({
                    "name": item.name,
                    "tokens": trunc_tokens,
                    "priority": item.priority,
                    "truncated": true
                }));
            } else {
                rejected.push(json!({
                    "name": item.name,
                    "tokens": item.tokens,
                    "priority": item.priority,
                    "reason": "budget_exceeded"
                }));
            }
        }
    }

    let assembled = assembled_parts.join("\n\n");
    let token_estimate = estimate_tokens(&assembled);

    // ── 5. Savings and observability ────────────────────────────────────────
    let raw_baseline = estimate_raw_baseline(home);
    let saved = raw_baseline.saturating_sub(token_estimate);
    let percent = if raw_baseline > 0 { (saved * 100) / raw_baseline } else { 0 };

    let _ = conn.execute(
        "INSERT INTO events (type, data, source_agent) VALUES (?1, ?2, ?3)",
        params![
            "boot_savings",
            serde_json::to_string(&json!({
                "agent": agent,
                "served": token_estimate,
                "baseline": raw_baseline,
                "saved": saved,
                "percent": percent,
                "admitted": admitted.len(),
                "rejected": rejected.len()
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
        capsules: admitted,
    }
}

// Dead code removed: find_memory_dir, read_memory_files, read_lessons
// (indexer.rs has its own implementation; these were ported but unused)
