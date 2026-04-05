# Cortex Roadmap

Public roadmap for contributors and users. Tasks organized by release milestone with assigned tooling.

**Legend**: Claude Code = CC, Cursor = CU, Codex CLI = CX, Gemini CLI = GC, Droid GLM 5 = D5, Droid GLM 4.7 = D4

---

## v0.4.0 -- Public Launch

All CRITICAL security and OSS-readiness tasks are complete. These remaining items ship with or immediately after the v0.4.0 tag.

| # | Task | Owner | Status |
|---|------|-------|--------|
| 86 | Version bump to v0.4.0 + GitHub release | CU | |
| ✓90 | README rewrite for public audience | GC | **DONE** |
| ✓92 | Review architecture docs: public vs internal vs remove | GC | **DONE** |
| 93 | ~~ROADMAP.md for contributors~~ | -- | Done (this file) |
| 94 | CONTRIBUTING.md + SECURITY.md | CX | |
| 91 | Recall quality baseline (surprise score analysis) | GC | |
| ✓109 | Auto-generate CHANGELOG on version tags (git-cliff) | D4 | Done - cliff.toml + .github/workflows/changelog.yml |
| 113 | Verify desktop app (Control Center) is wired end-to-end | CC | In progress -- CORS + auth header fixes done |
| ✓114 | Wire Agents panel: show connected MCP sessions, active agents, team user presence | D5 | Done - Already wired: AgentItem renders sessions, SSE listens for session events, api(/sessions) fetches data. Fixed CSS bug: --agent-droid had quotes around hex value. |
| ✓115 | Desktop app: fix About icon, update License field, version bump to v0.4.0 | D4 | Done - icon uses BASE_URL, license AGPL-3.0, version 0.4.0 |
| ✓116 | Version bump ALL files to v0.4.0 + AGPL-3.0 license sweep | D4 | Done - all Cargo.toml, package.json, README updated; 49 .rs files have SPDX headers |

---

## v0.5.0 -- Foundation Hardening

Schema discipline, derived-state repair, and memory quality. Makes Cortex reliable enough for daily use.

| # | Task | Source | Owner | Details |
|---|------|--------|-------|---------|
| G1 | TTL / hard expiration (`expires_at` column) | Gemini #1 | D5 | Temporal facts need explicit expiry, not just decay |
| G2 | Session-based rollback (`trace_id` index) | Gemini #2 | D5 | `cortex admin rollback --session-id` for faulty agent runs |
| C1 | Schema versioning + migration epochs | Codex #1 | CC | Named migrations, `cortex doctor` verification command |
| C2 | Derived-state repair (`reindex`, `re-embed`, `recrystallize`) | Codex #2 | CC | Consistency scanners + idempotent rebuild commands |
| C6 | Semantic dedup at store time | Codex #6 | CC | Distinguish "new fact" from "reinforcement of existing" |
| C5 | Boot prompt audit trail | Codex #5 | D5 | Record which sources were included + ranking metadata |

---

## v0.6.0 -- Governance & Economics

Budget controls, retention policies, and human review surfaces. Makes Cortex manageable at scale.

| # | Task | Source | Owner | Details |
|---|------|--------|-------|---------|
| C3 | Budget governance (per-endpoint limits) | Codex #3 | CC | Max recalls/turn, max boot budget, invocation frequency caps |
| C9 | Retention policy classes | Codex #9 | CC | Durable knowledge vs operational context vs audit records vs ephemera |
| C10 | Human review workflows | Codex #10 | CC | Inboxes for shared knowledge, review queues, promotion paths |
| G5 | Dynamic context ranking for injectors | Gemini #5 | D5 | Rank by activeness + relevance, inject top 3-5 items only |
| G3 | Epistemology worker (contradiction triage) | Gemini #3 | CC | Background worker creates resolution tasks for disputed facts |
| C4 | Adapter conformance spec + shared tests | Codex #4 | CX | Canonical behavior spec for store/recall/peek/boot/inject |

---

## v0.7.0 -- Multi-Tenant Hardening

Privacy, fairness, and agent identity for team deployments.

| # | Task | Source | Owner | Details |
|---|------|--------|-------|---------|
| G4 | Deep erasure (`DELETE /forget`) | Gemini #4 | CC | Scrub row + FTS + embedding + re-crystallize affected crystals |
| G8 | Crystal lineage tracking | Gemini #8 | D5 | `source_memory_ids` on crystals, re-crystallize on source delete |
| G9 | Invocation-bound capability tokens (IBCTs) | Gemini #9 | CC | Cryptographic agent identity + scoped authority per request |
| C7 | Multi-tenant fairness (quotas, admission control) | Codex #7 | D5 | Per-user rate/store/recall limits, queue prioritization |
| C8 | Backup, restore, disaster recovery | Codex #8 | CC | Source-of-truth backup, derived rebuild, encrypted key handling |
| G7 | Namespace-isolated embedding spaces | Gemini #7 | CC | Separate HNSW index per team for enterprise security |

