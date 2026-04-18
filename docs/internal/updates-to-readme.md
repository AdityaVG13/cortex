# README Update Queue (Living)

Last updated: 2026-04-18  
Scope baseline: `v0.4.1` -> current `HEAD`

## Purpose
This is the staging queue for README updates that should be considered for the next release.
It is intentionally broader than final README copy. Use this file to decide what ships publicly.

## How To Use
1. Keep this file additive while work continues.
2. For each item, keep at least one concrete commit reference.
3. When drafting final README copy, move only validated, externally appropriate content.

## Candidate README Additions

### 0.1) Startup Fanout Coalescing + Event-Pressure Controls (new)
- Add a short startup-reliability section for large-history operators:
  - Control Center refresh flow now coalesces timer/SSE/retry triggers through one in-flight cycle to prevent duplicate protected API bursts.
  - Startup-critical daemon GET routes now avoid write-side cleanup and use read-path query filtering (reduces read/write contention under load).
  - Event-growth safeguards now enforce per-event-type caps and a global non-boot event cap in compaction, preventing high-volume `decision_stored` growth from degrading startup over time.
  - Startup-heavy tables now have additional read-path indexes (including owner-scoped variants) to reduce cold-start scan pressure.
  - `/savings` analytics now operate on a bounded recent window (`30d`) for heavy aggregate surfaces.
- Validation references:
  - `rtk cargo test` in `daemon-rs` (`367` passing, isolated target dir).
  - `rtk cargo test compaction` (`11` passing).
  - `rtk cargo test --test recall_benchmark -- --nocapture` (`7` passing).
  - `rtk npm --prefix desktop/cortex-control-center test` (`57` passing).
  - `rtk npm --prefix desktop/cortex-control-center run web:build` (pass).
- Commit:
  - `8a3cb21`

### 0) Core Operating Defaults (Policy Clarity)
- Keep a dedicated README section that states three defaults clearly:
  - solo mode => authenticated user is admin by default
  - app-managed workflows => one daemon ever; clients attach, they do not spawn
  - remote targets => explicit token required (no implicit local token carryover)
- Rationale and copy-ready draft language are staged in:
  - `docs/internal/v050/README-policy-writeups.md`

### 1) Lifecycle Model: One Daemon, Service-First
- Add/update a clear statement that Cortex now runs with strict service-first startup in app flows.
- Clarify that Control Center no longer falls back to sidecar spawn.
- Clarify that MCP proxy paths no longer contain daemon respawn/shutdown ownership behavior.
- Clarify the attach-only app-client contract:
  - app registrations now write `CORTEX_APP_REQUIRED=1`, `CORTEX_DAEMON_OWNER_LOCAL_SPAWN=0`, and `CORTEX_APP_CLIENT=<agent>`
  - attach-only clients now return machine-readable `APP_INIT_REQUIRED` when the app-managed daemon is offline
  - if `CORTEX_APP_CLIENT` is present but spawn policy env is incomplete, daemon policy now fails closed instead of allowing local spawn
  - operator guidance should point users to initialize Cortex from Control Center rather than auto-spawning from arbitrary clients
- Suggested message: one canonical daemon, deterministic startup decisions, and explicit blocked state when service path is unavailable.
- Document hard startup path denylist rules that reject temp/runtime wrapper binaries (`cortex-daemon-run*`, `daemon-lifecycle-runtime`, temp roots) before daemon startup.
- Commits:
  - `9dee06b` service-first app boot + `service ensure`
  - `f9f2c1a` strict service-first gate
  - `6d40e79` startup path decision telemetry
  - `b6a5d5b` removed Control Center sidecar fallback and legacy sidecar module
  - `df5c367` removed dormant MCP auto-spawn branches from daemon main path
  - `7989d82` removed MCP proxy respawn/shutdown ownership branches + daemon lifecycle dead spawn machinery
  - `80019f5` app/plugin/daemon runtime-path hardening (deny temp wrapper execution)
  - `b7e4b7b` attach-only env contract + app-required daemon-init enforcement
  - `c8d5fb0` fail-closed policy for partial app-client env contracts
  -   - 95e215c lock-test stabilization to avoid env-race flakes under parallel suites while preserving one-daemon guarantees

### 2) Readiness Semantics (Liveness vs Readiness)
- Document `/readiness` as the canonical startup gate and `/health` as compatibility/liveness diagnostics.
- Mention readiness-first probing across daemon/plugin/control-center.
- Commits:
  - `94c5475` readiness endpoint + readiness-first probes
  - `16cb6ae` control-center readiness-first reachability
  - `6e17dca` plugin SessionStart readiness-first with fallback
  - `56978ca` bind-accurate readiness transition + service ensure payload validation

### 3) Plugin Routing and Binary Resolution
- Document plugin attach-only routing behavior by default.
- Add binary resolution order (app/canonical first, bundled fallback last) for development lockstep.
- Remove legacy route/env references where applicable.
- Document local attach safety gate:
  - local mode blocks temporary/bundled plugin binaries by default
  - plugin now fails fast with app-managed binary guidance instead of unsafe fallback runtime paths
  - plugin can promote bundled binary into canonical `~/.cortex/bin/` install in local mode to avoid temp execution while preserving plugin-only usability
  - optional escape hatch for advanced workflows (`CORTEX_PLUGIN_ALLOW_BUNDLED_BINARY=1`)
