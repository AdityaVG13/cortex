# Cortex / JARVIS Review

**Author:** Codex (GPT-5)
**Date:** 2026-03-27
**Scope:** Review Gemini's JARVIS proposal, review current Cortex implementation, and recommend the next architecture moves for the stated goal: cut repeated token/context usage so only genuinely new work consumes meaningful tokens.

---

## Executive Summary

Gemini is directionally right about the problem: the real enemy is repeated context tax. Cortex should exist to turn repeated explanation into reusable state.

Where I disagree is the ordering.

The fastest path to "JARVIS" is **not** "add the most ambitious features first." It is:

1. Make Cortex operationally reliable.
2. Make context selection relevance-aware.
3. Make state compaction first-class.
4. Add provider cache adapters only after the state model is stable.

Right now Cortex has a good shape:

- one daemon
- one SQLite store
- one compiler
- one recall path
- one conflict system
- one diary handoff

That is the correct foundation.

But the current implementation is still closer to a promising prototype than a dependable brain. There are contract mismatches between CLI and daemon, missing auth on mutating routes, broken embedding-based conflict detection, brittle keyword recall, and no real test coverage. Until those are fixed, adding ambitious cache orchestration or autonomous "dreaming" will compound instability instead of compounding intelligence.

My core recommendation is:

**Do Phase 0 first: reliability, contracts, auth, observability, tests.**

Then build a **capsule-based context system**:

- stable identity capsule
- project capsule
- workspace capsule
- file/task capsule
- recent delta capsule
- dispute capsule

That gives you the practical version of "zero-token boot": not literally zero, but near-zero repeated project explanation with much tighter relevance control.

---

## Review of Gemini's JARVIS Proposal

## What Gemini got right

### 1. The problem framing is correct

The biggest long-term waste is not inference itself. It is re-sending old project knowledge every time a session starts.

That means Cortex should optimize for:

- reusable compiled context
- small deltas
- fast retrieval of the right facts
- aggressively avoiding repeated narration

That framing is right.

### 2. Tiered memory is the right mental model

The L1/L2/L3 framing is useful:

- L1 for active session state
- L2 for recent or task-adjacent episodic memory
- L3 for durable rules and archived knowledge

The current codebase already gestures at this, but only implicitly. Making it explicit would help both implementation and boot compilation.

### 3. Background consolidation is necessary

Some form of offline compaction is eventually required. Otherwise the system becomes a pile of stale, duplicate, partially overlapping entries, and retrieval quality collapses.

### 4. Relationships matter more than flat notes

Gemini is right that flat rows are not enough forever. Files, bugs, agents, decisions, and workarounds need relationships.

---

## Where I would change Gemini's proposal

### 1. "Zero-token boot" is not the immediate first move

Provider caching is attractive, but it should not be the first major investment.

Why:

- caches expire
- providers differ
- cache invalidation becomes a real system problem
- the underlying compiled state is not stable enough yet
- bad cached context is worse than expensive context

If the base capsules are wrong, stale, or noisy, a cache layer just lets you deliver the wrong thing faster.

**My recommendation:** build provider caching as an adapter layer after the compiled context model has stabilized.

### 2. Do not centralize provider API keys in Cortex by default

Gemini suggests putting Anthropic/Google keys into the daemon. That increases blast radius and turns Cortex into a secrets hub.

Better options:

- let each client own provider auth and ask Cortex only for compiled capsules
- or add an optional cache-adapter mode later with explicit trust boundaries

Make Cortex the state authority, not the mandatory credential broker.

### 3. Ambient auto-store must be gated

The idea is good. The naive version is dangerous.

If Cortex silently stores anything inferred from tool use, it will accumulate:

- weak inferences
- accidental secrets
- transient implementation details
- wrong conclusions stated confidently

**Better model:** ambient capture goes into an inbox with confidence labels and compaction later decides what becomes durable memory.

### 4. Knowledge graph should come after structured references, not before

A graph is useful, but graph infrastructure is easy to overbuild.

Before introducing entity/edge tables, Cortex should first store structured references in ordinary rows:

- `scope`
- `project`
- `file_path`
- `topic`
- `decision_type`
- `supersedes`
- `related_to`

That gets most of the value without immediately paying graph-system complexity.

### 5. "Dreaming" should synthesize, not delete

Gemini proposes making a master rule and deleting redundant entries. I would not delete originals early.

Keep:

- canonical synthesized rule
- source lineage
- superseded / compressed statuses

Do not throw away provenance until the compactor has proven trustworthy.

---

## Review of Current Cortex Implementation

## What is already strong

### 1. The architecture is appropriately simple

`daemon.js`, `brain.js`, `compiler.js`, `db.js`, `embeddings.js`, and `conflict.js` are a good split. The boundaries are understandable.

### 2. One local store is the right default

Using one SQLite-backed store for memories, decisions, embeddings, and events is the right simplification for this stage.

### 3. Profiles are a good primitive

The idea of `full`, `operational`, `subagent`, and `index` profiles is useful, even if the current compiler is still static.

### 4. Diary + recall + conflict detection is the right product surface

