# Changelog

All notable changes to this project are documented in this file.

The format is inspired by [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/AdityaVG13/cortex/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/AdityaVG13/cortex/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/AdityaVG13/cortex/releases/tag/v0.1.0
