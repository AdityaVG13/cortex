<p align="center">
  <img src="assets/cortex-header.gif" alt="Cortex" width="100%">
</p>

<h2 align="center">One local memory system for every coding agent you use.</h2>
<p align="center">
  Cortex gives Claude Code, Codex, Cursor, Gemini, and local LLM workflows a shared brain that survives restarts,
  compresses boot context, and stays on your machine.
</p>

<p align="center">
  <a href="https://github.com/AdityaVG13/cortex/releases/latest">Install or download</a> |
  <a href="Info/connecting.md">Connect your tools</a> |
  <a href="Info/research.md">Read the research</a> |
  <a href="Info/roadmap.md">View the roadmap</a>
</p>

<p align="center">
  <a href="https://github.com/AdityaVG13/cortex/releases/tag/v0.4.1"><img src="https://img.shields.io/badge/release-v0.4.1-0f172a?style=for-the-badge" alt="Release v0.4.1"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-111827?style=for-the-badge" alt="License: MIT"></a>
  <a href="Info/connecting.md"><img src="https://img.shields.io/badge/local%20first-yes-00c2ff?style=for-the-badge" alt="Local first"></a>
  <a href="Info/mcp-tools.md"><img src="https://img.shields.io/badge/MCP%20native-yes-22c55e?style=for-the-badge" alt="MCP native"></a>
</p>

<p align="center">
  <a href="https://ko-fi.com/adityavg13"><strong>Support Cortex</strong></a> funds releases, benchmarks, app polish, and long-term maintenance.
</p>

<p align="center">
  <a href="#see-the-payoff">Proof</a> |
  <a href="#why-teams-keep-it-running">Why it works</a> |
  <a href="#works-with-your-stack">Stack</a> |
  <a href="#quickstart">Quickstart</a> |
  <a href="#what-ships-in-the-box">What ships</a> |
  <a href="#built-in-public-backed-by-research">Research</a> |
  <a href="#documentation">Docs</a> |
  <a href="#security-and-roadmap">Security</a>
</p>

<p align="center">
  <strong>Cortex exists for one reason:</strong> memory should feel like infrastructure, not a party trick.
</p>

<table>
  <tr>
    <td width="25%" align="center"><strong>10.7M</strong><br><sub>boot tokens saved</sub></td>
    <td width="25%" align="center"><strong>99%</strong><br><sub>live avg compression</sub></td>
    <td width="25%" align="center"><strong>90%</strong><br><sub>benchmark hit rate</sub></td>
    <td width="25%" align="center"><strong>97.5ms</strong><br><sub>avg recall latency</sub></td>
  </tr>
</table>

## Quickstart

If you want the shortest path from "that looks interesting" to "this is helping," use the Claude Code plugin. Cortex handles daemon lifecycle for you.

```bash
claude plugin marketplace add AdityaVG13/cortex
claude plugin install cortex@cortex-marketplace
```

