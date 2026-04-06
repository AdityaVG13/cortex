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

- **Your decisions persist.** Architecture choices, coding conventions, and debugging lessons are remembered and surfaced in future sessions.
- **Every session starts warm.** No more re-explaining your toolchain or project structure. Claude already knows.
- **Works across your tools.** A shared brain for Claude Code, Cursor, Gemini CLI, and any MCP-compatible tool.

## Installation

### Claude Code Plugin (Recommended)

Cortex is now available as a Claude Code plugin. This is the fastest way to get started.

```bash
claude plugin marketplace add AdityaVG13/cortex
claude plugin install cortex@cortex-marketplace
```

Start a new session. That's it. Cortex boots automatically.

### Desktop App (Control Center)

The Cortex Control Center provides a visual dashboard for your brain. Download the latest installer for Windows, macOS, or Linux from the [Releases](https://github.com/AdityaVG13/cortex/releases) page.

### From Source

For contributors and power users:

```bash
git clone https://github.com/AdityaVG13/cortex.git
cd cortex/daemon-rs
cargo build --release
```

## First Session Experience

When you start a new session with the Cortex plugin installed, you'll see a boot message:

```text
Brain: READY | Cortex initialized at ~/.cortex | 42 memories
```

Try storing a coding convention:
> "Cortex, remember that we use early returns and avoid nested if statements in this project."

In a future session, Claude will recall this:
> "I've recalled your convention for early returns. I'll ensure the new function follows this pattern."

## Team Mode

Cortex supports shared brains for engineering teams. Run a shared instance on a server and every team member's agent will contribute to and learn from the same collective memory.

1. Run `cortex serve --host 0.0.0.0` on your shared server.
2. Team lead runs `cortex setup --team` to create API keys.
3. Team members enter the server URL and their API key when prompted by the plugin.

See the [Team Mode Setup Guide](docs/team-mode-setup.md) for full details.

## How It Works

Cortex is a high-performance Rust daemon that lives at `~/.cortex`. It uses an embedded SQLite database for persistence and in-process ONNX embeddings for lightning-fast semantic search. 

When an agent starts a session, it calls the `/boot` endpoint. Cortex compiles an **Identity Capsule** (who you are and your global rules) and a **Delta Capsule** (what changed since your last session). These are pushed into the agent's context window, ensuring it's "warm" from the first prompt.

- **No external dependencies:** No Ollama or external API calls required for embeddings.
- **MCP Native:** Speaks the Model Context Protocol for seamless integration.
- **Surgical Recall:** Sub-100ms hybrid search ensures the right memory is found at the right time.

## CLI Reference

| Command | Description |
|---------|-------------|
| `cortex serve` | Start the Cortex daemon |
| `cortex paths --json` | Output canonical file and port paths in JSON |
| `cortex plugin ensure-daemon` | Internal: verify or start local daemon for plugin |
| `cortex plugin mcp` | Internal: bridge MCP stdio to Cortex HTTP API |
| `cortex setup --team` | Initialize team mode and generate API keys |
| `cortex export` | Export all memories to JSON |
| `cortex import` | Import memories from a JSON file |

## License

Cortex is licensed under the [AGPL-3.0-only](LICENSE) license.

[Contributing](CONTRIBUTING.md) | [Security](SECURITY.md) | [Desktop App](https://github.com/AdityaVG13/cortex/releases)
