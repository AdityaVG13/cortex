# v0.6.0 — Foundation Carryovers

> Four v0.5.0 slips that ride along with the accessibility + governance headline. Low-scope, high-confidence. Each one has a small, verifiable done state.

---

## G2 — Session rollback CLI

**Status:** schema shipped in v0.5.0 (`trace_id` column on `feed`, `events`, `decisions`). Operator tooling missing.

**Problem:** Agent goes off the rails → user wants to unwind every store/decision from that session. Today they'd have to hand-craft SQL against a column most users don't know exists.

### Scope

Add subcommand `cortex admin rollback --session-id <id> [--dry-run] [--json]`:

1. Resolve `session-id` to its `trace_id` via `sessions` table.
2. Enumerate affected rows across `memories`, `decisions`, `feed`, `events` where `trace_id = ?`.
3. Dry-run default — prints count per table + sample rows, makes no changes.
4. With `--commit`: soft-delete (flip `status` to `rolled_back`), never physical delete. Emit one `session_rolled_back` event with full affected ID list for traceability.
5. Idempotent — running twice is a no-op on second pass.

### Out of scope

- Cascading rollback of crystals / derived state (user runs `cortex recrystallize` after if needed)
- Automatic rollback on agent-feedback thresholds (later version)

### Files touched

- `daemon-rs/src/main.rs` — new `admin rollback` CLI branch near existing `reindex`/`re-embed` at ~line 613
- `daemon-rs/src/admin.rs` *(new)* — rollback logic + event emission
- `daemon-rs/tests/admin_rollback.rs` *(new)* — fixture-based test: seed session → rollback → assert row counts + event

### Acceptance

- [ ] Dry-run on non-existent session returns "0 affected" cleanly
- [ ] Commit on existing session flips rows in 4 tables + emits event
- [ ] `status = 'rolled_back'` rows excluded from default `/recall`
- [ ] `--json` output matches OpenAPI spec addition
- [ ] Tests + clippy green

**Effort:** 2-3 days.

---

## C5 - Boot prompt audit trail

**Status:** landed in two slices: base audit table/HTTP endpoint `83509b4`, MCP/OpenAPI completion `0a4575d`.

**Problem:** Hard to debug recall quality without knowing which sources contributed to the returned capsule and how they ranked. Users need to know whether poor recall came from bad inputs, bad ranking, or truncation.

### Scope

Actual shipped table: migration 015 creates `boot_audits` with `agent`, `profile`, `budget_tokens`, `token_estimate`, `token_savings`, `capsules_count`, `capsules_json`, `latency_ms`, and `created_at`.

On every HTTP `/boot` or MCP `cortex_boot` call:
1. Assemble prompt as today.
2. After assembly, write one `boot_audits` row with capsule metadata and token accounting.
3. Prune rows older than `CORTEX_BOOT_AUDIT_RETENTION_DAYS`; default is 90 days, and `0` disables prune.

Endpoint: `GET /boot/audit?agent=X&limit=N` returns recent audit rows, newest first, cap 500.
MCP tool: `cortex_boot_audit({ agent?, limit? })` is a read-permission wrapper over the same query helper.

### Out of scope

- Real-time audit UI in Control Center (separate task if valuable)
- Per-source contribution scoring / attribution beyond current capsule metadata

### Files touched

- `daemon-rs/src/db.rs` - migration 015 for `boot_audits` table
- `daemon-rs/src/handlers/boot.rs` - HTTP audit write/query helper and `GET /boot/audit`
- `daemon-rs/src/handlers/mcp.rs` - MCP `cortex_boot` audit write + `cortex_boot_audit` tool
- `daemon-rs/src/server.rs` - `/boot/audit` route
- `specs/cortex-openapi.yaml` - `/boot/audit`, `BootAuditResponse`, and `BootAudit` schemas

### Acceptance

- [x] Every HTTP `/boot` call writes exactly one audit row.
- [x] Every MCP `cortex_boot` call writes exactly one audit row.
- [x] Configurable auto-prune runs in boot path; default 90 days.
- [x] `GET /boot/audit` returns well-formed JSON matching spec.
- [x] `cortex_boot_audit` MCP tool is advertised, read-scoped, and tested.
- [x] OpenAPI spec includes `/boot/audit` response schemas.
- [x] Clippy green; full daemon suite 515/515 on `0a4575d`.
- [ ] Storage overhead measured on realistic workload (target: < 1MB / 1000 boots); still release-polish evidence, not a code blocker.

**Effort:** shipped.

---

## R2 — Score-adaptive truncation for boot

**Status:** slipped quietly in v0.5.0 despite SmartSearch research backing it. Current truncation is fixed-size capsules.

