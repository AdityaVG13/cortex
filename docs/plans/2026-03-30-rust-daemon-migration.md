# Cortex Rust Daemon Migration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract the embedded Rust daemon from the Tauri app into a standalone binary that replaces the Node.js daemon as the production Cortex server.

**Architecture:** The existing Tauri embedded daemon (`embedded_daemon.rs`, 4,278 lines) is a near-complete port of the Node.js daemon. We extract it into a standalone Cargo project at `cortex/daemon-rs/`, decouple it from Tauri, add the 4 missing endpoints, fix known bugs, and add MCP stdio transport. The Node.js daemon remains as fallback during migration.

**Tech Stack:** Rust 1.94+, Axum 0.8, Tokio, rusqlite (bundled SQLite), serde_json, chrono, uuid

---

## File Structure

```
cortex/daemon-rs/
  Cargo.toml              — Standalone binary, no Tauri dependency
  src/
    main.rs               — Entry point: CLI args (serve/mcp), signal handlers, startup
    server.rs             — Axum router, HTTP server setup, graceful shutdown
    state.rs              — RuntimeState struct, initialization
    db.rs                 — SQLite connection, schema, CRUD, WAL config
    handlers/
      mod.rs              — Re-exports all handlers
      boot.rs             — GET /boot (compiler + profile loading)
      recall.rs           — GET /recall, GET /recall/budget, GET /peek
      store.rs            — POST /store (conflict detection, surprise scoring)
      health.rs           — GET /health, GET /digest, GET /savings, GET /dump
      mutate.rs           — POST /forget, POST /resolve, POST /archive
      diary.rs            — POST /diary (state.md writer) [NEW]
      shutdown.rs         — POST /shutdown [NEW]
      conductor.rs        — Locks, activity, messages, sessions, tasks
      feed.rs             — Feed CRUD + ack
      events.rs           — SSE event stream
      mcp.rs              — MCP HTTP transport (JSON-RPC over POST)
    mcp_stdio.rs          — MCP stdio transport (JSON-RPC over stdin/stdout) [NEW]
    compiler.rs           — Boot prompt compiler (profiles, capsules, token budgets)
    conflict.rs           — Jaccard-only conflict detection (Ollama cosine deferred to M1)
    co_occurrence.rs      — Co-occurrence recording + prediction + reset
    auth.rs               — Token generation, validation
    logging.rs            — Log file writer
```

**Source material:**
- `desktop/cortex-control-center/src-tauri/src/embedded_daemon.rs` (4,278 lines — primary source)
- `src/daemon.js` (2,378 lines — reference for missing features)
- `src/brain.js` (1,042 lines — reference for diary, budget recall, archive)
- `src/compiler.js` (707 lines — reference for boot compiler)

---

### Task 1: Scaffold Rust Project

**Files:**
- Create: `daemon-rs/Cargo.toml`
- Create: `daemon-rs/src/main.rs`

- [ ] **Step 1: Create Cargo.toml**

```toml
[package]
name = "cortex-daemon"
version = "2.1.0"
edition = "2021"
description = "Cortex memory daemon — standalone Rust binary"

[[bin]]
name = "cortex"
path = "src/main.rs"

[dependencies]
axum = { version = "0.8.4", features = ["macros"] }
chrono = { version = "0.4.42", features = ["serde", "clock"] }
futures-util = "0.3.31"
regex = "1.12.2"
rusqlite = { version = "0.37.0", features = ["bundled"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1.48.0", features = ["rt-multi-thread", "macros", "sync", "time", "net", "signal", "io-std"] }
tokio-stream = { version = "0.1.17", features = ["sync"] }
uuid = { version = "1.18.1", features = ["v4", "serde"] }

[profile.release]
opt-level = 3
lto = true
strip = true
```

> **Note:** Versions pinned to match the Tauri build (Cargo.toml in `src-tauri/`) to ensure identical rusqlite/SQLite behavior.

- [ ] **Step 2: Create minimal main.rs that compiles**

```rust
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("");

    match mode {
        "serve" => println!("Starting HTTP daemon..."),
        "mcp" => println!("Starting MCP + HTTP daemon..."),
        _ => {
            eprintln!("Usage: cortex <serve|mcp>");
            std::process::exit(1);
        }
    }
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cd C:/Users/aditya/cortex/daemon-rs && cargo build`
Expected: Compiles successfully, produces `target/debug/cortex.exe`

