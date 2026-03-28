# Phase 4 Spec: cortex-dream (Compaction Worker)

**Goal:** A Python process that runs periodically (daily or on-demand), reads all memories and decisions from Cortex via HTTP, uses a local LLM (Ollama) to find duplicates, synthesize canonical rules, and flag stale entries — then writes the results back to Cortex.

---

## Why This Matters

After 50+ sessions, Cortex will have hundreds of memories. Many will overlap ("use uv not pip" stored 4 times with slightly different wording). Some will conflict silently (missed by real-time detection because they were stored days apart). Some will be outdated but never decayed because they were retrieved once.

The dreaming worker turns a pile of memories into a curated knowledge base.

---

## Architecture

```
┌─────────────────────────────────────────┐
│  cortex-dream (Python)                  │
│                                         │
│  1. GET /recall?q=*  (fetch all)        │
│  2. Cluster by semantic similarity      │
│  3. For each cluster:                   │
│     - Ollama: "synthesize these into    │
│       one canonical rule"               │
│     - POST /store (canonical version)   │
│     - Mark originals as 'archived'      │
│  4. Flag stale entries for review       │
│  5. Generate dream report               │
└─────────────────────────────────────────┘
```

## Input

- All active memories: `GET /health` for counts, then batch fetch
- All active decisions: query via recall or direct DB access
- Need a new endpoint: `GET /dump?type=memories&status=active` that returns all rows (for batch processing, not search)

## Processing Steps

### 1. Fetch all active entries
New daemon endpoint needed: `GET /dump` returns all active memories and decisions as JSON arrays. This is for batch worker use only, not for AI boot.

### 2. Embed and cluster
Use Ollama's embedding endpoint to get vectors for all entries. Cluster using cosine similarity (threshold: 0.80). Group entries that are semantically similar.

### 3. Synthesize clusters
For each cluster with 2+ entries, send to local LLM (GLM-4.7 or Qwen 2.5 via Ollama):

```
Prompt: "These are overlapping memories from an AI coding assistant's brain.
Synthesize them into ONE canonical rule that captures the essential knowledge.
Keep it under 100 words. Preserve specific technical details.

Entries:
1. {entry1}
2. {entry2}
3. {entry3}

Canonical rule:"
```

### 4. Store canonical, archive originals
- `POST /store` the synthesized canonical rule with `confidence: 0.9` and `source_agent: 'cortex-dream'`
- Mark original entries as `status: 'archived'` (new status, not deleted)
- Store lineage: canonical entry's `context` field lists the IDs it was synthesized from

### 5. Flag stale entries
Entries with `score < 0.3` AND `retrievals = 0` AND `age > 14 days` → flag for review.
Don't auto-delete. Write a report.

### 6. Generate dream report
Write to `~/.cortex/dream-reports/YYYY-MM-DD.md`:

```markdown
# Cortex Dream Report — 2026-03-29

## Synthesized
- "Use uv for Python" (merged 3 entries → 1 canonical)
- "Windows bash path" (merged 2 entries → 1 canonical)

## Flagged for Review
- "Old OMEGA config" (score: 0.15, 0 retrievals, 21 days old)

## Stats
- Entries before: 152
- Entries after: 143 (9 archived, 2 synthesized)
- Clusters found: 4
- LLM calls: 4
```

---

## Local LLM Options

The user has these models available via Ollama:
- **GLM-4.7-Flash** (just downloaded) — good for synthesis tasks, uncensored
- **Qwen 2.5 32B** — strong reasoning, slower on CPU
- **nomic-embed-text** — already used for embeddings

Recommended: Use GLM-4.7 for synthesis (faster), nomic-embed-text for clustering (already integrated).

---

## Implementation Plan

### Files to create
- `workers/cortex_dream.py` — main worker script
- `workers/cortex_client.py` — shared HTTP client for Cortex API

### Daemon changes needed
- Add `GET /dump` endpoint (returns all active memories + decisions)
- Add `POST /archive` endpoint (bulk status change to 'archived')
- Or: extend `POST /forget` to support `action: 'archive'`

### Dependencies (Python)
- `httpx` — async HTTP client
- `ollama` — Python SDK for local LLM calls
- No torch/numpy needed — Ollama handles embeddings

### Trigger options
- Manual: `python workers/cortex_dream.py`
- Cron: Windows Task Scheduler daily at 3am
- Hook: SessionStart hook checks if >24h since last dream, triggers if needed

---

## Safety Rules

1. **Never delete originals immediately.** Archive first. Delete only after 30 days archived.
2. **Never auto-resolve conflicts.** Flag them. Human confirms.
3. **Synthesis must preserve technical specifics.** "Use uv" is useless. "Use uv for all Python package management, never pip directly" is useful.
4. **Rate limit LLM calls.** Max 20 synthesis calls per dream run.
5. **Dream report is mandatory.** Every run produces a human-readable report.
6. **Dry run mode.** `--dry-run` shows what would happen without changing anything.

---

## Who Should Build This

This is a good Codex task if using the cheap plan (less context needed — it's a standalone Python script that talks to Cortex via HTTP). Or a Claude task if you want tighter integration with the existing architecture.

The daemon changes (dump/archive endpoints) should be done by whoever built the daemon (Claude), then the Python worker can be built independently.
