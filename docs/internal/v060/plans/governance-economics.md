# v0.6.0 — Governance & Economics

> Make Cortex manageable at team scale without pulling full multi-tenant privacy work forward (that's v0.7.0). Focus on **what a team admin needs to see and control** by default.

---

## C9 — Retention policy classes *(foundational — ship first)*

**Problem:** All memories today are equal citizens. Durable architectural decisions compete with one-off debug observations for storage and recall budget. No admin signal to distinguish "keep forever" from "drop after 30 days".

### Classes

| Class | Example | Default TTL | Decay curve |
|-------|---------|-------------|-------------|
| `durable` | Architecture decisions, coding conventions, API contracts | none (∞) | flat |
| `operational` | Current task context, recent file edits, active debugging | 90 days | linear decay after day 30 |
| `audit` | Security events, rollbacks, permission grants | 365 days | flat within window |
| `ephemeral` | Ping-pong chatter, throwaway notes, transient observations | 14 days | exponential |

### Schema

Migration 009 adds `retention_class TEXT NOT NULL DEFAULT 'operational'` to `memories`, `decisions`. `CHECK` constraint to enum above.

### Store-path classification

Auto-classify on store using (in order):
1. Explicit `retention_class` param if caller supplied it
2. Decision `type` → class mapping (`decision` → `durable`, `observation` → `operational`, `trace` → `audit`, `chatter` → `ephemeral`)
3. Text heuristic: presence of "architectural", "convention", "always", "never" → `durable` bias
4. Fallback: `operational`

Classification logged in `boot_audits.decisions_json` (ties back to C5).

### Existing TTL (G1) is the enforcement layer

G1's `expires_at` gets populated from `retention_class` default at store time. No new compaction path — existing cleanup loop reads `expires_at` as it does today.

### Files touched

- `daemon-rs/src/db.rs` — migration 009
- `daemon-rs/src/store.rs` — classifier before insert
- `daemon-rs/src/types.rs` — `RetentionClass` enum
- `daemon-rs/tests/retention_classes.rs` *(new)*
- `specs/cortex-openapi.yaml` — `retention_class` field on store request

### Acceptance

- [ ] All 4 classes round-trip through store → recall → export
- [ ] Defaults apply when caller omits the field
- [ ] Existing stored rows migrated to `operational` (safest middle)
- [ ] Classifier heuristics have rule-by-rule unit tests
- [ ] TTL enforcement works end-to-end per class

**Effort:** 3-4 days.

---

## C3 — Budget governance

**Problem:** No per-endpoint caps today. An agent loop can hammer `/recall` 500× a minute and no signal surfaces until the DB gets slow.

**Backend status 2026-05-05:** Landed and pushed in `b41f7be` (`v0.6.0 - C3: add budget governance backend`). This first slice is call-count based and deliberately does not implement token budgets, per-agent durable accounting, billing, cloud accounts, or adapter work.

### Landed backend contract

`~/.cortex/budgets.toml` now uses the v0.6.0 backend contract:

```toml
[defaults]
enabled = true

[endpoints.store]
limit = 120
window_seconds = 60

[endpoints.recall]
limit = 300
window_seconds = 60

[endpoints.boot]
limit = 60
window_seconds = 60

[endpoints.mcp]
limit = 240
window_seconds = 60
```

Semantics:
- Missing `budgets.toml` means unlimited/backward-compatible.
- `defaults.enabled = false` disables enforcement while preserving syntax validation.
- Missing endpoint sections are unlimited for that endpoint.
- Unknown endpoint names or non-positive `limit` / `window_seconds` fail closed by disabling enforcement and surfacing structured health/admin errors.
- Denials write `budget_rejected` events when the DB is available and always increment in-memory recent-denial counters.

Visibility:
- `/health.budgets` exposes `configLoaded`, `enabled`, `source`, `error`, configured `endpoints`, and `recentDenials`.
- Local admin CLI shipped:
  - `cortex admin budgets status --json`
  - `cortex admin budgets validate --path <file> --json`

Validation:
- `rtk cargo test --manifest-path daemon-rs/Cargo.toml budget` -> 36 passed.
- `rtk cargo check --manifest-path daemon-rs/Cargo.toml --tests` -> clean.
- `git diff --check` -> clean.
- `rtk cargo clippy --manifest-path daemon-rs/Cargo.toml --all-targets -- -D warnings` -> clean.
- `rtk cargo test --manifest-path daemon-rs/Cargo.toml` -> 513 passed.

Files touched:
- `daemon-rs/src/budgets.rs`
- `daemon-rs/src/rate_limit.rs`
- `daemon-rs/src/state.rs`
- `daemon-rs/src/handlers/mod.rs`
- `daemon-rs/src/handlers/store.rs`
- `daemon-rs/src/handlers/recall.rs`
- `daemon-rs/src/handlers/boot.rs`
- `daemon-rs/src/handlers/health.rs`
- `daemon-rs/src/server.rs`
- `daemon-rs/src/main.rs`

U1 Settings handoff:
- Settings can consume `/health.budgets` immediately for read-only status, disabled/error states, endpoint rows, and recent-denial state.
- Writing/editing `budgets.toml` from Control Center remains the next UI/backend workflow decision; no daemon write endpoint shipped in this slice.

### Scope

Per-endpoint limits, enforceable via `~/.cortex/budgets.toml`:

```toml
[recall]
max_tokens_per_call = 2000
max_calls_per_minute = 60
max_depth = 20

[boot]
max_tokens_per_call = 2000
max_calls_per_minute = 20

[store]
max_rows_per_minute = 120

[global]
per_agent_daily_recall_tokens = 500_000
```

### Enforcement

- Existing rate-limit infra in `daemon-rs/src/rate_limit.rs` extended with budget-aware buckets per endpoint.
- Budget violation → 429 + machine-readable `Retry-After` + JSON body with `{reason, current, limit, window}`.
- All rejections logged to `events` table with `kind = 'budget_rejected'` (counts toward audit class).

### UI

Settings panel section **"Budgets"** (part of the accessibility/settings program — dependency tie-in):
- Show current usage vs limit for each endpoint, past 1h window
- Edit caps (writes to `budgets.toml`)
- "Pause enforcement" toggle for debug sessions (logs the toggle)

### Files touched

- `daemon-rs/src/rate_limit.rs` — extend `AgentBucket` with per-endpoint tiers
- `daemon-rs/src/config.rs` *(new or existing)* — `budgets.toml` loader
- `daemon-rs/src/handlers/*.rs` — inject budget check before expensive work
- `desktop/cortex-control-center/src/settings/budgets.jsx` *(new)*

### Acceptance

- [x] 429 path tested for each of recall/boot/store
- [x] Unlimited-budget default (backward compatible) when `budgets.toml` absent
- [x] MCP exhausted path returns JSON-RPC error data
- [x] Health/admin visibility shipped for Settings read path
- [x] Rejection events persisted as `budget_rejected`
- [ ] Settings UI round-trips values
- [ ] Load test: sustained 100 rps on `/recall` with cap=60/m → expected 40% rejection rate, stable latency

**Effort:** backend slice landed 2026-05-05; U1 UI/write slice remains.

---

## G5 — Dynamic context ranking for injectors

**Problem:** Boot prompt injects fixed sources in fixed order. Stale "durable" memory shows ahead of today's active work.

### Scope

Replace static injector order with score = `w₁·class_weight + w₂·recency_score + w₂·relevance_score + w₃·activity_score` where:
- `class_weight` from retention class (durable=1.0, operational=0.8, audit=0.4, ephemeral=0.2)
- `recency_score` = decay function on `updated_at`
- `relevance_score` from recall hit (already computed)
- `activity_score` = touch count in last 24h (new column `last_touched_at` already present on most tables)

Top-N (default N=5) injected, rest shelved. N configurable per agent via Settings.

### Ties into R2

R2's score-adaptive truncation allocates tokens **within** G5's selected top-N. G5 picks, R2 sizes.

### Files touched

- `daemon-rs/src/prompt_inject.rs` — new `rank_candidates()` pass before existing truncation
- `daemon-rs/src/config.rs` — weight config
- `daemon-rs/tests/injector_ranking.rs` *(new)*

### Acceptance

- [ ] Unit tests per component (class, recency, activity)
- [ ] A durable architecture decision with 0 recent activity ranks below an operational task touched 10 minutes ago
- [ ] Boot latency unchanged (± 5%)
- [ ] Boot audit (C5) records rank components for each injected source

**Effort:** 2-3 days. Depends on C9 (needs retention_class) and C5 (audit writes).

---

## C4 — Adapter conformance spec + shared tests

**Problem:** MCP tools, HTTP endpoints, and SDKs each have their own tests. Behavior drift between transports has already happened (e.g. auth semantics). No single source of truth.

### Scope

Canonical behavior spec in `specs/cortex-adapter-contract.yaml` covering:
- Equivalent surface across transports (every MCP tool has an HTTP counterpart with identical semantics)
- Auth rules (which calls require tokens, which accept loopback bypass, rate-limit buckets)
- Error envelope shape (`{code, reason, retryable, details}`)
- Idempotency keys (store + forget + rollback)

Shared contract test runner in `daemon-rs/tests/adapter_conformance.rs`:
1. Boots the daemon
2. For each entry in the spec, runs the same scenario through MCP, HTTP, and Python+TS SDKs
3. Asserts identical response envelopes (modulo transport-specific fields)

New CI job `adapter-conformance-matrix` runs the full sweep.

### Files touched

- `specs/cortex-adapter-contract.yaml` *(new — machine-readable)*
- `daemon-rs/tests/adapter_conformance.rs` *(new)*
- `sdks/python/tests/test_conformance.py` *(new)*
- `sdks/ts/tests/conformance.test.ts` *(new)*
- `.github/workflows/ci.yml` — new job

### Acceptance

- [ ] At least 10 scenarios covered (store / recall / boot / forget / permissions / etc.)
- [ ] CI job green across 3 transports
- [ ] First violation surfaces a clear diff, not just a failed assert
- [ ] Spec version tagged with daemon semver (drift gate)

**Effort:** 5-6 days.

---

## Combined budget

| Task | Days |
|------|------|
| C9 retention classes | 3-4 |
| C3 budget governance | 4-5 |
| G5 dynamic ranking | 2-3 |
| C4 adapter conformance | 5-6 |
| **Total** | **14-18 days** |

This is the heavier governance subset — Tier 2 of the scope doc. Can be trimmed to C9 + C3 (7-9 days) if scope question #3 picks the tight option.

---

## Sequencing

1. **C9 first** — prereq for both C3 (budget enforcement can use retention class to prioritize which rows to reject first) and G5 (ranker needs class weights).
2. **C3** in parallel with C5 (from carryovers) — both touch Settings panel.
3. **G5** after C9 and C5 (needs audit trail).
4. **C4** — independent; schedule whenever an engineer is free.

## Dependencies on other v0.6.0 work

- Budget Settings UI needs the Accessibility/Settings program's panel chrome (`accessibility-motion-settings.md`)
- G5 audit logging needs C5 (boot audit trail, in carryovers)
- C4 contract tests must run under the clippy/test CI gates shipped in v0.5.0 (H5/H6)

## Explicitly deferred to v0.7.0

- **C10 Human review workflows** — requires UI surface (review queue, inbox) that's too much to land alongside accessibility in v0.6.0
- **G3 Epistemology worker** — contradiction triage background worker depends on C4 adapter conformance being locked
- **G7 Namespace-isolated embedding** — multi-tenant privacy hardening
- **Audit export / compliance reports** — needs C10 first
