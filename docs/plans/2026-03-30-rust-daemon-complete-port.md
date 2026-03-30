# Cortex Rust Daemon: Complete Port Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close all feature gaps between Node.js and Rust Cortex daemon, add embedded ONNX embeddings, make Rust strictly superior.

**Architecture:** Embedded ONNX (all-MiniLM-L6-v2, 384-dim) for in-process embeddings -- no Ollama dependency for semantic search. Knowledge indexing on startup reads 6 file sources. Full MCP parity. Graceful degradation when model not yet downloaded.

**Tech Stack:** Rust, axum 0.8, rusqlite (bundled), ort (ONNX Runtime), tokenizers (HuggingFace), reqwest (model download + Ollama health), tokio

**Parity Audit Reference:** See agent audit from 2026-03-30 session. 3 CRITICAL gaps, 7 IMPORTANT gaps, several nice-to-haves.

---

### Task 1: Add ONNX Embedding Infrastructure

**Files:**
- Modify: `daemon-rs/Cargo.toml`
- Create: `daemon-rs/src/embeddings.rs`

This task adds the core embedding module: model download, tokenization, inference, cosine similarity, and BLOB conversion. No integration with store/recall yet -- that's Tasks 2-3.

- [ ] **Step 1: Add dependencies to Cargo.toml**

Add after the existing `[dependencies]` entries:

```toml
# Embeddings (in-process ONNX)
ort = { version = "2", features = ["download-binaries"] }
ndarray = "0.16"
tokenizers = { version = "0.21", default-features = false, features = ["progressbar"] }
reqwest = { version = "0.12", features = ["rustls-tls", "blocking"], default-features = false }
dirs = "6"
```

- [ ] **Step 2: Create embeddings.rs with model management**

Create `daemon-rs/src/embeddings.rs`:

