# Cortex v0.6.0 Unified Status + Plan

> **v0.6.0 doc triad — update all three after every commit or commit batch.**
>
> Companion docs (update in lockstep with this file):
> - `docs/internal/v060/comprehensive-changelog.md` — living per-commit log
> - `docs/internal/v060/updates-to-readme.md` — README / `Info/roadmap.md` staging queue
>
> **Commit convention:** every v0.6.0 commit message starts with
> `v0.6.0 - [title]` and includes a meaningful description (what + why + any
> validation run). Plugin and daemon version bumps must move in lockstep per
> `Info/plugin-lockstep.md`.
>
---


**Last updated:** 2026-05-05 (C3 budget governance backend plus docs/full-suite evidence through `1231d58`)
**Baseline:** `v0.5.0` shipped on `master` (commit `e64887c` pre-release hygiene pass)
**Target cut:** **2026-07-16** (Thursday). Week-6 checkpoint 2026-06-05.
**Canonical owner doc:** this file
**Purpose:** single source of truth for what is done, what is not done, what is deferred, and what ships next in the `v0.6.0` cycle.

**Scope locked 2026-04-23.** All 7 open questions resolved. See `scope/scope-lock.md` for decision rationale, `scope/open-questions.md` for decision log.

Mirror of `docs/internal/archive/pre-v060-docs-2026-04-26.zip::pre-v060/status/CORTEX-UNIFIED-STATUS-PLAN.md` pattern. All future v0.6.0 planning work updates this file. Keep additive — append, don't overwrite.

---

## 1) Current Reality (No Hype)

- `v0.5.0` cut on master. No regressions detected post-release pass.
- Repo hygiene closed on 2026-04-23: `docs/internal/`, `benchmark/` (stale v0.4.1 dump), `extensions/cortex-chrome-extension/`, `tools/sync-security-rules.sh`, `tools/validate_chrome_extension_policy.py` all untracked and gitignored. Browser-harness scratch patterns added to `.gitignore`. CI `chrome-extension-validation` job removed.
- Public `Info/roadmap.md` v0.6.0 section is stale — 5 of 7 "Foundation Hardening" promises already shipped in v0.5.0 (see `scope/v050-shipped-vs-slipped.md`). Public roadmap update drafted at `scope/public-roadmap-update.md`, awaiting approval.
- Internal `docs/internal/archive/pre-v060-docs-2026-04-26.zip::pre-v060/planning/roadmap-internal.md` v0.6.0 table is the canonical scope starting point: Usability, Accessibility, Governance, Economics + R1/R7 research bridge.
- v0.5.0 closeout (`v050-closeout-plan.md:87`) explicitly committed accessibility/settings/motion to v0.6.0. That commitment holds.
- Claude Code plugin MCP path reworked on 2026-04-26 (`09e97a7`) to be HTTP attach-only. `run-mcp.cjs` now proxies stdio directly to the running daemon's `/mcp-rpc`; SessionStart is status-only; no plugin MCP path shells into `cortex.exe`.
- C9 retention classes landed on 2026-04-26 (`bd85025`): store/MCP/OpenAPI/export/import now support `durable`, `operational`, `audit`, and `ephemeral`; class TTLs feed the existing `expires_at` cleanup path.
- R2 score-adaptive boot truncation landed on 2026-04-26 (`a547a07`): `compiler.rs` now uses score-adaptive capsule allocation when priority variance exists and preserves legacy greedy packing for flat scores.
- RQ1 embedding profile implementation landed on 2026-04-26 (`c971a5a`): `bge-base-en-v1.5` is now the daemon default, MiniLM profiles remain explicit opt-ins, and `qwen3-embedding-0.6b` is available as an opt-in via the live `onnx-community` q8 export. Public README / Info docs remain unchanged until release; release copy is staged in `updates-to-readme.md`.
- Daemon stability hardening landed on 2026-04-30 (`db491ca`): the Control Center gets a supervisor loop, daemon panics write `~/.cortex/panic.log`, handler panics become JSON 500s via `CatchPanicLayer`, and MCP heartbeat tolerance increased to survive normal recovery windows.
- Storage hygiene acceleration landed on 2026-04-30 (`fe78000`, `84d20cc`, `2fb1c20`, `4c3b43c`): compaction now optimizes FTS5 shadow tables, prunes stale-model embeddings, prunes singleton co-occurrence rows, triggers on FTS segment pressure, and migrates legacy f32 embedding/cluster-centroid blobs to PQ8. Live verification reduced the production DB from 412 MB to 26 MB (~94%) with `db_pressure="normal"`.
- RQ2 reranker plumbing landed and was pushed to `origin/main` on 2026-05-05 (`f07d61f`): `ms-marco-MiniLM-L-6-v2` int8 ONNX can be downloaded/loaded directly through `ort` + `tokenizers`, `/recall` supports off/shadow/primary modes behind env flags, health/setup expose reranker readiness, and primary mode only reorders the configured top-N window.
- RQ2 local benchmark gate landed and was pushed on 2026-05-05 (`2707913`, `f6c1ebb`): artifact bundle `benchmarking/results/rq2-rerank-20260505-031510/`, release posture **CAUTION**. Local deterministic gate passes model load, shadow order preservation, owned no-regression, and p95 delta (+19.004ms <= +80ms). Scored Pure LongMemEval-S did not run because no answerer/judge API key was configured, so no public primary-rerank quality claim yet.
- C3 budget-governance backend landed and was pushed to `origin/main` on 2026-05-05 (`b41f7be`; docs/full-suite evidence `efe38a2`, `1231d58`): `~/.cortex/budgets.toml` now supports per-endpoint call budgets for `store`, `recall`, `boot`, and broad `mcp`; missing config is unlimited, invalid config disables enforcement and surfaces health/admin errors, HTTP denials return stable `429` JSON, MCP denials return JSON-RPC error data, and `/health` exposes budget status for U1 Settings.
- Phase-2A retrieval benchmark path is clean: last strict no-helper LongMemEval run is `20/20` on `cortex-http-base` (`benchmarking/runs/amb-run-20260419-074548`). `v0.6.0` retrieval work (R1/R7 measurement-only harness) starts from this baseline.
- v0.6.0 work is landing to `origin/main`. C3 backend/docs evidence is current through `1231d58`; `origin/master` remains at `4c3b43c`.

