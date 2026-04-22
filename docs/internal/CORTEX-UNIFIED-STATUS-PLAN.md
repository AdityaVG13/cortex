# Cortex Unified Status + Plan

**Last updated:** 2026-04-20 19:14  
**Canonical owner doc:** this file  
**Purpose:** one source of truth for what is done, what is not done, what is deferred, and what ships next.

---

## 0) 2026-04-20 Autonomous Optimization Tranche (Shipped)

Shipped in separate commits:

- `735deb9` - `perf(ui): reduce tab-switch cost and improve connection UX`
  - Brain panel lifecycle optimized to avoid remount churn (`brainPanelMounted`), hidden 3D work paused/resumed, and Brain visualizer memoized to reduce non-active rerenders.
  - Throughput analytics helper introduced (`summarizeBootThroughput`) with deterministic calendar-window handling and test coverage.
  - Connection Settings popup gained explicit close `x` control for immediate dismiss UX.
  - `/peek` and `/recall` memory actions in desktop UI now explicitly use auth-aware API path.
- `4223be2` - `perf(recall): strengthen ranking signals and reduce served-dedup lock churn`
  - Recall quality upgrades: temporal-intent weighting for recency queries, query-alignment boost, deterministic relevance comparator handling non-finite values, and expanded synonym alias coverage for real-user phrasing.
  - Served-content dedup path now snapshots/stages hash writes to avoid repeated async lock churn in hot loops.
  - Deterministic recall/peek smoke regressions expanded in `handlers::recall::tests`.
- `c79e073` - `chore(runtime): add browser-harness real-app perf and recall smoke probe`
  - Added a browser-harness runtime probe script during optimization iteration.
  - Follow-up scope correction landed in `a464251`: this probe is internal operator tooling only and is intentionally not kept in Cortex repo/product code.
- `e78aa91` - `chore(gitignore): ignore runtime screenshot artifacts and local dev pid files`
  - Added ignore entries for runtime artifact screenshots and local dev PID files.
- `7ffc86c` - `chore(gitignore): ignore local codex cache workspace`
  - Added local tooling-cache ignore (`.codex-cache/`) to keep status noise from operator-only automation assets.
- `a464251` - `chore(tooling): keep browser-harness automation out of Cortex repo`
  - Removed in-repo browser-harness automation script; browser harness remains an external/internal operator tool.
- `154c884` + `c38cf3e` + `581ee46` - recall hot-path optimization follow-up
  - Query-alignment scoring now reuses per-query profiles in ranking paths instead of repeated per-comparison recomputation.
  - Fallback ranking now uses precomputed alignment metadata and precomputed excerpt focus terms to reduce repeated text-processing overhead.
- `9516892` - release footprint optimization
  - Release profile now uses `panic = "abort"` for smaller/faster daemon release binaries.
- `3bdd9b1` - DB runtime tuning follow-up
  - SQLite runtime now supports bounded env-driven mmap/cache tuning and uses `temp_store=MEMORY` to reduce disk temp-table overhead in heavy query paths.
- `99a8d27` - recall budget-result quality + redundancy guard
  - Budget packing now drops near-duplicate snippet candidates unless they add new query-term coverage.
  - Query-focused excerpt selection now prefers `[user-answer]` spans in QA-style memories, improving concrete-answer precision.
  - Added deterministic regressions for answer-span preference and redundancy-aware candidate retention/drop behavior.
- `5b4a2f4` - adaptive default recall budget (query-shape aware)
  - Default `/recall` calls with no explicit policy/budget now adapt budget by query shape + requested `k` (short/exact queries spend less; broader natural queries keep headroom).
  - Explicit mode/budget requests remain unchanged.
- `5cafbd7` - budget packing early-stop under high pressure
  - Budget selection now stops early when query-term coverage is already satisfied and budget pressure is high, avoiding low-marginal tail token spend.
  - Added regression coverage for early-stop decision gating.
- `e4fcab3` - benchmark adapter deterministic personal-memory query shaping
  - `cortex_http_base_provider` now expands personal-memory query intent for names/items/location qualifiers with stricter budget adherence (`300`) and improved answer specificity ranking.
  - Added deterministic adapter regressions for name-change, repaint/color detail, and specific-vs-generic item answers.
- `0a59e60` - rate-limit route bucket isolation for store vs recall
  - Request limiter now tracks route class buckets (`default`, `recall`, `store`) per IP, preventing heavy `/store` bursts from consuming `/recall` capacity in the same window.
  - Added optional class-specific env controls:
    - `CORTEX_RATE_LIMIT_RECALL_REQUESTS_PER_MIN`
    - `CORTEX_RATE_LIMIT_RECALL_LOOPBACK_REQUESTS_PER_MIN`
    - `CORTEX_RATE_LIMIT_STORE_REQUESTS_PER_MIN`
    - `CORTEX_RATE_LIMIT_STORE_LOOPBACK_REQUESTS_PER_MIN`
  - Auth helpers now support class-aware enforcement and `/store` + recall-family handlers opt into dedicated buckets.
- `5978c81` - `feat(tokens): expose usage/savings across cortex tool and recall surfaces`
  - MCP `tools/call` responses now consistently include `tokenUsage` + `tokenUsageLine` across Cortex tool payloads (generic decorator path).
  - Boot responses (`cortex_boot` and HTTP `/boot`) now include explicit usage/savings/budget visibility.
  - HTTP `/peek` now emits token-use/savings visibility relative to full-excerpt recall output.
  - Recall-path budget accounting now uses a shared usage invariant (`compute_recall_budget_usage` + `enforce_budget_token_invariant`) with deterministic regression coverage.
- `1b158d8` - `fix(recall): report real token usage in headlines and peek paths`
  - Headlines-mode `/recall` (`budget=0`) now reports real source-token usage and estimated savings versus full excerpts (instead of hard-coded zeros).
  - `/peek` now uses shared headlines token accounting for consistent spent/saved visibility.
  - Added deterministic regression coverage to lock headlines event/payload token accounting.
- `8629c4f` - `perf(mcp): avoid tools list rebuild on tools/call hot path`
  - `tools/call` validation now uses direct known-tool matching via permission map instead of rebuilding/parsing the full `mcp_tools()` schema list on every call.
  - Reduces avoidable allocation work on a high-frequency daemon interaction path.

Validation snapshots (same local machine/session):

- Baseline: `desktop/cortex-control-center/runtime-artifacts/baseline-harness-2026-04-20T17-30-00/metrics.json`
- After: `desktop/cortex-control-center/runtime-artifacts/after-harness-final-r2-2026-04-20T17-49-35/metrics.json`
- Aggregated tab-switch average (core panels) improved from `182.15ms` -> `167.84ms` (delta `-14.31ms`, ~`-7.9%`).
- Worst tab-switch improved from `323.55ms` -> `280.66ms`.

Health/reliability state after this tranche:

- Degraded DB event was remediated by daemon restart + integrity repair path.
- Post-repair checks:
  - `/health`: `status=ok`, `degraded=false`, `db_corrupted=false`
  - `cortex doctor`: `GREEN` (`integrity_check` and `fts indexes` OK)

---

## 1) Current Reality (No Hype)

- `v0.5.0` has strong lifecycle, auth-recovery, and daemon hardening progress already landed.
- The internal `v050-implementation-plan.md` checklist is stale in multiple sections (some items are marked unchecked there but are already completed in code and tracked in `v050-tracker.md`).
- Phase-2A core retrieval milestones are now implemented; remaining work is broader matrix coverage and production-variance proving.
- Core retrieval now defaults to `all-MiniLM-L12-v2`; sqlite-vec routing defaults to guarded `primary` mode with fail-closed baseline fallback, and migration closeout is handled via landed `cortex embeddings status|drain` controls.
- Benchmark tracks are now explicitly split into two paths:
  - tuned adapter path (`cortex-http`) for production-optimized recall
  - raw no-helper path (`cortex-http-base`) for truthful core baseline measurement
- Latest strict no-helper LongMemEval run is now `20/20` on `cortex-http-base` (`benchmarking/runs/amb-run-20260419-074548`) with gate pass (`avg_recall_tokens=201.95`, `max_recall_tokens=297`).
- Latest tuned adapter strict run is also `20/20` on `cortex-http` (`benchmarking/runs/amb-run-20260419-073924`) with gate pass (`avg_recall_tokens=199.35`, `max_recall_tokens=294`).
- Raw no-helper quality floor is no longer blocked by the earlier `4/20` baseline; remaining Phase-2A work is now about sustained quality/token efficiency under broader matrix workloads and production variance.
- Desktop daemon lifecycle is now more deterministic in app-managed mode: startup prefers local managed ensure first, stale child-handle states are scrubbed on poll/kill failures, and service ensure fallback is now explicit opt-in (`CORTEX_ALLOW_SERVICE_ENSURE_FALLBACK=1`).

---

## 2) Completed (High-Confidence)

These are implemented and tracked (see `docs/internal/v050/v050-tracker.md`):

- Phase 1 retrieval foundations (tiered retrieval + RRF fusion + compound scoring).
- Schema versioning/migration framework (Phase 3A).
- TTL/hard expiration behavior and cleanup loop coverage (Phase 3B implementation landed, despite stale unchecked lines in implementation plan).
- Clippy CI gate and warning cleanup (Phase 3C.3).
- DB resilience tranche (integrity gate, rolling backups, crash-safe WAL: 5A/5B/5C).
- Storage retention/cleanup tranche (5E.1-5E.7 complete).
- Large daemon/app lifecycle hardening sequence (7A through 7Z and follow-on stabilization commits in tracker).
- Agent outcome telemetry and adaptive recall depth APIs (`/agent-feedback`, `/agent-feedback/stats`, MCP tools, adaptive `k` policy wiring).
- Retrieval transparency endpoint landed at `GET /stats` with tier distribution, latency rollups, and recall savings accounting fed from `recall_query` events.
- `v0.4.1` -> current upgrade regression now exists as a real on-disk fixture booted through `state::initialize`, then advanced through the tracked migration ledger to latest schema.
- Crystal-family budget compaction now has final explain-payload regression coverage, so the public policy JSON is tested in addition to the internal trace.
- MCP session truth now self-heals on the daemon side:
  - `cortex_boot` emits a real `session` event so the control center updates immediately when an MCP agent comes online
  - read-path memory tools (`cortex_peek`, `cortex_recall`, `cortex_recall_policy_explain`, `cortex_semantic_recall`, `cortex_unfold`) recreate missing MCP session rows and refresh heartbeat/expiry without clobbering richer existing boot descriptions
  - missing-session recreation is now regression-tested, so a daemon restart no longer depends on a manual reconnect call before the app/plugin surfaces look truthful again
