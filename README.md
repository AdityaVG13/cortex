<p align="center">
  <img src="assets/cortex-header.gif" alt="Cortex" width="100%">
</p>

<h1 align="center">Cortex</h1>
<p align="center"><b>Private local memory for your AI tools.</b><br>
Install once. Your tools stop starting from scratch.</p>

<p align="center">
  <a href="https://ko-fi.com/adityavg13">
    <img src="https://img.shields.io/badge/☕_Ko--fi-Donations_help_support_Cortex_development-FF5E5B?style=for-the-badge&logo=ko-fi&logoColor=white" alt="Support Cortex on Ko-fi">
  </a>
</p>

<p align="center">
  <a href="CHANGELOG.md#060---2026-06-06"><img src="https://img.shields.io/badge/release-0.6.0-blue?style=flat-square" alt="release 0.6.0"></a>&nbsp;
  <a href="LICENSE"><img src="https://img.shields.io/github/license/AdityaVG13/cortex?style=flat-square" alt="MIT License"></a>&nbsp;
  <img src="https://img.shields.io/badge/platforms-Windows_|_macOS_|_Linux-333?style=flat-square" alt="Windows | macOS | Linux">&nbsp;
  <img src="https://img.shields.io/badge/Rust_+_React-daemon_+_desktop-orange?style=flat-square" alt="Rust + React">
</p>

<p align="center">
  <a href="https://github.com/AdityaVG13/cortex/releases/latest">Download</a>&nbsp;&nbsp;·&nbsp;&nbsp;
  <a href="Info/connecting.md">Connect your tools</a>&nbsp;&nbsp;·&nbsp;&nbsp;
  <a href="ARCHITECTURE.md">Architecture</a>&nbsp;&nbsp;·&nbsp;&nbsp;
  <a href="CHANGELOG.md">What's new</a>&nbsp;&nbsp;·&nbsp;&nbsp;
  <a href="Info/roadmap.md">Roadmap</a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/works_with-Claude_Code-6B4FBB?style=flat-square&logo=anthropic&logoColor=white">&nbsp;&nbsp;
  <img src="https://img.shields.io/badge/works_with-Codex-412991?style=flat-square&logo=openai&logoColor=white">&nbsp;&nbsp;
  <img src="https://img.shields.io/badge/works_with-Cursor-000?style=flat-square&logo=cursor&logoColor=white">&nbsp;&nbsp;
  <img src="https://img.shields.io/badge/works_with-Factory_Droid-F97316?style=flat-square">&nbsp;&nbsp;
  <img src="https://img.shields.io/badge/works_with-MCP_·_HTTP-58a6ff?style=flat-square">
</p>

---

<p align="center">
  🔒 <b>Private by default</b>: localhost only, data never leaves your machine<br>
  🔗 <b>One memory, every tool</b>: HTTP and MCP, same brain, no per-tool silos<br>
  📊 <b>Prove it works</b>: token savings, recall quality, and Monte Carlo projections
</p>

---

## Quick Start

Get to the first memory moment before learning daemon internals.

### 1. Install or build Cortex

Use the latest desktop installer, or build the `0.6.0` source CLI:

```bash
git clone https://github.com/AdityaVG13/cortex.git
cd cortex
cargo build -p cortex-daemon --release
```

### 2. Start local memory

Open Cortex Control Center and start Cortex from the app. CLI-only users can run:

```bash
cortex serve
```

### 3. Check readiness

```bash
cortex status --json
```

Success is `"status": "ready"`. If status is `needs_action` or `error`, follow the returned `nextAction` / `repair` before continuing.

### 4. Connect one AI tool

Claude Code:

```bash
claude plugin marketplace add AdityaVG13/cortex
claude plugin install cortex@cortex-marketplace
```

Codex:

```bash
codex mcp add cortex -- cortex.exe mcp --agent codex
```

Restart the AI tool after changing MCP config.

### 5. Store and recall one memory