- Commits:
  - `2e186ca` attach-only routing baseline
  - `77f73e6` SessionStart no longer spawns daemon
  - `b6a5d5b` app-first plugin binary resolver (`resolve-binary.cjs`)
  - `01da61f` removed legacy `CORTEX_DEV_APP_URL` alias from plugin scripts/docs
  - `0cc0082` local attach binary safety gate + bundled fallback override
  - `80019f5` local-mode canonical binary promotion + startup denylist enforcement

### 4) Security and Identity Hardening
- Add a security/reliability section for daemon identity validation and ownership controls.
- Suggested bullets:
  - signed owner claims
  - parent process linkage enforcement
  - wrong-instance token fallback protections
  - fail-closed behavior in team-mode auth boundaries
- Commits:
  - `7ed6927`, `9bd7f7a`, `4216dc1`, `1b1291d`, `b1717e2`
  - `92876b8`, `82a47f6`, `2e48637`, `9516648`, `9c081aa`

### 5) Client Permission Enforcement (Foundation)
- Add a release-note section for MCP access policy controls.
- Suggested bullets:
  - new `client_permissions` schema for per-client and per-tool grants
  - read/write/admin gate mapping across MCP tools
  - admin MCP tools for permission management (`grant`, `revoke`, `list`)
  - team-mode policy scoping by caller owner ID
  - backward-compatible permissive fallback until explicit policy rows are configured
- Commits:
  - `e0dd421` client permission table + migration + MCP enforcement foundation
  - `7374f31` permission-management MCP admin tools
  - `3ba3544` HTTP admin surfaces + Control Center permission UI

### 5.2) Permission Ops in Control Center
- Add README coverage for runtime permission operations in the desktop app.
- Suggested bullets:
  - list active permission grants from daemon (`/permissions`)
  - grant/revoke client permission + scope from Memory panel
  - team mode requires admin role for these endpoints
  - solo mode remains authenticated-owner management
- Commit:
  - `3ba3544`
  - `9254c7c` (admin remediation endpoint regression coverage for `/admin/unowned`, `/admin/assign-owner`, `/admin/set-visibility`)

### 1.1) Benchmark Safety for App-Managed Mode (new)
- Add README note that benchmark/eval tooling in app-attached mode now self-cleans benchmark-injected rows after each run.
- Clarify this protects power-user recall quality/startup by preventing benchmark namespace buildup in the live app DB.
- Mention audit artifact (`namespace-cleanup.json`) and env toggle (`CORTEX_BENCHMARK_CLEANUP_ON_EXIT`, default on).
- Pending commit (2026-04-17 local batch):
  - `benchmarking/run_amb_cortex.py` app-daemon attach enforcement + namespace cleanup-on-exit
  - `benchmarking/adapters/tests/test_run_amb_cortex_shims.py` cleanup regression coverage

### 1.2) Power-User Recall/Startup Scalability (new)
- Add README note for large-memory operational tuning:
  - scoped recall (`source_prefix`) now filters semantic/shadow scans in SQL before vector scoring to avoid unrelated-corpus scans
  - embedding session pool size is now tunable via `CORTEX_EMBED_SESSION_POOL_SIZE` (default `2`, clamp `1..8`) for startup/throughput tradeoffs
- Add benchmark fairness note:
  - AMB cortex provider now defaults to conservative concurrency (`1`) with env override (`CORTEX_BENCHMARK_PROVIDER_CONCURRENCY`) to prevent burst-rate artifacts on local daemons
- Commit `373dce0` (2026-04-17):
  - `daemon-rs/src/handlers/recall.rs` scoped semantic + shadow-semantic SQL filtering + streaming row processing
  - `daemon-rs/src/embeddings.rs` configurable session pool sizing
  - `benchmarking/adapters/cortex_amb_provider.py` concurrency default + override
  - `benchmarking/adapters/tests/test_run_amb_cortex_shims.py` concurrency regression coverage

### 5.1) Provenance and Trust Scoring (Foundation)
- Add a release-note section for memory/decision provenance metadata.
- Suggested bullets:
  - persisted source provenance fields (`source_client`, `source_model`, `reasoning_depth`)
  - persisted trust score at write-time (`trust_score`)
  - trust score currently computed from confidence and model-tier weighting
  - import/export paths preserve provenance/trust metadata
- Commits:
  - `6116a9b` provenance schema migration + write path + import/export support

