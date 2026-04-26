# Cortex Plugin Routing Policy (HTTP Attach-Only)

The Claude Code plugin never starts a Cortex daemon and never shells into a
`cortex` binary for the MCP bridge. The Control Center/service owns daemon
lifecycle. The plugin attaches to an already-running daemon over HTTP.

## Priority Order

1. Explicit plugin URL (`CLAUDE_PLUGIN_OPTION_CORTEX_URL`)
2. App route URL (`CORTEX_APP_URL`)
3. Local attach-only route (`http://127.0.0.1:7437`)

`CORTEX_DEV_PREFER_APP=1` is strict: it requires `CORTEX_APP_URL` and fails
instead of falling back.

## Supported Environment Inputs

- `CLAUDE_PLUGIN_OPTION_CORTEX_URL`
  - Preferred explicit route for team/remote endpoint.
- `CLAUDE_PLUGIN_OPTION_CORTEX_API_KEY`
  - Required API key for non-local explicit remote routes.
- `CORTEX_APP_URL`
  - Preferred app-managed endpoint route during development.
- `CORTEX_API_KEY`
  - Fallback API key for app-managed remote routes.
- `CORTEX_PLUGIN_DRY_RUN=1`
  - Prints resolved route and exits without opening the MCP proxy loop.

Legacy local-spawn toggles are ignored by the MCP entry point because no local
spawn path exists there anymore.

## Route Matrix

- Plugin URL set -> route `remote` -> Node stdio-to-HTTP proxy
- No plugin URL, app URL set -> route `remote` -> Node stdio-to-HTTP proxy
- No plugin/app URL -> route `local` -> Node stdio-to-HTTP proxy to `127.0.0.1:7437`
- `CORTEX_DEV_PREFER_APP=1` without `CORTEX_APP_URL` -> explicit failure

## Lifecycle Guarantees

- Plugin SessionStart hook is status-only and never starts/stops daemon.
- SessionStart probes `/readiness` first and falls back to `/health`.
- Plugin MCP bridge posts JSON-RPC to `/mcp-rpc` with:
  - `X-Cortex-Request: true`
  - `Authorization: Bearer <token>` for local or remote authenticated routes
  - `X-Source-Agent`
  - optional `X-Source-Model`
- Local token auth is read from `CORTEX_TOKEN_PATH`, `CORTEX_HOME/cortex.token`,
  or `~/.cortex/cortex.token`.
- If local mode cannot reach a ready daemon, the bridge exits with
  `APP_INIT_REQUIRED` and instructs the user to open Cortex Control Center.

## Lockstep Requirement

Plugin release artifacts and app daemon versions should still ship in lockstep,
but plugin MCP routing no longer depends on a bundled or canonical daemon binary.

