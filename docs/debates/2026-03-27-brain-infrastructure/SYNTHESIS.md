# Debate Synthesis: Brain Infrastructure Upgrade
**Date:** 2026-03-27 | **Models:** 5 of 7 (Gemini rate-limited, Codex CLI error)
**Participants:** Opus (Architect), Sonnet (Pragmatist), Qwen 2.5 32B, DeepSeek R1, Qwen3 8B

---

## Vote Tallies

### Architecture

| Question | Consensus | Vote | Dissent |
|----------|-----------|------|---------|
| **Q1: Merge brain+OMEGA?** | **MERGE** | 4-1 | DeepSeek: keep separate (fears cascading failures) |
| **Q2: OMEGA every prompt?** | **NO** — event-based only | 4-1 | Qwen32B: yes for full coordination |
| **Q3: Startup sequence?** | **Sequential + fallback** | 5-0 | Unanimous |
| **Q4: Who owns what?** | **Cortex owns memory, auto-memory owns persistence** | ~5-0 | Minor differences in naming |

### MemStack Integration

| Question | Consensus | Vote | Dissent |
|----------|-----------|------|---------|
| **Q5: Port to private?** | **Work (Step 0) + Diary (Handoff)** | 4-1 | DeepSeek wants Echo private too |
| **Q6: Add to public fork?** | **Governor** | 4-1 | None serious |
| **Q7: Redundant?** | **State, Grimoire, Compress, Forge, Humanize** | ~4-1 | Some disagreement on Echo |
| **Q8: Anti-rat + Step 0 standard?** | **Step 0: YES. Anti-rat: LEAN YES** | 4-1 (Step 0), 3-2 (anti-rat) | Sonnet: anti-rat adds complexity |
| **Q9: Diary > state.md?** | **YES** — Diary replaces manual state.md | 5-0 | Unanimous |

### Self-Improvement

| Question | Consensus | Vote | Dissent |
|----------|-----------|------|---------|
| **Q10: Self-healing?** | **Loud failures + health checks + auto-restart** | 5-0 | Unanimous |
| **Q11: Vector embeddings?** | **YES** (use nomic-embed-text from ollama) | 4-1 | Sonnet: not yet, prove need first |
| **Q12: Path to JARVIS?** | **Consistency first, then compound** | 5-0 | Unanimous on "one working system > five broken" |

---

## Key Disagreements Resolved

### DeepSeek's "Keep Separate" Argument (Q1)
DeepSeek worried about cascading failures from OMEGA's instability. **Resolution:** The merge eliminates the OMEGA Python process entirely — rewriting coordination in Node.js removes the Windows compatibility class of bugs. The cascading failure risk was from OMEGA crashing, not from merging. Merging INTO a stable platform (Node.js brain-server) solves DeepSeek's concern.

### Qwen 32B's "Every Prompt" OMEGA (Q2)
Qwen32B argued for full coordination on every prompt. **Resolution:** The token and latency cost is too high (4 models disagree). But the INTENT is valid — coordination should happen automatically where needed. The compromise: event-driven hooks (SessionStart, Agent dispatch, session end) cover 95% of coordination needs without the per-prompt tax.

### Sonnet's "Not Yet" on Vectors (Q11)
Sonnet argued current keyword search is "good enough." **Resolution:** The nomic-embed-text model is already installed in ollama (274MB, loaded). The marginal cost of adding embeddings is near-zero since the infrastructure exists. Sonnet's caution about LanceDB as a dependency is valid — so use a simpler approach: embed on index, cosine similarity on query, store in SQLite. No new dependencies.

### Sonnet's "No" on Anti-Rationalization (Q8)
Sonnet called anti-rationalization tables "too complex now." **Resolution:** The tables are 10-15 lines of markdown per skill — they're not complex, they're a checklist. The problem they solve (Claude skipping required steps) was demonstrated TODAY when brain-server was silently broken. Add them to the 5 most critical skills first, not all skills at once.

---

