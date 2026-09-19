use super::*;
use crate::db::ACTIVE_TEMPORAL_SQL;
use crate::handlers::estimate_tokens;
use regex::Regex;
use rusqlite::{Connection, params};
use std::sync::OnceLock;
pub fn content_hash(data: &str) -> String {
    cortex_logic::traces::content_hash(data)
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

fn take_matching(rows: &[(i64, String)], re: &Regex, n: usize, chars: usize) -> Vec<String> {
    rows.iter()
        .map(|(_, text)| text.as_str())
        .filter(|t| re.is_match(t))
        .take(n)
        .map(|t| t.chars().take(chars).collect())
        .collect()
}
pub fn cache_get(conn: &Connection, key: &str, expected_hash: &str) -> Option<(String, usize)> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT compressed, tokens, content_hash FROM context_cache WHERE cache_key = ?1",
        )
        .ok()?;
    stmt.query_row(params![key], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)? as usize,
            row.get::<_, String>(2)?,
        ))
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
    if let Ok(mut stmt) = conn.prepare_cached("INSERT OR REPLACE INTO context_cache (cache_key, content_hash, compressed, tokens) VALUES (?1, ?2, ?3, ?4)") { let _ = stmt.execute(params![key, hash, compressed, tokens as i64]); }
}
fn identity_feedback_texts(conn: &Connection) -> Result<Vec<(i64, String)>, String> {
    let mem_scope = super::owner_clause(conn, "memories", super::boot_owner());
    let mut stmt = conn.prepare_cached(&format!("SELECT id, text FROM memories WHERE type = 'feedback' AND {ACTIVE_TEMPORAL_SQL} {mem_scope} ORDER BY score DESC, id ASC LIMIT 20")).map_err(|err| err.to_string())?;
    let rows: Vec<(i64, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|err| err.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|err| err.to_string())?;
    let ids: Vec<i64> = rows.iter().map(|row| row.0).collect();
    let allow = super::capsules::boot_scope_allowlist(conn, "memory", &ids)?;
    Ok(rows
        .into_iter()
        .filter(|(id, _)| super::capsules::keep_boot_id(&allow, *id))
        .collect())
}

fn identity_unavailable() -> (String, usize) {
    let text = format!(
        "{}\nRules: [unavailable] identity feedback could not be loaded; do not assume this set is empty",
        detect_identity()
    );
    let tokens = estimate_tokens(&text);
    (text, tokens)
}

pub fn build_identity_capsule(conn: &Connection) -> (String, usize) {
    // Cache key over exactly the rows the capsule renders (top 20 by score,
    // then path-filtered), not O(all feedback bytes) per boot. Paths belong
    // in the key so a scoped boot cannot reuse an unscoped identity cache.
    let rows = match identity_feedback_texts(conn) {
        Ok(rows) => rows,
        // A failed SELECT/allowlist is not "no identity rules". Do not cache
        // a platform-only capsule under the empty digest — that would look
        // like a complete identity until the next distinct row set.
        Err(_) => return identity_unavailable(),
    };
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
    let constraints = take_matching(&rows, identity_constraint_re(), 5, 120);
    if !constraints.is_empty() {
        parts.push(format!("Rules: {}", constraints.join(" | ")));
    }
    let edges = take_matching(&rows, identity_edge_re(), 3, 100);
    if !edges.is_empty() {
        parts.push(format!("Sharp edges: {}", edges.join(" | ")));
    }
    let text = parts.join("\n");
    let tokens = estimate_tokens(&text);
    cache_set(conn, "identity_capsule", &feedback_hash, &text, tokens);
    (text, tokens)
}
