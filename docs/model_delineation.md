# Model Delineation -- Cortex Work Assignments

82 tasks from docs/compatibility and docs/schema, assigned by tool and model.

## Tool Reference

| Tool | Model | Cost | Use For |
|------|-------|------|---------|
| Claude Code | Opus 4.6 | $$$$$ | Multi-file Rust, architecture, complex reasoning |
| Cursor | Sonnet / GLM 5 | $-$$ | Single-file edits, inline refactors |
| Codex CLI | GPT-5.3 | $$ | Batch tasks, tests, overnight autonomous |
| Gemini CLI | 2.5 Pro | $ | 1M context analysis, research, docs |
| Droid | GLM 5 (0.4x) | $ | Medium features, API endpoints |
| Droid | GLM 4.7 (0.25x) | ¢ | Config, schema DDL, boilerplate |
| Gemini Flash | 2.5 Flash | ¢ | Formatting, drafts, lookups, light code tasks |

---

## Claude Code (Opus) -- 7 tasks

Architecture, multi-file coordination, complex algorithms.

| # | Task | Source | Done |
|---|------|--------|------|
| 4 | Graceful degradation across all system layers | compatibility/01 | ✓ |
| 9 | Chrome extension with content scripts for claude.ai, chatgpt.com, gemini.com | compatibility/02 | |
| 66 | Over-fetch-then-filter embedding recall strategy (raw_k=max(k*5,50), retry) | schema/04 | ✓ |
| 71 | Admin endpoints: /admin/user/*, /admin/team/*, /admin/assign-owner, /admin/stats | schema/05 | ✓ |
| 72 | CLI commands: cortex user add/rotate-key/remove, cortex team create/add/remove | schema/05 | ✓ |
| 74 | Solo-to-team migration (cortex setup --team, assign legacy rows to owner) | schema/05 | ✓ |
| 40 | Test Chrome extension workflow across 3 platforms | compatibility/06 | |

---

## Cursor (Sonnet/GLM 5) -- 20 tasks

Single-file edits with clear patterns. Tab completion and inline refactors.

| # | Task | Source | Done |
|---|------|--------|------|
| 1 | HTTP REST API as core transport layer | compatibility/01 | ✓ |
| 2 | Unified auth layer validating adapter requests | compatibility/01 | ✓ |
| 3 | TLS via rustls with configurable modes | compatibility/01 | ✓ |
| 5 | MCP adapter: stdio-to-HTTP bridge proxying tool calls | compatibility/02 | ✓ |
| 7 | Python SDK (cortex-memory on PyPI) -- httpx wrapper | compatibility/02 | ✓ |
| 8 | TypeScript SDK (@cortex-memory/client) -- fetch wrapper | compatibility/02 | ✓ |
| 10 | System prompt injector CLI with file-based context auto-refresh | compatibility/02 | ✓ |
| 11 | Standalone fallback mode for solo MCP | compatibility/02 | ✓ |
| 15 | Rate limiting: 10 failed auth/min, 100 req/min per user | compatibility/03 | ✓ |
| 19 | Key rotation with 72h grace period (dual-active keys) | compatibility/03 | ⏳ Droid |
| 21 | SQLCipher encryption at rest (AES-256) for team mode | compatibility/03 | ⏳ Droid |
| 26 | ONNX embedding fallback to FTS5 keyword search | compatibility/05 | ✓ |
| 27 | Daemon crash detection in MCP adapter with retry | compatibility/05 | ✓ |
| 28 | SQLite integrity check on startup with recovery | compatibility/05 | ✓ |
| 31 | Export/import (JSON and SQL formats) | compatibility/05 | ✓ |
| 58 | Add owner_id + visibility to memories, decisions, crystals | schema/03 | ⏳ Droid |
| 59 | Add owner_id + visibility to conductor tables | schema/03 | ⏳ Droid |
| 65 | Team mode recall with visibility filtering | schema/04 | ⏳ Droid |
| 67 | Conductor queries with ownership scoping | schema/04 | ⏳ Droid |
| 80 | ONNX embedding session pooling (2-4 sessions) | schema/06 | ✓ |

---

## Codex CLI -- 10 tasks

Batch autonomous work. Fire-and-forget, review next morning.

| # | Task | Source | Done |
|---|------|--------|------|
| 37 | OpenAPI spec for Custom GPT Actions (Cloudflare tunnel) | compatibility/06 | ✓ |
| 38 | Gemini CLI integration via system prompt injection | compatibility/06 | ✓ |
| 39 | Local LLM integration (llama.cpp, Ollama, LM Studio) | compatibility/06 | ✓ |
| 42 | Validate solo mode schema unchanged | schema/01 | ✓ |
| 43 | Team mode schema with multi-tenancy tables | schema/01 | ✓ |
| 44 | Validate API surface identical in both modes | schema/01 | ✓ |
| 60 | Recreate sessions table with UNIQUE(owner_id, agent) | schema/03 | ✓ |
| 61 | Recreate locks table with UNIQUE(owner_id, path) | schema/03 | ✓ |
| 62 | Recreate feed_acks with PRIMARY KEY(owner_id, agent) | schema/03 | ✓ |
| 75 | Export/import path for user data migration | schema/05 | ✓ |

**Shipped artifacts (v0.2.0, not in table above):** `specs/cortex-openapi.yaml`, `examples/gemini-cli/`, `examples/local-llm/`, GitHub Actions release workflow + `CHANGELOG.md`, version bump `0.2.0`, `cortex export` / `cortex import` CLI, `setup --team` + Argon2id `ctx_` keys (overlaps Droid DDL items 54–57 in implementation).

---

## Gemini CLI -- 5 tasks

1M context analysis, research, documentation.

| # | Task | Source |
|---|------|--------|
| 77 | Validate visibility enforcement at query level (no bypasses) | schema/06 |
| 81 | Database size monitoring and growth trajectory analysis | schema/06 |
| 82 | Document deferred features (org hierarchy, OAuth, Postgres, HNSW) | schema/06 |
| -- | Review entire compatibility/ and schema/ docs for contradictions | meta |
| -- | Analyze recall quality across sample queries after entropy changes | meta |

---

## Droid (GLM 5, 0.4x) -- 12 tasks

Medium complexity, clear patterns, single-module scope.

| # | Task | Source |
|---|------|--------|
| 6 | OpenAI function adapter spec and handler code | compatibility/02 |
| 22 | MCP tool-to-REST endpoint mapping | compatibility/04 |
| 23 | MCP error code translation (HTTP to JSON-RPC) | compatibility/04 |
| 24 | OpenAI function adapter schema definitions | compatibility/04 |
| 25 | Response format translation (REST JSON to LLM-compatible) | compatibility/04 |
| 64 | Solo mode recall (unchanged, no owner filter) | schema/04 |
| 68 | Solo mode API unchanged (POST /store, GET /recall) | schema/05 |
| 69 | Add optional visibility field to POST /store | schema/05 |
| 70 | Add store_response.visibility_confirmed field | schema/05 |
| 73 | Fresh install (solo mode by default) | schema/05 |
| 76 | Role enforcement with CHECK constraints | schema/06 |
| 78 | Row-level NULL owner_id prevention in store handler | schema/06 |

---

## Droid (GLM 4.7, 0.25x) -- 19 tasks

Cheapest. Schema DDL, config, docs, boilerplate.

| # | Task | Source |
|---|------|--------|
| 12 | API key generation: ctx_ prefix, base62, FNV-1a checksum | compatibility/03 |
| 13 | Argon2id password hashing config (64MB, 3 iter, 4 parallel) | compatibility/03 |
| 14 | SSRF protection via X-Cortex-Request header | compatibility/03 |
| 16 | Audit logging for mutations (store, archive, delete) | compatibility/03 |
| 17 | TLS auto-generated self-signed cert or user-provided | compatibility/03 |
| 18 | CORS rejection of non-localhost in solo mode | compatibility/03 |
| 20 | Secret redaction in logs (ctx_ pattern scanning) | compatibility/03 |
| 29 | WAL mode with 60s periodic checkpoint | compatibility/05 |
| 30 | Pre-migration backup (cp cortex.db cortex.db.bak) | compatibility/05 |
| 32 | /health endpoint (unauthenticated) | compatibility/05 |
| 33 | /digest endpoint (detailed system status) | compatibility/05 |
| 34 | Document Claude Code setup flow | compatibility/06 |
| 35 | Document Claude Desktop config setup | compatibility/06 |
| 36 | Document Cursor setup (.cursor/mcp.json) | compatibility/06 |
| 41 | Document direct HTTP REST API + curl examples | compatibility/06 |
| 54 | Add config table (mode: solo vs team) | schema/03 |
| 55 | Add users table with Argon2id api_key_hash | schema/03 |
| 56 | Add teams table with reserved parent_team_id | schema/03 |
| 57 | Add team_members join table with role enforcement | schema/03 |
| 63 | Create partial indexes for visibility-filtered queries | schema/03 |
| 79 | Per-user rate limiting config (100 req/min default) | schema/06 |

---

## Gemini Flash -- 9 tasks (already done)

Schema tasks 45-53 are DONE (solo mode tables exist). Gemini Flash handles formatting, comment cleanup, draft generation, and light code tasks.

---

## Open-Source Release Tasks (v0.3.0-public)

### Claude Code (Opus) -- 3 new tasks

| # | Task | Priority | Details |
|---|------|----------|---------|
| 83 | Fix /unfold visibility bypass (root cause) | CRITICAL | Thread RecallContext through unfold handler. Zero access control currently. |
| 84 | Fix is_visible NULL owner_id policy (root cause) | CRITICAL | Fail closed in team mode. Migration must guarantee zero NULLs. Add CHECK constraint. |
| 85 | Fix MCP per-caller identity (root cause) | HIGH | API key or caller_id per JSON-RPC request. from_state is a workaround, not a fix. |
| 101 | First-run identity: cortex_store must use the new user's identity, not the developer's | CRITICAL | When a new user downloads and runs Cortex, cortex_store/recall/boot must reflect THEIR identity -- not "Aditya" or any hardcoded developer state. Audit every MCP tool handler and HTTP endpoint for hardcoded identity, default agent names, or user-specific data baked into the binary. The capsule compiler, boot prompt, indexer, and knowledge sources all need to initialize fresh for a new user. First `cortex serve` on a clean install must produce a blank brain with the new user's context, not the developer's. |

### Cursor -- 8 new tasks

| # | Task | Priority | Details |
|---|------|----------|---------|
| 86 | Version bump to v0.3.0 + git tag + GitHub release | HIGH | Cargo.toml, CHANGELOG.md, build release binary, attach to GH release. |
| 87 | Desktop app: sidecar the real daemon binary | CRITICAL | Delete embedded_daemon.rs (3000+ lines of duplicated, drifted code). Tauri app launches cortex.exe as a sidecar process. One download = one installer that bundles both. User double-clicks, daemon starts, dashboard opens, /health confirms green. |
| 88 | App icon: replace default with adityasmile.png | MEDIUM | In desktop/cortex-control-center/src-tauri/icons/: remove all old icons, rename adityasmile.png to icon.png, generate required sizes (icon.ico, icon.icns, 32x32, 128x128, 128x128@2x). Single source image, script the resize. Check tauri.conf.json for required entries. |
| 89 | README rewrite: release badge, download link, "What's New in v0.3.0" | MEDIUM | Top badge box currently empty. Add release link, version badge, feature highlights. |
| 97 | Desktop app: fix all dead UI, remove Ollama box | HIGH | Start/Stop buttons must actually launch/kill the daemon (currently do nothing). Audit every button and field across all 11 panels. Remove the Ollama status box. If a feature isn't wired up, remove the UI element -- don't ship dead buttons. |
| 98 | Desktop app: add About tab (panel #12) | MEDIUM | Shows creator photo (icon.png) and "Created by Aditya". Contributors section that updates as people contribute (GitHub contributors API or manual list). Display app version number. |
| 99 | Desktop app: auto-update via tauri-plugin-updater | MEDIUM | In-app update check -- button or notification when new version is available. Document manual update process in README for users who download directly. |
| 100 | Desktop app: version sync to 0.3.0 | HIGH | Tauri app version, Cargo.toml version, package.json version all must match daemon v0.3.0. |

### Gemini CLI -- 3 new tasks

| # | Task | Priority | Details |
|---|------|----------|---------|
| 90 | README rewrite for public audience | HIGH | 1M context read of entire repo. Rewrite for external developers, not internal team. Remove personal references. |
| 91 | Recall quality baseline analysis | MEDIUM | Distribution of surprise scores across 220+ decisions. Define thresholds for meaningful/noise. |
| 92 | Review architecture docs: classify as public/internal/remove | MEDIUM | docs/architecture/, docs/compatibility/, docs/schema/, docs/archive/ -- what stays, what goes. |

### Codex CLI -- 2 new tasks

| # | Task | Priority | Details |
|---|------|----------|---------|
| 93 | ROADMAP.md for contributors | HIGH | Process all architecture docs (Codex + Gemini longterm considerations) into public roadmap with clear contribution areas. |
| 94 | CONTRIBUTING.md + SECURITY.md | HIGH | Dev setup, build instructions, PR guidelines, vulnerability disclosure policy. |

### Droid (GLM 4.7) -- 2 new tasks

| # | Task | Priority | Details |
|---|------|----------|---------|
| 95 | Repo cleanup: delete personal files, update .gitignore | HIGH | Delete: CLAUDE.md, AGENTS.md, GEMINI.md, .cursor/, .aider*, .planning/, PLAN.md, RECON.md, cortex-profiles.json, personal scripts, docs.zip, cortex_corrupt.db, debug-*.log. Add 20+ patterns to .gitignore. |
| 96 | Remove legacy Node.js src/ or add deprecation notice | LOW | Rust daemon is the product. Legacy code confuses contributors. |

---

## Summary

| Tool | Original | New | Total |
|------|----------|-----|-------|
| Claude Code (Opus) | 7 | 3 | 10 |
| Cursor (Sonnet/GLM 5) | 20 | 8 | 28 |
| Codex CLI | 10 | 2 | 12 |
| Gemini CLI | 5 | 3 | 8 |
| Droid (GLM 5) | 12 | 0 | 12 |
| Droid (GLM 4.7) | 19 | 2 | 21 |
| Gemini Flash | 9 | 0 | 9 |
| **Total** | **82** | **18** | **100** |

---

## Notes (2026-04-04)

### Opus tasks shipped (5/7)
Commits 4fb00ea..f889d80 on master. 8 commits total (5 features + 2 critic fixes + 1 line endings).

### Open-source readiness: 85%
All 36 critical-path features shipped. Blocking items: 3 security root causes (#83-85), repo cleanup (#95), missing docs (#93-94). Desktop app functional (11 panels) but needs version sync and icon update (#87-88).

### Blockers before Chrome extension (#9, #40)
- Need a **team-mode test environment** with 2+ users to validate visibility filtering end-to-end

### Recall quality analysis (Gemini CLI task #91)
Surprise metric from cortex_store has no baseline. Need distribution analysis across all 220+ decisions to know what the scores actually mean. Without this, surprise is a number, not a signal.
