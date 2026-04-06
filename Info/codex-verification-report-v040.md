# Codex Verification Report -- v0.4.0

## Date
2026-04-06

## Scope
- Desktop app critical fixes (split-brain path, port resolution, binary discovery, Windows path handling)
- Pre-release remediation sweep
- Gemini Phase 4A deliverable verification
- Phase 5 readiness checks

## Pre-Release Remediation Summary

| Finding | Severity | Status |
|---|---|---|
| Desktop split-brain DB path | CRITICAL | FIXED |
| Desktop hardcoded port | HIGH | FIXED |
| Desktop binary discovery (`~/.cortex/bin`) | HIGH | FIXED |
| Windows path replacement in desktop | HIGH | FIXED |
| CHANGELOG coverage gaps | CRITICAL | FIXED |
| CONTRIBUTING multi-model section | HIGH | FIXED |
| Team mode setup guide | HIGH | FIXED (published in `Info/`) |
| MCP tool reference docs | MEDIUM | FIXED |
| Windows extraction fallback in plugin runtime | MEDIUM | FIXED |

## Gemini 4A Verification

| Item | Status | Notes |
|---|---|---|
| README rewrite | VERIFIED (with fixes) | Header preserved; command table/tool names corrected |
| Team mode guide | VERIFIED | Public guide exists at `Info/team-mode-setup.md` |
| Launch content | VERIFIED (with fixes) | Claims aligned to benchmark data |

## Phase 5 Verification

| Item | Status | Notes |
|---|---|---|
| `scripts/release-smoke-test.sh` exists | VERIFIED | Present and runnable shell script |
| Smoke-test coverage vs tracker claims | PARTIAL | Missing explicit legacy-migration/concurrency checks |
| Plugin assets package | MISSING | `plugins/cortex-plugin/assets/` contains only `.gitkeep` |
| Full install flow test | BLOCKED | Depends on packaged platform assets |
| Changelog update | VERIFIED | v0.4.0 now includes plugin + desktop coverage |
| Git tag creation | NOT RUN | Intentionally deferred to release manager |

## Plugin Health
- Scripts parse: **YES**
- Plugin/marketplace JSON valid: **YES**
- Skill files present: **YES**

## Release Readiness
**BLOCKED** pending:
1. Packaged plugin assets + checksums
2. Full install-flow validation
3. Manual GUI custom-port runtime verification in Cortex Control Center