```rust
//! In-process ONNX embedding engine.
//! Uses all-MiniLM-L6-v2 (23MB, 384-dim) downloaded on first run.
//! No Ollama dependency -- embeddings work the moment Cortex starts.

use ndarray::{Array1, Array2, ArrayView1, Axis, CowArray};
use ort::{Session, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tokenizers::Tokenizer;
use tokio::sync::Mutex;

const MODEL_REPO: &str = "sentence-transformers/all-MiniLM-L6-v2";
const MODEL_FILE: &str = "all-MiniLM-L6-v2.onnx";
const TOKENIZER_FILE: &str = "tokenizer.json";
const EMBED_DIM: usize = 384;
const MAX_INPUT_TOKENS: usize = 256;

// Download URLs (HuggingFace CDN)
const MODEL_URL: &str = "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/onnx/model.onnx";
const TOKENIZER_URL: &str = "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/tokenizer.json";

/// Shared embedding engine. None if model not yet downloaded or load failed.
pub struct EmbeddingEngine {
    session: Session,
    tokenizer: Tokenizer,
}

impl EmbeddingEngine {
    /// Load from cached model files. Returns None if files missing.
    pub fn load(models_dir: &std::path::Path) -> Option<Self> {
        let model_path = models_dir.join(MODEL_FILE);
        let tok_path = models_dir.join(TOKENIZER_FILE);

        if !model_path.exists() || !tok_path.exists() {
            return None;
        }

        let session = Session::builder()
            .ok()?
            .with_intra_threads(2)
            .ok()?
            .commit_from_file(&model_path)
            .ok()?;

        let tokenizer = Tokenizer::from_file(&tok_path).ok()?;

        Some(Self { session, tokenizer })
    }

    /// Generate embedding for a text string. Returns 384-dim f32 vector.
    pub fn embed(&self, text: &str) -> Option<Vec<f32>> {
        let truncated = if text.len() > 2000 { &text[..2000] } else { text };

        let encoding = self.tokenizer.encode(truncated, true).ok()?;
        let ids = encoding.get_ids();
        let attention = encoding.get_attention_mask();
        let type_ids = encoding.get_type_ids();

        // Truncate to max tokens
        let len = ids.len().min(MAX_INPUT_TOKENS);
        let ids = &ids[..len];
        let attention = &attention[..len];
        let type_ids = &type_ids[..len];

        // Build input tensors [1, seq_len]
        let ids_array = Array2::from_shape_vec(
            (1, len),
            ids.iter().map(|&x| x as i64).collect(),
        ).ok()?;
        let mask_array = Array2::from_shape_vec(
            (1, len),
            attention.iter().map(|&x| x as i64).collect(),
        ).ok()?;
        let type_array = Array2::from_shape_vec(
            (1, len),
            type_ids.iter().map(|&x| x as i64).collect(),
        ).ok()?;

        let outputs = self.session.run(ort::inputs![
            "input_ids" => ids_array,
            "attention_mask" => mask_array,
            "token_type_ids" => type_array,
        ].ok()?).ok()?;

        // Output shape: [1, seq_len, 384] -- mean pool over seq_len
        let output_tensor = outputs[0].try_extract_tensor::<f32>().ok()?;
        let shape = output_tensor.shape();

        if shape.len() != 3 || shape[2] != EMBED_DIM {
            eprintln!("[embeddings] Unexpected output shape: {:?}", shape);
            return None;
        }

        // Mean pooling with attention mask
        let mut pooled = vec![0.0f32; EMBED_DIM];
        let mut mask_sum = 0.0f32;

        for seq_idx in 0..shape[1] {
            let mask_val = attention[seq_idx.min(len - 1)] as f32;
            mask_sum += mask_val;
            for dim in 0..EMBED_DIM {
                pooled[dim] += output_tensor[[0, seq_idx, dim]] * mask_val;
            }
        }

        if mask_sum > 0.0 {
            for dim in 0..EMBED_DIM {
                pooled[dim] /= mask_sum;
            }
        }

        // L2 normalize
        let norm: f32 = pooled.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut pooled {
                *x /= norm;
            }
        }

        Some(pooled)
    }
}

/// Cosine similarity between two f32 vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;

    for i in 0..a.len() {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }

    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 {
        return 0.0;
    }

    (dot / denom).clamp(0.0, 1.0)
}

/// Convert Vec<f32> to bytes for SQLite BLOB storage.
pub fn vector_to_blob(vec: &[f32]) -> Vec<u8> {
    vec.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Convert SQLite BLOB bytes back to Vec<f32>.
pub fn blob_to_vector(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

/// Download model files to ~/.cortex/models/ if not already cached.
/// Returns the models directory path.
pub async fn ensure_model_downloaded() -> Option<PathBuf> {
    let cortex_dir = dirs::home_dir()?.join(".cortex");
    let models_dir = cortex_dir.join("models");
    std::fs::create_dir_all(&models_dir).ok()?;

    let model_path = models_dir.join(MODEL_FILE);
    let tok_path = models_dir.join(TOKENIZER_FILE);

    if model_path.exists() && tok_path.exists() {
        return Some(models_dir);
    }

    eprintln!("[embeddings] Downloading embedding model (first run, ~23MB)...");

    // Download model
    if !model_path.exists() {
        match download_file(MODEL_URL, &model_path).await {
            Ok(_) => eprintln!("[embeddings] Model downloaded: {}", model_path.display()),
            Err(e) => {
                eprintln!("[embeddings] Failed to download model: {e}");
                return None;
            }
        }
    }

    // Download tokenizer
    if !tok_path.exists() {
        match download_file(TOKENIZER_URL, &tok_path).await {
            Ok(_) => eprintln!("[embeddings] Tokenizer downloaded: {}", tok_path.display()),
            Err(e) => {
                eprintln!("[embeddings] Failed to download tokenizer: {e}");
                return None;
            }
        }
    }

    Some(models_dir)
}

async fn download_file(url: &str, dest: &std::path::Path) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client.get(url).send().await.map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    std::fs::write(dest, &bytes).map_err(|e| e.to_string())?;

    Ok(())
}

/// Embedding dimension (for schema validation).
pub const DIMENSION: usize = EMBED_DIM;
```

- [ ] **Step 3: Register module in main.rs**

Add `mod embeddings;` to the module declarations at top of `main.rs`.

- [ ] **Step 4: Add EmbeddingEngine to RuntimeState**

In `state.rs`, add:

```rust
pub embedding_engine: Option<Arc<crate::embeddings::EmbeddingEngine>>,
```

In the `initialize()` function, after DB setup:

```rust
// Load embedding engine (non-blocking -- model may not be downloaded yet)
let models_dir = crate::auth::cortex_dir().join("models");
let embedding_engine = crate::embeddings::EmbeddingEngine::load(&models_dir)
    .map(Arc::new);

if embedding_engine.is_some() {
    eprintln!("[cortex] Embedding engine loaded (384-dim, in-process)");
} else {
    eprintln!("[cortex] Embedding engine not available -- will download model on first use");
}
```

