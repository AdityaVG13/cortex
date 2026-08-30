use crate::state::{BrainFiringEvent, BrainKind};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use tokio::sync::broadcast;

pub type BrainFiringSender = Option<broadcast::Sender<BrainFiringEvent>>;
pub const CRYSTAL_RELEVANCE_BOOST: f64 = 1.15;

const MAX_SCAN_ROWS: i64 = 500;
const COSINE_THRESHOLD: f64 = 0.83;
const JACCARD_THRESHOLD: f64 = 0.30;

const STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "in", "on", "at", "to", "for", "of", "with", "by", "is", "are", "was", "were", "be", "been", "has", "have", "had",
    "it", "this", "that", "these", "those", "as", "from", "will", "would", "can", "could", "should", "do", "does", "did", "so", "if", "then", "than", "when",
    "what", "which", "who", "how", "why", "not", "no", "yes", "we", "you", "they", "our", "your", "their", "its", "my", "me", "i", "he", "she", "them", "us",
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
    source: String,
    score: f64,
    created_at: String,
    target_type: String,
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
        set.insert(fold_jaccard_token(word));
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
    if uni == 0.0 {
        0.0
    } else {
        inter / uni
    }
}

fn cosine_similarity(_a: &[f32], _b: &[f32]) -> f32 {
    0.0
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
        let tokens: Vec<String> = first.split_whitespace().filter(|w| w.len() > 1).take(4).map(|w| fold_jaccard_token(w)).collect();
        if tokens.is_empty() {
            return "cluster".to_string();
        }
        return tokens.join(" ");
    }
    items.into_iter().map(|(w, _)| w).collect::<Vec<_>>().join(" ")
}

fn mean_vector(vectors: &[Vec<f32>]) -> Option<Vec<f32>> {
    if vectors.is_empty() {
        return None;
    }
    let dim = vectors[0].len();
    if dim == 0 {
        return None;
    }
    let mut mean = vec![0f32; dim];
    for v in vectors {
        if v.len() != dim {
            return None;
        }
        for (i, val) in v.iter().enumerate() {
            mean[i] += *val;
        }
    }
    let n = vectors.len() as f32;
    for m in &mut mean {
        *m /= n;
    }
    let norm: f32 = mean.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 && norm.is_finite() {
        for m in &mut mean {
            *m /= norm;
        }
    }
    Some(mean)
}

