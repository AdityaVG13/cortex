# Cortex Roadmap

Public roadmap for contributors and users. Tasks organized by release milestone with assigned tooling.

**Legend**: Claude Code = CC, Cursor = CU, Codex CLI = CX, Gemini CLI = GC, Droid GLM 5 = D5, Droid GLM 4.7 = D4

---

## v0.4.0 -- Public Launch

All CRITICAL security and OSS-readiness tasks are complete. These remaining items ship with or immediately after the v0.4.0 tag.

| # | Task | Owner | Status |
|---|------|-------|--------|
| 86 | Version bump to v0.4.0 + GitHub release | CU | |
| ✓90 | README rewrite for public audience | GC | **DONE** |
| ✓92 | Review architecture docs: public vs internal vs remove | GC | **DONE** |
| ✓93 | ~~ROADMAP.md for contributors~~ | -- | **DONE** |
| ✓94 | CONTRIBUTING.md + SECURITY.md | CX | Done - GEMINI |
| 91 | Recall quality baseline (surprise score analysis) | GC | |
| ✓109 | Auto-generate CHANGELOG on version tags (git-cliff) | D4 | Done - cliff.toml + .github/workflows/changelog.yml |
| ✓113 | Verify desktop app (Control Center) is wired end-to-end | CC | Done |
| ✓114 | Wire Agents panel: show connected MCP sessions, active agents, team user presence | D5 | Done - Already wired: AgentItem renders sessions, SSE listens for session events, api(/sessions) fetches data. Fixed CSS bug: --agent-droid had quotes around hex value. |
| ✓115 | Desktop app: fix About icon, update License field, version bump to v0.4.0 | D4 | Done - icon uses BASE_URL, license AGPL-3.0, version 0.4.0 |
| ✓116 | Version bump ALL files to v0.4.0 + AGPL-3.0 license sweep | D4 | Done - all Cargo.toml, package.json, README updated; 49 .rs files have SPDX headers |

---

## v0.5.0 -- Foundation Hardening

Schema discipline, derived-state repair, and memory quality. Makes Cortex reliable enough for daily use.

| # | Task | Source | Owner | Details |
|---|------|--------|-------|---------|
| G1 | TTL / hard expiration (`expires_at` column) | Gemini #1 | D5 | Temporal facts need explicit expiry, not just decay |
| G2 | Session-based rollback (`trace_id` index) | Gemini #2 | D5 | `cortex admin rollback --session-id` for faulty agent runs |
| C1 | Schema versioning + migration epochs | Codex #1 | CC | Named migrations, `cortex doctor` verification command |
| C2 | Derived-state repair (`reindex`, `re-embed`, `recrystallize`) | Codex #2 | CC | Consistency scanners + idempotent rebuild commands |
| C6 | Semantic dedup at store time | Codex #6 | CC | Distinguish "new fact" from "reinforcement of existing" |
| C5 | Boot prompt audit trail | Codex #5 | D5 | Record which sources were included + ranking metadata |

---

## v0.6.0 -- Governance & Economics

Budget controls, retention policies, and human review surfaces. Makes Cortex manageable at scale.

| # | Task | Source | Owner | Details |
|---|------|--------|-------|---------|
| C3 | Budget governance (per-endpoint limits) | Codex #3 | CC | Max recalls/turn, max boot budget, invocation frequency caps |
| C9 | Retention policy classes | Codex #9 | CC | Durable knowledge vs operational context vs audit records vs ephemera |
| C10 | Human review workflows | Codex #10 | CC | Inboxes for shared knowledge, review queues, promotion paths |
| G5 | Dynamic context ranking for injectors | Gemini #5 | D5 | Rank by activeness + relevance, inject top 3-5 items only |
| G3 | Epistemology worker (contradiction triage) | Gemini #3 | CC | Background worker creates resolution tasks for disputed facts |
| C4 | Adapter conformance spec + shared tests | Codex #4 | CX | Canonical behavior spec for store/recall/peek/boot/inject |

---

## v0.7.0 -- Multi-Tenant Hardening

Privacy, fairness, and agent identity for team deployments.

| # | Task | Source | Owner | Details |
|---|------|--------|-------|---------|
| G4 | Deep erasure (`DELETE /forget`) | Gemini #4 | CC | Scrub row + FTS + embedding + re-crystallize affected crystals |
| G8 | Crystal lineage tracking | Gemini #8 | D5 | `source_memory_ids` on crystals, re-crystallize on source delete |
| G9 | Invocation-bound capability tokens (IBCTs) | Gemini #9 | CC | Cryptographic agent identity + scoped authority per request |
| C7 | Multi-tenant fairness (quotas, admission control) | Codex #7 | D5 | Per-user rate/store/recall limits, queue prioritization |
| C8 | Backup, restore, disaster recovery | Codex #8 | CC | Source-of-truth backup, derived rebuild, encrypted key handling |
| G7 | Namespace-isolated embedding spaces | Gemini #7 | CC | Separate HNSW index per team for enterprise security |

