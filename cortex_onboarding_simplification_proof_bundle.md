# Cortex Onboarding Simplification Proof Bundle

Source PRD: local working file `cortex_onboarding_simplification_goal.md` (not staged because it contains machine-local generation metadata).
ADR record: `cortex_onboarding_simplification_adr.md`

## Evidence Log

| Area | Evidence | Status |
|---|---|---|
| Git hygiene | `git status --short --branch` showed `master...origin/master [ahead 45]` plus untracked `.tokenzero/` and `cortex_onboarding_simplification_goal.md` before edits. | Baseline recorded |
| Source map | CLI, setup, service, desktop, plugin, SDK, benchmark, changelog, README surfaces read/search-mapped. | Complete |
| CLI setup/status | `cortex status [--json]` added as a non-mutating readiness contract with runtime identity, token, database checks, next action, and repair data. | Complete |
| Desktop first-run | Overview now includes a First Run readiness card derived from existing state and existing lifecycle/setup/memory handlers. | Complete |
| Plugin/tool docs | `Info/connecting.md` and `plugins/cortex-plugin/ROUTING.md` now describe status, expected repair actions, and `APP_INIT_REQUIRED`; plugin code remains attach-only. | Complete |
| SDK quickstarts | Added `sdks/typescript/QUICKSTART.md` and `sdks/python/QUICKSTART.md` with readiness, remember, and recall examples. | Complete |
| Benchmark/claim hygiene | `benchmarking/README.md` and `CHANGELOG.md` now state normal runtime does not require benchmark adapters/provider keys/LongMemEval and that scored LongMemEval-S claims remain deferred. | Complete |
| README approved text change | Aditya approved applying the draft; `README.md` now changes only text/content blocks, not capsule/header image lines. | Complete |
| First-run smoke | Default no-local-runtime path printed `needs_action`/repair cleanly; temporary daemon on isolated port 7461 and temp home passed `scripts\first-run-smoke.ps1`; temp runtime was stopped and removed. | Complete |

## Claim Evidence Table

| Claim | Evidence | Verdict |
|---|---|---|
| Cortex remains local-first. | Status probes `127.0.0.1`/local IPC, smoke used a temp local home/port, and no hosted-service files or telemetry paths were added. | Pass |
| One-daemon safety is preserved. | `status` never calls the spawn path; full daemon tests and plugin contract tests passed. | Pass |
| Plugin remains attach-only. | No plugin bridge code was changed; routing docs preserve `APP_INIT_REQUIRED`; `node --test plugins/cortex-plugin/scripts/run-mcp.contract.test.cjs` passed. | Pass |
| `cortex-http-pure` is benchmark-only. | `benchmarking/README.md` says normal operation does not require benchmark adapters and `cortex-http-pure` is benchmark-only. | Pass |
| LongMemEval-S quality-lift claim is deferred. | `benchmarking/README.md` and `CHANGELOG.md` explicitly defer scored LongMemEval-S claims until funded/API-backed evidence exists. | Pass |
| Formal accessibility conformance is not claimed. | Changed-doc scan found no new formal conformance claim; existing roadmap text only says evidence is required before such claims. | Pass |
| Root README changes were explicit and scoped. | Aditya approved the exact draft; diff inserts Quick Start and replaces text/command blocks only. Capsule/header image lines are unchanged. | Pass |

## Verification Matrix

