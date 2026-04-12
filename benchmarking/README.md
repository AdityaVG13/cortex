# Benchmarking Workspace

This directory is the reproducible home for external benchmark harnesses, Cortex adapters, configs, and saved benchmark outputs.

It intentionally separates:

- `benchmark/`: existing in-repo Cortex recall/metric scripts
- `benchmarking/`: external benchmark suites plus the glue needed to run Cortex against them

## Layout

- `benchmarks.lock.json`: pinned external benchmark repos and their target commits
- `setup-benchmarks.ps1`: clone/update the external suites into `benchmarking/tools/`
- `tools/`: ignored local clones of external benchmark repos
- `adapters/`: tracked Cortex adapters and harness glue
- `configs/`: tracked benchmark configs and run manifests
- `results/`: tracked summary outputs we want to keep in git
- `runs/`: ignored raw run logs, temp files, datasets, and scratch outputs

## Current External Suites

- `agent-memory-benchmark`: primary harness for baseline adapter work
- `LongMemEval`: long-context memory evaluation
- `locomo`: long conversation memory benchmark
- `MemoryAgentBench`: agent-memory benchmark suite

## Deferred / unresolved

- `MemBench` does not need a separate clone right now because AMB already has first-class support for it.
- `DMR` is still unresolved as a standalone canonical repo.
- Until we confirm a better upstream source, treat DMR coverage as something we may implement through the AMB adapter layer instead of cloning an unknown repo.

## Setup

From repo root:

```powershell
powershell -ExecutionPolicy Bypass -File benchmarking\setup-benchmarks.ps1
```

That will clone or update the pinned suites into `benchmarking/tools/` without polluting git history, because `benchmarking/tools/` is ignored.
