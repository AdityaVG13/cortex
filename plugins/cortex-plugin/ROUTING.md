# Cortex Plugin Routing Policy (Attach-Only)

The plugin no longer manages daemon lifecycle. It only connects to an existing
Cortex daemon endpoint.

## Priority Order
1. Explicit plugin URL (`CLAUDE_PLUGIN_OPTION_CORTEX_URL`)
2. App route URL (`CORTEX_APP_URL`, legacy: `CORTEX_DEV_APP_URL`)
3. Local attach route (`localhost` daemon expected to already be running)

## Binary Selection Order (for MCP bridge and SessionStart probes)
1. Explicit binary override (`CORTEX_APP_BINARY`, `CORTEX_DAEMON_BINARY`, `CORTEX_PLUGIN_CORTEX_BINARY`)
2. App-managed canonical install (`~/.cortex/bin/cortex[.exe]`)
3. Common workspace dev/release builds (`~/cortex/daemon-rs/target*`)
4. Bundled plugin runtime binary (`CLAUDE_PLUGIN_DATA/bin/cortex[.exe]`)

This keeps plugin tooling aligned with the app daemon in development and avoids
relying on stale bundled binaries when a canonical app-managed binary exists.

## Supported Environment Inputs
- `CLAUDE_PLUGIN_OPTION_CORTEX_URL`
  - Preferred explicit route for team/remote endpoint.
- `CLAUDE_PLUGIN_OPTION_CORTEX_API_KEY`
  - Optional API key for explicit remote route.
- `CORTEX_APP_URL`
  - Preferred app-managed endpoint route during development.
- `CORTEX_DEV_APP_URL`
  - Legacy alias for app-managed endpoint route.
- `CORTEX_PLUGIN_DRY_RUN=1`
  - Prints resolved route and exits without launching MCP bridge child.
- `CORTEX_APP_BINARY`, `CORTEX_DAEMON_BINARY`, `CORTEX_PLUGIN_CORTEX_BINARY`
  - Optional explicit binary override for plugin bridge/hook execution.
- `CORTEX_WORKSPACE_ROOT`
  - Optional workspace root used to discover dev/release daemon builds.

## Deprecated / Ignored for Routing
- `CORTEX_DEV_PREFER_APP`
- `CORTEX_DEV_DISABLE_LOCAL_SPAWN`
- `CORTEX_PLUGIN_ALLOW_LOCAL_SPAWN`

These no longer affect plugin routing behavior.

## Route Matrix
- Plugin URL set -> route `remote` -> pass `--url` and optional `--api-key`
- No plugin URL, app URL set -> route `remote` -> pass `--url`
- No plugin/app URL -> route `local` attach-only -> never spawn daemon

## Lifecycle Guarantees
- Plugin SessionStart hook is status-only and never starts/stops daemon.
- SessionStart probes `/readiness` first and falls back to `/health` for older daemons.
- Plugin MCP bridge does not request local daemon spawn.
- If local daemon is unavailable, plugin fails fast with attach-only guidance.

## Lockstep Requirement
Plugin-bundled daemon versions and app daemon versions should ship in lockstep.

Minimum release guard:
1. Build plugin bundle from the same daemon commit used by app release artifacts.
2. Keep plugin version and daemon release manifest aligned in release checklist.
3. Add CI guard that fails release when plugin bundle daemon version differs from app daemon version.
