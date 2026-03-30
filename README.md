```
 ██████╗ ██████╗ ██████╗ ████████╗███████╗██╗  ██╗
██╔════╝██╔═══██╗██╔══██╗╚══██╔══╝██╔════╝╚██╗██╔╝
██║     ██║   ██║██████╔╝   ██║   █████╗   ╚███╔╝
██║     ██║   ██║██╔══██╗   ██║   ██╔══╝   ██╔██╗
╚██████╗╚██████╔╝██║  ██║   ██║   ███████╗██╔╝ ██╗
 ╚═════╝ ╚═════╝ ╚═╝  ╚═╝   ╚═╝   ╚══════╝╚═╝  ╚═╝
```

**A persistent, self-improving brain for AI coding agents.**

Cortex is a memory daemon that gives Claude Code, Gemini CLI, Codex, Cursor, and any AI a shared, long-term brain.
It stores decisions, detects conflicts between agents, compiles token-efficient boot prompts, and compounds intelligence across sessions - so every conversation starts smarter than the last.
One daemon. One database. Every AI connects. Knowledge compounds.

---

## About

Cortex solves the biggest problem with AI coding assistants: they forget everything between sessions.
Every time you start a new conversation, your AI re-discovers your toolchain, your conventions, your past decisions - burning tokens and your patience.
Cortex gives every AI a shared brain that persists, compresses, and pushes context before being asked.

---

## Features

### Core Memory
- **Capsule Compiler** - Identity + delta capsules compile ~300-token boot prompts (vs 4,000+ raw)
- **Unified Smart Recall** - Hybrid semantic (Ollama embeddings) + keyword search with token budgeting
- **Predictive Cache** - Co-occurrence matrix preloads context you're likely to need next
- **Cross-Agent Conflict Detection** - Semantic similarity flags contradictions between agents
- **Dream Compaction** - Nightly deduplication and synthesis via local LLM workers

### Token Optimization
- **Cost Ladder** - Three retrieval modes: headlines (minimal) → balanced (default) → full (deep dive)
- **Budget Recall** - Token-aware search that fits results within a caller-specified budget
- **Context Dedup** - Prevents the same information from appearing in both boot and recall
- **Savings Analytics** - Real token savings tracking per session and cumulative

### Multi-Agent Coordination
- **Session Bus** - Agents register, heartbeat, and see who else is online
- **File Locking** - Prevents conflicting edits across agents working on the same repo
- **Task Board** - Priority-routed task queue with claim/complete lifecycle
- **Inter-Agent Feed** - Shared message feed for cross-agent communication
- **SSE Event Stream** - Real-time push for dashboards and agent subscribers

### Desktop App
- **Jarvis-Inspired UI** - Tauri-powered native app with agent dashboard and task board
- **3D Brain Visualizer** - Interactive Three.js memory graph with co-occurrence edges
- **Memory Explorer** - Browse, search, and manage all stored memories
- **Analytics Dashboard** - Token savings, boot frequency, agent activity

### Session Boot
- **Mechanical Boot Hook** - SessionStart hook calls `/boot` via HTTP, injects context before the AI processes any message - no AI cooperation needed
- **StatusLine Integration** - Live Cortex/Ollama status in the Claude Code status bar via CCStatusline
- **Universal Connectivity** - `brain-status.json` written at boot for any AI to read

---

## Quick Start

```bash
# Clone and install
git clone https://github.com/AdityaVG13/cortex.git
cd cortex
npm install

# Start the daemon
node src/daemon.js serve

# Verify
curl http://localhost:7437/health
```

The daemon runs on `localhost:7437`. Any AI that can make HTTP requests can use it.

### Register with Claude Code

```bash
claude mcp add cortex -s user -- node /path/to/cortex/src/daemon.js mcp
```

### Launch the Desktop App

```bash
cd desktop/cortex-control-center
npm install
npm run tauri dev
```

---

## How It Works

### The Capsule Compiler

When an AI boots, Cortex compiles a minimal prompt from two capsules:

**Identity capsule** (~200 tokens, stable): Who you are, platform rules, hard constraints, known sharp edges. Doesn't change between sessions.

**Delta capsule** (~50-100 tokens, fresh): What changed since this specific agent last connected. New decisions, new conflicts, state changes. Only the diff.

Result: ~300 tokens to fully orient any AI, versus 4,000+ tokens of raw file reads.

### Conflict Detection

When Claude stores "Use Python 3.12" and Gemini stores "Use Python 3.10," Cortex detects the semantic conflict via embedding similarity, marks both as disputed, and surfaces the disagreement in every subsequent boot prompt until a human resolves it.

### Predictive Cache

Cortex tracks which memories are recalled together via a co-occurrence matrix. When you recall memory A, and memories B and C are frequently accessed alongside it, Cortex preloads B and C before you ask - reducing round trips and latency.

