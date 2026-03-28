---
date: 2026-03-27
topic: brain-infrastructure-upgrade
focus: Cortex unified memory, JARVIS-level persistent intelligence, memstack integration
---

# Ideation: Brain Infrastructure Upgrade — Cortex & JARVIS Path

## Codebase Context

5 overlapping memory systems (brain-server, OMEGA, claude-mem, auto-memory, state.md) being merged into unified "Cortex" Node.js MCP server. Adding vector embeddings (nomic-embed-text), memstack framework (77 skills), anti-rationalization patterns. Prior 5-model debate reached consensus on merge architecture. User also wants to install full memstack and disable duplicates.

## Ranked Ideas

### 1. Brain as Compiler, Not Database
**Description:** Instead of querying memory at runtime and loading raw files, treat session start as a compilation step. Cortex reads all memory systems, resolves contradictions, prunes stale entries, and emits a minimal ~300-token "boot prompt" that encodes priors, preferences, active project state, and top 5 relevant decisions. Every session starts from a compiled artifact, not raw reads.
**Rationale:** This is the single most powerful reframe. Current approach loads 10+ files costing 4,600+ tokens. A compiled boot prompt compresses that to 300 tokens with higher relevance because the compiler can do consistency checks, conflict resolution, and importance weighting. It also means skills and memstack don't add to startup cost — the compiler absorbs them.
**Downsides:** Compilation logic must be robust — a bad compiler produces bad context. Needs careful testing. The compiler itself needs a ~200 LOC implementation in cortex.js.
**Confidence:** 90%
**Complexity:** Medium
**Status:** Unexplored

### 2. Ambient Episodic Compression via Local LLM
**Description:** At session end, a background process sends the session transcript to Qwen 2.5 32B (already running in Ollama) which extracts 3-5 "episodes" — compressed decision records with context, participants, tools used, outcome, and surprise level. These feed the knowledge base automatically without requiring Claude to self-report or the user to write state.md.
**Rationale:** Claude's self-reporting of lessons is inconsistent and token-expensive. Qwen does the extraction for free (already running, no API cost). Over 200 sessions this generates a dense episodic memory that makes every session start with richer context. The compounding effect is enormous because each session adds training signal at zero marginal cost.
**Downsides:** Qwen 32B runs slow on CPU-only (user has no GPU). Session transcripts can be large. Need to handle encoding issues on Windows (cp1252).
**Confidence:** 85%
**Complexity:** Medium
**Status:** Unexplored

### 3. Memory Decay + Reinforcement Scoring
**Description:** Every memory gets a half-life. Each time a memory is retrieved AND actually used to solve a problem, its score increases. Memories never retrieved decay toward deletion after 30 days. Vector similarity search becomes more discriminating as noise drops, which improves retrieval, which further prunes noise — a self-reinforcing loop.
**Rationale:** All memories are currently treated equally. A lesson learned once and never needed again occupies the same retrieval space as a pattern that fires every session. This mirrors biological memory consolidation (unused synaptic connections prune during sleep) and spaced repetition systems (Anki).
**Downsides:** Aggressive decay could delete a memory that's needed rarely but critically (e.g., a security fix). Need a "pinned" flag for user-critical memories.
**Confidence:** 85%
**Complexity:** Low-Medium (scoring on read/write, cron job for decay)
**Status:** Unexplored

### 4. Predictive Context Injection (Push, Don't Pull)
**Description:** Instead of Claude querying memory reactively, Cortex monitors the first 2-3 tool calls of a session, classifies the task domain (git, browser, Python, job apps), and injects the 3-5 most relevant memories into context before the first real response. The brain predicts what Claude needs before being asked.
**Rationale:** The CLAUDE.md rule "call brain_recall BEFORE file reads" exists because Claude's default is to skip it. A push model makes the rule unnecessary by making skipping impossible. Combined with the compiler (Idea 1), this means zero-cost orientation for every session.
**Downsides:** Mis-classification injects wrong context. Needs a fallback for novel tasks. Initial classifier needs training data from session history.
**Confidence:** 80%
**Complexity:** Medium
**Status:** Unexplored

### 5. Decision Replay Cache (Not Facts, Decisions)
**Description:** Stop storing what happened. Start storing what was decided and why. The cortex logs decisions (with context snapshots, alternatives considered, outcome), not facts. Session start loads the 5 most relevant prior decisions, not a full state file. This reframes the entire architecture from "general-purpose memory" to "specialized decision cache."
**Rationale:** The real pain isn't "Claude forgot what happened." It's "Claude re-derived the same decision at 10x the cost." State.md, MEMORY.md, OMEGA, brain-server, claude-mem — each is a different attempt to solve decision replay using storage primitives. One decision cache replaces all five. Value density is much higher than fact storage.
**Downsides:** Requires discipline in what counts as a "decision" vs a fact. Some context (project status, pending tasks) isn't a decision.
**Confidence:** 80%
**Complexity:** Medium
**Status:** Unexplored

