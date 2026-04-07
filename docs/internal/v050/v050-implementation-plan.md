# Cortex v0.5.0 -- Implementation Plan & Tracker

**Primary goal:** Make the brain get smarter, faster, and cheaper the more you use it.
**Secondary goal:** Fix recall precision through hybrid retrieval and data hygiene.
**Tertiary goal:** Foundation hardening (schema, dedup, test coverage).
**Started:** 2026-04-06
**Competitive analysis:** `competitive-learnings-v050.md`
**Research basis:** `research-memory-intelligence.md` (25 papers, 2023-2026)

---

## Architecture: Current vs Target

### Current Recall Pipeline (v0.4.1)
```
Query → FTS5 keyword search (memories + decisions)
      → Brute-force embedding scan (cosine sim > SIM_FLOOR)
      → Crystal search (pre-compiled summaries)
      → Merge by source into HashMap
      → Sort by relevance
      → Return top-K
```
**Problems:**
- FTS5 and embedding results are merged but not fused -- no rank combination
- Brute-force vector scan (O(n) over ALL embeddings) -- doesn't scale
- No reranking -- first-pass scores are final scores
- No dedup -- storing the same insight twice pollutes results
- Keyword and semantic results compete instead of complementing each other

### Target Recall Pipeline (v0.5.0) -- REVISED after competitive analysis
```
Query → Tier 0: Exact hash cache (0ms)
      → Tier 1: Fuzzy cache, Jaccard >= 0.6 on keywords (50ms)
      → Tier 2: FTS5/BM25 with field boosting + synonym expansion
         → If score >= 0.93 with gap >= 0.08: return directly (100ms, NO embeddings)
      → Tier 3: [Parallel]
         → FTS5/BM25 keyword search → rank list A
         → Embedding cosine search → rank list B
      → Reciprocal Rank Fusion (k=60) → fused rank list
      → Compound scoring: BM25 * 0.6 + importance * 0.2 + recency_decay * 0.2
      → Budget-aware truncation
      → Return top-K
```

**Key insight from competitive analysis:** ByteRover achieves 96.1% LoCoMo WITHOUT embeddings -- just BM25 + hierarchy + explicit relations. Most queries should resolve at Tier 2 (BM25-only). Embeddings are the fallback, not the primary path.

---

## Pre-Implementation: Every Agent Must Do This First

1. **cortex_recall** the queries listed in their phase section below
2. **Read these files:**
   - `daemon-rs/src/handlers/recall.rs` (1884 lines) -- current recall pipeline
   - `daemon-rs/src/embeddings.rs` -- ONNX embedding engine
   - `daemon-rs/src/db.rs` -- schema, FTS5 setup
   - `docs/internal/competitive-learnings-v050.md` -- ByteRover/agentmemory analysis
3. **Use rtk prefix for all shell commands**
4. **Run `rtk cargo test` after every change** -- 65 tests must stay green
5. **Commit format:** `0.5.0 - type: description`

---

## Phase 0: Baseline Benchmark (BEFORE ANY CHANGES)

**Owner:** CX (Codex CLI)
**Why CX:** Mechanical benchmark run, no architectural decisions. Async batch is perfect.

| # | Task | Est. | Status | Details |
|---|------|------|--------|---------|
| 0.1 | Run full recall benchmark on v0.4.1 | 1hr | **DONE** | Commit 6bdf63e. GT precision 0.552, MRR 0.692, hit rate 0.900, avg latency 97.5ms. 562 embeddings, 271 mem + 273 dec. |
| 0.2 | Snapshot baseline results | 30m | **DONE** | baseline-v041.md + baseline-v041-benchmark.json + baseline-v041-metrics.json committed |

**Acceptance:** ~~Baseline numbers committed to repo. No code changes in this phase.~~ **DONE** -- commit 6bdf63e. Codex also fixed benchmark scripts (X-Cortex-Request header, health check, error diagnostics).

