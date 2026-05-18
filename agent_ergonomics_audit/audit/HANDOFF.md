# Agent Ergonomics Handoff

Implemented pass 1 against `daemon-rs/src/main.rs`.

Changed behavior:
- `cortex`, `cortex help`, `cortex --help`, and `cortex -h` now print help successfully.
- `cortex capabilities --json` prints a deterministic machine-readable CLI contract.
- `cortex capabilities` prints a short human summary.
- `cortex robot-docs guide` prints a compact guide for coding agents.
- unknown top-level commands now include likely suggestions for common agent discovery intents.

Verification completed:
- `rtk cargo fmt`
- `rtk cargo test cli_`
- `rtk cargo check --all-targets`
- `rtk cargo clippy --all-targets -- -D warnings`
- `rtk cargo run --quiet -- --help`
- `rtk cargo run --quiet --`
- `rtk cargo run --quiet -- capabilities --json | jq -r '.contract_version, .tool.name, .commands.paths.output'`
- `rtk cargo run --quiet -- robot-docs guide`
- `rtk cargo run --quiet -- capability 2>&1`
- `powershell -NoProfile -ExecutionPolicy Bypass -File .\agent_ergonomics_audit\audit\regression_tests\R-001_cli_agent_surfaces.ps1`

Known limitation: the stock skill preflight could not complete inside WSL because `jq` was unavailable there. Windows has `jq.exe`, so command probes used the Windows environment.
