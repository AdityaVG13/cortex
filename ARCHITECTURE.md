# Cortex - Technical Architecture Report

Generated: 2026-05-22

## Executive Summary

Cortex is a private local memory system for AI tools. The main product is a Rust daemon that exposes HTTP and MCP-compatible memory operations, backed by SQLite and local ONNX embeddings; the companion Control Center is a Tauri desktop app with a React/Vite frontend.

Key statistics:
- Version: `0.6.0` in the root package, daemon crate, and Control Center package (`package.json:3`, `daemon-rs/Cargo.toml:3`, `desktop/cortex-control-center/package.json:3`).
- Source footprint: about 71,793 lines across 104 Rust/JavaScript/JSX/TypeScript source files under the daemon, desktop frontend, and Tauri shell.
- Main runtime surfaces: daemon binary `cortex`, local HTTP API, MCP stdio proxy, Tauri desktop app, React Control Center, plugin skills/docs, benchmark tooling.

## Entry Points

| Entry | Location | Purpose |
| --- | --- | --- |
| Root package scripts | `package.json:6` | Top-level build, test, daemon clippy, desktop build/dev, and ops wrappers. |
| Daemon binary declaration | `daemon-rs/Cargo.toml:8` | Builds the `cortex` binary from `daemon-rs/src/main.rs`. |
| Daemon CLI dispatcher | `daemon-rs/src/main.rs:445` | Parses command mode and dispatches `serve`, `mcp`, `boot`, `paths`, plugin, import/export, admin, cleanup, and other CLI flows. |
| HTTP daemon mode | `daemon-rs/src/main.rs:487` | `cortex serve` installs shutdown handlers and starts the Axum daemon runtime. |
| MCP stdio mode | `daemon-rs/src/main.rs:514` | `cortex mcp` ensures or targets a daemon and runs the MCP proxy transport. |
| Runtime initialization | `daemon-rs/src/main.rs:4744` | Creates `RuntimeState`, opens/configures SQLite, loads token state, and prepares graceful shutdown. |
| HTTP router | `daemon-rs/src/server.rs:46` | Defines public, core, conductor, feed, admin, event, brain telemetry, import/export, and MCP-RPC routes. |
| Tauri desktop main | `desktop/cortex-control-center/src-tauri/src/main.rs:2903` | Enforces single Control Center instance, wires Tauri plugins/state, bootstraps/supervises the daemon. |
| Tauri daemon commands | `desktop/cortex-control-center/src-tauri/src/main.rs:1551` | Exposes desktop commands such as quit, hide-to-tray, daemon status, daemon start, and later lifecycle operations. |
| React mount | `desktop/cortex-control-center/src/main.jsx:7` | Mounts the React `App` into the Vite/Tauri webview. |
| Control Center app | `desktop/cortex-control-center/src/App.jsx:1456` | Holds the main panel state, daemon state, analytics, live-surface data, settings, and UI workflows. |
| Frontend API client | `desktop/cortex-control-center/src/api-client.js:154` | Calls the daemon through Tauri IPC when available, falling back to authenticated local HTTP. |

## Key Types