Those are the core primitives a cross-agent memory system actually needs.

---

## Code-level concerns

### 1. Reliability comes before intelligence

Cortex is trying to become the always-on substrate for every agent. That makes correctness and availability non-negotiable.

At the moment, several implementation issues undermine that:

- mutating HTTP routes are not consistently protected
- daemon and CLI contracts disagree
- detached lifecycle appears unstable from the CLI path
- there is effectively no test suite

### 2. The current compiler is still static and broad

Gemini criticized this correctly. The compiler currently emits fixed section bundles per profile, and those bundles are not conditioned on task, working directory, file, or recent user intent.

This means Cortex still spends tokens on "possible relevance," not "probable relevance."

### 3. Indexing is batch-at-startup, not incremental

`brain.init()` indexes sources and builds embeddings at startup. That works for bootstrapping but not for maintaining fresh context during a live work session.

Without incremental refresh, Cortex becomes stale quickly.

### 4. Persistence strategy will become the bottleneck

The current sql.js export-and-rewrite approach is acceptable for a small prototype, but it becomes a tax as data grows and as write frequency rises.

Gemini's concern here is valid.

### 5. Keyword fallback is weaker than it looks

The system extracts keywords, then joins them back into a single phrase for SQL `LIKE` matching. That means the fallback is far more brittle than "hybrid search" implies.

### 6. The current system has no real staleness model

There is a `forget()` API and scores, but no continuous policy for:

- recency decay
- confirmation boosts
- archive thresholds
- "hot" vs "cold" memory

Without those, memory quality will degrade over time.

---

## Direct Findings From The Current Code

### 1. Embedding-based conflict detection is effectively broken

`conflict.js` loads a new embedding as a `Float32Array` and existing embeddings as vectors, then passes them into `embeddings.cosineSim()`. But `cosineSim()` calls `blobToVector()`, which only accepts `Buffer` or `Uint8Array`.

That means the embedding path catches and returns `0`, so semantic conflict detection does not really work when embeddings are available.

**Impact:** Cortex misses semantically similar conflicting decisions and falls back to weaker behavior than intended.

### 2. HTTP mutation routes are not consistently authenticated

`/store` and `/shutdown` validate auth. `/diary`, `/forget`, and `/resolve` do not.

That directly contradicts the project docs and means local callers can mutate session handoff state, decay memory, and resolve disputes without the bearer token.

**Impact:** trust boundary is inconsistent, and local memory poisoning is easier than intended.

### 3. CLI and daemon response contracts are mismatched

The daemon returns `/health` as `{ status: 'ok', stats }`, while the CLI expects top-level fields like `res.memories`, `res.decisions`, and `res.ollama`.

Similarly, `/store` returns `{ stored, entry }`, while the CLI expects fields like `reason`, `status`, `conflictWith`, and `surprise` at top level.

**Impact:** health/status output is wrong right now, and store behavior is not reliably surfaced to the user.

### 4. Diary key decisions are not wired through correctly

The diary writer preserves or writes `keyDecisions`, but the daemon and MCP route pass `decisions`.

**Impact:** the API surface suggests "decisions" are being written, but the `## Key Decisions` section is not correctly updated through those routes.

### 5. Detached daemon behavior appears unreliable in practice

During this review, repeated CLI invocations re-started the daemon, `status` returned unknown stats, and `recall` after auto-start hit a connection reset.

The logs show repeated startup events. That means the "always-on daemon" invariant is not trustworthy yet.

**Impact:** if Cortex is not reliably resident, then every downstream optimization becomes less meaningful.

### 6. The project has effectively no tests

`node --test test/*.test.js` reports zero suites and zero tests.

**Impact:** core memory behavior can regress silently across store/recall/conflict/diary/lifecycle changes.

---

## My Architecture Recommendation

## Phase 0: Make Cortex trustworthy

Before new intelligence features:

1. Fix route auth consistency.
2. Fix daemon/CLI response contracts.
3. Fix embedding conflict detection.
4. Fix daemon lifecycle and detached startup reliability.
5. Add real tests for:
   - store
   - recall
   - conflict resolution
   - diary writes
   - health/status
   - CLI daemon startup

If Cortex cannot be trusted as a stable substrate, every "JARVIS" feature becomes theater.

---

## Phase 1: Replace static boot with capsules

Instead of one boot prompt assembled from fixed sections, Cortex should compile **capsules**.

Proposed capsules:

- `identity`
- `global_rules`
- `project_summary`
- `recent_delta`
- `workspace_map`
- `active_file_history`
- `task_intent`
- `open_conflicts`
- `sharp_edges`

Each capsule should have:

- `scope`
- `freshness`
- `cost_estimate`
- `priority`
- `confidence`
- `source lineage`

Then boot becomes:

- select minimum required capsules
- assemble within a strict budget
- omit anything not justified by task/workspace/user intent

This is the single biggest improvement to relevance per token.

---

## Phase 2: Add a delta-first state model

The current model stores many durable-ish rows, but it does not distinguish clearly between:

- canonical knowledge
- recent observations
- session handoff
- ephemeral noise

I would formalize four classes:

