# Security Rules -- All Agents

This file is the single source of truth. CLAUDE.md, AGENTS.md, and GEMINI.md reference it.

## Windows Defender -- NEVER TRIGGER
This runs on Windows. These patterns cause ML-based AV false positives (Bearfoos, SuspExec, ClickFix):
- **Never** spawn a detached process that then kills other processes via taskkill
- **Never** read a token/credential file and immediately POST it over HTTP in the same script
- **Never** use `execSync('taskkill /IM ...')` patterns in test or production code
- **Never** write PowerShell that reads secrets then pipes to curl in a single command
- Instead: use Rust's native process management, pass auth via environment variables, keep token reads and HTTP calls in separate logical steps with clear application context
- Test scripts must avoid spawn-sleep-kill-read-token-POST chains -- break into discrete steps with named functions
