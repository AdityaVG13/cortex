# Cortex Startup Matrix and Troubleshooting

This guide is the release-facing startup truth for `v0.5.0`.

Core invariants:
- Local-first by default.
- Exactly one daemon per user profile.
- App, plugin, and CLI attach to the same daemon instead of spawning sidecar duplicates.

## Startup Matrix

| Surface | Normal startup behavior | Expected daemon result |
|---|---|---|
| Control Center app | Checks daemon health and attaches to existing process. If missing, uses service-managed ensure path. | Existing daemon reused, or one managed daemon started. |
| Claude plugin / MCP | Connect-only baseline. Uses existing daemon/service path; no silent sidecar fallback in strict mode. | Existing daemon reused. |
| CLI (`cortex boot`, `cortex recall`, etc.) | Connects to running daemon on resolved runtime URL. | Existing daemon reused. |
| Fresh machine first run | `cortex setup` then first attach/serve initializes runtime files/token/profile. | One daemon created; subsequent clients attach. |
| Team mode remote client | Connects to configured remote URL and API key; no local sidecar required for remote target. | No local duplicate daemon spawned for remote-only usage. |

## Quick Verification

1. Check liveness:
   ```bash
   curl http://127.0.0.1:7437/health
   ```
2. Check readiness:
   ```bash
   curl http://127.0.0.1:7437/readiness
   ```
3. Confirm spawn-path policy (repo/developer check):
   ```bash
   python tools/audit_spawn_paths.py --strict
   ```

## Startup Troubleshooting

### 1) `connection refused`

Cause:
- Daemon is not running on the expected address/port.

Fix:
- Start daemon:
  ```bash
  cortex serve
  ```
- Verify health/readiness as above.
- If using custom URL/port, ensure the same value is configured in your client.

### 2) `401 unauthorized` or auth failures

Cause:
- Missing/incorrect API key, wrong user key, or stale key in client settings.

Fix:
- Re-enter the personal `ctx_...` key.
- Verify you are targeting the intended Cortex instance.

### 3) App says running but tools cannot recall

Cause:
- Service attached before readiness, or client targeting stale URL.

Fix:
- Check `/readiness` is `ready=true`.
- Restart client session (MCP clients usually hot-load servers only at session start).

### 4) Multiple daemon suspicion

Cause:
- Historical stale process residue or old workflow assumptions.

Fix:
- Run supported cleanup:
  ```bash
  cortex cleanup
  ```
- Recheck health/readiness and reconnect clients.

### 5) Team mode remote safety concerns

Cause:
- Non-loopback bind without clear transport security boundary.

Fix:
- Prefer Tailscale/WireGuard private mesh or TLS reverse proxy/tunnel.
- Never expose raw `0.0.0.0:7437` directly to the public internet.

## Notes for Operators

- `health` is liveness/diagnostic.
- `readiness` is startup-gate truth for clients.
- For policy/security details, also read:
  - `Info/team-mode-setup.md`
  - `docs/compatibility/03-security-model.md`
