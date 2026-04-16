// SPDX-License-Identifier: MIT
//! Knowledge Crystallization Engine
//!
//! Detects clusters of semantically related memories/decisions and synthesizes
//! them into consolidated "crystal" nodes. Creates a two-tier recall hierarchy:
//!
//!   Tier 1: Crystal nodes (dense, high-signal, searched first)
//!   Tier 2: Source memories (full detail, accessed via unfold)
//!
//! Algorithm:
//!   1. Load all active embeddings
//!   2. Greedy clustering: cosine similarity > CLUSTER_THRESHOLD
//!   3. Extractive synthesis: best sentence from each member, deduped
//!   4. Store crystal + member links
//!   5. Recall searches crystals with a relevance boost
//!
//! Runs as a background job (like aging). No LLM dependency -- pure embeddings
//! + extractive text synthesis. The same zero-runtime-dep architecture.

use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};

use crate::embeddings::{self, EmbeddingEngine};

// ─── Constants ──────────────────────────────────────────────────────────────

/// Minimum cosine similarity to join a cluster.
const CLUSTER_THRESHOLD: f32 = 0.70;

/// Minimum cluster size to warrant crystallization.
const MIN_CLUSTER_SIZE: usize = 3;

/// Maximum sentences in a crystal summary.
const MAX_CRYSTAL_SENTENCES: usize = 8;

/// Maximum characters in a crystal summary.
const MAX_CRYSTAL_CHARS: usize = 600;

/// Jaccard threshold for sentence deduplication.
const DEDUP_JACCARD: f64 = 0.5;

/// Relevance boost applied to crystal nodes during recall.
pub const CRYSTAL_RELEVANCE_BOOST: f64 = 1.15;

fn is_missing_team_visibility_columns(err: &rusqlite::Error) -> bool {
    let normalized = err.to_string().to_ascii_lowercase();
    normalized.contains("no such column")
        && (normalized.contains("owner_id") || normalized.contains("visibility"))
}

// ─── Types ──────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct EmbeddedEntry {
    target_type: String,
    target_id: i64,
    vector: Vec<f32>,
    #[allow(dead_code)]
    source: String,
    text: String,
}

#[derive(Debug)]
pub struct CrystallizeResult {
    pub clusters_found: usize,
    pub crystals_created: usize,
    pub crystals_updated: usize,
    pub entries_consolidated: usize,
}

// ─── Main entry point ───────────────────────────────────────────────────────

/// Run one crystallization pass. Safe to call repeatedly.
/// Returns stats about what was created/updated.
pub fn run_crystallize_pass(
    conn: &Connection,
    engine: Option<&EmbeddingEngine>,
    owner_id: Option<i64>,
) -> CrystallizeResult {
    let mut result = CrystallizeResult {
        clusters_found: 0,
        crystals_created: 0,
        crystals_updated: 0,
        entries_consolidated: 0,
    };

    // 1. Load all active entries with embeddings
    let entries = load_embedded_entries(conn);
    if entries.len() < MIN_CLUSTER_SIZE {
        return result;
    }

    // 2. Greedy clustering
    let clusters = cluster_entries(&entries);
    result.clusters_found = clusters.len();

    if clusters.is_empty() {
        return result;
    }

    // 3. For each cluster, synthesize and store
    for cluster in &clusters {
        let member_entries: Vec<&EmbeddedEntry> =
            cluster.iter().map(|&idx| &entries[idx]).collect();

        result.entries_consolidated += member_entries.len();

        // Generate label from most common words across members
        let label = generate_cluster_label(&member_entries);

        // Extractive synthesis
        let consolidated_text = synthesize_crystal(&member_entries);

        // Compute centroid embedding
        let centroid = compute_centroid(
            &member_entries
                .iter()
                .map(|e| e.vector.as_slice())
                .collect::<Vec<_>>(),
        );
        let centroid_blob = embeddings::vector_to_blob(&centroid);

        // Check if a crystal already exists for this cluster (by label overlap)
        let existing_id = find_existing_crystal(conn, &label);

        match existing_id {
            Some(crystal_id) => {
                // Update existing crystal
                let _ = conn.execute(
                    "UPDATE memory_clusters SET consolidated_text = ?1, centroid = ?2, \
                     member_count = ?3, updated_at = datetime('now') WHERE id = ?4",
                    params![
                        consolidated_text,
                        centroid_blob,
                        member_entries.len() as i64,
                        crystal_id
                    ],
                );
                update_cluster_members(conn, crystal_id, &member_entries);
                result.crystals_updated += 1;
            }
            None => {
                // Create new crystal
                if let Some(oid) = owner_id {
                    let _ = conn.execute(
                        "INSERT INTO memory_clusters (label, centroid, consolidated_text, member_count, owner_id, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'), datetime('now'))",
                        params![label, centroid_blob, consolidated_text, member_entries.len() as i64, oid],
                    );
                } else {
                    let _ = conn.execute(
                        "INSERT INTO memory_clusters (label, centroid, consolidated_text, member_count, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now'))",
                        params![label, centroid_blob, consolidated_text, member_entries.len() as i64],
                    );
                }
                let crystal_id = conn.last_insert_rowid();
                update_cluster_members(conn, crystal_id, &member_entries);

                // Embed the crystal text for recall
                if let Some(eng) = engine {
                    if let Some(vec) = eng.embed(&consolidated_text) {
                        let blob = embeddings::vector_to_blob(&vec);
                        let model_key = eng.model_key();
                        let _ = conn.execute(
                            "INSERT OR REPLACE INTO embeddings (target_type, target_id, vector, model) \
                             VALUES ('crystal', ?1, ?2, ?3)",
                            params![crystal_id, blob, model_key],
                        );
                    }
                }

                result.crystals_created += 1;
            }
        }
    }

    if result.crystals_created > 0 || result.crystals_updated > 0 {
        eprintln!(
            "[crystallize] Pass complete: {} clusters, {} created, {} updated, {} entries consolidated",
            result.clusters_found,
            result.crystals_created,
            result.crystals_updated,
            result.entries_consolidated
        );
    }

    result
}

