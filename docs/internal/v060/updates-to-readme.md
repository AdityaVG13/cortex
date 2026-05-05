# README Update Queue — v0.6.0 (Living)

> **v0.6.0 doc triad — update all three after every commit or commit batch.**
>
> Companion docs (update in lockstep with this file):
> - `docs/internal/v060/unified-status-plan.md` — canonical status + workstream tracker
> - `docs/internal/v060/comprehensive-changelog.md` — living per-commit log
>
> **Commit convention:** every v0.6.0 commit message starts with
> `v0.6.0 - [title]` and includes a meaningful description (what + why + any
> validation run). When deciding whether a commit touches public-facing copy,
> stage the entry here first — cull at release draft time. Plugin and daemon
> version bumps must move in lockstep per `Info/plugin-lockstep.md`.
>
---


Last updated: 2026-05-05 (C3 backend + docs/full-suite evidence through `1231d58`)
Scope baseline: `v0.5.0` (master) -> C3 evidence through `1231d58` on `origin/main`
Target: `v0.6.0` release cut

## Purpose

Staging queue for README + public-docs updates earmarked for the v0.6.0 release. Intentionally broader than the final README copy — capture everything that could land, then cull at release time.

## How To Use

1. Keep this file **additive** while v0.6.0 work progresses.
2. For each entry, keep at least one concrete commit reference as work lands.
3. When drafting final README + `Info/roadmap.md` copy at release cut, move only validated, externally appropriate content.
4. Explicitly mark entries as `DROPPED` (don't delete) if a theme doesn't ship — the history matters.

---

## Internal Source Catalog For Public Copy

This section maps the reorganized `docs/internal/v060/` files to their public-doc implications. It is not a promise that every source ships public copy. Use it to avoid losing release notes hidden in planning docs.

| Source | Public-doc status | README / roadmap implication |
|--------|-------------------|------------------------------|
| `README.md` | internal-only | Folder index; no public copy. |
| `unified-status-plan.md` | canonical source | Use to decide what is actually shipped, gated, pending, or deferred. |
| `comprehensive-changelog.md` | canonical source | Source for final `CHANGELOG.md` v0.6.0 cut. |
| `updates-to-readme.md` | canonical source | This file; final public-copy staging queue. |
| `scope/scope.md` | internal scope source | Public roadmap should reflect Accessibility & Governance plus validated recall work only. |
| `scope/scope-lock.md` | internal rationale | No direct copy; informs why bridge/code scope is deferred. |
| `scope/open-questions.md` | historical decisions | No direct copy except unresolved roadmap/benchmark caveats. |
| `scope/v050-shipped-vs-slipped.md` | public correction source | Use to avoid re-promising v0.5.0 shipped features as v0.6.0 work. |
| `scope/public-roadmap-update.md` | pending public copy | Draft source for `Info/roadmap.md` and README positioning. |
| `plans/accessibility-motion-settings.md` | pending headline copy | Release copy only after U1/U2/U3 land and accessibility validation exists. |
| `plans/foundation-carryovers.md` | partial public copy | H3/G2/C5/R2 entries only where landed; C5 MCP/OpenAPI still deferred. |
| `plans/governance-economics.md` | C9 + C3 backend landed | C9 copy can ship; C3 backend copy can ship as governed local budgets with health/admin visibility; C3 Settings write UI and G5 copy wait for implementation/tests. |
| `plans/recall-improvement-plan.md` | partially public | RQ1/RQ2 copy must stay benchmark-gated and cautious. |
| `plans/bridge-track-spec.md` | spec-only | Public roadmap may mention future bridge criteria, not shipped bridge code. |
| `plans/db-stats-cli-spec.md` | future | No public copy unless DB stats CLI lands. |
| `execution/phase-0-purity-execution.md` | infra landed / benchmark deferred | Copy may mention purity infrastructure; do not claim completed LongMemEval triad run. |
| `execution/phase-1-embedding-upgrade.md` | code landed / gates pending | BGE default copy waits for benchmark gate language to stay precise. |
| `execution/phase-2-reranker-execution.md` | code landed / CAUTION | Reranker copy must say default-off, shadow-first, experimental/local validation. |
| `future/phase-3-0-ingest-extraction-execution.md` | future | No v0.6.0 public copy. |
| `future/phase-3-5-execution.md` | future | No v0.6.0 public copy. |
| `future/phase-4-adaptive-k-execution.md` | future | No v0.6.0 public copy. |
| `future/phase-5-hipporag-ppr-execution.md` | future | No v0.6.0 public copy. |
| `future/phase-5-5-observer-reflector-execution.md` | future | No v0.6.0 public copy. |
| `future/observer-reflector-prompts.md` | future | No v0.6.0 public copy. |
| `future/phase-6-execution.md` | future | No v0.6.0 public copy. |
| `prompts/c3-budget-governance-goal-prompt.md` | executed internal aid | No direct public copy; evidence is captured in the C3 budget governance queue entry. |
| `archive/next-big-pass-goal-prompt.md` | executed prompt | No public copy; evidence is captured through RQ2 entries. |
| `archive/reranking-harness.md` | superseded | Do not use for current public claims. |
| `status/SESSION-HANDOFF.md` | historical handoff | No direct copy; provenance only. |
| `status/rerank-findings.md` | RQ2 evidence | Supports cautious reranker release wording. |
| `research/a11y-codebase-audit.md` | source evidence | U1 accessibility copy only after fixes and validation land. |
| `research/a11y-motion-research.md` | source evidence | U1 motion copy only after motion tokens/reduced-motion behavior land. |
| `research/a11y-react-libs.md` | source evidence | No direct copy; informs Settings/accessibility implementation. |
| `research/a11y-wcag22-research.md` | source evidence | WCAG copy only after automated/manual evidence exists. |
| `research/agent-memory-production.md` | source evidence | No v0.6.0 capability claim by itself. |
| `research/benchmark-purity-audit.md` | source evidence | Supports benchmark-purity copy only through shipped gates. |
| `research/daemon-bloat-compression.md` | source evidence | Supports storage-hygiene copy already staged under Storage Hygiene. |
| `research/helper-audit-second-pass.md` | source evidence | Supports RQ0 purity caveats; no standalone copy. |
| `research/ingest-extraction-models.md` | future source | No v0.6.0 public copy. |
| `research/judge-reliability-adversarial-eval.md` | source evidence | Supports judge/triad wording only after benchmark runs. |
| `research/memory-benchmarks-landscape.md` | source evidence | Benchmark names acceptable; no score claim without committed results. |
| `research/memory-graph-algorithms.md` | future source | No v0.6.0 public copy. |
| `research/memory-systems-survey.md` | future source | No v0.6.0 public copy. |
| `research/multihop-retrieval.md` | future source | No v0.6.0 public copy. |
| `research/rerank-models-landscape.md` | source evidence | Supports RQ2 model choice; public copy still CAUTION. |
| `research/retrieval-sota.md` | source evidence | Supports RQ1/RQ2 framing; no unvalidated quality claim. |
| `research/sota-memory-architectures.md` | future source | No v0.6.0 public copy. |
| `research/sqlite-vec-hybrid.md` | source evidence | Supports vector/storage strategy only through landed code. |
| `research/tiny-llm-landscape.md` | future source | No v0.6.0 public copy. |

Public-copy rule after the catalog pass: the final README/roadmap should advertise landed code plus validation. Future guides and research docs can shape roadmap language, but they must not be phrased as shipped capability.

---

## Plugin Reliability

### 0.1) Claude plugin MCP attach-only bridge

**Status:** `landed 09e97a7`
- README release-note candidate: "Claude Code plugin MCP now attaches to the running Cortex daemon over HTTP instead of launching a second Cortex process."
- Contributor note candidate: plugin MCP route order is explicit URL → `CORTEX_APP_URL` → local `127.0.0.1:7437`; no local-spawn fallback exists in `run-mcp.cjs`.
- Operator note candidate: if Control Center is not running, plugin MCP returns `APP_INIT_REQUIRED` instead of starting a daemon.

**Validation:**
- `node --test plugins/cortex-plugin/scripts/run-mcp.contract.test.cjs` → 12/12 pass
- `node plugins/cortex-plugin/scripts/dry-run-matrix.cjs` → 8/8 pass
- `python tools/audit_spawn_paths.py --strict` → clean
- Live stdin smoke against local daemon returned JSON-RPC parse error via Node bridge with no new `cortex.exe` process

**Commits:** `09e97a7`

---

## Daemon Reliability

### 0.2) Control Center supervisor + daemon panic breadcrumbs

**Status:** `landed db491ca`
- Release-note candidate: "Control Center now supervises the app-managed daemon and automatically restarts it after unexpected exits, while explicit user stops remain honored."
- Operator note candidate: daemon panics write a local breadcrumb to `~/.cortex/panic.log`; handler panics return a JSON 500 without taking down the daemon.
- MCP reliability note candidate: heartbeat recovery now tolerates normal supervisor respawn windows before surfacing an MCP exit to plugins.

**Validation:**
- Dev daemon rebuilt and swapped into the app-managed runtime.
- Single and burst `cortex_store` calls kept the daemon healthy.
- Supervisor activates on the next Control Center restart.

**Commits:** `db491ca`

---

## Pre-release roadmap reconciliation (apply before first v0.6.0 commit)

### 0.0) Public roadmap rescope — 2026-04-23

**Status:** `pending` — draft in `public-roadmap-update.md`, awaiting approval.

**Changes:**
- Rewrite `Info/roadmap.md` v0.5.0 `shipped` section to include G1 TTL, C1 migrations + `cortex doctor`, C2 repair CLIs (`reindex` / `re-embed` / `recrystallize`), C6 semantic dedup, R9 recall feedback loop, R8 `all-MiniLM-L12-v2` profile option. Remove the Chrome extension bullet (untracked).
- Rewrite `Info/roadmap.md` v0.6.0 section to **Accessibility & Governance** headline. Themes: accessibility + settings, motion, budget governance (C3), retention classes (C9), dynamic context ranking (G5), foundation carryovers (G2, C5, R2, H3).
- Rewrite `Info/roadmap.md` v0.7.0 section to absorb query expansion / HyDE while keeping multi-tenant hardening. Reranking is now a v0.6.0 production candidate behind gates.
- Update `README.md:195` from `"Reranking and query expansion planned for v0.6.0+"` → `"Reranking production-ships in v0.6.0 Phase 2 behind benchmark gates; query expansion (HyDE) targeted for v0.7.0"`.

**Validation:**
- Grep `README.md` and `Info/roadmap.md` for any remaining v0.6.0 "Foundation Hardening" language.
- Confirm `v060/v050-shipped-vs-slipped.md` table matches public v0.5.0 section post-rewrite.

**Commits:** *(pending)*

---

## Accessibility + Settings + Motion (Tier 1 headline)

### 1.1) Settings panel first-class navigation entry

**Status:** `pending`
- Add `Settings` to primary navigation (sidebar or top-bar, consistent with existing nav).
- Land four sub-sections: `Accessibility`, `Appearance & Motion`, `Connection`, `Keyboard & Navigation`.
- README copy: "Cortex Control Center now has a first-class Settings surface with accessibility-first defaults, motion controls, and per-agent budget visibility."
- Supporting screenshots: capture at `1920×1080` for each of the 4 sections, drop into `C:\Users\aditya\Desktop\Claude\Cortex GitHub Pictures\v060-settings\`.

**Validation:**
- `npm --prefix desktop/cortex-control-center test`
- Manual walkthrough: open Settings → each sub-section → verify state persists after app restart
- axe-core audit on Settings panel main flow

**Commits:** *(pending)*

---

### 1.2) Accessibility engine — WCAG 2.2 AA baseline

**Status:** `pending`
- Add a short reliability/access section to README:
  - "Cortex Control Center now builds to WCAG 2.2 AA, validates against 2.1 AA expectations, and respects OS reduced-motion and contrast preferences."
- Specifically call out:
  - Keyboard-only operation across the entire app
  - Screen-reader verified on NVDA+Firefox, VoiceOver+Safari, Narrator+Edge
  - `prefers-reduced-motion` honored at runtime (not just a config flag)
  - Visible, durable focus rings on all interactive elements
  - Focus trapping + focus return on dialogs

**Validation:**
- Automated: `axe-core` integration run in desktop build — zero critical/serious violations on main flows
- Automated: keyboard-only smoke in `verify:lifecycle:dev`
- Manual: NVDA+Firefox full walkthrough checklist (archived to `docs/internal/v060/a11y-walkthroughs/nvda-firefox-<DATE>.md`)
- Manual: VoiceOver+Safari walkthrough
- Manual: Narrator+Edge walkthrough
- Manual: zoom + reflow check on `375×812` and desktop widths

**Commits:** *(pending)*

---

### 1.3) Motion system unification

**Status:** `pending`
- Add a short UX note to README:
  - "Sidebar and tab transitions now share one restrained motion language with reduced-motion support. No decorative animation that carries no state meaning."
- Technical callouts for contributors:
  - Central timing tokens + easing language in `desktop/cortex-control-center/src/design/motion.js`
  - One canonical sidebar collapsed width (remove duplicated collapse rules)
  - Panel/tab transitions preserve spatial continuity

**Validation:**
- Visual regression: capture before/after clips of sidebar collapse + tab change
- `prefers-reduced-motion: reduce` automated browser smoke (JSDOM or headless Chromium)
- Performance: 60fps sustained during transitions on reference hardware

**Commits:** *(pending)*

---

## Foundation Carryovers (Tier 1 ride-along)

### 2.1) `cortex admin rollback --session-id` CLI (G2)

**Status:** `pending`
- Add a short operator note:
  - "When an agent run goes sideways, `cortex admin rollback --session-id <id>` soft-deletes every store/decision tied to that session's `trace_id`. Dry-run by default. Emits a `session_rolled_back` event for audit."
- Flag: `--commit` to actually apply; `--json` for machine-readable output.

**Validation:**
- `cargo test --test admin_rollback`
- Integration: seed session → rollback → assert row counts + event emission
- Idempotency: rollback twice on same session → second pass is a no-op

**Commits:** *(pending)*

---

### 2.2) Boot prompt audit trail (C5)

**Status:** `pending`
- Add a short transparency note:
  - "Every `/boot` and `cortex_boot` call now writes one row to `boot_audits` capturing which sources contributed, how they ranked, and how tokens were allocated. Audits auto-prune after 30 days. Query via `GET /boot/audit?session_id=X` or `cortex_boot_audit` MCP tool."
- Prereq for understanding R2 (score-adaptive truncation) behavior in the wild.

**Validation:**
- Migration 008 applies cleanly on v0.5.0 databases
- Storage overhead measurement: < 1MB per 1000 boots on realistic data
- MCP tool test in `tests/mcp_rpc_headers.rs` style
- OpenAPI spec includes new endpoint

**Commits:** *(pending)*

---

### 2.3) Score-adaptive truncation for boot (R2)

**Status:** `landed a547a07`
- Add a short recall-quality note:
  - "Boot prompt assembly now allocates token budget proportionally to recall confidence scores, with per-source floor and ceiling caps. Flat-signal queries fall back to the v0.5.0 fixed allocation."
- Expose tuning knobs: `CORTEX_BOOT_MIN_SOURCE_TOKENS` (default 40), `CORTEX_BOOT_MAX_SOURCE_TOKENS` (default 600).
- Contributor note: implementation lives in `daemon-rs/src/compiler.rs`, not `prompt_inject.rs`; the latter only fetches `/boot`.

**Validation:**
- `rtk cargo check --manifest-path daemon-rs/Cargo.toml --tests` with isolated `CARGO_TARGET_DIR=target-codex-r2`
- `rtk cargo test --manifest-path daemon-rs/Cargo.toml score_adaptive -- --nocapture` -> 2 passed
- `rtk cargo test --manifest-path daemon-rs/Cargo.toml flat_score_fallback_matches_legacy_greedy_packing -- --nocapture` -> 1 passed
- `rtk cargo clippy --manifest-path daemon-rs/Cargo.toml --tests -- -D warnings` -> clean
- Full daemon suite: `rtk cargo test --manifest-path daemon-rs/Cargo.toml -- --nocapture` -> 469 passed
- Public release claim still needs: p50 boot latency and v0.5.0 GT-precision benchmark

**Commits:** `a547a07`

---

### 2.4) `DEFAULT_CORTEX_PORT` consolidation (H3)

**Status:** `pending`
- Internal-only change, no user-facing README update needed. But add a contributor note to README:
  - "Daemon production code now reads `DEFAULT_CORTEX_PORT` from `daemon-rs/src/lib.rs` instead of scattered `7437` literals. Test fixtures and doc examples keep literals deliberately — they pin specific values for copy-paste."

**Validation:**
- `grep -rn "7437" daemon-rs/src/ desktop/cortex-control-center/src/` shows only literals in test files and annotated CSP strings
- `cortex start --port 9999` smoke: full end-to-end read/write/boot round-trip on non-default port
- Clippy green

**Commits:** *(pending)*

---

## Governance + Economics (Tier 2)

### 3.1) Retention policy classes (C9)

**Status:** `landed bd85025`
- Add a data-lifecycle section to README:
  - "Every memory and decision now has a retention class: `durable` (architecture decisions, conventions — no expiry), `operational` (task context — 90 days), `audit` (security events, rollbacks — 365 days), or `ephemeral` (transient — 14 days). Classes apply at store time via caller hint, type heuristic, or text pattern."
- Migration 016 backfills/defaults existing rows to `operational` (safest middle).
- TTL enforcement reuses the v0.5.0 G1 cleanup loop.
- MCP `cortex_store` and OpenAPI expose `retention_class`; export/import preserve it for memories and decisions.

**Validation:**
- `rtk cargo check --manifest-path daemon-rs/Cargo.toml --tests` with isolated `CARGO_TARGET_DIR=target-codex-c9`
- `rtk cargo test --manifest-path daemon-rs/Cargo.toml store_decision_ -- --nocapture` -> 5 passed
- `rtk cargo test --manifest-path daemon-rs/Cargo.toml import_payload_normalizes_types_and_preserves_temporal_fields -- --nocapture` -> 1 passed
- `rtk cargo test --manifest-path daemon-rs/Cargo.toml test_run_pending_migrations_applies_all_once -- --nocapture` -> 1 passed
- `rtk cargo clippy --manifest-path daemon-rs/Cargo.toml --tests -- -D warnings` -> clean
- Full daemon suite: `rtk cargo test --manifest-path daemon-rs/Cargo.toml -- --nocapture` -> 465 passed

**Commits:** `bd85025`

---

### 3.2) Budget governance — per-endpoint limits (C3)

**Status:** `backend landed b41f7be; release docs/full-suite evidence 1231d58; Settings UI pending`
- Add a team-admin section to README:
  - "Per-endpoint local budgets via `~/.cortex/budgets.toml` cap how often agents can call `/store`, recall-family routes, `/boot`, and MCP RPC. Missing config remains unlimited; denials return stable `429` JSON or JSON-RPC error data with retry hints."
- Include a minimal TOML example:
  ```toml
  [endpoints.recall]
  limit = 300
  window_seconds = 60
  ```
- Operator/admin note:
  - "`/health.budgets` and `cortex admin budgets status --json` expose whether budgets are loaded, enabled, invalid, and which endpoints are configured. `cortex admin budgets validate --path <file> --json` validates draft files before use."
- UI note:
  - Control Center can now render a read-only Budgets section from `/health.budgets`; writing `budgets.toml` from Settings remains a U1 follow-up.
- Release inclusion guidance:
  - Include budget governance in the v0.6.0 README if the release ships after C3; this is operator-facing behavior, not just internal plumbing.
  - Keep public copy precise: local per-endpoint call budgets only. Do not imply centralized billing controls, multi-tenant quotas, or Control Center write/edit support until U1 lands.

**Validation:**
- `rtk cargo test --manifest-path daemon-rs/Cargo.toml budget` -> 36 passed, including parser, limiter, store/recall/boot HTTP denial, health reachability, and MCP JSON-RPC denial tests.
- `rtk cargo check --manifest-path daemon-rs/Cargo.toml --tests` -> clean.
- `git diff --check` -> clean.
- `rtk cargo clippy --manifest-path daemon-rs/Cargo.toml --all-targets -- -D warnings` -> clean.
- `rtk cargo test --manifest-path daemon-rs/Cargo.toml` -> 513 passed.
- Remaining before final public release polish: U1 Settings write/edit path and optional load test with sustained recall traffic.

**Commits:** `b41f7be`, `efe38a2`, `1231d58`

---

### 3.3) Dynamic context ranking for injectors (G5)

**Status:** `pending`
- Add a recall-quality note:
  - "Boot prompt injectors now rank candidates by `class × recency × relevance × activity` and inject top-N (default 5). Durable-but-stale memories no longer crowd out operational-and-active work."
- Audit trail (C5) captures component scores for every ranked source.

**Validation:**
- Unit tests per ranking component
- Regression: durable decision with 0 recent activity ranks below operational touched 10min ago
- Boot latency ±5%
- Audit output includes rank components

**Commits:** *(pending)*

---

### 3.4) Adapter conformance spec + shared contract tests (C4)

**Status:** `pending`
- Add a developer-facing note to README:
  - "`specs/cortex-adapter-contract.yaml` is now the canonical behavior spec for MCP, HTTP, and the Python/TS SDKs. Every adapter passes the same scenario battery. Drift between transports is a CI failure."
- Call out to contributors: new CI job `adapter-conformance-matrix`.

**Validation:**
- ≥ 10 scenarios covered
- CI green across MCP, HTTP, Python SDK, TS SDK
- First violation produces a clean diff in CI output
- Spec version tagged with daemon semver (drift gate)

**Commits:** *(pending)*

---

## Retrieval research bridge (Tier 3 measurement-only)

### 4.1) Reranker measurement harness (R1-M)

**Status:** `SUPERSEDED 2026-04-24 by RQ2 production candidate; keep this note for history only.`
- Add a roadmap-clarifying note to README (near line 195):
  - Historical draft, do not use: "v0.6.0 ships an offline reranking measurement harness (`cortex eval rerank`). Production reranking is gated on harness results and ships in v0.7.0 if the signal is strong enough."
- Superseded non-goal: production `/recall` rerank routing is now implemented locally behind off/shadow/primary gates; release still depends on benchmark proof.
- Adapters covered: `noop_baseline`, `cross_encoder_minilm`, `cross_encoder_tinybert`, `colbert_v2`.

**Validation:**
- Harness runs green on Win/Mac/Linux CI matrix
- At least one full measurement pass committed to `benchmarking/results/`
- Report markdown committed
- Go/no-go writeup in `docs/internal/v060/rerank-findings.md`
- If no-go: v0.7.0 roadmap line dropped; `README.md:195` updated

**Commits:** *(pending)*

---

## Release-cut items (apply at release)

### 5.1) CHANGELOG.md v0.6.0 section

**Status:** `pending` (draft builds in `comprehensive-changelog.md` as work lands)

- Sections: `Added`, `Changed`, `Fixed`, `Performance`, `Security`, `Desktop`, `Documentation`, `CI`
- Pull from `comprehensive-changelog.md` — do not duplicate detail; include summary + commit refs + link to comprehensive
- Reference URL: `[0.6.0]: https://github.com/AdityaVG13/cortex/compare/v0.5.0...v0.6.0`

---

### 5.2) OpenAPI spec version bump to 0.6.0

**Status:** `pending`
- `specs/cortex-openapi.yaml`: version → `0.6.0`
- New endpoints documented: `/boot/audit`, any new admin endpoints
- Drift gate test updated

---

### 5.3) Plugin + marketplace version bump

**Status:** `pending`
- `plugins/cortex-plugin/.claude-plugin/plugin.json`: version → `0.6.0`
- `.claude-plugin/marketplace.json`: version → `0.6.0`
- `plugins/cortex-plugin/scripts/prepare-runtime.cjs`: default → `'0.6.0'`
- `daemon-rs/Cargo.toml` `cortex-daemon` version → `0.6.0`
- Lockstep check per `plugins/cortex-plugin/ROUTING.md:60-65`

---

### 5.4) Screenshots + asset refresh

**Status:** `pending`
- Re-capture `overview`, `analytics`, `agents`, `work`, `memory`, `brain`, `about`, `settings` (new) at `1920×1080`
- Drop into `C:\Users\aditya\Desktop\Claude\Cortex GitHub Pictures\v060\`
- Update `README.md` asset paths if layout changed

---

### 5.5) Graphify refresh

**Status:** `pending`
- `graphify update .` full code graph refresh
- Capture new node/edge/community counts in `comprehensive-changelog.md` release entry

---

### 5.6) Release verification

**Status:** `pending`
- All Gate and Verification Matrix items in `unified-status-plan.md` §7 pass
- `cargo audit` + `npm audit` clean
- Hardcoded developer-path scan clean
- `.env` tracking policy check clean
- Benchmark visibility policy check clean
- Release bundle build: MSI + NSIS + .dmg + .AppImage + .deb
- Updater signing with canonical key (not ad-hoc)
- Release smoke test: store/recall/boot round-trip against release binary on each platform

---

## Foundation carryovers (2026-04-24 — H3 + G2 + C5 landed)

### H3 landed — `c734886`

Internal consolidation; no public README copy needed.

### C5 landed — `83509b4`

**Public impact:** new HTTP endpoint + env var.

**Draft release-notes copy:**

> `GET /boot/audit` — returns the most recent boot events Cortex served
> to each agent (agent, profile, budget, token estimate + savings, capsule
> count, latency, timestamp). Supports `?agent=<name>` + `?limit=<N>` (cap
> 500). Auth-guarded like `/boot`. Rows auto-prune after 90 days; override
> via `CORTEX_BOOT_AUDIT_RETENTION_DAYS=<days>` (0 disables prune).

**Limitations:**
- MCP tool wrapper deferred (HTTP endpoint covers the surface).
- OpenAPI spec row still pending — tracked for release-gate sweep.
- `boot_audits` is diagnostic metadata only; memory retention (C9)
  is separate and handles the actual "which memories survive" logic.

### G2 landed — `f1d23ae` — `cortex admin rollback` CLI

**Public impact:** new CLI command. Worth a short release-notes line.

**Draft release-notes copy:**

> `cortex admin rollback --session-id <id> [--apply] [--json]` — undo
> a session's memory writes without touching the DB. Memories + decisions
> the session's agent stored get soft-deleted (status flipped to
> `rolled_back`) and are automatically excluded from recall. Dry-run
> by default; pass `--apply` to persist. Idempotent. A
> `session.rolled_back` event is written to the `events` table for
> SSE subscribers and offline audit.

**Limitations to mention:**
- Looks up sessions via the `sessions` table — only the agent's
  *current* session_id is rollback-able. Rotated / expired sessions
  require a follow-up schema change (session_id column on memories +
  decisions) tracked for v0.7.0.
- Scope is memories + decisions only. Events are audit-preserving
  history and remain.

**Status:** pending — stage into CHANGELOG.md v0.6.0 `Added` at release
cut; no immediate README edit needed (the command surfaces via
`cortex --help`).

---

## Plugin/App parity (2026-04-24 — shipped, internal-only)

### 3.1) Plugin routing policy + lockstep guard

**Status:** `landed` — commits `c2ba28d`, `9632b6a`, `0c6bb62`, `d50744c` (master).

**Public README impact:** **none recommended.** Plugin parity is a developer-facing invariant; the UX is identical for end users (plugin-only users still work without the app; app users now explicitly route through their running app daemon under `CORTEX_DEV_PREFER_APP=1`).

**Contributor-facing doc:** `Info/plugin-lockstep.md` already ships — links from CONTRIBUTING.md if/when that file gets a v0.6.0 refresh.

**If a release-notes line is needed:** "v0.6.0 hardens plugin/app daemon co-existence: the plugin now refuses to spawn a local daemon when `CORTEX_DEV_PREFER_APP=1` is set, and a version-lockstep guard blocks any release where `daemon-rs/Cargo.toml` and `plugin.json` disagree."

### 3.2) Triangle judge + CAS-100

**Status:** `landed` — commits `f625614`, `323c5cf` (master).

**Public README impact:** already staged under §§4.2/4.3 above. Swap "pending" to `landed` references once Phase 0 purity adapter lands.

---

## Recall Purity + Benchmarks (new sections added 2026-04-24)

### 4.1) Recall purity pledge — public messaging

**Status:** **infra landed f4488c3; triad run deferred** — adapter + gates + CODEOWNERS + CI job shipped 2026-04-24. First pure-mode JSON still needs API credits.

**README additions:**
- Replace any existing "benchmark accuracy" line with: "Cortex ships three measurement adapters: `cortex-http-pure` (canonical, zero helpers — the only one used for public claims), `cortex-http-base` (deprecated partial helpers), and `cortex-http` (tuned). All v0.6.0+ recall-quality claims are measured via `cortex-http-pure` with 5 CI purity gates green."
- Add a short "Measurement integrity" subsection linking to `benchmarking/README.md` and `docs/internal/v060/research/benchmark-purity-audit.md` (if shipped internally) or an external-facing purity doc.

**Validation:**
- CI job `purity-gates` passing on master
- CHANGELOG correction commit `b9a6458` already live on master

**Commits:** `b9a6458` (preemptive correction), Phase 0 kickoff pending

---

### 4.2) Triangle judge protocol

**Status:** `pending` — ships with Phase 0 execution.

**README copy (likely in "Evaluation" section):**
- "Cortex benchmarks use a cross-family triangle judge protocol: GPT-4o + Claude + local Qwen3-30B. Answerer LLM must differ from every judge family — this avoids 5-25% self-preference bias observed in single-judge protocols."
- Link to `benchmarking/judges/triangle.py`

**Validation:** First triangle-judge run on master with κ ≥ 0.75 committed in `benchmarking/results/`.

**Commits:** pending (depends on first live run)

---

### 4.3) CAS-100 adversarial suite

**Status:** `pending` — ships with Phase 0 execution.

**README copy:**
- "Cortex ships CAS-100, a 100-item hand-authored adversarial suite covering 15 failure categories (paraphrase, typo, temporal, negation, abstention, conflict, etc.). Wilson 95% CI ±7pp at N=100; the suite detects ≥12pp changes as real."
- Link to `benchmarking/adversarial/cas-100.spec.md`

**Validation:** First CAS-100 run result JSON in `benchmarking/results/`.

**Commits:** pending

---

## Recall improvement story (amended Tier 2-essential, 2026-04-24)

### 5.1) Embedding upgrade — v0.6.0 default

**Status:** `landed c971a5a; benchmark gates pending` — code path shipped, public copy waits for release cut.

**README copy:**
- "v0.6.0 promotes `bge-base-en-v1.5` (MIT, ~438 MB, MTEB 64.23) to the default embedding model, replacing MiniLM-L12 (MTEB ~59). MiniLM profiles remain selectable via `CORTEX_EMBEDDING_MODEL` for footprint-sensitive deployments."
- Optional opt-in line: "`CORTEX_EMBEDDING_MODEL=qwen3-embedding-0.6b` unlocks 1024-dim Qwen3 embeddings for quality-first users. Cortex downloads the q8 ONNX export from `onnx-community/Qwen3-Embedding-0.6B-ONNX` (~614 MB)."

**Validation:** Code validation landed in `c971a5a` (`cargo check --tests`, focused `embeddings` tests 13/13, clippy `-D warnings`, full daemon suite 473/473). Public quality copy still requires Pure-mode LongMemEval-S score improvement ≥ 3pp committed to `benchmarking/results/`, backfill throughput ≥500 emb/hr, and p50 recall regression check.

**Commits:** `c971a5a`

---

### 5.2) Cross-encoder reranker — v0.6.0 production

**Status:** `landed f6c1ebb; local gate CAUTION; LongMemEval pending` — **REVISED from measurement-only on 2026-04-24.**

**README copy:**
- Replace `README.md:195` "Reranking and query expansion planned for v0.6.0+" with, if benchmark gates pass: "v0.6.0 ships `ms-marco-MiniLM-L-6-v2` int8 (Apache 2.0, ~22 MB) as a gated cross-encoder reranker for `/recall`. Enable observation with `CORTEX_RERANK_MODE=shadow`; promote with `CORTEX_RERANK_MODE=primary` after local validation. Query expansion (HyDE, R7) is deferred to v0.7.0."
- If benchmark gates fail or latency is unacceptable, ship contributor-facing docs only: "Cortex includes an experimental shadow-mode cross-encoder reranker path for local evaluation; primary recall ranking remains disabled by default."
- Current recommended release copy after the 2026-05-05 local gate: "Cortex includes a gated cross-encoder reranker path for local evaluation. Use `CORTEX_RERANK_MODE=shadow` to observe rerank telemetry without changing recall order; promote to `primary` only after validating against your corpus. The path remains default-off while benchmark gates continue."

**Validation:** Code validation landed across `f07d61f` and `2707913`: focused rerank tests 6/6, real model load/inference smoke 1/1, `cargo check --tests` clean, clippy `--all-targets -- -D warnings` clean, full daemon suite 497/497 using isolated `CARGO_TARGET_DIR=daemon-rs/target-codex-rq2-gate`. Committed local gate artifact `benchmarking/results/rq2-rerank-20260505-031510/`: owned deterministic top-1 improved `0.0000 -> 0.6667`, top-3 stayed `1.0000`, primary p95 delta `+19.004ms`, shadow order matched off, clean manifest points at tested code commit `2707913`. Public quality copy still requires Pure-mode LongMemEval-S with rerank ≥ Phase 1 baseline + 2pp.

**Commits:** `f07d61f`, `2707913`, `f6c1ebb`

---

## Storage Hygiene

### 6.1) Compaction + PQ8 vector footprint reduction

**Status:** `partial landed fe78000, 84d20cc, 2fb1c20, 4c3b43c` — v0.6.0 storage hygiene acceleration. Keep remaining v0.6.1-only claims separate.

**README / release-note copy (short):**
- "v0.6.0 materially reduces local database growth: compaction now optimizes FTS5 shadow tables, prunes stale embedding and singleton co-occurrence rows, and stores embeddings / crystal centroids in compact PQ8 form."
- "On the live development database, the storage hygiene pass reduced Cortex state from 412 MB to 26 MB (~94%) while keeping recall, store, and MCP flows functional."
- Contributor note: PQ8 is read-compatible with legacy f32 blobs; compaction migrates old rows in bounded batches.

**Validation:**
- `fe78000`: live compaction recovered 371 MB (`390 MB -> 27 MB`) and pruned 9767 stale embeddings + 20642 singleton co-occurrence rows.
- `84d20cc`: focused FTS segment-pressure tests.
- `2fb1c20`: PQ8 embedding layout, drift, ordering, and migration tests; live recall smoke.
- `4c3b43c`: centroid migration regression and live drain to 9701 PQ8 / 0 legacy centroids.

**Commits:** `fe78000`, `84d20cc`, `2fb1c20`, `4c3b43c`

### 6.2) v0.6.1 storage hygiene still pending

**Status:** `pending`

Keep these as future-work claims unless they land before cut:
- `auto_vacuum=incremental` / periodic incremental vacuum
- Zstd text compression
- DB stats CLI + bloat smoke CI gate
- Text/table-level storage dashboards
- Residual centroid/member encoding, if schema support is added

**Validation:** future `bloat-smoke.sh` CI gate at ≤ 25 MB @ 1000 memories.

---

## Parking lot (captured, not committed)

Things that might be worth mentioning in README but haven't been decided yet:

- Whether to surface Chrome extension roadmap line now that it's untracked (probably "coming in a future release" only)
- Whether v0.6.0 introduces a breaking CLI change that warrants an upgrade note
- Whether to add a short "migrating from v0.5.0" section for team admins adopting retention classes + budgets
- Whether the shadow rerank telemetry (stretch goal on R1-S) is worth a README line or lives in contributor docs only — **RESOLVED 2026-04-24:** R1-S superseded; production rerank in Phase 2 lands with its own README line (§5.2)
- Whether Phase 3.0 ingest extraction gets a public README line — depends on whether it lands in v0.6.0 stretch or slips to v0.6.1; draft both versions
- Whether CAS-100 should be public — **YES** per research; it's the purity-pledge evidence
- Whether the `/answer` endpoint (Phase 6) deserves a README section — likely yes in v0.6.2 cut, as it turns Cortex into a callable QA service

Resolve at release draft time — don't over-commit now.

---

## Dropped / superseded

*(empty — tracks themes that got pulled; keep the history)*