---

## 2) Completed (High-Confidence)

### Planning infrastructure (2026-04-23)

- `docs/internal/v060/` folder created and gitignored.
- Scope reconciliation done:
  - `scope/scope.md` — Tier 1/2/3 scope draft
  - `scope/open-questions.md` — 7 decision points, #1 and #2 closed by audit
  - `v050-shipped-vs-slipped.md` — proved 5 of 7 public-roadmap v0.6.0 promises already shipped
  - `scope/public-roadmap-update.md` — proposed rewrite for `Info/roadmap.md` v0.5.0 and v0.6.0 sections + `README.md:195`
  - `accessibility-motion-settings.md` — moved from misfiled `v050/` location
  - `reranking-harness.md` — v0.6.0 measurement-only scope for R1 + R7
  - `foundation-carryovers.md` — detailed plans for G2 CLI / C5 audit trail / R2 truncation / H3 port sweep
  - `governance-economics.md` — detailed plans for C9 / C3 / G5 / C4

At initial scope lock, planning surface was the only deliverable. Code landings below are appended as they ship.

### Plugin lifecycle hardening (2026-04-26)

- `09e97a7` landed the Claude plugin MCP hard-rule fix after the 2026-04-25 second-process incident.
- Normal plugin MCP startup no longer resolves canonical/bundled daemon binaries and no longer runs `cortex.exe plugin mcp --agent claude-code`.
- Local fallback is now `http://127.0.0.1:7437` attach-only. If the daemon is absent, the plugin returns `APP_INIT_REQUIRED` instead of trying to start anything.
- Validation: plugin contract tests 12/12, dry-run matrix 8/8, `python tools/audit_spawn_paths.py --strict` clean, live stdin smoke against local daemon clean.

### C9 retention classes (2026-04-26)

- `bd85025` landed retention policy classes across store/MCP/OpenAPI/export/import.
- Class defaults: `durable` no expiry, `operational` 90 days, `audit` 365 days, `ephemeral` 14 days.
- Explicit class wins over entry type; entry type wins over text heuristics; fallback is `operational`.
- Validation: `cargo check --tests`, focused store/import/migration tests, clippy `-D warnings`, and full daemon suite (`465 passed`) green using isolated `CARGO_TARGET_DIR=target-codex-c9`.

### R2 score-adaptive boot truncation (2026-04-26)

- `a547a07` landed score-adaptive boot capsule allocation in `daemon-rs/src/compiler.rs`.
- Flat-score fallback preserves the previous greedy packer exactly; non-flat priority scores allocate token budget proportionally with env-tunable floor/ceiling.
- C5 audit path persists new capsule metadata (`packing`, `allocatedTokens`, `truncated`) through `boot_audits.capsules_json`.
- Validation: focused score-adaptive + flat-fallback tests, clippy `-D warnings`, and full daemon suite (`469 passed`) green using isolated `CARGO_TARGET_DIR=target-codex-r2`.
- Remaining release-proof work: dedicated p50 boot-latency and GT-precision benchmark run before public claims.

### RQ1 embedding profile implementation (2026-04-26)

- `c971a5a` landed the Phase 1 embedding profile code path.
- `daemon-rs/src/embeddings.rs` now defines profile-specific pooling, max-token limits, query/passage prefixes, streaming downloads, and selectable assets for:
  - default `bge-base-en-v1.5` (768-dim, CLS pooling, BGE query instruction prefix)
  - legacy `all-MiniLM-L6-v2` / `all-MiniLM-L12-v2` (384-dim, mean pooling)
  - opt-in `qwen3-embedding-0.6b` (1024-dim, last-token pooling, q8 ONNX export from `onnx-community/Qwen3-Embedding-0.6B-ONNX`)
- Recall and feedback query paths now call `embed_query()` so profile query instructions are applied only to retrieval queries; store, MCP, crystallize, conflict, and backfill paths keep passage embeddings through `embed()`.
- `/health` and `cortex setup` disclose active profile `dimension`, `max_input_tokens`, and `pooling`; setup checks all selected-profile assets.
- No new DB migration was needed: model-tagged embeddings and active-model backlog detection already exist. Public docs were intentionally not edited pre-release.
- Validation: `cargo check --tests`, focused `embeddings` tests (`13 passed`), clippy `-D warnings`, and full daemon suite (`473 passed`) green using isolated `CARGO_TARGET_DIR=target-codex-rq1`.
- Remaining release-proof work: Pure LongMemEval-S improvement, backfill throughput (>=500 emb/hr), and p50 recall regression gates still need measured benchmark artifacts before public claims.

### Daemon stability hardening (2026-04-30)

- `db491ca` landed the smallest viable reliability stack for silent daemon exits.
- Control Center Tauri side now runs a background supervisor thread that respawns the app-managed daemon unless the user intentionally stopped it.
- Daemon startup installs a panic hook that writes panic payload, location, and backtrace to `~/.cortex/panic.log`.
- Axum router is wrapped in `CatchPanicLayer`, converting handler panics into JSON 500 responses without killing the daemon.
- MCP proxy heartbeat recovery tolerance moved from 2 cycles (~30s) to 5 cycles (~75s).
- Validation in commit: rebuilt dev daemon, swapped app-managed binary, single and burst `cortex_store` calls kept daemon healthy.

### Storage hygiene acceleration (2026-04-30)