### 6) Recall / Retrieval Quality Improvements
- Add a release-note section for recall precision and budget-aware improvements.
- Suggested bullets:
  - compound scoring + RRF retrieval pipeline
  - query-adaptive weighted RRF so terse/code-like queries lean keyword-first while longer natural-language prompts lean semantic-first
  - synonym-expanded term-group parity across FTS ranking, fallback ranking, and excerpt centering so shorthand queries still land on expanded source text
  - sqlite-vec foundation wired into daemon bootstrap/health with a verified `vec0` smoke path, while live recall remains on the current blob-scan backend until shadow validation is complete
  - semantic budget packing is now query-shape adaptive (relevance floor, max-items, and excerpt caps tuned for exact/code-like vs broad natural-language prompts)
  - fallback ranking for memories/decisions now uses query-shape adaptive keyword/score/recency/retrieval weighting, including score-aware ordering when term groups collapse
  - AMB isolated runner now applies compatibility shims for pinned dataset variants:
    - unsupported `user_ids` kwargs are dropped safely with best-effort user filtering on returned docs
    - LongMemEval prompt construction is enforced as context-first (raw payload telemetry cannot replace retrieved context)
  - Python/TypeScript SDKs now include prompt-ready recall context helpers that preserve excerpt content and optionally append compact retrieval metrics (`format_recall_context`/`recall_for_prompt`, `formatRecallContext`/`recallForPrompt`)
  - recall explain/query-vector efficiency improvements now avoid duplicate embedding work by reusing one query embedding across ranking and shadow diagnostics
  - embedding profile groundwork is now in place (`CORTEX_EMBEDDING_MODEL`) with migration-safe model tagging, model-aware semantic filtering, and model-aware startup backfill targeting
  - quality-gated dedup
  - relevance-band compaction
  - source-prefix scoping and query-aware feedback
  - trust-aware ranking blend (`score + trust_score`) in keyword + semantic recall paths
  - mixed-scale importance normalization compatibility (`0-1` and `0-100`)
  - associative memory expansion in budget recall using co-occurrence graph signals, with strict low-budget/noise guards
  - crystal results now act as family heads instead of duplicate add-ons:
    - related member hits collapse under the crystal
    - recall payloads surface `familyMembers`, `familySize`, and `collapsedSources`
    - unfold on a crystal returns the consolidated summary plus visible member sources
    - if the crystal summary was already served, recall can fall back to a collapsed child source instead of repeating the same family summary
    - crystal family-head excerpts are query-focused instead of a blind leading slice
    - ranked child fallback selection prefers the strongest unseen family member instead of lexical source order
    - cached recall payloads preserve family collapse metadata across pre-cache hits
    - low/medium budget packing now compacts same-family candidates before token spend, so one crystal family does not crowd out unrelated context
    - recall policy diagnostics can now explain family compaction explicitly instead of lumping those drops into generic budget/rank loss
  - legacy crystal/member schemas still work during upgrades (missing ACL columns no longer hide crystals)
  - null-sourced memories still collapse correctly via canonical `memory::{id}` keys
- Commits:
  - `ad74a92`, `082f275`, `33def16`
  - `200fbef`
  - `d26657a`
  - `f7f3c3d`
  - `3d75bdd`
  - `f6f878d`
  - `6d19f9f`
  - `bc87c26`
  - `5d9ea40`, `cc97ec6`, `f348870`
  - `1fb0ca4`
  - `e5251dd`
  - `ead685e`, `7f2b7ce`
  - `dd1131e`, `b3295a2`, `8792d24`
  - `7e1dd40`
  - `0f58482` associative co-occurrence expansion for budget recall

### 6.1) Conflict Intelligence + Resolution Surfaces
- Add a release-note section for conflict handling that goes beyond simple disputed pairs.
- Suggested bullets:
  - explicit conflict classes (`AGREES`, `CONTRADICTS`, `REFINES`, `UNRELATED`)
  - trust-aware policy engine on store (auto-resolve/merge/supersede/open conflict routing)
  - durable conflict records (`decision_conflicts`) with classification/similarity/resolution metadata
  - MCP admin conflict management tools (`cortex_conflicts_list`, `cortex_conflicts_get`, `cortex_conflicts_resolve`)
  - upgraded Control Center conflict dashboard with classification badges, trust context, timestamps, and manual resolve controls
- Commit:
  - `dd5bbd0`

### 7) Operational Reliability / Data Hygiene
- Consider adding an operations section for:
  - startup log rotation
  - backup retention / cleanup
  - stale PID cleanup
  - cleanup CLI
  - health storage metrics
  - health embedding migration metrics (`embedding_inventory`, active-model ratio, re-embed backlog)
  - TTL expiration cleanup support
- Commits:
  - `2a619f5`, `cb3459d`, `b4878f0`, `c4d35e1`, `5fbfb39`, `1cb0db5`
  - `34b2bc6`
  - `4b617a3`, `dfb6302`
  - `214f253`

### 7.1) Upgrade Safety / Migration Guarantees
- Add a short release-facing statement that current daemon builds are regression-tested against real `v0.4.1` on-disk databases.
- Suggested coverage:
  - current boot path accepts a legacy `v0.4.1` database without hand-migration steps
  - legacy data survives upgrade into the latest schema
  - tracked schema migrations advance the database to the current `user_version`
  - startup reseeds FTS bookkeeping and preserves shared-token/auth file creation behavior
- Commit:
  - `7beef8a`

