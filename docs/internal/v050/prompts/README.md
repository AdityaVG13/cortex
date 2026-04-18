# v0.5.0 Agent Prompts

Each file is a self-contained prompt for one agent to execute one phase.

**How to use:** Paste `00-agentic-preamble.md` first, then the phase prompt. The agent reads `phase_finished/` for prior phase context automatically.

## Runtime Reliability Note (2026-04-18)

The v0.5.0 runtime hardening stream now includes launch-path fixes that should be treated as already landed context for any lifecycle/reliability prompts:
- app-managed daemon startup in Control Center now forces loopback bind (`127.0.0.1`)
- dashboard refresh is staged to avoid cold-start timeout fanout
- startup-critical refresh fanout now excludes `/savings`; savings analytics refresh only when Analytics panel is active
- IPC timeout budgets are tuned (`/health` warmup budget + aligned abort/transport timing)
- daemon startup heavy maintenance passes are deferred/staggered under app-managed owner flow
- daemon `/savings` now uses SQL-side aggregation + short TTL cache instead of full event-log Rust parsing under one long shared read lock
- dev ensure script now handles stale binary rebuilds and Windows locked-binary retry automatically
- `/health` heavy metrics now use warmup-aware caching (`cache`/`warmup-deferred`) to keep startup responsiveness stable under large event histories
- benchmark ingestion now enforces payload compaction caps (`CORTEX_BENCHMARK_STORE_MAX_CHARS` + tighter matrix fact/context ceilings) to control event/token growth during evaluation runs
- fair-run benchmarking is now fail-closed in both single and matrix mode: gate-bypass shortcuts (`no_enforce_gate`, `allow_missing_recall_metrics`) are rejected at preflight time
- Control Center refresh flow now uses single-flight coalescing to prevent overlapping startup fanout from timer/SSE/retry triggers
- startup-heavy daemon GET handlers now use read-only DB paths and avoid write-side cleanup in read routes
- compaction governor now enforces per-event-type and non-boot event pressure caps so large event families do not grow unbounded

## Agent Roster

| Code | Model | Deployment | Use For |
|------|-------|------------|---------|
| Sonnet | Claude Sonnet 4.6 | CC subagent | Complex Rust, pipelines, critical systems |
| GLM | GLM 4.7 | Droid | Scoped tasks, schema, CLI commands, docs |
| CX | Codex CLI | Codex | Short patches, monitoring, infra |
| CC | Claude Opus 4.6 | Direct | Integration, review, research page |

**Retired:** K2 (Kimi K2.5) -- slow, mixed Chinese output, fabricated completions. D5 (DeepSeek) -- replaced by GLM. D4 -- replaced by GLM.

## Dependency Chain

```
task-0c (Sonnet)       -- DONE (a6e5d9d)
  |
  v
phase-5a (Sonnet)      -- depends on: 0C
  |
  v
phase-5bc (GLM)        -- depends on: 5A

phase-5d (CX)          -- no deps, PARALLEL with 0C/5A/5BC/6
phase-6 (CC)           -- no deps, PARALLEL with everything

phase-1 (Sonnet+GLM)   -- depends on: 0C
  |
  v
phase-2 (Sonnet)       -- depends on: Phase 1

phase-3a (CX)          -- no deps, PARALLEL with Phase 1
  |
  v
phase-3b (CX)          -- depends on: 3A
phase-3c (Sonnet+CC)   -- depends on: Phase 1 + Phase 2
phase-3d (GLM)         -- no deps, PARALLEL with everything

phase-4a (CC)          -- depends on: Phase 2, Phase 3A
phase-4b (Sonnet)      -- depends on: Phase 3A
phase-4c (CC)          -- no deps beyond existing code
phase-4d (GLM)         -- no deps beyond existing code
phase-4e (Sonnet)      -- depends on: 4A (content_type column)
```

## Parallel-Safe Lanes (NEXT)

| Lane | Agent | Prompt | Files |
|------|-------|--------|-------|
| 1 | Sonnet | `phase-5a--sonnet--integrity-gate.md` | db.rs, main.rs |
| 2 | CC | `phase-6--cc--research-page.md` | Info/, README.md |
| 3 | CX | `phase-5d--cx--db-monitoring.md` | health.rs, compaction.rs |

## All Prompts

| File | Agent | Format | Deps |
|------|-------|--------|------|
| `00-agentic-preamble.md` | ALL | Universal | -- |
| `task-0c--k2--baseline-fix.md` | ~~K2~~ Sonnet | Full | None (DONE) |
| `phase-5a--sonnet--integrity-gate.md` | Sonnet | Full | 0C |
| `phase-5bc--glm--wal-backups.md` | GLM 4.7 | Full | 5A |
| `phase-5d--cx--db-monitoring.md` | CX | Codex | None |
| `phase-6--cc--research-page.md` | CC | Full | None |
| `phase-1--sonnet-glm--tiered-retrieval.md` | Sonnet+GLM | Full | 0C |
| `phase-2--sonnet--dedup-quality.md` | Sonnet | Full | Phase 1 |
| `phase-3a--cx--schema-versioning.md` | CX | Codex | None |
| `phase-3b--cx--ttl.md` | CX | Codex | 3A |
| `phase-3--multi-agent--hardening.md` | ALL | Overview | Various |
| `phase-4--multi-agent--intelligence.md` | ALL | Overview | Phase 2, 3A |

## Completion Reports

Every agent creates `docs/internal/v050/phase_finished/{phase}--{agent}.md` when done.
Next-phase agents read these for context. If a dependency report is missing, don't start.

## Prompt Formats

- **Full:** Agentic preamble + detailed task spec (for CC, Sonnet, GLM)
- **Codex:** Goal/Constraints/Mode/Verify/Avoid fields (for CX -- saves tokens)
- **Overview:** Multi-agent coordination doc (for phases with 3+ agents)