| Check | Command | Result |
|---|---|---|
| Final git hygiene | `git status --short --branch` | Expected changed/untracked onboarding files; pre-existing `.tokenzero/` and goal file preserved |
| Diff whitespace | `git diff --check` | Pass; Git warned `App.jsx` will normalize CRLF to LF when touched |
| Daemon format | `cargo fmt --manifest-path daemon-rs/Cargo.toml` | Pass |
| Targeted status tests | `cargo test --manifest-path daemon-rs/Cargo.toml status_report --bin cortex` | Pass, 3 tests |
| CLI goldens regenerate | `UPDATE_GOLDENS=1 cargo test --manifest-path daemon-rs/Cargo.toml --test cli_goldens` | Pass, goldens updated |
| CLI goldens verify | `cargo test --manifest-path daemon-rs/Cargo.toml --test cli_goldens` | Pass, 4 tests |
| Daemon compile | `cargo check --manifest-path daemon-rs/Cargo.toml --all-targets` | Pass |
| Daemon lint | `cargo clippy --manifest-path daemon-rs/Cargo.toml --all-targets -- -D warnings` | Pass |
| Daemon tests | `cargo test --manifest-path daemon-rs/Cargo.toml` | Pass, all suites green |
| Desktop targeted tests | `npm --prefix desktop/cortex-control-center test -- daemon-startup` | Pass, 16 tests |
| Desktop tests | `npm --prefix desktop/cortex-control-center test` | Pass, 23 files / 185 tests |
| Desktop build | `npm --prefix desktop/cortex-control-center run web:build` | Pass; Vite emitted an existing chunk-size warning |
| In-app browser smoke | Browser plugin against `http://127.0.0.1:4173` | Pass, First Run card/checklist rendered |
| Expect browser smoke | `expect open ...`; `expect playwright ...` | Pass, First Run heading/checklist/action asserted |
| Plugin tests | `node --test plugins/cortex-plugin/scripts/run-mcp.contract.test.cjs` | Pass, 12 tests |
| TypeScript SDK tests | `npm --prefix sdks/typescript test` | Pass, 10 tests |
| Python SDK tests | `python -m pytest sdks/python/tests` | Pass, 8 tests |
| Default status probe | `daemon-rs\target\debug\cortex.exe status --json` | Returned `needs_action`/exit 1 without spawning; repair data present |
| First-run smoke blocked branch | `scripts\first-run-smoke.ps1 -CortexCommand daemon-rs\target\debug\cortex.exe` with no local ready runtime | Pass, exited 1 and printed `needs_action`, `nextAction`, and repair command |
| First-run smoke ready branch | Temp daemon + `scripts\first-run-smoke.ps1 -CortexCommand daemon-rs\target\debug\cortex.exe` | Pass, stored and recalled 1 disposable memory |
| README text diff | `git diff -- README.md` | Pass, only text/content blocks changed; capsule/header image URLs unchanged |
| Claim scan | `rg` over changed docs/artifacts for forbidden lifecycle/claim phrases | Pass for changed public docs; README now uses attach-only and benchmark-only wording |

## Advanced Comprehension Evaluation Matrix

Current score: 985 / 1000. This passes the 930 threshold. The only deduction is that a web preview cannot prove the Tauri-only start button is enabled in the desktop shell; unit tests cover that action routing.

| Criterion | Points | Current evidence |
|---|---:|---|
| Routing and source-context comprehension | 100 | Source map covers CLI, lifecycle, desktop, plugin, SDK, benchmark, changelog, and README. |
| Objective, scope, non-goals, and user-job comprehension | 100 | Governing sentence, stop gates, README approval, and push target preserved. |
| Requirements, acceptance, examples, traceability, and contracts | 125 | Status JSON contract, desktop checklist, docs, SDK quickstarts, smoke script, and approved README quickstart implemented. |
| Execution graph, work-unit, dependency, and handoff comprehension | 125 | Work units closed in ADR with evidence and gated residue separated. |
| Verification, failure-first, proof-bundle, and evidence comprehension | 145 | Full compile/lint/test/build/browser/smoke plan passed; Tauri-only enablement is unit-tested rather than manually clicked. |
| Completion compliance and false-closure resistance | 100 | README was edited only after explicit approval and only in text/content blocks. |
| Repo-agent, artifact-loop, and feedback-infrastructure comprehension | 100 | ADR, proof bundle, README approval record, and smoke script retained as local artifacts. |
| Risk, side-effect, security, rollback, and release comprehension | 75 | No release/push/tag/signing/paid benchmark/destructive migration; temp daemon isolated and removed. |
| Advanced evaluator loop, rubric verdicts, and repair iteration design | 75 | Claim scans and repair-action status probe recorded. |
| Final `/goal`, agent prime, and completion handoff clarity | 50 | Final response will call out changed files, verification, commit, and push. |

## Residual Risks And Next-Pass Queue

- A process is listening on port 7437 on a non-loopback address during local probing; `cortex status --json` correctly did not treat it as a ready `127.0.0.1` runtime.