### 6. Behavioral Drift Detector
**Description:** After each session, compute a diff between what Cortex's rules say Claude should do (read CLAUDE.md, memory feedback files) and what it actually did (inferred from tool call logs). Surface violations as scored findings. The detection mechanism becomes automated, not reliant on the user noticing.
**Rationale:** The current self-improvement loop depends on Aditya catching drift and filing a feedback memory. This is highest-friction, lowest-reliability detection. Composer 2 research confirms behavioral rewards work for this. Over time, the drift detector improves the rules themselves by surfacing which rules are routinely violated (too strict? misunderstood? obsolete?).
**Downsides:** False positives could be noisy. Needs careful rule parsing. Tool call logs may not capture intent clearly.
**Confidence:** 75%
**Complexity:** Medium-High
**Status:** Unexplored

### 7. Liveness Receipts (Memory Proves Itself Alive)
**Description:** Invert health checks. Instead of polling to check if memory is alive, every memory read/write returns a liveness receipt. A watchdog middleware checks for the receipt on each turn. If absent, it injects a visible warning before the next response. Silence itself becomes the alarm.
**Rationale:** Memory was silently broken for months because the model requires active polling. Inverting means the system announces its own failure. This is how TCP keepalive works — the absence of a heartbeat IS the signal, not a periodic check.
**Downsides:** Adds a small amount of overhead per memory operation. Watchdog must be reliable itself (turtles all the way down).
**Confidence:** 85%
**Complexity:** Low
**Status:** Unexplored

---

## Cross-Cutting Synthesis

**The JARVIS Loop:** Ideas 1 + 2 + 3 + 5 form a closed compounding system:
- **Ambient Compression** (2): Local LLM extracts decisions from each session → feeds decision cache
- **Decision Replay** (5): Cortex stores decisions, not facts → focused knowledge base
- **Memory Decay** (3): Unused decisions fade, reinforced ones strengthen → self-cleaning
- **Compiler** (1): At session start, compile top decisions into 300-token boot prompt → zero-cost orientation

Each session adds signal → compiler gets better inputs → next session starts smarter → generates more signal. This is the compounding flywheel.

---

## Rejection Summary

| # | Idea | Reason Rejected |
|---|------|-----------------|
| 1 | Session Boot Health Check | Already built today (brain-check.js). Done, not an idea. |
| 2 | Auto-Updating State Chronicle | Subsumed by Compiler + Ambient Compression (ideas 1+2). |
| 3 | Semantic Deduplication | Standard practice, not surprising. Included in Compiler logic. |
| 4 | Skill Capability Registry | Practical but low leverage. Can be a simple dependency check. |
| 5 | Failure Taxonomy | Good engineering but not an insight. Included in Liveness Receipts. |
| 6 | Subagent Memory Namespaces | Premature — no evidence of write conflicts yet. |
| 7 | Compressed Context Budgets | Subsumed by Compiler (idea 1) which does this automatically. |
| 8 | MCP Self-Registration | Claude Code API doesn't support programmatic MCP registration. |
| 9 | MEMORY.md as Materialized View | Good but minor. Can be part of Compiler output. |
| 10 | Windows Compat Shim Registry | One-off fixes, not a system. Current approach (patch + feedback memory) works. |
| 11 | Ambient Re-Entry (single boot call) | Subsumed by Compiler (idea 1). |
| 12 | Cross-Session Causal Attribution | Powerful but very high complexity. Phase 3+ idea. |
| 13 | Skill Genealogy Graph | Autoresearch already tracks skill evolution. Redundant. |
| 14 | Task Difficulty Modeling | Subsumed by Predictive Context (idea 4). |
| 15 | Peer Memory Propagation | Premature — multi-agent workflows not yet common enough. |
| 16 | Leverage Index (meta-memory) | Interesting but high complexity. Can emerge from Decay Scoring (idea 3). |
| 17 | Memory as Attention Bias | Too abstract to implement directly. Compiler (idea 1) achieves this. |
| 18 | Skills Fork Like Code | Autoresearch already does this better. |
| 19 | Anti-Library (aggressive forgetting) | Subsumed by Memory Decay (idea 3). |
| 20 | Memory as Social Contract | Too abstract, hard to implement. Namespacing is simpler. |
| 21 | Contradiction Detection | Good, included as part of Compiler step. Not standalone. |
| 22 | Load Balancing for Embeddings | Premature — single user, one machine. |
| 23 | Scalable Memory Architecture | Premature — not a scaling problem. |
| 24 | Automated Performance Monitoring | Standard ops, not an insight. |
| 25 | Fallback for Skill Failures | Standard error handling. |
| 26 | Conflict Detection | Included in Compiler's consistency check. |
| 27 | Redundant Backup | Overkill at current scale. Git is the backup. |

## Session Log
- 2026-03-27: Initial ideation — 38 raw ideas generated across 5 frames (pain, inversion, compounding, assumption-breaking, extreme cases), 7 survived after adversarial filtering. Cross-cutting synthesis identified "JARVIS Loop" from ideas 1+2+3+5.