- [ ] **Step 4: Commit**

```bash
git add daemon-rs/Cargo.toml daemon-rs/src/main.rs
git commit -m "feat: scaffold standalone Rust daemon project"
```

---

### Task 2: Extract DB Layer

**Files:**
- Create: `daemon-rs/src/db.rs`
- Create: `daemon-rs/src/co_occurrence.rs`

- [ ] **Step 1: Extract SQLite setup from embedded_daemon.rs**

Copy `configure_sqlite()`, `initialize_schema()`, `checkpoint_wal()`, `checkpoint_wal_best_effort()` from `embedded_daemon.rs:321-489`. Wrap connection in dedicated module with:
- `pub fn open(path: &Path) -> Result<Connection>`
- `pub fn configure(conn: &Connection) -> Result<()>`
- `pub fn initialize_schema(conn: &Connection) -> Result<()>`
- `pub fn checkpoint_wal(conn: &Connection) -> Result<()>`

- [ ] **Step 2: Add integrity check on open**

```rust
pub fn verify_integrity(conn: &Connection) -> Result<bool> {
    let result: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    Ok(result == "ok")
}
```

- [ ] **Step 3: Create co_occurrence.rs with the missing reset function**

```rust
pub fn reset_co_occurrence_table(conn: &Connection) -> Result<()> {
    conn.execute_batch("DELETE FROM co_occurrence;")?;
    Ok(())
}
```

Also extract `record_co_occurrence()` and `predict_from_co_occurrence()` from embedded_daemon.rs:3055-3148.

- [ ] **Step 4: Write test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn test_open_and_schema() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        initialize_schema(&conn).unwrap();
        assert!(verify_integrity(&conn).unwrap());
    }

    #[test]
    fn test_co_occurrence_reset() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        initialize_schema(&conn).unwrap();
        reset_co_occurrence_table(&conn).unwrap();
    }
}
```

- [ ] **Step 5: Verify tests pass**

Run: `cd daemon-rs && cargo test`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
git add daemon-rs/src/db.rs daemon-rs/src/co_occurrence.rs
git commit -m "feat: extract DB layer with integrity checks and co_occurrence fix"
```

---

### Task 3: Extract State + Auth

**Files:**
- Create: `daemon-rs/src/state.rs`
- Create: `daemon-rs/src/auth.rs`

- [ ] **Step 1: Extract RuntimeState from embedded_daemon.rs:87-125**

Port the `RuntimeState` struct and `initialize_state()`. Add `shutdown_tx: Arc<tokio::sync::Mutex<Option<oneshot::Sender<()>>>>` for graceful shutdown (used by `/shutdown` handler and signal handling). The `oneshot::Receiver` is returned separately for use as `with_graceful_shutdown()` future.

- [ ] **Step 2: Extract auth from embedded_daemon.rs**

Port token generation (`Uuid::new_v4`), token file write to `~/.cortex/cortex.token`, PID file write, and `validate_auth()` header check.

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`

- [ ] **Step 4: Commit**

```bash
git add daemon-rs/src/state.rs daemon-rs/src/auth.rs
git commit -m "feat: extract runtime state and auth module"
```

---

### Task 4: Extract HTTP Server + Core Handlers

**Files:**
- Create: `daemon-rs/src/server.rs`
- Create: `daemon-rs/src/handlers/mod.rs`
- Create: `daemon-rs/src/handlers/health.rs`
- Create: `daemon-rs/src/handlers/store.rs`
- Create: `daemon-rs/src/handlers/recall.rs`
- Create: `daemon-rs/src/handlers/boot.rs`

- [ ] **Step 1: Create server.rs with Axum router**

Port `build_router()` from embedded_daemon.rs:235-269. Separate into server.rs with:
- `pub fn build_router(state: RuntimeState) -> Router`
- `pub async fn run(router: Router, shutdown: impl Future)` — binds to 127.0.0.1:7437

- [ ] **Step 2: Extract core handlers (health, store, recall, boot)**

Port these handlers from embedded_daemon.rs, one file per domain:
- `health.rs`: handle_health, handle_digest, handle_savings, handle_dump
- `store.rs`: handle_store (lines 1148-1187)
- `recall.rs`: handle_recall (lines 1114-1147), handle_peek (lines 758-999)
- `boot.rs`: handle_boot (lines 1000-1113)

- [ ] **Step 3: Wire main.rs to start the server**

```rust
#[tokio::main]
async fn main() {
    // ... arg parsing ...
    let state = state::initialize().expect("Failed to initialize state");
    let router = server::build_router(state);
    server::run(router, signal::ctrl_c()).await;
}
```

- [ ] **Step 4: Add DB path verification log**

DB path must resolve to `C:\Users\aditya\cortex\cortex.db` regardless of working directory. Add startup log:
```rust
eprintln!("[cortex] DB: {}", db_path.display());
```

- [ ] **Step 5: Verify it compiles and starts**

Run: `cargo build && cargo run -- serve`
Expected: "Listening on http://127.0.0.1:7437" and "DB: C:\Users\aditya\cortex\cortex.db"

- [ ] **Step 6: Smoke test**

Run: `curl -s http://localhost:7437/health`
Expected: `{"status":"ok","stats":{...}}`

