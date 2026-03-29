# Cortex Competitive Intelligence — 2026-03-29

For any AI reading this: these are tools in the adjacent space. None compete directly with Cortex.
Our lane (multi-agent persistent shared brain) is currently empty.

## Landscape Summary

| Tool | Stars | Repo | Category | Cortex Overlap |
|------|-------|------|----------|----------------|
| Agent Lightning (Microsoft) | 15,774 | https://github.com/microsoft/agent-lightning | Agent training (RL/APO) | None — training-time, not runtime |
| 724-office | 1,085 | https://github.com/wangziqi06/724-office | Self-evolving chatbot | Partial — validates memory pipeline, single-agent only |
| Parchi | 462 | https://github.com/0xSero/parchi | Browser automation | None — session-scoped, no memory |
| cx | 174 | https://github.com/ind-igo/cx | Code navigation optimizer | None — complementary (structure vs knowledge) |

## What They Lack That Cortex Has
- Multi-agent memory serving (all are single-agent or no-memory)
- Conflict detection between competing memories
- Decay scoring for memory freshness
- Per-agent profiles and retrieval filtering
- Cross-session state management
- Compiled context digests (pre-built briefings)

## Patterns to Steal for Cortex

### High Priority
1. **cortex_peek** — one-line summaries before full recall (from cx cost ladder)
2. **cortex skill** command — emit an optimal agent prompt for self-teaching (from cx)
3. **0.92 cosine threshold** — reference calibration for capsule dedup (from 724-office)
4. **Async capsule compilation** — LLM compression in background threads (from 724-office)
5. **Emit helpers** — `cortex.emit_decision()` for zero-friction stores (from Agent Lightning)

### Medium Priority
6. **Memory pressure eviction** — 70% trigger → evict to 56% for in-memory caches (from Agent Lightning)
7. **Formal JSON-RPC relay** — tighten agent API surface (from Parchi)
8. **Self-check cron** — scheduled self-audits with LLM remediation (from 724-office)
9. **Resource versioning** — immutable snapshots with "latest" pointer (from Agent Lightning)

## Strategic Position
Cortex occupies a unique position: persistent, semantic, multi-agent shared brain.
The market is building orchestration (how agents talk) and training (how agents improve),
but nobody is building the brain (what agents remember together). That's our lane.