### 8) Control Center UX/Resilience Improvements
- Add/update notes around improved startup/restart reliability and dashboard smoothness.
- Suggested coverage:
  - restart transition hardening
  - duplicate instance prevention
  - panel recovery/perf polish
  - richer conflict resolution UX with metadata-aware cards
- Commits:
  - `99f2620`, `d5d58cd`, `0551baa`, `6930a2c`, `090baef`, `628f15b`
  - `dd5bbd0`

### 9) Benchmarking / Validation Infrastructure
- Add a short reliability statement that benchmark infrastructure is now isolated and reproducible.
- Add first-class matrix workflow coverage so benchmark claims are not LongMemEval-only:
  - `python benchmarking\\run_amb_cortex.py matrix --matrix-file benchmarking/configs/amb-eval-matrix.stage1.json`
  - include dry-run validation and continue-on-error semantics for operational visibility
  - include clean fail-fast behavior when provider credentials are missing (explicit per-case errors, no fabricated scores)
- Add a note that daemon benchmark coverage now includes distilled-proxy-vs-full ranking tracking:
  - benchmark harness now uses `/recall/explain` to evaluate a distilled 6-feature proxy against full recall ordering
  - regression gates now assert top-1 agreement and mean absolute rank error for proxy tracking quality
- Add a note that sqlite-vec shadow KNN diagnostics now ride on the explain surface for parity tracking before production cutover:
  - `/recall/explain` now emits `shadowSemantic` status and overlap diagnostics (`ok` / `unavailable` / `error`)
  - benchmark regression now verifies shadow diagnostic payload presence on each benchmark case
- Add a note that semantic recall now remains active on solo-schema databases:
  - semantic candidate SQL now falls back when team-only owner/visibility columns are absent
  - this avoids silent semantic-candidate dropouts in default solo-mode installs
- Add a note that shadow semantic status now differentiates capability gaps from probe faults:
  - known sqlite-vec missing-module failures are reported as `unavailable` (not `error`)
  - explain diagnostics include explicit `sqlite_vec_not_available` reason for operator clarity
- Add a note that sqlite-vec shadow diagnostics are now visible on the live unified recall telemetry path (still non-routing):
  - `recall_query` events now include compact `shadow_semantic` status/reason/overlap metadata on uncached and headlines requests
  - cache-hit events explicitly mark `shadow_semantic` as `skipped` (`reason=cache_hit`)
  - source-list arrays are intentionally omitted from event payloads to control token/storage overhead
- Add a note that `/stats` now exposes shadow-semantic parity rollups from live recall telemetry:
  - status counts for `ok` / `unavailable` / `error` / `skipped`
  - average overlap ratio and jaccard for `ok` shadow probes
  - both `shadow_semantic` and `shadowSemantic` aliases for client compatibility
- Add a note that `/stats` now includes explicit vec0 rollout gate diagnostics:
  - threshold-driven `hold` vs `ready_for_vec0_trial` decision output
  - blocker reasons and gate metrics (sample volume, unavailable/error rates, overlap/jaccard quality)
  - dual aliases (`shadow_semantic_gate` and `shadowSemanticGate`) for client compatibility
- Add a note that vec0 gate diagnostics now include ranking-drift metrics, not just set overlap:
  - `meanAbsRankDelta` and `top1Match` now flow from shadow explain into recall telemetry and `/stats` rollups
  - gate decisions now include thresholds for max mean rank delta and minimum top-1 match rate
- Add a note that benchmark runs now honor strict singleton daemon policy:
  - benchmark harness self-skips with an explicit reason when an app-managed daemon already holds the singleton lock
  - avoids false benchmark failures caused by attempted second-daemon startup during local app sessions
- Add a note that distilled-proxy benchmark guardrails now validate explain accounting integrity:
  - benchmark now checks `policy.budgetReasoning.spent` against returned item token sums
  - benchmark now checks pre-budget drop math (`totalPreBudgetDrops == droppedCount + familyCompactedCount`) and required ranking factor fields
  - benchmark now enforces top-1 hit rate, recall coverage, p95 query-token, and tokens-per-relevant-hit thresholds
- Commits:
  - `f6016f4` first-class matrix runner + tracked stage-1 matrix spec + matrix helper tests
  - `92212b5`, `e210ffb`, `dffeb8d`, `c25ba7f`
  - `c8ace09` cross-platform plugin routing dry-run validation in CI (Linux/macOS/Windows)
  - `39c32a2` distilled proxy-vs-full recall ordering benchmark regression
  - `0b41668` distilled proxy/token-accounting gate expansion
  - `06088fd` sqlite-vec shadow KNN explain diagnostics and schema-compatible shadow row collection
  - `f938ea0` solo-schema semantic candidate fallback for memory/decision retrieval
  - `deeecb3` shadow-status hardening for sqlite-vec-unavailable classification
  - `3654260` unified-recall shadow semantic telemetry mirror + branch-complete event regressions
  - `52739fe` `/stats` shadow-semantic rollups for status/overlap gating
  - `d63257d` `/stats` vec0 routing gate diagnostics with blockers/thresholds
  - `b06a056` shadow gate rank-delta/top1 parity metrics + gate thresholds
  - `b7e4b7b` singleton-aware benchmark skip behavior under active app daemon

