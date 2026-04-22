# README Update Queue (Living)

Last updated: 2026-04-20 19:14  
Scope baseline: `v0.4.1` -> current `HEAD`

## Purpose
This is the staging queue for README updates that should be considered for the next release.
It is intentionally broader than final README copy. Use this file to decide what ships publicly.

## How To Use
1. Keep this file additive while work continues.
2. For each item, keep at least one concrete commit reference.
3. When drafting final README copy, move only validated, externally appropriate content.

## Candidate README Additions

### 0.0u) Browser-Harness Runtime Validation Workflow (internal-only, do not publish)
- Internal operator-tooling note:
  - browser-harness is now the preferred Codex operator path for local runtime verification loops.
  - this is intentionally not a Cortex product/repo feature (the in-repo harness probe was removed in follow-up scope correction).
- Validation references:
  - baseline report: `desktop/cortex-control-center/runtime-artifacts/baseline-harness-2026-04-20T17-30-00/metrics.json`
  - after report: `desktop/cortex-control-center/runtime-artifacts/after-harness-final-r2-2026-04-20T17-49-35/metrics.json`
- Commit:
  - `c79e073`
  - `a464251`

### 0.0v) Brain Panel Offscreen Runtime Cost Reduction (new)
- Add a desktop runtime-performance note:
  - Brain panel now stays mounted after first visit, avoiding remount/teardown spikes on tab churn.
  - hidden Brain panel now explicitly pauses animation and pointer interaction work.
  - Brain visualizer is memoized to reduce rerenders when non-Brain surfaces poll/update.
- Validation references:
  - `npm test` in `desktop/cortex-control-center` (`89` passing)
  - runtime tab-switch aggregate improved (`182.15ms` -> `167.84ms`, same machine/session harness capture)
- Commit:
  - `735deb9`

### 0.0w) Recall Relevance + Determinism Hardening (new)
- Add a retrieval-quality note:
  - recall ranking now applies recency-aware weighting for latest/current intent and adds query-alignment boost for source/excerpt relevance.
  - comparator tie behavior is more stable with explicit non-finite relevance handling.
  - served-result dedup hot path now reduces lock churn to improve high-frequency recall responsiveness.
  - synonym expansion now includes user-memory phrasing aliases (`lastname/surname`, `color/colour`, `gray/grey`, etc.).
- Validation references:
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml recall::tests::` (`109` passing)
- Commit:
  - `4223be2`

### 0.0x) Recall Hot-Path CPU Reduction (new)
- Add a retrieval-performance note:
  - fallback ranking now avoids repeated per-comparison alignment rescoring by reusing precomputed alignment metadata.
  - query excerpt focusing now precomputes focus terms once per query in fallback/FTS candidate paths.
  - family-compaction candidate preference now reuses query-alignment profile instead of reparsing query terms each compare.
- Validation references:
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml recall::tests::` (`109` passing)
- Commit:
  - `154c884`
  - `c38cf3e`
  - `581ee46`

### 0.0y) Daemon Runtime + Release Footprint Tuning (new)
- Add a runtime-performance note:
  - SQLite runtime now supports bounded mmap/cache tuning via env and uses in-memory temp-store for heavy query paths.
  - release profile now sets `panic = "abort"` for smaller/faster daemon release binaries.
- Validation references:
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml configure -- --nocapture` (`4` passing)
  - `rtk cargo check --manifest-path daemon-rs/Cargo.toml --release` (pass)
- Commit:
  - `3bdd9b1`
  - `9516892`

### 0.0z) Recall Budget Efficiency + Answer-Precision Hardening (new)
- Add a recall-quality/perf note:
  - budget packing now suppresses near-duplicate candidates unless they add fresh query-term coverage.
  - recall snippet extraction now prioritizes `[user-answer]` spans in QA-style memories to improve concrete answer precision.
  - default `/recall` budget (when no mode/budget is provided) now adapts to query shape + requested `k`, reducing spend on short exact queries while preserving headroom for broader natural prompts.
  - budget packing now early-stops when query-term coverage is already met and budget pressure is high.
- Validation references:
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml recall::tests::` (`116` passing)
  - `rtk cargo check --manifest-path daemon-rs/Cargo.toml --release` (pass)
- Commit:
  - `99a8d27`
  - `5b4a2f4`
  - `5cafbd7`

### 0.0aa) Benchmark Base-Adapter Personal Recall Determinism (new)
- Add a benchmark-readiness note:
  - `cortex-http-base` personal-memory retrieval query shaping now better handles name-change, item/color repaint, and location-qualifier cases.
  - strict budget adherence in the base adapter now keeps detail-query variants under configured budget while improving specificity ranking.
- Validation references:
  - `pytest benchmarking/adapters/tests/test_cortex_http_base_provider.py -q` (`17` passing)
- Commit:
  - `e4fcab3`

### 0.0ab) Route-Class Rate-Limit Isolation For Recall Availability (new)
- Add an operational reliability note:
  - daemon request limiter now isolates `store` and `recall` request buckets per client IP.
  - high-volume `/store` bursts no longer consume `/recall` request capacity in the same minute window.
  - optional per-class tuning env vars now exist for advanced operators:
    - `CORTEX_RATE_LIMIT_RECALL_REQUESTS_PER_MIN`
    - `CORTEX_RATE_LIMIT_RECALL_LOOPBACK_REQUESTS_PER_MIN`
    - `CORTEX_RATE_LIMIT_STORE_REQUESTS_PER_MIN`
    - `CORTEX_RATE_LIMIT_STORE_LOOPBACK_REQUESTS_PER_MIN`