// ─── Load entries ───────────────────────────────────────────────────────────

fn load_embedded_entries(conn: &Connection) -> Vec<EmbeddedEntry> {
    let mut entries = Vec::new();

    // Load memories
    if let Ok(mut stmt) = conn.prepare(
        "SELECT e.target_id, e.vector, m.text, m.source \
         FROM embeddings e \
         JOIN memories m ON e.target_type = 'memory' AND e.target_id = m.id \
         WHERE m.status = 'active'",
    ) {
        let rows: Vec<(i64, Vec<u8>, String, Option<String>)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .into_iter()
            .flatten()
            .flatten()
            .collect();

        for (id, blob, text, source) in rows {
            entries.push(EmbeddedEntry {
                target_type: "memory".to_string(),
                target_id: id,
                vector: embeddings::blob_to_vector(&blob),
                source: source.unwrap_or_else(|| format!("memory::{id}")),
                text,
            });
        }
    }

    // Load decisions
    if let Ok(mut stmt) = conn.prepare(
        "SELECT e.target_id, e.vector, d.decision, d.context \
         FROM embeddings e \
         JOIN decisions d ON e.target_type = 'decision' AND e.target_id = d.id \
         WHERE d.status = 'active'",
    ) {
        let rows: Vec<(i64, Vec<u8>, String, Option<String>)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .into_iter()
            .flatten()
            .flatten()
            .collect();

        for (id, blob, decision, context) in rows {
            entries.push(EmbeddedEntry {
                target_type: "decision".to_string(),
                target_id: id,
                vector: embeddings::blob_to_vector(&blob),
                source: context.unwrap_or_else(|| format!("decision::{id}")),
                text: decision,
            });
        }
    }

    entries
}

// ─── Clustering ─────────────────────────────────────────────────────────────

/// Greedy single-linkage clustering. Returns clusters as vectors of indices.
fn cluster_entries(entries: &[EmbeddedEntry]) -> Vec<Vec<usize>> {
    let n = entries.len();
    let mut assigned = vec![false; n];
    let mut clusters: Vec<Vec<usize>> = Vec::new();

    for i in 0..n {
        if assigned[i] {
            continue;
        }

        let mut cluster = vec![i];
        assigned[i] = true;

        // Find all entries similar to the seed
        for j in (i + 1)..n {
            if assigned[j] {
                continue;
            }
            let sim = embeddings::cosine_similarity(&entries[i].vector, &entries[j].vector);
            if sim >= CLUSTER_THRESHOLD {
                cluster.push(j);
                assigned[j] = true;
            }
        }

        if cluster.len() >= MIN_CLUSTER_SIZE {
            clusters.push(cluster);
        }
    }

    clusters
}