### 10) Docs and Research Surfaces
- If desired for public README:
  - mention research page availability and positioning
  - mention updated proof/benchmark visual surfaces
- Commits:
  - `e24a0ec`, `db5e986`, `2d1ea5e`, `8a6fdcc`
  - multiple README polish commits during `2026-04-10`

### 11) IPC Transport Foundation (Step 3 Start)
- Add/update architecture notes that Cortex now exposes optional local IPC endpoints in parallel with HTTP.
- Suggested bullets:
  - daemon resolves canonical local IPC endpoint per platform
  - Windows named pipe endpoint support + Unix socket endpoint support
  - health/readiness payloads now surface runtime IPC metadata
  - MCP proxy now prefers IPC for local traffic with automatic HTTP fallback for compatibility
  - hook/status/admin/boot local client paths now share the same IPC-first transport policy
  - remote target URLs continue to use explicit HTTP(S) behavior (no implicit local IPC rewrite)
  - HTTP remains fully supported during transition while client-side IPC preference rolls out
- Commits:
  - `223686f` admin surface hardening + gate cleanup batch
  - `04e869d` IPC listener foundation + transport resolver centralization
  - `ed73143` MCP proxy IPC-first transport migration + parser tests
  - `2538d43` shared transport helpers wired into boot/status/admin + health probes

### 12) Agent Intelligence Telemetry (Step 9.3 Foundations)
- Add a section for model-agnostic intelligence loops that make *any* connected AI smarter over time.
- Suggested bullets:
  - recall policy explainability API for deterministic retrieval introspection
  - agent outcome telemetry schema (`agent_feedback`) with owner scoping
  - adaptive recall depth (`k`) tuning from historical task outcomes (`adaptive=true`, `taskClass`)
  - new MCP tools for write/read feedback loops:
    - `cortex_agent_feedback_record`
    - `cortex_agent_feedback_stats`
  - new HTTP telemetry endpoints:
    - `POST /agent-feedback`
    - `GET /agent-feedback/stats`
  - reliability rollups by agent/task class with memory-source coverage metrics
- Commits:
  - `85b28dc` recall policy explain endpoint + MCP tool
  - `8523bcb` agent feedback telemetry schema + HTTP/MCP surfaces
  - `78f98b5` adaptive recall depth from agent feedback telemetry

### 13) Plugin Local Mode + Cross-Platform Lifecycle
- Update plugin lifecycle messaging to reflect current behavior:
  - app-first routing remains preferred
  - plugin-only local mode is supported
  - local plugin mode uses service-first ensure on Windows
  - local plugin mode now has safe daemon ensure on non-Windows (macOS/Linux)
- Clarify binary safety posture:
  - temporary runtime binaries are blocked in local mode
  - app-managed binary preferred
  - bundled binary fallback is allowed when safe, with strict-mode knobs
- Commits:
  - `d1ec974` app-first routing with safe plugin-only local mode + ROUTING policy update
  - `005c6c9` cross-platform local plugin daemon ensure

### 14) Storage Pressure Governor + Recall Stats Surface
- Add a release-note section for bounded local storage behavior without write hard-fails.
- Suggested bullets:
  - pressure-aware compaction governor (soft/hard thresholds)
  - health payload surfaces DB pressure/utilization/limits
  - compaction remains transparent to clients (no "DB too big" write rejection mode)
  - `/stats` endpoint for recall-tier and latency telemetry rollups
- Commits:
  - `62db111` storage pressure governor + health pressure telemetry + lifecycle hardening
  - `56e6169` governor threshold tests to prevent silent compaction-trigger regressions
  - `1b39357` recall stats endpoint and tier telemetry

### 15) Session Truth and Restart Recovery
- Add a release-note section for why Cortex now feels more reliable after daemon restarts and transient disconnects.
- Suggested bullets:
  - MCP boot now emits a real `session` event so the control center refreshes agent presence immediately
  - read-path memory tools also refresh authoritative session presence instead of assuming a prior boot/reconnect path
  - missing MCP session rows are recreated on demand by `peek`, `recall`, `recall_policy_explain`, `semantic_recall`, and `unfold`
  - existing session descriptions are preserved, so truth-refresh traffic does not downgrade richer boot context in the Agents panel
  - daemon-side regressions now lock in missing-session recovery and duplicate-event suppression behavior
  - model-less follow-on MCP memory reads now reattach to the existing modeled session instead of creating a second logical session row
  - the desktop app now has a real `desktop:verify:lifecycle:dev` restart/reconnect proof path, not just browser smoke
- Commit:
  - `4f1c5ea` read-path MCP session truth refresh + boot session event coverage
  - `aebefce` app-managed restart verification runner + modeled-session refresh hardening

### 16) Desktop Lifecycle Clarity
- Add/update desktop-app wording so the README matches the actual ownership model.
- Suggested bullets:
  - the desktop app manages one local Cortex daemon instance in local mode
  - local restart/start/stop semantics are app-managed, not generic sidecar semantics
  - plugin-only mode is still supported, but app-managed mode stays the primary development/test path
  - a dedicated `desktop/cortex-control-center/DEVELOPING.md` now documents one-daemon development rules and commands
  - Windows preflight daemon probes now run hidden during app-managed startup/restart, avoiding daemon console popup flashes in local development
