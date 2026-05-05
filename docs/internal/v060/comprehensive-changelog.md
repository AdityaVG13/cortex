# Comprehensive Changelog — v0.6.0 (Living)

> **v0.6.0 doc triad — update all three after every commit or commit batch.**
>
> Companion docs (update in lockstep with this file):
> - `docs/internal/v060/unified-status-plan.md` — canonical status + workstream tracker
> - `docs/internal/v060/updates-to-readme.md` — README / `Info/roadmap.md` staging queue
>
> **Commit convention:** every v0.6.0 commit message starts with
> `v0.6.0 - [title]` and includes a meaningful description (what + why + any
> validation run). Log every landed commit here with hash, subject line, and
> touched workstream. Plugin and daemon version bumps must move in lockstep
> per `Info/plugin-lockstep.md`.
>
---


Last updated: 2026-05-05
Baseline: `v0.5.0` (`b9a6458` benchmark-claims correction, 2026-04-23)
C3 evidence head: `1231d58` on `origin/main` (C3 backend + docs/full-suite validation)
Commit range size through C3 evidence: 28 pushed commits on v0.6.0 track (plugin parity + benchmarking infra + H3 + cleanup + G2 + Phase 0 + C5 + plugin MCP hardening + C9 + R2 + RQ1 + daemon stability + storage hygiene + RQ2 + C3 backend/docs)

## Purpose

Track every meaningful change as v0.6.0 develops. Mirror of `docs/internal/archive/pre-v060-docs-2026-04-26.zip::pre-v060/status/comprehensive-changelog.md` (v0.5.0) structure. Append-only. Do not delete history; if a decision reverses, log the reversal with new dated entry.

Every entry must include:
- Date + time (UTC or local-with-tz note)
- Primary files touched (relative to repo root)
- Why the change was made (one or two lines)
- Commit hash (once committed)
- Validation steps run
- Any build/smoke/graphify updates tied to the change

This file is the source of truth for:
- Cutting the public `CHANGELOG.md` v0.6.0 section at release
- Populating `updates-to-readme.md` validation references as work lands
- Audit trail during post-release incident review

---

## 2026-05-05 — C3 budget governance backend

- Commit: `b41f7be` — `v0.6.0 - C3: add budget governance backend`
- Related commits:
  - `efe38a2` - `v0.6.0 - C3: update budget governance handoff docs`
  - `1231d58` - `v0.6.0 - C3: record full-suite validation`
- Why: RQ2 improved local recall but left agent loops unbounded. C3 adds the operator boundary for daemon work before U1 Settings and future bridge/adapters build on top of local memory.
- Backend files touched:
  - `daemon-rs/src/budgets.rs` — new structured parser/status/denial metadata for `~/.cortex/budgets.toml`.
  - `daemon-rs/src/rate_limit.rs` — existing limiter extended with endpoint budget buckets, recent-denial counters, and budget decisions.
  - `daemon-rs/src/state.rs` — loads budget status at startup from resolved Cortex home.
  - `daemon-rs/src/handlers/mod.rs` — shared HTTP 429 helper and `budget_rejected` event logging.
  - `daemon-rs/src/handlers/store.rs`, `recall.rs`, `boot.rs`, `server.rs` — pre-work budget checks for `/store`, recall-family routes, `/boot`, and `/mcp-rpc`.
  - `daemon-rs/src/handlers/health.rs` — `/health.budgets` read contract for Settings.
  - `daemon-rs/src/main.rs` — local admin CLI: `cortex admin budgets status --json` and `validate --path <file> --json`.
- Documentation files touched:
  - `docs/internal/v060/README.md` - index updated with C3 status.
  - `docs/internal/v060/unified-status-plan.md` - status, open-work, validation, and source-catalog updates.
  - `docs/internal/v060/comprehensive-changelog.md` - C3 commit/evidence log.
  - `docs/internal/v060/updates-to-readme.md` - release README queue for budgets.
  - `docs/internal/v060/plans/accessibility-motion-settings.md` - U1 Budgets Settings handoff.
  - `docs/internal/v060/plans/governance-economics.md` - C3 backend result and remaining governance work.
- Behavior:
  - Missing `budgets.toml` remains unlimited/backward-compatible.
  - `defaults.enabled = false` disables enforcement while still validating syntax.
  - Missing endpoint sections are unlimited.
  - Unknown endpoint names or invalid non-positive limits/windows fail visibly/open for availability: enforcement is disabled and health/admin expose a structured error.
  - HTTP denials return `429` with stable `budget_exceeded` JSON and `Retry-After`.
  - MCP denials return JSON-RPC error `-32029` with budget metadata in `error.data`.
  - Denials write `budget_rejected` audit rows when the DB is reachable; health also keeps recent denial counters.
- Validation:
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml budget` -> 36 passed.
  - `rtk cargo check --manifest-path daemon-rs/Cargo.toml --tests` -> clean.
  - `git diff --check` -> clean.
  - `rtk cargo clippy --manifest-path daemon-rs/Cargo.toml --all-targets -- -D warnings` -> clean.
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml` -> 513 passed.
- Release posture:
  - Backend slice is landed and pushed to `origin/main`.
  - Admin CLI shipped for local status/validation.
  - Docs triad and C3 plan handoff are in lockstep through `1231d58`.
  - U1 Settings can consume `/health.budgets` for read-only status, disabled/error states, configured endpoint rows, and recent-denial signals.
  - Budget editing from Control Center remains the next U1 task; no daemon write endpoint shipped.
  - No reranking defaults changed.

---

## Pre-work setup + scope reconciliation

### 2026-04-23 — v0.6.0 planning surface bootstrap

- `docs/internal/v060/` created and gitignored via `.gitignore` entries at parent `docs/internal/` scope (inherited).
- Planning documents written:
  - `docs/internal/v060/README.md` — folder index
  - `docs/internal/v060/scope.md` — Tier 1/2/3 scope draft
  - `docs/internal/v060/open-questions.md` — 7 decisions, #1 and #2 closed
  - `docs/internal/v060/v050-shipped-vs-slipped.md` — v0.5.0 audit proving 5 of 7 public "Foundation Hardening" promises shipped
  - `docs/internal/v060/public-roadmap-update.md` — proposed rewrite of `Info/roadmap.md` + `README.md:195`
  - `docs/internal/v060/accessibility-motion-settings.md` — moved from misfiled `docs/internal/archive/pre-v060-docs-2026-04-26.zip::pre-v060/v050/` location via `git mv`
  - `docs/internal/v060/foundation-carryovers.md` — detailed plans for G2, C5, R2, H3
  - `docs/internal/v060/governance-economics.md` — detailed plans for C9, C3, G5, C4
  - `docs/internal/v060/reranking-harness.md` — v0.6.0 measurement-only scope for R1/R7
  - `docs/internal/v060/unified-status-plan.md` — canonical status tracker (sections 1-10)
  - `docs/internal/v060/updates-to-readme.md` — README staging queue
  - `docs/internal/v060/comprehensive-changelog.md` — this file
- Repo hygiene completed on 2026-04-23 (pre-v0.6.0):
  - `0406b02` untracked `docs/internal/` (7 files) + stale `benchmark/` v0.4.1 dump (8 files); dropped dead link in `README.md`
  - `ca9ea2c` added `.gitignore` patterns for `tmp-browser-harness-*`, `*browser-harness*`, `*browser_harness*`
  - `e64887c` untracked `extensions/cortex-chrome-extension/` (16 files), `tools/sync-security-rules.sh`, `tools/validate_chrome_extension_policy.py`; removed `chrome-extension-validation` CI job; added gitignore patterns
- Commits: `0406b02`, `ca9ea2c`, `e64887c` on `master` (all pushed to `origin/master`)
- Validation: `git push origin master` successful; `git ls-files extensions/ tools/sync-security-rules.sh` empty; `git check-ignore` confirms patterns active
- No code changes yet on v0.6.0 track. Next pending: public roadmap rewrite per `public-roadmap-update.md`.

### 2026-04-23 — v0.6.0 scope lock

- All 7 open questions resolved. See `docs/internal/v060/scope-lock.md` for full rationale.
- Locked decisions:
  - **#3 Scope size:** Tier 1 (accessibility + H3/G2/C5/R2) + Tier 2-essential (C9/C3/G5) = floor. C4 adapter conformance + R1-M reranker harness = stretch (cut first if behind).
  - **#5 Repowise:** Scoped cleanup, ≤ 300 LOC cap, per-file risk annotation, separate commit, weeks 2-3.
  - **#6 Bridge gate:** Spec-only in v0.6.0 (`bridge-track-spec.md`). Zero bridge code. Write week 1.
  - **#7 Target date:** 2026-07-16 (Thursday). 12-week cycle with 1-week buffer. Mandatory checkpoint 2026-06-05.
- Cut-plan triggers locked:
  - Week 4: If Tier 1 accessibility < 50% complete → drop R1-M.
  - Week 6: If Tier 1 + C9 not landed → drop C4.
  - Week 10: If floor not ready → slip date, do not cut floor.
- Files updated at scope lock:
  - `open-questions.md` — all 7 marked resolved
  - `scope.md` — status set to LOCKED, tier structure finalized, execution order mirrored
  - `unified-status-plan.md` — §1 target cut + scope lock banner, §3 added 3D-bridge workstream + 3E Repowise cleanup + renamed 3D→3F, §6 week-by-week execution order
  - `scope-lock.md` — new, canonical decision rationale
  - `comprehensive-changelog.md` — this entry
- No commits (internal-only planning).

### 2026-04-23 — Recursive deep research wave 4: competitive intel + ingest extraction + Mastra + judge reliability

Four more agents launched autonomously. Output:

- `research/agent-memory-production.md` (1430 lines) — 16 products surveyed. **Cursor removed Memories in 2.1 late 2025** (users furious). **Claude Code has most sophisticated shipping memory** (4 layers incl. Auto Dream REM-consolidation). **Copilot Memory default-on 2026-03-04**. Cortex moat confirmed: only Rust+SQLite+sqlite-vec+MCP+HTTP+local-first+cross-tool. Market gaps Cortex must address: auto-write, consolidation loop, multi-scope, citation/provenance UI, git-sync for teams.

