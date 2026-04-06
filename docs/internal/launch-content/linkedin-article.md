# LinkedIn Article

**Headline:** Why AI Coding Agents Need Persistent Memory (and How We Built It)

**Body:**

AI coding assistants are transforming software engineering, but they have a fundamental flaw: **they forget everything between sessions.**

Every time you start a new Claude Code or Cursor session, your AI agent starts from scratch. It has to re-discover your toolchain, re-learn your architectural decisions, and re-process your coding conventions. 

This leads to "context-cold start"—a period of low productivity and high token usage at the beginning of every session. In our benchmarks, we've seen raw file reads at session boot consume up to **20,000 tokens** just to provide the agent with necessary context.

### The Solution: Persistent Brain for AI

We built **Cortex** to solve this problem. 

Cortex is an open-source (AGPL-3.0) Rust daemon that acts as a shared, persistent memory for all your AI agents. 

Rather than re-reading every file, Cortex uses a "capsule compiler" to push an **Identity Capsule** and a **Delta Capsule** into the agent's context window at the start of every session. 

[INSERT hero-divergence chart here]

### The Impact: 97% Token Compression

The results have been massive. Our latest benchmarks show that we can reduce the initial context payload from **19,422 tokens to just 505 tokens served via capsules.** 

That’s a **97% reduction in token consumption on boot**, allowing agents to start every session "warm" and highly productive from the very first prompt. 

[INSERT token-efficiency chart here]

### A Shared Brain for All Tools

One of the most powerful features of Cortex is that it’s **multi-agent native.** 

The decisions your agent makes in Claude Code are immediately available to your agents in Cursor or the Gemini CLI. It provides a single, unified source of truth for every AI tool on your machine. 

### Built for Teams

We’ve also implemented **Team Mode**, allowing engineering teams to share a persistent collective memory on a shared server. This ensures that architecture decisions made by one team member are instantly available to every other agent on the team. 

[INSERT cumulative-savings chart here]

### How to Get Started

Cortex is available as a Claude Code plugin, a standalone desktop app, or a CLI tool built from source. 

**Install the plugin with 2 lines of bash:**
```bash
claude plugin marketplace add AdityaVG13/cortex
claude plugin install cortex@cortex-marketplace
```

Check out the full source code and documentation on GitHub: https://github.com/AdityaVG13/cortex

We’re excited to see how this changes your AI-driven development workflow. Try it out and let us know your feedback!

#AI #SoftwareDevelopment #OpenSource #ClaudeCode #RustLang #SoftwareEngineering
