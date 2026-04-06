# Security Policy

## Supported Releases

Security fixes are prioritized for the latest released version and the current `master` branch.

## Reporting a Vulnerability

Please do not disclose security issues in public GitHub issues or pull requests.

Use one of these paths:

1. GitHub private vulnerability reporting, if it is enabled for the repository.
2. If private reporting is not available, open a minimal public issue requesting a secure contact channel and do not include exploit details, secrets, reproduction steps, or affected local paths.

Include, when possible:

- affected version or commit,
- operating system,
- reproduction steps,
- impact,
- whether the issue involves auth, local file access, token handling, or data exfiltration.

## What to Expect

- Initial triage as soon as practical.
- A request for clarification if the report is incomplete.
- Public disclosure only after a fix or mitigation is ready.

## Security Notes for Users

- Sensitive endpoints require the bearer token stored in `~/.cortex/cortex.token`.
- Cortex is intended to run locally and restricts CORS to localhost by default.
- Review your local environment before exposing Cortex through tunnels, proxies, remote desktops, or shared machines.
