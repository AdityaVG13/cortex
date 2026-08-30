<p align="center"><a href="../README.md">← Back to README</a></p>

# Connecting to Cortex

> One daemon, one brain, every tool. Connect any AI that speaks HTTP or MCP.

---

## The 30-second version

```bash
cortex status --json
cortex boot --agent YOUR_NAME --json
```

Start with `cortex status --json`. It reports `ready`, `needs_action`, or `error`, plus one `nextAction` and machine-readable repair data. Replace `YOUR_NAME` with your agent ID (`cursor`, `claude`, `gemini`, `codex`, etc). Read the `bootPrompt` from the response — that's your context.

Not ready? Use the `nextAction` from status. In normal local use that means opening Cortex Control Center or, for CLI-only setups, running `cortex serve`.

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
cortex status --json
```

Expected status: `ready`. Repair: if the plugin reports `APP_INIT_REQUIRED`, open Cortex Control Center or start your explicit local runtime, then retry the MCP tool.

</details>

<details>
<summary><b>Codex CLI</b> — MCP</summary>

Register the MCP sidecar:
```bash
codex mcp add cortex -- /path/to/cortex.exe mcp --agent codex
cortex status --json
```
Restart Codex. MCP servers added mid-session take effect next session.

Expected status: `ready`; smoke signal is a successful `cortex_boot` followed by a `cortex_store` / `cortex_recall` round trip from Codex. Repair: follow the `nextAction` from `cortex status --json`.

</details>

<details>
<summary><b>Cursor / Cline / Gemini</b> — MCP</summary>

Point your MCP client at:
```bash
/path/to/cortex.exe mcp --agent cursor
cortex status --json
```
Use `--agent gemini` for Gemini, `--agent cline` for Cline. The proxy also infers the parent client automatically, but explicit `--agent` is the stable path.

Expected status: `ready`; repair is `APP_INIT_REQUIRED` -> start/open Cortex first, then restart the MCP client.

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

### 0. Status — know the next action

```bash
cortex status --json
```

The JSON contract includes `schemaVersion`, `status`, `runtime`, `nextAction`, `repair`, and `checks`. Treat `ready` as usable. Treat `needs_action` or `error` as a stop-and-repair state, not a partial success.

### 1. Boot — get context (call first)

```bash
curl -s \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-Cortex-Request: true" \
  "http://localhost:7437/boot?agent=YOUR_NAME"
```

Returns extractive capsules (no summarizer):

| Capsule | Tokens | Contents |
|---------|--------|----------|
| **Identity** | ~200 | User identity, platform rules, constraints. Stable across sessions. |
| **Delta** | ~50–100 | What changed since last boot: conflicts, tasks, focus, messages, locks, agents, feed. |
| **TRUTH** | variable | Top current facts with `FACT!` / `FACT?` / `FACT~` sigils, packed to budget. |

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
    {"name": "delta", "tokens": 55, "freshness": "since 2026-03-28 04:17"},
    {"name": "truth", "tokens": 80, "freshness": "current"}
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

Clock-Quorum Recall (CQR): Cortex admits a result when a hard engineering anchor matches or when two or more independent clocks agree — write (FTS/exact observation), truth (entities/aliases), task (path/symbol/goal), and history (explicit as-of or version). It does not guess from paraphrase similarity. A miss with no shared anchor is expected, not a defect.

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
| GET | `/recall?q=QUERY&k=7` | ✓ | Clock-Quorum Recall (admit on hard anchor, two clocks, or strong lexical write) |
| GET | `/as-of?q=QUERY&t=RFC3339&k=7` | ✓ | CQR at an explicit validity time; returns stored status and validity windows |
| POST | `/recall` | ✓ | Same as GET, body avoids query-string leakage |
| POST | `/store` | ✓ | Store decision with conflict detection |
| GET | `/health` | — | Liveness and system status |
| GET | `/readiness` | — | Startup gate (daemon fully ready?) |
| GET | `/digest` | ✓ | Daily health digest with token savings |

### Memory management

| Method | Path | Auth | Description |
|--------|------|:----:|-------------|
| GET | `/dump` | ✓ | Active, disputed, and superseded memories + decisions with validity fields |
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
| **Connection refused** | Run `cortex status --json` and follow `nextAction`; open Control Center or run `cortex serve` for CLI-only local mode. |
| **403 Missing X-Cortex-Request** | Add `X-Cortex-Request: true` header to every non-health request. |
| **401 Unauthorized** | Refresh token from `~/.cortex/cortex.token`. |
| **APP_INIT_REQUIRED** | The client is attach-only. Open Cortex Control Center or explicitly start the local runtime, then retry. |
| **MCP tools missing after add** | Restart your MCP client. Servers added mid-session take effect next session. |
| **Empty boot prompt** | No memories stored yet. Store some context and boot again. |
| **Honest miss / no results** | CQR does not return a plausible neighbor without lexical, alias, task, history, or graph evidence. Add a path, symbol, alias, or citation rather than expecting paraphrase match. |
| **Auth token not found** | Token generates on daemon start. Start daemon first. |

---

## Architecture

```
cortex/
├─ crates/
│  ├─ daemon/           HTTP, MCP, SQLite, CQR collection, boot compiler
│  └─ logic/            clocks, graph, traces, conflict, budgets
├─ tests/contracts/     public daemon contracts
├─ desktop/cortex-control-center/
├─ plugins/cortex-plugin/
├─ sdks/
└─ Info/                product docs (this folder)
```

**Database** (`~/.cortex/cortex.db`): `memories`, `decisions`, `clock_anchors`, `clock_links`, `entities`, `traces`. Legacy `embeddings` rows stay inert.

**Recall**: Clock-Quorum Recall. No local model download. Empty is a valid answer.

See [ARCHITECTURE.md](../ARCHITECTURE.md) for the full map.
