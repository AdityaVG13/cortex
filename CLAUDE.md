# Cortex v2 — Universal AI Brain

## What This Is
Multi-transport memory daemon serving Claude Code, Gemini CLI, Codex CLI, Cursor, and local LLMs.
Persistent memory with semantic search, conflict resolution, and per-agent compiler profiles.

## Quick Start (Rust daemon -- primary)
```bash
cortex-start              # Start daemon (uses Rust binary)
cortex-start stop         # Stop daemon
cortex-start status       # Check health
```

The Rust daemon is at `daemon-rs/target/release/cortex.exe`. It includes:
- In-process ONNX embeddings (384-dim, all-MiniLM-L6-v2) -- no Ollama needed for search
- Knowledge indexing from 6 filesystem sources on startup
- Score decay, background embedding builder, SQLite-backed conductor

## Architecture (Rust)
```
daemon-rs/src/
  main.rs          # Entry point, startup orchestration, embedding builder
  server.rs        # Axum router, CORS middleware
  state.rs         # RuntimeState (DB, auth, embeddings, SSE, caches)
  embeddings.rs    # In-process ONNX engine (model download, embed, cosine)
  indexer.rs       # Knowledge indexer (6 sources) + score decay
  compiler.rs      # Capsule boot prompt compiler
  conflict.rs      # Conflict detection (cosine + Jaccard)
  co_occurrence.rs # Co-occurrence tracking + prediction
  auth.rs          # Token, PID, stale daemon kill
  db.rs            # SQLite schema, WAL, integrity checks
  mcp_stdio.rs     # MCP stdio transport
  handlers/        # HTTP endpoint handlers (store, recall, boot, etc.)
```

## Legacy (Node.js -- kept for reference)
```
src/
  daemon.js, brain.js, embeddings.js, compiler.js, conflict.js, db.js, cli.js
```

## Key Conventions
- Rust (axum + rusqlite + ort), single binary, zero runtime deps
- SQLite stored at cortex.db (project root)
- Daemon PID/token/logs at ~/.cortex/
- Embedding model at ~/.cortex/models/ (auto-downloaded on first run)
- Port 7437 (HTTP), Ollama at 11434 (optional -- for LLM tasks only)
- Conflict check: cosine (0.85) first, Jaccard (0.6) fallback
- Auth token required for POST endpoints via HTTP

## Testing
```bash
cargo build --release    # Build Rust daemon
cargo test               # Run Rust tests
curl localhost:7437/health
```

## Brain Boot (do this FIRST)
Even when editing Cortex code, connect to the running daemon:
1. Call `cortex_boot()` -- print the brain status line
2. Before changes: `cortex_recall("topic")` to check prior context
3. After changes: `cortex_store(decision, context)` and confirm: "Stored to Cortex: [summary]"

## MCP Registration
```bash
claude mcp add cortex -s user -- C:\Users\aditya\cortex\daemon-rs\target\release\cortex.exe mcp
```