- Desktop lifecycle/operator clarity improved:
  - About/lifecycle copy now consistently describes the daemon as app-managed instead of a sidecar
  - `desktop/cortex-control-center/DEVELOPING.md` now documents the one-daemon rule for local development and verification
  - desktop build + browser smoke verified the wording change and runtime sanity
- App-managed restart/reconnect validation is now closed end-to-end through the real Tauri dev build:
  - the Control Center has a dedicated `verify:lifecycle:dev` runner that boots the app, restarts the daemon through the app surface, reconnects, and verifies session truth through the final read path
  - Windows preflight daemon subprocesses (`paths --json`, `service ensure`) now run hidden, so app-managed startup no longer flashes a daemon console popup during verification
  - daemon-side session refresh now reuses the existing modeled session when a read-path MCP call omits `model`, and reconnect no longer downgrades richer boot descriptions
- Work-surface compatibility/polling polish landed during the same validation pass:
  - missing `/permissions` capability is now treated as a non-fatal compatibility gap instead of a repeating dashboard warning loop
  - Shared Feed filters now trigger a real refresh when the operator changes filter state, so kind-filter UX matches the actual backend query behavior
- Weighted/query-adaptive RRF is now landed as a first retrieval-quality upgrade inside Phase 2A:
  - reciprocal-rank fusion now adjusts keyword vs semantic weight based on the query shape instead of treating every query like the same workload
  - short/exact/code-like queries bias toward keyword retrieval, while longer natural-language prompts bias toward semantic retrieval
  - model-unavailable paths now degrade explicitly to keyword-only fusion instead of pretending semantic weight still exists
- Synonym-expanded term-group parity now holds across keyword retrieval paths:
  - fallback memory/decision ranking now scores term groups the same way the FTS `MATCH` path already does
  - post-FTS keyword ranking now counts synonym-expanded group matches instead of only literal shorthand token hits
  - query-focused excerpts now center on the same expanded terms, so shorthand queries like `db timeout` can lock onto `database timeout` spans even when recall falls back
- sqlite-vec groundwork is now landed without changing the live recall backend:
  - DB open now bootstraps the sqlite-vec auto-extension before runtime connections are created
  - health now reports sqlite-vec availability/version while the live semantic path remains on blob-scan
  - the daemon test suite includes a real `vec0` smoke query against synthetic vectors, proving the extension is wired correctly on this stack
- sqlite-vec shadow KNN diagnostics are now landed on the explain surface without changing production ranking:
  - `GET /recall/explain` now emits `explain.shadowSemantic` status (`ok`/`unavailable`/`error`) with top-k overlap diagnostics against the baseline semantic candidate ordering
  - the shadow probe builds a vec0 KNN mirror from filtered embeddings and degrades safely when sqlite-vec or query embeddings are unavailable
  - shadow-row collection now handles both team-mode and solo schemas (owner/visibility columns present or absent) so diagnostics do not silently disappear on solo connections
  - known sqlite-vec-missing failures are now classified as `unavailable` (with detail) instead of hard `error`, so shadow status cleanly separates environment capability from probe faults
- sqlite-vec shadow diagnostics are now mirrored into unified recall telemetry without changing production ranking:
  - unified `recall_query` events now emit compact `shadow_semantic` status/reason/overlap metadata on uncached and headlines requests
  - cache-hit recall events explicitly mark `shadow_semantic` as `skipped` with `reason=cache_hit`
  - event payloads intentionally omit baseline/shadow source arrays so telemetry stays low-cost and storage-efficient
- Live DB anti-ballooning has a second compression tranche focused on persistent analytics + dead-row cleanup:
  - write-path event logging now compacts large telemetry payloads (including `merge` and `recall_query`) before persistence, with a hard JSON-size ceiling and analytics-field preservation
  - compaction now rolls old `recall_query`/`store_savings`/`tool_call_savings` rows into `event_savings_rollups` (day+hour+operation aggregates) before deleting raw rows
  - `/savings` now reads `event_savings_rollups` plus recent raw events, preserving 30-day analytics while reducing scan pressure on large event histories
  - compaction now prunes orphan `cluster_members` rows (dangling memory/decision/cluster references), preventing long-lived crystal-index bloat
- Live benchmark telemetry cleanup + vacuum pass has been re-run on the local operator DB with backup-first safety:
  - backups: `~/.cortex/backups/cortex-pre-benchmark-cleanup-20260419-075715.db` and `~/.cortex/backups/cortex-pre-benchmark-cleanup-2-20260419-075738.db`
  - post-cleanup local snapshot: events `597`, decisions `489`, embeddings `9,749`, cluster_members `5,142`
  - local DB footprint reduced from `720.93 MB` backup snapshot to `386.37 MB` active DB after cleanup + vacuum
- `/stats` now rolls up shadow-semantic parity signals from unified recall telemetry:
  - status counts (`ok` / `unavailable` / `error` / `skipped`) are aggregated from `recall_query` events
  - overlap/jaccard averages for `ok` shadow probes are now surfaced for gating decisions
  - rank-delta drift signals are now surfaced for `ok` shadow probes (`meanAbsRankDelta`, `top1MatchRate`) and included in gate decisions
  - both `shadow_semantic` (snake_case) and `shadowSemantic` (camelCase) payload aliases are emitted for client compatibility
  - vec0 routing gate diagnostics now ship directly in `/stats` (`shadow_semantic_gate` / `shadowSemanticGate`) with explicit thresholds, blocker reasons, and `hold` vs `ready_for_vec0_trial` decision output
- Embedding-profile hardening is now landed as Phase-2A groundwork:
  - embedding runtime/config now resolves a selectable profile via `CORTEX_EMBEDDING_MODEL` (defaulting to `all-MiniLM-L12-v2`; legacy `all-MiniLM-L6-v2` remains explicitly selectable) and reports the active profile in health payloads
  - semantic + shadow semantic candidate collection now filter vectors by selected model tag and expected vector dimensionality, so mixed-model rows do not pollute recall quality during migration windows
  - startup backfill now queues entries missing the active model tag (not just missing any embedding row), enabling deterministic re-embed on profile changes
  - write paths now persist `embeddings.model` from the loaded engine identity (startup backfill, store, MCP store, crystallize) instead of late env lookups
- Embedding migration observability is now shipped in `/health`:
  - `vector_search.embedding_inventory` now reports active/other/unknown model counts and active-model ratio
  - `/health` now includes `reembed_backlog` counts (memories + decisions) derived from active-model coverage checks
  - regression coverage now asserts mixed-model inventory + backlog accounting against real schema/migration setup
- Embedding profile modernization + bounded migration draining are now landed:
  - selectable modern profile `all-MiniLM-L12-v2` is available via `CORTEX_EMBEDDING_MODEL` aliases (`all-minilm-l12-v2`, `minilm-l12`, `minilm-modern`)
  - startup/background backfill now runs in bounded passes with periodic drain loops (`CORTEX_EMBED_BACKFILL_BATCH_SIZE`, `CORTEX_EMBED_BACKFILL_MAX_BATCHES_PER_PASS`, `CORTEX_EMBED_BACKFILL_INTERVAL_SECS`)
  - setup init now reports active embedding profile details and current re-embed backlog totals when a DB is present
- FTS tokenizer + BM25 tuning pass are now landed:
  - schema migration `012` rebuilds FTS tables/triggers onto `tokenize='porter unicode61'`, and fresh schema bootstrap now uses the same tokenizer
  - memory/decision BM25 weights are now explicit tuned constants wired through query params (no inline literals), with top-k ranking regressions for text-first behavior
  - BM25 field weights can now be overridden at runtime (`CORTEX_BM25_MEM_TEXT_WEIGHT`, `CORTEX_BM25_MEM_SOURCE_WEIGHT`, `CORTEX_BM25_MEM_TAGS_WEIGHT`, `CORTEX_BM25_DECISION_WEIGHT`, `CORTEX_BM25_CONTEXT_WEIGHT`) with safe clamp bounds to accelerate benchmark iteration without rebuild churn
- OpenAPI/version sweep is now advanced for `v0.5.0` closeout:
  - `specs/cortex-openapi.yaml` is now version-aligned to `0.5.0`
  - spec now declares `/readiness`, `/recall/explain`, and `/stats` plus expanded `/health` runtime/vector-search fields
  - daemon tests now guard spec version/path parity to prevent drift
- Semantic candidate collection now preserves solo-schema recall quality:
  - `collect_semantic_candidates` now falls back to no-ACL SQL projections when owner/visibility columns are absent
  - semantic candidates now remain available in default solo-mode databases instead of silently collapsing to empty vectors-only misses
  - new regression coverage asserts solo-schema semantic candidate collection still surfaces expected embedding matches
- Distilled proxy-vs-full benchmark coverage is now landed for recall ordering quality:
  - `daemon-rs/tests/recall_benchmark.rs` now calls `/recall/explain` and computes a distilled 6-feature proxy score per returned candidate
  - the benchmark suite now enforces top-1 agreement and mean absolute rank-error thresholds between proxy ordering and full recall ordering
  - this is a benchmark/regression surface only; production recall ranking behavior is unchanged
- Recall explain paths now remove duplicate embedding/baseline work:
  - `execute_recall_policy_explain_inner` now computes one query vector and reuses it across recall ranking + shadow diagnostics
  - headlines explain now uses the traced recall path so semantic baseline metadata can flow directly into shadow diagnostics without re-collecting candidates
  - budget explain paths now route through query-vector trace directly, avoiding extra engine embed calls
- App-owned singleton daemon enforcement is now hard-locked for app AI clients:
  - Control Center registration now writes an explicit attach-only env contract (`CORTEX_APP_REQUIRED=1`, `CORTEX_DAEMON_OWNER_LOCAL_SPAWN=0`, `CORTEX_APP_CLIENT=<agent>`) into both JSON and TOML server configs
  - daemon `ensure_daemon` now returns machine-readable `APP_INIT_REQUIRED` for attach-only clients when the app-managed daemon is unavailable, instead of silently trying local spawn
  - spawn policy now fails closed when `APP_CLIENT` is marked but explicit local-spawn policy is missing, preventing partial env contracts from re-enabling auto-spawn
  - plugin local scripts now emit explicit operator guidance that app-mode clients must initialize Cortex through Control Center first
  - runtime-lock tests now isolate global lock home to avoid false failures when a real app daemon is active
