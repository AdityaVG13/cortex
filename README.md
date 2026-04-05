<p align="center">
  <img src="assets/cortex-header.gif" alt="CORTEX" width="100%">
</p>

<h3 align="center">A persistent, self-improving brain for AI coding agents.</h3>
<h4 align="center">Single Rust binary. Zero runtime dependencies. In-process ONNX embeddings.</h4>

<p align="center">
  <a href="https://github.com/AdityaVG13/cortex/releases/tag/v0.4.0"><img src="https://img.shields.io/badge/release-v0.4.0-blue?style=for-the-badge" alt="Release v0.4.0"></a>
  <a href="https://github.com/AdityaVG13/cortex/blob/master/LICENSE"><img src="https://img.shields.io/badge/License-AGPL--3.0--only-blue?style=for-the-badge" alt="License: AGPL-3.0-only"></a>
  <a href="https://github.com/AdityaVG13/cortex"><img src="https://img.shields.io/badge/Rust-1.78+-orange?style=for-the-badge" alt="Rust"></a>
  <a href="https://github.com/AdityaVG13/cortex"><img src="https://img.shields.io/badge/ONNX-embedded-blueviolet?style=for-the-badge" alt="ONNX"></a>
  <a href="https://github.com/AdityaVG13/cortex"><img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey?style=for-the-badge" alt="Platform"></a>
</p>

<p align="center"><strong>Current release:</strong> <a href="https://github.com/AdityaVG13/cortex/releases/tag/v0.4.0">v0.4.0</a> (download below)</p>

<p align="center">
  <a href="#installation">Installation</a> --
  <a href="#quick-start">Quick Start</a> --
  <a href="#desktop-app">Desktop App</a> --
  <a href="#how-it-works">How It Works</a> --
  <a href="#features">Features</a> --
  <a href="#connecting-your-ai">Connect Your AI</a> --
  <a href="#api-reference">API Reference</a> --
  <a href="#security">Security</a> --
  <a href="#community">Community</a> --
  <a href="#roadmap">Roadmap</a>
</p>

---

AI coding assistants forget everything between sessions. Every conversation starts from scratch -- re-discovering your toolchain, conventions, and past decisions. Burning tokens and patience.

Cortex gives every AI a shared brain that persists, compresses, and pushes context before being asked. It works with Claude Code, Cursor, and any tool that speaks HTTP or MCP.

**By the numbers:**
- 97% token compression on boot (19K raw -> ~500 tokens served)
- Sub-100ms recall with hybrid semantic + keyword search
- Bearer auth on all sensitive endpoints, CORS-locked to localhost
- 13 MCP tools, 35+ HTTP endpoints, SSE real-time stream
- Zero external runtime dependencies -- no Ollama or Python required

| You want to... | Cortex gives you... |
|---|---|
| Stop repeating setup and project context to every agent | Capsule-compiled boot prompts with durable identity + recent delta |
| Share decisions across Claude Code, Cursor, Codex, Gemini, and others | A single local brain with HTTP and MCP access |
| Keep memory local and fast | Rust daemon, SQLite persistence, and in-process ONNX embeddings |
| Avoid silent contradictions between agents | Conflict detection and human-resolvable disputed decisions |
| Manage the system visually | A desktop control center for graph exploration, activity, tasks, and conflicts |

---

## Features

Built for local-first AI workflows where multiple coding agents need shared memory without shared confusion.

- **Persistent shared memory for multiple AIs:** Claude Code, Cursor, Codex, Gemini, and any HTTP/MCP-capable tool can read and write the same brain.
- **Token-aware context delivery:** Capsule boot, peek, unfold, and recall flows minimize startup/context costs without losing key history.
- **Local-first architecture:** Rust daemon, SQLite persistence, and in-process ONNX embeddings keep the core stack self-contained.
- **Conflict-aware knowledge model:** Contradictory decisions are surfaced instead of silently overwritten.
- **Desktop control plane:** A Tauri app provides graph exploration, task coordination, live agent activity, and conflict resolution.

---

## Installation

Choose the release binary if you want the fastest path to a working daemon. Build from source if you are developing Cortex itself or need to inspect internals.

### Download (recommended)

