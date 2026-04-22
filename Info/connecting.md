<p align="center"><a href="../README.md">← Back to README</a></p>

# Connecting to Cortex

> One daemon, one brain, every tool. Connect any AI that speaks HTTP or MCP.

---

## The 30-second version

```bash
cortex boot --agent YOUR_NAME --json
```

Replace `YOUR_NAME` with your agent ID (`cursor`, `claude`, `gemini`, `codex`, etc). Read the `bootPrompt` from the response — that's your context.

Connection refused? Start the daemon first: `cortex serve`

---

## How Cortex works

Cortex is a persistent memory daemon on `localhost:7437`. It stores decisions, preferences, project context, and lessons across all AI sessions. When you connect, you get instant context — no re-explaining.

**You are not the only AI.** Claude Code, Codex, Cursor, Gemini, Cline, and local LLMs all share the same brain. What you store becomes available to every other AI. What they stored is available to you.

**Token savings are tracked.** Every boot logs tokens consumed vs what raw file reads would cost. Typical efficiency: ~97% reduction.

---

## Platform setup

<details>
<summary><b>Claude Code</b> — MCP, automatic</summary>

Already connected via MCP plugin. Use `cortex_boot`, `cortex_recall`, `cortex_store` tools directly.

```bash
claude plugin marketplace add AdityaVG13/cortex
claude plugin install cortex@cortex-marketplace
```

</details>

<details>
<summary><b>Codex CLI</b> — MCP</summary>

Register the MCP sidecar:
```bash
codex mcp add cortex -- /path/to/cortex.exe mcp --agent codex
```
Restart Codex. MCP servers added mid-session take effect next session.

</details>

<details>
<summary><b>Cursor / Cline / Gemini</b> — MCP</summary>

Point your MCP client at:
```bash
/path/to/cortex.exe mcp --agent cursor
```
Use `--agent gemini` for Gemini, `--agent cline` for Cline. The proxy also infers the parent client automatically, but explicit `--agent` is the stable path.

</details>

<details>
<summary><b>Aider / any CLI tool</b> — HTTP</summary>

Run before starting work:
```bash
cortex boot --agent aider --json
```
Use the output as context for your session.

Store decisions:
```bash
curl -X POST http://localhost:7437/store \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-Cortex-Request: true" \
  -H "X-Source-Agent: YOUR_NAME" \
  -d '{"decision": "What you learned", "context": "Why it matters"}'
```

</details>

<details>
<summary><b>Any new AI tool</b> — HTTP</summary>

If it can make HTTP requests or run shell commands, it can connect. Three endpoints:

1. `GET /boot?agent=your-name` — get context
2. `GET /recall?q=topic` — search memories
3. `POST /store` — save decisions

All require `Authorization: Bearer <token>` and `X-Cortex-Request: true` headers.

</details>

---

## Core operations

### 1. Boot — get context (call first)

```bash
curl -s \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-Cortex-Request: true" \
  "http://localhost:7437/boot?agent=YOUR_NAME"
```

Returns two capsules:

| Capsule | Tokens | Contents |
|---------|--------|----------|
| **Identity** | ~200 | User identity, platform rules, constraints. Stable across sessions. |
| **Delta** | ~50-100 | What changed since your last boot. New decisions, conflicts, state. |

<details>
<summary>Example response</summary>

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

</details>

### 2. Recall — search memories

```bash
curl -s \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-Cortex-Request: true" \
  "http://localhost:7437/recall?q=authentication+architecture&k=5"
```

Hybrid search: ONNX embeddings (in-process) + tokenized keyword fallback. Always works, no external dependencies.

### 3. Store — save a decision

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

> **Conflict detection is automatic.** If you store something that contradicts another AI's decision, both are flagged as "disputed" and surfaced in every future boot until a human resolves it.

### 4. Digest — brain health

```bash
curl -s \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-Cortex-Request: true" \
  http://localhost:7437/digest
```

Returns memory counts, today's activity, token savings, top recalled entries, agent boot history.

### 5. Health and readiness

```bash
# Liveness (public, no auth)
curl http://localhost:7437/health

# Startup gate (is the daemon fully ready?)
curl http://localhost:7437/readiness
```

---

## Rules for all AIs

| # | Rule |
|---|------|
| 1 | **Boot first.** Call `/boot?agent=your-name` before doing anything else. |
| 2 | **Recall before researching.** Check if Cortex already knows before spending tokens. |
| 3 | **Store sparingly.** Only durable insights — decisions, lessons, preferences. Not session chatter. |
| 4 | **Use your real agent name.** Set `X-Source-Agent` honestly. Provenance tracking matters. |
| 5 | **Don't overwrite.** Disagree? Store your perspective. The conflict system handles it. |
| 6 | **Don't delete.** Never remove another AI's entries. Archive, don't destroy. |