- sqlite-vec shadow integration and production-route hardening advanced before a default production switch:
  - unified recall + budget trace now carry semantic baseline metadata from the already computed recall trace into shadow explain, eliminating duplicate baseline recomputation
  - empty-result budget traces now preserve semantic baseline metadata for consistent shadow telemetry
- sqlite-vec route controls + semantic routing hardening advanced for production rollout:
  - runtime now supports explicit sqlite-vec route modes (`baseline`, `trial`, `primary`) via env, with `force_off` still fail-closing to baseline
  - semantic recall and unified recall traces now emit route metadata (`routeMode`, effective trial percent, route reason) for post-rollout observability
  - sqlite-vec shadow KNN SQL path is now parameterized for vector literals + `k` values (prepared statements) to avoid string-built SQL payloads in diagnostics
  - primary/trial route reordering now hydrates shadow-only sources from persisted memory/decision rows (id/source/context fallback), and preserves deterministic baseline-tail ordering after shadow-first promotion
- embedding migration drain controls advanced beyond bounded background passes:
  - startup backfill pass now avoids premature early-exit when only one table still has full batches
  - optional one-time extended startup drain (`CORTEX_EMBED_BACKFILL_DRAIN_ON_STARTUP=1`) can push backlog toward completion deterministically without changing steady-state interval behavior
  - operator-facing migration controls now exist as first-class CLI commands:
    - `cortex embeddings status` reports active-model backlog counts
    - `cortex embeddings drain` supports bounded or until-exhausted backlog draining with explicit lock-wait and iteration controls
- benchmark harness now self-skips (with explicit reason) when a singleton daemon is already active, so local app sessions no longer produce false benchmark failures while preserving one-daemon policy
- Phase-2A benchmark app-daemon safety now protects real user memory state:
  - AMB benchmark runs now default to strict app-daemon attach (`CORTEX_BENCHMARK_REQUIRE_APP_DAEMON=1`) and fail fast when Control Center is not online, so there is no hidden fallback daemon
  - app-attached benchmark runs now auto-clean their own namespace rows on exit (`source_agent=amb-cortex::<run>`), removing benchmark decisions/embeddings/events from the live app DB and preventing recall contamination + startup bloat for power users
  - cleanup behavior is regression-tested in `benchmarking/adapters/tests/test_run_amb_cortex_shims.py`
- Control Center startup timeout and dev-build lock resilience are now hardened end-to-end:
  - app-managed daemon spawn now forces loopback bind (`127.0.0.1`) in the desktop path, preventing broad-bind drift (`0.0.0.0`) in local app sessions
  - dashboard refresh now stages core/protected fetches and gates secondary endpoint fanout behind healthy core readiness, preventing concurrent 8s timeout storms during cold start
  - IPC timeout handling now aligns abort/transport budgets (`transport ~= abort - 500ms`) and raises `/health` timeout budget to `12000ms` for startup warmup
  - daemon app-managed startup now defers/staggers heavy maintenance passes (indexing, embed backfill, aging, crystallization) so readiness/health remain responsive while warmup work drains
  - `ensure-daemon-dev-binary.mjs` now rebuilds when stale and auto-recovers from Windows locked-binary failures by stopping only the locked dev daemon binary and retrying the build once
- Control Center transport + UI stability follow-up is now landed:
  - desktop runtime now suppresses Windows debug ghost console windows in app builds (`windows_subsystem = "windows"` on Windows non-test builds), removing the terminal flash during dev launch
  - IPC fallback now classifies raw transport failures (`Read failed`, `Write failed`, connect/setup timeout messages) as HTTP-fallback-eligible in both GET and POST client paths, and daemon-side partial-timeout classification now recognizes Windows `WSAETIMEDOUT` (`os error 10060`) when bytes are already buffered
  - Monte Carlo projection now handles sparse savings history without explosive drift and all projection surfaces now render large values with compact suffixes (K/M/B/T/Q) instead of scientific notation
  - regression coverage added in Control Center (`api-client.test.js`, `analytics-projection.test.js`, `number-format.test.js`) and desktop tauri tests remain green
- Startup lock-contention hardening pass is now landed for high-event-volume operators:
  - `GET /savings` no longer performs full event-log row parsing in Rust under one long DB-read lock; operation rollups now use SQL aggregation and a short TTL payload cache
  - savings analytics are no longer in startup-critical dashboard fanout; Control Center now refreshes `/savings` only when the Analytics panel is active
  - this removes a primary source of startup timeout cascades where heavy `/savings` reads starved `/sessions`, `/locks`, `/tasks`, and related protected routes on shared read-connection lock contention
  - live-database diagnostics confirm event-volume concentration in `decision_stored` (current ~433k total events with ~420k `decision_stored` rows), validating startup contention root cause
- Follow-up startup optimization pass is now landed for heavy operator histories:
  - compaction governor now includes event-pressure controls (per-event-type caps plus global non-boot cap) so large `decision_stored` growth cannot expand unbounded
  - startup-critical read endpoints now avoid write-side cleanup in GET handlers and run against `state.db_read` to reduce read/write contention
  - DB schema now includes startup-focused indexes (including owner-scoped variants) for `sessions`, `locks`, `tasks`, `feed`, `messages`, `activity`, and `events`
  - Control Center refresh now runs through a single-flight queue so interval/SSE/recovery refreshes coalesce instead of overlapping
- Startup timeout-storm hardening + app-managed spawn/runtime safeguards are now landed:
  - Control Center first-load fanout now prioritizes core routes (`/sessions`, `/locks`, `/tasks`) and defers secondary routes (`/feed`, `/messages`, `/activity`, `/conflicts`, `/permissions`) into non-blocking background refresh
  - secondary endpoint timeouts now surface partial availability instead of forcing global-offline app state when core daemon connectivity is healthy
  - desktop daemon path validation now rejects non-runtime test artifacts (`target-tests`, `target-test`, `nextest`, `target*/deps`) so app-managed startup cannot accidentally launch test binaries
  - daemon runtime now uses a bounded pooled read-connection provider (`CORTEX_DB_READ_POOL_SIZE`) plus bounded background DB-lock waits (`CORTEX_BACKGROUND_DB_LOCK_MAX_WAIT_MS`) to reduce startup contention
  - startup catch-up now uses a startup-safe storage governor pass (no VACUUM) and clamps app-managed heavy delay to `120s` max so misconfigured values (for example `777`) cannot defer stabilization for many minutes
- Savings telemetry realism + benchmark-bloat remediation are now landed:
  - live benchmark namespace bloat cleanup path is validated with backup-first procedure for oversized local DBs
  - boot savings baseline accounting now excludes custom-source filesystem-size heuristics and tracks only DB-backed boot context families actually assembled in prompt flow
  - local protected-route probe after cleanup is stable (`/sessions`, `/locks`, `/tasks`, `/feed`, `/messages`, `/activity`, `/conflicts`, `/permissions`, `/savings` all healthy in local measurement window)
- Additional startup/source-path safety and workspace-bloat prevention are now landed:
  - desktop daemon binary guard now rejects non-runtime isolated target trees beyond the intended runtime set (for example `target-rtk-*`, `target-codex-test`, `target-tests`, and `target*/build|deps|incremental`) while still allowing `target`, `target-control-center-dev`, and `target-control-center-release` runtime binaries
  - root ignore hygiene now excludes recurring transient test/cache directories (`target-tests`, `src-tauri/target-tests`, `pytest-cache-files-*`, and local pytest temp folders) from tracked/untracked noise
  - repository search hygiene now includes a dedicated `.ignore` to prevent repeated scans over Rust target trees and heavy benchmark artifact folders during engineering/debug sessions
  - added `scripts/prune-build-bloat.ps1` plus root npm wrappers (`ops:prune-build-bloat`, `ops:prune-build-bloat:apply`) for measured, operator-controlled cleanup
  - measured dry-run reclaim from current workspace: ~`22.044 GB` (largest buckets: `daemon-rs/target-tests`, `target-rtk-isolated`, `target-codex-test`, `target-rtk-audit`, `src-tauri/target-tests`)
- Benchmark precision and qualifier-completeness hardening are now landed:
  - `benchmarking/adapters/cortex_http_client.py` now augments study-abroad location seed context with one best-fit country qualifier when sibling memories contain a strong candidate and the primary span lacks country detail
  - this closes the recurring LongMemEval miss where the answer omitted country despite available context
  - regression coverage added in `benchmarking/adapters/tests/test_cortex_http_client.py` (`test_recall_documents_location_queries_append_abroad_country_qualifier_to_primary_context`)
  - strict fair-run validation (no benchmark shortcut flags): `benchmarking/runs/amb-run-20260418-224211` scored `20/20` (`accuracy=1.0`, `avg_recall_tokens=191.1`, `max_recall_tokens=295`, `over_budget_count=0`)
- Git/env performance audit workflow is now explicit and executable:
  - added `scripts/git-perf-health.ps1` with measured status timings, object-store diagnostics, known bloat sizing, and optional local git tuning apply path
  - added root npm wrappers `ops:git-perf-audit` and `ops:git-perf-apply`
  - `.gitignore` now explicitly ignores permission-noise temp roots seen in local status scans (`.tmp/pytest/`, `tmp/ptbase-*/`, `tmp/pytest-local/`)
  - local audit confirms no intrinsic git-object slowdown (status ~25ms, object store ~152.51 MiB); dominant risk remains build-artifact bloat in target/test directories
- Control Center IPC fallback + startup false-negative hardening is now landed:
  - `createApi` and `createPostApi` now fail over to direct HTTP when Tauri IPC request envelopes fail or time out, preventing full dashboard blackout from IPC-only stalls
  - IPC timeout routing now normalizes URL-style route inputs before timeout classification, so core endpoints retain core timeout budgets instead of collapsing to default `8s`
  - startup refresh now treats healthy `/health` as a reachability fallback when `daemon_status` reports false-negative unreachable, so app startup no longer stays pinned in “still starting” loops
  - stale duplicate `cortex.exe mcp --agent codex` processes were identified as startup contention noise and reduced to a single app-managed daemon in local remediation
  - frontend regression coverage now includes explicit IPC-timeout fallback tests for both GET and POST client paths