From a connected MCP client, call `cortex_store`, then `cortex_recall`. From the repo, run the matching smoke script:

Windows:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\first-run-smoke.ps1
```

macOS / Linux:

```bash
bash tests/scripts/first-run-smoke.sh
```

That smoke checks status, stores one disposable local memory, and recalls it. Normal use does not require benchmark adapters, provider keys, or LongMemEval.

More tool-specific setup: [Info/connecting.md](Info/connecting.md).

---

![](https://capsule-render.vercel.app/api?type=waving&color=0:6B4FBB,80:4a2d8a,100:1a1030&height=110&text=Before%20/%20After&fontSize=36&fontColor=ffffff&fontAlign=50&fontAlignY=35)
<table>
<tr>
<td align="center" valign="top">
<img width="400" height="1" src="https://raw.githubusercontent.com/AdityaVG13/cortex/main/assets/spacer.png"><br>
<h3>❌ Without Cortex</h3>
<p>
Session 1 &nbsp;→&nbsp; explain preferences<br>
Session 2 &nbsp;→&nbsp; explain them again<br>
Session 3 &nbsp;→&nbsp; and again, new tool<br>
Session 14 &nbsp;→&nbsp; still explaining<br><br>
<b>~15,000 tokens wasted</b>
</p>
<br>
</td>
<td align="center" valign="top">
<img width="400" height="1" src="https://raw.githubusercontent.com/AdityaVG13/cortex/main/assets/spacer.png"><br>
<h3>✅ With Cortex</h3>
<p>
Session 1 &nbsp;→&nbsp; store once<br>
Session 2 &nbsp;→&nbsp; boot, already knows<br>
Session 3 &nbsp;→&nbsp; boot, already knows<br>
Session 14 &nbsp;→&nbsp; boot, still knows<br><br>
<b>~300 tokens per boot (97% less)</b>
</p>
<br>
</td>
</tr>
</table>

---

![](https://capsule-render.vercel.app/api?type=waving&color=0:4a2d8a,80:6B4FBB,100:2d1b69&height=110&text=How%20It%20Works&fontSize=36&fontColor=ffffff&fontAlign=50&fontAlignY=35&reversal=true)

<p align="center">
  <img src="https://img.shields.io/badge/1-STORE-6B4FBB?style=for-the-badge" alt="Store">&nbsp;&nbsp;
  <img src="https://img.shields.io/badge/→-grey?style=for-the-badge" alt="→">&nbsp;&nbsp;
  <img src="https://img.shields.io/badge/2-RECALL-4a2d8a?style=for-the-badge" alt="Recall">&nbsp;&nbsp;
  <img src="https://img.shields.io/badge/→-grey?style=for-the-badge" alt="→">&nbsp;&nbsp;
  <img src="https://img.shields.io/badge/3-BOOT-8B5CF6?style=for-the-badge" alt="Boot">
</p>

<table align="center">
<tr>
<td align="center" width="33%">

**`POST /store`**

Save decisions, lessons, preferences. Conflict detection is automatic.

</td>
<td align="center" width="33%">

**`GET /recall`**

Clock-Quorum Recall: admit a stored row when a hard anchor matches, two clocks agree, or a strong lexical hit holds. Empty is a valid answer. Use `/as-of` for an explicit validity time.

</td>
<td align="center" width="33%">

**`GET /boot`**

Extractive identity + delta + current-truth pack. ~300 tokens served instead of ~15,000 raw. No summarizer.

</td>
</tr>
</table>

---

![](https://capsule-render.vercel.app/api?type=waving&color=0:4a2d8a,80:6B4FBB,100:2d1b69&height=110&text=Savings&fontSize=36&fontColor=ffffff&fontAlign=50&fontAlignY=35&reversal=true)
<p align="center">Memory tools are easy to pitch and hard to trust. Cortex starts to matter when the savings stop looking theoretical.</p>

<table>
<tr>
<td width="50%">

<p align="center"><b>📊 Analytics</b></p>
<img src="assets/grid-control-center-analytics.png" width="100%">
<p align="center"><sub>Savings, compression, and activity heatmaps</sub></p>

</td>
<td width="50%">

<p align="center"><b>📈 Monte Carlo</b></p>
<img src="assets/grid-cc-monte-carlo.png" width="100%">
<p align="center"><sub>30-day projection with confidence bands</sub></p>

</td>
</tr>
<tr>
<td width="50%">

<p align="center"><b>🤖 Agents</b></p>
<img src="assets/grid-cc-agents.png" width="100%">
<p align="center"><sub>Live sessions, inbox, deduped by identity</sub></p>

</td>
<td width="50%">

<p align="center"><b>🎛️ Overview</b></p>
<img src="assets/grid-cc-overview.png" width="100%">
<p align="center"><sub>Memory counts, health, and navigation</sub></p>

</td>
</tr>
</table>

---

![](https://capsule-render.vercel.app/api?type=waving&color=0:8B5CF6,70:5B21B6,100:2e1065&height=110&text=How%20Recall%20Works&fontSize=38&fontColor=ffffff&fontAlign=50&fontAlignY=35&reversal=true)
<p align="center">Cortex admits a memory. It does not guess a neighbor.</p>

<div align="center">

| Gate | Meaning |
|------|---------|
| **Hard anchor** | Path, symbol, alias, entity, or citation matches |
| **Two clocks** | Write, truth, task, and history are independent evidence |
| **Strong lexical** | Quoted phrase, stem, or closed-lexicon hit — not BM25 alone |
| **Empty** | No shared handle → no result. That is correct |

</div>

<p align="center">No local embedding or reranker model. <code>/recall/semantic</code> is the same engine under a compatibility name.<br>
LongMemEval quality claims are deferred. CQR is scored on honest miss, as-of windows, and determinism — see <a href="tests/contracts/clock_quorum.rs"><code>clock_quorum</code> contracts</a>.</p>

<details>
<summary>Historical v0.5.0 embedding-era numbers (not the current engine)</summary>

<p>These were measured against a 20-query set via a helper-augmented adapter while MiniLM was still on the hot path. They are not CQR scores and are not a v0.6 quality claim.</p>

<table align="center">
<tr>
<th></th>
<th align="center">v0.4.1</th>
<th align="center">v0.5.0</th>
<th align="center">Δ</th>
</tr>
<tr>
<td align="center"><b>Precision</b></td>
<td align="center">55.2%</td>
<td align="center">87.5%</td>
<td align="center">+32.3%</td>
</tr>
<tr>
<td align="center"><b>MRR</b></td>
<td align="center">69.2%</td>
<td align="center">95.0%</td>
<td align="center">+25.8%</td>
</tr>
<tr>
<td align="center"><b>Top-1 hit</b></td>
<td align="center">90.0%</td>
<td align="center">90.0%</td>
<td align="center">—</td>
</tr>
</table>

<p align="center">
<sub><a href="benchmarking/results/raw-recall-no-helper-dev-20260421-224217.json">Raw v0.5.0 JSON</a> · <a href="benchmarking/README.md">benchmarking/README.md</a></sub>
</p>
</details>

---

![](https://capsule-render.vercel.app/api?type=waving&color=0:5B3FA0,60:7B5FCC,100:1a1030&height=110&text=v0.6.0%20Improvements&fontSize=36&fontColor=ffffff&fontAlign=50&fontAlignY=35)
<p align="center">v0.6.0 makes settings, governance, boot audits, and recall-quality measurement first-class. Full details in <a href="CHANGELOG.md">CHANGELOG.md</a>.</p>

### Accessibility and settings

- **Settings panel**: Accessibility, Appearance & Motion, Connection, Budgets, and Keyboard & Navigation
- **Runtime preferences**: high contrast, reduced motion, keyboard hints, and compact navigation
- **Accessibility gates**: stronger focus states, ARIA/live regions, contrast checks, and 375px reflow checks

### Governance

- **Retention classes** across store, MCP, OpenAPI, export, and import
- **Local endpoint budgets** with stable HTTP `429` / JSON-RPC denial metadata
- **Budget UI** in Control Center, backed by the local `budgets.toml`
- **Boot audits** plus `GET /boot/audit` and the read-only `cortex_boot_audit` MCP tool
- **Admin rollback** with dry-run/apply workflow and audit events

### Recall quality

- **`cortex-http-pure` adapter** as the canonical helper-free measurement floor
- **Purity gates, CAS-100, and triangle judge tooling** for safer quality claims
- **Clock-Quorum Recall**: deterministic evidence from write, truth, task, and history clocks. No local embedding or reranker model.

### Reliability

- Claude plugin MCP is attach-only and no longer starts a second daemon from plugin MCP paths
- Control Center supervises the app-managed daemon and honors intentional stops
- Handler panics return JSON 500 responses, with local panic breadcrumbs
- Storage hygiene compacts FTS and keeps legacy embedding rows inert

---

![](https://capsule-render.vercel.app/api?type=waving&color=0:6B4FBB,50:3b2580,100:0d1117&height=110&text=Connected%20Agents&fontSize=36&fontColor=ffffff&fontAlign=50&fontAlignY=35&reversal=true)
<p align="center">Cortex tracks active agent sessions when clients identify themselves through <code>cortex_boot</code> or <code>GET /boot?agent=NAME</code>.</p>

<table>
<tr>
<td width="55%">

![Connected agents in Control Center](assets/cc-agents.png)

</td>
<td width="45%" valign="top">

### Multi-agent, one brain

- Each boot call registers a session. Control Center shows active sessions, **deduplicated by agent identity**.
- Read-path tools (recall, peek, unfold) reattach to existing sessions. No duplicates.
- Session descriptions preserved across reconnects and daemon restarts.
- What one agent stores, every other agent can recall.

Claude Code, Codex, Cursor, and custom scripts can all be connected simultaneously. Each tracks its own session while sharing the same memory.

</td>
</tr>
</table>

---

![](https://capsule-render.vercel.app/api?type=waving&color=0:4a2d8a,70:6B4FBB,100:2d1b69&height=110&text=Works%20With&fontSize=36&fontColor=ffffff&fontAlign=50&fontAlignY=35)
<div align="center">

| Tool | Connection | Setup |
|------|-----------|-------|
| **Claude Code** | MCP (plugin) or desktop app | Plugin: `claude plugin install cortex@cortex-marketplace` |
| **Codex** | MCP | `codex mcp add cortex -- cortex.exe mcp --agent codex` |
| **Cursor** | MCP | Point MCP server at `cortex mcp --agent cursor` |
| **Factory Droid** | MCP | `cortex mcp --agent droid` |
| **Aider** | CLI / HTTP | `cortex boot --agent aider` |
| **Custom tools** | HTTP | Three endpoints: `/boot`, `/recall`, `/store` |
| **Local LLMs** | HTTP / MCP | Same protocol, any runtime |

</div>

<p align="center">Full setup guide: <a href="Info/connecting.md"><b>Info/connecting.md</b></a></p>

---

![](https://capsule-render.vercel.app/api?type=waving&color=0:7C3AED,60:5B21B6,100:1e1040&height=110&text=Install&fontSize=36&fontColor=ffffff&fontAlign=50&fontAlignY=35&reversal=true)
<p align="center"><b>Desktop app (Control Center)</b><br>
Download from the <a href="https://github.com/AdityaVG13/cortex/releases/latest">latest tagged release page</a>. The Control Center manages daemon lifecycle for you.</p>

<div align="center">

| Platform | Desktop installer | Daemon archive |
|----------|------------------|----------------|
| **Windows** | [`.exe` (NSIS installer)](https://github.com/AdityaVG13/cortex/releases/latest) | [`.zip`](https://github.com/AdityaVG13/cortex/releases/latest) |
| **macOS** | [`.dmg`](https://github.com/AdityaVG13/cortex/releases/latest) | [`.tar.gz`](https://github.com/AdityaVG13/cortex/releases/latest) |
| **Linux** | [`.AppImage` / `.deb`](https://github.com/AdityaVG13/cortex/releases/latest) | [`.tar.gz`](https://github.com/AdityaVG13/cortex/releases/latest) |

</div>

<p align="center"><sub>Current release: <code>v0.6.0</code>.</sub></p>

<p align="center"><b>From source</b></p>

```bash
git clone https://github.com/AdityaVG13/cortex.git
cd cortex
cargo build -p cortex-daemon --release
```

<p align="center"><b>Claude Code plugin</b></p>

```bash
claude plugin marketplace add AdityaVG13/cortex
claude plugin install cortex@cortex-marketplace
```

<p align="center">The plugin attaches to a running Cortex runtime. If Cortex is not ready, it reports <code>APP_INIT_REQUIRED</code>; open Control Center or start the local runtime, then retry.</p>

---

![](https://capsule-render.vercel.app/api?type=waving&color=0:5B3FA0,80:4a2d8a,100:1a1030&height=110&text=Daemon%20Behavior&fontSize=36&fontColor=ffffff&fontAlign=50&fontAlignY=35)
<p align="center">Cortex enforces a <b>single-daemon invariant</b>: only one daemon process runs at a time.</p>

<div align="center">

| Mode | How it works |
|------|-------------|
| **Desktop app** | Control Center owns the daemon. Restart and monitor from the app. |
| **CLI** | `cortex serve` starts the daemon. Exits cleanly if one is already running. |
| **Plugin** | Attach-only MCP bridge. It connects to the running app/service daemon and does not silently spawn a second daemon. |

</div>

<p align="center">Default bind: <code>127.0.0.1:7437</code>. Non-loopback binds require TLS. Auth token at <code>~/.cortex/cortex.token</code>.<br>
If using the Control Center, manage the daemon from there. Do not run a second <code>cortex serve</code> alongside it.</p>

---

![](https://capsule-render.vercel.app/api?type=waving&color=0:6B4FBB,80:3b2580,100:0d1117&height=110&text=Release%20Verification&fontSize=36&fontColor=ffffff&fontAlign=50&fontAlignY=35&reversal=true)
<p align="center">After installing, verify the product path:</p>

```bash
cortex status --json
```

Windows:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\first-run-smoke.ps1
```

