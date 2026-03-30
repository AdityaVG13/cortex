# Cortex

**A persistent, self-improving brain for AI coding agents.**

Cortex is a memory daemon that gives Claude Code, Gemini CLI, Codex, Cursor, and any other AI a shared, long-term brain. It stores decisions, detects conflicts between agents, compiles token-efficient boot prompts, and compounds intelligence across sessions — so every conversation starts smarter than the last.

```
You: "Build a game with neural network AI opponents"
AI:  [already knows your toolchain, past research, platform quirks, project conventions]
AI:  [pulls relevant decisions from 200+ sessions instantly]
AI:  [builds the plan in one shot because context is pre-loaded]
```

One daemon. One database. Every AI connects. Knowledge compounds.

---

## Features

- **Capsule Compiler** — Identity + delta capsules compile ~300-token boot prompts (vs 4,000+ raw)
- **Unified Smart Recall** — Hybrid semantic (Ollama embeddings) + keyword search with token budgeting
- **Predictive Cache** — Co-occurrence matrix preloads context you're likely to need next
- **Token Optimization** — Cost ladder (headlines → balanced → full), budget recall, context dedup
- **Cross-Agent Conflict Detection** — Semantic similarity flags contradictions between agents
- **3D Brain Visualizer** — Interactive Three.js visualization of your memory graph
- **Jarvis-Inspired Desktop UI** — Tauri-powered native app with agent dashboard, task board, messaging
- **Analytics** — Real token savings tracking across sessions
- **Memory Explorer** — Browse, search, and manage all stored memories
- **Multi-Agent Orchestration** — File locking, session bus, task board with priority routing
- **Dual Daemon Architecture** — Node daemon (feature-ahead) + Rust/Tauri desktop app

---

## Quick Start

```bash
# Install
cd ~/cortex
npm install

# Start the daemon
node src/daemon.js serve

# Register with Claude Code
claude mcp add cortex -s user -- node C:\Users\aditya\cortex\src\daemon.js mcp

# Verify
curl http://localhost:7437/health

# Launch desktop app (dev mode)
cd desktop/cortex-control-center
npm install
npm run tauri dev
```

The daemon runs on `localhost:7437`. Any AI that can make HTTP requests can use it.

---

## How It Works

### The Capsule Compiler

When an AI boots, Cortex compiles a minimal prompt from two capsules:

**Identity capsule** (~200 tokens, stable): Who you are, platform rules, hard constraints, known sharp edges. Doesn't change between sessions.

**Delta capsule** (~50-100 tokens, fresh): What changed since this specific agent last connected. New decisions, new conflicts, state changes. Only the diff.

Result: ~300 tokens to fully orient any AI, versus 4,000+ tokens of raw file reads.

### Token Optimization

Cortex minimizes context window usage through multiple strategies:

- **Cost Ladder**: Three retrieval modes — headlines (minimal), balanced (default), full (deep dive) — so agents only pull the depth they need
- **Budget Recall**: Token-aware search that fits results within a caller-specified budget, trimming intelligently rather than truncating
- **Predictive Preloading**: Co-occurrence matrix tracks which memories are recalled together, preloading likely-needed context before the agent asks
- **Context Dedup**: Prevents the same information from appearing in both boot prompts and recall results

### Conflict Detection

When Claude stores "Use Python 3.12" and Gemini stores "Use Python 3.10," Cortex detects the semantic conflict via embedding similarity, marks both as disputed, and surfaces the disagreement in every subsequent boot prompt until a human resolves it.

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

- **Node Daemon** (`src/daemon.js`) — Feature-ahead HTTP + MCP server. All new features land here first. Handles memory operations, compilation, conflict detection, session bus, task board, and file locking.
- **Rust/Tauri Desktop App** (`desktop/cortex-control-center/`) — Native Windows app wrapping the daemon with a React frontend. Includes the 3D brain visualizer, Jarvis-inspired UI, and system tray integration.

```
src/
  daemon.js      # HTTP + MCP server, auth, lifecycle, session bus, task board
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
    main.jsx             # React entry point
    styles.css           # UI styles
  src-tauri/
    src/                 # Rust backend (daemon lifecycle, system tray)
    tauri.conf.json      # Tauri configuration
```

**Design constraints:**
- Node daemon: one npm dependency (`sql.js` — SQLite compiled to WASM)
- No native compilation for the daemon, no build step
- Works on Windows 10 without WSL
- Daemon stays alive across sessions, auto-starts on boot
- Desktop app built with Tauri (Rust + React + Vite)

---

## API

### HTTP Endpoints

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `GET` | `/boot?agent=<id>` | No | Compiled boot prompt (capsule system) |
| `GET` | `/recall?q=<query>&budget=<tokens>` | No | Hybrid semantic + keyword search with token budgeting |
| `GET` | `/health` | No | Daemon status, memory counts, Ollama status |
| `GET` | `/savings` | No | Token savings analytics |
| `GET` | `/dump` | No | Full memory dump |
| `GET` | `/sessions` | No | Active agent sessions |
| `GET` | `/locks` | No | Active file locks |
| `GET` | `/tasks` | No | Task board |
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

## Multi-AI Connection

### Claude Code (MCP + HTTP)
```bash
claude mcp add cortex -s user -- node C:\path\to\cortex\src\daemon.js mcp
```

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

## Future

- **Import History** — Ingest ChatGPT, Claude, and Gemini conversation exports into the brain
- **Progressive Memory Aging** — Fresh memories at full fidelity, week-old compressed, month-old as one-liners
- **Local LLM Classification** — Ollama-powered session-type detection on boot for smarter context loading
- **Hierarchical Memory Tree** — Organize memories into project → topic → detail trees for scoped recall
- **Ambient Capture** — Auto-extract decisions from tool use via PostToolUse hooks
- **Dream Compaction** — Nightly deduplication and synthesis via local LLM workers

See [docs/TODO.md](docs/TODO.md) for the full prioritized backlog and [docs/ROADMAP.md](docs/ROADMAP.md) for the phase-by-phase plan.

---

## Design Principles

1. **Compound, don't accumulate.** Every memory should make the next session smarter, not just bigger. Unused facts decay. Overlapping facts merge. The brain gets denser, not larger.

2. **Push, don't pull.** The brain injects context before being asked. AIs boot already knowing what matters. No "let me check my memory" — it's already there.

3. **Universal interface.** HTTP is the API. Any AI, any language, any platform. MCP is a convenience transport for Claude Code. HTTP is the truth.

4. **Reliability over intelligence.** A brain that crashes is worse than no brain. Every feature is tested. Every mutation is authenticated. Every failure is graceful.

5. **Node for the kernel, Rust for the shell.** The daemon is infrastructure — minimal, fast, boring. The desktop app is Rust/Tauri — native, lightweight, zero-Electron. Python workers handle intelligence tasks.

6. **Evidence before assertions.** Never claim a feature works without a test. Never claim a bug is fixed without verification output. The brain holds itself to the same standard it holds its users.

---

## License

MIT
