# Cortex CLI Agent Ergonomics Pass 1

| Area | Before | After | Status |
| --- | --- | --- | --- |
| First-run discovery | Bare invocation exited as a usage failure | Bare invocation prints help and exits 0 | Improved |
| Help | Human help existed, but no agent-specific discovery path | Help names capabilities and robot-docs entrypoints | Improved |
| Machine discovery | No top-level contract | `cortex capabilities --json` added | Improved |
| Agent guide | No compact command guide | `cortex robot-docs guide` added | Improved |
| Unknown command recovery | Generic usage only | Intent hints for common agent mistakes | Improved |
| Regression coverage | No tests for these surfaces | Four focused helper tests added | Improved |

Residual risk: nested subcommands still have inconsistent help, JSON, and error shapes. A follow-up pass should standardize those families without changing operational semantics.