- Validation references:
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml rate_limit::tests::` (`7` passing)
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml handlers::store::tests:: -- --nocapture` (`21` passing)
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml recall::tests::` (`116` passing)
- Commit:
  - `0a59e60`

### 0.0ac) Token Usage/Savings Visibility Across Cortex Function Surfaces (new)
- Add an observability note:
  - Cortex tool payloads returned via MCP `tools/call` now consistently include `tokenUsage` and `tokenUsageLine` fields.
  - Boot payloads now expose explicit `used/saved/budget` visibility in both MCP and HTTP `/boot` paths.
  - HTTP `/peek` now includes token usage plus estimated savings vs full-excerpt recall output.
  - recall budget responses now report stable `overBudget` + usage-line metadata through a shared budget-invariant path.
- Validation references:
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml tools_call_includes_token_usage_line_for_cortex_tools -- --nocapture` (`1` passing)
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml handlers::mcp::tests:: -- --nocapture` (`21` passing)
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml recall::tests:: -- --nocapture` (`119` passing)
  - `npm test` in `desktop/cortex-control-center` (`89` passing)
- Commit:
  - `5978c81`
  - `1b158d8` (headlines/peek token accounting accuracy follow-up)

### 0.0ad) MCP Call-Path Allocation Reduction (new)
- Add a daemon runtime note:
  - MCP `tools/call` hot path no longer rebuilds/parses the full tool schema list for known-tool validation.
  - validation now uses direct known-tool matching via the existing permission map.
  - reduces per-call allocation/JSON traversal overhead in frequent desktop↔daemon interactions.
- Validation references:
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml handlers::mcp::tests:: -- --nocapture` (`21` passing)
- Commit:
  - `8629c4f`

### 0.0t) Throughput Metric Window Consistency (new)
- Add a short analytics-telemetry consistency note:
  - Throughput headline now uses a deterministic rolling 7-day boot count.
  - metric copy is explicit (`7d Boot Compilations`) to avoid ambiguity with 30-day aggregates.
  - Boots Per Day card now labels total as `last 30d` for clear window semantics.
- Validation references:
  - `rtk npm --prefix desktop/cortex-control-center test` (`85` passing)
  - `rtk npm --prefix desktop/cortex-control-center run web:build` (pass)
- Commit:
  - `52fc17a`

### 0.0s) Analytics Metric Semantics Cleanup (new)
- Add a short analytics-UX integrity note:
  - Economic value card no longer references recall quality in its footnote.
  - recall hit-rate language is now isolated to the Recall Quality health box.
  - keeps financial and quality telemetry narratives separated for operator clarity.
- Validation references:
  - `rtk npm --prefix desktop/cortex-control-center test` (`85` passing)
  - `rtk npm --prefix desktop/cortex-control-center run web:build` (pass)
- Commit:
  - `11703b1`

### 0.0r) Analytics Economic Value Assumptions Legend (new)
- Add a short analytics-readability note:
  - Economic value now shows an inline assumptions legend in the card itself (not only in Mission Control legend surfaces).
  - estimate basis is explicit: `$15 USD per 1M tokens saved`.
  - non-USD display is conversion-adjusted to selected currency using release-pinned FX rates.
- Validation references:
  - `rtk npm --prefix desktop/cortex-control-center test` (`85` passing)
  - `rtk npm --prefix desktop/cortex-control-center run web:build` (pass)
- Commit:
  - `744bcd4`

### 0.0j) Optional Multi-Device Sync Primitives (new)
- Add a concise sync-operations section for advanced users who want transport-agnostic cross-device workflows:
  - new CLI namespace: `cortex sync export|import|watch`.
  - `sync export` supports incremental windows with `--since` and persistent cursor checkpoints via `--cursor-file`.
  - `sync watch --dir <path>` supports opt-in folder-based import/export loops without requiring any Cortex cloud service.
  - watch state is stored locally under `~/.cortex/runtime/sync-watch`, preserving transport-agnostic behavior and avoiding shared-folder marker collisions.
- Validation references:
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml` (`418` passing)
  - `rtk cargo clippy --manifest-path daemon-rs/Cargo.toml -- -D warnings` (pass)
- Commit:
  - `1a03003`

### 0.0k) Activation + Idle Economics (new)
- Add an operations note for low-footprint deployments:
  - Unix socket activation support now adopts inherited listeners (`LISTEN_FDS`/`LISTEN_PID`) before attempting direct bind.
  - optional idle shutdown controls now exist for daemon economics:
    - `CORTEX_IDLE_SHUTDOWN_SECS`
    - `CORTEX_IDLE_SHUTDOWN_MIN_UPTIME_SECS`
  - runtime now tracks request activity directly and can perform graceful idle exits while preserving existing explicit shutdown paths.