- Commit:
  - `3c949e5` app-managed lifecycle copy cleanup + desktop developer guide
  - `aebefce` hidden preflight subprocesses + lifecycle verification runner

### 16.1) Desktop Compatibility / Work-Surface Polish
- Consider a small release-note note for desktop compatibility behavior instead of treating these as invisible implementation details.
- Suggested bullets:
  - older/mock daemons that do not expose `/permissions` no longer spam the desktop dashboard with repeating warnings
  - Shared Feed kind filters now immediately refresh the underlying query instead of waiting for the background poll interval
- Commit:
  - `aebefce`

## README Sections That Likely Need Revision
- `Quick Start`
  - Add `cortex service ensure` guidance for Windows lifecycle.
  - Ensure plugin flow language matches attach-only behavior.
- `Architecture` / `How It Works`
  - Explicitly describe service ownership and daemon readiness model.
- `Security`
  - Add ownership-token and identity-hardening summary bullets.
- `Troubleshooting`
  - Add blocked startup guidance when service path is unavailable.
  - Add readiness/health probe checks.
- `Environment Variables`
  - Keep current supported vars.
  - Remove legacy/deprecated vars from public examples.

## Env Var Cleanup Queue (README)
Potentially remove/de-emphasize from public docs if still present:
- `CORTEX_DEV_APP_URL` (removed from plugin scripts in `01da61f`)
- `CORTEX_ALLOW_SIDECAR_FALLBACK` (removed from Control Center flow in `b6a5d5b`)
- legacy local-spawn toggles in plugin narratives (cleaned from active plugin routing docs/tests in `34b72b9`)

Potentially emphasize:
- `CORTEX_APP_URL`
- explicit API key usage for remote targets
- explicit binary override vars only for advanced development workflows

## Migration Notes To Consider (v0.4.1 -> current)
- Startup behavior is stricter in favor of service-managed lifecycle.
- Plugin behavior is attach-only by default and no longer expected to bootstrap daemon ownership.
- Health checks are readiness-aware; stale or wrong-instance local targets are rejected more aggressively.

## Pre-Release README Checklist
- Validate all command examples against current CLI behavior.
- Validate all environment variable names against current source.
- Validate architecture statements against one-daemon policy and current tests.
- Ensure no claim conflicts with current Windows/manual-service defaults.

## Coverage Audit (v0.4.1..HEAD)
- Full commit sweep requirement: include **both** `0.5.0`-prefixed and non-prefixed commits in release prep.
- Current sweep source:
  - `git log --oneline --reverse v0.4.1..HEAD`
  - `git rev-list --count v0.4.1..HEAD` (current: `227`)
- This queue intentionally references major grouped tracks, while raw full commit history lives in `docs/internal/comprehensive-changelog.md`.
- 2026-04-16 00:33:10 -04:00 | c5ffa4 | Small task: hardened benchmark singleton-skip detection and env scrub in daemon-rs/tests/recall_benchmark.rs.
- 2026-04-16 00:33:24 -04:00 | 1ad69b3 | Small task: added allow_service_ensure short-circuit coverage for local spawn policy in daemon-rs/src/main.rs.
- 2026-04-16 00:33:48 -04:00 | d3d7c37 | Small task: added regression test asserting shadow gate holds on high rank-delta / low top1 match in daemon-rs/src/handlers/health.rs.
- 2026-04-16 01:18:36 -04:00 | 4cb8941 | Queue: document stricter vec0 shadow gate semantics (normalized status buckets + per-metric sample sufficiency) for rollout-readiness transparency.
- 2026-04-16 01:19:42 -04:00 | ebcf704 | Queue: document benchmark quality gates for distilled proxy tracking (pairwise agreement + coverage floors) and CI fail-closed singleton skip semantics.
- 2026-04-16 01:21:54 -04:00 | 96c7942 | Queue: mention that MCP transport/header validation suites are now singleton-safe and aligned with attach-only APP_INIT_REQUIRED policy semantics.
- 2026-04-16 01:42:38 -04:00 | 335ea61 | Queue: update release/docs narrative to reflect FTS tokenizer migration to porter+unicode61 (migration 012) and mention improved stemming/word-level recall behavior.
- 2026-04-16 01:44:50 -04:00 | c3b6300 | Queue: document BM25 tuning direction (text-first relevance under porter tokenizer) for release-facing retrieval notes.
- 2026-04-16 01:49:01 -04:00 | 854165e | Queue: call out selectable embedding profile support (L6 default + L12 modern option) and bounded background re-embed policy knobs for operators.
- 2026-04-16 01:53:04 -04:00 | 67553c9 | Queue: reflect OpenAPI 0.5.0 surface parity updates (/readiness, /recall/explain, /stats) in external docs when README refresh is unblocked.

