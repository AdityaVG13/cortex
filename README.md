<p align="center"><strong>Private local memory for your tools.</strong></p>

<p align="center">
  <img src="assets/cortex-header.gif" alt="Cortex" width="100%">
</p>

<h1 align="center">One shared memory.<br>Every tool you use.</h1>
<p align="center">
  Install Cortex once.
  Your tools stop starting from scratch.
</p>

<p align="center">
  <a href="https://github.com/AdityaVG13/cortex/releases/latest">Get started</a> |
  <a href="Info/connecting.md">Connect your tools</a>
</p>

<p align="center">
  <sub>Private by default &nbsp;&nbsp;|&nbsp;&nbsp; HTTP + MCP &nbsp;&nbsp;|&nbsp;&nbsp; MIT</sub>
</p>

<table align="center">
  <tr>
    <td width="250" align="center"><strong>Private by default</strong><br><sub>your data stays on your machine</sub></td>
    <td width="250" align="center"><strong>One memory across tools</strong><br><sub>HTTP or MCP, same Cortex memory</sub></td>
    <td width="250" align="center"><strong>Real proof below</strong><br><sub>benchmarks, live analytics, and a usage-based forecast</sub></td>
  </tr>
</table>

<p align="center">
  <sub><a href="https://github.com/sponsors">Support Cortex</a> helps fund releases, benchmarks, and long-term maintenance.</sub>
</p>

## Start in two commands

If you want the quickest path from install to a working setup, start with the Claude Code plugin. Cortex handles the background setup for you.

```bash
claude plugin marketplace add AdityaVG13/cortex
claude plugin install cortex@cortex-marketplace
```