- `fe78000` landed compaction passes for FTS5 optimize, stale-model embedding pruning, and singleton co-occurrence pruning. Live verification recovered 371 MB (`390 MB -> 27 MB`) and surfaced new `CompactionResult` counters through `/compact`.
- `84d20cc` added an FTS segment-pressure governor trigger so FTS bloat is optimized before the DB crosses size/freelist limits. Tests cover the predicate, row-count helper, and end-to-end segment shrinkage.
- `2fb1c20` changed canonical embedding blob writes from legacy f32 to PQ8 int8 format (`3072 B -> 774 B` for BGE-768) with auto-detect read compatibility, semantic query length filters for both formats, and batched legacy migration.
- `4c3b43c` extended the PQ8 migration to `memory_clusters.centroid` using the same collision-safe 2-byte signature + length-mod-4 detection. Live verification drained 9603 legacy centroids over 10 batched passes, ending at 9701 PQ8 / 0 legacy.
- Remaining storage work: true residual centroid/member encoding is deferred because `cluster_members` stores references, not member vectors; text compression and DB stats CLI remain v0.6.1 candidates.

### RQ2 cross-encoder reranker plumbing + local gate (2026-05-05)

- `f07d61f` adds `daemon-rs/src/rerank.rs` with the `Reranker` trait, `RerankConfig`, `MiniLmReranker`, direct Xenova `ms-marco-MiniLM-L-6-v2` int8 ONNX asset download, `ort` inference, tokenizer pair encoding, and score fusion.
- Runtime state now initializes reranking only when enabled by `CORTEX_RERANK_MODE=shadow|primary` or legacy `CORTEX_RERANK_ENABLED`; default remains off and does not download or load the model.
- `/recall` and recall policy explain now emit `rerankRoute` telemetry. Shadow mode reports reranked candidates without changing order; primary mode reorders only the configured top-N window and marks methods with `+rerank`.
- `/health` and `cortex setup` expose selected model metadata, configured mode, top-N, fusion alpha, and asset/model readiness.
- Code validation: `rtk cargo test --manifest-path daemon-rs/Cargo.toml rerank` -> 6 passed; real model smoke -> 1 passed; `rtk cargo check --manifest-path daemon-rs/Cargo.toml --tests` -> clean; `rtk cargo clippy --manifest-path daemon-rs/Cargo.toml --all-targets -- -D warnings` -> clean; full daemon suite -> 497 passed, all using isolated `CARGO_TARGET_DIR=daemon-rs/target-codex-rq2-gate`.
- Local gate validation: `benchmarking/scripts/rq2_rerank_gate.py` produced committed artifact bundle `benchmarking/results/rq2-rerank-20260505-031510/`. Owned deterministic top-1 improved `0.0000 -> 0.6667`, top-3 stayed `1.0000`, primary p95 delta was `+19.004ms`, shadow order matched off, and health reported reranker `available=true`.
- Release posture: **CAUTION**. Keep public copy shadow/experimental until scored Pure LongMemEval-S proves >= Phase 1 +2pp with no retriever regression.

### C3 budget governance backend (2026-05-05)

- `b41f7be` landed the backend slice for configurable per-endpoint budgets. Follow-up commits `efe38a2` and `1231d58` captured the U1 handoff, release-note queue, and full-suite validation evidence.
- Config contract: `~/.cortex/budgets.toml` with `[defaults] enabled = true|false` plus optional `[endpoints.store|recall|boot|mcp] limit/window_seconds` sections.
- Semantics: missing config is unlimited/backward-compatible; `defaults.enabled = false` validates config while disabling enforcement; missing endpoint section is unlimited; invalid/unknown endpoint config fails visibly/open for availability by disabling enforcement and surfacing a structured health/admin error instead of partially enforcing a surprising policy.
- Enforcement: existing `RateLimiter` now owns budget endpoint buckets keyed by endpoint + source IP, without weakening auth-failure/request-volume protections. Store, recall-family, boot, and HTTP MCP RPC paths check budgets after auth/cheap validation and before expensive work or mutation.
- Denials: HTTP store/recall/boot return stable `429` bodies with `error=budget_exceeded`, endpoint, limit, window, retry hint, and source. MCP RPC returns a JSON-RPC error (`-32029`) with the same metadata in `error.data`.
- Visibility: `/health` exposes `budgets.configLoaded`, `enabled`, `source`, `error`, configured endpoints, and in-memory `recentDenials`. Local CLI shipped: `cortex admin budgets status --json` and `cortex admin budgets validate --path <file> --json`.
- Audit: budget denials write `budget_rejected` event rows with endpoint, limit, window, retry hint, source, request source, and source IP. Health tracks recent denial counters even if event insertion fails.
- Backend files touched: `daemon-rs/src/budgets.rs`, `daemon-rs/src/rate_limit.rs`, `daemon-rs/src/state.rs`, `daemon-rs/src/handlers/mod.rs`, `store.rs`, `recall.rs`, `boot.rs`, `health.rs`, `server.rs`, `main.rs`.
- Documentation files touched: `docs/internal/v060/README.md`, `unified-status-plan.md`, `comprehensive-changelog.md`, `updates-to-readme.md`, `plans/accessibility-motion-settings.md`, `plans/governance-economics.md`.
- Validation: `rtk cargo test --manifest-path daemon-rs/Cargo.toml budget` -> 36 passed; `rtk cargo check --manifest-path daemon-rs/Cargo.toml --tests` -> clean; `git diff --check` -> clean; `rtk cargo clippy --manifest-path daemon-rs/Cargo.toml --all-targets -- -D warnings` -> clean; full daemon suite `rtk cargo test --manifest-path daemon-rs/Cargo.toml` -> 513 passed and recorded in `1231d58`.
- U1 Settings handoff: Settings can consume `/health.budgets` immediately for read-only status, disabled/error states, configured endpoint rows, and recent-denial state. Write/edit support is still a UI/file-write workflow decision; no daemon write endpoint shipped in this slice.

---

## 3) Open Work (Scoped, Not Started)

Ordered by execution sequence. Every entry has file touches, acceptance gates, and estimated effort. Update status in place: `pending → in-progress → in-review → landed` with commit hash.

### 3A) Foundation Carryovers (Tier 1 ride-along, 8-11 days)

