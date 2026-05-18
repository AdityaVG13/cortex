# Cortex CLI Agent Playbook

1. Discover the CLI contract with `cortex capabilities --json`.
2. Resolve local paths with `cortex paths --json` before inspecting files.
3. Attach locally with `cortex boot --json` or `cortex mcp --agent <name>`.
4. Prefer JSON flags where available and parse stdout only after exit code 0.
5. Treat stderr as diagnostics; for exit code 1, inspect the message before retrying.
6. Keep destructive operations explicit: `restore`, `admin rollback --apply`, user removal, and team removal are gated.