- [ ] **Step 6: Commit**

```bash
git add daemon-rs/src/server.rs daemon-rs/src/handlers/
git commit -m "feat: extract HTTP server with core handlers (health, store, recall, boot)"
```

---

### Task 5: Extract Remaining Handlers

**Files:**
- Create: `daemon-rs/src/handlers/mutate.rs`
- Create: `daemon-rs/src/handlers/conductor.rs`
- Create: `daemon-rs/src/handlers/feed.rs`
- Create: `daemon-rs/src/handlers/events.rs`

- [ ] **Step 1: Port mutate handlers**

From embedded_daemon.rs: handle_forget, handle_resolve. Also implement:
- `handle_archive` (from Node.js daemon.js:753): accepts `{ type: "memories"|"decisions", ids: [1,2,3] }`, sets `status = 'archived'` for matching rows.
- Add `archive_entries(conn, table, ids) -> Result<usize>` to `db.rs`.
- Add `.route("/archive", post(handle_archive))` to `server.rs`.

- [ ] **Step 2: Port conductor handlers**

From embedded_daemon.rs: locks (handle_lock, handle_unlock, handle_locks), activity, messages, sessions (start/heartbeat/end/list), tasks (create/get/claim/complete/abandon/next).

- [ ] **Step 3: Port feed + events handlers**

From embedded_daemon.rs: handle_post_feed, handle_get_feed, handle_get_feed_by_id, handle_feed_ack, handle_events_stream (SSE).

- [ ] **Step 4: Smoke test all endpoints**

```bash
TOKEN=$(cat ~/.cortex/cortex.token)
curl -s localhost:7437/health
curl -s localhost:7437/sessions
curl -s localhost:7437/tasks
curl -s localhost:7437/feed
curl -s localhost:7437/locks
```

- [ ] **Step 5: Commit**

```bash
git add daemon-rs/src/handlers/
git commit -m "feat: port all remaining handlers (mutate, conductor, feed, events)"
```

---

### Task 5.5: Implement Jaccard Conflict Detection

**Files:**
- Create: `daemon-rs/src/conflict.rs`

The Rust embedded daemon **skips conflict detection entirely** — `handle_store` calls `store_decision` directly without checking for duplicates. The Node.js `conflict.js` uses Ollama cosine similarity with Jaccard fallback. For cutover, implement Jaccard-only (pure text math, no external deps). Cosine via Ollama is deferred to M1.

- [ ] **Step 1: Port Jaccard similarity from Node.js conflict.js:15-27**

```rust
pub fn jaccard_similarity(a: &str, b: &str) -> f64 {
    let set_a: HashSet<&str> = a.split_whitespace().collect();
    let set_b: HashSet<&str> = b.split_whitespace().collect();
    let intersection = set_a.intersection(&set_b).count() as f64;
    let union = set_a.union(&set_b).count() as f64;
    if union == 0.0 { return 0.0; }
    intersection / union
}

pub struct ConflictResult {
    pub is_conflict: bool,
    pub is_update: bool,
    pub matched_id: Option<i64>,
    pub matched_agent: Option<String>,
}

pub fn detect_conflict(conn: &Connection, decision: &str, source_agent: &str) -> ConflictResult {
    // Check last 50 active decisions for similarity
    // Same agent + sim > 0.7 = update (supersede)
    // Different agent + sim > 0.7 = conflict (disputed)
    // Otherwise = new
}
```

- [ ] **Step 2: Wire into store handler**

Call `detect_conflict()` before `store_decision()` in `handlers/store.rs`.

- [ ] **Step 3: Test**

