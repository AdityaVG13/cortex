# Plugin ↔ Daemon Version Lockstep

Cortex ships two artifacts that **must** move in lockstep:

1. The daemon binary version from `daemon-rs/Cargo.toml`
2. The plugin bundle version from `plugins/cortex-plugin/.claude-plugin/plugin.json`

A plugin bundle shipped with a different daemon version is a split-brain bug:
the bundled binary may speak an older MCP protocol, emit health payloads the
hook cannot parse, or write to schema columns the installed daemon does not
have. Historically this failure was discovered only during release smoke; the
lockstep guard catches it at PR time.

## Guard script

```bash
node scripts/check-plugin-lockstep.cjs
```

Exit codes:

| Code | Meaning |
|------|---------|
| 0 | Lockstep — versions match. (Warnings may still print.) |
| 1 | Version mismatch or missing input file. Release is blocked. |

Warnings cover advisory drift (e.g. the `prepare-runtime.cjs` hard-coded
fallback version). They do not block the release, but they should be fixed
alongside the next version bump.

## CI hook

Add the following to `.github/workflows/ci.yml` under any job that builds or
publishes the plugin:

```yaml
  - name: Plugin lockstep guard
    run: node scripts/check-plugin-lockstep.cjs
```

Recommended placement: first step of the `plugin-build` job, before any
`pnpm install` / `tar`-packaging steps. A failing lockstep check should abort
the build before any artifacts are produced.

## Local workflow — bumping versions

1. Edit `daemon-rs/Cargo.toml` `[package] version`.
2. Edit `plugins/cortex-plugin/.claude-plugin/plugin.json` `version`.
3. If `prepare-runtime.cjs` has a hard-coded fallback (`let version = '...'`),
   update it too.
4. Run `node scripts/check-plugin-lockstep.cjs` locally. It must print PASS
   with zero warnings before you commit.
5. Commit all version changes in a **single commit** so `git log -- daemon-rs
   Cargo.toml plugins/` never shows one file drifting ahead.

## Future enhancement TODO

Automate steps 1-3 via a `scripts/bump-release.sh VERSION` helper that edits
all three files and runs the guard. Tracked in
`docs/internal/v060/open-questions.md` Q11 as an advisory follow-up once the
v0.6.0 release stabilizes.