- [ ] **Step 5: Build and verify compilation**

Run: `cd daemon-rs && cargo build --release 2>&1`
Expected: Compiles with no errors (warnings OK for now).

- [ ] **Step 6: Commit**

```bash
cd ~/cortex && git add daemon-rs/Cargo.toml daemon-rs/src/embeddings.rs daemon-rs/src/main.rs daemon-rs/src/state.rs
git commit -m "feat(daemon-rs): add embedded ONNX embedding engine (all-MiniLM-L6-v2, 384-dim)"
```

---

### Task 2: Integrate Embeddings into Store

**Files:**
- Modify: `daemon-rs/src/handlers/store.rs`
- Modify: `daemon-rs/src/conflict.rs`

Wire embedding generation into the store path: generate embedding on store (fire-and-forget), use cosine similarity for conflict detection.

- [ ] **Step 1: Add fire-and-forget embedding generation to store handler**

In `handlers/store.rs`, after the decision is inserted into the DB, add:

```rust
// Fire-and-forget: generate embedding for the new decision
if let Some(engine) = &state.embedding_engine {
    let engine = engine.clone();
    let db = state.db.clone();
    let text = decision_text.clone();
    let new_id = inserted_id;
    tokio::spawn(async move {
        if let Some(vec) = engine.embed(&text) {
            let blob = crate::embeddings::vector_to_blob(&vec);
            let conn = db.lock().await;
            let _ = conn.execute(
                "INSERT OR REPLACE INTO embeddings (target_type, target_id, vector, model) VALUES ('decision', ?1, ?2, 'all-MiniLM-L6-v2')",
                rusqlite::params![new_id, blob],
            );
        }
    });
}
```

- [ ] **Step 2: Add cosine conflict detection path to conflict.rs**

Add to `conflict.rs`:

```rust
/// Embedding-based conflict detection. Returns best match if cosine > 0.85.
pub fn detect_conflict_cosine(
    new_text: &str,
    source_agent: &str,
    engine: &crate::embeddings::EmbeddingEngine,
    conn: &rusqlite::Connection,
) -> Option<ConflictResult> {
    let new_vec = engine.embed(new_text)?;
    let new_blob = crate::embeddings::vector_to_blob(&new_vec);

    let mut stmt = conn.prepare(
        "SELECT d.id, d.decision, d.source_agent, e.vector
         FROM decisions d
         JOIN embeddings e ON e.target_type = 'decision' AND e.target_id = d.id
         WHERE d.status = 'active'"
    ).ok()?;

    let mut best_sim = 0.0f32;
    let mut best_match: Option<(i64, String, String)> = None;

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Vec<u8>>(3)?,
        ))
    }).ok()?;

    for row in rows.flatten() {
        let (id, decision, agent, blob) = row;
        let existing_vec = crate::embeddings::blob_to_vector(&blob);
        let sim = crate::embeddings::cosine_similarity(&new_vec, &existing_vec);

        if sim > best_sim {
            best_sim = sim;
            best_match = Some((id, decision, agent));
        }
    }

    const COSINE_THRESHOLD: f32 = 0.85;

    if let Some((id, _decision, matched_agent)) = best_match {
        if best_sim > COSINE_THRESHOLD {
            let is_update = matched_agent == source_agent;
            return Some(ConflictResult {
                is_conflict: !is_update,
                is_update,
                matched_id: Some(id),
                similarity: Some(best_sim),
                matched_agent: Some(matched_agent),
            });
        }
    }

    None
}
```

- [ ] **Step 3: Wire cosine detection into store handler before Jaccard fallback**

In the store handler, before the existing Jaccard conflict check:

```rust
// Try cosine conflict detection first (if embeddings available)
if let Some(engine) = &state.embedding_engine {
    let conn = state.db.lock().await;
    if let Some(result) = crate::conflict::detect_conflict_cosine(&decision_text, &source_agent, engine, &conn) {
        // Handle conflict same way as Jaccard path
        // ...
    }
}
// Fallback: Jaccard (existing code)
```

- [ ] **Step 4: Fix Jaccard threshold to 0.6 (match Node)**

In `conflict.rs`, change the Jaccard threshold from 0.7 to 0.6:

```rust
const JACCARD_THRESHOLD: f64 = 0.6; // Was 0.7, now matches Node
```