```rust
#[test]
fn test_jaccard_similarity() {
    assert!(jaccard_similarity("hello world foo", "hello world bar") > 0.3);
    assert!(jaccard_similarity("hello world foo", "hello world foo") > 0.99);
    assert!(jaccard_similarity("completely different", "nothing alike here") < 0.1);
}
```

- [ ] **Step 4: Commit**

```bash
git add daemon-rs/src/conflict.rs
git commit -m "feat: add Jaccard conflict detection for store dedup"
```

---

### Task 6: Add Missing Endpoints

**Files:**
- Create: `daemon-rs/src/handlers/diary.rs`
- Create: `daemon-rs/src/handlers/shutdown.rs`
- Modify: `daemon-rs/src/handlers/recall.rs` (add budget recall)

- [ ] **Step 1: Implement diary handler**

Port from Node.js brain.js `writeDiary()` (line 700+). Reads `~/.claude/state.md`, preserves permanent sections (`## DO NOT REMOVE`), updates dynamic sections (What Was Done, Next Session, Pending, Known Issues, Key Decisions).

- [ ] **Step 2: Implement shutdown handler**

`RuntimeState` must include a `shutdown_tx: Arc<tokio::sync::Mutex<Option<oneshot::Sender<()>>>>`. The oneshot receiver is passed to `axum::serve().with_graceful_shutdown()`. The handler takes the sender:

```rust
async fn handle_shutdown(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if !validate_auth(&headers, &state.token) {
        return json_error(StatusCode::UNAUTHORIZED, "Unauthorized");
    }
    // Take the sender (can only fire once)
    if let Some(tx) = state.shutdown_tx.lock().await.take() {
        let _ = tx.send(());
    }
    json_response(StatusCode::OK, json!({ "shutdown": true }))
}
```

- [ ] **Step 3: Add budget recall**

Port from Node.js daemon.js:570. Budget recall truncates results to fit within a token budget.

- [ ] **Step 4: Smoke test new endpoints**

```bash
TOKEN=$(cat ~/.cortex/cortex.token)
curl -s -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  localhost:7437/diary -d '{"accomplished":"test"}'
curl -s "localhost:7437/recall/budget?q=test&budget=200"
```

- [ ] **Step 5: Commit**

```bash
git add daemon-rs/src/handlers/diary.rs daemon-rs/src/handlers/shutdown.rs
git commit -m "feat: add missing endpoints (diary, shutdown, budget recall)"
```

---

### Task 7: Extract Boot Compiler

**Files:**
- Create: `daemon-rs/src/compiler.rs`

- [ ] **Step 1: Port compiler logic**

The boot compiler is the most complex piece. It reads memory files from `~/.claude/projects/*/memory/`, state.md, lessons from `~/self-improvement-engine/`, and compresses them into a token-budgeted boot prompt. Port from:
- `embedded_daemon.rs` handle_boot (lines 1000-1113) for the Rust version
- `src/compiler.js` (707 lines) for the full Node.js reference

Key functions: `compile_boot_prompt(agent, budget)`, `compile_capsules()`, `estimate_tokens()`.

- [ ] **Step 2: Test compiler output matches Node.js**

```bash
# Compare outputs
curl -s "localhost:7437/boot?agent=claude-opus&budget=600" > /tmp/rust_boot.json
# (start Node.js daemon on different port or compare saved output)
```

- [ ] **Step 3: Commit**

```bash
git add daemon-rs/src/compiler.rs
git commit -m "feat: port boot prompt compiler with capsule system"
```

---

### Task 8: Add MCP Stdio Transport

**Files:**
- Create: `daemon-rs/src/mcp_stdio.rs`
- Modify: `daemon-rs/src/main.rs`

- [ ] **Step 1: Implement MCP stdio transport**

Read JSON-RPC lines from stdin, dispatch to same handler as HTTP MCP, write responses to stdout. Redirect all other output (logs, errors) to stderr or log file.

```rust
pub async fn run_mcp_stdio(state: RuntimeState) {
    let stdin = tokio::io::stdin();
    let reader = tokio::io::BufReader::new(stdin);
    let mut lines = reader.lines();

    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() { continue; }
        let msg: Value = match serde_json::from_str(&line) { ... };
        let response = handle_mcp_message(&state, &msg).await;
        if let Some(resp) = response {
            println!("{}", serde_json::to_string(&resp).unwrap());
        }
    }
}
```

- [ ] **Step 2: Wire into main.rs mcp mode**

