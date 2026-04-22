<p align="center"><a href="../README.md">← Back to README</a></p>

# Security Policy

> Cortex security posture, threat model, and vulnerability reporting.

Top-level GitHub policy file: [`SECURITY.md`](../SECURITY.md)

---

## Supported versions

| Version | Status |
|---------|--------|
| 0.5.x | Current release, actively supported |
| 0.4.x | Security fixes only |
| < 0.4 | Not supported |

---

## Reporting a vulnerability

**Do not post exploit details in public issues or pull requests.**

1. **Preferred**: [GitHub Security Advisory](https://github.com/AdityaVG13/cortex/security/advisories/new) (private report)
2. **Fallback**: Open a minimal public issue requesting a secure channel — no exploit details, no secrets, no local paths

Include: affected version/commit, OS, reproduction steps, impact, and whether auth/token handling/data exfiltration is involved.

| Step | Timeline |
|------|----------|
| Initial triage | Within 3 business days |
| Follow-up | Based on severity and report quality |
| Public disclosure | After fix or mitigation is available |

---

## Threat model

### Protected data

Cortex stores developer knowledge that may be sensitive intellectual property: architecture decisions, coding conventions, debugging lessons, memory/decision embeddings, team metadata, and API-key hashes.

### Trust boundary

- The daemon trusts the **local user** and **authenticated API clients**.
- In team mode, network requests are treated as untrusted unless authenticated.
- Health checks are intentionally public for monitoring; mutation endpoints require auth.

### Attack surfaces

| Surface | Entry point |
|---------|------------|
| **Filesystem** | `~/.cortex/` (or `CORTEX_HOME`) |
| **HTTP API** | `cortex serve` listener |
| **MCP transport** | `cortex plugin mcp` / `cortex mcp` (stdio) |

---

## Authentication and authorization

### Auth modes

| Mode | Credential | Storage |
|------|-----------|---------|
| **Solo** | Bearer token | `~/.cortex/cortex.token` (generated at first start) |
| **Team** | API key (`ctx_...`) | Argon2id hash in DB |

### Team API key format

- Prefix: `ctx_`
- Body: 43 base62 random characters
- Checksum: 3 base62 characters (FNV-1a derived)

### Argon2id parameters

| Parameter | Value |
|-----------|------:|
| Memory cost | 64 MB |
| Time cost | 3 |
| Parallelism | 4 |
| Variant | Argon2id |

### Authorization model

- Protected endpoints require Bearer auth + `X-Cortex-Request: true` (SSRF guard)
- `/health` and `/readiness` are intentionally public for liveness checks
- Team mode enforces per-user ownership via `owner_id` / `visibility` columns
- Localhost callers are exempt from auth-failure lockout; non-loopback brute-force protections apply
- Team-mode destructive endpoints require admin + rated auth

---

## Data handling

### Storage locations

| Item | Default | Override |
|------|---------|---------|
| Home directory | `~/.cortex/` | `CORTEX_HOME` |
| SQLite database | `~/.cortex/cortex.db` | `CORTEX_DB` |
| Auth token | `~/.cortex/cortex.token` | Via home override |
| PID/lock files | `~/.cortex/cortex.pid`, `~/.cortex/cortex.lock` | Via home override |

### What's in the database

Memories, decisions, events, embeddings (ONNX-generated), recall feedback, team/user ownership metadata, API key hashes (not plaintext keys).

### Encryption and telemetry

- SQLite is **not encrypted at rest** by default. At-rest encryption requires a SQLCipher-enabled build.
- Cortex includes **no telemetry**, no phone-home analytics, and no cloud sync.

---

## Network security

### Defaults

- Default bind: `127.0.0.1` (localhost only)
- CORS restricted to localhost origins including dynamic daemon port
- MCP primary transport: stdio

### Team mode

- Non-loopback binds require TLS on public/routed interfaces
- Private mesh (Tailscale/WireGuard) may use transport-level encryption with explicit operator acknowledgment
- Invalid transport security config causes startup refusal

### SSRF

Cortex does not execute outbound HTTP requests from user-provided memory payloads. Daemon network traffic is limited to explicit service/proxy flows.

---

## Known limitations

| Limitation | Notes |
|-----------|-------|
| No at-rest encryption by default | SQLCipher requires custom build |
| No binary code signing | Targeted for future release |
| Token protection = filesystem permissions | `cortex.token` relies on OS-level file access control |
| No built-in API rate limiting | Optimized for trusted local/team deployments |
| No dedicated access audit log | Planned for future version |

---

## Deployment recommendations

### Solo (single machine)

Keep default localhost bind. No extra hardening needed for local-only usage.

### Team (shared daemon)

- Enable TLS (self-signed minimum, managed cert preferred)
- HTTP without TLS only on private encrypted mesh with explicit exemption
- Rotate API keys at least quarterly
- Restrict access to VPN/private network segments

### Production-like

- Place behind a reverse proxy (Nginx/Caddy/Traefik) with TLS termination
- Consider SQLCipher for at-rest encryption requirements
- Monitor permissions on `~/.cortex/` directories

> **Never expose Cortex directly to the public internet without a reverse proxy and TLS.**

---

## Scope

This policy covers:

- Rust daemon (`daemon-rs/`)
- Claude Code plugin (`plugins/cortex-plugin/`)
- Control Center desktop app (`desktop/cortex-control-center/`)
- MCP tools and proxy surfaces
