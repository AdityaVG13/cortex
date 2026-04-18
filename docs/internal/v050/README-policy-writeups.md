# README Policy Writeups (v0.5.0)

Last updated: 2026-04-18
Purpose: copy-ready rationale and language for core operating-policy sections in the public README refresh.

## 1) Solo Mode: Admin by Default

### Why this is the right default
- In solo mode, there is one operator and no multi-user blast radius.
- Forcing separate role setup in solo adds friction without adding real security value.
- Admin-by-default keeps setup fast and predictable while still requiring valid auth.

### User impact
- Easier onboarding.
- Fewer early "permission denied" support failures.
- Better out-of-box experience.

### Product impact
- Better product clarity (solo semantics match user expectations).
- Neutral-to-positive security posture (auth still required).
- Lower support burden.

### README copy draft
In solo mode, Cortex treats the authenticated user as the admin by default. This keeps setup simple and avoids unnecessary role-management overhead for single-operator workflows.

## 2) One Daemon Ever (App-Controlled in App Mode)

### Why this is the right default
- Multiple daemons create lock contention, confusing state, and inconsistent memory visibility.
- A single canonical daemon guarantees one source of truth for storage, recall, and lifecycle state.
- In app-managed flows, startup authority belongs to the app so AI clients attach instead of spawning competing instances.

### User impact
- Easier mental model: one daemon, one memory state.
- Fewer startup and reconnect edge cases.
- Better reliability during long-running sessions.

### Product impact
- Strongly better reliability/debuggability.
- Cleaner lifecycle guarantees across app/plugin/SDK surfaces.
- Stronger benchmark integrity (no accidental second-daemon contamination).

### README copy draft
Cortex enforces a single-daemon model. In app-managed mode, the desktop app owns daemon lifecycle, and AI clients attach to that running instance instead of spawning their own daemon.

## 3) Explicit Token Required for Remote Targets

### Why this is the right default
- Auto-loading local credentials for remote URLs can leak trust assumptions and create accidental cross-boundary auth.
- Explicit token requirements make remote connections intentional and auditable.
- Local-first remains frictionless while remote mode remains safe by design.

### User impact
- Slightly more setup for advanced remote flows.
- Much lower risk of accidental misconfiguration.
- Clearer error handling and operator intent.

### Product impact
- Better security posture.
- Better correctness for team/remote deployments.
- Better compliance story for enterprise adoption.

### README copy draft
For remote Cortex targets, clients must provide an explicit API token. Cortex does not auto-apply local credentials to remote URLs. This preserves local-first convenience while keeping remote auth explicit and safe.

## 4) Combined Positioning Snippet

### README copy draft
Cortex is local-first by default: fast startup, private storage, and minimal setup. In solo mode, you are admin by default. In app-managed workflows, Cortex enforces one daemon and requires AI clients to attach to it. For remote deployments, authentication is always explicit with a provided API token.

## 5) Optional FAQ Inserts

### Q: Does one-daemon policy make setup harder?
A: No. It simplifies setup by removing race conditions and duplicate-instance confusion. The app owns lifecycle in app mode, and clients just attach.

### Q: Why require explicit token for remote URLs?
A: To prevent accidental credential crossover and keep remote access intentional, auditable, and secure.

### Q: Is solo mode less secure because admin is default?
A: Solo mode still requires authentication. Admin-by-default in solo removes unnecessary role friction when there is only one operator.

## 6) App-Managed Startup Reliability (Internal Draft for Next Public README Refresh)

### Why this is the right default
- App-first users care about perceived startup speed and reliability more than raw daemon background throughput.
- Running heavy maintenance immediately at startup can make `/health` and dashboard calls appear flaky even when the daemon is technically online.
- Lock-aware dev rebuild behavior is required on Windows to keep `tauri dev` deterministic.

### User impact
- Fewer first-launch timeout cascades in Control Center.
- Faster transition from "daemon starting" to usable dashboard state.
- Reduced "build failed because cortex.exe is in use" loops during local dev.

### Product impact
- Stronger one-daemon app-managed lifecycle behavior with explicit loopback ownership.
- Lower startup contention by deferring/staggering heavy startup maintenance.
- Better diagnostics: timeout-classified failures route through daemon-recovery UX instead of looking like random endpoint breakage.

