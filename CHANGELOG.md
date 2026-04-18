# Changelog

All notable changes to this project are documented in this file.

The format is inspired by [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- Control Center startup timeout cascades on large event histories by moving `/savings` out of startup-critical refresh fanout and loading it only when the Analytics panel is active.
- Daemon savings analytics lock contention by replacing full event-log Rust parsing with SQL-side aggregation and short-lived payload caching in `GET /savings`.
- App-managed startup timeout sensitivity by staggering heavy startup maintenance passes for Control Center ownership and accepting partial daemon probe responses when timeout/would-block happens after partial bytes are received.
- Control Center refresh overlap storms by introducing a single-flight refresh queue, so background timers/SSE/recovery retries coalesce instead of blasting concurrent protected IPC calls.
- Read-path lock contention on startup surfaces by routing key GET endpoints through `state.db_read` and removing write-side cleanup work from hot read paths.

### Performance
- Improved high-volume analytics behavior on large `events` tables, reducing shared read-lock occupancy and lowering cross-endpoint timeout risk during cold start.
- Added warmup-aware heavy-metrics caching in `/health` (cache/deferred source tagging) so startup fetches avoid repeated expensive metrics work.
- Added benchmark ingestion compaction caps (`CORTEX_BENCHMARK_STORE_MAX_CHARS`, tighter fact/context matrix caps) to bound benchmark-side payload growth and token pressure.
- Added targeted DB indexes for startup-critical queries (`sessions`, `locks`, `tasks`, `feed`, `messages`, `activity`, `events`) including owner-scoped variants to cut first-load scan pressure.
- Added event-pressure compaction controls (per-event-type caps + global non-boot cap with hard/soft thresholds) to prevent unbounded event growth from degrading startup.
- Scoped `/savings` analytics rollups to the recent window (`30d`) to bound heavy aggregates for large long-lived histories.

### Changed
- Benchmark fair-run posture is now fail-closed for both single-run and matrix modes: gate-bypass shortcuts (`no_enforce_gate`, `allow_missing_recall_metrics`) are rejected in preflight, and matrix-requested shortcut flags are explicitly surfaced/rejected before execution.

## [0.4.1] - 2026-04-06

### Features
- **Cross-platform desktop builds** -- macOS (`.dmg`) and Linux (`.AppImage`, `.deb`) installers now ship alongside Windows in every release
- **Daemon auto-respawn** -- MCP proxy detects daemon death mid-session and restarts it automatically (bounded to 3 attempts with backoff). Sessions survive transient crashes without user intervention

### Fixes
- Unix daemon respawn uses `setsid()` so the child process survives the parent CLI exiting
- MCP proxy re-resolves `CortexPaths` after respawn to pick up port changes

### Documentation
- README: analytics screenshot, known limitations, cross-platform download table, temporary logo note
- Removed `docs/internal/roadmap-internal.md` from tracking (internal codenames)

### CI
- `build-desktop` converted to 3-platform matrix; reuses daemon artifacts as Tauri sidecars
- Release workflow uploads `.dmg`, `.AppImage`, `.deb` alongside `.exe`

## [0.4.0] - 2026-04-05

### Security
- `is_visible` fails closed: NULL owner_id and mismatched visibility now return false in team mode
- MCP caller identity resolved at startup from `CORTEX_API_KEY` or `cortex.token`
- Owner_id threaded through all INSERT paths (store, indexer, crystallize, focus)
- `/unfold` endpoint now applies visibility filtering by owner_id

### Features
- Configurable custom knowledge sources via `~/.cortex/sources.toml` or `CORTEX_EXTRA_SOURCES` env
- Replaced 6 hardcoded extended indexers with generic `index_custom_sources` function
- Extension indexing gated behind `CORTEX_INDEX_EXTENDED=1` for opt-in
- Tauri auto-update enabled with signing public key
- Claude Code plugin scaffold under `plugins/cortex-plugin/` with:
  - `.claude-plugin/plugin.json`, `.mcp.json`, and `hooks/hooks.json`
  - Runtime scripts: `prepare-runtime.cjs`, `hook-boot.cjs`, `run-mcp.cjs`
  - Mandatory SHA256 verification for packaged daemon archives before extraction
  - Team-mode `userConfig` support (`cortex_url`, `cortex_api_key`)
  - Built-in skills: `help`, `recall`, `store`, `status`

### Documentation
- Public ROADMAP.md with milestones v0.4.0 through v1.0.0
- Updated model_delineation.md with completed GLM 5 tasks (#58-59, #64-65, #67-70, #73, #76, #78)

### Cleanup
- Removed all Ollama references from daemon, desktop app, workers, and docs
- Fixed .gitignore patterns to ignore specific docs/ subdirs only
- Removed tracked personal config files (Modelfile.glm, Modelfile.deepseek)
- Eliminated hardcoded developer identity paths from compiler.rs, indexer.rs, and service.rs
- Clean install smoke test and verification implemented

### Desktop (Control Center)
- Sidecar process manager - launches real cortex.exe instead of embedded copy
- Version-synced all artifacts to 0.3.0 (package.json, Cargo.toml, tauri.conf.json)
- New app icons generated from source image
- About panel with version, stack info, contributors, and links
- Fixed split-brain path bug by using canonical `~/.cortex/cortex.db`
- Port resolution now uses `cortex paths --json` with `CORTEX_PORT`/`7437` fallback
- Binary discovery now includes plugin runtime path `~/.cortex/bin/cortex(.exe)`
- Windows MCP registration now uses path-safe string conversion (no manual slash replacement)

## [0.3.0] - 2026-04-04

[Unreleased]: https://github.com/AdityaVG13/cortex/compare/v0.4.1...HEAD
[0.4.1]: https://github.com/AdityaVG13/cortex/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/AdityaVG13/cortex/compare/v0.3.0...v0.4.0