macOS / Linux:

```bash
bash tests/scripts/first-run-smoke.sh
```

<details>
<summary>Development build verification</summary>

```bash
# Daemon contract tests
cargo test -p cortex-tests

# Desktop test suite
npm --prefix desktop/cortex-control-center test

# Lifecycle smoke test
npm --prefix desktop/cortex-control-center run verify:lifecycle:dev

# Security audit
npm audit --omit=dev --audit-level=high
cargo audit
```

</details>

---

![](https://capsule-render.vercel.app/api?type=waving&color=0:4a2d8a,60:6B4FBB,100:2d1b69&height=110&text=Documentation&fontSize=36&fontColor=ffffff&fontAlign=50&fontAlignY=35)
<div align="center">

| Document | Covers |
|----------|--------|
| **[Docs index](Info/README.md)** | All product and operator docs |
| **[Connecting](Info/connecting.md)** | Setup, MCP, HTTP, auth, troubleshooting |
| **[Architecture](ARCHITECTURE.md)** | Store, CQR, boot, schema, crate map |
| **[MCP Tools](Info/mcp-tools.md)** | All 29 MCP tool definitions and parameters |
| **[Roadmap](Info/roadmap.md)** | What shipped and what's next |
| **[Security](Info/security-rules.md)** | Threat model, auth rules, vulnerability reporting |
| **[Team mode](Info/team-mode-setup.md)** | Shared-server setup for engineering teams |
| **[Contributing](CONTRIBUTING.md)** | Development setup and PR guidelines |

</div>

<details>
<summary>CLI reference</summary>

| Command | Description |
|---------|-------------|
| `cortex serve` | Start the daemon |
| `cortex mcp` | MCP stdio bridge to the running daemon |
| `cortex --help` | Full command reference |
| `cortex doctor` | Run diagnostics |
| `cortex paths --json` | Show file and port paths |
| `cortex status --json` | Local memory readiness and next action |
| `cortex rebuild-anchors` | Rebuild derived clock projections |
| `cortex setup --team` | Initialize team mode and generate API keys |
| `cortex export` | Export data (json or sql) |
| `cortex import` | Import from a previous export |
| `cortex admin rollback --session-id <id>` | Soft-delete a session's memory writes (dry-run default; `--apply` to persist) |

</details>

---

![](https://capsule-render.vercel.app/api?type=waving&color=0:6B4FBB,80:4a2d8a,100:1a1030&height=110&text=Security&fontSize=36&fontColor=ffffff&fontAlign=50&fontAlignY=35&reversal=true)
<p align="center">Cortex defaults to localhost-only access with bearer-token auth.<br>
Full threat model, auth rules, and vulnerability reporting: <a href="Info/security-rules.md"><b>Info/security-rules.md</b></a></p>

---

![](https://capsule-render.vercel.app/api?type=waving&color=0:5B3FA0,60:7B5FCC,100:1a1030&height=110&text=FAQ&fontSize=36&fontColor=ffffff&fontAlign=50&fontAlignY=35)

<details>
<summary>How much disk space does Cortex use?</summary>
<br>
The daemon binary is ~30 MB. The SQLite database grows with usage. Clock-Quorum Recall does not download or load a local model. Older installs may still have leftover files under <code>~/.cortex/models</code>; they are unused.
</details>

<details>
<summary>Can multiple agents write to Cortex at the same time?</summary>
<br>
Yes. SQLite WAL mode handles concurrent reads and serialized writes. Each agent maintains its own session while sharing the same memory. Conflict detection handles contradictions automatically.
</details>

<details>
<summary>Does Cortex send any data externally?</summary>
<br>
No. In solo mode, Cortex runs entirely on localhost. No telemetry, no phone-home, no cloud sync. Team mode sends data only to the configured team server over your network.
</details>

<details>
<summary>What happens if the daemon crashes mid-session?</summary>
<br>
The MCP proxy detects daemon death and restarts automatically (bounded to 3 attempts with backoff). SQLite WAL mode ensures no data corruption. Sessions survive transient crashes.
</details>

<details>
<summary>How do I reset Cortex to a clean state?</summary>
<br>
Delete <code>~/.cortex/cortex.db</code> and restart the daemon. A new empty database and auth token are generated. Control Center settings are preserved. No model download is required.
</details>

---

<p align="center"><b>Built by</b></p>

<p align="center">
  <a href="https://github.com/AdityaVG13/cortex/graphs/contributors">
    <img src="https://contrib.rocks/image?repo=AdityaVG13/cortex&max=20&columns=10" />
  </a>
</p>

---

<p align="center">
  <a href="https://ko-fi.com/adityavg13"><b>Support Cortex</b></a>&nbsp;&nbsp;·&nbsp;&nbsp;
  <a href="Info/README.md">Docs</a>&nbsp;&nbsp;·&nbsp;&nbsp;
  <a href="Info/connecting.md">Connecting</a>&nbsp;&nbsp;·&nbsp;&nbsp;
  <a href="Info/security-rules.md">Security</a>&nbsp;&nbsp;·&nbsp;&nbsp;
  <a href="CONTRIBUTING.md">Contributing</a>&nbsp;&nbsp;·&nbsp;&nbsp;
  <a href="CODE_OF_CONDUCT.md">Code of Conduct</a>&nbsp;&nbsp;·&nbsp;&nbsp;
  <a href="CHANGELOG.md">Changelog</a>&nbsp;&nbsp;·&nbsp;&nbsp;
  <a href="LICENSE">MIT License</a>
</p>