- `research/sota-memory-architectures.md` (1587 lines) — **Mastra OM overtaken**: AgentMemory V4 96.20% / Chronos 95.60% / OMEGA 95.4%. **Biggest single lever: ingest-time structured extraction — Chronos ablation shows -34.5pp if removed.** Mastra Observer/Reflector pattern stack-independent and directly copyable to Rust+SQLite. **Mem0's 93.4% disputed — independent measurement 49%.** Supermemory's 99% is pass@8, not production (~85%). Realistic Cortex ceiling with GPT-4o answerer: 94-96%; local Llama 70B: 78-85%.

- `research/ingest-extraction-models.md` (1401 lines) — **3-stage hybrid pipeline locked:**
  - Stage 0: regex fast-path (chrono + dateparser + chrono-english), 0.5-2ms sync
  - Stage 1: GLiNER-multitask-v1.0 via gline-rs (Apache 2.0, 200M, flat latency 130-200ms for 5-50 labels)
  - Stage 2: Qwen2.5-1.5B-Instruct Q4_K_M via llama-cpp-2 with GBNF grammar (zero extra storage — shared with Phase 3.5)
  - Fallback Stage 2: NuExtract-2.0-2B Q4_K_M (986MB, MIT)
  - Confidence: Stage 1 logit-calibrated via isotonic regression; Stage 2 mean logprob per value; skip verbalized (ECE > 0.377)
  - Rollout: v0.6.0 full 3-stage + 300-case eval; v0.6.1 Duckling + alias resolution; v0.7.0 Mem0-style Add/Update/Merge/Delete

- `research/judge-reliability-adversarial-eval.md` (1635 lines) — **CRITICAL: 20-question LongMemEval runs have ±16pp Wilson CI (useless signal)**. Same-family answerer + judge has 5-25% self-preference bias. Triangle judge protocol mandatory (GPT-4o + Claude + local Qwen3-30B). Local judge viable: Qwen3-30B κ=0.813 exceeds human-human κ=0.801. Requirements: N≥100 for any claim, N≥500 for SOTA, pre-registration, 5pp minimum effect size. CAS-100 adversarial suite to author per 15 categories.

Master plan updates landed:
- **`recall-improvement-plan.md` restructured with two NEW promoted phases:**
  - **Phase 3.0 INGEST EXTRACTION** — highest-ROI workstream (biggest single lever per Chronos ablation)
  - **Phase 5.5 Observer/Reflector** — Mastra-style pattern promoted from v0.7.0 stretch to v0.6.1 core
- Targets updated: LongMemEval-S **0.94-0.97 pure in v0.6.0** @ N≥100 with triangle judge
- Phase 0 expanded from 2-3 days → **5-7 days** to absorb N≥100 dataset scale-up + triangle judge + CAS-100 + expanded CI gates
- Tier 2 scope widened from 20-27 days → **28-39 days**
- Honest ceiling narrative: 94-96% only reachable with frontier cloud answerer; local-only ceilings documented
- Mem0 99%+ vendor claims flagged as disputed; purity pledge tightened

### 2026-04-23 — Recursive deep research wave 3: implementation-depth technical research

Four more agents launched autonomously. Output:

- `research/tiny-llm-landscape.md` (1396 lines) — **Phase 3.5 pick: Qwen2.5-1.5B-Instruct Q4_K_M** (Apache 2.0, 994MB, IFEval 42.5). **Phase 6 pick: Qwen3-1.7B Q4_K_M** (1.11GB, thinking/non-thinking mode). **Engine: `llama-cpp-2`** (NOT ORT — `onnxruntime-genai` has no Rust binding). CRITICAL reframe: my 200/500ms latency targets unachievable on laptop CPU; Phase 3.5 must be **async background write-path (≤5s P95)**, not hot-path.

- `research/rerank-models-landscape.md` (1488 lines) — **Phase 2 pick: ms-marco-MiniLM-L-6-v2 int8 ONNX** via fastembed-rs on ort 2.0-rc (22MB Apache 2.0, 74.30 NDCG@10). Fallback: TinyBERT-L-2-v2 int8 (4.3MB). Ruled out: Jina v2/v3 (CC-BY-NC-4.0, MIT-incompatible), bge-reranker-v2-m3 (570MB + 130ms, 3× over budget). **Critical finding: arxiv 2409.07691 proves MiniLM-L-12 can regress strong first-stage retrievers — harness MUST measure this scenario against Cortex's BM25+dense RRF baseline.**

- `research/sqlite-vec-hybrid.md` (1007 lines) — **CRITICAL: sqlite-vec has NO HNSW as of v0.1.9 (2026-03-31)**, only brute-force KNN. Prior plan assumed HNSW — wrong. **Cortex currently on sqlite-vec v0.1.6** — must upgrade to ≥v0.1.7 for DELETE support before production flip. **Current route default `Primary` reorders candidates but does NOT generate them.** Production flip needs persistent `vec_memories` vec0 table + migration 015 backfill. Tighten gate thresholds 0.60/0.45 → 0.92/0.95.

- `research/memory-graph-algorithms.md` (1536 lines) — **Phase 5 PPR confirmed: deterministic power iteration α=0.5, tol=1e-4, max_iters=30, CSR at query time. ~290 LOC new `cortex-graph` crate. 65ms cache-miss / 3ms cache-hit on 50K nodes / 500K edges (well under 100ms p95 budget).** Petgraph has `page_rank` but NO personalized teleport — write from scratch. HippoRAG 2 canonical: igraph `personalized_pagerank(damping=0.5, passage_node_weight=0.05)`. **Cortex replaces HippoRAG's LLM filter with cross-encoder triple rerank** — LLM-free hot path.

Master plan updates landed:
- `recall-improvement-plan.md` — Phase 1.5 added (sqlite-vec production promotion, ~3-5 days latency-only win). Phase 2 model picks locked. Phase 3.5 reframed as async background. Phase 5 PPR details concrete (power iteration, ~290 LOC, LRU cache, seed weights 0.35/0.35/0.05).
- v0.6.0 Tier 2 scope extended from 17-22 days → 20-27 days to absorb Phase 1.5.

**README.md + CHANGELOG.md benchmark stats corrected and pushed in commit b9a6458** (per user directive — only benchmark stats, no other copy touched).

### 2026-04-23 — Recursive deep research wave 2: multi-hop retrieval + second-pass adapter audit

Two more agents launched autonomously after wave 1 completed. Output:
- `research/multihop-retrieval.md` (1563 lines, 18 appendices) — MemoryAgentBench CR-MH deep dive. **Critical correction:** CR = Conflict Resolution, not Cross-Referencing. Multi-hop CR ceiling is ~6-7%, every paradigm collapses from single-hop CR 60%. Failure is both retrieval AND reasoning.
- `research/helper-audit-second-pass.md` (~500 lines) — surfaces 8th helper (LongMemEval prompt-injection `run_amb_cortex.py:302-365`, +2-3pp). Confirms daemon code is VERIFIED CLEAN. Pure baseline revised 0.80-0.85 → 0.82-0.84.

Master plan updates landed:
- `recall-improvement-plan.md` — new Phase 3.5 (contextual prefixing promoted to v0.6.0 stretch), Phase 5 refined (HippoRAG PPR + cross-encoder triple rerank, NOT LLM filter, ~1670 LOC over 2-3 weeks), NEW Phase 6 (IRCoT-style 2-hop iterative retrieval, ~640 LOC, 1 week), updated rejected list (HopRAG, PropRAG, long-context substitute, MDR/Beam), new CR-MH target: **18-26% vs current SOTA 6% (3×)**.
- `benchmark-purity-audit.md` — 8th helper section added, score estimates revised.
- `phase-0-purity-execution.md` — `CORTEX_LONGMEMEVAL_*` added to forbidden env var prefixes.

All daemon code remains verified clean. All inflation lives in Python adapters + harness.

### 2026-04-23 — Memory systems + benchmark purity + retrieval SOTA deep research

- Four parallel research agents ran, ~10-25min each, outputs written to `docs/internal/v060/research/`:
  - `benchmark-purity-audit.md` (~500 lines) — **CRITICAL:** `cortex-http-base` ("raw") adapter still runs 4 of 7 major helpers; CHANGELOG claim of "20/20 both" is wrong; raw is actually 0.875, pure core likely 0.80-0.85
  - `memory-systems-survey.md` (1065 lines) — 11 systems surveyed (Letta/MemGPT, A-MEM, MAGMA, Zep/Graphiti, Mem0, HippoRAG 1+2, LangMem, Hindsight, Cognee, ENGRAM, ChatGPT/Claude platform memory); Cortex's moat = only SQLite+Rust+local-first+multi-agent+MCP-first
  - `retrieval-sota.md` (2037 lines, 167 citations) — 2025-2026 SOTA landscape across embeddings, reranking, fusion, expansion, graph retrieval; biggest single lever = bge-base-en-v1.5 (+5-8 MTEB); cross-encoder rerank = non-negotiable (Anthropic 67% failure-rate reduction)
  - `memory-benchmarks-landscape.md` (949 lines, 52KB) — LongMemEval "largely solved by simple RAG", MemoryAgentBench multi-hop CR at 7% ceiling is real frontier; canonical scoreboard of 4 benchmarks; cost estimates ($60-100 per-release with GPT-5-nano, $300-500 with GPT-4o)
- Consolidated synthesis written to `docs/internal/v060/recall-improvement-plan.md` (~1200 lines):
  - 6 phases (0 Purity, 1 Embedding, 2 Reranker, 3 Temporal+Link, 4 Adaptive+Context, 5 Associative+Reflect)
  - Phase 0 blocking — pure baseline must be locked before any recall-quality work
  - Top 3 adopt (ranked by ROI): bge-base embedding upgrade, cross-encoder reranker, bi-temporal validity intervals
  - Top 3 reject: SPLADE (license), GraphRAG (cost), full MAGMA (complexity vs gain)
  - Realistic target: LongMemEval-S **0.88-0.92 pure** in v0.6.0, **0.92-0.95 pure** in v0.6.1
  - 5 CI purity-gate scripts + CODEOWNERS protection + reproducibility policy
- Scope-lock amendment recommended: promote Phases 0-2 (purity + embedding + reranker) from Tier 3 stretch to Tier 2-essential. Awaiting user approval before amending `scope-lock.md`.
- Key reality check: v0.5.0 CHANGELOG line 45-46 must be corrected — "Both tuned and raw reach 20/20" is inaccurate. Raw is 0.875. Fix lands in Phase 0 CHANGELOG update.

### 2026-04-23 — A11y deep research + master plan

