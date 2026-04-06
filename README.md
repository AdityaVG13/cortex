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

<p align="center"><a href="https://ko-fi.com/adityavg13">☕ Support Cortex</a> -- all donations go directly toward funding AI projects (API costs, compute, tooling)</p>

Claude Code remembers everything. Across every session.

AI coding assistants forget everything between sessions. Every conversation starts from scratch -- re-discovering your toolchain, conventions, and past decisions. Cortex gives every AI a shared brain that persists, compresses, and pushes context before being asked.

| You want to... | Cortex gives you... |
|---|---|
| Stop repeating setup and project context | Capsule-compiled boot prompts with durable identity + recent delta |
| Share decisions across every tool | A single local brain for Claude Code, Cursor, Gemini, and more |
| Keep memory local and fast | Rust daemon, SQLite persistence, and in-process ONNX embeddings |
| Avoid contradictions | Conflict detection and human-resolvable disputed decisions |
| Manage the system visually | A desktop control center for graph exploration and task coordination |

- **Your decisions persist.** Architecture choices and debugging lessons are remembered and surfaced.
- **Every session starts warm.** No more re-explaining your toolchain. Claude already knows.
- **97% token efficiency.** Boot context is compressed from 19K+ raw tokens down to ~500.

## Installation

### Claude Code Plugin (Recommended)
Cortex is now available as a primary Claude Code plugin. This handles daemon lifecycle automatically.

```bash
claude plugin marketplace add AdityaVG13/cortex
claude plugin install cortex@cortex-marketplace
```
**Restart your session. That's it.**

### Desktop App (Cortex Control Center)
Visual dashboard for your brain. Download for your platform:

| Platform | Download |
|----------|----------|
| **Windows** | [`cortex-v0.4.0-windows-x86_64.zip`](https://github.com/AdityaVG13/cortex/releases/download/v0.4.0/cortex-v0.4.0-windows-x86_64.zip) |
| **macOS** | [`cortex-v0.4.0-macos-aarch64.tar.gz`](https://github.com/AdityaVG13/cortex/releases/download/v0.4.0/cortex-v0.4.0-macos-aarch64.tar.gz) |
| **Linux** | [`cortex-v0.4.0-linux-x86_64.tar.gz`](https://github.com/AdityaVG13/cortex/releases/download/v0.4.0/cortex-v0.4.0-linux-x86_64.tar.gz) |

### From Source
```bash
git clone https://github.com/AdityaVG13/cortex.git
cd cortex/daemon-rs && cargo build --release
```

## First Session Experience
When you start a session with Cortex, you'll see:
```text
Brain: READY | Cortex initialized at ~/.cortex | 42 memories
```
**The "Aha" moment:** Store a convention like *"Cortex, remember we use early returns here."* In any later session, ask for a code review—Claude will recall that specific convention without being reminded.

## Team Mode
Run a shared instance on a server to give your whole engineering team a collective memory.
1. Run `cortex serve --host 0.0.0.0` on a server.
2. Initialize with `cortex setup --team`.
3. Members enter the server URL and API key when prompted by the plugin.
*Detailed guide: [Info/team-mode-setup.md](Info/team-mode-setup.md)*

## How It Works
Cortex is a high-performance Rust daemon living at `~/.cortex`. It uses an embedded SQLite DB and in-process ONNX embeddings for lightning-fast semantic search.

| Component | Description |
|-----------|-------------|
| **Capsule Compiler** | Compiles boot prompts from **Identity** (stable) and **Delta** (recent) capsules. |
| **In-Process ONNX** | Uses `all-MiniLM-L6-v2` locally. No network hops or Ollama required. |
| **Conflict Detection** | Flags semantic contradictions between different AIs automatically. |
| **Progressive Recall** | Three-tier retrieval: **Peek** (headlines) -> **Unfold** (full text) -> **Recall** (search). |

*Full connection guide: [Info/connecting.md](Info/connecting.md)*

## Core MCP Tools
These tools are injected into your agent's context automatically:
- `cortex_boot`: Get compiled boot prompt with session context.
- `cortex_recall`: Hybrid semantic + keyword search with token budgeting.
- `cortex_store`: Persist a decision or insight with conflict detection.
- `cortex_digest`: Daily health digest and token savings analytics.
- `cortex_status`: Check brain health and connection stats.

## CLI Reference
| Command | Description |
|---------|-------------|
| `cortex serve` | Start the Cortex daemon |
| `cortex paths --json` | Output canonical file and port paths |
| `cortex plugin mcp` | Bridge MCP stdio to Cortex HTTP API |
| `cortex setup --team` | Initialize team mode and generate API keys |
| `cortex export/import` | Bulk memory management |

## Security & Roadmap
- **Security:** Bearer auth required (`~/.cortex/cortex.token`), CORS-locked to localhost. See [Info/security-rules.md](Info/security-rules.md).
- **v0.5.0:** Foundation hardening (TTL, Rollback, Schema migration).
- **v0.6.0:** Governance (Budgets, Retention, Human review).
- **v1.0.0:** Multi-agent information ingesters.

*Full Roadmap: [Info/roadmap.md](Info/roadmap.md)*

[Contributing](CONTRIBUTING.md) | [Security Rules](Info/security-rules.md) | [Changelog](CHANGELOG.md) | [License](LICENSE)