---

## Full endpoint reference

### Core endpoints

| Method | Path | Auth | Description |
|--------|------|:----:|-------------|
| GET | `/boot?agent=NAME` | ✓ | Capsule-compiled boot prompt |
| GET | `/recall?q=QUERY&k=7` | ✓ | Hybrid semantic + keyword search |
| POST | `/recall` | ✓ | Same as GET, body avoids query-string leakage |
| POST | `/store` | ✓ | Store decision with conflict detection |
| GET | `/health` | — | Liveness and system status |
| GET | `/readiness` | — | Startup gate (daemon fully ready?) |
| GET | `/digest` | ✓ | Daily health digest with token savings |

### Memory management

| Method | Path | Auth | Description |
|--------|------|:----:|-------------|
| GET | `/dump` | ✓ | All active memories + decisions (batch) |
| POST | `/archive` | ✓ | Bulk status change to archived |
| POST | `/forget` | ✓ | Decay entries matching keyword |
| POST | `/resolve` | ✓ | Resolve disputed decision pair |
| POST | `/diary` | ✓ | Write session handoff to state.md |

### Analytics and diagnostics

| Method | Path | Auth | Description |
|--------|------|:----:|-------------|
| GET | `/stats` | ✓ | Tier distribution, latency, recall savings |
| GET | `/savings` | ✓ | Token savings with rollup aggregates |
| GET | `/recall/explain` | ✓ | Recall ranking explanation with diagnostics |

### Agent telemetry

| Method | Path | Auth | Description |
|--------|------|:----:|-------------|
| POST | `/agent-feedback` | ✓ | Record task outcome telemetry |
| GET | `/agent-feedback/stats` | ✓ | Reliability trends from recorded outcomes |

### System

| Method | Path | Auth | Description |
|--------|------|:----:|-------------|
| POST | `/shutdown` | ✓ | Graceful daemon shutdown |

> **Auth** = `Authorization: Bearer TOKEN` + `X-Cortex-Request: true`
>
> Token is at `~/.cortex/cortex.token`. The `X-Cortex-Request` header is the SSRF guard — any non-empty value works, but `true` is canonical.

---

## Troubleshooting

| Problem | Fix |
|---------|-----|
| **Connection refused** | Daemon not running. `cortex serve` to start. |
| **403 Missing X-Cortex-Request** | Add `X-Cortex-Request: true` header to every non-health request. |
| **401 Unauthorized** | Refresh token from `~/.cortex/cortex.token`. |
| **MCP tools missing after add** | Restart your MCP client. Servers added mid-session take effect next session. |
| **Empty boot prompt** | No memories stored yet. Store some context and boot again. |
| **No semantic results** | ONNX model may still be downloading on first run. Keyword fallback works meanwhile. Check `~/.cortex/models/`. |
| **Auth token not found** | Token generates on daemon start. Start daemon first. |

---

## Architecture

```
cortex/
├─ daemon-rs/src/
│  ├─ main.rs           Entry, startup, background tasks
│  ├─ server.rs         Axum router, CORS, auth middleware
│  ├─ state.rs          RuntimeState (DB, auth, embeddings, SSE)
│  ├─ embeddings.rs     In-process ONNX (384-dim, MiniLM-L6/L12)
│  ├─ indexer.rs        Knowledge indexer + score decay
│  ├─ compiler.rs       Capsule boot prompt compiler
│  ├─ conflict.rs       Conflict detection (cosine + Jaccard)
│  ├─ auth.rs           Token, PID, stale daemon management
│  ├─ db.rs             SQLite schema, WAL, indexes, migrations
│  └─ mcp_proxy.rs      MCP stdio proxy (JSON-RPC 2.0 → HTTP)
├─ desktop/cortex-control-center/
│  ├─ src/App.jsx        React dashboard
│  └─ src-tauri/src/     Tauri sidecar lifecycle
├─ workers/              Background processing
└─ tools/                CLI utilities
```

**Database** at `~/.cortex/cortex.db` — tables: `memories`, `decisions`, `embeddings`, `events`, `sessions`, `locks`, `tasks`, `feed`, `messages`, `activity`, `client_permissions`, `agent_feedback`, `event_savings_rollups`.

**Embeddings**: 384-dim vectors via in-process ONNX. Default: `all-MiniLM-L6-v2`. Configurable via `CORTEX_EMBEDDING_MODEL` (e.g., `all-MiniLM-L12-v2`).
