use crate::state::{BrainFiringEvent, BrainKind};
use asupersync::{Cx, channel::broadcast};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

pub type BrainFiringSender = Option<broadcast::Sender<BrainFiringEvent>>;
pub const CRYSTAL_RELEVANCE_BOOST: f64 = 1.15;

const MAX_SCAN_ROWS: i64 = 500;
const JACCARD_THRESHOLD: f64 = 0.30;
/// `last_accessed` / `created_at` mix RFC3339 (`T`) and SQLite `datetime('now')`
/// (space). Lexicographic ORDER BY ranks the formats, not the instants.
const CANDIDATE_RECENCY: &str = "julianday(COALESCE(NULLIF(TRIM(last_accessed), ''), NULLIF(TRIM(created_at), ''))) DESC, id DESC";

const STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "in", "on", "at", "to", "for", "of", "with", "by", "is",
    "are", "was", "were", "be", "been", "has", "have", "had", "it", "this", "that", "these",
    "those", "as", "from", "will", "would", "can", "could", "should", "do", "does", "did", "so",
    "if", "then", "than", "when", "what", "which", "who", "how", "why", "not", "no", "yes", "we",
    "you", "they", "our", "your", "their", "its", "my", "me", "i", "he", "she", "them", "us",
];

#[derive(Debug)]
pub struct CrystallizeResult {
    pub clusters_found: usize,
    pub crystals_created: usize,
    pub crystals_updated: usize,
    pub entries_consolidated: usize,
}

#[derive(Debug, Clone)]
struct Candidate {
    id: i64,
    text: String,
    score: f64,
    target_type: String,
    recency: Option<f64>,
}

fn fold_jaccard_token(token: &str) -> String {
    if token.bytes().all(|b| b.is_ascii()) {
        if token.bytes().any(|b| b.is_ascii_uppercase()) {
            token.to_ascii_lowercase()
        } else {
            token.to_owned()
        }
    } else {
        token.to_lowercase()
    }
}

fn jaccard_token_set(text: &str) -> HashSet<String> {
    let mut set = HashSet::new();
    for word in text.split_whitespace().filter(|w| w.len() > 1) {
        let token = fold_jaccard_token(word);
        if STOPWORDS.contains(&token.as_str()) {
            continue;
        }
        set.insert(token);
    }
    set
}

fn jaccard_similarity_sets(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let (smaller, larger) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let inter = smaller.iter().filter(|t| larger.contains(*t)).count() as f64;
    let uni = (a.len() + b.len()) as f64 - inter;
    if uni == 0.0 { 0.0 } else { inter / uni }
}

