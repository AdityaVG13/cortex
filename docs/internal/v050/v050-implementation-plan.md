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

> **Phase 0, 0A: DONE** -- see `v050-tracker.md` for commits and deliverables.

> **Phase 0C: DONE** -- see `v050-tracker.md` for commits and deliverables.

### 0B: Benchmark Embedding Path Fix

**Owner:** CC
**Why:** Phase 0 baseline ran with `has_embeddings=false` because the script tested a non-existent `/embed` endpoint. **FIXED** -- benchmark now loads embeddings directly from DB. Re-run with embeddings shows 60% GT precision (up from 55% without).

- [x] **0B.1** ~~Fix benchmark script to exercise embedding path~~ -- **CC** -- **DONE** -- removed `/embed` endpoint check, loads from DB directly
- [ ] **0B.2** Re-run baseline with clean corpus (after 0A purge), freeze as true baseline -- **CX**

---

> **Phase 1: DONE** -- see `v050-tracker.md` for commits and deliverables.

---

## Phase 2: Quality-Gated Stores + Smart Dedup

**Owner:** SN (Claude Sonnet)
**Why SN:** Fast, clean Rust, well-defined dedup logic -- not architectural.
**Review:** CC reviews SN's PR before merge (polarity pair per Council pattern)

**cortex_recall before starting:**
```
cortex_recall("semantic dedup store merge duplicate memory")
cortex_recall("Memori semantic triples agentmemory Jaccard")
```