- Desktop startup lifecycle now has explicit retry-state abstraction and verification fallback:
  - new `desktop/cortex-control-center/src/daemon-startup.js` centralizes bounded startup attempts/time windows and retry delay policy
  - lifecycle verification now falls back to polling when SSE bootstrap events are missing, preventing false startup failure in noisy local dev environments
  - desktop tauri binary fallback to PATH is now opt-in via `CORTEX_ALLOW_PATH_BINARY_FALLBACK` instead of implicit behavior
- Live DB anti-ballooning storage hardening is now landed for high-volume telemetry workloads:
  - write-path event pruning now applies cap-based trimming across high-volume event families (`agent_boot`, `boot_savings`, `store_savings`, `tool_call_savings`, `decision_*`, `recall_query`, `merge`) instead of only `decision_stored`
  - compaction now treats stale `agent_boot` rows as pruneable while preserving long-horizon savings integrity via `boot_savings_rollup`
  - `boot_savings_rollup` rows are now explicitly protected from generic old-event deletion, preventing silent all-time savings drift on low-activity installs
  - event-pressure accounting/global-overflow pruning now excludes only savings history rows (`boot_savings`, `boot_savings_rollup`) so noisy non-savings telemetry cannot grow unbounded
  - daemon regressions now cover rollup-retention correctness and updated nonboot-overflow behavior

---

## 3) Phase-2A Status Matrix (Research vs Implementation)

Source of target sequence: `docs/internal/PHASE-2A-RESEARCH.md`

1. **AMB adapter + baseline:** **DONE (strict LongMemEval gate path)**  
   - credentialed strict fair-run baselines are now frozen for both benchmark backends:
     - tuned helper path (`cortex-http`): `benchmarking/runs/amb-run-20260419-073924` -> `20/20`, gate passed
     - raw no-helper path (`cortex-http-base`): `benchmarking/runs/amb-run-20260419-074548` -> `20/20`, gate passed
2. **sqlite-vec integration:** **DONE (guarded production default)**  
   - bootstrap/health/smoke-test groundwork, explain-surface shadow diagnostics, unified-recall shadow telemetry mirror, semantic-baseline reuse in shadow explain, explicit route-mode semantic routing (`baseline`/`trial`/`primary`), and shadow-only source hydration/deterministic route ordering are landed.
   - default route mode now resolves to `primary`, while routing still fails closed to baseline whenever shadow-gate blockers are present.
3. **Embedding model upgrade (MiniLM -> modern model):** **DONE**  
   - modern profile (`all-MiniLM-L12-v2`) is now the default selection (legacy L6 remains explicitly selectable).
4. **Re-embed corpus with new model:** **DONE (operator-closeout path landed)**  
   - model-aware backfill targeting, bounded periodic drains, optional startup extended-drain controls, and explicit operator-driven drain tooling (`cortex embeddings status|drain`) are landed.
   - migration completion is now deterministic and auditable through explicit backlog status + drain controls.
5. **FTS tokenizer switch (trigram -> porter/unicode):** **DONE**
6. **BM25 tuning:** **DONE (second pass + runtime calibration)**  
   - text-forward default weights have been iterated again and runtime override controls remain available for matrix-specific calibration.
7. **Weighted/query-adaptive RRF:** **DONE (first cut)**  
   - adaptive weighting now ships in the unified recall fusion path; historical hit-rate tuning is still open.
8. **Benchmark reruns after each retrieval step:** **DONE (advanced cadence path landed)**
   - strict reruns are now in place for the key LongMemEval gate path (both tuned + raw backends); matrix execution now skips known missing dataset prerequisites per case and continues with explicit skip reporting instead of all-or-nothing aborts.
   - broader cross-dataset cadence is now executable via the dedicated `cadence` command path in `benchmarking/run_amb_cortex.py` (credential-gated scoring still requires provider keys).
9. **`/stats` transparency endpoint (tier hit rates + latency + savings):** **DONE (first cut)**  
   - `GET /stats` now ships with tier/mode distribution, avg latency, and savings vs budget.
   - New recall events now emit `tier`, `method_breakdown`, and `latency_ms` fields for accurate tier attribution over time.
   - `GET /stats` now also includes shadow-semantic status/overlap rollups derived from unified recall telemetry.
   - `GET /stats` now emits explicit vec0 routing gate diagnostics (thresholds + blockers + decision), turning shadow telemetry into actionable rollout criteria.

---

## 4) Open Work (v0.5.0 Closeout Blockers)

These should be finished before calling `v0.5.0` closed:

1. **8C reconnection completion**
   - daemon-side session truth for MCP boot + recall-path tools: **DONE**
   - app Agents tab refresh path from daemon `session` events: **DONE through existing UI**
   - live restart integration verification through the dev build only: **DONE**
2. **7D / 7E clarity + dev workflow**
   - in-app lifecycle help clarity: **DONE**
   - `desktop/cortex-control-center/DEVELOPING.md`: **DONE**
   - fresh-clone build verification: **DONE** (`desktop:build` from an isolated temp clone completed through release build + MSI/NSIS bundle generation; signing was the only expected local stop without `TAURI_SIGNING_PRIVATE_KEY`)
3. **12C release logistics**
   - OpenAPI/version sweep: **DONE**
   - roadmap + release-facing docs sync (**DONE**; tracked `Info/roadmap.md` synchronized for v0.5 closeout)
   - startup matrix + troubleshooting docs refresh from v1 lifecycle cleanup (**DONE**; `Info/startup-matrix-troubleshooting.md` + refresh links in `Info/connecting.md` and `Info/team-mode-setup.md`)
4. **Docs + security contract alignment**
   - reconcile `docs/team-mode-setup.md` with compatibility security guidance so non-loopback team quick-start does not imply insecure `0.0.0.0` usage without TLS or explicit Tailscale/WireGuard exemption context (**DONE** in tracked release docs via `Info/team-mode-setup.md` + `Info/security-rules.md`)
   - keep `CORTEX-UNIFIED-STATUS-PLAN.md` as the single live-status owner and mark stale planning docs as historical/superseded where needed (**DONE**; stale plan markers updated in `CORTEX-v1-PLAN.md` + `v050-implementation-plan.md`)
5. **Adapter conformance + migration/admin remediation verification**
   - expand `AMB adapter + baseline` into explicit adapter conformance checks across MCP, OpenAI Function adapter contract, Python SDK, TypeScript SDK, Chrome extension, system-prompt injector, and direct HTTP parity (**DONE (implementation + contract tests + CI gates)**)
   - add explicit regression coverage/status tracking for schema-documented admin remediation endpoints (`/admin/unowned`, `/admin/assign-owner`, related ownership/visibility operations) (**DONE**)
6. **Reliability hardening queue (single-daemon and spawn safety)**
   - cross-process `control_center_is_active` lock-held coverage (**DONE**)
   - sleep/wake + parent-process-death respawn coordination regression coverage (**DONE**; includes process-level parent-exit watcher regression coverage)
   - concurrent startup stress coverage across app + plugin + CLI surfaces (**DONE**)
   - strict spawn-path audit enforcement (`tools/audit_spawn_paths.py --strict`) (**DONE**, with CI + pytest regression coverage)
   - app cold-start timeout-storm reduction + IPC timeout-budget tuning in Control Center (**DONE**)
   - Windows dev-daemon locked-binary rebuild retry path in desktop prebuild script (**DONE**)
7. **Benchmark operational runbook tracking (credential-gated)**
   - preserve fair-benchmark gate (no scored baseline without provider key)
   - track concrete runbook progression: first valid small-scope LongMemEval pass, persisted artifacts/metrics, then expansion to LoCoMo/MemBench/MemoryAgentBench once stable
   - cadence runner support is now landed for broader post-gate matrix progression (`run_amb_cortex.py cadence`)

---

## 5) Explicit Defer List (Do Not Pull Into v0.5.0)

Defer to `v0.6.0+` unless there is a critical blocker:

- Full settings/accessibility/motion program
- Broad repowise-driven cleanup outside validated low-risk paths
- Large new feature families that expand scope (most of Phase 9/10/11 workstreams) before core recall benchmark gains are proven
- External memory-bridge/orchestrator program (for example Hindsight/Supermemory connectors) until Phase-2A benchmark gates are locked:
  - keep Cortex local-first and canonical; bridges are optional adapters, not required infrastructure
  - prioritize read/import-first + provenance mapping (`source_system`, external IDs, sync timestamps) before any write-back sync
  - require bridge quality/token benchmarks versus native Cortex recall before broad rollout
- Per-tenant embedding-space isolation for high-security team deployments:
  - current shared embedding space with post-filtering is acceptable for trusted/team default deployments
  - namespace-isolated embedding spaces move to `v0.6+`/`v0.7+` hardening track
- Cross-instance data mobility and sync:
  - solo->team import/sync and broader cross-instance migration workflows are deferred until core benchmark and reliability gates are complete
- Team hierarchy evolution:
  - flat teams remain the default; parent/child org hierarchy progression remains deferred beyond `v0.5.0`
- Compatibility transport/security trigger matrix:
  - MCP OAuth, WebSocket/gRPC transport surfaces, and mandatory HMAC signing remain trigger-gated deferred features (customer/compliance/tooling demand required before pull-in)

References:
- `docs/internal/v050/v050-closeout-plan.md`
- `docs/internal/v050/v060-accessibility-motion-settings-plan.md`
- `docs/internal/v050/repowise-cleanup-framework.md`

---

## 6) What To Execute Next (Strict Order)

1. Keep fair-benchmark policy strict (no scoring without provider keys, no shortcut flags, no fabricated gate outcomes).
2. Run and freeze the next credentialed scored matrix baseline as soon as answer/judge keys are present:
   - first scored LongMemEval checkpoint
   - persisted artifacts + metrics
   - expansion to LoCoMo/MemBench/MemoryAgentBench after baseline stability
   - matrix runner now degrades optional-dataset prerequisite misses to per-case skips (`ed0995f`) so baseline freeze is not blocked by non-critical dataset gaps