- [ ] **Step 5: Build and verify**

Run: `cargo build --release 2>&1`

- [ ] **Step 6: Commit**

```bash
git add daemon-rs/src/handlers/store.rs daemon-rs/src/conflict.rs
git commit -m "feat(daemon-rs): embedding-based conflict detection + fire-and-forget embed on store"
```

---

### Task 3: Integrate Embeddings into Recall (Semantic Search)

**Files:**
- Modify: `daemon-rs/src/handlers/recall.rs`

Add semantic search pass before keyword search, merging results.

- [ ] **Step 1: Add semantic search function**

Add to `recall.rs`:

```rust
/// Semantic search: embed query, compare against all stored embeddings.
fn semantic_search(
    query: &str,
    engine: &crate::embeddings::EmbeddingEngine,
    conn: &rusqlite::Connection,
) -> Vec<RecallResult> {
    let query_vec = match engine.embed(query) {
        Some(v) => v,
        None => return vec![],
    };

    let mut results = Vec::new();

    // Search memory embeddings
    let mut stmt = conn.prepare(
        "SELECT e.target_type, e.target_id, e.vector, m.text, m.source
         FROM embeddings e
         JOIN memories m ON e.target_type = 'memory' AND e.target_id = m.id AND m.status = 'active'"
    ).unwrap_or_else(|_| return); // graceful fallback

    // ... (iterate, compute cosine, filter > 0.3, collect)

    // Search decision embeddings similarly
    // ...

    results
}
```

- [ ] **Step 2: Wire semantic search into recall handler**

Before the existing keyword search in `handle_recall` and `handle_budget_recall`:

```rust
// 1. Semantic search (if embeddings available)
if let Some(engine) = &state.embedding_engine {
    let conn = state.db.lock().await;
    let semantic_results = semantic_search(&query, engine, &conn);
    for r in semantic_results {
        results.entry(r.source.clone())
            .and_modify(|existing: &mut RecallResult| {
                if r.relevance > existing.relevance {
                    *existing = r.clone();
                }
            })
            .or_insert(r);
    }
}

// 2. Keyword search (existing code)
// ...
```

- [ ] **Step 3: Build and verify**

Run: `cargo build --release 2>&1`

- [ ] **Step 4: Commit**

```bash
git add daemon-rs/src/handlers/recall.rs
git commit -m "feat(daemon-rs): semantic search in recall via embedded ONNX"
```

---

### Task 4: Knowledge Indexing on Startup

**Files:**
- Create: `daemon-rs/src/indexer.rs`
- Modify: `daemon-rs/src/main.rs`

Port brain.js `indexAll()` -- reads 6 knowledge sources and upserts into memories table.

- [ ] **Step 1: Create indexer.rs**