**Papers to read:**
- [Memori (arxiv.org/abs/2603.19935)](https://arxiv.org/abs/2603.19935) -- semantic triples, dedup
- agentmemory README -- Jaccard > 0.7 for supersession, > 0.9 for contradiction

### Prompt for SN:
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

- [ ] **2.1** Semantic dedup on store -- 4hr -- **SN** -- Cosine > 0.92 merge, 0.90-0.92 Jaccard fallback, log to events
- [ ] **2.2** Quality scoring -- 2hr -- **SN** -- Score 0-100, reject < 20, store in quality column
- [ ] **2.3** Schema migration -- 1hr -- **SN** -- merged_count + quality columns on memories + decisions
- [ ] **2.4** Unit tests for dedup + quality -- 2hr -- **SN** -- Threshold boundary, merge vs insert, quality edge cases

**Acceptance:** Storing "use early returns in Go code" then "always use early returns" produces 1 merged memory. Storing "?" returns 400.

### Post Phase 1+2: Benchmark
- [ ] **2.5** CX runs recall benchmark, diffs against Phase 0 baseline (commit 6bdf63e)
- [ ] **2.6** If precision < 70%, CC samples 20 low-precision recalls to classify failures (GIGO/RANKING/SPARSE)
- [ ] **2.7** Decision: pull R1/R9 from deferred for v0.5.1, or not

---

## Phase 3: Foundation Hardening

> **Phase 3A: DONE** -- see `v050-tracker.md` for commits and deliverables.

### 3B: TTL / Hard Expiration
**Owner:** CX (Codex)

- [ ] **3B.1** expires_at column -- 1hr -- **CX**
- [ ] **3B.2** ttl_seconds param in store API -- 1hr -- **CX**
- [ ] **3B.3** Recall filters expired rows -- 1hr -- **CX**
- [ ] **3B.4** Background cleanup every 6h -- 1hr -- **CX**

### 3C: Test Suite
**Owner:** D4 (unit tests), CC (integration test)

- [ ] **3C.1** Unit tests: RRF, compound scoring, synonyms -- 3hr -- **D4**
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
- [ ] **4A.3** Gate integration in store handler -- 2hr -- **D4** -- Wire scoring into store.rs. Reject < threshold with 400 + factor breakdown in response. Log to events.
- [ ] **4A.4** Tune thresholds on real data -- 2hr -- **CC** -- Run admission scoring on existing 242 memories retroactively. Identify what would have been rejected. Adjust threshold to reject obvious noise without losing signal.

**Acceptance:** Storing "it worked" returns 400. Storing "Always use rtk prefix for shell commands in Cortex repo" scores 70+ and persists.

### 4B: Memory Maturity Tiers (ByteRover AKL + MemoryOS)
**Owner:** SN (Claude Sonnet) + D4 (GLM-4.7)
**Why:** Schema change (D4) + promotion/decay logic (SN). Well-defined tiers.
**Papers:** ByteRover (arXiv 2604.01599), MemoryOS (arXiv 2506.06326) -- tiered lifecycle with per-tier decay.

Memories progress through four maturity stages with different decay curves:

| Tier | Decay Half-Life | Promotion Criteria | Description |
|------|----------------|-------------------|-------------|
| **Raw** | 7 days | Recalled 2+ times | Just stored, unvalidated |
| **Validated** | 30 days | Recalled 5+ times OR corroborated by another agent | Proven useful at least once |
| **Stable** | 180 days | Survived 3+ crystallization passes | Reliably valuable |
| **Crystallized** | Permanent (no decay) | Incorporated into a crystal | Compressed into higher-order knowledge |

- [ ] **4B.1** Schema: add `maturity TEXT DEFAULT 'raw'` to memories and decisions -- 1hr -- **D4**
- [ ] **4B.2** Promotion logic in recall feedback path -- 2hr -- **SN** -- After bump_retrievals, check if memory qualifies for promotion. Update maturity tier.
- [ ] **4B.3** Per-tier decay curves in aging.rs -- 2hr -- **SN** -- Replace single decay formula with tier-aware decay. Raw decays at `exp(-days/7)`, Validated at `exp(-days/30)`, Stable at `exp(-days/180)`, Crystallized at 1.0 (no decay).
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
**Owner:** D4 (GLM-4.7 via Droid)
**Why D4:** Builds directly on 4B maturity tiers. Straightforward extension of aging.rs.
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

- [ ] **4E.1** Extend aging.rs decay formula: multiply base decay by content_type modifier -- 2hr -- **D4** -- Depends on 4A.1 (content_type column exists).
- [ ] **4E.2** Backfill content_type on existing memories via classifier -- 1hr -- **CX** -- Run the 4A.1 classifier on all active memories, UPDATE the column.

**Acceptance:** A convention memory ("always use uv for Python") decays 10x slower than a debug memory ("Defender flagged RTK as Bearfoos").

---

## Phase 5: DB Resilience & Corruption Prevention

**Priority:** CRITICAL -- this affects every end user, not just us.
**Added:** 2026-04-07 after B-tree corruption incident caused by crash/freeze.
**Root cause:** Terminal freeze --> daemon killed mid-write --> B-tree pages corrupted (Tree 326: duplicated page refs + invalid page numbers). Both `~/cortex/cortex.db` and `~/.cortex/cortex.db` were corrupted.

### What already exists (v0.4.1):
- WAL checkpoint every 60s (main.rs:1132-1143)
- WAL checkpoint + PRAGMA optimize on graceful shutdown (main.rs:1229-1236)
- `verify_integrity()` in db.rs (used only in legacy migration, NOT on startup)
- `checkpoint_wal_best_effort()` in db.rs

### What's missing (the gaps that caused corruption):
1. **No integrity check on startup** -- daemon opens corrupted DB and serves bad data silently
2. **No auto-recovery** -- if DB is corrupted, user must manually fix (they won't know how)
3. **No periodic backups** -- crash = total loss of write buffer + potentially the whole DB
4. **Force-kill (crash/freeze) skips shutdown** -- WAL checkpoint never runs, dirty pages left
5. **No DB size monitoring** -- 19.2 MB bloat went undetected (should have been ~6 MB)

> **Phases 5A, 5B, 5C: DONE** -- see `v050-tracker.md` for commits and deliverables.

### 5D: DB Size Monitoring & Compaction
**Owner:** CX
**Why now:** 19.2 MB for 242 memories is 80x larger than necessary. Bloat = slower backups, slower integrity checks.

Analysis from 2026-04-07 rebuild:
- Clean DB with all data: 5.9 MB (242 mem, 277 dec, 638 embeddings, 2527 co-occurrence, 1089 events)
- Corrupted DB: 19.2 MB (same data + dead B-tree pages, orphaned pages)
- Embeddings are the largest table (~1 MB for 384-dim x 638 vectors)

- [ ] **5D.1** Add `PRAGMA freelist_count` check to health endpoint -- surface reclaimable pages -- 30m
- [ ] **5D.2** Auto-VACUUM after compaction pass (every 6h, already runs aging + compaction) -- 30m
- [ ] **5D.3** Add DB size to health endpoint response (`db_size_bytes`, `freelist_pages`) -- 30m
- [ ] **5D.4** Event rotation: reduce event retention from 30d to 14d for non-boot events -- 30m

**Acceptance:** No user ever sees a corrupted DB silently. On crash --> restart --> auto-detect --> auto-repair --> back to serving in <5s. DB stays under 10 MB for typical usage (500 nodes).

### 5E: Storage Compression & Retention Policy
**Owner:** CX (Codex)
**Priority:** ASAP -- `.cortex/` folder grows unbounded. Power users and teams will hit this first.
**Why now:** Observed 7 backup files (bridge-backups/), daily rolling backups, crash logs, write buffer, stale PID/lock files accumulating with no cleanup. For a team of 5 engineers, this compounds fast.

**Current `.cortex/` contents (2026-04-08):**
- `backups/` -- daily rolling DB snapshots (growing 1/day, ~6 MB each)
- `bridge-backups/` -- 7 files from legacy migration, including corrupted copies (dead weight)
- `daemon.log`, `daemon.err.log`, `daemon.out.log`, `mcp-crash.log`, `rust-daemon.err.log` -- 5 log files, no rotation
- `write_buffer.jsonl` -- unbounded append-only buffer
- `cortex.lock`, `cortex.pid` -- stale after crash (no cleanup)
- `models/` -- ~40 MB ONNX model + tokenizer (fixed, not compressible)
- `store-droid.json` -- legacy bridge artifact

**Tasks:**

- [ ] **5E.1** Backup retention policy -- 1hr -- **CX** -- Keep last 3 daily backups, delete older. Run on daemon startup.
- [ ] **5E.2** Delete bridge-backups/ on startup if schema version >= 5 (post-migration) -- 30m -- **CX** -- One-time cleanup of legacy migration artifacts.
- [ ] **5E.3** Log rotation -- 1hr -- **CX** -- Cap each log file at 1 MB. On exceeding, rotate to `.1` (keep 1 previous). Delete `.1` on next rotation.
- [ ] **5E.4** Write buffer compaction -- 30m -- **CX** -- Truncate write_buffer.jsonl after entries are flushed to DB. Currently append-only.
- [ ] **5E.5** Stale PID/lock cleanup on startup -- 30m -- **CX** -- If cortex.pid exists but process isn't running, delete pid + lock files.
- [ ] **5E.6** Add `cortex cleanup --dry-run` CLI command -- 1hr -- **CX** -- Shows what would be deleted/rotated. Without `--dry-run`, executes cleanup. Useful for manual intervention.
- [ ] **5E.7** Add `.cortex/` size to health endpoint (`storage_bytes`, `backup_count`, `log_bytes`) -- 30m -- **CX** -- Enables monitoring from Control Center.

**Acceptance:** `.cortex/` stays under 25 MB for typical single-user usage (500 nodes). `cortex cleanup --dry-run` shows actionable output. No manual cleanup needed -- daemon self-maintains on startup.

---

## Phase 6: Public Research & Sources Page

**Added:** 2026-04-07. Make the project truly open-source by showing our work.
**Source material:** `docs/internal/v050/research-memory-intelligence.md` (25+ papers)

- [ ] **6.1** Create `Info/research.md` -- public-facing page listing every paper, what we took from each, and how it maps to Cortex features -- CC or D5
- [ ] **6.2** Update README.md to link to the research page -- CC
- [ ] **6.3** Add "Built on Research" section to README with 3-4 highlight papers -- CC

**Format per paper:**
```
### [Paper Title] (Year)
**Link:** arxiv/doi URL
**Key insight:** One sentence about what the paper showed
**What we took:** How it influenced Cortex (specific feature/phase)
**Status:** Implemented (v0.X) | Planned (Phase N) | Deferred
```

---

## Phase 7: Desktop App Lifecycle & Developer Workflow

**Priority:** CRITICAL -- this is the make-or-break UX for adoption over competitors.
**Added:** 2026-04-07
**Invariant:** If the app is open, the daemon is ALWAYS running. Any AI connects immediately. No exceptions.

### Design Principles

1. **App open = daemon on.** The app's primary job is keeping the daemon alive. If the daemon dies, the app restarts it silently.
2. **Claude plugin users** get auto-start on Claude launch, auto-timeout after Claude closes. No app needed.
3. **Other AI users** (Cursor, Windsurf, Droid, etc.) keep the app open. The app is their daemon manager.
4. **Session persistence across restarts.** A daemon restart must not break active MCP connections.

> **Phases 7A, 7B: DONE** -- see `v050-tracker.md` for commits and deliverables.

### 7C: Daemon Restart Button
**Owner:** D4 (GLM-4.7 via Droid)
**Why D4:** Simple UI addition, calls stop then start.

- [ ] **7C.1** Add "Restart" button to daemon controls -- 1hr -- **D4** -- Calls `/shutdown`, waits for process exit, starts sidecar, polls health until ready. Shows "Restarting..." intermediate state.

**Acceptance:** Click Restart. Daemon stops, restarts, UI shows running + agents reconnect (via 7A). Total time < 5s.

### 7D: In-App Lifecycle Documentation
**Owner:** D4 (GLM-4.7 via Droid)
**Why D4:** Static content, straightforward.

Add an "About" or "Help" section in the app that explains:

| Action | What Happens |
|--------|-------------|
| **X button** | Minimize to tray. Daemon stays alive. AI connections persist. |
| **Stop** | Stop daemon. All AI connections drop. Use when you want the brain offline. |
| **Restart** | Stop + start daemon. AI connections auto-recover (sessions re-register). |
| **Exit** (tray menu) | Close app + stop sidecar. Daemon stops. |
| **Using Claude** | Daemon auto-starts via plugin. Auto-stops after Claude closes. App not needed. |
| **Using other AIs** | Keep this app open. It manages the daemon for you. |

- [ ] **7D.1** Add Help/About section with lifecycle table -- 1hr -- **D4**
- [ ] **7D.2** Add tooltip on X button: "Minimizes to tray -- daemon stays alive" -- 15m -- **D4**

**Acceptance:** New user opens app, reads Help, understands the full lifecycle without external docs.

### 7E: Developer Build Workflow
**Owner:** CC (document), anyone (execute)
**Why now:** We rebuild the Tauri app frequently during dev. Needs to be copy-paste simple.

**Quick rebuild steps (copy-paste into terminal):**
```bash
cd ~/cortex/desktop/cortex-control-center
npm install
npm run tauri build
# Installer at: src-tauri/target/release/bundle/nsis/Cortex Control Center_*.exe
```

**Dev mode (hot-reload, no install needed):**
```bash
cd ~/cortex/desktop/cortex-control-center
npm install
npm run tauri dev
# App launches with hot-reload. Daemon builds + starts as sidecar.
```

**After install:**
- Launch "Cortex Control Center" from Start menu
- Daemon starts automatically as sidecar
- Verify: health indicator shows green, Agents panel populates when an AI connects

- [ ] **7E.1** Add `DEVELOPING.md` in `desktop/cortex-control-center/` with these steps -- 30m -- **CC**
- [ ] **7E.2** Verify clean build works from fresh clone -- 30m -- **CC** -- `npm install && npm run tauri build` produces working installer

**Acceptance:** Fresh clone -> copy-paste commands -> working app with daemon. No guesswork.

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
| Desktop app UI polish (beyond Phase 7) | Phase 7 covers lifecycle UX; remaining visual polish deferred | v0.5.1 |
| Hypergraph brain visualizer | Needs design spec | v0.5.1+ |
| Mobile app | Needs Mac for iOS | Separate project |

---

## Execution Schedule

```
DONE:    CX: Phase 0 (baseline benchmark) -- commit 6bdf63e
DONE:    CX: Phase 0A (duplicate purge) -- 29 memories purged
DONE:    Phase 0C (boot savings baseline bug) -- commit a6e5d9d
DONE:    Phase 1 (tiered retrieval + RRF fusion) -- commits ed1d3a7 through ad74a92
DONE:    Phase 5A-5C (DB resilience) -- commits 3576c5c, a82d747, 980f66b
DONE:    Phase 3A (schema versioning + cortex doctor) -- commit 145766b (D4)
DONE:    Phase 7A (MCP reconnect + session telemetry) -- commit 7081dc1 (D4)
DONE:    Phase 7B (immediate UI state reflection) -- commit d5d58cd (D4)
---
ASAP:    CX: Phase 5E (storage compression) -- .cortex/ folder growing fast, critical for power users/teams
         D4: Phase 7C-7D (restart button + lifecycle docs)
         CC: Phase 7E (dev workflow doc)
Next:    SN: Phase 2 (dedup + quality) | D4: Phase 3D (boot audit) | CX: Phase 3A+3B + 5D  [ALL PARALLEL]
         CX: post-Phase-2 benchmark | D4: Phase 3C.1 unit tests
         CC/D5: Phase 6 (research & sources page)  [PARALLEL with above]
Week 2:  CC: Phase 4A (admission control) | SN+D4: Phase 4B (maturity tiers)  [ALL PARALLEL]
         D5: Phase 4D (provenance tracking)
Week 3:  CC: Phase 4C (BMM crystallization gate)
         D4: Phase 4E (content-type decay) | CX: Phase 4E.2 (backfill)
Week 4:  CC: integration test + final benchmark + release prep
```

**Agent Legend:**
| Agent | Strengths | Best For |
|-------|-----------|----------|
| CC (Claude Code / Opus) | Deep reasoning, architecture, complex Rust, review | Core pipeline, integration, Phase 4 intelligence |
| SN (Claude Sonnet) | Fast, clean Rust, well-defined subtasks | Dedup logic, promotion/decay, scoring |
| CX (Codex CLI) | Async batch, mechanical ops, benchmarks | Migrations, backups, benchmarks, CI gates |
| D5 (GLM-5 via Droid) | Agentic tool use, interleaved thinking | Schema additions, handler changes, provenance |
| D4 (GLM-4.7 via Droid) | DB + handler work, schema changes, cheap | Unit tests, gate wiring, straightforward features |

**Agent load distribution (remaining work):**
| Agent | Tasks | Est. Hours |
|-------|-------|------------|
| CC | 7A, 7E, 3C.2, 3C.4, 4A.1-4A.2, 4A.4, 4B.4, 4C.1-4C.3, 6.1-6.3, release | ~28hr |
| SN | 2.1-2.4, 4B.2-4B.3 | ~14hr |
| CX | 3A, 3B, 3C.3, 4C.4, 4E.2, 5D.1-5D.4, benchmarks | ~14hr |
| D5 | 4D.1-4D.4, 6.1 (assist) | ~7hr |
| D4 | 7B-7D, 3C.1, 3D, 4A.3, 4B.1, 4E.1 | ~13hr |

**Total: ~76 hours across 5 agents** (down from ~97 -- Phases 0, 0C, 1, 5A-5C complete)

---

## Research References

| Paper | Key Insight | Phase | Who Reads |
|-------|-------------|-------|-----------|
| [ByteRover (2604.01599)](https://arxiv.org/abs/2604.01599) | 96.1% LoCoMo WITHOUT embeddings. 5-tier retrieval, AKL scoring, field boosting | P1 | CC |
| [RRF (Cormack et al.)](https://dl.acm.org/doi/10.1145/1571941.1572114) | `1/(k+rank)` fusion beats any single retriever | P1 | CC |
| [Rethinking Hybrid Retrieval (2506.00049)](https://arxiv.org/abs/2506.00049) | MiniLM + reranking beats bigger models | P1, Deferred (R1) | CC |
| [SmartSearch (2603.15599)](https://arxiv.org/abs/2603.15599) | Ranking > structure. 8.5x fewer tokens | Deferred (R1) | CC |
| [Memori (2603.19935)](https://arxiv.org/abs/2603.19935) | Semantic triples + dedup = 81.95% accuracy at 5% context | P2 | SN |
| [MemFactory (2603.29493)](https://arxiv.org/html/2603.29493) | RL-optimized memory ops (GRPO). 14.8% improvement | Future | CC |
| [agentmemory](https://github.com/rohitg00/agentmemory) | Triple-stream RRF k=60, Jaccard dedup, quality scoring | P1, P2 | CC, SN |
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

---

## Appendix: Incident Log

### 2026-04-07: B-tree Corruption & DB Consolidation

**What happened:** Terminal running Cortex froze. On restart, both `~/cortex/cortex.db` (legacy) and `~/.cortex/cortex.db` (canonical) had B-tree corruption (Tree 326: duplicated page references, invalid page numbers 4279-4597).

**Root cause:** Force-kill during write. WAL checkpoint was on 60s interval; crash happened between checkpoints. The two DBs were identical copies (Droid's bridge had synced them), so both inherited the same corruption.

**Resolution:**
1. Data was still readable despite B-tree corruption (SQLite is resilient)
2. Exported all rows via `SELECT *` into a fresh DB
3. Rebuilt FTS5 indexes with trigram tokenizer
4. VACUUMed: 19.2 MB → 5.9 MB (69% reduction -- most of the 19 MB was dead pages)
5. Removed legacy `~/cortex/cortex.db` entirely
6. Daemon now runs against single `~/.cortex/cortex.db`
7. Updated .gitignore with all runtime files (cortex.lock, cortex.pid, cortex.token, write_buffer.jsonl)

**Verified:** 242 memories, 277 decisions, 638 embeddings intact. FTS working. Integrity OK. Daemon healthy.

**Prevention:** Phase 5 (above) -- startup integrity gate, auto-repair, rolling backups, crash-safe WAL.

### 2026-04-07: Compression Analytics Showing 90% (was 97%)

**What happened:** Analytics page shows avgPercent=91%, down from advertised 97%.

**Root cause:** `estimate_raw_baseline()` in compiler.rs:654 uses `claude_project_slug()` which depends on `env::current_dir()`. When agents boot from different CWDs (Codex, Droid, MCP proxy), the slug doesn't match, so it finds zero or few memory files. Recent boots show baseline=435 tokens (should be 19K+), resulting in 0-45% compression instead of 97%.

**Evidence:** Last 15 boots all show `baseline=435` or `baseline=0`. This tanks the average.

**Fix:** Already planned as task 0C -- replace file-system scanning with DB-based baseline (SUM of all active memory/decision text lengths). This is agent-agnostic and always correct.

**Impact on avg:** Once 0C is fixed, new boots will report correct 97%+ compression. The historical 0-45% entries will gradually age out of the average, or we can add a migration to backfill correct baselines.

---

## Phase 8: Agent Efficiency -- Navigation & Session Intelligence

**Priority:** HIGH -- directly reduces token waste and improves multi-agent UX.
**Added:** 2026-04-08
**Primary goal:** Reduce agent exploration overhead by 80%+ through precomputed navigation in Cortex boot prompts.
**Secondary goal:** Add session continuity tools (`cortex_last`, `cortex_reconnection`) so any AI can pick up where it left off or recover from daemon restarts.
**Invariant:** No changes to Cortex daemon's core retrieval API. All optimizations are additive overlays on `cortex_boot` + new MCP tools.

### Problem Statement

Analysis of agent sessions revealed systematic token waste:

| Waste Category | Tokens Lost | Root Cause |
|----------------|-------------|------------|
| Path exploration | ~2000 | Agent doesn't know where files are, scans directories |
| Deprecated searches | ~500 | Agent searches `.gsd` (deprecated), other dead paths |
| Tool overhead | ~100/call | Using verbose LS/Grep output instead of compact `rtk` commands |
| Session cold-start | ~1000+ | New conversation has no idea what the last one did |
| Reconnection failures | variable | Daemon restart drops agents from Agents panel silently |
| **Total per session** | **~4000+** | Could be near-zero with precomputed navigation + session tools |

### 8A: Navigation Capsule in cortex_boot

**Owner:** CC (Claude Code / Opus)
**Why CC:** Architectural change to the boot compiler, needs deep understanding of capsule system.

**Problem:** Every agent session starts without knowing where anything is. Agents waste tokens discovering paths that the brain already knows.

**Solution:** Add a `paths` capsule to `cortex_boot` that contains:
1. Active project locations (detected via recent file access patterns in cortex.db)
2. Deprecated directories (paths agents should skip)
3. Common shortcuts (daemon → cortex/daemon-rs/, hooks → .claude/hooks/, etc.)

**Capsule schema:**
```json
{
  "paths": {
    "active_projects": {
      "cortex": {
        "path": "C:/Users/aditya/cortex/",
        "daemon": "daemon-rs/",
        "desktop": "desktop/cortex-control-center/",
        "status": "active"
      }
    },
    "deprecated": [
      {"path": ".gsd", "reason": "Replaced by Cortex in 2026-03"},
      {"path": ".claude-mem", "reason": "Migrated to Cortex in 2026-03"}
    ],
    "shortcuts": {
      "daemon": "cortex/daemon-rs/",
      "hooks": ".claude/hooks/",
      "brain_db": ".cortex/cortex.db"
    }
  }
}
```

**Implementation:**

- [ ] **8A.1** Add `paths` capsule type to compiler.rs -- 2hr -- **CC**
- [ ] **8A.2** Project detection: scan recent file references in cortex.db, extract parent directories -- 2hr -- **CC**
- [ ] **8A.3** Deprecated path registry: load from `~/.cortex/config.toml` or hardcode known deprecated -- 1hr -- **CC**
- [ ] **8A.4** Shortcut definitions: load from config or use sensible defaults -- 1hr -- **CC**
- [ ] **8A.5** Wire into boot compiler: add paths capsule to boot output -- 1hr -- **CC**
- [ ] **8A.6** Unit test: verify paths capsule appears in boot output -- 30m -- **D4**

**Acceptance:** Agent receives paths capsule in boot. No more blind directory scanning. Deprecated directories never searched.

**Token savings estimate:** 2000+ per session (eliminates exploration overhead).

### 8B: `cortex_last` -- Session Continuity Tool

**Owner:** CC (Claude Code / Opus)
**Why CC:** New MCP tool, needs daemon handler + MCP proxy wiring.
**Why now:** When starting a new conversation, agents have no way to know "what just happened." `cortex_recall` requires a query -- but you don't know what to query for if you don't know what changed. `cortex_last` gives you the exact last thing stored, so you can immediately pick up context.

**API:**
```
POST /last
Headers: Authorization: Bearer <token>
Query params: ?limit=N (default 1, max 10)
Response: { entries: [{ type: "memory"|"decision", text, agent, created_at, context }] }
```

**MCP tool signature:**
```
cortex_last(limit?: number) → last N entries stored to Cortex, newest first
```

**Use cases:**
- Start new conversation: `cortex_last(3)` → see what the previous session stored → immediate context
- Verify a store worked: `cortex_last(1)` → confirm what was just persisted
- Cross-agent handoff: Agent B calls `cortex_last(5)` → sees what Agent A just did

**Implementation:**

- [ ] **8B.1** Add `GET /last` endpoint to daemon -- 1hr -- **CC** -- Query memories + decisions by `created_at DESC`, limit N, return combined sorted list
- [ ] **8B.2** Add `cortex_last` to MCP tool definitions in mcp_proxy.rs -- 1hr -- **CC**
- [ ] **8B.3** Unit test: store 3 items, call /last?limit=2, verify order and content -- 30m -- **D4**
- [ ] **8B.4** Update MCP tool docs (Info/mcp-tools.md) -- 15m -- **CC**

**Acceptance:** `cortex_last(1)` returns the most recently stored memory or decision. Works from any agent. Response includes agent name and timestamp.

### 8C: `cortex_reconnection` Auto-Registration

**Owner:** D4 (GLM-4.7 via Droid)
**Status:** Function implemented, Agents Tab display in progress (D4 actively fixing).

**Problem:** When the Cortex daemon restarts (crash, manual restart, app restart), connected AIs drop off the Agents panel. The MCP proxy re-establishes its HTTP connection (7A, done), but non-plugin AIs (Cursor, Windsurf, Droid) don't auto-register their session -- they appear disconnected.

**Solution:** `cortex_reconnection` is a new MCP tool that any AI can call to re-register with the daemon after a restart. It should also be called automatically as a sub-step inside `cortex_boot` and `cortex_recall` -- if those calls detect the daemon is up but the agent isn't registered, they trigger reconnection silently.

**Implementation:**

- [x] **8C.1** `cortex_reconnection` MCP tool -- **D4** -- **DONE** (commit 7081dc1, function exists)
- [ ] **8C.2** Fix Agents Tab display for reconnected agents -- **D4** -- IN PROGRESS
- [ ] **8C.3** Auto-reconnect in `cortex_boot`: if boot succeeds but agent not in sessions table, call reconnection internally -- 1hr -- **CC**
- [ ] **8C.4** Auto-reconnect in `cortex_recall`: same logic -- if recall works but session missing, re-register -- 30m -- **CC**
- [ ] **8C.5** Integration test: restart daemon, call cortex_boot from external agent, verify Agents panel shows them -- 30m -- **D4**

**Acceptance:** Daemon restarts. Any AI calls `cortex_boot` or `cortex_recall`. They automatically appear in the Agents panel without manually calling `cortex_reconnection`. Manual `cortex_reconnection` still available as fallback.

### 8D: RTK Integration for Droid

**Owner:** D4 (GLM-4.7 via Droid)
**Why D4:** Hook already exists, just needs debugging on Windows.

**Current state:**
- RTK installed (v0.34.3)
- Hook exists: `~/.factory/hooks/rtk-droid-pretool.sh`
- Hook configured in `~/.factory/settings.json` (PreToolUse → Execute)

**Problem:** Hook may not be triggering correctly on Windows/Git Bash.

- [ ] **8D.1** Verify hook fires: add logging to rtk-droid-pretool.sh -- 30m -- **D4**
- [ ] **8D.2** Test hook end-to-end with Execute tool -- 30m -- **D4**
- [ ] **8D.3** Fix Windows path issues if needed (Git Bash vs PowerShell) -- 1hr -- **D4**
- [ ] **8D.4** Document which commands get rewritten vs need explicit `rtk` prefix -- 30m -- **D4**

**Acceptance:** Droid's shell commands go through RTK automatically. 60-90% token savings on command output.

### 8E: Exploration Pattern Detection (Future Research)

**Owner:** CC (research)
**Status:** DEFERRED -- requires corpus of agent transcripts.

**Hypothesis:** 80% of searches target the same 20 paths (Pareto). Deprecated paths are searched ~15% of the time.

**If approved for v0.5.1:**
- Build exploration profiler that tracks waste patterns per agent
- Auto-suggest path shortcuts based on access frequency
- Feed patterns back into navigation capsule (8A)

---

## Phase 9: Analytics & Visualization Enhancements

**Priority:** MEDIUM -- improves the Control Center's value proposition for daily use.
**Added:** 2026-04-08
**Primary goal:** Make the Analytics page actionable for international users and power users.
**Secondary goal:** Add predictive and historical visualizations that demonstrate Cortex's compounding value.

### 9A: Currency Localization

**Owner:** D4 (GLM-4.7 via Droid)
**Why D4:** Frontend React work, straightforward state management.

**Problem:** Analytics page shows estimated savings in USD. International users (most of the world) want to see values in their local currency.

**Solution:** Add a currency selector to the Analytics page settings. Store preference in `localStorage`. Apply conversion at display time using a static exchange rate table (no API calls -- privacy first).

**Implementation:**

- [ ] **9A.1** Add currency selector dropdown to Analytics page header -- 1hr -- **D4** -- Options: USD, EUR, GBP, INR, JPY, CAD, AUD, BRL, KRW, CNY (top 10 currencies)
- [ ] **9A.2** Static exchange rate table (updated manually per release) -- 30m -- **D4** -- `src/constants.js`
- [ ] **9A.3** Apply conversion to all dollar amounts in Analytics -- 30m -- **D4**
- [ ] **9A.4** Persist preference in localStorage -- 15m -- **D4**

**Acceptance:** User selects INR. All savings amounts display in INR with correct symbol. Preference persists across sessions. No external API calls.

### 9B: Granular Savings Breakdown Toggle

**Owner:** D4 (GLM-4.7 via Droid)
**Why D4:** Frontend, extends existing analytics data.

**Problem:** Analytics shows aggregate savings but doesn't break down WHERE the savings come from. Power users want to see how much they save on recall, store, boot, and tool-call compression separately.

**Solution:** Add a toggle/tab to switch the analytics view between:
1. **Aggregate** (current view -- total savings)
2. **By operation** -- recall savings, store savings, boot compression savings, tool-call savings

**Implementation:**

- [ ] **9B.1** Add toggle component to Analytics page -- 1hr -- **D4**
- [ ] **9B.2** Break down savings data by operation type from existing events table -- 2hr -- **D4** -- Query events grouped by `event_type`
- [ ] **9B.3** Per-operation charts (bar chart or stacked area) -- 1hr -- **D4**
- [ ] **9B.4** Tooltip showing raw vs compressed token counts per operation -- 30m -- **D4**

**Acceptance:** Toggle to "By operation". See separate bars for recall/store/boot/tool savings. Numbers sum to aggregate total.

### 9C: Advanced Graphs & Projections

**Owner:** CC (Monte Carlo model), D4 (React charts)
**Why CC for model:** Statistical modeling needs careful assumptions. D4 for chart rendering.

**Problem:** Users can see current savings but can't visualize the trajectory. Cortex's value compounds over time (more memories → better boot prompts → more savings), but there's no way to SEE this.

**Graphs to add:**

1. **Monte Carlo Savings Projection** -- Given current usage patterns, project estimated savings over 1/3/6/12 months with confidence intervals.
   - Inputs: current daily store rate, current daily recall rate, average compression ratio, token cost per model
   - Output: fan chart showing P10/P50/P90 savings trajectories
   - Assumptions: usage grows 10% monthly (conservative), compression ratio improves with corpus size

2. **Cumulative Savings Over Time** -- Line chart showing total tokens saved since install, with a "break-even" marker (when Cortex saved more than it cost to set up).

3. **Memory Growth Curve** -- Stacked area chart: raw memories, validated, stable, crystallized (requires Phase 4B maturity tiers).

4. **Recall Hit Rate Over Time** -- Tracks how often recall returns useful results. Should trend upward as corpus quality improves.

5. **Agent Activity Heatmap** -- Which agents store/recall most, and when (hour of day × day of week).

**Implementation:**

- [ ] **9C.1** Monte Carlo engine (pure JS, no deps) -- 3hr -- **CC** -- 1000 simulations, configurable growth rate and compression curve
- [ ] **9C.2** Cumulative savings line chart -- 1hr -- **D4** -- Uses existing event data
- [ ] **9C.3** Memory growth stacked area chart -- 1hr -- **D4** -- Requires 4B maturity tiers (deferred until 4B done)
- [ ] **9C.4** Recall hit rate trend line -- 1hr -- **D4** -- Track recalls with results vs empty recalls
- [ ] **9C.5** Agent activity heatmap -- 2hr -- **D4** -- Matrix visualization, hour × day
- [ ] **9C.6** Wire Monte Carlo into a "Projected Savings" card on Analytics page -- 1hr -- **D4**

**Acceptance:** Analytics page shows at least 3 new graphs on first release. Monte Carlo shows a realistic fan chart. All data is derived from local cortex.db -- no external calls.

---

## Phase 10: Daemon Restart Button (Desktop App)

**Priority:** HIGH -- critical UX for non-plugin AI users.
**Added:** 2026-04-08
**Note:** Phase 7C defined the task; this section expands the full UX spec.

### 10A: Restart Button Implementation

**Owner:** D4 (GLM-4.7 via Droid)

The restart button should be prominently placed next to the existing Start/Stop controls. It's the primary recovery action for users when the daemon gets into a bad state.

**UX flow:**
1. User clicks "Restart"
2. Button shows "Restarting..." with a spinner
3. App calls `/shutdown` on the daemon
4. App waits for process exit (poll sidecar status)
5. App starts sidecar (same as Start button)
6. App polls `/health` until daemon responds
7. Button returns to normal state, health indicator goes green
8. Agents auto-reconnect via `cortex_reconnection` (8C)

**Edge cases:**
- Daemon already stopped → just start (skip shutdown)
- Shutdown hangs (>5s) → force-kill process, then start
- Start fails → show error state, offer retry

- [ ] **10A.1** Add Restart button to daemon controls panel -- 1hr -- **D4**
- [ ] **10A.2** Implement restart flow: shutdown → wait → start → health check -- 2hr -- **D4**
- [ ] **10A.3** Handle edge cases (already stopped, shutdown hang, start failure) -- 1hr -- **D4**
- [ ] **10A.4** Test: restart with active agent connections, verify they reconnect -- 30m -- **D4**

**Acceptance:** Click Restart. Daemon cycles in <5s. Agents reappear in panel. No stale state.

---

## Phase 11: Knowledge Graph & Resilience (New -- 2026-04-08)

Inspired by Obsidian Mind (github.com/breferrari/obsidian-mind) and PageIndex (github.com/VectifyAI/PageIndex).

### 11A. Auto-Classification Hook
Detect decisions, incidents, and wins in user messages automatically. UserPromptSubmit hook classifies content and prompts cortex_store without manual intervention. Closes the biggest gap: most knowledge is lost because nobody remembers to store it.

- [ ] **11A.1** Build classifier (keyword + pattern matching) for message types: decision, incident, win, architecture, person-context -- 2hr -- **CC**
- [ ] **11A.2** Wire into UserPromptSubmit hook: classify, prompt store with pre-filled type/context -- 1hr -- **CC**
- [ ] **11A.3** Add confidence threshold (only prompt when classification confidence > 0.7) -- 30m -- **CC**
- [ ] **11A.4** Test: 20 sample messages, verify correct classification and no false positives on casual chat -- 1hr -- **CC**

**Acceptance:** Decisions auto-detected and stored without user manually calling cortex_store. False positive rate < 10%.

### 11B. Backlink Graph
When storing a node that references existing nodes (by keyword overlap or explicit mention), auto-create bidirectional links. Enables "show me everything related to X" graph traversal.

- [ ] **11B.1** Add `links` table to schema (source_id, target_id, link_type, created_at) -- 1hr -- **D4**
- [ ] **11B.2** On cortex_store: scan new node text against existing node titles/keywords, auto-link if similarity > 0.8 -- 2hr -- **Sonnet**
- [ ] **11B.3** Add `cortex_graph(query)` MCP tool: returns linked nodes for a given node -- 1hr -- **D4**
- [ ] **11B.4** Test: store 5 related decisions, verify graph traversal returns connected set -- 30m -- **D4**

**Acceptance:** Storing "auth token migration" auto-links to existing "auth middleware rewrite" node. cortex_graph returns the cluster.

### 11C. Markdown Export (Vault-First Resilience)
`cortex export` dumps all nodes to git-tracked markdown files. If daemon dies, knowledge survives as browsable files. Can be imported back.

- [ ] **11C.1** Add `cortex export [--dir path]` CLI command: writes one .md per node with YAML frontmatter -- 2hr -- **D4**
- [ ] **11C.2** Add `cortex import [--dir path]` CLI command: reads .md files back into DB -- 2hr -- **D4**
- [ ] **11C.3** Auto-export on daemon shutdown (optional, config flag) -- 30m -- **D4**
- [ ] **11C.4** Test: export all 608 nodes, delete DB, import, verify node count and content match -- 1hr -- **D4**

**Acceptance:** Full round-trip export/import with zero data loss. Exported files are human-readable markdown.

### 11D. PageIndex-Style Tree Retrieval
Hierarchical document indexing via LLM reasoning traversal. Instead of flat vector similarity, build a tree-of-contents index and let the LLM navigate it. Enhances recall for long documents and codebases.

- [ ] **11D.1** Research: read PageIndex paper, evaluate feasibility with local LLM (Qwopus 9B) -- 2hr -- **CC**
- [ ] **11D.2** Design: tree index schema for cortex nodes (parent/child hierarchy by topic) -- 1hr -- **CC**
- [ ] **11D.3** Implement tree builder: cluster nodes by topic, create hierarchy -- 3hr -- **Sonnet**
- [ ] **11D.4** Implement tree traversal retrieval: given query, navigate tree to find relevant cluster -- 3hr -- **Sonnet**
- [ ] **11D.5** Benchmark: compare tree retrieval vs current RRF fusion on recall@10 -- 1hr -- **CX**

**Acceptance:** Tree retrieval matches or exceeds RRF on recall@10. Works without external embedding service.

---

## Success Metrics (Updated)

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
| Exploration tokens | ~2000/session | <200 | -- |
| Session cold-start | ~1000 tok | <100 tok | -- |
| Agent reconnection | manual | automatic | -- |

---

## Appendix: Codex Phase 0A Report
- Dry-run output: Summary: Would delete 29 memories, 0 decisions. Estimated noise reduction: 5.9%
- Deletions applied: 29 memories, 0 decisions
- Orphan check: pass (broad orphan query = 0, type-aware orphan query = 0; FTS MATCH `cortex` returned 25 memory rows and 135 decision rows)
- Post-purge benchmark: GT precision 51.3%, MRR 0.74, hit rate 85.0% (below the 60% post-embedding-fix target; likely retrieval/ranking instability remains)
- Commit: this commit (hash reported by Codex final response)
