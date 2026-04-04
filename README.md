<p align="center">

```
 ██████╗ ██████╗ ██████╗ ████████╗███████╗██╗  ██╗
██╔════╝██╔═══██╗██╔══██╗╚══██╔══╝██╔════╝╚██╗██╔╝
██║     ██║   ██║██████╔╝   ██║   █████╗   ╚███╔╝
██║     ██║   ██║██╔══██╗   ██║   ██╔══╝   ██╔██╗
╚██████╗╚██████╔╝██║  ██║   ██║   ███████╗██╔╝ ██╗
 ╚═════╝ ╚═════╝ ╚═╝  ╚═╝   ╚═╝   ╚══════╝╚═╝  ╚═╝
```

</p>

<h3 align="center">A persistent, self-improving brain for AI coding agents.</h3>
<h4 align="center">Single Rust binary. Zero runtime dependencies. In-process ONNX embeddings.</h4>

<p align="center">
  <a href="https://github.com/AdityaVG13/cortex/releases/latest"><img src="https://img.shields.io/github/v/release/AdityaVG13/cortex?style=for-the-badge&color=blue" alt="Release"></a>
  <a href="https://github.com/AdityaVG13/cortex/blob/master/LICENSE"><img src="https://img.shields.io/badge/License-MIT-green?style=for-the-badge" alt="License: MIT"></a>
  <a href="https://github.com/AdityaVG13/cortex"><img src="https://img.shields.io/badge/Rust-1.78+-orange?style=for-the-badge" alt="Rust"></a>
  <a href="https://github.com/AdityaVG13/cortex"><img src="https://img.shields.io/badge/ONNX-embedded-blueviolet?style=for-the-badge" alt="ONNX"></a>
  <a href="https://github.com/AdityaVG13/cortex"><img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey?style=for-the-badge" alt="Platform"></a>
</p>

<p align="center">
  <a href="#installation">Installation</a> --
  <a href="#quick-start">Quick Start</a> --
  <a href="#desktop-app">Desktop App</a> --
  <a href="#how-it-works">How It Works</a> --
  <a href="#features">Features</a> --
  <a href="#connecting-your-ai">Connect Your AI</a> --
  <a href="#api">API Reference</a> --
  <a href="#security">Security</a> --
  <a href="#whats-next">Roadmap</a>
</p>

---

AI coding assistants forget everything between sessions. Every conversation starts from scratch -- re-discovering your toolchain, conventions, and past decisions. Burning tokens and patience.

Cortex gives every AI a shared brain that persists, compresses, and pushes context before being asked. It works with Claude Code, Cursor, Gemini CLI, Codex CLI, and any tool that speaks HTTP or MCP.

**By the numbers:**
- 97% token compression on boot (19K raw -> ~500 tokens served)
- Sub-100ms recall with hybrid semantic + keyword search
- Bearer auth on all sensitive endpoints, CORS-locked to localhost
- 13 MCP tools, 35+ HTTP endpoints, SSE real-time stream

---

## Installation

### Download (recommended)