- Validation references:
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml` (`418` passing)
  - `rtk cargo clippy --manifest-path daemon-rs/Cargo.toml -- -D warnings` (pass)
- Commit:
  - `656e9ad`

### 0.0l) Mission Control Metric Readability (new)
- Add a desktop UX/readability note:
  - Mission Control now includes an inline metric legend to explain units/labels.
  - token metrics can now toggle between compact and full-number modes for operator clarity.
  - Mission Control now also keeps an always-visible unit key (`t`, `K`, `M`, `B`, `T`, `t/day`) so operators can decode compact metrics without opening the legend dialog.
  - Clarify case-sensitive semantics in docs/UI examples: lowercase `t` means tokens; uppercase `T` means trillions.
- Validation references:
  - `rtk npm test` in `desktop/cortex-control-center` (`85` passing)
  - `rtk npm run web:build` in `desktop/cortex-control-center` (pass)
- Commit:
  - `b7a225b`
  - `55c13c3`

### 0.0m) Recall Policy Modes + Fail-Closed Latency Budgets (new)
- Add a reliability/performance section for retrieval policy controls:
  - recall now supports explicit policy modes: `headlines`, `fast`, `balanced`, `deep`.
  - mode selection applies mode-aware budget/`k` defaults consistently across HTTP and MCP recall paths.
  - hard latency budgets are enforced per mode with deterministic fail-closed fallback when budgets are exceeded.
- Document operator env controls for strict latency behavior:
  - `CORTEX_RECALL_HEADLINES_MAX_LATENCY_MS`
  - `CORTEX_RECALL_FAST_MAX_LATENCY_MS`
  - `CORTEX_RECALL_BALANCED_MAX_LATENCY_MS`
  - `CORTEX_RECALL_DEEP_MAX_LATENCY_MS`
- Validation references:
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml` (`422` passing at tranche validation time)
  - `rtk cargo clippy --manifest-path daemon-rs/Cargo.toml -- -D warnings` (pass)
- Commit:
  - `85d3ede`

### 0.0n) Temporal Semantics in Import/Export (new)
- Add a data-compatibility note for advanced operators:
  - memories/decisions now support temporal semantics fields: `observed_at`, `valid_from`, `valid_until`.
  - JSON full export and changeset export include temporal fields.
  - import path preserves temporal fields and normalizes entry type taxonomy for memories/decisions.
- Validation references:
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml export_data::tests -- --nocapture` (`3` passed)
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml` (`424` passed)
- Commit:
  - `833fb8d`

### 0.0o) Local Eval Regression Gate Workflow (new)
- Add an evaluation discipline note for local quality gating:
  - `cortex eval` now supports baseline-vs-current regression checks via:
    - `--baseline-file`
    - `--max-regression`
    - `--fail-on-regression`
  - snapshot output now includes task-family reliability metrics and memory-quality metrics suitable for CI/local gates.
- Validation references:
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml` (`423` passing at tranche validation time)
  - `rtk cargo clippy --manifest-path daemon-rs/Cargo.toml -- -D warnings` (pass)
- Commit:
  - `fbff1f5`

### 0.0p) Recall Temporal Validity Enforcement (new)
- Add a retrieval-correctness note for temporal memory semantics:
  - recall now enforces `valid_from`/`valid_until` windows (not just TTL `expires_at`) across keyword, semantic, shadow-semantic, and unfold retrieval paths.
  - rows scheduled for future validity or already outside validity windows are excluded from recall candidates.
- Validation references:
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml temporally_invalid -- --nocapture` (`3` passed)
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml handlers::recall::tests -- --nocapture` (`101` passed)
- Commit:
  - `8cffed5`

### 0.0q) Deterministic Recall Ordering (new)
- Add a deterministic retrieval behavior note:
  - recall candidate ordering now has explicit stable tie-break rules so equivalent-score runs return the same order.
  - weighted RRF output now uses deterministic id tie-break when fused scores match.
  - keyword tie resolution now prefers query-aligned excerpt evidence before lexical source fallback.
- Validation references:
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml handlers::recall::tests -- --nocapture` (`102` passed)
- Commit:
  - `04c9e2e`

### 0.0) Phase-2A Route + Re-embed Completion Controls (new)
- Add a focused Phase-2A progress note for rollout operators:
  - sqlite-vec semantic routing now supports explicit runtime modes (`baseline`, `trial`, `primary`) with fail-closed force-off behavior and route telemetry in recall/explain outputs.
  - sqlite-vec route promotion now hydrates shadow-only ranked sources from persisted memory/decision rows, so primary/trial mode can include vec0-only candidates without dropping to synthetic placeholders.
  - startup re-embed pass now avoids premature early-stop when one table still has full batches.
  - optional one-time startup backlog drain is now available via `CORTEX_EMBED_BACKFILL_DRAIN_ON_STARTUP=1` and `CORTEX_EMBED_BACKFILL_STARTUP_DRAIN_MAX_BATCHES`.
  - sqlite-vec shadow KNN SQL paths now use parameterized statements for vector literal + `k` bindings.
- Validation references:
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml --target-dir daemon-rs/target-codex-work backfill_batch_may_have_more_only_when_a_table_hits_limit` (`1` passing)
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml --target-dir daemon-rs/target-codex-work collect_unembedded_targets_for_model` (`2` passing)
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml --target-dir daemon-rs/target-codex-work maybe_apply_sqlite_vec_trial_` (`2` passing)
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml --target-dir daemon-rs/target-codex-work sqlite_vec_canary_config_route_mode_aliases_are_supported` (`1` passing)
  - `rtk cargo test maybe_apply_sqlite_vec_primary_includes_shadow_only_sources` (`1` passing)
  - `rtk cargo test` in `daemon-rs` with isolated target dir (`402` passing)
  - `rtk cargo clippy -- -D warnings` in `daemon-rs` with isolated target dir (pass)
- Commit:
  - `35a7cff`
  - `7461e85`

