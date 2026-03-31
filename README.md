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
  <a href="https://github.com/AdityaVG13/cortex/blob/master/LICENSE"><img src="https://img.shields.io/badge/License-MIT-green?style=for-the-badge" alt="License: MIT"></a>
  <a href="https://github.com/AdityaVG13/cortex"><img src="https://img.shields.io/badge/Rust-1.78+-orange?style=for-the-badge" alt="Rust"></a>
  <a href="https://github.com/AdityaVG13/cortex"><img src="https://img.shields.io/badge/ONNX-embedded-blueviolet?style=for-the-badge" alt="ONNX"></a>
  <a href="https://github.com/AdityaVG13/cortex"><img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey?style=for-the-badge" alt="Platform"></a>
</p>

<p align="center">
  <a href="#quick-start">Quick Start</a> --
  <a href="#how-it-works">How It Works</a> --
  <a href="#features">Features</a> --
  <a href="#connecting-your-ai">Connect Your AI</a> --
  <a href="#api">API Reference</a> --
  <a href="#security">Security</a> --
  <a href="#whats-next">Roadmap</a>
</p>

---

AI coding assistants forget everything between sessions. Every conversation starts from scratch -- re-discovering your toolchain, conventions, and past decisions. Burning tokens and patience.

Cortex gives every AI a shared brain that persists, compresses, and pushes context before being asked. It works with Claude Code, Gemini CLI, Codex CLI, Cursor, and any tool that speaks HTTP.

**By the numbers:**
- 97% token compression on boot (19K raw -> ~500 tokens served)
- 264 embeddings indexed across 159 memories and 106 decisions
- Sub-100ms recall with hybrid semantic + keyword search
- Bearer auth on all sensitive endpoints, CORS-locked to localhost

---

## Quick Start

```bash
git clone https://github.com/AdityaVG13/cortex.git
cd cortex/daemon-rs
cargo build --release
./target/release/cortex serve
```

Verify:

```bash
curl http://localhost:7437/health
# {"status":"ok","stats":{"memories":0,"decisions":0,"embeddings":0}}
```

Register with Claude Code:

```bash
claude mcp add cortex -s user -- /path/to/cortex/daemon-rs/target/release/cortex mcp
```

The daemon runs on `localhost:7437`. Any AI that can make HTTP requests can use it.

---

## How It Works

| Component | What It Does |
|-----------|-------------|
| **Capsule Compiler** | Compiles a minimal boot prompt from two capsules: **Identity** (~200 tokens, stable -- who you are, platform rules, sharp edges) and **Delta** (~50-100 tokens, fresh -- what changed since this agent last connected). Result: ~300 tokens to fully orient any AI, versus 4,000+ raw. |
| **In-Process ONNX Embeddings** | 384-dimensional all-MiniLM-L6-v2 vectors computed inside the daemon -- no external Ollama dependency required. Model auto-downloads on first run. |
| **Conflict Detection** | When Claude stores "Use Python 3.12" and Gemini stores "Use Python 3.10," Cortex detects the semantic conflict via cosine similarity (0.85 threshold) with Jaccard fallback (0.6), marks both as disputed, and surfaces the disagreement in every boot prompt until resolved. |
| **Predictive Cache** | Co-occurrence matrix tracks which memories are recalled together. Recall memory A, and memories B and C that frequently co-occur get preloaded -- reducing round trips and latency. |
| **Progressive Recall** | Three-step token-efficient retrieval: **Peek** (one-line summaries, ~10 tok/result) -> **Budget** (key points within caller-specified budget) -> **Full** (complete context with semantic ranking). |
| **Score Decay** | Ebbinghaus-inspired memory aging. Unretrieved memories decay over time. Frequently accessed memories strengthen. The brain gets sharper, not just bigger. |
| **Knowledge Indexer** | On startup, indexes 6 filesystem sources (state.md, memory files, lessons, goals, skill tracker, recent decisions) and builds embeddings for new entries. |
| **Multi-Agent Coordination** | Session bus, file locking, task board, inter-agent feed, and SSE event streaming -- all built in. |