| ID | Task | Status | Primary files | Acceptance |
|----|------|--------|---------------|------------|
| H3 | Consolidate `localhost:7437` literals into `DEFAULT_CORTEX_PORT` const | `landed c734886` | `daemon-rs/src/main.rs` (new const; no lib.rs in binary crate), `daemon-rs/src/auth.rs:64`, `desktop/cortex-control-center/src/App.jsx:61,293` | Single const in daemon (`pub const DEFAULT_CORTEX_PORT: u16 = 7437` in `main.rs`); test/doc literals unchanged; `cargo clippy -D warnings` green; 434/434 daemon tests (1 Windows flake isolated); 90/90 desktop tests |
| G2 | Add `cortex admin rollback --session-id` CLI + `session_rolled_back` event | `landed f1d23ae` | `daemon-rs/src/main.rs` (mod admin + admin rollback subcommand + run_admin_rollback_cli), `daemon-rs/src/admin.rs` *(new ~310 LOC)* | Dry-run default; soft-delete via `status='rolled_back'`; idempotent via status-guarded UPDATE; `--json` emits stable RollbackStats; recall excludes rolled-back rows via existing `status='active'` filter (no hot-path edits); event row persisted to `events` table for SSE/audit. Integration test file deferred — 5 unit tests in admin.rs cover full logic against real schema |
| C5 | Boot prompt audit trail (`boot_audits` table + `GET /boot/audit` + MCP tool) | `landed 83509b4` | `daemon-rs/src/db.rs` migration 015 (not 008 — 008 was `client_permissions`), `daemon-rs/src/handlers/boot.rs` audit write in `handle_boot` + new `handle_boot_audit`, `daemon-rs/src/server.rs` route. MCP tool + OpenAPI row deferred. | One row per boot shipped; configurable prune via `CORTEX_BOOT_AUDIT_RETENTION_DAYS` env (default **90 days**, revised from 30 in spec); GET endpoint supports `agent=` + `limit=`; 462/462 tests green, clippy clean. Remaining: MCP tool wrapper + `specs/cortex-openapi.yaml` row. |
| R2 | Score-adaptive truncation in boot assembly (prereq: C5) | `landed a547a07` | `daemon-rs/src/compiler.rs` (actual `/boot` assembly path; `prompt_inject.rs` only fetches `/boot`) | High-score sources get more tokens; flat-score fallback matches current; C5 boot-audit capsules include allocation metadata; p50/GT benchmark still needed before release claim |

### 3B) Accessibility + Settings + Motion (Tier 1 headline, 3-4 weeks)

See `accessibility-motion-settings.md` for full scope. High-level breakdown:

| ID | Task | Status | Primary files | Acceptance |
|----|------|--------|---------------|------------|
| U1 | First-class Settings panel (Accessibility / Appearance & Motion / Connection / Keyboard & Navigation) | `pending; Budgets read contract available b41f7be; docs/full-suite evidence 1231d58` | `desktop/cortex-control-center/src/settings/` *(new tree)*, navigation changes in `App.jsx` | All 4 initial sections render; Budgets can read `/health.budgets`; changes persist to localStorage + optional daemon/file sync; round-trips across app restart |
| U2a | Keyboard-only operation across app | `pending` | All interactive components in `desktop/cortex-control-center/src/` | Manual walkthrough passes: every action reachable without mouse |
| U2b | ARIA pass: dialogs, tablists, live regions | `pending` | Dialog + nav components | Automated axe-core audit + manual NVDA/VoiceOver walkthrough |
| U2c | Reduced-motion runtime plumbing | `pending` | Motion tokens module + all animated components | Respects `prefers-reduced-motion` OS pref; Settings toggle overrides OS |
| U2d | Contrast + non-text contrast pass | `pending` | Theme tokens | WCAG 2.2 AA automated contrast check in CI |
| U2e | Zoom + reflow to `375×812` | `pending` | Layout CSS | Responsive breakpoint test job |
| U3a | Unified sidebar collapsed-width animation | `pending` | Sidebar component | One canonical collapsed width; shared timing token |
| U3b | Panel/tab transition system | `pending` | Tab shell, panel chrome | Short spatial transitions; no hard snap on tab change |
| U3c | Central motion timing + easing tokens | `pending` | New tokens file in `desktop/cortex-control-center/src/design/motion.js` | One shared easing language; reduced-motion bypass |

### 3C) Governance + Economics (Tier 2-essential, 9-12 days)

See `governance-economics.md`. Ordered by dependency. **C4 demoted to Tier 3 stretch 2026-04-24** per scope-lock amendment.

| ID | Task | Status | Primary files | Acceptance |
|----|------|--------|---------------|------------|
| C9 | Retention policy classes (`durable` / `operational` / `audit` / `ephemeral`) | `landed bd85025` | `daemon-rs/src/db.rs` migration 016, `daemon-rs/src/api_types.rs` `RetentionClass`, `daemon-rs/src/handlers/store.rs` classifier, `daemon-rs/src/handlers/mcp.rs` MCP schema/validation, `daemon-rs/src/export_data.rs`, `specs/cortex-openapi.yaml` | Store/MCP support class input; auto-classification heuristics tested; existing rows migrate/default to `operational`; TTL per class; full daemon suite 465/465 |
| C3 | Budget governance - per-endpoint limits via `~/.cortex/budgets.toml` | `backend landed b41f7be; docs/full-suite evidence 1231d58; U1 UI pending` | `daemon-rs/src/budgets.rs`, `rate_limit.rs`, `state.rs`, `handlers/{mod,store,recall,boot,health,mcp via server}.rs`, `server.rs`, `main.rs`; UI still pending | Parser + limiter + store/recall/boot/MCP denial tests; missing config unlimited; health/admin status shipped; Settings read UI can now consume `/health.budgets`; write UI/load test still pending |
| G5 | Dynamic context ranking for injectors (prereq: C9 + C5) | `pending` | `prompt_inject.rs` new `rank_candidates()` pass, `tests/injector_ranking.rs` *(new)* | Score = `w₁·class + w₂·recency + w₃·relevance + w₄·activity`; durable+stale ranks below operational+active; boot latency ±5%; audit records components |

