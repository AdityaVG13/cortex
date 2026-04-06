---
name: status
description: Show Cortex brain status including daemon health, memory counts, and mode. Use when the user wants to check if Cortex is working, debug issues, or see memory statistics.
---

# Cortex Status

Show the current state of your Cortex brain.

## Usage

```
/cortex:status
```

## What It Shows

1. **DB Path**: Where your brain is stored (`~/.cortex/cortex.db`)
2. **Daemon Status**: Running/not running/remote connection
3. **Memory Counts**: Number of memories and decisions stored
4. **Mode**: Solo (local) or Team (remote server)
5. **Plugin Version**: Current installed version

## Example Output

```
Cortex Status
=============
DB Path:    ~/.cortex/cortex.db
Daemon:     Running on port 7437
Memories:   142
Decisions:  89
Mode:       Solo (local)
Version:    0.4.0
```

## Troubleshooting

If status shows issues:

| Problem | Check |
|---------|-------|
| Daemon not running | Run `cortex serve` or restart plugin |
| Can't connect | Check port 7437 is not blocked |
| Team mode fails | Verify `CORTEX_URL` and `CORTEX_API_KEY` |
| Low memory count | Check if you're using the correct DB path |

## Modes

- **Solo mode**: All data is local at `~/.cortex/`
- **Team mode**: Connected to remote server, data is shared

## Integration

This skill uses the `cortex_health` MCP tool to fetch daemon status and memory counts.