- 2026-04-16 03:16:48 -04:00 | ebab223 | Queue: when README updates are unblocked, add SDK reliability note that Python/TypeScript clients now have CI-backed conformance tests (headers/query/body contract).
- 2026-04-16 03:33:52 -04:00 | 025a91d | Queue: when README edits are unblocked, document new canary env controls (CORTEX_SQLITE_VEC_TRIAL_PERCENT / CORTEX_SQLITE_VEC_TRIAL_FORCE_OFF) and explain that vec0 routing is guarded, sampled, and fail-closed by default.

- 2026-04-16 03:53:31 -04:00 | 94c90d7 | Prompt injector hardening: strict CLI value validation, true <file>.injected output suffixing, URL-safe boot query construction, higher-resolution watch change detection, and non-global token-reader test coverage.

- 2026-04-16 04:20:50 -04:00 | fd49244 | Reliability + portability guards: added health runtime-path home-scope integration coverage, concurrent attach-only app-client policy regression, and removed developer-specific daemon fixture paths from source tests.

- 2026-04-16 04:52:11 -04:00 | f60a2d1 | Hardened scripts/clean_install_smoke.sh for out-of-box reliability: LF-safe bash bootstrap, cargo.exe fallback on Windows bash, fail-closed search error handling, graphify/target scan exclusions, ASCII status output, and optional README enforcement via CORTEX_ENFORCE_PUBLIC_README.
- 2026-04-16 05:31:27 -04:00 | 77c175d | One-daemon hardening: improved control-center lock contention detection (including Windows lock codes 32/33), enforced lock-aware attach-only gating in ensure_daemon, and added debug/test-only single-daemon bypass path for daemon integration suites; validated with full daemon test/clippy/benchmark/audit stack.

- [2026-04-16] commit fba87af test(benchmarking): add cortex adapter contract coverage
  - Noted new internal benchmark adapter conformance tests (README public update deferred).

- [2026-04-16] commit b236c21 test(lifecycle): harden preflight lock gating and deterministic startup checks
  - Internal note only: lifecycle reliability/tests strengthened; public README update remains deferred.

- [2026-04-16] commit 4b057ff chore(smoke): harden fresh-install and one-daemon release checks
  - Internal-only release gating improvements; public README remains intentionally unchanged.

- [2026-04-16] commit a09a5db test(admin): expand ownership remediation regression coverage
  - Internal endpoint regression expansion only; no public README edits.
- [2026-04-16 12:23:16 -04:00] commit cceaef9 test(lifecycle): cover spawn-parent orphan watcher shutdown path
  - Extracted daemon orphan-watch loop into a reusable helper and added async regression coverage proving parent identity break triggers shutdown sender consumption.

- [2026-04-16 12:26:29 -04:00] commit ad9ba3c test(plugin): add route and attach-contract conformance coverage
  - Refactored run-mcp bridge into testable contract helpers, added node contract suite for route/args/env ownership behavior, and enforced that suite in CI plugin validation.

- [2026-04-16 12:29:50 -04:00] commit 01ef26b feat(recall): boost ranking with entity-alignment signals
  - Added entity-like term extraction/alignment boosts to unified + semantic recall ranking and surfaced entity match/overlap/boost factors in recall explain payloads with dedicated regressions.

- [2026-04-16 12:40:08 -04:00] commit 699c351 test(reliability): add cross-surface attach and spawn-audit regressions
  - Added cross-surface concurrent attach-only regression for shared-home CLI/plugin callers, made spawn audit root override + missing-path handling robust, and added strict audit pytest regressions wired into CI.

- [2026-04-16 13:06:41 -04:00] commit 35adfde feat(extension): add secure MV3 cortex chrome companion scaffold
  - Added a production-oriented Manifest V3 Chrome extension (background service worker, popup/options UI, context-menu store flow, runtime origin-permission gating, local-first defaults) plus policy-alignment notes and core unit tests.

- [2026-04-16 13:07:07 -04:00] commit 931536a feat(adapter): add openai function-call cortex adapter contract
  - Implemented OpenAI function adapter execution layer over Cortex HTTP (health, store, 
ecall) with strict argument validation and dedicated pytest contract coverage.

- [2026-04-16 13:07:34 -04:00] commit 300ca30 test(ci): enforce chrome extension and adapter contract suites
  - Added CI jobs for benchmark adapter pytest coverage and Chrome extension manifest/core tests to prevent adapter and extension contract drift before merge.

- [2026-04-16 13:18:58 -04:00] commit ede6634 test(lifecycle): verify orphan watcher against real parent exit
  - Added process-level regression proving spawn-parent orphan watcher observes a real parent-process exit and triggers daemon shutdown signaling, tightening one-daemon parent-death coverage.


## 2026-04-16 13:41 � 5eeb1ad
- Added daemon POST /recall support and wired extension recall traffic to body-based requests to avoid query-string leakage of recall prompts.
- Hardened Chrome extension Web Store posture: loopback-only host model, removed wildcard optional host permissions, session-default API key storage, and opt-in page metadata capture.
- Added extension privacy policy draft and refreshed policy-compliance references to official Chrome documentation.
- Validation: 
ode --test extensions/cortex-chrome-extension/tests/core.test.mjs; 
tk cargo fmt; 
tk cargo test; 
tk cargo clippy -- -D warnings; 
tk cargo test --test recall_benchmark -- --nocapture.