- Four parallel research agents ran ~10min each, output written to `docs/internal/v060/research/`:
  - `a11y-codebase-audit.md` (413 lines) — Cortex Control Center current state, file:line citations for every gap
  - `a11y-wcag22-research.md` (1303 lines, 40+ sources 2026-04-23) — WCAG 2.2 vs 2.1, axe-core coverage math (~57%), testing matrix, APCA status
  - `a11y-motion-research.md` (743 lines, 50+ sources) — motion token surveys (Cloudscape/Fluent/Primer/MD3/GOV.UK/USWDS), library comparison, recommended stack
  - `a11y-react-libs.md` (851 lines, 60KB) — Radix vs React Aria vs Headless UI vs Ariakit vs Base UI vs Reach, focus/toast/combobox deep dive, bundle math
- Research synthesized into `accessibility-motion-settings.md` master plan (rewritten from original 140L draft to full sprint-level plan, ~1000 lines):
  - 5 sprints A-E, 180 hrs total over 8 weeks
  - Stack locked: Radix primitives + react-focus-lock + Sonner + custom useAnnouncer + RHF + Downshift + CSS tokens + auto-animate + motion/react + vitest-axe + @axe-core/playwright
  - 9 WCAG 2.2 AA new SCs mapped with per-SC plan
  - File touch map: 30 new files + 8 modified (including App.jsx, BrainVisualizer.jsx, styles.css, ci.yml)
  - Bundle impact: ~45kB gzipped total
  - Rejected libraries documented (Reach, react-modal, React Aria Components, Headless UI, Base UI v1, focus-trap-react, APCA, JAWS) with reasons
  - 9 risks with mitigations
  - 11 explicit non-goals
- Current state measured: Cortex Control Center at ~20% WCAG 2.2 AA today. Key gaps: 8+ div-onClick, 5 outline:none, 3 unfixed dialogs, 9 unlabeled inputs, 24+ unresponsive animations, 0 a11y tests.
- Target state at v0.6.0 cut: WCAG 2.2 AA compliance, keyboard 100%, 3 screen readers verified, Settings panel with 4 sections, token-driven motion, CI a11y gate.

### 2026-04-23 — v0.6.0 scope-locked planning surface complete

- Full planning surface landed in `docs/internal/v060/`:
  - 13 files, ~115KB total
  - Matches or exceeds v0.5.0 trio depth (`docs/internal/archive/pre-v060-docs-2026-04-26.zip::pre-v060/status/CORTEX-UNIFIED-STATUS-PLAN.md` + `docs/internal/archive/pre-v060-docs-2026-04-26.zip::pre-v060/status/updates-to-readme.md` + `docs/internal/archive/pre-v060-docs-2026-04-26.zip::pre-v060/status/comprehensive-changelog.md`)
  - Every workstream has file-level plans, acceptance gates, effort estimates, validation commands
- Next: week 1 execution — public roadmap rewrite + bridge spec + H3 cleanup.

---

## Milestones expected during v0.6.0 development

The entries below are **placeholders** — fill in as work lands. Keep placeholder structure so the order of events stays documented even before commits exist.

### 0A) Public roadmap rewrite (2026-04-25)

**Files touched:**
- `Info/roadmap.md` — added 6 v0.5.0 shipped bullets (TTL/G1, migrations/C1, repair CLIs/C2, semantic dedup/C6, recall feedback/R9, embedding profiles/R8); removed Chrome extension bullet (extension untracked, not yet public); rewrote v0.6.0 from "Foundation Hardening" → "Accessibility, Governance & Recall Quality" with 7 themes (Accessibility & Settings, Motion, Recall quality Phase 0/1/2, Budget governance, Retention classes, Context ranking, Foundation carryovers); rewrote v0.7.0 from "Governance & Economics" → "Multi-Tenant Hardening" absorbing privacy/auth/fairness/isolation + HyDE query expansion + adapter conformance + first read-only bridge against the v0.6.0 acceptance gate spec; renumbered v0.9.0 "Advanced Agent Support" → v0.8.0 (single milestone consolidation); v1.0.0 unchanged.
- `README.md` — line 195 already current ("Reranking production-ships in v0.6.0 Phase 2; query expansion (HyDE) targeted for v0.7.0"); no edit required.

**Why:** Public roadmap was stale — five "Foundation Hardening" v0.6.0 promises actually shipped in v0.5.0 per `v050-shipped-vs-slipped.md`. Reconciled with the 2026-04-24 scope-lock amendment that promoted recall Phases 0-2 (purity / bge / reranker) into the v0.6.0 Tier 2-essential floor, so reranker now ships in v0.6.0 (not v0.7.0 as the original `public-roadmap-update.md` draft assumed).

**Validation:**
- `grep -n "Foundation Hardening" Info/roadmap.md` — empty.
- `grep -n "v0.6.0+" README.md` — empty.
- v0.5.0 bullets cover all six items from `v050-shipped-vs-slipped.md` "Shipped from public v0.6.0 roadmap" table.
- v0.6.0 themes match `unified-status-plan.md` §3 workstreams.

**Commit:** *(this batch)*

---

### 1A) H3 — `DEFAULT_CORTEX_PORT` consolidation

**Expected files:**
- `daemon-rs/src/lib.rs` — new `pub const DEFAULT_CORTEX_PORT: u16 = 7437;`
- `daemon-rs/src/auth.rs:64` — read from const
- `daemon-rs/src/daemon_lifecycle.rs` — prod paths only (test fixtures keep literals deliberately)
- `desktop/cortex-control-center/src/App.jsx:61,293` — read from shared config module
- `tauri.conf.json` — annotate CSP strings with comment reference
- `README.md`, `Info/connecting.md` — one-liner "examples assume default port 7437" note

**Planned validation:**
- `grep -rn "7437" daemon-rs/src/ desktop/cortex-control-center/src/` — shows only literals in test files and annotated CSP strings
- `cortex start --port 9999` end-to-end read/write/boot smoke
- `cargo test --manifest-path daemon-rs/Cargo.toml` (baseline: 255 passing from v0.5.0)
- `cargo clippy --manifest-path daemon-rs/Cargo.toml --all-targets -- -D warnings`
- `npm --prefix desktop/cortex-control-center test`

**Commit:** *(pending)*

---

### 1B) G2 — `cortex admin rollback` CLI

**Expected files:**
- `daemon-rs/src/main.rs` — new CLI subcommand branch near existing `reindex` path (~line 613)
- `daemon-rs/src/admin.rs` *(new)* — rollback logic, emits `session_rolled_back` event with affected ID list
- `daemon-rs/tests/admin_rollback.rs` *(new)* — fixture-based test: seed session → rollback → assert row counts + event emission + idempotency
- `specs/cortex-openapi.yaml` — new `/admin/rollback` endpoint if HTTP surface added
- `daemon-rs/src/db.rs` — no schema change (reuses `trace_id` + `status` columns shipped in v0.5.0)

**Planned validation:**
- `cargo test --test admin_rollback` green
- Dry-run on non-existent session → "0 affected" cleanly
- Commit on existing session flips rows in 4 tables + emits event
- `status='rolled_back'` rows excluded from default `/recall`
- `--json` output matches OpenAPI spec entry
- Idempotency: rollback twice → second pass no-op

**Commit:** *(pending)*

---

### 1C) C5 — Boot prompt audit trail

**Expected files:**
- `daemon-rs/src/db.rs` — migration 008 creating `boot_audits` table with columns: `id`, `session_id`, `agent`, `timestamp`, `token_budget`, `tokens_used`, `sources_json`, `decisions_json`, `recall_latency_ms`, `expires_at`
- `daemon-rs/src/prompt_inject.rs` — write one audit row on every boot assembly
- `daemon-rs/src/handlers/boot.rs` — new `GET /boot/audit?session_id=X` endpoint
- `daemon-rs/src/handlers/mcp.rs` — new `cortex_boot_audit` MCP tool
- `specs/cortex-openapi.yaml` — `/boot/audit` entry
- `daemon-rs/src/compaction.rs` — 30-day auto-prune on existing cleanup loop (reuses `expires_at` machinery from G1)

**Planned validation:**
- Migration 008 applies cleanly on v0.5.0 database fixture (use existing `state::initialize` regression pattern from v0.5.0)
- Every boot call writes exactly one `boot_audits` row
- `GET /boot/audit` returns well-formed JSON matching spec
- MCP tool tested in `tests/mcp_rpc_headers.rs` style
- Storage overhead: < 1MB per 1000 boots on realistic workload (measured explicitly, recorded here)
- 30-day auto-prune runs in existing compaction loop (integration test)

**Commit:** *(pending)*

---

### 1D) R2 — Score-adaptive truncation for boot (prereq: C5)

**Landed files (`a547a07`):**
- `daemon-rs/src/compiler.rs` — actual boot assembly surface (the earlier `prompt_inject.rs` note was stale; it only fetches `/boot`). Extracted the legacy greedy packer as the flat-score fallback, added score-adaptive token allocation by capsule priority, and exposed `CORTEX_BOOT_MIN_SOURCE_TOKENS` / `CORTEX_BOOT_MAX_SOURCE_TOKENS`.
- C5 boot audit linkage is implicit: `/boot` already serializes `BootResult.capsules` into `boot_audits.capsules_json`; R2 now includes `packing`, `allocatedTokens`, and `truncated` metadata in admitted capsules.

**Validation:**
- `$env:CARGO_TARGET_DIR='target-codex-r2'; rtk cargo check --manifest-path daemon-rs/Cargo.toml --tests` -> clean.
- `$env:CARGO_TARGET_DIR='target-codex-r2'; rtk cargo test --manifest-path daemon-rs/Cargo.toml score_adaptive -- --nocapture` -> 2 passed.
- `$env:CARGO_TARGET_DIR='target-codex-r2'; rtk cargo test --manifest-path daemon-rs/Cargo.toml flat_score_fallback_matches_legacy_greedy_packing -- --nocapture` -> 1 passed.
- `$env:CARGO_TARGET_DIR='target-codex-r2'; rtk cargo clippy --manifest-path daemon-rs/Cargo.toml --tests -- -D warnings` -> clean.
- Full daemon suite: `$env:CARGO_TARGET_DIR='target-codex-r2'; rtk cargo test --manifest-path daemon-rs/Cargo.toml -- --nocapture` -> 469 passed.
- Not separately measured in this pass: p50 boot latency and v0.5.0 GT-precision delta. Code-level gates and full regression suite are green.

**Commit:** `a547a07`

---

### 2A-Z) Accessibility + Settings + Motion

Each sub-task gets its own entry as it lands. High-level milestones expected:

- **2A** — Settings panel skeleton + navigation entry (new `desktop/cortex-control-center/src/settings/` tree)
- **2B** — Keyboard-only operation across all components
- **2C** — ARIA pass: dialogs, tablists, live regions
- **2D** — Reduced-motion runtime plumbing (not just config flag)
- **2E** — Contrast + non-text contrast compliance pass
- **2F** — Zoom + reflow to `375×812`
- **2G** — Sidebar collapsed-width animation unification
- **2H** — Panel/tab transition system
- **2I** — Central motion timing + easing tokens (`motion.js`)
- **2J** — axe-core integration in desktop build
- **2K** — Manual walkthrough: NVDA+Firefox
- **2L** — Manual walkthrough: VoiceOver+Safari
- **2M** — Manual walkthrough: Narrator+Edge
- **2N** — Walkthrough reports archived to `docs/internal/v060/a11y-walkthroughs/`

**Planned validation per sub-task:**
- `npm --prefix desktop/cortex-control-center test` green
- `npm --prefix desktop/cortex-control-center run build` succeeds
- axe-core report included in PR
- Screenshot + keyboard trace where relevant

**Commits:** *(pending)*

---

### 3A) C9 — Retention policy classes

**Landed files (`bd85025`):**
- `daemon-rs/src/db.rs` — migration 016 adds `retention_class TEXT NOT NULL DEFAULT 'operational'` to `memories` and `decisions`; fresh schema includes enum `CHECK`; migration normalizes invalid/empty existing values to `operational`; indexes class columns.
- `daemon-rs/src/api_types.rs` — `RetentionClass` enum (`durable`, `operational`, `audit`, `ephemeral`) with serde, parser, default TTLs, type mapping, and text heuristics.
- `daemon-rs/src/handlers/store.rs` — classifier invoked before decision insert: (1) explicit caller param, (2) decision `type` mapping, (3) text heuristic, (4) fallback. Default class TTLs feed existing `expires_at` cleanup loop; explicit `ttl_seconds` still overrides class TTL.
- `daemon-rs/src/handlers/mcp.rs` — `cortex_store` accepts `retention_class` / `retentionClass`, advertises schema enum, rejects invalid values, and returns the stored class.
- `daemon-rs/src/export_data.rs` — JSON changesets, SQL export, and import payloads preserve `retention_class` for both memories and decisions.
- `specs/cortex-openapi.yaml` — `RetentionClass` component plus store/import fields.

**Validation:**
- `$env:CARGO_TARGET_DIR='target-codex-c9'; rtk cargo check --manifest-path daemon-rs/Cargo.toml --tests` -> clean.
- `$env:CARGO_TARGET_DIR='target-codex-c9'; rtk cargo test --manifest-path daemon-rs/Cargo.toml store_decision_ -- --nocapture` -> 5 passed.
- `$env:CARGO_TARGET_DIR='target-codex-c9'; rtk cargo test --manifest-path daemon-rs/Cargo.toml import_payload_normalizes_types_and_preserves_temporal_fields -- --nocapture` -> 1 passed.
- `$env:CARGO_TARGET_DIR='target-codex-c9'; rtk cargo test --manifest-path daemon-rs/Cargo.toml test_run_pending_migrations_applies_all_once -- --nocapture` -> 1 passed.
- `$env:CARGO_TARGET_DIR='target-codex-c9'; rtk cargo clippy --manifest-path daemon-rs/Cargo.toml --tests -- -D warnings` -> clean.
- `$env:CARGO_TARGET_DIR='target-codex-c9'; rtk cargo test --manifest-path daemon-rs/Cargo.toml -- --nocapture` -> 465 passed.

**Commit:** `bd85025`

---

### 3B) C3 — Budget governance

**Expected files:**
- `daemon-rs/src/config.rs` — `budgets.toml` loader (new `BudgetConfig` struct)
- `daemon-rs/src/rate_limit.rs` — extend `AgentBucket` with per-endpoint tiers (recall/boot/store/global)
- `daemon-rs/src/handlers/recall.rs`, `handlers/boot.rs`, `handlers/store.rs` — inject budget check before expensive work
- `daemon-rs/src/handlers/mod.rs` — 429 response with machine-readable `Retry-After` + JSON `{reason, current, limit, window}`
- `daemon-rs/src/events.rs` — new `budget_rejected` event kind (counts toward `audit` retention class)
- `desktop/cortex-control-center/src/settings/budgets.jsx` *(new)* — Settings UI sub-panel
- `daemon-rs/tests/budget_enforcement.rs` *(new)* — 429 path per endpoint, unlimited-default test, load test scaffold

**Planned validation:**
- 429 tested for each of recall/boot/store
- Unlimited-budget default (backward compatible) when `budgets.toml` absent
- Settings UI round-trips values + persists across app restart
- Rejection events surface in Feed with correct `kind`
- Load test: sustained 100 rps on `/recall` with cap=60/m → expected ~40% rejection rate, no latency collapse (quantitative: record p50/p95 during and after)
- `cargo test` suite green

**Commit:** *(pending)*

---

### 3C) G5 — Dynamic context ranking (prereq: C9 + C5)

**Expected files:**
- `daemon-rs/src/prompt_inject.rs` — new `rank_candidates()` pass before existing truncation path
  - `score = w₁·class_weight + w₂·recency_score + w₃·relevance_score + w₄·activity_score`
  - Top-N (default N=5) selected; rest shelved
  - N configurable per agent via Settings
- `daemon-rs/src/config.rs` — weight constants (initial values from research, tunable via env)
- `daemon-rs/tests/injector_ranking.rs` *(new)*

**Planned validation:**
- Unit tests per component (class, recency, activity)
- Regression: durable with 0 recent activity ranks below operational touched 10min ago
- Boot latency ±5% of v0.5.0
- C5 boot audit records rank components for each injected source

**Commit:** *(pending)*

---

### 3D) C4 — Adapter conformance spec + shared tests

**Expected files:**
- `specs/cortex-adapter-contract.yaml` *(new)* — machine-readable scenario spec
- `daemon-rs/tests/adapter_conformance.rs` *(new)* — Rust runner that boots daemon + exercises MCP + HTTP
- `sdks/python/tests/test_conformance.py` *(new)*
- `sdks/ts/tests/conformance.test.ts` *(new)*
- `.github/workflows/ci.yml` — new job `adapter-conformance-matrix`

**Planned validation:**
- ≥ 10 scenarios (store / recall / boot / forget / permissions / events / resolve / peek / unfold / feedback)
- CI job green on MCP + HTTP + Python SDK + TS SDK
- First intentional violation produces clean diff in CI output
- Spec version tagged with daemon semver → drift is a CI failure

**Commit:** *(pending)*

---

### 4A) R1-M — Reranker measurement harness

**Expected files:**
- `daemon-rs/src/rerank/mod.rs` *(new)* — `Reranker` trait, `RerankCandidate`/`RerankedScore` structs
- `daemon-rs/src/rerank/adapters/noop.rs` *(new)* — passthrough control arm
- `daemon-rs/src/rerank/adapters/cross_encoder_minilm.rs` *(new)* — `cross-encoder/ms-marco-MiniLM-L-6-v2` via ORT
- `daemon-rs/src/rerank/adapters/cross_encoder_tinybert.rs` *(new)* — `cross-encoder/ms-marco-TinyBERT-L-2-v2`
- `daemon-rs/src/rerank/adapters/colbert_v2.rs` *(new)* — distilled ColBERTv2 via ORT
- `daemon-rs/src/main.rs` — new `cortex eval rerank --adapter X --fixture Y` CLI branch
- `benchmarking/rerank/run_harness.py` *(new)* — parallel to `run_amb_cortex.py`
- `benchmarking/rerank/report.py` *(new)* — comparison markdown generator
- `benchmarking/results/rerank-<YYYYMMDD>.json` — first measurement pass artifact

**Planned validation:**
- Harness runs green on Win/Mac/Linux CI matrix (`cargo test` + smoke on each platform)
- At least one full measurement pass committed to `benchmarking/results/`
- Report markdown committed alongside JSON
- Model download via checksum-verified HTTP (no surprise footprint in plugin bundle)
- Go/no-go writeup in `docs/internal/v060/rerank-findings.md`
- If go: v0.7.0 rerank ship confirmed in `Info/roadmap.md`
- If no-go: `README.md:195` updated, v0.7.0 rerank line dropped

**Commit:** *(pending)*

---

### 4B) R1-S — Shadow rerank telemetry (optional stretch)

**Expected files:**
- `daemon-rs/src/handlers/recall.rs` — emit `rerank_shadow` key in `/recall/explain` response when `CORTEX_RERANK_SHADOW=<adapter>` env set
- `daemon-rs/src/events.rs` — optional shadow rerank signal on unified recall telemetry events

**Planned validation:**
- Zero impact on production scores (production path untouched)
- Mirrors sqlite-vec shadow pattern shipped in v0.5.0
- Shadow status cleanly separates environment capability from probe faults
- `cargo test` + `clippy` green

**Commit:** *(pending)*

---

## Release-cut entries (expected at release)

### 5A) Version bumps

- `daemon-rs/Cargo.toml`: `cortex-daemon` version `0.5.0` → `0.6.0`
- `plugins/cortex-plugin/.claude-plugin/plugin.json`: version `0.5.0` → `0.6.0`
- `.claude-plugin/marketplace.json`: version `0.5.0` → `0.6.0`
- `plugins/cortex-plugin/scripts/prepare-runtime.cjs:43`: default `'0.5.0'` → `'0.6.0'`
- `specs/cortex-openapi.yaml`: version `0.5.0` → `0.6.0`
- `package.json` + `package-lock.json` root metadata aligned to `0.6.0`
- `desktop/cortex-control-center/package.json` + lock aligned

Planned validation:
- Plugin lockstep check per `plugins/cortex-plugin/ROUTING.md:60-65`
- Daemon tests that guard spec-version/path drift pass

**Commits:** *(pending)*

---

### 5B) CHANGELOG.md v0.6.0 section

Pulled from this comprehensive-changelog at release time. Sections: `Added`, `Changed`, `Fixed`, `Performance`, `Security`, `Desktop`, `Documentation`, `CI`. Ref URL added:

```
[0.6.0]: https://github.com/AdityaVG13/cortex/compare/v0.5.0...v0.6.0
```

**Commit:** *(pending)*

---

### 5C) Public roadmap — move v0.6.0 → `shipped`, expose v0.7.0 → `next`