3. Close the event-volume remediation loop for power-user databases:
   - **DONE:** root-cause accounting surface now ships in `cortex doctor` via event-pressure summary (`normal`/`elevated`/`critical`), explicit `decision_stored` row counts, and top event-type breakdown.
   - **DONE:** one-time oversized-event remediation now ships as `cortex cleanup --events [--dry-run] [--max-passes <n>]`.
   - **DONE:** startup responsiveness guardrails are now validated in code paths + tests:
     - startup recovery now preserves bounded retry windows even across transient daemon-status managed-state drops, preventing indefinite “still starting” loops.
     - dev-runtime stale wrapper sweep is verified (`~/.cortex/runtime/control-center-dev/session-*` cleanup).
     - targeted startup/control-center regressions are passing (`daemon-startup.test.js`) along with daemon event-pressure diagnostics + cleanup coverage.
4. Continue credential-free retrieval and startup optimizations with measured deltas:
   - monitor `/stats` shadow gate outputs and primary-route guard fallback reasons
   - continue embedding/recall upgrades with explicit benchmark and latency deltas
5. Keep targeted app/daemon polish scoped to validated user-facing defects and startup latency regressions.
6. Define explicit bridge-track acceptance gates (quality/token deltas, provenance guarantees, failure handling) for the `v0.6+` external memory adapter program.
7. Only then reopen broader Phase 9/10/11 expansion items.
8. Keep the remaining `v1` backlog explicitly mirrored here (do not leave unchecked items only in `CORTEX-v1-PLAN.md`):
   - **Already landed in this cycle (do not track as unresolved):** Step 7 sync primitives (`1a03003`), Step 8 activation/idle economics (`656e9ad`), and Step 9.3 daemon intelligence APIs (`639bc0a`).
   - **Step 9.2 remaining intelligence targets:** Memory Object Model v2; contradiction precision upgrade (embedding + NLI); agent skill graph; cross-agent synthesis pipeline maturity; source reliability learning; deterministic context assembly.
   - **Step 9.4 eval discipline (PARTIAL):** local eval snapshot + baseline/regression primitives are landed (`fbff1f5`, `5be5310`); remaining work is broader task-family suites and mandatory release-gate enforcement on eval deltas.
   - **Step 9.5 anti-bloat guardrails (PARTIAL):** hard recall latency budgets/fail-closed fallback and feature-intake guardrails are landed (`85d3ede`, `6a61989`); remaining work is strict ongoing enforcement of measurable benchmark uplift before intelligence-scope expansion.

---

## 7) Governance Rule (To Prevent Plan Drift)

- Treat this file as **canonical**.
- Keep historical files for context, but do not use them as live status.
- Any completed item must be reflected here in the same change that ships code.
- Any newly deferred item must be explicitly listed in Section 5 with reason.
- Owner labels in non-canonical docs are execution hints, not hard dependencies. If a task is unblocked technically, execute it with the active agent instead of waiting on a specific model/provider.

---

## 8) Full `docs/` Inventory + Ownership Model

Filesystem inventory (current):

| Scope | Files | Role |
|---|---:|---|
| `docs/architecture` | 5 | Product architecture references |
| `docs/archive` | 66 | Historical research, proposals, debates, old roadmaps |
| `docs/compatibility` | 10 | Integration compatibility specs |
| `docs/internal` | 78 | Live operator/internal planning + research + prompts |
| `docs/schema` | 9 | DB/data model references |
| root docs files | 2 (`mcp-tools.md`, `team-mode-setup.md`) | Public operator docs |
| **Total** | **170** |  |

Authoritative status model:

| Type | Authority | Notes |
|---|---|---|
| **Live status + sequencing** | `docs/internal/CORTEX-UNIFIED-STATUS-PLAN.md` | Only source-of-truth for done/open/defer |
| **Deep reference** | architecture/schema/compatibility docs + selected internal research docs | Input material, not task truth |
| **Historical archive** | `docs/archive/*`, `docs/internal/v040-archive/*`, `docs/internal/v050/phase_finished/*` | Never used as live state |
| **Automation prompts** | `docs/internal/automation/*`, `docs/internal/v050/prompts/*` | Execution aids, not product requirements |

---

## 9) Git Tracking Truth (Why "Untracked" Happened)

Current repository behavior:

- `.gitignore` explicitly ignores:
  - `docs/`
  - `docs/architecture/`
  - `docs/compatibility/`
  - `docs/schema/`
  - `docs/internal/`
  - `docs/archive/`
- Because of that, most docs are local-only and do not carry normal git lineage.
- Current tracked docs file count from `git ls-files docs`: **1**
  - tracked: `docs/internal/v050/v050-tracker.md`

Meaning of "local/untracked" in this repo:

- The file exists on disk and is usable.
- Git does not record history for it unless it is force-added or ignore rules change.
- No reliable `git log` author chain can be produced for those ignored files.

---

## 10) `automation/morning-expert-prompt.md` and Automation Subfolder

`docs/internal/automation/morning-expert-prompt.md`:

- Category: **automation prompt**.
- Purpose: operator workflow bootstrap, not release/status authority.
- Git status: currently local/ignored (no normal commit history in this repo).
- File metadata snapshot:
  - Created: `2026-04-14 05:52:28`
  - Last modified: `2026-04-14 05:53:46`
  - Size: `106,637` bytes

Policy:

1. Keep automation prompts in `internal/automation/`.
2. Do not treat them as implementation truth.
3. Any durable decision discovered from prompt runs must be distilled into this unified file.

---

## 11) Subfolder-by-Subfolder Decision Table

| Path | Keep | Use As | Notes |
|---|---|---|---|
| `docs/internal` | Yes | internal operating layer | contains active planning + evidence |
| `docs/internal/automation` | Yes | prompt runtime scripts | non-authoritative |
| `docs/internal/benchmarking` | Yes | benchmark evidence snapshots | non-authoritative for roadmap |
| `docs/internal/v040-archive` | Yes | frozen historical archive | immutable |
| `docs/internal/v050/prompts` | Yes | historical execution prompts | immutable/history |
| `docs/internal/v050/phase_finished` | Yes | proof snapshots | immutable/history |
| `docs/architecture` | Yes | reference architecture | not status |
| `docs/schema` | Yes | schema reference | not status |
| `docs/compatibility` | Yes | compatibility contracts | not status |
| `docs/archive` | Yes | long-term archive | not status |

---

## 12) Documentation Workflow Contract (Going Forward)

1. Update this file first for any status change.
2. If needed, update deep-dive docs second.
3. Never mark work "done" in scattered plan files without reflecting it here.
4. Keep defer decisions in one place (Section 5).
5. Treat historical and prompt docs as context only.

---

- 2026-04-19 20:05:00 -04:00 | local closeout batch | Startup-loop closeout + graph refresh: startup recovery now keeps bounded retry windows across transient managed-state drops (preventing indefinite “Daemon is still starting” banners), targeted startup regressions are green, and local dev runtime stale session wrappers were swept to zero.
- 2026-04-19 20:04:00 -04:00 | local graphify refresh | Rebuilt repository graph with `rtk graphify update C:\\\\Users\\\\aditya\\\\cortex` (12,465 nodes, 48,672 edges, 558 communities); no new high-risk cross-module vulnerability edges were surfaced in this pass.
- 2026-04-19 20:05:00 -04:00 | 87b0b01 | Embedding migration closeout tooling: added `cortex embeddings status` (active-model backlog visibility) and `cortex embeddings drain` (bounded or until-exhausted re-embed passes with explicit lock/iteration controls), plus regression coverage for model-specific backlog counting.
- 2026-04-19 20:22:00 -04:00 | local graphify refresh | Rebuilt repository graph with `rtk graphify update C:\\Users\\aditya\\cortex` (12,449 nodes, 48,630 edges, 590 communities) after embedding-CLI landing; no new high-risk cross-module vulnerability edges were surfaced in this pass.
- 2026-04-19 19:15:00 -04:00 | 4bde521 | Desktop daemon lifecycle hardening: app startup now prefers app-managed local ensure first, service ensure fallback is explicit opt-in (`CORTEX_ALLOW_SERVICE_ENSURE_FALLBACK=1`), stale managed-child handles are scrubbed on poll/kill errors, and prebuild daemon helper now sweeps stale `target/debug` daemon processes plus stale `~/.cortex/runtime/control-center-dev/session-*` wrappers before retrying.
- 2026-04-19 18:55:00 -04:00 | local graphify refresh | Rebuilt repository graph with `rtk graphify update C:\\Users\\aditya\\cortex` (12,443 nodes, 48,611 edges, 561 communities); no new high-risk cross-module vulnerability edges were flagged in this pass.
- 2026-04-19 18:42:00 -04:00 | ed0995f | Matrix resilience pass: AMB matrix runner now treats known dataset prerequisite gaps as per-case skips with explicit reason reporting and continues consolidated summary generation; regression coverage expanded in `test_run_amb_cortex_matrix.py`.
- 2026-04-19 18:20:00 -04:00 | 7abc919 | Monte Carlo projection precision hardening: analytics projection surfaces now clamp/round display precision before formatting to prevent long-mantissa and scientific-notation leakage in 30-day gain/run-rate cards.
- 2026-04-19 18:05:00 -04:00 | abfd03d | Desktop startup stale-daemon recovery: reconnect/startup now recovers from stale managed-daemon states and avoids persistent \"Daemon is still starting\" loops when daemon reachability is healthy but status transitions degrade.
- 2026-04-19 17:24:00 -04:00 | 360c55b | BM25 iteration enablement: recall BM25 field weights are now runtime-overridable with safe bounds, and regression coverage validates default fallback plus override/clamp behavior (`bm25_weights_from_resolver_*`) for faster benchmark calibration loops.
- 2026-04-19 17:10:00 -04:00 | 70e24a9 | Desktop dev-runtime hygiene: predev cleanup now prunes stale `~/.cortex/runtime/control-center-dev/session-*` wrapper directories and kills wrapper-owned stale processes before launch, reducing repeated ghost runtime artifacts.
- 2026-04-19 17:03:00 -04:00 | local graphify refresh | Regenerated `graphify-out/daemon-rs` via `.graphify/build_daemon_graph.py` (1620 nodes, 3483 edges, 46 communities) and reviewed `GRAPH_REPORT.md`; no new cross-file surprise-risk edges were surfaced in this AST-heavy pass.
- 2026-04-19 16:48:00 -04:00 | 7461e85 | sqlite-vec primary-route hardening: recall route promotion now hydrates shadow-only sources from persisted memory/decision rows (including id/context fallbacks), preserves deterministic baseline-tail order after shadow-first routing, and adds regression coverage for primary-mode shadow-only source inclusion.
- 2026-04-16 02:49:37 -04:00 | 95e215c | Reliability follow-up: stabilized cross-process global lock regression by testing explicit lock-path acquisition (avoids shared env races under parallel test threads) in daemon-rs/src/auth.rs.
- 2026-04-16 02:41:48 -04:00 | 9254c7c | Admin remediation coverage: added team-mode handler regressions for /admin/unowned, /admin/assign-owner, and /admin/set-visibility with direct DB effect assertions in daemon-rs/src/handlers/admin.rs.
- 2026-04-16 02:40:34 -04:00 | 214f253 | Reliability hardening: added cross-process global-lock contention regression, concurrent runtime-lock burst coverage, and dead spawn-parent claim rejection tests in daemon-rs/src/auth.rs + daemon-rs/src/main.rs.
- 2026-04-16 00:33:10 -04:00 | fc5ffa4 | Small task: hardened benchmark singleton-skip detection and env scrub in daemon-rs/tests/recall_benchmark.rs.
- 2026-04-16 00:33:24 -04:00 | 1ad69b3 | Small task: added allow_service_ensure short-circuit coverage for local spawn policy in daemon-rs/src/main.rs.
- 2026-04-16 00:33:48 -04:00 | d3d7c37 | Small task: added regression test asserting shadow gate holds on high rank-delta / low top1 match in daemon-rs/src/handlers/health.rs.
- 2026-04-16 01:18:36 -04:00 | 4cb8941 | Phase-2A hardening: normalized shadow status buckets and enforced per-metric sample sufficiency for vec0 shadow gating in daemon-rs/src/handlers/health.rs.
- 2026-04-16 01:19:42 -04:00 | ebcf704 | Benchmark hardening: expanded proxy explain sampling (k=8/pool_k=32), added pairwise-agreement + evaluated-query coverage gates, and made singleton skips fail-closed under CI in daemon-rs/tests/recall_benchmark.rs.
- 2026-04-16 01:21:54 -04:00 | 96c7942 | Reliability hardening: serialized daemon-spawn integration suites (mcp_rpc_headers/mcp_transport) to align tests with one-daemon policy and avoid false failures under parallel test threads.
- 2026-04-16 01:42:38 -04:00 | 335ea61 | Phase-2A tokenizer migration: added schema migration 012 to rebuild FTS with tokenize='porter unicode61', switched fresh-schema FTS tokenizer, and added migration/tokenizer regression coverage in daemon-rs/src/db.rs + daemon-rs/src/state.rs.
- 2026-04-16 01:44:50 -04:00 | c3b6300 | Phase-2A BM25 tuning: switched memory/decision FTS BM25 from inline literals to tuned constants (text-forward weighting for porter tokenizer) and added ranking regressions in daemon-rs/src/handlers/recall.rs.
- 2026-04-16 01:49:01 -04:00 | 854165e | Phase-2A embedding upgrade groundwork: added selectable all-MiniLM-L12-v2 profile, introduced bounded periodic re-embed passes (CORTEX_EMBED_BACKFILL_*), and surfaced profile/backlog notes in setup init output.
- 2026-04-16 01:53:04 -04:00 | 67553c9 | 12C closeout progress: synced OpenAPI spec to 0.5.0, documented /readiness + /recall/explain + /stats, and added daemon test guards for spec-version/path drift in daemon-rs/src/main.rs.


