use super::*;
use crate::handlers::estimate_tokens;
use regex::Regex;
use rusqlite::{params, Connection};
pub(crate) fn content_hash(data: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in data.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
pub(crate) fn cache_get(conn: &Connection, key: &str, expected_hash: &str) -> Option<(String, usize)> {
    conn.query_row("SELECT compressed, tokens, content_hash FROM context_cache WHERE cache_key = ?1", params![key], |row| {
        let compressed: String = row.get(0)?;
        let tokens: usize = row.get::<_, i64>(1)? as usize;
        let stored_hash: String = row.get(2)?;
        Ok((compressed, tokens, stored_hash))
    })
    .ok()
    .and_then(|(compressed, tokens, stored_hash)| {
        if stored_hash == expected_hash {
            let _ = conn.execute("UPDATE context_cache SET hits = hits + 1 WHERE cache_key = ?1", params![key]);
            Some((compressed, tokens))
        } else {
            None
        }
    })
}
pub(crate) fn cache_set(conn: &Connection, key: &str, hash: &str, compressed: &str, tokens: usize) {
    let _ = conn.execute(
        "INSERT OR REPLACE INTO context_cache (cache_key, content_hash, compressed, tokens) \
         VALUES (?1, ?2, ?3, ?4)",
        params![key, hash, compressed, tokens as i64],
    );
}
pub(crate) fn build_identity_capsule(conn: &Connection) -> (String, usize) {
    let feedback_hash = {
        let mut all_feedback = String::new();
        if let Ok(mut stmt) = conn.prepare("SELECT text FROM memories WHERE type = 'feedback' AND status = 'active' ORDER BY id") {
            if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
                for text in rows.flatten() {
                    all_feedback.push_str(&text);
                    all_feedback.push('\n');
                }
            }
        }
        content_hash(&all_feedback)
    };
    if let Some((cached, tokens)) = cache_get(conn, "identity_capsule", &feedback_hash) {
        return (cached, tokens);
    }
    let mut parts = vec![detect_identity()];
    if let Ok(constraint_re) = Regex::new(r"(?i)\b(never|always|must|do not|don't|required|mandatory)\b") {
        if let Ok(mut stmt) =
            conn.prepare("SELECT text FROM memories WHERE type = 'feedback' AND status = 'active' ORDER BY score DESC LIMIT 20")
        {
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
    if let Ok(edge_re) = Regex::new(r"(?i)\b(windows|win32|encoding|cp1252|bash\.exe|CRLF)\b") {
        if let Ok(mut stmt) =
            conn.prepare("SELECT text FROM memories WHERE type = 'feedback' AND status = 'active' ORDER BY score DESC LIMIT 20")
        {
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
    cache_set(conn, "identity_capsule", &feedback_hash, &text, tokens);
    (text, tokens)
}
