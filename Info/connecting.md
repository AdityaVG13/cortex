# Connecting to Cortex -- Universal AI Brain

## Start Here (ANY AI, ANY tool)

You are an AI connecting to a Cortex persistent brain. Run this now:

```bash
cortex boot --agent YOUR_NAME --json
```

Replace `YOUR_NAME` with your agent ID (e.g., `cursor`, `claude`, `gemini`, `codex`).

If you get "connection refused", start the daemon first:

```bash
cortex serve
```

Read the `bootPrompt` from the JSON response -- that is your context. Then read this document for the full API.

Startup/lifecycle truth for `v0.5.0`:
- See `Info/startup-matrix-troubleshooting.md` for one-daemon startup behavior and troubleshooting.

---

## What This Is

Cortex is a persistent memory daemon running on `localhost:7437`. It stores decisions, preferences, project context, and lessons across ALL AI sessions. When you connect, you get instant context about the owner's projects and conventions -- without them re-explaining anything.

**You are not the only AI.** Claude Code, Codex CLI, Gemini CLI, Cline, Cursor, and local LLMs all share the same brain. What you store becomes available to every other AI. What they stored is available to you.

**Token savings are tracked.** Every boot logs how many tokens you consumed vs what raw file reads would cost. Current efficiency: ~97% reduction.

---

## Quick Setup by Platform

### Claude Code (MCP -- automatic)
Already connected via MCP. Use `cortex_boot`, `cortex_recall`, `cortex_store` tools directly.

### Any CLI tool (recommended)
At session start, run:
```bash
cortex boot --agent YOUR_NAME --json
```

Store decisions with:
```bash
curl -X POST http://localhost:7437/store \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-Cortex-Request: true" \
  -H "X-Source-Agent: YOUR_NAME" \
  -d '{"decision": "What you learned", "context": "Why it matters"}'
```

### Generic AI Agents
Most agents can be configured to read a file or call a URL at startup. Point your agent to the `/boot` endpoint to receive its persistent context.

### Codex CLI (MCP)
Register the MCP sidecar:
```bash
codex mcp add cortex -- /path/to/cortex.exe mcp --agent codex
```
Then restart Codex. Newly-added MCP servers do not hot-attach to the current session.

### Cline / Cursor / other MCP clients
Register the MCP sidecar with the command syntax your client expects. For Codex use the command above. For other clients, point the MCP server at:
```bash
/path/to/cortex.exe mcp --agent cursor
```
For Gemini, use `--agent gemini`. For Cline, use `--agent cline`. The proxy also tries to infer the parent client automatically, but explicit `--agent` is the stable path.

### Aider / Any CLI tool
Run before starting work:
```bash
cortex boot --agent aider
```
Use the output as context for your session.

### Any new AI tool
If it can make HTTP requests or run shell commands, it can connect. The protocol is:
1. `GET /boot?agent=your-name` with `Authorization: Bearer <token>` and `X-Cortex-Request: true` -- get context
2. `GET /recall?q=topic` with the same headers -- search memories
3. `POST /store` with the same headers -- save decisions

That's it. Three endpoints. Any language, any platform.

---

## Core Operations

### 1. Boot (get context -- call FIRST)

```bash
curl -s \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-Cortex-Request: true" \
  "http://localhost:7437/boot?agent=YOUR_NAME"
```

The capsule compiler returns two things:
- **Identity capsule** (~200 tokens): user identity, platform rules, constraints. Stable across sessions. Populated from stored feedback memories.
- **Delta capsule** (~50-100 tokens): what changed since YOUR last boot. New decisions, conflicts, state changes.

Response:
```json
{
  "bootPrompt": "## Identity\n...\n\n## Delta\n...",
  "tokenEstimate": 300,
  "profile": "capsules",
  "savings": {
    "rawBaseline": 14777,
    "served": 300,
    "saved": 14477,
    "percent": 97
  },
  "capsules": [
    {"name": "identity", "tokens": 245, "freshness": "stable"},
    {"name": "delta", "tokens": 55, "freshness": "since 2026-03-28 04:17"}
  ]
}
```

### 2. Recall (search memories)

```bash
curl -s \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-Cortex-Request: true" \
  "http://localhost:7437/recall?q=authentication+architecture&k=5"
```

Hybrid search: ONNX embeddings (in-process) + tokenized keyword fallback. Always works even without external dependencies.

### 3. Store (save a decision)

```bash
TOKEN=$(cat ~/.cortex/cortex.token)
curl -X POST http://localhost:7437/store \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-Cortex-Request: true" \
  -H "X-Source-Agent: YOUR_AGENT_NAME" \
  -d '{"decision": "What you learned", "context": "Why", "type": "decision"}'
```

Types: `decision`, `lesson`, `preference`, `bugfix`

**Conflict detection is automatic.** If you store something that contradicts another AI's decision, both are flagged as "disputed" and surfaced in every future boot until a human resolves it.

### 4. Digest (check brain health)

```bash
curl -s \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-Cortex-Request: true" \
  http://localhost:7437/digest
```

Returns: memory counts, today's activity, token savings, top recalled entries, agent boot history.

