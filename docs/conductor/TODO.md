# Conductor — Master Plan (ARCHIVED)

**Superseded by `docs/TODO.md` — read that instead.**

**Last updated:** 2026-03-28
**Status:** Phase 0 COMPLETE. Planning moved to docs/TODO.md.

## What Is The Conductor?

A multi-AI orchestration layer built INSIDE Cortex. Not a separate program — new endpoints and a dashboard on the existing daemon (localhost:7437/dashboard). Lets Aditya direct Claude Code, Factory Droid, Codex, and local Ollama models to work on projects simultaneously with shared memory, coordination, and zero redundant token usage.

## CURRENT PRIORITY: Phase 0 — File Locking & Inter-Agent Communication

**Why this is first:** Claude and Droid already collided on the same file (mcp.json incident, 2026-03-28). Nothing else works until agents can see what each other are doing and avoid stepping on each other.

### Phase 0 Scope

#### 0a: File Lock Ledger
- `POST /lock` — agent claims a file (path + agent name + TTL, default 5min)
- `POST /unlock` — agent releases a file
- `GET /locks` — list all active locks
- Auto-expire after TTL
- Return 409 with holder info if file already locked
- In-memory map (no new table needed for MVP)

#### 0b: Activity Channel
- `POST /activity` — agent reports what it's doing (agent + description + files)
- `GET /activity?since=5m` — poll recent activity from all agents
- `POST /message` — inter-agent message ("don't touch auth.js")
- `GET /messages?agent=X` — get messages for a specific agent

#### 0c: Boot-Injected Awareness
- Extend capsule compiler to include active locks in delta capsule
- On boot, agent sees: "Droid is editing src/routes.js — avoid this file"
- On boot, agent sees: "Claude sent you a message: review my auth changes"

### How Claude and Droid Coordinate (Interim Process)

Until Phase 0 is built, Aditya relays between terminals:
1. Tell Claude what to work on → Claude stores decision to Cortex
2. Tell Droid what to work on → Droid recalls Claude's decisions first
3. If either AI needs to edit a shared file, Aditya says "check with the other AI first"

After Phase 0: agents check `/locks` and `/activity` automatically via hooks.

### Spec
Read `specs/phase-0-file-locking.md` for implementation details (to be written).

### Tests
Write tests BEFORE implementation (TDD). Test file: `test/conductor.test.js`

## Key Design Decisions (Confirmed)

1. **Lives inside Cortex** — not a separate program, not software bloat
2. **Dashboard is a watcher** — user types in each AI's native terminal, Conductor shows unified view
3. **Event-driven coordination** — hooks at session start/end, NOT per-prompt brain checks (4-1 debate consensus)
4. **Local models for grunt work** — Qwen/DeepSeek/GLM do tests, reviews, synthesis at zero API cost
5. **Cortex recall as token multiplier** — every recall eliminates a full research chain
6. **Commercial product potential** — Cortex = brain + orchestration + dashboard
7. **File locking is FIRST priority** — proven by real collision incident (mcp.json, 2026-03-28)

## Documents

| File | Purpose |
|------|---------|
| `ideation/2026-03-28-conductor-raw-ideas.md` | All 40 raw ideas from 5 ideation agents |
| `specs/phase-0-file-locking.md` | Phase 0 implementation spec (TBD) |
| `debates/` | Structured debates on architectural decisions |
| `specs/` | Detailed specs for chosen features |

## Future Phases (after Phase 0)

- [ ] Filter 40 raw ideas → top 5-7 survivors (adversarial filtering)
- [ ] Have Droid review and rank ideas via Cortex
- [ ] Dashboard UI at localhost:7437/dashboard
- [ ] Task routing with agent capability profiles
- [ ] Ollama sidecar workers (test gen + review)
- [ ] Ambient capture with inbox/promotion pipeline
- [ ] Commercial packaging

## Reading Order for New AIs

1. Read THIS file first
2. Read `specs/phase-0-file-locking.md` for what we're building NOW
3. Read `ideation/2026-03-28-conductor-raw-ideas.md` for the full idea set
4. Read `C:/Users/aditya/cortex/CONNECTING.md` for the Cortex API
5. Read `C:/Users/aditya/cortex/docs/ROADMAP.md` for the broader Cortex roadmap
