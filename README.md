<p align="center">
  <img src="assets/cortex-header.gif" alt="CORTEX" width="100%">
</p>

<h3 align="center">A persistent, self-improving brain for AI coding agents.</h3>
<h4 align="center">Single Rust binary. Zero runtime dependencies. In-process ONNX embeddings.</h4>

<p align="center">
  <a href="https://github.com/AdityaVG13/cortex/releases/tag/v0.4.1"><img src="https://img.shields.io/badge/release-v0.4.1-blue?style=for-the-badge" alt="Release v0.4.1"></a>
  <a href="https://github.com/AdityaVG13/cortex/blob/master/LICENSE"><img src="https://img.shields.io/badge/License-MIT-blue?style=for-the-badge" alt="License: MIT"></a>
  <a href="https://github.com/AdityaVG13/cortex"><img src="https://img.shields.io/badge/Rust-1.78+-orange?style=for-the-badge" alt="Rust"></a>
  <a href="https://github.com/AdityaVG13/cortex"><img src="https://img.shields.io/badge/ONNX-embedded-blueviolet?style=for-the-badge" alt="ONNX"></a>
  <a href="https://github.com/AdityaVG13/cortex"><img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey?style=for-the-badge" alt="Platform"></a>
</p>

<p align="center"><strong>Current release:</strong> <a href="https://github.com/AdityaVG13/cortex/releases/tag/v0.4.1">v0.4.1</a> (download below)</p>

<p align="center">
  <a href="#installation">Installation</a> --
  <a href="#first-session-experience">Quick Start</a> --
  <a href="#desktop-app">Desktop App</a> --
  <a href="#how-it-works">How It Works</a> --
  <a href="#core-mcp-tools">MCP Tools</a> --
  <a href="CONNECTING.md">Connect Your AI</a> --
  <a href="#cli-reference">CLI Reference</a> --
  <a href="SECURITY.md">Security</a> --
  <a href="CONTRIBUTING.md">Community</a> --
  <a href="Info/roadmap.md">Roadmap</a>
</p>

---

<p align="center"><a href="https://ko-fi.com/adityavg13">☕ Support Cortex</a> -- all donations go directly toward funding AI projects (API costs, compute, tooling)</p>

