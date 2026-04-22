# Contributing to Cortex

Thanks for contributing. Cortex is a local-first memory system for AI agents, with a Rust daemon and a Tauri desktop app. Keep changes focused, documented, and easy to verify.

## Before You Start

- Search existing issues and pull requests before opening a new one.
- Prefer small, reviewable PRs over broad refactors.
- If your change affects behavior, update docs in the same PR.
- If your change affects APIs or workflows, update [README.md](README.md) or [Info/connecting.md](Info/connecting.md).

## Multi-Model Development

- Cortex work is coordinated across Claude, Gemini, Codex, and local models.
- Check open issues and [Info/roadmap.md](Info/roadmap.md) before starting to avoid duplicate work.
- If you claim a roadmap item, note scope and milestone clearly in your PR description.

## Development Setup

### Prerequisites

- **Rust 1.78+** (required for daemon): Install from <https://rustup.rs/>
- **Node.js 18+** (required for desktop app): Install from <https://nodejs.org/>
- **Windows users**: MSVC build tools (required for Rust): `x86 Native Tools Command Prompt for VS`
- **Linux users**: `pkg-config` and `libssl-dev` (required for some dependencies): `sudo apt install pkg-config libssl-dev`
- **macOS users**: Xcode command line tools: `xcode-select --install`

### Core daemon

```bash
git clone https://github.com/cortex-project/cortex.git
cd cortex
cd daemon-rs
cargo build --release
# Binary at target/release/cortex(.exe)
```

### Desktop app

```bash
cd desktop/cortex-control-center
npm ci
npm run dev
```

## Recommended Checks

Run the checks relevant to the area you changed.

### Daemon / Rust changes

```bash
cd daemon-rs
cargo fmt
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

### Desktop / frontend changes

```bash
cd desktop/cortex-control-center
npm test
npm run build
npm run expect:smoke
```

### Root convenience scripts

From the repo root:

```bash
npm run test
npm run build
npm run desktop:build
npm run desktop:expect
```

## Pull Request Guidelines

- Use a clear title and explain the user-visible impact.
- Include screenshots or GIFs for UI changes.
- Call out breaking changes explicitly.
- Note any follow-up work that is intentionally out of scope.

## Documentation Expectations

Please update docs when you change:

- installation or setup steps,
- public HTTP or MCP behavior,
- auth or security expectations,
- desktop workflows,
- release artifacts or supported platforms.

## Scope and Style

- Keep dependencies justified; local-first and low-runtime-complexity are project priorities.
- Avoid silently changing security-sensitive defaults.
- Do not commit local databases, logs, personal config, or machine-specific artifacts.

For security issues, do not open a public issue with exploit details. Follow [Info/security-rules.md](Info/security-rules.md).