Grab the latest release from [GitHub Releases](https://github.com/AdityaVG13/cortex/releases/latest):

| Platform | Download |
|----------|----------|
| **Windows** | [`cortex-v0.1.0-windows-x86_64.zip`](https://github.com/AdityaVG13/cortex/releases/download/v0.1.0/cortex-v0.1.0-windows-x86_64.zip) |
| **macOS** | Coming soon |
| **Linux** | Coming soon |

Extract the archive and place `cortex.exe` somewhere on your PATH (e.g. `C:\Users\<you>\.local\bin\`).

### Build from source

Requires [Rust 1.78+](https://rustup.rs/):

```bash
git clone https://github.com/AdityaVG13/cortex.git
cd cortex/daemon-rs
cargo build --release
# Binary at target/release/cortex(.exe)
```

---

## Quick Start

```bash
cortex serve
```

Verify:

```bash
curl http://localhost:7437/health
# {"status":"ok","stats":{"memories":0,"decisions":0,"embeddings":0}}
```

Register with your editor:

```bash
# Claude Code
claude mcp add cortex -s user -- /path/to/cortex mcp

# Cursor -- add to ~/.cursor/mcp.json:
# { "mcpServers": { "cortex": { "command": "/path/to/cortex.exe", "args": ["mcp"] } } }
```

Or use the [Desktop App](#desktop-app) which handles daemon lifecycle and MCP registration automatically.

---

## Desktop App

Cortex ships with a Tauri desktop app -- **Cortex Control Center** -- that manages the daemon, visualizes your brain, and coordinates agents from a single window.

```bash
cd cortex/desktop/cortex-control-center
npm install
npm run dev    # Development
npm run build  # Production bundle
```

### Panels

| Panel | Description |
|-------|-------------|
| **Overview** | Live metrics, active agents, pending tasks, system status, MCP setup |
| **Memory** | Search and explore stored memories with peek/unfold |
| **Analytics** | Token savings charts, daily sparklines, boot compression stats |
| **Agents** | Active agent sessions with heartbeat tracking |
| **Tasks** | Kanban-style task board (Pending / In Progress / Done) |
| **Feed** | Shared inter-agent message feed with filters |
| **Messages** | Direct agent-to-agent messaging |
| **Activity** | Timestamped activity log across all agents |
| **Locks** | Active file locks with agent attribution and expiry |
| **Brain** | 3D force-directed graph visualization of your memory network |
| **Conflicts** | Side-by-side dispute resolution with Keep/Merge/Archive actions |

### MCP Auto-Registration

Click the **MCP** button in the Overview panel to automatically detect and register Cortex with:
- **Claude Code** -- writes to `~/.claude/settings.json`
- **Cursor** -- writes to `~/.cursor/mcp.json`

Preserves existing editor settings. Idempotent -- safe to run multiple times.

### Desktop Features

- System tray with minimize-to-tray (close button hides, tray quit exits)
- Embedded daemon management (start/stop/status)
- Real-time SSE event streaming across all panels
- Bearer token auth handled automatically
- WAL checkpoint and optimize on graceful shutdown

---

## How It Works

| Component | What It Does |
|-----------|-------------|
| **Capsule Compiler** | Compiles a minimal boot prompt from two capsules: **Identity** (~200 tokens, stable -- who you are, platform rules, sharp edges) and **Delta** (~50-100 tokens, fresh -- what changed since this agent last connected). Result: ~300 tokens to fully orient any AI, versus 4,000+ raw. |
| **In-Process ONNX Embeddings** | 384-dimensional all-MiniLM-L6-v2 vectors computed inside the daemon -- no external dependency. Model auto-downloads on first run to `~/.cortex/models/`. |
| **Conflict Detection** | When Claude stores "Use Python 3.12" and Cursor stores "Use Python 3.10," Cortex detects the semantic conflict via cosine similarity (0.85 threshold) with Jaccard fallback (0.6), marks both as disputed, and surfaces the disagreement in every boot prompt until resolved. |
| **Predictive Cache** | Co-occurrence matrix tracks which memories are recalled together. Recall memory A, and memories B and C that frequently co-occur get preloaded -- reducing round trips and latency. |
| **Progressive Recall** | Three-step token-efficient retrieval: **Peek** (one-line summaries, ~10 tok/result) -> **Unfold** (full text of specific items) -> **Budget Recall** (complete context within caller-specified budget). |
| **Score Decay** | Ebbinghaus-inspired memory aging. Unretrieved memories decay over time (0.95^days). Frequently accessed memories strengthen. The brain gets sharper, not just bigger. |
| **Knowledge Indexer** | On startup, indexes 6 filesystem sources (state.md, memory files, lessons, goals, skill tracker, recent decisions) and builds embeddings for new entries. |
| **Relevance Feedback** | Implicit signals from unfold (positive) and explicit POST /feedback for reranking. Boosts from feedback compound over time. |
| **Crystallization** | Clusters similar memories into "crystals" -- semantic groups that compress related knowledge into navigable units. |
| **Compaction** | Prunes old events, archives stale text, deduplicates embeddings to keep storage lean. |
| **Multi-Agent Coordination** | Session bus, file locking, task board, inter-agent feed, and SSE event streaming -- all built in. |

---

## Architecture

```
              ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
              │  Claude Code │  │   Cursor     │  │  Gemini CLI  │  │  Codex CLI   │
              │  (MCP+HTTP)  │  │  (MCP+HTTP)  │  │  (HTTP)      │  │  (HTTP)      │
              └──────┬───────┘  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘
                     │                 │                 │                 │
                     └─────────────────┼─────────────────┼─────────────────┘
                                       │                 │
                            ┌──────────▼─────────────────▼──┐
                            │   cortex.exe                  │
                            │   Rust (axum + tokio)         │
                            │   localhost:7437              │
                            │                               │
                            │  SQLite (WAL mode)            │
                            │  ONNX Runtime (in-process)    │
                            │  384-dim embeddings           │
                            │  MCP JSON-RPC 2.0 (stdio)    │
                            │  SSE event stream             │
                            └───────────────────────────────┘
                                       │
                            ┌──────────▼──────────┐
                            │  Control Center     │
                            │  Tauri Desktop App  │
                            │  (React + Three.js) │
                            └─────────────────────┘
```

Single Rust binary. No Node.js, no Python, no Docker. Compiles to ~30MB with embedded ONNX runtime.

```
daemon-rs/src/
  main.rs           # Entry, startup, background tasks (embedding builder, aging, crystallization)
  server.rs         # Axum router, CORS, auth middleware
  state.rs          # RuntimeState (DB, auth, embeddings, SSE, caches)
  embeddings.rs     # In-process ONNX (model download, embed, cosine)
  indexer.rs        # Knowledge indexer (6 sources) + score decay
  compiler.rs       # Capsule boot prompt compiler
  conflict.rs       # Conflict detection (cosine + Jaccard)
  co_occurrence.rs  # Co-occurrence tracking + prediction
  compaction.rs     # Storage compaction engine
  crystallize.rs    # Memory crystallization (semantic clustering)
  auth.rs           # Token, PID, stale daemon management
  db.rs             # SQLite schema, WAL, indexes, integrity, migrations
  mcp_stdio.rs      # MCP stdio transport (JSON-RPC 2.0)
  mcp_proxy.rs      # HTTP-to-daemon MCP relay (thin client mode)
  hook_boot.rs      # SessionStart hook (CLI subcommand)
  service.rs        # Windows Service registration
  handlers/
    boot.rs         # GET /boot -- capsule compiler
    recall.rs       # GET /recall, /peek, /unfold -- semantic search
    store.rs        # POST /store -- persist with conflict detection
    mutate.rs       # POST /forget, /resolve, /archive + GET /conflicts
    health.rs       # GET /health, /digest, /savings, /dump
    diary.rs        # POST /diary -- session state persistence
    conductor.rs    # Multi-agent: locks, tasks, activity, sessions, messages
    feed.rs         # Shared inbox: POST/GET /feed, SSE /events/stream
    feedback.rs     # Relevance feedback + reranking boosts
    mcp.rs          # MCP tool definitions + JSON-RPC dispatch

desktop/cortex-control-center/
  src/App.jsx           # 11-panel dashboard (React)
  src/BrainVisualizer.jsx  # 3D force-directed brain graph (Three.js)
  src-tauri/src/main.rs    # Tauri commands, tray, lifecycle
  src-tauri/src/embedded_daemon.rs  # In-process daemon for bundled mode
```

---

## Features

### Core Memory
- **Capsule Compiler** -- Identity + delta capsules compile ~300-token boot prompts (97% compression)
- **Hybrid Recall** -- Semantic (ONNX embeddings) + keyword search with token budgeting
- **Progressive Disclosure** -- Peek -> Unfold -> Full recall, each level costs more tokens
- **Predictive Cache** -- Co-occurrence matrix preloads context you'll need next
- **Conflict Detection** -- Cosine + Jaccard flags contradictions between agents
- **Conflict Resolution** -- Keep, merge, or archive disputed decisions via API or desktop UI
- **Score Decay** -- Ebbinghaus-inspired aging keeps the brain sharp
- **Knowledge Indexer** -- 6 filesystem sources auto-indexed on startup
- **Relevance Feedback** -- Implicit (unfold) and explicit signals improve recall ranking
- **Crystallization** -- Clusters similar memories into navigable semantic groups
- **Compaction** -- Prunes events, archives text, deduplicates embeddings

### Security
- **Bearer Auth** -- All mutation endpoints and sensitive GET endpoints require token auth
- **CORS Locked** -- tower-http CorsLayer restricts to localhost origins only
- **Parameterized Queries** -- All SQL uses prepared statements, zero string interpolation
- **Auth Token Isolation** -- Token stored at `~/.cortex/cortex.token`, auto-generated on first run
- **Stale PID Protection** -- Validates process identity before killing stale daemons

### Token Optimization
- **Cost Ladder** -- Three retrieval depths: peek (minimal) -> budget (default) -> full (deep)
- **Budget Recall** -- Token-aware search fits results within caller-specified budget
- **Context Dedup** -- Same info never appears in both boot and recall
- **Savings Analytics** -- Real token savings tracked per session and cumulative

### Multi-Agent Coordination
- **Session Bus** -- Agents register, heartbeat, see who's online
- **File Locking** -- Prevents conflicting edits across agents with TTL-based locks
- **Task Board** -- Priority-routed task queue with claim/complete/abandon lifecycle
- **Inter-Agent Feed** -- Shared message feed for cross-agent communication
- **Direct Messages** -- Agent-to-agent messaging with inbox per agent
- **SSE Events** -- Real-time push for dashboards and agent subscribers

### Performance
- **7+ Database Indexes** -- Optimized for recall, boot, and event queries
- **WAL Mode** -- Concurrent readers, fast writes
- **codegen-units=1 + LTO** -- Maximum compiler optimization
- **In-Process ONNX** -- No network hop for embeddings, ~5ms per vector
- **MCP Proxy Mode** -- Thin client forwards to running daemon, single ONNX engine shared across all editors

---

## Connecting Your AI

### Claude Code (MCP + HTTP)

```bash
claude mcp add cortex -s user -- /path/to/cortex/daemon-rs/target/release/cortex mcp
```

Or use the desktop app's MCP auto-registration button.

The `hook-boot` SessionStart hook calls `/boot` mechanically and injects the compiled prompt as context before the AI processes any message.

### Cursor (MCP + HTTP)

Add to `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "cortex": {
      "command": "/path/to/cortex/daemon-rs/target/release/cortex.exe",
      "args": ["mcp"]
    }
  }
}
```

Or use the desktop app's MCP auto-registration button.

### Gemini CLI / Codex CLI / Any AI (HTTP)

```bash
# Boot (requires auth)
TOKEN=$(cat ~/.cortex/cortex.token)
curl -H "Authorization: Bearer $TOKEN" "http://localhost:7437/boot?agent=my-agent"

# Recall
curl -H "Authorization: Bearer $TOKEN" "http://localhost:7437/recall?q=auth+patterns&budget=500"

# Store
curl -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  http://localhost:7437/store \
  -d '{"decision": "Use JWT for API auth", "context": "api-design", "source_agent": "my-agent"}'
```

### Teaching Your AI to Use Cortex

Registering the MCP server makes the tools *available*. To make your AI actually *use* them, add instructions to your editor's AI config file. Below are copy-paste snippets for each editor.

#### Claude Code (`CLAUDE.md` or `~/.claude/CLAUDE.md`)

```markdown
## Memory (Cortex)

Cortex is your persistent brain. It runs on localhost:7437 and is registered as an MCP server.

### Boot
- At session start, call `cortex_boot()` to load compressed context from prior sessions.
- Print the returned status line as your first output.

### Before Work
- Before investigating any bug, implementing a feature, or making a decision, call `cortex_recall("topic")` to check for prior context.
- Use progressive recall to save tokens: `cortex_peek(query)` first (one-line summaries), then `cortex_unfold(sources)` for items you need full text on.

### After Work
- After making a decision or learning something non-obvious, store it: `cortex_store(decision, context)`.
- Confirm stores visibly: "Stored to Cortex: [summary]".

### Session End
- Before ending a session, call `cortex_diary(accomplished, nextSteps)` to persist state for the next session.
```

#### Cursor (`~/.cursor/rules/cortex.mdc`)

```markdown
---
description: Persistent AI memory via Cortex daemon
globs: **/*
alwaysApply: true
---

Cortex MCP is registered and provides persistent memory across sessions.

- Start every session by calling `cortex_boot` to load prior context.
- Before investigating bugs or making decisions, call `cortex_peek` to check for existing knowledge.
- After decisions, call `cortex_store` with the decision and context.
- At session end, call `cortex_diary` with what was accomplished and next steps.
```

#### Gemini CLI / Codex CLI / Other (system prompt or `AGENTS.md`)

```markdown
## Persistent Memory

A memory daemon is running at http://localhost:7437. Auth token is at ~/.cortex/cortex.token.

On session start:
  GET /boot?agent=<your-id> -- returns compressed context from prior sessions.

Before investigating or deciding:
  GET /peek?q=<topic>&limit=5 -- check for prior knowledge (minimal tokens).
  GET /unfold?sources=<id1>,<id2> -- expand items you need full text on.

After making decisions:
  POST /store {"decision": "...", "context": "...", "source_agent": "<your-id>"}

On session end:
  POST /diary {"accomplished": "...", "next_steps": "...", "agent": "<your-id>"}
```

#### Optional: SessionStart Hook (Claude Code)

For automatic boot injection, add a SessionStart hook that calls Cortex before the AI processes any message. This eliminates the need for the AI to call `cortex_boot()` manually:

```bash
# Register the hook (assumes cortex.exe is on PATH)
# In ~/.claude/settings.json, add under "hooks.SessionStart":
# { "type": "command", "command": "cortex hook-boot --agent claude" }
```

The hook calls `/boot`, formats the compiled prompt, and injects it as context -- so the AI starts every session already oriented.

### Windows Service

```bash
cortex service install    # Register (requires Admin)
cortex service start      # Start service
cortex service status     # Check health
cortex service stop       # Stop
cortex service uninstall  # Remove
```

Auto-start on boot with failure recovery (5s, 10s, 30s restart intervals).

---

## API

### HTTP Endpoints

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `GET` | `/health` | No | Daemon status, memory/decision/embedding counts |
| `GET` | `/boot?agent=<id>&budget=600` | Yes | Compiled boot prompt (capsule system) |
| `GET` | `/recall?q=<query>&k=7&budget=200` | Yes | Hybrid semantic + keyword search |
| `GET` | `/peek?q=<query>&limit=10` | Yes | Headlines-only recall (minimal tokens) |
| `GET` | `/unfold?sources=<s1>,<s2>` | Yes | Full text of specific memory/decision nodes |
| `GET` | `/digest` | Yes | Daily health digest with activity summary |
| `GET` | `/savings` | Yes | Token savings analytics |
| `GET` | `/dump` | Yes | Full memory export |
| `GET` | `/conflicts` | Yes | Disputed decision pairs for resolution |
| `GET` | `/crystals` | Yes | Memory clusters (crystallized groups) |
| `GET` | `/storage` | Yes | Storage breakdown by table |
| `GET` | `/events/stream` | No | SSE real-time event stream |
| `POST` | `/store` | Yes | Store decision with conflict detection |
| `POST` | `/diary` | Yes | Write session state to state.md |
| `POST` | `/forget` | Yes | Decay matching memories by keyword |
| `POST` | `/resolve` | Yes | Resolve dispute: keep, merge, or archive |
| `POST` | `/archive` | Yes | Archive entries by table and IDs |
| `POST` | `/feedback` | Yes | Record relevance feedback for reranking |
| `POST` | `/focus/start` | Yes | Start focus session (sawtooth compression) |
| `POST` | `/focus/end` | Yes | End focus session with summary |
| `POST` | `/compact` | Yes | Run storage compaction |
| `POST` | `/crystallize` | Yes | Run memory crystallization pass |
| `POST` | `/shutdown` | Yes | Graceful daemon shutdown |

**Conductor (multi-agent):**

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `POST/GET` | `/tasks` | Yes | Create / list tasks |
| `GET` | `/tasks/next?agent=<id>` | Yes | Claim next available task |
| `POST` | `/tasks/claim\|complete\|abandon` | Yes | Task lifecycle |
| `POST` | `/lock` / `/unlock` | Yes | File locking |
| `GET` | `/locks` | Yes | Active locks |
| `POST/GET` | `/activity` | Yes | Activity log |
| `POST` | `/message` | Yes | Send inter-agent message |
| `GET` | `/messages?agent=<id>` | Yes | Agent inbox |
| `POST` | `/session/start\|heartbeat\|end` | Yes | Agent session lifecycle |
| `GET` | `/sessions` | Yes | Active sessions |
| `POST/GET` | `/feed` | Yes | Shared feed |
| `POST` | `/feed/ack` | Yes | Mark feed items read |

Auth: `Authorization: Bearer <token>` (token at `~/.cortex/cortex.token`)

### MCP Tools (13)

| Tool | Description |
|------|-------------|
| `cortex_boot` | Compiled boot prompt with capsule metadata |
| `cortex_recall` | Hybrid search with token budgeting (0=headlines, 200=balanced, 500+=full) |
| `cortex_peek` | Lightweight source-only recall (~80% token savings vs full) |
| `cortex_unfold` | Full text of specific memory/decision/crystal nodes |
| `cortex_store` | Store decision with conflict detection and dedup |
| `cortex_forget` | Decay memories matching a keyword |
| `cortex_resolve` | Resolve a dispute between decisions (keep/merge/archive) |
| `cortex_diary` | Write session handoff to state.md |
| `cortex_health` | System health check |
| `cortex_digest` | Token savings and activity digest |
| `cortex_focus_start` | Start focus session (sawtooth token compression) |
| `cortex_focus_end` | End focus session with summary |
| `cortex_focus_status` | Check active focus sessions |

### CLI

```bash
cortex serve                   # Start HTTP daemon on :7437
cortex mcp                     # MCP stdio proxy to running daemon
cortex hook-boot [--agent X]   # SessionStart hook for Claude Code
cortex hook-status             # Statusline one-liner
cortex service install         # Register as Windows Service (Admin)
cortex service start|stop|status|uninstall
```

---

## Security

Cortex holds sensitive data -- your decisions, project context, and AI memory. Security is mandatory, not optional.

- All GET endpoints returning data require Bearer token auth (except `/health`)
- CORS restricted to localhost origins via tower-http (no wildcard overrides)
- All SQL queries use parameterized statements
- Auth token generated on first run, stored at `~/.cortex/cortex.token`
- Daemon binds to `127.0.0.1` only -- not accessible from network by default
- Stale daemon detection validates process identity before termination (Windows: process command line check)
- MCP proxy mode inherits auth from the running daemon

---

## Design Principles

1. **Compound, don't accumulate.** Every memory should make the next session smarter, not just bigger. Unused facts decay. Overlapping facts merge. The brain gets denser, not larger.

2. **Push, don't pull.** The brain injects context before being asked. AIs boot already knowing what matters.

3. **Universal interface.** HTTP is the API. Any AI, any language, any platform. MCP is a convenience transport for Claude Code and Cursor.

4. **Security by default.** Auth on every sensitive endpoint. CORS locked down. No string interpolation in SQL. Token isolation.

5. **Single binary, zero deps.** One Rust executable, no Node.js, no Python, no Docker. Compiles and runs anywhere.

6. **Evidence before assertions.** Never claim a feature works without a test. Never claim a bug is fixed without verification output.

---

## What's Next

### In Progress
- [ ] Remote connection support (team mode -- connect desktop app to non-localhost daemon)
- [ ] Merge `feat/rust-daemon` to `main`
- [ ] CI/CD pipeline with GitHub Actions (build + release artifacts)

### Planned
- [ ] Connection pooling / RwLock for concurrent read access
- [ ] Query embedding LRU cache
- [ ] Rate limiting middleware
- [ ] PII detection and scrubbing on store
- [ ] Cross-machine brain sync (export/import with conflict-aware merge)
- [ ] Auto-update mechanism for desktop app (tauri-plugin-updater)

---

## License

MIT