**Known issues from baseline:**
- ~~`has_embeddings=false`~~ **FIXED** -- benchmark was testing non-existent `/embed` endpoint. Now loads from DB directly. With embeddings: 60% GT precision.
- 15.2% of memories are near-duplicates (36 pairs at cosine > 0.90, including exact 1.000 copies). **Pre-Phase-1 cleanup planned (task 0A).**
- Hit rate dropped to 0.900 (was 1.000 on Apr 5) -- some queries returning zero results with more nodes.

---

## Pre-Phase-1: Corpus Cleanup + Benchmark Fix

### 0A: One-Time Duplicate Purge
**Owner:** CX (Codex CLI)
**Why CX:** Mechanical DB operation, well-defined thresholds, no architectural judgment needed. Script runs once, commits results.
**Why now:** Audit found 36 near-duplicate memories (15.2% noise). Includes exact 1.000 cosine copies (identical text stored 2-3x) and high-similarity pairs (>0.90). This noise dilutes recall precision and inflates boot token usage. Cleaning before Phase 1 ensures the retrieval improvements are measured against a clean corpus, not a noisy one.

**Approach:**
1. Query all memory embeddings from DB
2. Pairwise cosine similarity scan (O(n^2) but n=237, so ~28K pairs -- trivial)
3. For sim > 0.92: keep the one with higher score (more retrievals = more validated). Delete the other. Log merges to events table.
4. For sim 0.90-0.92: check Jaccard on text tokens. If Jaccard > 0.7: merge (same logic). Else: keep both.
5. Re-run benchmark after purge, compare against Phase 0 baseline.

**Research needed:** Check if rusqlite batch DELETE is safe within a transaction, or if we need to delete one-by-one. Also verify FTS index auto-updates on DELETE (it should via triggers in db.rs, but confirm).

- [ ] **0A.1** Write Python cleanup script (`benchmark/cleanup-duplicates.py`) -- **CX** -- Connects to cortex.db directly. Dry-run mode by default (`--apply` flag to execute). Prints merge plan before executing.
- [ ] **0A.2** Run dry-run, review merge plan -- **CC** reviews output
- [ ] **0A.3** Execute purge with `--apply`, verify node count dropped -- **CX**
- [ ] **0A.4** Re-run both benchmarks, compare precision/compression vs Phase 0 baseline -- **CX**

**Acceptance:** Duplicate count drops to 0 (sim > 0.92). Precision improves or holds steady. Compression recovers toward 97%+.

### 0C: Boot Savings Baseline Bug
**Owner:** K2 (Kimi K2.5)
**Why K2:** Straightforward Rust fix, single file, well-defined behavior. Clean code, cheap.
**Why now:** Boot savings shows 0% for non-Claude-Code agents (Droid, Codex, Gemini). Root cause: `estimate_raw_baseline()` in compiler.rs:654 uses `env::current_dir()` to find memory files, but the daemon's CWD doesn't match the agent's project directory. Droid gets slug `C--Users-aditya-cortex-daemon-rs` instead of `C--Users-aditya`, finds no memory files, baseline = 0.

**Fix:** Replace file-system scanning with a DB-based baseline. The raw baseline is simply the sum of all active memory and decision text lengths, which is already in SQLite:
```sql
SELECT COALESCE(SUM(LENGTH(text)), 0) FROM memories WHERE status = 'active'
UNION ALL
SELECT COALESCE(SUM(LENGTH(decision)), 0) FROM decisions WHERE status = 'active'
```
Convert char count to tokens via `estimate_tokens()`. This works for every agent regardless of CWD.

- [ ] **0C.1** Replace `estimate_raw_baseline()` in compiler.rs with DB-based baseline query -- **K2**
- [ ] **0C.2** Verify boot savings shows correct % for Claude Code, Droid, and MCP agents -- **K2**
- [ ] **0C.3** Remove `claude_project_slug()` dependency from baseline estimation (keep it for other uses if needed) -- **K2**

**Acceptance:** Boot savings in Control Center shows >90% for all agents, not just Claude Code. Droid entry shows real baseline, not 0.

### 0B: Benchmark Embedding Path Fix

**Owner:** CC
**Why:** Phase 0 baseline ran with `has_embeddings=false` because the script tested a non-existent `/embed` endpoint. **FIXED** -- benchmark now loads embeddings directly from DB. Re-run with embeddings shows 60% GT precision (up from 55% without).

