//! Knowledge indexer: reads filesystem sources and upserts into memories table.
//! Ported from Node.js brain.js indexAll().

use rusqlite::Connection;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

const STATE_SECTIONS: &[&str] = &[
    "## What Was Done",
    "## Next Session",
    "## Pending",
    "## Known Issues",
];

/// Run all 6 indexers. Returns total entries indexed.
pub fn index_all(conn: &Connection, home: &Path) -> usize {
    let mut total = 0;
    total += index_state_file(conn, home);
    total += index_memory_files(conn, home);
    total += index_lessons(conn, home);
    total += index_goals(conn, home);
    total += index_skill_tracker(conn, home);
    total += index_gorci(conn, home);
    total
}

/// Upsert a memory by source. If source exists, update text. Otherwise insert.
fn upsert_memory(conn: &Connection, text: &str, source: &str, mem_type: &str, agent: &str) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }

    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM memories WHERE source = ? AND status = 'active'",
            [source],
            |row| row.get(0),
        )
        .ok();

    if let Some(id) = existing {
        let _ = conn.execute(
            "UPDATE memories SET text = ?, updated_at = datetime('now') WHERE id = ?",
            rusqlite::params![text, id],
        );
        // Invalidate stale embedding so background builder re-computes it.
        let _ = conn.execute(
            "DELETE FROM embeddings WHERE target_type = 'memory' AND target_id = ?",
            [id],
        );
    } else {
        let _ = conn.execute(
            "INSERT INTO memories (text, source, type, source_agent) VALUES (?, ?, ?, ?)",
            rusqlite::params![text, source, mem_type, agent],
        );
    }

    true
}

// ── Source 1: state.md ──────────────────────────────────────────────────────

fn index_state_file(conn: &Connection, home: &Path) -> usize {
    let state_path = home.join(".claude").join("state.md");
    if !state_path.exists() {
        return 0;
    }

    let content = match fs::read_to_string(&state_path) {
        Ok(c) => c,
        Err(_) => return 0,
    };

    let mut count = 0;
    for section in STATE_SECTIONS {
        if let Some(text) = extract_section(&content, section) {
            let source = format!("state.md::{}", section.trim_start_matches("## "));
            if upsert_memory(conn, &text, &source, "state", "indexer") {
                count += 1;
            }
        }
    }
    count
}

fn extract_section(markdown: &str, header: &str) -> Option<String> {
    let idx = markdown.find(header)?;
    let start = idx + header.len();
    let rest = &markdown[start..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    let text = rest[..end].trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

// ── Source 2: Memory files ──────────────────────────────────────────────────

fn index_memory_files(conn: &Connection, home: &Path) -> usize {
    let mem_dir = home
        .join(".claude")
        .join("projects")
        .join("C--Users-aditya")
        .join("memory");
    if !mem_dir.exists() {
        return 0;
    }

    let mut count = 0;
    let entries = match fs::read_dir(&mem_dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if path.file_name().and_then(|f| f.to_str()) == Some("MEMORY.md") {
            continue;
        }

        if let Ok(raw) = fs::read_to_string(&path) {
            let (fm, body) = parse_frontmatter(&raw);
            let name = fm.get("name").cloned().unwrap_or_else(|| {
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            });
            let mem_type = fm
                .get("type")
                .cloned()
                .unwrap_or_else(|| "memory".to_string());
            let desc = fm.get("description").cloned().unwrap_or_default();

            let body_preview: String = body.chars().take(500).collect();
            let text = if !desc.is_empty() {
                format!("[{name}] ({mem_type}) {desc}\n{body_preview}")
            } else {
                format!("[{name}] ({mem_type})\n{body_preview}")
            };

            let source = format!(
                "memory::{}",
                path.file_name().unwrap_or_default().to_string_lossy()
            );
            if upsert_memory(conn, &text, &source, &mem_type, "indexer") {
                count += 1;
            }
        }
    }
    count
}

fn parse_frontmatter(raw: &str) -> (HashMap<String, String>, String) {
    let mut fm = HashMap::new();
    let body;

    if raw.starts_with("---") {
        if let Some(end) = raw[3..].find("---") {
            let yaml_block = &raw[3..3 + end];
            body = raw[3 + end + 3..].trim().to_string();

            for line in yaml_block.lines() {
                if let Some(colon) = line.find(':') {
                    let key = line[..colon].trim().to_string();
                    let val = line[colon + 1..].trim().to_string();
                    fm.insert(key, val);
                }
            }
        } else {
            body = raw.to_string();
        }
    } else {
        body = raw.to_string();
    }

    (fm, body)
}

// ── Source 3: Lessons ───────────────────────────────────────────────────────

fn index_lessons(conn: &Connection, home: &Path) -> usize {
    let path = home
        .join("self-improvement-engine")
        .join("lessons")
        .join("lessons.jsonl");
    if !path.exists() {
        return 0;
    }

    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return 0,
    };

    let mut count = 0;
    for line in content.lines().filter(|l| !l.is_empty()) {
        if let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) {
            let lesson_type = entry["type"].as_str().unwrap_or("lesson");
            let lesson = entry["lesson"].as_str().unwrap_or("");
            let evidence = entry["evidence"].as_str().unwrap_or("");
            let skill = entry["skill"].as_str().unwrap_or("general");
            let ts = entry["timestamp"].as_str().unwrap_or("unknown");

            let text = if !evidence.is_empty() {
                format!("[{lesson_type}] {lesson} -- Evidence: {evidence}")
            } else {
                format!("[{lesson_type}] {lesson}")
            };

            let source = format!("lessons::{skill}::{ts}");
            if upsert_memory(conn, &text, &source, "lesson", "indexer") {
                count += 1;
            }
        }
    }
    count
}