### 0.0a) BM25 Runtime Calibration Knobs (new)
- Add a short operator note for retrieval tuning workflows:
  - BM25 field weights are now runtime-overridable without rebuilds for faster benchmark iteration loops.
  - supported env controls:
    - `CORTEX_BM25_MEM_TEXT_WEIGHT`
    - `CORTEX_BM25_MEM_SOURCE_WEIGHT`
    - `CORTEX_BM25_MEM_TAGS_WEIGHT`
    - `CORTEX_BM25_DECISION_WEIGHT`
    - `CORTEX_BM25_CONTEXT_WEIGHT`
  - invalid/non-positive values fail safe to defaults, and all values are clamp-bounded (`0.1..12.0`) to avoid pathological configs.
- Validation references:
  - `rtk cargo test handlers::recall::tests::` (`97` passing)
  - `rtk cargo test` in `daemon-rs` with isolated target dir (`404` passing)
  - `rtk cargo clippy -- -D warnings` in `daemon-rs` with isolated target dir (pass)
- Commit:
  - `360c55b`

### 0.0b) Matrix Execution Resilience For Missing Dataset Prereqs (new)
- Add a benchmark-operations note for matrix users:
  - matrix execution now skips known missing dataset prerequisites per case and continues instead of aborting the entire matrix.
  - skip reasons are explicit in summary output so missing optional datasets are visible without losing full-run progress.
  - this keeps strict gate posture while removing avoidable all-or-nothing failures from partially provisioned benchmark environments.
- Validation references:
  - `rtk python -m pytest benchmarking/adapters/tests/test_run_amb_cortex_matrix.py benchmarking/adapters/tests/test_run_amb_cortex_shims.py -q` (`74` passing)
  - `rtk python -m pytest benchmarking/adapters/tests -q` (`155` passing)
- Commit:
  - `ed0995f`

### 0.0c) Embedding Migration Completion CLI (new)
- Add an operator workflow note for model-migration closeout:
  - new command: `cortex embeddings status [--json]` to show active-model backlog counts (`memories`, `decisions`, `total`).
  - new command: `cortex embeddings drain [--batch-size <n>] [--max-batches <n>] [--lock-wait-ms <n>] [--until-exhausted] [--max-iterations <n>] [--json]`.
  - this provides explicit deterministic re-embed completion controls, instead of relying only on startup/background passes.
- Validation references:
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml count_unembedded_targets_for_model_reports_model_specific_backlog -- --nocapture` (`1` passing)
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml --bin cortex` (`382` passing)
  - `rtk cargo clippy --manifest-path daemon-rs/Cargo.toml -- -D warnings` (pass)
  - `rtk cargo run --manifest-path daemon-rs/Cargo.toml -- embeddings status --json` (pass)
  - `rtk cargo run --manifest-path daemon-rs/Cargo.toml -- embeddings drain --json --max-batches 1` (pass)
- Commit:
  - `87b0b01`

### 0.0d) Phase-2A Retrieval Defaults Closeout (new)
- Add a short retrieval-defaults note for v0.5.0:
  - sqlite-vec route now defaults to `primary` while retaining guarded fail-closed baseline fallback when shadow gate blockers are present.
  - default embedding model is now `all-MiniLM-L12-v2` (legacy L6 remains explicitly selectable).
  - BM25 defaults received a second text-forward calibration pass while keeping runtime override knobs.
- Validation references:
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml --bin cortex sqlite_vec_canary_config_defaults_to_primary_route -- --nocapture` (`1` passing)
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml --bin cortex selected_model_defaults_to_minilm_modern -- --nocapture` (`1` passing)
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml --bin cortex selected_model_accepts_legacy_l6_aliases -- --nocapture` (`1` passing)
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml --bin cortex selected_model_accepts_l12_aliases -- --nocapture` (`1` passing)
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml --bin cortex unknown_model_falls_back_to_default -- --nocapture` (`1` passing)
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml --bin cortex bm25_weights_from_resolver_uses_defaults_when_env_missing -- --nocapture` (`1` passing)
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml --bin cortex bm25_weights_from_resolver_applies_overrides_and_clamps -- --nocapture` (`1` passing)
  - `rtk cargo clippy --manifest-path daemon-rs/Cargo.toml -- -D warnings` (pass)
- Commit:
  - pending (local closeout batch)

### 0.0e) Benchmark Matrix Cadence Command (new)
- Add benchmark-operations guidance for broader rerun cadence:
  - new command: `python benchmarking/run_amb_cortex.py cadence ...`
  - executes an ordered matrix-file sequence with per-matrix run directories plus `cadence-summary.json`.
  - supports `--continue-on-error` for matrix-level failures while preserving honest final non-zero exit when any matrix fails.
  - supports `--max-matrices` for bounded cadence slices.
- Validation references:
  - `rtk pytest benchmarking/adapters/tests/test_run_amb_cortex_matrix.py -vv` (`28` passing)
- Commit:
  - pending (local closeout batch)

### 0.0f) Event-Pressure Diagnostics + One-Time Cleanup Workflow (new)
- Add a power-user maintenance note for large event histories:
  - `cortex doctor` now reports event-pressure classification, non-boot event rows, `decision_stored` row counts, and top event-type contributors.
  - `cortex cleanup` now supports event-table remediation mode:
    - `--events` enables one-time event compaction cleanup.
    - `--dry-run` previews impact.
    - `--max-passes <n>` caps repeated governor passes.
  - this gives operators an explicit workflow to diagnose and reduce oversized historical event tables without ad-hoc SQL.
- Validation references:
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml event_ -- --nocapture` (`14` passing)
  - `rtk cargo clippy --manifest-path daemon-rs/Cargo.toml -- -D warnings` (pass)
- Commit:
  - pending (local optimization batch)