## 2026-04-16 13:46 � eb11ef0
- Added 	ools/validate_chrome_extension_policy.py to enforce MV3 Web Store guardrails (loopback-only hosts, no wildcard optional hosts, no remote scripts/eval patterns, required policy docs present).
- Wired the guardrail into .github/workflows/ci.yml under chrome-extension-validation.
- Validation: python tools/validate_chrome_extension_policy.py; 
ode --test extensions/cortex-chrome-extension/tests/core.test.mjs; python tools/audit_spawn_paths.py --strict.

## 2026-04-16 16:25 � 32f955b
- Synced release-facing docs for v0.5 closeout in tracked Info/ surfaces:
  - Info/roadmap.md now reflects v0.5 stabilization closeout status and remaining release-doc targets.
  - Info/team-mode-setup.md replaced insecure broad-bind examples with security-first deployment matrix.
  - Added Info/startup-matrix-troubleshooting.md as one-daemon startup truth + failure triage playbook.
  - Linked startup matrix from Info/connecting.md.
- Closeout impact: directly addresses unified-plan blockers for roadmap/release-facing sync and startup/troubleshooting refresh.

## 2026-04-16 16:26 � 80dab99
- Hardened tracked public security guidance for team-mode transport boundaries in Info/security-rules.md:
  - non-loopback binds now explicitly require TLS on public/routed interfaces
  - HTTP-only path explicitly limited to private encrypted mesh interfaces
  - deployment recommendations tightened to prevent accidental raw internet exposure
- Closeout impact: reinforces docs/security contract alignment for v0.5 release surfaces.

- 2026-04-16 18:04:32 -04:00 | Queue for public README refresh (security + team-mode clarity)
  - Document that team-mode endpoints require caller-scoped ctx_ keys for user data paths; daemon token alone is no longer accepted for team-scoped read/write/feed flows.
  - Clarify admin/destructive endpoint protection: rated auth + admin role in team mode.
  - Note SDK safety default: local token autoload applies only to loopback; remote base URLs require explicit token.
  - Note plugin secret handling: remote API key passed via env, never via CLI args.
  - Note desktop behavior: external/unmanaged daemon instances are not forcibly shut down by app stop/shutdown.
  - Note extension policy: Web Store build is loopback-only over HTTP local Cortex URL.
- 2026-04-16 18:04:32 -04:00 | f525244 | README queue updated for team-mode caller-scoped auth, admin gating, SDK remote token policy, plugin API-key transport, desktop external-daemon semantics, and extension loopback-only policy.


## 2026-04-17 13:52 - 8b3ab47
- README-impacting queue from this Phase-2A batch:
  - Document new benchmark hard cap for single-run path (--max-runtime-seconds, env CORTEX_BENCHMARK_RUN_MAX_RUNTIME_SECONDS, enforced 15-20 minute window).
  - Document retrieval-quality tuning in benchmark adapter/client: detail-aware reranking (date/location/item/speed) and reduced low-signal assistant snippet preference.
  - Document updated benchmark namespace/source-prefix scoping and retry behavior for transient daemon backpressure.
- Validation completed:
  - rtk python -m pytest benchmarking/adapters/tests/test_run_amb_cortex_shims.py sdks/python/tests/test_client.py -q (26 passed)
  - rtk python -m pytest benchmarking/adapters/tests/test_cortex_http_client.py benchmarking/adapters/tests/test_run_amb_cortex_matrix.py -q (21 passed)
  - rtk npm test in sdks/typescript (6 passed)
- Benchmark status note:
  - Strict-capped scored LongMemEval rerun was attempted but blocked due missing answer/judge provider key in active shell env.

## 2026-04-17 21:30 - README impact queue (Phase-2A fair-run + scoring)
- Add benchmark fairness section for `benchmarking/run_amb_cortex.py`:
  - `fair-run-preflight.json` is emitted for every `run`/`matrix` invocation.
  - Fair-run policy is fail-fast before execution: no oracle, no `query_id`, no `doc_limit` shortcuts.
  - Hard runtime cap policy remains mandatory: single run `900-1200s`; matrix run `<=1200s`; matrix case `<=900s`.
- Add benchmark result snapshot (strict/fair):
  - LongMemEval `20/20` achieved at `benchmarking/runs/amb-run-20260417-211640` with avg recall tokens `200.6`.
  - Non-LongMem strict matrix baseline run available at `benchmarking/runs/amb-matrix-20260417-212238`.
- Mention location-detail answer-format hardening:
  - LongMemEval shim now explicitly preserves country/state qualifiers in location answers when context provides them.
- Validation references to include:
  - `rtk python -m pytest benchmarking/adapters/tests/test_run_amb_cortex_shims.py sdks/python/tests/test_client.py -q`
  - `rtk python -m pytest benchmarking/adapters/tests/test_cortex_http_client.py benchmarking/adapters/tests/test_run_amb_cortex_matrix.py -q`
  - `rtk npm test` in `sdks/typescript`