// ── Source 4: Goals ─────────────────────────────────────────────────────────

fn index_goals(conn: &Connection, home: &Path) -> usize {
    let path = home
        .join("self-improvement-engine")
        .join("tools")
        .join("goal-setter")
        .join("current-goals.json");
    if !path.exists() {
        return 0;
    }

    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    let data: serde_json::Value = match serde_json::from_str(&content) {
        Ok(d) => d,
        Err(_) => return 0,
    };

    let mut count = 0;
    if let Some(goals) = data["goals"].as_array() {
        for goal in goals {
            let rank = goal["rank"].as_u64().unwrap_or(0);
            let text_val = goal["goal"].as_str().unwrap_or("");
            let cat = goal["category"].as_str().unwrap_or("unknown");
            let priority = goal["priority"]
                .as_f64()
                .map(|p| format!("{p:.2}"))
                .unwrap_or_else(|| "?".to_string());
            let effort = goal["effort"].as_str().unwrap_or("?");

            let text = format!(
                "[Goal #{rank}] {text_val} (category: {cat}, priority: {priority}, effort: {effort})"
            );
            let source = format!("goals::rank{rank}");
            if upsert_memory(conn, &text, &source, "goal", "indexer") {
                count += 1;
            }
        }
    }
    count
}

// ── Source 5: Skill tracker ─────────────────────────────────────────────────

fn index_skill_tracker(conn: &Connection, home: &Path) -> usize {
    let path = home
        .join("self-improvement-engine")
        .join("tools")
        .join("skill-tracker")
        .join("invocations.jsonl");
    if !path.exists() {
        return 0;
    }

    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return 0,
    };

    // Aggregate by skill.
    let mut by_skill: HashMap<String, (u32, u32, u32, u32, String)> = HashMap::new();

    for line in content.lines().filter(|l| !l.is_empty()) {
        if let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) {
            let skill = entry["skill"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            let outcome = entry["outcome"].as_str().unwrap_or("");
            let ts = entry["timestamp"].as_str().unwrap_or("").to_string();

            let stats = by_skill
                .entry(skill)
                .or_insert((0, 0, 0, 0, String::new()));
            stats.0 += 1; // total
            match outcome {
                "success" => stats.1 += 1,
                "correction" => stats.2 += 1,
                "retry" => stats.3 += 1,
                _ => {}
            }
            if ts > stats.4 {
                stats.4 = ts;
            }
        }
    }

    let mut count = 0;
    for (skill, (total, success, correction, retry, last)) in &by_skill {
        let rate = if *total > 0 {
            (*success as f64 / *total as f64 * 100.0) as u32
        } else {
            0
        };
        let text = format!(
            "[Skill: {skill}] {total} invocations, {rate}% success ({correction} corrections, {retry} retries). Last: {last}"
        );
        let source = format!("skills::{skill}");
        if upsert_memory(conn, &text, &source, "skill_stats", "indexer") {
            count += 1;
        }
    }
    count
}

// ── Source 6: GORCI ─────────────────────────────────────────────────────────

fn index_gorci(conn: &Connection, home: &Path) -> usize {
    let path = home
        .join("self-improvement-engine")
        .join("tools")
        .join("gorci")
        .join("last-run.json");
    if !path.exists() {
        return 0;
    }

    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    let data: serde_json::Value = match serde_json::from_str(&content) {
        Ok(d) => d,
        Err(_) => return 0,
    };

    let text = format!(
        "[GORCI] Skill: {}, Mode: {}, Tier: {}, Cases: {}, Pass: {}, Score: {}. Run: {}",
        data["skill"].as_str().unwrap_or("unknown"),
        data["mode"].as_str().unwrap_or("?"),
        data["tier"].as_str().unwrap_or("?"),
        data["cases"].as_u64().unwrap_or(0),
        data["pass"]
            .as_str()
            .or_else(|| data["pass"].as_u64().map(|_| "?"))
            .unwrap_or("?"),
        data["overallScore"]
            .as_str()
            .or_else(|| data["overallScore"].as_f64().map(|_| "?"))
            .unwrap_or("?"),
        data["timestamp"].as_str().unwrap_or("unknown"),
    );

    if upsert_memory(conn, &text, "gorci::last-run", "gorci", "indexer") {
        1
    } else {
        0
    }
}

// ── Score decay ─────────────────────────────────────────────────────────────

/// Apply 0.95^days score decay to all active entries.
/// Entries with score already at floor (0.1) are skipped.
pub fn decay_pass(conn: &Connection) -> usize {
    let result = conn.execute(
        "UPDATE memories SET score = MAX(0.1, score * POWER(0.95,
            CAST((julianday('now') - julianday(COALESCE(updated_at, created_at))) AS REAL)))
         WHERE status = 'active' AND score > 0.1
           AND (julianday('now') - julianday(COALESCE(updated_at, created_at))) > 1",
        [],
    );
    result.unwrap_or(0)
}
