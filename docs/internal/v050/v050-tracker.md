# Cortex v0.5.0 -- Completed Phases Tracker

Compressed record of all completed v0.5.0 work. Each entry includes commit hash, agent, and deliverables. Full task details were in `v050-implementation-plan.md` before being archived here.

---

## Phase 0: Baseline Benchmark -- DONE
- **Commit:** `6bdf63e` | **Agent:** CX (Codex)
- Ran full recall benchmark on v0.4.1. Baseline: GT precision 0.552, MRR 0.692, hit rate 0.900, avg latency 97.5ms. Results in `baseline-v041.md`, `baseline-v041-benchmark.json`.

## Phase 0A: Duplicate Purge -- DONE
- **Commit:** (reported by Codex in final response) | **Agent:** CX
- 29 memories purged, 0 decisions. Post-purge: GT precision 51.3%, MRR 0.74, hit rate 85.0%. FTS orphan check: pass.

## Phase 0C: Boot Savings Baseline Bug -- DONE
- **Commit:** `a6e5d9d` | **Agent:** SN (Sonnet)
- **Branch:** `feat/v050-task-0c`
- Replaced filesystem-scanning `estimate_raw_baseline()` with DB-based baseline (SQL SUM of all active memory/decision text). Boot savings now correct for all agents regardless of CWD.

## Phase 1: Tiered Retrieval + RRF Fusion -- DONE
- **Commits:** `ed1d3a7` (1.1-1.2), `33def16` (1.3), `082f275` (1.4), `ad74a92` (1.5) | **Agents:** SN, D5, CC
- **Branch:** `feat/v050-phase-1-retrieval`
- Full tiered retrieval: Tier 0/1 query cache, FTS5 field boosting + synonym expansion, RRF fusion (k=60), compound scoring (BM25*0.6 + importance*0.2 + recency*0.2). 81 tests passing.

## Phase 3A: Schema Versioning -- DONE
- **Commit:** `145766b` | **Agent:** D4 (GLM-4.7)
- `schema_migrations` table, named migration runner on startup, `cortex doctor` CLI for schema verification.

## Phase 5A: Startup Integrity Gate -- DONE
- **Commits:** `3576c5c` (5A.1), `a82d747` (5A.2-5A.3) | **Agent:** CC
- **Branch:** `feat/v050-phase-5a`
- `PRAGMA integrity_check` on startup, `PRAGMA quick_check` every 30m background task, auto-repair via dump-and-rebuild.

## Phase 5B: Rolling Backups -- DONE
- **Commit:** `980f66b` | **Agent:** CC
- **Branch:** `feat/v050-phase-5bc`
- Rolling daily backups on WAL checkpoint. `cortex backup` and `cortex restore <file>` CLI commands.

## Phase 5C: Crash-Safe WAL Handling -- DONE
- **Commit:** `980f66b` | **Agent:** CC
- **Branch:** `feat/v050-phase-5bc`
- WAL checkpoint every 10s (was 60s), startup WAL recovery, `PRAGMA synchronous` verification.

## Phase 7A: MCP Proxy Session Re-registration -- DONE
- **Commit:** `7081dc1` | **Agent:** D4
- `POST /session/start` on daemon respawn in `mcp_proxy.rs`. Agents panel auto-repopulates. Reconnect flow hardened with session telemetry.

## Phase 7B: Immediate UI State Reflection -- DONE
- **Commit:** `d5d58cd` | **Agent:** D4
- Stop/Start buttons immediately update UI. Agents panel clears on stop, shows "Starting..." then "Running" on health check. Lifecycle commands non-blocking, port cached.

## Phase 7C: Connectivity + Auth Hardening -- DONE
- **Commit:** `9d9b318` | **Agent:** CX (Codex)
- Fixed MCP/HTTP health drift by moving `cortex_health` onto the same payload builder as `/health`, including degraded/db/runtime fields.
- Fixed Codex setup docs and installer flow to use current `codex mcp add cortex -- <exe> mcp` syntax and documented that MCP servers added mid-session require a new Codex session.
- Fixed HTTP usage docs and smoke coverage so protected endpoints consistently include `Authorization: Bearer <token>` and `X-Cortex-Request: true`.
- Relaxed SSRF header parsing so any non-empty `X-Cortex-Request` value is accepted in new builds, preventing false 403s from header-value casing differences.
- Made direct `cortex mcp` startup ensure the daemon without polluting stdio output, while keeping `plugin ensure-daemon` port output for existing callers.

## Phase 7D: CLI Troubleshooting Entry Point -- DONE
- **Commit:** `76f305d` | **Agent:** CX (Codex)
- Added a troubleshooting section to `cortex --help` so users can discover `cortex doctor`, the required HTTP auth headers, the Codex MCP hot-attach limitation, and the app-hosted daemon restart path without reading repo docs first.
- Added README guidance pointing users to `cortex --help`, `cortex doctor`, and `Info/connecting.md` as the primary recovery path for connectivity and auth issues.

## Phase 7E: Review Fixes for Feed + Desktop Auth Retry -- DONE
- **Commit:** `261574e` | **Agent:** CX (Codex)
- Fixed `GET /feed?unread=true` so a stale `feed_acks.last_seen_id` no longer suppresses every unread item after feed TTL pruning removes the anchor row.
- Added daemon regression tests covering both the stale-ack fallback path and the normal "after ack, skip self entries" unread path.
- Fixed Cortex Control Center POST requests to refresh and retry once after missing/stale auth tokens, matching the existing GET behavior during daemon token rotation.
- Added desktop regression tests covering POST token refresh before first call and retry-after-401 flows for both IPC and browser fallback.

## Phase 6A: Public README + Research Redesign -- DONE
- **Commit:** `8a6fdcc` | **Agent:** CX (Codex)
- Rebuilt `README.md` into a stronger landing page with clearer product framing, proof-driven sections, benchmark-backed metrics, sharper nav, and proper `Research` / `Code of Conduct` surfacing in repo-controlled navigation.
- Expanded `Info/research.md` from a paper list into a public design record with richer per-reference adaptation notes, stronger `Inspired by` wording for open-source influences, and explicit shipped / planned / deferred status.
- Added `assets/proof-surface.svg` and `assets/research-lineage.svg` so the public docs now carry a consistent premium visual language instead of relying on plain text alone.

---

## Branches Awaiting Merge to Master
| Branch | Phase | Status |
|--------|-------|--------|
| `feat/v050-task-0c` | 0C | Done, merge ready |
| `feat/v050-phase-5a` | 5A | Done, merge ready |
| `feat/v050-phase-5bc` | 5B+5C | Done, merge ready |
| `feat/v050-phase-1-retrieval` | 1 | Done, merge ready |

---

*Last updated: 2026-04-10*