### 5. Dump (batch read -- for workers)

```bash
TOKEN=$(cat ~/.cortex/cortex.token)
curl -s \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-Cortex-Request: true" \
  http://localhost:7437/dump
```

Returns ALL active memories and decisions. Used by the dreaming/compaction worker.

### 6. Archive (batch status change)

```bash
curl -X POST http://localhost:7437/archive \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-Cortex-Request: true" \
  -d '{"type": "memories", "ids": [1, 2, 3]}'
```

### 7. Health

```bash
curl http://localhost:7437/health
```

---

## Rules for All AIs

1. **Boot first.** Call `/boot?agent=your-name` before doing anything else.
2. **Recall before researching.** Check if Cortex already knows before spending tokens: `curl "localhost:7437/recall?q=your+topic"`.
3. **Store sparingly.** Only durable insights -- decisions, lessons, preferences. Not session chatter.
4. **Use your real agent name.** Set `X-Source-Agent` honestly. Provenance tracking matters.
5. **Don't overwrite.** If you disagree with an existing entry, store your perspective. The conflict system handles it.
6. **Don't delete.** Never try to remove another AI's entries. Archive, don't destroy.

---

## User Context

Call `/boot` to get compiled context about the owner. The identity capsule is built from stored feedback memories -- it reflects whatever the owner has taught Cortex over time. There is no hardcoded identity; it is 100% derived from the database.

---

## Full Endpoint Reference

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/boot?agent=NAME` | Bearer token + `X-Cortex-Request` | Capsule-compiled boot prompt |
| GET | `/recall?q=QUERY&k=7` | Bearer token + `X-Cortex-Request` | Hybrid semantic + keyword search |
| POST | `/store` | Bearer token + `X-Cortex-Request` | Store decision with conflict detection |
| POST | `/diary` | Bearer token + `X-Cortex-Request` | Write session handoff to state.md |
| GET | `/health` | No | System status |
| GET | `/digest` | Bearer token + `X-Cortex-Request` | Daily health digest with token savings |
| GET | `/dump` | Bearer token + `X-Cortex-Request` | All active memories + decisions (batch) |
| POST | `/archive` | Bearer token + `X-Cortex-Request` | Bulk status change to archived |
| POST | `/forget` | Bearer token + `X-Cortex-Request` | Decay entries matching keyword |
| POST | `/resolve` | Bearer token + `X-Cortex-Request` | Resolve disputed decision pair |
| POST | `/shutdown` | Bearer token + `X-Cortex-Request` | Graceful daemon shutdown |

Protected endpoints require both:
- `Authorization: Bearer TOKEN` where `TOKEN` is from `~/.cortex/cortex.token`
- `X-Cortex-Request: true`

`X-Cortex-Request` is the SSRF guard. Any non-empty value satisfies the check, but `true` is the canonical value shown in examples.

---

## Troubleshooting

**Connection refused:** Daemon not running. Start it:
```bash
cortex serve
```

**403 Missing X-Cortex-Request header:** Add `X-Cortex-Request: true` to every non-health request.

**401 Unauthorized:** Refresh the token from `~/.cortex/cortex.token` and send it as `Authorization: Bearer <token>`.

**MCP tools still missing after `codex mcp add`:** Restart Codex. MCP servers added mid-session are available in the next session, not the current one.

**Empty boot prompt:** No memories stored yet. Store some context and boot again.

**No semantic results:** ONNX model may still be downloading on first run. Keyword fallback still works. Check `~/.cortex/models/` for the model file.

**Auth token not found:** Token generates on daemon start. Start daemon first.

---

## Architecture

```
cortex/
-- daemon-rs/src/
   -- main.rs           Entry, startup, background tasks
   -- server.rs         Axum router, CORS, auth middleware
   -- state.rs          RuntimeState (DB, auth, embeddings, SSE)
   -- embeddings.rs     In-process ONNX (384-dim all-MiniLM-L6-v2)
   -- indexer.rs        Knowledge indexer (6 sources) + score decay
   -- compiler.rs       Capsule boot prompt compiler
   -- conflict.rs       Conflict detection (cosine + Jaccard)
   -- auth.rs           Token, PID, stale daemon management
   -- db.rs             SQLite schema, WAL, indexes, migrations
   -- mcp_stdio.rs      MCP stdio transport (JSON-RPC 2.0)
-- desktop/cortex-control-center/
   -- src/App.jsx        React dashboard (12 panels)
   -- src-tauri/src/     Tauri sidecar lifecycle
-- workers/
   -- drift_detector.py  Stale reference checker
-- tools/
   -- ingest_chatgpt.py  ChatGPT export ingestion adapter
-- cortex.db             SQLite database (at repo root or CWD)
```

Database tables: `memories`, `decisions`, `embeddings`, `events`, `sessions`, `locks`, `tasks`, `feed`.
Every entry has: `source_agent`, `confidence`, `status`, `score`, `last_accessed`, `pinned`.
Embeddings: 384-dim vectors from all-MiniLM-L6-v2 via in-process ONNX (no Ollama required).