<p align="center"><em>The logo is temporary (yes, that's my face). If you have ideas for what the real one should look like, open an issue or let me know!</em></p>

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
- **Up to 97% token compression (tested).** Boot context is compressed from 19K+ raw tokens down to ~500.
- **Self-healing connections.** If the daemon crashes mid-session, the MCP proxy automatically detects the failure and respawns it -- your session stays alive without manual intervention.

## Documentation Map
- [README.md](README.md) -- install + usage overview
- [CONNECTING.md](CONNECTING.md) -- AI/tool integration quickstart
- [SECURITY.md](SECURITY.md) -- threat model and security posture
- [CONTRIBUTING.md](CONTRIBUTING.md) -- development workflow
- [Info/roadmap.md](Info/roadmap.md) -- contributor roadmap

## Installation

### Claude Code Plugin (Recommended)
Cortex is now available as a primary Claude Code plugin. This handles daemon lifecycle automatically.

```bash
claude plugin marketplace add AdityaVG13/cortex
claude plugin install cortex@cortex-marketplace
```
**Restart your session. That's it.**

### Desktop App (Cortex Control Center)
Visual dashboard for your brain. Download the installer for your platform from the [latest release](https://github.com/AdityaVG13/cortex/releases/latest):

| Platform | Installer | Daemon Only |
|----------|-----------|-------------|
| **Windows** | [`.exe` (NSIS installer)](https://github.com/AdityaVG13/cortex/releases/latest) | [`cortex-v0.4.1-windows-x86_64.zip`](https://github.com/AdityaVG13/cortex/releases/download/v0.4.1/cortex-v0.4.1-windows-x86_64.zip) |
| **macOS** | [`.dmg`](https://github.com/AdityaVG13/cortex/releases/latest) | [`cortex-v0.4.1-macos-aarch64.tar.gz`](https://github.com/AdityaVG13/cortex/releases/download/v0.4.1/cortex-v0.4.1-macos-aarch64.tar.gz) |
| **Linux** | [`.AppImage` / `.deb`](https://github.com/AdityaVG13/cortex/releases/latest) | [`cortex-v0.4.1-linux-x86_64.tar.gz`](https://github.com/AdityaVG13/cortex/releases/download/v0.4.1/cortex-v0.4.1-linux-x86_64.tar.gz) |

<p align="center">
  <img src="assets/control-center-analytics.png" alt="Cortex Control Center -- Analytics" width="90%">
  <br>
  <em>Control Center analytics: real-time token savings, boot history, and per-agent compression rates.</em>
</p>

### From Source
```bash
git clone https://github.com/AdityaVG13/cortex.git
cd cortex/daemon-rs && cargo build --release
```

## Compatibility Matrix

| Component | Windows x86_64 | macOS arm64 | Linux x86_64 |
|---|---|---|---|
| Daemon binary (`cortex`) | ✅ | ✅ | ✅ |
| Claude plugin runtime archive | ✅ `.zip` | ✅ `.tar.gz` | ✅ `.tar.gz` |
| Control Center desktop app | ✅ `.exe` | ✅ `.dmg` | ✅ `.AppImage` `.deb` |

## First Session Experience
When you start a session with Cortex, you'll see:
```text
Brain: READY | Cortex initialized at ~/.cortex | 42 memories
```
**The "Aha" moment:** Store a convention like *"Cortex, remember we use early returns here."* In any later session, ask for a code review--Claude will recall that specific convention without being reminded.

## Team Mode
Run a shared instance on a server to give your whole engineering team a collective memory.
1. Run `CORTEX_BIND=0.0.0.0 cortex serve` on a server.
2. Initialize with `cortex setup --team`.
3. Members enter the server URL and API key when prompted by the plugin.
*Detailed guide: [Info/team-mode-setup.md](Info/team-mode-setup.md)*

## How It Works
Cortex is a high-performance Rust daemon living at `~/.cortex`. It uses an embedded SQLite DB and in-process ONNX embeddings for lightning-fast semantic search.

| Component | Description |
|-----------|-------------|
| **Capsule Compiler** | Compiles boot prompts from **Identity** (stable) and **Delta** (recent) capsules. |
| **In-Process ONNX** | Uses `all-MiniLM-L6-v2` locally with no external inference service. |
| **Conflict Detection** | Flags semantic contradictions between different AIs automatically. |
| **Progressive Recall** | Three-tier retrieval: **Peek** (headlines) -> **Unfold** (full text) -> **Recall** (search). |
| **Auto-Respawn** | MCP proxy detects daemon failure and restarts it automatically -- sessions survive crashes. |

*Full connection guide: [CONNECTING.md](CONNECTING.md)*

## Core MCP Tools
These tools are injected into your agent's context automatically:
- `cortex_boot`: Get compiled boot prompt with session context.
- `cortex_recall`: Hybrid semantic + keyword search with token budgeting.
- `cortex_store`: Persist a decision or insight with conflict detection.
- `cortex_digest`: Daily health digest and token savings analytics.
- `cortex_health`: Check brain health and connection stats.

Full tool list and parameters: [Info/mcp-tools.md](Info/mcp-tools.md)

## CLI Reference
| Command | Description |
|---------|-------------|
| `cortex serve` | Start the Cortex daemon |
| `cortex paths --json` | Output canonical file and port paths |
| `cortex plugin ensure-daemon` | Start (or reuse) daemon with migration + lock safety |
| `cortex plugin mcp` | Bridge MCP stdio to Cortex HTTP API |
| `cortex setup --team` | Initialize team mode and generate API keys |
| `cortex export` | Export data (`--format json|sql`) |
| `cortex import` | Import JSON export into solo/team mode |

## Known Limitations
- **Recall ranking is approximate.** Semantic search works well for broad queries but may not always surface the most relevant result first. Improving retrieval precision is a priority for v0.5.0.
- **First-run experience varies by platform.** We test on Windows, macOS, and Linux, but edge environments may need minor troubleshooting. If install doesn't work cleanly, open an issue -- we want to hear about it.

## Security & Roadmap
- **Security:** Bearer auth required (`~/.cortex/cortex.token`), CORS-locked to localhost. See [SECURITY.md](SECURITY.md) (mirror: [Info/security-rules.md](Info/security-rules.md)).
- **v0.5.0:** Foundation hardening (TTL, Rollback, Schema migration).
- **v0.6.0:** Governance (Budgets, Retention, Human review).
- **v1.0.0:** Multi-agent information ingesters.

More features coming soon! Check the roadmap or open an issue if there's something you'd like to see.

*Full Roadmap: [Info/roadmap.md](Info/roadmap.md)*

[Contributing](CONTRIBUTING.md) | [Security](SECURITY.md) | [Connecting](CONNECTING.md) | [Changelog](CHANGELOG.md) | [License](LICENSE)