pub fn run_crystallize_pass_with_brain(conn: &Connection, owner_id: Option<i64>, brain: &BrainFiringSender) -> CrystallizeResult {
    let candidates = scan_candidates(conn, owner_id);
    if candidates.is_empty() {
        return CrystallizeResult { clusters_found: 0, crystals_created: 0, crystals_updated: 0, entries_consolidated: 0 };
    }

    let vector_map: HashMap<(String, i64), Vec<f32>> = HashMap::new();
    let use_cosine = false;
    let clusters: Vec<Vec<usize>> = cluster_by_jaccard(&candidates);

    let qualified: Vec<Vec<usize>> = clusters.into_iter().filter(|c| c.len() >= 2).collect();

    let mut crystals_created = 0usize;
    let mut entries_consolidated = 0usize;

    for member_indices in &qualified {
        let member_candidates: Vec<&Candidate> = member_indices.iter().map(|&i| &candidates[i]).collect();
        let texts: Vec<String> = member_candidates.iter().map(|c| c.text.clone()).collect();
        let label = label_for_members(&texts);
        let mut best = &member_candidates[0];
        for c in &member_candidates[1..] {
            if c.score > best.score || (c.score == best.score && c.id > best.id) {
                best = c;
            }
        }
        let consolidated_text = best.text.clone();

        let centroid_blob: Vec<u8> = if use_cosine {
            let vectors: Vec<Vec<f32>> = member_indices
                .iter()
                .filter_map(|&i| {
                    let c = &candidates[i];
                    vector_map.get(&(c.target_type.clone(), c.id)).cloned()
                })
                .collect();
            if let Some(mean) = mean_vector(&vectors) {
                Vec::<u8>::new()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        let member_count = member_indices.len() as i64;
        let insert = conn.execute(
            "INSERT INTO memory_clusters (label, centroid, consolidated_text, member_count, owner_id, visibility) VALUES (?1, ?2, ?3, ?4, ?5, 'private')",
            params![label, centroid_blob, consolidated_text, member_count, owner_id],
        );
        if let Ok(_) = insert {
            let cluster_id = conn.last_insert_rowid();
            for &idx in member_indices {
                let cand = &candidates[idx];
                let _ = conn.execute(
                    "INSERT INTO cluster_members (cluster_id, source, target_type, target_id) VALUES (?1, ?2, ?3, ?4)",
                    params![cluster_id, cand.text, cand.target_type, cand.id],
                );
            }
            crystals_created += 1;
            entries_consolidated += member_indices.len();
        }
    }

    let result = CrystallizeResult { clusters_found: crystals_created, crystals_created, crystals_updated: 0, entries_consolidated };

    if result.crystals_created > 0 || result.crystals_updated > 0 {
        if let Some(tx) = brain {
            let _ = tx.send(BrainFiringEvent {
                kind: BrainKind::ConsolidationStarted,
                payload: json!({
                    "clusters_found": result.clusters_found,
                    "crystals_created": result.crystals_created,
                    "crystals_updated": result.crystals_updated,
                    "entries_consolidated": result.entries_consolidated,
                }),
                owner_id,
            });
        }
    }

    result
}

fn scan_candidates(conn: &Connection, owner_id: Option<i64>) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();

    let mem_sql = if owner_id.is_some() {
        "SELECT id, text, COALESCE(source,'') , COALESCE(score,1.0), COALESCE(created_at,'') FROM memories WHERE status NOT IN ('superseded','archived') AND id NOT IN (SELECT target_id FROM cluster_members WHERE target_type='memory') AND (owner_id = ?1 OR owner_id IS NULL) ORDER BY COALESCE(last_accessed, created_at) DESC LIMIT ?2"
    } else {
        "SELECT id, text, COALESCE(source,'') , COALESCE(score,1.0), COALESCE(created_at,'') FROM memories WHERE status NOT IN ('superseded','archived') AND id NOT IN (SELECT target_id FROM cluster_members WHERE target_type='memory') ORDER BY COALESCE(last_accessed, created_at) DESC LIMIT ?2"
    };
    let mut stmt = match conn.prepare(mem_sql) {
        Ok(s) => s,
        Err(_) => return out,
    };
    let rows = if owner_id.is_some() {
        stmt.query_map(params![owner_id, MAX_SCAN_ROWS], |row| {
            Ok(Candidate {
                id: row.get(0)?,
                text: row.get(1)?,
                source: row.get(2)?,
                score: row.get(3)?,
                created_at: row.get(4)?,
                target_type: "memory".to_string(),
            })
        })
    } else {
        return scan_candidates_no_owner(conn);
    };
    if let Ok(mapped) = rows {
        for r in mapped.flatten() {
            out.push(r);
        }
    }
    let dec_sql = if owner_id.is_some() {
        "SELECT id, decision, COALESCE(context,'') , COALESCE(score,1.0), COALESCE(created_at,'') FROM decisions WHERE status NOT IN ('superseded','archived') AND id NOT IN (SELECT target_id FROM cluster_members WHERE target_type='decision') AND (owner_id = ?1 OR owner_id IS NULL) ORDER BY COALESCE(last_accessed, created_at) DESC LIMIT ?2"
    } else {
        "SELECT id, decision, COALESCE(context,'') , COALESCE(score,1.0), COALESCE(created_at,'') FROM decisions WHERE status NOT IN ('superseded','archived') AND id NOT IN (SELECT target_id FROM cluster_members WHERE target_type='decision') ORDER BY COALESCE(last_accessed, created_at) DESC LIMIT ?2"
    };
    if owner_id.is_some() {
        if let Ok(mut dstmt) = conn.prepare(dec_sql) {
            if let Ok(mapped) = dstmt.query_map(params![owner_id, MAX_SCAN_ROWS], |row| {
                Ok(Candidate {
                    id: row.get(0)?,
                    text: row.get(1)?,
                    source: row.get(2)?,
                    score: row.get(3)?,
                    created_at: row.get(4)?,
                    target_type: "decision".to_string(),
                })
            }) {
                for r in mapped.flatten() {
                    out.push(r);
                }
            }
        }
    }

    if out.len() as i64 > MAX_SCAN_ROWS {
        out.sort_by(|a, b| b.id.cmp(&a.id));
        out.truncate(MAX_SCAN_ROWS as usize);
    }
    out
}

fn scan_candidates_no_owner(conn: &Connection) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT id, text, COALESCE(source,'') , COALESCE(score,1.0), COALESCE(created_at,'') FROM memories WHERE status NOT IN ('superseded','archived') AND id NOT IN (SELECT target_id FROM cluster_members WHERE target_type='memory') ORDER BY COALESCE(last_accessed, created_at) DESC LIMIT ?1",
    ) {
        if let Ok(mapped) = stmt.query_map(params![MAX_SCAN_ROWS], |row| {
            Ok(Candidate {
                id: row.get(0)?,
                text: row.get(1)?,
                source: row.get(2)?,
                score: row.get(3)?,
                created_at: row.get(4)?,
                target_type: "memory".to_string(),
            })
        }) {
            for r in mapped.flatten() {
                out.push(r);
            }
        }
    }
    if let Ok(mut stmt) = conn.prepare(
        "SELECT id, decision, COALESCE(context,'') , COALESCE(score,1.0), COALESCE(created_at,'') FROM decisions WHERE status NOT IN ('superseded','archived') AND id NOT IN (SELECT target_id FROM cluster_members WHERE target_type='decision') ORDER BY COALESCE(last_accessed, created_at) DESC LIMIT ?1",
    ) {
        if let Ok(mapped) = stmt.query_map(params![MAX_SCAN_ROWS], |row| {
            Ok(Candidate {
                id: row.get(0)?,
                text: row.get(1)?,
                source: row.get(2)?,
                score: row.get(3)?,
                created_at: row.get(4)?,
                target_type: "decision".to_string(),
            })
        }) {
            for r in mapped.flatten() {
                out.push(r);
            }
        }
    }
    if out.len() as i64 > MAX_SCAN_ROWS {
        out.sort_by(|a, b| b.id.cmp(&a.id));
        out.truncate(MAX_SCAN_ROWS as usize);
    }
    out
}

