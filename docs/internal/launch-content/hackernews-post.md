# Hacker News Post: Show HN

**Title:** Show HN: Cortex – Persistent memory for AI coding agents (Rust, MCP)

**Body:**

Cortex is a high-performance Rust daemon that provides persistent, shared memory for AI coding assistants like Claude Code, Cursor, and the Gemini CLI.

The core problem we're solving is "context-cold start." AI agents currently forget everything between sessions, forcing users to re-explain their toolchains, architectural decisions, and coding conventions every time they start a new session. This is both token-inefficient and cognitively taxing.

Cortex addresses this with a local SQLite database and a "capsule compiler." When an agent boots, Cortex serves an **Identity Capsule** (who the user is and their global rules) and a **Delta Capsule** (what changed since the last boot). These are pushed into the agent's context window, allowing it to start every session "warm."

Technically, Cortex is a single Rust binary with zero external runtime dependencies. We use in-process ONNX embeddings (all-MiniLM-L6-v2) for sub-100ms semantic recall. It supports both solo mode (local daemon) and team mode (remote server with API keys).

**Benchmarks:**
- 97% token reduction on session boot (19K raw -> 505 tokens served)
- 59% precision on average recall (currently working on improved semantic deduplication for v0.5.0)
- 4M+ cumulative tokens saved across our internal test agents

The project is open source (AGPL-3.0) and designed to be a "shared brain" across all the AI tools on your machine.

**GitHub:** https://github.com/AdityaVG13/cortex

We’d love to hear your thoughts on the architecture and the multi-agent convergence model.
