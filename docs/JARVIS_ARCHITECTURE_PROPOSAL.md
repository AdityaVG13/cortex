# Project JARVIS: Cortex Master Architecture Proposal

**Author:** Gemini CLI (Senior AI Architect)
**Objective:** Transform Cortex from a passive SQLite memory store into a proactive, zero-token context engine and long-term autonomous brain.

---

## The Core Challenge: The "Token Tax"
Currently, every time an AI spins up, it must read a `bootPrompt`. Even if it's compressed, it costs tokens and latency. As the project scales, reading past decisions, debugging steps, and architectural rules becomes prohibitively expensive. 

**The Goal:** Instant context loading *without* spending input tokens on redundant information.

---

## Pillar 1: Zero-Token Context Injection (Context Caching)

Modern LLM APIs (Anthropic, Google Gemini, OpenAI) now support **Prompt Caching / Context Caching**. Cortex must evolve from a "text provider" to a **Cache Manager**.

### How it works:
1. **The Pre-Warmed Brain:** Cortex runs as a background daemon. Every 5-10 minutes (or triggered by a git commit/file save), Cortex compiles the *entire* state of the workspace (all recent decisions, active bugs, current codebase map) into a massive context block.
2. **API Injection:** Cortex sends this block to the LLM provider's Caching API. The provider returns a `cache_id`.
3. **Zero-Token Booting:** When I (Gemini CLI), Claude, or Codex boot up, we do *not* ask Cortex for the text. We ask Cortex for the `cache_id`. We pass this `cache_id` in our first API call.
4. **The Result:** The AI instantly "remembers" 100,000+ tokens of project history, user preferences, and recent debates. **Latency drops by 80%, and input token cost drops by 90%.**

### Implementation Steps:
*   Modify `daemon.js` to securely hold API keys for Anthropic/Google.
*   Create a background worker in Cortex that periodically bundles `memories`, `decisions`, and `state.md` and pushes them to the respective API cache endpoints.
*   Update the `/boot` endpoint to return caching pointers instead of raw text when applicable.

---

## Pillar 2: The Tiered Memory System (L1, L2, L3)

Just like a computer, JARVIS needs RAM and a Hard Drive. Currently, Cortex treats everything similarly.

### L1: Working Memory (RAM)
*   **What it is:** What the user and AI are doing *right now*. The current file, the active bug, the immediate intent.
*   **Storage:** The LLM Context Window + API Cache.
*   **Management:** Handled via the `/diary` and `state.md` sync.

### L2: Episodic Memory (Fast Storage)
*   **What it is:** Things learned in the past week, or related specifically to the current file.
*   **Storage:** Local Vector Database (Ollama embeddings in SQLite, which you currently have).
*   **Management (The Upgrade):** Cortex should monitor the user's active window (via a VSCode extension or OS hook). When the user opens `auth.js`, Cortex *automatically* performs a semantic search for "authentication" and silently pushes those memories into the L1 Cache. By the time the user asks a question, the AI already knows the history of `auth.js`.

### L3: Semantic/Archival Memory (Cold Storage)
*   **What it is:** Universal rules, deeply held preferences, old completed projects.
*   **Storage:** Compressed knowledge graphs.
*   **Management:** See Pillar 4 (The Subconscious).

---

## Pillar 3: Ambient Knowledge Graphs (Beyond Flat Tables)

Currently, Cortex stores memories as isolated rows in a database. JARVIS needs to understand *relationships*.

### The Problem:
If I learn that "Windows PowerShell handles quotes differently," and later learn that "Script X failed to run," Cortex doesn't inherently link them unless I search for the right keyword.

### The Upgrade:
*   Move toward a lightweight graph structure within SQLite. 
*   **Nodes:** `Entities` (Files, Agents, Errors, Concepts).
*   **Edges:** `Relationships` ("caused_by", "depends_on", "resolved_by").
*   When an AI queries "Why did `deploy.sh` fail?", Cortex traverses the graph: `deploy.sh` -> *depends_on* -> `aws_cli` -> *has_known_issue* -> "Windows quote escaping".

---

## Pillar 4: Autonomous Background Processing (The Subconscious)

A true JARVIS doesn't just sleep when the user isn't typing. It processes.

### The Feature: "Cortex Dreaming"
1.  **Deduplication & Synthesis:** Overnight, Cortex spins up a local model (like Ollama's Qwen or Llama 3 8B) and has it read through the day's `memories` and `decisions`.
2.  **Compression:** If I (Gemini) solved a bug, and Claude solved a similar bug, Cortex's "dream" process realizes they are the same issue. It creates a single, highly refined "Master Rule" and deletes the redundant entries.
3.  **Conflict Auto-Resolution:** If the graph detects conflicting rules (e.g., "Use Python 3.10" vs "Use Python 3.12"), the subconscious model flags it or attempts a logical resolution based on timestamps, alerting the user in the morning.
4.  **Temporal Decay:** The subconscious model runs the decay algorithms, slowly lowering the score of obsolete decisions.

---

## Immediate Next Steps for Phase 2

To start building this JARVIS architecture, here is the prioritized strike list:

1.  **Build the "Context Caching" layer in `daemon.js`:** This is the highest ROI. We need to integrate Anthropic Prompt Caching and Gemini Context Caching APIs directly into the daemon.
2.  **Implement Automated Context Pushing ("Push-on-Connect"):** The `cortex.js` daemon needs a webhook or polling system to know when a file is opened or a session starts, so it can pre-load L2 memory into L1.
3.  **Implement the Subconscious Worker:** A lightweight cron job within the Node.js daemon that uses local Ollama to compress and decay memories without costing API credits.

---
*End of Document*