| Type | Location | Purpose |
| --- | --- | --- |
| `RuntimeState` | `daemon-rs/src/state.rs:251` | Shared Axum state containing SQLite connections, token, event channels, sessions, recall caches, embedding/rerank engines, rate limiter, team-mode metadata, and readiness/degradation flags. |
| `StoreRequest` | `daemon-rs/src/api_types.rs:127` | JSON body for storing a memory/decision, including text, context, source identity, model metadata, confidence, TTL, and retention class. |
| `RetentionClass` | `daemon-rs/src/api_types.rs:23` | Durable/operational/audit/ephemeral classification that controls default TTLs and retention semantics. |
| `RecallQuery` / `RecallBody` | `daemon-rs/src/handlers/recall.rs:985` | Query/body model for recall, semantic recall, budgeted recall, and related lookup endpoints. |
| `BootQuery` | `daemon-rs/src/handlers/boot.rs:87` | Query model for boot profile, agent identity, and token budget. |
| `BootResult` | `daemon-rs/src/compiler.rs:75` | Compiled boot capsule with prompt text, token estimate, savings metadata, and source capsules. |
| `EmbeddingEngine` | `daemon-rs/src/embeddings.rs:253` | Local ONNX embedding engine with a session pool and model metadata for semantic indexing/search. |
| `BudgetConfig` / `BudgetEndpoint` | `daemon-rs/src/budgets.rs:12` | Local endpoint budget model for store, recall, boot, and MCP request enforcement. |
| `SourceIdentity` | `daemon-rs/src/handlers/mod.rs:31` | Normalized caller identity assembled from source-agent and source-model headers. |
| `RerankConfig` | `daemon-rs/src/rerank.rs:41` | Optional cross-encoder rerank settings with off/shadow/primary modes. |

## Data Flow

### Store Path

```text
AI tool / Control Center
    -> POST /store with X-Cortex-Request + bearer token
    -> Axum router
    -> auth, source identity, endpoint budgets
    -> StoreRequest validation and retention classification
    -> optional embedding generation
    -> SQLite write transaction, conflict/dedup policy, FTS/vector rows
    -> JSON result + event/savings telemetry
```

`/store` is registered on the router at `daemon-rs/src/server.rs:58`. The handler validates TTL, resolves provenance, computes an embedding if the engine is loaded, locks the write connection, persists the decision, optionally stores the embedding, updates focus state, and returns `{ "stored": true }` on success (`daemon-rs/src/handlers/store.rs:249`).

### Recall Path

```text
AI tool / Control Center
    -> GET or POST /recall, /recall/semantic, /recall/budget, /peek, /unfold
    -> auth + rate/budget checks
    -> recall policy and token budget resolution
    -> keyword FTS + semantic/vector candidates
    -> RRF/fallback ranking, optional rerank, budget compaction
    -> JSON matches and token-savings metadata
```

Recall routes are registered at `daemon-rs/src/server.rs:59`. `handle_recall` authenticates the caller, enforces team-mode caller scope, validates `q`, resolves recall policy/budget, and then enters the unified recall pipeline (`daemon-rs/src/handlers/recall.rs:1009`). `/peek` returns lighter headlines and token-usage metadata from the same underlying recall machinery (`daemon-rs/src/handlers/recall.rs:1421`).

### Boot Path

```text
AI tool startup
    -> GET /boot?agent=...&budget=...
    -> auth + endpoint budget
    -> register agent presence
    -> clear recently served content for that agent
    -> compiler builds identity, delta, conductor/feed/task capsules
    -> response becomes session boot context
```

`/boot` is registered at `daemon-rs/src/server.rs:73`. The handler authenticates, checks team-mode caller requirements, registers the agent, clears scoped served-content cache, cleans expired conductor state, and calls the compiler (`daemon-rs/src/handlers/boot.rs:95`). The compiler returns a `BootResult` with prompt, token estimate, savings data, and capsules (`daemon-rs/src/compiler.rs:75`).

### Desktop Path

```text
Tauri app startup
    -> single-instance guard
    -> locate bundled cortex binary
    -> bootstrap and supervise daemon
    -> React app polls status/stats/sessions/tasks/feed
    -> API client uses Tauri IPC fetch_cortex, then local HTTP fallback
```

The Tauri main function acquires an app-instance guard and supervises the daemon (`desktop/cortex-control-center/src-tauri/src/main.rs:2903`). The React app owns dashboard state and panel selection (`desktop/cortex-control-center/src/App.jsx:1456`). The frontend API client prefers Tauri IPC, handles token refresh/retry, and falls back to local HTTP when IPC is unavailable or times out (`desktop/cortex-control-center/src/api-client.js:154`).