```rust
//! Knowledge indexer: reads filesystem sources and upserts into memories table.
//! Ported from Node.js brain.js indexAll().

use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};

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
    if text.is_empty() { return false; }

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
        // Invalidate stale embedding
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

// Source 1: state.md
fn index_state_file(conn: &Connection, home: &Path) -> usize {
    let state_path = home.join(".claude").join("state.md");
    if !state_path.exists() { return 0; }

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
    if text.is_empty() { None } else { Some(text.to_string()) }
}

// Source 2: Memory files (~/.claude/projects/C--Users-aditya/memory/*.md)
fn index_memory_files(conn: &Connection, home: &Path) -> usize {
    let mem_dir = home.join(".claude").join("projects").join("C--Users-aditya").join("memory");
    if !mem_dir.exists() { return 0; }

    let mut count = 0;
    let entries = match fs::read_dir(&mem_dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") { continue; }
        if path.file_name().and_then(|f| f.to_str()) == Some("MEMORY.md") { continue; }

        if let Ok(raw) = fs::read_to_string(&path) {
            let (fm, body) = parse_frontmatter(&raw);
            let name = fm.get("name").cloned()
                .unwrap_or_else(|| path.file_stem().unwrap_or_default().to_string_lossy().to_string());
            let mem_type = fm.get("type").cloned().unwrap_or_else(|| "memory".to_string());
            let desc = fm.get("description").cloned().unwrap_or_default();

            let text = if !desc.is_empty() {
                format!("[{}] ({}) {}\n{}", name, mem_type, desc, &body[..body.len().min(500)])
            } else {
                format!("[{}] ({})\n{}", name, mem_type, &body[..body.len().min(500)])
            };

            let source = format!("memory::{}", path.file_name().unwrap_or_default().to_string_lossy());
            if upsert_memory(conn, &text, &source, &mem_type, "indexer") {
                count += 1;
            }
        }
    }
    count
}

fn parse_frontmatter(raw: &str) -> (std::collections::HashMap<String, String>, String) {
    let mut fm = std::collections::HashMap::new();
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

// Source 3: Lessons (~/self-improvement-engine/lessons/lessons.jsonl)
fn index_lessons(conn: &Connection, home: &Path) -> usize {
    let path = home.join("self-improvement-engine").join("lessons").join("lessons.jsonl");
    if !path.exists() { return 0; }

    let content = match fs::read_to_string(&path) { Ok(c) => c, Err(_) => return 0 };
    let mut count = 0;

    for line in content.lines().filter(|l| !l.is_empty()) {
        if let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) {
            let lesson_type = entry["type"].as_str().unwrap_or("lesson");
            let lesson = entry["lesson"].as_str().unwrap_or("");
            let evidence = entry["evidence"].as_str().unwrap_or("");
            let skill = entry["skill"].as_str().unwrap_or("general");
            let ts = entry["timestamp"].as_str().unwrap_or("unknown");

            let text = if !evidence.is_empty() {
                format!("[{}] {} -- Evidence: {}", lesson_type, lesson, evidence)
            } else {
                format!("[{}] {}", lesson_type, lesson)
            };

            let source = format!("lessons::{}::{}", skill, ts);
            if upsert_memory(conn, &text, &source, "lesson", "indexer") { count += 1; }
        }
    }
    count
}

// Source 4: Goals (~/self-improvement-engine/tools/goal-setter/current-goals.json)
fn index_goals(conn: &Connection, home: &Path) -> usize {
    let path = home.join("self-improvement-engine").join("tools").join("goal-setter").join("current-goals.json");
    if !path.exists() { return 0; }

    let content = match fs::read_to_string(&path) { Ok(c) => c, Err(_) => return 0 };
    let data: serde_json::Value = match serde_json::from_str(&content) { Ok(d) => d, Err(_) => return 0 };

    let mut count = 0;
    if let Some(goals) = data["goals"].as_array() {
        for goal in goals {
            let rank = goal["rank"].as_u64().unwrap_or(0);
            let text_val = goal["goal"].as_str().unwrap_or("");
            let cat = goal["category"].as_str().unwrap_or("unknown");
            let priority = goal["priority"].as_f64().map(|p| format!("{:.2}", p)).unwrap_or_else(|| "?".to_string());
            let effort = goal["effort"].as_str().unwrap_or("?");

            let text = format!("[Goal #{}] {} (category: {}, priority: {}, effort: {})", rank, text_val, cat, priority, effort);
            let source = format!("goals::rank{}", rank);
            if upsert_memory(conn, &text, &source, "goal", "indexer") { count += 1; }
        }
    }
    count
}

// Source 5: Skill tracker (~/self-improvement-engine/tools/skill-tracker/invocations.jsonl)
fn index_skill_tracker(conn: &Connection, home: &Path) -> usize {
    let path = home.join("self-improvement-engine").join("tools").join("skill-tracker").join("invocations.jsonl");
    if !path.exists() { return 0; }

    let content = match fs::read_to_string(&path) { Ok(c) => c, Err(_) => return 0 };
    let mut by_skill: std::collections::HashMap<String, (u32, u32, u32, u32, String)> = std::collections::HashMap::new();

    for line in content.lines().filter(|l| !l.is_empty()) {
        if let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) {
            let skill = entry["skill"].as_str().unwrap_or("unknown").to_string();
            let outcome = entry["outcome"].as_str().unwrap_or("");
            let ts = entry["timestamp"].as_str().unwrap_or("").to_string();

            let stats = by_skill.entry(skill).or_insert((0, 0, 0, 0, String::new()));
            stats.0 += 1; // total
            match outcome {
                "success" => stats.1 += 1,
                "correction" => stats.2 += 1,
                "retry" => stats.3 += 1,
                _ => {}
            }
            if ts > stats.4 { stats.4 = ts; }
        }
    }

    let mut count = 0;
    for (skill, (total, success, correction, retry, last)) in &by_skill {
        let rate = if *total > 0 { (*success as f64 / *total as f64 * 100.0) as u32 } else { 0 };
        let text = format!("[Skill: {}] {} invocations, {}% success ({} corrections, {} retries). Last: {}", skill, total, rate, correction, retry, last);
        let source = format!("skills::{}", skill);
        if upsert_memory(conn, &text, &source, "skill_stats", "indexer") { count += 1; }
    }
    count
}

// Source 6: GORCI (~/self-improvement-engine/tools/gorci/last-run.json)
fn index_gorci(conn: &Connection, home: &Path) -> usize {
    let path = home.join("self-improvement-engine").join("tools").join("gorci").join("last-run.json");
    if !path.exists() { return 0; }

    let content = match fs::read_to_string(&path) { Ok(c) => c, Err(_) => return 0 };
    let data: serde_json::Value = match serde_json::from_str(&content) { Ok(d) => d, Err(_) => return 0 };

    let text = format!(
        "[GORCI] Skill: {}, Mode: {}, Tier: {}, Cases: {}, Pass: {}, Score: {}. Run: {}",
        data["skill"].as_str().unwrap_or("unknown"),
        data["mode"].as_str().unwrap_or("?"),
        data["tier"].as_str().unwrap_or("?"),
        data["cases"].as_u64().unwrap_or(0),
        data["pass"].as_str().or(data["pass"].as_u64().map(|_| "").as_deref()).unwrap_or("?"),
        data["overallScore"].as_str().or(data["overallScore"].as_f64().map(|_| "").as_deref()).unwrap_or("?"),
        data["timestamp"].as_str().unwrap_or("unknown"),
    );

    if upsert_memory(conn, &text, "gorci::last-run", "gorci", "indexer") { 1 } else { 0 }
}

/// Score decay pass: apply 0.95^days to all entries.
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
```

