# Connecting to Cortex — Universal AI Brain

## Start Here (ANY AI, ANY tool)

You are an AI connecting to Aditya's persistent brain. Run this now:

```bash
curl http://localhost:7437/boot?agent=YOUR_NAME
```

Replace `YOUR_NAME` with your agent ID (e.g., `factory-glm`, `cline`, `cursor`, `gemini`).

If you get "connection refused", start the daemon first:

```bash
node C:/Users/aditya/cortex/src/daemon.js serve
```

Read the `bootPrompt` from the JSON response — that is your context. Then read this document for the full API.

---

## What This Is

Cortex is a persistent memory daemon running on `localhost:7437`. It stores decisions, preferences, project context, and lessons across ALL AI sessions. When you connect, you get instant context about Aditya, his projects, and his conventions — without him re-explaining anything.

**You are not the only AI.** Claude Code, Codex CLI, Gemini CLI, Factory Droids, Cline, Cursor, and local Ollama models all share the same brain. What you store becomes available to every other AI. What they stored is available to you.

**Token savings are tracked.** Every boot logs how many tokens you consumed vs what raw file reads would cost. Current efficiency: ~97% reduction.

---

## Quick Setup by Platform

### Claude Code (MCP — automatic)
Already connected via MCP. Use `cortex_boot`, `cortex_recall`, `cortex_store` tools directly.

### Factory Droid CLI
At session start, read this file and run:
```bash
TOKEN=$(cat C:/Users/aditya/.cortex/cortex.token)
curl -s http://localhost:7437/boot?agent=factory-MODEL_NAME
```
Replace `MODEL_NAME` with the model you're using (e.g., `factory-glm47`, `factory-minimax`).

Store decisions with:
```bash
curl -X POST http://localhost:7437/store \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-Source-Agent: factory-MODEL_NAME" \
  -d '{"decision": "What you learned", "context": "Why it matters"}'
```

### Gemini CLI
Reads `~/GEMINI.md` at startup which instructs it to check `~/.claude/brain-status.json` and call `/boot`.

### Codex CLI
Reads `~/AGENTS.md` at startup with the same protocol.

### Cline (VS Code)
Connect via MCP or HTTP. Add to your Cline MCP config:
```json
{
  "cortex": {
    "command": "node",
    "args": ["C:/Users/aditya/cortex/src/daemon.js", "mcp"]
  }
}
```
Or use HTTP directly via terminal commands.

### Cursor
Uses Claude Code's MCP registration. If MCP is registered, Cursor sees Cortex tools automatically.

### Aider / Any CLI tool
Run before starting work:
```bash
curl -s http://localhost:7437/boot?agent=aider | python -c "import json,sys; print(json.load(sys.stdin)['bootPrompt'])"
```
Use `--read` flag to pass the output as context.

### Any new AI tool
If it can make HTTP requests or run shell commands, it can connect. The protocol is:
1. `GET /boot?agent=your-name` — get context
2. `GET /recall?q=topic` — search memories
3. `POST /store` with auth — save decisions
That's it. Three endpoints. Any language, any platform.

---

## Core Operations

### 1. Boot (get context — call FIRST)

```bash
curl "http://localhost:7437/boot?agent=YOUR_NAME"
```

The capsule compiler returns two things:
- **Identity capsule** (~200 tokens): who Aditya is, platform rules, constraints. Stable across sessions.
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
curl "http://localhost:7437/recall?q=authentication+architecture&k=5"
```

Hybrid search: semantic (Ollama embeddings) + tokenized keyword fallback. Always works even if Ollama is down.

### 3. Store (save a decision)

```bash
TOKEN=$(cat C:/Users/aditya/.cortex/cortex.token)
curl -X POST http://localhost:7437/store \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-Source-Agent: YOUR_AGENT_NAME" \
  -d '{"decision": "What you learned", "context": "Why", "type": "decision"}'