### 0.0g) Startup Recovery Bounded-Retry Guard (new)
- Add a startup reliability note for app-managed local mode:
  - startup recovery now keeps one bounded retry window even when daemon status temporarily drops `managed` during warmup polling.
  - this prevents indefinite “Daemon is still starting. Reconnect will continue automatically.” loops and ensures timeout/recovery escalation still triggers.
  - complements existing stale runtime wrapper cleanup to keep local dev startup stable over repeated runs.
- Validation references:
  - `rtk npm --prefix desktop/cortex-control-center run test -- src/daemon-startup.test.js src/analytics-projection.test.js src/number-format.test.js` (`21` passing)
  - `rtk node desktop/cortex-control-center/scripts/cleanup-dev-runtime.mjs` + runtime session-dir check (`session_dirs=0`)
- Commit:
  - pending (local optimization batch)

### 0.0h) Derived-State Repair Commands (new)
- Add an operator repair workflow note:
  - new `cortex reindex [--json]` command fully rebuilds FTS indexes from canonical memory/decision tables.
  - new `cortex re-embed` / `cortex reembed` aliases force `embeddings drain --until-exhausted` for deterministic model-migration completion.
  - new `cortex recrystallize [--json]` command clears and rebuilds crystal graph/embeddings in one pass.
  - reindex now uses FTS-native `delete-all` semantics to remove stale/orphan searchable rows before repopulating.
- Validation references:
  - `npm test` in `desktop/cortex-control-center` (`85` passing)
  - `cargo test --manifest-path daemon-rs/Cargo.toml reindex_fts -- --nocapture` with isolated target dir (`1` passing)
  - `cargo test --manifest-path daemon-rs/Cargo.toml --test mcp_transport -- --nocapture` with isolated target dir (`11` passing)
- Commit:
  - `c35603a`

### 0.0i) MCP Intelligence + Maintenance APIs (new)
- Add a daemon-MCP operations note for advanced agents/operators:
  - new admin tools: `cortex_consensus_promote`, `cortex_memory_decay_run`, `cortex_eval_run`.
  - `cortex_consensus_promote` auto-resolves open disputed decision pairs when trust margin is high enough (with `dryRun` and `minMargin` controls).
  - `cortex_memory_decay_run` allows explicit on-demand decay/aging/expiry maintenance passes instead of waiting for background cadence.
  - `cortex_eval_run` returns a reproducible horizon-based snapshot (`windowDays`) with conflict burden, decay burden, and resolution velocity signals.
- Validation references:
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml --target-dir daemon-rs/target-tests handlers::mcp::tests -- --nocapture` (`20` passed)
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml --target-dir daemon-rs/target-tests --test mcp_transport -- --nocapture` (`11` passed)
  - `rtk cargo check --manifest-path daemon-rs/Cargo.toml --target-dir daemon-rs/target-tests` (pass)
- Commit:
  - `639bc0a`

### 0.1) Desktop Startup + Analytics Readability Polish (new)
- Add a short reliability/readability section for the next desktop release notes:
  - Windows desktop app launch no longer flashes a ghost console window during Control Center startup.
  - local IPC failures that surface as raw transport errors (for example `Read failed ... os error 10060`) now fail over to HTTP automatically for dashboard routes.
  - startup reachability now accepts readiness/health fallbacks when daemon-status IPC probes false-negative during warmup, preventing repeated "still starting" loops.
  - startup/warmup/timed-out daemon banners are now treated as transient and auto-cleared once healthy connection state resumes.
  - Windows dev prebuild stale-daemon sweep no longer uses inherited console handles, reducing ghost terminal popups during local launch.
  - Monte Carlo horizon cards now avoid scientific-notation output and display compact values with bounded suffix formatting (K/M/B/T/Q).
  - sparse daily-history projection paths now clamp drift/volatility so short noisy windows do not explode into unrealistic projections.
  - predev runtime cleanup now removes stale `session-*` wrapper folders under `~/.cortex/runtime/control-center-dev` and kills wrapper-owned stale processes before launch.
  - app-managed startup now prefers local managed daemon ensure first, and only falls back to service ensure when explicitly enabled (`CORTEX_ALLOW_SERVICE_ENSURE_FALLBACK=1`).
  - stale managed-child daemon handles are now auto-cleared on poll/kill failure paths to avoid persistent "starting" loops after crashy/stale child state.
- Validation references:
  - `rtk npm --prefix desktop/cortex-control-center test` (`75` passing)
  - `rtk npm --prefix desktop/cortex-control-center test -- src/daemon-startup.test.js src/api-client.test.js src/analytics-projection.test.js src/number-format.test.js` (`61` passing)
  - `rtk npm --prefix desktop/cortex-control-center run verify:lifecycle:dev` (pass)
  - `rtk cargo test --manifest-path desktop/cortex-control-center/src-tauri/Cargo.toml` (`22` passing)
- Commits:
  - `4bde521`
  - `abfd03d`
  - `7abc919`
  - `70e24a9`
  - `a2828ad`
  - `156ef3c`
  - `794cebb`
  - `341cafd`

### 0.2) Startup Fanout Coalescing + Event-Pressure Controls (new)
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