Grab the latest release from [GitHub Releases](https://github.com/AdityaVG13/cortex/releases/tag/v0.4.0):

| Platform | Download |
|----------|----------|
| **Windows (x86_64)** | [`cortex-v0.4.0-windows-x86_64.zip`](https://github.com/AdityaVG13/cortex/releases/download/v0.4.0/cortex-v0.4.0-windows-x86_64.zip) |
| **macOS (arm64)** | [`cortex-v0.4.0-macos-aarch64.tar.gz`](https://github.com/AdityaVG13/cortex/releases/download/v0.4.0/cortex-v0.4.0-macos-aarch64.tar.gz) |
| **Linux (x86_64)** | [`cortex-v0.4.0-linux-x86_64.tar.gz`](https://github.com/AdityaVG13/cortex/releases/download/v0.4.0/cortex-v0.4.0-linux-x86_64.tar.gz) |

Extract the archive and place the binary on your PATH. On Windows, the Control Center app handles the daemon lifecycle and auto-updates automatically.

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

This is the shortest path from zero to a running local brain.

1. **Start the daemon:**
   ```bash
   cortex serve
   ```

2. **Verify it's alive:**
   ```bash
   curl http://localhost:7437/health
   # {"status":"ok","stats":{"memories":0,"decisions":0,"embeddings":0}}
   ```

3. **Read the connection guide:**
   - Start with [docs/CONNECTING.md](docs/CONNECTING.md) for auth, `/boot`, and platform-specific setup.
   - `/health` is public; most useful endpoints require the bearer token in `~/.cortex/cortex.token`.

For the best experience, use the [Desktop App](#desktop-app).

---

## Desktop App

Cortex Control Center is a Tauri-powered desktop application with a Jarvis-inspired UI for managing your AI brain.

### Control Center Features
- **Visual Memory Graph:** 3D force-directed visualization of semantic relationships.
- **Agent Coordination:** Live monitoring of active agent sessions and heartbeats.
- **Task & Feed Management:** Shared Kanban board and inter-agent message feed.
- **Conflict Resolution:** Side-by-side dispute resolution UI for contradictory AI memories.
- **Auto-Updater:** Built-in update management for Windows users via `tauri-plugin-updater`.
- **System Tray:** Runs silently in the background with quick access to logs and status.

---

## Connecting Your AI

Every integration reduces to the same pattern: boot for context, recall when needed, and store durable decisions back into the brain.

- **Claude Code:** `claude mcp add cortex -s user -- /path/to/cortex mcp`
- **Cursor:** Add Cortex to `~/.cursor/mcp.json` as an MCP server.
- **Any CLI/agent:** Call `/boot?agent=YOUR_NAME` with the bearer token from `~/.cortex/cortex.token`.
- **Full setup guide:** See [docs/CONNECTING.md](docs/CONNECTING.md) for curl examples, auth, and supported workflows.

---

## How It Works

The system is designed to compress context aggressively without losing the pieces that matter across sessions.

| Component | Description |
|-----------|-------------|
| **Capsule Compiler** | Compiles a minimal boot prompt from **Identity** (stable context) and **Delta** (what changed since last boot) capsules. |
| **In-Process ONNX** | Uses `all-MiniLM-L6-v2` vectors computed locally. No network hops or external LLM requirements for embeddings. |
| **Conflict Detection** | Automatically flags semantic contradictions between different AIs (e.g., conflicting architectural decisions). |
| **Progressive Recall** | Three-tier retrieval: **Peek** (headlines only) -> **Unfold** (selected full text) -> **Budget Recall** (token-aware search). |
| **Score Decay** | Ebbinghaus-inspired aging. Memories strengthen with use and decay with time, keeping the brain sharp. |

---

## API Reference

The full API surface is larger than the summary below. Use the tables here for orientation, then go to the OpenAPI spec or connection guide when wiring tools up.

### MCP Tools

| Tool | Description |
|------|-------------|
| `cortex_boot` | Get compiled boot prompt with session context. |
| `cortex_peek` | Headlines-only recall (~80% token savings). |
| `cortex_recall` | Hybrid search with token budgeting. |
| `cortex_store` | Persist a decision or insight with conflict detection. |
| `cortex_unfold` | Drill into specific memory/decision/crystal nodes. |
| `cortex_digest` | Daily health digest and token savings analytics. |
| `cortex_focus_start` | Start context checkpoint (sawtooth compression). |
| `cortex_focus_end` | Summarize and consolidate focus session. |
| `cortex_health` | System health check. |
| `cortex_forget` | Decay memories matching a keyword. |
| `cortex_resolve` | Resolve disputed decisions (keep/merge). |

### HTTP Endpoints (Summary)

| Path | Method | Description |
|------|--------|-------------|
| `/boot` | GET | Capsule-compiled boot prompt. |
| `/recall` | GET | Semantic + keyword search. |
| `/store` | POST | Store memory with conflict detection. |
| `/health` | GET | System status and metrics. |
| `/digest` | GET | Activity summary and savings data. |
| `/mcp-rpc` | POST | HTTP-to-MCP JSON-RPC proxy. |
| `/events/stream`| GET | Real-time SSE event stream. |
| `/tasks` | GET/POST| Multi-agent task management. |
| `/locks` | GET/POST| File locking for agent coordination. |

*Full OpenAPI spec available in `specs/cortex-openapi.yaml`.*

---

## Security

Cortex is built for local use first. The defaults assume the daemon is running on your machine and should not be exposed broadly without deliberate hardening.

- **Bearer Authentication:** Sensitive endpoints require the token found in `~/.cortex/cortex.token`.
- **CORS Protection:** Restricted to localhost origins by default to prevent SSRF and unauthorized access.
- **Data Integrity:** SQLite with WAL mode and prepared statements; zero string interpolation in SQL.
- **Process Isolation:** Stale daemon detection validates process identity before termination.

---

## Community

If you are evaluating the project for adoption, these are the root docs that matter most after the README.

- Contribution guide: [CONTRIBUTING.md](CONTRIBUTING.md)
- Security policy: [SECURITY.md](SECURITY.md)
- Code of conduct: [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)
- Changelog: [CHANGELOG.md](CHANGELOG.md)

## Roadmap

The roadmap tracks hardening, governance, and multi-tenant work beyond the current release.

See [ROADMAP.md](ROADMAP.md) for detailed milestone tracking.

- **v0.5.0:** Foundation hardening (TTL, Rollback, Schema migration).
- **v0.6.0:** Governance (Budgets, Retention, Human review).
- **v0.7.0:** Multi-tenant hardening (Privacy, Scoped tokens).
- **v1.0.0:** AI Information Ingester (ChatGPT/Claude/Gemini export import).

---

## License

AGPL-3.0-only -- See [LICENSE](LICENSE) for details.
