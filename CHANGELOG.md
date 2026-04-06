# Changelog

All notable changes to this project are documented in this file.

The format is inspired by [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

[Unreleased]: https://github.com/AdityaVG13/cortex/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/AdityaVG13/cortex/compare/v0.3.0...v0.4.0

