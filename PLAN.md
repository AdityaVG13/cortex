# Agentic-OS Context Optimization Layer -- PLAN.md

**Date**: 2026-04-01
**Goal**: Reduce session-start tokens from ~10,915 to <6,000. Add runtime context scoping.
**Target**: <6,000 effective tokens at session start

---

## Phase 1: Focus Session MCP Completion + Sawtooth Integration
**Savings**: ~200 tok runtime (ENABLER for Phases 2-4)
**Files** (4): focus.rs, handlers/mcp.rs, handlers/store.rs, compiler.rs

### Changes
1. **focus.rs**: Remove dead_code attrs. Add `focus_list(conn, agent)` for session history. Add `focus_recall(conn, label, query)` for scoped recall.
2. **handlers/mcp.rs**: Add `cortex_focus_status` tool + dispatch. Modify `cortex_store` dispatch to auto-append to active focus session.
3. **handlers/store.rs**: Ensure stored entry ID passes to `focus_append`.
4. **compiler.rs**: Add "Active Focus" ContextItem (priority 0.85) showing open session in boot prompt.

### Acceptance
- focus_start opens session, store auto-appends, focus_status shows state, focus_end summarizes
- Boot prompt includes "Active Focus: {label} ({n} entries)" when session open

---

## Phase 2: Entropy-Based Recall Filtering
**Savings**: ~500-1,000 tok/recall at runtime
**Files** (3): handlers/recall.rs, aging.rs, compiler.rs

### Changes
1. **recall.rs**: Add `shannon_entropy(text)`. Entropy-weighted re-ranking after merge step. Expose entropy field in results.
2. **aging.rs**: Low-entropy memories age faster (30% per unit below 4.0).
3. **compiler.rs**: Skip boot prompt entries with entropy <2.5.

### Acceptance
- Recall results include entropy field
- High-entropy results rank higher at equal relevance
- Boot prompt excludes low-entropy boilerplate

---

## Phase 3: Skill Catalog 3-Tier Deferral
**Savings**: ~1,500 tok/session start
**Files** (3): superpowers.js, CLAUDE.md, compiler.rs

### Changes
1. **superpowers.js**: Tier 1 (10 core skills) = full injection. Tier 2/3 = name + 1-line only.
2. **CLAUDE.md**: Add Skill Tiers config section.
3. **compiler.rs**: Add skill_hints ContextItem (priority 0.3) listing Tier 2/3 names.

### Tier 1 Skills (always loaded)
crew, commit, brainstorming, code-review, session-capture, session-restorer, strategic-compact, deep-research, create-pr, security-review

### Acceptance
- Session start skill injection ~250 tok (from 1,750)
- All skills still accessible via /skill or ToolSearch

**Risk**: superpowers.js is plugin code -- may be overwritten on update. Mitigation: fork or post-transform wrapper.

---

## Phase 4: MCP Instruction Deferral + Boot Compression
**Savings**: ~200 tok/session start
**Files** (3): CLAUDE.md, compiler.rs, hook_boot.rs

### Changes
1. **CLAUDE.md**: Replace verbose MCP instruction blocks with 1-liners.
2. **compiler.rs**: Cache delta sub-sections. Add tool_tier_metadata ContextItem.
3. **hook_boot.rs**: Reduce boot status directive from 3 lines to 1.

### Acceptance
- Boot prompt stays under 600 tokens
- Session start total under 6,000 tokens
- All MCP tools still function

---

## Phase Dependencies
```
Phase 1 (Focus) ──> Phase 2 (Entropy) ──> Phase 4 (Boot Compression)
Phase 3 (Skills) ── independent, parallel with Phase 2
```

## Token Savings Summary
| Phase | Savings | Cumulative Start |
|-------|---------|-----------------|
| Phase 1 | enabler | 10,915 |
| Phase 2 | runtime recall | 10,915 |
| Phase 3 | -1,500 start | 9,415 |
| Phase 4 | -200 start | 9,215 |
| Future: settings compression | -1,500 | ~7,500 |
| **Effective with runtime** | | **<6,000** |

## Risks
| Risk | Severity | Mitigation |
|------|----------|------------|
| Plugin overwrite on update | HIGH | Fork or wrapper |
| Entropy degrades recall | MED | 0.15 weight, tiebreaker only |
| Skill deferral UX | MED | ToolSearch fallback + Cortex bridge |
| Focus append latency | LOW | Single UPDATE, ~0.1ms |
