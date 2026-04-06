# Team Mode Setup

Team mode gives multiple developers a shared Cortex brain with per-user ownership and visibility controls.

## 1) Server Setup

Choose one:

### Option A — Direct binary

```bash
chmod +x cortex
CORTEX_BIND=0.0.0.0 ./cortex serve
```

### Option B — Docker

```bash
docker run -d -p 7437:7437 \
  -v cortex_data:/root/.cortex \
  -e CORTEX_BIND=0.0.0.0 \
  adityavg13/cortex:latest
```

### Option C — Build from source

```bash
git clone https://github.com/AdityaVG13/cortex.git
cd cortex/daemon-rs
cargo build --release
CORTEX_BIND=0.0.0.0 ./target/release/cortex serve
```

## 2) Initialize Team Mode

Run once on the server:

```bash
./cortex setup --team
```

Create member credentials with:

```bash
./cortex user add alice --role member --display-name "Alice"
```

Save the generated `ctx_...` API key for each member.

## 3) Member Onboarding (3 steps)

1. Install plugin:
   ```bash
   claude plugin marketplace add AdityaVG13/cortex
   claude plugin install cortex@cortex-marketplace
   ```
2. Restart Claude Code.
3. Enter team server URL + personal API key when prompted.

## Troubleshooting

- **Connection refused:** verify server is running and reachable on port `7437`, and firewall/VPN rules allow access.
- **Authentication failed:** confirm the key is correct and includes the `ctx_` prefix.
- **Cannot see teammate memories:** verify entries are stored with team visibility and that users are in the same team-mode instance.