**Problem:** Fixed capsule sizes waste budget on low-confidence matches and starve high-confidence ones. SmartSearch paper showed 98.6% recall collapses to 22.5% post-truncation without score-weighted allocation.

### Scope

Replace fixed per-source token caps in `prompt_inject.rs` with a score-weighted allocator:

1. Compute per-source scores (already done — feed from recall).
2. Normalize scores across all candidates.
3. Allocate token budget proportionally to normalized score, with floor (every selected source gets ≥ `MIN_SOURCE_TOKENS`) and ceiling (no single source > `MAX_SOURCE_TOKENS`).
4. Fall back to current fixed allocation if score signal is flat (variance below threshold — e.g. all sources near-equal).

Expose via env vars for tuning: `CORTEX_BOOT_MIN_SOURCE_TOKENS` (default 40), `CORTEX_BOOT_MAX_SOURCE_TOKENS` (default 600).

### Files touched

- `daemon-rs/src/prompt_inject.rs` — replace truncation logic; existing tests augmented
- `daemon-rs/tests/boot_truncation.rs` *(new)* — variance + proportional allocation tests

### Acceptance

- [ ] High-score sources get more tokens than low-score on the same query
- [ ] Flat-score fallback matches current behavior exactly
- [ ] p50 boot latency unchanged (± 5%)
- [ ] Boot-audit trail (C5) reflects new allocation decisions
- [ ] Regression test on v0.5.0 baseline fixture shows ≥ 0.02 GT precision gain on boot-derived recall
- [ ] Clippy green

**Effort:** 2-3 days. Gated on C5 so the audit trail shows the new decisions.

---

## H3 — localhost:7437 sweep

**Status:** 15+ literal `7437` occurrences across daemon + desktop + docs. Some are legit (examples, tests, CSP headers); some leak the port into code paths that should read config.

### Scope — three-pass sweep

**Pass 1: Consolidate the single source of truth.**
- `daemon-rs/src/lib.rs` (or new `consts.rs`): `pub const DEFAULT_CORTEX_PORT: u16 = 7437;`
- Rust code sites (`auth.rs:64`, daemon_lifecycle fixtures, etc.) read from the const, not the literal.

**Pass 2: Desktop.**
- `desktop/cortex-control-center/src/App.jsx:61,293` — replace with `DEFAULT_CORTEX_PORT` exported from a shared config module.
- Test files keep literals (that's fine — tests pin specific values).

**Pass 3: Docs + CSP.**
- CSP headers in `tauri.conf.json`: keep literal but annotate with comment pointing at `DEFAULT_CORTEX_PORT`.
- `README.md`, `Info/connecting.md` curl examples: keep literals (users need copy-pasteable commands), but add a one-liner near the top: "examples assume default port 7437; override with `CORTEX_PORT`".

### What does *not* change

- Test fixtures with hardcoded `7437`
- Docs curl examples (deliberate)
- CSP strings in tauri.conf.json (static, but annotated)

### Files touched

- `daemon-rs/src/lib.rs` or new `consts.rs`
- `daemon-rs/src/auth.rs`, `daemon-rs/src/daemon_lifecycle.rs` (prod paths only — test paths keep literals)
- `desktop/cortex-control-center/src/App.jsx` + any import sites
- `README.md`, `Info/connecting.md` — one-liner note
- `tauri.conf.json` — annotate

### Acceptance

- [ ] Single `DEFAULT_CORTEX_PORT` const in daemon
- [ ] `grep -rn "7437" daemon-rs/src/ desktop/cortex-control-center/src/` shows only literals in test files and annotated strings
- [ ] `cortex start --port 9999` works end-to-end (smoke test)
- [ ] No doc churn beyond the one-liner
- [ ] Clippy green

**Effort:** 1 day.

---

## Combined budget

| Task | Days |
|------|------|
| G2 rollback CLI | 2-3 |
| C5 boot audit trail | 3-4 |
| R2 score-adaptive truncation | 2-3 |
| H3 localhost sweep | 1 |
| **Total** | **8-11 days** |

Small next to accessibility (the dominant v0.6.0 workstream). Can be assigned to one engineer in sequence, or parallelized across two.

---

## Sequencing

1. **H3 first** — trivial, unblocks clean diffs on the larger tasks.
2. **C5** next — boot audit trail is prereq for R2's verification story.
3. **R2** — builds on C5's audit output.
4. **G2** independent — schedule whenever.

## Dependencies

- C5 → R2 (R2 needs the audit trail to verify allocation)
- All four benefit from the accessibility program shipping a unified Settings panel — `CORTEX_BOOT_MIN_SOURCE_TOKENS` etc. get user-facing switches there instead of being env-var only.
