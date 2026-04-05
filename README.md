<p align="center">

```
  ______   ____   _____   _______   ______   __   __
 / ____| / __ \ |  __ \ |__   __| |  ____|  \ \ / /
| |     | |  | || |__) |   | |    | |__      \ V /
| |     | |  | ||  _  /    | |    |  __|      > <
| |____ | |__| || | \ \    | |    | |____    / . \
 \_____| \____/ |_|  \_\   |_|    |______|  /_/ \_\
```

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

---

## Installation

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

1. **Start the daemon:**
   ```bash
   cortex serve
   ```

2. **Verify it's alive:**
   ```bash
   curl http://localhost:7437/health
   # {"status":"ok","stats":{"memories":0,"decisions":0,"embeddings":0}}
   ```

3. **Connect your AI:**
   - **Claude Code:** `claude mcp add cortex -s user -- /path/to/cortex mcp`
   - **Cursor:** Add to `~/.cursor/mcp.json`: `{ "mcpServers": { "cortex": { "command": "/path/to/cortex", "args": ["mcp"] } } }`

For the best experience, use the [Desktop App](#desktop-app).

---

## Desktop App

Cortex Control Center is a Tauri-powered desktop application with a Jarvis-inspired UI for managing your AI brain.

### Features
- **Visual Memory Graph:** 3D force-directed visualization of semantic relationships.
- **Agent Coordination:** Live monitoring of active agent sessions and heartbeats.
- **Task & Feed Management:** Shared Kanban board and inter-agent message feed.
- **Conflict Resolution:** Side-by-side dispute resolution UI for contradictory AI memories.
- **Auto-Updater:** Built-in update management for Windows users via `tauri-plugin-updater`.
- **System Tray:** Runs silently in the background with quick access to logs and status.

---

## How It Works

| Component | Description |
|-----------|-------------|
| **Capsule Compiler** | Compiles a minimal boot prompt from **Identity** (stable context) and **Delta** (what changed since last boot) capsules. |
| **In-Process ONNX** | Uses `all-MiniLM-L6-v2` vectors computed locally. No network hops or external LLM requirements for embeddings. |
| **Conflict Detection** | Automatically flags semantic contradictions between different AIs (e.g., conflicting architectural decisions). |
| **Progressive Recall** | Three-tier retrieval: **Peek** (headlines only) -> **Unfold** (selected full text) -> **Budget Recall** (token-aware search). |
| **Score Decay** | Ebbinghaus-inspired aging. Memories strengthen with use and decay with time, keeping the brain sharp. |

---

## API Reference

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

- **Bearer Authentication:** Sensitive endpoints require the token found in `~/.cortex/cortex.token`.
- **CORS Protection:** Restricted to localhost origins by default to prevent SSRF and unauthorized access.
- **Data Integrity:** SQLite with WAL mode and prepared statements; zero string interpolation in SQL.
- **Process Isolation:** Stale daemon detection validates process identity before termination.

---

## Roadmap

See [ROADMAP.md](ROADMAP.md) for detailed milestone tracking.

- **v0.5.0:** Foundation hardening (TTL, Rollback, Schema migration).
- **v0.6.0:** Governance (Budgets, Retention, Human review).
- **v0.7.0:** Multi-tenant hardening (Privacy, Scoped tokens).
- **v1.0.0:** AI Information Ingester (ChatGPT/Claude/Gemini export import).

---

## License

AGPL-3.0-only -- See [LICENSE](LICENSE) for details.