---

## v0.8.0 -- Advanced Agent Support

Branch awareness, provenance, deadlock detection, and task dispatch for autonomous agent swarms.

| # | Task | Source | Owner | Details |
|---|------|--------|-------|---------|
| TD1 | Task dispatch UI in Control Center | -- | CC | Create tasks in Pending tab with full prompt text, assign to a connected agent from dropdown |
| TD2 | Agent task pull on boot | -- | CC | Cortex boot/MCP injects assigned pending tasks; agent auto-claims and starts working |
| TD3 | Multi-agent coordination protocol | -- | CC | Agents use /message for coordination, /lock for file exclusivity, status updates via /feed |
| TD4 | Task dependency graph | -- | D5 | Tasks can block/depend on other tasks; UI shows DAG; agents respect ordering |
| TD5 | Live task progress in Control Center | -- | CC | Real-time status updates via SSE; show which agent is working on what, files touched, progress |
| G10 | Branch-aware filtering (`git_ref` on store/recall) | Gemini #10 | CC | Prioritize memories from current branch + ancestors |
| G11 | Reasoning provenance (RICR) | Gemini #11 | CC | Provenance links to source commit/session/parent decision |
| G6 | Multi-agent deadlock detection | Gemini #6 | D5 | Dependency graph on tasks, cycle detection, lock-breaking |
| 9 | Chrome extension for claude.ai/chatgpt/gemini | CC | | Manifest V3, content scripts, background worker |
| 40 | Test Chrome extension across 3 platforms | CC | | Depends on #9 |

---

## v1.0.0 -- AI Information Ingester

A new product surface: import, separate, analyze, and index data from external AI platforms.

| # | Task | Owner | Details |
|---|------|-------|---------|
| I1 | ChatGPT export parser (conversations.json) | CC | Parse OpenAI export format, extract decisions/facts/preferences |
| I2 | Claude conversation ingester | CC | Parse claude.ai export, separate by project/topic |
| I3 | Gemini conversation ingester | CC | Parse Google AI Studio / Gemini exports |
| I4 | Intelligent separator (topic detection + classification) | CC | ML/heuristic pipeline to classify: decision, preference, fact, ephemeral |
| I5 | Dedup against existing Cortex memories | CC | Cross-reference ingested data with existing memories |
| I6 | Confidence scoring for ingested data | CC | Lower confidence for older/ambiguous imports |
| I7 | Bulk import CLI (`cortex ingest <export.json>`) | D5 | User-facing command with progress, preview, and dry-run |

---

## Existing Future Tasks (from v0.3.0 cycle)

Carried forward from model_delineation.md. Not yet assigned to a milestone.

### Completed (verified 2026-04-05)

| # | Task | Owner | Status | Evidence |
|---|------|-------|--------|----------|
| ✓58-59 | Owner_id + visibility on remaining tables | D5 | **DONE** | `db.rs:379-580` - All 12 tables have owner_id/visibility columns |
| ✓64-65 | Solo/team mode recall scoping | D5 | **DONE** | `recall.rs:110-126` - `is_visible()` + over-fetch strategy at line 534 |
| ✓67-70 | Conductor ownership + visibility API (4 tasks) | D5 | **DONE** | `conductor.rs` - All endpoints filter by owner_id; tasks/feed have visibility |
| ✓73 | Fresh install defaults to solo mode | D5 | **DONE** | `db.rs:346` - `INSERT ... VALUES ('mode', 'solo')` |
| ✓76 | Role enforcement with CHECK constraints | D5 | **DONE** | `db.rs:322-323, 338-339` - CHECK constraints on role/visibility |
| ✓78 | Row-level NULL owner_id prevention | D5 | **DONE** | `recall.rs:118-120` - `is_visible()` returns false for NULL; migration assigns all rows |

### Remaining (needs implementation)

