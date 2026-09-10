# Cortex Plugin Routing Policy (Local Subprocess)

The Claude Code plugin talks to a **local `cortex` binary** over stdio. It never
opens an HTTP client, never attaches to `:7437`, and never depends on a Control
Center service. There is no remote route in this runtime.

## MCP bridge

1. Resolve a local `cortex` / `cortex.exe` via `resolve-binary.cjs`
   (env override → `~/.cortex/bin` → workspace build → plugin-bundled).
2. Spawn `cortex mcp --agent <agent>` as a child process.
3. Pipe host stdin/stdout/stderr bidirectionally. Lifetime is the MCP client
   connection; when the host disconnects the child exits.

Default agent id is `claude-code` (`CORTEX_PLUGIN_AGENT` overrides).

## SessionStart boot

`hook-boot.cjs` spawns `cortex hook-boot --agent <agent>` and relays its JSON.
It does not probe `/readiness`, `/health`, or any port.

## Other hooks

`hook-event.cjs` already uses the same local pattern: resolve binary, spawn
`cortex hook-event <kind>` (or V5 `capture host-cycle` when
`CORTEX_V5_CAPTURE` is set), relay stdout. Missing binary → `UNAVAILABLE`.

## Environment inputs

| Variable | Role |
|----------|------|
| `CORTEX_PLUGIN_AGENT` | Agent id passed to `--agent` (default `claude-code`) |
| `CORTEX_PLUGIN_DATA` | Plugin data root used to find a bundled binary |
| `CORTEX_APP_BINARY` / `CORTEX_DAEMON_BINARY` / `CORTEX_PLUGIN_CORTEX_BINARY` | Explicit binary path overrides |
| `CORTEX_PLUGIN_DRY_RUN=1` | MCP bridge prints the resolved binary/args and exits without spawning |

`CLAUDE_PLUGIN_OPTION_CORTEX_URL`, `CORTEX_APP_URL`, `CORTEX_API_KEY`, and
token-file auth are **not used**. The local runtime has no HTTP listener.

## Failure reporting

| Condition | Behavior |
|-----------|----------|
| No binary found | Crash-log + exit 1 (MCP); SessionStart emits `UNAVAILABLE` |
| Binary fails to start | Same as no binary |
| Brain open/read fails inside `cortex` | Child stderr + non-zero exit; hooks report `UNAVAILABLE` |

Do not treat `UNAVAILABLE` as an empty brain.

## Host coverage (Claude Code)

| Event / tool | Packaged hook | Path |
|---|---|---|
| SessionStart | `hook-boot.cjs` | Local `cortex hook-boot` |
| UserPromptSubmit | `hook-event.cjs` | Local `hook-event` (V5 native when sidecar set) |
| PostToolUse Bash | `hook-event.cjs` | V5 native (2.1.260 pin) or legacy ToolResult |
| PostToolUse Edit/Write/MultiEdit | `hook-event.cjs` | Legacy ToolResult (matcher only) |
| PreCompact | `hook-event.cjs` | Compaction checkpoint |
| Stop | `hook-event.cjs` | SessionEnd lifecycle |
| Read / other tools | — | Not covered by the native Bash adapter |

MCP stdio alone does **not** grant transcript access. Automatic capture needs
the host hook events above. Unknown/private tool shapes fail closed.

## First-run smoke

1. Build or install a local `cortex` binary (`cargo build -p cortex-daemon`).
2. `cortex status --json` → `"status": "ready"`.
3. Restart the Claude Code session after installing the plugin.
4. From the host, call `cortex_capabilities`, then `cortex_commit` + `cortex_query`.

## Lockstep

Plugin release artifacts and daemon versions still ship in lockstep
(`Info/plugin-lockstep.md`). MCP routing requires a matching local binary.