fn load_candidate_vectors(_conn: &Connection, _candidates: &[Candidate]) -> HashMap<(String, i64), Vec<f32>> {
    HashMap::new()
}

fn cluster_by_cosine(candidates: &[Candidate], vectors: &HashMap<(String, i64), Vec<f32>>) -> Vec<Vec<usize>> {
    let mut clusters: Vec<Vec<usize>> = Vec::new();
    let mut centroids: Vec<Vec<f32>> = Vec::new();

    for (idx, cand) in candidates.iter().enumerate() {
        let key = (cand.target_type.clone(), cand.id);
        let Some(vec) = vectors.get(&key) else {
            clusters.push(vec![idx]);
            centroids.push(Vec::new());
            continue;
        };
        let mut best_cluster: Option<usize> = None;
        let mut best_sim = COSINE_THRESHOLD;
        for (ci, centroid) in centroids.iter().enumerate() {
            if centroid.is_empty() || centroid.len() != vec.len() {
                continue;
            }
            let sim = cosine_similarity(vec, centroid) as f64;
            if sim >= best_sim {
                best_sim = sim;
                best_cluster = Some(ci);
            }
        }
        if let Some(ci) = best_cluster {
            clusters[ci].push(idx);
            let member_vecs: Vec<Vec<f32>> = clusters[ci]
                .iter()
                .filter_map(|&mi| {
                    let cc = &candidates[mi];
                    vectors.get(&(cc.target_type.clone(), cc.id)).cloned()
                })
                .collect();
            if let Some(mean) = mean_vector(&member_vecs) {
                centroids[ci] = mean;
            }
        } else {
            clusters.push(vec![idx]);
            centroids.push(vec.clone());
        }
    }
    clusters
}

fn cluster_by_jaccard(candidates: &[Candidate]) -> Vec<Vec<usize>> {
    let token_sets: Vec<HashSet<String>> = candidates.iter().map(|c| jaccard_token_set(&c.text)).collect();
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
    conn: &Connection, query_vector: &[f32], limit: usize, owner_id: Option<i64>, team_mode: bool,
) -> Vec<(i64, String, String, f64)> {
    if query_vector.is_empty() {
        return Vec::new();
    }
    let mut stmt = match conn.prepare(
        "SELECT id, label, consolidated_text, centroid, owner_id, visibility
         FROM memory_clusters
         ORDER BY updated_at DESC
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
        .filter(|(_, _, _, _, row_owner, visibility)| !team_mode || row_owner == &owner_id || matches!(visibility.as_deref(), Some("shared" | "team")))
        .map(|(id, label, text, _blob, _, _)| (id, label, text, 0.0))
        .collect()
}

pub fn unfold_crystal(conn: &Connection, crystal_id: i64) -> Vec<String> {
    let mut stmt = match conn.prepare("SELECT source FROM cluster_members WHERE cluster_id = ?1 ORDER BY id ASC") {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };
    stmt.query_map(params![crystal_id], |row| row.get::<_, String>(0))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

pub fn list_crystals(conn: &Connection) -> Vec<Value> {
    let mut stmt = match conn.prepare("SELECT id, label, consolidated_text, member_count, updated_at FROM memory_clusters ORDER BY updated_at DESC") {
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
