# Connecting to Cortex — Universal AI Brain

## Start Here

To boot yourself with full context, run this command now:

```bash
curl http://localhost:7437/boot
```

If you get "connection refused", the daemon is not running. Start it first:

```bash
node C:/Users/aditya/cortex/src/daemon.js serve
```

Then run the curl command again. Read the `bootPrompt` field from the JSON response — that is your context about Aditya, his projects, and his preferences. Then read the rest of this document to learn how to recall, store, and interact with the brain.

---

## What This Is

Cortex is a persistent local memory system that runs on Aditya's machine.
It stores decisions, preferences, project context, lessons learned, and institutional knowledge across all AI sessions.
When you connect, you get full context about who Aditya is, what projects are active, what conventions to follow, and what was decided in prior sessions — without Aditya having to re-explain anything.

You are not the only AI using this brain.
Claude Code, Gemini CLI, Codex CLI, Cursor, and local models all read from and write to the same memory.
Anything you store becomes available to every other AI in future sessions.

## How to Connect

Cortex runs as an HTTP daemon on localhost port 7437.
If the daemon is not running, start it:

```bash
node C:/Users/aditya/cortex/src/daemon.js serve
```

Or use the CLI (auto-starts the daemon if needed):

```bash
node C:/Users/aditya/cortex/src/cli.js boot
```

## Core Operations

### 1. Boot (get full context)

Call this FIRST at the start of any session.

```bash
curl http://localhost:7437/boot
```

With a specific profile:

```bash
curl "http://localhost:7437/boot?profile=index"
```

**Profiles available:**
- `full` (default) — ~300-700 tokens. Identity, active project, recent decisions, key rules, pending work, lessons, conflicts.
- `operational` — ~500 tokens. Identity, hard constraints, active decisions, sharp edges. Best for execution-focused work.
- `subagent` — ~200 tokens. Identity and hard constraints only. Best for sub-agents doing focused tasks.
- `index` — ~60 tokens. Identity and a topic list of what Cortex knows. Best for lightweight discovery before targeted recall.

The response is JSON:
```json
{
  "bootPrompt": "## Identity\nUser: Aditya...",
  "tokenEstimate": 310,
  "profile": "full"
}
```

Use the `bootPrompt` field as system context for your session.

### 2. Recall (search memories)

When you need context about a specific topic:

```bash
curl "http://localhost:7437/recall?q=authentication+architecture&k=5"
```

Returns ranked results with relevance scores:
```json
{
  "results": [
    { "source": "memory::feedback_auth.md", "relevance": 0.87, "excerpt": "...", "method": "semantic" }
  ]
}
```

The search is hybrid: semantic (Ollama embeddings) + keyword fallback.
If Ollama is not running, you still get keyword results.

### 3. Store (save a learning)

When you discover something important that future sessions should know:

```bash
curl -X POST http://localhost:7437/store \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer TOKEN" \
  -H "X-Source-Agent: YOUR_AGENT_NAME" \
  -d '{
    "decision": "What you learned or decided",
    "context": "Why this matters (optional)",
    "type": "decision"
  }'
```

**Getting the auth token:** Read from `~/.cortex/cortex.token` (C:\Users\aditya\.cortex\cortex.token).

```bash
TOKEN=$(cat ~/.cortex/cortex.token)
```

**Types:** `decision`, `lesson`, `preference`, `bugfix`

**Your agent name:** Use a consistent identifier. Examples: `gemini-2.5`, `codex-gpt5.4`, `cursor`, `qwen-32b`. This gets recorded as provenance so we can trace who stored what.

**Conflict handling:** If you store something that contradicts what another AI previously stored, both entries are kept and flagged as "disputed." Neither is deleted. Aditya or a future session resolves disputes.

### 4. Health Check

```bash
curl http://localhost:7437/health
```

Returns system status including Ollama connectivity and entry counts.

## What You Should Know About Aditya

Rather than listing everything here, call `/boot` and read the compiled context.
Key facts the boot prompt will include:
- Platform: Windows 10
- Python: Always use `uv`, never pip
- Git: Conventional commits (feat:, fix:, etc.)
- Shell: bash (Unix syntax even on Windows)
- Identity: The user is ADITYA. Diya and Adi are family members.

## Rules for Using the Brain

1. **Boot first.** Call `/boot` or `/boot?profile=index` at session start. This is the single most important step.
2. **Recall before researching.** Before spending tokens exploring files or searching the web, check if Cortex already has the answer: `curl "localhost:7437/recall?q=your+topic"`.
3. **Store sparingly.** Only store durable insights — decisions, lessons, preferences, bug fixes. Do not store transient task state or session chatter.
4. **Use your real agent name.** Set `X-Source-Agent` honestly so provenance tracking works.
5. **Do not overwrite.** If you disagree with an existing entry, store your perspective. The conflict system handles it. Never try to delete or modify another AI's entries.
6. **Sub-agents should not store directly.** If you are a sub-agent running a focused task, return your findings to the orchestrator and let it decide what to persist.

## Endpoints Reference

| Method | Path | Auth Required | Description |
|--------|------|---------------|-------------|
| GET | `/boot?profile=full` | No | Compiled boot prompt |
| GET | `/recall?q=QUERY&k=7` | No | Semantic + keyword search |
| POST | `/store` | Yes (Bearer token) | Store decision/lesson |
| POST | `/diary` | Yes | Write session handoff to state.md |
| GET | `/health` | No | System status |
| POST | `/forget` | Yes | Decay old entries |
| POST | `/resolve` | Yes | Resolve disputed entries |
| POST | `/shutdown` | Yes | Stop the daemon |

## Troubleshooting

**Connection refused on port 7437:**
The daemon is not running. Start it:
```bash
node C:/Users/aditya/cortex/src/daemon.js serve
```

**Empty boot prompt:**
The database may not have been migrated yet. Run:
```bash
node C:/Users/aditya/cortex/scripts/migrate-v1.js
```

**Recall returns no semantic results:**
Ollama may not be running. Start it, then recall falls back to keyword search automatically.

**Auth token not found:**
The token is generated on daemon startup. Start the daemon first, then read `~/.cortex/cortex.token`.

## Architecture (for AIs that want to understand the internals)

```
C:\Users\aditya\cortex\
├── src/
│   ├── daemon.js      HTTP server + MCP stdio + lifecycle
│   ├── brain.js       Index, recall, store, forget logic
│   ├── embeddings.js  Ollama vectors + cosine similarity
│   ├── compiler.js    Per-profile boot prompt compilation
│   ├── conflict.js    Cross-agent conflict detection
│   ├── profiles.js    Profile loader
│   ├── db.js          SQLite via sql.js (WASM)
│   └── cli.js         CLI wrapper
├── cortex.db          SQLite database (all memories, decisions, embeddings)
├── cortex-profiles.json  Profile definitions
└── CONNECTING.md      This file
```

The database has 4 tables: `memories`, `decisions`, `embeddings`, `events`.
Every entry has `source_agent`, `confidence`, `status`, and `score` fields.
Embeddings are 768-dimensional vectors from Ollama's nomic-embed-text model, stored as binary BLOBs.
