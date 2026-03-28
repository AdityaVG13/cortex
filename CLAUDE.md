# Cortex v2 — Universal AI Brain

## What This Is
Multi-transport memory daemon serving Claude Code, Gemini CLI, Codex CLI, Cursor, and local LLMs.
Persistent memory with semantic search, conflict resolution, and per-agent compiler profiles.

## Quick Start
```bash
npm install          # sql.js (only dependency)
node src/daemon.js serve   # Start HTTP daemon at localhost:7437
node src/daemon.js mcp     # Start MCP mode (HTTP + MCP stdio)
node src/cli.js boot       # CLI (auto-starts daemon)
```

## Architecture
```
src/
  daemon.js     # HTTP + MCP server, daemon lifecycle, auth
  brain.js      # Core: indexAll, recall, store, forget, writeDiary
  embeddings.js # Ollama nomic-embed-text vectors, cosine similarity
  compiler.js   # Per-profile boot prompt compilation with token budgets
  conflict.js   # Conflict detection (cosine + Jaccard fallback), resolution
  profiles.js   # Profile loader (full/operational/subagent/index)
  db.js         # SQLite via sql.js, schema, CRUD helpers
  cli.js        # CLI wrapper, auto-start daemon, HTTP client
```

## Key Conventions
- Node.js, zero external deps except sql.js
- SQLite stored at cortex.db (project root)
- Daemon PID/token/logs at ~/.cortex/
- Port 7437 (HTTP), Ollama at 11434
- All paths use process.env.USERPROFILE for Windows
- Conflict check runs BEFORE surprise filter on store
- Auth token required for POST endpoints via HTTP

## Testing
```bash
npm test             # node:test runner
curl localhost:7437/health
node src/cli.js health
```

## Brain Boot (do this FIRST)
Even when editing Cortex code, connect to the running daemon:
1. Call `cortex_boot()` — print the brain status line
2. Before changes: `cortex_recall("topic")` to check prior context
3. After changes: `cortex_store(decision, context)` and confirm: "Stored to Cortex: [summary]"

## MCP Registration
```bash
claude mcp add cortex -s user -- node C:\Users\aditya\cortex\src\daemon.js mcp
```