### Multi-AI Architecture

```
┌──────────────┐  ┌──────────────┐  ┌──────────────┐
│  Claude Code │  │  Gemini CLI  │  │  Codex CLI   │
│  (MCP+HTTP)  │  │  (HTTP)      │  │  (HTTP)      │
└──────┬───────┘  └──────┬───────┘  └──────┬───────┘
       │                 │                 │
       └─────────────────┼─────────────────┘
                         │
              ┌──────────▼──────────┐
              │   Cortex Daemon     │
              │   localhost:7437    │
              │                     │
              │  ┌───────────────┐  │
              │  │  SQLite DB    │  │
              │  │  (sql.js)     │  │
              │  └───────────────┘  │
              │                     │
              │  ┌───────────────┐  │
              │  │  Ollama       │  │
              │  │  Embeddings   │  │
              │  └───────────────┘  │
              └──────────┬──────────┘
                         │
              ┌──────────▼──────────┐
              │  Tauri Desktop App  │
              │  (Rust + React)     │
              │                     │
              │  3D Brain Visualizer│
              │  Agent Dashboard    │
              │  Task Board         │
              │  Memory Explorer    │
              │  Analytics          │
              └─────────────────────┘
```

---

## Architecture

Cortex runs as a dual-daemon system:

- **Node Daemon** (`src/daemon.js`) - Feature-ahead HTTP + MCP server. Handles memory operations, capsule compilation, conflict detection, session bus, task board, file locking, SSE events, and inter-agent feed.
- **Rust/Tauri Desktop App** (`desktop/cortex-control-center/`) - Native app wrapping the daemon with a React frontend. Includes the 3D brain visualizer, Jarvis-inspired UI, and system tray integration.

```
src/
  daemon.js      # HTTP + MCP server, auth, lifecycle, session bus, task board, SSE
  brain.js       # Core: indexAll, recall, store, forget, diary, budget recall
  compiler.js    # Capsule compiler (identity + delta), cost ladder, predictive cache
  embeddings.js  # Ollama nomic-embed-text vectors, cosine similarity, co-occurrence
  conflict.js    # Semantic conflict detection, dispute management
  profiles.js    # Profile loader (full/operational/subagent/index)
  db.js          # SQLite via sql.js, schema, CRUD helpers
  cli.js         # CLI wrapper, auto-start daemon, HTTP client

desktop/cortex-control-center/
  src/
    App.jsx              # Main app shell, Jarvis-inspired layout
    BrainVisualizer.jsx  # 3D memory graph (Three.js)
  src-tauri/
    src/                 # Rust backend (daemon lifecycle, system tray)
```

**Design constraints:**
- Node daemon: one npm dependency (`sql.js` - SQLite compiled to WASM)
- No native compilation for the daemon, no build step
- Works on Windows 10 without WSL
- Daemon stays alive across sessions, auto-starts on boot

---

## API

### HTTP Endpoints

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `GET` | `/boot?agent=<id>` | No | Compiled boot prompt (capsule system) |
| `GET` | `/recall?q=<query>&budget=<tokens>` | No | Hybrid semantic + keyword search with token budgeting |
| `GET` | `/health` | No | Daemon status, memory counts, Ollama status |
| `GET` | `/savings` | No | Token savings analytics |
| `GET` | `/digest` | No | Daily health digest with activity summary |
| `GET` | `/peek?q=<query>` | No | Headlines-only recall (minimal tokens) |
| `GET` | `/dump` | Yes | Full memory export |
| `GET` | `/sessions` | No | Active agent sessions |
| `GET` | `/locks` | No | Active file locks |
| `GET` | `/tasks` | No | Task board |
| `GET` | `/feed` | No | Inter-agent feed entries |
| `POST` | `/store` | Yes | Store a decision with conflict detection |
| `POST` | `/diary` | Yes | Write session state to state.md |
| `POST` | `/forget` | Yes | Decay matching memories by keyword |
| `POST` | `/resolve` | Yes | Resolve a disputed decision pair |
| `POST` | `/lock` | Yes | Acquire a file lock |
| `POST` | `/unlock` | Yes | Release a file lock |
| `POST` | `/session/start` | Yes | Start agent session |
| `POST` | `/session/heartbeat` | Yes | Session keepalive |
| `POST` | `/session/end` | Yes | End agent session |
| `POST` | `/tasks` | Yes | Create a task |
| `POST` | `/tasks/claim` | Yes | Claim a task |
| `POST` | `/tasks/complete` | Yes | Complete a task |
| `POST` | `/tasks/abandon` | Yes | Abandon a task |
| `POST` | `/feed` | Yes | Post to inter-agent feed |
| `POST` | `/message` | Yes | Send message to specific agent |
| `POST` | `/shutdown` | Yes | Graceful daemon shutdown |

