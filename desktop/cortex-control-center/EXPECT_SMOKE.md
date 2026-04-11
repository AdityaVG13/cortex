# Expect Smoke Workflow

This app now has a committed browser smoke path that does not depend on a live Cortex daemon.

## What it does

- Builds the Vite frontend.
- Starts a mock Cortex API on `127.0.0.1:7438` by default.
- Starts `vite preview` on `127.0.0.1:4173`.
- Injects browser bootstrap query params for `cortexBase` and `authToken`.
- Runs the repo-local `expect-cli` binary through `npm exec`.
- Verifies both the overview shell and the live Work surface against the same fixture-backed state.

That keeps local runs, source builds, and CI aligned on the same fixture-backed flow.

## Local usage

From the desktop app directory:

```bash
npm run expect:smoke
```

For the faster Work-surface-only loop:

```bash
npm run expect:smoke:work
```

From the repo root:

```bash
npm run desktop:expect
```

For the repo-root Work-only path:

```bash
npm run desktop:expect:work
```

## Requirements

- `npm ci` must have been run in `desktop/cortex-control-center`.
- A supported agent CLI must be installed and authenticated on your machine.
- `expect-cli` is already pinned in `package.json`, so the version is deterministic.

If you want to force a specific agent:

```bash
EXPECT_CLI_AGENT=codex npm run expect:smoke
```

Supported agent values match `expect-cli`: `codex`, `claude`, `gemini`, `copilot`, `opencode`, and `droid`.

If `7438` is occupied on your machine, override it:

```bash
EXPECT_SMOKE_API_PORT=7448 npm run expect:smoke
```

## CI usage

The desktop job in `.github/workflows/ci.yml` can run the same smoke flow.

Enable it by setting:

- repo variable `EXPECT_CLI_ENABLED=true`
- repo variable `EXPECT_CLI_AGENT=codex` (or another CLI-installable agent)
- the matching provider secret, for example `OPENAI_API_KEY` for `codex`

When enabled, CI will:

- install the selected agent CLI on the runner
- run `npm run expect:smoke:ci`

The smoke step is intentionally opt-in because hosted runners do not come with agent auth configured by default.

## Why the mock API exists

- The app’s browser fallback expects auth on protected endpoints.
- Live daemon state makes smoke verification flaky and machine-specific.
- The smoke path runs against browser preview, so it does not need the live daemon port and will not collide with a running Control Center session.

## Files

- `scripts/mock-cortex-server.mjs`
- `scripts/run-expect-smoke.mjs`
- `package.json`
- `.github/workflows/ci.yml`

## Current Work-surface coverage

The default smoke run now checks that the mock-backed Work surface can:

- select an explicit operator
- claim and complete a pending task
- unlock an operator-owned file lock
- send a message from the selected operator
- acknowledge visible feed entries

For day-to-day UI iteration, prefer `expect:smoke:work`; keep the full `expect:smoke` run for broader pre-commit coverage.