## User's Additional Proposal: Install Full MemStack Framework

The user proposed: **install the entire memstack repo as a skill framework, disable duplicates, and use the novel skills directly.** This is a third option beyond "port individual skills" and "ignore memstack."

### Council Assessment of This Approach

**Pros:**
- Immediately access 77 skills without porting effort
- Get updates from upstream (memstack is actively maintained)
- Skills like Echo's vector search, Diary's SQLite backend, Governor's governance are production-tested across 35+ projects
- The brain-server + cortex merge means we won't be spending thousands of tokens on memory startup, so we have budget for additional skill context

**Cons:**
- 77 skills = massive token cost when listing available skills (mitigated by brain's compressed context)
- MemStack assumes specific paths (`C:/Projects/memstack/`) that need configuration
- Some skills hardcode SQLite paths that differ from our setup
- Risk of skill conflicts with superpowers-aditya's existing skills (same trigger words)

**Recommendation: HYBRID APPROACH**
1. Install memstack as a separate skills directory (not merged with superpowers)
2. Disable duplicates: State, Humanize, Compress, Forge, Grimoire, Scan, Quill, KDP-Format
3. Keep enabled: **Echo** (vector recall), **Diary** (session handoff), **Work** (task planning), **Governor** (tier governance), **Familiar** (parallel dispatch), **Shard** (refactoring), **Sight** (architecture diagrams)
4. Keep all content/marketing/SEO/deployment skills disabled unless explicitly needed
5. Configure memstack's SQLite to share data with Cortex (single truth store)

---

## Final Action Plan (Ordered by Priority)

### Phase 1: Cortex v1 — The Merge (This Session or Next)
1. **Create `cortex.js`** — merge brain.js indexing/recall + OMEGA's store/query/coordination into single Node.js MCP server
2. **Add nomic-embed-text** indexing — embed lessons, diary entries, memory files into SQLite FTS5 + vector column
3. **Self-healing startup:** SessionStart hook auto-starts Cortex if dead, verifies connectivity, prints status banner
4. **Auto-diary:** PostSession hook calls `cortex_diary()` to write structured handoff (replacing manual state.md)
5. **Register as single MCP:** `claude mcp add cortex -s user -- node cortex.js serve`
6. **Remove old MCPs:** brain-server + omega

### Phase 2: MemStack Integration (Next Session)
7. **Clone memstack** to `~/.claude/skills/memstack/`
8. **Configure paths** in `config.local.json` for Windows + our directory structure
9. **Initialize SQLite** — point to Cortex's database for shared storage
10. **Disable duplicates** — State, Humanize, Compress, Forge, Grimoire, + all business/content/deployment skills
11. **Test Echo + Diary + Work + Governor** integration with Cortex

### Phase 3: Standardize Patterns (Following Session)
12. **Add Silent Context Compilation (Step 0)** to top 5 superpowers skills: brainstorming, systematic-debugging, writing-plans, executing-plans, verification-before-completion
13. **Add anti-rationalization tables** to: session-restorer, verification-before-completion, error-recovery, systematic-debugging, premise-check
14. **Add Governor** to public superpowers-aditya fork

### Phase 4: Cortex v2 — Learning Loop (Week 2)
15. **Auto-state generation** — Cortex generates compressed state briefing at session end
16. **Cross-session learning** — each session's lessons improve next session's briefing quality
17. **Predictive context** — Cortex pre-loads relevant memories based on working directory + recent git activity

---

## Debate Statistics
- **Unanimous decisions:** 5 (startup fallback, diary>state.md, self-healing, JARVIS=consistency, who-owns-what)
- **Strong consensus (4-1):** 4 (merge, no-every-prompt, vectors yes, Step 0 standard)
- **Split (3-2):** 1 (anti-rationalization tables)
- **Models participating:** Opus, Sonnet, Qwen 2.5 32B, DeepSeek R1, Qwen3 8B
- **Models unavailable:** Gemini (429 rate limit), Codex (CLI config error)
