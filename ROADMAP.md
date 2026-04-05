# Cortex Roadmap

Public roadmap for contributors and users. Tasks organized by release milestone with assigned tooling.

**Legend**: Claude Code = CC, Cursor = CU, Codex CLI = CX, Gemini CLI = GC, Droid GLM 5 = D5, Droid GLM 4.7 = D4

---

## v0.4.0 -- Public Launch

All CRITICAL security and OSS-readiness tasks are complete. These remaining items ship with or immediately after the v0.4.0 tag.

| # | Task | Owner | Status |
|---|------|-------|--------|
| 86 | Version bump to v0.4.0 + GitHub release | CU | |
| 90 | README rewrite for public audience | GC | |
| 92 | Review architecture docs: public vs internal vs remove | GC | |
| 93 | ~~ROADMAP.md for contributors~~ | -- | Done (this file) |
| 94 | CONTRIBUTING.md + SECURITY.md | CX | |
| 91 | Recall quality baseline (surprise score analysis) | GC | |
| 109 | Auto-generate CHANGELOG on version tags (git-cliff) | D4 | |
| 113 | Verify desktop app (Control Center) is wired end-to-end | CC | |

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

| # | Task | Owner | Details |
|---|------|-------|---------|
| 6 | OpenAI function adapter spec + handler | D5 | compatibility/02 |
| 19 | Key rotation with 72h grace period | D5 | compatibility/03 |
| 21 | SQLCipher encryption at rest | D5 | compatibility/03 |
| 22-25 | MCP/OpenAI adapter protocol work (4 tasks) | D5 | compatibility/04 |
| 58-59 | Owner_id + visibility on remaining tables | D5 | schema/03 |
| 64-65 | Solo/team mode recall scoping | D5 | schema/04 |
| 67-70 | Conductor ownership + visibility API (4 tasks) | D5 | schema/04-05 |
| 73 | Fresh install defaults to solo mode | D5 | schema/05 |
| 76 | Role enforcement with CHECK constraints | D5 | schema/06 |
| 78 | Row-level NULL owner_id prevention | D5 | schema/06 |
| 77 | Validate visibility enforcement at query level | GC | schema/06 |
| 81 | Database size monitoring + growth trajectory | GC | schema/06 |
| 82 | Document deferred features | GC | schema/06 |

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and PR guidelines. Tasks marked with an owner are suggestions, not locks -- anyone can pick up any task. Open an issue or PR to claim work.