### 0.2) Startup Timeout-Storm Hardening + App-Managed Delay Guard (new)
- Add a short startup reliability section for app-managed local mode:
  - Control Center startup now treats core routes (`/sessions`, `/locks`, `/tasks`) as first-load critical and defers secondary routes (`/feed`, `/messages`, `/activity`, `/conflicts`, `/permissions`) to non-blocking background refresh.
  - Secondary timeout failures no longer flip the dashboard into global-offline state when core daemon connectivity is healthy.
  - Control Center binary safety guard now rejects non-runtime test artifacts (`target-tests`, `target-test`, `nextest`, `target*/deps`) so app-managed spawn cannot accidentally run test binaries.
  - Daemon runtime now uses a pooled query-only read path (`CORTEX_DB_READ_POOL_SIZE`) to reduce read lock contention under startup fanout.
  - Daemon startup now includes early startup-safe storage-governor catch-up passes (no VACUUM) to relieve event pressure quickly on large event histories.
  - App-managed heavy startup delay now has a hard clamp (`APP_MANAGED_STARTUP_HEAVY_DELAY_MAX_SECS=120`) so misconfigured values (for example `777`) cannot defer critical background stabilization for many minutes.
- Validation references:
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml` (`374` passing).
  - `rtk cargo clippy --manifest-path daemon-rs/Cargo.toml -- -D warnings` (pass).
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml --test recall_benchmark -- --nocapture` (`7` passing).
  - `rtk cargo test --manifest-path desktop/cortex-control-center/src-tauri/Cargo.toml` (`16` passing).
  - `rtk npm test` in `desktop/cortex-control-center` (`57` passing).
  - `rtk npm run web:build` in `desktop/cortex-control-center` (pass).
  - `rtk python benchmarking/run_amb_cortex.py matrix --dry-run --matrix-file benchmarking/configs/amb-eval-matrix.nonlongmem.q5.json` (strict fair-run preflight passed).
  - `rtk python benchmarking/run_amb_cortex.py matrix --matrix-file benchmarking/configs/amb-eval-matrix.nonlongmem.q5.json --continue-on-error --summary-file benchmarking/runs/matrix-summary-latest-q5.json` (expected fail-fast: missing answer/judge provider keys; no fabricated score output).
- Commit:
  - `90ade1d`

### 0.3) Savings Telemetry Realism + Benchmark Bloat Remediation (new)
- Add an analytics-clarity section so operators do not misread inflated savings:
  - One-time benchmark namespace cleanup removes historical `amb-cortex*` decision/event/embedding bloat from live operator DBs (with pre-cleanup backup).
  - One-time cleanup also purges legacy `boot_savings` rows produced by the pre-fix baseline heuristic so dashboard totals reset to honest post-fix values.
  - Boot savings baseline accounting now excludes filesystem-size heuristics from custom source directories and reflects DB-backed boot context families only (active memories + active decisions).
  - This prevents inflated `tokens saved`/`$ saved` totals that were disconnected from real boot payload composition.
- Validation references:
  - Live DB audit before/after cleanup:
    - events: `433,479 -> 1,133`
    - decisions: `31,185 -> 489`
    - decision embeddings: `11,946 -> 483`
  - `rtk cargo check --manifest-path daemon-rs/Cargo.toml` (pass)
  - `rtk cargo clippy --manifest-path daemon-rs/Cargo.toml -- -D warnings` (pass)
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml --test recall_benchmark -- --nocapture` (`7` passing)
- Commit:
  - `f1dc152`

### 0.4) Runtime Binary Safety + Build-Bloat Guardrails (new)
- Add a short reliability/operations section for local app-managed mode:
  - Control Center daemon binary selection now fails closed for non-runtime target trees (`target-tests`, `target-test`, `nextest`, `target*/deps|build|incremental`, `target-rtk-*`, `target-codex-test`).
  - Runtime-safe daemon binaries remain explicitly allowed from `target`, `target-control-center-dev`, and `target-control-center-release`.
  - New operator cleanup helper: `npm run ops:prune-build-bloat` (dry-run) and `npm run ops:prune-build-bloat:apply`.
  - Current local dry-run snapshot showed ~`22.044 GB` reclaimable across stale test/isolated target folders.
- Mention scan/noise hygiene for developer tooling:
  - repo `.ignore` now suppresses heavy build/benchmark artifacts from code-search context loading.
  - `.gitignore` now explicitly covers recurring transient target-test and pytest-temp folders that produced access-denied/untracked noise.
- Validation references:
  - `rtk cargo fmt --manifest-path desktop/cortex-control-center/src-tauri/Cargo.toml`
  - `rtk cargo test --manifest-path desktop/cortex-control-center/src-tauri/Cargo.toml disallowed_daemon_binary_path_blocks_wrappers_temp_and_test_artifacts -- --nocapture` (`1` passed)
  - `npm run -s ops:prune-build-bloat` (dry-run totals + candidate list emitted)
- Commit:
  - `c75ed6e`

### 0.5) Study-Abroad Recall Qualifier Precision + Base Git Perf Audit (new)
- Add a benchmark-quality note for location qualifier completeness:
  - benchmark adapter now augments study-abroad location seed context with a country qualifier when sibling memories contain one strong candidate and primary context omits country.
  - this closes the recurring LongMemEval miss where the response returned only institution name without country.
  - strict fair-run benchmark evidence: `benchmarking/runs/amb-run-20260418-224211` (`20/20`, `accuracy=1.0`, `avg_recall_tokens=191.1`, `max_recall_tokens=295`, gate passed).
- Add operator-facing git/env performance diagnostics:
  - new script: `scripts/git-perf-health.ps1`
  - new npm commands:
    - `npm run ops:git-perf-audit`
    - `npm run ops:git-perf-apply`
  - script reports status latency, object-store health, and known target/test bloat buckets; apply mode enables local `feature.manyFiles`, `core.untrackedCache`, and `git maintenance run --auto`.
  - `.gitignore` now suppresses local temp scan-noise paths observed in real status warnings (`.tmp/pytest/`, `tmp/ptbase-*`, `tmp/pytest-local/`).
- Validation references:
  - `rtk python -m pytest benchmarking/adapters/tests/test_cortex_http_client.py -q` (`59` passing)
  - `rtk python benchmarking/run_amb_cortex.py run --dataset longmemeval --split s --query-limit 20 --max-runtime-seconds 1200` (`20/20`, gate passed)
  - `npm run -s ops:git-perf-audit`
  - `npm run -s ops:git-perf-apply`
- Commit:
  - pending (local optimization batch)

### 0.6) Raw Benchmark Backend Mode + Honest No-Helper Baseline (new)
- Add a benchmark-methodology note that separates tuned adapter performance from raw daemon path performance:
  - new benchmark backend selector: `--memory-backend` (`cortex-http`, `cortex-http-base`)
  - `cortex-http-base` is direct HTTP `/store` + `/recall` without helper-client rerank/detail-variant logic
  - matrix runs now allow per-case `memory_backend` to compare tuned vs raw paths under the same harness
  - baseline scenario keys now include backend suffix so baseline gates do not cross-contaminate tuned and raw tracks
- Add explicit no-fudging evidence section:
  - raw daemon benchmark (Rust, no Python adapter path): `rtk cargo test --manifest-path daemon-rs/Cargo.toml --test recall_benchmark -- --nocapture` (`7` passing)
  - strict raw backend LongMemEval run: `benchmarking/runs/amb-run-20260418-230402` with `--memory-backend cortex-http-base`
    - result: `4/20`, `accuracy=0.2`
    - gate failed on both quality floor and avg recall token ceiling (`avg_recall_tokens=267.3`, `limit=246.0`)
  - this baseline should be presented as the truthful starting point for daemon-level optimization work
- Validation references:
  - `rtk python -m pytest benchmarking/adapters/tests -q` (`142` passing)
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml --test recall_benchmark -- --nocapture` (`7` passing)
- Commit:
  - pending (local optimization batch)