---

## Architecture

```
                  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
                  │  Claude Code │  │  Gemini CLI  │  │  Codex CLI   │
                  │  (MCP+HTTP)  │  │  (HTTP)      │  │  (HTTP)      │
                  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘
                         │                 │                 │
                         └─────────────────┼─────────────────┘
                                           │
                                ┌──────────▼──────────┐
                                │   cortex.exe        │
                                │   Rust (axum+tokio) │
                                │   localhost:7437    │
                                │                     │
                                │  SQLite (WAL mode)  │
                                │  ONNX Runtime       │
                                │  384-dim embeddings │
                                │  SSE event stream   │
                                └─────────────────────┘
```

Single Rust binary. No Node.js, no Python, no Docker. Compiles to ~15MB with embedded ONNX runtime.

```
daemon-rs/src/
  main.rs          # Entry, startup, background embedding builder
  server.rs        # Axum router, CORS, auth middleware
  state.rs         # RuntimeState (DB, auth, embeddings, SSE, caches)
  embeddings.rs    # In-process ONNX (model download, embed, cosine)
  indexer.rs       # Knowledge indexer (6 sources) + score decay
  compiler.rs      # Capsule boot prompt compiler
  conflict.rs      # Conflict detection (cosine + Jaccard)
  co_occurrence.rs # Co-occurrence tracking + prediction
  auth.rs          # Token, PID, stale daemon kill
  db.rs            # SQLite schema, WAL, indexes, integrity
  mcp_stdio.rs     # MCP stdio transport (JSON-RPC)
  hook_boot.rs     # SessionStart hook (CLI subcommand)
  service.rs       # Windows Service registration
  handlers/        # HTTP endpoint handlers
```

---

## Features

### Core Memory
- **Capsule Compiler** -- Identity + delta capsules compile ~300-token boot prompts (97% compression)
- **Hybrid Recall** -- Semantic (ONNX embeddings) + keyword search with token budgeting
- **Predictive Cache** -- Co-occurrence matrix preloads context you'll need next
- **Conflict Detection** -- Cosine + Jaccard flags contradictions between agents
- **Score Decay** -- Ebbinghaus-inspired aging keeps the brain sharp
- **Knowledge Indexer** -- 6 filesystem sources auto-indexed on startup

### Security
- **Bearer Auth** -- All mutation endpoints and sensitive GET endpoints require token auth
- **CORS Locked** -- tower-http CorsLayer restricts to localhost origins only
- **Parameterized Queries** -- All SQL uses prepared statements, zero string interpolation
- **Auth Token Isolation** -- Token stored at `~/.cortex/cortex.token`, read-only

### Token Optimization
- **Cost Ladder** -- Three retrieval depths: peek (minimal) -> budget (default) -> full (deep)
- **Budget Recall** -- Token-aware search fits results within caller-specified budget
- **Context Dedup** -- Same info never appears in both boot and recall
- **Savings Analytics** -- Real token savings tracked per session and cumulative

### Multi-Agent Coordination
- **Session Bus** -- Agents register, heartbeat, see who's online
- **File Locking** -- Prevents conflicting edits across agents
- **Task Board** -- Priority-routed task queue with claim/complete lifecycle
- **Inter-Agent Feed** -- Shared message feed for cross-agent communication
- **SSE Events** -- Real-time push for dashboards and agent subscribers

### Performance
- **7 Database Indexes** -- Optimized for recall, boot, and event queries
- **WAL Mode** -- Concurrent readers, fast writes
- **codegen-units=1 + LTO** -- Maximum compiler optimization
- **In-Process ONNX** -- No network hop for embeddings, ~5ms per vector

---

## Connecting Your AI

### Claude Code (MCP + HTTP)

```bash
claude mcp add cortex -s user -- /path/to/cortex/daemon-rs/target/release/cortex mcp
```

The `hook-boot` SessionStart hook calls `/boot` mechanically and injects the compiled prompt as context before the AI processes any message.

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
  -d '{"text": "Use JWT for API auth", "context": "api-design"}'
