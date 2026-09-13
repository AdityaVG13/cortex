use super::*;
use crate::handlers::estimate_tokens;
use regex::Regex;
use rusqlite::{params, Connection};
use std::sync::OnceLock;
pub fn content_hash(data: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in data.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
fn identity_constraint_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b(never|always|must|do not|don't|required|mandatory)\b")
            .expect("static identity constraint regex")
    })
}
fn identity_edge_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b(windows|win32|encoding|cp1252|bash\.exe|CRLF)\b")
            .expect("static identity edge regex")
    })
}
pub fn cache_get(conn: &Connection, key: &str, expected_hash: &str) -> Option<(String, usize)> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT compressed, tokens, content_hash FROM context_cache WHERE cache_key = ?1",
        )
        .ok()?;
    stmt.query_row(params![key], |row| {
        let compressed: String = row.get(0)?;
        let tokens: usize = row.get::<_, i64>(1)? as usize;
        let stored_hash: String = row.get(2)?;
        Ok((compressed, tokens, stored_hash))
    })
    .ok()
    .and_then(|(compressed, tokens, stored_hash)| {
        if stored_hash == expected_hash {
            if let Ok(mut update) =
                conn.prepare_cached("UPDATE context_cache SET hits = hits + 1 WHERE cache_key = ?1")
            {
                let _ = update.execute(params![key]);
            }
            Some((compressed, tokens))
        } else {
            None
        }
    })
}
pub fn cache_set(conn: &Connection, key: &str, hash: &str, compressed: &str, tokens: usize) {
    if let Ok(mut stmt) = conn.prepare_cached(
        "INSERT OR REPLACE INTO context_cache (cache_key, content_hash, compressed, tokens) \
         VALUES (?1, ?2, ?3, ?4)",
    ) {
        let _ = stmt.execute(params![key, hash, compressed, tokens as i64]);
    }
}
fn identity_feedback_texts(conn: &Connection) -> Vec<(i64, String)> {
    let mem_scope = super::owner_clause(conn, "memories", super::boot_owner());
    let Ok(mut stmt) = conn.prepare_cached(&format!(
        "SELECT id, text FROM memories WHERE type = 'feedback' AND status = 'active' \
         AND (expires_at IS NULL OR TRIM(expires_at) = '' OR julianday(expires_at) > julianday('now')) \
         AND (valid_from IS NULL OR TRIM(valid_from) = '' OR julianday(valid_from) <= julianday('now')) \
         AND (valid_until IS NULL OR TRIM(valid_until) = '' OR julianday(valid_until) > julianday('now')){mem_scope} \
         ORDER BY score DESC, id ASC LIMIT 20"
    )) else {
        return Vec::new();
    };
    let rows: Vec<(i64, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default();
    let ids: Vec<i64> = rows.iter().map(|row| row.0).collect();
    let allow = match super::capsules::boot_scope_allowlist(conn, "memory", &ids) {
        Ok(allow) => allow,
        Err(_) => return Vec::new(),
    };
    rows.into_iter()
        .filter(|(id, _)| super::capsules::keep_boot_id(&allow, *id))
        .collect()
}

pub fn build_identity_capsule(conn: &Connection) -> (String, usize) {
    // Cache key over exactly the rows the capsule renders (top 20 by score,
    // then path-filtered), not O(all feedback bytes) per boot. Paths belong
    // in the key so a scoped boot cannot reuse an unscoped identity cache.
    let rows = identity_feedback_texts(conn);
    let owner_tag = super::boot_owner()
        .map(|id| id.to_string())
        .unwrap_or_default();
    let path_tag = super::with_boot_paths(|paths| paths.join("\u{1f}"));
    let digest: String = rows
        .iter()
        .map(|(id, text)| format!("{id}:{}", text.len()))
        .collect::<Vec<_>>()
        .join(",");
    let feedback_hash = content_hash(&format!("{digest}|{owner_tag}|{path_tag}"));
    if let Some((cached, tokens)) = cache_get(conn, "identity_capsule", &feedback_hash) {
        return (cached, tokens);
    }
    let mut parts = vec![detect_identity()];
    let constraint_re = identity_constraint_re();
    let constraints: Vec<String> = rows
        .iter()
        .map(|(_, text)| text.as_str())
        .filter(|t| constraint_re.is_match(t))
        .take(5)
        .map(|t| t.chars().take(120).collect::<String>())
        .collect();
    if !constraints.is_empty() {
        parts.push(format!("Rules: {}", constraints.join(" | ")));
    }
    let edge_re = identity_edge_re();
    let edges: Vec<String> = rows
        .iter()
        .map(|(_, text)| text.as_str())
        .filter(|t| edge_re.is_match(t))
        .take(3)
        .map(|t| t.chars().take(100).collect::<String>())
        .collect();
    if !edges.is_empty() {
        parts.push(format!("Sharp edges: {}", edges.join(" | ")));
    }
    let text = parts.join("\n");
    let tokens = estimate_tokens(&text);
    cache_set(conn, "identity_capsule", &feedback_hash, &text, tokens);
    (text, tokens)
}