### 3G) Recall improvements Phase 0-2 (Tier 2-essential, 10-12 days) — **NEW 2026-04-24**

Promoted from Tier 3 stretch per `scope-lock.md` amendment. Execution guides already written at code-level fidelity. Runs in parallel with 3C on a second engineer track.

| ID | Task | Status | Primary files | Acceptance |
|----|------|--------|---------------|------------|
| RQ0 | Phase 0 — Purity: `cortex-http-pure` adapter + 5 CI gates + CODEOWNERS + CAS-100 suite + triangle judge + N≥100 dataset scale | `infra landed f4488c3; triad run deferred` | `benchmarking/adapters/cortex_http_pure_provider.py` *(101 LOC, shipped)*, `scripts/purity-gates/*.sh` *(5 shipped)*, `benchmarking/adversarial/cas-100.jsonl` *(shipped f625614)*, `benchmarking/judges/triangle.py` *(shipped 323c5cf)*, `CODEOWNERS` *(shipped)*, `.github/workflows/ci.yml` purity-gates job *(shipped)*, `scripts/benchmark-triad.sh` *(shipped)*, `benchmarking/README.md` *(purity pledge + mode table shipped)*, CHANGELOG correction shipped b9a6458 | Infra complete: adapter + 5 gates all green locally + in CI job; CODEOWNERS protects surface; triad script ready. OUTSTANDING: first real triad run against LongMemEval-S for the pure-mode baseline JSON. Blocked on API credits, not on code. |
| RQ1 | Phase 1 — Embedding upgrade: `bge-base-en-v1.5` default + `qwen3-embedding-0.6b` opt-in | `landed c971a5a; benchmark gates pending` | `daemon-rs/src/embeddings.rs`, `handlers/recall.rs`, `handlers/feedback.rs`, `handlers/health.rs`, `setup.rs`; public docs staged in `updates-to-readme.md` | Code path shipped: BGE default, MiniLM opt-ins, Qwen3 q8 ONNX opt-in, query/passage prefix split, health/setup disclosure. Release gates still required: Pure LongMemEval-S improves ≥3pp absolute over post-Phase-0 baseline; backfill drain ≥500 emb/hr; no p50 regression >10ms |
| RQ2 | Phase 2 — Cross-encoder reranker (`ms-marco-MiniLM-L-6-v2` int8 ONNX via `ort` + `tokenizers`) | `landed f6c1ebb; local gate CAUTION; LongMemEval pending` | `daemon-rs/src/rerank.rs`, `daemon-rs/src/state.rs`, `daemon-rs/src/setup.rs`, `daemon-rs/src/handlers/health.rs`, `daemon-rs/src/handlers/recall.rs`, `benchmarking/scripts/rq2_rerank_gate.py`, `benchmarking/results/rq2-rerank-20260505-031510/` | Code path green with full daemon suite 497/497. Local gate: model load pass, owned no-regression pass, p95 +19.004ms. Release gate still required: Pure LongMemEval-S improves ≥2pp over post-Phase-1 before public primary claim. |

**Cross-references:** `execution/phase-0-purity-execution.md`, `execution/phase-1-embedding-upgrade.md`, `execution/phase-2-reranker-execution.md`, `plans/recall-improvement-plan.md`.

### 3D-bridge) External memory bridge gate spec (week 1, ~1 day)

See `bridge-track-spec.md` (to be written week 1). Zero bridge code ships in v0.6.0.

| ID | Task | Status | Primary files | Acceptance |
|----|------|--------|---------------|------------|
| BR | Bridge-track acceptance gate spec | `landed 2026-04-23 (internal-only doc, gitignored)` | `docs/internal/v060/bridge-track-spec.md` *(270 lines)* | 7 gates documented: quality (≥90% baseline MRR), token cost (≤120%), provenance columns mandatory, fail-closed failure modes, read-only-only in v0.7.0, `BridgeAdapter` reference trait, user-visible Control Center surface. Anti-patterns enumerated. Zero bridge code in v0.6.0 — first reference impl (ChatGPT import) ships v0.7.0 against this spec. |

### 3E) Repowise cleanup pass (weeks 2-3, ~300 LOC cap)

| ID | Task | Status | Primary files | Acceptance |
|----|------|--------|---------------|------------|
| CL | Scoped cleanup — delete / archive / document / false-positive classification | `landed 7460484` | `daemon-rs/src/logging.rs` (deleted, 27 LOC), `daemon-rs/src/mcp_stdio.rs` (deleted, 139 LOC), `daemon-rs/src/main.rs` (4-line edit) | 166 LOC removed (under 300 cap); per-file risk table in commit body; no high-risk deletes; separate commit; 457/457 tests green, clippy -D warnings green |

### 3F) Tier 3 stretch (cut-first candidates) — **AMENDED 2026-04-24**

R1-M superseded by §3G Phase 2 (which now ships production reranker). R1-M no longer a separate workstream.

| ID | Task | Status | Primary files | Acceptance |
|----|------|--------|---------------|------------|
| C4 | Adapter conformance spec + shared contract tests (DEMOTED from Tier 2 on 2026-04-24) | `pending` | `specs/cortex-adapter-contract.yaml` *(new)*, `daemon-rs/tests/adapter_conformance.rs` *(new)*, SDK test mirrors, new CI job `adapter-conformance-matrix` | ≥10 scenarios covered; CI green on MCP + HTTP + Python SDK + TS SDK; first violation surfaces clean diff |
| RQ3.0 | Phase 3.0 ingest extraction — 3-stage pipeline (regex + GLiNER via gline-rs + Qwen2.5-1.5B GBNF via llama-cpp-2) | `future / not v0.6.0 floor` | `daemon-rs/src/extraction/*.rs` *(new tree)*, migration 013, `daemon-rs/tests/extraction_e2e.rs` *(new)* | Pure LongMemEval-S improves ≥5pp on temporal + multi-session subsets; MemoryAgentBench CR-MH baseline measured; p50 store ≤200ms with stage0+1 sync, stage2 async. Execution guide: `future/phase-3-0-ingest-extraction-execution.md` |
| RQ3 | Phase 3 schema — bi-temporal validity + memory_links schema (populated by Phase 3.0) | `pending` | Migration 014, `handlers/recall.rs` temporal filter, `store.rs` insertion | Migrations apply cleanly; temporal filter honored; ≥2pp improvement on LongMemEval temporal subset |