Auth: `Authorization: Bearer <token>` (token at `~/.cortex/cortex.token`).

### MCP Tools

| Tool | Description |
|------|-------------|
| `cortex_boot` | Compiled boot prompt with capsule metadata |
| `cortex_recall` | Hybrid search across all memories and decisions |
| `cortex_store` | Store decision with conflict detection and dedup |
| `cortex_diary` | Write session handoff to state.md |
| `cortex_health` | System health check |
| `cortex_digest` | Token savings digest |
| `cortex_forget` | Decay memories matching a keyword |
| `cortex_resolve` | Resolve a dispute between decisions |

### CLI

```bash
cortex boot                    # Print compiled boot prompt
cortex recall "auth tokens"    # Search memories
cortex store "Use uv only"     # Store a decision
cortex health                  # System health
cortex status                  # PID, uptime, counts
cortex forget "deprecated"     # Decay matching entries
cortex resolve 42 --keep 37    # Resolve a conflict
cortex stop                    # Shutdown daemon
```

---

## Connecting Your AI

### Claude Code (MCP + HTTP)

```bash
claude mcp add cortex -s user -- node /path/to/cortex/src/daemon.js mcp
```

The `brain-boot.js` SessionStart hook calls `/boot` mechanically and injects the boot prompt as context - the AI receives it before processing any user message. Add the hook to your Claude Code settings for automatic boot.

### Gemini CLI (HTTP)

Add to `~/GEMINI.md`:

```markdown
## Brain Boot Protocol
At session start, read ~/.claude/brain-status.json and print the oneliner.
Use http://localhost:7437 for memory operations.
```

### Codex CLI (HTTP)

Add to `~/AGENTS.md`:

```markdown
## Brain Boot Protocol
At session start, read ~/.claude/brain-status.json and print the oneliner.
Use http://localhost:7437 for memory operations.
```

### Any Other AI (HTTP)

```bash
# Boot
curl http://localhost:7437/boot?agent=my-agent

# Recall (with token budget)
curl "http://localhost:7437/recall?q=authentication+patterns&budget=500"

# Store (with auth)
curl -X POST http://localhost:7437/store \
  -H "Authorization: Bearer $(cat ~/.cortex/cortex.token)" \
  -H "Content-Type: application/json" \
  -d '{"decision": "Use JWT for API auth", "context": "api-design"}'
```

---

## What's Next

### Near-term

- [ ] **Production Tauri Build** - `.exe` installer with embedded daemon, system tray, minimize-to-tray
- [ ] **Ambient Capture** - PostToolUse hook auto-captures decisions to inbox table with confidence gating
- [ ] **Progressive Memory Aging** - Fresh memories at full fidelity, week-old compressed, month-old as one-liners
- [ ] **Session-Type Classification** - Detect session intent on boot via local Ollama, bias recall accordingly
- [ ] **Co-occurrence Matrix v2** - Upgrade from frequency-based to embedding-based prediction

### Mid-term

- [ ] **Import History** - Ingest ChatGPT, Claude, and Gemini conversation exports into the brain
- [ ] **Ollama Sidecar Workers** - Python workers poll `/tasks`, run local model review on changed files
- [ ] **Port Node Features to Rust Daemon** - Smart recall, co-occurrence, budget recall, savings endpoints
- [ ] **Custom App Icon** - Cortex-branded icon for taskbar/tray/installer

### Long-term

- [ ] **Full Rust Daemon Rewrite** - Eliminate Node.js dependency. Single binary, zero external deps, <5MB
- [ ] **Self-Tuning Compiler** - Track which boot content gets referenced, shift budget to what matters
- [ ] **Decision Provenance DAG** - Trace decision lineage across agents and sessions
- [ ] **Event-Sourced Brain** - Events table as source of truth, current tables as materialized views

---

## Design Principles

1. **Compound, don't accumulate.** Every memory should make the next session smarter, not just bigger. Unused facts decay. Overlapping facts merge. The brain gets denser, not larger.

2. **Push, don't pull.** The brain injects context before being asked. AIs boot already knowing what matters. No "let me check my memory" - it's already there.

3. **Universal interface.** HTTP is the API. Any AI, any language, any platform. MCP is a convenience transport for Claude Code. HTTP is the truth.

4. **Reliability over intelligence.** A brain that crashes is worse than no brain. Every feature is tested. Every mutation is authenticated. Every failure is graceful.

5. **Node for the kernel, Rust for the shell.** The daemon is infrastructure - minimal, fast, boring. The desktop app is Rust/Tauri - native, lightweight, zero-Electron.

6. **Evidence before assertions.** Never claim a feature works without a test. Never claim a bug is fixed without verification output.

---

## License

MIT