## External Dependencies

| Dependency | Location | Purpose | Critical? |
| --- | --- | --- | --- |
| `axum`, `tower`, `tower-http` | `daemon-rs/Cargo.toml:13`, `daemon-rs/Cargo.toml:40` | HTTP server, routing, middleware, CORS, panic handling. | Yes |
| `tokio`, `tokio-stream` | `daemon-rs/Cargo.toml:22` | Async runtime, background tasks, signal handling, channels, network IO. | Yes |
| `rusqlite` with bundled SQLite | `daemon-rs/Cargo.toml:17` | Local durable storage for memories, decisions, sessions, events, migrations, FTS. | Yes |
| `sqlite-vec` | `daemon-rs/Cargo.toml:37` | Local vector storage/search support. | Yes for semantic path |
| `ort`, `tokenizers`, `ndarray` | `daemon-rs/Cargo.toml:31` | In-process ONNX embeddings and reranking model execution. | Yes for semantic path |
| `argon2`, `hmac`, `sha2` | `daemon-rs/Cargo.toml:25` | Team-mode API key hashing and auth-related crypto. | Yes |
| `rustls`, `tokio-rustls` | `daemon-rs/Cargo.toml:42` | TLS for explicit team/non-loopback deployments. | Conditional |
| `reqwest` | `daemon-rs/Cargo.toml:34` | HTTP client work such as model/download or remote-target interactions. | Conditional |
| `tauri`, `tauri-plugin-updater` | `desktop/cortex-control-center/src-tauri/Cargo.toml:17` | Native desktop shell, updater plugin, daemon supervision commands. | Yes for desktop app |
| `react`, `react-dom`, `three` | `desktop/cortex-control-center/package.json:26` | Control Center UI and brain visualization. | Yes for desktop UI |
| `vite`, `vitest` | `desktop/cortex-control-center/package.json:31` | Frontend dev/build and unit test runner. | Development |

## Configuration

| Source | Location / Example | Priority |
| --- | --- | --- |
| CLI flags | `--home`, `--db`, `--port`, `--bind` parsed in `daemon-rs/src/auth.rs:83` | Highest for path/daemon bind fields |
| Environment variables | `CORTEX_HOME`, `CORTEX_DB`, `CORTEX_PORT`, `CORTEX_BIND` in `daemon-rs/src/auth.rs:48` | Below CLI flags |
| Runtime defaults | `~/.cortex`, `cortex.db`, `cortex.token`, `cortex.pid`, default port `7437` in `daemon-rs/src/auth.rs:48` and `daemon-rs/src/main.rs:6` | Fallback |
| Local budgets | `~/.cortex/budgets.toml` loaded by `BudgetConfigStatus::load_from_home` (`daemon-rs/src/budgets.rs:200`) | Runtime file |
| Embedding model | `CORTEX_EMBEDDING_MODEL` profile selection and fallback in `daemon-rs/src/embeddings.rs:181` | Optional runtime env |
| Embedding pool | `CORTEX_EMBEDDING_POOL_SIZE` clamped in `daemon-rs/src/embeddings.rs:233` | Optional runtime env |
| Rerank mode | `CORTEX_RERANK_MODE`, `CORTEX_RERANK_ENABLED`, `CORTEX_RERANK_TOP_N`, `CORTEX_RERANK_FUSION_ALPHA` in `daemon-rs/src/rerank.rs:16` | Optional runtime env |
| Root env template | `.env.example:1` | Documentation/sample only |
| Vite dev server | `desktop/cortex-control-center/vite.config.js:4` | Desktop frontend dev config |
| Tauri app config | `desktop/cortex-control-center/src-tauri/tauri.conf.json:1` | Desktop product, build, CSP, bundle, updater |

## Module Structure