| # | Task | Owner | Priority | Details |
|---|------|-------|----------|---------|
| 6 | OpenAI function adapter spec + handler | D5 | **HIGH** | No code exists. Need: REST endpoint for function definitions JSON, tool_call handler |
| 19 | Key rotation with 72h grace period | D5 | MEDIUM | No `prev_key_hash`/`prev_key_expires` columns. Need schema migration + auth dual-key check |
| 21 | SQLCipher encryption at rest | D5 | LOW (OPT) | **Optional per spec** - "RECOMMENDED for team mode, not required". Documentation task |
| 22-25 | MCP/OpenAI adapter protocol work (4 tasks) | D5 | PARTIAL | MCP ✅ done. OpenAI ❌ missing (same as #6) |
| 77 | Validate visibility enforcement at query level | GC | LOW | Add integration tests for visibility edge cases |
| 81 | Database size monitoring + growth trajectory | GC | LOW | Metrics endpoint + dashboard |
| 82 | Document deferred features | GC | LOW | Update docs/schema/06 with what's intentionally excluded |

### Day-1 Release Assessment (2026-04-05)

**Security model: COMPLETE**
- All visibility enforcement ✅
- Owner scoping on all tables ✅
- NULL owner_id prevention ✅
- CHECK constraints on enums ✅
- Solo mode default on fresh install ✅

**Not blockers for open-source release:**
- #6 (OpenAI adapter) - Enhancement, users can use Python/TS SDKs
- #19 (Key rotation) - Enhancement, basic auth works
- #21 (SQLCipher) - Optional feature, documentation-only

**Recommendation:** Ship v0.4.0 now, add #6 and #19 in v0.5.0.

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and PR guidelines. Tasks marked with an owner are suggestions, not locks -- anyone can pick up any task. Open an issue or PR to claim work.

---

# Long-Term Vision (post-v1.0.0)

> **Note:** This section contains research-derived roadmap items from archived documents. These are aspirational features beyond v1.0.0.

## Intelligence Layer
| # | Feature | Source | Priority | Status |
|---|---------|--------|----------|--------|
| 1 | Ebbinghaus decay scoring | ROADMAP-v3.md M1.3, Cognitive Aging Research | HIGH | IN_PROGRESS |
| 2 | Semantic dedup on writes (sim > 0.92) | ROADMAP-v3.md M1.4, TODO.md stolen ideas, memory-systems research | HIGH | SPEC_ONLY |
| 3 | Memory type system (preference/decision/observation/rule/trace) | ROADMAP-v3.md M1.5, memory-systems research | HIGH | IN_PROGRESS |
| 4 | Temporal validity (triple-date: created/accessed/referenced) | ROADMAP-v3.md M1.6, Cognitive Aging Research | HIGH | SPEC_ONLY |
| 5 | PostToolUse Observer hook for ambient capture | ROADMAP-v3.md M2.1, AI Memory Capture Research | HIGH | SPEC_ONLY |
| 6 | Qwen-2.5-Coder-1.5B fact extraction via llama.cpp | ROADMAP-v3.md M2.2, AI Memory Capture Research | MEDIUM | SPEC_ONLY |
| 7 | Tiered compression (Observer→Episodic→Semantic) | ROADMAP-v3.md M2.3, AI Memory Capture Research | HIGH | SPEC_ONLY |
| 8 | NLI confidence gating for contradictions | ROADMAP-v3.md M2.4, AI Memory Capture Research | HIGH | SPEC_ONLY |
| 9 | Failure-to-rule promotion (3+ episodic failures→auto-promote) | ROADMAP-v3.md M2.5, AI Memory Capture Research | HIGH | SPEC_ONLY |
| 10 | cortex-dream --execute pipeline | ROADMAP-v3.md M2.6, TODO.md done items | MEDIUM | SHIPPED (partial) |
| 11 | Focus pruning tools (start_focus/complete_focus) | ROADMAP-v3.md M2.7, Cognitive Aging Research | HIGH | SPEC_ONLY |
| 12 | Composite retrieval scoring (similarity×recency×importance×frequency) | memory-systems research, Cognitive Aging Research | HIGH | IN_PROGRESS |
| 13 | Failure reflection storage (what/why/next trial) | memory-systems research | HIGH | SPEC_ONLY |
| 14 | Self-Tuning Compiler (track boot content usage, shift budget) | TODO.md deferred, cognitive-aging research | MEDIUM | SPEC_ONLY |
| 15 | Recall Learns from Downstream Success (RL for relevance) | TODO.md deferred | MEDIUM | SPEC_ONLY |
| 16 | Async capsule compilation (background LLM compression) | TODO.md stolen ideas, memory-systems research | MEDIUM | SPEC_ONLY |
| 17 | cortex_skill command (emit optimal agent prompt for self-teaching) | TODO.md stolen ideas, competitive-intel.md | MEDIUM | SPEC_ONLY |
| 18 | Memory pressure eviction (70% trigger→evict to 56%) | TODO.md stolen ideas, memory-systems research | LOW | SPEC_ONLY |
| 19 | Self-check cron (scheduled self-audits with LLM remediation) | TODO.md stolen ideas, competitive-intel.md | LOW | SPEC_ONLY |
| 20 | emit_decision helpers (cortex.emit_decision() zero-friction stores) | TODO.md stolen ideas, competitive-intel.md | MEDIUM | SPEC_ONLY |
| 21 | Structured Dissent Protocol (store debates as first-class objects) | TODO.md deferred, conductor ideation | LOW | SPEC_ONLY |
| 22 | Transcript Tap / Ghost Protocol (auto-extract from raw transcripts) | TODO.md deferred, conductor ideation | MEDIUM | SPEC_ONLY |
| 23 | Dream Consensus Protocol (2-3 local models, 2-of-3 agreement) | TODO.md deferred, conductor ideation | MEDIUM | SPEC_ONLY |
| 24 | Background compression via Qwen-2.5-Coder worker | Local Model Research | MEDIUM | SPEC_ONLY |

## Coordination Layer
| # | Feature | Source | Priority | Status |
|---|---------|--------|----------|--------|
| 25 | Semantic Rebase protocol (state=base+deltas on conflict) | ROADMAP-v3.md M3.2, Multi-Agent Orchestration Research | HIGH | SPEC_ONLY |
| 26 | Git-branch awareness (HEAD check on every query) | ROADMAP-v3.md M3.3, Security Research | HIGH | SPEC_ONLY |
| 27 | SSE event stream for real-time push | ROADMAP-v3.md M3.4, conductor ideation | HIGH | IN_PROGRESS |
| 28 | TODO-Claim CRDT (atomic task assignment) | ROADMAP-v3.md M3.5, Security Research, conductor ideation | HIGH | SHIPPED |
| 29 | MCP federation (discoverable Cortex as MCP server) | ROADMAP-v3.md M3.6, Multi-Agent Orchestration Research | MEDIUM | SHIPPED |
| 30 | Provenance citations (map facts to COMMIT_HASH, UTTERANCE_ID) | ROADMAP-v3.md M3.7, Security Research, AI Memory Capture Research | HIGH | SPEC_ONLY |
| 31 | Decision Provenance DAG (trace lineage across agents/sessions) | TODO.md deferred, conductor ideation | MEDIUM | SPEC_ONLY |
| 32 | Event-Sourced Brain (events table as source, tables as views) | TODO.md deferred, Multi-Agent Orchestration Research | HIGH | SPEC_ONLY |
| 33 | Token Dedup Router (cross-agent context elimination via bloom filters) | TODO.md deferred, conductor ideation | MEDIUM | SPEC_ONLY |
| 34 | AWCP Workspace Projection (scoped filesystem view to executors) | Security Research, Local Model Research | MEDIUM | SPEC_ONLY |
| 35 | APBDA Routing Engine (adaptive Dijkstra task allocation) | Multi-Agent Orchestration Research | HIGH | SPEC_ONLY |
| 36 | Lesson Banking Module (persistent success lessons peer learning) | Multi-Agent Orchestration Research | MEDIUM | SPEC_ONLY |
| 37 | Agent Priority Queue with complexity-based model routing | Multi-Agent Orchestration Research | MEDIUM | SPEC_ONLY |
| 38 | Decision contracts on conductor tasks (advance/rework/skip/fail) | competitive-intel.md AO CLI | MEDIUM | SPEC_ONLY |
| 39 | Rework loops with context (feedback as structured input) | competitive-intel.md AO CLI | MEDIUM | SPEC_ONLY |

## Performance & Security Layer
| # | Feature | Source | Priority | Status |
|---|---------|--------|----------|--------|
| 40 | DB integrity checks on startup | ROADMAP-v3.md M1.1, model_delineation.md task #111 | HIGH | SHIPPED |
| 41 | Rust write-loss fix (missing flush/sync on shutdown) | ROADMAP-v3.md M1.2 | HIGH | SPEC_ONLY |
| 42 | Single-writer SQLite thread with mpsc channel | ROADMAP-v3.md M4.1, Multi-Agent Orchestration Research | HIGH | SPEC_ONLY |
| 43 | SimSIMD vector kernels (AVX-512/NEON cosine similarity) | ROADMAP-v3.md M4.2, Local Model Research | MEDIUM | SPEC_ONLY |
| 44 | ORT embedding engine (bundled ONNX Runtime, zero external API calls) | ROADMAP-v3.md M4.3, model_delineation.md | HIGH | SHIPPED (v0.3.0) |
| 45 | Biscuit auth tokens (Ed25519-signed capability tokens) | ROADMAP-v3.md M4.4, Security Research | HIGH | SPEC_ONLY |
| 46 | Zenoh message bus (50Gbps throughput, 13µs latency) | ROADMAP-v3.md M4.5, Security Research | LOW | SPEC_ONLY |
| 47 | DLP secrets scrubber (Bayesian filter redact keys/PII) | ROADMAP-v3.md M4.6, Security Research | MEDIUM | SPEC_ONLY |
| 48 | AIP auth middleware (Invocation-Bound Capability Tokens) | Security Research | HIGH | SPEC_ONLY |
| 49 | Datalog scope attenuation (Biscuit tokens restrict write access) | Security Research | MEDIUM | SPEC_ONLY |
| 50 | Grammar-constrained sampling (JSON Schema enforcement on tool calls) | Local Model Research | MEDIUM | SPEC_ONLY |
| 51 | JIT citation verification (force agents to verify code exists) | Security Research, Local Model Research | HIGH | SPEC_ONLY |
| 52 | KV Cache Global Store (distributed L3 storage of prompt KV tensors) | Multi-Agent Orchestration Research | MEDIUM | SPEC_ONLY |
| 53 | LLMLingua compression (token pruning based on perplexity) | Multi-Agent Orchestration Research | MEDIUM | SPEC_ONLY |
| 54 | MAST failure auditing (automated 14-failure-mode monitoring) | Multi-Agent Orchestration Research | LOW | SPEC_ONLY |
| 55 | FSRS-6 memory decay algorithm (power-curve retrievability) | Local Model Research | MEDIUM | SPEC_ONLY |
| 56 | Formal JSON-RPC relay (tightened agent API surface) | TODO.md stolen ideas | LOW | SPEC_ONLY |

## Desktop App (Control Center)
| # | Feature | Source | Priority | Status |
|---|---------|--------|----------|--------|
| 57 | 3D Brain Visualizer (Interactive Three.js memory graph) | TODO.md done, VISION.md | HIGH | SHIPPED |
| 58 | Agent Presence Dashboard (heartbeat animation, activity streams) | VISION.md, conductor ideation | HIGH | SHIPPED |
| 59 | Memory Explorer (card-based browser, semantic clustering) | VISION.md | HIGH | SHIPPED |
| 60 | Tauri Desktop App production build (embedded daemon, system tray) | TODO.md in progress, VISION.md | HIGH | SHIPPED |
| 61 | SSE → Tauri dashboard wiring | ROADMAP-v3.md M3.4 | HIGH | IN_PROGRESS |
| 62 | Storm ritual codebase graph visualization | conductor ideation | LOW | SPEC_ONLY |
| 63 | Workspace projection via AWCP | Security Research | MEDIUM | SPEC_ONLY |
| 64 | Auto-update via tauri-plugin-updater | model_delineation.md done #99 | HIGH | SHIPPED |
| 65 | App icon (all Tauri icon sizes) | model_delineation.md done #88 | MEDIUM | SHIPPED |

## Developer Experience
| # | Feature | Source | Priority | Status |
|---|---------|--------|----------|--------|
| 66 | Import History (ingest ChatGPT/Claude/Gemini conversation exports) | TODO.md up next | MEDIUM | SPEC_ONLY |
| 67 | OpenAI function adapter (compatibility layer) | model_delineation.md task #6 | MEDIUM | SPEC_ONLY |
| 68 | Key rotation with 72h grace period | model_delineation.md task #19 | MEDIUM | SPEC_ONLY |
| 69 | SQLCipher encryption at rest | model_delineation.md task #21 | LOW | SPEC_ONLY |
| 70 | cortex_peek (one-line summaries before full recall) | TODO.md stolen ideas, competitive-intel.md | HIGH | SHIPPED |
| 71 | Blast radius metadata (store impact scope on decisions) | competitive-intel.md SoulForge | HIGH | SPEC_ONLY |
| 72 | Git co-change correlation feed into recall relevance | competitive-intel.md SoulForge | HIGH | SPEC_ONLY |
| 73 | Automatic model routing (recommend which model by complexity) | competitive-intel.md AO CLI | HIGH | SPEC_ONLY |

---

**Long-term summary:** Found 73 roadmap items across 5 categories. Status distribution: 13 SHIPPED, 5 IN_PROGRESS, 55 SPEC_ONLY. Key unimplemented features include ambient capture (PostToolUse hook), semantic conflict resolution (Semantic Rebase), and offline local model integration (Qwen workers).