- `Info/roadmap.md` — move v0.6.0 section to `shipped`, expose v0.7.0 as `next`
- Update contributor task lists per what actually shipped

**Commit:** *(pending)*

---

### 5D) Graphify + screenshot refresh

- `graphify update .` full refresh; record new node/edge/community counts here
- Re-capture Control Center screenshots including new Settings panel at `1920×1080`
- Drop into `C:\Users\aditya\Desktop\Claude\Cortex GitHub Pictures\v060\`
- Update `README.md` asset references if layout changed

**Commits:** *(pending)*

---

### 5E) Final release verification

- All Gate and Verification Matrix items in `unified-status-plan.md` §7 pass
- `cargo audit` + `npm audit` both clean (root + desktop)
- Hardcoded developer-path scan clean
- `.env` tracking policy check clean
- Benchmark visibility policy check still enforced (`benchmarking/results` public, `benchmarking/runs` ignored)
- Release bundle build: MSI + NSIS + .dmg + .AppImage + .deb
- Updater signing with canonical `TAURI_SIGNING_PRIVATE_KEY` (not ad-hoc)
- Release smoke test: store/recall/boot round-trip against release binary on each platform
- Tag `v0.6.0` + GitHub release with bundled artifacts

**Commits:** *(pending)*

---

## 2026-04-24 — Autonomous continuation session

Single-session ~14-call autonomous pass cleared all 7 pending items from 2026-04-23 SESSION-HANDOFF plus added 4 follow-up planning artifacts.

### Planning docs authored

- `docs/internal/v060/research/daemon-bloat-compression.md` — 14-section bloat audit + compression strategy; live DB measurement 387 MB @ 286 memories; committed target 100 MB @ 5K memories; migration sequence 017/018/019 designed.
- `docs/internal/v060/phase-3-0-ingest-extraction-execution.md` — 10-day code-level guide; 3-stage pipeline (regex + GLiNER via gline-rs + Qwen2.5-1.5B GBNF); migration 013; tokio mpsc worker + pipeline orchestration; 4 e2e tests.
- `docs/internal/v060/phase-3-5-execution.md` — 3-5 day guide; async contextual prefix worker reusing shared Qwen model; migration 015; re-embed signal; backfill CLI; 6 e2e tests.
- `docs/internal/v060/phase-5-hipporag-ppr-execution.md` — 10-15 day guide; new `cortex-graph` workspace crate (~290 LOC); power iteration α=0.5; RRF fusion; triple rerank via Phase 2 model; CR-MH target 18-26%.
- `docs/internal/v060/phase-5-5-observer-reflector-execution.md` — 4-6 day guide; Mastra-style Observer/Reflector workers; migration 016; emoji priority parser; boot injection wiring; 5 e2e tests.
- `docs/internal/v060/phase-6-execution.md` — 5-7 day guide; Qwen3-1.7B thinking-mode decomposer + synthesizer; `/answer` endpoint + `cortex_answer` MCP tool; complexity gate; 6 e2e tests.
- `docs/internal/v060/phase-4-adaptive-k-execution.md` — 2-3 day guide; adaptive-k policy + time-aware expansion; 4 e2e tests.
- `docs/internal/v060/observer-reflector-prompts.md` — primary prompts + 4 A/B variants + 18 eval fixtures; calibration protocol.
- `docs/internal/v060/db-stats-cli-spec.md` — JSON schema v1 + table format for `cortex admin db-stats`; CI gate integration; Control Center forward compat.

### Benchmarking infra authored

- `benchmarking/adversarial/cas-100.jsonl` — 100-item adversarial suite across 15 categories (paraphrase, typo, code-reference, semantic-drift, distractor-heavy, multi-hop-distractor, temporal-trick, temporal-anchor, negation, ambiguity, near-duplicate, rolled-back, conflict, abstention-missing, abstention-wrong-domain); Wilson CI ±7pp at N=100.
- `benchmarking/adversarial/cas-100.spec.md` — suite format, authoring protocol, run instructions, purity pledge.
- `benchmarking/judges/triangle.py` — 3-judge async runner (OpenAI + Anthropic + Ollama/local Qwen3-30B); pairwise Cohen's κ + Fleiss' κ; cross-family enforcement via `--answerer-family`; kappa math smoke-tested.

### Scope-lock amendment applied (2026-04-24, user-approved)

- `scope-lock.md` — Question #3 rewritten. Tier 2-essential floor expanded to 19-24 days to include recall Phases 0-2.
- `scope.md` — Tier 2 list items 8/9/10 (Phase 0/1/2) added; Tier 3 stretch items 11/12/13 (C4 + Phase 3.0 + Phase 3) added.
- `unified-status-plan.md` — new §3G workstream (RQ0/RQ1/RQ2); §3F retitled Tier 3 stretch; R1-M superseded by Phase 2; C4 demoted; RQ3.0 + RQ3 added. Duplicate §3D-bridge header fixed.
- `recall-improvement-plan.md §15` — marked APPROVED.
- `open-questions.md` — Q3 amended; Q4 revised from "measurement only" to "production rerank in Phase 2"; 4 new follow-up questions added (Q8-Q11).
- `SESSION-HANDOFF.md` — autonomous continuation status block prepended.

### Cortex memory stores

- id 1082182: v0.6.1 bloat target decision (100 MB @ 5K memories via migrations 017/018/019)
- id 1082183: Phase 5.5 Observer/Reflector design (Mastra-copy, shared Qwen, emoji priority column)
- id 1082184: Scope-lock amendment applied (recall Phase 0-2 promoted; public roadmap + Phase 0 exec deferred)

### Net artifact counts after 2026-04-24

- Execution guides: 8 (Phase 0/1/2 prior + Phase 3.0/3.5/4/5/5.5/6 new)
- Research briefs: 15 (+ daemon-bloat-compression)
- Prompt + CLI spec artifacts: 2 (observer-reflector-prompts, db-stats-cli-spec)
- Benchmarking infra: CAS-100 suite + triangle judge
- Planning folder size: ~1.4 MB → ~2.1 MB

### Validation on planning artifacts

- `triangle.py --help` smoke: passes
- Kappa math self-test: perfect agreement → 1.0, anti-correlated → -1.0, partial → 0.74 (matches Cohen 1960 expected values)
- CAS-100 JSONL format: each line valid JSON with required schema fields (inspection)
- Live DB audit via python sqlite3: 387 MB / 286 memories / 10560 embeddings / 23081 co_occurrence rows confirmed

---

## 2026-04-24 — Plugin/daemon parity + benchmarking infra (6 commits)

First code-landing commits on v0.6.0 track. Plugin routing hardened, lockstep guard added, CAS-100 adversarial suite + triangle judge shipped to `benchmarking/`.

### Commits pushed (origin/master → `323c5cf`)

- **`c2ba28d`** v0.6.0 - plugin routing: dev-prefer-app policy + explicit failure modes
  - `plugins/cortex-plugin/scripts/run-mcp.cjs` — rewrites `resolveRoute` with documented 4-level priority (explicit URL > dev-prefer-app > app URL > local), adds `spawnAllowed` boolean to every route, handles `fail` mode before binary resolver. Policy flags: `CORTEX_DEV_PREFER_APP`, `CORTEX_DEV_DISABLE_LOCAL_SPAWN`, `CORTEX_PLUGIN_ALLOW_LOCAL_SPAWN`. Dry-run honors success + failure paths.
  - `plugins/cortex-plugin/scripts/dry-run-matrix.cjs` (new) — 8-case assertion suite (5 spec-required + 3 corner). All green locally.
  - Validation: `node --check` on both files; matrix prints `8/8 passed`.

- **`9632b6a`** v0.6.0 - plugin hook-boot: route-aware status + backward-compat health
  - `plugins/cortex-plugin/scripts/hook-boot.cjs` — mirrors run-mcp routing with UI-facing mode names (`team`/`app`/`solo`/`solo-standby`). New `solo-standby` mode surfaces policy blocks without probing a missing daemon. `isCortexReadinessResponse` relaxed to accept `status+stats` without `runtime` (pre-v0.5.0 payload). `validateHealthIdentity` soft-accepts when no path fields expected + runtime block absent.
  - Status line now names the selected route mode and a resolvable next-step hint.

- **`0c6bb62`** v0.6.0 - plugin/daemon lockstep version guard + CI hook doc
  - `scripts/check-plugin-lockstep.cjs` (new) — compares `Cargo.toml` [package] version against `plugin.json` version. Exit 1 on mismatch, exit 0 + warn on `prepare-runtime.cjs` hard-coded fallback drift.
  - Smoke: `daemon=0.5.0 plugin=0.5.0 PASS`.

- **`d50744c`** v0.6.0 - docs(plugin): lockstep guard usage under Info/
  - `Info/plugin-lockstep.md` — guard script usage, CI hook placement, local bump workflow, future-automation TODO. Originally written to `docs/` but that path is gitignored; relocated to `Info/`.

- **`f625614`** v0.6.0 - benchmarking: CAS-100 adversarial suite (100 items, 15 categories)
  - `benchmarking/adversarial/cas-100.spec.md` + `cas-100.jsonl` — suite now public (previously untracked in planning pass).
  - 30/45/25 tier split. ≥6 items per category. Wilson CI ±7pp at N=100. Purity-pledge compliant (gold answers judge-only).

- **`323c5cf`** v0.6.0 - benchmarking: triangle judge (GPT-4o + Claude + local Qwen3-30B)
  - `benchmarking/judges/triangle.py` (new, 600 LOC) — async cross-family runner. Pairwise Cohen's κ + Fleiss' κ + consensus. `--answerer-family` overlap refuses to run. Kappa math smoke-tested: perfect → 1.0, anti-correlated → -1.0, partial → 0.7368.

### Validation

- `node --check` green on all 4 changed/new JS files.
- `node plugins/cortex-plugin/scripts/dry-run-matrix.cjs` → 8/8 pass.
- `node scripts/check-plugin-lockstep.cjs` → PASS.
- `python benchmarking/judges/triangle.py --help` → clean arg parser output.
- Triangle kappa self-test on inline toy data → math correct.
- Grep for `RunAs|ShellExecute|elevated|UAC|sudo|-Verb RunAs` across plugin scripts + new scripts → zero matches. No elevation APIs introduced.

### Behavior matrix (plugin routing)

| env / config | route mode | spawn allowed | url |
|---|---|---|---|
| `CLAUDE_PLUGIN_OPTION_CORTEX_URL=...` | remote | false | explicit |
| `CORTEX_DEV_PREFER_APP=1 + CORTEX_DEV_APP_URL=...` | remote | false | dev app |
| `CORTEX_DEV_PREFER_APP=1 + CORTEX_APP_URL=...` | remote | false | app fallback |
| `CORTEX_DEV_PREFER_APP=1` (no URL) | **fail** | false | — |
| `CORTEX_APP_URL=...` | remote | false | app route |
| (nothing set) | local | **true** | — |
| `CORTEX_DEV_DISABLE_LOCAL_SPAWN=1` (no URL) | **fail** | false | — |
| `CORTEX_DEV_DISABLE_LOCAL_SPAWN=1 + CORTEX_PLUGIN_ALLOW_LOCAL_SPAWN=1` | local | true | — |

### Residual risks

- `prepare-runtime.cjs` hard-coded fallback `let version = '0.5.0'` remains — advisory warn only when bumped. Consider replacing with a read of `plugin.json` to remove the drift surface entirely (follow-up, not blocking).
- No CI workflow edit in this pass — `docs/plugin-lockstep.md` + `Info/plugin-lockstep.md` show the placement but the actual workflow YAML edit is pending a coordinated release-infra pass.
- Health parser relaxation is compatible with older daemons but a malicious daemon could emit matching `status+stats` without a real runtime. Identity enforcement is preserved when the caller passes `expectedIdentity`, so this only affects local attach cases where Cortex already trusts its own binary.

### Next lockstep enforcement step

Add a `plugin-lockstep` CI step before any plugin-build or plugin-publish job. Enforce via required check on the GitHub branch protection rule. Currently tracked in `Info/plugin-lockstep.md` §CI hook.

---

## 2026-04-26 — Plugin MCP HTTP attach-only hardening (1 commit)

Fixes the 2026-04-25 Claude Code plugin spawn-path incident where plugin MCP could launch `cortex.exe plugin mcp --agent claude-code` while the Control Center daemon already owned `127.0.0.1:7437`.

### Commit pushed (origin/master -> `09e97a7`)

- **`09e97a7`** v0.6.0 - plugin MCP: HTTP attach-only bridge
  - `plugins/cortex-plugin/scripts/run-mcp.cjs` — replaced the Cortex child-process bridge with a Node stdio-to-HTTP proxy. JSON-RPC now posts to `/mcp-rpc` with `X-Cortex-Request: true`, `Authorization: Bearer <token>`, `X-Source-Agent`, and optional `X-Source-Model`. Local route defaults to `http://127.0.0.1:7437`; no local-spawn fallback remains.
  - `plugins/cortex-plugin/scripts/hook-boot.cjs` — SessionStart is status-only and no longer resolves or shells into any `cortex` binary.
  - `plugins/cortex-plugin/scripts/run-mcp.contract.test.cjs` — rewritten around the HTTP proxy contract: route resolution, local identity-validated readiness, `/mcp-rpc` headers, JSON-RPC parse error, and dry-run no-spawn behavior.
  - `plugins/cortex-plugin/scripts/dry-run-matrix.cjs` — matrix updated to HTTP attach-only semantics; legacy local-spawn flags are ignored because no spawn path exists.
  - `plugins/cortex-plugin/ROUTING.md` — documents the new HTTP attach-only policy and local token lookup order.