### README copy draft (non-public until next release)
Control Center now prioritizes startup responsiveness in app-managed mode. The app-managed daemon path binds to loopback (`127.0.0.1`), dashboard loading is staged (core health first, secondary panels after readiness), and IPC timeout budgets are tuned for startup warmup. Heavy daemon maintenance tasks are deferred/staggered so first-load APIs remain responsive. On Windows dev builds, stale daemon binaries are rebuilt automatically and locked-binary failures are recovered by stopping the locked dev daemon and retrying once.

## 7) Event-Volume Analytics Scalability (Internal Draft for Next Public README Refresh)

### Why this is the right default
- Large operator histories can accumulate hundreds of thousands of `events` rows quickly.
- If analytics endpoints hold a shared DB read lock too long, startup-critical panels can time out even when daemon health is otherwise fine.
- SQL-side aggregation plus short-lived response caching prevents lock starvation while preserving observability.

### User impact
- Startup becomes usable sooner on large histories.
- Fewer timeout storms across sessions/locks/tasks/feed during cold start.
- Analytics remain available without dominating every refresh cycle.

### Product impact
- Better behavior under power-user load without changing public API shape.
- Lower contention on single-read-connection architecture.
- Clearer root-cause diagnostics for event growth (`decision_stored` concentration from heavy automated ingestion).

### README copy draft (non-public until next release)
Cortex now treats savings analytics as a heavy lane. The `/savings` path is SQL-aggregated and cached briefly to reduce lock contention, and Control Center keeps savings refresh out of startup-critical dashboard fanout. This keeps core operational panels responsive even with very large event histories.

## 8) Benchmark Compaction Caps + Fail-Closed Fairness (Internal Draft for Next Public README Refresh)

### Why this is the right default
- Large benchmark corpora can bloat stored payloads and token budgets if ingestion is unconstrained.
- Shortcut flags that bypass quality/token gates break fair-run comparability and produce misleading benchmark outcomes.
- Explicit ingestion caps and fail-closed preflight checks keep benchmark evidence credible and reproducible.

### User impact
- Lower benchmark-side event/payload growth in app-attached evaluation runs.
- More predictable token usage across single-run and matrix workflows.
- Clear early failures when a run configuration would violate fair-run policy.

### Product impact
- Stronger benchmark integrity (quality/token gates cannot be silently bypassed).
- Better operational safety for long benchmark batches on real operator environments.
- Cleaner comparison between matrix runs because retrieval policy/cap posture is explicit and consistent.

### README copy draft (non-public until next release)
Benchmark mode now runs with bounded ingestion and fail-closed fairness checks. Cortex applies storage compaction caps for benchmark writes (`CORTEX_BENCHMARK_STORE_MAX_CHARS` plus tighter fact/context matrix ceilings), and preflight rejects gate-bypass shortcuts such as `no_enforce_gate` and `allow_missing_recall_metrics` in both single and matrix runs.

## 9) Startup Coalescing + Event-Pressure Control (Internal Draft for Next Public README Refresh)

### Why this is the right default
- Large operator datasets produce bursty startup demand across multiple protected panels at once.
- Overlapping refresh triggers (timer + SSE + retry) can fan out duplicate protected requests and amplify timeout storms.
- Event growth concentrated in a few high-volume families (`decision_stored`) must be bounded to preserve first-load responsiveness over time.

### User impact
- Faster and more stable first-load behavior on large histories.
- Fewer repeated "IPC request timed out after 8000ms" failures across multiple dashboard panels.
- Better long-run responsiveness because high-volume event classes are compacted aggressively before they dominate startup queries.

### Product impact
- Lower read/write contention by moving hot GETs to read-only connections and removing write cleanup from read paths.
- Better scalability from startup-focused indexes and event-pressure compaction policies.
- More deterministic startup behavior because refresh fanout is coalesced through a single-flight scheduler.

### README copy draft (non-public until next release)
Control Center now coalesces dashboard refreshes into a single in-flight cycle, so real-time events, retries, and timer refreshes do not overlap into duplicate protected API bursts. Cortex also enforces event-pressure compaction caps for high-volume event families and uses startup-focused query/index paths to keep first-load panels responsive on large histories.
