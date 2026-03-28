# Cortex v2 Agent Instructions

## Project
Universal AI Brain — multi-transport memory daemon for all AI clients.

## Stack
Node.js 24+, sql.js (WASM SQLite), Ollama nomic-embed-text embeddings.

## File Layout
```
src/daemon.js     HTTP :7437 + MCP stdio, auth, lifecycle
src/brain.js      indexAll, recall, store, forget, writeDiary
src/embeddings.js Ollama vectors, cosine similarity
src/compiler.js   Per-profile boot compilation with token budgets
src/conflict.js   Cross-agent conflict detection and resolution
src/profiles.js   Profile loader
src/db.js         SQLite schema, CRUD, persistence
src/cli.js        CLI wrapper, auto-start, HTTP client
```

## Rules
- Use process.env.USERPROFILE || process.env.HOME for all paths
- Conflict check runs BEFORE surprise filter
- Auth token required for HTTP POST endpoints
- sql.js is async init (getDb returns Promise), then sync operations
- Persist to disk via db.persist() after writes (auto-debounced)
- No external deps beyond sql.js
- Conventional commits: feat:, fix:, refactor:

## Brain Boot (do this FIRST)
Even when editing this codebase, connect to the running daemon:
```bash
curl -s http://localhost:7437/boot?agent=YOUR_NAME
```
Print the brain status at session start. Store important decisions.
Confirm every store to the user: "Stored to Cortex: [summary]"
Full protocol: see `CONNECTING.md` in this repo.

## Testing
```bash
npm test    # node:test built-in runner
```