### A. Canonical

Long-lived rules and settled architectural decisions.

### B. Episodic

Recent task-local facts with expiry pressure.

### C. Delta

"What changed since the last session / last boot / last recall."

### D. Inbox

Untrusted ambient captures and unreviewed candidate memories.

This matters because the token-saving goal depends on **sending tiny deltas, not replaying the world.**

---

## Phase 3: Add scoped retrieval, not just global recall

The next retrieval layer should not be a single global `recall(query)`.

It should support:

- recall by file path
- recall by project
- recall by agent
- recall by recency window
- recall by memory type
- recall by confidence threshold
- recall by disputed / canonical / archived state

That enables "only load what matters now" instead of "search the entire brain every time."

---

## Phase 4: Add compaction and summarization

The right way to reduce tokens over time is not just retrieval. It is **compaction**.

Add a background job that:

- finds overlapping decisions
- creates canonical summaries
- links source entries
- decays or archives low-value duplicates
- marks stale workarounds for review

Important rule:

**Summarize first, archive second, delete last.**

---

## Phase 5: Add provider cache adapters

Only after phases 0-4 are stable:

- Anthropic prompt cache adapter
- Gemini cache adapter
- OpenAI prompt/cache adapter if useful

But expose them as optional outputs of compiled capsules:

- raw text
- cacheable block
- provider-specific cache handle

That keeps Cortex portable and avoids over-coupling the brain to one vendor's mechanics.

---

## Specific Ideas To Make Cortex Better

## 1. Introduce a "boot manifest"

Instead of only returning `bootPrompt`, return:

- selected capsules
- omitted capsules
- token estimate per capsule
- freshness per capsule
- rationale for inclusion

That gives observability into why Cortex chose what it chose.

## 2. Track retrieval quality metrics

You need explicit instrumentation:

- boot tokens used
- recall hit rate
- duplicate rejection rate
- conflict detection rate
- unresolved dispute count
- stale entry count
- cache hit rate
- time-to-first-useful-context

Without metrics, "better memory" remains anecdotal.

## 3. Add source scopes to every stored entry

At minimum:

- `workspace`
- `project`
- `file_path`
- `topic`
- `session_id`
- `agent_id`

This unlocks much better compaction and retrieval without a full graph system.

## 4. Add confidence classes

Not just `confidence` as a float. Add semantic classes:

- inferred
- observed
- user-confirmed
- synthesized
- disputed

This would help Cortex decide what is safe to auto-include in boot context.

## 5. Add a "candidate memory inbox"

Ambient collection should write here first.

A later compactor can promote candidates into:

- canonical memory
- episodic memory
- discard

## 6. Build a file/workspace change watcher

Do not wait for next daemon restart to re-index.

Watch:

- `state.md`
- workspace memory files
- key project docs
- debate documents

Then only re-embed changed items.

## 7. Add a session-delta compiler

One of the highest ROI features would be:

`What changed since the last time this agent worked on this project?`

That is far more valuable than replaying the full boot prompt on every session.

## 8. Move from phrase-LIKE search to proper recall fallback

If semantic search is unavailable, keyword fallback should still be strong:

- tokenized OR matching
- weighted fields
- exact phrase bonus
- recency bonus
- score bonus

Right now the fallback is too brittle for a brain that is supposed to stay useful offline.

## 9. Add a canonical "workspace map" artifact

For code projects, Cortex should maintain a compact map:

- top-level modules
- critical files
- entry points
- known hotspots
- current workstream

That single artifact would save large amounts of repeated orientation context.

## 10. Treat debates as first-class structured objects

Gemini is right here.

Move long-form debates out of flat markdown only. Keep markdown exports if useful, but store:

- topic
- point
- stance
- rebuttal links
- consensus status
- surviving open questions

Then boot can include only:

- current consensus
- unresolved disagreements

That is exactly the kind of compaction Cortex should be doing.

---

## Recommended Priority Order

### Immediate

1. Fix auth gaps.
2. Fix response contract mismatches.
3. Fix embedding conflict detection.
4. Fix daemon lifecycle persistence.
5. Add actual tests.

### Next

6. Replace static profile compilation with capsule selection.
7. Add scoped retrieval and delta summaries.
8. Add incremental re-indexing and re-embedding.

### After that

9. Add compaction/background summarization.
10. Add structured debate storage.
11. Add provider cache adapters.

### Later

12. Add graph-style relationships if structured metadata no longer suffices.
13. Add ambient memory inbox and promotion workflow.
14. Add autonomous maintenance jobs with strong guardrails.

---

## Bottom Line

The premise is correct:

**all durable understanding should be loaded once, compacted aggressively, and reused so only genuinely new work burns meaningful tokens.**

Cortex can absolutely become that system.

But the path is:

**reliability -> relevance -> compaction -> caching -> autonomy**

not

**caching -> autonomy -> hope the substrate catches up.**

Gemini's proposal is a strong vision document. My view is that Cortex should pursue that vision, but with much stricter sequencing and much more emphasis on contract correctness, staleness control, and capsule-based context assembly before any provider-specific "zero-token" machinery.
