# Changelog

All notable changes to this project are documented in this file.

The format is inspired by [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0] - 2026-04-22

349 commits since v0.4.1. This release focuses on retrieval quality, daemon reliability, Control Center resilience, and security hardening.

### Added

- **Retrieval**: Tiered retrieval with reciprocal rank fusion (RRF) and query-adaptive keyword/semantic weighting
- **Retrieval**: Crystal family recall with collapsed member hits (`familyMembers`, `familySize`, `collapsedSources`)
- **Retrieval**: Synonym-expanded term-group parity across all keyword retrieval paths
- **Retrieval**: Entity-alignment ranking boost and associative co-occurrence expansion
- **Retrieval**: Trust-aware ranking blend with provenance scoring (`source_client`, `source_model`, `trust_score`)
- **Retrieval**: FTS tokenizer upgrade to `porter unicode61` (migration 012)
- **Retrieval**: `POST /recall` endpoint to avoid query-string leakage for sensitive queries
- **Retrieval**: `GET /recall/explain` with shadow semantic diagnostics and family-compaction breakdown
- **Schema**: Migration framework with v0.4.1 to v0.5.0 upgrade regression testing via on-disk fixture
- **Storage**: DB integrity gate, rolling backups, and crash-safe WAL
- **Storage**: Storage pressure governor with soft/hard thresholds and health telemetry
- **Storage**: Event-pressure controls (per-type caps plus global non-boot cap)
- **Storage**: Persistent savings rollups (`event_savings_rollups` table) for long-window analytics
- **Storage**: TTL/hard-expiration behavior with cleanup loop
- **Daemon**: `GET /readiness` canonical startup gate (separate from `/health` liveness)
- **Daemon**: `GET /stats` transparency endpoint (tier distribution, latency rollups, recall savings, shadow-semantic diagnostics)
- **Daemon**: Single-daemon enforcement with app-owned lifecycle management
- **Daemon**: IPC transport foundation (Windows named pipe, Unix socket)
- **Daemon**: MCP session self-healing — `cortex_boot` emits session events, read-path tools reattach to existing sessions
- **Daemon**: Embedding profile selection via `CORTEX_EMBEDDING_MODEL` (default `all-MiniLM-L6-v2`, `all-MiniLM-L12-v2` available)
- **Daemon**: Bounded embedding backfill (`CORTEX_EMBED_BACKFILL_BATCH_SIZE`, `CORTEX_EMBED_BACKFILL_MAX_BATCHES_PER_PASS`, `CORTEX_EMBED_BACKFILL_INTERVAL_SECS`)
- **Daemon**: sqlite-vec shadow infrastructure and KNN diagnostics (shadow only, not production routing)
- **Agents**: Feedback telemetry — `POST /agent-feedback`, `GET /agent-feedback/stats`, and matching MCP tools
- **Agents**: Recall policy explainability via `cortex_recall_policy_explain`
- **Agents**: Client permissions system (`client_permissions` schema) with read/write/admin gates
- **Agents**: Conflict detection and resolution (AGREES, CONTRADICTS, REFINES, UNRELATED classification)
- **MCP**: 15 new tools — `cortex_semantic_recall`, `cortex_recall_policy_explain`, `cortex_reconnect`, `cortex_lastCall`, `cortex_consensus_promote`, `cortex_eval_run`, `cortex_memory_decay_run`, `cortex_agent_feedback_record`, `cortex_agent_feedback_stats`, `cortex_conflicts_list`, `cortex_conflicts_get`, `cortex_conflicts_resolve`, `cortex_permissions_grant`, `cortex_permissions_list`, `cortex_permissions_revoke`
- **Specs**: OpenAPI spec (`specs/cortex-openapi.yaml`) version-aligned to 0.5.0
- **Benchmark**: Isolated benchmark runner with multi-dataset matrix workflow and fail-closed gate enforcement
- **Benchmark**: Both tuned (`cortex-http`) and raw (`cortex-http-base`) backends reach 20/20 accuracy

### Changed

- Benchmark posture is now fail-closed: gate-bypass shortcuts rejected in preflight
- Plugin behavior is attach-only by default; no longer bootstraps daemon ownership
- Startup behavior is stricter in favor of service-managed lifecycle
- Health checks are readiness-aware; stale or wrong-instance targets rejected
- `/savings` scoped to 30-day recent window for bounded aggregation
- `cortex_recall` default budget remains 200 tokens

### Fixed

- Control Center startup timeout cascades on large event histories
- Dashboard refresh overlap storms via single-flight coalescing
- Read-path lock contention on startup by routing GET endpoints through `db_read`
- App-managed startup timeout sensitivity with staggered maintenance passes
- IPC timeout handling alignment (`transport ~= abort - 500ms`)
- Dev binary lock resilience on Windows with retry logic
- Startup-safe storage governor (no VACUUM during startup)
- Analytics hydration on startup core readiness signal
- Browser auth token cleared on auth-only failures to prevent stale state
- Connection Settings dialog dismiss behavior
- Health-poll dedup in browser flow