/// Compute the centroid (average) of a set of vectors.
fn compute_centroid(vectors: &[&[f32]]) -> Vec<f32> {
    if vectors.is_empty() {
        return vec![];
    }
    let dim = vectors[0].len();
    let mut centroid = vec![0.0f32; dim];
    for vec in vectors {
        for (i, &v) in vec.iter().enumerate() {
            centroid[i] += v;
        }
    }
    let n = vectors.len() as f32;
    for v in &mut centroid {
        *v /= n;
    }
    // L2 normalize
    let norm: f32 = centroid.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut centroid {
            *v /= norm;
        }
    }
    centroid
}

// ─── Extractive synthesis ───────────────────────────────────────────────────

/// High-signal keywords that indicate important content.
const SIGNAL_WORDS: &[&str] = &[
    "must",
    "never",
    "always",
    "critical",
    "important",
    "decision",
    "fixed",
    "bug",
    "error",
    "confirmed",
    "approved",
    "rejected",
    "architecture",
    "design",
    "migration",
    "breaking",
    "security",
    "performance",
    "prefer",
    "avoid",
    "use",
    "requires",
    "deprecated",
    "instead",
];

/// Synthesize a crystal summary from cluster members using extractive methods.
fn synthesize_crystal(members: &[&EmbeddedEntry]) -> String {
    // Extract best sentence from each member
    let mut candidates: Vec<(String, f64)> = Vec::new();

    for entry in members {
        let sentences: Vec<&str> = entry
            .text
            .split(['.', '\n'])
            .map(|s| s.trim())
            .filter(|s| s.len() > 10)
            .collect();

        if sentences.is_empty() {
            // Use truncated full text if no sentences
            let trunc: String = entry.text.chars().take(80).collect();
            candidates.push((trunc, 1.0));
            continue;
        }

        // Score each sentence by signal word density
        let mut best_sentence = sentences[0].to_string();
        let mut best_score = 0.0f64;

        for sentence in &sentences {
            let lower = sentence.to_lowercase();
            let word_count = lower.split_whitespace().count().max(1) as f64;
            let signal_count = SIGNAL_WORDS
                .iter()
                .filter(|kw| lower.contains(**kw))
                .count() as f64;
            let score = signal_count / word_count
                + if sentence == sentences.first().unwrap() {
                    0.1
                } else {
                    0.0
                };

            if score > best_score {
                best_score = score;
                best_sentence = sentence.to_string();
            }
        }

        candidates.push((best_sentence, best_score));
    }

    // Sort by score (best first), dedup, take top N
    candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut kept: Vec<String> = Vec::new();
    for (sentence, _score) in &candidates {
        if kept.len() >= MAX_CRYSTAL_SENTENCES {
            break;
        }
        // Dedup: skip if too similar to an already-kept sentence
        let dominated = kept
            .iter()
            .any(|existing| jaccard_words(existing, sentence) > DEDUP_JACCARD);
        if !dominated {
            kept.push(sentence.clone());
        }
    }

    // Join with periods, cap at MAX_CRYSTAL_CHARS
    let mut result = kept.join(". ");
    if result.len() > MAX_CRYSTAL_CHARS {
        result = result.chars().take(MAX_CRYSTAL_CHARS).collect::<String>();
        result.push_str("...");
    }

    result
}

/// Word-level Jaccard similarity between two strings.
fn jaccard_words(a: &str, b: &str) -> f64 {
    let set_a: HashSet<&str> = a
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| w.len() > 2)
        .collect();
    let set_b: HashSet<&str> = b
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| w.len() > 2)
        .collect();
    let intersection = set_a.intersection(&set_b).count() as f64;
    let union = set_a.union(&set_b).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

// ─── Label generation ───────────────────────────────────────────────────────

