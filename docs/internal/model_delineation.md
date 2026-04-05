# Model Delineation -- Cortex Work Assignments

<!-- When you finish a task, put a ✓ in the Done column. -->

## Tool Reference

| Tool | Model | Cost | Use For |
|------|-------|------|---------|
| Claude Code | Opus 4.6 | $$$$$ | Multi-file Rust, architecture, security, complex reasoning |
| Cursor (Opus) | Opus 4.6 | $$$$ | Complex single-codebase work needing deep understanding |
| Cursor (Sonnet) | Sonnet 4.6 | $$ | Single-file edits, straightforward refactors |
| Codex CLI | GPT-5.3 | $$ | Batch tasks, overnight autonomous, docs generation |
| Gemini CLI | 2.5 Pro | $ | 1M context analysis, research, full-repo documentation |
| Droid (GLM 5) | GLM 5 (0.4x) | $ | Medium features, API endpoints, adapters |
| Droid (GLM 4.7) | GLM 4.7 (0.25x) | ¢ | Config, schema DDL, boilerplate, mechanical cleanup |
| Gemini Flash | 2.5 Flash | ¢ | Formatting, drafts, lookups, light code tasks |

---

# Open-Source Release Tasklist

## CRITICAL (blocks public release)

### Claude Code (Opus)

| # | Task | Done | Details |
|---|------|------|---------|
| 83 | ✓ Fix /unfold visibility bypass | `d5fd199` | Plumbing by Cursor. Fully secure now that #84 + #85 have landed. |
| 84 | ✓ Fix is_visible NULL owner_id policy | `c58e573`, `4182869` | `is_visible` fails closed: `caller_id=None` → deny, `owner_id=None` → deny. All INSERT paths (store, indexer, crystallize, focus) now set `owner_id` when present. Conditional SQL for solo/team compat. 6 visibility unit tests. Clippy clean. |
| 85 | ✓ Fix MCP per-caller identity | `08d12c2` | `mcp_stdio.rs` resolves caller at startup from `CORTEX_API_KEY` env var or `cortex.token` file. Matches against `team_api_key_hashes`. Removed dead `handle_mcp_message` wrapper. |
| 111 | ✓ Replace hardcoded indexer paths with configurable custom sources | `58b221d` | Deleted all 6 extended indexer functions. Replaced with `index_custom_sources` reading `~/.cortex/sources.toml` or `CORTEX_EXTRA_SOURCES` env var. `estimate_raw_baseline` updated. 59 tests, 8/8 smoke checks pass. |

## HIGH (should ship with release)

### Cursor (Opus)

| # | Task | Done | Details |
|---|------|------|---------|
| 86 | Version bump to v0.3.1 + GitHub release | | v0.3.0 tag exists. Needs: Cargo.toml bump, CHANGELOG.md entry, build release binary, `gh release create`, attach bundled desktop installer. |

### Codex CLI

| # | Task | Done | Details |
|---|------|------|---------|
| 93 | ROADMAP.md for contributors | | Process all architecture docs into public roadmap with contribution areas. |
| 94 | CONTRIBUTING.md + SECURITY.md | | Dev setup, build instructions, PR guidelines, vulnerability disclosure policy. |

### Gemini CLI

| # | Task | Done | Details |
|---|------|------|---------|
| 90 | README rewrite for public audience | | 1M context read of entire repo. Rewrite for external devs, not internal team. Keep AdityaVG13 as repo owner. |
| 92 | Review architecture docs: public vs internal vs remove | | docs/architecture/, docs/compatibility/, docs/schema/, docs/archive/ -- classify each. |

### Droid (GLM 4.7)