### 0.7) Control Center IPC Fallback + Startup False-Negative Recovery (new)
- Add a startup resilience note for app-managed desktop sessions:
  - Control Center API client now fails over from Tauri IPC transport to direct local HTTP when IPC request envelopes hang or fail.
  - IPC timeout routing now canonicalizes URL-style paths before endpoint classification so core routes keep core timeout budgets.
  - Startup now accepts healthy `/health` as a reachability fallback when daemon-status probes return false-negative unreachable results.
  - This prevents prolonged “Daemon is still starting...” loops when daemon HTTP is healthy but IPC/status probes degrade.
- Include operator remediation guidance:
  - if dashboard startup regresses, verify duplicate `cortex.exe mcp --agent codex` processes are not accumulating and keep only one app-managed daemon on `:7437`.
- Validation references:
  - `rtk npm --prefix desktop/cortex-control-center run test -- src/api-client.test.js` (`40` passing)
  - `rtk npm --prefix desktop/cortex-control-center run web:build` (pass)
  - local endpoint probe: core/work protected routes healthy, `/savings` stable (higher latency expected)
- Commit:
  - `a2828ad`

### 0.8) Live DB Anti-Ballooning Controls (new)
- Add an operator-facing storage-bounds note for long-running local installs:
  - high-volume event write paths now enforce cap-based trimming across event families (`agent_boot`, `boot_savings`, `store_savings`, `tool_call_savings`, `decision_*`, `recall_query`, `merge`) instead of only `decision_stored`
  - compaction now prunes stale `agent_boot` telemetry and keeps global non-savings event pressure bounded
  - `boot_savings_rollup` rows are now retention-safe and excluded from generic old-event deletion, preserving all-time savings summaries while still compacting raw history
  - startup/storage governor pressure calculations now exclude only savings-history rows (`boot_savings`, `boot_savings_rollup`) so noisy operational telemetry cannot grow unchecked
- Validation references:
  - `rtk cargo test compaction:: -- --nocapture` (`12` passing)
  - `rtk cargo test prune_event_type_keep_latest_trims_old_rows -- --nocapture` (`1` passing)
- Commit:
  - pending (local optimization batch)

### 0.9) Persistent Savings Rollups + Event Payload Compression (new)
- Add an operator-facing analytics/storage section for long-running installs:
  - `recall_query` / `store_savings` / `tool_call_savings` rows are now compacted into `event_savings_rollups` (day+hour+operation aggregates) during compaction, then raw old rows are removed.
  - `/savings` now combines rollup rows with recent raw events, preserving 30-day analytics surfaces while reducing heavy raw-event scans.
  - write-path telemetry compaction now bounds large event payloads:
    - `merge` events persist `incoming_chars` + short preview instead of full merged text blobs.
    - `recall_query` telemetry persists analytics-safe summary fields with a hard payload-size budget.
  - crystal index hygiene now prunes orphan `cluster_members` rows, preventing stale member bloat after historical decision churn.
- Validation references:
  - `rtk cargo test compaction:: -- --nocapture` (`14` passing)
  - `rtk cargo test log_event_ -- --nocapture` (`2` passing)
  - `rtk cargo test health:: -- --nocapture` (`10` passing)
  - `rtk cargo test --quiet` attempted; exceeded lean-ctx shell timeout (`105s`) before completion
- Commit:
  - pending (local optimization batch)