- 2026-04-16 03:16:48 -04:00 | ebab223 | Adapter conformance: added Python + TypeScript SDK request-shape regression suites and CI jobs to fail fast on SDK drift.
- 2026-04-16 03:33:52 -04:00 | 025a91d | Phase-2A sqlite-vec canary: added guarded semantic trial routing (sampled, fail-closed, force-off), per-request route telemetry in recall events/responses/explain, and runtime env config via CORTEX_SQLITE_VEC_TRIAL_*.

- 2026-04-16 03:53:31 -04:00 | 94c90d7 | Prompt injector hardening: strict CLI value validation, true <file>.injected output suffixing, URL-safe boot query construction, higher-resolution watch change detection, and non-global token-reader test coverage.

- 2026-04-16 04:20:50 -04:00 | fd49244 | Reliability + portability guards: added health runtime-path home-scope integration coverage, concurrent attach-only app-client policy regression, and removed developer-specific daemon fixture paths from source tests.

- 2026-04-16 04:52:11 -04:00 | f60a2d1 | Hardened scripts/clean_install_smoke.sh for out-of-box reliability: LF-safe bash bootstrap, cargo.exe fallback on Windows bash, fail-closed search error handling, graphify/target scan exclusions, ASCII status output, and optional README enforcement via CORTEX_ENFORCE_PUBLIC_README.
- 2026-04-16 05:31:27 -04:00 | 77c175d | One-daemon hardening: improved control-center lock contention detection (including Windows lock codes 32/33), enforced lock-aware attach-only gating in ensure_daemon, and added debug/test-only single-daemon bypass path for daemon integration suites; validated with full daemon test/clippy/benchmark/audit stack.

- [2026-04-16] commit fba87af test(benchmarking): add cortex adapter contract coverage
  - Added adapter contract test suite for Cortex HTTP client (health/store/recall/metrics/reset).

- [2026-04-16] commit b236c21 test(lifecycle): harden preflight lock gating and deterministic startup checks
  - Added deterministic startup preflight tests (canonical-ready, non-canonical payload, runtime-identity mismatch) and lock-snapshot gating to enforce app-held attach-only behavior under slow health probes.

- [2026-04-16] commit 4b057ff chore(smoke): harden fresh-install and one-daemon release checks
  - Hardened fresh-install + release smoke scripts with one-daemon duplicate-start assertions, user-home path scoping checks, and strict spawn-path audit gating.

- [2026-04-16] commit a09a5db test(admin): expand ownership remediation regression coverage
  - Added explicit remediation endpoint coverage for unowned backfill, table allowlist rejection, and empty-visibility no-op behavior.
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
  - Implemented OpenAI function adapter execution layer over Cortex HTTP (health, store, recall) with strict argument validation and dedicated pytest contract coverage.

- [2026-04-16 13:07:34 -04:00] commit 300ca30 test(ci): enforce chrome extension and adapter contract suites
  - Added CI jobs for benchmark adapter pytest coverage and Chrome extension manifest/core tests to prevent adapter and extension contract drift before merge.

- [2026-04-16 13:18:58 -04:00] commit ede6634 test(lifecycle): verify orphan watcher against real parent exit
  - Added process-level regression proving spawn-parent orphan watcher observes a real parent-process exit and triggers daemon shutdown signaling, tightening one-daemon parent-death coverage.


## 2026-04-16 13:41 - 5eeb1ad
- Added daemon POST /recall support and wired extension recall traffic to body-based requests to avoid query-string leakage of recall prompts.
- Hardened Chrome extension Web Store posture: loopback-only host model, removed wildcard optional host permissions, session-default API key storage, and opt-in page metadata capture.
- Added extension privacy policy draft and refreshed policy-compliance references to official Chrome documentation.
- Validation:
  - node --test extensions/cortex-chrome-extension/tests/core.test.mjs
  - rtk cargo fmt
  - rtk cargo test
  - rtk cargo clippy -- -D warnings
  - rtk cargo test --test recall_benchmark -- --nocapture

## 2026-04-16 13:46 - eb11ef0
- Added `tools/validate_chrome_extension_policy.py` to enforce MV3 Web Store guardrails (loopback-only hosts, no wildcard optional hosts, no remote scripts/eval patterns, required policy docs present).
- Wired the guardrail into .github/workflows/ci.yml under chrome-extension-validation.
- Validation: python tools/validate_chrome_extension_policy.py; node --test extensions/cortex-chrome-extension/tests/core.test.mjs; python tools/audit_spawn_paths.py --strict.

## 2026-04-16 16:25 - 32f955b
- Synced release-facing docs for v0.5 closeout in tracked Info/ surfaces:
  - Info/roadmap.md now reflects v0.5 stabilization closeout status and remaining release-doc targets.
  - Info/team-mode-setup.md replaced insecure broad-bind examples with security-first deployment matrix.
  - Added Info/startup-matrix-troubleshooting.md as one-daemon startup truth + failure triage playbook.
  - Linked startup matrix from Info/connecting.md.
- Closeout impact: directly addresses unified-plan blockers for roadmap/release-facing sync and startup/troubleshooting refresh.

## 2026-04-16 16:26 - 80dab99
- Hardened tracked public security guidance for team-mode transport boundaries in Info/security-rules.md:
  - non-loopback binds now explicitly require TLS on public/routed interfaces
  - HTTP-only path explicitly limited to private encrypted mesh interfaces
  - deployment recommendations tightened to prevent accidental raw internet exposure
