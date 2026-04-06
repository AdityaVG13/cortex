# Security Policy

Public mirror: [`Info/security-rules.md`](Info/security-rules.md).

## Supported Versions

Security fixes are prioritized for:

- the latest tagged release
- the current `master` branch

## Reporting a Vulnerability

Do **not** post exploit details in public issues or pull requests.

Preferred reporting path:
1. GitHub Security Advisory (private report)
2. If private reporting is unavailable, open a minimal public issue requesting a secure channel (no exploit details, no secrets, no local paths)

Please include:
- affected version or commit
- operating system
- reproduction steps
- impact
- whether auth, token handling, local file access, or data exfiltration is involved

---

# Threat Model

## Protected Data

Cortex stores developer knowledge that may be sensitive intellectual property:
- architecture decisions
- coding conventions
- debugging lessons
- memory/decision embeddings
- team metadata and API-key hashes

## Trust Boundary

- The daemon trusts the **local user** and **authenticated API clients**.
- In team mode, network requests are treated as untrusted unless authenticated.
- Health checks are intentionally public for monitoring; mutation and memory endpoints require auth.

## Attack Surfaces

1. Local filesystem under `~/.cortex/` (or `CORTEX_HOME`)
2. HTTP API listener (`cortex serve`)
3. MCP JSON-RPC over stdio (`cortex plugin mcp` / `cortex mcp`)

---

# Authentication & Authorization

## Modes

| Mode | Credential | Storage | Source |
|---|---|---|---|
| Solo | Bearer token | `~/.cortex/cortex.token` | `daemon-rs/src/auth.rs` (`generate_token`, `read_token`, lines ~199-212) |
| Team | API key (`ctx_...`) | Argon2id hash in DB | `daemon-rs/src/auth.rs` (`generate_ctx_api_key`, `hash_api_key_argon2id`, lines ~225-256) |

## Team API Key Format

Team keys are generated as:
- prefix: `ctx_`
- random body: 43 base62 chars
- checksum: 3 base62 chars (FNV-1a derived)

Reference: `daemon-rs/src/auth.rs` (`generate_ctx_api_key`, lines ~225-243).

## Argon2id Parameters

| Parameter | Value | Source |
|---|---:|---|
| Memory cost | `64 * 1024` KiB (64 MB) | `daemon-rs/src/auth.rs:246` |
| Time cost | `3` | `daemon-rs/src/auth.rs:246` |
| Parallelism | `4` | `daemon-rs/src/auth.rs:246` |
| Variant | Argon2id | `daemon-rs/src/auth.rs:247` |

## Authorization Model

- Sensitive HTTP/MCP endpoints require Bearer auth (`ensure_auth` in `daemon-rs/src/server.rs`, e.g., `/mcp-rpc` at lines ~167-179).
- `/health` is intentionally public (`daemon-rs/src/server.rs:53`) for liveness checks.
- Team mode enforces per-user ownership and visibility controls via `owner_id`/`visibility` columns (`daemon-rs/src/db.rs`, migration lines ~402-627).

---

# Data Handling

## Storage

| Item | Default location | Configurable | Notes |
|---|---|---|---|
| Cortex home | `~/.cortex/` | `CORTEX_HOME` | `CortexPaths::resolve` (`daemon-rs/src/auth.rs`, lines ~59-77) |
| SQLite DB | `~/.cortex/cortex.db` | `CORTEX_DB` | Main memory store |
| Auth token | `~/.cortex/cortex.token` | via home override | Solo-mode bearer token |
| PID/lock files | `~/.cortex/cortex.pid`, `~/.cortex/cortex.lock` | via home override | Process coordination |

## Data Contents

Database includes:
- memories, decisions, events
- embeddings (ONNX-generated)
- recall feedback
- team/user ownership metadata
- API key hashes (not plaintext keys)

## Encryption & Telemetry

- SQLite is **not encrypted at rest by default**.
- SQLCipher is not bundled; at-rest encryption requires a custom SQLCipher-enabled build/deployment.
- Cortex does **not** include telemetry, phone-home analytics, or cloud sync.

## Export / Import

- `cortex export` writes user-controlled JSON/SQL export artifacts.
- `cortex import` restores exported data into solo/team databases.

---

# Network Security

## Defaults

- Default bind is localhost (`127.0.0.1`) via `CORTEX_BIND` fallback (`daemon-rs/src/server.rs:346`).
- CORS is restricted to localhost origins, including dynamic daemon port (`daemon-rs/src/server.rs:30-46`).

## Team Mode

- Team deployments may bind to `0.0.0.0` (operator configured).
- TLS is enforced when team mode is detected; invalid TLS config causes startup refusal (`daemon-rs/src/server.rs:358-363`).

## MCP Transport

- Primary MCP transport is stdio.
- MCP proxy forwards JSON-RPC to `/mcp-rpc` with Bearer auth (`daemon-rs/src/mcp_proxy.rs:54-111`).

## SSRF

Cortex does not execute arbitrary outbound HTTP requests from user-provided memory payloads; daemon network traffic is limited to explicit service/proxy flows.

---

# Known Limitations

- No encryption at rest by default (SQLCipher requires custom build/deployment)
- No binary code signing yet (targeted for future release)
- `cortex.token` protection relies on filesystem permissions
- No built-in API rate limiting (optimized for trusted local/team deployments)
- No dedicated audit log of all access attempts
- Cortex Control Center does not yet support custom TLS certificate trust configuration

---

# Deployment Recommendations

## Solo (single developer machine)

- Keep default localhost bind.
- No extra network hardening is required for local-only usage.

## Team (shared daemon)

- Always enable TLS (self-signed minimum; managed cert preferred).
- Rotate API keys at least quarterly.
- Restrict access to VPN/private network segments.

## Corporate / production-like use

- Place Cortex behind a reverse proxy (nginx/Caddy/Traefik) with TLS termination and access controls.
- Consider SQLCipher-enabled builds for at-rest encryption requirements.
- Monitor permissions on `~/.cortex/cortex.token` and daemon data directories.

**Never expose Cortex directly to the public internet without a reverse proxy and TLS.**

---

# Vulnerability Disclosure

## Preferred Channels

1. GitHub Security Advisories (private)
2. Minimal public issue requesting secure follow-up when private channel is unavailable

## Expected Response

- Initial triage target: within 3 business days
- Follow-up/clarification: as needed based on report quality and severity
- Public disclosure: after fix or mitigation is available

## Scope

This policy covers:
- Rust daemon (`daemon-rs/`)
- Claude plugin scaffold (`plugins/cortex-plugin/`)
- Cortex Control Center desktop app (`desktop/cortex-control-center/`)
- MCP tools and proxy surfaces