fn label_for_members(texts: &[String]) -> String {
    let mut freq: HashMap<String, usize> = HashMap::new();
    for t in texts {
        for word in t.split_whitespace().filter(|w| w.len() > 1) {
            let lower = fold_jaccard_token(word);
            let cleaned: String = lower.chars().filter(|c| c.is_alphanumeric()).collect();
            if cleaned.len() <= 1 {
                continue;
            }
            if STOPWORDS.contains(&cleaned.as_str()) {
                continue;
            }
            *freq.entry(cleaned).or_insert(0) += 1;
        }
    }
    let mut items: Vec<(String, usize)> = freq.into_iter().collect();
    items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    items.truncate(4);
    if items.is_empty() {
        let first = texts.first().map(|s| s.as_str()).unwrap_or("");
        let tokens: Vec<String> = first
            .split_whitespace()
            .filter(|w| w.len() > 1)
            .take(4)
            .map(fold_jaccard_token)
            .collect();
        if tokens.is_empty() {
            return "cluster".to_string();
        }
        return tokens.join(" ");
    }
    items
        .into_iter()
        .map(|(w, _)| w)
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn run_crystallize_pass_with_brain(
    cx: &Cx,
    conn: &Connection,
    owner_id: Option<i64>,
    brain: &BrainFiringSender,
) -> Result<CrystallizeResult, String> {
    cx.checkpoint().map_err(|e| e.to_string())?;
    let candidates = scan_candidates(conn, owner_id)?;
    if candidates.is_empty() {
        return Ok(CrystallizeResult {
            clusters_found: 0,
            crystals_created: 0,
            crystals_updated: 0,
            entries_consolidated: 0,
        });
    }

    let clusters: Vec<Vec<usize>> = cluster_by_jaccard(&candidates);

    let qualified: Vec<Vec<usize>> = clusters.into_iter().filter(|c| c.len() >= 2).collect();

    let mut crystals_created = 0usize;
    let mut entries_consolidated = 0usize;
    // One savepoint for the pass: a failed member INSERT must not leave a
    // cluster whose stored member_count disagrees with cluster_members.
    let sp = crate::db::SqliteSavepoint::enter(conn, "crystallize_pass")
        .map_err(|e| e.to_string())?;

    for member_indices in &qualified {
        let member_candidates: Vec<&Candidate> =
            member_indices.iter().map(|&i| &candidates[i]).collect();
        let texts: Vec<String> = member_candidates.iter().map(|c| c.text.clone()).collect();
        let label = label_for_members(&texts);
        let mut best = &member_candidates[0];
        for c in &member_candidates[1..] {
            if c.score > best.score || (c.score == best.score && c.id > best.id) {
                best = c;
            }
        }
        let consolidated_text = best.text.clone();
        let centroid_blob = Vec::<u8>::new();

        let member_count = member_indices.len() as i64;
        conn.execute(
            "INSERT INTO memory_clusters (label, centroid, consolidated_text, member_count, owner_id, visibility) VALUES (?1, ?2, ?3, ?4, ?5, 'private')",
            params![label, centroid_blob, consolidated_text, member_count, owner_id],
        )
        .map_err(|e| e.to_string())?;
        let cluster_id = conn.last_insert_rowid();
        let mut inserted = 0usize;
        for &idx in member_indices {
            let cand = &candidates[idx];
            conn.execute(
                "INSERT INTO cluster_members (cluster_id, source, target_type, target_id) VALUES (?1, ?2, ?3, ?4)",
                params![cluster_id, cand.text, cand.target_type, cand.id],
            )
            .map_err(|e| e.to_string())?;
            inserted += 1;
        }
        crystals_created += 1;
        entries_consolidated += inserted;
    }
    sp.release().map_err(|e| e.to_string())?;

    let result = CrystallizeResult {
        clusters_found: crystals_created,
        crystals_created,
        crystals_updated: 0,
        entries_consolidated,
    };

    if result.crystals_created > 0 || result.crystals_updated > 0 {
        if let Some(tx) = brain {
            let sent = tx.send(
                cx,
                BrainFiringEvent {
                    kind: BrainKind::ConsolidationStarted,
                    payload: json!({
                        "clusters_found": result.clusters_found,
                        "crystals_created": result.crystals_created,
                        "crystals_updated": result.crystals_updated,
                        "entries_consolidated": result.entries_consolidated,
                    }),
                    owner_id,
                },
            );
            // No subscribers is normal for embedded callers; cancellation is not.
            match sent {
                Ok(_) | Err(broadcast::SendError::Closed(_)) => {}
                Err(error) => return Err(error.to_string()),
            }
        }
    }

    Ok(result)
}

const CANDIDATE_SELECT_TAIL: &str = "julianday(COALESCE(NULLIF(TRIM(last_accessed), ''), NULLIF(TRIM(created_at), '')))";

fn candidate_from_row(row: &rusqlite::Row<'_>, target_type: &str) -> rusqlite::Result<Candidate> {
    Ok(Candidate {
        id: row.get(0)?,
        text: row.get(1)?,
        score: row.get(3)?,
        target_type: target_type.to_string(),
        recency: row.get(4)?,
    })
}

fn cap_candidates_by_recency(out: &mut Vec<Candidate>) {
    if out.len() as i64 <= MAX_SCAN_ROWS {
        return;
    }
    out.sort_by(|a, b| {
        b.recency
            .unwrap_or(f64::NEG_INFINITY)
            .total_cmp(&a.recency.unwrap_or(f64::NEG_INFINITY))
            .then_with(|| b.id.cmp(&a.id))
    });
    out.truncate(MAX_SCAN_ROWS as usize);
}

fn scan_candidates(conn: &Connection, owner_id: Option<i64>) -> Result<Vec<Candidate>, String> {
    if owner_id.is_none() {
        return scan_candidates_no_owner(conn);
    }
    let mut out: Vec<Candidate> = Vec::new();
    let mem_sql = format!(
        "SELECT id, text, COALESCE(source,''), COALESCE(score,1.0), {CANDIDATE_SELECT_TAIL} FROM memories WHERE status NOT IN ('superseded','archived') AND id NOT IN (SELECT target_id FROM cluster_members WHERE target_type='memory') AND (owner_id = ?1 OR owner_id IS NULL) ORDER BY {CANDIDATE_RECENCY} LIMIT ?2"
    );
    let mut stmt = conn.prepare(&mem_sql).map_err(|e| e.to_string())?;
    let mapped = stmt
        .query_map(params![owner_id, MAX_SCAN_ROWS], |row| {
            candidate_from_row(row, "memory")
        })
        .map_err(|e| e.to_string())?;
    for r in mapped {
        out.push(r.map_err(|e| e.to_string())?);
    }
    drop(stmt);
    let dec_sql = format!(
        "SELECT id, decision, COALESCE(context,''), COALESCE(score,1.0), {CANDIDATE_SELECT_TAIL} FROM decisions WHERE status NOT IN ('superseded','archived') AND id NOT IN (SELECT target_id FROM cluster_members WHERE target_type='decision') AND (owner_id = ?1 OR owner_id IS NULL) ORDER BY {CANDIDATE_RECENCY} LIMIT ?2"
    );
    let mut dstmt = conn.prepare(&dec_sql).map_err(|e| e.to_string())?;
    let mapped = dstmt
        .query_map(params![owner_id, MAX_SCAN_ROWS], |row| {
            candidate_from_row(row, "decision")
        })
        .map_err(|e| e.to_string())?;
    for r in mapped {
        out.push(r.map_err(|e| e.to_string())?);
    }
    cap_candidates_by_recency(&mut out);
    Ok(out)
}

fn scan_candidates_no_owner(conn: &Connection) -> Result<Vec<Candidate>, String> {
    let mut out: Vec<Candidate> = Vec::new();
    let mut stmt = conn
        .prepare(&format!(
            "SELECT id, text, COALESCE(source,''), COALESCE(score,1.0), {CANDIDATE_SELECT_TAIL} FROM memories WHERE status NOT IN ('superseded','archived') AND id NOT IN (SELECT target_id FROM cluster_members WHERE target_type='memory') ORDER BY {CANDIDATE_RECENCY} LIMIT ?1"
        ))
        .map_err(|e| e.to_string())?;
    let mapped = stmt
        .query_map(params![MAX_SCAN_ROWS], |row| candidate_from_row(row, "memory"))
        .map_err(|e| e.to_string())?;
    for r in mapped {
        out.push(r.map_err(|e| e.to_string())?);
    }
    drop(stmt);
    let mut stmt = conn
        .prepare(&format!(
            "SELECT id, decision, COALESCE(context,''), COALESCE(score,1.0), {CANDIDATE_SELECT_TAIL} FROM decisions WHERE status NOT IN ('superseded','archived') AND id NOT IN (SELECT target_id FROM cluster_members WHERE target_type='decision') ORDER BY {CANDIDATE_RECENCY} LIMIT ?1"
        ))
        .map_err(|e| e.to_string())?;
    let mapped = stmt
        .query_map(params![MAX_SCAN_ROWS], |row| {
            candidate_from_row(row, "decision")
        })
        .map_err(|e| e.to_string())?;
    for r in mapped {
        out.push(r.map_err(|e| e.to_string())?);
    }
    cap_candidates_by_recency(&mut out);
    Ok(out)
}

fn cluster_by_jaccard(candidates: &[Candidate]) -> Vec<Vec<usize>> {
    let token_sets: Vec<HashSet<String>> = candidates
        .iter()
        .map(|c| jaccard_token_set(&c.text))
        .collect();
    let mut clusters: Vec<Vec<usize>> = Vec::new();
    for (idx, set) in token_sets.iter().enumerate() {
        let mut assigned = false;
        for cluster in &mut clusters {
            let rep_idx = cluster[0];
            let rep_set = &token_sets[rep_idx];
            let sim = jaccard_similarity_sets(set, rep_set);
            if sim >= JACCARD_THRESHOLD {
                cluster.push(idx);
                assigned = true;
                break;
            }
        }
        if !assigned {
            clusters.push(vec![idx]);
        }
    }
    clusters
}

pub fn search_crystals_filtered(
    conn: &Connection,
    query_vector: &[f32],
    limit: usize,
    owner_id: Option<i64>,
    team_mode: bool,
) -> Vec<(i64, String, String, f64)> {
    if query_vector.is_empty() {
        return Vec::new();
    }
    let mut stmt = match conn.prepare(
        "SELECT id, label, consolidated_text, centroid, owner_id, visibility
         FROM memory_clusters
         ORDER BY julianday(updated_at) DESC, id DESC
         LIMIT ?1",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Vec<u8>>(3)?,
            row.get::<_, Option<i64>>(4).ok().flatten(),
            row.get::<_, Option<String>>(5).ok().flatten(),
        ))
    });
    let Ok(rows) = rows else {
        return Vec::new();
    };
    rows.filter_map(Result::ok)
        .filter(|(_, _, _, _, row_owner, visibility)| {
            !team_mode
                || row_owner == &owner_id
                || matches!(visibility.as_deref(), Some("shared" | "team"))
        })
        .map(|(id, label, text, _blob, _, _)| (id, label, text, 0.0))
        .collect()
}

