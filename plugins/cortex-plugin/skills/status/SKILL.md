---
name: status
description: Show Cortex brain status including local binary health, memory counts, and mode. Use when the user wants to check if Cortex is working, debug issues, or see memory statistics.
---

# Cortex Status

Show the current state of your Cortex brain.

## Usage

```
/cortex:status
```

## What It Shows

1. **DB Path**: Where your brain is stored (`~/.cortex/cortex.db`)
2. **Runtime**: Local binary resolved / missing (no HTTP port)
3. **Memory Counts**: Number of memories and decisions stored
4. **Mode**: Solo (local)
5. **Plugin Version**: Current installed version

## Example Output

```
Cortex Status
=============
DB Path:    ~/.cortex/cortex.db
Runtime:    local cortex binary
Memories:   142
Decisions:  89
Mode:       Solo (local)
Version:    0.6.0
```

## Troubleshooting

If status shows issues:

| Problem | Check |
|---------|-------|
| No local cortex binary | Build/install `cortex` or set `CORTEX_APP_BINARY`. The plugin never opens an HTTP port. |
| Brain unavailable | Run `cortex status --json` and follow `nextAction`. |
| Low memory count | Check if you're using the correct DB path (`CORTEX_HOME` / `CORTEX_DB`). |

## Integration

This skill uses the `cortex_capabilities` / `cortex_health` MCP tools over local stdio (`cortex mcp`).