---

## 4) v0.6.0 Theme Focus

### Headline: **Accessibility & Governance**

**Why:** `v0.5.0` closeout explicitly deferred accessibility to `v0.6.0`. Governance backlog has been accumulating since `v0.4.0`. Foundation Hardening work already shipped in v0.5.0 — re-promising it would mislead.

**Non-goal:** broad multi-tenant privacy work. That's v0.7.0.
**Non-goal:** query expansion / HyDE production routing. Reranking scope was amended on 2026-04-24: production rerank may ship behind a flag only if RQ2's no-regression and benchmark gates pass.

### Success criteria (for v0.6.0 closeout)

All of the following must hold at release time:

1. **Accessibility**
   - Every interactive element reachable via keyboard
   - WCAG 2.2 AA contrast compliance on default theme
   - Reduced-motion runtime respects OS preference + Settings override
   - Manual walkthrough passes: NVDA+Firefox, VoiceOver+Safari, Narrator+Edge
   - axe-core automated audit clean on main flows
2. **Settings panel**
   - Four sections populated and functional
   - Settings persist across app restart
   - Budget UI (from C3) renders and round-trips
3. **Governance**
   - C9 retention classes enforced on store + respected by compaction
   - C3 budget 429 responses include machine-readable `Retry-After` and JSON body
   - G5 ranker audit trail (via C5) shows component scores
   - C4 contract tests green on all 3 transports
4. **Foundation carryovers**
   - G2 rollback CLI shipped + tested
   - C5 boot audit trail on every boot call, 30-day prune
   - R2 truncation allocator with variance fallback
   - H3 single `DEFAULT_CORTEX_PORT` const across daemon prod paths
5. **Retrieval quality**
   - RQ0 pure baseline committed when API credits are available
   - RQ1 BGE default has p50/backfill/LongMemEval proof before public claims
   - RQ2 rerank on/off measurement pass committed with no retriever-regression
   - Go/no-go recommendation in `rerank-findings.md`
6. **Release gates**
   - `cargo test` + `cargo clippy -D warnings` green
   - Desktop `npm test` green
   - Adapter conformance CI job green
   - Clean `cargo audit` + `npm audit`
   - OpenAPI spec version-aligned to `0.6.0`
   - `CHANGELOG.md` entry + release notes drafted
   - Public `Info/roadmap.md` v0.6.0 section matches what shipped

---

## 5) Explicit Defer List (Do Not Pull Into v0.6.0)

Defer to `v0.7.0+` unless a critical blocker surfaces:

### Privacy + multi-tenant hardening (v0.7.0 home)
- G4 deep erasure (`DELETE /forget`) across core rows + derived indices
- G8 crystal lineage tracking + re-crystallize on source delete
- G9 invocation-bound capability tokens (IBCTs)
- C7 multi-tenant fairness — quotas, admission control, queue prioritization
- C8 backup/restore/disaster recovery tooling
- G7 namespace-isolated embedding spaces
- C10 human review workflows (inbox + review queue + promotion paths)
- G3 epistemology worker (background contradiction triage)

### Agent orchestration (v0.8.0 home)
- TD1-TD5 task dispatch UI, agent task pull on boot, coordination protocol, dependency DAG, live progress
- G10 branch-aware filtering (`git_ref` on store/recall)
- G11 reasoning provenance (RICR)
- G6 multi-agent deadlock detection

### Data ingestion (v1.0.0 home)
- I1-I7 ChatGPT / Claude / Gemini conversation import + classification + dedup + CLI

### Other
- Chrome extension (separate release track, not yet public)
- External memory bridges (Hindsight/Supermemory) — `v0.6.0` ships gate definition only in `reranking-harness.md` style; actual bridges defer to v0.7.0+
- Broad Repowise cleanup outside validated low-risk paths — open question #5, recommendation to cap at ~300 LOC if pursued
- Cross-instance data mobility + solo→team migration workflows
- MCP OAuth, WebSocket/gRPC transport surfaces, mandatory HMAC signing

References:
- `docs/internal/archive/pre-v060-docs-2026-04-26.zip::pre-v060/planning/roadmap-internal.md` v0.7.0 / v0.8.0 / v0.9.0 / v1.0.0 tables
- `docs/internal/archive/pre-v060-docs-2026-04-26.zip::pre-v060/status/CORTEX-UNIFIED-STATUS-PLAN.md:290-307` (pre-v0.6.0 defer list — still applies)
- `docs/internal/archive/pre-v060-docs-2026-04-26.zip::pre-v060/planning/CORTEX-EVOLUTION-PLAN.md` (long-horizon positioning)

---

## 6) What To Execute Next (Strict Order, Locked 2026-04-23)

### Week 1 (2026-04-27 → 2026-05-03)
1. ~~Apply public roadmap update~~ — `landed 2026-04-25` (rescoped to v0.6.0 = Accessibility, Governance & Recall Quality; v0.7.0 = Multi-Tenant Hardening; v0.8.0 = Advanced Agent Support; chrome extension bullet dropped; recall Phases 0/1/2 surfaced publicly).
2. ~~Write `bridge-track-spec.md`~~ — `landed 2026-04-23` (internal-only doc, 270 lines, 7 gates).
3. ~~Ship H3 `DEFAULT_CORTEX_PORT`~~ — `landed c734886`.

### Weeks 2-3 (2026-05-04 → 2026-05-17)
4. ~~Repowise cleanup pass~~ — `landed 7460484`.
5. Start Accessibility Tier 1A; C9 retention classes already landed (`bd85025`).
6. Start RQ2 reranker implementation on the recall-quality track; RQ0/RQ1 benchmark proof remains a release-gate follow-up.

