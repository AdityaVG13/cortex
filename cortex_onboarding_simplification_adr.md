# Cortex Onboarding Simplification ADR Execution Record

Source PRD: local working file `cortex_onboarding_simplification_goal.md` (not staged because it contains machine-local generation metadata).

## Governing Sentence

Cortex v0.6.x must make private local memory feel like one product switch, not a daemon operations project, while preserving one-daemon safety and honest benchmark claims.

## Source Map

| Surface | Current entry points | Onboarding risk | Decision |
|---|---|---|---|
| CLI | `daemon-rs/src/main.rs`, `daemon-rs/src/setup.rs`, `cortex setup`, `cortex doctor`, `cortex plugin ensure-daemon`, `cortex paths --json` | `setup` mutates config and can skip live verification when the daemon is stopped; no stable non-mutating JSON readiness contract. | Add `cortex status [--json]` as canonical readiness/next-action output; keep `setup` for configuration. |
| Daemon lifecycle | `daemon-rs/src/main.rs`, `daemon-rs/src/service.rs`, `daemon-rs/src/daemon_lifecycle.rs` | Simplification could accidentally start duplicate daemons or accept wrong runtime identity. | Reuse existing readiness and identity checks; do not add new spawn behavior. |
| Control Center | `desktop/cortex-control-center/src/App.jsx`, `src/daemon-startup.js`, `src/api-client.js` | UI has daemon controls but first-run readiness is not expressed as a single memory next action. | Add a derived readiness checklist from existing state; no lifecycle rewrite. |
| Plugin | `plugins/cortex-plugin/scripts/run-mcp.cjs`, `plugins/cortex-plugin/ROUTING.md`, `Info/connecting.md` | Plugin copy can still sound like machinery instead of connection/status/repair. | Preserve attach-only code; update docs and tests for app-required repair wording. |
| SDKs | `sdks/typescript`, `sdks/python` | SDK docs/packages describe daemon transport before remember/recall value. | Add quickstart docs that lead with store/recall snippets and explicit local-only setup. |
| Benchmarks and claims | `benchmarking/README.md`, `CHANGELOG.md`, `Info/roadmap.md` | `cortex-http-pure` and LongMemEval wording can look like runtime setup requirements or release claims. | Add benchmark-only and deferred-scored-validation wording; no quality-lift claim. |
| README | `README.md` | Top path exposed benchmark/daemon/port/token details before first success. | Apply the approved text-only README changes after Aditya approval; do not modify capsule/header image lines. |

## ADR Ledger

| ADR | Decision | Status | Consequence | Verification |
|---|---|---|---|---|
| ADR-001 | Canonical first-run path is CLI-led via `cortex status [--json]`, with desktop/docs/README mirroring it. | Accepted | One non-mutating command owns runtime state and repair action. | CLI unit tests, golden CLI tests, default local status probe, first-run smoke, README text diff. |
| ADR-002 | Startup safety is unchanged: status reports readiness and repair, but never spawns a daemon. | Accepted | No duplicate daemon convenience path is added. | Full daemon test suite, plugin contract test, temp-smoke daemon isolated on port 7461. |
| ADR-003 | Desktop first-run UI derives readiness from existing daemon/app state. | Accepted | Control Center shows one memory next action without duplicating daemon ownership logic. | Full Vitest suite, Vite build, in-app browser snapshot, Expect Playwright smoke. |
| ADR-004 | Public claim wording is conservative: benchmark adapters are benchmark-only and LongMemEval-S is deferred until funded. | Accepted | Onboarding no longer implies paid benchmark requirements. | Changed-docs scan and claim table in proof bundle. |

## Stop Gates

- No hosted service, telemetry, destructive migration, paid benchmark, push, tag, signing, or release publication.
- README edit and push were approved by Aditya after draft review; no release/tag/signing/publication beyond the requested push.
- No daemon/plugin change may weaken one-daemon or attach-only invariants.

## Work Units

| Work unit | Status | Evidence |
|---|---|---|
| WU-001 Source map and ADR | Complete | Source map and ADR ledger recorded here. |
| WU-003 CLI setup/status contract | Complete | `cortex status [--json]` implemented with `schemaVersion`, `status`, `runtime`, `nextAction`, `repair`, and `checks`; golden docs updated. |
| WU-005 Desktop first-run alignment | Complete | `buildFirstRunReadiness` plus Overview First Run card wired to existing lifecycle/setup/memory actions. |
| WU-006 Plugin docs/safety alignment | Complete | Plugin code remains attach-only; routing and connection docs now lead with status/repair and `APP_INIT_REQUIRED`. |
| WU-007 SDK quickstarts | Complete | TypeScript and Python quickstarts lead with readiness, store, recall, and local token guidance. |
| WU-008 Claim hygiene | Complete | Benchmark docs separate normal runtime from benchmark adapters and defer LongMemEval-S scored claims. |
| WU-002 README quickstart | Complete | `README.md` now has the approved text-only Quick Start, claim-hygiene, attach-only plugin, and product-smoke wording; capsule/header image lines were not changed. `cortex_onboarding_simplification_readme_draft.md` records the approval. |
| WU-009 Proof and reconciliation | Complete | `cortex_onboarding_simplification_proof_bundle.md` records verification, scans, score, and approved README application. |