---

## v0.8.0 -- Advanced Agent Support

Branch awareness, provenance, deadlock detection for autonomous agent swarms.

| # | Task | Source | Owner | Details |
|---|------|--------|-------|---------|
| G10 | Branch-aware filtering (`git_ref` on store/recall) | Gemini #10 | CC | Prioritize memories from current branch + ancestors |
| G11 | Reasoning provenance (RICR) | Gemini #11 | CC | Provenance links to source commit/session/parent decision |
| G6 | Multi-agent deadlock detection | Gemini #6 | D5 | Dependency graph on tasks, cycle detection, lock-breaking |
| 9 | Chrome extension for claude.ai/chatgpt/gemini | CC | | Manifest V3, content scripts, background worker |
| 40 | Test Chrome extension across 3 platforms | CC | | Depends on #9 |

---

## v1.0.0 -- AI Information Ingester

A new product surface: import, separate, analyze, and index data from external AI platforms.

| # | Task | Owner | Details |
|---|------|-------|---------|
| I1 | ChatGPT export parser (conversations.json) | CC | Parse OpenAI export format, extract decisions/facts/preferences |
| I2 | Claude conversation ingester | CC | Parse claude.ai export, separate by project/topic |
| I3 | Gemini conversation ingester | CC | Parse Google AI Studio / Gemini exports |
| I4 | Intelligent separator (topic detection + classification) | CC | ML/heuristic pipeline to classify: decision, preference, fact, ephemeral |
| I5 | Dedup against existing Cortex memories | CC | Cross-reference ingested data with existing memories |
| I6 | Confidence scoring for ingested data | CC | Lower confidence for older/ambiguous imports |
| I7 | Bulk import CLI (`cortex ingest <export.json>`) | D5 | User-facing command with progress, preview, and dry-run |

---

## Existing Future Tasks (from v0.3.0 cycle)

Carried forward from model_delineation.md. Not yet assigned to a milestone.

### Completed (verified 2026-04-05)

| # | Task | Owner | Status | Evidence |
|---|------|-------|--------|----------|
| ✓58-59 | Owner_id + visibility on remaining tables | D5 | **DONE** | `db.rs:379-580` - All 12 tables have owner_id/visibility columns |
| ✓64-65 | Solo/team mode recall scoping | D5 | **DONE** | `recall.rs:110-126` - `is_visible()` + over-fetch strategy at line 534 |
| ✓67-70 | Conductor ownership + visibility API (4 tasks) | D5 | **DONE** | `conductor.rs` - All endpoints filter by owner_id; tasks/feed have visibility |
| ✓73 | Fresh install defaults to solo mode | D5 | **DONE** | `db.rs:346` - `INSERT ... VALUES ('mode', 'solo')` |
| ✓76 | Role enforcement with CHECK constraints | D5 | **DONE** | `db.rs:322-323, 338-339` - CHECK constraints on role/visibility |
| ✓78 | Row-level NULL owner_id prevention | D5 | **DONE** | `recall.rs:118-120` - `is_visible()` returns false for NULL; migration assigns all rows |

### Remaining (needs implementation)

| # | Task | Owner | Priority | Details |
|---|------|-------|----------|---------|
| 6 | OpenAI function adapter spec + handler | D5 | **HIGH** | No code exists. Need: REST endpoint for function definitions JSON, tool_call handler |
| 19 | Key rotation with 72h grace period | D5 | MEDIUM | No `prev_key_hash`/`prev_key_expires` columns. Need schema migration + auth dual-key check |
| 21 | SQLCipher encryption at rest | D5 | LOW (OPT) | **Optional per spec** - "RECOMMENDED for team mode, not required". Documentation task |
| 22-25 | MCP/OpenAI adapter protocol work (4 tasks) | D5 | PARTIAL | MCP ✅ done. OpenAI ❌ missing (same as #6) |
| 77 | Validate visibility enforcement at query level | GC | LOW | Add integration tests for visibility edge cases |
| 81 | Database size monitoring + growth trajectory | GC | LOW | Metrics endpoint + dashboard |
| 82 | Document deferred features | GC | LOW | Update docs/schema/06 with what's intentionally excluded |

### Day-1 Release Assessment (2026-04-05)

**Security model: COMPLETE**
- All visibility enforcement ✅
- Owner scoping on all tables ✅
- NULL owner_id prevention ✅
- CHECK constraints on enums ✅
- Solo mode default on fresh install ✅

**Not blockers for open-source release:**
- #6 (OpenAI adapter) - Enhancement, users can use Python/TS SDKs
- #19 (Key rotation) - Enhancement, basic auth works
- #21 (SQLCipher) - Optional feature, documentation-only

**Recommendation:** Ship v0.4.0 now, add #6 and #19 in v0.5.0.

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and PR guidelines. Tasks marked with an owner are suggestions, not locks -- anyone can pick up any task. Open an issue or PR to claim work.