### Weeks 4-5 (2026-05-18 → 2026-05-31)
7. Accessibility Tier 1B (ARIA, reduced-motion, contrast).
8. ~~C5 boot audit trail~~ — `landed 83509b4`; MCP/OpenAPI wrapper remains release-gate cleanup.
9. ~~G2 rollback CLI~~ — `landed f1d23ae`.

### Week 6 checkpoint (2026-06-05) — mandatory
- If Tier 1 accessibility < 50% complete → drop R1-M stretch.
- If Tier 1 is not on track → drop C4 stretch. C9 prerequisite is landed (`bd85025`).

### Weeks 6-7 (2026-06-01 → 2026-06-14)
10. ~~R2 score-adaptive truncation~~ — `landed a547a07`; benchmark proof remains a release-gate follow-up.
11. ~~RQ1 embedding profile implementation~~ — `landed c971a5a`; benchmark proof remains a release-gate follow-up.
12. ~~C3 budget governance backend~~ - `landed b41f7be`; U1 Settings write/edit path and optional load test remain release follow-ups.
13. G5 dynamic ranking (needs C9 + C5).

### Weeks 8-9 (2026-06-15 → 2026-06-28)
14. Accessibility polish (manual walkthroughs NVDA/VoiceOver/Narrator, contrast pass).
15. C4 adapter conformance (stretch — drop if behind).

### Weeks 10-11 (2026-06-29 → 2026-07-12)
16. RQ2/R1-M benchmark proof: pure LongMemEval-S rerank on/off, retriever-regression guard, and go/no-go writeup.
17. Release gate sweep: `cargo test`, `cargo clippy -D warnings`, desktop `npm test`, adapter conformance, OpenAPI bump to 0.6.0, `cargo audit`, `npm audit`, graphify refresh.

### Week 12 (2026-07-13 → 2026-07-18)
18. Pull final `CHANGELOG.md` v0.6.0 section from `comprehensive-changelog.md`.
18. Public `Info/roadmap.md` v0.6.0 → `shipped`, v0.7.0 → `next`.
19. Screenshot refresh (include new Settings panel).
20. Version bumps (daemon / plugin / marketplace / runtime / spec / package).
21. **Thursday 2026-07-16:** tag + GitHub release with MSI / NSIS / .dmg / .AppImage / .deb + canonical signing.

---

## 7) Gate and Verification Matrix

Every workstream lands only after all gates pass. No "I'll fix it later."

| Gate | When | Command | Pass criteria |
|------|------|---------|---------------|
| Daemon unit tests | Every commit touching `daemon-rs/src/` | `cargo test --manifest-path daemon-rs/Cargo.toml` | All passing, ≥ current v0.5.0 count (255) |
| Daemon clippy | Every commit touching `daemon-rs/src/` | `cargo clippy --manifest-path daemon-rs/Cargo.toml --all-targets -- -D warnings` | Zero warnings |
| Desktop test | Every commit touching `desktop/cortex-control-center/src/` | `npm --prefix desktop/cortex-control-center test` | All passing |
| Desktop build | Every UI-touching PR | `npm --prefix desktop/cortex-control-center run build` | Bundle succeeds |
| Adapter conformance | Every handler + MCP + SDK change | `cargo test --test adapter_conformance` | All scenarios green |
| axe-core audit | Every accessibility-touching PR | Integration in desktop build | No new violations on main flows |
| WCAG manual | Every accessibility milestone | NVDA+Firefox, VoiceOver+Safari, Narrator+Edge walkthrough | Checklist pass |
| OpenAPI drift | Every handler/MCP change | `cargo test spec_version_parity` | Spec matches handler surface |
| Audit + policy | Before release | `cargo audit`, `npm audit`, secret-pattern scan, `.env` tracking check | All clean |
| Graphify refresh | Before release | `graphify update .` | Nodes/edges/communities snapshot captured in `comprehensive-changelog.md` |

---

