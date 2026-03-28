# Conductor — Master Plan

**Last updated:** 2026-03-28
**Status:** Ideation phase — 40 raw ideas generated, filtering pending

## What Is The Conductor?

A multi-AI orchestration layer built INSIDE Cortex. Not a separate program — new endpoints and a dashboard on the existing daemon (localhost:7437/dashboard). Lets Aditya direct Claude Code, Factory Droid, Codex, and local Ollama models to work on projects simultaneously with shared memory, coordination, and zero redundant token usage.

## Key Design Decisions (Confirmed)

1. **Lives inside Cortex** — not a separate program, not software bloat
2. **Dashboard is a watcher** — user types in each AI's native terminal, Conductor shows unified view
3. **Event-driven coordination** — hooks at session start/end, NOT per-prompt brain checks (4-1 debate consensus)
4. **Local models for grunt work** — Qwen/DeepSeek/GLM do tests, reviews, synthesis at zero API cost
5. **Cortex recall as token multiplier** — every recall eliminates a full research chain
6. **Commercial product potential** — Cortex = brain + orchestration + dashboard. Download it, connect your AIs, get persistent memory

## Documents

| File | Purpose |
|------|---------|
| `ideation/2026-03-28-conductor-raw-ideas.md` | All 40 raw ideas from 5 ideation agents |
| `debates/` | Future: structured debates on architectural decisions |
| `specs/` | Future: detailed specs for chosen features |

## Next Steps

- [ ] Filter 40 raw ideas → top 5-7 survivors (adversarial filtering)
- [ ] Have Droid review and add its perspective via Cortex
- [ ] Brainstorm the top idea in depth
- [ ] Write spec for MVP (Phase 1)
- [ ] TDD: write tests before implementation
- [ ] Build Phase 1 MVP
- [ ] Test cross-AI coordination with real tasks

## Phase 1 MVP (TBD after filtering)

To be determined after adversarial filtering. Likely candidates:
- Activity channel (agent presence + event stream)
- File lock ledger (advisory locks via `/lock` endpoint)
- Boot-injected coordination (extend capsule compiler with workspace awareness)

## Reading Order for New AIs

1. Read THIS file first
2. Read `ideation/2026-03-28-conductor-raw-ideas.md` for the full idea set
3. Read `C:/Users/aditya/cortex/CONNECTING.md` for the Cortex API
4. Read `C:/Users/aditya/cortex/docs/ROADMAP.md` for the broader Cortex roadmap
