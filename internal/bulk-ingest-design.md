# Bulk Ingest Design — thousands of sessions, eight harnesses, one brain

Problem: first launch on a lived-in Mac means absorbing thousands of
sessions/transcripts (pi, codex, claude, …) — ranked, stored, recallable —
without ballooning the DB, blocking the hot path, or surrendering the
no-LLM-on-path bet. LLM assistance is welcome *driving* ingestion (as a
client); it must never sit *inside* it.

## Scale math (why this is tractable)

- 1,000 sessions × ~500KB raw ≈ 500MB. Deflate on JSON text ≈ 5–10× →
  ~50–100MB cold. FTS index ≈ 2–3× hot text, but only hot tiers stay
  indexed; cold rehydrates verbatim on `expand`.
- The binding constraints are ingest throughput, FTS size, recall
  precision at scale, and writer contention — not raw bytes. SQLite + WAL
  handles all four with the design below.

## The shape: two planes, same as ever

Bulk ingest is the observation/CQR split at scale:

1. **Observation plane absorbs everything verbatim.** Each session becomes
   a registered source (generation = BLAKE3 of canonical bytes); each
   message/turn becomes an event with byte cursors and replay-safe
   receipts. Nothing parsed away, nothing summarized away.
2. **CQR plane gets proposed structure.** An LLM driver (or deterministic
   fallback) proposes deposits — retention classes, thread assignment,
   obligation extraction — each **citing `obs:` evidence**. Cortex
   verifies (redaction, conflict, anchors) and commits or rejects.
   Lossy proposals are safe because the verbatim layer beneath is intact.

The LLM proposes; Cortex disposes. The bet holds: zero models between an
experience and its record.

## Pipeline phases

- [ ] **Phase A — Discover + capture (deterministic, no LLM needed).**
  Transcript importers per format (pi JSONL, codex, claude sessions):
  deterministic parsers → canonical bytes → content-hash dedup (same
  bytes absorbed once, even across harnesses) → RefZero intern (shared
  identity) → observation sources + events with cursors. Idempotent:
  re-running ingest over the same files is a no-op by generation match.
- [ ] **Phase B — Structure (LLM-assisted, deterministic fallback).**
  Driver proposes: thread membership, retention classes, deposits for
  durable facts/decisions, obligation extraction. Deterministic fallback
  when no LLM: anchors/entities/threads from structure alone. Every
  deposit cites `obs:` evidence; every rejection is logged with cause.
- [ ] **Phase C — Rank + tier (governed maintenance, off hot path).**
  Initial rank from structure (threads, refs, co-occurrence); credit
  accrues with use. Aging tiers move bytes cold; crystals consolidate
  Jaccard clusters by reference; GC burns ephemera. Ingest runs as
  budgeted background maintenance — never blocking deposits or recall.
- [ ] **Phase D — Verify.** Recall-positive spot checks over ingested
  sessions (known-answer probes: "what did session X decide about Y?").
  Ingest defects show up as recall misses, caught by the eval suite.

## Multi-harness rules

- One brain, N principals/scopes — isolation already exists (owner_id,
  principal, scope labels, policy-isolated stats). Eight harnesses are a
  supported shape, not a stress case.
- Cross-harness dedup by content hash + RefZero: identical bytes intern
  once, referenced per principal. Shared knowledge, isolated credit.
- Writer contention: WAL + busy timeouts + the runtime lock discipline;
  bulk ingest yields to interactive deposits (maintenance budgets).

## What we refuse (even at thousand-session scale)

- No summarization of record. Derived rollups exist only as labeled,
  sourced, retractable revisions.
- No silent drops. Anything not absorbed is reported with cause, re-runnable.
- No hot-path LLM. The driver is a client; outage degrades to
  deterministic fallback, never to corruption.
