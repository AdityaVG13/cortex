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

<!-- SECURITY-RULES:START (auto-synced from docs/SECURITY-RULES.md -- do not edit here) -->
## Windows Defender -- NEVER TRIGGER
This runs on Windows. These patterns cause ML-based AV false positives (Bearfoos, SuspExec, ClickFix):
- **Never** spawn a detached process that then kills other processes via taskkill
- **Never** read a token/credential file and immediately POST it over HTTP in the same script
- **Never** use `execSync('taskkill /IM ...')` patterns in test or production code
- **Never** write PowerShell that reads secrets then pipes to curl in a single command
- Instead: use Rust's native process management, pass auth via environment variables, keep token reads and HTTP calls in separate logical steps with clear application context
- Test scripts must avoid spawn-sleep-kill-read-token-POST chains -- break into discrete steps with named functions
<!-- SECURITY-RULES:END -->

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

## How to Recall (Token-Efficient)
ONE function: `cortex_recall(query, budget)`. The budget controls detail level:
- `budget=0` — Headlines only. Source + relevance. ~30 tokens. Use to check if anything exists.
- `budget=200` — Balanced (DEFAULT). Top result full, rest compressed. ~200 tokens.
- `budget=500` — Full detail. All results with complete excerpts. ~500 tokens.

Call ONCE with the right budget. Do NOT call peek then recall — that wastes tokens.
```bash
# Quick check — anything about Python?
curl "http://localhost:7437/recall?q=python&budget=0"

# Normal recall — get useful context
curl "http://localhost:7437/recall?q=python+packaging&budget=200"

# Deep research — need everything
curl "http://localhost:7437/recall?q=cortex+architecture&budget=500"
```

The system also has a predictive cache — if you recall A then B repeatedly,
it pre-caches B so your second call is instant (0 tokens spent on search).

## Testing
```bash
npm test    # node:test built-in runner
```
