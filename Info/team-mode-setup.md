# Team Mode Setup Guide

Cortex Team Mode allows multiple developers to share a persistent brain. This ensures that architecture decisions, project conventions, and debugging lessons learned by one team member's agent are available to every other agent on the team.

## Prerequisites

-   A server with network access for the team (can be local, on-prem, or cloud).
-   Docker installed (optional, for containerized deployment).
-   Rust and Node.js (for building from source).

## 1. Server Setup

Choose one of the three options below to set up the Cortex daemon on your shared server.

### Option A: Direct Binary (Recommended)

Download the latest `cortex` binary for your server's platform from the [GitHub Releases](https://github.com/AdityaVG13/cortex/releases) page.

```bash
chmod +x cortex
./cortex serve --host 0.0.0.0
```

### Option B: Docker

Run the Cortex daemon in a Docker container with one command:

```bash
docker run -d -p 7437:7437 \
  -v cortex_data:/root/.cortex \
  -e CORTEX_BIND=0.0.0.0 \
  adityavg13/cortex:latest
```

### Option C: Build from Source

```bash
git clone https://github.com/AdityaVG13/cortex.git
cd cortex/daemon-rs
cargo build --release
./target/release/cortex serve --host 0.0.0.0
```

## 2. Initialize the Team

Once the daemon is running, the team lead must initialize team mode and generate API keys for each member.

```bash
./cortex setup --team
```

This will:
1.  Initialize the database for team mode.
2.  Provide the server's API key generation command.
3.  Allow you to create API keys for your team members.

To generate an API key for a member:
```bash
./cortex team add-member --name "Alice"
# Output: alice_ctx_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx
```

Each member needs their unique API key and the server's public URL (e.g., `https://cortex.yourteam.com:7437`).

## 3. Member Onboarding (3 Steps)

Each team member should follow these steps to connect their local agents to the shared brain:

1.  **Install the Claude Code plugin:**
    ```bash
    claude plugin marketplace add AdityaVG13/cortex
    claude plugin install cortex@cortex-marketplace
    ```
2.  **Restart Claude Code.**
3.  **Enter the server URL and API key** when prompted by the plugin's `userConfig` step.

Cortex will now handle everything. Every session started by Alice will contribute to the shared team memory.

## What Teams Get

-   **Shared Context:** Your agent knows about the architectural decisions Alice's agent made yesterday.
-   **Attributed Stores:** Every memory is tagged with the `owner_id` of the member who created it.
-   **Visibility Controls:** Use the `cortex_store` tool to set visibility to `team` (default) or `private`.
-   **Cross-Tool Persistence:** The shared brain works across Claude Code, Cursor, and any tool the team members use.

## Security Considerations

-   **AGPL-3.0 License:** Cortex is open-source. Ensure your organization's legal policies allow for AGPL-3.0 software.
-   **API Key Management:** API keys use Argon2id hashing for secure storage on the server.
-   **Network Security:** We strongly recommend running the Cortex server behind a reverse proxy (like Nginx or Caddy) with TLS enabled to encrypt communication.
-   **Data at Rest:** SQLite data is unencrypted by default. For sensitive environments, ensure the server's storage is encrypted.

## Troubleshooting

-   **"Connection refused":** Ensure the server's firewall allows traffic on port 7437 and that the daemon is bound to `0.0.0.0`.
-   **"Authentication failed":** Double-check that the API key is entered correctly and includes the `ctx_` prefix.
-   **"Can't see teammate's memories":** Ensure teammates are storing memories with `visibility: team` (the default).

Need help? Join our [Discord community](https://github.com/AdityaVG13/cortex#community) or open an issue on GitHub.