- [x] **0B.1** ~~Fix benchmark script to exercise embedding path~~ -- **CC** -- **DONE** -- removed `/embed` endpoint check, loads from DB directly
- [ ] **0B.2** Re-run baseline with clean corpus (after 0A purge), freeze as true baseline -- **CX**

---

## Phase 1: Tiered Retrieval + RRF Fusion

**Owner:** CC (Claude Code / Opus)
**Why CC:** Core recall pipeline is 1884 lines of Rust with complex scoring logic. Needs deep reasoning + architectural awareness.

**cortex_recall before starting:**
```
cortex_recall("recall pipeline RRF fusion compound scoring tiered retrieval")
cortex_recall("ByteRover agentmemory competitive analysis retrieval")
```

**Papers to read:**
- [RRF (Cormack et al.)](https://dl.acm.org/doi/10.1145/1571941.1572114) -- `1/(k+rank)` fusion
- [ByteRover (arxiv.org/abs/2604.01599)](https://arxiv.org/abs/2604.01599) -- 5-tier retrieval, AKL scoring, field boosting
- [Rethinking Hybrid Retrieval (arxiv.org/abs/2506.00049)](https://arxiv.org/abs/2506.00049) -- MiniLM + reranking beats bigger models

### Tasks

### Tasks

- [ ] **1.1** Query result cache (Tier 0/1) -- 3hr -- **K2** -- LRU cache by query hash (Tier 0) + Jaccard fuzzy match on keywords (Tier 1, >= 0.6). 5-min TTL, max 100 entries. Extend existing `get_pre_cached`/`predict_and_cache` at recall.rs:~1800
- [ ] **1.2** FTS5 field boosting + synonym expansion -- 3hr -- **K2** -- Use `bm25(memories_fts, 5.0, 1.0, 3.0)` for source 5x, tags 3x, text 1x weighting. Add coding synonym map. Modify FTS query construction at recall.rs:782-815
- [ ] **1.3** RRF fusion function -- 1hr -- **K2** -- `fn rrf_fuse(lists, k) -> Vec`. Score = sum(1/(k+rank+1)). k=60 per Cormack et al. ~20 lines of Rust
- [ ] **1.4** Compound scoring -- 2hr -- **D5** -- `compound = rrf * 0.6 + importance_norm * 0.2 + recency * 0.2`. Recency: `exp(-days/30)` (21-day half-life)
- [ ] **1.5** Pipeline integration (wire Tiers 0-3) -- 4hr -- **CC** -- Refactor `run_recall_with_engine()` at recall.rs:420-520. CC reviews K2/D5 subtasks before wiring

**Acceptance:** Hybrid recall scores higher precision@5 than keyword-only or semantic-only. Tier 2 resolves 40%+ of queries without touching embeddings.

---

## Phase 2: Quality-Gated Stores + Smart Dedup

**Owner:** K2 (Kimi K2.5)
**Why K2:** Clean Rust, auto-generates unit tests, 10x cheaper than Opus. Dedup logic is well-defined -- not architectural.
**Review:** CC reviews K2's PR before merge (polarity pair per Council pattern)

**cortex_recall before starting:**
```
cortex_recall("semantic dedup store merge duplicate memory")
cortex_recall("Memori semantic triples agentmemory Jaccard")
```

**Papers to read:**
- [Memori (arxiv.org/abs/2603.19935)](https://arxiv.org/abs/2603.19935) -- semantic triples, dedup
- agentmemory README -- Jaccard > 0.7 for supersession, > 0.9 for contradiction

### Prompt for K2:
```
You are implementing semantic dedup and quality scoring for Cortex v0.5.0.

Read these files first:
- daemon-rs/src/handlers/store.rs -- current store handler
- daemon-rs/src/embeddings.rs -- cosine_similarity function
- daemon-rs/src/db.rs -- schema
- docs/internal/competitive-learnings-v050.md -- competitive analysis

Task 2.1: Semantic dedup on store
Before inserting, embed the new text and find top-3 existing by cosine sim.
- sim > 0.92: merge (bump score +5, update timestamp, append context, increment merged_count)
- sim 0.90-0.92: check Jaccard on text tokens. If Jaccard > 0.7: merge. Else: insert as new.
- sim < 0.90: insert as new
- Log merges to events table (event_type: "merge", source + target IDs)

Task 2.2: Quality scoring on store
Score 0-100:
- Length: < 10 chars = 0, 10-50 = 30, 50-200 = 70, 200+ = 100
- Specificity bonus: contains code/file paths/function names = +20
- Question penalty: ends with ? = -30
- Reject stores with quality < 20 (return 400 "Memory too vague")

Task 2.3: Schema migration
- memories: add merged_count INTEGER DEFAULT 0, quality INTEGER DEFAULT 50
- decisions: add merged_count INTEGER DEFAULT 0, quality INTEGER DEFAULT 50

Write unit tests for all. Use rtk prefix for all shell commands.
Commit format: "0.5.0 - type: description"
```

### Tasks

### Tasks

- [ ] **2.1** Semantic dedup on store -- 4hr -- **K2** -- Cosine > 0.92 merge, 0.90-0.92 Jaccard fallback, log to events
- [ ] **2.2** Quality scoring -- 2hr -- **K2** -- Score 0-100, reject < 20, store in quality column
- [ ] **2.3** Schema migration -- 1hr -- **K2** -- merged_count + quality columns on memories + decisions
- [ ] **2.4** Unit tests for dedup + quality -- 2hr -- **K2** -- Threshold boundary, merge vs insert, quality edge cases

**Acceptance:** Storing "use early returns in Go code" then "always use early returns" produces 1 merged memory. Storing "?" returns 400.

### Post Phase 1+2: Benchmark
- [ ] **2.5** CX runs recall benchmark, diffs against Phase 0 baseline (commit 6bdf63e)
- [ ] **2.6** If precision < 70%, CC samples 20 low-precision recalls to classify failures (GIGO/RANKING/SPARSE)
- [ ] **2.7** Decision: pull R1/R9 from deferred for v0.5.1, or not

---

## Phase 3: Foundation Hardening

### 3A: Schema Versioning
**Owner:** CX (Codex)
**Why CX:** Mechanical, well-defined, perfect for async batch.

**Prompt for CX:**
```
Add schema versioning to Cortex daemon. Read daemon-rs/src/db.rs.

1. schema_migrations table: id, version, name, applied_at
2. Named migration functions that run SQL
3. On startup, check applied migrations, run pending in order
4. `cortex doctor` CLI command: verify all tables exist, schema current
5. Include Phase 2's merged_count + quality columns as a migration

Use rtk prefix. Commit format: "0.5.0 - type: description"
```

- [ ] **3A.1** schema_migrations table -- 2hr -- **CX**
- [ ] **3A.2** cortex doctor command -- 2hr -- **CX**
- [ ] **3A.3** Migration runner on startup -- 3hr -- **CX**

### 3B: TTL / Hard Expiration
**Owner:** CX (Codex)

- [ ] **3B.1** expires_at column -- 1hr -- **CX**
- [ ] **3B.2** ttl_seconds param in store API -- 1hr -- **CX**
- [ ] **3B.3** Recall filters expired rows -- 1hr -- **CX**
- [ ] **3B.4** Background cleanup every 6h -- 1hr -- **CX**

### 3C: Test Suite
**Owner:** K2 (unit tests), CC (integration test)

- [ ] **3C.1** Unit tests: RRF, compound scoring, synonyms -- 3hr -- **K2**
- [ ] **3C.2** Integration test: store -> recall -> verify ranking -- 3hr -- **CC**
- [ ] **3C.3** Clippy CI gate -- 30m -- **CX**
- [ ] **3C.4** Recall benchmark as regression test -- 2hr -- **CC**

### 3D: Boot Audit Trail
**Owner:** D4 (GLM-4.7 via Droid)
**Why D4:** Straightforward DB + handler work. Interleaved thinking suits tool-augmented tasks.

**Prompt for D4:**
```
Add boot prompt audit trail to Cortex. Read daemon-rs/src/compiler.rs.

1. boot_log table: id, session_id, agent, sources_json, scores_json, total_tokens, created_at
2. After compiling boot prompt, INSERT record
3. GET /boot-history?agent=X&limit=10 endpoint
4. Update cortex_digest: avg boot token count, top 5 most-sourced memories

Use rtk prefix. Commit format: "0.5.0 - type: description"
```

- [ ] **3D.1** boot_log table + insert -- 2hr -- **D4**
- [ ] **3D.2** /boot-history endpoint -- 1hr -- **D4**
- [ ] **3D.3** cortex_digest boot stats -- 1hr -- **D4**

---

## Phase 4: Memory Intelligence

**Research basis:** `research-memory-intelligence.md` -- 25 papers surveyed. These techniques are what make the brain compound over time rather than just accumulate.

### Backward Compatibility Constraint (MANDATORY)

All Phase 4 changes must be **additive overlays** on existing data. Users upgrading from v0.4.x must experience zero data loss and zero behavioral regression.

Rules for every Phase 4 task:
1. **New columns use DEFAULT values.** Existing rows get sensible defaults (e.g., `maturity DEFAULT 'raw'`, `content_type DEFAULT 'observation'`, `confidence DEFAULT 0.5`). No NULLs that break queries.
2. **New scoring layers are additive.** Admission control, maturity decay, and content-type decay all multiply against the existing score. A memory with no new metadata behaves exactly as it did before (all modifiers default to 1.0).
3. **Migration runs on startup via schema versioning (Phase 3A).** Existing databases get new columns silently. No manual steps.
4. **Backfill is a separate step, not a migration blocker.** The daemon works without backfilled data; backfill improves quality when it runs but isn't required for correctness.
5. **No existing memories are deleted or modified by schema changes.** Only the duplicate purge (Phase 0A) deletes, and it's a one-time manual operation with dry-run review.

### 4A: Adaptive Admission Control (A-MAC paper)
**Owner:** CC (Claude Code / Opus)
**Why CC:** Requires judgment on factor design and weight tuning for coding-specific memory. Architectural decision about the store pipeline.
**Paper:** A-MAC (arXiv 2603.04549) -- five-factor admission scoring, 31% latency reduction, F1 improvement over accept-everything baseline.

Replace the current "accept everything" store path with a five-factor admission gate. Each store gets scored on:
1. **Semantic novelty** -- cosine distance to nearest existing memory. Already partially implemented via conflict detection; extend to gate admission.
2. **Content type prior** -- classify as decision/convention/debug/preference/observation. A-MAC found this is the single highest-impact factor.
3. **Specificity** -- does it contain concrete identifiers (file paths, function names, error codes)? Vague memories get penalized.
4. **Factual confidence** -- is the agent confident or hedging? ("I think" vs "confirmed that")
5. **Future utility** -- is this likely to be recalled? Short temporal scope items (today's debugging) score lower than architectural decisions.

Score 0-100. Reject below threshold (configurable, default 30). Log rejections to events table for tuning.

- [ ] **4A.1** Content type classifier -- 3hr -- **CC** -- Regex/keyword classifier for decision, convention, debug, preference, observation, meta. Add `content_type TEXT` column to memories and decisions.
- [ ] **4A.2** Five-factor scoring function -- 4hr -- **CC** -- `fn admission_score(text, embedding, conn) -> (u8, AdmissionFactors)`. Returns composite score + per-factor breakdown.
- [ ] **4A.3** Gate integration in store handler -- 2hr -- **K2** -- Wire scoring into store.rs. Reject < threshold with 400 + factor breakdown in response. Log to events.
- [ ] **4A.4** Tune thresholds on real data -- 2hr -- **CC** -- Run admission scoring on existing 242 memories retroactively. Identify what would have been rejected. Adjust threshold to reject obvious noise without losing signal.

**Acceptance:** Storing "it worked" returns 400. Storing "Always use rtk prefix for shell commands in Cortex repo" scores 70+ and persists.

### 4B: Memory Maturity Tiers (ByteRover AKL + MemoryOS)
**Owner:** K2 (Kimi K2.5)
**Why K2:** Schema change + scoring logic, well-defined tiers. Clean Rust work.
**Papers:** ByteRover (arXiv 2604.01599), MemoryOS (arXiv 2506.06326) -- tiered lifecycle with per-tier decay.

Memories progress through four maturity stages with different decay curves:

| Tier | Decay Half-Life | Promotion Criteria | Description |
|------|----------------|-------------------|-------------|
| **Raw** | 7 days | Recalled 2+ times | Just stored, unvalidated |
| **Validated** | 30 days | Recalled 5+ times OR corroborated by another agent | Proven useful at least once |
| **Stable** | 180 days | Survived 3+ crystallization passes | Reliably valuable |
| **Crystallized** | Permanent (no decay) | Incorporated into a crystal | Compressed into higher-order knowledge |

- [ ] **4B.1** Schema: add `maturity TEXT DEFAULT 'raw'` to memories and decisions -- 1hr -- **K2**
- [ ] **4B.2** Promotion logic in recall feedback path -- 2hr -- **K2** -- After bump_retrievals, check if memory qualifies for promotion. Update maturity tier.
- [ ] **4B.3** Per-tier decay curves in aging.rs -- 2hr -- **K2** -- Replace single decay formula with tier-aware decay. Raw decays at `exp(-days/7)`, Validated at `exp(-days/30)`, Stable at `exp(-days/180)`, Crystallized at 1.0 (no decay).
- [ ] **4B.4** Update boot compiler to prefer higher-maturity memories -- 1hr -- **CC** -- When budget is tight, Crystallized > Stable > Validated > Raw.

**Acceptance:** A new memory starts as Raw. After 2 recalls it becomes Validated. Decay rate visibly slows.

### 4C: BMM Crystallization Gate (FluxMem paper)
**Owner:** CC (Claude Code / Opus)
**Why CC:** Statistical modeling, needs to understand the embedding distribution. Architectural change to crystallize.rs.
**Paper:** FluxMem (arXiv 2602.14038) -- Beta Mixture Model replaces brittle cosine thresholds.

Replace the fixed cosine threshold in crystallize.rs with a probabilistic Beta Mixture Model that learns the boundary between "should cluster" and "shouldn't cluster" from the actual embedding similarity distribution.

- [ ] **4C.1** Compute pairwise similarity histogram for all active memories -- 2hr -- **CC** -- Store as a cached distribution in events or a new table. Recompute on crystallization pass.
- [ ] **4C.2** Fit two-component Beta Mixture Model -- 3hr -- **CC** -- One component for "same-topic" pairs (high sim), one for "different-topic" pairs (low sim). Use EM algorithm. Pure Rust implementation, no external deps.
- [ ] **4C.3** Replace fixed threshold with BMM posterior -- 2hr -- **CC** -- Cluster if P(same-topic | sim) > 0.7. This adapts automatically as the embedding space evolves.
- [ ] **4C.4** Benchmark crystal quality before/after -- 1hr -- **CX** -- Count crystals, avg crystal size, crystal coverage (% of memories in a crystal).

**Acceptance:** Crystallization produces meaningful clusters without manual threshold tuning. Cluster quality improves as more memories are added (the model gets more data to fit).

### 4D: Provenance Tracking (Collaborative Memory paper)
**Owner:** D5 (GLM-5 via Droid)
**Why D5:** Schema additions and straightforward handler changes. Good agentic task.
**Paper:** Collaborative Memory (arXiv 2505.18279) -- immutable provenance on every memory fragment.

Every memory and decision should track where it came from and how it got there.

- [ ] **4D.1** Schema: add `source_agent TEXT`, `confidence REAL DEFAULT 0.5`, `session_id TEXT` to memories and decisions -- 1hr -- **D5**
- [ ] **4D.2** Populate source_agent from X-Source-Agent header on store -- 1hr -- **D5** -- Already partially exists; make it consistent.
- [ ] **4D.3** Crystal lineage: when crystallizing, store `source_memory_ids JSON` on the crystal -- 1hr -- **D5**
- [ ] **4D.4** Recall filter by agent: `?agent_filter=claude-code` returns only memories from that agent -- 2hr -- **D5**

**Acceptance:** Every new memory shows which agent stored it. Crystals link back to their source memories.

### 4E: Content-Type-Aware Decay (novel, no paper)
**Owner:** K2 (Kimi K2.5)
**Why K2:** Builds directly on 4B maturity tiers. Clean extension of aging.rs.
**Research gap:** No paper addresses content-type-specific decay. Cortex pioneers this.

Different memory types have fundamentally different lifespans:

| Content Type | Decay Modifier | Rationale |
|-------------|---------------|-----------|
| convention | 0.1x (very slow) | "Use rtk prefix" stays valid until explicitly superseded |
| decision | 0.3x (slow) | "We chose SQLite over Postgres" is long-lived |
| preference | 0.5x (moderate) | User preferences change but not fast |
| debug | 2.0x (fast) | Workarounds expire when the bug is fixed |
| observation | 1.5x (faster) | Situational context loses relevance quickly |
| meta | 3.0x (fastest) | Session logistics, tool status, transient state |

- [ ] **4E.1** Extend aging.rs decay formula: multiply base decay by content_type modifier -- 2hr -- **K2** -- Depends on 4A.1 (content_type column exists).
- [ ] **4E.2** Backfill content_type on existing memories via classifier -- 1hr -- **CX** -- Run the 4A.1 classifier on all active memories, UPDATE the column.

**Acceptance:** A convention memory ("always use uv for Python") decays 10x slower than a debug memory ("Defender flagged RTK as Bearfoos").

---

## NOT in v0.5.0 (Deferred)

Council verdict (2026-04-06): defer complexity until boring fixes prove insufficient.

| Feature | Why Deferred | Reinstate Trigger |
|---------|-------------|-------------------|
| **R1: Cross-Encoder reranking** | Council: fix data + fusion first. Adds 80MB model, latency, failure modes. Premature at 504 nodes. | Phase 2.5 diagnostic shows ranking-type failures AND precision < 70% post-fusion |
| **R9: Recall feedback loop** | Council: amplifies bias on broken ranker. "Self-licking ice cream cone" (Torvalds). Trains broken router. | Validated precision baseline exists, offline replay infrastructure built, kill switch in place |
| R7: HyDE (query expansion) | Requires LLM per query, assumes embeddings are trustworthy | Embedding model upgrade (R8) shows alignment improvement |
| R3: Multi-graph views (MAGMA) | Architecture overhaul, PhD-level complexity, who debugs the causal view? | v0.6.0+ if simple pipeline plateaus |
| R4: Zettelkasten auto-linking (A-MEM) | Architectural complexity before retrieval foundation solid | v0.6.0+ |
| R8: Embedding model upgrade eval | Research task, not blocking. Run in sandbox. | Can run anytime as parallel research via Codex batch |
| RL recall feedback (MemFactory) | Training infrastructure | v0.6.0+ |
| Desktop app UI polish | Recall > visuals | v0.5.1 |
| Hypergraph brain visualizer | Needs design spec | v0.5.1+ |
| Mobile app | Needs Mac for iOS | Separate project |

---

## Execution Schedule

```
Pre:     CX: Phase 0 (baseline benchmark) -- DONE (commit 6bdf63e)
Pre:     CX: Phase 0A (duplicate purge) | CC: review merge plan
         K2: Phase 0C (boot savings baseline bug)
Week 1:  K2: Phase 1 tasks 1.1-1.3 | D5: Phase 1 task 1.4 | CX: Phase 3A+3B  [ALL PARALLEL]
         CC: Phase 1 task 1.5 (integration, after K2/D5 land) + review
Week 2:  K2: Phase 2 (dedup + quality) | D4: Phase 3D (boot audit)  [ALL PARALLEL]
         CX: post-Phase-1+2 benchmark | K2: Phase 3C unit tests
Week 3:  CC: Phase 4A (admission control) | K2: Phase 4B (maturity tiers)  [ALL PARALLEL]
         D5: Phase 4D (provenance tracking)
Week 4:  CC: Phase 4C (BMM crystallization gate)
         K2: Phase 4E (content-type decay) | CX: Phase 4E.2 (backfill)
Week 5:  CC: integration test + final benchmark + release prep
```

**Agent load distribution:**
| Agent | Tasks | Est. Hours |
|-------|-------|------------|
| CC | 1.5, 3C.2, 3C.4, 4A.1-4A.2, 4A.4, 4C.1-4C.3, review, release | ~28hr |
| K2 | 0C, 1.1-1.3, 2.1-2.4, 3C.1, 4A.3, 4B.1-4B.3, 4E.1 | ~26hr |
| CX | 0A, 3A, 3B, 3C.3, 4C.4, 4E.2, benchmarks | ~16hr |
| D5 | 1.4, 4D.1-4D.4 | ~7hr |
| D4 | 3D | ~4hr |

**Total: ~81 hours across 5 agents**

---

## Research References

| Paper | Key Insight | Phase | Who Reads |
|-------|-------------|-------|-----------|
| [ByteRover (2604.01599)](https://arxiv.org/abs/2604.01599) | 96.1% LoCoMo WITHOUT embeddings. 5-tier retrieval, AKL scoring, field boosting | P1 | CC |
| [RRF (Cormack et al.)](https://dl.acm.org/doi/10.1145/1571941.1572114) | `1/(k+rank)` fusion beats any single retriever | P1 | CC |
| [Rethinking Hybrid Retrieval (2506.00049)](https://arxiv.org/abs/2506.00049) | MiniLM + reranking beats bigger models | P1, Deferred (R1) | CC |
| [SmartSearch (2603.15599)](https://arxiv.org/abs/2603.15599) | Ranking > structure. 8.5x fewer tokens | Deferred (R1) | CC |
| [Memori (2603.19935)](https://arxiv.org/abs/2603.19935) | Semantic triples + dedup = 81.95% accuracy at 5% context | P2 | K2 |
| [MemFactory (2603.29493)](https://arxiv.org/html/2603.29493) | RL-optimized memory ops (GRPO). 14.8% improvement | Future | CC |
| [agentmemory](https://github.com/rohitg00/agentmemory) | Triple-stream RRF k=60, Jaccard dedup, quality scoring | P1, P2 | CC, K2 |
| [DS@GT Fusion (2601.15518)](https://arxiv.org/abs/2601.15518) | BM25 + dense + Gemini reranking. 0.66 recall | P1, Deferred (R1) | CC |
| [Active Context Compression (2601.07190)](https://arxiv.org/abs/2601.07190) | Autonomous compression. 22.7% token reduction | Future | -- |
| [HyDE (2212.10496)](https://arxiv.org/abs/2212.10496) | Hypothetical doc embeddings for short queries | Future | -- |

---

## Success Metrics

| Metric | v0.4.1 (est.) | v0.5.0 Target | Competitors |
|--------|---------------|---------------|-------------|
| Recall@10 | ~45% | 60%+ | agentmemory: 64.1% |
| NDCG@10 | ~70% | 85%+ | agentmemory: 94.9% |
| MRR | ~80% | 95%+ | agentmemory: 100% |
| Tokens/query | ~500 | < 1,000 | agentmemory: 1,571 |
| Tier 2 resolution | 0% | 40%+ | ByteRover: "most queries" |
| Duplicate memories | unchecked | 0 (sim > 0.92) | -- |
| Test count | 65 | 100+ | -- |
| Clippy warnings | unchecked | 0 | -- |
| LoCoMo | N/A | future benchmark | ByteRover: 96.1% |

## Codex Phase 0A Report
- Dry-run output: Summary: Would delete 29 memories, 0 decisions. Estimated noise reduction: 5.9%
- Deletions applied: 29 memories, 0 decisions
- Orphan check: pass (broad orphan query = 0, type-aware orphan query = 0; FTS MATCH `cortex` returned 25 memory rows and 135 decision rows)
- Post-purge benchmark: GT precision 51.3%, MRR 0.74, hit rate 85.0% (below the 60% post-embedding-fix target; likely retrieval/ranking instability remains)
- Commit: this commit (hash reported by Codex final response)