/// Generate a human-readable label for a cluster from its members' text.
/// Uses TF-IDF-like scoring: words frequent in the cluster but not universal.
fn generate_cluster_label(members: &[&EmbeddedEntry]) -> String {
    let stop_words: HashSet<&str> = [
        "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
        "do", "does", "did", "will", "would", "could", "should", "may", "might", "shall", "can",
        "to", "of", "in", "for", "on", "with", "at", "by", "from", "as", "into", "about", "like",
        "through", "after", "over", "between", "out", "up", "and", "but", "or", "not", "no", "if",
        "then", "than", "that", "this", "it", "its", "all", "each", "every", "both", "few", "more",
        "most", "other", "some", "such", "only", "own", "same", "so", "too", "very", "just",
        "because", "when", "where", "how", "what", "which", "who", "whom", "why", "use", "using",
        "used",
    ]
    .iter()
    .cloned()
    .collect();

    let mut word_freq: HashMap<String, usize> = HashMap::new();
    let mut doc_freq: HashMap<String, usize> = HashMap::new();

    for entry in members {
        let mut seen_in_doc: HashSet<String> = HashSet::new();
        for word in entry
            .text
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
        {
            let w = word.trim();
            if w.len() < 3 || stop_words.contains(w) {
                continue;
            }
            *word_freq.entry(w.to_string()).or_insert(0) += 1;
            if seen_in_doc.insert(w.to_string()) {
                *doc_freq.entry(w.to_string()).or_insert(0) += 1;
            }
        }
    }

    let n = members.len() as f64;
    let mut scored: Vec<(String, f64)> = word_freq
        .into_iter()
        .filter(|(word, _)| {
            let df = *doc_freq.get(word).unwrap_or(&0) as f64;
            // Appears in at least 40% of members (cluster-characteristic)
            df / n >= 0.4
        })
        .map(|(word, freq)| {
            let df = *doc_freq.get(&word).unwrap_or(&1) as f64;
            let tf_idf = freq as f64 * (n / df).ln().max(0.1);
            (word, tf_idf)
        })
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let top_words: Vec<&str> = scored.iter().take(4).map(|(w, _)| w.as_str()).collect();
    if top_words.is_empty() {
        "misc".to_string()
    } else {
        top_words.join("-")
    }
}

// ─── Database helpers ───────────────────────────────────────────────────────

fn find_existing_crystal(conn: &Connection, label: &str) -> Option<i64> {
    conn.query_row(
        "SELECT id FROM memory_clusters WHERE label = ?1",
        params![label],
        |row| row.get(0),
    )
    .ok()
}

fn update_cluster_members(conn: &Connection, crystal_id: i64, members: &[&EmbeddedEntry]) {
    // Clear old members
    let _ = conn.execute(
        "DELETE FROM cluster_members WHERE cluster_id = ?1",
        params![crystal_id],
    );

    // Insert new members
    for entry in members {
        let _ = conn.execute(
            "INSERT OR IGNORE INTO cluster_members (cluster_id, target_type, target_id, similarity) \
             VALUES (?1, ?2, ?3, ?4)",
            params![crystal_id, entry.target_type, entry.target_id, 1.0],
        );
    }
}

// ─── Recall integration ─────────────────────────────────────────────────────

/// Search crystal nodes by semantic similarity. Returns (crystal_id, label,
/// consolidated_text, similarity) sorted by relevance.
/// Crystal search with optional visibility filtering for team mode.
#[allow(clippy::type_complexity)]
pub fn search_crystals_filtered(
    conn: &Connection,
    query_vec: &[f32],
    limit: usize,
    caller_id: Option<i64>,
    team_mode: bool,
) -> Vec<(i64, String, String, f64)> {
    let query_rows = |sql: &str,
                      with_visibility: bool|
     -> Result<
        Vec<(i64, Vec<u8>, String, String, Option<i64>, Option<String>)>,
        rusqlite::Error,
    > {
        let mut stmt = conn.prepare(sql)?;
        let mapped = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                if with_visibility {
                    row.get::<_, Option<i64>>(4)?
                } else {
                    None
                },
                if with_visibility {
                    row.get::<_, Option<String>>(5)?
                } else {
                    None
                },
            ))
        })?;
        Ok(mapped.flatten().collect())
    };

    let sql_with_visibility =
        "SELECT mc.id, e.vector, mc.label, mc.consolidated_text, mc.owner_id, mc.visibility \
         FROM embeddings e \
         JOIN memory_clusters mc ON e.target_type = 'crystal' AND e.target_id = mc.id";
    let sql_legacy = "SELECT mc.id, e.vector, mc.label, mc.consolidated_text \
         FROM embeddings e \
         JOIN memory_clusters mc ON e.target_type = 'crystal' AND e.target_id = mc.id";

    let rows = match query_rows(sql_with_visibility, true) {
        Ok(rows) => rows,
        Err(err) if is_missing_team_visibility_columns(&err) => {
            query_rows(sql_legacy, false).unwrap_or_default()
        }
        Err(_) => Vec::new(),
    };

    let mut results: Vec<(i64, String, String, f64)> = rows
        .into_iter()
        .filter_map(|(id, blob, label, text, owner_id, visibility)| {
            // Visibility: solo mode sees everything; team mode fails closed
            if team_mode {
                let caller = match caller_id {
                    Some(c) => c,
                    None => return None, // fail closed: unidentified caller
                };
                let owner = match owner_id {
                    Some(o) => o,
                    None => return None, // fail closed: unowned data
                };
                if owner != caller
                    && !matches!(visibility.as_deref(), Some("shared") | Some("team"))
                {
                    return None;
                }
            }
            let vec = embeddings::blob_to_vector(&blob);
            let sim = embeddings::cosine_similarity(query_vec, &vec) as f64;
            if sim > 0.3 {
                Some((id, label, text, sim * CRYSTAL_RELEVANCE_BOOST))
            } else {
                None
            }
        })
        .collect();

    results.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(limit);
    results
}