- Closeout impact: reinforces docs/security contract alignment for v0.5 release surfaces.
- 2026-04-16 18:04:32 -04:00 | f525244 | Security hardening closure: resolved all 13 findings across daemon auth boundaries, owner scoping, SDK token handling, plugin secret transport, desktop ownership semantics, benchmark path neutrality, extension protocol consistency, and script safety.
  - Team-mode destructive endpoints now require admin + rated auth (including admin surfaces) and no longer permit member/global destructive actions.
  - Team data paths now fail closed on missing caller identity and are owner-scoped across conflict/dedup/merge/feed flows.
  - Auth guard ordering/rate-limit coverage expanded to protected handlers (SSRF header gate before token verification, request/auth-failure counters enforced).
  - Validation: rtk cargo fmt; rtk cargo test (345 pass); rtk cargo clippy -D warnings; rtk cargo test --test recall_benchmark (7 pass); node plugins/cortex-plugin/scripts/run-mcp.contract.test.cjs; npm test (sdks/typescript); python -m pytest (sdks/python); node --test extensions/cortex-chrome-extension/tests/*.test.mjs.
- 2026-04-16 18:52:10 -04:00 | f6f878d | Phase-2A retrieval calibration: query-shape adaptive semantic budgeting + adaptive fallback ranking.
  - Semantic budget packing now adapts relevance floor, max-items, and excerpt caps by query shape (exact identifiers vs broad natural-language prompts).
  - Memory/decision fallback ranking now uses query-shape adaptive keyword/score/recency/retrieval weighting in both FTS and non-FTS paths.
  - Empty-term fallback sorting now prioritizes retained score signal before pure recency.
  - Validation: rtk cargo fmt; rtk cargo test (351 pass); rtk cargo clippy -- -D warnings; rtk cargo test --test recall_benchmark -- --nocapture (7 pass).
- 2026-04-16 19:06:00 -04:00 | 0b41668 | Benchmark guardrail expansion: stronger distilled-proxy and token-accounting regression gates.
  - `recall_benchmark_regression_thresholds_hold` now enforces top-1 hit rate, recall coverage, p95 tokens, and tokens-per-relevant-hit thresholds.
  - `distilled_proxy_tracks_full_recall_ranking` now asserts explain-token accounting consistency (`spent` == returned token sum, pre-budget drop math) and required ranking-factor fields.
  - Validation: rtk cargo fmt; rtk cargo test (351 pass); rtk cargo clippy -- -D warnings; rtk cargo test --test recall_benchmark -- --nocapture (7 pass).
- 2026-04-16 23:08:00 -04:00 | 3d75bdd | Phase-2A benchmark/runtime hardening: AMB dataset-compat shims plus prompt-ready SDK recall context helpers.
  - Added runtime-safe AMB dataset shims in `benchmarking/run_amb_cortex.py` so pinned datasets that do not accept `user_ids` still work (with best-effort user filtering), and LongMemEval prompt construction always keeps retrieved context as primary while appending compact retrieval metrics.
  - Added Python and TypeScript SDK helpers (`format_recall_context`/`recall_for_prompt`, `formatRecallContext`/`recallForPrompt`) to keep memory excerpts content-first and optionally append compact retrieval metrics for token-efficient prompt assembly.
  - Added regressions in `benchmarking/adapters/tests/test_run_amb_cortex_shims.py`, `sdks/python/tests/test_client.py`, and `sdks/typescript/test/client.test.mjs`.
  - Validation: `python -m pytest benchmarking/adapters/tests/test_run_amb_cortex_shims.py sdks/python/tests/test_client.py -q`; `npm --prefix sdks/typescript test`; `python benchmarking/run_amb_cortex.py smoke`.
- 2026-04-16 23:19:00 -04:00 | f6016f4 | Phase-2A benchmark operations: promoted multi-dataset AMB evaluation matrix to first-class runner workflow.
  - Added `matrix` command to `benchmarking/run_amb_cortex.py` with JSON case loading, per-case isolated run orchestration, deterministic summary emission, dry-run mode, and continue-on-error controls.
  - Added tracked stage-1 matrix spec `benchmarking/configs/amb-eval-matrix.stage1.json` covering LongMemEval, LoCoMo, MemBench splits, PersonaMem splits, MemSim splits, LifeBench, and BEAM.
  - Added unit coverage in `benchmarking/adapters/tests/test_run_amb_cortex_matrix.py` for matrix-schema validation, per-case argument synthesis, and summary extraction behavior.
  - Validation: `python -m pytest benchmarking/adapters/tests/test_run_amb_cortex_shims.py benchmarking/adapters/tests/test_run_amb_cortex_matrix.py sdks/python/tests/test_client.py -q`; `npm --prefix sdks/typescript test`; `python benchmarking/run_amb_cortex.py matrix --dry-run --matrix-file benchmarking/configs/amb-eval-matrix.stage1.json`; `python benchmarking/run_amb_cortex.py matrix --matrix-file benchmarking/configs/amb-eval-matrix.stage1.json --query-limit 5 --continue-on-error --no-enforce-gate --allow-missing-recall-metrics --summary-file benchmarking/runs/matrix-summary-latest-q5.json` (clean fail-fast across all cases due missing provider key).
- 2026-04-17 06:45:00 -04:00 | 373dce0 | Phase-2A power-user performance + benchmark stability hardening.
  - Semantic/shadow retrieval now pushes `source_prefix` into SQL filtering for memories/decisions before vector scoring, reducing scoped-recall scan cost on large corpora.
  - Semantic/shadow candidate collectors now stream query rows instead of collecting full intermediate vectors first.
  - Added scoped-prefix regressions in `daemon-rs/src/handlers/recall.rs` to guard semantic and shadow-semantic isolation behavior.
  - Embedding engine session pool is now runtime-configurable (`CORTEX_EMBED_SESSION_POOL_SIZE`, default `2`, clamp `1..8`) to reduce startup overhead while preserving throughput control for power users.
  - AMB provider concurrency now defaults to `1` (env override available) with regression coverage to reduce local-daemon burst 429s during fair matrix execution.
  - Validation: `rtk python -m pytest benchmarking/adapters/tests/test_run_amb_cortex_shims.py sdks/python/tests/test_client.py -q`; `rtk npm --prefix sdks/typescript test`; `rtk cargo test --manifest-path daemon-rs/Cargo.toml embeddings::tests -- --nocapture`; `rtk cargo test --manifest-path daemon-rs/Cargo.toml handlers::recall::tests -- --nocapture`; `rtk cargo test --manifest-path daemon-rs/Cargo.toml --test recall_benchmark -- --nocapture`.
- 2026-04-17 13:52:41 -04:00 | 8b3ab47 | Phase-2A retrieval precision + single-run timeout hardening batch.
  - `benchmarking/run_amb_cortex.py` now enforces single-run hard runtime caps via `--max-runtime-seconds` (15-20 minute guardrail, env override `CORTEX_BENCHMARK_RUN_MAX_RUNTIME_SECONDS`) and executes run cases through a hard-kill timeout worker path.
  - Matrix/runtime controls were tightened for consistent cap handling (`start_index`, `max_cases`, matrix/global and per-case runtime caps) while preserving fair-run behavior.
  - `benchmarking/adapters/cortex_http_client.py` now applies stronger query-intent/detail-aware reranking (date/location/speed/item), penalizes low-signal assistant guidance snippets, retries transient HTTP failures, and keeps user-scoped source-prefix fanout explicit.
  - `benchmarking/adapters/cortex_amb_provider.py` now expands high-signal fact extracts from conversation payloads with stronger filtering of generic assistant advice and configurable full-doc/fact-only storage controls.
  - Added/expanded regression coverage in:
    - `benchmarking/adapters/tests/test_run_amb_cortex_shims.py`
    - `benchmarking/adapters/tests/test_run_amb_cortex_matrix.py`
    - `benchmarking/adapters/tests/test_cortex_http_client.py`
  - Validation:
    - `rtk python -m pytest benchmarking/adapters/tests/test_run_amb_cortex_shims.py sdks/python/tests/test_client.py -q` (26 passed)
    - `rtk python -m pytest benchmarking/adapters/tests/test_cortex_http_client.py benchmarking/adapters/tests/test_run_amb_cortex_matrix.py -q` (21 passed)
    - `rtk npm test` in `sdks/typescript` (6 passed)
    - `rtk cargo test --manifest-path daemon-rs/Cargo.toml rate_limit -- --nocapture` (6 passed, 353 filtered)
  - Scored LongMemEval rerun attempt under strict cap (`--max-runtime-seconds 1200`) failed fast because no answer/judge provider key was configured in-shell (`GEMINI_API_KEY`/`GOOGLE_API_KEY`/`OPENAI_API_KEY`/`GROQ_API_KEY` missing).

## 2026-04-17 21:30 - Phase-2A fair-run preflight + real-score validation batch
- Added explicit fair-run preflight artifacts for both `run` and `matrix` commands in `benchmarking/run_amb_cortex.py`:
  - writes `fair-run-preflight.json` per run directory,
  - prints checklist output before execution,
  - aborts on fairness violations before benchmark execution starts.
- Closed a fairness gap in matrix mode by rejecting default CLI shortcut inputs (`--query-id`, `--doc-limit`) in addition to case-level checks.
- Strengthened LongMemEval answer-format shim for location questions to require country/state qualifiers when present in context.
- Validation completed:
  - `rtk python -m pytest benchmarking/adapters/tests/test_run_amb_cortex_shims.py sdks/python/tests/test_client.py -q` (43 passed)
  - `rtk python -m pytest benchmarking/adapters/tests/test_cortex_http_client.py benchmarking/adapters/tests/test_run_amb_cortex_matrix.py -q` (49 passed)
  - `rtk npm test` in `sdks/typescript` (6 passed)
- Strict capped scored runs (no oracle, no query pinning, no doc-limit shortcut):
  - `benchmarking/runs/amb-run-20260417-210951`: `19/20`, preflight passed, gate failed on avg recall tokens (`234.15 > 213.20`).
  - `benchmarking/runs/amb-run-20260417-211640`: `20/20`, preflight passed, gate passed, avg recall tokens `200.6`, max `250`, total `4012`.
- Expanded real benchmark participation beyond LongMemEval:
  - `benchmarking/runs/amb-matrix-20260417-212238` (`amb-eval-matrix.nonlongmem.q5.json`, first 2 cases) ran under strict preflight (passed) and produced baseline failures on `locomo` and `lifebench` under strict gates.

## 2026-04-18 15:40 - Control Center startup timeout + locked dev-binary hardening batch
- Desktop launch/runtime reliability updates (`desktop/cortex-control-center`):
  - App-managed spawn now forces loopback bind (`127.0.0.1`) in `src-tauri/src/main.rs` for local Control Center ownership path.
  - Dashboard refresh sequencing in `src/App.jsx` now runs staged fetches (health/core first, secondary panels after readiness), preventing parallel timeout cascades when daemon warmup is still running.
  - IPC timeout budgets in `src/api-client.js` now use aligned abort/transport timing (`transport = max(500, abort - 500)`), with explicit `/health` budget at `12000ms`.
  - Timeout-classification in app UI now treats IPC timeout conditions as daemon-unavailable recovery state, so app retry UX triggers correctly.
- Daemon startup responsiveness updates (`daemon-rs/src/main.rs`):
  - Added app-managed owner-sensitive startup deferral/staggering so heavy background maintenance (index, embedding backfill, aging, crystallization) does not starve first-load API responsiveness.
  - Added regressions asserting startup heavy-delay policy only applies to Control Center app-managed owner flow.
- Dev workflow lock resilience updates (`desktop/cortex-control-center/scripts/ensure-daemon-dev-binary.mjs`):
  - Script now detects stale daemon sources and rebuilds dev daemon binary when stale/missing.
  - Lock-aware retry path now detects Windows locked executable failures, stops only the locked dev daemon binary, and retries build once.
- Validation:
  - `rtk npm test -- api-client.test.js` (38 passed)
  - `rtk npm run web:build`
  - `rtk cargo check --manifest-path desktop/cortex-control-center/src-tauri/Cargo.toml`
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml app_managed_startup_heavy_delay_only_applies_to_control_center_owner`

## 2026-04-18 16:20 - Phase-2A startup analytics + benchmark posture optimization pass
- Event-volume controls:
  - `GET /savings` now computes rollups with SQL-side aggregation and short-lived payload caching (`SAVINGS_CACHE_TTL_SECS`) instead of full ordered `events` row parsing in Rust.
  - `/health` heavy metrics now use cached snapshots (`HEALTH_HEAVY_CACHE_TTL_SECS`) with explicit source tagging (`live`, `cache`, `warmup-deferred`) so startup reads avoid repeated expensive metrics work.
  - Savings/store telemetry accounting now explicitly includes decision event families (`decision_stored`, `decision_supersede`, `decision_conflict`, `decision_rejected_duplicate`) for volume-aware diagnostics.
- Compaction caps:
  - Benchmark ingestion now enforces per-document store compaction caps via `CORTEX_BENCHMARK_STORE_MAX_CHARS` (default `12000`) with deterministic chunked store-part tagging.
  - Phase-2A matrix profiles tighten extraction/context ceilings (`CORTEX_BENCHMARK_MAX_FACT_EXTRACTS_PER_DOC`, `CORTEX_BENCHMARK_FACT_EXTRACT_MAX_CHARS`, `CORTEX_BENCHMARK_CONTEXT_MAX_CHARS`, `CORTEX_BENCHMARK_QUERY_WINDOW_CHARS`) and set explicit retrieval-policy mode per case.
- Startup-timeout mitigation:
  - App-managed daemon startup now staggers index/aging/embed/crystallize passes for Control Center ownership using `CORTEX_APP_MANAGED_STARTUP_DELAY_SECS` with lane offsets, reducing cold-start timeout storms.
  - Daemon probe reads now accept partial timeout/would-block responses when bytes are already present, reducing false-negative startup probes under slow warmup.
- Benchmarking posture:
  - Fair-run preflight now fails closed for both single-run and matrix mode when gate-bypass shortcuts are requested (`no_enforce_gate`, `allow_missing_recall_metrics`).
  - Matrix preflight now inspects and rejects requested shortcut flags from the matrix spec payload before execution begins, preserving strict quality/token gate posture.
- Validation:
  - Follow-up regression `683e938` adds explicit `GET /savings` cache TTL coverage in `daemon-rs/src/handlers/health.rs`.

## 2026-04-18 17:10 - Control Center refresh coalescing + daemon read/compaction hardening pass
- Control Center refresh scheduling:
  - Added a global single-flight refresh queue in `desktop/cortex-control-center/src/App.jsx` and routed background/manual refresh triggers through it.
  - This prevents concurrent fanout overlap from mount interval + SSE + recovery retries, reducing 8s IPC timeout storms on cold start.
- Daemon read-path contention reductions:
  - `conductor`/`feed`/`mutate` read handlers now use `state.db_read`.
  - Removed write-side cleanup work from hot GET paths (expired locks/sessions now filtered in SQL query predicates instead of deleted during reads).
- Event-growth pressure controls:
  - `daemon-rs/src/compaction.rs` now enforces soft/hard non-boot event thresholds and per-event-type caps before/within compaction passes.
  - Added regression coverage for event-pressure trigger, per-type cap pruning, and global non-boot overflow pruning while preserving boot events.
- Startup query acceleration:
  - Added targeted indexes in `daemon-rs/src/db.rs` for startup-heavy surfaces (`/sessions`, `/locks`, `/tasks`, `/feed`, `/messages`, `/activity`, `/events`) including owner-scoped variants.
- Validation:
  - `rtk cargo test` in `daemon-rs` with isolated target dir (`367 passed`).
  - `rtk cargo test compaction` (`11 passed`).
  - `rtk cargo check --release` in `daemon-rs`.
  - `rtk npm --prefix desktop/cortex-control-center run web:build` (passes; chunk-size warning only).

## 2026-04-18 18:20 - Startup timeout-storm hardening + app-managed delay guard batch
- Control Center startup reliability:
  - Startup now hydrates core routes first (`/sessions`, `/locks`, `/tasks`) and defers secondary routes (`/feed`, `/messages`, `/activity`, `/conflicts`, `/permissions`) to non-blocking background refresh.
  - Secondary route timeout failures now surface partial availability instead of flipping the app into global-offline state when core daemon connectivity is healthy.
- App-managed daemon binary safety:
  - Desktop runtime now rejects test-artifact daemon paths (`target-tests`, `target-test`, `nextest`, `target*/deps`) to prevent accidental startup from non-runtime binaries.
- Daemon startup/runtime contention reductions:
  - Replaced single query-read mutex path with bounded pooled read connections (`CORTEX_DB_READ_POOL_SIZE`).
  - Added bounded background DB lock waits (`CORTEX_BACKGROUND_DB_LOCK_MAX_WAIT_MS`) so non-critical startup tasks skip instead of queuing behind hot locks.
  - Added startup-safe storage-governor catch-up pass (no VACUUM) and clamped app-managed heavy startup delay to `120s` max.
  - Stabilized startup preflight fixture tests by disabling live IPC endpoint use in those tests.
- Validation:
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml` (`374` passing)
  - `rtk cargo clippy --manifest-path daemon-rs/Cargo.toml -- -D warnings` (pass)
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml --test recall_benchmark -- --nocapture` (`7` passing)
  - `rtk cargo test --manifest-path desktop/cortex-control-center/src-tauri/Cargo.toml` (`16` passing)
  - `rtk npm test` in `desktop/cortex-control-center` (`57` passing)
  - `rtk npm run web:build` in `desktop/cortex-control-center` (pass)
  - `rtk python benchmarking/run_amb_cortex.py matrix --dry-run --matrix-file benchmarking/configs/amb-eval-matrix.nonlongmem.q5.json` (strict fair-run preflight passed)
  - `rtk python benchmarking/run_amb_cortex.py matrix --matrix-file benchmarking/configs/amb-eval-matrix.nonlongmem.q5.json --continue-on-error --summary-file benchmarking/runs/matrix-summary-latest-q5.json` (expected fail-fast: missing answer/judge provider keys)

## 2026-04-18 18:45 - Savings telemetry realism + benchmark-bloat cleanup batch
- Live DB remediation (backup-first) completed on local operator database:
  - backup: `~/.cortex/backups/cortex-pre-amb-cleanup-20260418-181602.db`
  - removed benchmark namespace rows (`amb-cortex` and `amb-cortex::%`) across decisions + linked decision embeddings + JSON-tagged events
  - before/after:
    - events: `433,479 -> 1,133`
    - decisions: `31,185 -> 489`
    - decision embeddings: `11,946 -> 483`
  - purged legacy `boot_savings` rows generated under old baseline math (`441` rows), bringing total events to `692` and clearing inflated historical savings totals
- Compiler savings accounting hardening:
  - `daemon-rs/src/compiler.rs` baseline estimator now uses active memories + active decisions only (no custom-source filesystem-size heuristic inflation)
- Validation:
  - protected endpoint probe (auth-bound): `/sessions`, `/locks`, `/tasks`, `/feed`, `/messages`, `/activity`, `/conflicts`, `/permissions`, `/savings` returned `200` in local checks
  - `rtk cargo check --manifest-path daemon-rs/Cargo.toml` (pass)
  - `rtk cargo clippy --manifest-path daemon-rs/Cargo.toml -- -D warnings` (pass)
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml --test recall_benchmark -- --nocapture` (`7` passing)

