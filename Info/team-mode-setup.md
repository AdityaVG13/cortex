# Team Mode Setup

Team mode gives multiple developers a shared Cortex brain with per-user ownership and visibility controls.

## Security First (Read Before Setup)

- Keep Cortex on `127.0.0.1` by default.
- For remote teammates, use an encrypted private network (Tailscale/WireGuard) or a TLS reverse proxy/tunnel.
- Never publish a raw `0.0.0.0:7437` listener directly to the internet.

## 1) Server Setup

Choose one deployment pattern.

### Option A — Localhost-only daemon (recommended base)

```bash
chmod +x cortex
./cortex serve
```

This is the safest default and works for local app/plugin workflows.

### Option B — Team access over private mesh (Tailscale/WireGuard)

```bash
TAILSCALE_IP=$(tailscale ip -4)
CORTEX_BIND=$TAILSCALE_IP ./cortex serve
```

Use the private mesh address as your team server URL (for example `http://100.x.y.z:7437`).

### Option C — Local daemon behind TLS reverse proxy

```bash
CORTEX_BIND=127.0.0.1 ./cortex serve
```

Terminate TLS at your gateway (Caddy, Nginx, Cloudflare Tunnel) and publish only the TLS endpoint.

### Option D — Docker (localhost publish)

```bash
docker run -d -p 127.0.0.1:7437:7437 \
  -v cortex_data:/root/.cortex \
  -e CORTEX_BIND=0.0.0.0 \
  cortex-project/cortex:latest
```

Container-only `0.0.0.0` is acceptable because the host port is published on `127.0.0.1`; do not use this pattern with a public host bind.

### Option E — Build from source

```bash
git clone https://github.com/cortex-project/cortex.git
cd cortex/daemon-rs
cargo build --release
./target/release/cortex serve
```

## 2) Initialize Team Mode

Run once on the server:

```bash
./cortex setup --team
```

Create member credentials:

```bash
./cortex user add alice --role member --display-name "Alice"
```

Save the generated `ctx_...` API key for each member.

## 3) Member Onboarding

1. Install plugin:
   ```bash
   claude plugin marketplace add cortex-project/cortex
   claude plugin install cortex@cortex-marketplace
   ```
2. Restart Claude Code.
3. Enter team server URL + personal API key when prompted.

## Troubleshooting

- **Connection refused:** verify daemon is running, host/port are reachable, and firewall/VPN rules allow access.
- **Authentication failed:** confirm key includes the `ctx_` prefix and matches the assigned user.
- **Cannot see teammate memories:** verify entries were stored with team visibility and both users target the same team-mode instance.
- **Remote deployment safety:** if using non-loopback bind, ensure transport encryption is provided by your VPN/mesh or TLS gateway.