### Validation

- `node --test plugins/cortex-plugin/scripts/run-mcp.contract.test.cjs` -> 12/12 pass.
- `node plugins/cortex-plugin/scripts/dry-run-matrix.cjs` -> 8/8 pass.
- `node --check` on `run-mcp.cjs`, `hook-boot.cjs`, `dry-run-matrix.cjs`, `run-mcp.contract.test.cjs` -> clean.
- `python tools/audit_spawn_paths.py --strict` -> clean; `plugin_spawn_primitive` none; `forbidden_plugin_legacy_app_url` none.
- Live stdin smoke: `'{bad json' | node plugins/cortex-plugin/scripts/run-mcp.cjs` returned local JSON-RPC parse error through the Node bridge and did not create a new `cortex.exe` process.
- Process check after smoke: only the Control Center daemon plus the pre-existing Codex MCP proxy remained; the stale Claude plugin MCP process was gone.

### Behavior matrix (plugin routing, revised)

| env / config | route mode | spawn allowed | url |
|---|---|---|---|
| `CLAUDE_PLUGIN_OPTION_CORTEX_URL=...` | remote | false | explicit |
| `CORTEX_DEV_PREFER_APP=1 + CORTEX_APP_URL=...` | remote | false | app route |
| `CORTEX_DEV_PREFER_APP=1` (no `CORTEX_APP_URL`) | **fail** | false | - |
| `CORTEX_APP_URL=...` | remote | false | app route |
| (nothing set) | local | false | `http://127.0.0.1:7437` |
| legacy local-spawn flags | local | false | ignored |

### Residual risks

- Direct `cortex mcp --agent ...` remains a separate stdio proxy process for hosts configured that way. This is not a second daemon and does not own daemon lifecycle, but the process list still shows it as `cortex.exe`.
- Plugin route now trusts local `/readiness` runtime identity fields when present and only sends the local token after a Cortex-shaped health/readiness response validates.

---

## 2026-04-26 — C9 retention classes (1 commit)

Adds lifecycle classes for stored decisions and imported/exported memory data so C3/G5 can reason over governance categories instead of raw TTLs.

### Commit pushed (origin/master -> `bd85025`)

- **`bd85025`** v0.6.0 - C9: retention classes
  - `daemon-rs/src/api_types.rs` — new `RetentionClass` enum with serde, explicit parser, default TTLs (`durable` no expiry, `operational` 90 days, `audit` 365 days, `ephemeral` 14 days), type mapping, and text heuristics.
  - `daemon-rs/src/db.rs` — migration 016 and fresh schema support for `retention_class` on `memories` and `decisions`.
  - `daemon-rs/src/handlers/store.rs` — store path classifies retention before insert; explicit `ttl_seconds` remains the override; response JSON includes `retention_class`.
  - `daemon-rs/src/handlers/mcp.rs` — MCP `cortex_store` accepts/validates `retention_class`, advertises the schema enum, and returns the stored class.
  - `daemon-rs/src/export_data.rs` — JSON/SQL export and import payloads preserve retention classes.
  - `specs/cortex-openapi.yaml` — StoreRequest, ImportMemory, and ImportDecision expose `retention_class`.

### Validation

- `rtk cargo check --manifest-path daemon-rs/Cargo.toml --tests` with isolated `CARGO_TARGET_DIR=target-codex-c9` -> clean.
- `rtk cargo test --manifest-path daemon-rs/Cargo.toml store_decision_ -- --nocapture` -> 5 passed.
- `rtk cargo test --manifest-path daemon-rs/Cargo.toml import_payload_normalizes_types_and_preserves_temporal_fields -- --nocapture` -> 1 passed.
- `rtk cargo test --manifest-path daemon-rs/Cargo.toml test_run_pending_migrations_applies_all_once -- --nocapture` -> 1 passed.
- `rtk cargo clippy --manifest-path daemon-rs/Cargo.toml --tests -- -D warnings` -> clean.
- Full daemon suite: `rtk cargo test --manifest-path daemon-rs/Cargo.toml -- --nocapture` -> 465 passed.

### Residual risks / follow-ups

- Existing memory-producing paths rely on the database default `operational`; this is acceptable for C9 but G5 may need richer memory-side classification when memory write paths become first-class.
- MCP invalid retention classes now fail fast; HTTP invalid classes are rejected by serde before handler execution.

---

## 2026-04-26 — R2 score-adaptive boot truncation (1 commit)

Replaces the boot compiler's all-or-nothing greedy truncation path with score-adaptive capsule allocation when priority variance exists, while preserving the legacy greedy path for flat scores.

### Commit pushed (origin/master -> `a547a07`)

- **`a547a07`** v0.6.0 - R2: score-adaptive boot truncation
  - `daemon-rs/src/compiler.rs` — added source-token bounds (`CORTEX_BOOT_MIN_SOURCE_TOKENS`, default 40; `CORTEX_BOOT_MAX_SOURCE_TOKENS`, default 600), score-variance detection, token-budget truncation helper, score-adaptive allocation, and legacy greedy fallback.
  - Boot capsule metadata now records `packing: "score_adaptive"`, `allocatedTokens`, and `truncated` for admitted adaptive capsules; C5 persists that through `boot_audits.capsules_json`.
  - Unit tests cover high-score > low-score allocation, flat-score fallback equivalence, max>=min bound normalization, and budget-below-floor behavior.

### Validation

- `rtk cargo check --manifest-path daemon-rs/Cargo.toml --tests` with isolated `CARGO_TARGET_DIR=target-codex-r2` -> clean.
- `rtk cargo test --manifest-path daemon-rs/Cargo.toml score_adaptive -- --nocapture` -> 2 passed.
- `rtk cargo test --manifest-path daemon-rs/Cargo.toml flat_score_fallback_matches_legacy_greedy_packing -- --nocapture` -> 1 passed.
- `rtk cargo clippy --manifest-path daemon-rs/Cargo.toml --tests -- -D warnings` -> clean.
- Full daemon suite: `rtk cargo test --manifest-path daemon-rs/Cargo.toml -- --nocapture` -> 469 passed.

### Residual risks / follow-ups

- The original R2 note pointed at `prompt_inject.rs`; implementation landed in `compiler.rs`, the actual `/boot` assembly code path.
- p50 boot latency and v0.5.0 GT-precision delta still need a dedicated benchmark run before release claims.

---

## 2026-04-26 — RQ1 BGE embedding default (1 commit)

Promotes Phase 1 embedding profiles in daemon code while keeping public docs staged until the v0.6.0 release cut.

### Commit pushed (origin/master + origin/main -> `c971a5a`)

- **`c971a5a`** v0.6.0 - RQ1: BGE embedding default
  - `daemon-rs/src/embeddings.rs` — added profile-specific pooling (`mean`, `cls`, `last_token`), max token limits, query/passage prefixes, active-asset inventory, streaming downloads, and selectable profiles.
  - Default profile is now `bge-base-en-v1.5` (768-dim, CLS pooling, BGE query instruction prefix). `all-MiniLM-L6-v2` and `all-MiniLM-L12-v2` remain explicit opt-ins.
  - `qwen3-embedding-0.6b` opt-in uses the live `onnx-community/Qwen3-Embedding-0.6B-ONNX` q8 export (`model_uint8.onnx`, ~614 MB) because Qwen's official `main` branch currently has no ONNX files.
  - `handlers/recall.rs` and `handlers/feedback.rs` now call `embed_query()` so retrieval queries get profile query instructions while stored passages remain unprefixed.
  - `handlers/health.rs` and `setup.rs` now disclose/check active profile `dimension`, `max_input_tokens`, `pooling`, model file, tokenizer file, and selected assets.
  - `mcp_proxy.rs` comment updated from MiniLM-specific wording to generic ONNX embedding engine wording.