- [ ] **Step 2: Register module and call on startup**

In `main.rs`, add `mod indexer;` and in the serve/mcp startup paths, after DB init:

```rust
// Index knowledge sources + decay pass
{
    let conn = state.db.lock().await;
    let home = dirs::home_dir().unwrap_or_default();
    let indexed = crate::indexer::index_all(&conn, &home);
    let decayed = crate::indexer::decay_pass(&conn);
    eprintln!("[cortex] Indexed {} entries, decayed {} scores", indexed, decayed);
}
```

- [ ] **Step 3: Build and verify**

Run: `cargo build --release 2>&1`

- [ ] **Step 4: Commit**

```bash
git add daemon-rs/src/indexer.rs daemon-rs/src/main.rs
git commit -m "feat(daemon-rs): knowledge indexing (6 sources) + score decay on startup"
```

---

### Task 5: Background Embedding Builder

**Files:**
- Modify: `daemon-rs/src/main.rs`

Port buildEmbeddings: find memories/decisions without embeddings, generate them in background.

- [ ] **Step 1: Add background embedding builder**

In main.rs, after indexing, spawn a background task:

```rust
// Build embeddings for un-embedded entries (background, non-blocking)
if let Some(engine) = state.embedding_engine.clone() {
    let db = state.db.clone();
    tokio::spawn(async move {
        let conn = db.lock().await;

        // Find un-embedded memories
        let mut stmt = conn.prepare(
            "SELECT m.id, m.text FROM memories m
             WHERE m.status = 'active'
               AND NOT EXISTS (SELECT 1 FROM embeddings e WHERE e.target_type = 'memory' AND e.target_id = m.id)"
        ).unwrap();
        let unembedded: Vec<(i64, String)> = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?))
        }).unwrap().flatten().collect();

        // Find un-embedded decisions
        let mut stmt2 = conn.prepare(
            "SELECT d.id, d.decision FROM decisions d
             WHERE d.status = 'active'
               AND NOT EXISTS (SELECT 1 FROM embeddings e WHERE e.target_type = 'decision' AND e.target_id = d.id)"
        ).unwrap();
        let unembedded_dec: Vec<(i64, String)> = stmt2.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?))
        }).unwrap().flatten().collect();

        let total = unembedded.len() + unembedded_dec.len();
        if total == 0 { return; }

        eprintln!("[embeddings] Building embeddings for {} entries...", total);
        let mut computed = 0;

        for (id, text) in &unembedded {
            if let Some(vec) = engine.embed(text) {
                let blob = crate::embeddings::vector_to_blob(&vec);
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO embeddings (target_type, target_id, vector, model) VALUES ('memory', ?1, ?2, 'all-MiniLM-L6-v2')",
                    rusqlite::params![id, blob],
                );
                computed += 1;
            }
        }

        for (id, text) in &unembedded_dec {
            if let Some(vec) = engine.embed(text) {
                let blob = crate::embeddings::vector_to_blob(&vec);
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO embeddings (target_type, target_id, vector, model) VALUES ('decision', ?1, ?2, 'all-MiniLM-L6-v2')",
                    rusqlite::params![id, blob],
                );
                computed += 1;
            }
        }

        eprintln!("[embeddings] Built {}/{} embeddings", computed, total);
    });
}
```