```text
daemon-rs/src/
  main.rs              CLI dispatcher, daemon lifecycle, background jobs
  server.rs            Axum router, HTTP/TLS/IPC serving
  state.rs             shared RuntimeState and startup initialization
  db.rs                SQLite config, migrations, repair, WAL/FTS helpers
  api_types.rs         shared API payload and retention types
  handlers/            HTTP/MCP handler modules for store/recall/boot/admin/etc.
  embeddings.rs        local ONNX embedding models and vector codecs
  compiler.rs          boot capsule compilation
  mcp_proxy.rs         MCP stdio-to-daemon proxy
  auth.rs              paths, token auth, API keys, source identity helpers
  budgets.rs           local request budget configuration
  rerank.rs            optional cross-encoder reranking

desktop/cortex-control-center/
  src-tauri/src/main.rs    native app, commands, daemon supervision
  src/main.jsx             React mount
  src/App.jsx              main Control Center UI state and panels
  src/api-client.js        IPC/HTTP daemon client
  src/brain-v2/            Three.js brain visualization modules
```

## Test Infrastructure

| Type | Location | Count / Notes |
| --- | --- | --- |
| Root test script | `package.json:8` | Runs daemon tests with `cargo test --all-features`. |
| Daemon Rust tests | `daemon-rs/src`, `daemon-rs/tests` | 563 inline Rust test attributes found across 50 Rust files. |
| Desktop Vitest suite | `desktop/cortex-control-center/package.json:17` | Runs `vitest run`. |
| Desktop unit/UI tests | `desktop/cortex-control-center/src` | 23 `*.test.*` files covering API client, analytics, settings, keyboard access, reflow, panels, brain-v2 helpers, and import cycles. |
| Desktop lifecycle smoke | `desktop/cortex-control-center/package.json:20` | `verify:lifecycle:dev` drives a dev lifecycle verification script. |
| Frontend build | `desktop/cortex-control-center/package.json:9` | `vite build` via `web:build`; Tauri build composes daemon and frontend work. |
| Daemon clippy wrapper | `package.json:9` | `cargo clippy --all-targets --all-features -- -D warnings`. |

Common local checks:

```bash
npm test
npm run daemon:clippy
npm --prefix desktop/cortex-control-center test
npm --prefix desktop/cortex-control-center run web:build
```

## Error Handling And Safety

- Handler panics are converted to HTTP 500 responses by `CatchPanicLayer`, keeping the daemon process alive (`daemon-rs/src/server.rs:225`).
- Protected HTTP endpoints require `X-Cortex-Request` as an SSRF guard before auth is accepted (`daemon-rs/src/handlers/mod.rs:150`).
- Auth uses bearer token comparison for solo mode and Argon2-verified `ctx_` API keys for team mode (`daemon-rs/src/handlers/mod.rs:190`).
- Startup opens/configures SQLite, runs schema initialization, quick-checks integrity, and can attempt auto-repair before continuing in degraded mode (`daemon-rs/src/state.rs:363`).
- Startup checkpoints WAL before background work and again on shutdown (`daemon-rs/src/main.rs:4790`, `daemon-rs/src/main.rs:5205`).
- Non-loopback/team TLS policy lives in the server layer; plain HTTP can be rejected for unsafe external binds (`daemon-rs/src/server.rs:691`).

## Notes And Gotchas

- The repository has two major products in one tree: the daemon and the desktop Control Center. Root package scripts primarily orchestrate daemon and desktop subprojects rather than owning a standalone Node app.
- The daemon is intentionally localhost-first. External bind and team-mode behavior are explicit configuration paths, not the default.
- Semantic recall depends on local model assets. The code can degrade or queue/download assets when the model is unavailable, so semantic behavior can differ between first-run and warm installations.
- The React app can run in browser/dev mode, but production desktop paths assume Tauri IPC is available and fall back to local HTTP when needed.
- `target`, `node_modules`, generated benchmark outputs, Graphify outputs, and local runtime databases/logs are present in the working tree but should be treated as build/runtime artifacts unless intentionally tracked.
