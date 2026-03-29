# Cortex Roadmap

**Goal:** A self-improving, multi-AI brain that compounds intelligence across sessions, agents, and projects.

**For the task list, read `docs/TODO.md` — that's the single source of truth for what to build next.**

---

## Architecture

Node.js daemon (HTTP + MCP + SQLite) is the kernel. Python workers handle ML, dashboards, and analysis. Everything talks through HTTP :7437.

```
┌─────────────────────────────────────────────┐
│  Cortex Daemon (Node.js)                    │
│  HTTP :7437 + MCP stdio                     │
│  SQLite, recall, store, conflict, boot      │
│  Capsule compiler, auth, lifecycle          │
│  Conductor: locks, sessions, tasks, events  │
└────────────┬────────────────────────────────┘
             │ HTTP API (universal interface)
    ┌────────┴──────────────────────┐
    │                               │
┌───▼──────────┐  ┌────────────────▼─────────────┐
│ Hooks (JS)   │  │ Python Workers                │
│ brain-boot   │  │ - cortex-dream (compaction)   │
│              │  │ - cortex-dash (Streamlit UI)  │
│              │  │ - cortex-capture (ambient)    │
│              │  │ - ollama workers (review/test) │
└──────────────┘  └──────────────────────────────┘
```

## Completed Milestones

| Milestone | What | When |
|-----------|------|------|
| Foundation | Daemon, SQLite, store/recall, conflict detection, boot compiler, embeddings, auth, MCP | Done |
| Reliability | Fixed embedding coercion, auth gaps, CLI contracts, disputes in boot, test suite | Done |
| Capsule Compiler | Identity + delta capsules, per-agent boot tracking, 96% token reduction | Done |
| Conductor Phase 0 | File locking, activity channel, messaging, boot injection of locks/messages | 2026-03-29 |
| Daemon Lifecycle | brain-boot.js connect-only, cortex-start.bat launcher, no more spawn races | 2026-03-29 |

## Vision (where we're headed)

**Near-term:** Agents know who's online, claim tasks from a queue, and coordinate without human relay. Dashboard shows the full picture.

**Mid-term:** Ambient capture eliminates voluntary reporting. Local models handle reviews and test gen at zero cost. Recall quality improves via decay scoring and usage tracking.

**Long-term:** Self-tuning compiler, event-sourced brain, decision provenance, public release.

## Design Principles

1. **Compound, don't accumulate.** Memories that aren't retrieved should decay. Overlapping facts should merge.
2. **Push, don't pull.** Inject context before being asked. Boot warm, not cold.
3. **Universal interface.** HTTP is the API. Any AI, any language, any platform.
4. **Reliability over intelligence.** A brain that crashes is worse than no brain.
5. **Node for the kernel, Python for the cortex.** Different jobs, different tools.