Restart your session and Cortex comes up on its own. Prefer an installer or a source build? Jump to [more install options](#more-install-options).

## See the payoff

Memory tools are easy to pitch and hard to trust. Cortex starts to matter when the savings stop looking theoretical.

<p align="center">
  <img src="assets/control-center-analytics.png" alt="Cortex Control Center analytics showing token savings, compression, recall hit rate, and agent activity" width="100%">
</p>

<p align="center"><em>Example Cortex analytics from a real active install: savings, compression, recall quality, startup history, and activity in one place.</em></p>

If Cortex is helping, you should be able to see it. If it is not, you should know that just as quickly.

<p align="center">
  <img src="assets/monte-carlo-readme.png" alt="Monte Carlo projection showing a 30-day Cortex savings horizon" width="100%">
</p>

<p align="center"><em>Monte Carlo savings horizon: an example 30-day projection based on one real live Cortex dataset.</em></p>

Source notes: the screenshot above is one real active Cortex install. The Monte Carlo chart is based on one maintainer-run Cortex history. Benchmark figures come from [`benchmark/baseline-v041.md`](benchmark/baseline-v041.md).

## Why teams keep it running

Cortex is built for the part after the demo, when the novelty wears off and the repetition gets old.

- Claude Code, Codex, Cursor, Gemini, and your own scripts can all use the same memory.
- New sessions start lighter because Cortex brings back the useful parts instead of replaying everything.
- Decisions, fixes, and project rules stay easy to pull back up instead of disappearing into old chats.
- The Control Center keeps savings, recall quality, and activity out in the open.

## Works with your stack

Cortex is meant to fit into the tools you already use, not make you learn a whole new workflow just to keep context around.

- Claude Code. The easiest setup path, with lifecycle handled for you.
- Codex. Native MCP support, plus HTTP fallback when you need it.
- Cursor, Gemini, and other MCP-capable tools. Point them at the same Cortex memory instead of giving each one its own silo.
- Any AI client that can call HTTP APIs.
- Local LLMs and custom tooling. Use HTTP or MCP from your own app, desktop tool, orchestration layer, or runtime.
- Team mode. Run one shared memory service for a whole engineering team when one machine is no longer enough.

## More install options

### Desktop app

Download the latest installer from the [release page](https://github.com/AdityaVG13/cortex/releases/latest).

<details>
<summary>Desktop installers and daemon archives</summary>

| Platform | Installer | Daemon only |
|---|---|---|
| Windows | [`.exe` (NSIS installer)](https://github.com/AdityaVG13/cortex/releases/latest) | [Daemon archive (`.zip`)](https://github.com/AdityaVG13/cortex/releases/latest) |
| macOS | [`.dmg`](https://github.com/AdityaVG13/cortex/releases/latest) | [Daemon archive (`.tar.gz`)](https://github.com/AdityaVG13/cortex/releases/latest) |
| Linux | [`.AppImage` / `.deb`](https://github.com/AdityaVG13/cortex/releases/latest) | [Daemon archive (`.tar.gz`)](https://github.com/AdityaVG13/cortex/releases/latest) |

</details>

### From source

```bash
git clone https://github.com/AdityaVG13/cortex.git
cd cortex/daemon-rs
cargo build --release
```

When Cortex boots cleanly, you should see a READY message and an active memory count. After that, the basic loop is simple: save something once, stop explaining it over and over.

## What ships in the box

You do not need a giant platform rewrite to get value from Cortex. The useful parts show up quickly.

- Smaller startup prompts. Cortex brings back the useful context instead of replaying raw history.
- Faster recall. Keyword and semantic search work together so the right memory shows up sooner.
- Flexible connections. Apps, scripts, and coding tools can all talk to the same memory through MCP or HTTP.
- Local embeddings. `all-MiniLM-L12-v2` runs in-process through ONNX, with no outside inference service required.
- Memory controls. Decay, supersession, and conflict handling keep the memory base from turning into a junk drawer.
- Control Center. One place to see health, savings, activity, and what Cortex is doing.

## Built in public, backed by research

Cortex is open about where ideas came from and how they changed once they hit real code. The research page shows what influenced Cortex, what changed in implementation, what shipped, and what is still on the roadmap.

- **ByteRover.** Helped shape progressive retrieval and the longer-term memory-tier model.
- **Reciprocal Rank Fusion.** Provides the ranking fusion rule behind the current retrieval stack.
- **Memori.** Informs the planned move toward stronger semantic structure and dedup.
- **A-MAC, MemoryOS, FluxMem.** Push the roadmap toward admission control, maturity tiers, and memory crystallization.

Full paper list, adaptation notes, and status tracking: [Info/research.md](Info/research.md)

## Documentation

The docs are organized around what you are trying to do, not around internal folder names.

- Connect Cortex. [Info/connecting.md](Info/connecting.md) covers setup, MCP, HTTP, auth, and troubleshooting.
- Research and roadmap. [Info/research.md](Info/research.md) and [Info/roadmap.md](Info/roadmap.md) show what shipped, what is planned, and why.
- Security and contribution. [Info/security-rules.md](Info/security-rules.md), [CONTRIBUTING.md](CONTRIBUTING.md), and [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) cover trust, reporting, and project standards.
- Team deployment. [Info/team-mode-setup.md](Info/team-mode-setup.md) covers the shared-server setup when one machine is no longer enough.

<details>
<summary>Open the docs map and CLI reference</summary>

### Docs map

- [README.md](README.md) - product overview and install path
- [Info/connecting.md](Info/connecting.md) - AI and tool integration quickstart
- [Info/mcp-tools.md](Info/mcp-tools.md) - MCP tool list and parameters
- [Info/research.md](Info/research.md) - papers, inspirations, and Cortex adaptation notes
- [Info/roadmap.md](Info/roadmap.md) - public roadmap
- [Info/team-mode-setup.md](Info/team-mode-setup.md) - shared team-memory setup
- [Info/security-rules.md](Info/security-rules.md) - security posture and reporting

### CLI reference

| Command | Description |
|---|---|
| `cortex serve` | Start the Cortex daemon |
| `cortex --help` | Show command reference plus troubleshooting guidance |
| `cortex doctor` | Run integrity and configuration diagnostics |
| `cortex paths --json` | Output canonical file and port paths |
| `cortex plugin ensure-daemon` | Ensure daemon health in plugin mode (service-first on Windows) and print its port |
| `cortex plugin mcp` | Bridge MCP stdio to the Cortex HTTP API (plugin local mode uses service-first daemon ensure) |
| `cortex setup --team` | Initialize team mode and generate API keys |
| `cortex export` | Export data in `json` or `sql` format |
| `cortex import` | Import a JSON export into solo or team mode |

</details>

## Security and roadmap

- Cortex defaults to localhost-only access with bearer-token auth. The token lives under `~/.cortex/cortex.token`.
- The v0.5.0 direction is stronger retrieval, better storage rules, public research traceability, and a cleaner operator experience.
- Longer-term work includes smarter memory admission, maturity tiers, shared multi-agent memory, and adaptive compression.

Roadmap details: [Info/roadmap.md](Info/roadmap.md)

<p align="center">
  <a href="https://github.com/sponsors">Support Cortex</a> |
  <a href="Info/research.md">Research</a> |
  <a href="Info/connecting.md">Connecting</a> |
  <a href="Info/security-rules.md">Security</a> |
  <a href="CONTRIBUTING.md">Contributing</a> |
  <a href="CODE_OF_CONDUCT.md">Code of Conduct</a> |
  <a href="CHANGELOG.md">Changelog</a> |
  <a href="LICENSE">License</a>
</p>
