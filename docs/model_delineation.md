# Model Delineation -- Cortex Work Assignments

<!-- When you finish a task, put a ✓ in the Done column. -->

## Tool Reference

| Tool | Model | Cost | Use For |
|------|-------|------|---------|
| Claude Code | Opus 4.6 | $$$$$ | Multi-file Rust, architecture, security, complex reasoning |
| Cursor (Opus) | Opus 4.6 | $$$$ | Complex single-codebase work needing deep understanding |
| Cursor (Sonnet) | Sonnet 4.6 | $$ | Single-file edits, straightforward refactors |
| Codex CLI | GPT-5.3 | $$ | Batch tasks, overnight autonomous, docs generation |
| Gemini CLI | 2.5 Pro | $ | 1M context analysis, research, full-repo documentation |
| Droid (GLM 5) | GLM 5 (0.4x) | $ | Medium features, API endpoints, adapters |
| Droid (GLM 4.7) | GLM 4.7 (0.25x) | ¢ | Config, schema DDL, boilerplate, mechanical cleanup |
| Gemini Flash | 2.5 Flash | ¢ | Formatting, drafts, lookups, light code tasks |

---

# Open-Source Release Tasklist

## CRITICAL (blocks public release)

### Claude Code (Opus)

| # | Task | Done | Details |
|---|------|------|---------|
| 83 | Fix /unfold visibility bypass | | Thread RecallContext through unfold handler. Zero access control currently. Root cause, not patch. |
| 84 | Fix is_visible NULL owner_id policy | | Fail closed in team mode. Migration must guarantee zero NULLs. Add CHECK constraint. |
| 85 | Fix MCP per-caller identity | | API key or caller_id per JSON-RPC request. `from_state` is a workaround, not a fix. |
| 104 | indexer.rs: graceful skip for missing knowledge sources | Done | 6 extended indexers gated behind `CORTEX_INDEX_EXTENDED=1`; core paths only by default. No `self-improvement-engine` literal in `daemon-rs/src`. |
| 108 | Clean install end-to-end test (THE GATE) | Partial | Automated: `indexer` clean-home test, `grep` gates on `daemon-rs/src` (no `aditya`, no `self-improvement-engine` substring). **Still run manually:** clone on clean machine, `cortex serve`, /health, store/recall, stderr scan before calling release done. |

### Cursor (Opus)

| # | Task | Done | Details |
|---|------|------|---------|
| 87 | Desktop app: sidecar real daemon, kill embedded copy | Done | Delete embedded_daemon.rs (3000+ lines duplicated, drifted). Tauri launches cortex.exe as sidecar. One installer bundles both. Double-click → daemon starts → dashboard opens → /health green. |
| 97 | Desktop app: fix all dead UI, remove Ollama box | Done | Start/Stop buttons do nothing currently -- must launch/kill daemon. Audit every button and field in all 11 panels. Remove Ollama status box. Dead buttons = remove, don't ship. |
| 101 | compiler.rs: replace hardcoded "User: Aditya" identity | Done | Line 113: baked into binary. Detect dynamically: `USERNAME`/`USER` env, `std::env::consts::OS`, shell from `SHELL`/`COMSPEC`. |
| 102 | compiler.rs + indexer.rs: replace hardcoded `C--Users-aditya` | Done | compiler.rs:628, indexer.rs:109. Dynamically resolve Claude projects dir from CWD slug. |

### Droid (GLM 4.7)

| # | Task | Done | Details |
|---|------|------|---------|
| 107 | Make a copy of all personal files to C:\Users\aditya\AI\Personal and then delete ALL personal files, update .gitignore | Done | Backup under `.personal-backup/`; `git rm` 14 tracked personal files; `.personal-backup/` in `.gitignore`. |

## HIGH (should ship with release)

### Cursor (Opus)

| # | Task | Done | Details |
|---|------|------|---------|
| 86 | Version bump to v0.3.0 + git tag + GitHub release | v0.3.0 tag exists; v0.3.1 pending commit | Cargo.toml, CHANGELOG.md, build release binary, `gh release create`, attach binary. |
| 100 | Desktop app: version sync to 0.3.0 | ✓ | tauri.conf.json, desktop Cargo.toml, package.json must all match daemon v0.3.0. |

### Cursor (Sonnet)

