# Cortex Plugin Routing Policy (App-First + Local Service-First)

The plugin prefers app-managed routing, but local plugin mode is allowed for
plugin-only users. In local mode, `cortex plugin mcp` performs service-first
daemon ensure on Windows and then bridges MCP.

## Priority Order
1. Explicit plugin URL (`CLAUDE_PLUGIN_OPTION_CORTEX_URL`)
2. App route URL (`CORTEX_APP_URL`)
3. Local route (`localhost` with service-first ensure)

## Binary Selection Order (for MCP bridge and SessionStart probes)
1. Explicit binary override (`CORTEX_APP_BINARY`, `CORTEX_DAEMON_BINARY`, `CORTEX_PLUGIN_CORTEX_BINARY`)
2. App-managed canonical install (`~/.cortex/bin/cortex[.exe]`)
3. Common workspace dev/release builds (`~/cortex/daemon-rs/target*`)
4. Bundled plugin runtime binary (`CLAUDE_PLUGIN_DATA/bin/cortex[.exe]`) -- allowed as local fallback when safe

This keeps plugin tooling aligned with the app daemon in development and avoids
relying on stale bundled binaries when a canonical app-managed binary exists.

## Local Binary Safety Gate
- In local mode, plugin scripts reject temporary runtime binaries by default.
- App-managed binaries are preferred; plugin-bundled fallback is allowed when safe.
- Optional strict mode:
  - `CORTEX_PLUGIN_REQUIRE_APP_BINARY=1` forces app-managed binary only.
- Optional compatibility override:
  - `CORTEX_PLUGIN_ALLOW_BUNDLED_BINARY=1` permits bundled fallback even when strict mode is set.

## Supported Environment Inputs
- `CLAUDE_PLUGIN_OPTION_CORTEX_URL`
  - Preferred explicit route for team/remote endpoint.
- `CLAUDE_PLUGIN_OPTION_CORTEX_API_KEY`
  - Optional API key for explicit remote route.
- `CORTEX_APP_URL`
  - Preferred app-managed endpoint route during development.
- `CORTEX_PLUGIN_DRY_RUN=1`
  - Prints resolved route and exits without launching MCP bridge child.
- `CORTEX_APP_BINARY`, `CORTEX_DAEMON_BINARY`, `CORTEX_PLUGIN_CORTEX_BINARY`
  - Optional explicit binary override for plugin bridge/hook execution.
- `CORTEX_WORKSPACE_ROOT`
  - Optional workspace root used to discover dev/release daemon builds.
- `CORTEX_PLUGIN_REQUIRE_APP_BINARY=1`
  - Optional strict policy to require app-managed binary in local mode.
- `CORTEX_PLUGIN_ALLOW_BUNDLED_BINARY=1`
  - Optional escape hatch to permit bundled plugin binaries when strict mode is active.

## Route Matrix
- Plugin URL set -> route `remote` -> pass `--url` and optional `--api-key`
- No plugin URL, app URL set -> route `remote` -> pass `--url`
- No plugin/app URL -> route `local` -> service-first ensure on Windows, then local bridge

## Lifecycle Guarantees
- Plugin SessionStart hook is status-only and never starts/stops daemon.
- SessionStart probes `/readiness` first and falls back to `/health` for older daemons.
- Plugin MCP bridge never direct-spawns daemon binaries itself.
- Local plugin mode delegates daemon readiness to `cortex plugin mcp` service-first policy (Windows) or returns a clear unsupported-local-ensure error on non-Windows.
- If local mode resolves only temporary binaries, plugin blocks fallback and surfaces safe-binary guidance.

## Lockstep Requirement
Plugin-bundled daemon versions and app daemon versions should ship in lockstep.

Minimum release guard:
1. Build plugin bundle from the same daemon commit used by app release artifacts.
2. Keep plugin version and daemon release manifest aligned in release checklist.
3. Add CI guard that fails release when plugin bundle daemon version differs from app daemon version.