In `mcp` mode: start both HTTP server AND stdio transport concurrently. Redirect stdout/stderr to log file (only stdio transport writes to real stdout).

- [ ] **Step 3: Test MCP registration**

```bash
# Register with Claude Code
claude mcp add cortex-rs -s user -- C:\Users\aditya\cortex\daemon-rs\target\release\cortex.exe mcp
```

- [ ] **Step 4: Commit**

```bash
git add daemon-rs/src/mcp_stdio.rs daemon-rs/src/main.rs
git commit -m "feat: add MCP stdio transport for Claude Code integration"
```

---

### Task 9: Signal Handling + Graceful Shutdown

**Files:**
- Modify: `daemon-rs/src/main.rs`
- Modify: `daemon-rs/src/server.rs`

- [ ] **Step 1: Add proper signal handling**

```rust
async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(SignalKind::terminate()).unwrap();

    tokio::select! {
        _ = ctrl_c => {},
        #[cfg(unix)]
        _ = sigterm.recv() => {},
    }
}
```

- [ ] **Step 2: WAL checkpoint + DB close on shutdown**

After graceful shutdown signal, before process exit:
```rust
{
    let conn = state.db.lock().await;
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").ok();
    conn.execute_batch("PRAGMA optimize;").ok();
}
```

- [ ] **Step 3: Test graceful shutdown**

```bash
# Start daemon, store something, kill -TERM, verify data persists
cargo run -- serve &
PID=$!
TOKEN=$(cat ~/.cortex/cortex.token)
curl -s -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  localhost:7437/store -d '{"decision":"shutdown test","context":"test"}'
kill $PID
sleep 2
# Verify data on disk with Python sqlite3
```

- [ ] **Step 4: Commit**

```bash
git add daemon-rs/src/main.rs daemon-rs/src/server.rs
git commit -m "feat: graceful shutdown with WAL checkpoint and signal handling"
```

---

### Task 10: Integration Testing + Cutover

**Files:**
- Create: `daemon-rs/tests/integration.rs`
- Modify: `cortex-start.bat`

- [ ] **Step 1: Write integration tests**

Test all critical paths: store → recall cycle, conflict detection, boot compilation, diary write, feed posting, task lifecycle, MCP message handling.

- [ ] **Step 2: Build release binary**

```bash
cd daemon-rs && cargo build --release
ls -la target/release/cortex.exe
```

- [ ] **Step 3: Run Node.js test suite against Rust daemon**

Start the Rust daemon and run the existing Node.js test suite (`npm test`) to verify API compatibility.

- [ ] **Step 4: Update cortex-start.bat**

Switch the launcher to use the Rust binary:
```batch
@echo off
start "" "C:\Users\aditya\cortex\daemon-rs\target\release\cortex.exe" serve
```

- [ ] **Step 5: Update MCP registration**

```bash
claude mcp remove cortex
claude mcp add cortex -s user -- C:\Users\aditya\cortex\daemon-rs\target\release\cortex.exe mcp
```

- [ ] **Step 6: Smoke test full system**

```bash
# Boot brain
node ~/.claude/hooks/brain-boot.js
# Store and recall
curl -s -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  localhost:7437/store -d '{"decision":"Rust daemon live","context":"migration complete"}'
curl -s "localhost:7437/recall?q=rust+daemon"
```

- [ ] **Step 7: Commit**

```bash
git add daemon-rs/ cortex-start.bat
git commit -m "feat: Rust daemon production-ready, replace Node.js as default"
```

---

## Migration Strategy

1. **Phase 1 (Tasks 1-5):** Extract + compile. Rust daemon runs alongside Node.js.
2. **Phase 2 (Tasks 6-8):** Feature parity. All endpoints + MCP stdio working.
3. **Phase 3 (Tasks 9-10):** Hardening + cutover. Signal handling, integration tests, swap default.

The Node.js daemon (`src/daemon.js`) stays in the repo as fallback. Remove it in a future cleanup pass after 1+ week of Rust daemon stability.

## Known Risks

- **Boot compiler complexity**: The capsule system + profile loading + memory file indexing is the hardest port. Budget extra time.
- **MCP stdio on Windows**: Must handle Windows line endings and stdin buffering carefully.
- **Database migration**: Both daemons use the same `cortex.db`. The Rust daemon uses native rusqlite (WAL mode); Node.js uses sql.js (in-memory). They can't run simultaneously against the same DB file.