### Performance

- SQL-side `/savings` aggregation replacing full Rust event-log parsing
- Warmup-aware heavy-metrics caching in `/health` with deferred source tagging
- Targeted DB indexes for startup-critical queries (`sessions`, `locks`, `tasks`, `feed`, `messages`, `activity`, `events`)
- Pooled read connections via `CORTEX_DB_READ_POOL_SIZE`
- Bounded background DB lock waits via `CORTEX_BACKGROUND_DB_LOCK_MAX_WAIT_MS`
- Event payload compaction, orphan `cluster_members` pruning, and savings rollup compression
- Local DB footprint reduced from 720 MB to 386 MB in maintainer install

### Security

- Localhost callers exempt from auth-failure lockout bucket (loopback brute-force still rate-limited)
- Non-loopback binds require TLS on public/routed interfaces
- API key masking on non-interactive stdout; TTY still shows full key for copy-once onboarding
- Team-mode destructive endpoints require admin plus rated auth
- SDK safety default: remote base URLs require explicit token, no silent local-token auto-load
- Public benchmark JSON metadata sanitized (developer-specific paths removed)
- Auth guard ordering and rate-limit coverage expanded

### Desktop

- IPC fallback: `createApi`/`createPostApi` fail over to direct HTTP when Tauri IPC fails
- Startup retry state abstraction (`daemon-startup.js`) with bounded retry and polling fallback
- Core route prioritization on cold start (sessions/locks/tasks first, secondary panels deferred)
- PATH binary fallback opt-in via `CORTEX_ALLOW_PATH_BINARY_FALLBACK`
- Missing `/permissions` endpoint degrades gracefully instead of erroring
- Shared Feed kind filters trigger immediate refresh

## [0.4.1] - 2026-04-06

### Added

- **Cross-platform desktop builds** — macOS (`.dmg`) and Linux (`.AppImage`, `.deb`) installers ship alongside Windows in every release
- **Daemon auto-respawn** — MCP proxy detects daemon death mid-session and restarts automatically (bounded to 3 attempts with backoff)

### Fixed

- Unix daemon respawn uses `setsid()` so the child process survives the parent CLI exiting
- MCP proxy re-resolves `CortexPaths` after respawn to pick up port changes

### Documentation

- README: analytics screenshot, known limitations, cross-platform download table
- Removed internal roadmap from tracking

### CI

- `build-desktop` converted to 3-platform matrix; reuses daemon artifacts as Tauri sidecars
- Release workflow uploads `.dmg`, `.AppImage`, `.deb` alongside `.exe`

## [0.4.0] - 2026-04-05

### Security

- `is_visible` fails closed: NULL `owner_id` and mismatched visibility return false in team mode
- MCP caller identity resolved at startup from `CORTEX_API_KEY` or `cortex.token`
- `owner_id` threaded through all INSERT paths (store, indexer, crystallize, focus)
- `/unfold` applies visibility filtering by `owner_id`

### Added

- Configurable custom knowledge sources via `~/.cortex/sources.toml` or `CORTEX_EXTRA_SOURCES`
- Replaced hardcoded extended indexers with generic `index_custom_sources`
- Extension indexing gated behind `CORTEX_INDEX_EXTENDED=1`
- Tauri auto-update with signing public key
- Claude Code plugin scaffold under `plugins/cortex-plugin/` with runtime scripts, SHA256 verification, team-mode support, and built-in skills

### Documentation

- Public roadmap with milestones v0.4.0 through v1.0.0

### Cleanup

- Removed all Ollama references from daemon, desktop, workers, and docs
- Fixed `.gitignore` patterns for docs subdirectories
- Removed tracked personal config files and hardcoded developer identity paths
- Clean install smoke test implemented

### Desktop

- Sidecar process manager launches real `cortex.exe` instead of embedded copy
- About panel with version, stack info, contributors, and links
- Fixed split-brain path bug by using canonical `~/.cortex/cortex.db`
- Port resolution via `cortex paths --json` with `CORTEX_PORT`/`7437` fallback
- Binary discovery includes plugin runtime path `~/.cortex/bin/cortex(.exe)`

## [0.3.0] - 2026-04-04

Initial public release.

[Unreleased]: https://github.com/AdityaVG13/cortex/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/AdityaVG13/cortex/compare/v0.4.1...v0.5.0
[0.4.1]: https://github.com/AdityaVG13/cortex/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/AdityaVG13/cortex/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/AdityaVG13/cortex/compare/v0.3.0...v0.3.0