| # | Task | Done | Details |
|---|------|------|---------|
| 95 | ✓ Fix .gitignore patterns for OSS release | `042138d` | Updated docs/ to ignore specific subdirs only, added 22+ patterns: *.db-journal, .env.*, *.pem, *.key, daemon-rs/target/, desktop paths |
| 96 | ✓ Verify no legacy Node.js src directory exists | `042138d` | No src/ directory found -- all Node.js code already migrated to daemon-rs Rust or removed |
| 112 | ✓ Remove all Ollama references | `042138d` | Removed from embeddings.rs, mcp.rs, App.jsx, constants.js, BrainVisualizer.jsx, styles.css, workers/*.md, workers/*.py, README.md, .gitignore |

## MEDIUM (nice to have for launch)

### Gemini CLI

| # | Task | Done | Details |
|---|------|------|---------|
| 91 | Recall quality baseline analysis | | Surprise score distribution across 220+ decisions. Define meaningful thresholds. |

## LOW (post-launch)

### Droid (GLM 4.7)

| # | Task | Done | Details |
|---|------|------|---------|
| 109 | Auto-generate CHANGELOG on version tags | | GitHub Actions on `v*` tag. `git-cliff` or conventional-changelog. |

---

# Future Roadmap (not blocking release)

### Claude Code (Opus)

| # | Task | Details |
|---|------|---------|
| 9 | Chrome extension for claude.ai, chatgpt.com, gemini.com | Manifest V3, content scripts, background worker. Blocked on team-mode test env. |
| 40 | Test Chrome extension across 3 platforms | Windows, macOS, Linux. Depends on #9. |

### Droid (GLM 5)

| # | Task | Done | Details |
|---|------|------|---------|
| 6 | | | OpenAI function adapter spec and handler (compatibility/02) -- NOT YET IMPLEMENTED |
| 19 | | | Key rotation with 72h grace period (compatibility/03) -- NOT YET IMPLEMENTED |
| 21 | | | SQLCipher encryption at rest (compatibility/03) -- OPTIONAL per spec, documentation task |
| 22-25 | | | MCP/OpenAI adapter protocol work (4 tasks) -- MCP ✅ done, OpenAI ❌ missing |
| ✓58-59 | ✓ | `db.rs:379-580` | Owner_id + visibility on all 12 tables (memories, decisions, memory_clusters, recall_feedback, tasks, messages, feed, feed_acks, activities, focus_sessions, sessions, locks) |
| ✓64-65 | ✓ | `recall.rs:110-126` | Solo/team mode recall scoping via `is_visible()` + over-fetch strategy |
| ✓67-70 | ✓ | `conductor.rs` | Conductor ownership + visibility API -- all endpoints filter by owner_id |
| ✓73 | ✓ | `db.rs:346` | Fresh install defaults to solo mode via `INSERT ... VALUES ('mode', 'solo')` |
| ✓76 | ✓ | `db.rs:322-339` | Role enforcement with CHECK constraints on role/visibility enums |
| ✓78 | ✓ | `recall.rs:118-120` | Row-level NULL owner_id prevention -- `is_visible()` returns false, migration assigns all rows |

### Droid (GLM 4.7)

Remaining original tasks #12-18, 20, 29-30, 32-36, 41, 54-57, 63, 79 -- see Completed section for reference.

### Gemini CLI

| # | Task | Details |
|---|------|---------|
| 77 | Validate visibility enforcement at query level | schema/06 |
| 81 | Database size monitoring and growth trajectory | schema/06 |
| 82 | Document deferred features | schema/06 |

---

# Long-Term Vision (post-release)

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

# Notes

### Open-source readiness: 100% (CRITICAL tasks complete)
All clean-slate identity tasks shipped (#101-108, #110). #111 custom sources complete. Security chain #83-#84-#85 fully resolved. **No CRITICAL blockers remaining.** HIGH/MEDIUM tasks are quality improvements, not release blockers.

### Security chain: RESOLVED
- #85 (`08d12c2`): MCP caller identity resolved at startup
- #84 (`c58e573`, `4182869`): `is_visible` fail-closed + owner_id on all writes
- #83 (`d5fd199`): unfold visibility plumbing (was already done)
- All three shipped together. 65 tests, 8/8 smoke checks, clippy clean.

### #104 status: cosmetic rename only (superseded by #111)
Cursor commit `74a43fe` renamed `self-improvement-engine` to `knowledge-sources` and `self-improvement` to `extended-knowledge`. Gated behind `CORTEX_INDEX_EXTENDED=1` (commit `ff887af`). But 6 functions still hardcode paths to directories no end user will have (`knowledge-sources/tools/gorci`, `extended-knowledge/crew`, etc.). #111 replaces all 6 with one generic `index_custom_sources` function reading from user config. Zero ghost dependencies.

### Team-mode test environment needed
Required for: Chrome extension (#9, #40), visibility E2E validation, MCP identity testing.

### Recall quality (Gemini CLI #91)
Surprise metric has no baseline. Distribution analysis needed before the score means anything.

### Builder prompt lesson
Critic catches cost 2-3x what prevention costs. Every builder prompt must include a `## Known Pitfalls` section with schema quirks, naming inconsistencies, and auth patterns.

---

# Completed Tasks (v0.3.0 release cycle)

### Clean-Slate OSS Gate

| # | Task | Commit | Details |
|---|------|--------|---------|
| 101 | ✓ compiler.rs: replace hardcoded "User: Aditya" identity | `33fa438` | Dynamic detection via `USERNAME`/`USER` env + OS detection. |
| 102 | ✓ compiler.rs + indexer.rs: replace hardcoded `C--Users-aditya` | `33fa438` | Dynamic Claude projects dir from CWD slug. |
| 103 | ✓ service.rs: replace `"aditya"` fallback username | `33fa438` | Changed to `"cortex-user"`. |
| 104 | ✓ indexer.rs: gate extended indexers behind env var | `ff887af` | `CORTEX_INDEX_EXTENDED=1` gates 6 functions. Cosmetic rename in `74a43fe`. **Superseded by #111.** |
| 105 | ✓ workers/drift_detector.py: hardcoded path | `33fa438` | Dynamic CWD derivation. |
| 106 | ✓ tools/ingest_chatgpt.py: remove personal refs | `6b2739c` | 30+ classification labels generalized. |
| 107 | ✓ Backup + remove personal files from git | `d5c390a`, `9c12a91` | 14 files backed up to `.personal-backup/`, `git rm`'d, `.gitignore` updated. |
| 108 | ✓ Clean install end-to-end test (THE GATE) | `86cdc15`, `cdaca43` | Unit tests + `scripts/clean_install_smoke.sh`. Grep gates pass. |
| 110 | ✓ Delete personal LLM configs | `33fa438` | Modelfile.glm, Modelfile.deepseek removed. |

### Desktop App

| # | Task | Details |
|---|------|---------|
| 87 | ✓ Sidecar real daemon, kill embedded copy | Tauri launches cortex.exe as sidecar. embedded_daemon.rs deleted. |
| 88 | ✓ App icon | All Tauri icon sizes generated. |
| 89 | ✓ README: release badge, download link | Badge box populated. |
| 97 | ✓ Fix all dead UI, remove Ollama box | All 11 panels audited. Dead buttons removed. |
| 98 | ✓ About tab (panel #12) | Creator attribution + contributors section. |
| 99 | ✓ Auto-update via tauri-plugin-updater | In-app update check functional. |
| 100 | ✓ Version sync to 0.3.0 | tauri.conf.json, desktop Cargo.toml, package.json all match. |

### Claude Code (Opus) -- earlier cycle

| # | Task | Source |
|---|------|--------|
| 4 | ✓ Graceful degradation across all system layers | compatibility/01 |
| 66 | ✓ Over-fetch-then-filter embedding recall | schema/04 |
| 71 | ✓ Admin endpoints | schema/05 |
| 72 | ✓ CLI commands: cortex user/team/admin management | schema/05 |
| 74 | ✓ Solo-to-team migration | schema/05 |

### Cursor (Sonnet/GLM 5) -- earlier cycle

| # | Task | Source |
|---|------|--------|
| 1 | ✓ HTTP REST API as core transport | compatibility/01 |
| 2 | ✓ Unified auth layer | compatibility/01 |
| 3 | ✓ TLS via rustls | compatibility/01 |
| 5 | ✓ MCP adapter: stdio-to-HTTP bridge | compatibility/02 |
| 7 | ✓ Python SDK (cortex-memory on PyPI) | compatibility/02 |
| 8 | ✓ TypeScript SDK (@cortex-memory/client) | compatibility/02 |
| 10 | ✓ System prompt injector CLI | compatibility/02 |
| 11 | ✓ Standalone fallback mode for solo MCP | compatibility/02 |
| 15 | ✓ Rate limiting | compatibility/03 |
| 26 | ✓ ONNX embedding fallback to FTS5 | compatibility/05 |
| 27 | ✓ Daemon crash detection in MCP adapter | compatibility/05 |
| 28 | ✓ SQLite integrity check on startup | compatibility/05 |
| 31 | ✓ Export/import (JSON and SQL) | compatibility/05 |
| 80 | ✓ ONNX embedding session pooling | schema/06 |

### Codex CLI -- 10/10 done

| # | Task | Source |
|---|------|--------|
| 37 | ✓ OpenAPI spec for Custom GPT Actions | compatibility/06 |
| 38 | ✓ Gemini CLI integration | compatibility/06 |
| 39 | ✓ Local LLM integration | compatibility/06 |
| 42 | ✓ Validate solo mode schema | schema/01 |
| 43 | ✓ Team mode schema with multi-tenancy | schema/01 |
| 44 | ✓ Validate API surface identical in both modes | schema/01 |
| 60 | ✓ Recreate sessions table | schema/03 |
| 61 | ✓ Recreate locks table | schema/03 |
| 62 | ✓ Recreate feed_acks | schema/03 |
| 75 | ✓ Export/import path for user data migration | schema/05 |

### Gemini Flash -- 9/9 done

Schema tasks 45-53 (solo mode tables). All complete.

### Shipped Artifacts (v0.2.0)

`specs/cortex-openapi.yaml`, `examples/gemini-cli/`, `examples/local-llm/`, GitHub Actions release workflow, `CHANGELOG.md`, `cortex export`/`cortex import` CLI, `setup --team` + Argon2id `ctx_` keys.