```

Types: `decision`, `lesson`, `preference`, `bugfix`

**Conflict detection is automatic.** If you store something that contradicts another AI's decision, both are flagged as "disputed" and surfaced in every future boot until a human resolves it.

### 4. Digest (check brain health)

```bash
curl http://localhost:7437/digest
```

Returns: memory counts, today's activity, token savings, top recalled entries, agent boot history.

### 5. Dump (batch read — for workers)

```bash
TOKEN=$(cat C:/Users/aditya/.cortex/cortex.token)
curl -H "Authorization: Bearer $TOKEN" http://localhost:7437/dump
```

Returns ALL active memories and decisions. Used by the dreaming/compaction worker.

### 6. Archive (batch status change)

```bash
curl -X POST http://localhost:7437/archive \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
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
3. **Store sparingly.** Only durable insights — decisions, lessons, preferences. Not session chatter.
4. **Use your real agent name.** Set `X-Source-Agent` honestly. Provenance tracking matters.
5. **Don't overwrite.** If you disagree with an existing entry, store your perspective. The conflict system handles it.
6. **Don't delete.** Never try to remove another AI's entries. Archive, don't destroy.

---

## What You Should Know About Aditya

Call `/boot` for compiled context. Key facts:
- Platform: Windows 10, no WSL
- Python: Always `uv`, never pip
- Git: Conventional commits (`feat:`, `fix:`, `docs:`)
- Shell: bash (Unix syntax even on Windows)
- Identity: The user is ADITYA. Diya and Adi are family members.
- Node.js for daemon code, Python for intelligence workers
- 12GB VRAM, CPU inference for local models

---

## Full Endpoint Reference

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/boot?agent=NAME` | No | Capsule-compiled boot prompt |
| GET | `/recall?q=QUERY&k=7` | No | Hybrid semantic + keyword search |
| POST | `/store` | Yes | Store decision with conflict detection |
| POST | `/diary` | Yes | Write session handoff to state.md |
| GET | `/health` | No | System status + Ollama connectivity |
| GET | `/digest` | No | Daily health digest with token savings |
| GET | `/dump` | Yes | All active memories + decisions (batch) |
| POST | `/archive` | Yes | Bulk status change to archived |
| POST | `/forget` | Yes | Decay entries matching keyword |
| POST | `/resolve` | Yes | Resolve disputed decision pair |
| POST | `/shutdown` | Yes | Graceful daemon shutdown |

Auth = `Authorization: Bearer TOKEN` where TOKEN is from `~/.cortex/cortex.token`.

---

## Troubleshooting

**Connection refused:** Daemon not running. Start it:
```bash
node C:/Users/aditya/cortex/src/daemon.js serve
```

**Empty boot prompt:** Database may need migration:
```bash
node C:/Users/aditya/cortex/scripts/migrate-v1.js
```

**No semantic results:** Ollama not running. Keyword fallback still works.

**Auth token not found:** Token generates on daemon start. Start daemon first.

---

## Architecture

```
C:\Users\aditya\cortex\
├── src/
│   ├── daemon.js      HTTP + MCP server, auth, lifecycle
│   ├── brain.js       Index, recall, store, forget, digest
│   ├── compiler.js    Capsule compiler (identity + delta)
│   ├── embeddings.js  Ollama vectors + cosine similarity
│   ├── conflict.js    Cross-agent semantic conflict detection
│   ├── profiles.js    Profile loader (full/operational/subagent/index)
│   ├── db.js          SQLite via sql.js, search, decay
│   └── cli.js         CLI wrapper with auto-start
├── workers/
│   ├── cortex_client.py   Python HTTP client (zero deps)
│   └── cortex_dream.py    Memory compaction worker
├── cortex.db              SQLite database
├── cortex-profiles.json   Compilation profiles
├── README.md              Full docs + roadmap
├── CONNECTING.md          This file
├── CLAUDE.md              Claude Code specific instructions
├── GEMINI.md              Gemini CLI instructions
└── AGENTS.md              Codex CLI instructions
```

Database tables: `memories`, `decisions`, `embeddings`, `events`.
Every entry has: `source_agent`, `confidence`, `status`, `score`, `last_accessed`, `pinned`.
Embeddings: 768-dim vectors from nomic-embed-text via Ollama.
