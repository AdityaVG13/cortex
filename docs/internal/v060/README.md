# Cortex v0.6.0 Internal Planning

Kicked off: 2026-04-23
Target cut: 2026-07-16
Latest pushed v0.6.0 head noted here: `b41f7be` on `origin/main`

This folder is intentionally ignored by git. Treat it as the local planning and audit workspace for the v0.6.0 cycle.

## Start Here

Open these first:

1. `unified-status-plan.md` — current source of truth for what is landed, pending, and gated.
2. `plans/accessibility-motion-settings.md` — next U1 Settings/Budgets handoff target.
3. `plans/governance-economics.md` — C3/C9/G5/C4 planning detail.
4. `comprehensive-changelog.md` — dated internal implementation log.
5. `updates-to-readme.md` — release/public-doc staging queue.

## Reality Check

Do not read every `phase-*` file as completed work. The old flat folder mixed executed work, active plans, future recall research, and superseded specs.

Current state as of 2026-05-05:

| Area | Status |
|------|--------|
| H3 port constant sweep | Landed |
| G2 admin rollback CLI | Landed |
| C5 boot audit trail | Partially landed; MCP/OpenAPI wrapper still deferred |
| R2 score-adaptive boot truncation | Landed; benchmark claim still needs release evidence |
| C9 retention classes | Landed |
| RQ0 purity infrastructure | Infra landed; real triad run deferred |
| RQ1 embedding upgrade | Code landed; benchmark gates pending |
| RQ2 reranker | Code landed; local gate **CAUTION**; LongMemEval-S pending |
| C3 budget governance | Backend landed `b41f7be`; U1 Budgets UI/write path pending |
| G5 dynamic ranking | Pending; depends on C9/C5 and benefits from C3 |
| U1 Settings / accessibility / motion | Pending; Budgets UI should follow C3 backend |
| Bridge track | Spec-only for v0.6.0 |
| Later recall phase docs | Future/research plans, not executed |

## Folder Map

| Folder | Contents |
|--------|----------|
| `status/` | Handoffs and focused findings. The live triad stays in the folder root. |
| `scope/` | Scope lock, open questions, v0.5 audit, roadmap reconciliation drafts. |
| `plans/` | Current workstream plans: accessibility/settings, foundation carryovers, governance/economics, recall master plan, bridge/db-stats specs. |
| `execution/` | Executed or near-term recall execution guides for RQ0/RQ1/RQ2. |
| `future/` | Later recall/research execution guides that are not v0.6.0 completed work. |
| `prompts/` | Pasteable `/goal` scripts. |
| `archive/` | Superseded specs or already-executed prompts kept for audit history. |
| `research/` | Research notes and audits used to inform plans. |

## Current Next Pass

Recommended next goal:

Build the U1 Settings/Budgets UI against the C3 backend contract, or pause for LongMemEval-S API credentials if deciding whether RQ2 can move from CAUTION to PASS/FAIL is higher priority.

C3 backend result:

`b41f7be` added configurable per-endpoint budgets via `~/.cortex/budgets.toml`, stable HTTP/MCP denial behavior, `/health.budgets`, `cortex admin budgets status --json`, and `cortex admin budgets validate --path <file> --json`.

## RQ2 Result Snapshot

RQ2 should remain default-off and shadow-first until scored LongMemEval-S runs.

| Metric | Result |
|--------|--------|
| Code commit | `f07d61f` |
| Gate harness commit | `2707913` |
| Artifact commit | `f6c1ebb` |
| Artifact bundle | `benchmarking/results/rq2-rerank-20260505-031510/` |
| Local posture | **CAUTION** |
| Primary top-1 vs off | `0.6667` vs `0.0000` |
| Top-3 | `1.0000` across off/shadow/primary |
| Primary p95 delta | `+19.004ms` |
| Shadow order | Matches off |
| LongMemEval-S | Not run; API/judge key missing |

## Maintenance Rules

- Keep `unified-status-plan.md`, `comprehensive-changelog.md`, and `updates-to-readme.md` in sync after meaningful work.
- Put new `/goal` scripts in `prompts/`.
- Move completed prompt scripts to `archive/` after execution.
- Keep speculative or post-v0.6.0 recall work in `future/`, not the root.
- Do not delete old docs unless their content is intentionally consolidated somewhere else.
- When a doc is superseded, mark that clearly at the top before moving it to `archive/`.

## Upstream References

- `Info/roadmap.md` — public-facing roadmap.
- `docs/internal/archive/pre-v060-docs-2026-04-26.zip::pre-v060/planning/roadmap-internal.md` — original internal v0.6.0 milestone table.
- `docs/internal/archive/pre-v060-docs-2026-04-26.zip::pre-v060/status/CORTEX-UNIFIED-STATUS-PLAN.md` — prior canonical status format.
- `docs/internal/archive/pre-v060-docs-2026-04-26.zip::pre-v060/v050/v050-closeout-plan.md` — v0.5.0 deferred work.
- `docs/internal/archive/pre-v060-docs-2026-04-26.zip::pre-v060/planning/CORTEX-EVOLUTION-PLAN.md` — long-horizon product strategy.
- `docs/internal/archive/pre-v060-docs-2026-04-26.zip::pre-v060/research/PHASE-2A-RESEARCH.md` — adaptive retrieval research.
