# Developing Cortex Control Center

## Goal

The desktop app is the primary local operator surface for Cortex. When you test live daemon behavior through the desktop app, the rule is simple:

- one daemon
- app-managed
- no ad-hoc temp runtime spawn paths

## Core Rules

1. Launch the daemon through the desktop app when you are testing app-managed local mode.
2. Do not start random daemon binaries by hand for UI verification.
3. If you are validating plugin-only local mode, treat that as a separate path from app-managed mode.
4. If the app is running, other local clients should attach to the app-managed daemon instead of trying to create their own.

## Useful Commands

From the repo root:

```powershell
npm run desktop:install
npm run desktop:dev
npm run desktop:build
npm run desktop:expect
```

## What Each Command Does

- `npm run desktop:dev`
  - starts the Tauri desktop shell for local development
  - use this when you need to verify daemon lifecycle behavior through the app
- `npm run desktop:build`
  - compiles the desktop app for a release-style sanity check
  - use this before shipping UI or lifecycle changes
- `npm run desktop:expect`
  - runs the desktop smoke automation path when available

## One-Daemon Testing Guidance

- App-managed local mode:
  - start the desktop app
  - let the app own daemon start, stop, and restart
  - verify health and session truth through the control center
- Plugin-only local mode:
  - use the plugin's supported local attach/ensure path
  - keep that path separate from app-managed development so ownership stays clear

## Files You Will Touch Most Often

- `src/App.jsx`
  - control-center UI, lifecycle actions, and operator-facing copy
- `src/api-client.js`
  - desktop HTTP/auth retry behavior
- `src-tauri/src/main.rs`
  - app-owned daemon lifecycle commands
- `../../daemon-rs/src/main.rs`
  - daemon boot, readiness, and lifecycle policy
- `../../daemon-rs/src/handlers/`
  - daemon HTTP/MCP behavior the app depends on

## Release-Safety Notes

- User-facing copy should say `app-managed daemon` or equivalent, not `sidecar`, unless the code truly behaves like a sidecar.
- Restart behavior must prefer truthful recovery over optimistic messaging.
- If a change can make the app appear connected while daemon/session truth is stale, treat it as a bug.
