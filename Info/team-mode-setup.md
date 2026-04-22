<p align="center"><a href="../README.md">← Back to README</a></p>

# Team Mode Setup

> Give multiple developers a shared Cortex brain with per-user ownership and visibility controls.

---

> **Read before setup:** Keep Cortex on `127.0.0.1` by default. For remote teammates, use an encrypted private network (Tailscale / WireGuard) or a TLS reverse proxy. Never expose a raw `0.0.0.0:7437` listener to the internet.

---

## Step 1 — Start the server

Choose one deployment pattern:

<details open>
<summary><b>Option A — Localhost only</b> (recommended base)</summary>

```bash
chmod +x cortex
./cortex serve
```

Safest default. Works for local app/plugin workflows.

</details>

<details>
<summary><b>Option B — Private mesh</b> (Tailscale / WireGuard)</summary>

```bash
TAILSCALE_IP=$(tailscale ip -4)
CORTEX_BIND=$TAILSCALE_IP ./cortex serve
```

Use the private mesh address as your team URL (e.g., `http://100.x.y.z:7437`).

</details>

<details>
<summary><b>Option C — TLS reverse proxy</b></summary>

```bash
CORTEX_BIND=127.0.0.1 ./cortex serve
```

Terminate TLS at your gateway (Caddy, Nginx, Cloudflare Tunnel). Publish only the TLS endpoint.

</details>

<details>
<summary><b>Option D — Docker</b></summary>

```bash
docker run -d -p 127.0.0.1:7437:7437 \
  -v cortex_data:/root/.cortex \
  -e CORTEX_BIND=0.0.0.0 \
  cortex-project/cortex:latest
```

Container `0.0.0.0` is fine because the host port is on `127.0.0.1`. Do not use this with a public host bind.

</details>

<details>
<summary><b>Option E — Build from source</b></summary>

```bash
git clone https://github.com/AdityaVG13/cortex.git
cd cortex/daemon-rs
cargo build --release
./target/release/cortex serve
```

</details>

---

## Step 2 — Initialize team mode

Run once on the server:

```bash
./cortex setup --team
```

Create member credentials:

```bash
./cortex user add alice --role member --display-name "Alice"
```

Save the generated `ctx_...` API key for each member.

---

## Step 3 — Member onboarding

**a)** Install plugin:

```bash
claude plugin marketplace add AdityaVG13/cortex
claude plugin install cortex@cortex-marketplace
```

**b)** Restart Claude Code.

**c)** Enter team server URL + personal API key when prompted.

---

## Troubleshooting

| Problem | Fix |
|---------|-----|
| **Connection refused** | Verify daemon is running, host/port reachable, firewall/VPN rules allow access. |
| **Authentication failed** | Confirm key includes `ctx_` prefix and matches the assigned user. |
| **Can't see teammate memories** | Verify entries stored with team visibility, both users on same instance. |
| **Remote deployment safety** | Non-loopback bind requires transport encryption (VPN/mesh or TLS gateway). |