pub fn unfold_crystal(conn: &Connection, crystal_id: i64) -> Vec<String> {
    let mut stmt = match conn
        .prepare("SELECT source FROM cluster_members WHERE cluster_id = ?1 ORDER BY id ASC")
    {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };
    stmt.query_map(params![crystal_id], |row| row.get::<_, String>(0))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

pub fn list_crystals(conn: &Connection) -> Vec<Value> {
    let mut stmt = match conn.prepare("SELECT id, label, consolidated_text, member_count, updated_at FROM memory_clusters ORDER BY julianday(updated_at) DESC, id DESC") {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };
    stmt.query_map([], |row| {
        Ok(json!({"id":row.get::<_,i64>(0)?,"label":row.get::<_,String>(1)?,"text":row.get::<_,String>(2)?,
            "members":row.get::<_,i64>(3)?,"updatedAt":row.get::<_,String>(4)?}))
    })
    .map(|rows| rows.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

pub fn migrate_crystal_tables(conn: &Connection) {
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory_clusters (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            label TEXT NOT NULL,
            centroid BLOB NOT NULL DEFAULT X'',
            consolidated_text TEXT NOT NULL,
            member_count INTEGER NOT NULL DEFAULT 0,
            owner_id INTEGER,
            visibility TEXT NOT NULL DEFAULT 'private',
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS cluster_members (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            cluster_id INTEGER NOT NULL,
            source TEXT NOT NULL,
            target_type TEXT,
            target_id INTEGER,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_cluster_members_cluster ON cluster_members(cluster_id);
        CREATE INDEX IF NOT EXISTS idx_memory_clusters_updated ON memory_clusters(updated_at);",
    );
}
