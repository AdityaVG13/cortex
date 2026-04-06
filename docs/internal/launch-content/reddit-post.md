# Reddit Post: r/ClaudeAI or r/ClaudeCode

**Title:** I built a persistent brain for Claude Code so it remembers my architecture decisions across every session. 

**Body:**

Hey r/ClaudeAI,

I got frustrated that every time I start a new Claude Code session, I have to re-explain my toolchain, my coding conventions, and the architecture decisions we made yesterday. 

So I built **Cortex**. 

It’s a high-performance Rust daemon that acts as a shared brain for your AI agents. 

### Why this exists:
AI agents currently "forget" everything between sessions. This burns tokens (re-reading files) and patience (re-explaining context). 

Cortex solves this by:
1. **Persistence:** Your decisions and debugging lessons are stored in a local SQLite database.
2. **Context Compression:** At the start of every session, Cortex compiles an "Identity Capsule" and a "Delta Capsule" of recent changes, pushing them directly into Claude's context window.
3. **Multi-Agent Convergence:** If you use Claude Code, Cursor, and the Gemini CLI, they all contribute to and learn from the same brain.

### The Numbers:
We’ve been benchmarking this for weeks. In our latest runs:
- **97% token reduction on boot:** (19,422 raw tokens -> 505 tokens served via capsules).
- **Sub-100ms recall:** Semantic search powered by in-process ONNX embeddings (zero external dependencies).
- **4M+ tokens saved** across our internal test agents.

[INSERT token-efficiency chart here]
[INSERT cumulative-savings chart here]

### How to Install (Claude Code):
```bash
claude plugin marketplace add AdityaVG13/cortex
claude plugin install cortex@cortex-marketplace
```

Start a new session. That’s it. Cortex boots automatically.

### It's Open Source:
Cortex is licensed under AGPL-3.0. You can check out the source, build it from scratch, or run it on a shared team server to give your whole engineering team a collective memory.

**GitHub:** https://github.com/AdityaVG13/cortex

I'd love for you all to try it and tell me what breaks. We're currently working on improving semantic deduplication to hit 70%+ precision on recall.

Happy coding!