/// Unfold a crystal: return its member sources for detailed retrieval.
pub fn unfold_crystal(conn: &Connection, crystal_id: i64) -> Vec<String> {
    conn.prepare(
        "SELECT cm.target_type, cm.target_id, \
                CASE WHEN cm.target_type = 'memory' THEN m.source \
                     ELSE COALESCE(d.context, 'decision::' || d.id) END as source \
         FROM cluster_members cm \
         LEFT JOIN memories m ON cm.target_type = 'memory' AND cm.target_id = m.id \
         LEFT JOIN decisions d ON cm.target_type = 'decision' AND cm.target_id = d.id \
         WHERE cm.cluster_id = ?1",
    )
    .and_then(|mut stmt| {
        let rows = stmt.query_map(params![crystal_id], |row| row.get::<_, String>(2))?;
        Ok(rows.flatten().collect())
    })
    .unwrap_or_default()
}

// ─── GET /crystals ──────────────────────────────────────────────────────────

/// List all crystals with their stats.
pub fn list_crystals(conn: &Connection) -> Vec<serde_json::Value> {
    conn.prepare(
        "SELECT id, label, consolidated_text, member_count, created_at, updated_at \
         FROM memory_clusters ORDER BY updated_at DESC",
    )
    .and_then(|mut stmt| {
        let rows = stmt.query_map([], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, i64>(0)?,
                "label": row.get::<_, String>(1)?,
                "text": row.get::<_, String>(2)?,
                "members": row.get::<_, i64>(3)?,
                "created": row.get::<_, String>(4)?,
                "updated": row.get::<_, String>(5)?,
            }))
        })?;
        Ok(rows.flatten().collect())
    })
    .unwrap_or_default()
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    #[test]
    fn test_jaccard_identical() {
        assert!((jaccard_words("hello world test", "hello world test") - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_jaccard_different() {
        let sim = jaccard_words("the quick brown fox", "lazy purple elephant jumps");
        assert!(sim < 0.2);
    }

    #[test]
    fn test_jaccard_partial() {
        let sim = jaccard_words("use python for backend", "use python for frontend");
        assert!(sim > 0.5, "Shared 'use python for' should give >0.5");
    }

    #[test]
    fn test_compute_centroid() {
        let v1 = [1.0, 0.0, 0.0];
        let v2 = [0.0, 1.0, 0.0];
        let centroid = compute_centroid(&[&v1[..], &v2[..]]);
        // Average is [0.5, 0.5, 0] normalized ≈ [0.707, 0.707, 0]
        assert!(centroid[0] > 0.6 && centroid[0] < 0.8);
        assert!(centroid[1] > 0.6 && centroid[1] < 0.8);
        assert!(centroid[2].abs() < 0.001);
    }

    #[test]
    fn test_generate_label_empty() {
        let entries: Vec<&EmbeddedEntry> = vec![];
        assert_eq!(generate_cluster_label(&entries), "misc");
    }

    #[test]
    fn test_synthesize_deduplicates() {
        let e1 = EmbeddedEntry {
            target_type: "memory".to_string(),
            target_id: 1,
            vector: vec![],
            source: "test1".to_string(),
            text: "Always use uv for Python package management. Never use pip directly."
                .to_string(),
        };
        let e2 = EmbeddedEntry {
            target_type: "memory".to_string(),
            target_id: 2,
            vector: vec![],
            source: "test2".to_string(),
            text: "Use uv for Python package management instead of pip.".to_string(),
        };
        let e3 = EmbeddedEntry {
            target_type: "memory".to_string(),
            target_id: 3,
            vector: vec![],
            source: "test3".to_string(),
            text: "Python type hints required on all function signatures.".to_string(),
        };
        let members: Vec<&EmbeddedEntry> = vec![&e1, &e2, &e3];
        let result = synthesize_crystal(&members);
        // Should not have two near-identical uv/pip sentences
        let period_count = result.matches('.').count();
        assert!(
            period_count <= 3,
            "Should deduplicate similar sentences, got: {result}"
        );
    }

    #[test]
    fn test_cluster_entries_basic() {
        // Two groups of identical vectors should form two clusters
        let make_entry = |id: i64, vec: Vec<f32>| EmbeddedEntry {
            target_type: "memory".to_string(),
            target_id: id,
            vector: vec,
            source: format!("test::{id}"),
            text: format!("Entry {id}"),
        };

        let entries = vec![
            make_entry(1, vec![1.0, 0.0, 0.0]),
            make_entry(2, vec![0.98, 0.1, 0.0]),
            make_entry(3, vec![0.95, 0.15, 0.0]),
            make_entry(4, vec![0.0, 1.0, 0.0]),
            make_entry(5, vec![0.1, 0.98, 0.0]),
            make_entry(6, vec![0.15, 0.95, 0.0]),
        ];

        let clusters = cluster_entries(&entries);
        assert_eq!(clusters.len(), 2, "Should find 2 clusters");
    }

    #[test]
    fn test_full_crystallize_pass() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::configure(&conn).unwrap();
        crate::db::initialize_schema(&conn).unwrap();
        migrate_crystal_tables(&conn);

        // Insert test memories with embeddings that cluster together
        for i in 1..=4 {
            conn.execute(
                "INSERT INTO memories (id, text, source, type, status) VALUES (?1, ?2, ?3, 'memory', 'active')",
                params![i, format!("Python requires uv for package management rule {i}"), format!("test::python_{i}")],
            ).unwrap();
            // All get similar embeddings (near [1,0,0...])
            let mut vec = vec![0.0f32; 384];
            vec[0] = 1.0;
            vec[1] = 0.01 * i as f32; // slight variation
            let blob = embeddings::vector_to_blob(&vec);
            conn.execute(
                "INSERT INTO embeddings (target_type, target_id, vector) VALUES ('memory', ?1, ?2)",
                params![i, blob],
            )
            .unwrap();
        }

        let result = run_crystallize_pass(&conn, None, None);
        assert_eq!(result.clusters_found, 1);
        assert_eq!(result.crystals_created, 1);
        assert_eq!(result.entries_consolidated, 4);

        // Verify crystal exists
        let crystals = list_crystals(&conn);
        assert_eq!(crystals.len(), 1);
        assert!(crystals[0]["members"].as_i64().unwrap() >= 4);
    }
}

// ─── Schema migration ───────────────────────────────────────────────────────

pub fn migrate_crystal_tables(conn: &Connection) {
    let sql = r#"
        CREATE TABLE IF NOT EXISTS memory_clusters (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            label TEXT NOT NULL,
            centroid BLOB,
            consolidated_text TEXT NOT NULL,
            member_count INTEGER DEFAULT 0,
            created_at TEXT DEFAULT (datetime('now')),
            updated_at TEXT DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS cluster_members (
            cluster_id INTEGER NOT NULL,
            target_type TEXT NOT NULL,
            target_id INTEGER NOT NULL,
            similarity REAL NOT NULL DEFAULT 1.0,
            PRIMARY KEY (cluster_id, target_type, target_id),
            FOREIGN KEY (cluster_id) REFERENCES memory_clusters(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_cluster_members_target ON cluster_members(target_type, target_id);
    "#;
    match conn.execute_batch(sql) {
        Ok(_) => {}
        Err(e) => eprintln!("[db] Crystal table migration: {e}"),
    }
}
