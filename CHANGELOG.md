# Changelog

All notable changes to this project are documented in this file.

The format is inspired by [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0] - 2026-04-04

### Desktop (Control Center)
- Replaced embedded daemon with sidecar process manager (`sidecar.rs`) -- Control Center now launches the real `cortex.exe` binary
- Version-synced all desktop artifacts (`package.json`, `src-tauri/Cargo.toml`, `tauri.conf.json`) to `0.3.0`
- New app icons generated from `icon_source.png` source image
- Removed Ollama metric card and stat from Overview panel; EMBEDDINGS strip now shows ONNX status based on daemon reachability
- Added About panel (12th nav entry) with version, stack info, contributors, and links

### Features
- Over-fetch-then-filter recall with visibility filtering (`raw_k=max(k*5,50)`, max 2 retries)
- 13 admin endpoints for user/team/data management (`/admin/user/*`, `/admin/team/*`, `/admin/stats`, etc.)
- CLI commands: `cortex user add/rotate-key/remove/list`, `cortex team create/add/remove/list`, `cortex admin stats/list-unowned/assign-owner`
- Solo-to-team migration enhancement: pre-migration backup, per-table row counts, interactive owner prompt, `cortex migrate --dry-run`
- Graceful degradation across 7 failure scenarios: ONNX fallback with `degraded_mode` flag, team-mode MCP fail-closed, write-ahead buffer for offline stores, TLS solo/team mode distinction

### Security
- `ensure_auth_with_caller` combines auth + identity resolution in single argon2 pass
- `RecallContext` threads visibility through entire recall pipeline (semantic, keyword, crystal, budget)
- Role-based admin auth via `ensure_admin` (owner/admin required)
- Table name allowlists prevent SQL injection in dynamic admin queries

### Fixes
- `unfold_source` / GET `/unfold` now take `RecallContext` and filter by `owner_id` / `visibility` (fixes MCP `unfold_source` 3-arg mismatch and release CI build)
- Decision search in retry loop used hardcoded limit instead of `fts_limit`
- Fallback recall paths returned NULL `owner_id` causing visibility issues
- MCP handlers bypassed visibility by hardcoding solo context
- `handle_user_add` used re-query instead of `last_insert_rowid()`
- Removed `tasks` from archive/visibility allowlists (uses `task_id` TEXT, not `id`)

### Known Issues
- MCP JSON-RPC lacks per-caller identity (uses default owner for all callers)
- `is_visible` treats NULL `owner_id` as visible in team mode (should fail closed after migration)
- Team-mode test environment needed to validate end-to-end

## [0.2.0] - 2026-04-04

### Added
- Team-mode schema migration path in `daemon-rs` with owner-aware tables and keys.
- Argon2id-backed `ctx_` API key generation and verification for team auth flows.
- CLI data portability commands: `cortex export` and `cortex import`.
- OpenAPI surface for HTTP integrations at `specs/cortex-openapi.yaml`.
- Example integrations under `examples/gemini-cli` and `examples/local-llm`.
- Automated GitHub release workflow for Windows, macOS (arm64), and Linux artifacts.

### Changed
- Version bump to `0.2.0` across daemon, desktop app, SDKs, and release metadata.
- Updated `README.md` release download table and integration references.
- Conductor/feed handlers and team setup flow updated for owner-scoped behavior.

### Fixed
- Release workflow portability across available GitHub-hosted runners.
- Unix target build compatibility by declaring `libc` for unix targets.

### Notes
- macOS `x86_64` artifacts are temporarily unavailable in `v0.2.0` due to ONNX Runtime prebuilt target availability in the current pipeline.

## [0.1.0] - 2026-03-31

### Added
- Initial public Cortex release.

[Unreleased]: https://github.com/AdityaVG13/cortex/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/AdityaVG13/cortex/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/AdityaVG13/cortex/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/AdityaVG13/cortex/releases/tag/v0.1.0