| # | Task | Done | Details |
|---|------|------|---------|
| 103 | service.rs: replace `"aditya"` fallback username | ✓ | Line 30: change to `"cortex-user"`. One-line fix. |
| 105 | workers/drift_detector.py: hardcoded `C--Users-aditya` | ✓ | Line 21: derive dynamically from CWD, same pattern as #102. |
| 110 | config/Modelfile.glm: delete personal LLM configs | ✓ | Delete config/Modelfile.glm and config/Modelfile.deepseek. Hardcoded `C:/Users/aditya/.lmstudio/` path. Add `config/` to .gitignore. |

### Codex CLI

| # | Task | Done | Details |
|---|------|------|---------|
| 93 | ROADMAP.md for contributors | | Process all architecture docs (Codex + Gemini longterm considerations) into public roadmap with contribution areas. |
| 94 | CONTRIBUTING.md + SECURITY.md | | Dev setup, build instructions, PR guidelines, vulnerability disclosure policy. |

### Gemini CLI

| # | Task | Done | Details |
|---|------|------|---------|
| 90 | README rewrite for public audience | | 1M context read of entire repo. Rewrite for external devs, not internal team. Remove personal references. Keep AdityaVG13 as repo owner (that's correct). |
| 92 | Review architecture docs: public vs internal vs remove | | docs/architecture/, docs/compatibility/, docs/schema/, docs/archive/ -- classify each. |

### Droid (GLM 4.7)

| # | Task | Done | Details |
|---|------|------|---------|
| 95 | Repo cleanup: .gitignore patterns for all personal/build artifacts | | 20+ new patterns: personal configs, editor dirs, build artifacts, debug logs, db backups. |
| 96 | Remove legacy Node.js src/ or add deprecation notice | | Rust daemon is the product. Legacy code confuses contributors. |

## MEDIUM (nice to have for launch)

### Cursor (Sonnet)

| # | Task | Done | Details |
|---|------|------|---------|
| 88 | App icon: replace with adityasmile.png | ✓ | Remove all old icons. Rename adityasmile.png → icon.png. Generate required Tauri sizes (icon.ico, icon.icns, 32x32, 128x128, 128x128@2x). Update generate-icon.py reference. |
| 89 | README: release badge, download link, "What's New" | ✓ | Top badge box currently empty. Add release link, version badge, feature highlights. |
| 98 | Desktop app: add About tab (panel #12) | ✓ | Creator photo + "Created by Aditya". Contributors section (GitHub API or manual). App version number. |
| 99 | Desktop app: auto-update via tauri-plugin-updater | ✓ | In-app update check, notification when new version available. Document in README. |
| 106 | tools/ingest_chatgpt.py: remove or generalize | ✓ | 30+ "aditya" refs as classification label. Either make configurable or remove from public repo. |

### Gemini CLI

| # | Task | Done | Details |
|---|------|------|---------|
| 91 | Recall quality baseline analysis | | Surprise score distribution across 220+ decisions. Define meaningful thresholds. |

## LOW (post-launch)

### Droid (GLM 4.7)

| # | Task | Done | Details |
|---|------|------|---------|
| 109 | Auto-generate CHANGELOG on version tags | | GitHub Actions on `v*` tag. `git-cliff` or conventional-changelog. Document in CONTRIBUTING.md. |

---

# Future Roadmap (not blocking release)

### Claude Code (Opus)

| # | Task | Details |
|---|------|---------|
| 9 | Chrome extension for claude.ai, chatgpt.com, gemini.com | Manifest V3, content scripts, background worker. Blocked on team-mode test env. |
| 40 | Test Chrome extension across 3 platforms | Windows, macOS, Linux. Depends on #9. |

### Droid (GLM 5)

| # | Task | Details |
|---|------|---------|
| 6 | OpenAI function adapter spec and handler | compatibility/02 |
| 19 | Key rotation with 72h grace period | compatibility/03 |
| 21 | SQLCipher encryption at rest | compatibility/03 |
| 22-25 | MCP/OpenAI adapter protocol work (4 tasks) | compatibility/04 |
| 58-59 | Owner_id + visibility on remaining tables (2 tasks) | schema/03 |
| 64-65 | Solo/team mode recall scoping (2 tasks) | schema/04 |
| 67-70 | Conductor ownership + visibility API (4 tasks) | schema/04-05 |
| 73 | Fresh install defaults to solo mode | schema/05 |
| 76 | Role enforcement with CHECK constraints | schema/06 |
| 78 | Row-level NULL owner_id prevention | schema/06 |

### Droid (GLM 4.7)

Remaining original tasks #12-18, 20, 29-30, 32-36, 41, 54-57, 63, 79 -- see Completed section for reference.

### Gemini CLI

| # | Task | Details |
|---|------|---------|
| 77 | Validate visibility enforcement at query level | schema/06 |
| 81 | Database size monitoring and growth trajectory | schema/06 |
| 82 | Document deferred features | schema/06 |

---

# Notes

### Open-source readiness: 85%
All 36 critical-path features shipped. Blocking: 3 security root causes (#83-85), personal file cleanup (#107), clean install test (#108).

### Team-mode test environment needed
Required for: Chrome extension (#9, #40), visibility E2E validation, MCP identity testing.

### Recall quality (Gemini CLI #91)
Surprise metric has no baseline. Distribution analysis needed before the score means anything.

### Builder prompt lesson
Critic catches cost 2-3x what prevention costs. Every builder prompt must include a `## Known Pitfalls` section with schema quirks, naming inconsistencies, and auth patterns.

---

# Completed Tasks

### Claude Code (Opus) -- 5/7 done

| # | Task | Source |
|---|------|--------|
| 4 | ✓ Graceful degradation across all system layers | compatibility/01 |
| 66 | ✓ Over-fetch-then-filter embedding recall (raw_k=max(k*5,50)) | schema/04 |
| 71 | ✓ Admin endpoints: /admin/user/*, /admin/team/*, /admin/stats | schema/05 |
| 72 | ✓ CLI commands: cortex user/team/admin management | schema/05 |
| 74 | ✓ Solo-to-team migration (setup --team, backup, counts, dry-run) | schema/05 |

### Cursor (Sonnet/GLM 5) -- 14/20 done

| # | Task | Source |
|---|------|--------|
| 1 | ✓ HTTP REST API as core transport | compatibility/01 |
| 2 | ✓ Unified auth layer | compatibility/01 |
| 3 | ✓ TLS via rustls | compatibility/01 |
| 5 | ✓ MCP adapter: stdio-to-HTTP bridge | compatibility/02 |
| 7 | ✓ Python SDK (cortex-memory on PyPI) | compatibility/02 |
| 8 | ✓ TypeScript SDK (@cortex-memory/client) | compatibility/02 |
| 10 | ✓ System prompt injector CLI | compatibility/02 |
| 11 | ✓ Standalone fallback mode for solo MCP | compatibility/02 |
| 15 | ✓ Rate limiting | compatibility/03 |
| 26 | ✓ ONNX embedding fallback to FTS5 | compatibility/05 |
| 27 | ✓ Daemon crash detection in MCP adapter | compatibility/05 |
| 28 | ✓ SQLite integrity check on startup | compatibility/05 |
| 31 | ✓ Export/import (JSON and SQL) | compatibility/05 |
| 80 | ✓ ONNX embedding session pooling | schema/06 |

### Codex CLI -- 10/10 done

| # | Task | Source |
|---|------|--------|
| 37 | ✓ OpenAPI spec for Custom GPT Actions | compatibility/06 |
| 38 | ✓ Gemini CLI integration | compatibility/06 |
| 39 | ✓ Local LLM integration | compatibility/06 |
| 42 | ✓ Validate solo mode schema | schema/01 |
| 43 | ✓ Team mode schema with multi-tenancy | schema/01 |
| 44 | ✓ Validate API surface identical in both modes | schema/01 |
| 60 | ✓ Recreate sessions table | schema/03 |
| 61 | ✓ Recreate locks table | schema/03 |
| 62 | ✓ Recreate feed_acks | schema/03 |
| 75 | ✓ Export/import path for user data migration | schema/05 |

### Gemini Flash -- 9/9 done

Schema tasks 45-53 (solo mode tables). All complete.

### Shipped Artifacts (v0.2.0)

`specs/cortex-openapi.yaml`, `examples/gemini-cli/`, `examples/local-llm/`, GitHub Actions release workflow, `CHANGELOG.md`, `cortex export`/`cortex import` CLI, `setup --team` + Argon2id `ctx_` keys.