Restart your session and Cortex boots itself. Prefer installers or a source build? Jump to [more install options](#more-install-options).

## See the payoff

Memory products are easy to demo and hard to trust. Cortex only gets interesting once the savings show up on screen.

<p align="center">
  <img src="assets/control-center-analytics.png" alt="Cortex Control Center analytics showing token savings, compression, recall hit rate, and agent activity" width="100%">
</p>

<p align="center"><em>Live Cortex analytics: saved tokens, compression, recall quality, boot history, and agent activity in one operator surface.</em></p>

The Control Center is there to answer the uncomfortable question fast: "Is this thing actually paying for itself?" If the answer is no, you should know that immediately. If the answer is yes, the page makes it obvious.

<p align="center">
  <img src="assets/monte-carlo-readme.gif" alt="Animated Monte Carlo projection showing a 30-day Cortex savings horizon" width="100%">
</p>

<p align="center"><em>Monte Carlo savings horizon: a restrained 30-day projection built from real Cortex benchmark data, not marketing math.</em></p>

Source notes: live savings and compression figures come from the current Control Center surface. Retrieval metrics come from [`benchmark/baseline-v041.md`](benchmark/baseline-v041.md).

## Why teams keep it running

| Without Cortex | With Cortex |
|---|---|
| Every agent has to relearn the repo from scratch. | One local daemon serves the same memory to every tool. |
| Boot prompts keep swelling as projects get older. | `cortex_boot` compiles a smaller prompt from stable memory plus recent delta. |
| Good decisions vanish into chat logs and terminal scrollback. | Decisions, rules, and bug fixes stay queryable through MCP and HTTP. |
| It is hard to tell whether "memory" is helping or just adding complexity. | The Control Center shows savings, recall quality, and activity in one place. |

## Works with your stack

| Stack | How Cortex fits |
|---|---|
| Claude Code | Primary plugin path with lifecycle handled for you. |
| Codex | Native MCP bridge plus HTTP fallback when you need it. |
| Cursor | Shared local memory through the same daemon instead of a separate silo. |
| Gemini | Works through MCP for CLI and tool-driven workflows. |
| Local LLMs | Use HTTP or MCP from your own orchestration stack, desktop app, or agent runtime. |

## More install options

### Desktop app

Download the latest installer from the [release page](https://github.com/AdityaVG13/cortex/releases/latest).

<details>
<summary>Desktop installers and daemon archives</summary>

| Platform | Installer | Daemon only |
|---|---|---|
| Windows | [`.exe` (NSIS installer)](https://github.com/AdityaVG13/cortex/releases/latest) | [`cortex-v0.4.1-windows-x86_64.zip`](https://github.com/AdityaVG13/cortex/releases/download/v0.4.1/cortex-v0.4.1-windows-x86_64.zip) |
| macOS | [`.dmg`](https://github.com/AdityaVG13/cortex/releases/latest) | [`cortex-v0.4.1-macos-aarch64.tar.gz`](https://github.com/AdityaVG13/cortex/releases/download/v0.4.1/cortex-v0.4.1-macos-aarch64.tar.gz) |
| Linux | [`.AppImage` / `.deb`](https://github.com/AdityaVG13/cortex/releases/latest) | [`cortex-v0.4.1-linux-x86_64.tar.gz`](https://github.com/AdityaVG13/cortex/releases/download/v0.4.1/cortex-v0.4.1-linux-x86_64.tar.gz) |

</details>

### From source

```bash
git clone https://github.com/AdityaVG13/cortex.git
cd cortex/daemon-rs
cargo build --release
```

When Cortex boots cleanly, you should see a READY message and an active memory count. From there, the workflow is simple: store a decision once, stop re-explaining it later.

## What ships in the box

Cortex does not ask you to buy into some giant platform shift on day one. The useful parts land quickly:

- **Capsule compiler.** Builds boot prompts from stable identity plus recent delta instead of replaying raw context.
- **Hybrid retrieval.** Blends keyword, semantic, and fused ranking locally so useful memory rises faster.
- **MCP and HTTP surfaces.** Lets coding agents, local apps, scripts, and orchestration layers all talk to the same memory system.
- **Local embeddings.** Runs `all-MiniLM-L6-v2` in-process through ONNX, with no external inference requirement.
- **Governance and conflict handling.** Supports decay, supersession, dispute handling, and future provenance-aware memory work.
- **Control Center.** Gives operators a visual surface for health, savings, activity, and memory-system behavior.

## Built in public, backed by research

Cortex is open about where ideas came from and where they changed shape. The research page is not a citation dump. It spells out what looked promising, what Cortex adapted, what shipped, and what is still waiting on the roadmap.

- **ByteRover.** Helped shape progressive retrieval and the longer-term memory-tier model.
- **Reciprocal Rank Fusion.** Provides the ranking fusion rule behind the current retrieval stack.
- **Memori.** Informs the planned move toward stronger semantic structure and dedup.
- **A-MAC, MemoryOS, FluxMem.** Push the roadmap toward admission control, maturity tiers, and memory crystallization.

Full paper list, adaptation notes, and status tracking: [Info/research.md](Info/research.md)

## Documentation

- **Connect Cortex.** [Info/connecting.md](Info/connecting.md) covers Claude Code, Codex, Cursor, Gemini, MCP, HTTP, auth, and the troubleshooting path.
- **Research and roadmap.** [Info/research.md](Info/research.md) and [Info/roadmap.md](Info/roadmap.md) show what shipped, what is planned, and why.
- **Security and contribution.** [security/SECURITY.md](security/SECURITY.md), [CONTRIBUTING.md](CONTRIBUTING.md), and [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) cover trust, reporting, and project standards.

<details>
<summary>Open the docs map and CLI reference</summary>

### Docs map

- [README.md](README.md) - product overview and install path
- [Info/connecting.md](Info/connecting.md) - AI and tool integration quickstart
- [Info/mcp-tools.md](Info/mcp-tools.md) - MCP tool list and parameters
- [Info/research.md](Info/research.md) - papers, inspirations, and Cortex adaptation notes
- [Info/roadmap.md](Info/roadmap.md) - public roadmap
- [Info/team-mode-setup.md](Info/team-mode-setup.md) - shared team-memory setup
- [security/SECURITY.md](security/SECURITY.md) - security posture and reporting

### CLI reference

| Command | Description |
|---|---|
| `cortex serve` | Start the Cortex daemon |
| `cortex --help` | Show command reference plus troubleshooting guidance |
| `cortex doctor` | Run integrity and configuration diagnostics |
| `cortex paths --json` | Output canonical file and port paths |
| `cortex plugin ensure-daemon` | Start or reuse the daemon with migration and lock safety |
| `cortex plugin mcp` | Bridge MCP stdio to the Cortex HTTP API |
| `cortex setup --team` | Initialize team mode and generate API keys |
| `cortex export` | Export data in `json` or `sql` format |
| `cortex import` | Import a JSON export into solo or team mode |

</details>

## Security and roadmap

- Cortex defaults to a localhost-only surface and bearer-token auth. The token lives under `~/.cortex/cortex.token`.
- The v0.5.0 direction is retrieval hardening, storage governance, public research traceability, and sharper operator surfaces.
- Longer-term work includes admission control, maturity tiers, provenance-aware multi-agent memory, and adaptive compression.

Roadmap details: [Info/roadmap.md](Info/roadmap.md)

<p align="center">
  <a href="https://ko-fi.com/adityavg13">Support Cortex</a> |
  <a href="Info/research.md">Research</a> |
  <a href="Info/connecting.md">Connecting</a> |
  <a href="security/SECURITY.md">Security</a> |
  <a href="CONTRIBUTING.md">Contributing</a> |
  <a href="CODE_OF_CONDUCT.md">Code of Conduct</a> |
  <a href="CHANGELOG.md">Changelog</a> |
  <a href="LICENSE">License</a>
</p>