## 2026-04-21 22:10 - Release hardening + security/autonomy closeout (v0.5.0)
- Completed full portability hardening for developer-specific paths/user markers in runtime-adjacent code/tests/scripts:
  - removed `C:/Users/aditya` / `C:\Users\aditya` literals from desktop tauri tests + mock server + benchmark artifact strings.
  - generalized clean-install grep guards to user-agnostic home-path regexes.
  - made `scripts/prune-build-bloat.ps1` default root dynamic (`$PSScriptRoot/..`) instead of developer-local absolute path.
- Completed security + reliability release checks:
  - release gate required tests all passing:
    - `rtk cargo test --manifest-path daemon-rs/Cargo.toml recall::tests:: -- --nocapture`
    - `rtk cargo test --manifest-path daemon-rs/Cargo.toml handlers::mcp::tests:: -- --nocapture`
    - `npm test` in `desktop/cortex-control-center`
  - additional verification passes:
    - `rtk cargo test --manifest-path desktop/cortex-control-center/src-tauri/Cargo.toml`
    - `rtk cargo clippy --manifest-path daemon-rs/Cargo.toml -- -D warnings`
    - `bash scripts/clean_install_smoke.sh` (`GATE: PASSED`)
- Completed benchmark smoke validation (isolated benchmark daemon):
  - `CORTEX_SINGLE_DAEMON_TEST_BYPASS=1 python benchmarking/run_amb_cortex.py smoke`
  - latest run dir: `benchmarking/runs/amb-smoke-20260421-220511`
- Security audit closeout:
  - `npm audit` reports `0` vulnerabilities in Control Center.
  - `cargo audit` shows `0` vulnerabilities for daemon + tauri lockfiles (remaining entries are allowed warnings: unmaintained/unsound ecosystem advisories).
  - `detect-secrets` scan still reports known false-positive classes (benchmark commit hashes, test fixtures, updater pubkey); no live credential leakage detected.
- Repo policy hardening:
  - added GitHub workflow `.github/workflows/security-hygiene.yml` with:
    - `gitleaks` scanning
    - hardcoded user-home path policy checks
    - tracked `.env` policy checks
    - benchmark visibility policy checks (`benchmarking/results` public, `benchmarking/runs` ignored).
- Bloat/deadcode action:
  - removed duplicate unused generated schema file:
    - `desktop/cortex-control-center/src-tauri/gen/schemas/windows-schema.json`