## 8) Risk Register

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Accessibility work overruns → v0.6.0 slippage | Medium | High | Cut Tier 3 (R1-M harness) first; defer R1-M to a v0.6.1 patch if accessibility needs the full timeline |
| WCAG 2.2 AA contrast pass requires theme changes users dislike | Low | Medium | Land as opt-in toggle first, ramp default theme to comply over two releases |
| C3 budget enforcement introduces 429 churn for existing users | Medium | Medium | Default to unlimited-mode when `budgets.toml` absent; require opt-in for first release |
| C4 conformance spec uncovers real cross-transport bugs | High | Low | Expected — fix before spec freeze; each fix tightens real behavior |
| R1-M measurement signal is ambiguous (small effect within noise) | Medium | Low | Report must include variance + sample size; go/no-go requires effect size ≥ 0.05 |
| ORT / CrossEncoder compatibility on macOS arm64 blocks R1-M harness | Medium | Low | Smoke test Mac first; fall back to `tract` or defer R1-M to v0.6.1 |
| Settings persistence clashes with daemon config | Low | Medium | Settings local-first by default, optional sync via explicit toggle |
| Public roadmap mismatch (if we don't update it) causes user confusion at launch | High | Medium | Apply `scope/public-roadmap-update.md` before release-public-doc pass |

---

## 9) Budget + Sequencing Summary

| Workstream | Effort | Path |
|------------|--------|------|
| Foundation carryovers (H3 + G2 + C5 + R2) | 8-11 days | One-engineer serial |
| Accessibility + Settings + Motion | 15-20 days | Two-engineer parallel (UI + daemon-side wiring) |
| Governance + Economics (C9 + C3 + G5 + C4) | 14-18 days | One engineer serial, C4 can parallelize |
| Reranking measurement harness (R1-M) | 5-6 days | ML-aware engineer, independent |
| Release gate sweep + public roadmap edit | 2-3 days | At cut |
| **Total (sequential, one engineer)** | ~10 weeks | |
| **Total (two engineers parallel where possible)** | ~6-7 weeks | |

---

## 10) Cross-References + Document Catalog

Root-level control plane:

| Path | Status | Role |
|------|--------|------|
| `README.md` | current | Folder index and reality check. |
| `unified-status-plan.md` | current / canonical | Source of truth for landed, pending, deferred, gated, and next work. |
| `comprehensive-changelog.md` | current / canonical | Append-only implementation and planning log. |
| `updates-to-readme.md` | current / canonical | Public README / `Info/roadmap.md` staging queue. |

Status evidence:

| Path | Status | Role |
|------|--------|------|
| `status/SESSION-HANDOFF.md` | historical handoff | 2026-04-24 continuation record. Useful for provenance, not current status. |
| `status/rerank-findings.md` | current RQ2 evidence | CAUTION release posture writeup for local RQ2 gate. |

Scope and roadmap reconciliation:

| Path | Status | Role |
|------|--------|------|
| `scope/scope.md` | locked with amendments | v0.6.0 scope floor/stretch source. |
| `scope/scope-lock.md` | locked | Decision rationale for scope questions. |
| `scope/open-questions.md` | resolved / historical | Records resolved decisions and deferred follow-ups. |
| `scope/v050-shipped-vs-slipped.md` | evidence | Proves which v0.5.0 public promises already shipped. |
| `scope/public-roadmap-update.md` | pending public-doc input | Draft rewrite for `Info/roadmap.md` and README positioning. |

Active and near-term plans:

| Path | Status | Role |
|------|--------|------|
| `plans/accessibility-motion-settings.md` | pending | U1 accessibility/settings/motion master plan. |
| `plans/foundation-carryovers.md` | mostly landed / partial | Tracks H3, G2, C5, R2; C5 MCP/OpenAPI remains deferred. |
| `plans/governance-economics.md` | partially landed / active | C9 and C3 backend are landed; U1 budget UI, G5, and C4 remain pending. |
| `plans/recall-improvement-plan.md` | partially executed | Master recall roadmap. RQ0 infra, RQ1 code, RQ2 code/local gate done; benchmark gates remain. |
| `plans/bridge-track-spec.md` | spec-only | v0.7+ external memory bridge gate; zero bridge code in v0.6.0. |
| `plans/db-stats-cli-spec.md` | future/storage spec | DB stats CLI output contract; not shipped. |

Executed or near-term recall guides:

| Path | Status | Role |
|------|--------|------|
| `execution/phase-0-purity-execution.md` | infra landed / run deferred | Purity adapter and CI gates shipped; real triad run pending API credits. |
| `execution/phase-1-embedding-upgrade.md` | code landed / gates pending | BGE default and Qwen opt-in code path; benchmark gates pending. |
| `execution/phase-2-reranker-execution.md` | code landed / CAUTION | RQ2 implementation guide; final code used direct `ort` + `tokenizers`, not the originally planned fastembed wrapper. |

Future recall/research execution guides:

| Path | Status | Role |
|------|--------|------|
| `future/phase-3-0-ingest-extraction-execution.md` | future / not executed | Ingest extraction pipeline plan. |
| `future/phase-3-5-execution.md` | future / not executed | Contextual prefixing plan. |
| `future/phase-4-adaptive-k-execution.md` | future / not executed | Adaptive-k and time-aware expansion plan. |
| `future/phase-5-hipporag-ppr-execution.md` | future / not executed | HippoRAG PPR + triple-rerank plan. |
| `future/phase-5-5-observer-reflector-execution.md` | future / not executed | Observer/Reflector consolidation loop plan. |
| `future/observer-reflector-prompts.md` | future prompt draft | Prompt text for Phase 5.5 only. |
| `future/phase-6-execution.md` | future / not executed | IRCoT iterative retrieval plan. |

Prompts and archive:

| Path | Status | Role |
|------|--------|------|
| `prompts/c3-budget-governance-goal-prompt.md` | executed / provenance | Produced C3 backend `b41f7be` plus docs/full-suite evidence `efe38a2` and `1231d58`; keep for audit trail, not as the next prompt. |
| `archive/next-big-pass-goal-prompt.md` | executed / archived | RQ2 gate prompt that produced `2707913` + `f6c1ebb`. |
| `archive/reranking-harness.md` | superseded | Original measurement-only reranker harness, superseded by RQ2 Phase 2 implementation. |

Research source library:

| Path | Status | Feeds |
|------|--------|-------|
| `research/a11y-codebase-audit.md` | source evidence | U1 accessibility remediation. |
| `research/a11y-motion-research.md` | source evidence | U1 motion system. |
| `research/a11y-react-libs.md` | source evidence | U1 component/library choices. |
| `research/a11y-wcag22-research.md` | source evidence | WCAG 2.2 AA acceptance. |
| `research/agent-memory-production.md` | source evidence | Future memory-product positioning. |
| `research/benchmark-purity-audit.md` | source evidence | RQ0 purity gates. |
| `research/daemon-bloat-compression.md` | source evidence | Storage hygiene and DB stats CLI. |
| `research/helper-audit-second-pass.md` | source evidence | RQ0 purity follow-up. |
| `research/ingest-extraction-models.md` | source evidence | Future Phase 3.0 extraction. |
| `research/judge-reliability-adversarial-eval.md` | source evidence | RQ0 triangle judge/CAS work. |
| `research/memory-benchmarks-landscape.md` | source evidence | Benchmark selection and recall gates. |
| `research/memory-graph-algorithms.md` | source evidence | Future PPR/HippoRAG work. |
| `research/memory-systems-survey.md` | source evidence | Long-term memory architecture strategy. |
| `research/multihop-retrieval.md` | source evidence | Future multi-hop/IRCoT work. |
| `research/rerank-models-landscape.md` | source evidence | RQ2 model selection. |
| `research/retrieval-sota.md` | source evidence | RQ1/RQ2 and later recall phases. |
| `research/sota-memory-architectures.md` | source evidence | Future Observer/Reflector and memory architecture. |
| `research/sqlite-vec-hybrid.md` | source evidence | Vector/storage strategy. |
| `research/tiny-llm-landscape.md` | source evidence | Future Phase 3.5/6 local model choices. |

Update this file every time a workstream changes status. Date stamp at section top.