### Validation

- Verified Hugging Face asset URLs with `HEAD`: BGE model/tokenizer and Qwen onnx-community q8 model/tokenizer return 200; Qwen official ONNX paths return 404 and were not used.
- `rtk cargo check --manifest-path daemon-rs/Cargo.toml --tests` with isolated `CARGO_TARGET_DIR=target-codex-rq1` -> clean.
- `rtk cargo test --manifest-path daemon-rs/Cargo.toml embeddings -- --nocapture` -> 13 passed.
- `rtk cargo clippy --manifest-path daemon-rs/Cargo.toml --tests -- -D warnings` -> clean.
- Full daemon suite: `rtk cargo test --manifest-path daemon-rs/Cargo.toml` -> 473 passed.

### Residual risks / follow-ups

- Pure LongMemEval-S improvement, p50 recall regression, and backfill throughput gates remain unmeasured; do not make public quality claims until benchmark artifacts land.
- Public `README.md`, `Info/connecting.md`, and `Info/roadmap.md` were intentionally left unchanged pre-release; release copy remains staged in `updates-to-readme.md`.
- No DB migration was added because model-scoped embedding storage and active-model backlog detection already exist.

---

## 2026-04-30 — Daemon stability + storage hygiene acceleration (5 commits)

This batch corrected the post-RQ1 operational reality before the next recall-quality feature: keep the daemon alive under handler failures, then attack the storage bloat found during the live v0.6.0 cycle. It also moved part of the v0.6.1 bloat plan into v0.6.0 because the wins were local, bounded, and already validated on the production DB.

### Commits pushed (origin/master -> `4c3b43c`)

- **`db491ca`** v0.6.0 - daemon stability hardening: supervisor + panic hook + catch_panic
  - `desktop/cortex-control-center/src-tauri/src/main.rs` — app-managed daemon supervisor thread, intentional-stop guard, and throttled respawn logging.
  - `daemon-rs/src/main.rs` — daemon panic hook writes payload, location, and backtrace to `~/.cortex/panic.log`.
  - `daemon-rs/src/server.rs`, `daemon-rs/Cargo.toml`, `daemon-rs/Cargo.lock` — `tower-http` catch-panic layer returns JSON 500s instead of letting handler panics take down the daemon.
  - `daemon-rs/src/mcp_proxy.rs` — heartbeat tolerance increased from 2 to 5 recovery cycles.

- **`fe78000`** v0.6.0 - compaction: FTS5 optimize, stale-model embeddings, singleton co-occurrence
  - `daemon-rs/src/compaction.rs` — adds FTS5 optimize pass, stale-model embedding pruning, singleton co-occurrence pruning, and new `CompactionResult` counters.
  - `daemon-rs/src/server.rs` — surfaces new compaction counters through the JSON response.
  - Live validation recovered 371 MB (`390 MB -> 27 MB`), pruned 9767 stale embeddings and 20642 singleton co-occurrence rows, and kept recall/store/MCP functional afterward.

- **`84d20cc`** v0.6.0 - compaction: FTS-segment-pressure governor trigger
  - `daemon-rs/src/compaction.rs` — adds `fts_segment_row_total`, a canonical pressure predicate that includes FTS shadow-table row count, and governor diagnostics for pre/post FTS row totals.
  - Tests cover the pressure predicate, FTS row counting on a fresh schema, and an end-to-end optimize pass shrinking segment rows after UPDATE churn.

- **`2fb1c20`** v0.6.0 - PQ8: int8 quantize embeddings (3072B -> 774B per vector, ~4x)
  - `daemon-rs/src/embeddings.rs` — canonical `vector_to_blob` now writes PQ8; `blob_to_vector` auto-detects PQ8 vs legacy f32; tests cover layout, round-trip drift, cosine drift, NaN/inf safety, ordering preservation, and collision-byte disambiguation.
  - `daemon-rs/src/handlers/recall.rs` — semantic recall length filters accept both legacy f32 and PQ8 during migration.
  - `daemon-rs/src/compaction.rs` — batched legacy embedding migration using 2-byte magic/version signature plus length-mod-4 legacy detection.
  - `daemon-rs/src/server.rs` — exposes migrated PQ8 row count.
  - Live validation ended with 631 embedding rows at 774 bytes each and recall returning expected documents.

- **`4c3b43c`** v0.6.0 - PQ8: extend migration to `memory_clusters.centroid` (~4x on crystals)
  - `daemon-rs/src/compaction.rs` — refactors embedding migration into generic `migrate_legacy_blob_column_to_pq8(table, column, pk)` and targets both `embeddings.vector` and `memory_clusters.centroid`.
  - Regression test seeds one legacy centroid and one already-PQ8 centroid, verifies exactly one migration, and checks idempotency.
  - Live validation drained 9603 legacy centroids over 10 batched passes; final distribution 9701 PQ8 / 0 legacy. End-to-end DB size since session start: `412 MB -> 26 MB` (~94% reduction).

### Validation

- `db491ca`: rebuilt dev daemon at `target-control-center-dev/debug/cortex.exe`, swapped in via app-managed shutdown + binary copy, single and burst `cortex_store` calls kept daemon healthy; supervisor activates on next Control Center restart.
- `fe78000`: live compaction against production DB, then recall/store/MCP smoke.
- `84d20cc`: focused compaction tests for FTS pressure and optimize shrinkage.
- `2fb1c20`: focused embedding/PQ8 test suite and live recall smoke on PQ8 corpus.
- `4c3b43c`: focused centroid migration regression plus live centroid drain.

### Residual risks / follow-ups

- `origin/main` still points at `c971a5a`; sync it explicitly if it remains part of the release process.
- Public copy should say storage hygiene improved materially in v0.6.0, but avoid the earlier v0.6.1-only wording that promised `auto_vacuum`, Zstd text compression, or DB stats CLI as already shipped.
- True centroid-residual encoding is deferred because `cluster_members` stores references to embeddings, not per-member vectors.

---

## 2026-05-05 — RQ2 cross-encoder reranker plumbing + local benchmark gate

This batch moves RQ2 from plan-only to runnable daemon code, while keeping the runtime default off until the benchmark gates prove quality and latency. The planned fastembed wrapper was not used because the existing daemon already carries `ort` + `tokenizers`, so the direct adapter keeps the dependency surface smaller.

### Commit status

- **`f07d61f`** v0.6.0 - RQ2: gated cross-encoder reranker
- **`2707913`** v0.6.0 - RQ2: add local reranker benchmark gate
- **`f6c1ebb`** v0.6.0 - RQ2: capture local reranker gate artifacts

### Primary files touched

- `daemon-rs/src/rerank.rs` *(new)* — `RerankMode`, `RerankConfig`, `Reranker` trait, `MiniLmReranker`, Xenova `ms-marco-MiniLM-L-6-v2` int8 ONNX/tokenizer asset inventory, downloader, tokenizer pair encoding, ORT inference, and score fusion.
- `daemon-rs/src/state.rs` — stores rerank config + optional reranker engine; model load happens only when mode is `shadow` or `primary`.
- `daemon-rs/src/setup.rs` — validates/downloads reranker assets only when rerank is enabled.
- `daemon-rs/src/handlers/health.rs` — exposes mode, availability, model metadata, `top_n`, and `fusion_alpha` under health.
- `daemon-rs/src/handlers/recall.rs` — adds `rerankRoute` telemetry to recall and policy explain; shadow mode observes only; primary mode reorders the configured top-N window and leaves the tail untouched.
- `daemon-rs/src/embeddings.rs` — adds an explicit `#[allow(dead_code)]` to the legacy f32 blob encoder because it is intentionally retained for tests and one-off migrations, and `clippy -D warnings` now runs clean.
- `benchmarking/scripts/rq2_rerank_gate.py` — local deterministic off/shadow/primary benchmark runner. Emits the required RQ2 artifact bundle.
- `benchmarking/results/rq2-rerank-20260505-031510/` — committed local gate artifacts: clean manifest, off/shadow/primary payloads, latency summary, quality summary, retriever-regression summary, README.
- `docs/internal/v060/rerank-findings.md` *(ignored internal doc)* — CAUTION release posture writeup.

### Validation