- [ ] **Step 2: Build and verify**

Run: `cargo build --release 2>&1`

- [ ] **Step 3: Commit**

```bash
git add daemon-rs/src/main.rs
git commit -m "feat(daemon-rs): background embedding builder on startup"
```

---

### Task 6: MCP Boot Uses Full Compiler

**Files:**
- Modify: `daemon-rs/src/handlers/mcp.rs`

Replace the hardcoded MCP boot stub with the full capsule compiler.

- [ ] **Step 1: Wire full compiler into MCP boot tool**

In `mcp.rs`, find the `cortex_boot` handler (around line 153). Replace the hardcoded response with:

```rust
// Use the full capsule compiler (same as HTTP /boot)
let agent = params.get("agent").and_then(|v| v.as_str()).unwrap_or("claude");
let profile = params.get("profile").and_then(|v| v.as_str()).unwrap_or("full");
let conn = state.db.lock().await;
let home = dirs::home_dir().unwrap_or_default();
let boot = crate::compiler::compile_boot(&conn, agent, profile, &home);
// Return the full boot prompt with liveness fields
```

- [ ] **Step 2: Build and verify**

- [ ] **Step 3: Commit**

```bash
git add daemon-rs/src/handlers/mcp.rs
git commit -m "fix(daemon-rs): MCP boot uses full capsule compiler instead of hardcoded stub"
```

---

### Task 7: Stale Daemon Kill on Startup

**Files:**
- Modify: `daemon-rs/src/auth.rs`
- Modify: `daemon-rs/src/main.rs`

Kill stale daemon process before binding port.

- [ ] **Step 1: Add kill_stale_daemon to auth.rs**

```rust
/// Kill a stale daemon process if PID file exists and process is alive.
pub fn kill_stale_daemon() {
    let pid_path = cortex_dir().join("cortex.pid");
    if !pid_path.exists() { return; }

    if let Ok(pid_str) = std::fs::read_to_string(&pid_path) {
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            // Don't kill ourselves
            if pid == std::process::id() { return; }

            #[cfg(windows)]
            {
                use std::process::Command;
                let _ = Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/F"])
                    .output();
            }

            #[cfg(unix)]
            {
                unsafe { libc::kill(pid as i32, libc::SIGTERM); }
            }

            eprintln!("[cortex] Killed stale daemon (PID {})", pid);
            let _ = std::fs::remove_file(&pid_path);

            // Brief pause for port release
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
}
```

- [ ] **Step 2: Call in main.rs before server::run**

```rust
auth::kill_stale_daemon();
```

- [ ] **Step 3: Build, verify, commit**

```bash
git add daemon-rs/src/auth.rs daemon-rs/src/main.rs
git commit -m "fix(daemon-rs): kill stale daemon process on startup"
```

---

### Task 8: Missing Conductor Features

**Files:**
- Modify: `daemon-rs/src/handlers/conductor.rs`
- Modify: `daemon-rs/src/handlers/boot.rs`

Add: task auto-post to feed, feed auto-ack on boot.

- [ ] **Step 1: Task completion auto-posts to feed**

In `handle_complete_task`, after marking the task complete, insert a feed entry:

```rust
// Auto-post task completion to feed
let _ = conn.execute(
    "INSERT INTO feed (type, agent, payload) VALUES ('task_complete', ?1, ?2)",
    rusqlite::params![agent, serde_json::json!({
        "task_id": task_id,
        "summary": summary,
    }).to_string()],
);
```

- [ ] **Step 2: Feed auto-ack on boot**

In `handlers/boot.rs`, after compiling the boot prompt, mark all feed entries as read for the booting agent:

```rust
// Auto-ack all unread feed for this agent
let _ = conn.execute(
    "INSERT OR IGNORE INTO feed_acks (feed_id, agent) SELECT id, ?1 FROM feed WHERE id NOT IN (SELECT feed_id FROM feed_acks WHERE agent = ?1)",
    [&agent],
);
```

- [ ] **Step 3: Build, verify, commit**

```bash
git add daemon-rs/src/handlers/conductor.rs daemon-rs/src/handlers/boot.rs
git commit -m "feat(daemon-rs): task auto-post to feed + feed auto-ack on boot"
```

---

### Task 9: Ollama Health Check

**Files:**
- Modify: `daemon-rs/src/handlers/health.rs`

Check if Ollama is running and report real status.

- [ ] **Step 1: Add Ollama health check**

In `handle_health`, replace the hardcoded `"offline"` with a real check:

```rust
// Check Ollama status
let ollama_status = match reqwest::Client::new()
    .get("http://localhost:11434/api/tags")
    .timeout(std::time::Duration::from_secs(2))
    .send()
    .await
{
    Ok(resp) if resp.status().is_success() => "online",
    _ => "offline",
};
```

- [ ] **Step 2: Build, verify, commit**

```bash
git add daemon-rs/src/handlers/health.rs
git commit -m "feat(daemon-rs): real Ollama health check instead of hardcoded offline"
```

---

### Task 10: MCP-over-HTTP + CORS + Polish

**Files:**
- Modify: `daemon-rs/src/server.rs`
- Modify: `daemon-rs/src/main.rs`

Add MCP-over-HTTP transport (POST /mcp, GET /mcp SSE), CORS OPTIONS, diary event logging.

- [ ] **Step 1: Add CORS middleware**

Add `tower-http` to Cargo.toml:

```toml
tower-http = { version = "0.6", features = ["cors"] }
```

In `server.rs`, wrap the router:

```rust
use tower_http::cors::{CorsLayer, Any};

let cors = CorsLayer::new()
    .allow_origin(Any)
    .allow_methods(Any)
    .allow_headers(Any);

Router::new()
    // ... routes ...
    .layer(cors)
    .with_state(state)
```

- [ ] **Step 2: Add MCP-over-HTTP routes**

Add to server.rs:

```rust
.route("/mcp", post(handlers::mcp::handle_mcp_http).get(handlers::mcp::handle_mcp_sse))
```

Implement in `handlers/mcp.rs` -- parse JSON-RPC request, dispatch to appropriate handler, return JSON-RPC response.

- [ ] **Step 3: Add diary event logging**

In `handlers/diary.rs`, after writing state.md, log an event:

```rust
let _ = conn.execute(
    "INSERT INTO events (type, agent, payload) VALUES ('diary_write', ?1, ?2)",
    rusqlite::params![agent, serde_json::json!({"sections": sections_written}).to_string()],
);
```

- [ ] **Step 4: Model download on first use**

In main.rs startup, spawn model download if not present:

```rust
// Download embedding model if needed (background, non-blocking)
tokio::spawn(async {
    if let Some(dir) = crate::embeddings::ensure_model_downloaded().await {
        eprintln!("[embeddings] Model ready at {}", dir.display());
    }
});
```

- [ ] **Step 5: Full build + test**

```bash
cargo build --release 2>&1
# Test: start Rust daemon on port 7438, hit all endpoints
./target/release/cortex serve &
curl -s http://localhost:7437/health | python -m json.tool
curl -s "http://localhost:7437/recall?q=cortex+architecture" | python -m json.tool
curl -s http://localhost:7437/boot?agent=claude | head -20
```

- [ ] **Step 6: Final commit**

```bash
git add daemon-rs/
git commit -m "feat(daemon-rs): MCP-over-HTTP, CORS, diary events, model auto-download -- full Node parity"
```

---

## Post-Completion: Validation

After all 10 tasks, run the full parity check:

1. Start Rust daemon: `./daemon-rs/target/release/cortex serve`
2. Hit every endpoint and compare output to Node daemon
3. Run `/codex:review` on the Rust codebase
4. Verify: `curl localhost:7437/health` shows embeddings count > 0 and Ollama status is real
5. Verify: `curl "localhost:7437/recall?q=test"` returns semantic results (not just keyword)
6. Verify: `curl localhost:7437/boot?agent=claude` returns full capsule boot prompt

Once validated, update `cortex-start.bat` to launch the Rust binary and register as Windows Service.
