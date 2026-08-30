# Testing philosophy

Cortex tests exist for **release confidence**, not so every contributor (human or AI) re-proves the whole system on every edit.

Production users and downstream agents will **assume shipped code works**. Our test suite should match that reality: a small set of high-signal gates that catch regressions users would actually hit, not an exhaustive catalog of internal helper behavior.

## Shared test suite

Duplicated harness code is centralized so boundary tests stay short:

| Layer | Location | Used for |
|-------|----------|----------|
| Shared Rust library | `tests/src/` | Environment and state helpers |
| Integration harness | `tests/support/harness.rs` | Spawn daemon, health wait, raw HTTP/MCP helpers |
| Rust contracts | `tests/contracts/` | Public daemon behavior by domain |
| Client contracts | `tests/control-center/`, `tests/sdk-*`, `tests/plugin/` | Desktop, SDK, and plugin boundaries |

Do not copy `test_state()` / `spawn_daemon()` blocks into new tests — extend the shared harness.

## What we keep

| Layer | Purpose | Examples |
|-------|---------|----------|
| **Smoke / first-run** | Proves install → status → store → recall | `tests/scripts/first-run-smoke.sh`, `tests/smoke_test.sh` |
| **CLI goldens** | Stable operator-facing output | `tests/contracts/cli_goldens.rs` |
| **Wire contracts** | MCP/HTTP shapes clients depend on | `adapter_conformance.rs`, `mcp_transport.rs`, `mcp_rpc_headers.rs` |
| **Product boundaries** | Desktop IPC, SDK auth/headers, plugin attach | `api-client.test.js`, SDK client tests, `run-mcp.contract.test.cjs` |
| **Clock-Quorum Recall** | Admit, abstain, as-of, morph/cluster paraphrase | `tests/contracts/clock_quorum.rs` |
| **Data integrity** | Migrations, retention, team scoping where users lose data | Selected handler tests (store/recall visibility, compaction prune) |

Run these before a release or when you touch the corresponding boundary.

## What we do not optimize for

- Unit tests for pure math helpers (`normalize`, `days_since`)
- Growing the closed developer lexicon into a general thesaurus
- Reintroducing embeddings or a reranker “just to be sure”
- Duplicate coverage of the same behavior at unit + integration + golden layers
- Splitting test files to satisfy line-count refactors — tests serve the product, not file metrics

If a test only documents how an internal function behaves today, it is a **developer note**, not a release requirement. Prefer deleting or not adding it.

## What to run when

| Change type | Minimum check |
|-------------|----------------|
| Daemon handler / recall / store / clocks | `cargo test -p cortex-tests --offline --test clock_quorum --test store_recall --test conflict --test temporal --test history --test recall_truth` |
| CLI / status / setup | `cargo test -p cortex-tests --test cli_goldens` |
| Desktop UI / IPC | `npm test` in control center |
| SDK | `pytest tests/sdk-python`; `npm --prefix sdks/typescript test` |
| Release tag | Smoke scripts + golden CLI + platform build (see `release.yml`) |

Full `cargo test -p cortex-tests` is optional for most PRs. CI is intentionally lean to preserve GitHub Actions budget; **local smoke + area tests** are the maintainer bar.

## Adding tests

Ask: *Would a user or integrator notice if this broke without us testing it?*

- **Yes** → add or extend a boundary/smoke/contract test.
- **No** → rely on types, code review, and existing integration coverage.

Do not add tests because an agent wants to "verify its work." Ship behavior; prove boundaries.