- `rtk cargo fmt --manifest-path daemon-rs/Cargo.toml` -> clean.
- `CARGO_TARGET_DIR=daemon-rs/target-codex-rq2-gate rtk cargo test --manifest-path daemon-rs/Cargo.toml rerank` -> 6 passed.
- `CORTEX_RERANK_REAL_MODEL_SMOKE=1 CORTEX_RERANK_REAL_MODEL_DIR=C:\Users\aditya\.cortex\models CARGO_TARGET_DIR=daemon-rs/target-codex-rq2-gate rtk cargo test --manifest-path daemon-rs/Cargo.toml real_minilm_model_loads_and_scores_when_enabled -- --nocapture` -> 1 passed.
- `CARGO_TARGET_DIR=daemon-rs/target-codex-rq2-gate rtk cargo check --manifest-path daemon-rs/Cargo.toml --tests` -> clean.
- `CARGO_TARGET_DIR=daemon-rs/target-codex-rq2-gate rtk cargo clippy --manifest-path daemon-rs/Cargo.toml --all-targets -- -D warnings` -> clean.
- `CARGO_TARGET_DIR=daemon-rs/target-codex-rq2-gate rtk cargo test --manifest-path daemon-rs/Cargo.toml` -> 497 passed.
- Normal `daemon-rs/target/debug/cortex.exe` was locked by running local Cortex processes, so validation used the isolated target directory above.
- Reranker setup/model download with `CORTEX_RERANK_MODE=shadow` downloaded all five required assets under `C:\Users\aditya\.cortex\models\rerank\ms-marco-MiniLM-L-6-v2\`.
- Real model load/inference smoke: `CORTEX_RERANK_REAL_MODEL_SMOKE=1`, `CORTEX_RERANK_REAL_MODEL_DIR=C:\Users\aditya\.cortex\models`, focused test -> 1 passed.
- Local deterministic RQ2 gate: `python benchmarking\scripts\rq2_rerank_gate.py --mode all --json --models-source C:\Users\aditya\.cortex\models --target-dir C:\Users\aditya\cortex\daemon-rs\target-codex-rq2-gate --skip-build --skip-model-download`.
- Gate result: CAUTION. Owned top-1 improved `0.0000 -> 0.6667`, top-3 stayed `1.0000`, primary p95 delta `+19.004ms`, shadow order matched off, model available in health for shadow/primary.

### Residual risks / follow-ups

- Scored Pure LongMemEval-S remains unrun because no cloud answerer/judge API key was configured. Do not claim public primary-rerank quality until that gate passes.
- Public release copy should stay shadow/experimental/default-off unless a scored LongMemEval-S rerank-on/off comparison passes.
- The committed local gate manifest is pristine (`git_status_short` empty) and points at tested code commit `2707913`; artifact commit is `f6c1ebb`.

---

## 2026-05-05 — v060 planning folder cleanup + triad catalog audit

Internal-only docs cleanup. No product code changed and no public docs changed. The cleanup corrected the folder structure after the RQ2 pass revealed that root-level planning docs mixed landed work, active plans, future research, superseded specs, and executed prompts.

### Why

- The live triad (`unified-status-plan.md`, `comprehensive-changelog.md`, `updates-to-readme.md`) belongs at `docs/internal/v060/` root for fast access.
- The rest of the folder needed explicit classification so old `phase-*` guides are not mistaken for completed v0.6.0 work.
- The user explicitly asked whether all phases had been completed; answer: no. This cleanup makes that visible in the index and unified status plan.

### File moves / organization

- Root retained:
  - `README.md`
  - `unified-status-plan.md`
  - `comprehensive-changelog.md`
  - `updates-to-readme.md`
- `scope/` now owns:
  - `scope/scope.md`
  - `scope/scope-lock.md`
  - `scope/open-questions.md`
  - `scope/v050-shipped-vs-slipped.md`
  - `scope/public-roadmap-update.md`
- `plans/` now owns:
  - `plans/accessibility-motion-settings.md`
  - `plans/foundation-carryovers.md`
  - `plans/governance-economics.md`
  - `plans/recall-improvement-plan.md`
  - `plans/bridge-track-spec.md`
  - `plans/db-stats-cli-spec.md`
- `execution/` now owns:
  - `execution/phase-0-purity-execution.md`
  - `execution/phase-1-embedding-upgrade.md`
  - `execution/phase-2-reranker-execution.md`
- `future/` now owns:
  - `future/phase-3-0-ingest-extraction-execution.md`
  - `future/phase-3-5-execution.md`
  - `future/phase-4-adaptive-k-execution.md`
  - `future/phase-5-hipporag-ppr-execution.md`
  - `future/phase-5-5-observer-reflector-execution.md`
  - `future/phase-6-execution.md`
  - `future/observer-reflector-prompts.md`
- `prompts/` now owns:
  - `prompts/c3-budget-governance-goal-prompt.md`
- `archive/` now owns:
  - `archive/next-big-pass-goal-prompt.md`
  - `archive/reranking-harness.md`
- `status/` now owns:
  - `status/SESSION-HANDOFF.md`
  - `status/rerank-findings.md`
- `research/` remains the source library with 19 research/audit notes cataloged in `unified-status-plan.md` section 10:
  - `research/a11y-codebase-audit.md`
  - `research/a11y-motion-research.md`
  - `research/a11y-react-libs.md`
  - `research/a11y-wcag22-research.md`
  - `research/agent-memory-production.md`
  - `research/benchmark-purity-audit.md`
  - `research/daemon-bloat-compression.md`
  - `research/helper-audit-second-pass.md`
  - `research/ingest-extraction-models.md`
  - `research/judge-reliability-adversarial-eval.md`
  - `research/memory-benchmarks-landscape.md`
  - `research/memory-graph-algorithms.md`
  - `research/memory-systems-survey.md`
  - `research/multihop-retrieval.md`
  - `research/rerank-models-landscape.md`
  - `research/retrieval-sota.md`
  - `research/sota-memory-architectures.md`
  - `research/sqlite-vec-hybrid.md`
  - `research/tiny-llm-landscape.md`

### Catalog / tracking updates

- `README.md` now states the reality check: not every phase doc has executed. It identifies C3 budget governance as the next big pass and RQ2 as CAUTION.
- 2026-05-05 supersession: C3 backend is now landed; the next public-facing C3 work is U1 budget Settings UI/write support plus optional load testing.
- `unified-status-plan.md` section 10 now catalogs every markdown file under `docs/internal/v060/` by status and role.
- `updates-to-readme.md` now records which internal planning surfaces have public-copy implications and which are internal-only.
- `prompts/c3-budget-governance-goal-prompt.md` now points to the root-level triad paths.

### Current interpretation after catalog pass

- Active next as of this catalog pass: C3 budget governance. Superseded 2026-05-05 by `b41f7be`/`efe38a2`/`1231d58`; active follow-ups are U1 Settings budget UI and benchmark-gated retrieval evidence.
- Active but gated: RQ0/RQ1/RQ2 benchmark evidence.
- Pending release headline: U1 accessibility/settings/motion.
- Future research, not done: Phase 3.0, 3.5, 4, 5, 5.5, 6.
- Archive/superseded: original reranking harness and executed RQ2 goal prompt.

### Validation

- Inventory generated for every `*.md` under `docs/internal/v060`.
- Root-level triad restored to `docs/internal/v060/`.
- No deletions; files were moved and cataloged.
- This is internal ignored-doc work only; no commit hash unless the folder is intentionally force-added later.

---

## Reversal / rework log

Empty at v0.6.0 start. Append-only: if a shipped change is later reverted or reworked, add a dated entry here with `REVERTED:` or `REWORKED:` prefix, new commit hash, and rationale. Never delete the original landing entry.

### 2026-04-24 — REVISED (not reversed)

- **Q4 "Reranking scope"** revised from "measurement harness only" → "production Phase 2 ship". Driven by Phase 2 execution guide being complete at code-level fidelity before the scope lock had been formalized. See `open-questions.md` Q4 + `scope-lock.md` Q3 amendment.

---

## Index of commits referenced (by theme)

Populated as v0.6.0 work lands. Each entry: `<hash> <short subject> → <workstream ID>`.

- `0406b02` — chore(repo): untrack internal docs and stale v0.4.1 benchmark dump → pre-v0.6.0 hygiene
- `ca9ea2c` — chore(gitignore): block browser-harness artifacts from public repo → pre-v0.6.0 hygiene
- `e64887c` — chore(repo): untrack chrome extension and internal sync script → pre-v0.6.0 hygiene
- `b9a6458` — docs(benchmark): correct misstated v0.5.0 benchmark claims → pre-v0.6.0 hygiene + Phase 0 purity prep
- `c2ba28d` — v0.6.0 - plugin routing: dev-prefer-app policy + explicit failure modes → plugin parity
- `9632b6a` — v0.6.0 - plugin hook-boot: route-aware status + backward-compat health → plugin parity
- `0c6bb62` — v0.6.0 - plugin/daemon lockstep version guard + CI hook doc → release infra
- `d50744c` — v0.6.0 - docs(plugin): lockstep guard usage under Info/ → release infra
- `f625614` — v0.6.0 - benchmarking: CAS-100 adversarial suite → Phase 0 purity prep
- `323c5cf` — v0.6.0 - benchmarking: triangle judge (GPT-4o + Claude + local Qwen3-30B) → Phase 0 purity prep
- `c734886` — v0.6.0 - H3: consolidate DEFAULT_CORTEX_PORT const across daemon + desktop → foundation carryovers
- `7460484` — v0.6.0 - Repowise cleanup: drop dead logging + mcp_stdio modules → cleanup pass (187 LOC removed, 2 files + 4-line main.rs edit)
- `f1d23ae` — v0.6.0 - G2: cortex admin rollback CLI + admin.rs module → foundation carryovers (new admin.rs ~310 LOC, session rollback via status flip, event audit, 5 unit tests, CLI smoke)
- `f4488c3` — v0.6.0 - Phase 0 purity: cortex-http-pure adapter + 5 CI gates + CODEOWNERS → recall Phase 0 (new 101-LOC adapter, 5 gate scripts, CODEOWNERS, benchmark-triad.sh, purity-gates CI job, benchmarking README update; no API costs; triad run deferred)
- `754c4b5` — v0.6.0 - docs(readme): reflect shipped Phase 0 adapter + admin rollback CLI → public surface alignment (benchmark note tense fix, CLI reference row for admin rollback)
- `83509b4` — v0.6.0 - C5: boot audit trail + GET /boot/audit + 90-day configurable prune → foundation carryovers (migration 015, boot_audits table, audit write in handle_boot, GET /boot/audit endpoint, CORTEX_BOOT_AUDIT_RETENTION_DAYS env override, default 90 days not 30 per spec revision)
- `2eefc9d` — v0.6.0 - docs(roadmap): reconcile v0.5.0 shipped + rescope v0.6.0 + v0.7.0 → public roadmap alignment
- `09e97a7` — v0.6.0 - plugin MCP: HTTP attach-only bridge → plugin hardening
- `bd85025` — v0.6.0 - C9: retention classes → governance + economics
- `a547a07` — v0.6.0 - R2: score-adaptive boot truncation → foundation carryovers
- `c971a5a` — v0.6.0 - RQ1: BGE embedding default → recall Phase 1
- `db491ca` — v0.6.0 - daemon stability hardening: supervisor + panic hook + catch_panic → daemon reliability
- `fe78000` — v0.6.0 - compaction: FTS5 optimize, stale-model embeddings, singleton co-occurrence → storage hygiene
- `84d20cc` — v0.6.0 - compaction: FTS-segment-pressure governor trigger → storage hygiene
- `2fb1c20` — v0.6.0 - PQ8: int8 quantize embeddings → storage hygiene / embedding footprint
- `4c3b43c` — v0.6.0 - PQ8: extend migration to memory_clusters.centroid → storage hygiene / crystal footprint
- `f07d61f` — v0.6.0 - RQ2: gated cross-encoder reranker → recall Phase 2
- `2707913` — v0.6.0 - RQ2: add local reranker benchmark gate → recall Phase 2 validation
- `f6c1ebb` — v0.6.0 - RQ2: capture local reranker gate artifacts → recall Phase 2 validation