```

---

## API

### HTTP Endpoints

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `GET` | `/health` | No | Daemon status, memory/decision/embedding counts |
| `GET` | `/boot?agent=<id>` | Yes | Compiled boot prompt (capsule system) |
| `GET` | `/recall?q=<query>&budget=<tokens>` | Yes | Hybrid semantic + keyword search |
| `GET` | `/peek?q=<query>` | Yes | Headlines-only recall (minimal tokens) |
| `GET` | `/recall/budget?q=<query>&budget=<n>` | Yes | Token-budgeted recall |
| `GET` | `/digest` | Yes | Daily health digest with activity summary |
| `GET` | `/savings` | Yes | Token savings analytics |
| `GET` | `/dump` | Yes | Full memory export |
| `GET` | `/events/stream` | No | SSE real-time event stream |
| `POST` | `/store` | Yes | Store decision with conflict detection |
| `POST` | `/diary` | Yes | Write session state to state.md |
| `POST` | `/forget` | Yes | Decay matching memories by keyword |
| `POST` | `/resolve` | Yes | Resolve a disputed decision pair |
| `POST` | `/shutdown` | Yes | Graceful daemon shutdown |

Auth: `Authorization: Bearer <token>` (token at `~/.cortex/cortex.token`)

### MCP Tools

| Tool | Description |
|------|-------------|
| `cortex_boot` | Compiled boot prompt with capsule metadata |
| `cortex_recall` | Hybrid search across all memories and decisions |
| `cortex_store` | Store decision with conflict detection and dedup |
| `cortex_diary` | Write session handoff to state.md |
| `cortex_health` | System health check |
| `cortex_peek` | Lightweight source-only recall |
| `cortex_digest` | Token savings digest |
| `cortex_forget` | Decay memories matching a keyword |
| `cortex_resolve` | Resolve a dispute between decisions |

### CLI

```bash
cortex serve                   # Start HTTP daemon on :7437
cortex mcp                     # MCP stdio proxy to running daemon
cortex hook-boot [--agent X]   # SessionStart hook
cortex hook-status             # Statusline one-liner
cortex service install         # Register as Windows Service
```

---

## Security

Cortex holds sensitive data -- your decisions, project context, and AI memory. Security is mandatory, not optional.

- All GET endpoints returning data require Bearer token auth (except `/health`)
- CORS restricted to localhost origins via tower-http (no wildcard overrides)
- All SQL queries use parameterized statements
- Auth token generated on first run, stored at `~/.cortex/cortex.token`
- Daemon binds to `127.0.0.1` only -- not accessible from network

---

## Design Principles

1. **Compound, don't accumulate.** Every memory should make the next session smarter, not just bigger. Unused facts decay. Overlapping facts merge. The brain gets denser, not larger.

2. **Push, don't pull.** The brain injects context before being asked. AIs boot already knowing what matters.

3. **Universal interface.** HTTP is the API. Any AI, any language, any platform. MCP is a convenience transport for Claude Code.

4. **Security by default.** Auth on every sensitive endpoint. CORS locked down. No string interpolation in SQL. Token isolation.

5. **Single binary, zero deps.** One Rust executable, no Node.js, no Python, no Docker. Compiles and runs anywhere.

6. **Evidence before assertions.** Never claim a feature works without a test. Never claim a bug is fixed without verification output.

---

## What's Next

### In Progress
- [ ] Merge `feat/rust-daemon` to `main`
- [ ] Ebbinghaus decay scoring with configurable half-life
- [ ] Semantic dedup on writes (prevent near-duplicate memories)
- [ ] Memory type system (user/feedback/project/reference classification)
- [ ] FTS5 full-text search for keyword recall (replaces linear scan)

### Planned
- [ ] Connection pooling / RwLock for concurrent read access
- [ ] Query embedding LRU cache
- [ ] Rate limiting middleware
- [ ] PII detection and scrubbing on store
- [ ] Self-improvement tool integration (symbol retriever, failure clusterer, hybrid router)
- [ ] Orchestration loop with local function-calling models (Dolphin 3.0)

---

## License

MIT