### 0.10) Strict Fair-Run Benchmark Closure + Startup Lifecycle Stabilization (new)
- Add a benchmark-results note for completed strict fair-run closure:
  - strict tuned backend run (`cortex-http`): `benchmarking/runs/amb-run-20260419-073924` -> `20/20`, gate passed, avg recall tokens `199.35`, max `294`
  - strict raw no-helper backend run (`cortex-http-base`): `benchmarking/runs/amb-run-20260419-074548` -> `20/20`, gate passed, avg recall tokens `201.95`, max `297`
  - raw-path quality is now restored from earlier `4/20` baseline without disabling gates or changing benchmark tests.
- Add a startup resiliency note for desktop lifecycle behavior:
  - new startup policy module (`desktop/cortex-control-center/src/daemon-startup.js`) centralizes bounded retry windows and delays.
  - lifecycle verification now has polling fallback when startup event streams are unavailable, reducing false negatives during dev verification.
  - desktop path-binary fallback is now explicit opt-in (`CORTEX_ALLOW_PATH_BINARY_FALLBACK`) instead of implicit runtime behavior.
- Add a local DB hygiene/size note for benchmark telemetry cleanup:
  - benchmark namespace cleanup rerun with backup-first safety (`cortex-pre-benchmark-cleanup-20260419-075715.db`, `cortex-pre-benchmark-cleanup-2-20260419-075738.db`).
  - post-cleanup local counts: events `597`, decisions `489`, embeddings `9,749`, cluster_members `5,142`.
  - local DB footprint: `720.93 MB` backup snapshot -> `386.37 MB` active DB after cleanup + vacuum.
- Validation references:
  - `rtk python -m pytest benchmarking/adapters/tests/test_run_amb_cortex_shims.py -q` (`48` passing)
  - `rtk python -m pytest benchmarking/adapters/tests/test_cortex_http_base_provider.py benchmarking/adapters/tests/test_cortex_http_client.py benchmarking/adapters/tests/test_run_amb_cortex_matrix.py benchmarking/adapters/tests/test_run_amb_cortex_shims.py -q` (`149` passing)
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml` (`399` passing)
  - `rtk cargo clippy --manifest-path daemon-rs/Cargo.toml -- -D warnings` (pass)
  - `rtk npm --prefix desktop/cortex-control-center test -- src/daemon-startup.test.js src/api-client.test.js` (`48` passing)
  - `rtk cargo test --manifest-path desktop/cortex-control-center/src-tauri/Cargo.toml` (`21` passing)
- Commit:
  - pending (local optimization batch)

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

## 2026-04-19 23:44 - README impact queue (analytics metric language)
- Update analytics metric descriptions to be user-facing and implementation-agnostic:
  - Compounding return footnote should describe the window and meaning (`rolling 30-day tokens saved`) without exposing internal event names.
  - Throughput footnote should describe the visible contract (`rolling 7-day boot compilations`, with optional per-day average) without benchmark/internal caveats.
- Add short dashboard legend section that maps:
  - `Compounding return` -> 30-day total boot tokens saved.
  - `Throughput` -> last 7-day boot compilations.
  - `Compiled context` -> 30-day total prompt tokens served at boot.
  - `Economic value` -> estimated currency value derived from saved tokens and the displayed assumption.

## 2026-04-19 23:52 - README impact queue (analytics legend + visual consistency)
- Add a concise Analytics legend section to README/UI docs mirroring the in-product legend strip:
  - `Compounding return`, `Efficiency`, `Throughput`, `Compiled context`, `Economic value`.
- Clarify that the economic-value assumption is a model assumption (conversion factor), not a metric legend key.
- Note top-row analytics card alignment update so values/labels/footnotes read consistently across cards.

## 2026-04-20 00:22 - README impact queue (Overview crash hardening)
- Document startup/persisted-state resilience for Control Center UI:
  - Persisted currency now validates against supported currency codes and falls back to USD when invalid.
  - Currency formatting now has defensive fallback behavior instead of risking runtime render failures.
  - Panel label rendering in topbar is null-safe with an explicit fallback (`Overview`).
- Mention that dashboard preferences are sanitized before render to prevent blank-screen failures from stale localStorage values.

## 2026-04-20 00:39 - README impact queue (internal test-clippy hygiene)
- Commit `ee864c6` is a test-harness correctness fix in `daemon-rs/src/handlers/recall.rs` (removed await-held mutex guard in async test path).
- Public README impact: none required for this commit.
- Validation:
  - `rtk cargo clippy --manifest-path daemon-rs/Cargo.toml --all-targets --all-features -- -D warnings` (pass).
  - `rtk cargo test --manifest-path daemon-rs/Cargo.toml --target-dir daemon-rs/target-codex-test execute_unified_recall_fail_closes_when_latency_budget_is_zero -- --nocapture` (pass).

## 2026-04-20 01:05 - README impact queue (canonical plan wording sync)
- Commit `c67c4ad` updates only the internal canonical status owner doc (`docs/internal/CORTEX-UNIFIED-STATUS-PLAN.md`) to remove stale wording drift in the top-level Phase-2A state summary.
- Public README impact: none required for this commit.

## 2026-04-21 22:14 - README impact queue (ownership-neutral copy + release links)
- Public README copy was neutralized to remove personal-name references in user-facing sections:
  - startup/plugin quickstart now points to neutral `cortex-project/cortex` owner placeholders.
  - Monte Carlo explanatory notes now describe maintainer-run/live dataset language without individual identity references.
  - support call-to-action now uses a neutral sponsor landing page.
- Release-link references were updated to neutral project owner path placeholders for org handoff readiness.
- Follow-up note for release owner:
  - verify canonical GitHub org/repo slug before tag publication so download/release links and updater endpoint resolve correctly.
